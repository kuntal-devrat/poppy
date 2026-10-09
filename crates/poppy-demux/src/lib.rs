//! # poppy-demux
//!
//! Pure safe-Rust container demuxers for the Poppy media pipeline.
//!
//! This crate provides parsers for:
//! - **ISOBMFF (MP4)** — ISO 14496-12 / ISO 14496-14 containers
//! - **EBML (Matroska / WebM)** — RFC 8794 / Matroska specification
//!
//! Both parsers are designed for zero-copy operation over memory-mapped files
//! or streaming byte sources.

pub mod bsf;
pub mod ebml;
pub mod isobmff;

pub use bsf::{avcc_to_annex_b, hevc_to_annex_b};
pub use ebml::MatroskaDemuxer;
pub use isobmff::Mp4Demuxer;

use poppy_core::{Packet, StreamInfo};

/// Errors from demuxing operations.
#[derive(Debug, thiserror::Error)]
pub enum DemuxError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid box at offset {offset:#x}: {detail}")]
    InvalidBox { offset: u64, detail: String },

    #[error("required box '{fourcc}' not found")]
    BoxNotFound { fourcc: String },

    #[error("unsupported container format")]
    UnsupportedFormat,

    #[error("unexpected end of data at offset {offset:#x}")]
    UnexpectedEof { offset: u64 },

    #[error("sample index {index} out of range (total: {total})")]
    SampleOutOfRange { index: u32, total: u32 },
}

/// Trait implemented by all container demuxers.
pub trait Demuxer: Send {
    /// Returns metadata for all streams (tracks) in the container.
    fn streams(&self) -> &[StreamInfo];

    /// Read the next packet from the container.
    ///
    /// Returns `Ok(None)` at end-of-stream.
    /// Packets are returned in decode order (DTS-sorted).
    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError>;

    /// Seek to the nearest keyframe at or before the given timestamp.
    ///
    /// `timestamp` is in units of the specified stream's timebase.
    fn seek_to_keyframe(&mut self, stream_index: usize, timestamp: i64) -> Result<(), DemuxError>;
}
