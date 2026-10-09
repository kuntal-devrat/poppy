//! Picture Parameter Set (PPS) parser.
//!
//! Implements ITU-T H.264 §7.3.2.2.

use crate::bitstream::{remove_emulation_prevention, BitReader, BitstreamError};

/// A parsed H.264 Picture Parameter Set (PPS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pps {
    pub pic_parameter_set_id: u32,
    pub seq_parameter_set_id: u32,
    /// 0 = CAVLC, 1 = CABAC
    pub entropy_coding_mode_flag: bool,
    pub bottom_field_pic_order_in_frame_present_flag: bool,
    pub num_slice_groups_minus1: u32,

    pub num_ref_idx_l0_default_active_minus1: u32,
    pub num_ref_idx_l1_default_active_minus1: u32,
    pub weighted_pred_flag: bool,
    pub weighted_bipred_idc: u8,
    pub pic_init_qp_minus26: i32,
    pub pic_init_qs_minus26: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present_flag: bool,
    pub constrained_intra_pred_flag: bool,
    pub redundant_pic_cnt_present_flag: bool,

    // High profile extensions
    pub transform_8x8_mode_flag: bool,
    pub second_chroma_qp_index_offset: i32,
}

impl Pps {
    /// Parse a PPS from a raw NAL unit (with or without 1-byte NAL header).
    pub fn parse(nal_data: &[u8]) -> Result<Self, BitstreamError> {
        // Strip NAL header byte if present (PPS NAL unit type is 8)
        let payload = if !nal_data.is_empty() && (nal_data[0] & 0x1F) == 8 {
            &nal_data[1..]
        } else {
            nal_data
        };

        let rbsp = remove_emulation_prevention(payload);
        let mut r = BitReader::new(&rbsp);

        let pic_parameter_set_id = r.read_ue()?;
        let seq_parameter_set_id = r.read_ue()?;
        let entropy_coding_mode_flag = r.read_bit()?;
        let bottom_field_pic_order_in_frame_present_flag = r.read_bit()?;
        let num_slice_groups_minus1 = r.read_ue()?;

        if num_slice_groups_minus1 > 0 {
            let slice_group_map_type = r.read_ue()?;
            match slice_group_map_type {
                0 => {
                    for _ in 0..=num_slice_groups_minus1 {
                        let _run_length = r.read_ue()?;
                    }
                }
                2 => {
                    for _ in 0..num_slice_groups_minus1 {
                        let _top_left = r.read_ue()?;
                        let _bottom_right = r.read_ue()?;
                    }
                }
                3..=5 => {
                    let _slice_group_change_direction_flag = r.read_bit()?;
                    let _slice_group_change_rate = r.read_ue()?;
                }
                6 => {
                    let pic_size_in_map_units = r.read_ue()?;
                    let bits = (num_slice_groups_minus1 + 1).next_power_of_two().trailing_zeros() as usize;
                    for _ in 0..=pic_size_in_map_units {
                        let _map = r.read_bits(bits)?;
                    }
                }
                _ => {}
            }
        }

        let num_ref_idx_l0_default_active_minus1 = r.read_ue()?;
        let num_ref_idx_l1_default_active_minus1 = r.read_ue()?;
        let weighted_pred_flag = r.read_bit()?;
        let weighted_bipred_idc = r.read_bits(2)? as u8;
        let pic_init_qp_minus26 = r.read_se()?;
        let pic_init_qs_minus26 = r.read_se()?;
        let chroma_qp_index_offset = r.read_se()?;
        let deblocking_filter_control_present_flag = r.read_bit()?;
        let constrained_intra_pred_flag = r.read_bit()?;
        let redundant_pic_cnt_present_flag = r.read_bit()?;

        let mut transform_8x8_mode_flag = false;
        let mut second_chroma_qp_index_offset = chroma_qp_index_offset;

        if r.has_more_rbsp_data() {
            transform_8x8_mode_flag = r.read_bit()?;
            let pic_scaling_matrix_present_flag = r.read_bit()?;
            if pic_scaling_matrix_present_flag {
                let count = 6 + (2 * if transform_8x8_mode_flag { 1 } else { 0 });
                for i in 0..count {
                    let pic_scaling_list_present_flag = r.read_bit()?;
                    if pic_scaling_list_present_flag {
                        let size = if i < 6 { 16 } else { 64 };
                        skip_scaling_list(&mut r, size)?;
                    }
                }
            }
            if r.has_more_rbsp_data() {
                second_chroma_qp_index_offset = r.read_se()?;
            }
        }

        Ok(Self {
            pic_parameter_set_id,
            seq_parameter_set_id,
            entropy_coding_mode_flag,
            bottom_field_pic_order_in_frame_present_flag,
            num_slice_groups_minus1,
            num_ref_idx_l0_default_active_minus1,
            num_ref_idx_l1_default_active_minus1,
            weighted_pred_flag,
            weighted_bipred_idc,
            pic_init_qp_minus26,
            pic_init_qs_minus26,
            chroma_qp_index_offset,
            deblocking_filter_control_present_flag,
            constrained_intra_pred_flag,
            redundant_pic_cnt_present_flag,
            transform_8x8_mode_flag,
            second_chroma_qp_index_offset,
        })
    }
}

fn skip_scaling_list(r: &mut BitReader, size: usize) -> Result<(), BitstreamError> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = r.read_se()?;
            next_scale = (last_scale + delta_scale + 256) % 256;
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_real_pps() {
        // Real-world PPS for CABAC, 1080p
        // 0x68 followed by payload
        let pps_bytes = [0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0];
        let pps = Pps::parse(&pps_bytes).expect("Failed to parse PPS");

        assert_eq!(pps.pic_parameter_set_id, 0);
        assert_eq!(pps.seq_parameter_set_id, 0);
        assert!(pps.entropy_coding_mode_flag); // CABAC
        assert!(!pps.bottom_field_pic_order_in_frame_present_flag);
        assert_eq!(pps.num_slice_groups_minus1, 0);
        assert!(pps.deblocking_filter_control_present_flag);
    }
}
