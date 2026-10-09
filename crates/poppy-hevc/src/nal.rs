//! H.265 / HEVC Network Abstraction Layer (NAL) unit parser.
//!
//! Implements ITU-T H.265 §7.3.1.2 NAL unit header syntax and Annex B byte stream framing.

use crate::bitstream::remove_emulation_prevention;

/// NAL unit errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NalError {
    #[error("NAL unit data too short: expected at least 2 bytes header, got {0}")]
    TooShort(usize),

    #[error("forbidden_zero_bit is not 0 (malformed NAL header)")]
    ForbiddenZeroBitSet,

    #[error("nuh_temporal_id_plus1 cannot be 0")]
    ZeroTemporalId,
}

/// HEVC NAL Unit Types per ITU-T H.265 Table 7-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HevcNalType {
    /// Coded slice segment of a non-TSA, non-STSA trailing picture (non-reference)
    TrailN = 0,
    /// Coded slice segment of a non-TSA, non-STSA trailing picture (reference)
    TrailR = 1,
    /// Temporal Sub-layer Access (non-reference)
    TsaN = 2,
    /// Temporal Sub-layer Access (reference)
    TsaR = 3,
    /// Step-wise Temporal Sub-layer Access (non-reference)
    StsaN = 4,
    /// Step-wise Temporal Sub-layer Access (reference)
    StsaR = 5,
    /// Random Access Decodable Leading (non-reference)
    RadlN = 6,
    /// Random Access Decodable Leading (reference)
    RadlR = 7,
    /// Random Access Skipped Leading (non-reference)
    RaslN = 8,
    /// Random Access Skipped Leading (reference)
    RaslR = 9,
    /// Broken Link Access with Leading Pictures (IRAP)
    BlaWLp = 16,
    /// Broken Link Access with RADL Pictures (IRAP)
    BlaWRadl = 17,
    /// Broken Link Access without Leading Pictures (IRAP)
    BlaNLp = 18,
    /// Instantaneous Decoder Refresh with RADL Pictures (IRAP / Keyframe)
    IdrWRadl = 19,
    /// Instantaneous Decoder Refresh without Leading Pictures (IRAP / Keyframe)
    IdrNLp = 20,
    /// Clean Random Access (IRAP / Keyframe)
    CraNut = 21,
    /// Video Parameter Set
    VpsNut = 32,
    /// Sequence Parameter Set
    SpsNut = 33,
    /// Picture Parameter Set
    PpsNut = 34,
    /// Access Unit Delimiter
    AudNut = 35,
    /// End of Sequence
    EosNut = 36,
    /// End of Bitstream
    EobNut = 37,
    /// Filler Data
    FdNut = 38,
    /// Prefix Supplemental Enhancement Information
    PrefixSeiNut = 39,
    /// Suffix Supplemental Enhancement Information
    SuffixSeiNut = 40,
    /// Other / Reserved / Unspecified NAL unit type
    Other(u8),
}

impl HevcNalType {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::TrailN,
            1 => Self::TrailR,
            2 => Self::TsaN,
            3 => Self::TsaR,
            4 => Self::StsaN,
            5 => Self::StsaR,
            6 => Self::RadlN,
            7 => Self::RadlR,
            8 => Self::RaslN,
            9 => Self::RaslR,
            16 => Self::BlaWLp,
            17 => Self::BlaWRadl,
            18 => Self::BlaNLp,
            19 => Self::IdrWRadl,
            20 => Self::IdrNLp,
            21 => Self::CraNut,
            32 => Self::VpsNut,
            33 => Self::SpsNut,
            34 => Self::PpsNut,
            35 => Self::AudNut,
            36 => Self::EosNut,
            37 => Self::EobNut,
            38 => Self::FdNut,
            39 => Self::PrefixSeiNut,
            40 => Self::SuffixSeiNut,
            other => Self::Other(other),
        }
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            Self::TrailN => 0,
            Self::TrailR => 1,
            Self::TsaN => 2,
            Self::TsaR => 3,
            Self::StsaN => 4,
            Self::StsaR => 5,
            Self::RadlN => 6,
            Self::RadlR => 7,
            Self::RaslN => 8,
            Self::RaslR => 9,
            Self::BlaWLp => 16,
            Self::BlaWRadl => 17,
            Self::BlaNLp => 18,
            Self::IdrWRadl => 19,
            Self::IdrNLp => 20,
            Self::CraNut => 21,
            Self::VpsNut => 32,
            Self::SpsNut => 33,
            Self::PpsNut => 34,
            Self::AudNut => 35,
            Self::EosNut => 36,
            Self::EobNut => 37,
            Self::FdNut => 38,
            Self::PrefixSeiNut => 39,
            Self::SuffixSeiNut => 40,
            Self::Other(o) => *o,
        }
    }

    /// Returns `true` if this is a VCL (Video Coding Layer) slice NAL unit.
    #[inline]
    pub fn is_vcl(&self) -> bool {
        self.as_u8() <= 31
    }

    /// Returns `true` if this is an IRAP (Intra Random Access Point - Keyframe) NAL unit.
    #[inline]
    pub fn is_irap(&self) -> bool {
        (16..=23).contains(&self.as_u8())
    }

    /// Returns `true` if this is an IDR (Instantaneous Decoder Refresh) picture.
    #[inline]
    pub fn is_idr(&self) -> bool {
        matches!(self, Self::IdrWRadl | Self::IdrNLp)
    }

    /// Returns `true` if this is a CRA (Clean Random Access) picture.
    #[inline]
    pub fn is_cra(&self) -> bool {
        matches!(self, Self::CraNut)
    }

    /// Returns `true` if this is a parameter set (VPS, SPS, or PPS).
    #[inline]
    pub fn is_parameter_set(&self) -> bool {
        matches!(self, Self::VpsNut | Self::SpsNut | Self::PpsNut)
    }
}

/// 2-byte NAL Unit Header (ITU-T H.265 §7.3.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HevcNalHeader {
    /// NAL unit type (6 bits: 0..63).
    pub nal_unit_type: HevcNalType,
    /// Layer identifier (6 bits: 0..62).
    pub nuh_layer_id: u8,
    /// Temporal identifier plus 1 (3 bits: 1..7).
    pub nuh_temporal_id_plus1: u8,
}

impl HevcNalHeader {
    /// Parses the 2-byte HEVC NAL unit header.
    pub fn parse(bytes: &[u8]) -> Result<Self, NalError> {
        if bytes.len() < 2 {
            return Err(NalError::TooShort(bytes.len()));
        }

        let b0 = bytes[0];
        let b1 = bytes[1];

        // Bit 7 of byte 0: forbidden_zero_bit
        if (b0 & 0x80) != 0 {
            return Err(NalError::ForbiddenZeroBitSet);
        }

        // Bits 6..1 of byte 0: nal_unit_type
        let nut_raw = (b0 >> 1) & 0x3F;
        let nal_unit_type = HevcNalType::from_u8(nut_raw);

        // Bit 0 of byte 0 and bits 7..3 of byte 1: nuh_layer_id
        let nuh_layer_id = ((b0 & 0x01) << 5) | ((b1 >> 3) & 0x1F);

        // Bits 2..0 of byte 1: nuh_temporal_id_plus1
        let nuh_temporal_id_plus1 = b1 & 0x07;
        if nuh_temporal_id_plus1 == 0 {
            return Err(NalError::ZeroTemporalId);
        }

        Ok(Self {
            nal_unit_type,
            nuh_layer_id,
            nuh_temporal_id_plus1,
        })
    }
}

/// Borrowed reference to an in-memory HEVC NAL unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HevcNalUnit<'a> {
    pub header: HevcNalHeader,
    /// Entire NAL unit data including the 2-byte header.
    pub data: &'a [u8],
}

impl<'a> HevcNalUnit<'a> {
    /// Creates a NAL unit from a slice beginning with the 2-byte header.
    pub fn parse(data: &'a [u8]) -> Result<Self, NalError> {
        let header = HevcNalHeader::parse(data)?;
        Ok(Self { header, data })
    }

    /// Returns the NAL payload slice following the 2-byte header.
    #[inline]
    pub fn payload(&self) -> &'a [u8] {
        if self.data.len() > 2 {
            &self.data[2..]
        } else {
            &[]
        }
    }

    /// Extracts the Raw Byte Sequence Payload (RBSP), removing emulation prevention bytes.
    #[inline]
    pub fn rbsp(&self) -> Vec<u8> {
        remove_emulation_prevention(self.payload())
    }
}

/// Splits an Annex B byte stream buffer into HEVC NAL units.
///
/// Handles both 3-byte (`0x00 0x00 0x01`) and 4-byte (`0x00 0x00 0x00 0x01`) start codes.
pub fn split_annex_b_nalus(buffer: &[u8]) -> Vec<HevcNalUnit<'_>> {
    let mut units = Vec::new();
    let len = buffer.len();
    if len < 4 {
        return units;
    }

    let mut start_indices = Vec::new();
    let mut i = 0;

    while i + 2 < len {
        if buffer[i] == 0 && buffer[i + 1] == 0 {
            if buffer[i + 2] == 1 {
                // 3-byte start code
                start_indices.push((i, i + 3));
                i += 3;
                continue;
            } else if buffer[i + 2] == 0 && i + 3 < len && buffer[i + 3] == 1 {
                // 4-byte start code
                start_indices.push((i, i + 4));
                i += 4;
                continue;
            }
        }
        i += 1;
    }

    for (k, &(_sc_start, nalu_start)) in start_indices.iter().enumerate() {
        let nalu_end = if k + 1 < start_indices.len() {
            start_indices[k + 1].0
        } else {
            len
        };

        if nalu_start < nalu_end {
            let nalu_data = &buffer[nalu_start..nalu_end];
            if let Ok(nalu) = HevcNalUnit::parse(nalu_data) {
                units.push(nalu);
            }
        }
    }

    units
}
