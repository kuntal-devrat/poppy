//! Matroska (MKV) and WebM container demuxer.

use std::fs::File;
use std::path::Path;

use bytes::Bytes;
use poppy_core::codec::{AvcCRecord, CodecConfig, CodecId};
use poppy_core::{Packet, StreamInfo, TimeBase};

use super::element::{read_uint, read_utf8, EbmlIter};
use super::vint::read_vint_id;
use crate::{DemuxError, Demuxer};

// Known Matroska Element IDs
pub const ID_EBML_HEADER: u32 = 0x1A45DFA3;
pub const ID_SEGMENT: u32 = 0x18538067;
pub const ID_INFO: u32 = 0x1549A966;
pub const ID_TIMESTAMP_SCALE: u32 = 0x2AD7B1;
pub const ID_DURATION: u32 = 0x4489;
pub const ID_TRACKS: u32 = 0x1654AE6B;
pub const ID_TRACK_ENTRY: u32 = 0xAE;
pub const ID_TRACK_NUMBER: u32 = 0xD7;
pub const ID_TRACK_TYPE: u32 = 0x83;
pub const ID_CODEC_ID: u32 = 0x86;
pub const ID_CODEC_PRIVATE: u32 = 0x63A2;
pub const ID_VIDEO_SETTINGS: u32 = 0xE0;
pub const ID_PIXEL_WIDTH: u32 = 0xB0;
pub const ID_PIXEL_HEIGHT: u32 = 0xBA;
pub const ID_CLUSTER: u32 = 0x1F43B675;
pub const ID_CLUSTER_TIMESTAMP: u32 = 0xE7;
pub const ID_SIMPLE_BLOCK: u32 = 0xA3;
pub const ID_BLOCK_GROUP: u32 = 0xA0;
pub const ID_BLOCK: u32 = 0xA1;

/// A parsed Matroska track.
#[derive(Debug, Clone)]
pub struct MatroskaTrack {
    pub track_num: u64,
    pub track_type: u64,
    pub codec_id_str: String,
    pub codec_id: CodecId,
    pub codec_config: CodecConfig,
    pub width: u32,
    pub height: u32,
}

/// A parsed media sample from a Matroska Cluster.
#[derive(Debug, Clone)]
pub struct MatroskaSample {
    pub track_num: u64,
    pub timestamp_ticks: i64,
    pub is_keyframe: bool,
    pub data_range: (usize, usize),
}

enum Source {
    Mmap(memmap2::Mmap),
    Bytes(Bytes),
}

impl Source {
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mmap(m) => &m[..],
            Self::Bytes(b) => &b[..],
        }
    }

    pub fn extract(&self, start: usize, len: usize) -> Bytes {
        match self {
            Self::Mmap(m) => Bytes::copy_from_slice(&m[start..start + len]),
            Self::Bytes(b) => b.slice(start..start + len),
        }
    }
}

/// Matroska (MKV / WebM) demuxer.
pub struct MatroskaDemuxer {
    source: Source,
    tracks: Vec<MatroskaTrack>,
    stream_infos: Vec<StreamInfo>,
    samples: Vec<MatroskaSample>,
    cursor: usize,
}

impl MatroskaDemuxer {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DemuxError> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Self::init(Source::Mmap(mmap))
    }

    pub fn from_bytes(bytes: Bytes) -> Result<Self, DemuxError> {
        Self::init(Source::Bytes(bytes))
    }

    fn init(source: Source) -> Result<Self, DemuxError> {
        let data = source.as_slice();

        // 1. Verify EBML header
        let mut iter = EbmlIter::new(data, 0);
        let header_elem = iter
            .next()
            .ok_or(DemuxError::UnsupportedFormat)??;
        if header_elem.id != ID_EBML_HEADER {
            return Err(DemuxError::UnsupportedFormat);
        }

        // 2. Find Segment element
        let segment_elem = iter
            .next()
            .ok_or(DemuxError::BoxNotFound {
                fourcc: "Segment".into(),
            })??;
        if segment_elem.id != ID_SEGMENT {
            return Err(DemuxError::UnsupportedFormat);
        }

        let mut timestamp_scale_ns = 1_000_000u64; // Default 1ms = 1,000,000 ns
        let mut tracks = Vec::new();
        let mut samples = Vec::new();

        // 3. Walk Segment children
        for seg_res in EbmlIter::new(segment_elem.payload, segment_elem.offset) {
            let elem = seg_res?;
            match elem.id {
                ID_INFO => {
                    for info_res in EbmlIter::new(elem.payload, elem.offset) {
                        let info_elem = info_res?;
                        if info_elem.id == ID_TIMESTAMP_SCALE {
                            timestamp_scale_ns = read_uint(info_elem.payload);
                        }
                    }
                }
                ID_TRACKS => {
                    for tracks_res in EbmlIter::new(elem.payload, elem.offset) {
                        let track_elem = tracks_res?;
                        if track_elem.id == ID_TRACK_ENTRY {
                            let mut track_num = 1;
                            let mut track_type = 1;
                            let mut codec_str = String::new();
                            let mut codec_config = CodecConfig::Unknown;
                            let mut width = 0;
                            let mut height = 0;

                            for entry_res in EbmlIter::new(track_elem.payload, track_elem.offset) {
                                let entry_elem = entry_res?;
                                match entry_elem.id {
                                    ID_TRACK_NUMBER => track_num = read_uint(entry_elem.payload),
                                    ID_TRACK_TYPE => track_type = read_uint(entry_elem.payload),
                                    ID_CODEC_ID => codec_str = read_utf8(entry_elem.payload),
                                    ID_CODEC_PRIVATE => {
                                        if let Ok(avcc) = AvcCRecord::parse(entry_elem.payload) {
                                            codec_config = CodecConfig::H264(avcc);
                                        }
                                    }
                                    ID_VIDEO_SETTINGS => {
                                        for v_res in EbmlIter::new(entry_elem.payload, entry_elem.offset) {
                                            let v_elem = v_res?;
                                            match v_elem.id {
                                                ID_PIXEL_WIDTH => width = read_uint(v_elem.payload) as u32,
                                                ID_PIXEL_HEIGHT => height = read_uint(v_elem.payload) as u32,
                                                _ => {}
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            let codec_id = CodecId::from_matroska_codec_id(&codec_str);
                            tracks.push(MatroskaTrack {
                                track_num,
                                track_type,
                                codec_id_str: codec_str,
                                codec_id,
                                codec_config,
                                width,
                                height,
                            });
                        }
                    }
                }
                ID_CLUSTER => {
                    let mut cluster_time = 0i64;
                    for cluster_res in EbmlIter::new(elem.payload, elem.offset) {
                        let c_elem = cluster_res?;
                        if c_elem.id == ID_CLUSTER_TIMESTAMP {
                            cluster_time = read_uint(c_elem.payload) as i64;
                        } else if c_elem.id == ID_SIMPLE_BLOCK {
                            // SimpleBlock layout:
                            // VINT track number
                            // i16 relative timestamp
                            // u8 flags
                            // data...
                            let block_data = c_elem.payload;
                            if let Ok((trk, trk_len)) = read_vint_id(block_data, c_elem.offset) {
                                if block_data.len() >= trk_len + 3 {
                                    let rel_time = i16::from_be_bytes([block_data[trk_len], block_data[trk_len + 1]]) as i64;
                                    let flags = block_data[trk_len + 2];
                                    let is_keyframe = (flags & 0x80) != 0;

                                    let payload_start = (c_elem.payload.as_ptr() as usize - data.as_ptr() as usize) + trk_len + 3;
                                    let payload_len = block_data.len() - (trk_len + 3);

                                    samples.push(MatroskaSample {
                                        track_num: trk as u64,
                                        timestamp_ticks: cluster_time + rel_time,
                                        is_keyframe,
                                        data_range: (payload_start, payload_len),
                                    });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Matroska timebase: timestampScale nanoseconds per tick
        // 1 tick = timestampScale / 1,000,000,000 sec
        let time_base = TimeBase::new(timestamp_scale_ns as u32, 1_000_000_000);

        let stream_infos: Vec<StreamInfo> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| StreamInfo {
                index: i,
                codec: t.codec_id,
                time_base,
                duration_ticks: None,
                config: t.codec_config.clone(),
            })
            .collect();

        Ok(Self {
            source,
            tracks,
            stream_infos,
            samples,
            cursor: 0,
        })
    }
}

impl Demuxer for MatroskaDemuxer {
    fn streams(&self) -> &[StreamInfo] {
        &self.stream_infos
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError> {
        if self.cursor >= self.samples.len() {
            return Ok(None);
        }

        let s = &self.samples[self.cursor];
        self.cursor += 1;

        let stream_index = self
            .tracks
            .iter()
            .position(|t| t.track_num == s.track_num)
            .unwrap_or(0);

        let data = self.source.extract(s.data_range.0, s.data_range.1);

        Ok(Some(Packet {
            stream_index,
            pts: Some(s.timestamp_ticks),
            dts: s.timestamp_ticks,
            duration: 0,
            is_keyframe: s.is_keyframe,
            data,
        }))
    }

    fn seek_to_keyframe(&mut self, stream_index: usize, timestamp: i64) -> Result<(), DemuxError> {
        let track_num = self.tracks.get(stream_index).map(|t| t.track_num).unwrap_or(1);
        let mut target_idx = 0;

        for (i, s) in self.samples.iter().enumerate() {
            if s.track_num == track_num {
                if s.timestamp_ticks <= timestamp {
                    if s.is_keyframe {
                        target_idx = i;
                    }
                } else {
                    break;
                }
            }
        }

        self.cursor = target_idx;
        Ok(())
    }
}
