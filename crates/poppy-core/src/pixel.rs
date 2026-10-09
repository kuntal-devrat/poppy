//! Pixel format, color space, and chroma subsampling definitions.
//!
//! These types describe the pixel layout of decoded video frames as produced
//! by hardware decoders. They are needed to correctly interpret GPU surface
//! contents and set up the right Vulkan/Metal texture formats for import.

/// Pixel format of a decoded video frame.
///
/// Hardware decoders typically output planar YCbCr formats. The most common
/// output formats are:
///
/// - **NV12**: 8-bit 4:2:0 with a full-resolution Y plane followed by an
///   interleaved UV plane at half resolution. This is the standard output
///   format for H.264 8-bit decoding on virtually all hardware.
///
/// - **P010**: 10-bit 4:2:0, same layout as NV12 but with 16-bit samples
///   (only upper 10 bits used). Standard for HDR / 10-bit HEVC and AV1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 8-bit 4:2:0, Y plane + interleaved UV plane.
    /// Vulkan: `VK_FORMAT_G8_B8R8_2PLANE_420_UNORM`
    /// VA-API: `VA_FOURCC_NV12`
    Nv12,

    /// 10-bit 4:2:0, Y plane + interleaved UV plane (16-bit containers).
    /// Vulkan: `VK_FORMAT_G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16`
    /// VA-API: `VA_FOURCC_P010`
    P010,

    /// 8-bit 4:2:0, three separate Y/U/V planes (legacy, less common from HW decoders).
    Yuv420p,

    /// 8-bit 4:4:4 RGBA (CPU readback format, not a typical HW decoder output).
    Rgba32,
}

impl PixelFormat {
    /// Returns the number of planes in this pixel format.
    pub fn num_planes(self) -> usize {
        match self {
            PixelFormat::Nv12 | PixelFormat::P010 => 2,
            PixelFormat::Yuv420p => 3,
            PixelFormat::Rgba32 => 1,
        }
    }

    /// Returns the chroma subsampling for this format.
    pub fn chroma_subsampling(self) -> ChromaSubsampling {
        match self {
            PixelFormat::Nv12 | PixelFormat::P010 | PixelFormat::Yuv420p => {
                ChromaSubsampling::Yuv420
            }
            PixelFormat::Rgba32 => ChromaSubsampling::Yuv444,
        }
    }

    /// Returns the number of bits per component (per channel).
    pub fn bit_depth(self) -> u8 {
        match self {
            PixelFormat::Nv12 | PixelFormat::Yuv420p | PixelFormat::Rgba32 => 8,
            PixelFormat::P010 => 10,
        }
    }

    /// Human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            PixelFormat::Nv12 => "NV12",
            PixelFormat::P010 => "P010",
            PixelFormat::Yuv420p => "YUV420P",
            PixelFormat::Rgba32 => "RGBA32",
        }
    }
}

impl std::fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Chroma subsampling scheme.
///
/// Describes how chroma (color) samples relate to luma (brightness) samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromaSubsampling {
    /// 4:2:0 — chroma is half resolution in both width and height.
    /// Each 2×2 block of luma samples shares one Cb and one Cr sample.
    Yuv420,

    /// 4:2:2 — chroma is half resolution in width only.
    Yuv422,

    /// 4:4:4 — full resolution chroma (no subsampling).
    Yuv444,
}

/// Color space / transfer characteristics of a video stream.
///
/// These are typically signaled in the H.264 VUI parameters or in
/// container-level metadata (e.g., Matroska `Colour` element).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// ITU-R BT.601 (SD video). Commonly used for 480i/576i content.
    Bt601,

    /// ITU-R BT.709 (HD video). Standard for 720p/1080p content.
    Bt709,

    /// ITU-R BT.2020 (UHD/HDR video). Standard for 4K HDR content.
    Bt2020,

    /// sRGB. Typically only for screen content or RGBA frames.
    Srgb,

    /// Unknown or not specified in the stream.
    Unspecified,
}

/// Interlacing structure of a video stream or frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InterlaceMode {
    /// Progressive frame (no interlacing).
    #[default]
    Progressive,
    /// Interlaced frame with top field displayed first (TFF).
    TopFieldFirst,
    /// Interlaced frame with bottom field displayed first (BFF).
    BottomFieldFirst,
    /// Single field (top field).
    TopField,
    /// Single field (bottom field).
    BottomField,
}

impl std::fmt::Display for InterlaceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Progressive => write!(f, "Progressive"),
            Self::TopFieldFirst => write!(f, "TopFieldFirst"),
            Self::BottomFieldFirst => write!(f, "BottomFieldFirst"),
            Self::TopField => write!(f, "TopField"),
            Self::BottomField => write!(f, "BottomField"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_properties() {
        assert_eq!(PixelFormat::Nv12.num_planes(), 2);
        assert_eq!(PixelFormat::Nv12.bit_depth(), 8);
        assert_eq!(
            PixelFormat::Nv12.chroma_subsampling(),
            ChromaSubsampling::Yuv420
        );
    }

    #[test]
    fn p010_properties() {
        assert_eq!(PixelFormat::P010.num_planes(), 2);
        assert_eq!(PixelFormat::P010.bit_depth(), 10);
    }

    #[test]
    fn pixel_format_display() {
        assert_eq!(format!("{}", PixelFormat::Nv12), "NV12");
        assert_eq!(format!("{}", PixelFormat::P010), "P010");
    }
}
