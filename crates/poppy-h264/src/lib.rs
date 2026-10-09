//! # poppy-h264
//!
//! Minimal H.264 bitstream parser for hardware decoder configuration.
//!
//! This crate parses NAL units, Sequence Parameter Sets (SPS), Picture
//! Parameter Sets (PPS), and slice headers to extract the exact metadata
//! fields required by hardware decoders (such as VA-API on Linux).

pub mod bitstream;
pub mod nal;
pub mod pps;
pub mod slice_header;
pub mod sps;

pub use bitstream::{remove_emulation_prevention, BitReader, BitstreamError};
pub use nal::{find_annex_b_nals, NalHeader, NalUnit, NalUnitType};
pub use pps::Pps;
pub use slice_header::{SliceHeader, SliceType};
pub use sps::{FrameCropping, Sps};
