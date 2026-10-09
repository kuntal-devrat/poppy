//! Sequence Header OBU parser (AV1 specification §5.5).

use crate::bitstream::{BitReader, BitstreamError};

/// AV1 Sequence Header syntax parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceHeader {
    pub seq_profile: u8,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub frame_width_bits_minus_1: u8,
    pub frame_height_bits_minus_1: u8,
    pub max_frame_width: u32,
    pub max_frame_height: u32,
    pub frame_id_numbers_present_flag: bool,
    pub delta_frame_id_length_minus_2: u8,
    pub additional_frame_id_length_minus_1: u8,
    pub use_128x128_superblock: bool,
    pub enable_filter_intra: bool,
    pub enable_intra_edge_filter: bool,
    pub enable_interintra_compound: bool,
    pub enable_masked_compound: bool,
    pub enable_warped_motion: bool,
    pub enable_dual_filter: bool,
    pub enable_order_hint: bool,
    pub enable_jnt_comp: bool,
    pub enable_ref_frame_mvs: bool,
    pub seq_force_screen_content_tools: u8,
    pub seq_force_integer_mv: u8,
    pub order_hint_bits_minus_1: u8,
    pub enable_superres: bool,
    pub enable_cdef: bool,
    pub enable_restoration: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub mono_chrome: bool,
    pub color_range: bool,
    pub subsampling_x: bool,
    pub subsampling_y: bool,
    pub separate_uv_delta_q: bool,
    pub matrix_coefficients: u8,
    pub film_grain_params_present: bool,
}

impl SequenceHeader {
    /// Parses an AV1 Sequence Header OBU payload.
    pub fn parse(payload: &[u8]) -> Result<Self, BitstreamError> {
        let mut r = BitReader::new(payload);

        let seq_profile = r.read_bits(3)? as u8;
        let still_picture = r.read_bit()?;
        let reduced_still_picture_header = r.read_bit()?;

        if reduced_still_picture_header {
            let _seq_level_idx = r.read_bits(5)?;
        } else {
            let timing_info_present_flag = r.read_bit()?;
            let mut decoder_model_info_present_flag = false;
            if timing_info_present_flag {
                let _num_units_in_display_tick = r.read_bits(32)?;
                let _time_scale = r.read_bits(32)?;
                let equal_picture_interval = r.read_bit()?;
                if equal_picture_interval {
                    let _num_ticks_per_picture_minus_1 = r.read_uvlc()?;
                }
                decoder_model_info_present_flag = r.read_bit()?;
                if decoder_model_info_present_flag {
                    let _buffer_delay_length_minus_1 = r.read_bits(5)?;
                    let _num_units_in_decoding_tick = r.read_bits(32)?;
                    let _buffer_removal_time_length_minus_1 = r.read_bits(5)?;
                    let _frame_presentation_time_length_minus_1 = r.read_bits(5)?;
                }
            }

            let initial_display_delay_present_flag = r.read_bit()?;
            let operating_points_cnt_minus_1 = r.read_bits(5)? as usize;
            for _ in 0..=operating_points_cnt_minus_1 {
                let _operating_point_idc = r.read_bits(12)?;
                let _seq_level_idx = r.read_bits(5)?;
                if _seq_level_idx > 7 {
                    let _seq_tier = r.read_bit()?;
                }
                if decoder_model_info_present_flag {
                    let decoder_model_present_for_this_op = r.read_bit()?;
                    if decoder_model_present_for_this_op {
                        let _decoder_buffer_delay = r.read_bits(10)?;
                        let _encoder_buffer_delay = r.read_bits(10)?;
                        let _low_delay_mode_flag = r.read_bit()?;
                    }
                }
                if initial_display_delay_present_flag {
                    let initial_display_delay_present_for_this_op = r.read_bit()?;
                    if initial_display_delay_present_for_this_op {
                        let _initial_display_delay_minus_1 = r.read_bits(4)?;
                    }
                }
            }
        }

        // Frame dimensions
        let frame_width_bits_minus_1 = r.read_bits(4)? as usize;
        let frame_height_bits_minus_1 = r.read_bits(4)? as usize;
        let max_frame_width = r.read_bits(frame_width_bits_minus_1 + 1)? + 1;
        let max_frame_height = r.read_bits(frame_height_bits_minus_1 + 1)? + 1;

        let mut frame_id_numbers_present_flag = false;
        let mut delta_frame_id_length_minus_2 = 0;
        let mut additional_frame_id_length_minus_1 = 0;

        if !reduced_still_picture_header {
            frame_id_numbers_present_flag = r.read_bit()?;
            if frame_id_numbers_present_flag {
                delta_frame_id_length_minus_2 = r.read_bits(4)? as u8;
                additional_frame_id_length_minus_1 = r.read_bits(3)? as u8;
            }
        }

        let use_128x128_superblock = r.read_bit()?;
        let enable_filter_intra = r.read_bit()?;
        let enable_intra_edge_filter = r.read_bit()?;

        let mut enable_interintra_compound = false;
        let mut enable_masked_compound = false;
        let mut enable_warped_motion = false;
        let mut enable_dual_filter = false;
        let mut enable_order_hint = false;
        let mut enable_jnt_comp = false;
        let mut enable_ref_frame_mvs = false;
        let mut seq_force_screen_content_tools = 2; // SELECT
        let mut seq_force_integer_mv = 2; // SELECT
        let mut order_hint_bits_minus_1 = 0;

        if !reduced_still_picture_header {
            enable_interintra_compound = r.read_bit()?;
            enable_masked_compound = r.read_bit()?;
            enable_warped_motion = r.read_bit()?;
            enable_dual_filter = r.read_bit()?;
            enable_order_hint = r.read_bit()?;
            if enable_order_hint {
                enable_jnt_comp = r.read_bit()?;
                enable_ref_frame_mvs = r.read_bit()?;
            }

            let seq_choose_screen_content_tools = r.read_bit()?;
            if seq_choose_screen_content_tools {
                seq_force_screen_content_tools = 2;
            } else {
                seq_force_screen_content_tools = r.read_bit()? as u8;
            }

            if seq_force_screen_content_tools > 0 {
                let seq_choose_integer_mv = r.read_bit()?;
                if seq_choose_integer_mv {
                    seq_force_integer_mv = 2;
                } else {
                    seq_force_integer_mv = r.read_bit()? as u8;
                }
            }

            if enable_order_hint {
                order_hint_bits_minus_1 = r.read_bits(3)? as u8;
            }
        }

        let enable_superres = r.read_bit()?;
        let enable_cdef = r.read_bit()?;
        let enable_restoration = r.read_bit()?;

        // Color config (§5.5.2)
        let high_bitdepth = r.read_bit()?;
        let mut twelve_bit = false;
        if seq_profile == 2 && high_bitdepth {
            twelve_bit = r.read_bit()?;
        }

        let mut mono_chrome = false;
        if seq_profile != 1 {
            mono_chrome = r.read_bit()?;
        }

        let color_description_present_flag = r.read_bit()?;
        let mut matrix_coefficients = 2; // MC_UNSPECIFIED
        if color_description_present_flag {
            let _color_primaries = r.read_bits(8)?;
            let _transfer_characteristics = r.read_bits(8)?;
            matrix_coefficients = r.read_bits(8)? as u8;
        }

        let mut subsampling_x = true;
        let mut subsampling_y = true;

        let color_range = if mono_chrome {
            r.read_bit()?
        } else if matrix_coefficients == 0 {
            // Identity (sRGB)
            subsampling_x = false;
            subsampling_y = false;
            true
        } else {
            let cr = r.read_bit()?;
            if seq_profile == 0 {
                subsampling_x = true;
                subsampling_y = true;
            } else if seq_profile == 1 {
                subsampling_x = false;
                subsampling_y = false;
            } else if twelve_bit {
                subsampling_x = r.read_bit()?;
                if subsampling_x {
                    subsampling_y = r.read_bit()?;
                }
            } else {
                subsampling_x = true;
                subsampling_y = false;
            }
            if subsampling_x && subsampling_y {
                let _chroma_sample_position = r.read_bits(2)?;
            }
            cr
        };

        let _separate_uv_delta_q = if !mono_chrome { r.read_bit()? } else { false };
        let film_grain_params_present = r.read_bit()?;

        Ok(Self {
            seq_profile,
            still_picture,
            reduced_still_picture_header,
            frame_width_bits_minus_1: frame_width_bits_minus_1 as u8,
            frame_height_bits_minus_1: frame_height_bits_minus_1 as u8,
            max_frame_width,
            max_frame_height,
            frame_id_numbers_present_flag,
            delta_frame_id_length_minus_2,
            additional_frame_id_length_minus_1,
            use_128x128_superblock,
            enable_filter_intra,
            enable_intra_edge_filter,
            enable_interintra_compound,
            enable_masked_compound,
            enable_warped_motion,
            enable_dual_filter,
            enable_order_hint,
            enable_jnt_comp,
            enable_ref_frame_mvs,
            seq_force_screen_content_tools,
            seq_force_integer_mv,
            order_hint_bits_minus_1,
            enable_superres,
            enable_cdef,
            enable_restoration,
            high_bitdepth,
            twelve_bit,
            mono_chrome,
            color_range,
            subsampling_x,
            subsampling_y,
            separate_uv_delta_q: _separate_uv_delta_q,
            matrix_coefficients,
            film_grain_params_present,
        })
    }

    /// Returns the bit depth in bits (8, 10, or 12).
    pub fn bit_depth(&self) -> u8 {
        if self.twelve_bit {
            12
        } else if self.high_bitdepth {
            10
        } else {
            8
        }
    }
}
