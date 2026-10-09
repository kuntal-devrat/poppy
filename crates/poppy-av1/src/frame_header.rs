//! Frame Header OBU parser (AV1 specification §5.9).

use crate::bitstream::{BitReader, BitstreamError};
use crate::sequence_header::SequenceHeader;

/// AV1 Frame Types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1FrameType {
    KeyFrame = 0,
    InterFrame = 1,
    IntraOnlyFrame = 2,
    SwitchFrame = 3,
}

/// Decoded Picture Buffer tracking state passed for inter-frame reference parsing.
#[derive(Debug, Clone, Copy, Default)]
pub struct DpbState {
    pub order_hints: [u32; 8],
    pub widths: [u32; 8],
    pub heights: [u32; 8],
    pub render_widths: [u32; 8],
    pub render_heights: [u32; 8],
}

/// Parsed AV1 Frame Header parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub show_existing_frame: bool,
    pub frame_to_show_map_idx: u8,
    pub frame_type: Av1FrameType,
    pub show_frame: bool,
    pub showable_frame: bool,
    pub error_resilient_mode: bool,
    pub disable_cdf_update: bool,
    pub allow_screen_content_tools: bool,
    pub force_integer_mv: bool,
    pub current_frame_id: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub superres_scale_denominator: u8,
    pub use_superres: bool,
    pub allow_intrabc: bool,
    pub allow_high_precision_mv: bool,
    pub is_motion_mode_switchable: bool,
    pub use_ref_frame_mvs: bool,
    pub disable_frame_end_update_cdf: bool,
    pub uniform_tile_spacing_flag: bool,
    pub allow_warped_motion: bool,
    pub large_scale_tile: bool,
    pub order_hint: u8,
    pub primary_ref_frame: u8,
    pub refresh_frame_flags: u8,
    pub ref_frame_idx: [u8; 7],
    pub interp_filter: u8,
    pub filter_level: [u8; 2],
    pub filter_level_u: u8,
    pub filter_level_v: u8,
    pub sharpness_level: u8,
    pub mode_ref_delta_enabled: bool,
    pub mode_ref_delta_update: bool,
    pub ref_deltas: [i8; 8],
    pub mode_deltas: [i8; 2],
    pub base_qindex: u8,
    pub y_dc_delta_q: i8,
    pub u_dc_delta_q: i8,
    pub u_ac_delta_q: i8,
    pub v_dc_delta_q: i8,
    pub v_ac_delta_q: i8,
    pub using_qmatrix: bool,
    pub qm_y: u8,
    pub qm_u: u8,
    pub qm_v: u8,
    pub delta_q_present_flag: bool,
    pub log2_delta_q_res: u8,
    pub delta_lf_present_flag: bool,
    pub log2_delta_lf_res: u8,
    pub delta_lf_multi: bool,
    pub segmentation_enabled: bool,
    pub segmentation_update_map: bool,
    pub segmentation_temporal_update: bool,
    pub segmentation_update_data: bool,
    pub feature_enabled: [[bool; 8]; 8],
    pub feature_data: [[i16; 8]; 8],
    pub tx_mode: u8,
    pub reference_select: bool,
    pub reduced_tx_set_used: bool,
    pub skip_mode_present: bool,
    pub cdef_damping_minus_3: u8,
    pub cdef_bits: u8,
    pub cdef_y_strengths: [u8; 8],
    pub cdef_uv_strengths: [u8; 8],
    pub yframe_restoration_type: u8,
    pub cbframe_restoration_type: u8,
    pub crframe_restoration_type: u8,
    pub lr_unit_shift: u8,
    pub lr_uv_shift: u8,
    pub tile_cols: u8,
    pub tile_rows: u8,
    pub context_update_tile_id: u16,
    pub width_in_sbs_minus_1: [u16; 63],
    pub height_in_sbs_minus_1: [u16; 63],
    pub gm_params: [[i32; 6]; 7],
    pub gm_type: [u8; 7],
    pub gm_invalid: [u8; 7],
    pub tile_data_offset: usize,
}

impl FrameHeader {
    /// Parses an AV1 Frame Header OBU or Frame OBU payload with default DPB state.
    pub fn parse(payload: &[u8], seq: &SequenceHeader) -> Result<Self, BitstreamError> {
        let dpb = DpbState {
            widths: [seq.max_frame_width; 8],
            heights: [seq.max_frame_height; 8],
            render_widths: [seq.max_frame_width; 8],
            render_heights: [seq.max_frame_height; 8],
            ..Default::default()
        };
        Self::parse_with_dpb(payload, seq, &dpb)
    }

    /// Parses an AV1 Frame Header OBU or Frame OBU payload with active DPB reference state.
    pub fn parse_with_dpb(
        payload: &[u8],
        seq: &SequenceHeader,
        dpb: &DpbState,
    ) -> Result<Self, BitstreamError> {
        let mut r = BitReader::new(payload);

        let mut show_existing_frame = false;
        let mut frame_to_show_map_idx = 0;

        if !seq.reduced_still_picture_header {
            show_existing_frame = r.read_bit()?;
            if show_existing_frame {
                frame_to_show_map_idx = r.read_bits(3)? as u8;
                return Ok(Self::default_show_existing(frame_to_show_map_idx));
            }
        }

        let frame_type = if seq.reduced_still_picture_header {
            Av1FrameType::KeyFrame
        } else {
            let ft = r.read_bits(2)? as u8;
            match ft {
                0 => Av1FrameType::KeyFrame,
                1 => Av1FrameType::InterFrame,
                2 => Av1FrameType::IntraOnlyFrame,
                _ => Av1FrameType::SwitchFrame,
            }
        };

        let frame_is_intra =
            frame_type == Av1FrameType::KeyFrame || frame_type == Av1FrameType::IntraOnlyFrame;

        let show_frame = if seq.reduced_still_picture_header {
            true
        } else {
            r.read_bit()?
        };

        let mut showable_frame = false;
        if !show_frame {
            showable_frame = r.read_bit()?;
        }

        let error_resilient_mode = if frame_type == Av1FrameType::SwitchFrame
            || (frame_type == Av1FrameType::KeyFrame && show_frame)
        {
            true
        } else {
            r.read_bit()?
        };

        let disable_cdf_update = r.read_bit()?;

        let mut allow_screen_content_tools = seq.seq_force_screen_content_tools == 1;
        if seq.seq_force_screen_content_tools == 2 {
            allow_screen_content_tools = r.read_bit()?;
        }

        let mut force_integer_mv = false;
        if allow_screen_content_tools {
            if seq.seq_force_integer_mv == 2 {
                force_integer_mv = r.read_bit()?;
            } else {
                force_integer_mv = seq.seq_force_integer_mv == 1;
            }
        }

        let mut current_frame_id = 0;
        if seq.frame_id_numbers_present_flag {
            let id_len = (seq.delta_frame_id_length_minus_2
                + seq.additional_frame_id_length_minus_1
                + 3) as usize;
            current_frame_id = r.read_bits(id_len)?;
        }

        let mut frame_size_override_flag = false;
        if frame_type == Av1FrameType::SwitchFrame {
            frame_size_override_flag = true;
        } else if !seq.reduced_still_picture_header {
            frame_size_override_flag = r.read_bit()?;
        }

        let mut order_hint = 0;
        let order_hint_bits = (seq.order_hint_bits_minus_1 + 1) as usize;
        if seq.enable_order_hint {
            order_hint = r.read_bits(order_hint_bits)? as u8;
        }

        let mut primary_ref_frame = 7; // PRIMARY_REF_NONE
        if !frame_is_intra {
            primary_ref_frame = r.read_bits(3)? as u8;
        }

        let mut refresh_frame_flags = 0xFF;
        if !(frame_type == Av1FrameType::SwitchFrame
            || (frame_type == Av1FrameType::KeyFrame && show_frame))
        {
            refresh_frame_flags = r.read_bits(8)? as u8;
        }

        if (!frame_is_intra || refresh_frame_flags != 0xFF)
            && (error_resilient_mode && seq.enable_order_hint)
        {
            for _ in 0..8 {
                let _ = r.read_bits(order_hint_bits)?;
            }
        }

        let frame_width;
        let frame_height;
        let render_width;
        let render_height;
        let mut superres_scale_denominator = 8;
        let mut use_superres = false;
        let mut allow_intrabc = false;
        let mut ref_frame_idx = [0u8; 7];
        let mut allow_high_precision_mv = false;
        let mut interp_filter = 0;
        let mut is_motion_mode_switchable = false;
        let mut use_ref_frame_mvs = false;

        if frame_is_intra {
            // frame_size()
            if frame_size_override_flag {
                let n_w = seq.frame_width_bits_minus_1 as usize + 1;
                let n_h = seq.frame_height_bits_minus_1 as usize + 1;
                frame_width = r.read_bits(n_w)? + 1;
                frame_height = r.read_bits(n_h)? + 1;
            } else {
                frame_width = seq.max_frame_width;
                frame_height = seq.max_frame_height;
            }

            if seq.enable_superres {
                use_superres = r.read_bit()?;
                if use_superres {
                    superres_scale_denominator = (r.read_bits(3)? + 9) as u8;
                }
            }

            // render_size()
            if r.read_bit()? {
                render_width = r.read_bits(16)? + 1;
                render_height = r.read_bits(16)? + 1;
            } else {
                render_width = frame_width;
                render_height = frame_height;
            }

            if allow_screen_content_tools && frame_width == seq.max_frame_width {
                allow_intrabc = r.read_bit()?;
            }
        } else {
            // Inter frame reference signaling
            let mut frame_refs_short_signaling = false;
            if seq.enable_order_hint {
                frame_refs_short_signaling = r.read_bit()?;
                if frame_refs_short_signaling {
                    let last_frame_idx = r.read_bits(3)? as u8;
                    let gold_frame_idx = r.read_bits(3)? as u8;
                    ref_frame_idx = set_frame_refs(
                        last_frame_idx,
                        gold_frame_idx,
                        order_hint,
                        seq.order_hint_bits_minus_1 + 1,
                        dpb,
                    );
                }
            }

            if !frame_refs_short_signaling {
                for slot in &mut ref_frame_idx {
                    *slot = r.read_bits(3)? as u8;
                }
            }

            // Dimensions: frame_size_with_refs or explicit frame_size
            if frame_size_override_flag && !error_resilient_mode {
                let mut found_ref = false;
                let mut ref_matched = 0;
                for &slot in &ref_frame_idx {
                    if r.read_bit()? {
                        found_ref = true;
                        ref_matched = slot as usize;
                        break;
                    }
                }
                if !found_ref {
                    let n_w = seq.frame_width_bits_minus_1 as usize + 1;
                    let n_h = seq.frame_height_bits_minus_1 as usize + 1;
                    frame_width = r.read_bits(n_w)? + 1;
                    frame_height = r.read_bits(n_h)? + 1;
                    if seq.enable_superres {
                        use_superres = r.read_bit()?;
                        if use_superres {
                            superres_scale_denominator = (r.read_bits(3)? + 9) as u8;
                        }
                    }
                    if r.read_bit()? {
                        render_width = r.read_bits(16)? + 1;
                        render_height = r.read_bits(16)? + 1;
                    } else {
                        render_width = frame_width;
                        render_height = frame_height;
                    }
                } else {
                    let ref_w = dpb.widths[ref_matched];
                    let ref_h = dpb.heights[ref_matched];
                    frame_width = if ref_w > 0 {
                        ref_w
                    } else {
                        seq.max_frame_width
                    };
                    frame_height = if ref_h > 0 {
                        ref_h
                    } else {
                        seq.max_frame_height
                    };
                    let ren_w = dpb.render_widths[ref_matched];
                    let ren_h = dpb.render_heights[ref_matched];
                    render_width = if ren_w > 0 { ren_w } else { frame_width };
                    render_height = if ren_h > 0 { ren_h } else { frame_height };
                    if seq.enable_superres {
                        use_superres = r.read_bit()?;
                        if use_superres {
                            superres_scale_denominator = (r.read_bits(3)? + 9) as u8;
                        }
                    }
                }
            } else {
                if frame_size_override_flag {
                    let n_w = seq.frame_width_bits_minus_1 as usize + 1;
                    let n_h = seq.frame_height_bits_minus_1 as usize + 1;
                    frame_width = r.read_bits(n_w)? + 1;
                    frame_height = r.read_bits(n_h)? + 1;
                } else {
                    frame_width = seq.max_frame_width;
                    frame_height = seq.max_frame_height;
                }
                if seq.enable_superres {
                    use_superres = r.read_bit()?;
                    if use_superres {
                        superres_scale_denominator = (r.read_bits(3)? + 9) as u8;
                    }
                }
                if r.read_bit()? {
                    render_width = r.read_bits(16)? + 1;
                    render_height = r.read_bits(16)? + 1;
                } else {
                    render_width = frame_width;
                    render_height = frame_height;
                }
            }

            if force_integer_mv {
                allow_high_precision_mv = false;
            } else {
                allow_high_precision_mv = r.read_bit()?;
            }

            let is_filter_switchable = r.read_bit()?;
            interp_filter = if is_filter_switchable {
                4
            } else {
                r.read_bits(2)? as u8
            };

            is_motion_mode_switchable = r.read_bit()?;

            if error_resilient_mode || !seq.enable_ref_frame_mvs {
                use_ref_frame_mvs = false;
            } else {
                use_ref_frame_mvs = r.read_bit()?;
            }
        }

        let disable_frame_end_update_cdf = if seq.reduced_still_picture_header || disable_cdf_update
        {
            true
        } else {
            r.read_bit()?
        };

        // Tile info (§5.9.15)
        let mi_cols = 2 * ((frame_width + 7) >> 3);
        let mi_rows = 2 * ((frame_height + 7) >> 3);
        let sb_shift = if seq.use_128x128_superblock { 5 } else { 4 };
        let sb_size = sb_shift + 2;
        let sb_cols = if seq.use_128x128_superblock {
            (mi_cols + 31) >> 5
        } else {
            (mi_cols + 15) >> 4
        };
        let sb_rows = if seq.use_128x128_superblock {
            (mi_rows + 31) >> 5
        } else {
            (mi_rows + 15) >> 4
        };

        let max_tile_width_sb = 4096 >> sb_size;
        let max_tile_area_sb = (4096 * 2304) >> (2 * sb_size);
        let min_log2_tile_cols = tile_log2(max_tile_width_sb, sb_cols);
        let max_log2_tile_cols = tile_log2(1, sb_cols.min(64));
        let max_log2_tile_rows = tile_log2(1, sb_rows.min(64));
        let min_log2_tiles = min_log2_tile_cols.max(tile_log2(max_tile_area_sb, sb_rows * sb_cols));

        let uniform_tile_spacing_flag = r.read_bit()?;
        let mut width_in_sbs_minus_1 = [0u16; 63];
        let mut height_in_sbs_minus_1 = [0u16; 63];
        let tile_cols_log2;
        let tile_rows_log2;
        let tile_cols;
        let tile_rows;

        if uniform_tile_spacing_flag {
            let mut tc_log2 = min_log2_tile_cols;
            while tc_log2 < max_log2_tile_cols {
                if r.read_bit()? {
                    tc_log2 += 1;
                } else {
                    break;
                }
            }
            tile_cols_log2 = tc_log2;
            let tile_width_sb = (sb_cols + (1 << tile_cols_log2) - 1) >> tile_cols_log2;
            let mut i = 0;
            let mut start_sb = 0;
            while start_sb < sb_cols {
                let size = tile_width_sb.min(sb_cols - start_sb);
                if i < 63 {
                    width_in_sbs_minus_1[i] = (size - 1) as u16;
                }
                start_sb += tile_width_sb;
                i += 1;
            }
            tile_cols = i as u8;

            let min_log2_tile_rows = min_log2_tiles.saturating_sub(tile_cols_log2);
            let mut tr_log2 = min_log2_tile_rows;
            while tr_log2 < max_log2_tile_rows {
                if r.read_bit()? {
                    tr_log2 += 1;
                } else {
                    break;
                }
            }
            tile_rows_log2 = tr_log2;
            let tile_height_sb = (sb_rows + (1 << tile_rows_log2) - 1) >> tile_rows_log2;
            let mut j = 0;
            let mut start_sb = 0;
            while start_sb < sb_rows {
                let size = tile_height_sb.min(sb_rows - start_sb);
                if j < 63 {
                    height_in_sbs_minus_1[j] = (size - 1) as u16;
                }
                start_sb += tile_height_sb;
                j += 1;
            }
            tile_rows = j as u8;
        } else {
            let mut widest_tile_sb = 0;
            let mut start_sb = 0;
            let mut i = 0;
            while start_sb < sb_cols && i < 63 {
                let max_w = (sb_cols - start_sb).min(max_tile_width_sb);
                let size_sb = r.read_ns(max_w)? + 1;
                width_in_sbs_minus_1[i] = (size_sb - 1) as u16;
                widest_tile_sb = widest_tile_sb.max(size_sb);
                start_sb += size_sb;
                i += 1;
            }
            tile_cols = i as u8;
            tile_cols_log2 = tile_log2(1, tile_cols as u32);

            let max_tile_area_sb_non_uniform = if min_log2_tiles > 0 {
                (sb_rows * sb_cols) >> (min_log2_tiles + 1)
            } else {
                sb_rows * sb_cols
            };
            let max_tile_height_sb = (max_tile_area_sb_non_uniform / widest_tile_sb.max(1)).max(1);
            let mut start_sb = 0;
            let mut j = 0;
            while start_sb < sb_rows && j < 63 {
                let max_h = (sb_rows - start_sb).min(max_tile_height_sb);
                let size_sb = r.read_ns(max_h)? + 1;
                height_in_sbs_minus_1[j] = (size_sb - 1) as u16;
                start_sb += size_sb;
                j += 1;
            }
            tile_rows = j as u8;
            tile_rows_log2 = tile_log2(1, tile_rows as u32);
        }

        let context_update_tile_id = if tile_cols_log2 > 0 || tile_rows_log2 > 0 {
            let id = r.read_bits((tile_rows_log2 + tile_cols_log2) as usize)? as u16;
            let _tile_size_bytes_minus_1 = r.read_bits(2)?;
            id
        } else {
            0
        };

        // Quantization params (§5.9.12)
        let base_qindex = r.read_bits(8)? as u8;
        let y_dc_delta_q = read_delta_q(&mut r)?;
        let mut u_dc_delta_q = 0;
        let mut u_ac_delta_q = 0;
        let mut v_dc_delta_q = 0;
        let mut v_ac_delta_q = 0;

        let num_planes = if seq.mono_chrome { 1 } else { 3 };
        if num_planes > 1 {
            let diff_uv_delta = if seq.separate_uv_delta_q {
                r.read_bit()?
            } else {
                false
            };
            u_dc_delta_q = read_delta_q(&mut r)?;
            u_ac_delta_q = read_delta_q(&mut r)?;
            if diff_uv_delta {
                v_dc_delta_q = read_delta_q(&mut r)?;
                v_ac_delta_q = read_delta_q(&mut r)?;
            } else {
                v_dc_delta_q = u_dc_delta_q;
                v_ac_delta_q = u_ac_delta_q;
            }
        }

        let using_qmatrix = r.read_bit()?;
        let mut qm_y = 0;
        let mut qm_u = 0;
        let mut qm_v = 0;
        if using_qmatrix {
            qm_y = r.read_bits(4)? as u8;
            qm_u = r.read_bits(4)? as u8;
            if !seq.separate_uv_delta_q {
                qm_v = qm_u;
            } else {
                qm_v = r.read_bits(4)? as u8;
            }
        }

        // Segmentation params (§5.9.14)
        let segmentation_enabled = r.read_bit()?;
        let mut segmentation_update_map = false;
        let mut segmentation_temporal_update = false;
        let mut segmentation_update_data = false;
        let mut feature_enabled = [[false; 8]; 8];
        let mut feature_data = [[0i16; 8]; 8];

        if segmentation_enabled {
            if primary_ref_frame == 7 {
                segmentation_update_map = true;
                segmentation_temporal_update = false;
                segmentation_update_data = true;
            } else {
                segmentation_update_map = r.read_bit()?;
                if segmentation_update_map {
                    segmentation_temporal_update = r.read_bit()?;
                }
                segmentation_update_data = r.read_bit()?;
            }
            if segmentation_update_data {
                for i in 0..8 {
                    for j in 0..8 {
                        let feat_enabled = r.read_bit()?;
                        feature_enabled[i][j] = feat_enabled;
                        if feat_enabled {
                            let bits_to_read = [8, 6, 6, 6, 6, 3, 0, 0][j];
                            let limit = [255, 63, 63, 63, 63, 7, 0, 0][j];
                            let signed = [true, true, true, true, true, false, false, false][j];
                            let val = if bits_to_read > 0 {
                                if signed {
                                    r.read_su(1 + bits_to_read)?
                                } else {
                                    r.read_bits(bits_to_read)? as i32
                                }
                            } else {
                                0
                            };
                            let clipped = val.clamp(if signed { -limit } else { 0 }, limit);
                            feature_data[i][j] = clipped as i16;
                        }
                    }
                }
            }
        }

        // Delta Q & Delta LF (§5.9.17 & §5.9.18)
        let mut delta_q_present_flag = false;
        let mut log2_delta_q_res = 0;
        if base_qindex > 0 {
            delta_q_present_flag = r.read_bit()?;
        }
        if delta_q_present_flag {
            log2_delta_q_res = r.read_bits(2)? as u8;
        }

        let mut delta_lf_present_flag = false;
        let mut log2_delta_lf_res = 0;
        let mut delta_lf_multi = false;
        if delta_q_present_flag {
            if !allow_intrabc {
                delta_lf_present_flag = r.read_bit()?;
            }
            if delta_lf_present_flag {
                log2_delta_lf_res = r.read_bits(2)? as u8;
                delta_lf_multi = r.read_bit()?;
            }
        }

        // Loop filter (§5.9.11)
        let mut filter_level = [0u8; 2];
        let mut filter_level_u = 0;
        let mut filter_level_v = 0;
        let mut sharpness_level = 0;
        let mut mode_ref_delta_enabled = false;
        let mut mode_ref_delta_update = false;
        let mut ref_deltas = [1i8, 0, 0, 0, -1, 0, -1, -1];
        let mut mode_deltas = [0i8; 2];

        if !allow_intrabc {
            filter_level[0] = r.read_bits(6)? as u8;
            filter_level[1] = r.read_bits(6)? as u8;
            if num_planes > 1 && (filter_level[0] > 0 || filter_level[1] > 0) {
                filter_level_u = r.read_bits(6)? as u8;
                filter_level_v = r.read_bits(6)? as u8;
            }
            sharpness_level = r.read_bits(3)? as u8;
            mode_ref_delta_enabled = r.read_bit()?;
            if mode_ref_delta_enabled {
                mode_ref_delta_update = r.read_bit()?;
                if mode_ref_delta_update {
                    for rd in &mut ref_deltas {
                        if r.read_bit()? {
                            *rd = r.read_su(7)? as i8;
                        }
                    }
                    for md in &mut mode_deltas {
                        if r.read_bit()? {
                            *md = r.read_su(7)? as i8;
                        }
                    }
                }
            }
        }

        // CDEF parameters (§5.9.19)
        let mut cdef_damping_minus_3 = 0;
        let mut cdef_bits = 0;
        let mut cdef_y_strengths = [0u8; 8];
        let mut cdef_uv_strengths = [0u8; 8];

        if seq.enable_cdef && !allow_intrabc {
            cdef_damping_minus_3 = r.read_bits(2)? as u8;
            cdef_bits = r.read_bits(2)? as u8;
            let num_cdef = 1 << cdef_bits;
            for i in 0..num_cdef {
                let pri = r.read_bits(4)? as u8;
                let mut sec = r.read_bits(2)? as u8;
                if sec == 3 {
                    sec += 1;
                }
                cdef_y_strengths[i] = (pri << 2) | (sec & 0x03);
                if num_planes > 1 {
                    let pri_uv = r.read_bits(4)? as u8;
                    let mut sec_uv = r.read_bits(2)? as u8;
                    if sec_uv == 3 {
                        sec_uv += 1;
                    }
                    cdef_uv_strengths[i] = (pri_uv << 2) | (sec_uv & 0x03);
                }
            }
        }

        // Loop restoration (§5.9.20)
        let mut yframe_restoration_type = 0;
        let mut cbframe_restoration_type = 0;
        let mut crframe_restoration_type = 0;
        let mut lr_unit_shift = 0;
        let mut lr_uv_shift = 0;

        if seq.enable_restoration && !allow_intrabc {
            let mut uses_lr = false;
            let mut uses_chroma_lr = false;
            for i in 0..num_planes {
                let lr_type = r.read_bits(2)? as u8;
                if i == 0 {
                    yframe_restoration_type = lr_type;
                } else if i == 1 {
                    cbframe_restoration_type = lr_type;
                } else {
                    crframe_restoration_type = lr_type;
                }
                if lr_type > 0 {
                    uses_lr = true;
                    if i > 0 {
                        uses_chroma_lr = true;
                    }
                }
            }
            if uses_lr {
                if seq.use_128x128_superblock {
                    lr_unit_shift = (r.read_bits(1)? + 1) as u8;
                } else {
                    lr_unit_shift = r.read_bits(1)? as u8;
                    if lr_unit_shift > 0 {
                        let lr_unit_extra_shift = r.read_bits(1)? as u8;
                        lr_unit_shift += lr_unit_extra_shift;
                    }
                }
                if seq.subsampling_x && seq.subsampling_y && uses_chroma_lr {
                    lr_uv_shift = r.read_bits(1)? as u8;
                } else {
                    lr_uv_shift = 0;
                }
            }
        }

        // TX Mode (§5.9.21)
        let tx_mode = if !allow_intrabc {
            let tx_mode_select = r.read_bit()?;
            if tx_mode_select { 2 } else { 1 }
        } else {
            0
        };

        // Frame Reference Mode (§5.9.23)
        let reference_select = if !frame_is_intra {
            r.read_bit()?
        } else {
            false
        };

        // Skip Mode Params (§5.9.22)
        let skip_mode_allowed = check_skip_mode_allowed(
            frame_is_intra,
            reference_select,
            seq.enable_order_hint,
            order_hint,
            seq.order_hint_bits_minus_1 + 1,
            &ref_frame_idx,
            dpb,
        );
        let skip_mode_present = if skip_mode_allowed {
            r.read_bit()?
        } else {
            false
        };

        let allow_warped_motion =
            if frame_is_intra || error_resilient_mode || !seq.enable_warped_motion {
                false
            } else {
                r.read_bit()?
            };

        let reduced_tx_set_used = r.read_bit()?;

        // Global motion parameters (§5.9.24)
        let mut gm_params = [[0i32; 6]; 7];
        let mut gm_type = [0u8; 7];
        let mut gm_invalid = [0u8; 7];
        for gm in &mut gm_params {
            gm[0] = 0;
            gm[1] = 0;
            gm[2] = 1 << 16;
            gm[3] = 0;
            gm[4] = 0;
            gm[5] = 1 << 16;
        }

        if !frame_is_intra {
            for i in 0..7 {
                let is_global = r.read_bit()?;
                if is_global {
                    let is_rot_zoom = r.read_bit()?;
                    let is_translation = if !is_rot_zoom { r.read_bit()? } else { false };
                    let warp_t = if is_rot_zoom {
                        2
                    } else if is_translation {
                        1
                    } else {
                        3
                    };
                    gm_type[i] = warp_t;
                    if warp_t >= 2 {
                        gm_params[i][2] = read_global_param(
                            &mut r,
                            warp_t,
                            2,
                            allow_high_precision_mv,
                            gm_params[i][2],
                        )?;
                        gm_params[i][3] = read_global_param(
                            &mut r,
                            warp_t,
                            3,
                            allow_high_precision_mv,
                            gm_params[i][3],
                        )?;
                        if warp_t == 3 {
                            gm_params[i][4] = read_global_param(
                                &mut r,
                                warp_t,
                                4,
                                allow_high_precision_mv,
                                gm_params[i][4],
                            )?;
                            gm_params[i][5] = read_global_param(
                                &mut r,
                                warp_t,
                                5,
                                allow_high_precision_mv,
                                gm_params[i][5],
                            )?;
                        } else {
                            gm_params[i][4] = -gm_params[i][3];
                            gm_params[i][5] = gm_params[i][2];
                        }
                    }
                    if warp_t >= 1 {
                        gm_params[i][0] = read_global_param(
                            &mut r,
                            warp_t,
                            0,
                            allow_high_precision_mv,
                            gm_params[i][0],
                        )?;
                        gm_params[i][1] = read_global_param(
                            &mut r,
                            warp_t,
                            1,
                            allow_high_precision_mv,
                            gm_params[i][1],
                        )?;
                    }
                } else {
                    gm_type[i] = 0;
                    gm_invalid[i] = 1;
                }
            }
        }

        // Film grain parameters (§5.9.25 / §5.9.30)
        if seq.film_grain_params_present && (show_frame || showable_frame) {
            let apply_grain = r.read_bit()?;
            if apply_grain {
                let _grain_seed = r.read_bits(16)?;
                let update_grain = if frame_type == Av1FrameType::InterFrame {
                    r.read_bit()?
                } else {
                    true
                };
                if !update_grain {
                    let _film_grain_params_ref_idx = r.read_bits(3)?;
                } else {
                    let num_y_points = r.read_bits(4)? as usize;
                    for _ in 0..num_y_points {
                        let _ = r.read_bits(8)?;
                        let _ = r.read_bits(8)?;
                    }
                    let chroma_scaling_from_luma = if seq.mono_chrome {
                        false
                    } else {
                        r.read_bit()?
                    };
                    let (num_cb_points, num_cr_points) = if seq.mono_chrome
                        || chroma_scaling_from_luma
                        || (seq.subsampling_x && seq.subsampling_y && num_y_points == 0)
                    {
                        (0, 0)
                    } else {
                        (r.read_bits(4)? as usize, r.read_bits(4)? as usize)
                    };
                    for _ in 0..num_cb_points {
                        let _ = r.read_bits(8)?;
                        let _ = r.read_bits(8)?;
                    }
                    for _ in 0..num_cr_points {
                        let _ = r.read_bits(8)?;
                        let _ = r.read_bits(8)?;
                    }
                    let _grain_scaling_minus_8 = r.read_bits(2)?;
                    let ar_coeff_lag = r.read_bits(2)? as usize;
                    let num_pos_luma = 2 * ar_coeff_lag * (ar_coeff_lag + 1);
                    let num_pos_chroma = if num_cb_points > 0 || num_cr_points > 0 {
                        num_pos_luma + 1
                    } else {
                        num_pos_luma
                    };
                    if num_y_points > 0 {
                        for _ in 0..num_pos_luma {
                            let _ = r.read_bits(8)?;
                        }
                    }
                    if chroma_scaling_from_luma || num_cb_points > 0 {
                        for _ in 0..num_pos_chroma {
                            let _ = r.read_bits(8)?;
                        }
                    }
                    if chroma_scaling_from_luma || num_cr_points > 0 {
                        for _ in 0..num_pos_chroma {
                            let _ = r.read_bits(8)?;
                        }
                    }
                    let _ar_coeff_shift_minus_6 = r.read_bits(2)?;
                    let _grain_scale_shift = r.read_bits(2)?;
                    if num_cb_points > 0 {
                        let _cb_mult = r.read_bits(8)?;
                        let _cb_luma_mult = r.read_bits(8)?;
                        let _cb_offset = r.read_bits(9)?;
                    }
                    if num_cr_points > 0 {
                        let _cr_mult = r.read_bits(8)?;
                        let _cr_luma_mult = r.read_bits(8)?;
                        let _cr_offset = r.read_bits(9)?;
                    }
                    let _overlap_flag = r.read_bit()?;
                    let _clip_to_restricted_range = r.read_bit()?;
                }
            }
        }

        r.byte_align();
        let tile_data_offset = r.bits_read() / 8;

        Ok(Self {
            show_existing_frame,
            frame_to_show_map_idx,
            frame_type,
            show_frame,
            showable_frame,
            error_resilient_mode,
            disable_cdf_update,
            allow_screen_content_tools,
            force_integer_mv,
            current_frame_id,
            frame_width,
            frame_height,
            render_width,
            render_height,
            superres_scale_denominator,
            use_superres,
            allow_intrabc,
            allow_high_precision_mv,
            is_motion_mode_switchable,
            use_ref_frame_mvs,
            disable_frame_end_update_cdf,
            uniform_tile_spacing_flag,
            allow_warped_motion,
            large_scale_tile: false,
            order_hint,
            primary_ref_frame,
            refresh_frame_flags,
            ref_frame_idx,
            interp_filter,
            filter_level,
            filter_level_u,
            filter_level_v,
            sharpness_level,
            mode_ref_delta_enabled,
            mode_ref_delta_update,
            ref_deltas,
            mode_deltas,
            base_qindex,
            y_dc_delta_q,
            u_dc_delta_q,
            u_ac_delta_q,
            v_dc_delta_q,
            v_ac_delta_q,
            using_qmatrix,
            qm_y,
            qm_u,
            qm_v,
            delta_q_present_flag,
            log2_delta_q_res,
            delta_lf_present_flag,
            log2_delta_lf_res,
            delta_lf_multi,
            segmentation_enabled,
            segmentation_update_map,
            segmentation_temporal_update,
            segmentation_update_data,
            feature_enabled,
            feature_data,
            tx_mode,
            reference_select,
            reduced_tx_set_used,
            skip_mode_present,
            cdef_damping_minus_3,
            cdef_bits,
            cdef_y_strengths,
            cdef_uv_strengths,
            yframe_restoration_type,
            cbframe_restoration_type,
            crframe_restoration_type,
            lr_unit_shift,
            lr_uv_shift,
            tile_cols,
            tile_rows,
            context_update_tile_id,
            width_in_sbs_minus_1,
            height_in_sbs_minus_1,
            gm_params,
            gm_type,
            gm_invalid,
            tile_data_offset,
        })
    }

    fn default_show_existing(idx: u8) -> Self {
        Self {
            show_existing_frame: true,
            frame_to_show_map_idx: idx,
            frame_type: Av1FrameType::InterFrame,
            show_frame: true,
            showable_frame: false,
            error_resilient_mode: false,
            disable_cdf_update: false,
            allow_screen_content_tools: false,
            force_integer_mv: false,
            current_frame_id: 0,
            frame_width: 0,
            frame_height: 0,
            render_width: 0,
            render_height: 0,
            superres_scale_denominator: 8,
            use_superres: false,
            allow_intrabc: false,
            allow_high_precision_mv: false,
            is_motion_mode_switchable: false,
            use_ref_frame_mvs: false,
            disable_frame_end_update_cdf: false,
            uniform_tile_spacing_flag: true,
            allow_warped_motion: false,
            large_scale_tile: false,
            order_hint: 0,
            primary_ref_frame: 7,
            refresh_frame_flags: 0,
            ref_frame_idx: [0; 7],
            interp_filter: 0,
            filter_level: [0; 2],
            filter_level_u: 0,
            filter_level_v: 0,
            sharpness_level: 0,
            mode_ref_delta_enabled: false,
            mode_ref_delta_update: false,
            ref_deltas: [0; 8],
            mode_deltas: [0; 2],
            base_qindex: 0,
            y_dc_delta_q: 0,
            u_dc_delta_q: 0,
            u_ac_delta_q: 0,
            v_dc_delta_q: 0,
            v_ac_delta_q: 0,
            using_qmatrix: false,
            qm_y: 0,
            qm_u: 0,
            qm_v: 0,
            delta_q_present_flag: false,
            log2_delta_q_res: 0,
            delta_lf_present_flag: false,
            log2_delta_lf_res: 0,
            delta_lf_multi: false,
            segmentation_enabled: false,
            segmentation_update_map: false,
            segmentation_temporal_update: false,
            segmentation_update_data: false,
            feature_enabled: [[false; 8]; 8],
            feature_data: [[0; 8]; 8],
            tx_mode: 0,
            reference_select: false,
            reduced_tx_set_used: false,
            skip_mode_present: false,
            cdef_damping_minus_3: 0,
            cdef_bits: 0,
            cdef_y_strengths: [0; 8],
            cdef_uv_strengths: [0; 8],
            yframe_restoration_type: 0,
            cbframe_restoration_type: 0,
            crframe_restoration_type: 0,
            lr_unit_shift: 0,
            lr_uv_shift: 0,
            tile_cols: 1,
            tile_rows: 1,
            context_update_tile_id: 0,
            width_in_sbs_minus_1: [0; 63],
            height_in_sbs_minus_1: [0; 63],
            gm_params: [[0; 6]; 7],
            gm_type: [0; 7],
            gm_invalid: [0; 7],
            tile_data_offset: 0,
        }
    }
}

fn read_delta_q(r: &mut BitReader) -> Result<i8, BitstreamError> {
    let delta_coded = r.read_bit()?;
    if delta_coded {
        Ok(r.read_su(7)? as i8)
    } else {
        Ok(0)
    }
}

fn tile_log2(blk_size: u32, target: u32) -> u32 {
    let mut k = 0;
    while (blk_size << k) < target {
        k += 1;
    }
    k
}

fn get_relative_dist(a: u32, b: u32, enable_order_hint: bool, order_hint_bits: u8) -> i32 {
    if !enable_order_hint {
        return 0;
    }
    let diff = (a as i32) - (b as i32);
    let m = 1 << (order_hint_bits - 1);
    (diff & (m - 1)) - (diff & m)
}

fn set_frame_refs(
    last_frame_idx: u8,
    gold_frame_idx: u8,
    order_hint: u8,
    order_hint_bits: u8,
    dpb: &DpbState,
) -> [u8; 7] {
    let mut ref_frame_idx = [0u8; 7];
    let mut used_frame = [false; 8];
    ref_frame_idx[0] = last_frame_idx; // LAST_FRAME
    ref_frame_idx[3] = gold_frame_idx; // GOLDEN_FRAME (index 3 corresponds to GOLDEN_FRAME - LAST_FRAME = 4 - 1 = 3)
    used_frame[last_frame_idx as usize] = true;
    used_frame[gold_frame_idx as usize] = true;

    let cur_frame_hint = 1i32 << (order_hint_bits - 1);
    let mut shifted_order_hints = [0i32; 8];
    for (i, hint) in shifted_order_hints.iter_mut().enumerate() {
        *hint = cur_frame_hint
            + get_relative_dist(dpb.order_hints[i], order_hint as u32, true, order_hint_bits);
    }

    // find_latest_backward
    let find_latest_backward = |used: &[bool; 8]| -> Option<usize> {
        let mut r = None;
        let mut latest_order_hint = i32::MIN;
        for (i, &hint) in shifted_order_hints.iter().enumerate() {
            if !used[i] && hint >= cur_frame_hint && (r.is_none() || hint >= latest_order_hint) {
                r = Some(i);
                latest_order_hint = hint;
            }
        }
        r
    };

    // find_earliest_backward
    let find_earliest_backward = |used: &[bool; 8]| -> Option<usize> {
        let mut r = None;
        let mut earliest_order_hint = i32::MAX;
        for (i, &hint) in shifted_order_hints.iter().enumerate() {
            if !used[i] && hint >= cur_frame_hint && (r.is_none() || hint < earliest_order_hint) {
                r = Some(i);
                earliest_order_hint = hint;
            }
        }
        r
    };

    // ALTREF_FRAME (index 6 = 7 - 1)
    if let Some(rf) = find_latest_backward(&used_frame) {
        ref_frame_idx[6] = rf as u8;
        used_frame[rf] = true;
    }

    // BWDREF_FRAME (index 4 = 5 - 1)
    if let Some(rf) = find_earliest_backward(&used_frame) {
        ref_frame_idx[4] = rf as u8;
        used_frame[rf] = true;
    }

    // ALTREF2_FRAME (index 5 = 6 - 1)
    if let Some(rf) = find_earliest_backward(&used_frame) {
        ref_frame_idx[5] = rf as u8;
        used_frame[rf] = true;
    }

    // find_latest_forward
    let find_latest_forward = |used: &[bool; 8]| -> Option<usize> {
        let mut r = None;
        let mut latest_order_hint = i32::MIN;
        for (i, &hint) in shifted_order_hints.iter().enumerate() {
            if !used[i] && hint < cur_frame_hint && (r.is_none() || hint >= latest_order_hint) {
                r = Some(i);
                latest_order_hint = hint;
            }
        }
        r
    };

    // Remaining references: LAST2 (1), LAST3 (2), BWDREF (4), ALTREF2 (5), ALTREF (6)
    let ref_list = [1, 2, 4, 5, 6];
    for &slot in &ref_list {
        #[allow(clippy::collapsible_if)]
        if !used_frame[ref_frame_idx[slot] as usize] {
            if let Some(rf) = find_latest_forward(&used_frame) {
                ref_frame_idx[slot] = rf as u8;
                used_frame[rf] = true;
            }
        }
    }

    // Earliest forward for any remaining unassigned
    let mut earliest_ref = 0;
    let mut earliest_order_hint = i32::MAX;
    for (i, &hint) in shifted_order_hints.iter().enumerate() {
        if hint < earliest_order_hint {
            earliest_ref = i;
            earliest_order_hint = hint;
        }
    }
    for slot in &mut ref_frame_idx {
        if *slot >= 8 {
            *slot = earliest_ref as u8;
        }
    }

    ref_frame_idx
}

fn check_skip_mode_allowed(
    frame_is_intra: bool,
    reference_select: bool,
    enable_order_hint: bool,
    order_hint: u8,
    order_hint_bits: u8,
    ref_frame_idx: &[u8; 7],
    dpb: &DpbState,
) -> bool {
    if frame_is_intra || !reference_select || !enable_order_hint {
        return false;
    }
    let mut forward_idx = -1i32;
    let mut forward_hint = 0u32;
    let mut backward_idx = -1i32;
    let mut backward_hint = 0u32;

    for (i, &slot) in ref_frame_idx.iter().enumerate() {
        let ref_idx = slot as usize;
        let ref_hint = dpb.order_hints[ref_idx];
        let d = get_relative_dist(ref_hint, order_hint as u32, true, order_hint_bits);
        if d < 0 {
            if forward_idx < 0
                || get_relative_dist(ref_hint, forward_hint, true, order_hint_bits) > 0
            {
                forward_idx = i as i32;
                forward_hint = ref_hint;
            }
        } else if d > 0
            && (backward_idx < 0
                || get_relative_dist(ref_hint, backward_hint, true, order_hint_bits) < 0)
        {
            backward_idx = i as i32;
            backward_hint = ref_hint;
        }
    }

    if forward_idx < 0 {
        false
    } else if backward_idx >= 0 {
        true
    } else {
        let mut second_forward_idx = -1i32;
        let mut second_forward_hint = 0u32;
        for (i, &slot) in ref_frame_idx.iter().enumerate() {
            let ref_idx = slot as usize;
            let ref_hint = dpb.order_hints[ref_idx];
            if get_relative_dist(ref_hint, forward_hint, true, order_hint_bits) < 0
                && (second_forward_idx < 0
                    || get_relative_dist(ref_hint, second_forward_hint, true, order_hint_bits) > 0)
            {
                second_forward_idx = i as i32;
                second_forward_hint = ref_hint;
            }
        }
        second_forward_idx >= 0
    }
}

fn read_global_param(
    r: &mut BitReader,
    warp_type: u8,
    idx: usize,
    allow_high_precision_mv: bool,
    prev_param: i32,
) -> Result<i32, BitstreamError> {
    let (abs_bits, prec_bits) = if idx < 2 {
        if warp_type == 1 {
            let prec_sub = (!allow_high_precision_mv) as usize;
            (12 - prec_sub, 3 - prec_sub)
        } else {
            (12, 3)
        }
    } else {
        (10, 1)
    };
    let prec_diff = 16 - prec_bits;
    let round = if (idx % 3) == 2 { 1 << 16 } else { 0 };
    let sub = if (idx % 3) == 2 { 1 << prec_bits } else { 0 };
    let mx = 1 << abs_bits;
    let ref_val = (prev_param >> prec_diff) - sub;
    let val = decode_signed_subexp_with_ref(r, -mx, mx + 1, ref_val)?;
    Ok((val << prec_diff) + round)
}

fn decode_signed_subexp_with_ref(
    r: &mut BitReader,
    low: i32,
    high: i32,
    ref_val: i32,
) -> Result<i32, BitstreamError> {
    let mx = (high - low) as u32;
    let ref_u = (ref_val - low) as u32;
    let x = decode_unsigned_subexp_with_ref(r, mx, ref_u)?;
    Ok(x as i32 + low)
}

fn decode_unsigned_subexp_with_ref(
    r: &mut BitReader,
    mx: u32,
    ref_val: u32,
) -> Result<u32, BitstreamError> {
    let v = decode_subexp(r, mx)?;
    if (ref_val << 1) <= mx {
        Ok(inverse_recenter(ref_val, v))
    } else {
        Ok(mx - 1 - inverse_recenter(mx - 1 - ref_val, v))
    }
}

fn inverse_recenter(r: u32, v: u32) -> u32 {
    if v > 2 * r {
        v
    } else if (v & 1) != 0 {
        r - ((v + 1) >> 1)
    } else {
        r + (v >> 1)
    }
}

fn decode_subexp(r: &mut BitReader, num_syms: u32) -> Result<u32, BitstreamError> {
    let mut i = 0;
    let mut mk = 0;
    let k = 3;
    loop {
        let b2 = if i == 0 { k } else { k + i - 1 };
        let a = 1 << b2;
        if num_syms <= mk + 3 * a {
            let subexp_final_bits = r.read_ns(num_syms - mk)?;
            return Ok(subexp_final_bits + mk);
        } else if r.read_bit()? {
            i += 1;
            mk += a;
        } else {
            let subexp_bits = r.read_bits(b2 as usize)?;
            return Ok(subexp_bits + mk);
        }
    }
}
