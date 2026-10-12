//! High-level media stream pipeline.
//!
//! Provides a clean, typed stream that demuxes containers and feeds frames directly
//! into hardware decoding.

use std::path::Path;

use poppy_core::{CodecId, Packet, StreamInfo, codec::CodecConfig};
use poppy_demux::{Demuxer, MatroskaDemuxer, Mp4Demuxer};
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
            _ => {
                // Try MP4 first, then MKV
                if let Ok(mp4) = Mp4Demuxer::open(p) {
                    Box::new(mp4)
                } else if let Ok(mkv) = MatroskaDemuxer::open(p) {
                    Box::new(mkv)
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
            let mut idx = 0;
            for (i, s) in demuxer.streams().iter().enumerate() {
                if matches!(
                    s.codec,
                    CodecId::H264 | CodecId::Hevc | CodecId::Av1 | CodecId::Vp9
                ) {
                    idx = i;
                    break;
                }
            }
            idx
        };

        let stream_info = &demuxer.streams()[target_stream_index];
        let codec = stream_info.codec;
        let config = stream_info.config.clone();

        // Attempt hardware decoder creation for primary video track (if enabled)
        let decoder: Option<Box<dyn HwDecoder>> = if !builder.hardware_acceleration {
            None
        } else {
            #[cfg(target_os = "linux")]
            let decoder: Option<Box<dyn HwDecoder>> = if stream_info.codec == CodecId::H264 {
                if let Ok(mut dec) = VaapiH264Decoder::new() {
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
                            let _ = dec.submit_packet(&pkt);
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
                            let _ = dec.submit_packet(&pkt);
                        }
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
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
                            let _ = dec.submit_packet(&pkt);
                        }
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
                            let _ = dec.submit_packet(&pkt);
                        }
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
                            let _ = dec.submit_packet(&pkt);
                        }
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
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
                    if let poppy_core::codec::CodecConfig::Av1(av1c) = &stream_info.config {
                        let _ = dec.set_extradata(&av1c.config_obus);
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
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
                    if let poppy_core::codec::CodecConfig::Vp9(vpcc) = &stream_info.config {
                        let _ = dec.set_extradata(&vpcc.raw);
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(target_os = "windows")]
            let decoder: Option<Box<dyn HwDecoder>> = if stream_info.codec == CodecId::H264
                || stream_info.codec == CodecId::Hevc
                || stream_info.codec == CodecId::Vp9
            {
                if let Ok(mut dec) = poppy_hw::D3D12VideoDecoder::new(stream_info.codec) {
                    match &stream_info.config {
                        poppy_core::codec::CodecConfig::H264(avcc) => {
                            let _ = dec.set_extradata(&avcc.raw);
                        }
                        poppy_core::codec::CodecConfig::Hevc(hvcc) => {
                            let _ = dec.set_extradata(&hvcc.raw);
                        }
                        poppy_core::codec::CodecConfig::Vp9(vpcc) => {
                            let _ = dec.set_extradata(&vpcc.raw);
                        }
                        _ => {}
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(target_os = "macos")]
            let decoder: Option<Box<dyn HwDecoder>> = if stream_info.codec == CodecId::H264
                || stream_info.codec == CodecId::Hevc
                || stream_info.codec == CodecId::Vp9
            {
                if let Ok(mut dec) = poppy_hw::VideoToolboxDecoder::new(stream_info.codec) {
                    match &stream_info.config {
                        poppy_core::codec::CodecConfig::H264(avcc) => {
                            let _ = dec.set_extradata(&avcc.raw);
                        }
                        poppy_core::codec::CodecConfig::Hevc(hvcc) => {
                            let _ = dec.set_extradata(&hvcc.raw);
                        }
                        poppy_core::codec::CodecConfig::Vp9(vpcc) => {
                            let _ = dec.set_extradata(&vpcc.raw);
                        }
                        _ => {}
                    }
                    Some(Box::new(dec) as Box<dyn HwDecoder>)
                } else {
                    None
                }
            } else {
                None
            };

            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
            let decoder: Option<Box<dyn HwDecoder>> = None;
            decoder
        };

        Ok(Self {
            demuxer,
            decoder,
            target_stream_index,
            codec,
            config,
            is_eos: false,
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

            if self.is_eos {
                return Ok(None);
            }

            // Demux next packet
            match self.demuxer.read_packet()? {
                Some(pkt) => {
                    if pkt.stream_index == target_stream_index {
                        let pkt = filter_to_annex_b(codec, config, pkt)?;
                        decoder.submit_packet(&pkt)?;
                    }
                }
                None => {
                    self.is_eos = true;
                    // Flush decoder at end-of-stream
                    decoder.flush()?;
                }
            }
        }
    }

    /// Seek to the nearest keyframe at or before `timestamp_ticks`.
    pub fn seek(&mut self, timestamp_ticks: i64) -> Result<(), PoppyError> {
        self.is_eos = false;
        self.demuxer
            .seek_to_keyframe(self.target_stream_index, timestamp_ticks)?;
        if let Some(d) = &mut self.decoder {
            d.reset()?;
        }
        Ok(())
    }

    /// Seek to the nearest keyframe at or before `target_seconds`.
    pub fn seek_seconds(&mut self, target_seconds: f64) -> Result<(), PoppyError> {
        let stream = &self.demuxer.streams()[self.target_stream_index];
        let ticks =
            (target_seconds * stream.time_base.den as f64 / stream.time_base.num as f64) as i64;
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
}

/// Internal helper to transform container length-prefixed NALs to Annex-B format.
fn filter_to_annex_b(
    codec: CodecId,
    config: &CodecConfig,
    pkt: Packet,
) -> Result<Packet, PoppyError> {
    match (codec, config) {
        (CodecId::H264, CodecConfig::H264(avcc)) => {
            let mut out = Vec::new();
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
            let mut out = Vec::new();
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
