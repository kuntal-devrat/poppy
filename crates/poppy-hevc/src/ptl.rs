//! HEVC Profile, Tier, and Level syntax parser (ITU-T H.265 §7.3.7.3).

use crate::bitstream::{BitReader, BitstreamError};

/// Profile, Tier, and Level parameters per ITU-T H.265 §7.3.7.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileTierLevel {
    /// Context / Profile space (2 bits: 0 = standard).
    pub profile_space: u8,
    /// Tier flag: `false` = Main tier, `true` = High tier.
    pub tier_flag: bool,
    /// Profile IDC: 1 = Main, 2 = Main 10, 3 = Main Still Picture, 4 = Range Extensions.
    pub profile_idc: u8,
    /// 32-bit bitmask indicating compatibility with other profiles.
    pub profile_compatibility_flags: u32,
    pub progressive_source_flag: bool,
    pub interlaced_source_flag: bool,
    pub non_packed_constraint_flag: bool,
    pub frame_only_constraint_flag: bool,
    /// Level IDC (e.g., 93 = 3.1, 120 = 4.0, 150 = 5.0, 153 = 5.1).
    pub level_idc: u8,
}

impl ProfileTierLevel {
    /// Parse `profile_tier_level(profilePresentFlag, maxNumSubLayersMinus1)`.
    pub fn parse(
        reader: &mut BitReader<'_>,
        profile_present: bool,
        max_sub_layers_minus1: u32,
    ) -> Result<Self, BitstreamError> {
        let mut ptl = Self {
            profile_space: 0,
            tier_flag: false,
            profile_idc: 0,
            profile_compatibility_flags: 0,
            progressive_source_flag: true,
            interlaced_source_flag: false,
            non_packed_constraint_flag: false,
            frame_only_constraint_flag: true,
            level_idc: 0,
        };

        if profile_present {
            ptl.profile_space = reader.read_bits(2)? as u8;
            ptl.tier_flag = reader.read_bit()?;
            ptl.profile_idc = reader.read_bits(5)? as u8;
            ptl.profile_compatibility_flags = reader.read_bits(32)?;
            ptl.progressive_source_flag = reader.read_bit()?;
            ptl.interlaced_source_flag = reader.read_bit()?;
            ptl.non_packed_constraint_flag = reader.read_bit()?;
            ptl.frame_only_constraint_flag = reader.read_bit()?;

            // 44 bits reserved constraint flags
            reader.skip_bits(44)?;
        }

        ptl.level_idc = reader.read_bits(8)? as u8;

        let mut sub_layer_profile_present = [false; 8];
        let mut sub_layer_level_present = [false; 8];

        for i in 0..max_sub_layers_minus1 as usize {
            if i < 8 {
                sub_layer_profile_present[i] = reader.read_bit()?;
                sub_layer_level_present[i] = reader.read_bit()?;
            }
        }

        if max_sub_layers_minus1 > 0 {
            for _ in max_sub_layers_minus1 as usize..8 {
                reader.skip_bits(2)?;
            }
        }

        for i in 0..max_sub_layers_minus1 as usize {
            if i < 8 {
                if sub_layer_profile_present[i] {
                    // 2 + 1 + 5 + 32 + 4 + 44 = 88 bits
                    reader.skip_bits(88)?;
                }
                if sub_layer_level_present[i] {
                    reader.skip_bits(8)?;
                }
            }
        }

        Ok(ptl)
    }

    /// Returns human-readable profile name.
    pub fn profile_name(&self) -> &'static str {
        match self.profile_idc {
            1 => "Main",
            2 => "Main 10",
            3 => "Main Still Picture",
            4 => "Range Extensions",
            _ => "Unknown",
        }
    }
}
