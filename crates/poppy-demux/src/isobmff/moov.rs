//! ISOBMFF `moov` (Movie) box and track hierarchy parser.
//!
//! Parses `moov` → `trak` → `mdia` → `minf` → `stbl` to extract stream metadata,
//! codec configuration (`avcC`, etc.), and reconstructed sample tables.

use poppy_core::codec::{
    AudioConfig, Av1CRecord, AvcCRecord, CodecConfig, CodecId, HvcCRecord, VpcCRecord,
};
use poppy_core::{StreamInfo, TimeBase};

use super::box_reader::BoxHeader;
use super::sample_table::{
    ChunkOffsets, CttsTable, SampleTable, StscTable, StssTable, StszTable, SttsTable,
};
use crate::DemuxError;
use bytes::Bytes;

/// Movie-level track defaults from `mvex/trex` (ISO 14496-12 §8.8.3).
/// Used as fallback when `tfhd/trun` omit per-sample duration/size/flags.
#[derive(Debug, Clone, Default)]
pub struct TrexDefaults {
    pub track_id: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

fn parse_trex(body: &[u8]) -> Option<TrexDefaults> {
    if body.len() < 20 {
        return None;
    }
    // FullBox: version(1)+flags(3) already part of body
    let track_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let _desc_index = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
    let dur = u32::from_be_bytes([body[12], body[13], body[14], body[15]]);
    let size = u32::from_be_bytes([body[16], body[17], body[18], body[19]]);
    let flags = if body.len() >= 24 {
        u32::from_be_bytes([body[20], body[21], body[22], body[23]])
    } else {
        0
    };
    Some(TrexDefaults {
        track_id,
        default_sample_duration: dur,
        default_sample_size: size,
        default_sample_flags: flags,
    })
}

fn parse_mvex(data: &[u8], file_offset: u64) -> Vec<TrexDefaults> {
    let mut out = Vec::new();
    for res in BoxIter::new(data, file_offset) {
        if let Ok((header, body, _)) = res
            && &header.box_type == b"trex"
            && let Some(t) = parse_trex(body)
        {
            out.push(t);
        }
    }
    out
}

/// A parsed track from an ISOBMFF container.
#[derive(Debug, Clone)]
pub struct ParsedTrack {
    pub track_id: u32,
    pub handler_type: [u8; 4],
    pub time_base: TimeBase,
    pub duration_ticks: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub codec_id: CodecId,
    pub codec_config: CodecConfig,
    pub sample_table: SampleTable,
    /// Defaults from `mvex/trex` for fragmented tracks.
    pub trex: Option<TrexDefaults>,
    /// Edit list (media-time, duration) if present. Applied to timestamps.
    pub edit_list: Vec<(i64, i64)>,
}

impl ParsedTrack {
    /// Convert to public `StreamInfo` metadata.
    pub fn to_stream_info(&self, index: usize) -> StreamInfo {
        StreamInfo {
            index,
            codec: self.codec_id,
            time_base: self.time_base,
            duration_ticks: self.duration_ticks,
            config: self.codec_config.clone(),
        }
    }
}

/// Helper to iterate over immediate child boxes within a container box payload.
pub struct BoxIter<'a> {
    data: &'a [u8],
    file_offset: u64,
    cursor: usize,
    /// Whether a top-level `size == 0` ("to EOF") box is allowed.
    /// Per ISO 14496-12 §4.2 this is only valid at the top level of the file.
    /// Inner containers must reject it as corrupt.
    allow_size_zero_to_eof: bool,
}

impl<'a> BoxIter<'a> {
    pub fn new(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
            allow_size_zero_to_eof: false,
        }
    }

    /// Top-level iterator where a trailing `size == 0` box extending to EOF
    /// (typically `mdat`) is legal.
    pub fn new_top_level(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
            allow_size_zero_to_eof: true,
        }
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Result<(BoxHeader, &'a [u8], u64), DemuxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.data.len() {
            return None;
        }

        let remaining = &self.data[self.cursor..];
        let current_offset = self.file_offset + self.cursor as u64;

        if remaining.len() < 8 {
            return Some(Err(DemuxError::UnexpectedEof {
                offset: current_offset,
            }));
        }

        match BoxHeader::parse(remaining, current_offset) {
            Ok(header) => {
                let box_size = if header.size == 0 {
                    if !self.allow_size_zero_to_eof {
                        return Some(Err(DemuxError::InvalidBox {
                            offset: current_offset,
                            detail: format!(
                                "inner box '{}' has size 0 (to-EOF only valid top-level)",
                                header.type_str(),
                            ),
                        }));
                    }
                    remaining.len()
                } else {
                    // u64 -> usize conversion checked for 32-bit platforms.
                    let sz: usize = match usize::try_from(header.size) {
                        Ok(v) => v,
                        Err(_) => {
                            return Some(Err(DemuxError::InvalidBox {
                                offset: current_offset,
                                detail: format!(
                                    "box '{}' size {} does not fit in usize",
                                    header.type_str(),
                                    header.size
                                ),
                            }));
                        }
                    };
                    sz
                };

                if box_size < header.header_size as usize || box_size > remaining.len() {
                    return Some(Err(DemuxError::UnexpectedEof {
                        offset: current_offset,
                    }));
                }

                let body_start = self.cursor + header.header_size as usize;
                let body_end = self.cursor + box_size;

                if body_start > body_end || body_end > self.data.len() {
                    return Some(Err(DemuxError::InvalidBox {
                        offset: current_offset,
                        detail: "invalid box body range".into(),
                    }));
                }

                let body = &self.data[body_start..body_end];
                self.cursor += box_size;

                Some(Ok((header, body, current_offset)))
            }
            Err(e) => Some(Err(e)),
        }
    }
}

/// DASH Segment Index (`sidx`) entry per ISO 14496-12 §8.16.
#[derive(Debug, Clone)]
pub struct SidxSegment {
    pub reference_id: u32,
    pub timescale: u32,
    /// Earliest presentation time of this segment (in sidx timescale).
    pub earliest_presentation_time: u64,
    /// First offset of segment (byte offset from anchor).
    pub first_offset: u64,
    /// Subsegment durations + byte ranges.
    pub references: Vec<SidxReference>,
}

/// Single reference inside `sidx`.
#[derive(Debug, Clone)]
pub struct SidxReference {
    pub reference_type: bool,
    pub referenced_size: u32,
    pub subsegment_duration: u32,
    pub starts_with_sap: bool,
    pub sap_type: u8,
    pub sap_delta_time: u32,
}

/// Public wrapper for DASH `sidx` parsing (used by `Mp4Demuxer` top-level scan).
pub fn parse_sidx_pub(body: &[u8]) -> Option<SidxSegment> {
    parse_sidx(body)
}

fn parse_sidx(body: &[u8]) -> Option<SidxSegment> {
    if body.len() < 12 {
        return None;
    }
    let version = body[0];
    // flags(3) skipped
    let reference_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let timescale = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
    if timescale == 0 {
        return None;
    }
    let mut cur = 12;
    let (earliest, first_offset) = if version == 0 {
        if body.len() < cur + 8 {
            return None;
        }
        let e = u32::from_be_bytes([body[cur], body[cur + 1], body[cur + 2], body[cur + 3]]) as u64;
        let f =
            u32::from_be_bytes([body[cur + 4], body[cur + 5], body[cur + 6], body[cur + 7]]) as u64;
        cur += 8;
        (e, f)
    } else {
        if body.len() < cur + 16 {
            return None;
        }
        let e = u64::from_be_bytes([
            body[cur],
            body[cur + 1],
            body[cur + 2],
            body[cur + 3],
            body[cur + 4],
            body[cur + 5],
            body[cur + 6],
            body[cur + 7],
        ]);
        let f = u64::from_be_bytes([
            body[cur + 8],
            body[cur + 9],
            body[cur + 10],
            body[cur + 11],
            body[cur + 12],
            body[cur + 13],
            body[cur + 14],
            body[cur + 15],
        ]);
        cur += 16;
        (e, f)
    };
    if body.len() < cur + 4 {
        return None;
    }
    let _reserved = u16::from_be_bytes([body[cur], body[cur + 1]]);
    let ref_count = u16::from_be_bytes([body[cur + 2], body[cur + 3]]) as usize;
    cur += 4;
    if ref_count > 256 {
        return None; // pathological
    }
    let mut references = Vec::with_capacity(ref_count);
    for _ in 0..ref_count {
        if body.len() < cur + 12 {
            return None;
        }
        let ref_and_size =
            u32::from_be_bytes([body[cur], body[cur + 1], body[cur + 2], body[cur + 3]]);
        let dur = u32::from_be_bytes([body[cur + 4], body[cur + 5], body[cur + 6], body[cur + 7]]);
        let sap =
            u32::from_be_bytes([body[cur + 8], body[cur + 9], body[cur + 10], body[cur + 11]]);
        cur += 12;
        references.push(SidxReference {
            reference_type: (ref_and_size >> 31) != 0,
            referenced_size: ref_and_size & 0x7FFF_FFFF,
            subsegment_duration: dur,
            starts_with_sap: (sap >> 31) != 0,
            sap_type: ((sap >> 28) & 0x07) as u8,
            sap_delta_time: sap & 0x0FFF_FFFF,
        });
    }
    Some(SidxSegment {
        reference_id,
        timescale,
        earliest_presentation_time: earliest,
        first_offset,
        references,
    })
}

/// Movie Fragment Random Access (`mfra`) entry: track fragment run boxes.
/// Minimal parser returning (track_id, [(time, moof_offset)]) for seeking.
#[derive(Debug, Clone, Default)]
pub struct MfraEntry {
    pub track_id: u32,
    pub entries: Vec<(u64, u64)>, // (time, moof_offset)
}

/// Public wrapper for `mfra` parsing (used by `Mp4Demuxer` top-level scan).
pub fn parse_mfra_pub(data: &[u8], file_offset: u64) -> Vec<MfraEntry> {
    parse_mfra(data, file_offset)
}

fn parse_mfra(data: &[u8], file_offset: u64) -> Vec<MfraEntry> {
    let mut out = Vec::new();
    for res in BoxIter::new(data, file_offset) {
        let Ok((header, body, off)) = res else {
            continue;
        };
        if &header.box_type != b"tfra" || body.len() < 16 {
            continue;
        }
        let version = body[0];
        let track_id = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
        let len_sizes = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
        let entry_count = u32::from_be_bytes([body[12], body[13], body[14], body[15]]) as usize;
        let mut cur = 16;
        if entry_count > 4096 {
            continue;
        }

        let traf_size = ((len_sizes >> 4) & 0x03) as usize + 1;
        let trun_size = ((len_sizes >> 2) & 0x03) as usize + 1;
        let sample_size = (len_sizes & 0x03) as usize + 1;
        let extra_bytes = traf_size + trun_size + sample_size;

        let mut entries = Vec::with_capacity(entry_count.min(64));
        for _ in 0..entry_count {
            let (time, moof_off, adv) = if version == 1 {
                if body.len() < cur + 16 {
                    break;
                }
                let t = u64::from_be_bytes([
                    body[cur],
                    body[cur + 1],
                    body[cur + 2],
                    body[cur + 3],
                    body[cur + 4],
                    body[cur + 5],
                    body[cur + 6],
                    body[cur + 7],
                ]);
                let m = u64::from_be_bytes([
                    body[cur + 8],
                    body[cur + 9],
                    body[cur + 10],
                    body[cur + 11],
                    body[cur + 12],
                    body[cur + 13],
                    body[cur + 14],
                    body[cur + 15],
                ]);
                (t, m, 16)
            } else {
                if body.len() < cur + 8 {
                    break;
                }
                let t = u32::from_be_bytes([body[cur], body[cur + 1], body[cur + 2], body[cur + 3]])
                    as u64;
                let m = u32::from_be_bytes([
                    body[cur + 4],
                    body[cur + 5],
                    body[cur + 6],
                    body[cur + 7],
                ]) as u64;
                (t, m, 8)
            };
            cur += adv;
            if body.len() < cur + extra_bytes {
                break;
            }
            cur += extra_bytes;
            entries.push((time, moof_off));
            let _ = off;
        }
        out.push(MfraEntry { track_id, entries });
    }
    out
}

/// Parses the `moov` box payload into a list of parsed tracks.
pub fn parse_moov(data: &[u8], file_offset: u64) -> Result<Vec<ParsedTrack>, DemuxError> {
    let mut tracks = Vec::new();
    let mut trex_list: Vec<TrexDefaults> = Vec::new();

    // First pass: collect mvex/trex defaults (needed for fragmented tracks).
    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        if &header.box_type == b"mvex" {
            trex_list.extend(parse_mvex(body, offset + header.header_size as u64));
        }
    }
    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        if &header.box_type == b"trak"
            && let Some(mut track) = parse_trak(body, offset + header.header_size as u64)?
        {
            // Attach matching trex defaults.
            if let Some(t) = trex_list
                .iter()
                .find(|t| t.track_id == track.track_id)
                .cloned()
            {
                track.trex = Some(t);
            }
            tracks.push(track);
        }
    }

    Ok(tracks)
}

/// Parses a single `trak` box payload.
fn parse_trak(data: &[u8], file_offset: u64) -> Result<Option<ParsedTrack>, DemuxError> {
    let mut track_id = 0u32;
    let mut tkhd_width = 0u32;
    let mut tkhd_height = 0u32;
    let mut tkhd_duration = 0u64;

    let mut mdhd_timescale = 0u32;
    let mut mdhd_duration = 0u64;
    let mut mdhd_duration_unknown = false;
    let mut handler_type = [0u8; 4];

    let mut sample_table = None;
    let mut codec_id = CodecId::Unknown(0);
    let mut codec_config = CodecConfig::Unknown;
    let mut sample_width = 0u32;
    let mut sample_height = 0u32;
    let mut edit_list: Vec<(i64, i64)> = Vec::new();

    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        match &header.box_type {
            b"tkhd" => {
                if body.len() >= 4 {
                    let version = body[0];
                    if version == 0 && body.len() >= 84 {
                        track_id = u32::from_be_bytes([body[12], body[13], body[14], body[15]]);
                        let dur = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
                        // 0xFFFFFFFF means unknown duration (spec).
                        if dur != 0xFFFF_FFFF {
                            tkhd_duration = dur as u64;
                        }
                        tkhd_width =
                            u32::from_be_bytes([body[76], body[77], body[78], body[79]]) >> 16;
                        tkhd_height =
                            u32::from_be_bytes([body[80], body[81], body[82], body[83]]) >> 16;
                    } else if version == 1 && body.len() >= 96 {
                        track_id = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
                        let dur = u64::from_be_bytes([
                            body[28], body[29], body[30], body[31], body[32], body[33], body[34],
                            body[35],
                        ]);
                        if dur != 0xFFFF_FFFF_FFFF_FFFF {
                            tkhd_duration = dur;
                        }
                        tkhd_width =
                            u32::from_be_bytes([body[88], body[89], body[90], body[91]]) >> 16;
                        tkhd_height =
                            u32::from_be_bytes([body[92], body[93], body[94], body[95]]) >> 16;
                    }
                }
            }
            b"edts" => {
                // Parse edit list: elst -> [(media_time, segment_duration)]
                let edts_offset = offset + header.header_size as u64;
                for edts_res in BoxIter::new(body, edts_offset) {
                    let (e_header, e_body, _) = edts_res?;
                    if &e_header.box_type == b"elst" && e_body.len() >= 8 {
                        let version = e_body[0];
                        let count = u32::from_be_bytes([e_body[4], e_body[5], e_body[6], e_body[7]])
                            as usize;
                        if count > 64 {
                            continue; // ignore pathological edit lists
                        }
                        let mut cur = 8;
                        for _ in 0..count {
                            if version == 1 {
                                if cur + 20 > e_body.len() {
                                    break;
                                }
                                let seg_dur = u64::from_be_bytes([
                                    e_body[cur],
                                    e_body[cur + 1],
                                    e_body[cur + 2],
                                    e_body[cur + 3],
                                    e_body[cur + 4],
                                    e_body[cur + 5],
                                    e_body[cur + 6],
                                    e_body[cur + 7],
                                ]) as i64;
                                let media_time = i64::from_be_bytes([
                                    e_body[cur + 8],
                                    e_body[cur + 9],
                                    e_body[cur + 10],
                                    e_body[cur + 11],
                                    e_body[cur + 12],
                                    e_body[cur + 13],
                                    e_body[cur + 14],
                                    e_body[cur + 15],
                                ]);
                                // media_time == -1 means empty edit (dwell).
                                edit_list.push((media_time, seg_dur));
                                cur += 20;
                            } else {
                                if cur + 12 > e_body.len() {
                                    break;
                                }
                                let seg_dur = u32::from_be_bytes([
                                    e_body[cur],
                                    e_body[cur + 1],
                                    e_body[cur + 2],
                                    e_body[cur + 3],
                                ]) as i64;
                                let media_time = i32::from_be_bytes([
                                    e_body[cur + 4],
                                    e_body[cur + 5],
                                    e_body[cur + 6],
                                    e_body[cur + 7],
                                ]) as i64;
                                edit_list.push((media_time, seg_dur));
                                cur += 12;
                            }
                        }
                    }
                }
            }
            b"mdia" => {
                let mdia_offset = offset + header.header_size as u64;
                for mdia_res in BoxIter::new(body, mdia_offset) {
                    let (m_header, m_body, m_offset) = mdia_res?;
                    match &m_header.box_type {
                        b"mdhd" => {
                            if m_body.len() >= 4 {
                                let version = m_body[0];
                                if version == 0 && m_body.len() >= 20 {
                                    mdhd_timescale = u32::from_be_bytes([
                                        m_body[12], m_body[13], m_body[14], m_body[15],
                                    ]);
                                    let dur = u32::from_be_bytes([
                                        m_body[16], m_body[17], m_body[18], m_body[19],
                                    ]);
                                    if dur == 0xFFFF_FFFF {
                                        mdhd_duration_unknown = true;
                                    } else {
                                        mdhd_duration = dur as u64;
                                    }
                                } else if version == 1 && m_body.len() >= 28 {
                                    mdhd_timescale = u32::from_be_bytes([
                                        m_body[20], m_body[21], m_body[22], m_body[23],
                                    ]);
                                    let dur = u64::from_be_bytes([
                                        m_body[24], m_body[25], m_body[26], m_body[27], m_body[28],
                                        m_body[29], m_body[30], m_body[31],
                                    ]);
                                    if dur == 0xFFFF_FFFF_FFFF_FFFF {
                                        mdhd_duration_unknown = true;
                                    } else {
                                        mdhd_duration = dur;
                                    }
                                }
                            }
                        }
                        b"hdlr" => {
                            if m_body.len() >= 12 {
                                handler_type = [m_body[8], m_body[9], m_body[10], m_body[11]];
                            }
                        }
                        b"minf" => {
                            let minf_offset = m_offset + m_header.header_size as u64;
                            for minf_res in BoxIter::new(m_body, minf_offset) {
                                let (minf_h, minf_b, minf_off) = minf_res?;
                                if &minf_h.box_type == b"stbl" {
                                    let stbl_parsed = parse_stbl(
                                        minf_b,
                                        minf_off + minf_h.header_size as u64,
                                        &handler_type,
                                    )?;
                                    sample_table = Some(stbl_parsed.sample_table);
                                    codec_id = stbl_parsed.codec_id;
                                    codec_config = stbl_parsed.codec_config;
                                    sample_width = stbl_parsed.width;
                                    sample_height = stbl_parsed.height;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let sample_table = match sample_table {
        Some(st) => st,
        None => return Ok(None),
    };

    // Timescale 0 is corrupt; default to 1000 only when the box was absent.
    // If mdhd was present but timescale==0, treat as corrupt rather than panic.
    let timescale = if mdhd_timescale > 0 {
        mdhd_timescale
    } else {
        // No valid mdhd timescale: fall back to movie timescale default.
        // This preserves playback for mildly corrupt files while avoiding
        // TimeBase::new panic (den==0).
        1000
    };
    let time_base = TimeBase::new(1, timescale);
    let duration_ticks = if mdhd_duration_unknown {
        None
    } else if mdhd_duration > 0 {
        Some(mdhd_duration.min(i64::MAX as u64) as i64)
    } else if tkhd_duration > 0 {
        Some(tkhd_duration.min(i64::MAX as u64) as i64)
    } else {
        None
    };

    let width = if sample_width > 0 {
        sample_width
    } else {
        tkhd_width
    };
    let height = if sample_height > 0 {
        sample_height
    } else {
        tkhd_height
    };

    Ok(Some(ParsedTrack {
        track_id,
        handler_type,
        time_base,
        duration_ticks,
        width,
        height,
        codec_id,
        codec_config,
        sample_table,
        trex: None,
        edit_list,
    }))
}

struct ParsedStbl {
    codec_id: CodecId,
    codec_config: CodecConfig,
    width: u32,
    height: u32,
    sample_table: SampleTable,
}

fn parse_stbl(
    data: &[u8],
    file_offset: u64,
    handler_type: &[u8; 4],
) -> Result<ParsedStbl, DemuxError> {
    let mut stsz = None;
    let mut stz2: Option<crate::isobmff::sample_table::Stz2Table> = None;
    let mut stsc = None;
    let mut chunk_offsets = None;
    let mut stts = None;
    let mut ctts = None;
    let mut stss = None;

    let mut codec_id = CodecId::Unknown(0);
    let mut codec_config = CodecConfig::Unknown;
    let mut width = 0u32;
    let mut height = 0u32;

    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        match &header.box_type {
            b"stsd" => {
                let parsed_stsd =
                    parse_stsd(body, offset + header.header_size as u64, handler_type)?;
                codec_id = parsed_stsd.codec_id;
                codec_config = parsed_stsd.codec_config;
                width = parsed_stsd.width;
                height = parsed_stsd.height;
            }
            b"stsz" => {
                stsz = Some(StszTable::parse(body)?);
            }
            b"stz2" => {
                stz2 = Some(crate::isobmff::sample_table::Stz2Table::parse(body)?);
            }
            b"stsc" => {
                stsc = Some(StscTable::parse(body)?);
            }
            b"stco" => {
                chunk_offsets = Some(ChunkOffsets::parse_stco(body)?);
            }
            b"co64" => {
                chunk_offsets = Some(ChunkOffsets::parse_co64(body)?);
            }
            b"stts" => {
                stts = Some(SttsTable::parse(body)?);
            }
            b"ctts" => {
                ctts = Some(CttsTable::parse(body)?);
            }
            b"stss" => {
                stss = Some(StssTable::parse(body)?);
            }
            // Encrypted content: report instead of silently returning ciphertext.
            b"saiz" | b"saio" | b"senc" | b"schm" => {
                return Err(DemuxError::InvalidBox {
                    offset,
                    detail: format!(
                        "encrypted sample box '{}' not supported (CENC)",
                        header.type_str()
                    ),
                });
            }
            _ => {}
        }
    }

    // Prefer stsz; fall back to stz2 compact sizes.
    let stsz_table = if let Some(s) = stsz {
        s
    } else if let Some(z2) = stz2 {
        StszTable {
            default_sample_size: 0,
            sample_sizes: z2.sample_sizes,
        }
    } else {
        StszTable::default()
    };
    // If no samples (fragmented init segment), return empty table without
    // requiring stsc/stco/stts.
    if stsz_table.total_samples() == 0 {
        return Ok(ParsedStbl {
            codec_id,
            codec_config,
            width,
            height,
            sample_table: SampleTable {
                samples: Vec::new(),
                is_pts_monotonic: true,
            },
        });
    }
    let stsc_table = stsc.ok_or_else(|| DemuxError::BoxNotFound {
        fourcc: "stsc".into(),
    })?;
    let co_table = chunk_offsets.ok_or_else(|| DemuxError::BoxNotFound {
        fourcc: "stco/co64".into(),
    })?;
    let stts_table = stts.ok_or_else(|| DemuxError::BoxNotFound {
        fourcc: "stts".into(),
    })?;

    let sample_table = SampleTable::build(
        &stsz_table,
        &stsc_table,
        &co_table,
        &stts_table,
        ctts.as_ref(),
        stss.as_ref(),
    )?;

    Ok(ParsedStbl {
        codec_id,
        codec_config,
        width,
        height,
        sample_table,
    })
}

struct ParsedStsd {
    codec_id: CodecId,
    codec_config: CodecConfig,
    width: u32,
    height: u32,
}

fn parse_stsd(
    data: &[u8],
    file_offset: u64,
    handler_type: &[u8; 4],
) -> Result<ParsedStsd, DemuxError> {
    if data.len() < 8 {
        return Err(DemuxError::InvalidBox {
            offset: file_offset,
            detail: "stsd payload too short".into(),
        });
    }

    let entry_count = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if entry_count == 0 {
        return Ok(ParsedStsd {
            codec_id: CodecId::Unknown(0),
            codec_config: CodecConfig::Unknown,
            width: 0,
            height: 0,
        });
    }

    let cursor = 8;
    let mut codec_id = CodecId::Unknown(0);
    let mut codec_config = CodecConfig::Unknown;
    let mut width = 0u32;
    let mut height = 0u32;

    // Parse the first entry
    if cursor + 8 <= data.len() {
        let entry_size = u32::from_be_bytes([
            data[cursor],
            data[cursor + 1],
            data[cursor + 2],
            data[cursor + 3],
        ]) as usize;
        let entry_fourcc: [u8; 4] = [
            data[cursor + 4],
            data[cursor + 5],
            data[cursor + 6],
            data[cursor + 7],
        ];
        codec_id = CodecId::from_mp4_fourcc(&entry_fourcc);

        if entry_size >= 8 && cursor + entry_size <= data.len() {
            let entry_payload = &data[cursor + 8..cursor + entry_size];
            let is_video = handler_type == b"vide";

            if is_video && entry_payload.len() >= 78 {
                width = u16::from_be_bytes([entry_payload[24], entry_payload[25]]) as u32;
                height = u16::from_be_bytes([entry_payload[26], entry_payload[27]]) as u32;

                // Child boxes start at byte 78 of entry_payload (ISO 14496-12 §8.5.2.2)
                let child_boxes = &entry_payload[78..];
                let child_offset = file_offset + (cursor + 8 + 78) as u64;

                for (c_header, c_body, _) in BoxIter::new(child_boxes, child_offset).flatten() {
                    if &c_header.box_type == b"avcC"
                        && let Ok(avcc) = AvcCRecord::parse(c_body)
                    {
                        codec_config = CodecConfig::H264(avcc);
                    } else if &c_header.box_type == b"hvcC"
                        && let Ok(hvcc) = HvcCRecord::parse(c_body)
                    {
                        codec_config = CodecConfig::Hevc(hvcc);
                    } else if &c_header.box_type == b"av1C"
                        && let Ok(av1c) = Av1CRecord::parse(c_body)
                    {
                        codec_config = CodecConfig::Av1(av1c);
                    } else if &c_header.box_type == b"vpcC"
                        && let Ok(vpcc) = VpcCRecord::parse(c_body)
                    {
                        codec_config = CodecConfig::Vp9(vpcc);
                    }
                }
            } else if entry_payload.len() >= 28 {
                // Audio sample entry: (ISO 14496-12 §8.5.2.2 AudioSampleEntry)
                let channels = u16::from_be_bytes([entry_payload[16], entry_payload[17]]);
                let bits_per_sample = u16::from_be_bytes([entry_payload[18], entry_payload[19]]);
                // sample_rate is 16.16 fixed point; 0 is corrupt — keep 0 so
                // downstream can report instead of hiding with max(1).
                let sample_rate = u32::from_be_bytes([
                    entry_payload[24],
                    entry_payload[25],
                    entry_payload[26],
                    entry_payload[27],
                ]) >> 16;

                let mut extradata = None;
                if entry_payload.len() > 28 {
                    let child_boxes = &entry_payload[28..];
                    let child_offset = file_offset + (cursor + 8 + 28) as u64;
                    for (c_header, c_body, _) in BoxIter::new(child_boxes, child_offset).flatten() {
                        if &c_header.box_type == b"esds" {
                            // Only use AudioSpecificConfig on successful parse.
                            // Previously stored the entire esds box as extradata
                            // on failure, feeding version/flags/tags to decoders.
                            extradata = extract_esds_audio_specific_config(c_body);
                            break;
                        } else if &c_header.box_type == b"dOps" || &c_header.box_type == b"dfLa" {
                            extradata = Some(bytes::Bytes::copy_from_slice(c_body));
                            break;
                        }
                    }
                }

                codec_config = CodecConfig::Audio(AudioConfig {
                    sample_rate,
                    channels,
                    bits_per_sample,
                    extradata,
                });
            }
        }
    }

    Ok(ParsedStsd {
        codec_id,
        codec_config,
        width,
        height,
    })
}

/// Extracts the AudioSpecificConfig payload from an MPEG-4 esds box body.
fn extract_esds_audio_specific_config(esds_body: &[u8]) -> Option<Bytes> {
    if esds_body.len() < 5 {
        return None;
    }
    let mut cursor = 4; // skip version (1) and flags (3)

    fn read_tag_and_len(data: &[u8], cursor: &mut usize) -> Option<(u8, usize)> {
        if *cursor >= data.len() {
            return None;
        }
        let tag = data[*cursor];
        *cursor += 1;

        let mut len = 0usize;
        for _ in 0..4 {
            if *cursor >= data.len() {
                return None;
            }
            let b = data[*cursor];
            *cursor += 1;
            len = (len << 7) | ((b & 0x7F) as usize);
            if (b & 0x80) == 0 {
                break;
            }
        }
        Some((tag, len))
    }

    // Expect Tag 0x03 (ES_DescrTag)
    let (tag, _) = read_tag_and_len(esds_body, &mut cursor)?;
    if tag != 0x03 || cursor + 3 > esds_body.len() {
        return None;
    }
    cursor += 2; // ES_ID
    let flags = esds_body[cursor];
    cursor += 1;
    if (flags & 0x80) != 0 {
        cursor += 2;
    }
    if (flags & 0x40) != 0 {
        if cursor >= esds_body.len() {
            return None;
        }
        let url_len = esds_body[cursor] as usize;
        cursor += 1 + url_len;
    }
    if (flags & 0x20) != 0 {
        cursor += 2;
    }

    // Expect Tag 0x04 (DecoderConfigDescrTag)
    let (tag, _) = read_tag_and_len(esds_body, &mut cursor)?;
    if tag != 0x04 || cursor + 13 > esds_body.len() {
        return None;
    }
    cursor += 13; // fixed fields

    // Expect Tag 0x05 (DecSpecificInfoTag)
    let (tag, len) = read_tag_and_len(esds_body, &mut cursor)?;
    if tag != 0x05 || cursor + len > esds_body.len() {
        return None;
    }

    Some(Bytes::copy_from_slice(&esds_body[cursor..cursor + len]))
}
