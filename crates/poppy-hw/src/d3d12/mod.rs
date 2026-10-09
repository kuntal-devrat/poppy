//! Windows Direct3D 12 Video (`D3D12_VIDEO_DECODE`) HAL backend.

pub mod decoder;
pub mod device;
pub mod ffi;

pub use decoder::D3D12VideoDecoder;
pub use device::{D3D12Surface, D3D12SurfacePool, D3D12VideoDevice};
