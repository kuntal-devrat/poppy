//! H.264 Slice Header parser.
//!
//! Implements ITU-T H.264 §7.3.3 to extract slice parameters required by
//! hardware decoders (such as VA-API's `VASliceParameterBufferH264`).

use crate::bitstream::{BitReader, BitstreamError, remove_emulation_prevention};
use crate::nal::NalUnitType;
use crate::pps::Pps;
use crate::sps::Sps;

/// H.264 Slice types (ITU-T Table 7-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceType {
    P = 0,
    B = 1,
    I = 2,
    Sp = 3,
    Si = 4,
}

impl SliceType {
    pub fn from_ue(val: u32) -> Self {
        match val % 5 {
            0 => Self::P,
            1 => Self::B,
            2 => Self::I,
            3 => Self::Sp,
            _ => Self::Si,
        }
    }

    pub fn is_intra(&self) -> bool {
        matches!(self, Self::I | Self::Si)
    }
}

/// A parsed slice header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceHeader {
    pub first_mb_in_slice: u32,
    pub slice_type: SliceType,
    pub raw_slice_type: u32,
    pub pic_parameter_set_id: u32,

    pub frame_num: u32,
    pub field_pic_flag: bool,
    pub bottom_field_flag: bool,
    pub idr_pic_id: Option<u32>,

    pub pic_order_cnt_lsb: Option<u32>,
    pub delta_pic_order_cnt_bottom: Option<i32>,
    pub delta_pic_order_cnt: [i32; 2],

    pub direct_spatial_mv_pred_flag: bool,
    pub num_ref_idx_l0_active_minus1: u32,
    pub num_ref_idx_l1_active_minus1: u32,
    pub cabac_init_idc: u32,
    pub slice_qp_delta: i32,

    pub disable_deblocking_filter_idc: u32,
    pub slice_alpha_c0_offset_div2: i32,
    pub slice_beta_offset_div2: i32,

    /// Bit position in the RBSP where macroblock / entropy data begins.
    pub slice_data_bit_offset: usize,
}

impl SliceHeader {
    /// Parse a slice header given its NAL unit type, nal_ref_idc, payload, active SPS, and PPS.
    pub fn parse(
        nal_unit_type: NalUnitType,
        nal_ref_idc: u8,
        nal_payload: &[u8],
        sps: &Sps,
        pps: &Pps,
    ) -> Result<Self, BitstreamError> {
        let rbsp = remove_emulation_prevention(nal_payload);
        let mut r = BitReader::new(&rbsp);

        let first_mb_in_slice = r.read_ue()?;
        let raw_slice_type = r.read_ue()?;
        let slice_type = SliceType::from_ue(raw_slice_type);
        let pic_parameter_set_id = r.read_ue()?;

        if sps.separate_colour_plane_flag {
            let _colour_plane_id = r.read_bits(2)?;
        }

        let frame_num_bits = (sps.log2_max_frame_num_minus4 + 4) as usize;
        let frame_num = r.read_bits(frame_num_bits)?;

        let mut field_pic_flag = false;
        let mut bottom_field_flag = false;
        if !sps.frame_mbs_only_flag {
            field_pic_flag = r.read_bit()?;
            if field_pic_flag {
                bottom_field_flag = r.read_bit()?;
            }
        }

        let mut idr_pic_id = None;
        if nal_unit_type == NalUnitType::SliceIdr {
            idr_pic_id = Some(r.read_ue()?);
        }

        let mut pic_order_cnt_lsb = None;
        let mut delta_pic_order_cnt_bottom = None;
        let mut delta_pic_order_cnt = [0i32; 2];

        if sps.pic_order_cnt_type == 0 {
            let poc_bits = (sps.log2_max_pic_order_cnt_lsb_minus4 + 4) as usize;
            pic_order_cnt_lsb = Some(r.read_bits(poc_bits)?);
            if pps.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
                delta_pic_order_cnt_bottom = Some(r.read_se()?);
            }
        } else if sps.pic_order_cnt_type == 1 && !sps.delta_pic_order_always_zero_flag {
            delta_pic_order_cnt[0] = r.read_se()?;
            if pps.bottom_field_pic_order_in_frame_present_flag && !field_pic_flag {
                delta_pic_order_cnt[1] = r.read_se()?;
            }
        }

        if pps.redundant_pic_cnt_present_flag {
            let _redundant_pic_cnt = r.read_ue()?;
        }

        let mut direct_spatial_mv_pred_flag = false;
        if slice_type == SliceType::B {
            direct_spatial_mv_pred_flag = r.read_bit()?;
        }

        let mut num_ref_idx_l0_active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
        let mut num_ref_idx_l1_active_minus1 = pps.num_ref_idx_l1_default_active_minus1;

        if matches!(slice_type, SliceType::P | SliceType::Sp | SliceType::B) {
            let num_ref_idx_active_override_flag = r.read_bit()?;
            if num_ref_idx_active_override_flag {
                num_ref_idx_l0_active_minus1 = r.read_ue()?;
                if slice_type == SliceType::B {
                    num_ref_idx_l1_active_minus1 = r.read_ue()?;
                }
            }
        }

        // ref_pic_list_modification()
        if slice_type != SliceType::I && slice_type != SliceType::Si {
            let ref_pic_list_modification_flag_l0 = r.read_bit()?;
            if ref_pic_list_modification_flag_l0 {
                loop {
                    let mod_idc = r.read_ue()?;
                    if mod_idc == 3 {
                        break;
                    }
                    let _abs_diff_pic_num_minus1 = r.read_ue()?;
                }
            }
        }
        if slice_type == SliceType::B {
            let ref_pic_list_modification_flag_l1 = r.read_bit()?;
            if ref_pic_list_modification_flag_l1 {
                loop {
                    let mod_idc = r.read_ue()?;
                    if mod_idc == 3 {
                        break;
                    }
                    let _abs_diff_pic_num_minus1 = r.read_ue()?;
                }
            }
        }

        // pred_weight_table()
        if (pps.weighted_pred_flag && (slice_type == SliceType::P || slice_type == SliceType::Sp))
            || (pps.weighted_bipred_idc == 1 && slice_type == SliceType::B)
        {
            let _luma_log2_weight_denom = r.read_ue()?;
            if sps.chroma_format_idc != 0 {
                let _chroma_log2_weight_denom = r.read_ue()?;
            }
            for _ in 0..=num_ref_idx_l0_active_minus1 {
                let luma_weight_flag = r.read_bit()?;
                if luma_weight_flag {
                    let _weight = r.read_se()?;
                    let _offset = r.read_se()?;
                }
                if sps.chroma_format_idc != 0 {
                    let chroma_weight_flag = r.read_bit()?;
                    if chroma_weight_flag {
                        let _w0 = r.read_se()?;
                        let _o0 = r.read_se()?;
                        let _w1 = r.read_se()?;
                        let _o1 = r.read_se()?;
                    }
                }
            }
            if slice_type == SliceType::B {
                for _ in 0..=num_ref_idx_l1_active_minus1 {
                    let luma_weight_flag = r.read_bit()?;
                    if luma_weight_flag {
                        let _weight = r.read_se()?;
                        let _offset = r.read_se()?;
                    }
                    if sps.chroma_format_idc != 0 {
                        let chroma_weight_flag = r.read_bit()?;
                        if chroma_weight_flag {
                            let _w0 = r.read_se()?;
                            let _o0 = r.read_se()?;
                            let _w1 = r.read_se()?;
                            let _o1 = r.read_se()?;
                        }
                    }
                }
            }
        }

        // dec_ref_pic_marking() (ITU-T H.264 §7.3.3: only present when nal_ref_idc != 0)
        if nal_ref_idc != 0 {
            if nal_unit_type == NalUnitType::SliceIdr {
                let _no_output_of_prior_pics_flag = r.read_bit()?;
                let _long_term_reference_flag = r.read_bit()?;
            } else {
                let adaptive_ref_pic_marking_mode_flag = r.read_bit()?;
                if adaptive_ref_pic_marking_mode_flag {
                    loop {
                        let mmco = r.read_ue()?;
                        if mmco == 0 {
                            break;
                        }
                        match mmco {
                            1 | 3 => {
                                let _diff = r.read_ue()?;
                                if mmco == 3 {
                                    let _long_term_frame_idx = r.read_ue()?;
                                }
                            }
                            2 => {
                                let _long_term_pic_num = r.read_ue()?;
                            }
                            4 => {
                                let _max_long_term = r.read_ue()?;
                            }
                            5 => {}
                            6 => {
                                let _long_term_frame_idx = r.read_ue()?;
                            }
                            _ => break,
                        }
                    }
                }
            }
        }

        let mut cabac_init_idc = 0;
        if pps.entropy_coding_mode_flag && slice_type != SliceType::I && slice_type != SliceType::Si
        {
            cabac_init_idc = r.read_ue()?;
        }

        let slice_qp_delta = r.read_se()?;

        let mut disable_deblocking_filter_idc = 0;
        let mut slice_alpha_c0_offset_div2 = 0;
        let mut slice_beta_offset_div2 = 0;

        if pps.deblocking_filter_control_present_flag {
            disable_deblocking_filter_idc = r.read_ue()?;
            if disable_deblocking_filter_idc != 1 {
                slice_alpha_c0_offset_div2 = r.read_se()?;
                slice_beta_offset_div2 = r.read_se()?;
            }
        }

        // Current bit offset is where slice data starts
        let slice_data_bit_offset = r.bits_read();

        Ok(Self {
            first_mb_in_slice,
            slice_type,
            raw_slice_type,
            pic_parameter_set_id,
            frame_num,
            field_pic_flag,
            bottom_field_flag,
            idr_pic_id,
            pic_order_cnt_lsb,
            delta_pic_order_cnt_bottom,
            delta_pic_order_cnt,
            direct_spatial_mv_pred_flag,
            num_ref_idx_l0_active_minus1,
            num_ref_idx_l1_active_minus1,
            cabac_init_idc,
            slice_qp_delta,
            disable_deblocking_filter_idc,
            slice_alpha_c0_offset_div2,
            slice_beta_offset_div2,
            slice_data_bit_offset,
        })
    }
}
