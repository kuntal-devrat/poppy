//! Video Parameter Set (VPS) parser (ITU-T H.265 §7.3.2.1).

use crate::bitstream::{BitReader, BitstreamError};
use crate::ptl::ProfileTierLevel;

/// Parsed Video Parameter Set (VPS).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoParameterSet {
    pub vps_video_parameter_set_id: u8,
    pub vps_base_layer_internal_flag: bool,
    pub vps_base_layer_available_flag: bool,
    pub vps_max_layers_minus1: u8,
    pub vps_max_sub_layers_minus1: u8,
    pub vps_temporal_id_nesting_flag: bool,
    pub profile_tier_level: ProfileTierLevel,
    pub max_dec_pic_buffering: [u32; 8],
    pub max_num_reorder_pics: [u32; 8],
}

impl VideoParameterSet {
    /// Parses a Video Parameter Set from its RBSP byte slice.
    pub fn parse(rbsp: &[u8]) -> Result<Self, BitstreamError> {
        let mut reader = BitReader::new(rbsp);

        let vps_video_parameter_set_id = reader.read_bits(4)? as u8;
        let vps_base_layer_internal_flag = reader.read_bit()?;
        let vps_base_layer_available_flag = reader.read_bit()?;
        let vps_max_layers_minus1 = reader.read_bits(6)? as u8;
        let vps_max_sub_layers_minus1 = reader.read_bits(3)? as u8;
        let vps_temporal_id_nesting_flag = reader.read_bit()?;

        // vps_reserved_0xffff_16bits
        reader.skip_bits(16)?;

        let profile_tier_level = ProfileTierLevel::parse(
            &mut reader,
            true,
            vps_max_sub_layers_minus1 as u32,
        )?;

        let vps_sub_layer_ordering_info_present_flag = reader.read_bit()?;
        let mut max_dec_pic_buffering = [0u32; 8];
        let mut max_num_reorder_pics = [0u32; 8];

        let start_sub_layer = if vps_sub_layer_ordering_info_present_flag {
            0
        } else {
            vps_max_sub_layers_minus1 as usize
        };

        for i in start_sub_layer..=vps_max_sub_layers_minus1 as usize {
            if i < 8 {
                let max_dpb = reader.read_ue()? + 1;
                let max_reorder = reader.read_ue()?;
                let _max_latency = reader.read_ue()?;
                max_dec_pic_buffering[i] = max_dpb;
                max_num_reorder_pics[i] = max_reorder;
            }
        }

        if !vps_sub_layer_ordering_info_present_flag {
            for i in 0..vps_max_sub_layers_minus1 as usize {
                if i < 8 {
                    max_dec_pic_buffering[i] = max_dec_pic_buffering[vps_max_sub_layers_minus1 as usize];
                    max_num_reorder_pics[i] = max_num_reorder_pics[vps_max_sub_layers_minus1 as usize];
                }
            }
        }

        Ok(Self {
            vps_video_parameter_set_id,
            vps_base_layer_internal_flag,
            vps_base_layer_available_flag,
            vps_max_layers_minus1,
            vps_max_sub_layers_minus1,
            vps_temporal_id_nesting_flag,
            profile_tier_level,
            max_dec_pic_buffering,
            max_num_reorder_pics,
        })
    }
}
