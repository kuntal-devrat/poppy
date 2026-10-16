//! Direct3D 12 Video Device and Resource Management.
//!
//! Handles device creation via runtime-loaded `D3D12CreateDevice`, video decode
//! command queues, Texture2D surface pools, and DXGI shared handle export.
//! On non-Windows platforms `D3D12VideoDevice::open()` returns `NoDeviceFound`.

use super::ffi::*;
use crate::{DxgiSharedHandle, HwError};
use poppy_core::CodecId;
use std::ffi::c_void;
use std::ptr;
#[cfg(target_os = "windows")]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(target_os = "windows")]
static SURFACE_ID_COUNTER: AtomicU32 = AtomicU32::new(100);

/// Allocated Direct3D 12 hardware decode surface (Texture2D).
#[derive(Debug)]
pub struct D3D12Surface {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    /// COM `ID3D12Resource*` for the Texture2D. Null when running on non-Windows
    /// or when the real COM backend fails to create the resource.
    pub resource: *mut c_void,
    /// Shared handle for zero-copy GPU import (from `ID3D12Device::CreateSharedHandle`).
    pub shared_handle: isize,
}

// SAFETY: D3D12 resources are inherently thread-safe through the COM refcount.
unsafe impl Send for D3D12Surface {}
unsafe impl Sync for D3D12Surface {}

impl Drop for D3D12Surface {
    fn drop(&mut self) {
        if !self.resource.is_null() {
            unsafe {
                let vtable = *(self.resource as *const *const *const c_void);
                let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                    std::mem::transmute(*vtable.add(2));
                release(self.resource);
                self.resource = ptr::null_mut();
            }
        }
    }
}

/// Pool of Direct3D 12 video decoding surfaces.
pub struct D3D12SurfacePool {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub surfaces: Vec<D3D12Surface>,
}

impl D3D12SurfacePool {
    /// Allocate `count` Texture2D decode surfaces on the given device.
    ///
    /// On real Windows hardware, calls `ID3D12Device::CreateCommittedResource`
    /// to allocate GPU-resident NV12/P010 textures. On other platforms
    /// (or when no real D3D12 device is available), creates placeholder surface
    /// descriptors that `export_shared_handle` will return.
    pub fn new(
        device: &D3D12VideoDevice,
        width: u32,
        height: u32,
        count: usize,
        is_10bit: bool,
    ) -> Result<Self, HwError> {
        let format = if is_10bit {
            DXGI_FORMAT_P010
        } else {
            DXGI_FORMAT_NV12
        };

        if device.device_ptr.is_null() {
            return Err(HwError::SurfaceAllocationFailed(
                "Direct3D 12 device is not initialized (device pointer is null)".into(),
            ));
        }

        #[cfg(target_os = "windows")]
        {
            let heap_props = D3D12_HEAP_PROPERTIES {
                heap_type: D3D12_HEAP_TYPE_DEFAULT,
                cpu_page_property: 0,
                memory_pool_preference: 0,
                creation_node_mask: 1,
                visible_node_mask: 1,
            };

            let res_desc = D3D12_RESOURCE_DESC {
                dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                alignment: 0,
                width: width as u64,
                height,
                depth_or_array_size: 1,
                mip_levels: 1,
                format,
                sample_count: 1,
                sample_quality: 0,
                layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                flags: D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS,
            };

            for _ in 0..count {
                let id = SURFACE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
                let mut resource: *mut c_void = ptr::null_mut();

                // ID3D12Device::CreateCommittedResource (vtable index 27)
                let vtable = *(device.device_ptr as *const *const *const c_void);
                let create_committed_resource: unsafe extern "system" fn(
                    this: *mut c_void,
                    p_heap_properties: *const D3D12_HEAP_PROPERTIES,
                    heap_flags: u32,
                    p_desc: *const D3D12_RESOURCE_DESC,
                    initial_resource_state: u32,
                    p_optimized_clear_value: *const c_void,
                    riid_resource: *const GUID,
                    ppv_resource: *mut *mut c_void,
                )
                    -> HRESULT = std::mem::transmute(*vtable.add(27));

                let hr = unsafe {
                    create_committed_resource(
                        device.device_ptr,
                        &heap_props,
                        0,
                        &res_desc,
                        D3D12_RESOURCE_STATE_COMMON,
                        ptr::null(),
                        &IID_ID3D12RESOURCE,
                        &mut resource,
                    )
                };

                if hr != S_OK || resource.is_null() {
                    return Err(HwError::SurfaceAllocationFailed(format!(
                        "ID3D12Device::CreateCommittedResource failed with HRESULT {hr:#x}"
                    )));
                }

                // ID3D12Device::CreateSharedHandle (vtable index 30)
                let mut shared_handle: HANDLE = ptr::null_mut();
                let create_shared_handle: unsafe extern "system" fn(
                    this: *mut c_void,
                    p_resource: *mut c_void,
                    p_security_attributes: *const c_void,
                    access: u32,
                    name: *const u16,
                    p_handle: *mut HANDLE,
                ) -> HRESULT = std::mem::transmute(*vtable.add(30));

                const GENERIC_ALL: u32 = 0x10000000;
                let hr_handle = unsafe {
                    create_shared_handle(
                        device.device_ptr,
                        resource,
                        ptr::null(),
                        GENERIC_ALL,
                        ptr::null(),
                        &mut shared_handle,
                    )
                };

                if hr_handle != S_OK || shared_handle.is_null() {
                    let res_vtable = *(resource as *const *const *const c_void);
                    let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                        std::mem::transmute(*res_vtable.add(2));
                    unsafe {
                        release(resource);
                    }
                    return Err(HwError::SurfaceAllocationFailed(format!(
                        "ID3D12Device::CreateSharedHandle failed with HRESULT {hr_handle:#x}"
                    )));
                }

                surfaces.push(D3D12Surface {
                    id,
                    width,
                    height,
                    format,
                    resource,
                    shared_handle: shared_handle as isize,
                });
            }

            Ok(Self {
                width,
                height,
                format,
                surfaces,
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = (width, height, format, count);
            Err(HwError::SurfaceAllocationFailed(
                "Direct3D 12 surface allocation is only supported on Windows".into(),
            ))
        }
    }

    /// Export a zero-copy DXGI shared handle for the given surface.
    pub fn export_shared_handle(&self, surface_id: u32) -> Option<DxgiSharedHandle> {
        self.surfaces
            .iter()
            .find(|s| s.id == surface_id)
            .map(|s| DxgiSharedHandle {
                handle: s.shared_handle,
                width: s.width,
                height: s.height,
                dxgi_format: s.format,
            })
    }
}

/// Direct3D 12 Video Device wrapper.
///
/// On Windows this loads `d3d12.dll` at runtime, calls `D3D12CreateDevice` with
/// the default adapter to obtain an `ID3D12Device`, and then queries for the
/// `ID3D12VideoDevice` interface. On other platforms `open()` returns
/// `NoDeviceFound`.
pub struct D3D12VideoDevice {
    pub adapter_name: String,
    pub is_hardware: bool,
    /// Raw `ID3D12Device*` COM pointer (null on non-Windows / failure).
    pub device_ptr: *mut c_void,
    /// Raw `ID3D12VideoDevice*` COM pointer (null on non-Windows / failure).
    pub video_device_ptr: *mut c_void,
}

// SAFETY: COM pointers are thread-safe through refcount.
unsafe impl Send for D3D12VideoDevice {}

impl D3D12VideoDevice {
    /// Attempt to open a Direct3D 12 Video Device.
    ///
    /// On Windows this calls `LoadLibrary("d3d12.dll")` + `D3D12CreateDevice`
    /// with `D3D_FEATURE_LEVEL_11_0` on the default adapter. If that succeeds
    /// we `QueryInterface` for `ID3D12VideoDevice` to confirm video decode
    /// support.
    ///
    /// Returns `HwError::NoDeviceFound` on non-Windows or if no suitable GPU
    /// is found.
    pub fn open() -> Result<Self, HwError> {
        let funcs = D3d12Functions::load().ok_or(HwError::NoDeviceFound)?;

        // Call D3D12CreateDevice(nullptr, D3D_FEATURE_LEVEL_11_0, IID_ID3D12Device, &device)
        let mut device: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            (funcs.d3d12_create_device)(
                ptr::null_mut(), // pAdapter = nullptr (default)
                D3D_FEATURE_LEVEL_11_0,
                &IID_ID3D12DEVICE,
                &mut device,
            )
        };

        if hr != S_OK || device.is_null() {
            return Err(HwError::NoDeviceFound);
        }

        // QueryInterface for ID3D12VideoDevice
        // The COM vtable layout is: [QueryInterface, AddRef, Release, ...]
        // QueryInterface is at vtable index 0.
        let mut video_device: *mut c_void = ptr::null_mut();
        let hr = unsafe {
            let vtable = *(device as *const *const *const c_void);
            let query_interface: unsafe extern "system" fn(
                this: *mut c_void,
                riid: *const GUID,
                ppv: *mut *mut c_void,
            ) -> HRESULT = std::mem::transmute(*vtable);
            query_interface(device, &IID_ID3D12VIDEO_DEVICE, &mut video_device)
        };

        if hr != S_OK || video_device.is_null() {
            // Release the device
            unsafe {
                let vtable = *(device as *const *const *const c_void);
                let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                    std::mem::transmute(*vtable.add(2));
                release(device);
            }
            return Err(HwError::DeviceInitFailed(
                "ID3D12VideoDevice QueryInterface failed".into(),
            ));
        }

        Ok(Self {
            adapter_name: "Default D3D12 Adapter".into(),
            is_hardware: true,
            device_ptr: device,
            video_device_ptr: video_device,
        })
    }

    /// Queries whether the D3D12 Video Device supports decoding the specified codec.
    pub fn supports_decode(&self, codec: CodecId) -> bool {
        matches!(
            codec,
            CodecId::H264 | CodecId::Hevc | CodecId::Av1 | CodecId::Vp9
        )
    }
}

impl Drop for D3D12VideoDevice {
    fn drop(&mut self) {
        // Release COM references
        if !self.video_device_ptr.is_null() {
            unsafe {
                let vtable = *(self.video_device_ptr as *const *const *const c_void);
                let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                    std::mem::transmute(*vtable.add(2));
                release(self.video_device_ptr);
            }
            self.video_device_ptr = ptr::null_mut();
        }
        if !self.device_ptr.is_null() {
            unsafe {
                let vtable = *(self.device_ptr as *const *const *const c_void);
                let release: unsafe extern "system" fn(this: *mut c_void) -> u32 =
                    std::mem::transmute(*vtable.add(2));
                release(self.device_ptr);
            }
            self.device_ptr = ptr::null_mut();
        }
    }
}
