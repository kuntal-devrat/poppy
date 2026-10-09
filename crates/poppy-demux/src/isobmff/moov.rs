//! ISOBMFF `moov` (Movie) box and track hierarchy parser.
//!
//! Parses `moov` → `trak` → `mdia` → `minf` → `stbl` to extract stream metadata,
//! codec configuration (`avcC`, etc.), and reconstructed sample tables.

use poppy_core::codec::{AvcCRecord, CodecConfig, CodecId};
use poppy_core::{StreamInfo, TimeBase};

use super::box_reader::BoxHeader;
use super::sample_table::{
    ChunkOffsets, CttsTable, SampleTable, StscTable, StssTable, StszTable, SttsTable,
};
use crate::DemuxError;

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
}

impl<'a> BoxIter<'a> {
    pub fn new(data: &'a [u8], file_offset: u64) -> Self {
        Self {
            data,
            file_offset,
            cursor: 0,
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
                    remaining.len()
                } else {
                    header.size as usize
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

/// Parses the `moov` box payload into a list of parsed tracks.
pub fn parse_moov(data: &[u8], file_offset: u64) -> Result<Vec<ParsedTrack>, DemuxError> {
    let mut tracks = Vec::new();

    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        if &header.box_type == b"trak" {
            if let Some(track) = parse_trak(body, offset + header.header_size as u64)? {
                tracks.push(track);
            }
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
    let mut handler_type = [0u8; 4];

    let mut sample_table = None;
    let mut codec_id = CodecId::Unknown(0);
    let mut codec_config = CodecConfig::Unknown;
    let mut sample_width = 0u32;
    let mut sample_height = 0u32;

    for res in BoxIter::new(data, file_offset) {
        let (header, body, offset) = res?;
        match &header.box_type {
            b"tkhd" => {
                if body.len() >= 4 {
                    let version = body[0];
                    if version == 0 && body.len() >= 84 {
                        track_id = u32::from_be_bytes([body[12], body[13], body[14], body[15]]);
                        tkhd_duration = u32::from_be_bytes([body[20], body[21], body[22], body[23]]) as u64;
                        tkhd_width = u32::from_be_bytes([body[76], body[77], body[78], body[79]]) >> 16;
                        tkhd_height = u32::from_be_bytes([body[80], body[81], body[82], body[83]]) >> 16;
                    } else if version == 1 && body.len() >= 96 {
                        track_id = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
                        tkhd_duration = u64::from_be_bytes([
                            body[28], body[29], body[30], body[31], body[32], body[33], body[34], body[35],
                        ]);
                        tkhd_width = u32::from_be_bytes([body[88], body[89], body[90], body[91]]) >> 16;
                        tkhd_height = u32::from_be_bytes([body[92], body[93], body[94], body[95]]) >> 16;
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
                                    mdhd_timescale = u32::from_be_bytes([m_body[12], m_body[13], m_body[14], m_body[15]]);
                                    mdhd_duration = u32::from_be_bytes([m_body[16], m_body[17], m_body[18], m_body[19]]) as u64;
                                } else if version == 1 && m_body.len() >= 28 {
                                    mdhd_timescale = u32::from_be_bytes([m_body[20], m_body[21], m_body[22], m_body[23]]);
                                    mdhd_duration = u64::from_be_bytes([
                                        m_body[24], m_body[25], m_body[26], m_body[27], m_body[28], m_body[29], m_body[30], m_body[31],
                                    ]);
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
                                    let stbl_parsed = parse_stbl(minf_b, minf_off + minf_h.header_size as u64, &handler_type)?;
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

    let timescale = if mdhd_timescale > 0 { mdhd_timescale } else { 1000 };
    let time_base = TimeBase::new(1, timescale);
    let duration_ticks = if mdhd_duration > 0 {
        Some(mdhd_duration as i64)
    } else if tkhd_duration > 0 {
        Some(tkhd_duration as i64)
    } else {
        None
    };

    let width = if sample_width > 0 { sample_width } else { tkhd_width };
    let height = if sample_height > 0 { sample_height } else { tkhd_height };

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
    }))
}

struct ParsedStbl {
    codec_id: CodecId,
    codec_config: CodecConfig,
    width: u32,
    height: u32,
    sample_table: SampleTable,
}

fn parse_stbl(data: &[u8], file_offset: u64, handler_type: &[u8; 4]) -> Result<ParsedStbl, DemuxError> {
    let mut stsz = None;
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
                let parsed_stsd = parse_stsd(body, offset + header.header_size as u64, handler_type)?;
                codec_id = parsed_stsd.codec_id;
                codec_config = parsed_stsd.codec_config;
                width = parsed_stsd.width;
                height = parsed_stsd.height;
            }
            b"stsz" => {
                stsz = Some(StszTable::parse(body)?);
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
            _ => {}
        }
    }

    let stsz_table = stsz.unwrap_or_default();
    let stsc_table = stsc.unwrap_or_default();
    let co_table = chunk_offsets.unwrap_or_default();
    let stts_table = stts.unwrap_or_default();

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

fn parse_stsd(data: &[u8], file_offset: u64, handler_type: &[u8; 4]) -> Result<ParsedStsd, DemuxError> {
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
        let entry_size = u32::from_be_bytes([data[cursor], data[cursor + 1], data[cursor + 2], data[cursor + 3]]) as usize;
        let entry_fourcc: [u8; 4] = [data[cursor + 4], data[cursor + 5], data[cursor + 6], data[cursor + 7]];
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

                for c_res in BoxIter::new(child_boxes, child_offset) {
                    if let Ok((c_header, c_body, _)) = c_res {
                        if &c_header.box_type == b"avcC" {
                            if let Ok(avcc) = AvcCRecord::parse(c_body) {
                                codec_config = CodecConfig::H264(avcc);
                            }
                        }
                    }
                }
            } else if entry_payload.len() >= 20 {
                // Audio sample entry: 20 bytes fixed fields after 8-byte entry header
                // Child boxes might contain esds
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
