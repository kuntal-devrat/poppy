use bytes::Bytes;
use poppy_core::{CodecId, Packet, PixelFormat};
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::HwDecoder;
use poppy_hw::videotoolbox::VideoToolboxDecoder;
use std::path::Path;

#[test]
fn test_videotoolbox_initialization_and_extradata() {
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            VideoToolboxDecoder::new(CodecId::H264).is_err(),
            "VideoToolbox decoder must fail off-macOS"
        );
        return;
    }
    #[allow(unreachable_code)]
    let mut decoder = match VideoToolboxDecoder::new(CodecId::H264) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping test: VideoToolbox unavailable ({e})");
            return;
        }
    };
    assert_eq!(decoder.codec, CodecId::H264);

    // Mock avcC extradata
    let mock_avcc = [
        0x01, // configurationVersion
        0x64, // AVCProfileIndication (High)
        0x00, // profile_compatibility
        0x1F, // AVCLevelIndication
        0xFF, // lengthSizeMinusOne (4 bytes)
        0xE1, // numOfSequenceParameterSets (1)
        0x00, 0x1A, // sequenceParameterSetLength (26 bytes)
        // SPS NAL: 67 64 00 1f ac d9 40 50 05 bb 01 10 00 00 03 00 10 00 00 03 03 20 f1 83 19 60
        0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50, 0x05, 0xBB, 0x01, 0x10, 0x00, 0x00, 0x03,
        0x00, 0x10, 0x00, 0x00, 0x03, 0x03, 0x20, 0xF1, 0x83, 0x19, 0x60,
        0x01, // numOfPictureParameterSets (1)
        0x00, 0x06, // pictureParameterSetLength (6 bytes)
        0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0, // PPS NAL
    ];

    let res = decoder.set_extradata(&mock_avcc);
    assert!(res.is_ok(), "set_extradata should succeed");
    assert!(decoder.width > 0);
    assert!(decoder.height > 0);
}

#[test]
fn test_videotoolbox_decode_multislice_and_cvpixelbuffer_export() {
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            VideoToolboxDecoder::new(CodecId::H264).is_err(),
            "VideoToolbox decoder must fail off-macOS"
        );
        return;
    }
    #[allow(unreachable_code)]
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
        println!("Sample not found, skipping multi-slice VideoToolbox test");
        return;
    }

    let mut demuxer = Mp4Demuxer::open(path).expect("failed to open multislice test video");
    let (video_track_idx, video_config) = {
        let stream = demuxer
            .streams()
            .iter()
            .find(|s| s.codec == CodecId::H264)
            .expect("video track not found");
        (stream.index, stream.config.clone())
    };

    let mut decoder = VideoToolboxDecoder::new(CodecId::H264).expect("create VideoToolbox decoder");

    // Ingest parameter sets
    if let poppy_core::codec::CodecConfig::H264(ref avcc) = video_config {
        for sps in &avcc.sps_list {
            let mut data = vec![0, 0, 0, 1];
            data.extend_from_slice(sps);
            let pkt = Packet {
                stream_index: video_track_idx,
                pts: None,
                dts: 0,
                duration: 0,
                data: Bytes::from(data),
                is_keyframe: false,
            };
            decoder.submit_packet(&pkt).expect("submit SPS");
        }
        for pps in &avcc.pps_list {
            let mut data = vec![0, 0, 0, 1];
            data.extend_from_slice(pps);
            let pkt = Packet {
                stream_index: video_track_idx,
                pts: None,
                dts: 0,
                duration: 0,
                data: Bytes::from(data),
                is_keyframe: false,
            };
            decoder.submit_packet(&pkt).expect("submit PPS");
        }
    }

    let mut decoded_frames = 0;
    while let Some(packet) = demuxer.read_packet().expect("read packet") {
        if packet.stream_index != video_track_idx {
            continue;
        }

        // Convert length-prefixed packet into Annex-B for decoding
        let mut annex_b = Vec::new();
        if let poppy_core::codec::CodecConfig::H264(ref avcc) = video_config {
            let _ = poppy_demux::bsf::avcc_to_annex_b(
                &packet.data,
                avcc.nal_length_size,
                None,
                &mut annex_b,
            );
        }

        let pkt = Packet {
            stream_index: packet.stream_index,
            pts: packet.pts,
            dts: packet.dts,
            duration: packet.duration,
            data: Bytes::from(annex_b),
            is_keyframe: packet.is_keyframe,
        };

        decoder
            .submit_packet(&pkt)
            .expect("submit multislice packet");

        while let Some(frame) = decoder.receive_frame().expect("receive frame") {
            assert!(frame.width > 0);
            assert!(frame.height > 0);
            assert_eq!(frame.pixel_format, PixelFormat::Nv12);
            assert!(
                frame.cv_pixel_buffer.is_some(),
                "VideoToolbox frame must export CVPixelBuffer descriptor"
            );
            let cv = frame.cv_pixel_buffer.as_ref().unwrap();
            assert!(cv.pixel_buffer_ptr != 0);
            assert!(cv.io_surface_ptr != 0);
            assert_eq!(cv.width, frame.width);
            assert_eq!(cv.height, frame.height);
            decoded_frames += 1;
        }
    }

    decoder.flush().expect("flush decoder");
    while let Some(frame) = decoder.receive_frame().expect("receive frame") {
        assert!(frame.cv_pixel_buffer.is_some());
        decoded_frames += 1;
    }

    assert!(
        decoded_frames > 0,
        "VideoToolbox decoder should have decoded frames from multislice video"
    );
    println!("✅ VideoToolbox decoded {decoded_frames} frames with zero-copy CVPixelBuffer export");
}

#[test]
fn test_videotoolbox_resilience_corrupted_packets() {
    #[cfg(not(target_os = "macos"))]
    {
        assert!(
            VideoToolboxDecoder::new(CodecId::H264).is_err(),
            "VideoToolbox decoder must fail off-macOS"
        );
        return;
    }
    #[allow(unreachable_code)]
    let mut decoder = match VideoToolboxDecoder::new(CodecId::H264) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping test: VideoToolbox unavailable ({e})");
            return;
        }
    };

    let malformed = [
        vec![0x11, 0x22, 0x33, 0x44],
        vec![0x00, 0x00, 0x01, 0x1F],
        vec![0x00, 0x00, 0x00, 0x01, 0x67],
        vec![0x00, 0x00, 0x00, 0x01, 0x41, 0x88],
    ];

    for junk in malformed {
        let pkt = Packet {
            stream_index: 0,
            pts: Some(0),
            dts: 0,
            duration: 100,
            data: Bytes::from(junk),
            is_keyframe: false,
        };
        let res = decoder.submit_packet(&pkt);
        assert!(
            res.is_ok(),
            "VideoToolbox decoder must gracefully handle corrupt packets"
        );
    }

    println!("✅ VideoToolbox decoder survived corrupt bitstreams gracefully");
}
