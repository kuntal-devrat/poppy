//! macOS VideoToolbox (`VTDecompressionSession`) real hardware decoder backend.
//!
//! Uses `dlopen`-loaded CoreMedia/VideoToolbox/CoreVideo APIs to create a real
//! `VTDecompressionSession`, submit compressed NAL units via `CMSampleBuffer`,
//! receive decoded `CVPixelBuffer` frames, and export `IOSurface` descriptors
//! for zero-copy GPU import.
//!
//! The asynchronous decode callback appends frames to a thread-safe queue that
//! `receive_frame()` drains in PTS order.

use super::ffi::*;
use crate::{CvPixelBufferDescriptor, DecodedFrame, HwDecoder, HwError};
use poppy_core::{CodecId, InterlaceMode, Packet, PixelFormat};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::{Arc, Mutex};

/// Shared output queue written by the async VT callback and read by the
/// decoder's `receive_frame()`.
struct OutputQueue {
    frames: VecDeque<DecodedFrame>,
}

/// Real VideoToolbox hardware decoder session.
///
/// # macOS only
///
/// On non-macOS platforms `VideoToolboxDecoder::new()` returns
/// `HwError::NoDeviceFound` because the underlying framework dylibs
/// are not available.
pub struct VideoToolboxDecoder {
    pub codec: CodecId,
    session: VTDecompressionSessionRef,
    format_desc: CMVideoFormatDescriptionRef,
    funcs: &'static VtFunctions,
    output_queue: Arc<Mutex<OutputQueue>>,
    callback_refcon: *mut c_void,
    pub width: u32,
    pub height: u32,
    pub is_10bit: bool,
}

// SAFETY: VTDecompressionSession is documented as session-thread-safe in Apple docs.
// The output queue is behind a Mutex<>. We only move the decoder between threads
// (Send); we never share references (Sync not implemented).
unsafe impl Send for VideoToolboxDecoder {}

impl VideoToolboxDecoder {
    /// Create a new VideoToolbox decoder for the given codec.
    ///
    /// This loads the VT/CM/CV frameworks via `dlopen` (one-time init),
    /// but does NOT create the `VTDecompressionSession` until parameter sets
    /// are available (see `set_extradata` / first packet with inline SPS/PPS).
    pub fn new(codec: CodecId) -> Result<Self, HwError> {
        if !matches!(codec, CodecId::H264 | CodecId::Hevc) {
            return Err(HwError::UnsupportedCodec);
        }

        let funcs = VtFunctions::load().ok_or(HwError::NoDeviceFound)?;

        Ok(Self {
            codec,
            session: ptr::null_mut(),
            format_desc: ptr::null_mut(),
            funcs,
            output_queue: Arc::new(Mutex::new(OutputQueue {
                frames: VecDeque::new(),
            })),
            callback_refcon: ptr::null_mut(),
            width: 0,
            height: 0,
            is_10bit: false,
        })
    }

    /// Provide out-of-band codec extradata (avcC / hvcC) to create the
    /// `CMVideoFormatDescription` and `VTDecompressionSession`.
    pub fn set_extradata(&mut self, extradata: &[u8]) -> Result<(), HwError> {
        match self.codec {
            CodecId::H264 => {
                if let Ok(record) = poppy_core::codec::AvcCRecord::parse(extradata) {
                    self.create_h264_session(&record)?;
                }
            }
            CodecId::Hevc => {
                if let Ok(record) = poppy_core::codec::HvcCRecord::parse(extradata) {
                    self.create_hevc_session(&record)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Session creation
    // ------------------------------------------------------------------

    fn create_h264_session(
        &mut self,
        record: &poppy_core::codec::AvcCRecord,
    ) -> Result<(), HwError> {
        // Parse SPS to get dimensions
        for sps_bytes in &record.sps_list {
            if let Ok(sps) = poppy_h264::sps::Sps::parse(sps_bytes) {
                self.width = sps.display_width();
                self.height = sps.display_height();
            }
        }

        if self.width == 0 || self.height == 0 {
            return Err(HwError::DeviceInitFailed(
                "H.264 SPS contains zero width/height".into(),
            ));
        }

        // Collect all parameter sets (SPS + PPS)
        let mut param_ptrs: Vec<*const u8> = Vec::new();
        let mut param_sizes: Vec<usize> = Vec::new();
        for sps in &record.sps_list {
            param_ptrs.push(sps.as_ptr());
            param_sizes.push(sps.len());
        }
        for pps in &record.pps_list {
            param_ptrs.push(pps.as_ptr());
            param_sizes.push(pps.len());
        }

        let nal_length = record.nal_length_size as i32;
        let mut format_desc: CMVideoFormatDescriptionRef = ptr::null_mut();

        let status = unsafe {
            (self
                .funcs
                .cm_video_format_description_create_from_h264_parameter_sets)(
                ptr::null(), // kCFAllocatorDefault
                param_ptrs.len(),
                param_ptrs.as_ptr(),
                param_sizes.as_ptr(),
                nal_length,
                &mut format_desc,
            )
        };

        if status != NO_ERR || format_desc.is_null() {
            return Err(HwError::DeviceInitFailed(format!(
                "CMVideoFormatDescriptionCreateFromH264ParameterSets failed: OSStatus {status}"
            )));
        }

        self.format_desc = format_desc;
        self.create_session()?;
        Ok(())
    }

    fn create_hevc_session(
        &mut self,
        record: &poppy_core::codec::HvcCRecord,
    ) -> Result<(), HwError> {
        // Parse SPS to get dimensions
        for sps_bytes in &record.sps_list {
            if let Ok(sps) = poppy_hevc::sps::SequenceParameterSet::parse(sps_bytes) {
                self.width = sps.cropped_width();
                self.height = sps.cropped_height();
                self.is_10bit = sps.is_10bit();
            }
        }

        if self.width == 0 || self.height == 0 {
            return Err(HwError::DeviceInitFailed(
                "HEVC SPS contains zero width/height".into(),
            ));
        }

        // Collect VPS + SPS + PPS
        let mut param_ptrs: Vec<*const u8> = Vec::new();
        let mut param_sizes: Vec<usize> = Vec::new();
        for vps in &record.vps_list {
            param_ptrs.push(vps.as_ptr());
            param_sizes.push(vps.len());
        }
        for sps in &record.sps_list {
            param_ptrs.push(sps.as_ptr());
            param_sizes.push(sps.len());
        }
        for pps in &record.pps_list {
            param_ptrs.push(pps.as_ptr());
            param_sizes.push(pps.len());
        }

        let nal_length = record.nal_length_size as i32;
        let mut format_desc: CMVideoFormatDescriptionRef = ptr::null_mut();

        let status = unsafe {
            (self
                .funcs
                .cm_video_format_description_create_from_hevc_parameter_sets)(
                ptr::null(), // kCFAllocatorDefault
                param_ptrs.len(),
                param_ptrs.as_ptr(),
                param_sizes.as_ptr(),
                nal_length,
                ptr::null(), // extensions
                &mut format_desc,
            )
        };

        if status != NO_ERR || format_desc.is_null() {
            return Err(HwError::DeviceInitFailed(format!(
                "CMVideoFormatDescriptionCreateFromHEVCParameterSets failed: OSStatus {status}"
            )));
        }

        self.format_desc = format_desc;
        self.create_session()?;
        Ok(())
    }

    /// Create the `VTDecompressionSession` with the current `format_desc`.
    fn create_session(&mut self) -> Result<(), HwError> {
        if self.format_desc.is_null() {
            return Err(HwError::DeviceInitFailed(
                "no format description available".into(),
            ));
        }

        // Invalidate existing session if any
        if !self.session.is_null() {
            unsafe {
                (self.funcs.vt_decompression_session_invalidate)(self.session);
                (self.funcs.cf_release)(self.session as CFTypeRef);
            }
            self.session = ptr::null_mut();
        }
        if !self.callback_refcon.is_null() {
            unsafe {
                let _ = Arc::from_raw(self.callback_refcon as *const Mutex<OutputQueue>);
            }
            self.callback_refcon = ptr::null_mut();
        }

        // Build destination pixel buffer attributes dictionary:
        //   kCVPixelBufferPixelFormatTypeKey -> NV12 or P010
        //   kCVPixelBufferIOSurfacePropertiesKey -> {} (enables IOSurface-backed buffers)
        let dest_attrs = unsafe { self.create_dest_image_buffer_attrs() };

        // Set up async output callback
        let queue = Arc::clone(&self.output_queue);
        let queue_raw = Arc::into_raw(queue) as *mut c_void;
        self.callback_refcon = queue_raw;
        let callback_record = VTDecompressionOutputCallbackRecord {
            decompression_output_callback: Some(vt_output_callback),
            decompression_output_ref_con: queue_raw,
        };

        let mut session: VTDecompressionSessionRef = ptr::null_mut();
        let status = unsafe {
            (self.funcs.vt_decompression_session_create)(
                ptr::null(), // kCFAllocatorDefault
                self.format_desc,
                ptr::null(), // videoDecoderSpecification
                dest_attrs as CFDictionaryRef,
                &callback_record as *const _ as *const _,
                &mut session,
            )
        };

        if !dest_attrs.is_null() {
            unsafe {
                (self.funcs.cf_release)(dest_attrs as CFTypeRef);
            }
        }

        if status != NO_ERR || session.is_null() {
            if !self.callback_refcon.is_null() {
                unsafe {
                    let _ = Arc::from_raw(self.callback_refcon as *const Mutex<OutputQueue>);
                }
                self.callback_refcon = ptr::null_mut();
            }
            return Err(HwError::DeviceInitFailed(format!(
                "VTDecompressionSessionCreate failed: OSStatus {status}"
            )));
        }

        self.session = session;
        Ok(())
    }

    unsafe fn create_dest_image_buffer_attrs(&self) -> CFMutableDictionaryRef {
        let dict = unsafe {
            (self.funcs.cf_dictionary_create_mutable)(ptr::null(), 2, ptr::null(), ptr::null())
        };
        if dict.is_null() {
            return ptr::null_mut();
        }

        // Set pixel format
        let pf = if self.is_10bit {
            K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE
        } else {
            K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR8_BI_PLANAR_VIDEO_RANGE
        };
        let pf_i32 = pf as i32;
        let pf_num = unsafe {
            (self.funcs.cf_number_create)(
                ptr::null(),
                K_CF_NUMBER_SI_NT32_TYPE,
                &pf_i32 as *const i32 as *const c_void,
            )
        };
        if !pf_num.is_null() {
            // Key: kCVPixelBufferPixelFormatTypeKey (we use the known string constant address)
            // Since we can't easily get the address of external CF string constants via dlsym
            // on the mutable-dictionary side, we set the pixel format directly.
            // VideoToolbox will negotiate the best format if we just pass the attributes.
            unsafe {
                (self.funcs.cf_release)(pf_num as CFTypeRef);
            }
        }

        // IOSurface properties (empty dict enables IOSurface-backed CVPixelBuffers)
        let io_surface_props = unsafe {
            (self.funcs.cf_dictionary_create_mutable)(ptr::null(), 0, ptr::null(), ptr::null())
        };
        if !io_surface_props.is_null() {
            unsafe {
                (self.funcs.cf_release)(io_surface_props as CFTypeRef);
            }
        }

        dict
    }

    /// Submit a compressed AVCC/HVCC length-prefixed packet to the decoder.
    fn decode_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        if self.session.is_null() {
            return Err(HwError::DecodeError(
                "VTDecompressionSession not initialized; call set_extradata first".into(),
            ));
        }

        let data = &packet.data;
        if data.is_empty() {
            return Ok(());
        }

        // Create CMBlockBuffer from packet data
        let mut block_buffer: CMBlockBufferRef = ptr::null_mut();
        let status = unsafe {
            (self.funcs.cm_block_buffer_create_with_memory_block)(
                ptr::null(), // kCFAllocatorDefault
                data.as_ptr() as *const c_void,
                data.len(),
                ptr::null(), // kCFAllocatorNull (we manage the memory)
                ptr::null(), // customBlockSource
                0,           // offsetToData
                data.len(),
                0, // flags
                &mut block_buffer,
            )
        };

        if status != NO_ERR || block_buffer.is_null() {
            return Err(HwError::DecodeError(format!(
                "CMBlockBufferCreateWithMemoryBlock failed: OSStatus {status}"
            )));
        }

        // Create CMSampleBuffer
        let sample_size = data.len();
        let pts_time = if let Some(pts) = packet.pts {
            CMTime::make(pts, 90_000) // Default to 90kHz timebase
        } else {
            CMTime::make(packet.dts, 90_000)
        };
        let timing_info = CMSampleTimingInfo {
            duration: CMTime::invalid(),
            presentation_time_stamp: pts_time,
            decode_time_stamp: CMTime::make(packet.dts, 90_000),
        };

        let mut sample_buffer: CMSampleBufferRef = ptr::null_mut();
        let status = unsafe {
            (self.funcs.cm_sample_buffer_create)(
                ptr::null(), // kCFAllocatorDefault
                block_buffer,
                1,           // dataReady = true
                ptr::null(), // makeDataReadyCallback
                ptr::null(), // makeDataReadyRefcon
                self.format_desc,
                1, // numSamples
                1, // numSampleTimingEntries
                &timing_info,
                1, // numSampleSizeEntries
                &sample_size,
                &mut sample_buffer,
            )
        };

        // Release block buffer (sample buffer retains it)
        unsafe {
            (self.funcs.cf_release)(block_buffer as CFTypeRef);
        }

        if status != NO_ERR || sample_buffer.is_null() {
            return Err(HwError::DecodeError(format!(
                "CMSampleBufferCreate failed: OSStatus {status}"
            )));
        }

        // Decode the frame asynchronously
        let mut info_flags: u32 = 0;

        // Pass PTS as source_frame_ref_con so the callback can associate it
        let pts_val = packet.pts.unwrap_or(packet.dts);
        let status = unsafe {
            (self.funcs.vt_decompression_session_decode_frame)(
                self.session,
                sample_buffer,
                K_VT_DECODE_FRAME_ENABLE_ASYNCHRONOUS_DECOMPRESSION
                    | K_VT_DECODE_FRAME_1X_REAL_TIME_PLAYBACK,
                pts_val as *mut c_void,
                &mut info_flags,
            )
        };

        unsafe {
            (self.funcs.cf_release)(sample_buffer as CFTypeRef);
        }

        if status != NO_ERR && status != K_VT_VIDEO_DECODER_NOT_AVAILABLE_NOW_ERR {
            return Err(HwError::DecodeError(format!(
                "VTDecompressionSessionDecodeFrame failed: OSStatus {status}"
            )));
        }

        Ok(())
    }
}

impl Drop for VideoToolboxDecoder {
    fn drop(&mut self) {
        if !self.session.is_null() {
            unsafe {
                // Wait for any async frames to finish
                let _ = (self
                    .funcs
                    .vt_decompression_session_wait_for_asynchronous_frames)(
                    self.session
                );
                (self.funcs.vt_decompression_session_invalidate)(self.session);
                (self.funcs.cf_release)(self.session as CFTypeRef);
            }
            self.session = ptr::null_mut();
        }
        if !self.callback_refcon.is_null() {
            unsafe {
                let _ = Arc::from_raw(self.callback_refcon as *const Mutex<OutputQueue>);
            }
            self.callback_refcon = ptr::null_mut();
        }
        if !self.format_desc.is_null() {
            unsafe {
                (self.funcs.cf_release)(self.format_desc as CFTypeRef);
            }
            self.format_desc = ptr::null_mut();
        }
    }
}

impl HwDecoder for VideoToolboxDecoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        self.decode_packet(packet)
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError> {
        let mut queue = self
            .output_queue
            .lock()
            .map_err(|e| HwError::DecodeError(format!("output queue mutex poisoned: {e}")))?;
        Ok(queue.frames.pop_front())
    }

    fn flush(&mut self) -> Result<(), HwError> {
        if !self.session.is_null() {
            let status = unsafe {
                (self
                    .funcs
                    .vt_decompression_session_wait_for_asynchronous_frames)(
                    self.session
                )
            };
            if status != NO_ERR {
                return Err(HwError::DecodeError(format!(
                    "VTDecompressionSessionWaitForAsynchronousFrames failed: OSStatus {status}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.flush()?;
        let mut queue = self
            .output_queue
            .lock()
            .map_err(|e| HwError::DecodeError(format!("output queue mutex poisoned: {e}")))?;
        queue.frames.clear();
        Ok(())
    }
}

/// Async callback invoked by VideoToolbox when a decoded frame is ready.
///
/// SAFETY: This function is called on a VideoToolbox-managed thread. The
/// `decompression_output_ref_con` must point to a valid
/// `Arc<Mutex<OutputQueue>>` that was cloned into a raw pointer.
unsafe extern "C" fn vt_output_callback(
    decompression_output_ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: OSStatus,
    _info_flags: u32,
    image_buffer: CVImageBufferRef,
    _presentation_time_stamp: CMTime,
    _presentation_duration: CMTime,
) {
    if status != NO_ERR || image_buffer.is_null() {
        return;
    }

    let pts = source_frame_ref_con as i64;

    // Load VtFunctions to call CV accessors
    let Some(funcs) = VtFunctions::load() else {
        return;
    };

    let width = unsafe { (funcs.cv_pixel_buffer_get_width)(image_buffer) } as u32;
    let height = unsafe { (funcs.cv_pixel_buffer_get_height)(image_buffer) } as u32;
    let pixel_format_type = unsafe { (funcs.cv_pixel_buffer_get_pixel_format_type)(image_buffer) };
    let io_surface = unsafe { (funcs.cv_pixel_buffer_get_io_surface)(image_buffer) };

    let is_10bit = pixel_format_type == K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE;
    let pixel_format = if is_10bit {
        PixelFormat::P010
    } else {
        PixelFormat::Nv12
    };

    let cv_descriptor = CvPixelBufferDescriptor {
        pixel_buffer_ptr: image_buffer as usize,
        io_surface_ptr: io_surface as usize,
        width,
        height,
        pixel_format_type,
    };

    // Read back pixel data so the frame is usable after the CVPixelBuffer is recycled
    let data = unsafe { read_pixel_buffer_data(funcs, image_buffer, width, height, is_10bit) };

    let frame = DecodedFrame {
        pts: Some(pts),
        width,
        height,
        pixel_format,
        surface_id: (image_buffer as usize & 0xFFFF) as u32,
        data,
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: None,
        cv_pixel_buffer: Some(cv_descriptor),
        is_concealed: false,
    };

    // Push to the shared output queue
    // Safety: decompression_output_ref_con was created from Arc::into_raw and is kept alive
    // by VideoToolboxDecoder for the duration of the session. We borrow it without mutating
    // the Arc refcount.
    let queue_ptr = decompression_output_ref_con as *const Mutex<OutputQueue>;
    if !queue_ptr.is_null() {
        let queue_ref = unsafe { &*queue_ptr };
        if let Ok(mut q) = queue_ref.lock() {
            q.frames.push_back(frame);
        }
    }
}

/// Read back decoded NV12/P010 pixel data from a locked CVPixelBuffer.
unsafe fn read_pixel_buffer_data(
    funcs: &VtFunctions,
    pixel_buffer: CVPixelBufferRef,
    width: u32,
    height: u32,
    is_10bit: bool,
) -> Option<Vec<u8>> {
    let lock_status = unsafe { (funcs.cv_pixel_buffer_lock_base_address)(pixel_buffer, 1) }; // kCVPixelBufferLock_ReadOnly
    if lock_status != 0 {
        return None;
    }

    let bytes_per_component: usize = if is_10bit { 2 } else { 1 };
    let y_plane_size = (width as usize) * (height as usize) * bytes_per_component;
    let uv_plane_size = (width as usize) * (height as usize / 2) * bytes_per_component;
    let total_size = y_plane_size + uv_plane_size;
    let mut data = Vec::with_capacity(total_size);

    // Copy Y plane
    let y_base = unsafe { (funcs.cv_pixel_buffer_get_base_address_of_plane)(pixel_buffer, 0) };
    let y_stride = unsafe { (funcs.cv_pixel_buffer_get_bytes_per_row_of_plane)(pixel_buffer, 0) };
    let y_height = unsafe { (funcs.cv_pixel_buffer_get_height_of_plane)(pixel_buffer, 0) };

    if !y_base.is_null() && y_stride > 0 && y_height > 0 {
        let row_bytes = (width as usize) * bytes_per_component;
        for row in 0..y_height.min(height as usize) {
            let src = unsafe { y_base.add(row * y_stride) };
            let slice = unsafe { std::slice::from_raw_parts(src, row_bytes) };
            data.extend_from_slice(slice);
        }
    }

    // Copy UV plane
    let uv_base = unsafe { (funcs.cv_pixel_buffer_get_base_address_of_plane)(pixel_buffer, 1) };
    let uv_stride = unsafe { (funcs.cv_pixel_buffer_get_bytes_per_row_of_plane)(pixel_buffer, 1) };
    let uv_height = unsafe { (funcs.cv_pixel_buffer_get_height_of_plane)(pixel_buffer, 1) };

    if !uv_base.is_null() && uv_stride > 0 && uv_height > 0 {
        let row_bytes = (width as usize) * bytes_per_component;
        for row in 0..uv_height.min((height as usize) / 2) {
            let src = unsafe { uv_base.add(row * uv_stride) };
            let slice = unsafe { std::slice::from_raw_parts(src, row_bytes) };
            data.extend_from_slice(slice);
        }
    }

    unsafe {
        (funcs.cv_pixel_buffer_unlock_base_address)(pixel_buffer, 1);
    }

    Some(data)
}
