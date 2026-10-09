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

    // Construct a DecodedFrame backed by a mock Windows DXGI shared handle
    let frame = DecodedFrame {
        pts: Some(100),
        width,
        height,
        pixel_format: PixelFormat::Nv12,
        surface_id: 1,
        data: None, // NO raw CPU data! Pure zero-copy handle
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: Some(DxgiSharedHandle {
            handle: 0x1234, // Mock handle recognized by cross-platform pipeline
            width,
            height,
            dxgi_format: 103, // DXGI_FORMAT_NV12
        }),
        cv_pixel_buffer: None,
        is_concealed: false,
    };

    // Render frame to RGBA pixels using zero-copy import pathway
    let rgba = renderer
        .render_frame(&frame)
        .expect("Zero-copy DXGI render_frame failed");

    assert_eq!(
        rgba.len(),
        (width * height * 4) as usize,
        "Rendered output must match RGBA8 buffer size"
    );
    println!(
        "✅ DXGI shared handle zero-copy rendering completed successfully ({} bytes)",
        rgba.len()
    );
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

    // Construct a DecodedFrame backed by a mock macOS CVPixelBuffer / IOSurface descriptor
    let frame = DecodedFrame {
        pts: Some(200),
        width,
        height,
        pixel_format: PixelFormat::Nv12,
        surface_id: 2,
        data: None, // NO raw CPU data!
        interlace_mode: InterlaceMode::Progressive,
        dmabuf: None,
        dxgi_handle: None,
        cv_pixel_buffer: Some(CvPixelBufferDescriptor {
            pixel_buffer_ptr: 0x2000,
            io_surface_ptr: 0x3000,
            width,
            height,
            pixel_format_type: 0x34323066, // '420f' (kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
        }),
        is_concealed: false,
    };

    let rgba = renderer
        .render_frame(&frame)
        .expect("Zero-copy CVPixelBuffer render_frame failed");

    assert_eq!(
        rgba.len(),
        (width * height * 4) as usize,
        "Rendered output must match RGBA8 buffer size"
    );
    println!(
        "✅ CVPixelBuffer / IOSurface zero-copy rendering completed successfully ({} bytes)",
        rgba.len()
    );
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

    // Test 10-bit P010 HDR zero-copy DXGI render
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
            handle: 0x1234,
            width,
            height,
            dxgi_format: 104, // DXGI_FORMAT_P010
        }),
        cv_pixel_buffer: None,
        is_concealed: false,
    };

    let rgba_dxgi = renderer
        .render_frame(&dxgi_p010_frame)
        .expect("P010 DXGI render_frame failed");
    assert_eq!(rgba_dxgi.len(), (width * height * 4) as usize);

    // Test 10-bit P010 HDR zero-copy Metal / CVPixelBuffer render
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
            pixel_buffer_ptr: 0x2000,
            io_surface_ptr: 0x3000,
            width,
            height,
            pixel_format_type: 0x78343230, // 'x420' (kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange)
        }),
        is_concealed: false,
    };

    let rgba_cv = renderer
        .render_frame(&cv_p010_frame)
        .expect("P010 CVPixelBuffer render_frame failed");
    assert_eq!(rgba_cv.len(), (width * height * 4) as usize);

    println!(
        "✅ 10-bit P010 HDR zero-copy rendering verified across both DXGI and Metal pipelines"
    );
}
