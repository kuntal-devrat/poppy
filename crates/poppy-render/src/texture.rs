//! GPU Texture abstraction for presentation engines.

use crate::dmabuf::DmaBufImage;

/// An abstract GPU Texture handle ready for rendering or presentation.
#[derive(Debug)]
pub enum GpuTexture {
    /// Zero-copy texture backed directly by a hardware DMA-BUF VRAM buffer.
    DmaBuf(DmaBufImage),
    /// CPU memory buffer fallback for environments without hardware DMA-BUF interop.
    CpuBuffer {
        width: u32,
        height: u32,
        stride: u32,
        data: Vec<u8>,
    },
}

impl GpuTexture {
    pub fn width(&self) -> u32 {
        match self {
            Self::DmaBuf(img) => img.width,
            Self::CpuBuffer { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::DmaBuf(img) => img.height,
            Self::CpuBuffer { height, .. } => *height,
        }
    }
}
