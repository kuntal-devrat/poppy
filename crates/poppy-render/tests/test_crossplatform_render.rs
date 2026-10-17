use poppy_core::{InterlaceMode, PixelFormat};
use poppy_hw::{CvPixelBufferDescriptor, DecodedFrame, DxgiSharedHandle};
use poppy_render::GpuRenderer;

#[test]
fn test_crossplatform_dxgi_zerocopy_render() {
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: wgpu GPU device not available ({e})");
            return;
        }
    };

    let width = 64;
    let height = 64;

    let frame = DecodedFrame {
        pts: Some(100),
        width,
        height,
        pixel_format: PixelFormat::Nv12,
        surface_id: 1,
        data: None,
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: Some(DxgiSharedHandle {
            handle: 1,
            width,
            height,
            dxgi_format: 103, // DXGI_FORMAT_NV12
        }),
        cv_pixel_buffer: None,
        is_concealed: false,
    };

    #[cfg(target_os = "windows")]
    {
        // On Windows with real graphics driver, validates direct DXGI import pathway
        let _ = renderer.render_frame(&frame);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let res = renderer.render_frame(&frame);
        assert!(
            res.is_err(),
            "DXGI shared handle import must fail cleanly on non-Windows platforms"
        );
    }
}

#[cfg(target_os = "macos")]
fn create_test_cv_pixel_buffer(
    width: usize,
    height: usize,
    pixel_format_type: u32,
) -> Option<*mut std::ffi::c_void> {
    #[link(name = "CoreVideo", kind = "framework")]
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferCreate(
            allocator: *const std::ffi::c_void,
            width: usize,
            height: usize,
            pixel_format_type: u32,
            pixel_buffer_attributes: *const std::ffi::c_void,
            pixel_buffer_out: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn CFDictionaryCreateMutable(
            allocator: *const std::ffi::c_void,
            capacity: isize,
            key_callbacks: *const std::ffi::c_void,
            value_callbacks: *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
        fn CFDictionarySetValue(
            dict: *mut std::ffi::c_void,
            key: *const std::ffi::c_void,
            value: *const std::ffi::c_void,
        );
        fn CFRelease(cf: *const std::ffi::c_void);
        static kCFTypeDictionaryKeyCallBacks: std::ffi::c_void;
        static kCFTypeDictionaryValueCallBacks: std::ffi::c_void;
    }

    let cv_handle = unsafe {
        libc::dlopen(
            b"/System/Library/Frameworks/CoreVideo.framework/CoreVideo\0".as_ptr() as *const _,
            libc::RTLD_LAZY,
        )
    };
    if cv_handle.is_null() {
        return None;
    }
    let key_sym = unsafe {
        libc::dlsym(
            cv_handle,
            b"kCVPixelBufferIOSurfacePropertiesKey\0".as_ptr() as *const _,
        )
    };
    if key_sym.is_null() {
        return None;
    }
    let io_surface_key = unsafe { *(key_sym as *const *const std::ffi::c_void) };

    unsafe {
        let key_cbs = &kCFTypeDictionaryKeyCallBacks as *const _ as *const std::ffi::c_void;
        let val_cbs = &kCFTypeDictionaryValueCallBacks as *const _ as *const std::ffi::c_void;
        let attrs = CFDictionaryCreateMutable(std::ptr::null(), 0, key_cbs, val_cbs);
        let io_dict = CFDictionaryCreateMutable(std::ptr::null(), 0, key_cbs, val_cbs);
        CFDictionarySetValue(attrs, io_surface_key, io_dict as *const _);
        CFRelease(io_dict as *const _);

        let mut pixel_buffer: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = CVPixelBufferCreate(
            std::ptr::null(),
            width,
            height,
            pixel_format_type,
            attrs as *const _,
            &mut pixel_buffer,
        );
        CFRelease(attrs as *const _);

        if status == 0 && !pixel_buffer.is_null() {
            Some(pixel_buffer)
        } else {
            None
        }
    }
}

#[test]
fn test_crossplatform_cvpixelbuffer_zerocopy_render() {
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: wgpu GPU device not available ({e})");
            return;
        }
    };

    let width = 64;
    let height = 64;

    let frame = DecodedFrame {
        pts: Some(200),
        width,
        height,
        pixel_format: PixelFormat::Nv12,
        surface_id: 2,
        data: None,
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: None,
        cv_pixel_buffer: Some(CvPixelBufferDescriptor {
            pixel_buffer_ptr: 1,
            io_surface_ptr: 1,
            width,
            height,
            pixel_format_type: 0x34323066, // '420f' (kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
        }),
        is_concealed: false,
    };

    #[cfg(target_os = "macos")]
    {
        // On macOS with Metal driver, allocates a real IOSurface-backed CVPixelBuffer if available
        if let Some(pb) = create_test_cv_pixel_buffer(width as usize, height as usize, 0x34323066) {
            #[link(name = "CoreVideo", kind = "framework")]
            #[link(name = "CoreFoundation", kind = "framework")]
            unsafe extern "C" {
                fn CVPixelBufferGetIOSurface(pb: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
                fn CFRelease(cf: *const std::ffi::c_void);
            }
            let io_surf = unsafe { CVPixelBufferGetIOSurface(pb) };
            let real_frame = DecodedFrame {
                pts: Some(200),
                width,
                height,
                pixel_format: PixelFormat::Nv12,
                surface_id: 2,
                data: None,
                interlace_mode: InterlaceMode::Progressive,
                dmabuf: None,
                dxgi_handle: None,
                cv_pixel_buffer: Some(CvPixelBufferDescriptor {
                    pixel_buffer_ptr: pb as usize,
                    io_surface_ptr: io_surf as usize,
                    width,
                    height,
                    pixel_format_type: 0x34323066,
                }),
                is_concealed: false,
            };
            let res = renderer.render_frame(&real_frame);
            unsafe {
                CFRelease(pb as *const _);
            }
            assert!(
                res.is_ok(),
                "Real CVPixelBuffer / IOSurface render should succeed on macOS: {:?}",
                res.err()
            );
        } else {
            let res = renderer.render_frame(&frame);
            assert!(
                res.is_err(),
                "Sentinel pointer frame should fail cleanly without crashing"
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let res = renderer.render_frame(&frame);
        assert!(
            res.is_err(),
            "CVPixelBuffer / IOSurface import must fail cleanly on non-macOS platforms"
        );
    }
}

#[test]
fn test_crossplatform_p010_hdr_zerocopy_render() {
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping test: wgpu GPU device not available ({e})");
            return;
        }
    };

    let width = 64;
    let height = 64;

    let dxgi_p010_frame = DecodedFrame {
        pts: Some(300),
        width,
        height,
        pixel_format: PixelFormat::P010,
        surface_id: 3,
        data: None,
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: Some(DxgiSharedHandle {
            handle: 1,
            width,
            height,
            dxgi_format: 104, // DXGI_FORMAT_P010
        }),
        cv_pixel_buffer: None,
        is_concealed: false,
    };

    #[cfg(target_os = "windows")]
    {
        let _ = renderer.render_frame(&dxgi_p010_frame);
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            renderer.render_frame(&dxgi_p010_frame).is_err(),
            "P010 DXGI render must fail cleanly on non-Windows"
        );
    }

    let cv_p010_frame = DecodedFrame {
        pts: Some(400),
        width,
        height,
        pixel_format: PixelFormat::P010,
        surface_id: 4,
        data: None,
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: None,
        cv_pixel_buffer: Some(CvPixelBufferDescriptor {
            pixel_buffer_ptr: 1,
            io_surface_ptr: 1,
            width,
            height,
            pixel_format_type: 0x78343230, // 'x420' (kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange)
        }),
        is_concealed: false,
    };

    #[cfg(target_os = "macos")]
    {
        if let Some(pb) = create_test_cv_pixel_buffer(width as usize, height as usize, 0x78343230) {
            #[link(name = "CoreVideo", kind = "framework")]
            #[link(name = "CoreFoundation", kind = "framework")]
            unsafe extern "C" {
                fn CVPixelBufferGetIOSurface(pb: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
                fn CFRelease(cf: *const std::ffi::c_void);
            }
            let io_surf = unsafe { CVPixelBufferGetIOSurface(pb) };
            let real_p010_frame = DecodedFrame {
                pts: Some(400),
                width,
                height,
                pixel_format: PixelFormat::P010,
                surface_id: 4,
                data: None,
                interlace_mode: InterlaceMode::Progressive,
                dmabuf: None,
                dxgi_handle: None,
                cv_pixel_buffer: Some(CvPixelBufferDescriptor {
                    pixel_buffer_ptr: pb as usize,
                    io_surface_ptr: io_surf as usize,
                    width,
                    height,
                    pixel_format_type: 0x78343230,
                }),
                is_concealed: false,
            };
            let res = renderer.render_frame(&real_p010_frame);
            unsafe {
                CFRelease(pb as *const _);
            }
            assert!(
                res.is_ok(),
                "Real P010 CVPixelBuffer / IOSurface render should succeed on macOS: {:?}",
                res.err()
            );
        } else {
            let res = renderer.render_frame(&cv_p010_frame);
            assert!(
                res.is_err(),
                "Sentinel pointer P010 frame should fail cleanly without crashing"
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            renderer.render_frame(&cv_p010_frame).is_err(),
            "P010 Metal render must fail cleanly on non-macOS"
        );
    }
}
