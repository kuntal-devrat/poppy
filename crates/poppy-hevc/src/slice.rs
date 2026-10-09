//! Slice segment header parser (ITU-T H.265 §7.3.6.1).

use crate::bitstream::{BitReader, BitstreamError};
use crate::nal::HevcNalType;
use crate::pps::PictureParameterSet;
use crate::sps::SequenceParameterSet;

/// HEVC slice coding type per ITU-T H.265 §7.4.7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcSliceType {
    /// B slice (bi-predictive)
    B = 0,
    /// P slice (predictive)
    P = 1,
    /// I slice (intra)
    I = 2,
}

impl HevcSliceType {
    pub fn from_u32(val: u32) -> Result<Self, BitstreamError> {
        match val {
            0 => Ok(Self::B),
            1 => Ok(Self::P),
            2 => Ok(Self::I),
            _ => Err(BitstreamError::InvalidSyntax(format!(
                "Invalid slice_type {val}"
            ))),
        }
    }
}

/// Parsed HEVC Slice Segment Header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceSegmentHeader {
    pub first_slice_segment_in_pic_flag: bool,
    pub no_output_of_prior_pics_flag: bool,
    pub slice_pic_parameter_set_id: u32,
    pub dependent_slice_segment_flag: bool,
    pub slice_segment_address: u32,
    pub slice_type: HevcSliceType,
    pub pic_output_flag: bool,
    pub slice_pic_order_cnt_lsb: u32,
    pub slice_sao_luma_flag: bool,
    pub slice_sao_chroma_flag: bool,
    pub num_ref_idx_l0_active: u32,
    pub num_ref_idx_l1_active: u32,
    pub slice_qp_delta: i32,
    pub slice_temporal_mvp_enabled_flag: bool,
    pub collocated_from_l0_flag: bool,
    pub collocated_ref_idx: u8,
    pub mvd_l1_zero_flag: bool,
    pub cabac_init_flag: bool,
    pub slice_deblocking_filter_disabled_flag: bool,
    pub slice_loop_filter_across_slices_enabled_flag: bool,
    pub five_minus_max_num_merge_cand: u8,
    pub st_rps_bits: u32,
    pub slice_data_byte_offset: usize,
}

impl SliceSegmentHeader {
    /// Parses a slice segment header given the active SPS and PPS.
    pub fn parse(
        rbsp: &[u8],
        nal_type: HevcNalType,
        sps: &SequenceParameterSet,
        pps: &PictureParameterSet,
    ) -> Result<Self, BitstreamError> {
        let mut reader = BitReader::new(rbsp);

        let first_slice_segment_in_pic_flag = reader.read_bit()?;
        let mut no_output_of_prior_pics_flag = false;

        if nal_type.is_irap() {
            no_output_of_prior_pics_flag = reader.read_bit()?;
        }

        let slice_pic_parameter_set_id = reader.read_ue()?;
        let mut dependent_slice_segment_flag = false;

        let mut slice_segment_address = 0;
        if !first_slice_segment_in_pic_flag {
            if pps.dependent_slice_segments_enabled_flag {
                dependent_slice_segment_flag = reader.read_bit()?;
            }

            // Read slice_segment_address
            let min_cb_log2 = sps.log2_min_luma_coding_block_size_minus3.saturating_add(3);
            let ctb_log2 = min_cb_log2
                .saturating_add(sps.log2_diff_max_min_luma_coding_block_size)
                .min(16);
            let ctb_size = (1u32 << ctb_log2).max(1);
            let pic_width_in_ctbs = sps.pic_width_in_luma_samples.div_ceil(ctb_size);
            let pic_height_in_ctbs = sps.pic_height_in_luma_samples.div_ceil(ctb_size);
            let pic_size_in_ctbs =
                (pic_width_in_ctbs as u64).saturating_mul(pic_height_in_ctbs as u64);

            let mut address_bits = 0;
            while address_bits < 64 && (1u64 << address_bits) < pic_size_in_ctbs {
                address_bits += 1;
            }
            if address_bits > 0 && address_bits <= 32 {
                slice_segment_address = reader.read_bits(address_bits)?;
            }
        }

        let mut slice_type = HevcSliceType::I;
        let mut pic_output_flag = true;
        let mut slice_pic_order_cnt_lsb = 0;
        let mut slice_sao_luma_flag = false;
        let mut slice_sao_chroma_flag = false;
        let mut num_ref_idx_l0_active = pps.num_ref_idx_l0_default_active_minus1 + 1;
        let mut num_ref_idx_l1_active = pps.num_ref_idx_l1_default_active_minus1 + 1;
        let mut slice_qp_delta = 0;
        let mut slice_temporal_mvp_enabled_flag = false;
        let mut collocated_from_l0_flag = true;
        let mut collocated_ref_idx = 0xffu8;
        let mut mvd_l1_zero_flag = false;
        let mut cabac_init_flag = false;
        let mut disable_deblocking_filter_flag = pps.pps_deblocking_filter_disabled_flag;
        let mut slice_loop_filter_across_slices_enabled_flag =
            pps.pps_loop_filter_across_slices_enabled_flag;
        let mut five_minus_max_num_merge_cand = 0u8;
        let mut st_rps_bits = 0;

        if !dependent_slice_segment_flag {
            if pps.num_extra_slice_header_bits > 0 {
                reader.skip_bits(pps.num_extra_slice_header_bits as usize)?;
            }

            let slice_type_raw = reader.read_ue()?;
            slice_type = HevcSliceType::from_u32(slice_type_raw)?;

            if pps.output_flag_present_flag {
                pic_output_flag = reader.read_bit()?;
            }

            if sps.separate_colour_plane_flag {
                reader.skip_bits(2)?;
            }

            if !nal_type.is_idr() {
                let poc_bits = (sps.log2_max_pic_order_cnt_lsb_minus4 + 4) as usize;
                slice_pic_order_cnt_lsb = reader.read_bits(poc_bits)?;

                let short_term_ref_pic_set_sps_flag = reader.read_bit()?;
                if !short_term_ref_pic_set_sps_flag {
                    // Inline short-term RPS
                    let rps_start = reader.bits_read();
                    let num_st_rps = sps.short_term_ref_pic_sets.len() as u32;
                    parse_inline_short_term_rps(
                        num_st_rps,
                        &sps.short_term_ref_pic_sets,
                        &mut reader,
                    )?;
                    st_rps_bits = (reader.bits_read() - rps_start) as u32;
                } else if sps.short_term_ref_pic_sets.len() > 1 {
                    let mut rps_idx_bits = 0;
                    while (1 << rps_idx_bits) < sps.short_term_ref_pic_sets.len() {
                        rps_idx_bits += 1;
                    }
                    if rps_idx_bits > 0 {
                        reader.skip_bits(rps_idx_bits)?;
                    }
                }

                if sps.sps_temporal_mvp_enabled_flag {
                    slice_temporal_mvp_enabled_flag = reader.read_bit()?;
                }
            }

            if sps.sample_adaptive_offset_enabled_flag {
                slice_sao_luma_flag = reader.read_bit()?;
                if sps.chroma_format_idc != 0 {
                    slice_sao_chroma_flag = reader.read_bit()?;
                }
            }

            if slice_type == HevcSliceType::P || slice_type == HevcSliceType::B {
                let num_ref_idx_active_override_flag = reader.read_bit()?;
                if num_ref_idx_active_override_flag {
                    num_ref_idx_l0_active = reader.read_ue()? + 1;
                    if slice_type == HevcSliceType::B {
                        num_ref_idx_l1_active = reader.read_ue()? + 1;
                    }
                }

                if slice_type == HevcSliceType::B {
                    mvd_l1_zero_flag = reader.read_bit()?;
                }

                if pps.cabac_init_present_flag {
                    cabac_init_flag = reader.read_bit()?;
                }

                if slice_temporal_mvp_enabled_flag {
                    collocated_from_l0_flag = if slice_type == HevcSliceType::B {
                        reader.read_bit()?
                    } else {
                        true
                    };
                    if (collocated_from_l0_flag && num_ref_idx_l0_active > 1)
                        || (!collocated_from_l0_flag && num_ref_idx_l1_active > 1)
                    {
                        collocated_ref_idx = reader.read_ue()? as u8;
                    } else {
                        collocated_ref_idx = 0;
                    }
                }

                if (pps.weighted_pred_flag && slice_type == HevcSliceType::P)
                    || (pps.weighted_bipred_flag && slice_type == HevcSliceType::B)
                {
                    let _luma_log2_weight_denom = reader.read_ue()?;
                    if !sps.separate_colour_plane_flag && sps.chroma_format_idc != 0 {
                        let _delta_chroma_log2_weight_denom = reader.read_se()?;
                    }
                    let mut luma_weight_l0 = Vec::with_capacity(num_ref_idx_l0_active as usize);
                    for _ in 0..num_ref_idx_l0_active {
                        luma_weight_l0.push(reader.read_bit()?);
                    }
                    let mut chroma_weight_l0 = Vec::with_capacity(num_ref_idx_l0_active as usize);
                    if !sps.separate_colour_plane_flag && sps.chroma_format_idc != 0 {
                        for _ in 0..num_ref_idx_l0_active {
                            chroma_weight_l0.push(reader.read_bit()?);
                        }
                    }
                    for (i, &flag) in luma_weight_l0.iter().enumerate() {
                        if flag {
                            let _ = reader.read_se()?;
                            let _ = reader.read_se()?;
                        }
                        if chroma_weight_l0.get(i).copied().unwrap_or(false) {
                            for _ in 0..2 {
                                let _ = reader.read_se()?;
                                let _ = reader.read_se()?;
                            }
                        }
                    }
                    if slice_type == HevcSliceType::B {
                        let mut luma_weight_l1 = Vec::with_capacity(num_ref_idx_l1_active as usize);
                        for _ in 0..num_ref_idx_l1_active {
                            luma_weight_l1.push(reader.read_bit()?);
                        }
                        let mut chroma_weight_l1 =
                            Vec::with_capacity(num_ref_idx_l1_active as usize);
                        if !sps.separate_colour_plane_flag && sps.chroma_format_idc != 0 {
                            for _ in 0..num_ref_idx_l1_active {
                                chroma_weight_l1.push(reader.read_bit()?);
                            }
                        }
                        for (i, &flag) in luma_weight_l1.iter().enumerate() {
                            if flag {
                                let _ = reader.read_se()?;
                                let _ = reader.read_se()?;
                            }
                            if chroma_weight_l1.get(i).copied().unwrap_or(false) {
                                for _ in 0..2 {
                                    let _ = reader.read_se()?;
                                    let _ = reader.read_se()?;
                                }
                            }
                        }
                    }
                }

                five_minus_max_num_merge_cand = reader.read_ue()? as u8;
            }

            slice_qp_delta = reader.read_se()?;

            if pps.pps_slice_chroma_qp_offsets_present_flag {
                let _ = reader.read_se()?;
                let _ = reader.read_se()?;
            }

            if pps.deblocking_filter_control_present_flag {
                let mut deblocking_filter_override_flag = false;
                if pps.deblocking_filter_override_enabled_flag {
                    deblocking_filter_override_flag = reader.read_bit()?;
                }
                if deblocking_filter_override_flag {
                    disable_deblocking_filter_flag = reader.read_bit()?;
                    if !disable_deblocking_filter_flag {
                        let _ = reader.read_se()?;
                        let _ = reader.read_se()?;
                    }
                }
            }

            if pps.pps_loop_filter_across_slices_enabled_flag
                && (slice_sao_luma_flag || slice_sao_chroma_flag || !disable_deblocking_filter_flag)
            {
                slice_loop_filter_across_slices_enabled_flag = reader.read_bit()?;
            }
        }

        if pps.tiles_enabled_flag || pps.entropy_coding_sync_enabled_flag {
            let num_entry_point_offsets = reader.read_ue()?;
            if num_entry_point_offsets > 0 {
                let offset_len = reader.read_ue()? + 1;
                for _ in 0..num_entry_point_offsets {
                    reader.skip_bits(offset_len as usize)?;
                }
            }
        }

        if pps.slice_segment_header_extension_present_flag {
            let length = reader.read_ue()?;
            for _ in 0..length {
                reader.skip_bits(8)?;
            }
        }

        // Byte alignment: rbsp_slice_segment_trailing_bits
        let _ = reader.read_bit();
        let remainder = reader.bits_read() % 8;
        if remainder != 0 {
            let _ = reader.skip_bits(8 - remainder);
        }

        let slice_data_byte_offset = 2 + (reader.bits_read() / 8);

        Ok(Self {
            first_slice_segment_in_pic_flag,
            no_output_of_prior_pics_flag,
            slice_pic_parameter_set_id,
            dependent_slice_segment_flag,
            slice_segment_address,
            slice_type,
            pic_output_flag,
            slice_pic_order_cnt_lsb,
            slice_sao_luma_flag,
            slice_sao_chroma_flag,
            num_ref_idx_l0_active,
            num_ref_idx_l1_active,
            slice_qp_delta,
            slice_temporal_mvp_enabled_flag,
            collocated_from_l0_flag,
            collocated_ref_idx,
            mvd_l1_zero_flag,
            cabac_init_flag,
            slice_deblocking_filter_disabled_flag: disable_deblocking_filter_flag,
            slice_loop_filter_across_slices_enabled_flag,
            five_minus_max_num_merge_cand,
            st_rps_bits,
            slice_data_byte_offset,
        })
    }
}

fn parse_inline_short_term_rps(
    st_rps_idx: u32,
    existing_sets: &[crate::sps::ShortTermRefPicSet],
    reader: &mut BitReader<'_>,
) -> Result<(), BitstreamError> {
    let mut inter_ref_pic_set_prediction_flag = false;
    if st_rps_idx != 0 {
        inter_ref_pic_set_prediction_flag = reader.read_bit()?;
    }

    if inter_ref_pic_set_prediction_flag {
        let delta_idx_minus1 = if st_rps_idx as usize == existing_sets.len() {
            reader.read_ue()?
        } else {
            0
        };
        let _ = reader.read_bit()?;
        let _ = reader.read_ue()?;

        let delta = match (delta_idx_minus1 as usize).checked_add(1) {
            Some(d) => d,
            None => {
                return Err(BitstreamError::InvalidSyntax(
                    "delta_idx_minus1 overflow".into(),
                ));
            }
        };
        if delta > st_rps_idx as usize {
            return Err(BitstreamError::InvalidSyntax(format!(
                "invalid ref_rps_idx: st_rps_idx {st_rps_idx} < delta {delta}"
            )));
        }
        let ref_rps_idx = st_rps_idx as usize - delta;
        let num_pics = if ref_rps_idx < existing_sets.len() {
            existing_sets[ref_rps_idx]
                .num_negative_pics
                .saturating_add(existing_sets[ref_rps_idx].num_positive_pics)
        } else {
            0
        };

        for _ in 0..=num_pics {
            let used = reader.read_bit()?;
            if !used {
                let _ = reader.read_bit()?;
            }
        }
    } else {
        let num_neg = reader.read_ue()?;
        let num_pos = reader.read_ue()?;
        if num_neg > 16 || num_pos > 16 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "num_pics exceeds max reference limit (neg: {num_neg}, pos: {num_pos})"
            )));
        }
        for _ in 0..num_neg {
            let _ = reader.read_ue()?;
            let _ = reader.read_bit()?;
        }
        for _ in 0..num_pos {
            let _ = reader.read_ue()?;
            let _ = reader.read_bit()?;
        }
    }
    Ok(())
}
