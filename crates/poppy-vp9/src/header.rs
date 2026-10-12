//! VP9 uncompressed frame header parser and types.
//!
//! Conforms strictly to VP9 Bitstream & Decoding Process Specification (§6).

use crate::bitstream::{Vp9BitReader, Vp9ParserError};

/// Frame coding type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vp9FrameType {
    KeyFrame = 0,
    InterFrame = 1,
}

/// Segment feature configuration for one of the 8 possible segments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Vp9SegmentFeature {
    pub alt_q_enabled: bool,
    /// Alt-Q offset in the range -255..=255 (needs i16; previously i8 truncated 255 to -1).
    pub alt_q_val: i16,

    pub alt_lf_enabled: bool,
    pub alt_lf_val: i8,

    pub ref_frame_enabled: bool,
    pub ref_frame_val: u8,

    pub skip_enabled: bool,
}

/// Segmentation state for the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vp9Segmentation {
    pub enabled: bool,
    pub update_map: bool,
    pub tree_probs: [u8; 7],
    pub temporal_update: bool,
    pub pred_probs: [u8; 3],
    pub update_data: bool,
    pub abs_or_delta_update: bool,
    pub features: [Vp9SegmentFeature; 8],
}

impl Default for Vp9Segmentation {
    fn default() -> Self {
        Self {
            enabled: false,
            update_map: false,
            tree_probs: [255; 7],
            temporal_update: false,
            pred_probs: [255; 3],
            update_data: false,
            abs_or_delta_update: false,
            features: [Vp9SegmentFeature::default(); 8],
        }
    }
}

/// Loop filter parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vp9LoopFilter {
    pub filter_level: u8,
    pub sharpness_level: u8,
    pub mode_ref_delta_enabled: bool,
    pub mode_ref_delta_update: bool,
    pub ref_deltas: [i8; 4],
    pub mode_deltas: [i8; 2],
}

impl Default for Vp9LoopFilter {
    fn default() -> Self {
        Self {
            filter_level: 0,
            sharpness_level: 0,
            mode_ref_delta_enabled: false,
            mode_ref_delta_update: false,
            ref_deltas: [1, 0, -1, -1], // Initial past independence values
            mode_deltas: [0, 0],
        }
    }
}

/// Quantization parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vp9Quant {
    pub base_q_idx: u8,
    pub delta_q_y_dc: i8,
    pub delta_q_uv_dc: i8,
    pub delta_q_uv_ac: i8,
}

impl Vp9Quant {
    /// Returns true if this frame operates in lossless mode.
    #[inline]
    pub fn is_lossless(&self) -> bool {
        self.base_q_idx == 0
            && self.delta_q_y_dc == 0
            && self.delta_q_uv_dc == 0
            && self.delta_q_uv_ac == 0
    }
}

/// Parsed VP9 uncompressed frame header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vp9FrameHeader {
    pub profile: u8,
    pub show_existing_frame: bool,
    pub frame_to_show_map_idx: u8,

    pub frame_type: Vp9FrameType,
    pub show_frame: bool,
    pub error_resilient_mode: bool,
    pub intra_only: bool,
    pub reset_frame_context: u8,
    pub refresh_frame_context: bool,
    pub frame_parallel_decoding_mode: bool,
    pub frame_context_idx: u8,

    pub bit_depth: u8,
    pub color_space: u8,
    pub color_range: bool,
    pub subsampling_x: u8,
    pub subsampling_y: u8,

    pub width: u32,
    pub height: u32,
    pub render_width: u32,
    pub render_height: u32,

    pub refresh_frame_flags: u8,
    pub ref_frame_indices: [u8; 3],
    pub ref_frame_sign_bias: [bool; 3],
    pub allow_high_precision_mv: bool,
    pub interpolation_filter: u8,

    pub loop_filter: Vp9LoopFilter,
    pub quant: Vp9Quant,
    pub segmentation: Vp9Segmentation,

    pub tile_cols_log2: u8,
    pub tile_rows_log2: u8,

    pub uncompressed_header_size: usize,
    pub first_partition_size: u16,
}

impl Vp9FrameHeader {
    /// Parses an uncompressed frame header from raw packet bytes.
    ///
    /// `ref_dims` can optionally provide current (width, height) for reference buffers [0..8].
    pub fn parse(data: &[u8], ref_dims: Option<&[(u32, u32); 8]>) -> Result<Self, Vp9ParserError> {
        let mut r = Vp9BitReader::new(data);

        // Frame marker must be 0b10
        let frame_marker = r.read_bits(2)?;
        if frame_marker != 2 {
            return Err(Vp9ParserError::InvalidFrameMarker(frame_marker as u8));
        }

        // Profile (2 bits)
        let profile_low = r.read_bits(1)?;
        let profile_high = r.read_bits(1)?;
        let profile = ((profile_high << 1) | profile_low) as u8;
        if profile == 3 {
            let res = r.read_bits(1)?;
            if res != 0 {
                return Err(Vp9ParserError::ReservedBitMismatch(res));
            }
        }
        // profile is 2 bits so >3 is unreachable, but keep explicit guard.
        if profile > 3 {
            return Err(Vp9ParserError::InvalidProfile(profile));
        }

        let show_existing_frame = r.read_bit()?;
        if show_existing_frame {
            let frame_to_show_map_idx = r.read_bits(3)? as u8;
            r.align_to_byte();
            let uncompressed_header_size = r.byte_offset();

            return Ok(Self {
                profile,
                show_existing_frame: true,
                frame_to_show_map_idx,
                frame_type: Vp9FrameType::InterFrame,
                show_frame: true,
                error_resilient_mode: false,
                intra_only: false,
                reset_frame_context: 0,
                refresh_frame_context: false,
                frame_parallel_decoding_mode: true,
                frame_context_idx: 0,
                bit_depth: 8,
                color_space: 1,
                color_range: false,
                subsampling_x: 1,
                subsampling_y: 1,
                width: 0,
                height: 0,
                render_width: 0,
                render_height: 0,
                refresh_frame_flags: 0,
                ref_frame_indices: [0; 3],
                ref_frame_sign_bias: [false; 3],
                allow_high_precision_mv: false,
                interpolation_filter: 0,
                loop_filter: Vp9LoopFilter::default(),
                quant: Vp9Quant::default(),
                segmentation: Vp9Segmentation::default(),
                tile_cols_log2: 0,
                tile_rows_log2: 0,
                uncompressed_header_size,
                first_partition_size: 0,
            });
        }

        let frame_type = if r.read_bit()? {
            Vp9FrameType::InterFrame
        } else {
            Vp9FrameType::KeyFrame
        };

        let show_frame = r.read_bit()?;
        let error_resilient_mode = r.read_bit()?;

        let intra_only;
        let reset_frame_context;
        let refresh_frame_flags;
        let mut ref_frame_indices = [0u8; 3];
        let mut ref_frame_sign_bias = [false; 3];
        let mut allow_high_precision_mv = false;
        let mut interpolation_filter = 0u8;

        let mut bit_depth = 8u8;
        let mut color_space = 1u8;
        let mut color_range = false;
        let mut subsampling_x = 1u8;
        let mut subsampling_y = 1u8;

        let width;
        let height;
        let render_width;
        let render_height;

        if frame_type == Vp9FrameType::KeyFrame {
            // Verify sync code 0x498342
            let sync = r.read_bits(24)?;
            if sync != 0x498342 {
                return Err(Vp9ParserError::InvalidSyncCode(sync));
            }

            // Color config
            Self::parse_color_config(
                &mut r,
                profile,
                &mut bit_depth,
                &mut color_space,
                &mut color_range,
                &mut subsampling_x,
                &mut subsampling_y,
            )?;

            // Dimensions
            let w = r.read_bits(16)? + 1;
            let h = r.read_bits(16)? + 1;

            if r.read_bit()? {
                render_width = r.read_bits(16)? + 1;
                render_height = r.read_bits(16)? + 1;
            } else {
                render_width = w;
                render_height = h;
            }
            width = w;
            height = h;

            refresh_frame_flags = 0xFF;
            intra_only = true;
            reset_frame_context = 0;
        } else {
            intra_only = if !show_frame { r.read_bit()? } else { false };

            reset_frame_context = if !error_resilient_mode {
                r.read_bits(2)? as u8
            } else {
                0
            };

            if intra_only {
                let sync = r.read_bits(24)?;
                if sync != 0x498342 {
                    return Err(Vp9ParserError::InvalidSyncCode(sync));
                }

                if profile > 0 {
                    Self::parse_color_config(
                        &mut r,
                        profile,
                        &mut bit_depth,
                        &mut color_space,
                        &mut color_range,
                        &mut subsampling_x,
                        &mut subsampling_y,
                    )?;
                } else {
                    color_space = 1;
                    subsampling_x = 1;
                    subsampling_y = 1;
                    bit_depth = 8;
                }

                refresh_frame_flags = r.read_bits(8)? as u8;

                let w = r.read_bits(16)? + 1;
                let h = r.read_bits(16)? + 1;

                if r.read_bit()? {
                    render_width = r.read_bits(16)? + 1;
                    render_height = r.read_bits(16)? + 1;
                } else {
                    render_width = w;
                    render_height = h;
                }
                width = w;
                height = h;
            } else {
                refresh_frame_flags = r.read_bits(8)? as u8;

                for i in 0..3 {
                    ref_frame_indices[i] = r.read_bits(3)? as u8;
                    ref_frame_sign_bias[i] = r.read_bit()?;
                }

                // Dimensions with refs: per VP9 spec §6.2, when found_ref==1
                // no explicit dimensions are sent — they are inherited from
                // the referenced frame. Previously fell through to
                // read_bits(16) consuming render-flag bits as width.
                // Now requires valid ref_dims; otherwise returns error instead
                // of mis-parsing.
                let mut found_ref = false;
                let mut ref_w = 0;
                let mut ref_h = 0;
                for &idx in &ref_frame_indices {
                    if r.read_bit()? {
                        if let Some(dims) = ref_dims {
                            let idx = idx as usize;
                            if idx < dims.len() {
                                ref_w = dims[idx].0;
                                ref_h = dims[idx].1;
                            }
                        }
                        found_ref = true;
                        break;
                    }
                }

                let (w, h) = if found_ref {
                    if ref_w > 0 && ref_h > 0 {
                        (ref_w, ref_h)
                    } else {
                        return Err(Vp9ParserError::UnexpectedEof {
                            needed_bits: 32,
                            available_bits: r.bits_remaining(),
                        });
                    }
                } else {
                    (r.read_bits(16)? + 1, r.read_bits(16)? + 1)
                };

                if r.read_bit()? {
                    render_width = r.read_bits(16)? + 1;
                    render_height = r.read_bits(16)? + 1;
                } else {
                    render_width = w;
                    render_height = h;
                }
                width = w;
                height = h;

                allow_high_precision_mv = r.read_bit()?;

                let is_filter_switchable = r.read_bit()?;
                interpolation_filter = if is_filter_switchable {
                    3 // Switchable
                } else {
                    r.read_bits(2)? as u8
                };
            }
        }

        let (refresh_frame_context, frame_parallel_decoding_mode) = if !error_resilient_mode {
            (r.read_bit()?, r.read_bit()?)
        } else {
            (false, true)
        };

        let frame_context_idx = r.read_bits(2)? as u8;

        // Loop filter params
        let filter_level = r.read_bits(6)? as u8;
        let sharpness_level = r.read_bits(3)? as u8;
        let mode_ref_delta_enabled = r.read_bit()?;
        let mut ref_deltas = [1, 0, -1, -1];
        let mut mode_deltas = [0, 0];
        let mut mode_ref_delta_update = false;
        if mode_ref_delta_enabled {
            mode_ref_delta_update = r.read_bit()?;
            if mode_ref_delta_update {
                for delta in &mut ref_deltas {
                    if r.read_bit()? {
                        *delta = r.read_signed_bits(6)? as i8;
                    }
                }
                for delta in &mut mode_deltas {
                    if r.read_bit()? {
                        *delta = r.read_signed_bits(6)? as i8;
                    }
                }
            }
        }
        let loop_filter = Vp9LoopFilter {
            filter_level,
            sharpness_level,
            mode_ref_delta_enabled,
            mode_ref_delta_update,
            ref_deltas,
            mode_deltas,
        };

        // Quantization params
        let base_q_idx = r.read_bits(8)? as u8;
        let delta_q_y_dc = r.read_delta_q()?;
        let delta_q_uv_dc = r.read_delta_q()?;
        let delta_q_uv_ac = r.read_delta_q()?;
        let quant = Vp9Quant {
            base_q_idx,
            delta_q_y_dc,
            delta_q_uv_dc,
            delta_q_uv_ac,
        };

        // Segmentation params
        let segmentation_enabled = r.read_bit()?;
        let mut segmentation = Vp9Segmentation {
            enabled: segmentation_enabled,
            ..Default::default()
        };
        if segmentation.enabled {
            segmentation.update_map = r.read_bit()?;
            if segmentation.update_map {
                for i in 0..7 {
                    segmentation.tree_probs[i] = if r.read_bit()? {
                        r.read_bits(8)? as u8
                    } else {
                        255
                    };
                }
                segmentation.temporal_update = r.read_bit()?;
                for i in 0..3 {
                    segmentation.pred_probs[i] = if segmentation.temporal_update && r.read_bit()? {
                        r.read_bits(8)? as u8
                    } else {
                        255
                    };
                }
            }

            segmentation.update_data = r.read_bit()?;
            if segmentation.update_data {
                segmentation.abs_or_delta_update = r.read_bit()?;
                for i in 0..8 {
                    // Feature 0: alt_q (8 bits magnitude + sign = -255..255, needs i16)
                    if r.read_bit()? {
                        segmentation.features[i].alt_q_enabled = true;
                        let mag = r.read_bits(8)? as i16;
                        let sign = r.read_bit()?;
                        let v = if sign { -mag } else { mag };
                        segmentation.features[i].alt_q_val = v;
                    }
                    // Feature 1: alt_lf (6 bits signed)
                    if r.read_bit()? {
                        segmentation.features[i].alt_lf_enabled = true;
                        let mag = r.read_bits(6)? as i32;
                        let sign = r.read_bit()?;
                        segmentation.features[i].alt_lf_val = if sign { -mag } else { mag } as i8;
                    }
                    // Feature 2: ref_frame (2 bits unsigned)
                    if r.read_bit()? {
                        segmentation.features[i].ref_frame_enabled = true;
                        segmentation.features[i].ref_frame_val = r.read_bits(2)? as u8;
                    }
                    // Feature 3: skip (0 bits data)
                    if r.read_bit()? {
                        segmentation.features[i].skip_enabled = true;
                    }
                }
            }
        }

        // Tile info (VP9 Spec §6.2.14)
        let mi_cols = (width + 7) >> 3;
        let sb64_cols = (mi_cols + 7) >> 3;

        let mut min_log2_tile_cols = 0u8;
        while (64u32 << min_log2_tile_cols) < sb64_cols {
            min_log2_tile_cols += 1;
        }
        let mut max_log2_tile_cols = 0u8;
        while (sb64_cols >> (max_log2_tile_cols + 1)) >= 4 {
            max_log2_tile_cols += 1;
        }

        let mut tile_cols_log2 = min_log2_tile_cols;
        while tile_cols_log2 < max_log2_tile_cols {
            if r.read_bit()? {
                tile_cols_log2 += 1;
            } else {
                break;
            }
        }

        let mut tile_rows_log2 = 0u8;
        if r.read_bit()? {
            tile_rows_log2 += 1;
            if r.read_bit()? {
                tile_rows_log2 += 1;
            }
        }

        let first_partition_size = r.read_bits(16)? as u16;

        r.align_to_byte();
        let uncompressed_header_size = r.byte_offset();

        Ok(Self {
            profile,
            show_existing_frame: false,
            frame_to_show_map_idx: 0,
            frame_type,
            show_frame,
            error_resilient_mode,
            intra_only,
            reset_frame_context,
            refresh_frame_context,
            frame_parallel_decoding_mode,
            frame_context_idx,
            bit_depth,
            color_space,
            color_range,
            subsampling_x,
            subsampling_y,
            width,
            height,
            render_width,
            render_height,
            refresh_frame_flags,
            ref_frame_indices,
            ref_frame_sign_bias,
            allow_high_precision_mv,
            interpolation_filter,
            loop_filter,
            quant,
            segmentation,
            tile_cols_log2,
            tile_rows_log2,
            uncompressed_header_size,
            first_partition_size,
        })
    }

    fn parse_color_config(
        r: &mut Vp9BitReader,
        profile: u8,
        bit_depth: &mut u8,
        color_space: &mut u8,
        color_range: &mut bool,
        subsampling_x: &mut u8,
        subsampling_y: &mut u8,
    ) -> Result<(), Vp9ParserError> {
        if profile >= 2 {
            let ten_or_twelve = r.read_bit()?;
            *bit_depth = if ten_or_twelve { 12 } else { 10 };
        } else {
            *bit_depth = 8;
        }

        *color_space = r.read_bits(3)? as u8;

        if *color_space != 7 {
            // CS_RGB = 7
            *color_range = r.read_bit()?;
            if profile == 1 || profile == 3 {
                *subsampling_x = r.read_bits(1)? as u8;
                *subsampling_y = r.read_bits(1)? as u8;
                let reserved = r.read_bits(1)?;
                if reserved != 0 {
                    return Err(Vp9ParserError::ReservedBitMismatch(reserved));
                }
            } else {
                *subsampling_x = 1;
                *subsampling_y = 1;
            }
        } else {
            *color_range = true;
            if profile == 1 || profile == 3 {
                *subsampling_x = 0;
                *subsampling_y = 0;
                let reserved = r.read_bits(1)?;
                if reserved != 0 {
                    return Err(Vp9ParserError::ReservedBitMismatch(reserved));
                }
            }
        }

        Ok(())
    }

    /// VA-API motion compensation filter type conversion according to libva spec:
    /// `filtermode ^ (filtermode <= 1)`
    #[inline]
    pub fn vaapi_mcomp_filter_type(&self) -> u32 {
        let f = self.interpolation_filter as u32;
        f ^ (if f <= 1 { 1 } else { 0 })
    }
}
