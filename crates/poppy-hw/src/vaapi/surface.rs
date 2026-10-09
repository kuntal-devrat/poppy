//! VA-API surface pool and DMA-BUF PRIME export.

use std::sync::Arc;

use super::ffi::*;
use crate::HwError;

/// An exported DMA-BUF DRM PRIME handle for a hardware surface.
#[derive(Debug)]
pub struct DmaBufSurface {
    pub surface_id: VASurfaceID,
    pub descriptor: VADRMPRIMESurfaceDescriptor,
}

impl Drop for DmaBufSurface {
    fn drop(&mut self) {
        // Close exported file descriptors
        for i in 0..self.descriptor.num_objects as usize {
            let fd = self.descriptor.objects[i].fd;
            if fd >= 0 {
                unsafe {
                    libc::close(fd);
                }
            }
        }
    }
}

/// A pool of hardware VASurfaces bound to a VAContext.
pub struct SurfacePool {
    va: Arc<VaApi>,
    display: VADisplay,
    pub context_id: VAContextID,
    pub surfaces: Vec<VASurfaceID>,
    pub width: u32,
    pub height: u32,
}

unsafe impl Send for SurfacePool {}
unsafe impl Sync for SurfacePool {}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
impl SurfacePool {
    /// Allocate `count` surfaces of format NV12 (`VA_RT_FORMAT_YUV420`) and create a decode context.
    pub fn new(
        va: Arc<VaApi>,
        display: VADisplay,
        config_id: VAConfigID,
        width: u32,
        height: u32,
        count: usize,
    ) -> Result<Self, HwError> {
        Self::new_with_format(
            va,
            display,
            config_id,
            width,
            height,
            count,
            VA_RT_FORMAT_YUV420,
        )
    }

    /// Allocate `count` surfaces with a specified runtime pixel format (NV12 or P010).
    pub fn new_with_format(
        va: Arc<VaApi>,
        display: VADisplay,
        config_id: VAConfigID,
        width: u32,
        height: u32,
        count: usize,
        rt_format: std::ffi::c_uint,
    ) -> Result<Self, HwError> {
        let mut surfaces = vec![0 as VASurfaceID; count];

        unsafe {
            let status = (va.vaCreateSurfaces)(
                display,
                rt_format,
                width,
                height,
                surfaces.as_mut_ptr(),
                count as u32,
                std::ptr::null_mut(),
                0,
            );
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::SurfaceAllocationFailed(format!(
                    "vaCreateSurfaces failed: {}",
                    va.err_str(status)
                )));
            }
            let mut context_id = 0;
            let status = (va.vaCreateContext)(
                display,
                config_id,
                width as i32,
                height as i32,
                VA_PROGRESSIVE as i32,
                surfaces.as_mut_ptr(),
                count as i32,
                &mut context_id,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaDestroySurfaces)(display, surfaces.as_mut_ptr(), count as u32);
                return Err(HwError::SurfaceAllocationFailed(format!(
                    "vaCreateContext failed: {}",
                    va.err_str(status)
                )));
            }

            Ok(Self {
                va,
                display,
                context_id,
                surfaces,
                width,
                height,
            })
        }
    }

    /// Exports a decoded surface as a DMA-BUF PRIME descriptor for zero-copy GPU import.
    pub fn export_dmabuf(&self, surface_id: VASurfaceID) -> Result<DmaBufSurface, HwError> {
        unsafe {
            let mut descriptor: VADRMPRIMESurfaceDescriptor = std::mem::zeroed();
            let flags = VA_EXPORT_SURFACE_READ_ONLY | VA_EXPORT_SURFACE_SEPARATE_LAYERS;
            let status = (self.va.vaExportSurfaceHandle)(
                self.display,
                surface_id,
                VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2,
                flags,
                &mut descriptor as *mut _ as *mut std::ffi::c_void,
            );
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DecodeError(format!(
                    "vaExportSurfaceHandle failed: {}",
                    self.va.err_str(status)
                )));
            }

            Ok(DmaBufSurface {
                surface_id,
                descriptor,
            })
        }
    }

    /// Exports a decoded surface as a high-level DmaBufDescriptor.
    pub fn export_dmabuf_descriptor(
        &self,
        surface_id: VASurfaceID,
    ) -> Result<crate::DmaBufDescriptor, HwError> {
        if let Ok(img) = self.derive_image(surface_id) {
            let prime_res = self.acquire_prime_fd(&img);
            let desc = match prime_res {
                Ok(prime_fd) => {
                    let dup_fd = unsafe { libc::dup(prime_fd) };
                    unsafe {
                        (self.va.vaReleaseBufferHandle)(self.display, img.buf);
                    }
                    if dup_fd >= 0 {
                        let mut offsets = Vec::new();
                        let mut pitches = Vec::new();
                        for i in 0..img.num_planes as usize {
                            offsets.push(img.offsets[i]);
                            pitches.push(img.pitches[i]);
                        }
                        self.destroy_image(&img);
                        return Ok(crate::DmaBufDescriptor {
                            fd: dup_fd,
                            drm_format: img.format.fourcc,
                            drm_modifier: 0,
                            width: img.width as u32,
                            height: img.height as u32,
                            offsets,
                            pitches,
                        });
                    }
                    None
                }
                Err(_) => None,
            };
            self.destroy_image(&img);
            if let Some(d) = desc {
                return Ok(d);
            }
        }

        // Fallback: DRM PRIME 2 export via vaExportSurfaceHandle
        if let Ok(dmabuf_surf) = self.export_dmabuf(surface_id) {
            let desc = &dmabuf_surf.descriptor;
            if desc.num_objects > 0 && desc.objects[0].fd >= 0 {
                let dup_fd = unsafe { libc::dup(desc.objects[0].fd) };
                if dup_fd >= 0 {
                    let mut offsets = Vec::new();
                    let mut pitches = Vec::new();
                    let drm_format = if desc.num_layers > 0 {
                        desc.layers[0].drm_format
                    } else {
                        desc.fourcc
                    };
                    for i in 0..desc.num_layers as usize {
                        for p in 0..desc.layers[i].num_planes as usize {
                            offsets.push(desc.layers[i].offset[p]);
                            pitches.push(desc.layers[i].pitch[p]);
                        }
                    }
                    return Ok(crate::DmaBufDescriptor {
                        fd: dup_fd,
                        drm_format,
                        drm_modifier: desc.objects[0].drm_format_modifier,
                        width: desc.width,
                        height: desc.height,
                        offsets,
                        pitches,
                    });
                }
            }
        }

        Err(HwError::DecodeError(
            "Failed to export surface as DMA-BUF descriptor".into(),
        ))
    }

    /// Derives a VAImage from a hardware surface without copying.
    pub fn derive_image(&self, surface_id: VASurfaceID) -> Result<VAImage, HwError> {
        unsafe {
            let mut image: VAImage = std::mem::zeroed();
            let status = (self.va.vaDeriveImage)(self.display, surface_id, &mut image);
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DecodeError(format!(
                    "vaDeriveImage failed: {}",
                    self.va.err_str(status)
                )));
            }
            Ok(image)
        }
    }

    /// Destroys a derived VAImage.
    pub fn destroy_image(&self, image: &VAImage) {
        unsafe {
            let _ = (self.va.vaDestroyImage)(self.display, image.image_id);
        }
    }

    /// Acquires a DRM PRIME file descriptor from a derived image buffer.
    pub fn acquire_prime_fd(&self, image: &VAImage) -> Result<i32, HwError> {
        unsafe {
            let mut buf_info: VABufferInfo = std::mem::zeroed();
            buf_info.mem_type = VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME;
            let status = (self.va.vaAcquireBufferHandle)(self.display, image.buf, &mut buf_info);
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DecodeError(format!(
                    "vaAcquireBufferHandle failed: {}",
                    self.va.err_str(status)
                )));
            }
            Ok(buf_info.handle as i32)
        }
    }
}

impl Drop for SurfacePool {
    fn drop(&mut self) {
        unsafe {
            if self.context_id != 0 {
                let _ = (self.va.vaDestroyContext)(self.display, self.context_id);
            }
            if !self.surfaces.is_empty() {
                let _ = (self.va.vaDestroySurfaces)(
                    self.display,
                    self.surfaces.as_mut_ptr(),
                    self.surfaces.len() as u32,
                );
            }
        }
    }
}
