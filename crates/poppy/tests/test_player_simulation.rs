use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn test_player_simulation_seeking_left_right() {
    let video_path = std::env::var("POPPY_TEST_VIDEO")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            let candidates = [
                "target/test_seek_video.mp4",
                "../../target/test_seek_video.mp4",
            ];
            candidates.iter().map(PathBuf::from).find(|p| p.exists())
        });
    let video_path = match video_path {
        Some(p) => p,
        None => return,
    };

    let mut stream = poppy::open(&video_path).expect("open failed");
    let audio_cfg = match stream.audio_config() {
        Some(cfg) => cfg,
        None => return,
    };
    let sink = poppy_audio::AudioSink::try_new(audio_cfg.sample_rate, audio_cfg.channels)
        .expect("sink init");
    let renderer = poppy_render::GpuRenderer::new_sync().expect("renderer init");

    let tb = stream.time_base();
    let first_pts_sec = tb.to_seconds(stream.next_frame().unwrap().unwrap().pts.unwrap());
    let mut current_pts_sec = first_pts_sec;

    // Helper to get relative pos
    let get_rel = |pts_sec: f64| (pts_sec - first_pts_sec).max(0.0);

    // Initial playback
    println!("--- Initial playback simulation ---");

    let pump_audio = |stream: &mut poppy::MediaStream, sink: &poppy_audio::AudioSink| {
        let target_samples =
            (sink.device_sample_rate() as usize) * (sink.device_channels() as usize);
        let mut pumped = 0;
        while sink.queued_samples() < target_samples && pumped < 32 {
            match stream.next_audio_frame() {
                Ok(Some(af)) => {
                    sink.push_frame(&af);
                    pumped += 1;
                }
                _ => break,
            }
        }
        pumped
    };

    let current_master_time =
        |sink: &poppy_audio::AudioSink| -> f64 { sink.clock().current_pts_seconds() };

    for _ in 0..4 {
        pump_audio(&mut stream, &sink);
    }

    // Play 10 frames
    for _ in 0..10 {
        let f = stream.next_frame().unwrap().unwrap();
        current_pts_sec = tb.to_seconds(f.pts.unwrap());
        let _ = renderer.render_frame(&f);
        pump_audio(&mut stream, &sink);
    }

    // Now simulate seeking left and right
    let seeks = [5.0, 5.0, -5.0, -5.0, 5.0, -5.0, 5.0, -5.0];
    for (seek_i, &delta) in seeks.iter().enumerate() {
        let current_rel = get_rel(current_pts_sec);
        let target = (current_rel + delta).max(0.0);
        let stream_target = first_pts_sec + target;
        println!(
            "\n[Seek #{seek_i}] Seeking relative {delta:+.1}s from {current_rel:.2}s to target {target:.2}s"
        );

        let t0 = Instant::now();
        let frame = stream
            .seek_exact(stream_target)
            .unwrap()
            .expect("frame on seek");
        let seek_duration = t0.elapsed();
        let landed_pts = tb.to_seconds(frame.pts.unwrap());
        current_pts_sec = landed_pts;
        let actual_pos = get_rel(current_pts_sec);
        println!(
            "  -> seek_exact took {seek_duration:?}, landed at {actual_pos:.3}s (concealed: {})",
            frame.is_concealed
        );
        assert!(!frame.is_concealed, "Frame must not be concealed!");

        sink.reset_to(actual_pos);

        for _ in 0..4 {
            pump_audio(&mut stream, &sink);
        }

        // Render the landed seek frame
        let r0 = Instant::now();
        renderer.render_frame(&frame).expect("render seek frame");
        println!("  -> Render frame took {:?}", r0.elapsed());

        // Now simulate the playback loop for 15 frames after this seek
        println!("  -> Simulating 15 frames of playback after seek:");
        let mut frames_advanced = 0;
        let sim_start = Instant::now();
        while frames_advanced < 15 && sim_start.elapsed() < Duration::from_secs(3) {
            pump_audio(&mut stream, &sink);
            let target_pos = current_master_time(&sink);
            let cur_rel = get_rel(current_pts_sec);

            if cur_rel < target_pos {
                let d0 = Instant::now();
                match stream.next_frame().unwrap() {
                    Some(next_f) => {
                        let decode_took = d0.elapsed();
                        current_pts_sec = tb.to_seconds(next_f.pts.unwrap());
                        let cur_pos = get_rel(current_pts_sec);
                        assert!(
                            !next_f.is_concealed,
                            "Frame #{frames_advanced} after seek #{seek_i} was concealed (PTS {cur_pos:.3}s)"
                        );
                        renderer.render_frame(&next_f).expect("render next frame");
                        frames_advanced += 1;
                        if frames_advanced <= 3 || frames_advanced == 15 {
                            println!(
                                "    Frame #{frames_advanced:02}: cur_pos={cur_pos:.3}s target_pos={target_pos:.3}s (decode took {decode_took:?})"
                            );
                        }
                    }
                    None => {
                        println!("    End of stream");
                        break;
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(
            frames_advanced, 15,
            "Must decode 15 frames without stalling"
        );
    }
}
