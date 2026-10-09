//! # Poppy
//!
//! A modern, memory-safe media pipeline with zero-copy hardware acceleration.
//!
//! Poppy provides:
//! - Pure safe-Rust container demuxers for MP4 (ISOBMFF) and MKV (EBML).
//! - Direct hardware-accelerated video decoding (VA-API).
//! - Zero-copy GPU surface export via DMA-BUF.
//! - Ergonomic high-level API: `poppy::open("video.mp4")`.

pub mod error;
pub mod pipeline;

pub use error::PoppyError;
pub use pipeline::MediaStream;

pub use poppy_core as core;
pub use poppy_demux as demux;
pub use poppy_h264 as h264;
pub use poppy_hevc as hevc;
pub use poppy_hw as hw;
pub use poppy_render as render;

use std::path::Path;

/// Opens a media file (MP4, MKV, WebM) and initializes hardware decoding pipeline.
///
/// # Example
/// ```no_run
/// use poppy;
///
/// let mut stream = poppy::open("tests/samples/test_clip.mp4").unwrap();
/// for s in stream.streams() {
///     println!("Found stream: {:?}", s.codec);
/// }
/// ```
pub fn open<P: AsRef<Path>>(path: P) -> Result<MediaStream, PoppyError> {
    MediaStream::open(path)
}
