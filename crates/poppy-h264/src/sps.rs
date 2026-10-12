//! Sequence Parameter Set (SPS) parser.
//!
//! Implements ITU-T H.264 §7.3.2.1.1.

use crate::bitstream::{BitReader, BitstreamError, remove_emulation_prevention};

/// Frame cropping specification from SPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameCropping {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

/// VUI parameters (ITU-T H.264 Annex E). Previously ignored; now parsed for
/// aspect ratio, timing (fps), and colour description needed by render/HDR.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VuiParameters {
    pub aspect_ratio_info_present: bool,
    pub aspect_ratio_idc: u8,
    pub sar_width: u32,
    pub sar_height: u32,
    pub overscan_info_present: bool,
    pub overscan_appropriate: bool,
    pub video_signal_type_present: bool,
    pub video_format: u8,
    pub video_full_range: bool,
    pub colour_description_present: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub chroma_loc_info_present: bool,
    pub timing_info_present: bool,
    pub num_units_in_tick: u32,
    pub time_scale: u32,
    pub fixed_frame_rate: bool,
    pub hrd_parameters_present: bool,
}

/// A parsed H.264 Sequence Parameter Set (SPS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sps {
    pub profile_idc: u8,
    pub constraint_set0_flag: bool,
    pub constraint_set1_flag: bool,
    pub constraint_set2_flag: bool,
    pub constraint_set3_flag: bool,
    pub constraint_set4_flag: bool,
    pub constraint_set5_flag: bool,
    pub level_idc: u8,
    pub seq_parameter_set_id: u32,

    // Chroma / Bit depth (High profiles)
    pub chroma_format_idc: u8,
    pub separate_colour_plane_flag: bool,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub qpprime_y_zero_transform_bypass_flag: bool,

    // Picture dimensions & order
    pub log2_max_frame_num_minus4: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb_minus4: u32,
    pub delta_pic_order_always_zero_flag: bool,
    pub offset_for_non_ref_pic: i32,
    pub offset_for_top_to_bottom_field: i32,
    pub num_ref_frames_in_pic_order_cnt_cycle: u32,
    pub offset_for_ref_frame: Vec<i32>,

    pub max_num_ref_frames: u32,
    pub gaps_in_frame_num_value_allowed_flag: bool,
    pub pic_width_in_mbs_minus1: u32,
    pub pic_height_in_map_units_minus1: u32,
    pub frame_mbs_only_flag: bool,
    pub mb_adaptive_frame_field_flag: bool,
    pub direct_8x8_inference_flag: bool,

    pub frame_cropping: Option<FrameCropping>,

    /// VUI parameters if present (aspect, timing, colour, HRD).
    pub vui: Option<VuiParameters>,
}

impl Sps {
    /// Parse an SPS from a raw NAL unit (with or without 1-byte NAL header).
    pub fn parse(nal_data: &[u8]) -> Result<Self, BitstreamError> {
        // Strip NAL header byte if present (SPS NAL unit type is 7).
        // Require forbidden_zero_bit==0 to avoid mis-stripping RBSP whose
        // first byte happens to have low 5 bits == 7.
        let payload = if !nal_data.is_empty()
            && (nal_data[0] & 0x1F) == 7
            && (nal_data[0] & 0x80) == 0
            && nal_data.len() >= 2
        {
            &nal_data[1..]
        } else {
            nal_data
        };

        let rbsp = remove_emulation_prevention(payload);
        let mut r = BitReader::new(&rbsp);

        let profile_idc = r.read_bits(8)? as u8;
        let constraint_set0_flag = r.read_bit()?;
        let constraint_set1_flag = r.read_bit()?;
        let constraint_set2_flag = r.read_bit()?;
        let constraint_set3_flag = r.read_bit()?;
        let constraint_set4_flag = r.read_bit()?;
        let constraint_set5_flag = r.read_bit()?;
        let _reserved_zero_2bits = r.read_bits(2)?;
        let level_idc = r.read_bits(8)? as u8;
        let seq_parameter_set_id = r.read_ue()?;
        if seq_parameter_set_id > 31 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "seq_parameter_set_id {seq_parameter_set_id} exceeds 31"
            )));
        }

        let mut chroma_format_idc = 1; // Default: 4:2:0
        let mut separate_colour_plane_flag = false;
        let mut bit_depth_luma_minus8 = 0;
        let mut bit_depth_chroma_minus8 = 0;
        let mut qpprime_y_zero_transform_bypass_flag = false;

        // Profiles with High fidelity syntax
        if matches!(
            profile_idc,
            100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
        ) {
            let cfi = r.read_ue()?;
            if cfi > 3 {
                return Err(BitstreamError::InvalidSyntax(format!(
                    "chroma_format_idc {cfi} exceeds 3"
                )));
            }
            chroma_format_idc = cfi as u8;
            if chroma_format_idc == 3 {
                separate_colour_plane_flag = r.read_bit()?;
            }
            let bdl = r.read_ue()?;
            let bdc = r.read_ue()?;
            if bdl > 6 || bdc > 6 {
                return Err(BitstreamError::InvalidSyntax(format!(
                    "bit_depth_minus8 {bdl}/{bdc} exceeds 6"
                )));
            }
            bit_depth_luma_minus8 = bdl as u8;
            bit_depth_chroma_minus8 = bdc as u8;
            qpprime_y_zero_transform_bypass_flag = r.read_bit()?;

            let seq_scaling_matrix_present_flag = r.read_bit()?;
            if seq_scaling_matrix_present_flag {
                let count = if chroma_format_idc != 3 { 8 } else { 12 };
                for i in 0..count {
                    let seq_scaling_list_present_flag = r.read_bit()?;
                    if seq_scaling_list_present_flag {
                        let size = if i < 6 { 16 } else { 64 };
                        skip_scaling_list(&mut r, size)?;
                    }
                }
            }
        }

        let log2_max_frame_num_minus4 = r.read_ue()?;
        if log2_max_frame_num_minus4 > 12 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "log2_max_frame_num_minus4 {log2_max_frame_num_minus4} exceeds 12"
            )));
        }
        let pic_order_cnt_type = r.read_ue()?;
        if pic_order_cnt_type > 2 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "pic_order_cnt_type {pic_order_cnt_type} exceeds 2"
            )));
        }
        let mut log2_max_pic_order_cnt_lsb_minus4 = 0;
        let mut delta_pic_order_always_zero_flag = false;
        let mut offset_for_non_ref_pic = 0;
        let mut offset_for_top_to_bottom_field = 0;
        let mut num_ref_frames_in_pic_order_cnt_cycle = 0;
        let mut offset_for_ref_frame = Vec::new();

        if pic_order_cnt_type == 0 {
            let v = r.read_ue()?;
            if v > 12 {
                return Err(BitstreamError::InvalidSyntax(format!(
                    "log2_max_pic_order_cnt_lsb_minus4 {v} exceeds 12"
                )));
            }
            log2_max_pic_order_cnt_lsb_minus4 = v;
        } else if pic_order_cnt_type == 1 {
            delta_pic_order_always_zero_flag = r.read_bit()?;
            offset_for_non_ref_pic = r.read_se()?;
            offset_for_top_to_bottom_field = r.read_se()?;
            let n = r.read_ue()?;
            // Cap to prevent OOM on malicious ue==4G (previously unbounded loop).
            if n > 255 {
                return Err(BitstreamError::InvalidSyntax(format!(
                    "num_ref_frames_in_pic_order_cnt_cycle {n} exceeds 255"
                )));
            }
            num_ref_frames_in_pic_order_cnt_cycle = n;
            for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
                offset_for_ref_frame.push(r.read_se()?);
            }
        }

        let max_num_ref_frames = r.read_ue()?;
        if max_num_ref_frames > 16 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "max_num_ref_frames {max_num_ref_frames} exceeds 16"
            )));
        }
        let gaps_in_frame_num_value_allowed_flag = r.read_bit()?;
        let pic_width_in_mbs_minus1 = r.read_ue()?;
        let pic_height_in_map_units_minus1 = r.read_ue()?;
        if pic_width_in_mbs_minus1 > 2000 || pic_height_in_map_units_minus1 > 2000 {
            return Err(BitstreamError::InvalidSyntax(format!(
                "SPS dimensions {pic_width_in_mbs_minus1}x{pic_height_in_map_units_minus1} exceed sane limits"
            )));
        }
        let frame_mbs_only_flag = r.read_bit()?;
        let mb_adaptive_frame_field_flag = if !frame_mbs_only_flag {
            r.read_bit()?
        } else {
            false
        };
        let direct_8x8_inference_flag = r.read_bit()?;

        let frame_cropping_flag = r.read_bit()?;
        let frame_cropping = if frame_cropping_flag {
            Some(FrameCropping {
                left: r.read_ue()?,
                right: r.read_ue()?,
                top: r.read_ue()?,
                bottom: r.read_ue()?,
            })
        } else {
            None
        };

        // VUI parameters (Annex E). Optional trailing syntax; parse when present.
        let vui_parameters_present_flag = r.read_bit().unwrap_or(false);
        let vui = if vui_parameters_present_flag {
            Some(parse_vui(&mut r)?)
        } else {
            None
        };

        Ok(Self {
            profile_idc,
            constraint_set0_flag,
            constraint_set1_flag,
            constraint_set2_flag,
            constraint_set3_flag,
            constraint_set4_flag,
            constraint_set5_flag,
            level_idc,
            seq_parameter_set_id,
            chroma_format_idc,
            separate_colour_plane_flag,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            qpprime_y_zero_transform_bypass_flag,
            log2_max_frame_num_minus4,
            pic_order_cnt_type,
            log2_max_pic_order_cnt_lsb_minus4,
            delta_pic_order_always_zero_flag,
            offset_for_non_ref_pic,
            offset_for_top_to_bottom_field,
            num_ref_frames_in_pic_order_cnt_cycle,
            offset_for_ref_frame,
            max_num_ref_frames,
            gaps_in_frame_num_value_allowed_flag,
            pic_width_in_mbs_minus1,
            pic_height_in_map_units_minus1,
            frame_mbs_only_flag,
            mb_adaptive_frame_field_flag,
            direct_8x8_inference_flag,
            frame_cropping,
            vui,
        })
    }

    /// Frame rate from VUI timing info, if present.
    pub fn fps(&self) -> Option<f64> {
        let v = self.vui.as_ref()?;
        if v.timing_info_present && v.num_units_in_tick > 0 {
            Some(v.time_scale as f64 / (2.0 * v.num_units_in_tick as f64))
        } else {
            None
        }
    }

    /// Macroblock-aligned coded width in pixels: `(pic_width_in_mbs_minus1 + 1) * 16`.
    pub fn coded_width(&self) -> u32 {
        (self.pic_width_in_mbs_minus1 + 1) * 16
    }

    /// Macroblock-aligned coded height in pixels.
    pub fn coded_height(&self) -> u32 {
        let mult = if self.frame_mbs_only_flag { 1 } else { 2 };
        (self.pic_height_in_map_units_minus1 + 1) * 16 * mult
    }

    /// Display width in pixels after applying cropping offsets.
    pub fn display_width(&self) -> u32 {
        let mut width = self.coded_width();
        if let Some(crop) = self.frame_cropping {
            let crop_unit_x = match self.chroma_format_idc {
                1 | 2 => 2, // 4:2:0 or 4:2:2
                3 => 1,     // 4:4:4
                _ => 1,     // Monochrome
            };
            let crop_pixels = crop
                .left
                .saturating_add(crop.right)
                .saturating_mul(crop_unit_x);
            width = width.saturating_sub(crop_pixels);
        }
        width
    }

    /// Display height in pixels after applying cropping offsets.
    pub fn display_height(&self) -> u32 {
        let mut height = self.coded_height();
        if let Some(crop) = self.frame_cropping {
            let mult = if self.frame_mbs_only_flag { 1 } else { 2 };
            let crop_unit_y = match self.chroma_format_idc {
                1 => 2 * mult, // 4:2:0
                2 => mult,     // 4:2:2
                _ => mult,
            };
            let crop_pixels = crop
                .top
                .saturating_add(crop.bottom)
                .saturating_mul(crop_unit_y);
            height = height.saturating_sub(crop_pixels);
        }
        height
    }
}

fn skip_scaling_list(r: &mut BitReader, size: usize) -> Result<(), BitstreamError> {
    let mut last_scale = 8i64;
    let mut next_scale = 8i64;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = r.read_se()? as i64;
            next_scale = (last_scale + delta_scale).rem_euclid(256);
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

fn parse_vui(r: &mut BitReader) -> Result<VuiParameters, BitstreamError> {
    let aspect_ratio_info_present = r.read_bit()?;
    let (aspect_ratio_idc, sar_width, sar_height) = if aspect_ratio_info_present {
        let idc = r.read_bits(8)? as u8;
        if idc == 255 {
            (idc, r.read_bits(16)?, r.read_bits(16)?)
        } else {
            (idc, 0, 0)
        }
    } else {
        (0, 0, 0)
    };
    let overscan_info_present = r.read_bit()?;
    let overscan_appropriate = if overscan_info_present {
        r.read_bit()?
    } else {
        false
    };
    let video_signal_type_present = r.read_bit()?;
    let (
        video_format,
        video_full_range,
        colour_description_present,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
    ) = if video_signal_type_present {
        let vf = r.read_bits(3)? as u8;
        let vfr = r.read_bit()?;
        let cdp = r.read_bit()?;
        if cdp {
            (
                vf,
                vfr,
                true,
                r.read_bits(8)? as u8,
                r.read_bits(8)? as u8,
                r.read_bits(8)? as u8,
            )
        } else {
            (vf, vfr, false, 0, 0, 0)
        }
    } else {
        (0, false, false, 0, 0, 0)
    };
    let chroma_loc_info_present = r.read_bit()?;
    if chroma_loc_info_present {
        let _ = r.read_ue()?;
        let _ = r.read_ue()?;
    }
    let timing_info_present = r.read_bit()?;
    let (num_units_in_tick, time_scale, fixed_frame_rate) = if timing_info_present {
        let n = r.read_bits(32)?;
        let t = r.read_bits(32)?;
        let f = r.read_bit()?;
        if n == 0 || t == 0 {
            return Err(BitstreamError::InvalidSyntax(
                "VUI timing num_units_in_tick/time_scale must be non-zero".into(),
            ));
        }
        (n, t, f)
    } else {
        (0, 0, false)
    };
    for _ in 0..2 {
        let hrd_present = r.read_bit().unwrap_or(false);
        if hrd_present {
            let cpb_cnt = r.read_ue().unwrap_or(0).min(31);
            let _ = r.read_bits(4);
            let _ = r.read_bits(4);
            for _ in 0..=cpb_cnt {
                let _ = r.read_ue();
                let _ = r.read_ue();
                let _ = r.read_bit();
            }
            let _ = r.read_bits(5);
            let _ = r.read_bits(5);
            let _ = r.read_bits(5);
            let _ = r.read_bits(5);
            let _ = r.read_bits(5);
        }
    }
    if r.read_bit().unwrap_or(false) {
        let _ = r.read_bit();
        let _ = r.read_ue();
        let _ = r.read_ue();
        let _ = r.read_ue();
        let _ = r.read_ue();
        let _ = r.read_ue();
        let _ = r.read_ue();
    }
    Ok(VuiParameters {
        aspect_ratio_info_present,
        aspect_ratio_idc,
        sar_width,
        sar_height,
        overscan_info_present,
        overscan_appropriate,
        video_signal_type_present,
        video_format,
        video_full_range,
        colour_description_present,
        colour_primaries,
        transfer_characteristics,
        matrix_coefficients,
        chroma_loc_info_present,
        timing_info_present,
        num_units_in_tick,
        time_scale,
        fixed_frame_rate,
        hrd_parameters_present: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_real_sps_1080p() {
        // Real-world SPS for 1920x1080 Baseline/Main profile
        // 0x67 (NAL header SPS) followed by payload
        // 1920x1080 -> 120 macroblocks wide (pic_width_in_mbs_minus1 = 119)
        // 68 macroblocks high (pic_height_in_map_units_minus1 = 67)
        // cropping: bottom = 8 pixels -> height 1080
        // Encoded raw SPS bytes:
        let sps_bytes = [
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x78, 0x02, 0x27, 0xE5, 0xC0, 0x44, 0x00,
            0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x36, 0x82, 0x21, 0x1B,
        ];

        let sps = Sps::parse(&sps_bytes).expect("Failed to parse SPS");
        assert_eq!(sps.profile_idc, 100); // High Profile
        assert_eq!(sps.level_idc, 31); // Level 3.1
        assert_eq!(sps.pic_width_in_mbs_minus1, 119); // (119 + 1) * 16 = 1920
        assert_eq!(sps.coded_width(), 1920);
        assert_eq!(sps.coded_height(), 1088);
        assert_eq!(sps.display_height(), 1080);
    }

    #[test]
    fn test_parse_4k_sps_dimensions() {
        // Synthesize an SPS for 4K (3840x2160):
        // pic_width_in_mbs_minus1 = 239 -> 3840
        // pic_height_in_map_units_minus1 = 134 -> 2160 (135 * 16 = 2160)
        // Let's create an Sps struct and verify display_width/height formulas
        let sps = Sps {
            profile_idc: 100,
            constraint_set0_flag: false,
            constraint_set1_flag: false,
            constraint_set2_flag: false,
            constraint_set3_flag: false,
            constraint_set4_flag: false,
            constraint_set5_flag: false,
            level_idc: 51,
            seq_parameter_set_id: 0,
            chroma_format_idc: 1,
            separate_colour_plane_flag: false,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            qpprime_y_zero_transform_bypass_flag: false,
            log2_max_frame_num_minus4: 0,
            pic_order_cnt_type: 0,
            log2_max_pic_order_cnt_lsb_minus4: 2,
            delta_pic_order_always_zero_flag: false,
            offset_for_non_ref_pic: 0,
            offset_for_top_to_bottom_field: 0,
            num_ref_frames_in_pic_order_cnt_cycle: 0,
            offset_for_ref_frame: Vec::new(),
            max_num_ref_frames: 4,
            gaps_in_frame_num_value_allowed_flag: false,
            pic_width_in_mbs_minus1: 239,
            pic_height_in_map_units_minus1: 134,
            frame_mbs_only_flag: true,
            mb_adaptive_frame_field_flag: false,
            direct_8x8_inference_flag: true,
            frame_cropping: None,
            vui: None,
        };

        assert_eq!(sps.coded_width(), 3840);
        assert_eq!(sps.coded_height(), 2160);
        assert_eq!(sps.display_width(), 3840);
        assert_eq!(sps.display_height(), 2160);
    }
}
