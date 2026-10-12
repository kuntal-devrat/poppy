//! # poppy-hevc
//!
//! High-performance, zero-allocation H.265 / HEVC bitstream parser for Project Poppy.
//!
//! Compliant with ITU-T Recommendation H.265 (ISO/IEC 23008-2 High Efficiency Video Coding):
//! - NAL unit parsing and Annex-B stream splitting (§7.3.1)
//! - Video Parameter Set (VPS) parsing (§7.3.2.1)
//! - Sequence Parameter Set (SPS) parsing (§7.3.2.2)
//! - Picture Parameter Set (PPS) parsing (§7.3.2.3)
//! - Slice Segment Header parsing (§7.3.6.1)
//! - Profile, Tier, and Level parsing (§7.3.7.3)
//! - 8-bit and 10-bit HDR format detection and crop rectangle resolution
//!
//! ## Example
//!
//! ```
//! use poppy_hevc::{HevcNalHeader, HevcNalType};
//!
//! // VPS NAL unit header: type = 32 (VpsNut), layer_id = 0, temporal_id_plus1 = 1
//! let header_bytes = [0x40, 0x01];
//! let header = HevcNalHeader::parse(&header_bytes).expect("valid HEVC NAL header");
//! assert_eq!(header.nal_unit_type, HevcNalType::VpsNut);
//! assert_eq!(header.nuh_layer_id, 0);
//! assert_eq!(header.nuh_temporal_id_plus1, 1);
//! ```

pub mod bitstream;
pub mod nal;
pub mod pps;
pub mod ptl;
pub mod slice;
pub mod sps;
pub mod vps;

pub use bitstream::{BitReader, BitstreamError, remove_emulation_prevention};
pub use nal::{HevcNalHeader, HevcNalType, HevcNalUnit, NalError, split_annex_b_nalus};
pub use pps::PictureParameterSet;
pub use ptl::ProfileTierLevel;
pub use slice::{HevcSliceType, SliceSegmentHeader};
pub use sps::{SequenceParameterSet, ShortTermRefPicSet};
pub use vps::VideoParameterSet;
