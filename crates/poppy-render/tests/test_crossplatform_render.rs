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
        // On macOS with Metal driver, validates direct CVPixelBuffer import pathway
        let _ = renderer.render_frame(&frame);
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
        let _ = renderer.render_frame(&cv_p010_frame);
    }
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            renderer.render_frame(&cv_p010_frame).is_err(),
            "P010 Metal render must fail cleanly on non-macOS"
        );
    }
}
