#![allow(unused_imports)]

use bytes::Bytes;
use poppy_core::{CodecId, Packet, PixelFormat};
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::HwDecoder;
use poppy_hw::d3d12::{D3D12SurfacePool, D3D12VideoDecoder, D3D12VideoDevice};
use std::path::Path;

#[test]
fn test_d3d12_device_and_pool_initialization() {
    // Verify device opening returns NoDeviceFound when D3D12 hardware is unavailable.
    // Pool allocation (pure metadata) still works for unit testing.
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            D3D12VideoDevice::open().is_err(),
            "D3D12 open must fail off-Windows"
        );
    }
    #[cfg(target_os = "windows")]
    {
        // On Windows CI without GPU, open() returns Err(NoDeviceFound); on real GPU it succeeds.
        let _ = D3D12VideoDevice::open();
    }
    // Direct3D 12 requires a valid ID3D12Device pointer; null device allocation must fail cleanly.
    let uninit_device = D3D12VideoDevice {
        adapter_name: "uninitialized".into(),
        is_hardware: false,
        device_ptr: std::ptr::null_mut(),
        video_device_ptr: std::ptr::null_mut(),
    };
    assert!(uninit_device.supports_decode(CodecId::H264));
    assert!(uninit_device.supports_decode(CodecId::Hevc));
    assert!(uninit_device.supports_decode(CodecId::Av1));

    assert!(
        D3D12SurfacePool::new(&uninit_device, 1920, 1080, 8, false).is_err(),
        "Allocation on uninitialized device pointer must fail"
    );
}

#[test]
fn test_d3d12_multislice_batching_and_decode_on_real_video() {
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
        println!("Sample not found, skipping multi-slice D3D12 test");
        return;
    }

    #[allow(unused_mut)]
    let mut demuxer = Mp4Demuxer::open(path).expect("failed to open multislice test video");
    #[allow(unused_variables)]
    let (video_track_idx, video_config) = {
        let stream = demuxer
            .streams()
            .iter()
            .find(|s| s.codec == CodecId::H264)
            .expect("video track not found");
        (stream.index, stream.config.clone())
    };

    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            D3D12VideoDecoder::new(CodecId::H264).is_err(),
            "D3D12 decoder must fail off-Windows"
        );
    }
    #[cfg(target_os = "windows")]
    {
        let mut decoder = match D3D12VideoDecoder::new(CodecId::H264) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Skipping hardware test: D3D12 unavailable ({e})");
                return;
            }
        };

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

        // Submit multislice packets
        let mut decoded_frames = 0;
        while let Some(packet) = demuxer.read_packet().expect("read packet") {
            if packet.stream_index != video_track_idx {
                continue;
            }

            // Convert length-prefixed packet into Annex-B
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
                    frame.dxgi_handle.is_some(),
                    "D3D12 frame must export DXGI shared handle"
                );
                let handle = frame.dxgi_handle.as_ref().unwrap();
                assert!(handle.handle != 0);
                assert_eq!(handle.width, frame.width);
                assert_eq!(handle.height, frame.height);
                decoded_frames += 1;
            }
        }

        decoder.flush().expect("flush decoder");
        while let Some(frame) = decoder.receive_frame().expect("receive frame") {
            assert!(frame.dxgi_handle.is_some());
            decoded_frames += 1;
        }

        assert!(
            decoded_frames > 0,
            "D3D12 decoder should have decoded frames from multislice video"
        );
        println!("D3D12 multi-slice batching decoded {decoded_frames} frames successfully");
    }
}

#[test]
fn test_d3d12_error_resilience_and_corrupt_stream_handling() {
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            D3D12VideoDecoder::new(CodecId::H264).is_err(),
            "D3D12 decoder must fail off-Windows"
        );
    }
    #[cfg(target_os = "windows")]
    {
        let mut decoder = match D3D12VideoDecoder::new(CodecId::H264) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Skipping hardware test: D3D12 unavailable ({e})");
                return;
            }
        };

        // Submit malformed packets
        let malformed = [
            vec![0xAA, 0xBB, 0xCC, 0xDD],                   // random noise
            vec![0x00, 0x00, 0x01, 0xFF],                   // invalid NAL unit type
            vec![0x00, 0x00, 0x00, 0x01, 0x67],             // 1-byte truncated SPS
            vec![0x00, 0x00, 0x00, 0x01, 0x41, 0x01, 0x02], // Slice without active parameter set
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
                "D3D12 decoder must gracefully handle corrupt packets"
            );
        }

        println!("D3D12 decoder survived corrupted packets gracefully");
    }
}
