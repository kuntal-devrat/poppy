use poppy_audio::{AudioError, AudioSink, create_audio_decoder};
use poppy_core::{AudioConfig, AudioFrame, AudioSampleFormat, ChannelLayout, CodecId};

#[test]
fn test_audio_channel_count_greater_than_8() {
    for ch in [9u16, 10, 12, 16, 24, 32] {
        let layout = ChannelLayout::from_channels(ch);
        assert_eq!(layout, ChannelLayout::Custom(ch));
        assert_eq!(layout.channels(), ch);

        let num_samples_per_channel = 480;
        let total_samples = (ch as usize) * num_samples_per_channel;
        let frame = AudioFrame {
            pts: Some(0),
            sample_rate: 48000,
            channels: ch,
            channel_layout: layout,
            format: AudioSampleFormat::F32Interleaved,
            samples: vec![0.02f32; total_samples],
            num_samples_per_channel,
        };

        // Frame must pass internal invariant validation
        assert!(
            frame.validate().is_ok(),
            "validation failed for {ch} channels"
        );

        // AudioSink in stereo mode must downmix multi-channel frames (>8 channels) safely
        let stereo_sink = AudioSink::try_new(48000, 2).expect("initialize stereo AudioSink");
        let dev_rate = stereo_sink.device_sample_rate();
        let frame = AudioFrame {
            pts: Some(0),
            sample_rate: dev_rate,
            channels: ch,
            channel_layout: layout,
            format: AudioSampleFormat::F32Interleaved,
            samples: vec![0.02f32; total_samples],
            num_samples_per_channel,
        };
        stereo_sink.push_frame(&frame);
        assert_eq!(
            stereo_sink.queued_samples(),
            num_samples_per_channel * (stereo_sink.device_channels() as usize)
        );

        // AudioSink in custom N-channel mode
        let multi_sink =
            AudioSink::try_new(dev_rate, ch).expect("initialize multi-channel AudioSink");
        let multi_frame = AudioFrame {
            pts: Some(0),
            sample_rate: multi_sink.device_sample_rate(),
            channels: ch,
            channel_layout: layout,
            format: AudioSampleFormat::F32Interleaved,
            samples: vec![0.02f32; total_samples],
            num_samples_per_channel,
        };
        multi_sink.push_frame(&multi_frame);
        assert!(multi_sink.queued_samples() > 0);
    }
}

#[test]
fn test_audio_frame_validation_failure_propagation() {
    // 1. Inconsistent sample count
    let bad_len_frame = AudioFrame {
        pts: Some(0),
        sample_rate: 48000,
        channels: 2,
        channel_layout: ChannelLayout::Stereo,
        format: AudioSampleFormat::F32Interleaved,
        samples: vec![0.1; 100], // Expected 2 * 960 = 1920
        num_samples_per_channel: 960,
    };
    let err = bad_len_frame
        .validate()
        .expect_err("should reject mismatched samples length");
    assert!(err.contains("AudioFrame samples len 100 != 960x2"));

    // 2. Zero sample rate
    let zero_rate_frame = AudioFrame {
        pts: Some(0),
        sample_rate: 0,
        channels: 2,
        channel_layout: ChannelLayout::Stereo,
        format: AudioSampleFormat::F32Interleaved,
        samples: vec![0.1; 1920],
        num_samples_per_channel: 960,
    };
    let err = zero_rate_frame
        .validate()
        .expect_err("should reject zero sample rate");
    assert!(err.contains("sample_rate is 0"));

    // 3. Zero channel count
    let zero_chan_frame = AudioFrame {
        pts: Some(0),
        sample_rate: 48000,
        channels: 0,
        channel_layout: ChannelLayout::Custom(0),
        format: AudioSampleFormat::F32Interleaved,
        samples: vec![],
        num_samples_per_channel: 960,
    };
    let err = zero_chan_frame
        .validate()
        .expect_err("should reject zero channels");
    assert!(err.contains("channels is 0"));

    // 4. Propagation via AudioError::DecodeError
    let decode_err = AudioError::DecodeError(err);
    let err_str = decode_err.to_string();
    assert!(err_str.contains("channels is 0"));
}

#[test]
fn test_audio_decoder_rejects_empty_packet() {
    let opus_config = AudioConfig {
        sample_rate: 48000,
        channels: 2,
        bits_per_sample: 16,
        extradata: None,
    };
    let mut dec = create_audio_decoder(CodecId::Opus, &opus_config).expect("create opus decoder");
    let empty_pkt = poppy_core::Packet {
        stream_index: 0,
        pts: Some(0),
        dts: 0,
        duration: 960,
        is_keyframe: true,
        data: bytes::Bytes::new(),
    };

    // Submitting 0-length packet should succeed gracefully without crashing
    assert!(dec.submit_packet(&empty_pkt).is_ok());
    // Receiving frame should return Ok(None) because no audio was decoded
    let frame = dec
        .receive_frame()
        .expect("receive frame from empty packet");
    assert!(frame.is_none());
}
