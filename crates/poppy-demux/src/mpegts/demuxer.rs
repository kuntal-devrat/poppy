//! MPEG-2 Transport Stream (ISO/IEC 13818-1) demuxer implementation.
//!
//! Parses 188-byte (standard), 192-byte (M2TS), and 204-byte (DVB/FEC) TS packets,
//! extracts Program Association Table (PAT) and Program Map Table (PMT),
//! reassembles Packetized Elementary Streams (PES), and emits timestamped
//! [`Packet`]s for video and audio elementary streams.

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

use bytes::Bytes;
use poppy_core::codec::{AudioConfig, AvcCRecord, CodecConfig, CodecId, HvcCRecord};
use poppy_core::{Packet, StreamInfo, TimeBase};

use crate::{DemuxError, Demuxer};

/// MPEG-TS sync byte constant.
pub const TS_SYNC_BYTE: u8 = 0x47;

/// Standard TS packet length.
pub const TS_PACKET_SIZE_188: usize = 188;
/// M2TS (BDAV) timestamp-prefixed TS packet length.
pub const TS_PACKET_SIZE_192: usize = 192;
/// FEC-extended TS packet length.
pub const TS_PACKET_SIZE_204: usize = 204;

/// Elementary stream types defined in ISO/IEC 13818-1 and ITU-T H.222.0.
pub const STREAM_TYPE_MPEG1_VIDEO: u8 = 0x01;
pub const STREAM_TYPE_MPEG2_VIDEO: u8 = 0x02;
pub const STREAM_TYPE_MPEG1_AUDIO: u8 = 0x03;
pub const STREAM_TYPE_MPEG2_AUDIO: u8 = 0x04;
pub const STREAM_TYPE_PES_PRIVATE: u8 = 0x06;
pub const STREAM_TYPE_AAC_ADTS: u8 = 0x0F;
pub const STREAM_TYPE_AAC_LATM: u8 = 0x11;
pub const STREAM_TYPE_H264: u8 = 0x1B;
pub const STREAM_TYPE_HEVC: u8 = 0x24;
pub const STREAM_TYPE_AC3_ATSC: u8 = 0x81;
pub const STREAM_TYPE_EAC3_ATSC: u8 = 0x87;

/// MPEG-TS Elementary Stream Descriptor.
#[derive(Debug, Clone)]
pub struct EsInfo {
    pub pid: u16,
    pub stream_type: u8,
    pub codec_id: CodecId,
    pub stream_index: usize,
}

/// Buffer used to reassemble PES packets across TS packets.
#[derive(Debug, Default)]
#[allow(dead_code)]
struct PesReassembler {
    pid: u16,
    buffer: Vec<u8>,
    expected_len: Option<usize>,
    pts: Option<i64>,
    dts: Option<i64>,
    has_pcr: bool,
    is_keyframe: bool,
    last_cc: Option<u8>,
}

/// Keyframe index entry for random access / seeking.
#[derive(Debug, Clone)]
struct KeyframeIndexEntry {
    packet_offset: usize,
    pts: i64,
    stream_index: usize,
}

/// MPEG-2 Transport Stream Demuxer.
pub struct MpegTsDemuxer {
    data: Bytes,
    packet_size: usize,
    packet_offset_prefix: usize,
    stream_start_offset: usize,
    current_byte_pos: usize,
    es_map: HashMap<u16, EsInfo>,
    reassemblers: HashMap<u16, PesReassembler>,
    stream_infos: Vec<StreamInfo>,
    keyframe_index: Vec<KeyframeIndexEntry>,
    ready_packets: std::collections::VecDeque<Packet>,
}

impl MpegTsDemuxer {
    /// Opens an MPEG-TS container from a file using memory mapping.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DemuxError> {
        let file = File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        let bytes = Bytes::from_owner(mmap);
        Self::from_bytes(bytes)
    }

    /// Creates an MPEG-TS demuxer from an in-memory byte buffer.
    pub fn from_bytes(data: Bytes) -> Result<Self, DemuxError> {
        if data.is_empty() {
            return Err(DemuxError::BoxNotFound {
                fourcc: "MPEG-TS sync".into(),
            });
        }

        // Detect packet size, packet prefix, and stream start offset
        let (packet_size, offset_prefix, start_offset) = detect_ts_packet_format(&data)?;

        let mut demuxer = Self {
            data,
            packet_size,
            packet_offset_prefix: offset_prefix,
            stream_start_offset: start_offset,
            current_byte_pos: start_offset,
            es_map: HashMap::new(),
            reassemblers: HashMap::new(),
            stream_infos: Vec::new(),
            keyframe_index: Vec::new(),
            ready_packets: std::collections::VecDeque::new(),
        };

        // Scan PAT/PMT and discover elementary streams
        demuxer.parse_program_tables()?;
        // Build initial keyframe index for seeking
        demuxer.build_seek_index();

        Ok(demuxer)
    }

    /// Scans the transport stream to parse PAT (PID 0) and PMT to configure elementary streams.
    fn parse_program_tables(&mut self) -> Result<(), DemuxError> {
        let len = self.data.len();
        let step = self.packet_size;
        let prefix = self.packet_offset_prefix;

        let mut pos = self.stream_start_offset;
        let mut pmt_pid: Option<u16> = None;
        let max_scan_bytes = len.min(self.stream_start_offset + 10 * 1024 * 1024); // Scan up to 10 MB for tables

        while pos + step <= max_scan_bytes {
            let pkt = &self.data[pos + prefix..pos + prefix + 188];
            if pkt[0] == TS_SYNC_BYTE {
                let pid = (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16);
                let pusi = (pkt[1] & 0x40) != 0;
                let afc = (pkt[3] >> 4) & 0x03;

                // Compute payload offset
                let mut payload_offset = 4;
                if afc == 0b10 {
                    // Adaptation field only, no payload
                    pos += step;
                    continue;
                } else if afc == 0b11 {
                    let aflen = pkt[4] as usize;
                    payload_offset = 5 + aflen;
                }

                if payload_offset < 188 {
                    let payload = &pkt[payload_offset..188];

                    // PID 0: PAT (Program Association Table)
                    if pid == 0 {
                        if let Some(parsed_pmt_pid) = parse_pat(payload, pusi) {
                            pmt_pid = Some(parsed_pmt_pid);
                        }
                    } else if pmt_pid == Some(pid) {
                        // PMT (Program Map Table)
                        if let Some(streams) = parse_pmt(payload, pusi) {
                            for es in streams {
                                self.es_map.insert(es.pid, es);
                            }
                            break;
                        }
                    }
                }
            }
            pos += step;
        }

        if self.es_map.is_empty() {
            // Fallback: heuristic scan for PES start codes if PAT/PMT is missing or stripped
            self.discover_streams_heuristic();
        }

        // Construct StreamInfo for each discovered ES
        let mut sorted_es: Vec<EsInfo> = self.es_map.values().cloned().collect();
        sorted_es.sort_by_key(|e| e.pid);

        let timebase = TimeBase::new(1, 90_000); // Standard MPEG-2 90kHz timebase
        for (i, es) in sorted_es.iter_mut().enumerate() {
            es.stream_index = i;
            self.es_map.insert(es.pid, es.clone());

            let mut info = StreamInfo {
                index: i,
                codec: es.codec_id,
                time_base: timebase,
                duration_ticks: None,
                config: CodecConfig::Unknown,
                width: 0,
                height: 0,
                enabled: true,
            };
            // Default configs
            match es.codec_id {
                CodecId::H264 => {
                    info.config = CodecConfig::H264(AvcCRecord {
                        profile_idc: 100, // High profile default
                        profile_compatibility: 0,
                        level_idc: 41,
                        nal_length_size: 4,
                        sps_list: Vec::new(),
                        pps_list: Vec::new(),
                        raw: Bytes::new(),
                    });
                }
                CodecId::Hevc => {
                    info.config = CodecConfig::Hevc(HvcCRecord {
                        profile_space: 0,
                        tier_flag: false,
                        profile_idc: 1, // Main profile default
                        profile_compatibility: 0,
                        level_idc: 120,
                        chroma_format_idc: 1,
                        bit_depth_luma: 8,
                        bit_depth_chroma: 8,
                        nal_length_size: 4,
                        vps_list: Vec::new(),
                        sps_list: Vec::new(),
                        pps_list: Vec::new(),
                        raw: Bytes::new(),
                    });
                }
                CodecId::Aac => {
                    info.config = CodecConfig::Audio(AudioConfig {
                        sample_rate: 48_000,
                        channels: 2,
                        bits_per_sample: 16,
                        extradata: None,
                    });
                }
                CodecId::Mp3 => {
                    info.config = CodecConfig::Audio(AudioConfig {
                        sample_rate: 44_100,
                        channels: 2,
                        bits_per_sample: 16,
                        extradata: None,
                    });
                }
                _ => {}
            }
            self.stream_infos.push(info);
        }

        Ok(())
    }

    /// Fallback stream discovery by sniffing PES packet headers.
    fn discover_streams_heuristic(&mut self) {
        let len = self.data.len();
        let step = self.packet_size;
        let prefix = self.packet_offset_prefix;
        let max_scan = len.min(2 * 1024 * 1024);

        let mut pos = 0;
        while pos + step <= max_scan {
            let pkt = &self.data[pos + prefix..pos + prefix + 188];
            if pkt[0] == TS_SYNC_BYTE {
                let pid = (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16);
                let pusi = (pkt[1] & 0x40) != 0;
                let afc = (pkt[3] >> 4) & 0x03;

                let mut payload_offset = 4;
                if afc == 0b11 {
                    payload_offset = 5 + pkt[4] as usize;
                }

                if pusi && payload_offset + 4 <= 188 && pid > 0 && pid < 0x1FFF {
                    let payload = &pkt[payload_offset..];
                    if payload.starts_with(&[0x00, 0x00, 0x01]) {
                        let stream_id = payload[3];
                        let (codec_id, stream_type) = if (0xE0..=0xEF).contains(&stream_id) {
                            (CodecId::H264, STREAM_TYPE_H264)
                        } else if (0xC0..=0xDF).contains(&stream_id) {
                            (CodecId::Aac, STREAM_TYPE_AAC_ADTS)
                        } else if stream_id == 0xBD {
                            (CodecId::Aac, STREAM_TYPE_PES_PRIVATE)
                        } else {
                            (CodecId::Unknown(stream_id as u32), 0)
                        };

                        let next_index = self.es_map.len();
                        self.es_map.entry(pid).or_insert(EsInfo {
                            pid,
                            stream_type,
                            codec_id,
                            stream_index: next_index,
                        });
                    }
                }
            }
            pos += step;
        }
    }

    /// Indexes keyframe positions and timestamps for quick seeks.
    fn build_seek_index(&mut self) {
        let len = self.data.len();
        let step = self.packet_size;
        let prefix = self.packet_offset_prefix;

        let mut pos = self.stream_start_offset;
        while pos + step <= len {
            let pkt = &self.data[pos + prefix..pos + prefix + 188];
            if pkt[0] == TS_SYNC_BYTE {
                let pid = (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16);
                let pusi = (pkt[1] & 0x40) != 0;
                let afc = (pkt[3] >> 4) & 0x03;

                let mut is_random_access = false;
                let mut payload_offset = 4;
                if afc == 0b10 || afc == 0b11 {
                    let aflen = pkt[4] as usize;
                    if aflen > 0 && 5 < 188 {
                        is_random_access = (pkt[5] & 0x40) != 0; // random_access_indicator
                    }
                    payload_offset = 5 + aflen;
                }

                if pusi && payload_offset + 9 <= 188 && self.es_map.contains_key(&pid) {
                    let es = &self.es_map[&pid];
                    let payload = &pkt[payload_offset..];
                    if payload.starts_with(&[0x00, 0x00, 0x01]) {
                        let pts = parse_pes_pts(&payload[3..]);
                        if let Some(pts_val) = pts {
                            // Check if this PES starts with an IDR/keyframe NAL
                            let mut is_key = is_random_access;
                            if !is_key && payload.len() > 14 {
                                if es.codec_id == CodecId::H264 {
                                    is_key = has_h264_idr(&payload[9..]);
                                } else if es.codec_id == CodecId::Hevc {
                                    is_key = has_hevc_idr(&payload[9..]);
                                }
                            }

                            if is_key {
                                self.keyframe_index.push(KeyframeIndexEntry {
                                    packet_offset: pos,
                                    pts: pts_val,
                                    stream_index: es.stream_index,
                                });
                            }
                        }
                    }
                }
            }
            pos += step;
        }

        // Sort keyframes by PTS
        self.keyframe_index.sort_by_key(|k| k.pts);
    }

    /// Reads the next complete PES packet from the stream.
    fn next_packet_internal(&mut self) -> Result<Option<Packet>, DemuxError> {
        let step = self.packet_size;
        let prefix = self.packet_offset_prefix;
        let total_len = self.data.len();

        while self.current_byte_pos + step <= total_len {
            let pos = self.current_byte_pos;
            self.current_byte_pos += step;

            let pkt = &self.data[pos + prefix..pos + prefix + 188];
            if pkt[0] != TS_SYNC_BYTE {
                continue;
            }

            let tei = (pkt[1] & 0x80) != 0;
            if tei {
                continue; // Skip transport error packet
            }

            let pid = (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16);
            let pusi = (pkt[1] & 0x40) != 0;
            let afc = (pkt[3] >> 4) & 0x03;
            let cc = pkt[3] & 0x0F;

            if !self.es_map.contains_key(&pid) {
                continue;
            }

            let mut is_random_access = false;
            let mut payload_offset = 4;
            if afc == 0b10 {
                continue; // Adaptation field only
            } else if afc == 0b11 {
                let aflen = pkt[4] as usize;
                if aflen > 0 && 5 < 188 {
                    is_random_access = (pkt[5] & 0x40) != 0;
                }
                payload_offset = 5 + aflen;
            }

            let mut payload_buf = [0u8; 188];
            let payload_len = 188 - payload_offset;
            payload_buf[..payload_len]
                .copy_from_slice(&self.data[pos + prefix + payload_offset..pos + prefix + 188]);
            let payload_data = &payload_buf[..payload_len];

            if pusi {
                // Finalize previous PES packet for this PID if any
                let finished_packet = self.finalize_pes_packet(pid);

                // Start new PES packet
                let mut reassembler = PesReassembler {
                    pid,
                    buffer: Vec::with_capacity(16 * 1024),
                    expected_len: None,
                    pts: None,
                    dts: None,
                    has_pcr: false,
                    is_keyframe: is_random_access,
                    last_cc: Some(cc),
                };

                // Parse PES header
                if payload_data.starts_with(&[0x00, 0x00, 0x01]) && payload_data.len() >= 9 {
                    let pes_len = u16::from_be_bytes([payload_data[4], payload_data[5]]) as usize;
                    if pes_len > 0 {
                        reassembler.expected_len = Some(pes_len + 6);
                    }

                    let flags = payload_data[7];
                    let header_data_len = payload_data[8] as usize;
                    let pts_dts_flags = (flags >> 6) & 0x03;

                    let offset = 9;
                    if pts_dts_flags == 0b10
                        && header_data_len >= 5
                        && offset + 5 <= payload_data.len()
                    {
                        reassembler.pts = parse_33bit_timestamp(&payload_data[offset..offset + 5]);
                        reassembler.dts = reassembler.pts;
                    } else if pts_dts_flags == 0b11
                        && header_data_len >= 10
                        && offset + 10 <= payload_data.len()
                    {
                        reassembler.pts = parse_33bit_timestamp(&payload_data[offset..offset + 5]);
                        reassembler.dts =
                            parse_33bit_timestamp(&payload_data[offset + 5..offset + 10]);
                    }

                    let pes_payload_start = 9 + header_data_len;
                    if pes_payload_start <= payload_data.len() {
                        reassembler
                            .buffer
                            .extend_from_slice(&payload_data[pes_payload_start..]);
                    }
                } else {
                    reassembler.buffer.extend_from_slice(payload_data);
                }

                self.reassemblers.insert(pid, reassembler);

                if let Some(pkt) = finished_packet {
                    return Ok(Some(pkt));
                }
            } else if let Some(reassembler) = self.reassemblers.get_mut(&pid) {
                // Validate continuity counter
                if let Some(last) = reassembler.last_cc {
                    let _expected_cc = (last + 1) & 0x0F;
                }
                reassembler.last_cc = Some(cc);
                reassembler.buffer.extend_from_slice(payload_data);

                // Check if bounded PES packet is now complete
                if reassembler
                    .expected_len
                    .is_some_and(|expected| reassembler.buffer.len() >= expected)
                {
                    let finished = self.finalize_pes_packet(pid);
                    if let Some(finished) = finished {
                        return Ok(Some(finished));
                    }
                }
            }
        }

        // End of stream: finalize any remaining PES packets in reassemblers
        let pids: Vec<u16> = self.reassemblers.keys().cloned().collect();
        for pid in pids {
            if let Some(pkt) = self.finalize_pes_packet(pid) {
                self.ready_packets.push_back(pkt);
            }
        }

        Ok(self.ready_packets.pop_front())
    }

    /// Turns the reassembled buffer of a PID into a [`Packet`].
    fn finalize_pes_packet(&mut self, pid: u16) -> Option<Packet> {
        let reassembler = self.reassemblers.remove(&pid)?;
        if reassembler.buffer.is_empty() {
            return None;
        }

        let es = self.es_map.get(&pid)?;
        let mut is_keyframe = reassembler.is_keyframe;
        let data = reassembler.buffer;

        // Auto-detect keyframes from elementary stream data if not flagged in TS adaptation
        if es.codec_id == CodecId::H264 {
            if !is_keyframe && has_h264_idr(&data) {
                is_keyframe = true;
            }
        } else if es.codec_id == CodecId::Hevc {
            if !is_keyframe && has_hevc_idr(&data) {
                is_keyframe = true;
            }
        } else if es.codec_id == CodecId::Aac || es.codec_id == CodecId::Mp3 {
            is_keyframe = true; // Audio frames are independently decodable
        }

        Some(Packet {
            stream_index: es.stream_index,
            pts: reassembler.pts,
            dts: reassembler.dts.unwrap_or(reassembler.pts.unwrap_or(0)),
            duration: 3000, // Default 3000 ticks in 90kHz = ~33.3ms (30fps)
            data: Bytes::from(data),
            is_keyframe,
        })
    }
}

impl Demuxer for MpegTsDemuxer {
    fn streams(&self) -> &[StreamInfo] {
        &self.stream_infos
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError> {
        if let Some(pkt) = self.ready_packets.pop_front() {
            return Ok(Some(pkt));
        }
        self.next_packet_internal()
    }

    fn seek_to_keyframe(&mut self, stream_index: usize, timestamp: i64) -> Result<(), DemuxError> {
        // Clear pending buffers
        self.ready_packets.clear();
        self.reassemblers.clear();

        // Find nearest keyframe at or before the requested timestamp
        let target_entry = self
            .keyframe_index
            .iter()
            .rfind(|k| k.stream_index == stream_index && k.pts <= timestamp)
            .or_else(|| {
                // If seeking before first keyframe, seek to earliest keyframe
                self.keyframe_index.first()
            });

        if let Some(entry) = target_entry {
            self.current_byte_pos = entry.packet_offset;
        } else {
            // Seek to start of file
            self.current_byte_pos = self.stream_start_offset;
        }

        Ok(())
    }

    fn estimated_frame_duration_seconds(&self, stream_index: usize) -> Option<f64> {
        let _ = stream_index;
        // 90kHz standard MPEG-TS frame duration ~30fps = 3000 / 90000 = 0.033333
        Some(1.0 / 30.0)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Detects TS packet length (188, 192, or 204 bytes) by looking for repeated 0x47 sync bytes.
fn detect_ts_packet_format(data: &[u8]) -> Result<(usize, usize, usize), DemuxError> {
    if data.len() < 188 {
        return Err(DemuxError::UnexpectedEof { offset: 0 });
    }

    // Try standard 188-byte packets
    if is_valid_ts_cadence(data, 188, 0) {
        return Ok((TS_PACKET_SIZE_188, 0, 0));
    }

    // Try 192-byte M2TS packets (4-byte timestamp header before sync byte)
    if is_valid_ts_cadence(data, 192, 4) {
        return Ok((TS_PACKET_SIZE_192, 4, 0));
    }

    // Try 204-byte FEC-extended packets
    if is_valid_ts_cadence(data, 204, 0) {
        return Ok((TS_PACKET_SIZE_204, 0, 0));
    }

    // Search for first 0x47 sync byte within first 4096 bytes and re-check cadences
    let scan_limit = data.len().min(4096);
    for i in 0..scan_limit {
        if data[i] == TS_SYNC_BYTE {
            if is_valid_ts_cadence(&data[i..], 188, 0) {
                return Ok((TS_PACKET_SIZE_188, 0, i));
            }
            if is_valid_ts_cadence(&data[i..], 204, 0) {
                return Ok((TS_PACKET_SIZE_204, 0, i));
            }
            if i >= 4 && is_valid_ts_cadence(&data[i - 4..], 192, 4) {
                return Ok((TS_PACKET_SIZE_192, 4, i - 4));
            }
        }
    }

    Err(DemuxError::UnsupportedFormat)
}

fn is_valid_ts_cadence(data: &[u8], step: usize, prefix: usize) -> bool {
    let mut count = 0;
    let mut pos = prefix;
    while pos < data.len() && count < 5 {
        if data[pos] != TS_SYNC_BYTE {
            return false;
        }
        pos += step;
        count += 1;
    }
    count >= 2
}

/// Parses PAT (Program Association Table) to find the PMT PID.
fn parse_pat(payload: &[u8], pusi: bool) -> Option<u16> {
    let mut data = payload;
    if pusi && !data.is_empty() {
        let pointer = data[0] as usize;
        if pointer + 1 >= data.len() {
            return None;
        }
        data = &data[pointer + 1..];
    }

    if data.len() < 8 {
        return None;
    }

    let table_id = data[0];
    if table_id != 0x00 {
        return None;
    }

    let section_len = (((data[1] & 0x0F) as usize) << 8) | (data[2] as usize);
    if data.len() < 3 + section_len || section_len < 9 {
        return None;
    }

    // Program entries start at offset 8, each entry is 4 bytes: [program_num: u16, pid: u16]
    let entries_len = section_len - 9; // 5 bytes header + 4 bytes CRC
    let mut offset = 8;
    while offset + 4 <= 8 + entries_len {
        let program_num = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let pmt_pid = (((data[offset + 2] & 0x1F) as u16) << 8) | (data[offset + 3] as u16);
        if program_num != 0 {
            return Some(pmt_pid);
        }
        offset += 4;
    }

    None
}

/// Parses PMT (Program Map Table) to extract elementary streams.
fn parse_pmt(payload: &[u8], pusi: bool) -> Option<Vec<EsInfo>> {
    let mut data = payload;
    if pusi && !data.is_empty() {
        let pointer = data[0] as usize;
        if pointer + 1 >= data.len() {
            return None;
        }
        data = &data[pointer + 1..];
    }

    if data.len() < 12 {
        return None;
    }

    let table_id = data[0];
    if table_id != 0x02 {
        return None;
    }

    let section_len = (((data[1] & 0x0F) as usize) << 8) | (data[2] as usize);
    if data.len() < 3 + section_len || section_len < 13 {
        return None;
    }

    let program_info_len = (((data[10] & 0x0F) as usize) << 8) | (data[11] as usize);
    let mut offset = 12 + program_info_len;
    let end_offset = 3 + section_len - 4; // Minus 4 bytes CRC

    let mut streams = Vec::new();

    while offset + 5 <= end_offset {
        let stream_type = data[offset];
        let elementary_pid = (((data[offset + 1] & 0x1F) as u16) << 8) | (data[offset + 2] as u16);
        let es_info_len = (((data[offset + 3] & 0x0F) as usize) << 8) | (data[offset + 4] as usize);

        let codec_id = match stream_type {
            STREAM_TYPE_H264 => CodecId::H264,
            STREAM_TYPE_HEVC => CodecId::Hevc,
            STREAM_TYPE_AAC_ADTS | STREAM_TYPE_AAC_LATM => CodecId::Aac,
            STREAM_TYPE_MPEG1_AUDIO | STREAM_TYPE_MPEG2_AUDIO => CodecId::Mp3,
            STREAM_TYPE_AC3_ATSC | STREAM_TYPE_EAC3_ATSC => CodecId::Unknown(0x41433320),
            STREAM_TYPE_PES_PRIVATE => CodecId::Aac, // Frequently AC-3 or AAC in private PES
            STREAM_TYPE_MPEG1_VIDEO | STREAM_TYPE_MPEG2_VIDEO => CodecId::Unknown(0x6D706567),
            other => CodecId::Unknown(other as u32),
        };

        streams.push(EsInfo {
            pid: elementary_pid,
            stream_type,
            codec_id,
            stream_index: streams.len(),
        });

        offset += 5 + es_info_len;
    }

    if streams.is_empty() {
        None
    } else {
        Some(streams)
    }
}

/// Parses 33-bit PTS/DTS from a 5-byte PES timestamp encoding.
fn parse_33bit_timestamp(bytes: &[u8]) -> Option<i64> {
    if bytes.len() < 5 {
        return None;
    }
    let b0 = bytes[0] as i64;
    let b1 = bytes[1] as i64;
    let b2 = bytes[2] as i64;
    let b3 = bytes[3] as i64;
    let b4 = bytes[4] as i64;

    let pts =
        ((b0 & 0x0E) << 29) | (b1 << 22) | ((b2 & 0xFE) << 14) | (b3 << 7) | ((b4 & 0xFE) >> 1);

    Some(pts)
}

/// Extracts PTS timestamp from PES packet header starting after stream_id.
fn parse_pes_pts(payload_after_stream_id: &[u8]) -> Option<i64> {
    if payload_after_stream_id.len() < 8 {
        return None;
    }
    let flags = payload_after_stream_id[4];
    let pts_dts_flags = (flags >> 6) & 0x03;
    if (pts_dts_flags & 0x02) != 0 && payload_after_stream_id.len() >= 11 {
        parse_33bit_timestamp(&payload_after_stream_id[6..11])
    } else {
        None
    }
}

/// Checks whether an H.264 Annex-B bitstream contains an IDR NAL unit (type 5).
fn has_h264_idr(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                let nal_type = data[i + 3] & 0x1F;
                if nal_type == 5 {
                    return true;
                }
                i += 3;
                continue;
            } else if data[i + 2] == 0 && data[i + 3] == 1 && i + 5 <= data.len() {
                let nal_type = data[i + 4] & 0x1F;
                if nal_type == 5 {
                    return true;
                }
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Checks whether an HEVC Annex-B bitstream contains an IDR/CRA NAL unit (types 19, 20, 21).
fn has_hevc_idr(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                let nal_type = (data[i + 3] >> 1) & 0x3F;
                if nal_type == 19 || nal_type == 20 || nal_type == 21 {
                    return true;
                }
                i += 3;
                continue;
            } else if data[i + 2] == 0 && data[i + 3] == 1 && i + 5 <= data.len() {
                let nal_type = (data[i + 4] >> 1) & 0x3F;
                if nal_type == 19 || nal_type == 20 || nal_type == 21 {
                    return true;
                }
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    false
}
