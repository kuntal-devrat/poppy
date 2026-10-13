use std::collections::VecDeque;

use poppy_core::{AudioConfig, AudioFrame, AudioSampleFormat, ChannelLayout, CodecId, Packet};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{
    CODEC_TYPE_AAC, CODEC_TYPE_FLAC, CODEC_TYPE_MP3, CODEC_TYPE_VORBIS, CodecParameters,
    DecoderOptions,
};
use symphonia::core::formats::Packet as SymphoniaPacket;

use crate::error::AudioError;

/// Trait implemented by all audio decoders in the Poppy audio pipeline.
pub trait AudioDecoder: Send {
    /// Submit a compressed packet to the audio decoder.
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), AudioError>;

    /// Retrieve the next decoded PCM audio frame, if available.
    fn receive_frame(&mut self) -> Result<Option<AudioFrame>, AudioError>;

    /// Signal end-of-stream and drain remaining buffered frames.
    fn flush(&mut self) -> Result<(), AudioError>;

    /// Reset decoder state (e.g. after a seek).
    fn reset(&mut self) -> Result<(), AudioError>;
}

/// In-process audio decoder backed by Symphonia (pure Rust AAC, MP3, FLAC, Vorbis).
pub struct SymphoniaAudioDecoder {
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    output_queue: VecDeque<AudioFrame>,
    sample_rate: u32,
    channels: u16,
    channel_layout: ChannelLayout,
}

impl SymphoniaAudioDecoder {
    pub fn new(codec: CodecId, config: &AudioConfig) -> Result<Self, AudioError> {
        let symphonia_codec = match codec {
            CodecId::Aac => CODEC_TYPE_AAC,
            CodecId::Mp3 => CODEC_TYPE_MP3,
            CodecId::Flac => CODEC_TYPE_FLAC,
            CodecId::Vorbis => CODEC_TYPE_VORBIS,
            _ => return Err(AudioError::UnsupportedCodec(codec)),
        };

        let mut params = CodecParameters::new();
        params.for_codec(symphonia_codec);

        if config.sample_rate > 0 {
            params.with_sample_rate(config.sample_rate);
        }
        if config.channels > 0 {
            // Map channel count to speaker layout explicitly; the previous
            // `(1<<n)-1` bitmask hack was wrong for 5.1/7.1.
            let ch = match config.channels {
                1 => symphonia::core::audio::Channels::FRONT_CENTRE,
                2 => {
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT
                }
                6 => {
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT
                        | symphonia::core::audio::Channels::FRONT_CENTRE
                        | symphonia::core::audio::Channels::LFE1
                        | symphonia::core::audio::Channels::REAR_LEFT
                        | symphonia::core::audio::Channels::REAR_RIGHT
                }
                8 => {
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT
                        | symphonia::core::audio::Channels::FRONT_CENTRE
                        | symphonia::core::audio::Channels::LFE1
                        | symphonia::core::audio::Channels::REAR_LEFT
                        | symphonia::core::audio::Channels::REAR_RIGHT
                        | symphonia::core::audio::Channels::SIDE_LEFT
                        | symphonia::core::audio::Channels::SIDE_RIGHT
                }
                _ => {
                    symphonia::core::audio::Channels::FRONT_LEFT
                        | symphonia::core::audio::Channels::FRONT_RIGHT
                }
            };
            // Only set for 1/2/6/8ch; otherwise leave unset and let Symphonia infer.
            if matches!(config.channels, 1 | 2 | 6 | 8) {
                params.with_channels(ch);
            }
        }
        if let Some(extra) = &config.extradata {
            params.with_extra_data(extra.to_vec().into_boxed_slice());
        }

        let registry = symphonia::default::get_codecs();
        let decoder = registry
            .make(&params, &DecoderOptions::default())
            .map_err(|e| AudioError::InitFailed(e.to_string()))?;

        if config.sample_rate == 0 {
            return Err(AudioError::InitFailed("sample_rate is 0".into()));
        }
        if config.channels == 0 {
            return Err(AudioError::InitFailed("channels is 0".into()));
        }
        let channels = config.channels;
        let sample_rate = config.sample_rate;
        let channel_layout = ChannelLayout::from_channels(channels);

        Ok(Self {
            decoder,
            output_queue: VecDeque::new(),
            sample_rate,
            channels,
            channel_layout,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn channel_layout(&self) -> ChannelLayout {
        self.channel_layout
    }
}

impl AudioDecoder for SymphoniaAudioDecoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), AudioError> {
        if packet.data.is_empty() {
            return Ok(());
        }

        let pts = packet.pts.unwrap_or(packet.dts).max(0) as u64;
        let sym_pkt = SymphoniaPacket::new_from_slice(0, pts, packet.duration as u64, &packet.data);

        match self.decoder.decode(&sym_pkt) {
            Ok(audio_buf) => {
                let spec = *audio_buf.spec();
                let mut sample_buf = SampleBuffer::<f32>::new(audio_buf.capacity() as u64, spec);
                sample_buf.copy_interleaved_ref(audio_buf);

                let samples = sample_buf.samples().to_vec();
                let num_channels = spec.channels.count();
                let num_samples_per_channel = samples.len().checked_div(num_channels).unwrap_or(0);

                let frame = AudioFrame {
                    pts: packet.pts,
                    sample_rate: spec.rate,
                    channels: num_channels as u16,
                    channel_layout: ChannelLayout::from_channels(num_channels as u16),
                    format: AudioSampleFormat::F32Interleaved,
                    samples,
                    num_samples_per_channel,
                };

                self.output_queue.push_back(frame);
                Ok(())
            }
            Err(symphonia::core::errors::Error::DecodeError(msg)) => {
                tracing::warn!("Symphonia decode error (continuing): {msg}");
                Ok(())
            }
            Err(e) => Err(AudioError::DecodeError(e.to_string())),
        }
    }

    fn receive_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        if let Some(frame) = self.output_queue.pop_front() {
            frame.validate().map_err(AudioError::DecodeError)?;
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    fn flush(&mut self) -> Result<(), AudioError> {
        self.output_queue.clear();
        Ok(())
    }

    fn reset(&mut self) -> Result<(), AudioError> {
        self.decoder.reset();
        self.output_queue.clear();
        Ok(())
    }
}

/// Pure-Rust RFC 8251 Opus audio decoder (used in WebM / Matroska containers).
pub struct OpusAudioDecoder {
    decoder: opus_decoder::OpusDecoder,
    output_queue: VecDeque<AudioFrame>,
    sample_rate: u32,
    channels: u16,
    channel_layout: ChannelLayout,
    scratch: Vec<f32>,
}

impl OpusAudioDecoder {
    pub fn new(config: &AudioConfig) -> Result<Self, AudioError> {
        // Opus standard sample rates: 48000, 24000, 16000, 12000, 8000
        let sample_rate = if config.sample_rate == 0 {
            48000
        } else {
            config.sample_rate
        };
        let channels = config.channels.clamp(1, 2) as usize;

        let decoder = opus_decoder::OpusDecoder::new(sample_rate, channels)
            .map_err(|e| AudioError::InitFailed(format!("Opus init failed: {e:?}")))?;

        let max_frame = decoder.max_frame_size_per_channel();
        let scratch = vec![0.0f32; max_frame * channels];
        let channel_layout = ChannelLayout::from_channels(channels as u16);

        Ok(Self {
            decoder,
            output_queue: VecDeque::new(),
            sample_rate,
            channels: channels as u16,
            channel_layout,
            scratch,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn channel_layout(&self) -> ChannelLayout {
        self.channel_layout
    }
}

impl AudioDecoder for OpusAudioDecoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), AudioError> {
        if packet.data.is_empty() {
            return Ok(());
        }
        let channels = self.channels as usize;
        let res = self
            .decoder
            .decode_float(&packet.data, &mut self.scratch, false);

        match res {
            Ok(samples_per_channel) => {
                let total_samples = samples_per_channel * channels;
                let samples = self.scratch[..total_samples].to_vec();

                let frame = AudioFrame {
                    pts: packet.pts,
                    sample_rate: self.sample_rate,
                    channels: self.channels,
                    channel_layout: self.channel_layout,
                    format: AudioSampleFormat::F32Interleaved,
                    samples,
                    num_samples_per_channel: samples_per_channel,
                };

                self.output_queue.push_back(frame);
                Ok(())
            }
            Err(e) => {
                tracing::warn!("Opus decode error (continuing): {e:?}");
                Ok(())
            }
        }
    }

    fn receive_frame(&mut self) -> Result<Option<AudioFrame>, AudioError> {
        if let Some(frame) = self.output_queue.pop_front() {
            frame.validate().map_err(AudioError::DecodeError)?;
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    fn flush(&mut self) -> Result<(), AudioError> {
        self.output_queue.clear();
        Ok(())
    }

    fn reset(&mut self) -> Result<(), AudioError> {
        self.decoder.reset();
        self.output_queue.clear();
        Ok(())
    }
}

/// Factory creating a pure-Rust audio decoder matching the stream's codec ID.
pub fn create_audio_decoder(
    codec: CodecId,
    config: &AudioConfig,
) -> Result<Box<dyn AudioDecoder>, AudioError> {
    match codec {
        CodecId::Aac | CodecId::Mp3 | CodecId::Flac | CodecId::Vorbis => {
            Ok(Box::new(SymphoniaAudioDecoder::new(codec, config)?))
        }
        CodecId::Opus => Ok(Box::new(OpusAudioDecoder::new(config)?)),
        _ => Err(AudioError::UnsupportedCodec(codec)),
    }
}
