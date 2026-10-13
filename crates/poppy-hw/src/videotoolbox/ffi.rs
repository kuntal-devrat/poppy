//! macOS VideoToolbox, CoreMedia, CoreVideo, and IOSurface FFI declarations.
//!
//! All symbols are loaded dynamically at runtime via `dlopen` so that
//! `poppy-hw` compiles on any platform but only attempts real decode on macOS.

use std::ffi::c_void;
use std::sync::OnceLock;

pub type OSStatus = i32;
pub const NO_ERR: OSStatus = 0;
pub const K_VT_VIDEO_DECODER_NOT_AVAILABLE_NOW_ERR: OSStatus = -12906;
pub const K_VT_VIDEO_DECODER_MALFUNCTION_ERR: OSStatus = -12911;
pub const K_VT_DECODE_FRAME_ENABLE_ASYNCHRONOUS_DECOMPRESSION: u32 = 1 << 0;
pub const K_VT_DECODE_FRAME_DO_NOT_OUTPUT_FRAME: u32 = 1 << 1;
pub const K_VT_DECODE_FRAME_1X_REAL_TIME_PLAYBACK: u32 = 1 << 2;
pub const K_VT_DECODE_FRAME_ENABLE_TEMPORAL_PROCESSING: u32 = 1 << 3;

// CoreVideo Bi-Planar Pixel Formats
pub const K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR8_BI_PLANAR_VIDEO_RANGE: u32 = 0x34323076; // '420v'
pub const K_CV_PIXEL_FORMAT_TYPE_420_YP_CB_CR10_BI_PLANAR_VIDEO_RANGE: u32 = 0x78343230; // 'x420'

pub type CFTypeRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFMutableDictionaryRef = *mut c_void;
pub type CFArrayRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFNumberRef = *const c_void;
pub type CFBooleanRef = *const c_void;
pub type CFIndex = isize;
pub type CMItemCount = CFIndex;
pub type CFNumberType = CFIndex;

pub type VTDecompressionSessionRef = *mut c_void;
pub type CMFormatDescriptionRef = *mut c_void;
pub type CMVideoFormatDescriptionRef = *mut c_void;
pub type CMSampleBufferRef = *mut c_void;
pub type CMBlockBufferRef = *mut c_void;
pub type CVPixelBufferRef = *mut c_void;
pub type CVImageBufferRef = *mut c_void;
pub type IOSurfaceRef = *mut c_void;

pub type Boolean = u8;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CMTime {
    pub value: i64,
    pub timescale: i32,
    pub flags: u32,
    pub epoch: i64,
}

pub const K_CM_TIME_FLAGS_VALID: u32 = 1;

impl CMTime {
    pub fn make(value: i64, timescale: i32) -> Self {
        Self {
            value,
            timescale,
            flags: K_CM_TIME_FLAGS_VALID,
            epoch: 0,
        }
    }

    pub fn invalid() -> Self {
        Self::default()
    }
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

// CFNumber types
pub const K_CF_NUMBER_SI_NT32_TYPE: CFNumberType = 3;

/// Dynamically loaded VideoToolbox function table.
///
/// On macOS we `dlopen` the system framework dylibs and resolve every symbol
/// we need. On other platforms `load()` returns `None`.
pub struct VtFunctions {
    _vt_handle: *mut c_void,
    _cm_handle: *mut c_void,
    _cv_handle: *mut c_void,
    _cf_handle: *mut c_void,

    // CoreFoundation
    pub cf_dictionary_create_mutable: unsafe extern "C" fn(
        allocator: CFAllocatorRef,
        capacity: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFMutableDictionaryRef,
    pub cf_dictionary_set_value: unsafe extern "C" fn(
        dict: CFMutableDictionaryRef,
        key: *const c_void,
        value: *const c_void,
    ),
    pub cf_number_create: unsafe extern "C" fn(
        allocator: CFAllocatorRef,
        the_type: CFNumberType,
        value_ptr: *const c_void,
    ) -> CFNumberRef,
    pub cf_release: unsafe extern "C" fn(cf: CFTypeRef),

    // CoreMedia – format descriptions
    pub cm_video_format_description_create: unsafe extern "C" fn(
        allocator: CFAllocatorRef,
        codec_type: u32,
        width: i32,
        height: i32,
        extensions: CFDictionaryRef,
        format_description_out: *mut CMVideoFormatDescriptionRef,
    ) -> OSStatus,
    pub cm_video_format_description_create_from_h264_parameter_sets:
        unsafe extern "C" fn(
            allocator: CFAllocatorRef,
            parameter_set_count: usize,
            parameter_set_pointers: *const *const u8,
            parameter_set_sizes: *const usize,
            nal_unit_header_length: i32,
            format_description_out: *mut CMVideoFormatDescriptionRef,
        ) -> OSStatus,
    pub cm_video_format_description_create_from_hevc_parameter_sets:
        unsafe extern "C" fn(
            allocator: CFAllocatorRef,
            parameter_set_count: usize,
            parameter_set_pointers: *const *const u8,
            parameter_set_sizes: *const usize,
            nal_unit_header_length: i32,
            extensions: CFDictionaryRef,
            format_description_out: *mut CMVideoFormatDescriptionRef,
        ) -> OSStatus,

    // CoreMedia – block/sample buffers
    pub cm_block_buffer_create_with_memory_block: unsafe extern "C" fn(
        structure_allocator: CFAllocatorRef,
        memory_block: *const c_void,
        block_length: usize,
        block_allocator: CFAllocatorRef,
        custom_block_source: *const c_void,
        offset_to_data: usize,
        data_length: usize,
        flags: u32,
        block_buffer_out: *mut CMBlockBufferRef,
    ) -> OSStatus,
    pub cm_sample_buffer_create: unsafe extern "C" fn(
        allocator: CFAllocatorRef,
        data_buffer: CMBlockBufferRef,
        data_ready: Boolean,
        make_data_ready_callback: *const c_void,
        make_data_ready_refcon: *const c_void,
        format_description: CMVideoFormatDescriptionRef,
        num_samples: CMItemCount,
        num_sample_timing_entries: CMItemCount,
        sample_timing_array: *const CMSampleTimingInfo,
        num_sample_size_entries: CMItemCount,
        sample_size_array: *const usize,
        sample_buffer_out: *mut CMSampleBufferRef,
    ) -> OSStatus,

    // VideoToolbox
    pub vt_decompression_session_create: unsafe extern "C" fn(
        allocator: CFAllocatorRef,
        video_format_description: CMVideoFormatDescriptionRef,
        video_decoder_specification: CFDictionaryRef,
        destination_image_buffer_attributes: CFDictionaryRef,
        output_callback: *const VTDecompressionOutputCallbackRecord,
        decompression_session_out: *mut VTDecompressionSessionRef,
    ) -> OSStatus,
    pub vt_decompression_session_decode_frame: unsafe extern "C" fn(
        session: VTDecompressionSessionRef,
        sample_buffer: CMSampleBufferRef,
        decode_flags: u32,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus,
    pub vt_decompression_session_wait_for_asynchronous_frames:
        unsafe extern "C" fn(session: VTDecompressionSessionRef) -> OSStatus,
    pub vt_decompression_session_invalidate:
        unsafe extern "C" fn(session: VTDecompressionSessionRef),

    // CoreVideo – pixel buffer access
    pub cv_pixel_buffer_lock_base_address:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef, lock_flags: u64) -> i32,
    pub cv_pixel_buffer_unlock_base_address:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef, lock_flags: u64) -> i32,
    pub cv_pixel_buffer_get_width: unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef) -> usize,
    pub cv_pixel_buffer_get_height: unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef) -> usize,
    pub cv_pixel_buffer_get_base_address_of_plane:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef, plane_index: usize) -> *mut u8,
    pub cv_pixel_buffer_get_bytes_per_row_of_plane:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef, plane_index: usize) -> usize,
    pub cv_pixel_buffer_get_height_of_plane:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef, plane_index: usize) -> usize,
    pub cv_pixel_buffer_get_io_surface:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef) -> IOSurfaceRef,
    pub cv_pixel_buffer_get_pixel_format_type:
        unsafe extern "C" fn(pixel_buffer: CVPixelBufferRef) -> u32,

    pub k_cf_type_dictionary_key_callbacks: *const c_void,
    pub k_cf_type_dictionary_value_callbacks: *const c_void,
    pub k_cv_pixel_buffer_pixel_format_type_key: *const c_void,
    pub k_cv_pixel_buffer_io_surface_properties_key: *const c_void,
    pub k_cf_allocator_null: *const c_void,
}

// SAFETY: The function pointers point to system framework functions that are
// inherently thread-safe (VideoToolbox is documented as session-thread-safe).
unsafe impl Send for VtFunctions {}
unsafe impl Sync for VtFunctions {}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CMSampleTimingInfo {
    pub duration: CMTime,
    pub presentation_time_stamp: CMTime,
    pub decode_time_stamp: CMTime,
}

static VT_FUNCTIONS: OnceLock<Option<VtFunctions>> = OnceLock::new();

impl VtFunctions {
    /// Load all VideoToolbox/CoreMedia/CoreVideo/CoreFoundation symbols.
    ///
    /// Returns `None` if any required symbol or framework cannot be loaded
    /// (always `None` on non-macOS).
    pub fn load() -> Option<&'static Self> {
        VT_FUNCTIONS
            .get_or_init(|| {
                #[cfg(target_os = "macos")]
                {
                    Self::load_macos()
                }
                #[cfg(not(target_os = "macos"))]
                {
                    None
                }
            })
            .as_ref()
    }

    #[cfg(target_os = "macos")]
    fn load_macos() -> Option<VtFunctions> {
        use std::ffi::CStr;

        unsafe fn dlopen_framework(name: &CStr) -> *mut c_void {
            unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL) }
        }

        unsafe fn sym(handle: *mut c_void, name: &CStr) -> *mut c_void {
            unsafe { libc::dlsym(handle, name.as_ptr()) }
        }

        macro_rules! load_sym {
            ($handle:expr, $name:literal) => {{
                let s = unsafe {
                    sym(
                        $handle,
                        CStr::from_bytes_with_nul_unchecked(concat!($name, "\0").as_bytes()),
                    )
                };
                if s.is_null() {
                    return None;
                }
                unsafe { std::mem::transmute(s) }
            }};
        }

        let vt_handle = unsafe {
            dlopen_framework(CStr::from_bytes_with_nul_unchecked(
                b"/System/Library/Frameworks/VideoToolbox.framework/VideoToolbox\0",
            ))
        };
        if vt_handle.is_null() {
            return None;
        }

        let cm_handle = unsafe {
            dlopen_framework(CStr::from_bytes_with_nul_unchecked(
                b"/System/Library/Frameworks/CoreMedia.framework/CoreMedia\0",
            ))
        };
        if cm_handle.is_null() {
            return None;
        }

        let cv_handle = unsafe {
            dlopen_framework(CStr::from_bytes_with_nul_unchecked(
                b"/System/Library/Frameworks/CoreVideo.framework/CoreVideo\0",
            ))
        };
        if cv_handle.is_null() {
            return None;
        }

        let cf_handle = unsafe {
            dlopen_framework(CStr::from_bytes_with_nul_unchecked(
                b"/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation\0",
            ))
        };
        if cf_handle.is_null() {
            return None;
        }

        Some(VtFunctions {
            _vt_handle: vt_handle,
            _cm_handle: cm_handle,
            _cv_handle: cv_handle,
            _cf_handle: cf_handle,

            // CoreFoundation
            cf_dictionary_create_mutable: load_sym!(cf_handle, "CFDictionaryCreateMutable"),
            cf_dictionary_set_value: load_sym!(cf_handle, "CFDictionarySetValue"),
            cf_number_create: load_sym!(cf_handle, "CFNumberCreate"),
            cf_release: load_sym!(cf_handle, "CFRelease"),

            // CoreMedia – format descriptions
            cm_video_format_description_create: load_sym!(
                cm_handle,
                "CMVideoFormatDescriptionCreate"
            ),
            cm_video_format_description_create_from_h264_parameter_sets: load_sym!(
                cm_handle,
                "CMVideoFormatDescriptionCreateFromH264ParameterSets"
            ),
            cm_video_format_description_create_from_hevc_parameter_sets: load_sym!(
                cm_handle,
                "CMVideoFormatDescriptionCreateFromHEVCParameterSets"
            ),

            // CoreMedia – block/sample buffers
            cm_block_buffer_create_with_memory_block: load_sym!(
                cm_handle,
                "CMBlockBufferCreateWithMemoryBlock"
            ),
            cm_sample_buffer_create: load_sym!(cm_handle, "CMSampleBufferCreate"),

            // VideoToolbox
            vt_decompression_session_create: load_sym!(vt_handle, "VTDecompressionSessionCreate"),
            vt_decompression_session_decode_frame: load_sym!(
                vt_handle,
                "VTDecompressionSessionDecodeFrame"
            ),
            vt_decompression_session_wait_for_asynchronous_frames: load_sym!(
                vt_handle,
                "VTDecompressionSessionWaitForAsynchronousFrames"
            ),
            vt_decompression_session_invalidate: load_sym!(
                vt_handle,
                "VTDecompressionSessionInvalidate"
            ),

            // CoreVideo – pixel buffer access
            cv_pixel_buffer_lock_base_address: load_sym!(cv_handle, "CVPixelBufferLockBaseAddress"),
            cv_pixel_buffer_unlock_base_address: load_sym!(
                cv_handle,
                "CVPixelBufferUnlockBaseAddress"
            ),
            cv_pixel_buffer_get_width: load_sym!(cv_handle, "CVPixelBufferGetWidth"),
            cv_pixel_buffer_get_height: load_sym!(cv_handle, "CVPixelBufferGetHeight"),
            cv_pixel_buffer_get_base_address_of_plane: load_sym!(
                cv_handle,
                "CVPixelBufferGetBaseAddressOfPlane"
            ),
            cv_pixel_buffer_get_bytes_per_row_of_plane: load_sym!(
                cv_handle,
                "CVPixelBufferGetBytesPerRowOfPlane"
            ),
            cv_pixel_buffer_get_height_of_plane: load_sym!(
                cv_handle,
                "CVPixelBufferGetHeightOfPlane"
            ),
            cv_pixel_buffer_get_io_surface: load_sym!(cv_handle, "CVPixelBufferGetIOSurface"),
            cv_pixel_buffer_get_pixel_format_type: load_sym!(
                cv_handle,
                "CVPixelBufferGetPixelFormatType"
            ),

            k_cf_type_dictionary_key_callbacks: unsafe {
                libc::dlsym(cf_handle, c"kCFTypeDictionaryKeyCallBacks".as_ptr())
            },
            k_cf_type_dictionary_value_callbacks: unsafe {
                libc::dlsym(cf_handle, c"kCFTypeDictionaryValueCallBacks".as_ptr())
            },
            k_cv_pixel_buffer_pixel_format_type_key: {
                let p =
                    unsafe { libc::dlsym(cv_handle, c"kCVPixelBufferPixelFormatTypeKey".as_ptr()) };
                if !p.is_null() {
                    unsafe { *(p as *const *const c_void) }
                } else {
                    std::ptr::null()
                }
            },
            k_cv_pixel_buffer_io_surface_properties_key: {
                let p = unsafe {
                    libc::dlsym(cv_handle, c"kCVPixelBufferIOSurfacePropertiesKey".as_ptr())
                };
                if !p.is_null() {
                    unsafe { *(p as *const *const c_void) }
                } else {
                    std::ptr::null()
                }
            },
            k_cf_allocator_null: {
                let p = unsafe { libc::dlsym(cf_handle, c"kCFAllocatorNull".as_ptr()) };
                if !p.is_null() {
                    unsafe { *(p as *const *const c_void) }
                } else {
                    std::ptr::null()
                }
            },
        })
    }
}
