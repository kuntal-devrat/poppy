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

/// Modification command for reference picture lists (ITU-T H.264 §7.3.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefPicListModification {
    pub modification_of_pic_nums_idc: u32,
    pub value: u32,
}

/// Prediction weight table for weighted prediction (ITU-T H.264 §7.3.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredWeightTable {
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

impl Default for PredWeightTable {
    fn default() -> Self {
        Self {
            luma_log2_weight_denom: 0,
            chroma_log2_weight_denom: 0,
            luma_weight_l0_flag: 0,
            luma_weight_l0: [1; 32],
            luma_offset_l0: [0; 32],
            chroma_weight_l0_flag: 0,
            chroma_weight_l0: [[1, 1]; 32],
            chroma_offset_l0: [[0, 0]; 32],
            luma_weight_l1_flag: 0,
            luma_weight_l1: [1; 32],
            luma_offset_l1: [0; 32],
            chroma_weight_l1_flag: 0,
            chroma_weight_l1: [[1, 1]; 32],
            chroma_offset_l1: [[0, 0]; 32],
        }
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
    pub ref_pic_list_modification_l0: Vec<RefPicListModification>,
    pub ref_pic_list_modification_l1: Vec<RefPicListModification>,
    pub pred_weight_table: Option<PredWeightTable>,
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
        let mut ref_pic_list_modification_l0 = Vec::new();
        if slice_type != SliceType::I && slice_type != SliceType::Si {
            let ref_pic_list_modification_flag_l0 = r.read_bit()?;
            if ref_pic_list_modification_flag_l0 {
                loop {
                    let mod_idc = r.read_ue()?;
                    if mod_idc == 3 {
                        break;
                    }
                    let val = r.read_ue()?;
                    ref_pic_list_modification_l0.push(RefPicListModification {
                        modification_of_pic_nums_idc: mod_idc,
                        value: val,
                    });
                }
            }
        }
        let mut ref_pic_list_modification_l1 = Vec::new();
        if slice_type == SliceType::B {
            let ref_pic_list_modification_flag_l1 = r.read_bit()?;
            if ref_pic_list_modification_flag_l1 {
                loop {
                    let mod_idc = r.read_ue()?;
                    if mod_idc == 3 {
                        break;
                    }
                    let val = r.read_ue()?;
                    ref_pic_list_modification_l1.push(RefPicListModification {
                        modification_of_pic_nums_idc: mod_idc,
                        value: val,
                    });
                }
            }
        }

        // pred_weight_table()
        let mut pred_weight_table = None;
        if (pps.weighted_pred_flag && (slice_type == SliceType::P || slice_type == SliceType::Sp))
            || (pps.weighted_bipred_idc == 1 && slice_type == SliceType::B)
        {
            let mut pwt = PredWeightTable::default();
            let luma_denom = r.read_ue()? as u8;
            pwt.luma_log2_weight_denom = luma_denom;
            let default_luma_weight = 1i16 << luma_denom.min(14);
            pwt.luma_weight_l0.fill(default_luma_weight);
            pwt.luma_weight_l1.fill(default_luma_weight);

            if sps.chroma_format_idc != 0 {
                let chroma_denom = r.read_ue()? as u8;
                pwt.chroma_log2_weight_denom = chroma_denom;
                let default_chroma_weight = 1i16 << chroma_denom.min(14);
                pwt.chroma_weight_l0
                    .fill([default_chroma_weight, default_chroma_weight]);
                pwt.chroma_weight_l1
                    .fill([default_chroma_weight, default_chroma_weight]);
            }

            let num_l0 = (num_ref_idx_l0_active_minus1 as usize).min(31);
            for i in 0..=num_l0 {
                let luma_weight_flag = r.read_bit()?;
                if luma_weight_flag {
                    pwt.luma_weight_l0_flag = 1;
                    pwt.luma_weight_l0[i] = r.read_se()? as i16;
                    pwt.luma_offset_l0[i] = r.read_se()? as i16;
                }
                if sps.chroma_format_idc != 0 {
                    let chroma_weight_flag = r.read_bit()?;
                    if chroma_weight_flag {
                        pwt.chroma_weight_l0_flag = 1;
                        let w0 = r.read_se()? as i16;
                        let o0 = r.read_se()? as i16;
                        let w1 = r.read_se()? as i16;
                        let o1 = r.read_se()? as i16;
                        pwt.chroma_weight_l0[i] = [w0, w1];
                        pwt.chroma_offset_l0[i] = [o0, o1];
                    }
                }
            }
            if slice_type == SliceType::B {
                let num_l1 = (num_ref_idx_l1_active_minus1 as usize).min(31);
                for i in 0..=num_l1 {
                    let luma_weight_flag = r.read_bit()?;
                    if luma_weight_flag {
                        pwt.luma_weight_l1_flag = 1;
                        pwt.luma_weight_l1[i] = r.read_se()? as i16;
                        pwt.luma_offset_l1[i] = r.read_se()? as i16;
                    }
                    if sps.chroma_format_idc != 0 {
                        let chroma_weight_flag = r.read_bit()?;
                        if chroma_weight_flag {
                            pwt.chroma_weight_l1_flag = 1;
                            let w0 = r.read_se()? as i16;
                            let o0 = r.read_se()? as i16;
                            let w1 = r.read_se()? as i16;
                            let o1 = r.read_se()? as i16;
                            pwt.chroma_weight_l1[i] = [w0, w1];
                            pwt.chroma_offset_l1[i] = [o0, o1];
                        }
                    }
                }
            }
            pred_weight_table = Some(pwt);
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
            ref_pic_list_modification_l0,
            ref_pic_list_modification_l1,
            pred_weight_table,
            cabac_init_idc,
            slice_qp_delta,
            disable_deblocking_filter_idc,
            slice_alpha_c0_offset_div2,
            slice_beta_offset_div2,
            slice_data_bit_offset,
        })
    }
}
