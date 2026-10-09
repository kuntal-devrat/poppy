//! DRM render node discovery and VA-API device initialization.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::ffi::*;
use crate::HwError;

/// A configured VA-API device backed by a DRM render node.
pub struct VaDevice {
    pub va: Arc<VaApi>,
    pub display: VADisplay,
    pub config_id: VAConfigID,
    pub profile: VAProfile,
    pub version: (i32, i32),
    _drm_file: File,
}

unsafe impl Send for VaDevice {}
unsafe impl Sync for VaDevice {}

impl VaDevice {
    /// Discovers an available DRM render node (`/dev/dri/renderD*`) and initializes
    /// the VA-API display for H.264 VLD decoding.
    pub fn open_auto() -> Result<Self, HwError> {
        Self::open_auto_for_profiles(&[
            VA_PROFILE_H264_HIGH,
            VA_PROFILE_H264_MAIN,
            VA_PROFILE_H264_CONSTRAINED_BASELINE,
            VA_PROFILE_H264_BASELINE,
        ])
    }

    /// Discovers an available DRM render node and initializes VA-API for HEVC VLD decoding.
    pub fn open_auto_hevc(is_10bit: bool) -> Result<Self, HwError> {
        if is_10bit {
            Self::open_auto_for_profiles(&[VA_PROFILE_HEVC_MAIN10])
        } else {
            Self::open_auto_for_profiles(&[VA_PROFILE_HEVC_MAIN, VA_PROFILE_HEVC_MAIN10])
        }
    }

    /// Discovers an available DRM render node and initializes VA-API for AV1 VLD decoding.
    pub fn open_auto_av1(_is_10bit: bool) -> Result<Self, HwError> {
        Self::open_auto_for_profiles(&[VA_PROFILE_AV1_PROFILE0, VA_PROFILE_AV1_PROFILE1])
    }

    /// Discovers an available DRM render node matching one of the desired profiles.
    pub fn open_auto_for_profiles(desired_profiles: &[VAProfile]) -> Result<Self, HwError> {
        let va = VaApi::load().map_err(HwError::DeviceInitFailed)?;

        // Scan DRM render nodes in order
        for num in 128..136 {
            let path = PathBuf::from(format!("/dev/dri/renderD{}", num));
            if path.exists()
                && let Ok(dev) =
                    Self::open_with_va_profiles(&path, Arc::clone(&va), desired_profiles)
            {
                return Ok(dev);
            }
        }

        Err(HwError::NoDeviceFound)
    }

    /// Open a specific DRM device path for H.264.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, HwError> {
        let va = VaApi::load().map_err(HwError::DeviceInitFailed)?;
        Self::open_with_va_profiles(
            path.as_ref(),
            va,
            &[
                VA_PROFILE_H264_HIGH,
                VA_PROFILE_H264_MAIN,
                VA_PROFILE_H264_CONSTRAINED_BASELINE,
                VA_PROFILE_H264_BASELINE,
            ],
        )
    }

    fn open_with_va_profiles(
        path: &Path,
        va: Arc<VaApi>,
        desired_profiles: &[VAProfile],
    ) -> Result<Self, HwError> {
        let drm_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| {
                HwError::DeviceInitFailed(format!("Failed to open {}: {}", path.display(), e))
            })?;

        let raw_fd: RawFd = drm_file.as_raw_fd();

        unsafe {
            let display = (va.vaGetDisplayDRM)(raw_fd);
            if display.is_null() {
                return Err(HwError::DeviceInitFailed(
                    "vaGetDisplayDRM returned null".into(),
                ));
            }

            let mut major = 0;
            let mut minor = 0;
            let status = (va.vaInitialize)(display, &mut major, &mut minor);
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DeviceInitFailed(format!(
                    "vaInitialize failed: {}",
                    va.err_str(status)
                )));
            }

            // Query profiles
            let max_profiles = (va.vaMaxNumProfiles)(display);
            let mut profiles = vec![0 as VAProfile; max_profiles as usize];
            let mut num_profiles = 0;
            let status =
                (va.vaQueryConfigProfiles)(display, profiles.as_mut_ptr(), &mut num_profiles);
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaTerminate)(display);
                return Err(HwError::DeviceInitFailed(format!(
                    "vaQueryConfigProfiles failed: {}",
                    va.err_str(status)
                )));
            }
            profiles.truncate(num_profiles as usize);

            // Select best matching profile from desired list
            let target_profile = match desired_profiles.iter().find(|&&p| profiles.contains(&p)) {
                Some(&p) => p,
                None => {
                    let _ = (va.vaTerminate)(display);
                    return Err(HwError::UnsupportedCodec);
                }
            };

            // Create VAConfig for VLD decode
            let mut config_id = 0;
            let status = (va.vaCreateConfig)(
                display,
                target_profile,
                VA_ENTRYPOINT_VLD,
                std::ptr::null_mut(),
                0,
                &mut config_id,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaTerminate)(display);
                return Err(HwError::DeviceInitFailed(format!(
                    "vaCreateConfig failed: {}",
                    va.err_str(status)
                )));
            }

            Ok(Self {
                va,
                display,
                config_id,
                profile: target_profile,
                version: (major, minor),
                _drm_file: drm_file,
            })
        }
    }
}

impl Drop for VaDevice {
    fn drop(&mut self) {
        unsafe {
            if self.config_id != 0 {
                let _ = (self.va.vaDestroyConfig)(self.display, self.config_id);
            }
            if !self.display.is_null() {
                let _ = (self.va.vaTerminate)(self.display);
            }
        }
    }
}
