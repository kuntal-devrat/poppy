#![cfg(target_os = "linux")]

use poppy_core::CodecId;
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::{HwDecoder, VaapiAv1Decoder};

#[test]
fn test_vaapi_av1_decoder_on_real_video() {
    let candidates = [
        "tests/samples/test_av1.mp4",
        "../../tests/samples/test_av1.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_av1.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_av1.mp4");

    let mut demuxer = match Mp4Demuxer::open(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping AV1 test (test_av1.mp4 not found): {e}");
            return;
        }
    };

    let streams = demuxer.streams();
    let video_idx = streams
        .iter()
        .position(|s| s.codec == CodecId::Av1)
        .expect("No AV1 track");

    let mut decoder = match VaapiAv1Decoder::new(false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping AV1 test (VA-API AV1 profile not available on system): {e}");
            return;
        }
    };

    if let poppy_core::codec::CodecConfig::Av1(av1c) = &streams[video_idx].config {
        let _ = decoder.set_extradata(&av1c.config_obus);
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

    println!("Total decoded AV1 frames: {}", frames_decoded);
    assert_eq!(
        frames_decoded, 30,
        "Expected all 30 AV1 frames to be decoded by VA-API hardware"
    );
}
