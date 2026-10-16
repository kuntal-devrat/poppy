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
            let mut resource_ptr: *mut std::ffi::c_void = std::ptr::null_mut();

            // ID3D12Device::OpenSharedHandle vtable call (vtable index 31)
            let vtable = *(raw_device.as_raw() as *const *const *const std::ffi::c_void);
            let open_shared_handle: unsafe extern "system" fn(
                this: *mut std::ffi::c_void,
                nt_handle: *mut std::ffi::c_void,
                riid: *const [u8; 16],
                ppv_resource: *mut *mut std::ffi::c_void,
            ) -> i32 = std::mem::transmute(*vtable.add(31));

            // IID_ID3D12Resource: 696442be-a72e-4059-bc79-5b5c98040fad
            let iid_id3d12resource: [u8; 16] = [
                0xbe, 0x42, 0x64, 0x69, 0x2e, 0xa7, 0x59, 0x40, 0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04,
                0x0f, 0xad,
            ];

            let hr = open_shared_handle(
                raw_device.as_raw(),
                desc.handle as *mut std::ffi::c_void,
                &iid_id3d12resource,
                &mut resource_ptr,
            );

            if hr != 0 || resource_ptr.is_null() {
                return Err(RenderError::ImportFailed(format!(
                    "ID3D12Device::OpenSharedHandle failed with HRESULT {hr:#x}"
                )));
            }

            let y_hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("poppy_dxgi_zero_copy_y_texture"),
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
                label: Some("poppy_dxgi_zero_copy_uv_texture"),
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
                label: Some("poppy_dxgi_zero_copy_y_texture"),
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
                label: Some("poppy_dxgi_zero_copy_uv_texture"),
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

            // Wrap D3D12 resource COM pointer with Release guard
            let cleanup_ptr = resource_ptr;
            let drop_callback: wgpu_hal::DropCallback = Box::new(move || {
                let vtbl = *(cleanup_ptr as *const *const *const std::ffi::c_void);
                let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*vtbl.add(2));
                release(cleanup_ptr);
            });

            let y_hal_tex = wgpu_hal::dx12::Device::texture_from_raw(
                resource_ptr as *mut _,
                &y_hal_desc,
                Some(drop_callback),
            );

            // AddRef for the second plane sharing the same underlying allocation
            let vtbl = *(resource_ptr as *const *const *const std::ffi::c_void);
            let add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                std::mem::transmute(*vtbl.add(1));
            add_ref(resource_ptr);

            let uv_cleanup_ptr = resource_ptr;
            let uv_drop_callback: wgpu_hal::DropCallback = Box::new(move || {
                let vtbl = *(uv_cleanup_ptr as *const *const *const std::ffi::c_void);
                let release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
                    std::mem::transmute(*vtbl.add(2));
                release(uv_cleanup_ptr);
            });

            let uv_hal_tex = wgpu_hal::dx12::Device::texture_from_raw(
                resource_ptr as *mut _,
                &uv_hal_desc,
                Some(uv_drop_callback),
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
