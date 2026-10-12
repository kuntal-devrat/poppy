//! Software video decoding backends and CPU fallbacks.
//!
//! Provides software decoding when no hardware acceleration device (VA-API,
//! D3D12, VideoToolbox) is available (e.g. headless CI environments, unsupported GPUs).

pub mod openh264;

pub use openh264::OpenH264Decoder;

use crate::{HwDecoder, HwError};
use poppy_core::CodecId;

/// Instantiates an available software decoder for the requested codec.
///
/// Falls back gracefully with `HwError::NoDeviceFound` or `HwError::UnsupportedCodec`
/// when no software decoder implementation is available.
pub fn create_software_decoder(codec: CodecId) -> Result<Box<dyn HwDecoder>, HwError> {
    match codec {
        CodecId::H264 => {
            let decoder = OpenH264Decoder::new()?;
            Ok(Box::new(decoder))
        }
        _ => Err(HwError::UnsupportedCodec),
    }
}
