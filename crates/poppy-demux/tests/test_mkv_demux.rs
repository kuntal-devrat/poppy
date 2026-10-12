use poppy_core::CodecId;
use poppy_demux::{Demuxer, MatroskaDemuxer};
use std::path::Path;

fn find_sample(name: &str) -> Option<String> {
    let candidates = [
        format!("tests/samples/{name}"),
        format!("../../tests/samples/{name}"),
        format!("{}/../../tests/samples/{name}", env!("CARGO_MANIFEST_DIR")),
    ];
    for c in &candidates {
        if Path::new(c).exists() {
            return Some(c.clone());
        }
    }
    None
}

#[test]
fn test_mkv_demux_real_av_opus_webm() {
    let path = match find_sample("test_av_opus.webm") {
        Some(p) => p,
        None => {
            eprintln!("Sample test_av_opus.webm not found, skipping");
            return;
        }
    };

    let mut demuxer = MatroskaDemuxer::open(&path).expect("open test_av_opus.webm");
    let streams = demuxer.streams().to_vec();
    assert!(streams.len() >= 2, "Expected at least video and audio streams");

    let video_stream = streams.iter().find(|s| s.is_video()).expect("video stream").clone();
    let audio_stream = streams.iter().find(|s| s.is_audio()).expect("audio stream").clone();

    assert_eq!(video_stream.codec, CodecId::Vp9);
    assert_eq!(audio_stream.codec, CodecId::Opus);
    assert!(video_stream.time_base.den > 0);
    assert!(audio_stream.time_base.den > 0);

    // Read all packets and verify interleaved audio and video packets
    let mut video_packet_count = 0;
    let mut audio_packet_count = 0;
    let mut last_video_dts: Option<i64> = None;

    while let Some(pkt) = demuxer.read_packet().expect("read_packet") {
        assert!(!pkt.data.is_empty(), "Packet data must not be empty");
        if pkt.stream_index == video_stream.index {
            if let Some(prev) = last_video_dts {
                assert!(pkt.dts >= prev, "Video DTS must be monotonically non-decreasing");
            }
            last_video_dts = Some(pkt.dts);
            video_packet_count += 1;
        } else if pkt.stream_index == audio_stream.index {
            audio_packet_count += 1;
        }
    }

    assert!(video_packet_count > 0, "Must demux at least one video packet");
    assert!(audio_packet_count > 0, "Must demux at least one audio packet");

    // Test seeking back to timestamp 0
    demuxer
        .seek_to_keyframe(video_stream.index, 0)
        .expect("seek to keyframe 0");
    let first_after_seek = demuxer
        .read_packet()
        .expect("read after seek")
        .expect("packet after seek");
    assert!(first_after_seek.is_keyframe || first_after_seek.stream_index == audio_stream.index);

    // Test seek to end/past EOF
    demuxer
        .seek_to_keyframe(video_stream.index, 1_000_000_000)
        .expect("seek past EOF must succeed gracefully");
}

#[test]
fn test_mkv_streaming_demuxer() {
    let path = match find_sample("test_av_opus.webm") {
        Some(p) => p,
        None => return,
    };
    let file = std::fs::File::open(&path).expect("open file");
    let mut demuxer = MatroskaDemuxer::open_streaming(file).expect("open_streaming on webm");
    assert!(demuxer.streams().len() >= 2);
    let pkt = demuxer.read_packet().expect("read_packet").expect("first packet");
    assert!(!pkt.data.is_empty());
}

#[test]
fn test_mkv_demux_real_vp9_webm() {
    let path = match find_sample("test_vp9.webm") {
        Some(p) => p,
        None => return,
    };

    let mut demuxer = MatroskaDemuxer::open(&path).expect("open test_vp9.webm");
    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec, CodecId::Vp9);

    let mut count = 0;
    while let Some(pkt) = demuxer.read_packet().expect("read_packet") {
        assert_eq!(pkt.stream_index, 0);
        assert!(!pkt.data.is_empty());
        count += 1;
    }
    assert!(count > 0);
}

#[test]
fn test_mkv_corrupt_data_resilience() {
    let path = match find_sample("test_av_opus.webm") {
        Some(p) => p,
        None => return,
    };
    let data = std::fs::read(&path).expect("read sample");
    // Truncate to just the EBML header (around 50 bytes)
    let truncated = &data[..50.min(data.len())];
    let cursor = std::io::Cursor::new(truncated.to_vec());
    let res = MatroskaDemuxer::open_streaming(cursor);
    // Should fail cleanly with error, never panic
    assert!(res.is_err());
}
