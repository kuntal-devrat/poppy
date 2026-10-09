//! macOS VideoToolbox, CoreMedia, CoreVideo, and IOSurface FFI declarations.

use std::ffi::c_void;

pub type OSStatus = i32;
pub const NO_ERR: OSStatus = 0;
pub const K_VT_VIDEO_DECODER_NOT_AVAILABLE_NOW_ERR: OSStatus = -12906;
pub const K_VT_VIDEO_DECODER_MALFUNCTION_ERR: OSStatus = -12911;

// CoreVideo Bi-Planar Pixel Formats
pub const K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR8_BI_PLANAR_VIDEO_RANGE: u32 = 0x34323076; // '420v'
pub const K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE: u32 = 0x78343230; // 'x420'

pub type CFTypeRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFAllocatorRef = *const c_void;

pub type VTDecompressionSessionRef = *mut c_void;
pub type CMFormatDescriptionRef = *mut c_void;
pub type CMSampleBufferRef = *mut c_void;
pub type CMBlockBufferRef = *mut c_void;
pub type CVPixelBufferRef = *mut c_void;
pub type CVImageBufferRef = *mut c_void;
pub type IOSurfaceRef = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

#[repr(C)]
pub struct VTDecompressionOutputCallbackRecord {
    pub decompression_output_callback: Option<
        unsafe extern "C" fn(
            decompression_output_ref_con: *mut c_void,
            source_frame_ref_con: *mut c_void,
            status: OSStatus,
            info_flags: u32,
            image_buffer: CVImageBufferRef,
            presentation_time_stamp: CMTime,
            presentation_duration: CMTime,
        ),
    >,
    pub decompression_output_ref_con: *mut c_void,
}

// VideoToolbox decoding flags
pub const K_VT_DECODE_FRAME_ENABLE_ASYNCHRONOUS_DECOMPRESSION: u32 = 1 << 0;
pub const K_VT_DECODE_FRAME_DO_NOT_OUTPUT_FRAME: u32 = 1 << 1;
pub const K_VT_DECODE_FRAME_1X_REAL_TIME_PLAYBACK: u32 = 1 << 2;
pub const K_VT_DECODE_FRAME_ENABLE_TEMPORAL_PROCESSING: u32 = 1 << 3;
