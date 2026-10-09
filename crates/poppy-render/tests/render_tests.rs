use poppy_render::{ColorSpace, GpuRenderer};

#[test]
fn test_wgpu_renderer_initialization_and_color_conversion() {
    // If no GPU is available in headless environment, GpuRenderer::new_sync will return AdapterNotFound.
    // That is graceful in CI or headless environments without Vulkan/OpenGL.
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping hardware rendering test (no GPU adapter): {e}");
            return;
        }
    };

    let width = 64;
    let height = 64;
    let stride_y = width;
    let stride_uv = width;

    // Create a pure green NV12 frame
    // In YUV (BT.709): Y ~ 173 (0.7152), U ~ 16 (0.0), V ~ 16 (0.0) -> roughly green
    let y_plane = vec![173u8; (width * height) as usize];
    let uv_plane = vec![128u8; ((width * height) / 2) as usize];

    let rgba = renderer
        .render_nv12_to_rgba(
            &y_plane,
            &uv_plane,
            width,
            height,
            stride_y,
            stride_uv,
            ColorSpace::Bt709Full,
        )
        .expect("render_nv12_to_rgba failed");

    assert_eq!(rgba.len(), (width * height * 4) as usize);
    // Alpha channel must be 255
    assert_eq!(rgba[3], 255);
}

#[test]
fn test_wgpu_renderer_p010_conversion() {
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping hardware rendering test (no GPU adapter): {e}");
            return;
        }
    };

    let width = 32;
    let height = 32;
    let stride_y = width;
    let stride_uv = width;

    let y_plane = vec![32768u16; (width * height) as usize];
    let uv_plane = vec![32768u16; ((width * height) / 2) as usize];

    match renderer.render_p010_to_rgba(
        &y_plane,
        &uv_plane,
        width,
        height,
        stride_y,
        stride_uv,
        ColorSpace::Bt2020Full,
    ) {
        Ok(rgba) => {
            assert_eq!(rgba.len(), (width * height * 4) as usize);
            assert_eq!(rgba[3], 255);
        }
        Err(poppy_render::RenderError::UnsupportedFormat(msg)) => {
            eprintln!("Skipping P010 test on this GPU: {msg}");
        }
        Err(e) => panic!("Unexpected render error: {e}"),
    }
}
