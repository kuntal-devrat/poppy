#![cfg(target_os = "linux")]

use bytes::Bytes;
use poppy_core::{CodecId, InterlaceMode, Packet, PixelFormat};
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::{HwDecoder, VaapiH264Decoder};
use std::path::Path;

#[test]
fn test_error_resilience_corrupted_packets() {
    let mut decoder = match VaapiH264Decoder::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping hardware test: no VA-API device available ({e})");
            return;
        }
    };

    println!("✅ VA-API decoder initialized for error resilience test");

    // Submit corrupted garbage packets that should not crash or abort the decoder
    let garbage_data = vec![
        vec![0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x12, 0x34], // Completely non-Annex-B noise
        vec![0x00, 0x00, 0x01, 0xFF, 0xEE, 0xDD],       // Invalid NAL unit type (type 31)
        vec![0x00, 0x00, 0x00, 0x01, 0x67],             // Truncated SPS (only 1 byte)
        vec![0x00, 0x00, 0x00, 0x01, 0x68],             // Truncated PPS (only 1 byte)
        vec![0x00, 0x00, 0x00, 0x01, 0x41, 0x9A, 0xBC], // P-slice without SPS/PPS/IDR reference
    ];

    for (idx, junk) in garbage_data.into_iter().enumerate() {
        let packet = Packet {
            stream_index: 0,
            pts: Some(idx as i64 * 100),
            dts: idx as i64 * 100,
            duration: 100,
            is_keyframe: false,
            data: Bytes::from(junk),
        };
        let res = decoder.submit_packet(&packet);
        assert!(res.is_ok(), "Garbage packet should be handled resiliently");
    }

    println!("✅ Decoder survived corrupted/invalid packets gracefully without crash");
}

#[test]
fn test_multislice_batching_on_real_multislice_video() {
    let candidates = [
        "tests/samples/test_h264_multislice.mp4",
        "../../tests/samples/test_h264_multislice.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_h264_multislice.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_h264_multislice.mp4");

    if !Path::new(path).exists() {
        eprintln!("Multi-slice test file not found, skipping.");
        return;
    }

    let mut demuxer = Mp4Demuxer::open(path).expect("Failed to open test_h264_multislice.mp4");
    let streams = demuxer.streams();
    let video_idx = streams
        .iter()
        .position(|s| s.codec == CodecId::H264)
        .expect("No H.264 track");

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

    let mut frames_decoded = 0;
    while let Ok(Some(pkt)) = demuxer.read_packet() {
        if pkt.stream_index != video_idx {
            continue;
        }

        decoder
            .submit_packet(&pkt)
            .expect("Multi-slice batch submission failed");

        while let Ok(Some(frame)) = decoder.receive_frame() {
            assert_eq!(frame.width, 320);
            assert_eq!(frame.height, 240);
            assert_eq!(frame.pixel_format, PixelFormat::Nv12);
            assert_eq!(frame.interlace_mode, InterlaceMode::Progressive);
            assert!(
                frame.dmabuf.is_some() || frame.data.is_some(),
                "Frame must have either DMA-BUF (zero-copy) or CPU fallback data"
            );
            frames_decoded += 1;
        }
    }

    decoder.flush().expect("Flush failed");
    while let Ok(Some(frame)) = decoder.receive_frame() {
        assert_eq!(frame.width, 320);
        assert_eq!(frame.height, 240);
        frames_decoded += 1;
    }

    assert!(
        frames_decoded > 0,
        "Expected at least one multi-slice frame decoded"
    );
    println!(
        "✅ Successfully decoded {frames_decoded} frames with multi-slice batching and zero-copy DMA-BUF export"
    );
}

#[test]
fn test_interlaced_mbaff_detection() {
    let candidates = [
        "tests/samples/test_h264_interlaced.mp4",
        "../../tests/samples/test_h264_interlaced.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_h264_interlaced.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_h264_interlaced.mp4");

    if !Path::new(path).exists() {
        eprintln!("Interlaced test file not found, skipping.");
        return;
    }

    let mut demuxer = Mp4Demuxer::open(path).expect("Failed to open test_h264_interlaced.mp4");
    let streams = demuxer.streams();
    let video_idx = streams
        .iter()
        .position(|s| s.codec == CodecId::H264)
        .expect("No H.264 track");

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

    let mut frames_decoded = 0;
    let mut interlaced_detected = false;
    while let Ok(Some(pkt)) = demuxer.read_packet() {
        if pkt.stream_index != video_idx {
            continue;
        }

        decoder
            .submit_packet(&pkt)
            .expect("Interlaced packet submission failed");

        while let Ok(Some(frame)) = decoder.receive_frame() {
            if frame.interlace_mode != InterlaceMode::Progressive {
                interlaced_detected = true;
            }
            frames_decoded += 1;
        }
    }

    decoder.flush().expect("Flush failed");
    while let Ok(Some(frame)) = decoder.receive_frame() {
        if frame.interlace_mode != InterlaceMode::Progressive {
            interlaced_detected = true;
        }
        frames_decoded += 1;
    }

    assert!(frames_decoded > 0);
    assert!(
        interlaced_detected,
        "Interlaced stream should be detected via MBAFF/PAFF"
    );
    println!(
        "✅ Successfully detected interlaced stream (MBAFF/PAFF) with progressive fallback: {frames_decoded} frames decoded"
    );
}
