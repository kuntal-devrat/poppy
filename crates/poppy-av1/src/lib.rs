//! Pure Rust AV1 bitstream parser for Open Bitstream Units (OBUs).
//!
//! Complies with the AOMedia Video 1 (AV1) Bitstream & Decoding Process Specification.
//!
//! ## Example
//!
//! ```
//! use poppy_av1::read_leb128;
//!
//! let data = [0x7f];
//! let (val, bytes_read) = read_leb128(&data).expect("valid LEB128");
//! assert_eq!(val, 127);
//! assert_eq!(bytes_read, 1);
//!
//! let multi_byte = [0x80, 0x01];
//! let (val, bytes_read) = read_leb128(&multi_byte).expect("valid LEB128");
//! assert_eq!(val, 128);
//! assert_eq!(bytes_read, 2);
//! ```

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
