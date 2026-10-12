#![cfg(unix)]

use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_dmabuf_desc_negative_fd_safe(
        neg_fd in -1000i32..=-1i32,
        w in 0u32..8192u32,
        h in 0u32..8192u32,
    ) {
        let d = poppy_hw::DmaBufDescriptor {
            fd: neg_fd,
            drm_format: 0x3231564e,
            drm_modifier: 0,
            width: w,
            height: h,
            offsets: vec![0, w * h],
            pitches: vec![w, w],
        };
        let res = d.try_clone();
        prop_assert!(res.is_ok());
        prop_assert_eq!(res.unwrap().fd, -1);
    }
}

#[test]
fn test_dmabuf_desc_valid_fd_clone() {
    use std::os::fd::IntoRawFd;
    let file = std::fs::File::open("/dev/null").expect("open /dev/null");
    let raw = file.into_raw_fd();
    let d = poppy_hw::DmaBufDescriptor {
        fd: raw,
        drm_format: 0x3231564e,
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };
    let cloned = d.try_clone().expect("try_clone must succeed on valid fd");
    assert!(cloned.fd >= 0);
    assert_ne!(cloned.fd, d.fd);

    let cloned2 = d.clone();
    assert!(cloned2.fd >= 0);
    assert_ne!(cloned2.fd, d.fd);
    assert_ne!(cloned2.fd, cloned.fd);
}

#[test]
fn test_dmabuf_desc_invalid_fd_returns_err() {
    let mut d = poppy_hw::DmaBufDescriptor {
        fd: 999999,
        drm_format: 0x3231564e,
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };
    let res = d.try_clone();
    assert!(res.is_err());
    d.fd = -1;
}
