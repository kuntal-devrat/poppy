//! Pure Rust bitstream parser for VP9 video uncompressed frame headers and superframes.
//!
//! Provides zero-copy parsing of VP9 bitstreams conforming strictly to the VP9 specification.

pub mod bitstream;
pub mod header;
pub mod superframe;
pub mod tables;

pub use bitstream::{Vp9BitReader, Vp9ParserError};
pub use header::{
    Vp9FrameHeader, Vp9FrameType, Vp9LoopFilter, Vp9Quant, Vp9SegmentFeature, Vp9Segmentation,
};
pub use superframe::parse_superframe_frames;
pub use tables::{
    AC_QLOOKUP, DC_QLOOKUP, calculate_segment_filter_levels, calculate_segment_qmul, clip_u6,
    clip_u8,
};
