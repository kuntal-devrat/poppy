//! Sample table (stbl) parser for ISOBMFF containers.
//!
//! Reconstructs exact sample offsets, sizes, timestamps (DTS/PTS), and sync flags
//! from `stts`, `ctts`, `stsc`, `stsz`, `stco`, `co64`, and `stss` boxes according
//! to ISO 14496-12 §8.5 - §8.8.

use std::collections::BTreeSet;

use crate::DemuxError;

/// A fully resolved media sample ready for packet extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleInfo {
    /// 0-indexed sample index within the track.
    pub index: u32,
    /// Absolute byte offset in the container file where sample data starts.
    pub file_offset: u64,
    /// Byte length of the compressed sample data.
    pub size: u32,
    /// Decode Timestamp (DTS) in units of the stream timebase.
    pub dts: i64,
    /// Presentation Timestamp (PTS) in units of the stream timebase.
    pub pts: Option<i64>,
    /// Duration of this sample in timebase units.
    pub duration: u32,
    /// True if this is a random access sync point (IDR/keyframe).
    pub is_keyframe: bool,
}

/// Raw parsed data from `stts` (Decoding Time to Sample).
#[derive(Debug, Clone, Default)]
pub struct SttsTable {
    pub entries: Vec<(u32, u32)>, // (sample_count, sample_delta)
}

impl SttsTable {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stts payload too short for header and entry count".into(),
            });
        }
        // version (1) + flags (3)
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stts payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let count = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]);
            let delta = u32::from_be_bytes([payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7]]);
            entries.push((count, delta));
            offset += 8;
        }

        Ok(Self { entries })
    }

    /// Computes (dts, duration) for each sample.
    pub fn compute_dts(&self, total_samples: usize) -> Vec<(i64, u32)> {
        let mut res = Vec::with_capacity(total_samples);
        let mut current_dts = 0i64;
        let mut count_so_far = 0;

        for &(count, delta) in &self.entries {
            for _ in 0..count {
                if count_so_far >= total_samples {
                    return res;
                }
                res.push((current_dts, delta));
                current_dts += delta as i64;
                count_so_far += 1;
            }
        }

        // If entries ran out before total_samples, fill with 0 delta
        while res.len() < total_samples {
            res.push((current_dts, 0));
        }

        res
    }
}

/// Raw parsed data from `ctts` (Composition Time to Sample).
#[derive(Debug, Clone, Default)]
pub struct CttsTable {
    pub entries: Vec<(u32, i64)>, // (sample_count, sample_offset)
}

impl CttsTable {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "ctts payload too short for header and entry count".into(),
            });
        }
        let version = payload[0];
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("ctts payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let count = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]);
            let sample_offset = if version == 0 {
                u32::from_be_bytes([payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7]]) as i64
            } else {
                i32::from_be_bytes([payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7]]) as i64
            };
            entries.push((count, sample_offset));
            offset += 8;
        }

        Ok(Self { entries })
    }

    /// Computes PTS offsets for each sample.
    pub fn compute_pts_offsets(&self, total_samples: usize) -> Vec<i64> {
        let mut res = Vec::with_capacity(total_samples);
        for &(count, offset) in &self.entries {
            for _ in 0..count {
                if res.len() >= total_samples {
                    return res;
                }
                res.push(offset);
            }
        }
        while res.len() < total_samples {
            res.push(0);
        }
        res
    }
}

/// Raw parsed data from `stss` (Sync Sample Box).
#[derive(Debug, Clone, Default)]
pub struct StssTable {
    /// 1-based sample numbers that are sync samples.
    pub sync_samples: BTreeSet<u32>,
}

impl StssTable {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stss payload too short".into(),
            });
        }
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(4);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stss payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut sync_samples = BTreeSet::new();
        let mut offset = 8;
        for _ in 0..entry_count {
            let sample_num = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]);
            sync_samples.insert(sample_num);
            offset += 4;
        }

        Ok(Self { sync_samples })
    }

    /// Check if 0-indexed sample is a keyframe.
    pub fn is_keyframe(&self, sample_idx_0_based: u32) -> bool {
        self.sync_samples.contains(&(sample_idx_0_based + 1))
    }
}

/// Raw parsed data from `stsz` (Sample Size Box).
#[derive(Debug, Clone, Default)]
pub struct StszTable {
    pub default_sample_size: u32,
    pub sample_sizes: Vec<u32>,
}

impl StszTable {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 12 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stsz payload too short".into(),
            });
        }
        let default_sample_size = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let sample_count = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;

        let sample_sizes = if default_sample_size == 0 {
            let expected_len = 12 + sample_count.saturating_mul(4);
            if payload.len() < expected_len {
                return Err(DemuxError::InvalidBox {
                    offset: 0,
                    detail: format!("stsz payload too short: expected {expected_len}, got {}", payload.len()),
                });
            }
            let mut sizes = Vec::with_capacity(sample_count);
            let mut offset = 12;
            for _ in 0..sample_count {
                let sz = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]);
                sizes.push(sz);
                offset += 4;
            }
            sizes
        } else {
            vec![default_sample_size; sample_count]
        };

        Ok(Self {
            default_sample_size,
            sample_sizes,
        })
    }

    pub fn total_samples(&self) -> usize {
        self.sample_sizes.len()
    }
}

/// Raw parsed data from `stsc` (Sample-to-Chunk Box).
#[derive(Debug, Clone, Default)]
pub struct StscEntry {
    pub first_chunk: u32, // 1-based
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct StscTable {
    pub entries: Vec<StscEntry>,
}

impl StscTable {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stsc payload too short".into(),
            });
        }
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(12);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stsc payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let first_chunk = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]);
            let samples_per_chunk = u32::from_be_bytes([payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7]]);
            let sample_description_index = u32::from_be_bytes([payload[offset + 8], payload[offset + 9], payload[offset + 10], payload[offset + 11]]);
            entries.push(StscEntry {
                first_chunk,
                samples_per_chunk,
                sample_description_index,
            });
            offset += 12;
        }

        Ok(Self { entries })
    }
}

/// Chunk offsets table from `stco` (32-bit) or `co64` (64-bit).
#[derive(Debug, Clone, Default)]
pub struct ChunkOffsets {
    pub offsets: Vec<u64>,
}

impl ChunkOffsets {
    pub fn parse_stco(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stco payload too short".into(),
            });
        }
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(4);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stco payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut offsets = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let chunk_off = u32::from_be_bytes([payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3]]) as u64;
            offsets.push(chunk_off);
            offset += 4;
        }

        Ok(Self { offsets })
    }

    pub fn parse_co64(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "co64 payload too short".into(),
            });
        }
        let entry_count = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("co64 payload too short: expected {expected_len}, got {}", payload.len()),
            });
        }

        let mut offsets = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let chunk_off = u64::from_be_bytes([
                payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
                payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7],
            ]);
            offsets.push(chunk_off);
            offset += 8;
        }

        Ok(Self { offsets })
    }
}

/// The complete resolved sample table for an ISOBMFF track.
#[derive(Debug, Clone, Default)]
pub struct SampleTable {
    pub samples: Vec<SampleInfo>,
}

impl SampleTable {
    /// Reconstructs all sample metadata by joining `stsz`, `stsc`, `stco`/`co64`,
    /// `stts`, `ctts`, and `stss`.
    pub fn build(
        stsz: &StszTable,
        stsc: &StscTable,
        chunk_offsets: &ChunkOffsets,
        stts: &SttsTable,
        ctts: Option<&CttsTable>,
        stss: Option<&StssTable>,
    ) -> Result<Self, DemuxError> {
        let total_samples = stsz.total_samples();
        if total_samples == 0 {
            return Ok(Self {
                samples: Vec::new(),
            });
        }

        let dts_durations = stts.compute_dts(total_samples);
        let pts_offsets = ctts.map(|c| c.compute_pts_offsets(total_samples));

        // Reconstruct chunk sample counts
        let total_chunks = chunk_offsets.offsets.len();
        let mut chunk_sample_counts = Vec::with_capacity(total_chunks);

        let mut stsc_idx = 0;
        for chunk_num in 1..=total_chunks as u32 {
            while stsc_idx + 1 < stsc.entries.len() && stsc.entries[stsc_idx + 1].first_chunk <= chunk_num {
                stsc_idx += 1;
            }
            let spc = if stsc_idx < stsc.entries.len() {
                stsc.entries[stsc_idx].samples_per_chunk
            } else {
                0
            };
            chunk_sample_counts.push(spc);
        }

        let mut samples = Vec::with_capacity(total_samples);
        let mut current_sample_idx = 0;

        for (chunk_idx, &chunk_offset) in chunk_offsets.offsets.iter().enumerate() {
            let samples_in_chunk = chunk_sample_counts.get(chunk_idx).copied().unwrap_or(0);
            let mut sample_offset_in_chunk = 0u64;

            for _ in 0..samples_in_chunk {
                if current_sample_idx >= total_samples {
                    break;
                }

                let size = stsz.sample_sizes[current_sample_idx];
                let file_offset = chunk_offset + sample_offset_in_chunk;
                let (dts, duration) = dts_durations[current_sample_idx];

                let pts = pts_offsets.as_ref().map(|offsets| dts + offsets[current_sample_idx]);

                // ISO 14496-12 §8.6.2: If stss is absent, EVERY sample is a sync sample
                let is_keyframe = match stss {
                    Some(s) => s.is_keyframe(current_sample_idx as u32),
                    None => true,
                };

                samples.push(SampleInfo {
                    index: current_sample_idx as u32,
                    file_offset,
                    size,
                    dts,
                    pts,
                    duration,
                    is_keyframe,
                });

                sample_offset_in_chunk += size as u64;
                current_sample_idx += 1;
            }

            if current_sample_idx >= total_samples {
                break;
            }
        }

        Ok(Self { samples })
    }

    /// Finds the nearest keyframe at or before `timestamp` (in timebase ticks).
    pub fn find_keyframe_at_or_before(&self, timestamp: i64) -> Option<&SampleInfo> {
        let mut candidate = None;
        for s in &self.samples {
            let ts = s.pts.unwrap_or(s.dts);
            if ts <= timestamp {
                if s.is_keyframe {
                    candidate = Some(s);
                }
            } else {
                break;
            }
        }
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_table_reconstruction() {
        // Mock stsz: 4 samples with sizes [100, 200, 150, 300]
        let stsz = StszTable {
            default_sample_size: 0,
            sample_sizes: vec![100, 200, 150, 300],
        };

        // Mock stsc: 2 samples per chunk
        let stsc = StscTable {
            entries: vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 2,
                sample_description_index: 1,
            }],
        };

        // Mock stco: chunk 1 at 1000, chunk 2 at 5000
        let chunk_offsets = ChunkOffsets {
            offsets: vec![1000, 5000],
        };

        // Mock stts: all samples duration 1000
        let stts = SttsTable {
            entries: vec![(4, 1000)],
        };

        // Mock ctts: sample 0 pts offset 0, sample 1 offset 2000, etc.
        let ctts = CttsTable {
            entries: vec![(1, 0), (1, 2000), (2, 1000)],
        };

        // Mock stss: sample 1 and 3 (1-based) are keyframes => 0 and 2 (0-based)
        let mut sync_set = BTreeSet::new();
        sync_set.insert(1);
        sync_set.insert(3);
        let stss = StssTable {
            sync_samples: sync_set,
        };

        let table = SampleTable::build(
            &stsz,
            &stsc,
            &chunk_offsets,
            &stts,
            Some(&ctts),
            Some(&stss),
        ).unwrap();

        assert_eq!(table.samples.len(), 4);

        // Sample 0: chunk 1, off 1000, size 100, dts 0, pts 0, keyframe true
        assert_eq!(table.samples[0].file_offset, 1000);
        assert_eq!(table.samples[0].size, 100);
        assert_eq!(table.samples[0].dts, 0);
        assert_eq!(table.samples[0].pts, Some(0));
        assert!(table.samples[0].is_keyframe);

        // Sample 1: chunk 1, off 1000 + 100 = 1100, size 200, dts 1000, pts 3000, keyframe false
        assert_eq!(table.samples[1].file_offset, 1100);
        assert_eq!(table.samples[1].size, 200);
        assert_eq!(table.samples[1].dts, 1000);
        assert_eq!(table.samples[1].pts, Some(3000));
        assert!(!table.samples[1].is_keyframe);

        // Sample 2: chunk 2, off 5000, size 150, dts 2000, pts 3000, keyframe true
        assert_eq!(table.samples[2].file_offset, 5000);
        assert_eq!(table.samples[2].size, 150);
        assert_eq!(table.samples[2].dts, 2000);
        assert_eq!(table.samples[2].pts, Some(3000));
        assert!(table.samples[2].is_keyframe);

        // Sample 3: chunk 2, off 5000 + 150 = 5150, size 300, dts 3000, pts 4000, keyframe false
        assert_eq!(table.samples[3].file_offset, 5150);
        assert_eq!(table.samples[3].size, 300);
        assert_eq!(table.samples[3].dts, 3000);
        assert_eq!(table.samples[3].pts, Some(4000));
        assert!(!table.samples[3].is_keyframe);

        // Seeking test: seek to pts 2500 -> candidate is sample 0 (pts 0) or sample 2 (pts 3000 is > 2500)
        let kf = table.find_keyframe_at_or_before(2500).unwrap();
        assert_eq!(kf.index, 0);

        let kf2 = table.find_keyframe_at_or_before(3500).unwrap();
        assert_eq!(kf2.index, 2);
    }
}
