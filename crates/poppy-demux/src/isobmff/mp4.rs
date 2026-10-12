//! Main MP4 (ISOBMFF) demuxer implementation.

use std::fs::File;
use std::path::Path;

use bytes::Bytes;
use poppy_core::{Packet, StreamInfo};

use super::moov::{BoxIter, ParsedTrack, parse_moov};
use crate::{DemuxError, Demuxer};

/// Backing storage for media data.
enum DemuxSource {
    Mmap(Bytes),
    Bytes(Bytes),
}

impl DemuxSource {
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Mmap(b) => &b[..],
            Self::Bytes(b) => &b[..],
        }
    }

    #[inline]
    pub fn extract(&self, start: usize, len: usize) -> Bytes {
        match self {
            // Zero-copy extraction for both mmap and in-memory sources:
            // Slices the underlying ref-counted buffer without copying payload bytes.
            Self::Mmap(b) => b.slice(start..start + len),
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
    /// DASH segment indexes (`sidx`) in file order.
    sidx_segments: Vec<super::moov::SidxSegment>,
    /// Fragment random access (`mfra/tfra`) entries.
    mfra_entries: Vec<super::moov::MfraEntry>,
}

impl Mp4Demuxer {
    /// Opens an MP4 file from disk using memory-mapping with zero-copy packet extraction.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DemuxError> {
        let file = File::open(path)?;
        // SAFETY: memmap2::Mmap::map is unsafe because the file could be
        // concurrently truncated/remapped by another process, which would
        // cause SIGBUS on access. We own the File handle for the duration of
        // mapping and only read through the mapping; writers truncating the
        // file concurrently are out of scope (same contract as FFmpeg mmap).
        // Empty files map to an empty slice and fail with BoxNotFound below.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let bytes = Bytes::from_owner(mmap);
        Self::init(DemuxSource::Mmap(bytes))
    }

    /// Opens from any `Read + Seek` source without requiring mmap.
    /// Reads the stream in 64 KiB chunks (size probed via Seek::seek(End),
    /// capped at 2 GiB) into a `Bytes` buffer, then parses. Works on
    /// non-mmap platforms, pipes-with-seek, and in-memory cursors.
    pub fn open_streaming<R: std::io::Read + std::io::Seek>(
        mut reader: R,
    ) -> Result<Self, DemuxError> {
        const MAX_STREAMING_BYTES: u64 = 2 * 1024 * 1024 * 1024;
        let len =
            std::io::Seek::seek(&mut reader, std::io::SeekFrom::End(0)).map_err(DemuxError::Io)?;
        std::io::Seek::seek(&mut reader, std::io::SeekFrom::Start(0)).map_err(DemuxError::Io)?;
        if len > MAX_STREAMING_BYTES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("streaming source {len} bytes exceeds 2GiB cap"),
            });
        }
        let mut buf = Vec::with_capacity(len.min(1 << 20) as usize);
        let mut chunk = [0u8; 65536];
        let mut remaining = len;
        while remaining > 0 {
            let want = (remaining.min(chunk.len() as u64)) as usize;
            let n = std::io::Read::read(&mut reader, &mut chunk[..want]).map_err(DemuxError::Io)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            remaining -= n as u64;
        }
        Self::from_bytes(bytes::Bytes::from(buf))
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

        for res in BoxIter::new_top_level(slice, 0) {
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

        // 3. Scan for movie fragments (`moof`), segment index (`sidx`),
        // and fragment random access (`mfra`) across the container.
        // Pass the moof box start (not body) so default-base-is-moof resolves
        // per ISO 14496-12 §8.8.7.
        let mut sidx_segments = Vec::new();
        let mut mfra_entries = Vec::new();
        for res in BoxIter::new_top_level(slice, 0) {
            let (header, body, offset) = res?;
            if &header.box_type == b"moof" {
                super::fragment::parse_moof(body, offset, &mut tracks)?;
            } else if &header.box_type == b"sidx" {
                if let Some(seg) = super::moov::parse_sidx_pub(body) {
                    sidx_segments.push(seg);
                }
            } else if &header.box_type == b"mfra" {
                mfra_entries.extend(super::moov::parse_mfra_pub(
                    body,
                    offset + header.header_size as u64,
                ));
            }
        }

        // 4. Apply edit lists (elst / edts) to adjust sample presentation timestamps and durations
        for t in &mut tracks {
            if let Some(&(media_time, segment_duration)) = t.edit_list.first() {
                if media_time > 0 {
                    // Positive media_time: composition starts at media_time in track timescale (ISO 14496-12 §8.6.6).
                    // Shift timestamps so presentation starts at 0 (e.g. trimming audio encoder delay / priming).
                    for s in &mut t.sample_table.samples {
                        if let Some(ref mut pts) = s.pts {
                            *pts -= media_time;
                        } else {
                            s.dts -= media_time;
                        }
                    }
                } else if media_time == -1 && segment_duration > 0 {
                    // Empty edit (dwell): presentation start is delayed by segment_duration.
                    for s in &mut t.sample_table.samples {
                        if let Some(ref mut pts) = s.pts {
                            *pts += segment_duration;
                        } else {
                            s.dts += segment_duration;
                        }
                    }
                }
            }

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
            sidx_segments,
            mfra_entries,
        })
    }

    /// Access internal parsed track metadata.
    pub fn tracks(&self) -> &[ParsedTrack] {
        &self.tracks
    }

    /// DASH segment indexes (`sidx`) in file order. Empty for progressive files.
    pub fn sidx_segments(&self) -> &[super::moov::SidxSegment] {
        &self.sidx_segments
    }

    /// Fragment random access (`mfra/tfra`) entries. Empty when absent.
    pub fn mfra_entries(&self) -> &[super::moov::MfraEntry] {
        &self.mfra_entries
    }
}

impl Demuxer for Mp4Demuxer {
    fn streams(&self) -> &[StreamInfo] {
        &self.stream_infos
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError> {
        // Find track whose next packet has the lowest DTS using exact integer
        // cross-multiplication (no f64 precision loss, correct across
        // timescales). Ties broken by lower track index for determinism.
        let mut best_track_idx: Option<usize> = None;

        for (idx, track) in self.tracks.iter().enumerate() {
            let cur_sample_idx = self.read_indices[idx];
            if cur_sample_idx >= track.sample_table.samples.len() {
                continue;
            }
            let is_better = match best_track_idx {
                None => true,
                Some(best) => {
                    let s = &track.sample_table.samples[cur_sample_idx];
                    let b = &self.tracks[best].sample_table.samples[self.read_indices[best]];
                    track
                        .time_base
                        .compare(s.dts, self.tracks[best].time_base, b.dts)
                        == std::cmp::Ordering::Less
                }
            };
            if is_better {
                best_track_idx = Some(idx);
            }
        }

        let track_idx = match best_track_idx {
            Some(idx) => idx,
            None => return Ok(None), // End of stream across all tracks
        };

        let sample_idx = self.read_indices[track_idx];
        let sample = &self.tracks[track_idx].sample_table.samples[sample_idx];

        let slice_len = self.source.as_slice().len();
        let start: usize =
            usize::try_from(sample.file_offset).map_err(|_| DemuxError::InvalidBox {
                offset: sample.file_offset,
                detail: "sample file_offset does not fit in usize".into(),
            })?;
        let size: usize = sample.size as usize;
        let end = start
            .checked_add(size)
            .ok_or_else(|| DemuxError::InvalidBox {
                offset: sample.file_offset,
                detail: "sample end offset overflows".into(),
            })?;

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

        let target_pts = {
            let track = &self.tracks[stream_index];
            match track.sample_table.find_keyframe_at_or_before(timestamp) {
                Some(sample) => sample.pts.unwrap_or(sample.dts),
                None => {
                    self.read_indices[stream_index] = 0;
                    // Still sync other tracks to start.
                    for (i, _) in self.tracks.iter().enumerate() {
                        if i != stream_index {
                            self.read_indices[i] = 0;
                        }
                    }
                    return Ok(());
                }
            }
        };
        let target_tb = self.tracks[stream_index].time_base;
        // Convert target PTS to each track's timebase with integer rescale.
        let target_idx = self.tracks[stream_index]
            .sample_table
            .find_keyframe_at_or_before(timestamp)
            .map(|s| s.index as usize)
            .unwrap_or(0);
        self.read_indices[stream_index] = target_idx;

        // Synchronize other tracks: pick the last sample with PTS <= target
        // (converted to that track's timebase). Uses O(log N) binary search on
        // monotonic tracks (such as audio, where scanning all samples would be O(N)).
        for i in 0..self.tracks.len() {
            if i == stream_index {
                continue;
            }
            let other_tb = self.tracks[i].time_base;
            let target_in_other = target_tb.rescale(target_pts, other_tb);
            self.read_indices[i] = self.tracks[i]
                .sample_table
                .find_sample_index_at_or_before(target_in_other);
        }

        Ok(())
    }

    fn estimated_frame_duration_seconds(&self, stream_index: usize) -> Option<f64> {
        let track = self.tracks.get(stream_index)?;
        if let Some(first_sample) = track.sample_table.samples.first() {
            if first_sample.duration > 0 {
                let sec = track.time_base.to_seconds(first_sample.duration as i64);
                if sec > 0.0 && sec.is_finite() {
                    return Some(sec);
                }
            }
        }
        if let Some(dur_ticks) = track.duration_ticks {
            let count = track.sample_table.samples.len();
            if count > 0 {
                let total_sec = track.time_base.to_seconds(dur_ticks);
                let avg = total_sec / (count as f64);
                if avg > 0.0 && avg.is_finite() {
                    return Some(avg);
                }
            }
        }
        if let Some(trex) = &track.trex {
            if trex.default_sample_duration > 0 {
                let sec = track.time_base.to_seconds(trex.default_sample_duration as i64);
                if sec > 0.0 && sec.is_finite() {
                    return Some(sec);
                }
            }
        }
        None
    }
}
