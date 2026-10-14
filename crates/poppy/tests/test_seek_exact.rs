use std::path::PathBuf;

fn ensure_test_video() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/test_seek_video.mp4");
    if !p.exists() {
        let _ = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=10:size=320x240:rate=30",
                "-c:v",
                "libx264",
                "-g",
                "60",
                "-keyint_min",
                "60",
                "-sc_threshold",
                "0",
                "-pix_fmt",
                "yuv420p",
                p.to_str().unwrap(),
            ])
            .output();
    }
    p
}

#[test]
fn test_seek_exact_advances_forward_multi_gop() {
    let video_path = ensure_test_video();

    if !video_path.exists() {
        return;
    }

    let mut stream = poppy::open(&video_path).expect("open failed");
    let tb = stream.time_base();

    let f0 = stream.next_frame().unwrap().unwrap();
    let sec0 = tb.to_seconds(f0.pts.unwrap());
    assert!((sec0 - 0.0).abs() < 1e-4);

    // Decode to ~1.0s
    for _ in 1..30 {
        let _ = stream.next_frame().unwrap().unwrap();
    }

    // Seek exact forward to 5.0s (in video with keyframes at 0, 2, 4, 6, 8s)
    let f_seek = stream.seek_exact(5.0).expect("seek_exact failed");
    assert!(f_seek.is_some(), "seek_exact must return a frame");
    let f5 = f_seek.unwrap();
    let sec5 = tb.to_seconds(f5.pts.unwrap());
    println!("Landed at sec5: {sec5} (target: 5.0)");
    assert!(
        (4.99..=5.1).contains(&sec5),
        "Landed at {sec5}s, expected ~5.0s"
    );

    // Seek forward again to 7.5s
    let f_seek2 = stream.seek_exact(7.5).expect("seek_exact(7.5) failed");
    assert!(f_seek2.is_some());
    let sec75 = tb.to_seconds(f_seek2.unwrap().pts.unwrap());
    println!("Landed at sec75: {sec75} (target: 7.5)");
    assert!(
        (7.49..=7.6).contains(&sec75),
        "Landed at {sec75}s, expected ~7.5s"
    );

    // Seek backward to 1.5s
    let f_seek_back = stream.seek_exact(1.5).expect("seek_exact(1.5) failed");
    assert!(f_seek_back.is_some());
    let sec15 = tb.to_seconds(f_seek_back.unwrap().pts.unwrap());
    println!("Landed at sec15: {sec15} (target: 1.5)");
    assert!(
        (1.49..=1.6).contains(&sec15),
        "Landed at {sec15}s, expected ~1.5s"
    );
}

#[test]
fn test_seek_exact_on_single_keyframe_video() {
    // test_h264.mp4 only has 1 keyframe at 0.0s (30 frames total, 1 second duration)
    let video_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !video_path.exists() {
        return;
    }

    let mut stream = poppy::open(&video_path).expect("open failed");
    let tb = stream.time_base();

    // In a video with only ONE keyframe at 0.0s, seek_exact(0.5) must decode forward to ~0.5s!
    let f_seek = stream.seek_exact(0.5).expect("seek_exact(0.5) failed");
    if stream.has_hardware_decoder() {
        assert!(f_seek.is_some());
        let sec = tb.to_seconds(f_seek.unwrap().pts.unwrap());
        println!("Single-keyframe video landed at: {sec}s (target: 0.5s)");
        assert!(
            (0.45..=0.55).contains(&sec),
            "Landed at {sec}s, expected ~0.5s"
        );
    }
}
