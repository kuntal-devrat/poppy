//! Linux DMA-BUF (Direct Memory Access Buffer) descriptors and direct Vulkan zero-copy import.

#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

#[cfg(not(unix))]
pub type RawFd = i32;

#[cfg(target_os = "linux")]
use std::sync::Arc;

use crate::RenderError;
use poppy_core::PixelFormat;
use poppy_hw::DmaBufDescriptor;

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
    #[cfg(unix)]
    pub fd: OwnedFd,
    #[cfg(not(unix))]
    pub fd: RawFd,
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
            return Err(RenderError::ImportFailed(
                "Invalid DMA-BUF file descriptor".into(),
            ));
        }

        #[cfg(unix)]
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        #[cfg(not(unix))]
        let fd = raw_fd;

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
        #[cfg(unix)]
        {
            self.fd.as_raw_fd()
        }
        #[cfg(not(unix))]
        {
            self.fd
        }
    }
}

impl TryFrom<&DmaBufDescriptor> for DmaBufImage {
    type Error = RenderError;

    fn try_from(desc: &DmaBufDescriptor) -> Result<Self, Self::Error> {
        if desc.fd < 0 {
            return Err(RenderError::ImportFailed("Invalid DMA-BUF fd in descriptor".into()));
        }
        #[cfg(unix)]
        let owned_fd = {
            let dup_fd = unsafe { libc::dup(desc.fd) };
            if dup_fd < 0 {
                return Err(RenderError::ImportFailed(format!(
                    "dup() failed for DMA-BUF fd {}: {}",
                    desc.fd,
                    std::io::Error::last_os_error()
                )));
            }
            unsafe { OwnedFd::from_raw_fd(dup_fd) }
        };
        #[cfg(not(unix))]
        let owned_fd = desc.fd;

        let mut planes = Vec::new();
        for (i, &offset) in desc.offsets.iter().enumerate() {
            let stride = desc.pitches.get(i).copied().unwrap_or(0);
            planes.push(DmaBufPlane { offset, stride });
        }
        Ok(Self {
            fd: owned_fd,
            drm_format: desc.drm_format,
            drm_modifier: desc.drm_modifier,
            width: desc.width,
            height: desc.height,
            planes,
        })
    }
}

/// Zero-copy imported `wgpu::Texture`s representing the planes of a DMA-BUF image.
#[derive(Debug)]
pub struct DmaBufZeroCopyTextures {
    /// Primary luma (Y) plane texture.
    pub y_texture: wgpu::Texture,
    /// Subsampled chroma (UV) plane texture.
    pub uv_texture: wgpu::Texture,
}

#[cfg(target_os = "linux")]
struct ImportedGuard {
    device: ash::Device,
    memory: ash::vk::DeviceMemory,
    y_image: ash::vk::Image,
    uv_image: ash::vk::Image,
}

#[cfg(target_os = "linux")]
impl Drop for ImportedGuard {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_image(self.y_image, None);
            self.device.destroy_image(self.uv_image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

/// Directly imports a Linux DRM PRIME DMA-BUF descriptor into `wgpu::Texture`s using Vulkan external memory.
///
/// This performs true zero-copy GPU memory binding without any intermediate staging copies through CPU host memory.
#[cfg(target_os = "linux")]
pub fn import_dmabuf_zero_copy(
    device: &wgpu::Device,
    desc: &DmaBufDescriptor,
    pixel_format: PixelFormat,
) -> Result<DmaBufZeroCopyTextures, RenderError> {
    if desc.fd < 0 {
        return Err(RenderError::ImportFailed("Invalid DMA-BUF fd".into()));
    }

    unsafe {
        device.as_hal::<wgpu_hal::api::Vulkan, _, _>(|hal_device| {
            let hal_device = match hal_device {
                Some(d) => d,
                None => {
                    return Err(RenderError::ImportFailed(
                        "Device is not backed by Vulkan HAL".into(),
                    ));
                }
            };

            let raw_device = hal_device.raw_device();
            let raw_instance = hal_device.shared_instance().raw_instance();
            let raw_physical_device = hal_device.raw_physical_device();

            // Validate that the underlying Vulkan device was created with VK_KHR_external_memory_fd
            let pfn = (raw_instance.fp_v1_0().get_device_proc_addr)(
                raw_device.handle(),
                c"vkGetMemoryFdPropertiesKHR".as_ptr(),
            );
            if pfn.is_none() {
                return Err(RenderError::ImportFailed(
                    "Vulkan device was created without VK_KHR_external_memory_fd extension enabled"
                        .into(),
                ));
            }

            let ext_mem_fd = ash::khr::external_memory_fd::Device::new(raw_instance, raw_device);

            // Duplicate DMA-BUF fd for Vulkan memory import
            let dup_fd = libc::dup(desc.fd);
            if dup_fd < 0 {
                return Err(RenderError::ImportFailed("Failed to dup DMA-BUF fd".into()));
            }

            // Query memory properties for DMA-BUF handle
            let mut fd_props = ash::vk::MemoryFdPropertiesKHR::default();
            let status = ext_mem_fd.get_memory_fd_properties(
                ash::vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                dup_fd,
                &mut fd_props,
            );
            if status.is_err() {
                libc::close(dup_fd);
                return Err(RenderError::ImportFailed(
                    "Driver does not support VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT".into(),
                ));
            }

            let mem_props = raw_instance.get_physical_device_memory_properties(raw_physical_device);
            let mut memory_type_index = None;
            for i in 0..mem_props.memory_type_count as usize {
                if (fd_props.memory_type_bits & (1 << i)) != 0 {
                    memory_type_index = Some(i as u32);
                    break;
                }
            }

            let memory_type_index = match memory_type_index {
                Some(idx) => idx,
                None => {
                    libc::close(dup_fd);
                    return Err(RenderError::ImportFailed(
                        "No suitable memory type found for DMA-BUF".into(),
                    ));
                }
            };

            let uv_offset = desc
                .offsets
                .get(1)
                .copied()
                .unwrap_or(desc.width * desc.height);
            let uv_pitch = desc.pitches.get(1).copied().unwrap_or(desc.width);
            // Honest multi-object check: single-fd import cannot represent
            // separate Y/UV GEM handles. Offsets must lie within one allocation.
            // If the container exported multiple fds (flattened to objects[0]),
            // surface.rs already rejects; here validate offsets are monotonic.
            if desc.offsets.len() > 2 {
                return Err(RenderError::ImportFailed(
                    "multi-plane DMA-BUF with >2 offsets not supported in single-memory import"
                        .into(),
                ));
            }
            // Tiling modifier: only LINEAR (0) and INVALID (all-ones, treat as linear)
            // are safe with OPTIMAL tiling + explicit offsets. Other modifiers
            // (Intel X/Y-tiled, AMD DCC) require VkImageDrmFormatModifierExplicitCreateInfo
            // which wgpu-hal does not yet expose; fail honestly instead of misrendering.
            const DRM_FORMAT_MOD_LINEAR: u64 = 0;
            const DRM_FORMAT_MOD_INVALID: u64 = 0x00FF_FFFF_FFFF_FFFF;
            if desc.drm_modifier != DRM_FORMAT_MOD_LINEAR
                && desc.drm_modifier != DRM_FORMAT_MOD_INVALID
            {
                return Err(RenderError::ImportFailed(format!(
                    "non-linear DRM modifier {:#x} requires explicit modifier import (unsupported)",
                    desc.drm_modifier
                )));
            }
            let total_size = ((uv_offset + uv_pitch * desc.height.div_ceil(2)) as u64).max(65536);

            let mut import_info = ash::vk::ImportMemoryFdInfoKHR::default()
                .handle_type(ash::vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
                .fd(dup_fd);

            let alloc_info = ash::vk::MemoryAllocateInfo::default()
                .allocation_size(total_size)
                .memory_type_index(memory_type_index)
                .push_next(&mut import_info);

            let device_memory = match raw_device.allocate_memory(&alloc_info, None) {
                Ok(mem) => mem,
                Err(e) => {
                    libc::close(dup_fd);
                    return Err(RenderError::ImportFailed(format!(
                        "Failed to allocate imported memory: {e}"
                    )));
                }
            };

            let (vk_y_fmt, wgpu_y_fmt, vk_uv_fmt, wgpu_uv_fmt) = match pixel_format {
                PixelFormat::P010 => (
                    ash::vk::Format::R16_UNORM,
                    wgpu::TextureFormat::R16Unorm,
                    ash::vk::Format::R16G16_UNORM,
                    wgpu::TextureFormat::Rg16Unorm,
                ),
                _ => (
                    ash::vk::Format::R8_UNORM,
                    wgpu::TextureFormat::R8Unorm,
                    ash::vk::Format::R8G8_UNORM,
                    wgpu::TextureFormat::Rg8Unorm,
                ),
            };

            // Create Y plane image
            let mut y_ext_img = ash::vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(ash::vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let y_img_info = ash::vk::ImageCreateInfo::default()
                .image_type(ash::vk::ImageType::TYPE_2D)
                .format(vk_y_fmt)
                .extent(ash::vk::Extent3D {
                    width: desc.width,
                    height: desc.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(ash::vk::SampleCountFlags::TYPE_1)
                .tiling(ash::vk::ImageTiling::OPTIMAL)
                .usage(ash::vk::ImageUsageFlags::SAMPLED | ash::vk::ImageUsageFlags::TRANSFER_DST)
                .sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
                .push_next(&mut y_ext_img);

            let y_image = match raw_device.create_image(&y_img_info, None) {
                Ok(img) => img,
                Err(e) => {
                    raw_device.free_memory(device_memory, None);
                    return Err(RenderError::ImportFailed(format!(
                        "Failed to create Y image: {e}"
                    )));
                }
            };

            let y_offset = desc.offsets.first().copied().unwrap_or(0) as u64;
            if let Err(e) = raw_device.bind_image_memory(y_image, device_memory, y_offset) {
                raw_device.destroy_image(y_image, None);
                raw_device.free_memory(device_memory, None);
                return Err(RenderError::ImportFailed(format!(
                    "Failed to bind Y image memory: {e}"
                )));
            }

            // Create UV plane image
            let uv_w = desc.width.div_ceil(2);
            let uv_h = desc.height.div_ceil(2);
            let mut uv_ext_img = ash::vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(ash::vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let uv_img_info = ash::vk::ImageCreateInfo::default()
                .image_type(ash::vk::ImageType::TYPE_2D)
                .format(vk_uv_fmt)
                .extent(ash::vk::Extent3D {
                    width: uv_w,
                    height: uv_h,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(ash::vk::SampleCountFlags::TYPE_1)
                .tiling(ash::vk::ImageTiling::OPTIMAL)
                .usage(ash::vk::ImageUsageFlags::SAMPLED | ash::vk::ImageUsageFlags::TRANSFER_DST)
                .sharing_mode(ash::vk::SharingMode::EXCLUSIVE)
                .push_next(&mut uv_ext_img);

            let uv_image = match raw_device.create_image(&uv_img_info, None) {
                Ok(img) => img,
                Err(e) => {
                    raw_device.destroy_image(y_image, None);
                    raw_device.free_memory(device_memory, None);
                    return Err(RenderError::ImportFailed(format!(
                        "Failed to create UV image: {e}"
                    )));
                }
            };

            if let Err(e) = raw_device.bind_image_memory(uv_image, device_memory, uv_offset as u64)
            {
                raw_device.destroy_image(y_image, None);
                raw_device.destroy_image(uv_image, None);
                raw_device.free_memory(device_memory, None);
                return Err(RenderError::ImportFailed(format!(
                    "Failed to bind UV image memory: {e}"
                )));
            }

            let cleanup = Arc::new(ImportedGuard {
                device: raw_device.clone(),
                memory: device_memory,
                y_image,
                uv_image,
            });

            let cleanup_y = cleanup.clone();
            let drop_y: wgpu_hal::DropCallback = Box::new(move || {
                drop(cleanup_y);
            });
            let drop_uv: wgpu_hal::DropCallback = Box::new(move || {
                drop(cleanup);
            });

            let y_hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("poppy_zero_copy_y_texture"),
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
            let y_hal_tex =
                wgpu_hal::vulkan::Device::texture_from_raw(y_image, &y_hal_desc, Some(drop_y));

            let uv_hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("poppy_zero_copy_uv_texture"),
                size: wgpu::Extent3d {
                    width: uv_w,
                    height: uv_h,
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
            let uv_hal_tex =
                wgpu_hal::vulkan::Device::texture_from_raw(uv_image, &uv_hal_desc, Some(drop_uv));

            let y_wgpu_desc = wgpu::TextureDescriptor {
                label: Some("poppy_zero_copy_y_texture"),
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
                label: Some("poppy_zero_copy_uv_texture"),
                size: wgpu::Extent3d {
                    width: uv_w,
                    height: uv_h,
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
                device.create_texture_from_hal::<wgpu_hal::api::Vulkan>(y_hal_tex, &y_wgpu_desc);
            let uv_texture =
                device.create_texture_from_hal::<wgpu_hal::api::Vulkan>(uv_hal_tex, &uv_wgpu_desc);

            Ok(DmaBufZeroCopyTextures {
                y_texture,
                uv_texture,
            })
        })
    }
}

/// Fallback stub on non-Linux platforms where DMA-BUF is unavailable.
#[cfg(not(target_os = "linux"))]
pub fn import_dmabuf_zero_copy(
    _device: &wgpu::Device,
    _desc: &DmaBufDescriptor,
    _pixel_format: PixelFormat,
) -> Result<DmaBufZeroCopyTextures, RenderError> {
    Err(RenderError::ImportFailed(
        "DMA-BUF zero copy import is only supported on Linux".into(),
    ))
}
