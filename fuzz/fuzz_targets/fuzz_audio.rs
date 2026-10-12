#![no_main]

use libfuzzer_sys::fuzz_target;
use poppy_core::{AudioConfig, CodecId};
use poppy_audio::create_audio_decoder;
use poppy_core::Packet;
use bytes::Bytes;

fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }
    // Fuzz Opus decode path (pure Rust, no HW).
    let config = AudioConfig {
        sample_rate: 48000,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    if let Ok(mut dec) = create_audio_decoder(CodecId::Opus, &config) {
        let pkt = Packet {
            stream_index: 0,
            pts: Some(0),
            dts: 0,
            duration: 960,
            is_keyframe: true,
            data: Bytes::copy_from_slice(data),
        };
        let _ = dec.submit_packet(&pkt);
        let _ = dec.receive_frame();
        let _ = dec.flush();
    }
    // Fuzz AAC config path (Symphonia init must not panic).
    let aac_config = AudioConfig {
        sample_rate: 44100,
        channels: 2,
        bits_per_sample: 0,
        extradata: Some(Bytes::copy_from_slice(&data[..data.len().min(64)])),
    };
    let _ = create_audio_decoder(CodecId::Aac, &aac_config);
});
