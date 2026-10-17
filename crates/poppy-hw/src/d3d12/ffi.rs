//! Direct3D 12 Video Decode (`D3D12_VIDEO_DECODE`) FFI definitions.
//!
//! Implements Microsoft Direct3D 12 Video API types, COM interfaces, and DXVA
//! decoding structures according to `<d3d12video.h>` and `<dxva.h>`.
//!
//! All functions are loaded at runtime via `LoadLibraryW` / `GetProcAddress`
//! so the crate compiles on any platform.

use std::ffi::c_void;
use std::sync::OnceLock;

/// Universal Unique Identifier for COM interfaces.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub const fn from_values(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        Self {
            data1: d1,
            data2: d2,
            data3: d3,
            data4: d4,
        }
    }
}

// Standard Direct3D 12 Video Decode Profile GUIDs
pub const D3D12_VIDEO_DECODE_PROFILE_H264: GUID = GUID::from_values(
    0x1b81be68,
    0xa0c7,
    0x11d3,
    [0xb9, 0x84, 0x00, 0xc0, 0x4f, 0x2e, 0x73, 0xc5],
);

pub const D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN: GUID = GUID::from_values(
    0x5b11d51b,
    0xec34,
    0x472d,
    [0xba, 0xae, 0x30, 0x26, 0x13, 0xb5, 0x2d, 0x43],
);

pub const D3D12_VIDEO_DECODE_PROFILE_HEVC_MAIN10: GUID = GUID::from_values(
    0x1077f03c,
    0x0b11,
    0x4433,
    [0x86, 0x99, 0x96, 0x52, 0xd0, 0x2b, 0x13, 0x4e],
);

pub const D3D12_VIDEO_DECODE_PROFILE_AV1_PROFILE0: GUID = GUID::from_values(
    0xb8be4ccb,
    0xcf53,
    0x4652,
    [0x84, 0xe7, 0x08, 0x70, 0x96, 0x78, 0x04, 0x51],
);

pub const D3D12_VIDEO_DECODE_PROFILE_VP9_PROFILE0: GUID = GUID::from_values(
    0x463707f8,
    0xa1d0,
    0x4585,
    [0x87, 0x6d, 0x83, 0xaa, 0x6d, 0x60, 0xb8, 0x9e],
);

// Interface GUIDs
pub const IID_ID3D12VIDEO_DEVICE: GUID = GUID::from_values(
    0x1F05141B,
    0x2F6B,
    0x4841,
    [0xB8, 0x3F, 0x5B, 0x63, 0xD3, 0xDE, 0x65, 0xF6],
);

pub const IID_ID3D12VIDEO_DECODER: GUID = GUID::from_values(
    0xC59E6B72,
    0x3059,
    0x4861,
    [0xB9, 0x58, 0x52, 0x34, 0xC5, 0xC3, 0x63, 0xD4],
);

pub const IID_ID3D12VIDEO_DECODER_HEAP: GUID = GUID::from_values(
    0x0946B2F3,
    0xEAE9,
    0x491F,
    [0xBB, 0x86, 0xCE, 0x86, 0xF1, 0x24, 0x7F, 0x3D],
);

pub const IID_ID3D12VIDEO_DECODE_COMMAND_LIST: GUID = GUID::from_values(
    0x3B605372,
    0xBE3F,
    0x493F,
    [0xB8, 0x2E, 0xEE, 0x8A, 0x70, 0xF8, 0x95, 0xF3],
);

pub const IID_ID3D12DEVICE: GUID = GUID::from_values(
    0x189819F1,
    0x1DB6,
    0x4B57,
    [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7],
);

pub const IID_ID3D12RESOURCE: GUID = GUID::from_values(
    0x696442BE,
    0xA72E,
    0x4059,
    [0xBC, 0x79, 0x5B, 0x5C, 0x98, 0x04, 0x0F, 0xAD],
);

pub type HRESULT = i32;
pub const S_OK: HRESULT = 0;
pub const E_FAIL: HRESULT = -2147467259; // 0x80004005
pub const E_NOINTERFACE: HRESULT = -2147467262; // 0x80004002
pub type HANDLE = *mut c_void;
pub type HMODULE = *mut c_void;

// DXGI Formats
pub const DXGI_FORMAT_UNKNOWN: u32 = 0;
pub const DXGI_FORMAT_NV12: u32 = 103;
pub const DXGI_FORMAT_P010: u32 = 104;

// D3D12 Command List & Queue types
pub const D3D12_COMMAND_LIST_TYPE_DIRECT: u32 = 0;
pub const D3D12_COMMAND_LIST_TYPE_VIDEO_DECODE: u32 = 4;

// D3D12 Video Frame Coded Interlace Type
pub const D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_NONE: u32 = 0;
pub const D3D12_VIDEO_FRAME_CODED_INTERLACE_TYPE_FIELD_BASED: u32 = 1;

// D3D12 Video Decode Frame Argument Types
pub const D3D12_VIDEO_DECODE_ARGUMENT_TYPE_PICTURE_PARAMETERS: u32 = 0;
pub const D3D12_VIDEO_DECODE_ARGUMENT_TYPE_INVERSE_QUANTIZATION_MATRIX: u32 = 1;
pub const D3D12_VIDEO_DECODE_ARGUMENT_TYPE_SLICE_CONTROL: u32 = 2;

// D3D12 Resource States & Flags
pub const D3D12_RESOURCE_STATE_COMMON: u32 = 0;
pub const D3D12_RESOURCE_STATE_VIDEO_DECODE_READ: u32 = 0x10000;
pub const D3D12_RESOURCE_STATE_VIDEO_DECODE_WRITE: u32 = 0x20000;
pub const D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS: u32 = 0x4;

// D3D_FEATURE_LEVEL
pub const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
pub const D3D_FEATURE_LEVEL_12_0: u32 = 0xc000;

// D3D12_HEAP_TYPE
pub const D3D12_HEAP_TYPE_DEFAULT: u32 = 1;

// D3D12_RESOURCE_DIMENSION
pub const D3D12_RESOURCE_DIMENSION_TEXTURE2D: u32 = 3;

// D3D12 texture layout
pub const D3D12_TEXTURE_LAYOUT_UNKNOWN: u32 = 0;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DXGI_RATIONAL {
    pub numerator: u32,
    pub denominator: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_VIDEO_DECODE_CONFIGURATION {
    pub decode_profile: GUID,
    pub bitstream_encryption: GUID,
    pub interlace_type: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_VIDEO_DECODER_DESC {
    pub node_mask: u32,
    pub configuration: D3D12_VIDEO_DECODE_CONFIGURATION,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_VIDEO_DECODER_HEAP_DESC {
    pub node_mask: u32,
    pub configuration: D3D12_VIDEO_DECODE_CONFIGURATION,
    pub decode_width: u32,
    pub decode_height: u32,
    pub format: u32,
    pub frame_rate: DXGI_RATIONAL,
    pub bit_rate: u32,
    pub max_decode_picture_buffer_count: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct D3D12_VIDEO_DECODE_FRAME_ARGUMENT {
    pub type_: u32,
    pub size: u32,
    pub p_data: *const c_void,
}

impl Default for D3D12_VIDEO_DECODE_FRAME_ARGUMENT {
    fn default() -> Self {
        Self {
            type_: 0,
            size: 0,
            p_data: std::ptr::null(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct D3D12_VIDEO_DECODE_REFERENCE_FRAMES {
    pub num_texture2_ds: u32,
    pub pp_texture2_ds: *mut *mut c_void,
    pub p_subresources: *mut u32,
    pub pp_heap: *mut *mut c_void,
}

impl Default for D3D12_VIDEO_DECODE_REFERENCE_FRAMES {
    fn default() -> Self {
        Self {
            num_texture2_ds: 0,
            pp_texture2_ds: std::ptr::null_mut(),
            p_subresources: std::ptr::null_mut(),
            pp_heap: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct D3D12_VIDEO_DECODE_INPUT_STREAM_ARGUMENTS {
    pub num_frame_arguments: u32,
    pub frame_arguments: [D3D12_VIDEO_DECODE_FRAME_ARGUMENT; 10],
    pub reference_frames: D3D12_VIDEO_DECODE_REFERENCE_FRAMES,
    pub p_compressed_bitstream: *mut c_void,
    pub compressed_bitstream_size: u64,
    pub compressed_bitstream_offset: u64,
    pub p_heap: *mut c_void,
}

impl Default for D3D12_VIDEO_DECODE_INPUT_STREAM_ARGUMENTS {
    fn default() -> Self {
        Self {
            num_frame_arguments: 0,
            frame_arguments: [D3D12_VIDEO_DECODE_FRAME_ARGUMENT::default(); 10],
            reference_frames: D3D12_VIDEO_DECODE_REFERENCE_FRAMES::default(),
            p_compressed_bitstream: std::ptr::null_mut(),
            compressed_bitstream_size: 0,
            compressed_bitstream_offset: 0,
            p_heap: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct D3D12_VIDEO_DECODE_OUTPUT_STREAM_ARGUMENTS {
    pub p_output_texture2_d: *mut c_void,
    pub output_subresource: u32,
    pub conversion_arguments: [u8; 32],
}

impl Default for D3D12_VIDEO_DECODE_OUTPUT_STREAM_ARGUMENTS {
    fn default() -> Self {
        Self {
            p_output_texture2_d: std::ptr::null_mut(),
            output_subresource: 0,
            conversion_arguments: [0; 32],
        }
    }
}

// DXVA H.264 Picture Parameters
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DXVA_PicParams_H264 {
    pub w_frame_width_in_mbs_minus1: u16,
    pub w_frame_height_in_mbs_minus1: u16,
    pub curr_pic_index: u8,
    pub field_pic_flag: u8,
    pub mbaff_frame_flag: u8,
    pub residual_colour_transform_flag: u8,
    pub sp_for_switch_flag: u8,
    pub chroma_format_idc: u8,
    pub ref_pic_flag: u8,
    pub constrained_intra_pred_flag: u8,
    pub weighted_pred_flag: u8,
    pub weighted_bipred_idc: u8,
    pub mbf_frame_flag: u8,
    pub direct_8x8_inference_flag: u8,
    pub pic_order_cnt_type: u8,
    pub log2_max_frame_num_minus4: u8,
    pub log2_max_pic_order_cnt_lsb_minus4: u8,
    pub delta_pic_order_always_zero_flag: u8,
    pub num_ref_idx_l0_default_active_minus1: u8,
    pub num_ref_idx_l1_default_active_minus1: u8,
    pub num_ref_frames: u8,
    pub curr_field_order_cnt: [i32; 2],
    pub ref_frame_list: [u8; 16],
    pub field_order_cnt_list: [[i32; 2]; 16],
    pub frame_num_list: [u16; 16],
    pub slice_group_map_type: u8,
}

// DXVA H.264 Inverse Quantization Matrix
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DXVA_Qmatrix_H264 {
    pub b_scaling_lists4x4: [[u8; 16]; 6],
    pub b_scaling_lists8x8: [[u8; 64]; 2],
}

impl Default for DXVA_Qmatrix_H264 {
    fn default() -> Self {
        Self {
            b_scaling_lists4x4: [[16; 16]; 6],
            b_scaling_lists8x8: [[16; 64]; 2],
        }
    }
}

// DXVA HEVC Picture Parameters
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DXVA_PicParams_HEVC {
    pub pic_width_in_min_cbs_y: u16,
    pub pic_height_in_min_cbs_y: u16,
    pub w_format_and_sequence_info_flags: u16,
    pub sps_max_dec_pic_buffering_minus1: u8,
    pub log2_min_luma_coding_block_size_minus3: u8,
    pub log2_diff_max_min_luma_coding_block_size: u8,
    pub log2_min_transform_block_size_minus2: u8,
    pub log2_diff_max_min_transform_block_size: u8,
    pub max_transform_hierarchy_depth_inter: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub num_short_term_ref_pic_sets: u8,
    pub num_long_term_ref_pics_sps: u8,
    pub num_ref_idx_l0_default_active_minus1: u8,
    pub num_ref_idx_l1_default_active_minus1: u8,
    pub init_qp_minus26: i8,
    pub diff_cu_qp_delta_depth: u8,
    pub pps_cb_qp_offset: i8,
    pub pps_cr_qp_offset: i8,
    pub num_tile_columns_minus1: u8,
    pub num_tile_rows_minus1: u8,
    pub column_width_minus1: [u16; 19],
    pub row_height_minus1: [u16; 21],
    pub curr_pic_order_cnt_val: i32,
    pub ref_pic_set_st_curr_before: [u8; 8],
    pub ref_pic_set_st_curr_after: [u8; 8],
    pub ref_pic_set_lt_curr: [u8; 8],
    pub ref_pic_list: [u8; 15],
}

// DXVA AV1 Picture Parameters
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DXVA_PicParams_AV1 {
    pub width: u32,
    pub height: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub curr_pic_texture_index: u8,
    pub superres_denom: u8,
    pub bit_depth: u8,
    pub seq_profile: u8,
    pub frame_type: u8,
    pub show_frame: u8,
    pub showable_frame: u8,
    pub subsampling_x: u8,
    pub subsampling_y: u8,
    pub ref_frame_map_texture_index: [u8; 8],
    pub primary_ref_frame: u8,
    pub refresh_frame_flags: u8,
    pub order_hints: [u8; 8],
    pub tile_cols: u8,
    pub tile_rows: u8,
    pub width_in_sbs_minus1: [u16; 64],
    pub height_in_sbs_minus1: [u16; 64],
    pub context_update_tile_id: u16,
    pub base_q_idx: u8,
    pub y_dc_delta_q: i8,
    pub u_dc_delta_q: i8,
    pub v_dc_delta_q: i8,
    pub u_ac_delta_q: i8,
    pub v_ac_delta_q: i8,
}

impl Default for DXVA_PicParams_AV1 {
    fn default() -> Self {
        // SAFETY: All-zeroes is a valid representation for this POD type.
        unsafe { std::mem::zeroed() }
    }
}

/// D3D12 Resource Description (simplified for Texture2D).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_RESOURCE_DESC {
    pub dimension: u32,
    pub alignment: u64,
    pub width: u64,
    pub height: u32,
    pub depth_or_array_size: u16,
    pub mip_levels: u16,
    pub format: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub layout: u32,
    pub flags: u32,
}

/// D3D12 Heap Properties.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_HEAP_PROPERTIES {
    pub heap_type: u32,
    pub cpu_page_property: u32,
    pub memory_pool_preference: u32,
    pub creation_node_mask: u32,
    pub visible_node_mask: u32,
}

/// D3D12 Command Queue Description.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct D3D12_COMMAND_QUEUE_DESC {
    pub type_: u32,
    pub priority: i32,
    pub flags: u32,
    pub node_mask: u32,
}

// ----------- Runtime-loaded D3D12/DXGI functions -----------

/// Dynamically loaded D3D12 + DXGI function table.
///
/// On Windows we `LoadLibraryW("d3d12.dll")` / `LoadLibraryW("dxgi.dll")` and
/// resolve every symbol we need. On other platforms `load()` returns `None`.
pub struct D3d12Functions {
    _d3d12_handle: *mut c_void,
    _dxgi_handle: *mut c_void,

    pub d3d12_create_device: unsafe extern "system" fn(
        p_adapter: *mut c_void,
        minimum_feature_level: u32,
        riid: *const GUID,
        pp_device: *mut *mut c_void,
    ) -> HRESULT,

    pub create_dxgi_factory1:
        unsafe extern "system" fn(riid: *const GUID, pp_factory: *mut *mut c_void) -> HRESULT,
}

// SAFETY: The function pointers point to system DLL functions that are
// inherently thread-safe (D3D12 is thread-safe by design).
unsafe impl Send for D3d12Functions {}
unsafe impl Sync for D3d12Functions {}

static D3D12_FUNCTIONS: OnceLock<Option<D3d12Functions>> = OnceLock::new();

impl D3d12Functions {
    /// Load D3D12 and DXGI symbols. Returns `None` on non-Windows or if DLLs
    /// are missing (e.g., Windows Server Core without graphics).
    pub fn load() -> Option<&'static Self> {
        D3D12_FUNCTIONS
            .get_or_init(|| {
                #[cfg(target_os = "windows")]
                {
                    Self::load_windows()
                }
                #[cfg(not(target_os = "windows"))]
                {
                    None
                }
            })
            .as_ref()
    }

    #[cfg(target_os = "windows")]
    fn load_windows() -> Option<D3d12Functions> {
        // On Windows we use the winapi-style LoadLibraryW / GetProcAddress.
        // We declare them as extern "system" to call the real Win32 APIs.
        unsafe extern "system" {
            fn LoadLibraryW(lp_lib_file_name: *const u16) -> HMODULE;
            fn GetProcAddress(
                h_module: HMODULE,
                lp_proc_name: *const std::ffi::c_char,
            ) -> *mut c_void;
        }

        fn to_wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        unsafe fn sym(handle: HMODULE, name: &[u8]) -> *mut c_void {
            unsafe { GetProcAddress(handle, name.as_ptr() as *const std::ffi::c_char) }
        }

        let d3d12_dll = to_wide("d3d12.dll");
        let dxgi_dll = to_wide("dxgi.dll");

        let d3d12_handle = unsafe { LoadLibraryW(d3d12_dll.as_ptr()) };
        if d3d12_handle.is_null() {
            return None;
        }

        let dxgi_handle = unsafe { LoadLibraryW(dxgi_dll.as_ptr()) };
        if dxgi_handle.is_null() {
            return None;
        }

        let d3d12_create_device = unsafe { sym(d3d12_handle, b"D3D12CreateDevice\0") };
        if d3d12_create_device.is_null() {
            return None;
        }

        let create_dxgi_factory1 = unsafe { sym(dxgi_handle, b"CreateDXGIFactory1\0") };
        if create_dxgi_factory1.is_null() {
            return None;
        }

        Some(D3d12Functions {
            _d3d12_handle: d3d12_handle,
            _dxgi_handle: dxgi_handle,
            d3d12_create_device: unsafe { std::mem::transmute(d3d12_create_device) },
            create_dxgi_factory1: unsafe { std::mem::transmute(create_dxgi_factory1) },
        })
    }
}
