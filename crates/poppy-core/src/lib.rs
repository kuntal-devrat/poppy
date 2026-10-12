//! # poppy-core
//!
//! Foundational types for the Poppy media pipeline.
//!
//! This crate defines the shared vocabulary used across all Poppy components:
//! packets, timestamps, stream metadata, codec configurations, and pixel formats.
//! It has zero platform-specific dependencies and compiles everywhere Rust does.
//!
//! ```
//! use poppy_core::{TimeBase, Packet, PixelFormat};
//! use bytes::Bytes;
//!
//! let tb = TimeBase::new(1, 90000);
//! assert_eq!(tb.to_seconds(90000), 1.0);
//!
//! let pkt = Packet {
//!     stream_index: 0,
//!     pts: Some(90000),
//!     dts: 90000,
//!     duration: 3000,
//!     data: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01]),
//!     is_keyframe: true,
//! };
//! assert_eq!(pkt.pts.map(|p| tb.to_seconds(p)), Some(1.0));
//! assert_eq!(PixelFormat::Nv12.num_planes(), 2);
//! assert_eq!(PixelFormat::Nv12.bits_per_pixel(), 12);
//! ```

pub mod codec;
pub mod error;
pub mod packet;
pub mod pixel;

pub use codec::{
    AudioConfig, AudioFrame, AudioSampleFormat, Av1CRecord, AvcCRecord, ChannelLayout, CodecConfig,
    CodecId, HvcCRecord, VpcCRecord,
};
pub use error::CoreError;
pub use packet::{Packet, StreamInfo, TimeBase};
pub use pixel::{ChromaSubsampling, ColorSpace, InterlaceMode, PixelFormat};
