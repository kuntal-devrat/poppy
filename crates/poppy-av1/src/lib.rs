//! Pure Rust AV1 bitstream parser for Open Bitstream Units (OBUs).
//!
//! Complies with the AOMedia Video 1 (AV1) Bitstream & Decoding Process Specification.

pub mod bitstream;
pub mod frame_header;
pub mod leb128;
pub mod obu;
pub mod sequence_header;

pub use bitstream::{BitReader, BitstreamError};
pub use frame_header::{Av1FrameType, DpbState, FrameHeader};
pub use leb128::{Leb128Error, read_leb128};
pub use obu::{ObuHeader, ObuParseError, ObuType, ObuUnit, split_obus};
pub use sequence_header::SequenceHeader;
