//! # poppy-render
//!
//! Zero-copy GPU interop bridges for Project Poppy.

/// Render errors.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GPU device creation failed: {0}")]
    DeviceInitFailed(String),

    #[error("DMA-BUF import failed: {0}")]
    ImportFailed(String),
}
