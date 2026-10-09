#![allow(clippy::missing_transmute_annotations)]
//! Safe Rust FFI definitions and dynamic loader for VA-API (`libva.so.2` & `libva-drm.so.2`).
//!
//! Direct x86_64/Linux C ABI matching `<va/va.h>`, `<va/va_drm.h>`, and `<va/va_dec_h264.h>`.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr};
use std::sync::Arc;

pub type VADisplay = *mut c_void;
pub type VAConfigID = c_uint;
pub type VAContextID = c_uint;
pub type VASurfaceID = c_uint;
pub type VABufferID = c_uint;
pub type VAStatus = c_int;
pub type VAProfile = c_int;
pub type VAEntrypoint = c_int;
pub type VABufferType = c_int;

pub const VA_STATUS_SUCCESS: VAStatus = 0;
pub const VA_RT_FORMAT_YUV420: c_uint = 0x00000001;
pub const VA_RT_FORMAT_YUV420_10: c_uint = 0x00000100;

pub const VA_PROFILE_H264_BASELINE: VAProfile = 5;
pub const VA_PROFILE_H264_MAIN: VAProfile = 6;
pub const VA_PROFILE_H264_HIGH: VAProfile = 7;
pub const VA_PROFILE_H264_CONSTRAINED_BASELINE: VAProfile = 21;
pub const VA_PROFILE_HEVC_MAIN: VAProfile = 16;
pub const VA_PROFILE_HEVC_MAIN10: VAProfile = 17;

pub const VA_ENTRYPOINT_VLD: VAEntrypoint = 1;

pub const VA_PICTURE_PARAMETER_BUFFER_TYPE: VABufferType = 0;
pub const VA_IQMATRIX_BUFFER_TYPE: VABufferType = 1;
pub const VA_SLICE_PARAMETER_BUFFER_TYPE: VABufferType = 4;
pub const VA_SLICE_DATA_BUFFER_TYPE: VABufferType = 5;

pub const VA_PROGRESSIVE: c_uint = 0x00000001;
pub const VA_INVALID_ID: c_uint = 0xffffffff;
pub const VA_INVALID_SURFACE: VASurfaceID = 0xffffffff;

pub const VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME: c_uint = 0x00000020;
pub const VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2: c_uint = 0x00000040;
pub const VA_EXPORT_SURFACE_READ_ONLY: c_uint = 0x00000001;
pub const VA_EXPORT_SURFACE_SEPARATE_LAYERS: c_uint = 0x00000004;
pub const VA_EXPORT_SURFACE_COMPOSED_LAYERS: c_uint = 0x00000008;

pub const VA_PICTURE_H264_INVALID: c_uint = 0x00000001;
pub const VA_PICTURE_H264_TOP_FIELD: c_uint = 0x00000002;
pub const VA_PICTURE_H264_BOTTOM_FIELD: c_uint = 0x00000004;
pub const VA_PICTURE_H264_SHORT_TERM_REFERENCE: c_uint = 0x00000008;
pub const VA_PICTURE_H264_LONG_TERM_REFERENCE: c_uint = 0x00000010;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VAPictureH264 {
    pub picture_id: VASurfaceID,
    pub frame_idx: c_uint,
    pub flags: c_uint,
    pub top_field_order_cnt: c_int,
    pub bottom_field_order_cnt: c_int,
}

impl Default for VAPictureH264 {
    fn default() -> Self {
        Self {
            picture_id: VA_INVALID_SURFACE,
            frame_idx: 0,
            flags: VA_PICTURE_H264_INVALID,
            top_field_order_cnt: 0,
            bottom_field_order_cnt: 0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAPictureParameterBufferH264 {
    pub curr_pic: VAPictureH264,
    pub reference_frames: [VAPictureH264; 16],
    pub picture_width_in_mbs_minus1: u16,
    pub picture_height_in_mbs_minus1: u16,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_ref_frames: u8,
    pub seq_fields: u32,
    pub num_slice_groups_minus1: u8,
    pub slice_group_map_type: u8,
    pub slice_group_change_rate_minus1: u16,
    pub pic_init_qp_minus26: i8,
    pub pic_init_qs_minus26: i8,
    pub chroma_qp_index_offset: i8,
    pub second_chroma_qp_index_offset: i8,
    pub pic_fields: u32,
    pub frame_num: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAIQMatrixBufferH264 {
    pub scaling_list4x4: [[u8; 16]; 6],
    pub scaling_list8x8: [[u8; 64]; 2],
}

impl Default for VAIQMatrixBufferH264 {
    fn default() -> Self {
        // Flat 16 scaling list default
        Self {
            scaling_list4x4: [[16; 16]; 6],
            scaling_list8x8: [[16; 64]; 2],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VASliceParameterBufferH264 {
    pub slice_data_size: u32,
    pub slice_data_offset: u32,
    pub slice_data_flag: u32,
    pub slice_data_bit_offset: u16,
    pub first_mb_in_slice: u16,
    pub slice_type: u8,
    pub direct_spatial_mv_pred_flag: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub cabac_init_idc: u8,
    pub slice_qp_delta: i8,
    pub disable_deblocking_filter_idc: u8,
    pub slice_alpha_c0_offset_div2: i8,
    pub slice_beta_offset_div2: i8,
    pub ref_pic_list0: [VAPictureH264; 32],
    pub ref_pic_list1: [VAPictureH264; 32],
    pub luma_log2_weight_denom: u8,
    pub chroma_log2_weight_denom: u8,
    pub luma_weight_l0_flag: u8,
    pub luma_weight_l0: [i16; 32],
    pub luma_offset_l0: [i16; 32],
    pub chroma_weight_l0_flag: u8,
    pub chroma_weight_l0: [[i16; 2]; 32],
    pub chroma_offset_l0: [[i16; 2]; 32],
    pub luma_weight_l1_flag: u8,
    pub luma_weight_l1: [i16; 32],
    pub luma_offset_l1: [i16; 32],
    pub chroma_weight_l1_flag: u8,
    pub chroma_weight_l1: [[i16; 2]; 32],
    pub chroma_offset_l1: [[i16; 2]; 32],
}

pub const VA_PICTURE_HEVC_INVALID: c_uint = 0x00000001;
pub const VA_PICTURE_HEVC_FIELD_PIC: c_uint = 0x00000002;
pub const VA_PICTURE_HEVC_BOTTOM_FIELD: c_uint = 0x00000004;
pub const VA_PICTURE_HEVC_SHORT_TERM_REFERENCE: c_uint = 0x00000008;
pub const VA_PICTURE_HEVC_LONG_TERM_REFERENCE: c_uint = 0x00000010;
pub const VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE: c_uint = 0x00000020;
pub const VA_PICTURE_HEVC_RPS_ST_CURR_AFTER: c_uint = 0x00000040;
pub const VA_PICTURE_HEVC_RPS_LT_CURR: c_uint = 0x00000080;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VAPictureHEVC {
    pub picture_id: VASurfaceID,
    pub pic_order_cnt: c_int,
    pub flags: c_uint,
    pub va_reserved: [c_uint; 4],
}

impl Default for VAPictureHEVC {
    fn default() -> Self {
        Self {
            picture_id: VA_INVALID_SURFACE,
            pic_order_cnt: 0,
            flags: VA_PICTURE_HEVC_INVALID,
            va_reserved: [0; 4],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAPictureParameterBufferHEVC {
    pub curr_pic: VAPictureHEVC,
    pub reference_frames: [VAPictureHEVC; 15],
    pub pic_width_in_luma_samples: u16,
    pub pic_height_in_luma_samples: u16,
    pub pic_fields: u32,
    pub sps_max_dec_pic_buffering_minus1: u8,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub pcm_sample_bit_depth_luma_minus1: u8,
    pub pcm_sample_bit_depth_chroma_minus1: u8,
    pub log2_min_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_luma_coding_block_size: u8,
    pub log2_min_transform_block_size_minus2: u8,
    pub log2_diff_max_min_transform_block_size: u8,
    pub log2_min_pcm_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_pcm_luma_coding_block_size: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub max_transform_hierarchy_depth_inter: u8,
    pub init_qp_minus26: i8,
    pub diff_cu_qp_delta_depth: u8,
    pub pps_cb_qp_offset: i8,
    pub pps_cr_qp_offset: i8,
    pub log2_parallel_merge_level_minus2: u8,
    pub num_tile_columns_minus1: u8,
    pub num_tile_rows_minus1: u8,
    pub column_width_minus1: [u16; 19],
    pub row_height_minus1: [u16; 21],
    pub slice_parsing_fields: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u8,
    pub num_short_term_ref_pic_sets: u8,
    pub num_long_term_ref_pic_sps: u8,
    pub num_ref_idx_l0_default_active_minus1: u8,
    pub num_ref_idx_l1_default_active_minus1: u8,
    pub pps_beta_offset_div2: i8,
    pub pps_tc_offset_div2: i8,
    pub num_extra_slice_header_bits: u8,
    pub st_rps_bits: u32,
    pub va_reserved: [u32; 8],
}

impl Default for VAPictureParameterBufferHEVC {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VASliceParameterBufferHEVC {
    pub slice_data_size: u32,
    pub slice_data_offset: u32,
    pub slice_data_flag: u32,
    pub slice_data_byte_offset: u32,
    pub slice_segment_address: u32,
    pub ref_pic_list: [[u8; 15]; 2],
    pub long_slice_flags: u32,
    pub collocated_ref_idx: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub slice_qp_delta: i8,
    pub slice_cb_qp_offset: i8,
    pub slice_cr_qp_offset: i8,
    pub slice_beta_offset_div2: i8,
    pub slice_tc_offset_div2: i8,
    pub luma_log2_weight_denom: u8,
    pub delta_chroma_log2_weight_denom: i8,
    pub delta_luma_weight_l0: [i8; 15],
    pub luma_offset_l0: [i8; 15],
    pub delta_chroma_weight_l0: [[i8; 2]; 15],
    pub chroma_offset_l0: [[i8; 2]; 15],
    pub delta_luma_weight_l1: [i8; 15],
    pub luma_offset_l1: [i8; 15],
    pub delta_chroma_weight_l1: [[i8; 2]; 15],
    pub chroma_offset_l1: [[i8; 2]; 15],
    pub five_minus_max_num_merge_cand: u8,
    pub num_entry_point_offsets: u16,
    pub entry_offset_to_subset_array: u16,
    pub slice_data_num_emu_prevn_bytes: u16,
    pub va_reserved: [u32; 2],
}

impl Default for VASliceParameterBufferHEVC {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hevc_struct_sizes() {
        println!("size_of VAPictureParameterBufferHEVC = {}", std::mem::size_of::<VAPictureParameterBufferHEVC>());
        println!("size_of VASliceParameterBufferHEVC = {}", std::mem::size_of::<VASliceParameterBufferHEVC>());
        assert_eq!(std::mem::size_of::<VAPictureParameterBufferHEVC>(), 604);
        assert_eq!(std::mem::size_of::<VASliceParameterBufferHEVC>(), 264);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAIQMatrixBufferHEVC {
    pub scaling_list4x4: [[u8; 16]; 6],
    pub scaling_list8x8: [[u8; 64]; 6],
    pub scaling_list16x16: [[u8; 64]; 6],
    pub scaling_list32x32: [[u8; 64]; 2],
    pub scaling_list_dc16x16: [u8; 6],
    pub scaling_list_dc32x32: [u8; 2],
    pub va_reserved: [u32; 4],
}

impl Default for VAIQMatrixBufferHEVC {
    fn default() -> Self {
        Self {
            scaling_list4x4: [[16; 16]; 6],
            scaling_list8x8: [[16; 64]; 6],
            scaling_list16x16: [[16; 64]; 6],
            scaling_list32x32: [[16; 64]; 2],
            scaling_list_dc16x16: [16; 6],
            scaling_list_dc32x32: [16; 2],
            va_reserved: [0; 4],
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VADRMPRIMESurfaceDescriptor {
    pub fourcc: u32,
    pub width: u32,
    pub height: u32,
    pub num_objects: u32,
    pub objects: [VADRMPRIMEObject; 4],
    pub num_layers: u32,
    pub layers: [VADRMPRIMELayer; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VADRMPRIMEObject {
    pub fd: c_int,
    pub size: u32,
    pub drm_format_modifier: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VADRMPRIMELayer {
    pub drm_format: u32,
    pub num_planes: u32,
    pub object_index: [u32; 4],
    pub offset: [u32; 4],
    pub pitch: [u32; 4],
}

pub type VAImageID = c_uint;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAImageFormat {
    pub fourcc: u32,
    pub byte_order: u32,
    pub bits_per_pixel: u32,
    pub depth: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub alpha_mask: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VAImage {
    pub image_id: VAImageID,
    pub format: VAImageFormat,
    pub buf: VABufferID,
    pub _va_reserved: [usize; 2],
    pub width: u16,
    pub height: u16,
    pub data_size: u32,
    pub num_planes: u32,
    pub pitches: [u32; 3],
    pub offsets: [u32; 3],
    pub entrypoint: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VABufferInfo {
    pub mem_type: c_uint,
    pub handle: usize,
    pub type_: c_uint,
    pub mem_size: usize,
}

/// Dynamically loaded VA-API symbols table.
#[allow(non_snake_case)]
pub struct VaApi {
    _lib_va: *mut c_void,
    _lib_va_drm: *mut c_void,

    pub vaGetDisplayDRM: unsafe extern "C" fn(c_int) -> VADisplay,
    pub vaInitialize: unsafe extern "C" fn(VADisplay, *mut c_int, *mut c_int) -> VAStatus,
    pub vaTerminate: unsafe extern "C" fn(VADisplay) -> VAStatus,
    pub vaErrorStr: unsafe extern "C" fn(VAStatus) -> *const c_char,
    pub vaQueryConfigProfiles: unsafe extern "C" fn(VADisplay, *mut VAProfile, *mut c_int) -> VAStatus,
    pub vaMaxNumProfiles: unsafe extern "C" fn(VADisplay) -> c_int,
    pub vaCreateConfig: unsafe extern "C" fn(VADisplay, VAProfile, VAEntrypoint, *mut c_void, c_uint, *mut VAConfigID) -> VAStatus,
    pub vaDestroyConfig: unsafe extern "C" fn(VADisplay, VAConfigID) -> VAStatus,
    pub vaCreateSurfaces: unsafe extern "C" fn(VADisplay, c_uint, c_uint, c_uint, *mut VASurfaceID, c_uint, *mut c_void, c_uint) -> VAStatus,
    pub vaDestroySurfaces: unsafe extern "C" fn(VADisplay, *mut VASurfaceID, c_uint) -> VAStatus,
    pub vaCreateContext: unsafe extern "C" fn(VADisplay, VAConfigID, c_int, c_int, c_int, *mut VASurfaceID, c_int, *mut VAContextID) -> VAStatus,
    pub vaDestroyContext: unsafe extern "C" fn(VADisplay, VAContextID) -> VAStatus,
    pub vaCreateBuffer: unsafe extern "C" fn(VADisplay, VAContextID, VABufferType, c_uint, c_uint, *mut c_void, *mut VABufferID) -> VAStatus,
    pub vaDestroyBuffer: unsafe extern "C" fn(VADisplay, VABufferID) -> VAStatus,
    pub vaBeginPicture: unsafe extern "C" fn(VADisplay, VAContextID, VASurfaceID) -> VAStatus,
    pub vaRenderPicture: unsafe extern "C" fn(VADisplay, VAContextID, *mut VABufferID, c_int) -> VAStatus,
    pub vaEndPicture: unsafe extern "C" fn(VADisplay, VAContextID) -> VAStatus,
    pub vaSyncSurface: unsafe extern "C" fn(VADisplay, VASurfaceID) -> VAStatus,
    pub vaExportSurfaceHandle: unsafe extern "C" fn(VADisplay, VASurfaceID, c_uint, c_uint, *mut c_void) -> VAStatus,
    pub vaDeriveImage: unsafe extern "C" fn(VADisplay, VASurfaceID, *mut VAImage) -> VAStatus,
    pub vaDestroyImage: unsafe extern "C" fn(VADisplay, VAImageID) -> VAStatus,
    pub vaMapBuffer: unsafe extern "C" fn(VADisplay, VABufferID, *mut *mut c_void) -> VAStatus,
    pub vaUnmapBuffer: unsafe extern "C" fn(VADisplay, VABufferID) -> VAStatus,
    pub vaAcquireBufferHandle: unsafe extern "C" fn(VADisplay, VABufferID, *mut VABufferInfo) -> VAStatus,
    pub vaReleaseBufferHandle: unsafe extern "C" fn(VADisplay, VABufferID) -> VAStatus,
}

unsafe impl Send for VaApi {}
unsafe impl Sync for VaApi {}

impl VaApi {
    pub fn load() -> Result<Arc<Self>, String> {
        unsafe {
            let lib_va_name = b"libva.so.2\0";
            let lib_va = libc::dlopen(lib_va_name.as_ptr() as *const c_char, libc::RTLD_NOW);
            if lib_va.is_null() {
                return Err("Failed to load libva.so.2 via dlopen".into());
            }

            let lib_va_drm_name = b"libva-drm.so.2\0";
            let lib_va_drm = libc::dlopen(lib_va_drm_name.as_ptr() as *const c_char, libc::RTLD_NOW);
            if lib_va_drm.is_null() {
                libc::dlclose(lib_va);
                return Err("Failed to load libva-drm.so.2 via dlopen".into());
            }

            macro_rules! sym {
                ($handle:expr, $name:expr) => {{
                    let s = libc::dlsym($handle, $name.as_ptr() as *const c_char);
                    if s.is_null() {
                        libc::dlclose(lib_va);
                        libc::dlclose(lib_va_drm);
                        return Err(format!("Symbol not found: {}", CStr::from_ptr($name.as_ptr() as *const c_char).to_string_lossy()));
                    }
                    std::mem::transmute(s)
                }};
            }

            let va = Self {
                _lib_va: lib_va,
                _lib_va_drm: lib_va_drm,

                vaGetDisplayDRM: sym!(lib_va_drm, b"vaGetDisplayDRM\0"),
                vaInitialize: sym!(lib_va, b"vaInitialize\0"),
                vaTerminate: sym!(lib_va, b"vaTerminate\0"),
                vaErrorStr: sym!(lib_va, b"vaErrorStr\0"),
                vaQueryConfigProfiles: sym!(lib_va, b"vaQueryConfigProfiles\0"),
                vaMaxNumProfiles: sym!(lib_va, b"vaMaxNumProfiles\0"),
                vaCreateConfig: sym!(lib_va, b"vaCreateConfig\0"),
                vaDestroyConfig: sym!(lib_va, b"vaDestroyConfig\0"),
                vaCreateSurfaces: sym!(lib_va, b"vaCreateSurfaces\0"),
                vaDestroySurfaces: sym!(lib_va, b"vaDestroySurfaces\0"),
                vaCreateContext: sym!(lib_va, b"vaCreateContext\0"),
                vaDestroyContext: sym!(lib_va, b"vaDestroyContext\0"),
                vaCreateBuffer: sym!(lib_va, b"vaCreateBuffer\0"),
                vaDestroyBuffer: sym!(lib_va, b"vaDestroyBuffer\0"),
                vaBeginPicture: sym!(lib_va, b"vaBeginPicture\0"),
                vaRenderPicture: sym!(lib_va, b"vaRenderPicture\0"),
                vaEndPicture: sym!(lib_va, b"vaEndPicture\0"),
                vaSyncSurface: sym!(lib_va, b"vaSyncSurface\0"),
                vaExportSurfaceHandle: sym!(lib_va, b"vaExportSurfaceHandle\0"),
                vaDeriveImage: sym!(lib_va, b"vaDeriveImage\0"),
                vaDestroyImage: sym!(lib_va, b"vaDestroyImage\0"),
                vaMapBuffer: sym!(lib_va, b"vaMapBuffer\0"),
                vaUnmapBuffer: sym!(lib_va, b"vaUnmapBuffer\0"),
                vaAcquireBufferHandle: sym!(lib_va, b"vaAcquireBufferHandle\0"),
                vaReleaseBufferHandle: sym!(lib_va, b"vaReleaseBufferHandle\0"),
            };

            Ok(Arc::new(va))
        }
    }

    pub fn err_str(&self, status: VAStatus) -> String {
        unsafe {
            let ptr = (self.vaErrorStr)(status);
            if ptr.is_null() {
                format!("VAStatus({status})")
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        }
    }
}

impl Drop for VaApi {
    fn drop(&mut self) {
        unsafe {
            if !self._lib_va_drm.is_null() {
                libc::dlclose(self._lib_va_drm);
            }
            if !self._lib_va.is_null() {
                libc::dlclose(self._lib_va);
            }
        }
    }
}
