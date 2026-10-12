//! # poppy-render
//!
//! High-performance GPU rendering and zero-copy interop bridges powered by `wgpu`.
//!
//! ## Example
//!
//! ```
//! use poppy_render::ColorSpace;
//!
//! let cs = ColorSpace::Bt709Limited;
//! assert_eq!(cs.matrix_index(), 0);
//! assert!(!cs.is_full_range());
//!
//! let cs_full = ColorSpace::Bt709Full;
//! assert!(cs_full.is_full_range());
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

pub mod dmabuf;
pub mod dxgi;
pub mod metal;
pub mod pipeline;
pub mod texture;

pub use dmabuf::{DmaBufImage, DmaBufPlane};
pub use dxgi::DxgiZeroCopyTextures;
pub use metal::MetalZeroCopyTextures;
pub use pipeline::{ColorSpace, GpuRenderer, VideoRenderPipeline};
pub use texture::GpuTexture;

/// Render errors.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GPU adapter request failed: {0}")]
    AdapterNotFound(String),

    #[error("GPU device creation failed: {0}")]
    DeviceInitFailed(String),

    #[error("Render pipeline creation failed: {0}")]
    PipelineCreationFailed(String),

    #[error("Buffer mapping failed: {0}")]
    BufferMapFailed(String),

    #[error("DMA-BUF import failed: {0}")]
    ImportFailed(String),

    #[error("Unsupported frame format: {0}")]
    UnsupportedFormat(String),
}
