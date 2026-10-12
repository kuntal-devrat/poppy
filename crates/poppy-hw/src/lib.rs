//! # poppy-hw
//!
//! Cross-platform hardware decoder abstraction layer for the Poppy media pipeline.
//! Supports:
//! - Linux: VA-API (Intel QuickSync, AMD VA-API, NVIDIA Mesa) with DRM PRIME DMA-BUF zero-copy
//! - Windows: Direct3D 12 Video (`D3D12_VIDEO_DECODE`) with DXGI Shared Handle zero-copy
//! - macOS: VideoToolbox (`VTDecompressionSession`) with CVPixelBuffer / IOSurface zero-copy

#[cfg(target_os = "linux")]
pub mod vaapi;

#[cfg(target_os = "linux")]
pub use vaapi::{VaapiAv1Decoder, VaapiH264Decoder, VaapiHevcDecoder, VaapiVp9Decoder};

pub mod d3d12;
pub use d3d12::D3D12VideoDecoder;

pub mod videotoolbox;
pub use videotoolbox::VideoToolboxDecoder;

use poppy_core::{CodecId, InterlaceMode, Packet, PixelFormat};

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

/// Raw exported Linux DRM PRIME DMA-BUF memory descriptor for zero-copy GPU import.
#[derive(Debug)]
pub struct DmaBufDescriptor {
    pub fd: i32,
    pub drm_format: u32,
    pub drm_modifier: u64,
    pub width: u32,
    pub height: u32,
    pub offsets: Vec<u32>,
    pub pitches: Vec<u32>,
}

impl Clone for DmaBufDescriptor {
    fn clone(&self) -> Self {
        let dup_fd = if self.fd >= 0 {
            unsafe { libc::dup(self.fd) }
        } else {
            -1
        };
        Self {
            fd: dup_fd,
            drm_format: self.drm_format,
            drm_modifier: self.drm_modifier,
            width: self.width,
            height: self.height,
            offsets: self.offsets.clone(),
            pitches: self.pitches.clone(),
        }
    }
}

impl Drop for DmaBufDescriptor {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe {
                libc::close(self.fd);
            }
            self.fd = -1;
        }
    }
}

/// Raw exported Windows DXGI / D3D12 shared handle for zero-copy GPU import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxgiSharedHandle {
    pub handle: isize,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
}

/// Raw exported macOS CVPixelBuffer / IOSurface descriptor for zero-copy GPU import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvPixelBufferDescriptor {
    pub pixel_buffer_ptr: usize,
    pub io_surface_ptr: usize,
    pub width: u32,
    pub height: u32,
    pub pixel_format_type: u32,
}

/// A decoded frame backed by a GPU surface or memory buffer across platforms.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub pts: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub surface_id: u32,
    pub data: Option<Vec<u8>>,
    pub interlace_mode: InterlaceMode,
    pub dmabuf: Option<DmaBufDescriptor>,
    pub dxgi_handle: Option<DxgiSharedHandle>,
    pub cv_pixel_buffer: Option<CvPixelBufferDescriptor>,
    pub is_concealed: bool,
}

impl Default for DecodedFrame {
    fn default() -> Self {
        Self {
            pts: None,
            width: 0,
            height: 0,
            pixel_format: PixelFormat::Nv12,
            surface_id: 0,
            data: None,
            interlace_mode: InterlaceMode::Progressive,
            dmabuf: None,
            dxgi_handle: None,
            cv_pixel_buffer: None,
            is_concealed: false,
        }
    }
}

/// Platform-agnostic hardware decoder interface.
pub trait HwDecoder: Send {
    /// Submit a compressed packet to the hardware decoder.
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError>;

    /// Retrieve the next decoded frame, if one is available.
    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError>;

    /// Signal end-of-stream and drain all remaining buffered frames.
    fn flush(&mut self) -> Result<(), HwError>;

    /// Reset decoder state upon a seek or stream discontinuity, clearing all queued output frames.
    fn reset(&mut self) -> Result<(), HwError> {
        self.flush()?;
        Ok(())
    }
}

/// Cross-platform factory that automatically instantiates the native hardware decoder for the host OS.
pub fn create_decoder(codec_id: CodecId) -> Result<Box<dyn HwDecoder>, HwError> {
    #[cfg(target_os = "linux")]
    {
        match codec_id {
            CodecId::H264 => Ok(Box::new(VaapiH264Decoder::new()?)),
            CodecId::Hevc => Ok(Box::new(VaapiHevcDecoder::new(false)?)),
            CodecId::Av1 => Ok(Box::new(VaapiAv1Decoder::new(false)?)),
            CodecId::Vp9 => Ok(Box::new(VaapiVp9Decoder::new(false)?)),
            _ => Err(HwError::UnsupportedCodec),
        }
    }

    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(D3D12VideoDecoder::new(codec_id)?))
    }

    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(VideoToolboxDecoder::new(codec_id)?))
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Err(HwError::NoDeviceFound)
    }
}
