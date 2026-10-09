//! Fragmented MP4 (fMP4) `moof` box and track fragment parser.
//!
//! Implements ISO 14496-12 §8.8:
//! - `moof`: Movie Fragment Box
//! - `mfhd`: Movie Fragment Header Box
//! - `traf`: Track Fragment Box
//! - `tfhd`: Track Fragment Header Box
//! - `tfdt`: Track Fragment Base Media Decode Time Box
//! - `trun`: Track Run Box

use super::moov::{BoxIter, ParsedTrack};
use super::sample_table::SampleInfo;
use crate::DemuxError;

/// Parsed `tfhd` (Track Fragment Header) metadata.
#[derive(Debug, Clone, Default)]
struct Tfhd {
    track_id: u32,
    base_data_offset: Option<u64>,
    default_sample_duration: Option<u32>,
    default_sample_size: Option<u32>,
    default_sample_flags: Option<u32>,
    #[allow(dead_code)]
    default_base_is_moof: bool,
}

impl Tfhd {
    fn parse(payload: &[u8]) -> Result<Self, DemuxError> {
        if payload.len() < 8 {
            return Err(DemuxError::InvalidBox {
                offset: 0,
                detail: "tfhd payload too short".into(),
            });
        }

        let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
        let track_id = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);

        let mut offset = 8;
        let mut base_data_offset = None;
        let mut default_sample_duration = None;
        let mut default_sample_size = None;
        let mut default_sample_flags = None;

        // Flag 0x000001: base-data-offset-present (u64)
        if flags & 0x000001 != 0 {
            if offset + 8 > payload.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            base_data_offset = Some(u64::from_be_bytes([
                payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
                payload[offset + 4], payload[offset + 5], payload[offset + 6], payload[offset + 7],
            ]));
            offset += 8;
        }

        // Flag 0x000002: sample-description-index-present (u32)
        if flags & 0x000002 != 0 {
            if offset + 4 > payload.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            // Skip sample_description_index for now
            offset += 4;
        }

        // Flag 0x000008: default-sample-duration-present (u32)
        if flags & 0x000008 != 0 {
            if offset + 4 > payload.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            default_sample_duration = Some(u32::from_be_bytes([
                payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
            ]));
            offset += 4;
        }

        // Flag 0x000010: default-sample-size-present (u32)
        if flags & 0x000010 != 0 {
            if offset + 4 > payload.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            default_sample_size = Some(u32::from_be_bytes([
                payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
            ]));
            offset += 4;
        }

        // Flag 0x000020: default-sample-flags-present (u32)
        if flags & 0x000020 != 0 {
            if offset + 4 > payload.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            default_sample_flags = Some(u32::from_be_bytes([
                payload[offset], payload[offset + 1], payload[offset + 2], payload[offset + 3],
            ]));
        }

        let default_base_is_moof = flags & 0x020000 != 0;

        Ok(Self {
            track_id,
            base_data_offset,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
            default_base_is_moof,
        })
    }
}

/// Parses a top-level `moof` box and appends reconstructed sample metadata to `tracks`.
pub fn parse_moof(
    moof_body: &[u8],
    moof_offset: u64,
    tracks: &mut [ParsedTrack],
) -> Result<(), DemuxError> {
    for res in BoxIter::new(moof_body, moof_offset) {
        let (header, body, offset) = res?;
        if &header.box_type == b"traf" {
            parse_traf(body, offset + header.header_size as u64, moof_offset, tracks)?;
        }
    }
    Ok(())
}

fn parse_traf(
    traf_body: &[u8],
    traf_offset: u64,
    moof_offset: u64,
    tracks: &mut [ParsedTrack],
) -> Result<(), DemuxError> {
    let mut tfhd: Option<Tfhd> = None;
    let mut tfdt_decode_time: Option<u64> = None;

    // Collect child boxes
    let mut child_boxes = Vec::new();
    for res in BoxIter::new(traf_body, traf_offset) {
        let (header, body, offset) = res?;
        child_boxes.push((header, body, offset));
    }

    // First pass: extract tfhd and tfdt
    for (header, body, _) in &child_boxes {
        match &header.box_type {
            b"tfhd" => {
                tfhd = Some(Tfhd::parse(body)?);
            }
            b"tfdt" if body.len() >= 8 => {
                let version = body[0];
                if version == 1 && body.len() >= 12 {
                    let dt = u64::from_be_bytes([
                        body[4], body[5], body[6], body[7], body[8], body[9], body[10], body[11],
                    ]);
                    tfdt_decode_time = Some(dt);
                } else if version == 0 && body.len() >= 8 {
                    let dt = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as u64;
                    tfdt_decode_time = Some(dt);
                }
            }
            _ => {}
        }
    }

    let tfhd = match tfhd {
        Some(t) => t,
        None => return Ok(()),
    };

    let track = match tracks.iter_mut().find(|t| t.track_id == tfhd.track_id) {
        Some(t) => t,
        None => return Ok(()),
    };

    // Base data offset resolution (ISO 14496-12 §8.8.8)
    let base_offset = tfhd.base_data_offset.unwrap_or(moof_offset);

    // Track running DTS
    let mut current_dts = tfdt_decode_time.unwrap_or_else(|| {
        track
            .sample_table
            .samples
            .last()
            .map(|s| (s.dts + s.duration as i64) as u64)
            .unwrap_or(0)
    }) as i64;

    // Second pass: process `trun` boxes
    for (header, body, _) in child_boxes {
        if &header.box_type == b"trun" {
            parse_trun(body, base_offset, &tfhd, &mut current_dts, track)?;
        }
    }

    Ok(())
}

fn parse_trun(
    body: &[u8],
    base_offset: u64,
    tfhd: &Tfhd,
    current_dts: &mut i64,
    track: &mut ParsedTrack,
) -> Result<(), DemuxError> {
    if body.len() < 8 {
        return Err(DemuxError::InvalidBox {
            offset: 0,
            detail: "trun payload too short".into(),
        });
    }

    let version = body[0];
    let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
    let sample_count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]) as usize;

    let data_offset_present = flags & 0x000001 != 0;
    let first_sample_flags_present = flags & 0x000004 != 0;
    let sample_duration_present = flags & 0x000100 != 0;
    let sample_size_present = flags & 0x000200 != 0;
    let sample_flags_present = flags & 0x000400 != 0;
    let sample_composition_time_present = flags & 0x000800 != 0;

    let mut cursor = 8;

    let data_offset = if data_offset_present {
        if cursor + 4 > body.len() {
            return Err(DemuxError::UnexpectedEof { offset: 0 });
        }
        let off = i32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]);
        cursor += 4;
        off
    } else {
        0
    };

    let first_sample_flags = if first_sample_flags_present {
        if cursor + 4 > body.len() {
            return Err(DemuxError::UnexpectedEof { offset: 0 });
        }
        let f = u32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]);
        cursor += 4;
        Some(f)
    } else {
        None
    };

    let mut file_offset = (base_offset as i64 + data_offset as i64) as u64;

    for i in 0..sample_count {
        let duration = if sample_duration_present {
            if cursor + 4 > body.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            let d = u32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]);
            cursor += 4;
            d
        } else {
            tfhd.default_sample_duration.unwrap_or(0)
        };

        let size = if sample_size_present {
            if cursor + 4 > body.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            let s = u32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]);
            cursor += 4;
            s
        } else {
            tfhd.default_sample_size.unwrap_or(0)
        };

        let sample_flags = if i == 0 && first_sample_flags.is_some() {
            first_sample_flags.unwrap_or(0)
        } else if sample_flags_present {
            if cursor + 4 > body.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            let f = u32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]);
            cursor += 4;
            f
        } else {
            tfhd.default_sample_flags.unwrap_or(0)
        };

        let composition_offset = if sample_composition_time_present {
            if cursor + 4 > body.len() {
                return Err(DemuxError::UnexpectedEof { offset: 0 });
            }
            let c_offset = if version == 0 {
                u32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]) as i64
            } else {
                i32::from_be_bytes([body[cursor], body[cursor + 1], body[cursor + 2], body[cursor + 3]]) as i64
            };
            cursor += 4;
            Some(c_offset)
        } else {
            None
        };

        let dts = *current_dts;
        let pts = composition_offset.map(|c| dts + c);

        // In ISOBMFF sample_flags: bit 24 is `sample_is_non_sync_sample` (0 = keyframe, 1 = non-keyframe)
        // If sample_flags == 0 and it's the first sample of run or sync sample
        let is_keyframe = (sample_flags & 0x01000000) == 0;

        let sample_index = track.sample_table.samples.len() as u32;

        track.sample_table.samples.push(SampleInfo {
            index: sample_index,
            file_offset,
            size,
            dts,
            pts,
            duration,
            is_keyframe,
        });

        file_offset += size as u64;
        *current_dts += duration as i64;
    }

    Ok(())
}
