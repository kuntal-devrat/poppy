#![cfg(target_os = "linux")]

use poppy_core::CodecId;
use poppy_demux::{Demuxer, MatroskaDemuxer, Mp4Demuxer};
use poppy_hw::{HwDecoder, VaapiVp9Decoder};

#[test]
fn test_vaapi_vp9_decoder_on_real_webm() {
    let candidates = [
        "tests/samples/test_vp9.webm",
        "../../tests/samples/test_vp9.webm",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_vp9.webm"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_vp9.webm");

    let mut demuxer = match MatroskaDemuxer::open(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping VP9 test (test_vp9.webm not found): {e}");
            return;
        }
    };

    let streams = demuxer.streams();
    let video_idx = streams
        .iter()
        .position(|s| s.codec == CodecId::Vp9)
        .expect("No VP9 track found in webm");

    let mut decoder = match VaapiVp9Decoder::new(false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping VP9 test (VA-API VP9 profile not available on system): {e}");
            return;
        }
    };

    let mut frames_decoded = 0;
    while let Ok(Some(pkt)) = demuxer.read_packet() {
        if pkt.stream_index != video_idx {
            continue;
        }

        match decoder.submit_packet(&pkt) {
            Ok(()) => {}
            Err(e) => {
                panic!("submit_packet failed at frame {}: {e}", frames_decoded + 1);
            }
        }

        while let Ok(Some(_frame)) = decoder.receive_frame() {
            frames_decoded += 1;
        }
    }

    println!("Total decoded VP9 frames from WebM: {}", frames_decoded);
    assert_eq!(
        frames_decoded, 30,
        "Expected all 30 VP9 frames to be decoded by VA-API hardware"
    );
}

#[test]
fn test_vaapi_vp9_decoder_on_real_mp4() {
    let candidates = [
        "tests/samples/test_vp9.mp4",
        "../../tests/samples/test_vp9.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_vp9.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_vp9.mp4");

    let mut demuxer = match Mp4Demuxer::open(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping VP9 test (test_vp9.mp4 not found): {e}");
            return;
        }
    };

    let streams = demuxer.streams();
    let video_idx = streams
        .iter()
        .position(|s| s.codec == CodecId::Vp9)
        .expect("No VP9 track found in mp4");

    let mut decoder = match VaapiVp9Decoder::new(false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping VP9 test (VA-API VP9 profile not available on system): {e}");
            return;
        }
    };

    if let poppy_core::codec::CodecConfig::Vp9(vpcc) = &streams[video_idx].config {
        let _ = decoder.set_extradata(&vpcc.raw);
    }

    let mut frames_decoded = 0;
    while let Ok(Some(pkt)) = demuxer.read_packet() {
        if pkt.stream_index != video_idx {
            continue;
        }

        match decoder.submit_packet(&pkt) {
            Ok(()) => {}
            Err(e) => {
                panic!("submit_packet failed at frame {}: {e}", frames_decoded + 1);
            }
        }

        while let Ok(Some(_frame)) = decoder.receive_frame() {
            frames_decoded += 1;
        }
    }

    println!("Total decoded VP9 frames from MP4: {}", frames_decoded);
    assert_eq!(
        frames_decoded, 30,
        "Expected all 30 VP9 frames to be decoded by VA-API hardware"
    );
}
