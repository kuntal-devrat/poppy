//! # poppy-h264
//!
//! Minimal H.264 bitstream parser for hardware decoder configuration.
//!
//! This crate parses NAL units, Sequence Parameter Sets (SPS), Picture
//! Parameter Sets (PPS), and slice headers to extract the exact metadata
//! fields required by hardware decoders (such as VA-API on Linux).
//!
//! ## Example
//!
//! ```
//! use poppy_h264::{find_annex_b_nals, NalUnit, NalUnitType};
//!
//! let data = [0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x0a];
//! let nals = find_annex_b_nals(&data);
//! assert_eq!(nals.len(), 1);
//!
//! let nal = NalUnit::parse(nals[0]).expect("valid NAL unit");
//! assert_eq!(nal.header.unit_type, NalUnitType::Sps);
//! ```

pub mod bitstream;
pub mod nal;
pub mod pps;
pub mod slice_header;
pub mod sps;

pub use bitstream::{BitReader, BitstreamError, remove_emulation_prevention};
pub use nal::{NalHeader, NalUnit, NalUnitType, find_annex_b_nals};
pub use pps::Pps;
pub use slice_header::{PredWeightTable, RefPicListModification, SliceHeader, SliceType};
pub use sps::{FrameCropping, Sps};
