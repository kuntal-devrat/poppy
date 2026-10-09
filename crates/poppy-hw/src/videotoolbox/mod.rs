//! macOS VideoToolbox (`VTDecompressionSession`) HAL backend.

pub mod decoder;
pub mod ffi;

pub use decoder::VideoToolboxDecoder;
