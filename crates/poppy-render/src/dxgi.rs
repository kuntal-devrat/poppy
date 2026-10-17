//! Windows DXGI shared handle zero-copy GPU import into `wgpu::Texture`.
//!
//! Imports an `ID3D12Resource` exported via `IDXGIResource1::CreateSharedHandle`
//! or D3D12 `CreateSharedHandle` directly into DirectX 12 / wgpu textures with zero CPU staging copies.

use crate::RenderError;
use poppy_core::PixelFormat;
use poppy_hw::DxgiSharedHandle;

/// Zero-copy imported textures representing the color planes of a DXGI shared resource.
#[derive(Debug)]
pub struct DxgiZeroCopyTextures {
    /// Luma (Y) texture plane.
    pub y_texture: wgpu::Texture,
    /// Subsampled chroma (UV) texture plane.
    pub uv_texture: wgpu::Texture,
}

#[cfg(target_os = "windows")]
pub fn import_dxgi_zero_copy(
    device: &wgpu::Device,
    desc: &DxgiSharedHandle,
    pixel_format: PixelFormat,
) -> Result<DxgiZeroCopyTextures, RenderError> {
    if desc.handle == 0 {
        return Err(RenderError::ImportFailed(
            "Invalid DXGI shared handle (NULL)".into(),
        ));
    }

    let (wgpu_y_fmt, wgpu_uv_fmt) = match pixel_format {
        PixelFormat::P010 => (
            wgpu::TextureFormat::R16Unorm,
            wgpu::TextureFormat::Rg16Unorm,
        ),
        _ => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
    };

    unsafe {
        device.as_hal::<wgpu_hal::api::Dx12, _, _>(|hal_device| {
            let hal_device = match hal_device {
                Some(d) => d,
                None => {
                    return Err(RenderError::ImportFailed(
                        "wgpu Device is not backed by DirectX 12 HAL".into(),
                    ));
                }
            };

            let raw_device = hal_device.raw_device();
            let nt_handle = windows::Win32::Foundation::HANDLE(desc.handle as *mut _);

            let mut resource: Option<windows::Win32::Graphics::Direct3D12::ID3D12Resource> = None;
            raw_device
                .OpenSharedHandle(nt_handle, &mut resource)
                .map_err(|e| {
                    RenderError::ImportFailed(format!("ID3D12Device::OpenSharedHandle failed: {e}"))
                })?;

            let resource = resource.ok_or_else(|| {
                RenderError::ImportFailed(
                    "ID3D12Device::OpenSharedHandle returned null resource".into(),
                )
            })?;

            let y_size = wgpu::Extent3d {
                width: desc.width,
                height: desc.height,
                depth_or_array_layers: 1,
            };

            let uv_size = wgpu::Extent3d {
                width: (desc.width / 2).max(1),
                height: (desc.height / 2).max(1),
                depth_or_array_layers: 1,
            };

            let y_wgpu_desc = wgpu::TextureDescriptor {
                label: Some("poppy_dxgi_zero_copy_y_texture"),
                size: y_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_y_fmt,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };

            let uv_wgpu_desc = wgpu::TextureDescriptor {
                label: Some("poppy_dxgi_zero_copy_uv_texture"),
                size: uv_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu_uv_fmt,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            };

            let y_hal_tex = wgpu_hal::dx12::Device::texture_from_raw(
                resource.clone(),
                wgpu_y_fmt,
                wgpu::TextureDimension::D2,
                y_size,
                1,
                1,
            );

            let uv_hal_tex = wgpu_hal::dx12::Device::texture_from_raw(
                resource,
                wgpu_uv_fmt,
                wgpu::TextureDimension::D2,
                uv_size,
                1,
                1,
            );

            let y_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Dx12>(y_hal_tex, &y_wgpu_desc);
            let uv_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Dx12>(uv_hal_tex, &uv_wgpu_desc);

            Ok(DxgiZeroCopyTextures {
                y_texture,
                uv_texture,
            })
        })
    }
}

/// Fallback on non-Windows platforms where DXGI is unavailable.
#[cfg(not(target_os = "windows"))]
pub fn import_dxgi_zero_copy(
    _device: &wgpu::Device,
    desc: &DxgiSharedHandle,
    _pixel_format: PixelFormat,
) -> Result<DxgiZeroCopyTextures, RenderError> {
    if desc.handle == 0 {
        return Err(RenderError::ImportFailed(
            "Invalid DXGI shared handle (NULL)".into(),
        ));
    }

    Err(RenderError::ImportFailed(
        "DXGI shared handle zero-copy import is only supported on Windows".into(),
    ))
}
