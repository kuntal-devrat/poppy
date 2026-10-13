//! High-level media stream pipeline.
//!
//! Provides a clean, typed stream that demuxes containers and feeds frames directly
//! into hardware decoding.

use std::path::Path;

use poppy_core::{CodecId, Packet, StreamInfo, codec::CodecConfig};
use poppy_demux::{Demuxer, MatroskaDemuxer, Mp4Demuxer, MpegTsDemuxer};
use poppy_hw::{DecodedFrame, HwDecoder};
#[cfg(target_os = "linux")]
use poppy_hw::{VaapiAv1Decoder, VaapiH264Decoder, VaapiHevcDecoder, VaapiVp9Decoder};

use crate::error::PoppyError;

/// High-level media stream representing an open media file.
pub struct MediaStream {
    demuxer: Box<dyn Demuxer>,
    decoder: Option<Box<dyn HwDecoder>>,
    target_stream_index: usize,
    codec: CodecId,
    config: CodecConfig,
    is_eos: bool,

    // Audio pipeline
    audio_stream_index: Option<usize>,
    audio_decoder: Option<Box<dyn poppy_audio::AudioDecoder>>,
    audio_config: Option<poppy_core::AudioConfig>,
    pending_audio_frames: std::collections::VecDeque<poppy_core::AudioFrame>,

    // Decoupled packet queues to prevent GPU surface pool exhaustion
    pending_video_packets: std::collections::VecDeque<Packet>,
    pending_audio_packets: std::collections::VecDeque<Packet>,
}

impl MediaStream {
    /// Creates a new builder for configuring stream playback.
    pub fn builder() -> crate::builder::MediaStreamBuilder {
        crate::builder::MediaStreamBuilder::new()
    }

    /// Open a media file with automatic container format and hardware decoder detection.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, PoppyError> {
        Self::open_with_builder(path, &crate::builder::MediaStreamBuilder::default())
    }

    /// Open a media file with explicit builder configuration options.
    pub fn open_with_builder<P: AsRef<Path>>(
        path: P,
        builder: &crate::builder::MediaStreamBuilder,
    ) -> Result<Self, PoppyError> {
        let p = path.as_ref();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let demuxer: Box<dyn Demuxer> = match ext.as_str() {
            "mp4" | "m4v" | "mov" => Box::new(Mp4Demuxer::open(p)?),
            "mkv" | "webm" => Box::new(MatroskaDemuxer::open(p)?),
            "ts" | "mts" | "m2ts" => Box::new(MpegTsDemuxer::open(p)?),
            _ => {
                // Try MP4 first, then MKV, then MPEG-TS
                if let Ok(mp4) = Mp4Demuxer::open(p) {
                    Box::new(mp4)
                } else if let Ok(mkv) = MatroskaDemuxer::open(p) {
                    Box::new(mkv)
                } else if let Ok(ts) = MpegTsDemuxer::open(p) {
                    Box::new(ts)
                } else {
                    return Err(PoppyError::UnsupportedFormat(p.display().to_string()));
                }
            }
        };

        if demuxer.streams().is_empty() {
            return Err(PoppyError::NoStreams);
        }

        // Find primary video track or user-specified track
        let target_stream_index = if let Some(idx) = builder.stream_index {
            if idx >= demuxer.streams().len() {
                return Err(PoppyError::Demux(
                    poppy_demux::DemuxError::SampleOutOfRange {
                        index: idx as u32,
                        total: demuxer.streams().len() as u32,
                    },
                ));
            }
            idx
        } else {
            let video_candidates: Vec<(usize, &StreamInfo)> = demuxer
                .streams()
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    matches!(
                        s.codec,
                        CodecId::H264 | CodecId::Hevc | CodecId::Av1 | CodecId::Vp9
                    )
                })
                .collect();

            if video_candidates.is_empty() {
                0
            } else {
                video_candidates
                    .into_iter()
                    .max_by_key(|&(i, s)| {
                        let area = (s.width as u64) * (s.height as u64);
                        (s.enabled, area, std::cmp::Reverse(i))
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
        };

        let stream_info = &demuxer.streams()[target_stream_index];
        let codec = stream_info.codec;
        let config = stream_info.config.clone();

        let mut audio_stream_index = None;
        let mut audio_config = None;
        let mut audio_decoder = None;

        for (i, s) in demuxer.streams().iter().enumerate() {
            if matches!(
                s.codec,
                CodecId::Aac | CodecId::Opus | CodecId::Mp3 | CodecId::Flac | CodecId::Vorbis
            ) {
                audio_stream_index = Some(i);
                if let CodecConfig::Audio(cfg) = &s.config {
                    audio_config = Some(cfg.clone());
                    audio_decoder = poppy_audio::create_audio_decoder(s.codec, cfg).ok();
                }
                break;
            }
        }

        // Attempt hardware decoder creation for primary video track (if enabled)
        let hw_decoder: Option<Box<dyn HwDecoder>> = if !builder.hardware_acceleration {
            None
        } else {
            #[cfg(target_os = "linux")]
            let decoder: Option<Box<dyn HwDecoder>> = if stream_info.codec == CodecId::H264 {
                if let Ok(mut dec) = VaapiH264Decoder::new() {
                    let mut init_ok = true;
                    if let poppy_core::codec::CodecConfig::H264(avcc) = &stream_info.config {
                        for sps in &avcc.sps_list {
                            let mut data = vec![0, 0, 0, 1];
                            data.extend_from_slice(sps);
                            let pkt = Packet {
                                stream_index: target_stream_index,
                                pts: None,
                                dts: 0,
                                duration: 0,
                                data: bytes::Bytes::from(data),
                                is_keyframe: false,
                            };
                            if let Err(e) = dec.submit_packet(&pkt) {
                                tracing::warn!(
                                    "Failed to submit SPS parameter set to VA-API H.264 decoder: {e:?}"
                                );
                                init_ok = false;
                                break;
                            }
                        }
                        if init_ok {
                            for pps in &avcc.pps_list {
                                let mut data = vec![0, 0, 0, 1];
                                data.extend_from_slice(pps);
                                let pkt = Packet {
                                    stream_index: target_stream_index,
                                    pts: None,
                                    dts: 0,
                                    duration: 0,
                                    data: bytes::Bytes::from(data),
                                    is_keyframe: false,
                                };
                                if let Err(e) = dec.submit_packet(&pkt) {
                                    tracing::warn!(
                                        "Failed to submit PPS parameter set to VA-API H.264 decoder: {e:?}"
                                    );
                                    init_ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if init_ok {
                        Some(Box::new(dec) as Box<dyn HwDecoder>)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else if stream_info.codec == CodecId::Hevc {
                let is_10bit =
                    if let poppy_core::codec::CodecConfig::Hevc(hvcc) = &stream_info.config {
                        hvcc.is_10bit()
                    } else {
                        false
                    };

                if let Ok(mut dec) = VaapiHevcDecoder::new(is_10bit) {
                    let mut init_ok = true;
                    if let poppy_core::codec::CodecConfig::Hevc(hvcc) = &stream_info.config {
                        for vps in &hvcc.vps_list {
                            let mut data = vec![0, 0, 0, 1];
                            data.extend_from_slice(vps);
                            let pkt = Packet {
                                stream_index: target_stream_index,
                                pts: None,
                                dts: 0,
                                duration: 0,
                                data: bytes::Bytes::from(data),
                                is_keyframe: false,
                            };
                            if let Err(e) = dec.submit_packet(&pkt) {
                                tracing::warn!(
                                    "Failed to submit VPS parameter set to VA-API HEVC decoder: {e:?}"
                                );
                                init_ok = false;
                                break;
                            }
                        }
                        if init_ok {
                            for sps in &hvcc.sps_list {
                                let mut data = vec![0, 0, 0, 1];
                                data.extend_from_slice(sps);
                                let pkt = Packet {
                                    stream_index: target_stream_index,
                                    pts: None,
                                    dts: 0,
                                    duration: 0,
                                    data: bytes::Bytes::from(data),
                                    is_keyframe: false,
                                };
                                if let Err(e) = dec.submit_packet(&pkt) {
                                    tracing::warn!(
                                        "Failed to submit SPS parameter set to VA-API HEVC decoder: {e:?}"
                                    );
                                    init_ok = false;
                                    break;
                                }
                            }
                        }
                        if init_ok {
                            for pps in &hvcc.pps_list {
                                let mut data = vec![0, 0, 0, 1];
                                data.extend_from_slice(pps);
                                let pkt = Packet {
                                    stream_index: target_stream_index,
                                    pts: None,
                                    dts: 0,
                                    duration: 0,
                                    data: bytes::Bytes::from(data),
                                    is_keyframe: false,
                                };
                                if let Err(e) = dec.submit_packet(&pkt) {
                                    tracing::warn!(
                                        "Failed to submit PPS parameter set to VA-API HEVC decoder: {e:?}"
                                    );
                                    init_ok = false;
                                    break;
                                }
                            }
                        }
                    }
                    if init_ok {
                        Some(Box::new(dec) as Box<dyn HwDecoder>)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else if stream_info.codec == CodecId::Av1 {
                let is_10bit =
                    if let poppy_core::codec::CodecConfig::Av1(av1c) = &stream_info.config {
                        av1c.is_10bit()
                    } else {
                        false
                    };

                if let Ok(mut dec) = VaapiAv1Decoder::new(is_10bit) {
                    let mut init_ok = true;
                    if let poppy_core::codec::CodecConfig::Av1(av1c) = &stream_info.config
                        && !av1c.config_obus.is_empty()
                    {
                        let res = dec.set_extradata(&av1c.config_obus);
                        if let Err(e) = res {
                            tracing::warn!(
                                "Failed to submit config OBUs to VA-API AV1 decoder: {e:?}"
                            );
                            init_ok = false;
                        }
                    }
                    if init_ok {
                        Some(Box::new(dec) as Box<dyn HwDecoder>)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else if stream_info.codec == CodecId::Vp9 {
                let is_10bit =
                    if let poppy_core::codec::CodecConfig::Vp9(vpcc) = &stream_info.config {
                        vpcc.is_10bit()
                    } else {
                        false
                    };

                if let Ok(mut dec) = VaapiVp9Decoder::new(is_10bit) {
                    let mut init_ok = true;
                    if let poppy_core::codec::CodecConfig::Vp9(vpcc) = &stream_info.config
                        && !vpcc.raw.is_empty()
                    {
                        let res = dec.set_extradata(&vpcc.raw);
                        if let Err(e) = res {
                            tracing::warn!(
                                "Failed to submit extradata to VA-API VP9 decoder: {e:?}"
                            );
                            init_ok = false;
                        }
                    }
                    if init_ok {
                        Some(Box::new(dec) as Box<dyn HwDecoder>)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(target_os = "windows")]
            let decoder: Option<Box<dyn HwDecoder>> = {
                if let Ok(mut dec) = poppy_hw::d3d12::D3D12VideoDecoder::new(stream_info.codec) {
                    if let poppy_core::codec::CodecConfig::H264(avcc) = &stream_info.config {
                        let _ = dec.set_extradata(&avcc.raw);
                    } else if let poppy_core::codec::CodecConfig::Hevc(hvcc) = &stream_info.config {
                        let _ = dec.set_extradata(&hvcc.raw);
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
                } else {
                    None
                }
            };

            #[cfg(target_os = "macos")]
            let decoder: Option<Box<dyn HwDecoder>> = {
                if let Ok(mut dec) =
                    poppy_hw::videotoolbox::VideoToolboxDecoder::new(stream_info.codec)
                {
                    let mut init_ok = false;
                    if let poppy_core::codec::CodecConfig::H264(avcc) = &stream_info.config {
                        if dec.set_extradata(&avcc.raw).is_ok() {
                            init_ok = true;
                        }
                    } else if let poppy_core::codec::CodecConfig::Hevc(hvcc) = &stream_info.config {
                        if dec.set_extradata(&hvcc.raw).is_ok() {
                            init_ok = true;
                        }
                    }
                    if init_ok {
                        Some(Box::new(dec) as Box<dyn HwDecoder>)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let decoder: Option<Box<dyn HwDecoder>> = None;
            decoder
        };

        // Fall back to software decode if hardware decoder is unavailable
        let decoder: Option<Box<dyn HwDecoder>> = if hw_decoder.is_some() {
            hw_decoder
        } else if builder.software_fallback {
            if let Ok(mut sw_dec) = poppy_hw::sw::create_software_decoder(stream_info.codec) {
                if let poppy_core::codec::CodecConfig::H264(avcc) = &stream_info.config {
                    for sps in &avcc.sps_list {
                        let mut data = vec![0, 0, 0, 1];
                        data.extend_from_slice(sps);
                        let pkt = Packet {
                            stream_index: target_stream_index,
                            pts: None,
                            dts: 0,
                            duration: 0,
                            data: bytes::Bytes::from(data),
                            is_keyframe: false,
                        };
                        let _ = sw_dec.submit_packet(&pkt);
                    }
                    for pps in &avcc.pps_list {
                        let mut data = vec![0, 0, 0, 1];
                        data.extend_from_slice(pps);
                        let pkt = Packet {
                            stream_index: target_stream_index,
                            pts: None,
                            dts: 0,
                            duration: 0,
                            data: bytes::Bytes::from(data),
                            is_keyframe: false,
                        };
                        let _ = sw_dec.submit_packet(&pkt);
                    }
                }
                Some(sw_dec)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            demuxer,
            decoder,
            target_stream_index,
            codec,
            config,
            is_eos: false,
            audio_stream_index,
            audio_decoder,
            audio_config,
            pending_audio_frames: std::collections::VecDeque::new(),
            pending_video_packets: std::collections::VecDeque::new(),
            pending_audio_packets: std::collections::VecDeque::new(),
        })
    }

    /// Access metadata for all streams in the media container.
    /// Internal filter to convert container length-prefixed NALs to Annex-B format.
    #[allow(dead_code)]
    pub fn streams(&self) -> &[StreamInfo] {
        self.demuxer.streams()
    }

    /// Read the next demuxed compressed packet.
    pub fn next_packet(&mut self) -> Result<Option<Packet>, PoppyError> {
        let pkt = self.demuxer.read_packet()?;
        Ok(pkt)
    }

    /// Read and decode the next hardware-accelerated video frame.
    ///
    /// Pulls compressed packets from the container and feeds them directly
    /// to the GPU decode ASIC until a frame is produced or end-of-stream is reached.
    pub fn next_frame(&mut self) -> Result<Option<DecodedFrame>, PoppyError> {
        let codec = self.codec;
        let config = &self.config;
        let target_stream_index = self.target_stream_index;
        let decoder = match &mut self.decoder {
            Some(d) => d,
            None => return Ok(None),
        };

        loop {
            // First check if decoder has buffered frames ready
            if let Some(frame) = decoder.receive_frame()? {
                return Ok(Some(frame));
            }

            // Check if we have queued video packets ready to submit to the GPU decoder
            if let Some(pkt) = self.pending_video_packets.pop_front() {
                let pkt = filter_to_annex_b(codec, config, pkt)?;
                decoder.submit_packet(&pkt)?;
                if let Some(frame) = decoder.receive_frame()? {
                    return Ok(Some(frame));
                }
                continue;
            }

            if self.is_eos {
                return Ok(None);
            }

            // Demux next packet
            match self.demuxer.read_packet()? {
                Some(pkt) => {
                    if pkt.stream_index == target_stream_index {
                        let pkt = filter_to_annex_b(codec, config, pkt)?;
                        decoder.submit_packet(&pkt)?;
                        if let Some(frame) = decoder.receive_frame()? {
                            return Ok(Some(frame));
                        }
                    } else if Some(pkt.stream_index) == self.audio_stream_index {
                        // Bound queue: drop oldest audio when video-only consumer stalls audio.
                        const MAX_PENDING: usize = 256;
                        if self.pending_audio_packets.len() >= MAX_PENDING {
                            self.pending_audio_packets.pop_front();
                        }
                        self.pending_audio_packets.push_back(pkt);
                    }
                }
                None => {
                    self.is_eos = true;
                    // Flush decoder at end-of-stream
                    decoder.flush()?;
                    if let Some(a) = &mut self.audio_decoder {
                        let _ = a.flush();
                    }
                }
            }
        }
    }

    /// Returns true if an audio stream was discovered and initialized.
    pub fn has_audio(&self) -> bool {
        self.audio_decoder.is_some()
    }

    /// Returns the active audio stream index, if present.
    pub fn audio_stream_index(&self) -> Option<usize> {
        self.audio_stream_index
    }

    /// Returns the active audio stream timebase, if present.
    pub fn audio_time_base(&self) -> Option<poppy_core::TimeBase> {
        let idx = self.audio_stream_index?;
        self.streams().get(idx).map(|s| s.time_base)
    }

    /// Access configuration for the audio stream, if present.
    pub fn audio_config(&self) -> Option<&poppy_core::AudioConfig> {
        self.audio_config.as_ref()
    }

    /// Read the next decoded audio frame from the media stream.
    pub fn next_audio_frame(&mut self) -> Result<Option<poppy_core::AudioFrame>, PoppyError> {
        if let Some(frame) = self.pending_audio_frames.pop_front() {
            frame
                .validate()
                .map_err(|e| PoppyError::Audio(poppy_audio::AudioError::DecodeError(e)))?;
            return Ok(Some(frame));
        }

        let a_dec = match &mut self.audio_decoder {
            Some(a) => a,
            None => return Ok(None),
        };

        loop {
            if let Some(frame) = a_dec.receive_frame()? {
                return Ok(Some(frame));
            }

            // Check if we have queued audio packets ready to submit
            if let Some(pkt) = self.pending_audio_packets.pop_front() {
                a_dec.submit_packet(&pkt)?;
                if let Some(frame) = a_dec.receive_frame()? {
                    return Ok(Some(frame));
                }
                continue;
            }

            if self.is_eos {
                return Ok(None);
            }

            match self.demuxer.read_packet()? {
                Some(pkt) => {
                    if Some(pkt.stream_index) == self.audio_stream_index {
                        a_dec.submit_packet(&pkt)?;
                        if let Some(frame) = a_dec.receive_frame()? {
                            return Ok(Some(frame));
                        }
                    } else if pkt.stream_index == self.target_stream_index {
                        // Store video packet in lightweight queue; do NOT decode on GPU yet.
                        // Bound to prevent memory bloat when audio-only consumer stalls video.
                        const MAX_PENDING: usize = 256;
                        if self.pending_video_packets.len() >= MAX_PENDING {
                            self.pending_video_packets.pop_front();
                        }
                        self.pending_video_packets.push_back(pkt);
                    }
                }
                None => {
                    self.is_eos = true;
                    if let Some(d) = &mut self.decoder {
                        let _ = d.flush();
                    }
                    let _ = a_dec.flush();
                }
            }
        }
    }

    /// Seek to the nearest keyframe at or before `timestamp_ticks`.
    pub fn seek(&mut self, timestamp_ticks: i64) -> Result<(), PoppyError> {
        self.is_eos = false;
        self.pending_audio_frames.clear();
        self.pending_video_packets.clear();
        self.pending_audio_packets.clear();
        self.demuxer
            .seek_to_keyframe(self.target_stream_index, timestamp_ticks)?;
        if let Some(d) = &mut self.decoder {
            d.reset()?;
        }
        if let Some(a) = &mut self.audio_decoder {
            a.reset()?;
        }
        Ok(())
    }

    /// Seek to the nearest keyframe at or before `target_seconds`.
    pub fn seek_seconds(&mut self, target_seconds: f64) -> Result<(), PoppyError> {
        if !target_seconds.is_finite() {
            return Err(PoppyError::InvalidSeek(format!(
                "non-finite seek target {target_seconds}"
            )));
        }
        let stream = &self.demuxer.streams()[self.target_stream_index];
        if stream.time_base.num == 0 || stream.time_base.den == 0 {
            return Err(PoppyError::InvalidSeek("zero timebase".into()));
        }
        let ticks = stream.time_base.from_seconds(target_seconds);
        self.seek(ticks)
    }

    /// Returns the target video track's rational time base.
    pub fn time_base(&self) -> poppy_core::TimeBase {
        self.demuxer.streams()[self.target_stream_index].time_base
    }

    /// Returns total stream duration in seconds if signaled by container metadata.
    pub fn duration_seconds(&self) -> Option<f64> {
        let stream = &self.demuxer.streams()[self.target_stream_index];
        stream
            .duration_ticks
            .map(|ticks| stream.time_base.to_seconds(ticks))
    }

    /// Target stream index currently decoded.
    pub fn target_stream_index(&self) -> usize {
        self.target_stream_index
    }

    /// Returns the active track's codec identifier.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// Returns the active track's configuration record.
    pub fn config(&self) -> &CodecConfig {
        &self.config
    }

    /// Total duration in time base ticks if signaled by container metadata.
    pub fn duration_ticks(&self) -> Option<i64> {
        self.demuxer.streams()[self.target_stream_index].duration_ticks
    }

    /// Indicates whether a hardware decoder is actively bound to this stream.
    pub fn has_hardware_decoder(&self) -> bool {
        self.decoder.is_some()
    }

    /// Returns the estimated or average frame duration in seconds for the target video track, if known.
    pub fn estimated_frame_duration_seconds(&self) -> Option<f64> {
        self.demuxer
            .estimated_frame_duration_seconds(self.target_stream_index)
    }

    /// Best-effort colour description for the active video track.
    /// Inspects H.264 SPS VUI, HEVC VPS/SPS VUI-equivalent, AV1 sequence header,
    /// and VP9 colour config when available; falls back to BT.709 (SDR).
    /// Returns `(matrix_coefficients, full_range, is_hdr)`-derived render `ColorSpace`-compatible tuple.
    /// Callers in `poppy-render` map this to `ColorSpace::Bt709/Bt601/Bt2020`.
    pub fn colour_description(&self) -> (u8, bool, bool) {
        match &self.config {
            CodecConfig::H264(avcc) => {
                for sps_bytes in avcc.sps_list.iter().take(1) {
                    if let Ok(sps) = poppy_h264::sps::Sps::parse(sps_bytes)
                        && let Some(vui) = &sps.vui
                        && vui.colour_description_present
                    {
                        let hdr = vui.transfer_characteristics == 16
                            || vui.transfer_characteristics == 18;
                        return (vui.matrix_coefficients, vui.video_full_range, hdr);
                    }
                }
                (1, false, false) // BT.709 limited SDR
            }
            CodecConfig::Hevc(hvcc) => {
                // HVC SPS has no full VUI parsed yet; use bit-depth as HDR hint.
                (if hvcc.is_10bit() { 9 } else { 1 }, false, hvcc.is_10bit())
            }
            CodecConfig::Av1(av1c) => {
                // av1C config OBUs contain a Sequence Header with colour config.
                // Parse first OBU if present.
                if !av1c.config_obus.is_empty()
                    && let Ok(seq) =
                        poppy_av1::sequence_header::SequenceHeader::parse(&av1c.config_obus)
                {
                    let hdr = seq.high_bitdepth;
                    return (seq.matrix_coefficients, seq.color_range, hdr);
                }
                (1, false, av1c.is_10bit())
            }
            CodecConfig::Vp9(vpcc) => {
                let hdr = vpcc.bit_depth >= 10;
                (vpcc.matrix_coefficients, vpcc.video_full_range_flag, hdr)
            }
            _ => (1, false, false),
        }
    }
}

/// Internal helper to transform container length-prefixed NALs to Annex-B format.
fn filter_to_annex_b(
    codec: CodecId,
    config: &CodecConfig,
    pkt: Packet,
) -> Result<Packet, PoppyError> {
    if pkt.data.is_empty() {
        return Ok(pkt);
    }
    match (codec, config) {
        (CodecId::H264, CodecConfig::H264(avcc)) => {
            let mut out = Vec::with_capacity(pkt.data.len() + 64);
            poppy_demux::bsf::avcc_to_annex_b(&pkt.data, avcc.nal_length_size, None, &mut out)
                .map_err(|e| PoppyError::Hardware(poppy_hw::HwError::DecodeError(e.to_string())))?;
            Ok(Packet {
                stream_index: pkt.stream_index,
                pts: pkt.pts,
                dts: pkt.dts,
                duration: pkt.duration,
                data: bytes::Bytes::from(out),
                is_keyframe: pkt.is_keyframe,
            })
        }
        (CodecId::Hevc, CodecConfig::Hevc(hvcc)) => {
            let mut out = Vec::with_capacity(pkt.data.len() + 64);
            poppy_demux::bsf::hevc_to_annex_b(
                &pkt.data,
                hvcc.nal_length_size,
                None,
                None,
                None,
                &mut out,
            )
            .map_err(|e| PoppyError::Hardware(poppy_hw::HwError::DecodeError(e.to_string())))?;
            Ok(Packet {
                stream_index: pkt.stream_index,
                pts: pkt.pts,
                dts: pkt.dts,
                duration: pkt.duration,
                data: bytes::Bytes::from(out),
                is_keyframe: pkt.is_keyframe,
            })
        }
        _ => Ok(pkt),
    }
}
