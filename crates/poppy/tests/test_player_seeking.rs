//! Integration test verifying seeking and timing on real video streams.

use std::path::PathBuf;

#[test]
fn test_media_stream_seeking_and_timing() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_av1_1080p.mp4");

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

    // Read first frame
    let frame1 = stream.next_frame().expect("next_frame failed");
    assert!(frame1.is_some());
    let f1 = frame1.unwrap();
    assert_eq!(f1.width, 1920);
    assert_eq!(f1.height, 1080);

    // Seek to 2.0s
    stream.seek_seconds(2.0).expect("seek_seconds failed");
    let frame_after_seek = stream.next_frame().expect("next_frame failed");
    assert!(frame_after_seek.is_some());
    let f_seek = frame_after_seek.unwrap();
    assert_eq!(f_seek.width, 1920);
    assert_eq!(f_seek.height, 1080);

    // Seek back to 0.0s
    stream.seek_seconds(0.0).expect("seek_seconds to 0 failed");
    let frame_reset = stream.next_frame().expect("next_frame failed");
    assert!(frame_reset.is_some());
}
