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

pub mod bitstream;
pub mod nal;
pub mod pps;
pub mod ptl;
pub mod slice;
pub mod sps;
pub mod vps;

pub use bitstream::{remove_emulation_prevention, BitReader, BitstreamError};
pub use nal::{split_annex_b_nalus, HevcNalHeader, HevcNalType, HevcNalUnit, NalError};
pub use pps::PictureParameterSet;
pub use ptl::ProfileTierLevel;
pub use slice::{HevcSliceType, SliceSegmentHeader};
pub use sps::{SequenceParameterSet, ShortTermRefPicSet};
pub use vps::VideoParameterSet;
