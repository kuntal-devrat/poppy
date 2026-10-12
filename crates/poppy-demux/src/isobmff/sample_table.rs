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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 1_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stts entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stts payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let count = u32::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]);
            let delta = u32::from_be_bytes([
                payload[offset + 4],
                payload[offset + 5],
                payload[offset + 6],
                payload[offset + 7],
            ]);
            entries.push((count, delta));
            offset += 8;
        }

        Ok(Self { entries })
    }

    /// Computes (dts, duration) for each sample with overflow protection and
    /// DoS caps. Malicious `stts` with `count == u32::MAX` is rejected.
    pub fn compute_dts(&self, total_samples: usize) -> Vec<(i64, u32)> {
        const MAX_TOTAL_SAMPLES: usize = 10_000_000;
        let mut res = Vec::with_capacity(total_samples.min(MAX_TOTAL_SAMPLES));
        let mut current_dts = 0i64;
        let mut count_so_far = 0usize;

        for &(count, delta) in &self.entries {
            // Guard: a single entry claiming billions of samples would OOM.
            // Cap expansion to what is actually needed.
            let remaining = total_samples.saturating_sub(count_so_far);
            if remaining == 0 {
                break;
            }
            // count is u32; on 32-bit this loop is still bounded by `remaining`.
            let to_emit = (count as usize).min(remaining);
            // Extra safety: if entries claim more than MAX_TOTAL_SAMPLES total,
            // compute_dts is only called with total_samples already capped by
            // StszTable, so this is bounded.
            for _ in 0..to_emit {
                res.push((current_dts, delta));
                current_dts = current_dts.saturating_add(delta as i64);
                count_so_far += 1;
            }
            // If count > remaining, the extra claimed samples are ignored
            // (still advance DTS correctly for the emitted ones only).
            if (count as usize) > to_emit {
                // Advance DTS for un-emitted tail? No — DTS only matters for
                // emitted samples. Break to avoid over-counting.
                // Continue to next entry in case total_samples was
                // under-counted (handled by fill below).
                continue;
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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 1_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("ctts entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "ctts payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let count = u32::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]);
            let sample_offset = if version == 0 {
                u32::from_be_bytes([
                    payload[offset + 4],
                    payload[offset + 5],
                    payload[offset + 6],
                    payload[offset + 7],
                ]) as i64
            } else {
                i32::from_be_bytes([
                    payload[offset + 4],
                    payload[offset + 5],
                    payload[offset + 6],
                    payload[offset + 7],
                ]) as i64
            };
            entries.push((count, sample_offset));
            offset += 8;
        }

        Ok(Self { entries })
    }

    /// Computes PTS offsets for each sample with the same DoS cap as stts.
    pub fn compute_pts_offsets(&self, total_samples: usize) -> Vec<i64> {
        let mut res = Vec::with_capacity(total_samples.min(10_000_000));
        for &(count, offset) in &self.entries {
            let remaining = total_samples.saturating_sub(res.len());
            if remaining == 0 {
                break;
            }
            let to_emit = (count as usize).min(remaining);
            res.extend(std::iter::repeat_n(offset, to_emit));
            if res.len() >= total_samples {
                return res;
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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 10_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stss entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(4);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stss payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut sync_samples = BTreeSet::new();
        let mut offset = 8;
        for _ in 0..entry_count {
            let sample_num = u32::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]);
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
        let default_sample_size =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let sample_count =
            u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;

        // Security guard: limit maximum sample count to prevent OOM denial-of-service
        const MAX_SAMPLE_COUNT: usize = 10_000_000;
        if sample_count > MAX_SAMPLE_COUNT {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stsz sample_count {sample_count} exceeds safety limit ({MAX_SAMPLE_COUNT})"
                ),
            });
        }

        let sample_sizes = if default_sample_size == 0 {
            let expected_len = 12 + sample_count.saturating_mul(4);
            if payload.len() < expected_len {
                return Err(DemuxError::InvalidBox {
                    offset: 0,
                    detail: format!(
                        "stsz payload too short: expected {expected_len}, got {}",
                        payload.len()
                    ),
                });
            }
            let mut sizes = Vec::with_capacity(sample_count);
            let mut offset = 12;
            for _ in 0..sample_count {
                let sz = u32::from_be_bytes([
                    payload[offset],
                    payload[offset + 1],
                    payload[offset + 2],
                    payload[offset + 3],
                ]);
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

/// Raw parsed data from `stz2` (Compact Sample Size Box).
/// Common with encrypted content; previously ignored which caused silent data loss.
#[derive(Debug, Clone, Default)]
pub struct Stz2Table {
    pub sample_sizes: Vec<u32>,
}

impl Stz2Table {
    pub fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 12 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stz2 payload too short".into(),
            });
        }
        // version(1)+flags(3) + reserved(3)+field_size(1) + sample_count(4)
        let field_size = payload[7];
        if field_size != 4 && field_size != 8 && field_size != 16 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stz2 unsupported field_size {field_size}"),
            });
        }
        let sample_count =
            u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
        const MAX_SAMPLE_COUNT: usize = 10_000_000;
        if sample_count > MAX_SAMPLE_COUNT {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stz2 sample_count {sample_count} exceeds safety limit ({MAX_SAMPLE_COUNT})"
                ),
            });
        }
        let mut sizes = Vec::with_capacity(sample_count);
        let mut offset = 12;
        match field_size {
            16 => {
                if payload.len() < offset + sample_count.saturating_mul(2) {
                    return Err(DemuxError::InvalidBox {
                        offset: 0,
                        detail: "stz2 payload too short for 16-bit entries".into(),
                    });
                }
                for _ in 0..sample_count {
                    let v = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as u32;
                    sizes.push(v);
                    offset += 2;
                }
            }
            8 => {
                let bytes_needed = sample_count.div_ceil(2);
                if payload.len() < offset + bytes_needed {
                    return Err(DemuxError::InvalidBox {
                        offset: 0,
                        detail: "stz2 payload too short for 8-bit entries".into(),
                    });
                }
                for i in 0..sample_count {
                    let b = payload[offset + i / 2];
                    let v = if i % 2 == 0 { b >> 4 } else { b & 0x0F } as u32;
                    sizes.push(v);
                }
            }
            4 => {
                let bytes_needed = sample_count.div_ceil(2);
                if payload.len() < offset + bytes_needed {
                    return Err(DemuxError::InvalidBox {
                        offset: 0,
                        detail: "stz2 payload too short for 4-bit entries".into(),
                    });
                }
                for i in 0..sample_count {
                    let b = payload[offset + i / 2];
                    let v = if i % 2 == 0 { b >> 4 } else { b & 0x0F } as u32;
                    sizes.push(v);
                }
            }
            _ => unreachable!("field_size validated above"),
        }
        Ok(Self {
            sample_sizes: sizes,
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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 1_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stsc entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(12);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stsc payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut entries = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let first_chunk = u32::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]);
            let samples_per_chunk = u32::from_be_bytes([
                payload[offset + 4],
                payload[offset + 5],
                payload[offset + 6],
                payload[offset + 7],
            ]);
            let sample_description_index = u32::from_be_bytes([
                payload[offset + 8],
                payload[offset + 9],
                payload[offset + 10],
                payload[offset + 11],
            ]);
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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 10_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stco entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(4);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stco payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut offsets = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let chunk_off = u32::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
            ]) as u64;
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
        let entry_count =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
        const MAX_ENTRIES: usize = 10_000_000;
        if entry_count > MAX_ENTRIES {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("co64 entry_count {entry_count} exceeds limit {MAX_ENTRIES}"),
            });
        }
        let expected_len = 8 + entry_count.saturating_mul(8);
        if payload.len() < expected_len {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "co64 payload too short: expected {expected_len}, got {}",
                    payload.len()
                ),
            });
        }

        let mut offsets = Vec::with_capacity(entry_count);
        let mut offset = 8;
        for _ in 0..entry_count {
            let chunk_off = u64::from_be_bytes([
                payload[offset],
                payload[offset + 1],
                payload[offset + 2],
                payload[offset + 3],
                payload[offset + 4],
                payload[offset + 5],
                payload[offset + 6],
                payload[offset + 7],
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
    /// True if presentation timestamps (pts || dts) are monotonically non-decreasing.
    /// Audio tracks and non-B-frame video tracks are always monotonic.
    pub is_pts_monotonic: bool,
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
                is_pts_monotonic: true,
            });
        }

        // Validate required boxes: stsc and chunk offsets must be non-empty
        // when samples exist. Previously silently produced zero samples.
        if stsc.entries.is_empty() {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!("stsc missing or empty but stsz claims {total_samples} samples"),
            });
        }
        if chunk_offsets.offsets.is_empty() {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stco/co64 missing or empty but stsz claims {total_samples} samples"
                ),
            });
        }
        // Validate stsc entries are sorted and 1-based.
        for w in stsc.entries.windows(2) {
            if w[1].first_chunk < w[0].first_chunk || w[1].first_chunk == 0 {
                return Err(DemuxError::InvalidBox {
                    offset: 0,
                    detail: "stsc entries not sorted or zero first_chunk".into(),
                });
            }
        }
        if stsc.entries[0].first_chunk == 0 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "stsc first_chunk must be >= 1".into(),
            });
        }

        let dts_durations = stts.compute_dts(total_samples);
        let pts_offsets = ctts.map(|c| c.compute_pts_offsets(total_samples));

        // Reconstruct chunk sample counts
        let total_chunks = chunk_offsets.offsets.len();
        let mut chunk_sample_counts = Vec::with_capacity(total_chunks);

        let mut stsc_idx = 0;
        for chunk_num in 1..=total_chunks as u32 {
            while stsc_idx + 1 < stsc.entries.len()
                && stsc.entries[stsc_idx + 1].first_chunk <= chunk_num
            {
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
                let file_offset = chunk_offset.saturating_add(sample_offset_in_chunk);
                let (dts, duration) = dts_durations[current_sample_idx];

                let pts = pts_offsets
                    .as_ref()
                    .map(|offsets| dts.saturating_add(offsets[current_sample_idx]));

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

                sample_offset_in_chunk = sample_offset_in_chunk.saturating_add(size as u64);
                current_sample_idx += 1;
            }

            if current_sample_idx >= total_samples {
                break;
            }
        }

        // If stsc described fewer samples than stsz claims, report instead of
        // silently dropping the tail.
        if current_sample_idx != total_samples {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: format!(
                    "stsc/stco describe {current_sample_idx} samples but stsz claims {total_samples}"
                ),
            });
        }

        let is_pts_monotonic = samples.windows(2).all(|w| {
            let t0 = w[0].pts.unwrap_or(w[0].dts);
            let t1 = w[1].pts.unwrap_or(w[1].dts);
            t0 <= t1
        });

        Ok(Self {
            samples,
            is_pts_monotonic,
        })
    }

    /// Finds the nearest keyframe at or before `timestamp` (in timebase ticks).
    ///
    /// For monotonic sample sequences, this uses binary search in O(log N).
    /// Falls back to scanning when non-monotonic presentation timestamps (B-frames) exist.
    pub fn find_keyframe_at_or_before(&self, timestamp: i64) -> Option<&SampleInfo> {
        if self.samples.is_empty() {
            return None;
        }

        if self.is_pts_monotonic {
            let idx = self.samples.partition_point(|s| {
                let ts = s.pts.unwrap_or(s.dts);
                ts <= timestamp
            });
            let search_slice = if idx == 0 {
                &self.samples[..1.min(self.samples.len())]
            } else {
                &self.samples[..idx]
            };
            for s in search_slice.iter().rev() {
                if s.is_keyframe {
                    return Some(s);
                }
            }
            return self.samples.iter().find(|s| s.is_keyframe);
        }

        let mut best: Option<&SampleInfo> = None;
        let mut best_pts = i64::MIN;
        let mut first_keyframe: Option<&SampleInfo> = None;
        for s in &self.samples {
            if s.is_keyframe {
                if first_keyframe.is_none() {
                    first_keyframe = Some(s);
                }
                let ts = s.pts.unwrap_or(s.dts);
                if ts <= timestamp && ts >= best_pts {
                    best_pts = ts;
                    best = Some(s);
                }
            }
        }
        best.or(first_keyframe)
    }

    /// Finds the index of the sample with the greatest timestamp <= `timestamp`.
    ///
    /// For monotonic sample sequences (such as all audio tracks or non-B-frame video),
    /// executes in O(log N) using binary search.
    pub fn find_sample_index_at_or_before(&self, timestamp: i64) -> usize {
        if self.samples.is_empty() {
            return 0;
        }

        if self.is_pts_monotonic {
            let idx = self.samples.partition_point(|s| {
                let ts = s.pts.unwrap_or(s.dts);
                ts <= timestamp
            });
            if idx == 0 { 0 } else { idx - 1 }
        } else {
            let mut best_idx = 0usize;
            let mut best_pts = i64::MIN;
            let mut found = false;
            for s in &self.samples {
                let ts = s.pts.unwrap_or(s.dts);
                if ts <= timestamp && (!found || ts >= best_pts) {
                    best_pts = ts;
                    best_idx = s.index as usize;
                    found = true;
                }
            }
            if found { best_idx } else { 0 }
        }
    }

    /// Binary-search helper over a PTS-sorted projection. Callers that need
    /// O(log N) seeks on large tables should pre-sort indices externally;
    /// this method preserves correctness for DTS-ordered storage.
    pub fn find_keyframe_sorted(
        &self,
        sorted_by_pts: &[usize],
        timestamp: i64,
    ) -> Option<&SampleInfo> {
        // `sorted_by_pts` holds indices into `self.samples` sorted by PTS.
        let mut lo = 0usize;
        let mut hi = sorted_by_pts.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let s = &self.samples[sorted_by_pts[mid]];
            let ts = s.pts.unwrap_or(s.dts);
            if ts <= timestamp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        // Walk backwards from insertion point to find a keyframe.
        for &idx in sorted_by_pts[..lo].iter().rev() {
            if self.samples[idx].is_keyframe {
                return Some(&self.samples[idx]);
            }
        }
        self.samples.iter().find(|s| s.is_keyframe)
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
        )
        .unwrap();

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
