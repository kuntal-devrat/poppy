//! Linux DMA-BUF (Direct Memory Access Buffer) descriptors for zero-copy GPU import.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use crate::RenderError;

/// Layout and memory description for a single color plane within a DMA-BUF image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaBufPlane {
    /// Byte offset from the start of the DMA-BUF memory object.
    pub offset: u32,
    /// Line stride (pitch) in bytes.
    pub stride: u32,
}

/// A zero-copy hardware DMA-BUF image descriptor ready for Vulkan / EGL import.
#[derive(Debug)]
pub struct DmaBufImage {
    /// File descriptor representing the backing GPU VRAM allocation.
    pub fd: OwnedFd,
    /// DRM pixel format FourCC (e.g. `DRM_FORMAT_NV12` = 0x3231564e).
    pub drm_format: u32,
    /// DRM format modifier (e.g. `DRM_FORMAT_MOD_LINEAR` = 0, or Intel/AMD tiling modifier).
    pub drm_modifier: u64,
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Color planes (e.g., plane 0: Y, plane 1: UV for NV12).
    pub planes: Vec<DmaBufPlane>,
}

impl DmaBufImage {
    /// Creates a new DMA-BUF image descriptor from a raw file descriptor.
    pub fn new(
        raw_fd: RawFd,
        drm_format: u32,
        drm_modifier: u64,
        width: u32,
        height: u32,
        planes: Vec<DmaBufPlane>,
    ) -> Result<Self, RenderError> {
        if raw_fd < 0 {
            return Err(RenderError::ImportFailed("Invalid DMA-BUF file descriptor".into()));
        }

        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        Ok(Self {
            fd,
            drm_format,
            drm_modifier,
            width,
            height,
            planes,
        })
    }

    /// Access the underlying raw file descriptor.
    #[inline]
    pub fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}
