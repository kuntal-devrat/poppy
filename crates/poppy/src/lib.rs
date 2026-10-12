//! # Poppy
//!
//! A modern, memory-safe media pipeline with zero-copy hardware acceleration.
//!
//! Poppy provides:
//! - Pure safe-Rust container demuxers for MP4 (ISOBMFF) and MKV (EBML).
//! - Direct hardware-accelerated video decoding (VA-API on Linux, D3D12 Video on Windows, VideoToolbox on macOS).
//! - Zero-copy GPU surface export via DMA-BUF, DXGI Shared Handles, and CVPixelBuffer.
//! - Ergonomic synchronous and asynchronous public APIs:
//!   - `poppy::open("video.mp4")`
//!   - `poppy::open_async("video.mp4").await`
//!   - `poppy::MediaStream::builder()` with backpressure and timeline scrubbing.

pub mod async_stream;
pub mod builder;
pub mod error;
pub mod ffi;
pub mod pipeline;

pub use async_stream::AsyncMediaStream;
pub use builder::MediaStreamBuilder;
pub use error::PoppyError;
pub use pipeline::MediaStream;
pub use poppy_audio::{AudioClock, AudioDecoder, AudioSink};
pub use poppy_core::{
    AudioConfig, AudioFrame, AudioSampleFormat, ChannelLayout, CodecId, InterlaceMode, Packet,
    PixelFormat, StreamInfo, TimeBase,
};
pub use poppy_hw::DecodedFrame;

pub use poppy_audio as audio;
pub use poppy_av1 as av1;
pub use poppy_core as core;
pub use poppy_demux as demux;
pub use poppy_h264 as h264;
pub use poppy_hevc as hevc;
pub use poppy_hw as hw;
pub use poppy_render as render;

use std::path::Path;

/// Opens a media file (MP4, MKV, WebM) synchronously with automatic hardware decoder detection.
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

/// Opens a media file (MP4, MKV, WebM) asynchronously with backpressure flow control.
///
/// # Example
/// ```no_run
/// use poppy;
/// use futures_util::StreamExt;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let mut stream = poppy::open_async("tests/samples/test_clip.mp4").await?;
///     while let Some(frame) = stream.next().await {
///         let frame = frame?;
///         println!("Decoded frame: {}x{}", frame.width, frame.height);
///     }
///     Ok(())
/// }
/// ```
pub async fn open_async<P: AsRef<Path>>(path: P) -> Result<AsyncMediaStream, PoppyError> {
    AsyncMediaStream::open(path).await
}
