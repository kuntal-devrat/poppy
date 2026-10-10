#![cfg(target_os = "linux")]

use bytes::Bytes;
use poppy_core::{Packet, PixelFormat};
use poppy_hw::{HwDecoder, VaapiH264Decoder};

#[test]
fn test_vaapi_hardware_decode_idr_frame() {
    let mut decoder = match VaapiH264Decoder::new() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping hardware decode test: no device available ({e})");
            return;
        }
    };

    println!("✅ VA-API hardware decoder created");

    // Canonical H.264 bitstream packet containing SPS, PPS, and IDR slice
    let bitstream: &[u8] = &[
        // SPS: High profile, 1920x1080
        0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x78, 0x02, 0x27, 0xE5,
        0xC0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x36, 0x82, 0x21,
        0x1B, // PPS: CABAC
        0x00, 0x00, 0x00, 0x01, 0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0, // IDR Slice
        0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00, 0x10, 0xFF,
    ];

    let packet = Packet {
        stream_index: 0,
        pts: Some(0),
        dts: 0,
        duration: 1000,
        is_keyframe: true,
        data: Bytes::from_static(bitstream),
    };

    decoder
        .submit_packet(&packet)
        .expect("Hardware decode submission failed");

    println!("✅ Submitted packet to GPU decode ASIC");

    decoder.flush().expect("Flush failed");

    let frame = decoder
        .receive_frame()
        .expect("Failed to receive frame")
        .expect("Expected a decoded frame from IDR packet");

    println!(
        "✨ Decoded Frame: {}x{}, format={:?}, pts={:?}",
        frame.width, frame.height, frame.pixel_format, frame.pts
    );

    assert_eq!(frame.width, 1920);
    assert_eq!(frame.height, 1080);
    assert_eq!(frame.pixel_format, PixelFormat::Nv12);
    assert_eq!(frame.pts, Some(0));
}
