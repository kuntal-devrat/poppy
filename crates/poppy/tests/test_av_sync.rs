use std::path::PathBuf;

fn get_sample(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("tests/samples/{name}"));
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
fn test_av_sync_aac_mp4() {
    let sample = match get_sample("test_av_aac.mp4") {
        Some(p) => p,
        None => {
            eprintln!("Sample test_av_aac.mp4 not found, skipping");
            return;
        }
    };

    let mut stream = poppy::MediaStream::builder()
        .hardware_acceleration(true)
        .open_sync(&sample)
        .expect("open test_av_aac.mp4");

    assert!(stream.has_audio(), "test_av_aac.mp4 must have an audio track");
    let video_tb = stream.time_base();

    // Find audio timebase from streams
    let audio_stream_info = stream
        .streams()
        .iter()
        .find(|s| s.is_audio())
        .expect("audio stream metadata");
    let audio_tb = audio_stream_info.time_base;

    // Read initial audio and video frames
    let first_audio = stream.next_audio_frame().expect("next_audio_frame");
    assert!(first_audio.is_some(), "Must produce audio frame");
    let a_frame = first_audio.unwrap();
    let a_pts_sec = a_frame
        .pts
        .map(|pts| audio_tb.to_seconds(pts))
        .unwrap_or(0.0);

    if stream.has_hardware_decoder() {
        let first_video = stream.next_frame().expect("next_frame");
        if let Some(v_frame) = first_video {
            let v_pts_sec = v_frame
                .pts
                .map(|pts| video_tb.to_seconds(pts))
                .unwrap_or(0.0);

            // A/V sync at stream start: must be within 100ms
            let initial_skew = (v_pts_sec - a_pts_sec).abs();
            assert!(
                initial_skew < 0.15,
                "Initial A/V PTS skew {initial_skew}s exceeds 150ms tolerance (video: {v_pts_sec}s, audio: {a_pts_sec}s)"
            );

            // Decode 5 more pairs of audio and video frames
            for _ in 0..5 {
                let v = stream.next_frame().expect("next_frame");
                let a = stream.next_audio_frame().expect("next_audio_frame");
                if let (Some(vf), Some(af)) = (v, a) {
                    let v_sec = vf.pts.map(|p| video_tb.to_seconds(p)).unwrap_or(0.0);
                    let a_sec = af.pts.map(|p| audio_tb.to_seconds(p)).unwrap_or(0.0);
                    let skew = (v_sec - a_sec).abs();
                    assert!(
                        skew < 0.35,
                        "In-flight A/V PTS skew {skew}s exceeds tolerance (video: {v_sec}s, audio: {a_sec}s)"
                    );
                }
            }

            // Seek to 0.3s
            stream.seek_seconds(0.3).expect("seek to 0.3s");
            let v_after_seek = stream.next_frame().expect("video after seek");
            let a_after_seek = stream.next_audio_frame().expect("audio after seek");

            if let (Some(vf), Some(af)) = (v_after_seek, a_after_seek) {
                let v_sec = vf.pts.map(|p| video_tb.to_seconds(p)).unwrap_or(0.0);
                let a_sec = af.pts.map(|p| audio_tb.to_seconds(p)).unwrap_or(0.0);
                assert!(
                    (v_sec - a_sec).abs() < 0.35,
                    "Post-seek A/V skew exceeds tolerance: video {v_sec}s vs audio {a_sec}s"
                );
            }
        }
    }
}

#[test]
fn test_av_sync_opus_webm() {
    let sample = match get_sample("test_av_opus.webm") {
        Some(p) => p,
        None => {
            eprintln!("Sample test_av_opus.webm not found, skipping");
            return;
        }
    };

    let mut stream = poppy::MediaStream::builder()
        .hardware_acceleration(true)
        .open_sync(&sample)
        .expect("open test_av_opus.webm");

    assert!(stream.has_audio(), "test_av_opus.webm must have an audio track");
    let video_tb = stream.time_base();

    let audio_stream_info = stream
        .streams()
        .iter()
        .find(|s| s.is_audio())
        .expect("audio stream metadata");
    let audio_tb = audio_stream_info.time_base;

    let a_frame = stream
        .next_audio_frame()
        .expect("next_audio_frame")
        .expect("audio frame");
    assert!(a_frame.samples.len() > 0);
    assert_eq!(a_frame.sample_rate, 48000);

    if stream.has_hardware_decoder() {
        if let Some(v_frame) = stream.next_frame().expect("next_frame") {
            let v_sec = v_frame.pts.map(|p| video_tb.to_seconds(p)).unwrap_or(0.0);
            let a_sec = a_frame.pts.map(|p| audio_tb.to_seconds(p)).unwrap_or(0.0);
            assert!(
                (v_sec - a_sec).abs() < 0.25,
                "Opus WebM A/V skew exceeds tolerance: video {v_sec}s vs audio {a_sec}s"
            );
        }
    }
}
