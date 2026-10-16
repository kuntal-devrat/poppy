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

#[test]
fn test_seek_on_long_video() {
    let video_path = std::env::var("POPPY_TEST_VIDEO")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.exists());
    let video_path = match video_path {
        Some(p) => p,
        None => return,
    };

    let mut stream = poppy::open(&video_path).expect("open failed");
    let tb = stream.time_base();

    // 1. Initial frames: monotonically increasing
    let mut prev_sec = -1.0;
    for _ in 0..5 {
        let f = stream.next_frame().unwrap().unwrap();
        let sec = tb.to_seconds(f.pts.unwrap());
        assert!(
            sec > prev_sec,
            "Initial frames must have ascending PTS: {sec} > {prev_sec}"
        );
        prev_sec = sec;
    }

    // 2. Seeking forward to 11.0s (GOP is 6s)
    let f11 = stream
        .seek_exact(11.0)
        .unwrap()
        .expect("seek_exact(11.0) must produce frame");
    let sec11 = tb.to_seconds(f11.pts.unwrap());
    assert!(
        (10.95..=11.1).contains(&sec11),
        "Expected ~11.0s, got {sec11}s"
    );

    let mut prev_sec = sec11;
    for _ in 0..5 {
        let f = stream.next_frame().unwrap().unwrap();
        let sec = tb.to_seconds(f.pts.unwrap());
        assert!(
            sec > prev_sec,
            "Post-seek 11s frames must have strictly ascending PTS: {sec} > {prev_sec}"
        );
        assert!(!f.is_concealed, "Frame must not be concealed/corrupted");
        prev_sec = sec;
    }

    // 3. Seeking backward to 8.2s (in middle of 6s-12s GOP)
    let f82 = stream
        .seek_exact(8.2)
        .unwrap()
        .expect("seek_exact(8.2) must produce frame");
    let sec82 = tb.to_seconds(f82.pts.unwrap());
    assert!(
        (8.18..=8.25).contains(&sec82),
        "Expected ~8.2s, got {sec82}s"
    );

    let mut prev_sec = sec82;
    for _ in 0..10 {
        let f = stream.next_frame().unwrap().unwrap();
        let sec = tb.to_seconds(f.pts.unwrap());
        assert!(
            sec > prev_sec,
            "Post-seek 8.2s frames must have strictly ascending PTS (no jump back to 11s): {sec} > {prev_sec}"
        );
        assert!(
            sec < 9.0,
            "Frames must be around 8.2s-8.6s, not jumped to 11s+: {sec}"
        );
        assert!(!f.is_concealed, "Frame must not be concealed/corrupted");
        prev_sec = sec;
    }

    // 4. Verify audio synchronization
    if let (Some(af), Some(audio_tb)) =
        (stream.next_audio_frame().unwrap(), stream.audio_time_base())
    {
        let af_sec = af.pts.map(|p| audio_tb.to_seconds(p)).unwrap_or(0.0);
        println!("Audio frame PTS right after 8.2s seek: {af_sec:.3}s");
        assert!(
            (8.1..=8.8).contains(&af_sec),
            "Audio must be near seek target, got {af_sec}s"
        );
    }

    // 5. Seeking forward to 25.0s (crosses multiple GOP boundaries)
    let f25 = stream
        .seek_exact(25.0)
        .unwrap()
        .expect("seek_exact(25.0) must produce frame");
    let sec25 = tb.to_seconds(f25.pts.unwrap());
    assert!(
        (24.95..=25.1).contains(&sec25),
        "Expected ~25.0s, got {sec25}s"
    );

    let mut prev_sec = sec25;
    for _ in 0..5 {
        let f = stream.next_frame().unwrap().unwrap();
        let sec = tb.to_seconds(f.pts.unwrap());
        assert!(
            sec > prev_sec,
            "Post-seek 25s frames must have strictly ascending PTS: {sec} > {prev_sec}"
        );
        assert!(!f.is_concealed);
        prev_sec = sec;
    }

    // 6. Seeking backward to 1.5s
    let f15 = stream
        .seek_exact(1.5)
        .unwrap()
        .expect("seek_exact(1.5) must produce frame");
    let sec15 = tb.to_seconds(f15.pts.unwrap());
    assert!(
        (1.45..=1.55).contains(&sec15),
        "Expected ~1.5s, got {sec15}s"
    );

    let mut prev_sec = sec15;
    for _ in 0..5 {
        let f = stream.next_frame().unwrap().unwrap();
        let sec = tb.to_seconds(f.pts.unwrap());
        assert!(
            sec > prev_sec,
            "Post-seek 1.5s frames must have strictly ascending PTS: {sec} > {prev_sec}"
        );
        assert!(!f.is_concealed);
        prev_sec = sec;
    }
}

#[test]
fn test_rapid_interactive_seeking_simulation() {
    let video_path = std::env::var("POPPY_TEST_VIDEO")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.exists())
        .unwrap_or_else(ensure_test_video);

    if !video_path.exists() {
        return;
    }

    let mut stream = poppy::open(&video_path).expect("open failed");
    let tb = stream.time_base();

    // Initial decode
    let mut current_pos = 0.0;
    for _ in 0..15 {
        let f = stream.next_frame().unwrap().unwrap();
        current_pos = tb.to_seconds(f.pts.unwrap());
    }

    // Sequence of interactive seeks (ArrowRight / ArrowLeft presses)
    let deltas = [5.0, 5.0, -5.0, -5.0, 5.0, 5.0, 5.0, -5.0, -5.0, -5.0];
    for (i, &delta) in deltas.iter().enumerate() {
        let target = (current_pos + delta).max(0.0);
        println!(
            "[Seek #{i}] Seeking relative {delta:+.1}s from {current_pos:.2}s to target {target:.2}s"
        );
        let seek_frame = stream
            .seek_exact(target)
            .unwrap()
            .expect("must produce frame");
        let landed_sec = tb.to_seconds(seek_frame.pts.unwrap());
        println!(
            "  -> Landed at {landed_sec:.3}s (concealed: {})",
            seek_frame.is_concealed
        );
        assert!(
            !seek_frame.is_concealed,
            "Seek frame must not be corrupted!"
        );

        // Decode subsequent frames and verify strictly ascending timestamps
        let mut prev = landed_sec;
        for frame_idx in 0..10 {
            let f = match stream.next_frame().unwrap() {
                Some(frame) => frame,
                None => break,
            };
            let s = tb.to_seconds(f.pts.unwrap());
            assert!(
                s > prev,
                "Frame #{frame_idx} PTS {s:.3}s must be > prev {prev:.3}s"
            );
            assert!(!f.is_concealed, "Frame #{frame_idx} must not be concealed!");
            prev = s;
        }
        current_pos = prev;

        // Verify audio frame can also be decoded without stall
        if let Some(af) = stream.next_audio_frame().unwrap()
            && let Some(atb) = stream.audio_time_base()
        {
            let a_sec = af.pts.map(|p| atb.to_seconds(p)).unwrap_or(0.0);
            println!("  -> Audio frame at {a_sec:.3}s");
        }
    }
}
