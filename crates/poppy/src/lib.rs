//! # Poppy
//!
//! A modern, memory-safe media pipeline with zero-copy hardware acceleration.
//!
//! Poppy provides:
//! - Pure safe-Rust container demuxers for MP4 (ISOBMFF) and MKV (EBML).
//! - Direct hardware-accelerated video decoding.
//! - Zero-copy GPU surface export via DMA-BUF.

pub use poppy_core as core;
pub use poppy_demux as demux;
pub use poppy_h264 as h264;
pub use poppy_hw as hw;
pub use poppy_render as render;
