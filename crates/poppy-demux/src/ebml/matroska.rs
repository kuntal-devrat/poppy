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
pub const ID_CUES: u32 = 0x1C53BB6B;
pub const ID_CUE_POINT: u32 = 0xBB;
pub const ID_CUE_TIME: u32 = 0xB3;
pub const ID_CUE_TRACK_POSITIONS: u32 = 0xB7;
pub const ID_CUE_TRACK: u32 = 0xF7;
pub const ID_CUE_CLUSTER_POSITION: u32 = 0xF1;

/// A parsed Matroska CuePoint for O(log N) seeking.
#[derive(Debug, Clone)]
pub struct MatroskaCue {
    pub time_ticks: u64,
    pub track_num: u64,
    pub cluster_position: u64,
}

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
    cues: Vec<MatroskaCue>,
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
        let mut cues = Vec::new();

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
                ID_CUES => {
                    for cue_res in EbmlIter::new(elem.payload, elem.offset) {
                        let cue_elem = cue_res?;
                        if cue_elem.id == ID_CUE_POINT {
                            let mut cue_time = 0u64;
                            let mut cue_track = 1u64;
                            let mut cluster_pos = 0u64;

                            for cp_res in EbmlIter::new(cue_elem.payload, cue_elem.offset) {
                                let cp_elem = cp_res?;
                                match cp_elem.id {
                                    ID_CUE_TIME => cue_time = read_uint(cp_elem.payload),
                                    ID_CUE_TRACK_POSITIONS => {
                                        for pos_res in EbmlIter::new(cp_elem.payload, cp_elem.offset) {
                                            let pos_elem = pos_res?;
                                            match pos_elem.id {
                                                ID_CUE_TRACK => cue_track = read_uint(pos_elem.payload),
                                                ID_CUE_CLUSTER_POSITION => {
                                                    cluster_pos = read_uint(pos_elem.payload);
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            cues.push(MatroskaCue {
                                time_ticks: cue_time,
                                track_num: cue_track,
                                cluster_position: cluster_pos,
                            });
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
            cues,
            cursor: 0,
        })
    }

    /// Access parsed Matroska cues (seek points).
    pub fn cues(&self) -> &[MatroskaCue] {
        &self.cues
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

        // Fast O(log N) binary search: find first sample with timestamp > requested timestamp
        let upper = self.samples.partition_point(|s| s.timestamp_ticks <= timestamp);
        let mut target_idx = 0;

        // Scan backwards from upper bound to find nearest keyframe for this track
        for i in (0..upper).rev() {
            if self.samples[i].track_num == track_num && self.samples[i].is_keyframe {
                target_idx = i;
                break;
            }
        }

        self.cursor = target_idx;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ebml_master(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        if id > 0xFFFFFF {
            buf.extend_from_slice(&id.to_be_bytes());
        } else if id > 0xFFFF {
            buf.extend_from_slice(&id.to_be_bytes()[1..]);
        } else if id > 0xFF {
            buf.extend_from_slice(&id.to_be_bytes()[2..]);
        } else {
            buf.push(id as u8);
        }
        if payload.len() < 127 {
            buf.push(0x80 | (payload.len() as u8));
        } else {
            buf.push(0x40 | ((payload.len() >> 8) as u8));
            buf.push((payload.len() & 0xFF) as u8);
        }
        buf.extend_from_slice(payload);
        buf
    }

    #[test]
    fn test_matroska_cues_and_seeking() {
        let ebml_hdr = make_ebml_master(ID_EBML_HEADER, &[]);

        let track_entry = make_ebml_master(ID_TRACK_ENTRY, &[
            // TrackNumber = 1
            0xD7, 0x81, 0x01,
            // TrackType = 1 (video)
            0x83, 0x81, 0x01,
            // CodecID = "V_MPEG4/ISO/AVC"
            0x86, 0x8F, b'V', b'_', b'M', b'P', b'E', b'G', b'4', b'/', b'I', b'S', b'O', b'/', b'A', b'V', b'C',
        ]);
        let tracks_elem = make_ebml_master(ID_TRACKS, &track_entry);

        // Cluster 0 at time 0
        let mut cluster_payload = Vec::new();
        cluster_payload.extend_from_slice(&[0xE7, 0x81, 0x00]);
        // SimpleBlock: track 1, rel_time 0, keyframe
        cluster_payload.extend_from_slice(&[0xA3, 0x85, 0x81, 0x00, 0x00, 0x80, 0xAA]);
        // SimpleBlock: track 1, rel_time 1000, non-keyframe
        cluster_payload.extend_from_slice(&[0xA3, 0x85, 0x81, 0x03, 0xE8, 0x00, 0xBB]);
        let cluster_elem = make_ebml_master(ID_CLUSTER, &cluster_payload);

        // Cues: CuePoint time 0
        let cue_track_pos = make_ebml_master(ID_CUE_TRACK_POSITIONS, &[
            0xF7, 0x81, 0x01, // CueTrack = 1
            0xF1, 0x81, 0x50, // CueClusterPosition = 0x50
        ]);
        let mut cue_point_payload = Vec::new();
        cue_point_payload.extend_from_slice(&[0xB3, 0x81, 0x00]); // CueTime = 0
        cue_point_payload.extend_from_slice(&cue_track_pos);
        let cue_point = make_ebml_master(ID_CUE_POINT, &cue_point_payload);
        let cues_elem = make_ebml_master(ID_CUES, &cue_point);

        let mut seg_payload = Vec::new();
        seg_payload.extend_from_slice(&tracks_elem);
        seg_payload.extend_from_slice(&cluster_elem);
        seg_payload.extend_from_slice(&cues_elem);
        let seg_elem = make_ebml_master(ID_SEGMENT, &seg_payload);

        let mut mkv_bytes = Vec::new();
        mkv_bytes.extend_from_slice(&ebml_hdr);
        mkv_bytes.extend_from_slice(&seg_elem);

        let mut demuxer = MatroskaDemuxer::from_bytes(Bytes::from(mkv_bytes)).expect("Failed to demux MKV");
        assert_eq!(demuxer.cues().len(), 1);
        assert_eq!(demuxer.cues()[0].time_ticks, 0);

        let pkt0 = demuxer.read_packet().unwrap().unwrap();
        assert_eq!(pkt0.dts, 0);
        assert!(pkt0.is_keyframe);

        let pkt1 = demuxer.read_packet().unwrap().unwrap();
        assert_eq!(pkt1.dts, 1000);
        assert!(!pkt1.is_keyframe);

        // Fast seeking back to keyframe
        demuxer.seek_to_keyframe(0, 500).unwrap();
        let pkt_seek = demuxer.read_packet().unwrap().unwrap();
        assert_eq!(pkt_seek.dts, 0);
        assert!(pkt_seek.is_keyframe);
    }
}
