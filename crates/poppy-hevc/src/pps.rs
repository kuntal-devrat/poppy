//! Picture Parameter Set (PPS) parser (ITU-T H.265 §7.3.2.3).

use crate::bitstream::{BitReader, BitstreamError};

/// Parsed Picture Parameter Set (PPS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureParameterSet {
    pub pps_pic_parameter_set_id: u32,
    pub pps_seq_parameter_set_id: u32,
    pub dependent_slice_segments_enabled_flag: bool,
    pub output_flag_present_flag: bool,
    pub num_extra_slice_header_bits: u8,
    pub sign_data_hiding_enabled_flag: bool,
    pub cabac_init_present_flag: bool,
    pub num_ref_idx_l0_default_active_minus1: u32,
    pub num_ref_idx_l1_default_active_minus1: u32,
    pub init_qp_minus26: i32,
    pub constrained_intra_pred_flag: bool,
    pub transform_skip_enabled_flag: bool,
    pub cu_qp_delta_enabled_flag: bool,
    pub diff_cu_qp_delta_depth: u32,
    pub pps_cb_qp_offset: i32,
    pub pps_cr_qp_offset: i32,
    pub pps_slice_chroma_qp_offsets_present_flag: bool,
    pub weighted_pred_flag: bool,
    pub weighted_bipred_flag: bool,
    pub transquant_bypass_enabled_flag: bool,
    pub tiles_enabled_flag: bool,
    pub entropy_coding_sync_enabled_flag: bool,
    pub num_tile_columns_minus1: u32,
    pub num_tile_rows_minus1: u32,
    pub uniform_spacing_flag: bool,
    pub column_width_minus1: Vec<u32>,
    pub row_height_minus1: Vec<u32>,
    pub loop_filter_across_tiles_enabled_flag: bool,
    pub pps_loop_filter_across_slices_enabled_flag: bool,
    pub deblocking_filter_control_present_flag: bool,
    pub deblocking_filter_override_enabled_flag: bool,
    pub pps_deblocking_filter_disabled_flag: bool,
    pub pps_beta_offset_div2: i32,
    pub pps_tc_offset_div2: i32,
    pub lists_modification_present_flag: bool,
    pub log2_parallel_merge_level_minus2: u32,
    pub slice_segment_header_extension_present_flag: bool,
    pub pps_scaling_list_data_present_flag: bool,
}

impl PictureParameterSet {
    /// Parses a Picture Parameter Set from its RBSP byte slice.
    pub fn parse(rbsp: &[u8]) -> Result<Self, BitstreamError> {
        let mut reader = BitReader::new(rbsp);

        let pps_pic_parameter_set_id = reader.read_ue()?;
        let pps_seq_parameter_set_id = reader.read_ue()?;
        let dependent_slice_segments_enabled_flag = reader.read_bit()?;
        let output_flag_present_flag = reader.read_bit()?;
        let num_extra_slice_header_bits = reader.read_bits(3)? as u8;
        let sign_data_hiding_enabled_flag = reader.read_bit()?;
        let cabac_init_present_flag = reader.read_bit()?;
        let num_ref_idx_l0_default_active_minus1 = reader.read_ue()?;
        let num_ref_idx_l1_default_active_minus1 = reader.read_ue()?;
        let init_qp_minus26 = reader.read_se()?;
        let constrained_intra_pred_flag = reader.read_bit()?;
        let transform_skip_enabled_flag = reader.read_bit()?;

        let cu_qp_delta_enabled_flag = reader.read_bit()?;
        let mut diff_cu_qp_delta_depth = 0;
        if cu_qp_delta_enabled_flag {
            diff_cu_qp_delta_depth = reader.read_ue()?;
        }

        let pps_cb_qp_offset = reader.read_se()?;
        let pps_cr_qp_offset = reader.read_se()?;
        let pps_slice_chroma_qp_offsets_present_flag = reader.read_bit()?;
        let weighted_pred_flag = reader.read_bit()?;
        let weighted_bipred_flag = reader.read_bit()?;
        let transquant_bypass_enabled_flag = reader.read_bit()?;
        let tiles_enabled_flag = reader.read_bit()?;
        let entropy_coding_sync_enabled_flag = reader.read_bit()?;

        let mut num_tile_columns_minus1 = 0;
        let mut num_tile_rows_minus1 = 0;
        let mut uniform_spacing_flag = true;
        let mut column_width_minus1 = Vec::new();
        let mut row_height_minus1 = Vec::new();
        let mut loop_filter_across_tiles_enabled_flag = true;

        if tiles_enabled_flag {
            num_tile_columns_minus1 = reader.read_ue()?;
            num_tile_rows_minus1 = reader.read_ue()?;
            uniform_spacing_flag = reader.read_bit()?;

            if !uniform_spacing_flag {
                for _ in 0..num_tile_columns_minus1 {
                    column_width_minus1.push(reader.read_ue()?);
                }
                for _ in 0..num_tile_rows_minus1 {
                    row_height_minus1.push(reader.read_ue()?);
                }
            }

            loop_filter_across_tiles_enabled_flag = reader.read_bit()?;
        }

        let pps_loop_filter_across_slices_enabled_flag = reader.read_bit()?;
        let deblocking_filter_control_present_flag = reader.read_bit()?;

        let mut deblocking_filter_override_enabled_flag = false;
        let mut pps_deblocking_filter_disabled_flag = false;
        let mut pps_beta_offset_div2 = 0;
        let mut pps_tc_offset_div2 = 0;

        if deblocking_filter_control_present_flag {
            deblocking_filter_override_enabled_flag = reader.read_bit()?;
            pps_deblocking_filter_disabled_flag = reader.read_bit()?;
            if !pps_deblocking_filter_disabled_flag {
                pps_beta_offset_div2 = reader.read_se()?;
                pps_tc_offset_div2 = reader.read_se()?;
            }
        }

        let pps_scaling_list_data_present_flag = reader.read_bit()?;
        if pps_scaling_list_data_present_flag {
            // Skip PPS scaling list data
            for size_id in 0..4 {
                let step = if size_id == 3 { 3 } else { 1 };
                for _matrix_id in (0..6).step_by(step) {
                    let flag = reader.read_bit()?;
                    if !flag {
                        let _ = reader.read_ue()?;
                    } else {
                        let coef_num = (1usize << (4 + (size_id << 1))).min(64);
                        if size_id > 1 {
                            let _ = reader.read_se()?;
                        }
                        for _ in 0..coef_num {
                            let _ = reader.read_se()?;
                        }
                    }
                }
            }
        }

        let lists_modification_present_flag = reader.read_bit()?;
        let log2_parallel_merge_level_minus2 = reader.read_ue()?;
        let slice_segment_header_extension_present_flag = reader.read_bit()?;

        Ok(Self {
            pps_pic_parameter_set_id,
            pps_seq_parameter_set_id,
            dependent_slice_segments_enabled_flag,
            output_flag_present_flag,
            num_extra_slice_header_bits,
            sign_data_hiding_enabled_flag,
            cabac_init_present_flag,
            num_ref_idx_l0_default_active_minus1,
            num_ref_idx_l1_default_active_minus1,
            init_qp_minus26,
            constrained_intra_pred_flag,
            transform_skip_enabled_flag,
            cu_qp_delta_enabled_flag,
            diff_cu_qp_delta_depth,
            pps_cb_qp_offset,
            pps_cr_qp_offset,
            pps_slice_chroma_qp_offsets_present_flag,
            weighted_pred_flag,
            weighted_bipred_flag,
            transquant_bypass_enabled_flag,
            tiles_enabled_flag,
            entropy_coding_sync_enabled_flag,
            num_tile_columns_minus1,
            num_tile_rows_minus1,
            uniform_spacing_flag,
            column_width_minus1,
            row_height_minus1,
            loop_filter_across_tiles_enabled_flag,
            pps_loop_filter_across_slices_enabled_flag,
            deblocking_filter_control_present_flag,
            deblocking_filter_override_enabled_flag,
            pps_deblocking_filter_disabled_flag,
            pps_beta_offset_div2,
            pps_tc_offset_div2,
            lists_modification_present_flag,
            log2_parallel_merge_level_minus2,
            slice_segment_header_extension_present_flag,
            pps_scaling_list_data_present_flag,
        })
    }
}
