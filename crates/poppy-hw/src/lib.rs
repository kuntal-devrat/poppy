//! # poppy-hw
//!
//! Cross-platform hardware decoder abstraction layer for the Poppy media pipeline.
//! - Linux: VA-API (real `dlopen(libva)` + `vaBegin/Render/End/Sync`) with DRM PRIME DMA-BUF zero-copy.
//! - Windows/macOS: native backends are **not yet implemented** — `create_decoder`
//!   returns `HwError::NoDeviceFound` on those platforms. The `d3d12` and
//!   `videotoolbox` modules are metadata-only shims retained for API
//!   compatibility and unit testing; they do not touch COM or VideoToolbox.
//!
//! ## Example
//!
//! ```
//! use poppy_hw::DmaBufDescriptor;
//!
//! let desc = DmaBufDescriptor {
//!     fd: -1,
//!     drm_format: 0x3231564e, // DRM_FORMAT_NV12
//!     drm_modifier: 0,
//!     width: 1920,
//!     height: 1080,
//!     offsets: vec![0, 1920 * 1080],
//!     pitches: vec![1920, 1920],
//! };
//!
//! let cloned = desc.try_clone().expect("successful clone with inactive fd");
//! assert_eq!(cloned.fd, -1);
//! assert_eq!(cloned.width, 1920);
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_os = "linux")]
pub mod vaapi;

#[cfg(target_os = "linux")]
pub use vaapi::{VaapiAv1Decoder, VaapiH264Decoder, VaapiHevcDecoder, VaapiVp9Decoder};

#[cfg(target_os = "windows")]
pub mod d3d12;
#[cfg(target_os = "windows")]
#[deprecated(note = "D3D12 native backend is a shim; use create_decoder()")]
pub use d3d12::D3D12VideoDecoder;
#[cfg(not(target_os = "windows"))]
#[doc(hidden)]
pub mod d3d12;

#[cfg(target_os = "macos")]
pub mod videotoolbox;
#[cfg(target_os = "macos")]
#[deprecated(note = "VideoToolbox native backend is a shim; use create_decoder()")]
pub use videotoolbox::VideoToolboxDecoder;
#[cfg(not(target_os = "macos"))]
#[doc(hidden)]
pub mod videotoolbox;

pub mod sw;

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

impl DmaBufDescriptor {
    /// Attempts to clone the DMA-BUF descriptor by duplicating its underlying file descriptor.
    ///
    /// If `self.fd >= 0` and `libc::dup` fails (e.g. fd table exhaustion `EMFILE`),
    /// this returns an error rather than silently creating a broken descriptor with fd = -1.
    pub fn try_clone(&self) -> Result<Self, HwError> {
        let dup_fd = if self.fd >= 0 {
            #[cfg(unix)]
            {
                let res = unsafe { libc::dup(self.fd) };
                if res < 0 {
                    return Err(HwError::DecodeError(format!(
                        "dup() failed for DMA-BUF fd {}: {}",
                        self.fd,
                        std::io::Error::last_os_error()
                    )));
                }
                res
            }
            #[cfg(not(unix))]
            {
                self.fd
            }
        } else {
            -1
        };
        Ok(Self {
            fd: dup_fd,
            drm_format: self.drm_format,
            drm_modifier: self.drm_modifier,
            width: self.width,
            height: self.height,
            offsets: self.offsets.clone(),
            pitches: self.pitches.clone(),
        })
    }
}

impl Clone for DmaBufDescriptor {
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|err| {
            panic!("DmaBufDescriptor::clone: {err}");
        })
    }
}

impl Drop for DmaBufDescriptor {
    fn drop(&mut self) {
        #[cfg(unix)]
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

impl DecodedFrame {
    /// Attempts to clone the frame, duplicating any underlying OS handle (such as DMA-BUF fd).
    ///
    /// Returns an error if the underlying handle duplication fails (e.g. fd exhaustion).
    pub fn try_clone(&self) -> Result<Self, HwError> {
        let dmabuf = match &self.dmabuf {
            Some(d) => Some(d.try_clone()?),
            None => None,
        };
        Ok(Self {
            pts: self.pts,
            width: self.width,
            height: self.height,
            pixel_format: self.pixel_format,
            surface_id: self.surface_id,
            data: self.data.clone(),
            interlace_mode: self.interlace_mode,
            dmabuf,
            dxgi_handle: self.dxgi_handle.clone(),
            cv_pixel_buffer: self.cv_pixel_buffer.clone(),
            is_concealed: self.is_concealed,
        })
    }
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

/// Cross-platform factory that instantiates the native hardware decoder for the host OS.
///
/// - **Linux**: Returns a VA-API hardware decoder (`libva.so.2`).
/// - **Windows**: Returns a Direct3D 12 Video decoder (`d3d12.dll`), or `HwError::NoDeviceFound` if unavailable.
/// - **macOS**: Returns a VideoToolbox hardware decoder, or `HwError::NoDeviceFound` if unavailable.
/// - Other platforms: Returns `HwError::NoDeviceFound`.
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
        match codec_id {
            CodecId::H264 | CodecId::Hevc | CodecId::Av1 | CodecId::Vp9 => {
                Ok(Box::new(d3d12::D3D12VideoDecoder::new(codec_id)?))
            }
            _ => Err(HwError::UnsupportedCodec),
        }
    }

    #[cfg(target_os = "macos")]
    {
        match codec_id {
            CodecId::H264 | CodecId::Hevc => {
                Ok(Box::new(videotoolbox::VideoToolboxDecoder::new(codec_id)?))
            }
            _ => Err(HwError::UnsupportedCodec),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = codec_id;
        Err(HwError::NoDeviceFound)
    }
}
