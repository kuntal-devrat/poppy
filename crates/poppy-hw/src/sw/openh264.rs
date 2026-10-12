//! Cisco OpenH264 software video decoder backend.
//!
//! Loads `libopenh264` dynamically at runtime via `dlopen` (Linux/macOS)
//! or `LoadLibrary` (Windows). Compiles universally without static C build
//! dependencies and falls back gracefully when `openh264` is not installed.

use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_long, c_uchar, c_uint, c_ulonglong, c_void};
use std::ptr;

use bytes::Bytes;
use poppy_core::{InterlaceMode, Packet, PixelFormat};

use crate::{DecodedFrame, HwDecoder, HwError};

// ---------------------------------------------------------------------------
// OpenH264 C API types and structs (ISO/IEC 14496-10 / OpenH264 2.x)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SVideoProperty {
    pub size: c_uint,
    pub in_return_video_format: c_int,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SDecodingParam {
    pub p_file_name_restructed: *mut c_char,
    pub ui_cpu_flag: c_uint,
    pub ui_target_dq_layer: c_uchar,
    pub e_ec_active_idc: c_int,
    pub b_parse_only: bool,
    pub s_video_property: SVideoProperty,
}

impl Default for SDecodingParam {
    fn default() -> Self {
        Self {
            p_file_name_restructed: ptr::null_mut(),
            ui_cpu_flag: 0,
            ui_target_dq_layer: 0xFF,
            e_ec_active_idc: 1, // ERROR_CON_SYN_CHECK_CONCEALMENT
            b_parse_only: false,
            s_video_property: SVideoProperty::default(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SSystemBuffer {
    pub i_width: c_int,
    pub i_height: c_int,
    pub i_format: c_int,
    pub i_stride: [c_int; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SBufferInfo {
    pub i_buffer_status: c_int,
    pub ui_in_bs_timestamp: c_ulonglong,
    pub ui_out_yuv_timestamp: c_ulonglong,
    pub usr_data: SSystemBuffer,
}

// OpenH264 ISVCDecoder Vtable layout
#[repr(C)]
struct ISVCDecoderVtbl {
    initialize: unsafe extern "C" fn(this: *mut c_void, p_param: *const SDecodingParam) -> c_long,
    uninitialize: unsafe extern "C" fn(this: *mut c_void) -> c_long,
    decode_frame: unsafe extern "C" fn(
        this: *mut c_void,
        p_src: *const u8,
        i_src_len: c_int,
        pp_dst: *mut *mut u8,
        p_stride: *mut c_int,
        i_width: *mut c_int,
        i_height: *mut c_int,
    ) -> c_long,
    decode_frame_no_delay: unsafe extern "C" fn(
        this: *mut c_void,
        p_src: *const u8,
        i_src_len: c_int,
        pp_dst: *mut *mut u8,
        p_dst_buf_info: *mut SBufferInfo,
    ) -> c_long,
    decode_frame2: unsafe extern "C" fn(
        this: *mut c_void,
        p_src: *const u8,
        i_src_len: c_int,
        pp_dst: *mut *mut u8,
        p_dst_buf_info: *mut SBufferInfo,
    ) -> c_long,
    decode_parser: unsafe extern "C" fn(
        this: *mut c_void,
        p_src: *const u8,
        i_src_len: c_int,
        p_dst_info: *mut c_void,
    ) -> c_long,
    set_option: unsafe extern "C" fn(
        this: *mut c_void,
        e_option_id: c_int,
        p_option: *mut c_void,
    ) -> c_long,
    get_option: unsafe extern "C" fn(
        this: *mut c_void,
        e_option_id: c_int,
        p_option: *mut c_void,
    ) -> c_long,
}

type WelsCreateDecoderFn = unsafe extern "C" fn(pp_decoder: *mut *mut c_void) -> c_int;
type WelsDestroyDecoderFn = unsafe extern "C" fn(p_decoder: *mut c_void);

struct OpenH264Lib {
    create_decoder: WelsCreateDecoderFn,
    destroy_decoder: WelsDestroyDecoderFn,
}

impl OpenH264Lib {
    fn load() -> Option<Self> {
        #[cfg(target_os = "linux")]
        let candidates = [
            "libopenh264.so.7",
            "libopenh264.so.6",
            "libopenh264.so",
            "/usr/lib/x86_64-linux-gnu/libopenh264.so.7",
            "/usr/lib/libopenh264.so.7",
        ];

        #[cfg(target_os = "macos")]
        let candidates = [
            "libopenh264.7.dylib",
            "libopenh264.dylib",
            "/opt/homebrew/lib/libopenh264.dylib",
            "/usr/local/lib/libopenh264.dylib",
        ];

        #[cfg(target_os = "windows")]
        let candidates = ["openh264-7.dll", "openh264.dll"];

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let candidates: [&str; 0] = [];

        for name in candidates {
            let c_name = match std::ffi::CString::new(name) {
                Ok(c) => c,
                Err(_) => continue,
            };

            #[cfg(unix)]
            unsafe {
                let handle = libc::dlopen(c_name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
                if !handle.is_null() {
                    let create_sym = libc::dlsym(handle, c"WelsCreateDecoder".as_ptr());
                    let destroy_sym = libc::dlsym(handle, c"WelsDestroyDecoder".as_ptr());

                    if !create_sym.is_null() && !destroy_sym.is_null() {
                        return Some(Self {
                            create_decoder: std::mem::transmute::<
                                *mut libc::c_void,
                                WelsCreateDecoderFn,
                            >(create_sym),
                            destroy_decoder: std::mem::transmute::<
                                *mut libc::c_void,
                                WelsDestroyDecoderFn,
                            >(destroy_sym),
                        });
                    }
                }
            }

            #[cfg(target_os = "windows")]
            unsafe {
                unsafe extern "system" {
                    fn LoadLibraryA(lpLibFileName: *const c_char) -> *mut c_void;
                    fn GetProcAddress(
                        hModule: *mut c_void,
                        lpProcName: *const c_char,
                    ) -> *mut c_void;
                }
                let handle = LoadLibraryA(c_name.as_ptr());
                if !handle.is_null() {
                    let create_sym = GetProcAddress(handle, c"WelsCreateDecoder".as_ptr());
                    let destroy_sym = GetProcAddress(handle, c"WelsDestroyDecoder".as_ptr());

                    if !create_sym.is_null() && !destroy_sym.is_null() {
                        return Some(Self {
                            create_decoder: std::mem::transmute::<*mut c_void, WelsCreateDecoderFn>(
                                create_sym,
                            ),
                            destroy_decoder: std::mem::transmute::<*mut c_void, WelsDestroyDecoderFn>(
                                destroy_sym,
                            ),
                        });
                    }
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// OpenH264Decoder implementation
// ---------------------------------------------------------------------------

/// Pure software H.264 video decoder using dynamic OpenH264 integration.
pub struct OpenH264Decoder {
    decoder_ptr: *mut c_void,
    lib: &'static OpenH264Lib,
    output_queue: VecDeque<DecodedFrame>,
    cur_pts: Option<i64>,
}

// SAFETY: ISVCDecoder is safe to move across threads when synchronized.
unsafe impl Send for OpenH264Decoder {}

static OPENH264_LIB: std::sync::OnceLock<Option<OpenH264Lib>> = std::sync::OnceLock::new();

impl OpenH264Decoder {
    /// Attempts to dynamically instantiate an OpenH264 software decoder.
    ///
    /// Returns `HwError::NoDeviceFound` if `libopenh264` is not installed on the system.
    pub fn new() -> Result<Self, HwError> {
        let lib_opt = OPENH264_LIB.get_or_init(OpenH264Lib::load);
        let lib = lib_opt.as_ref().ok_or(HwError::NoDeviceFound)?;

        let mut decoder_ptr: *mut c_void = ptr::null_mut();
        let ret = unsafe { (lib.create_decoder)(&mut decoder_ptr) };
        if ret != 0 || decoder_ptr.is_null() {
            return Err(HwError::DeviceInitFailed(
                "WelsCreateDecoder failed to allocate decoder".into(),
            ));
        }

        // Initialize decoder with default parameters
        let param = SDecodingParam::default();
        let vtbl = unsafe { *(decoder_ptr as *const *const ISVCDecoderVtbl) };
        let init_ret = unsafe { ((*vtbl).initialize)(decoder_ptr, &param) };
        if init_ret != 0 {
            unsafe { (lib.destroy_decoder)(decoder_ptr) };
            return Err(HwError::DeviceInitFailed(format!(
                "ISVCDecoder::Initialize failed: {init_ret}"
            )));
        }

        Ok(Self {
            decoder_ptr,
            lib,
            output_queue: VecDeque::new(),
            cur_pts: None,
        })
    }

    /// Feeds parameter sets (SPS / PPS) to the decoder.
    pub fn set_parameter_sets(
        &mut self,
        sps_list: &[Bytes],
        pps_list: &[Bytes],
    ) -> Result<(), HwError> {
        for sps in sps_list {
            let mut annex_b = vec![0, 0, 0, 1];
            annex_b.extend_from_slice(sps);
            let pkt = Packet {
                stream_index: 0,
                pts: None,
                dts: 0,
                duration: 0,
                data: Bytes::from(annex_b),
                is_keyframe: false,
            };
            let _ = self.submit_packet(&pkt);
        }
        for pps in pps_list {
            let mut annex_b = vec![0, 0, 0, 1];
            annex_b.extend_from_slice(pps);
            let pkt = Packet {
                stream_index: 0,
                pts: None,
                dts: 0,
                duration: 0,
                data: Bytes::from(annex_b),
                is_keyframe: false,
            };
            let _ = self.submit_packet(&pkt);
        }
        Ok(())
    }
}

impl HwDecoder for OpenH264Decoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        let data = &packet.data;
        if data.is_empty() {
            return Ok(());
        }

        self.cur_pts = packet.pts.or(Some(packet.dts));

        let vtbl = unsafe { *(self.decoder_ptr as *const *const ISVCDecoderVtbl) };
        let mut dst_planes: [*mut u8; 3] = [ptr::null_mut(); 3];
        let mut buf_info = SBufferInfo::default();

        let ret = unsafe {
            ((*vtbl).decode_frame_no_delay)(
                self.decoder_ptr,
                data.as_ptr(),
                data.len() as c_int,
                dst_planes.as_mut_ptr(),
                &mut buf_info,
            )
        };

        if ret != 0 {
            return Err(HwError::DecodeError(format!(
                "OpenH264 DecodeFrameNoDelay error: {ret}"
            )));
        }

        // Check if a new decoded frame is available
        if buf_info.i_buffer_status == 1 {
            let width = buf_info.usr_data.i_width as u32;
            let height = buf_info.usr_data.i_height as u32;
            let stride_y = buf_info.usr_data.i_stride[0] as usize;
            let stride_uv = buf_info.usr_data.i_stride[1] as usize;

            if width > 0
                && height > 0
                && !dst_planes[0].is_null()
                && !dst_planes[1].is_null()
                && !dst_planes[2].is_null()
            {
                // Pack I420/YV12 planes into NV12 layout for zero-copy wgpu renderer upload
                let y_size = (width * height) as usize;
                let uv_size = ((width / 2) * (height / 2) * 2) as usize;
                let mut nv12_buf = vec![0u8; y_size + uv_size];

                unsafe {
                    // Copy Y plane row-by-row
                    let src_y = dst_planes[0];
                    for row in 0..height as usize {
                        let src_row = src_y.add(row * stride_y);
                        let dst_row =
                            &mut nv12_buf[row * width as usize..(row + 1) * width as usize];
                        std::ptr::copy_nonoverlapping(
                            src_row,
                            dst_row.as_mut_ptr(),
                            width as usize,
                        );
                    }

                    // Interleave U and V planes into NV12 UV plane
                    let uv_start = y_size;
                    let src_u = dst_planes[1];
                    let src_v = dst_planes[2];
                    let half_w = (width / 2) as usize;
                    let half_h = (height / 2) as usize;

                    for row in 0..half_h {
                        let u_row = src_u.add(row * stride_uv);
                        let v_row = src_v.add(row * stride_uv);
                        let dst_row_start = uv_start + row * (half_w * 2);

                        for col in 0..half_w {
                            let u_val = *u_row.add(col);
                            let v_val = *v_row.add(col);
                            nv12_buf[dst_row_start + col * 2] = u_val;
                            nv12_buf[dst_row_start + col * 2 + 1] = v_val;
                        }
                    }
                }

                self.output_queue.push_back(DecodedFrame {
                    pts: self.cur_pts,
                    width,
                    height,
                    pixel_format: PixelFormat::Nv12,
                    surface_id: 0,
                    data: Some(nv12_buf),
                    interlace_mode: InterlaceMode::Progressive,
                    dmabuf: None,
                    dxgi_handle: None,
                    cv_pixel_buffer: None,
                    is_concealed: false,
                });
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError> {
        Ok(self.output_queue.pop_front())
    }

    fn flush(&mut self) -> Result<(), HwError> {
        // Drain any delayed frames from OpenH264
        let vtbl = unsafe { *(self.decoder_ptr as *const *const ISVCDecoderVtbl) };
        let mut dst_planes: [*mut u8; 3] = [ptr::null_mut(); 3];
        let mut buf_info = SBufferInfo::default();

        let _ = unsafe {
            ((*vtbl).decode_frame_no_delay)(
                self.decoder_ptr,
                ptr::null(),
                0,
                dst_planes.as_mut_ptr(),
                &mut buf_info,
            )
        };

        Ok(())
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.output_queue.clear();
        self.flush()
    }
}

impl Drop for OpenH264Decoder {
    fn drop(&mut self) {
        if !self.decoder_ptr.is_null() {
            let vtbl = unsafe { *(self.decoder_ptr as *const *const ISVCDecoderVtbl) };
            unsafe {
                let _ = ((*vtbl).uninitialize)(self.decoder_ptr);
                (self.lib.destroy_decoder)(self.decoder_ptr);
            }
            self.decoder_ptr = ptr::null_mut();
        }
    }
}
