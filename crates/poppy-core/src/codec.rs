//! Codec identification and codec-specific configuration records.
//!
//! These types represent the metadata extracted from container-level codec
//! configuration boxes (e.g., `avcC` for H.264, `hvcC` for HEVC, `av1C` for AV1).
//! They do NOT contain decoded pixel data — only the parameters needed to
//! initialize a hardware or software decoder.

use bytes::Bytes;

/// Identifies the codec used for a media stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecId {
    // Video
    H264,
    Hevc,
    Av1,
    Vp9,

    // Audio
    Aac,
    Opus,
    Mp3,
    Flac,
    Vorbis,

    /// Unknown or unsupported codec. The `u32` is the raw FourCC or codec tag.
    Unknown(u32),
}

impl CodecId {
    /// Attempt to identify a codec from an ISOBMFF sample entry FourCC.
    ///
    /// Reference: ISO 14496-12 §8.5.2, ISO 14496-15 §5.3.4
    ///
    /// Note: `mp4a` is ambiguous (AAC, MP3, AC-3, ALAC all use `mp4a` with
    /// different `esds` object types). This maps to AAC by default; use
    /// [`CodecId::from_mp4_fourcc_with_esds`] when the `esds` object type is
    /// available.
    pub fn from_mp4_fourcc(fourcc: &[u8; 4]) -> Self {
        match fourcc {
            b"avc1" | b"avc3" => CodecId::H264,
            b"hev1" | b"hvc1" => CodecId::Hevc,
            b"av01" => CodecId::Av1,
            b"vp09" => CodecId::Vp9,
            b"mp4a" => CodecId::Aac, // ambiguous; refine via esds when available
            b"Opus" => CodecId::Opus,
            b".mp3" | b"mp3 " => CodecId::Mp3,
            b"fLaC" => CodecId::Flac,
            b"vorb" | b"vor1" => CodecId::Vorbis,
            other => {
                let raw = u32::from_be_bytes(*other);
                CodecId::Unknown(raw)
            }
        }
    }

    /// Refine `mp4a` using the `esds` DecoderConfig `objectTypeIndication`.
    /// `0x40` = AAC, `0x66/0x67/0x69` = MP3, `0xA5` = AC-3 (unsupported),
    /// `0xAC` = Opus-ish, others fall back to `from_mp4_fourcc`.
    pub fn from_mp4_fourcc_with_esds(fourcc: &[u8; 4], object_type: Option<u8>) -> Self {
        if fourcc != b"mp4a" {
            return Self::from_mp4_fourcc(fourcc);
        }
        match object_type {
            Some(0x40) => CodecId::Aac,
            Some(0x66) | Some(0x67) | Some(0x69) => CodecId::Mp3,
            Some(0xA5) | Some(0xA6) => CodecId::Unknown(0x61332D63), // "ac-3"
            _ => CodecId::Aac,
        }
    }

    /// Attempt to identify a codec from a Matroska CodecID string.
    ///
    /// Unknown IDs are hashed with FNV-1a into `Unknown(u32)` so distinct
    /// strings (e.g. `V_VP8` vs `S_TEXT/UTF8`) remain distinguishable instead
    /// of collapsing to `Unknown(0)`.
    ///
    /// Reference: Matroska specification, §8.1 Codec ID
    pub fn from_matroska_codec_id(id: &str) -> Self {
        match id {
            "V_MPEG4/ISO/AVC" => CodecId::H264,
            "V_MPEGH/ISO/HEVC" => CodecId::Hevc,
            "V_AV1" => CodecId::Av1,
            "V_VP9" => CodecId::Vp9,
            "A_AAC" | "A_AAC/MPEG4/LC" | "A_AAC/MPEG4/LC/SBR" => CodecId::Aac,
            "A_OPUS" => CodecId::Opus,
            "A_MPEG/L3" => CodecId::Mp3,
            "A_FLAC" => CodecId::Flac,
            "A_VORBIS" => CodecId::Vorbis,
            _ => CodecId::Unknown(fnv1a32(id)),
        }
    }

    /// Returns a human-readable name for this codec.
    pub fn name(self) -> &'static str {
        match self {
            CodecId::H264 => "H.264 (AVC)",
            CodecId::Hevc => "H.265 (HEVC)",
            CodecId::Av1 => "AV1",
            CodecId::Vp9 => "VP9",
            CodecId::Aac => "AAC",
            CodecId::Opus => "Opus",
            CodecId::Mp3 => "MP3",
            CodecId::Flac => "FLAC",
            CodecId::Vorbis => "Vorbis",
            CodecId::Unknown(_) => "Unknown",
        }
    }
}

impl std::fmt::Display for CodecId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// FNV-1a 32-bit hash for unknown Matroska CodecIDs so distinct strings do
/// not collapse to the same `Unknown(0)`.
fn fnv1a32(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    // Avoid 0 so "unknown, hashed" is distinguishable from legacy Unknown(0).
    if h == 0 { 1 } else { h }
}

/// Codec-specific configuration extracted from the container.
///
/// Each variant holds the parsed contents of the corresponding codec
/// configuration box or element.
#[derive(Debug, Clone)]
pub enum CodecConfig {
    /// H.264 (AVC) configuration from the `avcC` box.
    H264(AvcCRecord),
    /// H.265 (HEVC) configuration from the `hvcC` box.
    Hevc(HvcCRecord),
    /// AV1 configuration from the `av1C` box.
    Av1(Av1CRecord),
    /// VP9 configuration from the `vpcC` box.
    Vp9(VpcCRecord),
    /// Audio stream configuration.
    Audio(AudioConfig),
    /// Unknown or not yet parsed.
    Unknown,
}

/// Parsed contents of the `avcC` box (ISO 14496-15, §5.3.3.1.2).
///
/// This record contains the decoder initialization parameters for H.264 streams
/// stored in ISOBMFF containers. The SPS and PPS NAL units are stored here
/// (out-of-band) rather than inline in the bitstream.
///
/// # NAL Length Size
///
/// In AVCC format, each NAL unit in the `mdat` samples is prefixed by a
/// big-endian integer indicating the NAL unit's byte length. The size of
/// this prefix is determined by `nal_length_size` (derived from the 2-bit
/// field `lengthSizeMinusOne`):
///
/// | `lengthSizeMinusOne` | `nal_length_size` | Notes |
/// | :--- | :--- | :--- |
/// | 0 | 1 byte | Max NAL size: 255 bytes |
/// | 1 | 2 bytes | Max NAL size: 65,535 bytes |
/// | 2 | *(reserved)* | Must not appear in conformant streams |
/// | 3 | 4 bytes | Max NAL size: ~4 GB (most common) |
#[derive(Debug, Clone)]
pub struct AvcCRecord {
    /// AVC profile indication (e.g., 66 = Baseline, 77 = Main, 100 = High).
    pub profile_idc: u8,

    /// Profile compatibility flags.
    pub profile_compatibility: u8,

    /// AVC level indication (e.g., 31 = Level 3.1, 40 = Level 4.0).
    pub level_idc: u8,

    /// Number of bytes used for NAL unit length prefixes in samples.
    /// Derived from `lengthSizeMinusOne + 1`. Valid values: 1, 2, or 4.
    pub nal_length_size: u8,

    /// Sequence Parameter Set NAL units (without start codes or length prefixes).
    pub sps_list: Vec<Bytes>,

    /// Picture Parameter Set NAL units (without start codes or length prefixes).
    pub pps_list: Vec<Bytes>,

    /// Raw unparsed avcC box payload bytes.
    pub raw: Bytes,
}

impl AvcCRecord {
    /// Return the raw byte representation of the avcC record.
    #[inline]
    pub fn raw_data(&self) -> &[u8] {
        &self.raw
    }
    /// Parse an `avcC` box payload from raw bytes.
    ///
    /// The input should be the box body (after the box header), starting
    /// with `configurationVersion`.
    ///
    /// Layout (ISO 14496-15, §5.3.3.1.2):
    /// ```text
    /// u8   configurationVersion  (must be 1)
    /// u8   AVCProfileIndication
    /// u8   profile_compatibility
    /// u8   AVCLevelIndication
    /// u8   lengthSizeMinusOne    (lower 2 bits, upper 6 bits reserved = 0b111111)
    /// u8   numOfSequenceParameterSets (lower 5 bits, upper 3 bits reserved = 0b111)
    /// for each SPS:
    ///   u16  sequenceParameterSetLength
    ///   u8[] sequenceParameterSetNALUnit
    /// u8   numOfPictureParameterSets
    /// for each PPS:
    ///   u16  pictureParameterSetLength
    ///   u8[] pictureParameterSetNALUnit
    /// ```
    pub fn parse(data: &[u8]) -> Result<Self, AvcCParseError> {
        if data.len() < 7 {
            return Err(AvcCParseError::TooShort {
                need: 7,
                have: data.len(),
            });
        }

        let version = data[0];
        if version != 1 {
            return Err(AvcCParseError::UnsupportedVersion(version));
        }

        let profile_idc = data[1];
        let profile_compatibility = data[2];
        let level_idc = data[3];

        // Lower 2 bits of byte 4
        let length_size_minus_one = data[4] & 0x03;
        let nal_length_size = length_size_minus_one + 1;
        // Value 2 is reserved per spec
        if nal_length_size == 3 {
            return Err(AvcCParseError::ReservedNalLengthSize);
        }

        let mut offset = 5;

        // Parse SPS list
        let num_sps = (data[offset] & 0x1F) as usize; // lower 5 bits
        offset += 1;

        let mut sps_list = Vec::with_capacity(num_sps);
        for _ in 0..num_sps {
            if offset + 2 > data.len() {
                return Err(AvcCParseError::TooShort {
                    need: offset + 2,
                    have: data.len(),
                });
            }
            let sps_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            if offset + sps_len > data.len() {
                return Err(AvcCParseError::TooShort {
                    need: offset + sps_len,
                    have: data.len(),
                });
            }
            sps_list.push(Bytes::copy_from_slice(&data[offset..offset + sps_len]));
            offset += sps_len;
        }

        // Parse PPS list
        if offset >= data.len() {
            return Err(AvcCParseError::TooShort {
                need: offset + 1,
                have: data.len(),
            });
        }
        let num_pps = data[offset] as usize;
        offset += 1;

        let mut pps_list = Vec::with_capacity(num_pps);
        for _ in 0..num_pps {
            if offset + 2 > data.len() {
                return Err(AvcCParseError::TooShort {
                    need: offset + 2,
                    have: data.len(),
                });
            }
            let pps_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;

            if offset + pps_len > data.len() {
                return Err(AvcCParseError::TooShort {
                    need: offset + pps_len,
                    have: data.len(),
                });
            }
            pps_list.push(Bytes::copy_from_slice(&data[offset..offset + pps_len]));
            offset += pps_len;
        }

        let raw = Bytes::copy_from_slice(data);
        Ok(Self {
            profile_idc,
            profile_compatibility,
            level_idc,
            nal_length_size,
            sps_list,
            pps_list,
            raw,
        })
    }
}

/// Errors when parsing an `avcC` record.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AvcCParseError {
    #[error("avcC data too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },

    #[error("unsupported avcC configuration version: {0} (expected 1)")]
    UnsupportedVersion(u8),

    #[error("reserved NAL length size value (lengthSizeMinusOne = 2)")]
    ReservedNalLengthSize,
}

/// Errors when parsing an `hvcC` record.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum HvcCParseError {
    #[error("hvcC data too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },

    #[error("unsupported hvcC configuration version: {0} (expected 1)")]
    UnsupportedVersion(u8),
}

/// Parsed contents of the `hvcC` box (ISO/IEC 14496-15 §8.3.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HvcCRecord {
    pub profile_space: u8,
    pub tier_flag: bool,
    pub profile_idc: u8,
    pub profile_compatibility: u32,
    pub level_idc: u8,
    pub chroma_format_idc: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub nal_length_size: u8,
    pub vps_list: Vec<Bytes>,
    pub sps_list: Vec<Bytes>,
    pub pps_list: Vec<Bytes>,
    pub raw: Bytes,
}

impl HvcCRecord {
    /// Return the raw byte representation of the hvcC record.
    #[inline]
    pub fn raw_data(&self) -> &[u8] {
        &self.raw
    }

    /// Parses an `hvcC` box payload from bytes.
    pub fn parse(data: &[u8]) -> Result<Self, HvcCParseError> {
        if data.len() < 23 {
            return Err(HvcCParseError::TooShort {
                need: 23,
                have: data.len(),
            });
        }

        let configuration_version = data[0];
        if configuration_version != 1 {
            return Err(HvcCParseError::UnsupportedVersion(configuration_version));
        }

        let profile_space = (data[1] >> 6) & 0x03;
        let tier_flag = (data[1] & 0x20) != 0;
        let profile_idc = data[1] & 0x1F;
        let profile_compatibility = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
        let level_idc = data[12];
        let chroma_format_idc = data[16] & 0x03;
        let bit_depth_luma = (data[17] & 0x07) + 8;
        let bit_depth_chroma = (data[18] & 0x07) + 8;
        let nal_length_size = (data[21] & 0x03) + 1;
        let num_arrays = data[22] as usize;

        let mut vps_list = Vec::new();
        let mut sps_list = Vec::new();
        let mut pps_list = Vec::new();

        let mut offset = 23;
        for _ in 0..num_arrays {
            if offset + 3 > data.len() {
                return Err(HvcCParseError::TooShort {
                    need: offset + 3,
                    have: data.len(),
                });
            }

            let nal_type = data[offset] & 0x3F;
            let num_nalus = u16::from_be_bytes([data[offset + 1], data[offset + 2]]) as usize;
            offset += 3;

            for _ in 0..num_nalus {
                if offset + 2 > data.len() {
                    return Err(HvcCParseError::TooShort {
                        need: offset + 2,
                        have: data.len(),
                    });
                }

                let nal_len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;

                if offset + nal_len > data.len() {
                    return Err(HvcCParseError::TooShort {
                        need: offset + nal_len,
                        have: data.len(),
                    });
                }

                let nal_bytes = Bytes::copy_from_slice(&data[offset..offset + nal_len]);
                match nal_type {
                    32 => vps_list.push(nal_bytes),
                    33 => sps_list.push(nal_bytes),
                    34 => pps_list.push(nal_bytes),
                    _ => {}
                }
                offset += nal_len;
            }
        }

        Ok(Self {
            profile_space,
            tier_flag,
            profile_idc,
            profile_compatibility,
            level_idc,
            chroma_format_idc,
            bit_depth_luma,
            bit_depth_chroma,
            nal_length_size,
            vps_list,
            sps_list,
            pps_list,
            raw: Bytes::copy_from_slice(data),
        })
    }

    /// Returns `true` if this stream has 10-bit color depth.
    pub fn is_10bit(&self) -> bool {
        self.bit_depth_luma >= 10
    }
}

/// Errors when parsing an `av1C` record.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum Av1CParseError {
    #[error("av1C data too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("unsupported av1C version: {0} (expected 1)")]
    UnsupportedVersion(u8),
}

/// Parsed contents of the `av1C` box (AV1 Codec ISO Media File Format Binding, §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1CRecord {
    pub marker: bool,
    pub version: u8,
    pub seq_profile: u8,
    pub seq_level_idx: u8,
    pub seq_tier: bool,
    pub high_bitdepth: bool,
    pub twelve_bit: bool,
    pub monochrome: bool,
    pub chroma_subsampling_x: bool,
    pub chroma_subsampling_y: bool,
    pub chroma_sample_position: u8,
    pub initial_presentation_delay_present: bool,
    pub initial_presentation_delay_minus_one: u8,
    pub config_obus: Bytes,
    pub raw: Bytes,
}

impl Av1CRecord {
    /// Parses an `av1C` box payload from raw bytes.
    pub fn parse(data: &[u8]) -> Result<Self, Av1CParseError> {
        if data.len() < 4 {
            return Err(Av1CParseError::TooShort {
                need: 4,
                have: data.len(),
            });
        }

        let marker = ((data[0] >> 7) & 1) != 0;
        let version = data[0] & 0x7F;
        if version != 1 {
            return Err(Av1CParseError::UnsupportedVersion(version));
        }

        let seq_profile = (data[1] >> 5) & 0x07;
        let seq_level_idx = data[1] & 0x1F;

        let seq_tier = ((data[2] >> 7) & 1) != 0;
        let high_bitdepth = ((data[2] >> 6) & 1) != 0;
        let twelve_bit = ((data[2] >> 5) & 1) != 0;
        let monochrome = ((data[2] >> 4) & 1) != 0;
        let chroma_subsampling_x = ((data[2] >> 3) & 1) != 0;
        let chroma_subsampling_y = ((data[2] >> 2) & 1) != 0;
        let chroma_sample_position = data[2] & 0x03;

        let initial_presentation_delay_present = ((data[3] >> 4) & 1) != 0;
        let initial_presentation_delay_minus_one = if initial_presentation_delay_present {
            data[3] & 0x0F
        } else {
            0
        };

        let config_obus = if data.len() > 4 {
            Bytes::copy_from_slice(&data[4..])
        } else {
            Bytes::new()
        };

        Ok(Self {
            marker,
            version,
            seq_profile,
            seq_level_idx,
            seq_tier,
            high_bitdepth,
            twelve_bit,
            monochrome,
            chroma_subsampling_x,
            chroma_subsampling_y,
            chroma_sample_position,
            initial_presentation_delay_present,
            initial_presentation_delay_minus_one,
            config_obus,
            raw: Bytes::copy_from_slice(data),
        })
    }

    /// Returns `true` if this AV1 stream is 10-bit or 12-bit.
    /// Per AV1-ISOBMFF binding: `high_bitdepth==1` means 10-bit, and
    /// `twelve_bit==1` implies 12-bit (which is >= 10-bit for pipeline purposes).
    pub fn is_10bit(&self) -> bool {
        self.high_bitdepth || self.twelve_bit
    }

    /// Exact bit depth: 8, 10, or 12.
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

/// Errors when parsing a `vpcC` record.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum VpcCParseError {
    #[error("vpcC data too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("unsupported vpcC version: {0} (expected 1)")]
    UnsupportedVersion(u8),
}

/// Parsed contents of the `vpcC` box (VP Codec ISO Media File Format Binding, §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VpcCRecord {
    pub profile: u8,
    pub level: u8,
    pub bit_depth: u8,
    pub chroma_subsampling: u8,
    pub video_full_range_flag: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub raw: Bytes,
}

impl VpcCRecord {
    /// Return the raw byte representation of the vpcC record.
    #[inline]
    pub fn raw_data(&self) -> &[u8] {
        &self.raw
    }

    /// Parses a `vpcC` box payload from raw bytes.
    ///
    /// The VP Codec ISO binding defines `vpcC` as a FullBox (1B version +
    /// 3B flags, all zero except version==1). Some WebM-derived muxers store
    /// the raw 8-byte record without the FullBox header. We detect FullBox
    /// strictly: `len >= 12` (or 10 for the minimal form), `data[0] <= 1`
    /// (version) and `data[1..4] == [0,0,0]` (flags). The previous heuristic
    /// (`data[0]==0||1` alone) misclassified raw records starting with
    /// profile 0/1 as FullBox and parsed garbage.
    pub fn parse(data: &[u8]) -> Result<Self, VpcCParseError> {
        let is_fullbox = data.len() >= 10
            && data[0] <= 1
            && data.len() >= 4
            && data[1] == 0
            && data[2] == 0
            && data[3] == 0;
        let (offset, _version) = if is_fullbox {
            // FullBox: 1 byte version + 3 bytes flags
            if data[0] != 0 && data[0] != 1 {
                return Err(VpcCParseError::UnsupportedVersion(data[0]));
            }
            (4, data[0])
        } else if data.len() >= 6 {
            // Direct record (6 bytes: profile/level/etc.)
            (0, 1)
        } else {
            return Err(VpcCParseError::TooShort {
                need: 6,
                have: data.len(),
            });
        };

        if data.len() < offset + 6 {
            return Err(VpcCParseError::TooShort {
                need: offset + 6,
                have: data.len(),
            });
        }

        let profile = data[offset];
        let level = data[offset + 1];
        let byte2 = data[offset + 2];
        let bit_depth = (byte2 >> 4) & 0x0F;
        let chroma_subsampling = (byte2 >> 1) & 0x07;
        let video_full_range_flag = (byte2 & 0x01) != 0;

        let colour_primaries = data[offset + 3];
        let transfer_characteristics = data[offset + 4];
        let matrix_coefficients = data[offset + 5];

        Ok(Self {
            profile,
            level,
            bit_depth,
            chroma_subsampling,
            video_full_range_flag,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            raw: Bytes::copy_from_slice(data),
        })
    }

    /// Returns `true` if this VP9 stream has 10-bit or 12-bit depth.
    /// Trusts the signalled `bit_depth` field. Profile is informational only
    /// (profile 2/3 *usually* means 10/12-bit, but a lying header must not
    /// force a 10-bit pipeline).
    pub fn is_10bit(&self) -> bool {
        self.bit_depth >= 10
    }
}

/// Audio stream configuration extracted from the container.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Sample rate in Hz (e.g., 44100, 48000).
    pub sample_rate: u32,

    /// Number of audio channels.
    pub channels: u16,

    /// Bits per sample (e.g., 16, 24). 0 if not applicable (compressed codecs).
    pub bits_per_sample: u16,

    /// Codec-specific initialization data (e.g., AudioSpecificConfig for AAC).
    pub extradata: Option<Bytes>,
}

/// Audio sample format of decoded PCM data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioSampleFormat {
    /// 32-bit floating point, planar (channel-by-channel).
    F32Planar,
    /// 32-bit floating point, interleaved (L, R, L, R...).
    F32Interleaved,
    /// 16-bit signed integer, interleaved.
    I16Interleaved,
    /// 32-bit signed integer, interleaved.
    I32Interleaved,
}

/// Channel layout configuration for audio streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    Mono,
    Stereo,
    Surround51,
    Surround71,
    Custom(u16),
}

impl ChannelLayout {
    pub fn channels(&self) -> u16 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Surround51 => 6,
            Self::Surround71 => 8,
            Self::Custom(c) => *c,
        }
    }

    pub fn from_channels(channels: u16) -> Self {
        match channels {
            1 => Self::Mono,
            2 => Self::Stereo,
            6 => Self::Surround51,
            8 => Self::Surround71,
            other => Self::Custom(other),
        }
    }
}

/// A decoded audio frame containing uncompressed PCM audio samples.
///
/// `samples` layout must match `format`: interleaved formats store
/// `L,R,L,R...`, planar formats store all samples of channel 0, then channel
/// 1, etc. Constructors must uphold
/// `samples.len() == num_samples_per_channel * channels as usize`.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    /// Presentation timestamp in container timebase units, if available.
    pub pts: Option<i64>,
    /// Sample rate in Hertz (e.g. 44100, 48000).
    pub sample_rate: u32,
    /// Number of audio channels.
    pub channels: u16,
    /// Channel layout.
    pub channel_layout: ChannelLayout,
    /// Audio sample representation format.
    pub format: AudioSampleFormat,
    /// Normalized floating point samples [-1.0, 1.0] in `format` layout.
    pub samples: Vec<f32>,
    /// Number of audio samples per channel.
    pub num_samples_per_channel: usize,
}

impl AudioFrame {
    /// Validate invariants; returns an error string on mismatch.
    pub fn validate(&self) -> Result<(), String> {
        let expected = self
            .num_samples_per_channel
            .saturating_mul(self.channels as usize);
        if self.samples.len() != expected {
            return Err(format!(
                "AudioFrame samples len {} != {}x{} ({expected})",
                self.samples.len(),
                self.num_samples_per_channel,
                self.channels
            ));
        }
        if self.sample_rate == 0 {
            return Err("AudioFrame sample_rate is 0".into());
        }
        if self.channels == 0 {
            return Err("AudioFrame channels is 0".into());
        }
        Ok(())
    }

    /// Calculate the duration of this audio frame in seconds.
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.num_samples_per_channel as f64 / self.sample_rate as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_id_from_mp4_fourcc() {
        assert_eq!(CodecId::from_mp4_fourcc(b"avc1"), CodecId::H264);
        assert_eq!(CodecId::from_mp4_fourcc(b"avc3"), CodecId::H264);
        assert_eq!(CodecId::from_mp4_fourcc(b"hev1"), CodecId::Hevc);
        assert_eq!(CodecId::from_mp4_fourcc(b"av01"), CodecId::Av1);
        assert_eq!(CodecId::from_mp4_fourcc(b"mp4a"), CodecId::Aac);
        assert_eq!(CodecId::from_mp4_fourcc(b"Opus"), CodecId::Opus);
    }

    #[test]
    fn codec_id_from_matroska() {
        assert_eq!(
            CodecId::from_matroska_codec_id("V_MPEG4/ISO/AVC"),
            CodecId::H264
        );
        assert_eq!(
            CodecId::from_matroska_codec_id("V_MPEGH/ISO/HEVC"),
            CodecId::Hevc
        );
        assert_eq!(CodecId::from_matroska_codec_id("A_OPUS"), CodecId::Opus);
    }

    #[test]
    fn codec_id_display() {
        assert_eq!(format!("{}", CodecId::H264), "H.264 (AVC)");
        assert_eq!(format!("{}", CodecId::Opus), "Opus");
    }

    /// Test parsing a minimal valid avcC record.
    ///
    /// This is a hand-constructed avcC with:
    /// - version=1, profile=100 (High), compat=0, level=31
    /// - nal_length_size=4 (lengthSizeMinusOne=3)
    /// - 1 SPS of 4 bytes: [0x67, 0x64, 0x00, 0x1F]
    /// - 1 PPS of 4 bytes: [0x68, 0xEB, 0xE3, 0xCB]
    #[test]
    fn parse_avcc_basic() {
        let data: Vec<u8> = vec![
            0x01, // configurationVersion
            0x64, // AVCProfileIndication (High)
            0x00, // profile_compatibility
            0x1F, // AVCLevelIndication (3.1)
            0xFF, // 6 reserved bits (111111) + lengthSizeMinusOne=3 → nal_length_size=4
            0xE1, // 3 reserved bits (111) + numSPS=1
            0x00, 0x04, // SPS length = 4
            0x67, 0x64, 0x00, 0x1F, // SPS NAL unit bytes
            0x01, // numPPS = 1
            0x00, 0x04, // PPS length = 4
            0x68, 0xEB, 0xE3, 0xCB, // PPS NAL unit bytes
        ];

        let record = AvcCRecord::parse(&data).expect("should parse successfully");
        assert_eq!(record.profile_idc, 100);
        assert_eq!(record.level_idc, 31);
        assert_eq!(record.nal_length_size, 4);
        assert_eq!(record.sps_list.len(), 1);
        assert_eq!(record.pps_list.len(), 1);
        assert_eq!(&record.sps_list[0][..], &[0x67, 0x64, 0x00, 0x1F]);
        assert_eq!(&record.pps_list[0][..], &[0x68, 0xEB, 0xE3, 0xCB]);
    }

    #[test]
    fn parse_avcc_nal_length_size_1() {
        let data: Vec<u8> = vec![
            0x01, 0x42, 0x00, 0x1E, // version, Baseline profile, level 3.0
            0xFC, // reserved(111111) + lengthSizeMinusOne=0 → nal_length_size=1
            0xE1, // reserved(111) + numSPS=1
            0x00, 0x02, // SPS length
            0x67, 0x42, // SPS data
            0x01, // numPPS=1
            0x00, 0x02, // PPS length
            0x68, 0xCE, // PPS data
        ];

        let record = AvcCRecord::parse(&data).expect("should parse");
        assert_eq!(record.nal_length_size, 1);
    }

    #[test]
    fn parse_avcc_reserved_length_size_rejected() {
        let data: Vec<u8> = vec![
            0x01, 0x64, 0x00, 0x1F, // version, High profile, level 3.1
            0xFE, // reserved(111111) + lengthSizeMinusOne=2 → RESERVED
            0xE0, // reserved(111) + numSPS=0
            0x00, // numPPS=0
        ];

        let result = AvcCRecord::parse(&data);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AvcCParseError::ReservedNalLengthSize
        ));
    }

    #[test]
    fn parse_avcc_too_short() {
        let result = AvcCRecord::parse(&[0x01, 0x64, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_av1c_basic() {
        let data: Vec<u8> = vec![
            0x81, // marker=1, version=1
            0x00, // seq_profile=0, seq_level_idx_0=0
            0x0C, // tier=0, high_bitdepth=0, twelve_bit=0, mono=0, subsampling_x=1, subsampling_y=1, chroma_sample_pos=0
            0x00, // delay_present=0, delay_minus_one=0
            0x12, 0x00, // configOBUs: Temporal delimiter OBU
        ];

        let record = Av1CRecord::parse(&data).expect("should parse av1C");
        assert_eq!(record.version, 1);
        assert_eq!(record.seq_profile, 0);
        assert!(!record.is_10bit());
        assert_eq!(record.config_obus.len(), 2);
    }

    #[test]
    fn parse_vpcc_basic() {
        let data: Vec<u8> = vec![
            0x01, 0x00, 0x00, 0x00, // FullBox version 1, flags 0
            0x00, // profile 0
            0x1F, // level 3.1 (31)
            0x82, // bit_depth=8 (1000b), subsampling=420 (001b), full_range=0 (0b)
            0x01, // primaries = bt709
            0x01, // transfer = bt709
            0x01, // matrix = bt709
        ];

        let record = VpcCRecord::parse(&data).expect("should parse vpcC");
        assert_eq!(record.profile, 0);
        assert_eq!(record.level, 31);
        assert_eq!(record.bit_depth, 8);
        assert_eq!(record.chroma_subsampling, 1);
        assert!(!record.video_full_range_flag);
        assert!(!record.is_10bit());
    }
}
