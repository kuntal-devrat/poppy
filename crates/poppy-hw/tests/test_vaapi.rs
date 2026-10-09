use poppy_hw::vaapi::{SurfacePool, VaDevice};

#[test]
#[cfg(target_os = "linux")]
fn test_vaapi_hardware_init_and_dmabuf_export() {
    let dev = match VaDevice::open_auto() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping hardware test: no VA-API device available ({e})");
            return;
        }
    };

    println!(
        "✅ VA-API initialized successfully: driver version {}.{}",
        dev.version.0, dev.version.1
    );

    // Allocate 4 NV12 hardware surfaces for 1920x1080
    let pool = SurfacePool::new(dev.va.clone(), dev.display, dev.config_id, 1920, 1080, 4)
        .expect("Failed to allocate hardware surfaces");

    assert_eq!(pool.surfaces.len(), 4);

    // Derive image from hardware surface
    let img = pool
        .derive_image(pool.surfaces[0])
        .expect("Failed to derive image from surface");
    println!(
        "✅ Derived VAImage: format fourcc={:#x}, size={}x{}, data_size={}, num_planes={}",
        img.format.fourcc, img.width, img.height, img.data_size, img.num_planes
    );
    assert_eq!(img.width, 1920);
    assert_eq!(img.height, 1080);
    assert_eq!(img.num_planes, 2);

    // Acquire PRIME buffer handle
    let prime_fd = pool
        .acquire_prime_fd(&img)
        .expect("Failed to acquire DRM PRIME buffer handle");
    println!("✅ Acquired DRM PRIME DMA-BUF fd: {}", prime_fd);
    assert!(prime_fd >= 0);

    // Clean up derived image
    pool.destroy_image(&img);
}
