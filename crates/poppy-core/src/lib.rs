//! # poppy-core
//!
//! Foundational types for the Poppy media pipeline.
//!
//! This crate defines the shared vocabulary used across all Poppy components:
//! packets, timestamps, stream metadata, codec configurations, and pixel formats.
//! It has zero platform-specific dependencies and compiles everywhere Rust does.

pub mod codec;
pub mod error;
pub mod packet;
pub mod pixel;

pub use codec::{AudioConfig, Av1CRecord, AvcCRecord, CodecConfig, CodecId, HvcCRecord};
pub use error::CoreError;
pub use packet::{Packet, StreamInfo, TimeBase};
pub use pixel::{ChromaSubsampling, ColorSpace, InterlaceMode, PixelFormat};
