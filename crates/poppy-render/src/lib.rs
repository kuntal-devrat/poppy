//! # poppy-render
//!
//! Zero-copy GPU interop bridges (DMA-BUF and texture abstractions) for Project Poppy.

pub mod dmabuf;
pub mod texture;

pub use dmabuf::{DmaBufImage, DmaBufPlane};
pub use texture::GpuTexture;

/// Render errors.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GPU device creation failed: {0}")]
    DeviceInitFailed(String),

    #[error("DMA-BUF import failed: {0}")]
    ImportFailed(String),
}
