//! High-level media stream pipeline.
//!
//! Provides a clean, typed stream that demuxes containers and feeds frames directly
//! into hardware decoding.

use std::path::Path;

use poppy_core::{codec::CodecConfig, CodecId, Packet, StreamInfo};
use poppy_demux::{Demuxer, MatroskaDemuxer, Mp4Demuxer};
use poppy_hw::{DecodedFrame, HwDecoder};
#[cfg(target_os = "linux")]
use poppy_hw::{VaapiH264Decoder, VaapiHevcDecoder};

use crate::error::PoppyError;

/// High-level media stream representing an open media file.
pub struct MediaStream {
    demuxer: Box<dyn Demuxer>,
    decoder: Option<Box<dyn HwDecoder>>,
    target_stream_index: usize,
    codec: CodecId,
    config: CodecConfig,
}

impl MediaStream {
    /// Open a media file with automatic container format and hardware decoder detection.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, PoppyError> {
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

        // Find primary video track
        let mut target_stream_index = 0;
        for (i, s) in demuxer.streams().iter().enumerate() {
            if matches!(s.codec, CodecId::H264 | CodecId::Hevc | CodecId::Av1) {
                target_stream_index = i;
                break;
            }
        }

        let stream_info = &demuxer.streams()[target_stream_index];
        let codec = stream_info.codec;
        let config = stream_info.config.clone();

        // Attempt hardware decoder creation for primary video track
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
            let is_10bit = if let poppy_core::codec::CodecConfig::Hevc(hvcc) = &stream_info.config {
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
        } else {
            None
        };

        #[cfg(not(target_os = "linux"))]
        let decoder: Option<Box<dyn HwDecoder>> = None;

        Ok(Self {
            demuxer,
            decoder,
            target_stream_index,
            codec,
            config,
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

            // Demux next packet
            match self.demuxer.read_packet()? {
                Some(pkt) => {
                    if pkt.stream_index == target_stream_index {
                        let pkt = filter_to_annex_b(codec, config, pkt)?;
                        decoder.submit_packet(&pkt)?;
                    }
                }
                None => {
                    // Flush decoder at end-of-stream
                    decoder.flush()?;
                    return Ok(decoder.receive_frame()?);
                }
            }
        }
    }

    /// Seek to the nearest keyframe at or before `timestamp_ticks`.
    pub fn seek(&mut self, timestamp_ticks: i64) -> Result<(), PoppyError> {
        self.demuxer
            .seek_to_keyframe(self.target_stream_index, timestamp_ticks)?;
        if let Some(d) = &mut self.decoder {
            d.flush()?;
        }
        Ok(())
    }
}

/// Internal helper to transform container length-prefixed NALs to Annex-B format.
fn filter_to_annex_b(codec: CodecId, config: &CodecConfig, pkt: Packet) -> Result<Packet, PoppyError> {
    if pkt.data.starts_with(&[0, 0, 0, 1]) || pkt.data.starts_with(&[0, 0, 1]) {
        return Ok(pkt);
    }

    match (codec, config) {
        (CodecId::H264, CodecConfig::H264(avcc)) => {
            let mut out = Vec::new();
            poppy_demux::bsf::avcc_to_annex_b(
                &pkt.data,
                avcc.nal_length_size,
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
