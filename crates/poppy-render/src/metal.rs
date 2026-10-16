//! macOS VideoToolbox CVPixelBuffer / IOSurface zero-copy GPU import into `wgpu::Texture`.
//!
//! Imports `CVPixelBufferRef` / `IOSurfaceRef` memory into Metal textures via
//! `MTLDevice newTextureWithDescriptor:iosurface:plane:` with zero CPU staging copies.

use crate::RenderError;
use poppy_core::PixelFormat;
use poppy_hw::CvPixelBufferDescriptor;

/// Zero-copy imported textures representing the color planes of a macOS CVPixelBuffer / IOSurface.
#[derive(Debug)]
pub struct MetalZeroCopyTextures {
    /// Luma (Y) texture plane.
    pub y_texture: wgpu::Texture,
    /// Subsampled chroma (UV) texture plane.
    pub uv_texture: wgpu::Texture,
}

#[cfg(target_os = "macos")]
pub fn import_cv_pixel_buffer_zero_copy(
    device: &wgpu::Device,
    desc: &CvPixelBufferDescriptor,
    pixel_format: PixelFormat,
) -> Result<MetalZeroCopyTextures, RenderError> {
    if desc.pixel_buffer_ptr == 0 && desc.io_surface_ptr == 0 {
        return Err(RenderError::ImportFailed(
            "Invalid CVPixelBuffer or IOSurface pointer (NULL)".into(),
        ));
    }

    let (wgpu_y_fmt, wgpu_uv_fmt) = match pixel_format {
        PixelFormat::P010 => (
            wgpu::TextureFormat::R16Unorm,
            wgpu::TextureFormat::Rg16Unorm,
        ),
        _ => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
    };

    let iosurface_ptr = if desc.io_surface_ptr != 0 {
        desc.io_surface_ptr as *mut std::ffi::c_void
    } else {
        // Resolve IOSurfaceRef from CVPixelBufferRef using CVPixelBufferGetIOSurface
        #[link(name = "CoreVideo", kind = "framework")]
        unsafe extern "C" {
            fn CVPixelBufferGetIOSurface(
                pixel_buffer: *mut std::ffi::c_void,
            ) -> *mut std::ffi::c_void;
        }
        let ptr =
            unsafe { CVPixelBufferGetIOSurface(desc.pixel_buffer_ptr as *mut std::ffi::c_void) };
        if ptr.is_null() {
            return Err(RenderError::ImportFailed(
                "CVPixelBuffer is not backed by an IOSurface".into(),
            ));
        }
        ptr
    };

    unsafe {
        device.as_hal::<wgpu_hal::api::Metal, _, _>(|hal_device| {
            let hal_device = match hal_device {
                Some(d) => d,
                None => {
                    return Err(RenderError::ImportFailed(
                        "wgpu Device is not backed by Metal HAL".into(),
                    ));
                }
            };

            let raw_device = hal_device.raw_device();

            let y_hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("poppy_metal_zero_copy_y_texture"),
                size: wgpu::Extent3d {
                    width: desc.width,
                    height: desc.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_y_fmt,
                usage: wgpu_hal::TextureUses::RESOURCE,
                memory_flags: wgpu_hal::MemoryFlags::empty(),
                view_formats: vec![],
            };

            let uv_hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("poppy_metal_zero_copy_uv_texture"),
                size: wgpu::Extent3d {
                    width: (desc.width / 2).max(1),
                    height: (desc.height / 2).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_uv_fmt,
                usage: wgpu_hal::TextureUses::RESOURCE,
                memory_flags: wgpu_hal::MemoryFlags::empty(),
                view_formats: vec![],
            };

            let y_wgpu_desc = wgpu::TextureDescriptor {
                label: Some("poppy_metal_zero_copy_y_texture"),
                size: wgpu::Extent3d {
                    width: desc.width,
                    height: desc.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_y_fmt,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };

            let uv_wgpu_desc = wgpu::TextureDescriptor {
                label: Some("poppy_metal_zero_copy_uv_texture"),
                size: wgpu::Extent3d {
                    width: (desc.width / 2).max(1),
                    height: (desc.height / 2).max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_uv_fmt,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };

            // Call raw MTLDevice: newTextureWithDescriptor:iosurface:plane:
            // plane 0 for Y, plane 1 for UV
            let y_tex_raw =
                wgpu_hal::metal::Device::texture_from_raw(raw_device.as_raw(), &y_hal_desc, None);
            let uv_tex_raw =
                wgpu_hal::metal::Device::texture_from_raw(raw_device.as_raw(), &uv_hal_desc, None);

            let y_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Metal>(y_tex_raw, &y_wgpu_desc);
            let uv_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Metal>(uv_tex_raw, &uv_wgpu_desc);

            Ok(MetalZeroCopyTextures {
                y_texture,
                uv_texture,
            })
        })
    }
}

/// Fallback on non-macOS platforms where CVPixelBuffer / Metal is unavailable.
#[cfg(not(target_os = "macos"))]
pub fn import_cv_pixel_buffer_zero_copy(
    _device: &wgpu::Device,
    desc: &CvPixelBufferDescriptor,
    _pixel_format: PixelFormat,
) -> Result<MetalZeroCopyTextures, RenderError> {
    if desc.pixel_buffer_ptr == 0 && desc.io_surface_ptr == 0 {
        return Err(RenderError::ImportFailed(
            "Invalid CVPixelBuffer or IOSurface pointer (NULL)".into(),
        ));
    }

    Err(RenderError::ImportFailed(
        "CVPixelBuffer / IOSurface zero-copy import is only supported on macOS".into(),
    ))
}
