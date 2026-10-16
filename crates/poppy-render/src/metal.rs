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
use metal::foreign_types::ForeignType;
#[cfg(target_os = "macos")]
use objc::{msg_send, sel, sel_impl};

#[allow(unexpected_cfgs)]
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

            let raw_device_guard = hal_device.raw_device().lock();
            let raw_device: &metal::DeviceRef = &raw_device_guard;

            let (mtl_y_fmt, mtl_uv_fmt) = match pixel_format {
                PixelFormat::Nv12 => (
                    metal::MTLPixelFormat::R8Unorm,
                    metal::MTLPixelFormat::RG8Unorm,
                ),
                PixelFormat::P010 => (
                    metal::MTLPixelFormat::R16Unorm,
                    metal::MTLPixelFormat::RG16Unorm,
                ),
                _ => {
                    return Err(RenderError::UnsupportedFormat(pixel_format.to_string()));
                }
            };

            let y_desc = metal::TextureDescriptor::new();
            y_desc.set_texture_type(metal::MTLTextureType::D2);
            y_desc.set_pixel_format(mtl_y_fmt);
            y_desc.set_width(desc.width as u64);
            y_desc.set_height(desc.height as u64);
            y_desc.set_usage(metal::MTLTextureUsage::ShaderRead);

            let uv_desc = metal::TextureDescriptor::new();
            uv_desc.set_texture_type(metal::MTLTextureType::D2);
            uv_desc.set_pixel_format(mtl_uv_fmt);
            uv_desc.set_width(((desc.width / 2).max(1)) as u64);
            uv_desc.set_height(((desc.height / 2).max(1)) as u64);
            uv_desc.set_usage(metal::MTLTextureUsage::ShaderRead);

            // Call -[MTLDevice newTextureWithDescriptor:iosurface:plane:]
            let y_raw: *mut objc::runtime::Object = msg_send![
                raw_device,
                newTextureWithDescriptor: &*y_desc
                iosurface: iosurface_ptr
                plane: 0usize
            ];
            if y_raw.is_null() {
                return Err(RenderError::ImportFailed(
                    "Metal failed to create texture from IOSurface plane 0".into(),
                ));
            }
            let y_tex: metal::Texture = metal::Texture::from_ptr(y_raw as *mut _);

            let uv_raw: *mut objc::runtime::Object = msg_send![
                raw_device,
                newTextureWithDescriptor: &*uv_desc
                iosurface: iosurface_ptr
                plane: 1usize
            ];
            if uv_raw.is_null() {
                return Err(RenderError::ImportFailed(
                    "Metal failed to create texture from IOSurface plane 1".into(),
                ));
            }
            let uv_tex: metal::Texture = metal::Texture::from_ptr(uv_raw as *mut _);

            let y_extent = wgpu_hal::CopyExtent {
                width: desc.width,
                height: desc.height,
                depth: 1,
            };

            let uv_extent = wgpu_hal::CopyExtent {
                width: (desc.width / 2).max(1),
                height: (desc.height / 2).max(1),
                depth: 1,
            };

            let y_hal_tex = wgpu_hal::metal::Device::texture_from_raw(
                y_tex,
                wgpu_y_fmt,
                metal::MTLTextureType::D2,
                1,
                1,
                y_extent,
            );

            let uv_hal_tex = wgpu_hal::metal::Device::texture_from_raw(
                uv_tex,
                wgpu_uv_fmt,
                metal::MTLTextureType::D2,
                1,
                1,
                uv_extent,
            );

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

            let y_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Metal>(y_hal_tex, &y_wgpu_desc);
            let uv_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Metal>(uv_hal_tex, &uv_wgpu_desc);

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
