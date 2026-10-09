//! # poppy-hw
//!
//! Hardware decoder abstraction layer for the Poppy media pipeline.

#[cfg(target_os = "linux")]
pub mod vaapi;

#[cfg(target_os = "linux")]
pub use vaapi::VaapiH264Decoder;

use poppy_core::{Packet, PixelFormat};

/// Errors originating from hardware decoder operations.
#[derive(Debug, thiserror::Error)]
pub enum HwError {
    #[error("device initialization failed: {0}")]
    DeviceInitFailed(String),

    #[error("surface allocation failed: {0}")]
    SurfaceAllocationFailed(String),

    #[error("decode error: {0}")]
    DecodeError(String),

    #[error("unsupported codec or profile")]
    UnsupportedCodec,

    #[error("no hardware device found")]
    NoDeviceFound,
}

/// A decoded frame backed by a GPU surface or memory buffer.
#[derive(Debug)]
pub struct DecodedFrame {
    pub pts: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
}

/// Platform-agnostic hardware decoder interface.
pub trait HwDecoder {
    /// Submit a compressed packet to the hardware decoder.
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError>;

    /// Retrieve the next decoded frame, if one is available.
    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError>;

    /// Signal end-of-stream and drain all remaining buffered frames.
    fn flush(&mut self) -> Result<(), HwError>;
}
