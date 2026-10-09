//! Main MP4 (ISOBMFF) demuxer implementation.

use std::fs::File;
use std::path::Path;

use bytes::Bytes;
use poppy_core::{Packet, StreamInfo};

use super::moov::{BoxIter, ParsedTrack, parse_moov};
use crate::{DemuxError, Demuxer};

/// Backing storage for media data.
enum DemuxSource {
    Mmap(memmap2::Mmap),
    Bytes(Bytes),
}

impl DemuxSource {
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mmap(m) => &m[..],
            Self::Bytes(b) => &b[..],
        }
    }

    #[inline]
    pub fn extract(&self, start: usize, len: usize) -> Bytes {
        match self {
            Self::Mmap(m) => Bytes::copy_from_slice(&m[start..start + len]),
            Self::Bytes(b) => b.slice(start..start + len),
        }
    }
}

/// An ISOBMFF / MP4 container demuxer.
pub struct Mp4Demuxer {
    source: DemuxSource,
    tracks: Vec<ParsedTrack>,
    stream_infos: Vec<StreamInfo>,
    read_indices: Vec<usize>,
}

impl Mp4Demuxer {
    /// Opens an MP4 file from disk using memory-mapping.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DemuxError> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Self::init(DemuxSource::Mmap(mmap))
    }

    /// Creates an MP4 demuxer from an in-memory `Bytes` buffer (zero-copy packet extraction).
    pub fn from_bytes(bytes: Bytes) -> Result<Self, DemuxError> {
        Self::init(DemuxSource::Bytes(bytes))
    }

    fn init(source: DemuxSource) -> Result<Self, DemuxError> {
        let slice = source.as_slice();

        // 1. Locate the `moov` box across top-level boxes
        let mut moov_data = None;
        let mut moov_offset = 0u64;

        for res in BoxIter::new(slice, 0) {
            let (header, body, offset) = res?;
            if &header.box_type == b"moov" {
                moov_data = Some(body);
                moov_offset = offset + header.header_size as u64;
                break;
            }
        }

        let moov_body = match moov_data {
            Some(b) => b,
            None => {
                return Err(DemuxError::BoxNotFound {
                    fourcc: "moov".into(),
                });
            }
        };

        // 2. Parse tracks from `moov`
        let mut tracks = parse_moov(moov_body, moov_offset)?;

        // 3. Scan for movie fragments (`moof`) across the container
        for res in BoxIter::new(slice, 0) {
            let (header, body, offset) = res?;
            if &header.box_type == b"moof" {
                let body_offset = offset + header.header_size as u64;
                super::fragment::parse_moof(body, body_offset, &mut tracks)?;
            }
        }

        // 4. Update track durations based on decoded samples
        for t in &mut tracks {
            if let Some(last) = t.sample_table.samples.last() {
                let total_ticks = last.dts + last.duration as i64;
                if t.duration_ticks.map(|d| total_ticks > d).unwrap_or(true) {
                    t.duration_ticks = Some(total_ticks);
                }
            }
        }

        let stream_infos: Vec<StreamInfo> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| t.to_stream_info(i))
            .collect();
        let read_indices = vec![0; tracks.len()];

        Ok(Self {
            source,
            tracks,
            stream_infos,
            read_indices,
        })
    }

    /// Access internal parsed track metadata.
    pub fn tracks(&self) -> &[ParsedTrack] {
        &self.tracks
    }
}

impl Demuxer for Mp4Demuxer {
    fn streams(&self) -> &[StreamInfo] {
        &self.stream_infos
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError> {
        let mut best_track_idx = None;
        let mut best_dts_sec = f64::MAX;

        // Find track whose next packet has the lowest DTS in seconds
        for (idx, track) in self.tracks.iter().enumerate() {
            let cur_sample_idx = self.read_indices[idx];
            if cur_sample_idx < track.sample_table.samples.len() {
                let s = &track.sample_table.samples[cur_sample_idx];
                let dts_sec = track.time_base.to_seconds(s.dts);
                if dts_sec < best_dts_sec {
                    best_dts_sec = dts_sec;
                    best_track_idx = Some(idx);
                }
            }
        }

        let track_idx = match best_track_idx {
            Some(idx) => idx,
            None => return Ok(None), // End of stream across all tracks
        };

        let sample_idx = self.read_indices[track_idx];
        let sample = &self.tracks[track_idx].sample_table.samples[sample_idx];

        let slice_len = self.source.as_slice().len();
        let start = sample.file_offset as usize;
        let end = start + sample.size as usize;

        if end > slice_len {
            return Err(DemuxError::UnexpectedEof {
                offset: sample.file_offset,
            });
        }

        let data = self.source.extract(start, sample.size as usize);
        self.read_indices[track_idx] += 1;

        Ok(Some(Packet {
            stream_index: track_idx,
            pts: sample.pts,
            dts: sample.dts,
            duration: sample.duration,
            is_keyframe: sample.is_keyframe,
            data,
        }))
    }

    fn seek_to_keyframe(&mut self, stream_index: usize, timestamp: i64) -> Result<(), DemuxError> {
        if stream_index >= self.tracks.len() {
            return Err(DemuxError::SampleOutOfRange {
                index: stream_index as u32,
                total: self.tracks.len() as u32,
            });
        }

        let track = &self.tracks[stream_index];
        let (target_sample_idx, target_pts_sec) =
            match track.sample_table.find_keyframe_at_or_before(timestamp) {
                Some(sample) => {
                    let ts = sample.pts.unwrap_or(sample.dts);
                    (sample.index as usize, track.time_base.to_seconds(ts))
                }
                None => (0, 0.0),
            };

        self.read_indices[stream_index] = target_sample_idx;

        // Synchronize other tracks close to target_pts_sec
        for (i, other_track) in self.tracks.iter().enumerate() {
            if i == stream_index {
                continue;
            }

            let mut best_idx = 0;
            for s in &other_track.sample_table.samples {
                let ts = s.pts.unwrap_or(s.dts);
                let sec = other_track.time_base.to_seconds(ts);
                if sec <= target_pts_sec {
                    best_idx = s.index as usize;
                } else {
                    break;
                }
            }
            self.read_indices[i] = best_idx;
        }

        Ok(())
    }
}
