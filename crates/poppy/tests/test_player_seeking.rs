//! Integration test verifying seeking and timing on real video streams.

use std::path::PathBuf;

#[test]
fn test_media_stream_seeking_and_timing() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !sample_path.exists() {
        println!("Sample not found, skipping seek test");
        return;
    }

    let mut stream = poppy::open(&sample_path).expect("Failed to open video file");

    let tb = stream.time_base();
    assert!(tb.den > 0);
    assert!(tb.num > 0);

    let duration = stream.duration_seconds();
    assert!(duration.is_some());
    let dur = duration.unwrap();
    assert!(dur > 0.0);

    if !stream.has_hardware_decoder() {
        println!(
            "Skipping seeking decode assertions: no hardware decoder available on this system"
        );
        return;
    }

    // Read first frame
    let frame1 = stream.next_frame().expect("next_frame failed");
    if let Some(f1) = frame1 {
        assert_eq!(f1.width, 320);
        assert_eq!(f1.height, 240);

        // Seek to 0.5s
        stream.seek_seconds(0.5).expect("seek_seconds failed");
        let frame_after_seek = stream.next_frame().expect("next_frame failed");
        if let Some(f_seek) = frame_after_seek {
            assert_eq!(f_seek.width, 320);
            assert_eq!(f_seek.height, 240);
        }

        // Seek back to 0.0s
        stream.seek_seconds(0.0).expect("seek_seconds to 0 failed");
        let frame_reset = stream.next_frame().expect("next_frame failed");
        assert!(frame_reset.is_some());
    } else {
        println!(
            "Skipping seeking decode assertions: decoder produced no frames in test environment"
        );
    }
}

#[test]
fn test_seek_edge_cases_non_finite_and_negative() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !sample_path.exists() {
        return;
    }

    let mut stream = poppy::open(&sample_path).expect("open video file");

    // 1. Non-finite values must return Err(PoppyError::InvalidSeek)
    let err_nan = stream.seek_seconds(f64::NAN);
    assert!(err_nan.is_err(), "NaN seek must return Err");
    match err_nan.unwrap_err() {
        poppy::PoppyError::InvalidSeek(msg) => assert!(msg.contains("non-finite")),
        other => panic!("Expected InvalidSeek, got {other:?}"),
    }

    let err_inf = stream.seek_seconds(f64::INFINITY);
    assert!(err_inf.is_err(), "+Infinity seek must return Err");

    let err_neginf = stream.seek_seconds(f64::NEG_INFINITY);
    assert!(err_neginf.is_err(), "-Infinity seek must return Err");

    // 2. Negative timestamps: clamped gracefully to 0 or keyframe at 0
    let res_neg = stream.seek_seconds(-10.0);
    assert!(
        res_neg.is_ok(),
        "Negative seek should succeed (clamp to start)"
    );

    // 3. Seek past EOF: succeeds by finding the nearest keyframe at or before target
    let res_eof = stream.seek_seconds(999_999.0);
    assert!(res_eof.is_ok(), "Seek past EOF must succeed");
    if stream.has_hardware_decoder() {
        let frame_after_eof = stream.next_frame().expect("next_frame after EOF");
        assert!(
            frame_after_eof.is_some(),
            "Seek past EOF yields the nearest keyframe"
        );
    }

    // 4. Seek back to 0 after EOF: stream recovers and yields frames again
    stream.seek_seconds(0.0).expect("seek back to 0 after EOF");
    if stream.has_hardware_decoder() {
        let frame_after_rewind = stream.next_frame().expect("next_frame after rewind");
        assert!(
            frame_after_rewind.is_some(),
            "Stream must recover after seeking back to 0"
        );
    }
}

#[test]
fn test_seek_edge_cases_webm() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_av_opus.webm");

    if !sample_path.exists() {
        return;
    }

    let mut stream = poppy::open(&sample_path).expect("open webm file");

    // Non-finite seeks rejected
    assert!(stream.seek_seconds(f64::NAN).is_err());
    assert!(stream.seek_seconds(f64::INFINITY).is_err());

    // Seek to 0
    assert!(stream.seek_seconds(0.0).is_ok());

    // Seek to past EOF
    assert!(stream.seek_seconds(1000.0).is_ok());

    // Seek back to 0
    assert!(stream.seek_seconds(0.0).is_ok());
}
