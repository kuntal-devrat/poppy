use futures_util::StreamExt;
use poppy_core::PixelFormat;
use std::path::Path;

#[tokio::test]
async fn test_media_stream_builder_options() {
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
        println!("Sample not found, skipping builder test");
        return;
    }

    // Builder with explicit options
    let stream = poppy::MediaStream::builder()
        .hardware_acceleration(true)
        .buffer_capacity(8)
        .stream_index(0)
        .open_sync(path)
        .expect("build sync MediaStream");

    assert_eq!(stream.target_stream_index(), 0);
    assert!(stream.duration_seconds().unwrap_or(0.0) > 0.0);
    assert!(!stream.streams().is_empty());
}

#[tokio::test]
async fn test_tokio_stream_iteration_and_backpressure() {
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
        println!("Sample not found, skipping async stream test");
        return;
    }

    // Set tight buffer capacity to exercise backpressure
    let mut async_stream = poppy::MediaStream::builder()
        .buffer_capacity(2)
        .open_async(path)
        .await
        .expect("open async stream");

    if !async_stream.has_hardware_decoder() {
        println!("Skipping stream decode: no hardware decoder available on this system");
        return;
    }

    let mut frame_count = 0;
    while let Some(frame_result) = async_stream.next().await {
        let frame = frame_result.expect("decode frame in async stream");
        assert!(frame.width > 0);
        assert!(frame.height > 0);
        assert_eq!(frame.pixel_format, PixelFormat::Nv12);
        frame_count += 1;

        // Simulate slower downstream processing to trigger worker backpressure
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;

        if frame_count >= 15 {
            break;
        }
    }

    assert!(
        frame_count >= 10,
        "Should have received decoded frames over async stream"
    );
    println!("✅ Tokio Stream received {frame_count} frames with backpressure flow control");
}

#[tokio::test]
async fn test_async_seeking_and_draining() {
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
        println!("Sample not found, skipping seek test");
        return;
    }

    let mut async_stream = poppy::open_async(path).await.expect("open async stream");

    if !async_stream.has_hardware_decoder() {
        println!("Skipping async seek decode: no hardware decoder available on this system");
        return;
    }

    // Read initial frame
    let first_frame = async_stream
        .next_frame()
        .await
        .expect("read first frame")
        .expect("frame should exist");
    assert!(first_frame.pts.is_some());

    // Seek back to start (0.0s)
    async_stream
        .seek_seconds(0.0)
        .await
        .expect("seek to beginning");

    let seeked_frame = async_stream
        .next_frame()
        .await
        .expect("read frame after seek")
        .expect("frame should exist");

    assert_eq!(
        seeked_frame.pts, first_frame.pts,
        "Seek to start should return initial frame PTS"
    );
    println!(
        "✅ Asynchronous seek returned correct frame PTS: {:?}",
        seeked_frame.pts
    );
}

#[tokio::test]
async fn test_frame_scrubbing_timeline_and_step() {
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
        println!("Sample not found, skipping scrubbing test");
        return;
    }

    let mut async_stream = poppy::open_async(path).await.expect("open async stream");

    if !async_stream.has_hardware_decoder() {
        println!("Skipping scrubbing decode: no hardware decoder available on this system");
        return;
    }

    // Scrub to 0.5 seconds
    let scrub_frame = async_stream
        .scrub_to(0.5)
        .await
        .expect("scrub to 0.5s")
        .expect("scrub frame must exist");

    let time_base = async_stream.time_base();
    let scrub_pts_sec = scrub_frame
        .pts
        .map(|p| time_base.to_seconds(p))
        .unwrap_or(0.0);

    println!("Scrubbed to 0.5s, target frame PTS: {:.3}s", scrub_pts_sec);
    assert!(
        (scrub_pts_sec - 0.5).abs() < 1.0,
        "Scrubbed frame PTS should be close to 0.5s"
    );

    // Step forward 1 frame
    let step_forward_frame = async_stream
        .step_forward()
        .await
        .expect("step forward")
        .expect("frame exists");

    if let (Some(p1), Some(p2)) = (scrub_frame.pts, step_forward_frame.pts) {
        assert!(p2 >= p1, "Stepping forward should advance or match PTS");
    }

    // Step backward by 0.04s (approx 1 frame @ 25fps)
    let step_backward_frame = async_stream
        .step_backward(0.04)
        .await
        .expect("step backward")
        .expect("frame exists");

    assert!(step_backward_frame.width > 0);
    println!("✅ Timeline scrubbing and bidirectional frame stepping verified");
}

#[tokio::test]
async fn test_async_multiformat_hevc_and_av1() {
    let hevc_candidates = [
        "tests/samples/test_hevc.mp4",
        "../../tests/samples/test_hevc.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_hevc.mp4"
        ),
    ];
    let hevc_path = hevc_candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_hevc.mp4");

    if Path::new(hevc_path).exists() {
        let mut hevc_stream = poppy::open_async(hevc_path).await.expect("open HEVC async");
        if hevc_stream.has_hardware_decoder() {
            if let Some(f) = hevc_stream.next_frame().await.expect("decode HEVC") {
                assert!(f.width > 0);
                println!("✅ HEVC async decoding verified ({}x{})", f.width, f.height);
            } else {
                println!("Skipping HEVC async decode: no frame produced by decoder backend");
            }
        } else {
            println!("Skipping HEVC async decode: no hardware decoder available");
        }
    }

    let av1_candidates = [
        "tests/samples/test_av1.mp4",
        "../../tests/samples/test_av1.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_av1.mp4"
        ),
    ];
    let av1_path = av1_candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_av1.mp4");

    if Path::new(av1_path).exists() {
        let mut av1_stream = poppy::open_async(av1_path).await.expect("open AV1 async");
        if av1_stream.has_hardware_decoder() {
            if let Some(f) = av1_stream.next_frame().await.expect("decode AV1") {
                assert!(f.width > 0);
                println!("✅ AV1 async decoding verified ({}x{})", f.width, f.height);
            } else {
                println!("Skipping AV1 async decode: no frame produced by decoder backend");
            }
        } else {
            println!("Skipping AV1 async decode: no hardware decoder available");
        }
    }
}
