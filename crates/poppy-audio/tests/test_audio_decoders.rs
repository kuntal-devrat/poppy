use poppy_audio::{AudioClock, AudioSink, create_audio_decoder};
use poppy_core::{AudioConfig, AudioFrame, AudioSampleFormat, ChannelLayout, CodecId};

#[test]
fn test_audio_clock_math() {
    let clock = AudioClock::new();
    clock.pause();
    assert_eq!(clock.current_pts_seconds(), 0.0);

    clock.reset_to(1.5);
    let pts = clock.current_pts_seconds();
    assert!((pts - 1.5).abs() < 1e-4);

    clock.reset_to(0.0);
    assert!((clock.current_pts_seconds() - 0.0).abs() < 1e-4);
}

#[test]
fn test_audio_sink_headless_mode() {
    let sink = AudioSink::try_new(48000, 2).expect("AudioSink must initialize");
    assert_eq!(sink.queued_samples(), 0);

    let rate = sink.device_sample_rate();
    let channels = sink.device_channels();
    let frame = AudioFrame {
        pts: Some(0),
        sample_rate: rate,
        channels,
        channel_layout: ChannelLayout::from_channels(channels),
        format: AudioSampleFormat::F32Interleaved,
        samples: vec![0.1; 960],
        num_samples_per_channel: 960 / channels as usize,
    };

    sink.push_frame(&frame);
    assert_eq!(sink.queued_samples(), 960);

    sink.pause();
    sink.play();

    sink.reset_to(0.5);
    assert_eq!(sink.queued_samples(), 0);
    assert!((sink.clock().current_pts_seconds() - 0.5).abs() < 1e-4);
}

#[test]
fn test_create_audio_decoders() {
    let aac_config = AudioConfig {
        sample_rate: 44100,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    let aac_dec = create_audio_decoder(CodecId::Aac, &aac_config);
    assert!(aac_dec.is_ok());

    let opus_config = AudioConfig {
        sample_rate: 48000,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    let opus_dec = create_audio_decoder(CodecId::Opus, &opus_config);
    assert!(opus_dec.is_ok());

    let unsupp_config = AudioConfig {
        sample_rate: 48000,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    let unsupp_dec = create_audio_decoder(CodecId::H264, &unsupp_config);
    assert!(unsupp_dec.is_err());

    let zero_rate_config = AudioConfig {
        sample_rate: 0,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    let zero_rate_dec = create_audio_decoder(CodecId::Aac, &zero_rate_config);
    assert!(zero_rate_dec.is_err(), "sample_rate == 0 must be rejected");

    let zero_chan_config = AudioConfig {
        sample_rate: 44100,
        channels: 0,
        bits_per_sample: 16,
        extradata: None,
    };
    let zero_chan_dec = create_audio_decoder(CodecId::Aac, &zero_chan_config);
    assert!(zero_chan_dec.is_err(), "channels == 0 must be rejected");
}

#[test]
fn test_demux_and_decode_real_aac_mp4() {
    use poppy_demux::{Demuxer, Mp4Demuxer};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/samples/test_av_aac.mp4"
    );
    let mut demuxer = Mp4Demuxer::open(path).expect("failed to open test_av_aac.mp4");

    let (audio_stream_idx, audio_config) = demuxer
        .streams()
        .iter()
        .enumerate()
        .find_map(|(i, s)| {
            if let poppy_core::CodecConfig::Audio(cfg) = &s.config {
                Some((i, cfg.clone()))
            } else {
                None
            }
        })
        .expect("must contain audio stream");

    let mut decoder =
        create_audio_decoder(CodecId::Aac, &audio_config).expect("create aac decoder");
    let mut decoded_frames = 0;

    while let Ok(Some(packet)) = demuxer.read_packet() {
        if packet.stream_index == audio_stream_idx {
            decoder.submit_packet(&packet).expect("submit packet");
            while let Ok(Some(frame)) = decoder.receive_frame() {
                assert!(frame.sample_rate > 0);
                assert!(!frame.samples.is_empty());
                decoded_frames += 1;
            }
        }
    }

    assert!(decoded_frames > 0, "must decode at least 1 AAC frame");
}

#[test]
fn test_demux_and_decode_real_opus_webm() {
    use poppy_demux::{Demuxer, MatroskaDemuxer};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/samples/test_av_opus.webm"
    );
    let mut demuxer = MatroskaDemuxer::open(path).expect("failed to open test_av_opus.webm");

    let (audio_stream_idx, audio_config) = demuxer
        .streams()
        .iter()
        .enumerate()
        .find_map(|(i, s)| {
            if let poppy_core::CodecConfig::Audio(cfg) = &s.config {
                Some((i, cfg.clone()))
            } else {
                None
            }
        })
        .expect("must contain audio stream");

    let mut decoder =
        create_audio_decoder(CodecId::Opus, &audio_config).expect("create opus decoder");
    let mut decoded_frames = 0;

    while let Ok(Some(packet)) = demuxer.read_packet() {
        if packet.stream_index == audio_stream_idx {
            decoder.submit_packet(&packet).expect("submit packet");
            while let Ok(Some(frame)) = decoder.receive_frame() {
                assert_eq!(frame.sample_rate, 48000);
                assert!(!frame.samples.is_empty());
                decoded_frames += 1;
            }
        }
    }

    assert!(decoded_frames > 0, "must decode at least 1 Opus frame");
}
