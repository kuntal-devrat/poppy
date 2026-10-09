//! Sequence Parameter Set (SPS) parser (ITU-T H.265 §7.3.2.2).

use crate::bitstream::{BitReader, BitstreamError};
use crate::ptl::ProfileTierLevel;

/// Short-Term Reference Picture Set (ITU-T H.265 §7.3.7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShortTermRefPicSet {
    pub num_negative_pics: u32,
    pub num_positive_pics: u32,
    pub delta_poc_s0: Vec<i32>,
    pub used_by_curr_pic_s0: Vec<bool>,
    pub delta_poc_s1: Vec<i32>,
    pub used_by_curr_pic_s1: Vec<bool>,
}

/// Parsed Sequence Parameter Set (SPS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceParameterSet {
    pub sps_video_parameter_set_id: u8,
    pub sps_max_sub_layers_minus1: u8,
    pub sps_temporal_id_nesting_flag: bool,
    pub profile_tier_level: ProfileTierLevel,
    pub sps_seq_parameter_set_id: u32,
    pub chroma_format_idc: u32,
    pub separate_colour_plane_flag: bool,
    pub pic_width_in_luma_samples: u32,
    pub pic_height_in_luma_samples: u32,
    pub conf_win_left_offset: u32,
    pub conf_win_right_offset: u32,
    pub conf_win_top_offset: u32,
    pub conf_win_bottom_offset: u32,
    pub bit_depth_luma_minus8: u32,
    pub bit_depth_chroma_minus8: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,
    pub max_dec_pic_buffering: [u32; 8],
    pub max_num_reorder_pics: [u32; 8],
    pub log2_min_luma_coding_block_size_minus3: u32,
    pub log2_diff_max_min_luma_coding_block_size: u32,
    pub log2_min_luma_transform_block_size_minus2: u32,
    pub log2_diff_max_min_luma_transform_block_size: u32,
    pub max_transform_hierarchy_depth_inter: u32,
    pub max_transform_hierarchy_depth_intra: u32,
    pub scaling_list_enabled_flag: bool,
    pub amp_enabled_flag: bool,
    pub sample_adaptive_offset_enabled_flag: bool,
    pub pcm_enabled_flag: bool,
    pub short_term_ref_pic_sets: Vec<ShortTermRefPicSet>,
    pub long_term_ref_pics_present_flag: bool,
    pub sps_temporal_mvp_enabled_flag: bool,
    pub strong_intra_smoothing_enabled_flag: bool,
    pub vui_parameters_present_flag: bool,
}

impl SequenceParameterSet {
    /// Parses a Sequence Parameter Set from its RBSP byte slice.
    pub fn parse(rbsp: &[u8]) -> Result<Self, BitstreamError> {
        let mut reader = BitReader::new(rbsp);

        let sps_video_parameter_set_id = reader.read_bits(4)? as u8;
        let sps_max_sub_layers_minus1 = reader.read_bits(3)? as u8;
        let sps_temporal_id_nesting_flag = reader.read_bit()?;

        let profile_tier_level = ProfileTierLevel::parse(
            &mut reader,
            true,
            sps_max_sub_layers_minus1 as u32,
        )?;

        let sps_seq_parameter_set_id = reader.read_ue()?;
        let chroma_format_idc = reader.read_ue()?;

        let mut separate_colour_plane_flag = false;
        if chroma_format_idc == 3 {
            separate_colour_plane_flag = reader.read_bit()?;
        }

        let pic_width_in_luma_samples = reader.read_ue()?;
        let pic_height_in_luma_samples = reader.read_ue()?;

        let conformance_window_flag = reader.read_bit()?;
        let (mut conf_win_left, mut conf_win_right, mut conf_win_top, mut conf_win_bottom) = (0, 0, 0, 0);

        if conformance_window_flag {
            conf_win_left = reader.read_ue()?;
            conf_win_right = reader.read_ue()?;
            conf_win_top = reader.read_ue()?;
            conf_win_bottom = reader.read_ue()?;
        }

        let bit_depth_luma_minus8 = reader.read_ue()?;
        let bit_depth_chroma_minus8 = reader.read_ue()?;
        let log2_max_pic_order_cnt_lsb_minus4 = reader.read_ue()?;

        let sps_sub_layer_ordering_info_present_flag = reader.read_bit()?;
        let mut max_dec_pic_buffering = [0u32; 8];
        let mut max_num_reorder_pics = [0u32; 8];

        let start_sub_layer = if sps_sub_layer_ordering_info_present_flag {
            0
        } else {
            sps_max_sub_layers_minus1 as usize
        };

        for i in start_sub_layer..=sps_max_sub_layers_minus1 as usize {
            if i < 8 {
                let max_dpb = reader.read_ue()? + 1;
                let max_reorder = reader.read_ue()?;
                let _max_latency = reader.read_ue()?;
                max_dec_pic_buffering[i] = max_dpb;
                max_num_reorder_pics[i] = max_reorder;
            }
        }

        if !sps_sub_layer_ordering_info_present_flag {
            for i in 0..sps_max_sub_layers_minus1 as usize {
                if i < 8 {
                    max_dec_pic_buffering[i] = max_dec_pic_buffering[sps_max_sub_layers_minus1 as usize];
                    max_num_reorder_pics[i] = max_num_reorder_pics[sps_max_sub_layers_minus1 as usize];
                }
            }
        }

        let log2_min_luma_coding_block_size_minus3 = reader.read_ue()?;
        let log2_diff_max_min_luma_coding_block_size = reader.read_ue()?;
        let log2_min_luma_transform_block_size_minus2 = reader.read_ue()?;
        let log2_diff_max_min_luma_transform_block_size = reader.read_ue()?;
        let max_transform_hierarchy_depth_inter = reader.read_ue()?;
        let max_transform_hierarchy_depth_intra = reader.read_ue()?;

        let scaling_list_enabled_flag = reader.read_bit()?;
        if scaling_list_enabled_flag {
            let sps_scaling_list_data_present_flag = reader.read_bit()?;
            if sps_scaling_list_data_present_flag {
                skip_scaling_list_data(&mut reader)?;
            }
        }

        let amp_enabled_flag = reader.read_bit()?;
        let sample_adaptive_offset_enabled_flag = reader.read_bit()?;
        let pcm_enabled_flag = reader.read_bit()?;

        if pcm_enabled_flag {
            reader.skip_bits(4)?; // pcm_sample_bit_depth_luma_minus1
            reader.skip_bits(4)?; // pcm_sample_bit_depth_chroma_minus1
            let _ = reader.read_ue()?; // log2_min_pcm_luma_coding_block_size_minus3
            let _ = reader.read_ue()?; // log2_diff_max_min_pcm_luma_coding_block_size
            reader.skip_bits(1)?; // pcm_loop_filter_disabled_flag
        }

        let num_short_term_ref_pic_sets = reader.read_ue()?;
        let mut short_term_ref_pic_sets = Vec::with_capacity(num_short_term_ref_pic_sets as usize);

        for i in 0..num_short_term_ref_pic_sets {
            let rps = parse_short_term_ref_pic_set(i, &short_term_ref_pic_sets, &mut reader)?;
            short_term_ref_pic_sets.push(rps);
        }

        let long_term_ref_pics_present_flag = reader.read_bit()?;
        if long_term_ref_pics_present_flag {
            let num_lt_rps = reader.read_ue()?;
            let poc_bits = (log2_max_pic_order_cnt_lsb_minus4 + 4) as usize;
            for _ in 0..num_lt_rps {
                reader.skip_bits(poc_bits)?;
                reader.skip_bits(1)?; // used_by_curr_pic_lt_sps_flag
            }
        }

        let sps_temporal_mvp_enabled_flag = reader.read_bit()?;
        let strong_intra_smoothing_enabled_flag = reader.read_bit()?;
        let vui_parameters_present_flag = reader.read_bit()?;

        Ok(Self {
            sps_video_parameter_set_id,
            sps_max_sub_layers_minus1,
            sps_temporal_id_nesting_flag,
            profile_tier_level,
            sps_seq_parameter_set_id,
            chroma_format_idc,
            separate_colour_plane_flag,
            pic_width_in_luma_samples,
            pic_height_in_luma_samples,
            conf_win_left_offset: conf_win_left,
            conf_win_right_offset: conf_win_right,
            conf_win_top_offset: conf_win_top,
            conf_win_bottom_offset: conf_win_bottom,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            log2_max_pic_order_cnt_lsb_minus4,
            max_dec_pic_buffering,
            max_num_reorder_pics,
            log2_min_luma_coding_block_size_minus3,
            log2_diff_max_min_luma_coding_block_size,
            log2_min_luma_transform_block_size_minus2,
            log2_diff_max_min_luma_transform_block_size,
            max_transform_hierarchy_depth_inter,
            max_transform_hierarchy_depth_intra,
            scaling_list_enabled_flag,
            amp_enabled_flag,
            sample_adaptive_offset_enabled_flag,
            pcm_enabled_flag,
            short_term_ref_pic_sets,
            long_term_ref_pics_present_flag,
            sps_temporal_mvp_enabled_flag,
            strong_intra_smoothing_enabled_flag,
            vui_parameters_present_flag,
        })
    }

    /// Cropped presentation width in pixels.
    pub fn cropped_width(&self) -> u32 {
        let sub_width_c = if self.chroma_format_idc == 1 || self.chroma_format_idc == 2 {
            2
        } else {
            1
        };
        let crop = sub_width_c * (self.conf_win_left_offset + self.conf_win_right_offset);
        self.pic_width_in_luma_samples.saturating_sub(crop)
    }

    /// Cropped presentation height in pixels.
    pub fn cropped_height(&self) -> u32 {
        let sub_height_c = if self.chroma_format_idc == 1 { 2 } else { 1 };
        let crop = sub_height_c * (self.conf_win_top_offset + self.conf_win_bottom_offset);
        self.pic_height_in_luma_samples.saturating_sub(crop)
    }

    /// Pixel bit depth (8 for standard, 10 for HDR).
    pub fn bit_depth(&self) -> u8 {
        8 + self.bit_depth_luma_minus8 as u8
    }

    /// Returns `true` if this stream has 10-bit color depth.
    pub fn is_10bit(&self) -> bool {
        self.bit_depth() == 10
    }
}

fn skip_scaling_list_data(reader: &mut BitReader<'_>) -> Result<(), BitstreamError> {
    for size_id in 0..4 {
        let step = if size_id == 3 { 3 } else { 1 };
        for _matrix_id in (0..6).step_by(step) {
            let scaling_list_pred_mode_flag = reader.read_bit()?;
            if !scaling_list_pred_mode_flag {
                let _pred_matrix_id = reader.read_ue()?;
            } else {
                let coef_num = (1usize << (4 + (size_id << 1))).min(64);
                if size_id > 1 {
                    let _scaling_list_dc_coef = reader.read_se()?;
                }
                for _ in 0..coef_num {
                    let _delta = reader.read_se()?;
                }
            }
        }
    }
    Ok(())
}

fn parse_short_term_ref_pic_set(
    st_rps_idx: u32,
    existing_sets: &[ShortTermRefPicSet],
    reader: &mut BitReader<'_>,
) -> Result<ShortTermRefPicSet, BitstreamError> {
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
        let _delta_rps_sign = reader.read_bit()?;
        let _abs_delta_rps_minus1 = reader.read_ue()?;

        let ref_rps_idx = st_rps_idx as usize - 1 - delta_idx_minus1 as usize;
        let num_pics = if ref_rps_idx < existing_sets.len() {
            existing_sets[ref_rps_idx].num_negative_pics + existing_sets[ref_rps_idx].num_positive_pics
        } else {
            0
        };

        for _ in 0..=num_pics {
            let used_by_curr_pic_flag = reader.read_bit()?;
            if !used_by_curr_pic_flag {
                let _use_delta_flag = reader.read_bit()?;
            }
        }

        Ok(ShortTermRefPicSet::default())
    } else {
        let num_negative_pics = reader.read_ue()?;
        let num_positive_pics = reader.read_ue()?;

        let mut delta_poc_s0 = Vec::with_capacity(num_negative_pics as usize);
        let mut used_by_curr_pic_s0 = Vec::with_capacity(num_negative_pics as usize);

        for _ in 0..num_negative_pics {
            let delta_poc_s0_minus1 = reader.read_ue()?;
            let used = reader.read_bit()?;
            delta_poc_s0.push(-(delta_poc_s0_minus1 as i32 + 1));
            used_by_curr_pic_s0.push(used);
        }

        let mut delta_poc_s1 = Vec::with_capacity(num_positive_pics as usize);
        let mut used_by_curr_pic_s1 = Vec::with_capacity(num_positive_pics as usize);

        for _ in 0..num_positive_pics {
            let delta_poc_s1_minus1 = reader.read_ue()?;
            let used = reader.read_bit()?;
            delta_poc_s1.push(delta_poc_s1_minus1 as i32 + 1);
            used_by_curr_pic_s1.push(used);
        }

        Ok(ShortTermRefPicSet {
            num_negative_pics,
            num_positive_pics,
            delta_poc_s0,
            used_by_curr_pic_s0,
            delta_poc_s1,
            used_by_curr_pic_s1,
        })
    }
}
