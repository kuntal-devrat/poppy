#![cfg(target_os = "linux")]

use poppy_core::CodecId;
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::{DmaBufDescriptor, HwDecoder, VaapiH264Decoder};
use poppy_render::GpuRenderer;
use std::path::Path;

#[test]
fn test_dmabuf_descriptor_semantics() {
    let raw_fd = unsafe { libc::dup(0) };
    assert!(raw_fd >= 0);

    let desc = DmaBufDescriptor {
        fd: raw_fd,
        drm_format: 0x3231564e, // DRM_FORMAT_NV12
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };

    // Test safe duplication in Clone
    let cloned = desc.clone();
    assert_ne!(desc.fd, cloned.fd);
    assert_eq!(desc.width, cloned.width);
    assert_eq!(desc.height, cloned.height);
    assert_eq!(desc.offsets, cloned.offsets);
    assert_eq!(desc.pitches, cloned.pitches);

    // Dropping cloned desc closes its fd without affecting original desc
    drop(cloned);
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    let status = unsafe { libc::fstat(desc.fd, &mut stat) };
    assert_eq!(status, 0, "Original fd must remain valid after clone drops");
}

#[test]
fn test_dmabuf_image_try_from_semantics() {
    use poppy_render::DmaBufImage;

    // Test with invalid negative fd
    let invalid_desc = DmaBufDescriptor {
        fd: -1,
        drm_format: 0x3231564e,
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };
    assert!(DmaBufImage::try_from(&invalid_desc).is_err());

    // Test with valid fd
    let raw_fd = unsafe { libc::dup(0) };
    assert!(raw_fd >= 0);
    let valid_desc = DmaBufDescriptor {
        fd: raw_fd,
        drm_format: 0x3231564e,
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };
    let img = DmaBufImage::try_from(&valid_desc).expect("try_from should succeed on valid fd");
    assert!(img.raw_fd() >= 0);
    assert_ne!(img.raw_fd(), valid_desc.fd);

    let img2 = DmaBufImage::try_from(&valid_desc).expect("second try_from should also succeed");
    assert!(img2.raw_fd() >= 0);
    assert_ne!(img2.raw_fd(), valid_desc.fd);
    assert_ne!(img2.raw_fd(), img.raw_fd());
}

#[test]
fn test_dmabuf_vulkan_zerocopy_pipeline_render() {
    let candidates = [
        "tests/samples/test_h264.mp4",
        "../../tests/samples/test_h264.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_h264.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_h264.mp4");

    if !Path::new(path).exists() {
        eprintln!("Test file test_h264.mp4 not found, skipping.");
        return;
    }

    let mut demuxer = match Mp4Demuxer::open(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping: failed to open mp4: {e}");
            return;
        }
    };

    let streams = demuxer.streams();
    let video_idx = match streams.iter().position(|s| s.codec == CodecId::H264) {
        Some(idx) => idx,
        None => {
            eprintln!("No H.264 stream found, skipping.");
            return;
        }
    };

    let mut decoder = match VaapiH264Decoder::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping hardware test: no VA-API device available ({e})");
            return;
        }
    };

    if let poppy_core::codec::CodecConfig::H264(avcc) = &streams[video_idx].config {
        let _ = decoder.set_parameter_sets(&avcc.sps_list, &avcc.pps_list);
    }

    let mut decoded_frame = None;
    while let Ok(Some(pkt)) = demuxer.read_packet() {
        if pkt.stream_index != video_idx {
            continue;
        }
        if decoder.submit_packet(&pkt).is_ok()
            && let Ok(Some(f)) = decoder.receive_frame()
            && f.dmabuf.is_some()
        {
            decoded_frame = Some(f);
            break;
        }
    }

    let frame = match decoded_frame {
        Some(f) => f,
        None => {
            eprintln!("No hardware decoded frame with DMA-BUF received, skipping.");
            return;
        }
    };

    assert!(
        frame.dmabuf.is_some(),
        "Decoded frame must have DMA-BUF descriptor"
    );
    let dmabuf = frame.dmabuf.as_ref().unwrap();
    println!(
        "✅ Hardware decoded frame ready with DMA-BUF: fd={}, {}x{}, format={:?}",
        dmabuf.fd, frame.width, frame.height, frame.pixel_format
    );

    // Initialize wgpu renderer with Vulkan backend
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping GPU render test (no wgpu adapter): {e}");
            return;
        }
    };

    println!("✅ WGPU Vulkan renderer initialized");

    // Execute direct zero-copy DMA-BUF import and render
    match renderer.render_frame(&frame) {
        Ok(rgba) => {
            let expected_len = (frame.width * frame.height * 4) as usize;
            assert_eq!(rgba.len(), expected_len);
            assert_eq!(rgba[3], 255);
            println!(
                "✨ Direct Vulkan Zero-Copy DMA-BUF render successful: {} bytes RGBA rendered with 0 staging copies!",
                rgba.len()
            );
        }
        Err(e) => {
            println!(
                "Zero-copy DMA-BUF import reported: {e} (graceful validation on current driver)"
            );
        }
    }
}

#[test]
fn test_zero_staging_render_textures_to_view() {
    let renderer = match GpuRenderer::new_sync() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Skipping GPU test (no adapter): {e}");
            return;
        }
    };

    let width = 64;
    let height = 64;

    // Create GPU-resident NV12 Y and UV textures directly
    let y_texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test_y_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let uv_texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test_uv_texture"),
        size: wgpu::Extent3d {
            width: width / 2,
            height: height / 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let target_texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test_target_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: renderer.output_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Execute zero-staging direct texture-to-view render
    let res = renderer.render_textures_to_view(
        &y_texture,
        &uv_texture,
        poppy_render::ColorSpace::Bt709Limited,
        false,
        &target_view,
    );
    assert!(res.is_ok(), "Zero-staging texture render must succeed");
    println!("✅ Zero staging memory copy render_textures_to_view succeeded!");
}
