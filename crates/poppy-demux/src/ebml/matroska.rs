//! Matroska (MKV) and WebM container demuxer.

use std::fs::File;
use std::path::Path;

use bytes::Bytes;
use poppy_core::codec::{
    AudioConfig, Av1CRecord, AvcCRecord, CodecConfig, CodecId, HvcCRecord, VpcCRecord,
};
use poppy_core::{Packet, StreamInfo, TimeBase};

use super::element::EbmlIter;
use super::vint::read_vint_size;
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
pub const ID_AUDIO_SETTINGS: u32 = 0xE1;
pub const ID_SAMPLING_FREQUENCY: u32 = 0xB5;
pub const ID_CHANNELS: u32 = 0x9F;
pub const ID_BIT_DEPTH: u32 = 0x6264;
pub const ID_CLUSTER: u32 = 0x1F43B675;
pub const ID_CLUSTER_TIMESTAMP: u32 = 0xE7;
pub const ID_SIMPLE_BLOCK: u32 = 0xA3;
pub const ID_BLOCK_GROUP: u32 = 0xA0;
pub const ID_BLOCK: u32 = 0xA1;
pub const ID_BLOCK_DURATION: u32 = 0x9B;
pub const ID_REFERENCE_BLOCK: u32 = 0xFB;
pub const ID_DISCARD_PADDING: u32 = 0x75A2;
pub const ID_CUES: u32 = 0x1C53BB6B;
pub const ID_CUE_POINT: u32 = 0xBB;
pub const ID_CUE_TIME: u32 = 0xB3;
pub const ID_CUE_TRACK_POSITIONS: u32 = 0xB7;
pub const ID_CUE_TRACK: u32 = 0xF7;
pub const ID_CUE_CLUSTER_POSITION: u32 = 0xF1;
pub const ID_DEFAULT_DURATION: u32 = 0x23E383;
pub const ID_CODEC_DELAY: u32 = 0x56AA;
pub const ID_SEEK_PREROLL: u32 = 0x56BB;
pub const ID_VOID: u32 = 0xEC;
pub const ID_CRC32: u32 = 0xBF;
pub const ID_SEEK_HEAD: u32 = 0x114D9B74;

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
    /// Default duration per frame in track ticks (from DefaultDuration).
    pub default_duration_ticks: Option<u64>,
    /// Codec delay (gapless trim) in ticks.
    pub codec_delay_ticks: Option<u64>,
}

/// A parsed media sample from a Matroska Cluster.
/// Each laced frame is expanded to its own sample (previously merged).
#[derive(Debug, Clone)]
pub struct MatroskaSample {
    pub track_num: u64,
    pub timestamp_ticks: i64,
    pub duration_ticks: u32,
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
            // Bounds are checked by callers; debug-assert to catch corrupt
            // data_range before panicking in release.
            Self::Mmap(m) => {
                debug_assert!(start.saturating_add(len) <= m.len());
                Bytes::copy_from_slice(&m[start..start + len])
            }
            Self::Bytes(b) => b.slice(start..start + len),
        }
    }
}

/// Build a Matroska timebase from TimestampScale (ns per tick).
/// Standard 1_000_000 fits in u32; larger custom scales are reduced by GCD
/// so the ratio is preserved without truncation.
fn matroska_timebase(timestamp_scale_ns: u64) -> TimeBase {
    const DEN: u64 = 1_000_000_000;
    if timestamp_scale_ns == 0 {
        return TimeBase::new(1, 1_000_000);
    }
    if timestamp_scale_ns <= u32::MAX as u64 {
        return TimeBase::new(timestamp_scale_ns as u32, DEN as u32);
    }
    // Reduce by GCD to fit in u32.
    fn gcd(mut a: u64, mut b: u64) -> u64 {
        while b != 0 {
            let t = b;
            b = a % b;
            a = t;
        }
        a.max(1)
    }
    let g = gcd(timestamp_scale_ns, DEN);
    let mut num = timestamp_scale_ns / g;
    let mut den = DEN / g;
    // Further reduce by powers of 10 if still too large.
    while num > u32::MAX as u64 {
        num = num.div_ceil(10);
        den = den.div_ceil(10);
    }
    TimeBase::new(num as u32, den.max(1) as u32)
}

/// Strict CodecPrivate parsing branched on CodecID.
/// Previously tried AvcC→HvcC→Av1C→VpcC blindly, misclassifying HEVC as H264.
fn parse_codec_private(codec_id_str: &str, payload: &[u8]) -> Option<CodecConfig> {
    match codec_id_str {
        "V_MPEG4/ISO/AVC" => AvcCRecord::parse(payload).ok().map(CodecConfig::H264),
        "V_MPEGH/ISO/HEVC" => HvcCRecord::parse(payload).ok().map(CodecConfig::Hevc),
        "V_AV1" => Av1CRecord::parse(payload).ok().map(CodecConfig::Av1),
        "V_VP9" => VpcCRecord::parse(payload).ok().map(CodecConfig::Vp9),
        _ => {
            // Fallback: try each in codec-specific order only for unknown
            // IDs, but require minimum lengths to avoid false positives.
            if payload.len() >= 7
                && let Ok(r) = AvcCRecord::parse(payload)
            {
                return Some(CodecConfig::H264(r));
            }
            if payload.len() >= 23
                && let Ok(r) = HvcCRecord::parse(payload)
            {
                return Some(CodecConfig::Hevc(r));
            }
            None
        }
    }
}

/// Parse laced frame sizes per Matroska spec §Block lacing.
/// `data` starts at the first laced byte (after track+timestamp+flags).
/// Returns frame byte ranges relative to `data`, or None if malformed.
/// Lacing types: 0=no, 1=Xiph, 3=Fixed, 2=EBML.
fn parse_lacing(data: &[u8], lace_type: u8) -> Option<Vec<(usize, usize)>> {
    if lace_type == 0 {
        if data.is_empty() {
            return None;
        }
        return Some(vec![(0, data.len())]);
    }
    if data.is_empty() {
        return None;
    }
    let frame_count = data[0] as usize + 1;
    if frame_count == 0 || frame_count > 32 {
        return None; // pathological
    }
    let mut cursor = 1usize;
    let mut sizes: Vec<usize> = Vec::with_capacity(frame_count);
    match lace_type {
        1 => {
            // Xiph: frame_count-1 sizes as 255-terminated varints.
            for _ in 0..frame_count - 1 {
                let mut sz = 0usize;
                loop {
                    if cursor >= data.len() {
                        return None;
                    }
                    let b = data[cursor] as usize;
                    cursor += 1;
                    sz = sz.saturating_add(b);
                    if b != 255 {
                        break;
                    }
                    if sz > 16_000_000 {
                        return None;
                    }
                }
                sizes.push(sz);
            }
        }
        3 => {
            // Fixed: all frames same size except last; size derived from remaining.
            // Need total remaining after lace byte; sizes computed below.
        }
        2 => {
            // EBML: first size as EBML vint, rest as signed offsets.
            if cursor >= data.len() {
                return None;
            }
            // Read first EBML-coded size manually (1-8 bytes).
            let first_len = (data[cursor].leading_zeros() as usize) + 1;
            if first_len > 8 || cursor + first_len > data.len() {
                return None;
            }
            let mask = if first_len == 8 {
                0x01
            } else {
                (1u8 << (8 - first_len)) - 1
            };
            let mut val = (data[cursor] & mask) as usize;
            for &b in &data[cursor + 1..cursor + first_len] {
                val = (val << 8) | b as usize;
            }
            cursor += first_len;
            sizes.push(val);
            let mut prev = val;
            for _ in 1..frame_count - 1 {
                if cursor >= data.len() {
                    return None;
                }
                // Signed EBML: bias by 2^(7*len-1)-1.
                let len = (data[cursor].leading_zeros() as usize) + 1;
                if len > 8 || cursor + len > data.len() {
                    return None;
                }
                let mask2 = if len == 8 {
                    0x01
                } else {
                    (1u8 << (8 - len)) - 1
                };
                let mut uval = (data[cursor] & mask2) as i64;
                for &b in &data[cursor + 1..cursor + len] {
                    uval = (uval << 8) | b as i64;
                }
                cursor += len;
                let bias = (1i64 << (7 * len as i64 - 1)) - 1;
                let cur = (prev as i64 + (uval - bias)) as usize;
                sizes.push(cur);
                prev = cur;
            }
        }
        _ => return None,
    }
    if lace_type == 3 {
        // Fixed-size: divide remaining equally.
        let remaining = data.len().saturating_sub(cursor);
        let each = remaining / frame_count;
        let mut off = cursor;
        let mut out = Vec::with_capacity(frame_count);
        for i in 0..frame_count {
            let len = if i + 1 == frame_count {
                data.len().saturating_sub(off)
            } else {
                each
            };
            if off + len > data.len() {
                return None;
            }
            out.push((off, len));
            off += len;
        }
        return Some(out);
    }
    // Xiph/EBML: last frame takes remainder.
    let mut out = Vec::with_capacity(frame_count);
    let mut off = cursor;
    for sz in sizes {
        if off + sz > data.len() {
            return None;
        }
        out.push((off, sz));
        off += sz;
    }
    // Last frame.
    if off > data.len() {
        return None;
    }
    out.push((off, data.len() - off));
    if out.len() != frame_count {
        return None;
    }
    Some(out)
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
        // SAFETY: see Mp4Demuxer::open — read-only mapping, no concurrent truncation.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };
        Self::init(Source::Mmap(mmap))
    }

    /// Opens from any `Read + Seek` source without requiring mmap.
    /// See `Mp4Demuxer::open_streaming` for semantics and 2 GiB cap.
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
        Self::from_bytes(Bytes::from(buf))
    }

    pub fn from_bytes(bytes: Bytes) -> Result<Self, DemuxError> {
        Self::init(Source::Bytes(bytes))
    }

    fn init(source: Source) -> Result<Self, DemuxError> {
        let data = source.as_slice();

        // 1. Verify EBML header, skipping leading Void/Crc32 if present.
        let mut iter = EbmlIter::new_top_level(data, 0);
        let mut header_elem = None;
        let mut segment_elem = None;
        for _ in 0..8 {
            let elem = iter.next().ok_or(DemuxError::UnsupportedFormat)??;
            if elem.id == ID_EBML_HEADER {
                header_elem = Some(elem);
                break;
            } else if elem.id == ID_VOID || elem.id == ID_CRC32 {
                continue;
            } else {
                return Err(DemuxError::UnsupportedFormat);
            }
        }
        let _header = header_elem.ok_or(DemuxError::UnsupportedFormat)?;

        // 2. Find Segment element (skip SeekHead/Void between header and Segment).
        for _ in 0..8 {
            let elem = iter.next().ok_or(DemuxError::BoxNotFound {
                fourcc: "Segment".into(),
            })??;
            if elem.id == ID_SEGMENT {
                segment_elem = Some(elem);
                break;
            } else if elem.id == ID_VOID || elem.id == ID_CRC32 || elem.id == ID_SEEK_HEAD {
                continue;
            } else {
                return Err(DemuxError::UnsupportedFormat);
            }
        }
        let segment_elem = segment_elem.ok_or(DemuxError::BoxNotFound {
            fourcc: "Segment".into(),
        })?;

        // Segment payload offset in the original file for absolute data_range.
        let _segment_payload_base: usize = {
            // segment_elem.payload borrows from data; compute offset.
            // SAFETY: payload is a subslice of data by construction.

            segment_elem.payload.as_ptr() as usize - data.as_ptr() as usize
        };

        let mut timestamp_scale_ns = 1_000_000u64; // Default 1ms = 1,000,000 ns
        let mut segment_duration_ticks: Option<u64> = None;
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
                        match info_elem.id {
                            ID_TIMESTAMP_SCALE => {
                                let v = super::element::try_read_uint(info_elem.payload)
                                    .unwrap_or(1_000_000);
                                if v > 0 {
                                    timestamp_scale_ns = v;
                                }
                            }
                            ID_DURATION => {
                                // Duration is float (4 or 8 bytes) in TimestampScale ticks.
                                if info_elem.payload.len() == 4 {
                                    let f = f32::from_be_bytes([
                                        info_elem.payload[0],
                                        info_elem.payload[1],
                                        info_elem.payload[2],
                                        info_elem.payload[3],
                                    ]);
                                    if f.is_finite() && f > 0.0 {
                                        segment_duration_ticks = Some(f as u64);
                                    }
                                } else if info_elem.payload.len() == 8 {
                                    let f = f64::from_be_bytes([
                                        info_elem.payload[0],
                                        info_elem.payload[1],
                                        info_elem.payload[2],
                                        info_elem.payload[3],
                                        info_elem.payload[4],
                                        info_elem.payload[5],
                                        info_elem.payload[6],
                                        info_elem.payload[7],
                                    ]);
                                    if f.is_finite() && f > 0.0 {
                                        segment_duration_ticks = Some(f as u64);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ID_TRACKS => {
                    for tracks_res in EbmlIter::new(elem.payload, elem.offset) {
                        let track_elem = tracks_res?;
                        if track_elem.id == ID_TRACK_ENTRY {
                            let mut track_num = 1u64;
                            let mut track_type = 1u64;
                            let mut codec_str = String::new();
                            let mut codec_private: Option<Bytes> = None;
                            let mut width = 0u32;
                            let mut height = 0u32;

                            // Audio settings: None until explicitly parsed.
                            // Previously defaulted to 2ch/48kHz which hid missing metadata.
                            let mut audio_channels: Option<u16> = None;
                            let mut audio_sample_rate: Option<u32> = None;
                            let mut audio_bit_depth: Option<u16> = None;
                            let mut default_duration: Option<u64> = None;
                            let mut codec_delay: Option<u64> = None;

                            for entry_res in EbmlIter::new(track_elem.payload, track_elem.offset) {
                                let entry_elem = entry_res?;
                                match entry_elem.id {
                                    ID_TRACK_NUMBER => {
                                        track_num =
                                            super::element::try_read_uint(entry_elem.payload)
                                                .unwrap_or(1)
                                    }
                                    ID_TRACK_TYPE => {
                                        track_type =
                                            super::element::try_read_uint(entry_elem.payload)
                                                .unwrap_or(1)
                                    }
                                    ID_CODEC_ID => {
                                        codec_str =
                                            super::element::try_read_utf8(entry_elem.payload)
                                                .unwrap_or_default()
                                    }
                                    ID_CODEC_PRIVATE => {
                                        codec_private =
                                            Some(Bytes::copy_from_slice(entry_elem.payload));
                                    }
                                    ID_DEFAULT_DURATION => {
                                        default_duration =
                                            super::element::try_read_uint(entry_elem.payload).ok();
                                    }
                                    ID_CODEC_DELAY => {
                                        codec_delay =
                                            super::element::try_read_uint(entry_elem.payload).ok();
                                    }
                                    ID_VIDEO_SETTINGS => {
                                        for v_res in
                                            EbmlIter::new(entry_elem.payload, entry_elem.offset)
                                        {
                                            let v_elem = v_res?;
                                            match v_elem.id {
                                                ID_PIXEL_WIDTH => {
                                                    width = super::element::try_read_uint(
                                                        v_elem.payload,
                                                    )
                                                    .unwrap_or(0)
                                                        as u32
                                                }
                                                ID_PIXEL_HEIGHT => {
                                                    height = super::element::try_read_uint(
                                                        v_elem.payload,
                                                    )
                                                    .unwrap_or(0)
                                                        as u32
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    ID_AUDIO_SETTINGS => {
                                        for a_res in
                                            EbmlIter::new(entry_elem.payload, entry_elem.offset)
                                        {
                                            let a_elem = a_res?;
                                            match a_elem.id {
                                                ID_CHANNELS => {
                                                    audio_channels = Some(
                                                        super::element::try_read_uint(
                                                            a_elem.payload,
                                                        )
                                                        .unwrap_or(0)
                                                            as u16,
                                                    );
                                                }
                                                ID_SAMPLING_FREQUENCY => {
                                                    let f = if a_elem.payload.len() == 4 {
                                                        f32::from_be_bytes([
                                                            a_elem.payload[0],
                                                            a_elem.payload[1],
                                                            a_elem.payload[2],
                                                            a_elem.payload[3],
                                                        ])
                                                            as u32
                                                    } else if a_elem.payload.len() == 8 {
                                                        f64::from_be_bytes([
                                                            a_elem.payload[0],
                                                            a_elem.payload[1],
                                                            a_elem.payload[2],
                                                            a_elem.payload[3],
                                                            a_elem.payload[4],
                                                            a_elem.payload[5],
                                                            a_elem.payload[6],
                                                            a_elem.payload[7],
                                                        ])
                                                            as u32
                                                    } else {
                                                        super::element::try_read_uint(
                                                            a_elem.payload,
                                                        )
                                                        .unwrap_or(0)
                                                            as u32
                                                    };
                                                    if f > 0 {
                                                        audio_sample_rate = Some(f);
                                                    }
                                                }
                                                ID_BIT_DEPTH => {
                                                    audio_bit_depth = Some(
                                                        super::element::try_read_uint(
                                                            a_elem.payload,
                                                        )
                                                        .unwrap_or(0)
                                                            as u16,
                                                    );
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }

                            // Strict CodecPrivate branching by CodecID.
                            let mut codec_config = codec_private
                                .as_deref()
                                .and_then(|p| parse_codec_private(&codec_str, p))
                                .unwrap_or(CodecConfig::Unknown);

                            if track_type == 2 {
                                // Audio track: preserve CodecPrivate as extradata;
                                // do NOT overwrite a successfully parsed video config
                                // on mis-typed tracks. Require explicit audio settings;
                                // fall back to Opus/AAC defaults only when absent.
                                let ch = audio_channels.unwrap_or(2);
                                let sr = audio_sample_rate.unwrap_or(48000);
                                let bd = audio_bit_depth.unwrap_or(16);
                                let extradata = codec_private.clone();
                                // If CodecPrivate was actually an audio-specific
                                // config (e.g. AAC ASC), keep it; otherwise None
                                // is fine — decoder will report UnsupportedCodec
                                // instead of silently using wrong defaults.
                                codec_config = CodecConfig::Audio(AudioConfig {
                                    sample_rate: sr,
                                    channels: ch,
                                    bits_per_sample: bd,
                                    extradata,
                                });
                            } else if matches!(codec_config, CodecConfig::Unknown) {
                                // No fake VP9 config: leave Unknown so the
                                // pipeline reports UnsupportedCodec instead of
                                // lying about 8-bit BT.709.
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
                                default_duration_ticks: default_duration,
                                codec_delay_ticks: codec_delay,
                            });
                        }
                    }
                }
                ID_CLUSTER => {
                    let mut cluster_time = 0i64;
                    // Helper closure to expand a Block/SimpleBlock payload into samples.
                    // Returns Vec of (timestamp, duration, is_keyframe, data_range).
                    let data_ptr = data.as_ptr() as usize;
                    let push_block =
                        |block_payload: &[u8],
                         _block_offset: u64,
                         is_simple: bool,
                         ref_block: Option<i64>,
                         block_duration: Option<u32>,
                         cluster_time: i64,
                         samples: &mut Vec<MatroskaSample>| {
                            // Track number is EBML vint (not fixed vint-size).
                            let (trk_opt, trk_len) = match read_vint_size(block_payload, 0) {
                                Ok(v) => v,
                                Err(_) => return,
                            };
                            let trk = match trk_opt {
                                Some(v) => v,
                                None => return,
                            };
                            if block_payload.len() < trk_len + 3 {
                                return;
                            }
                            let rel_time = i16::from_be_bytes([
                                block_payload[trk_len],
                                block_payload[trk_len + 1],
                            ]) as i64;
                            let flags = block_payload[trk_len + 2];
                            let lace_bits = (flags >> 1) & 0x03;
                            // Keyframe: SimpleBlock uses flags&0x80; Block inside
                            // BlockGroup uses ReferenceBlock (0 refs = keyframe).
                            let is_keyframe = if is_simple {
                                (flags & 0x80) != 0
                            } else {
                                ref_block.map(|r| r == 0).unwrap_or((flags & 0x80) != 0)
                            };
                            let base_ts = cluster_time.saturating_add(rel_time);
                            // Laced payload starts after track+timestamp+flags.
                            let lace_data_start = trk_len + 3;
                            if lace_data_start > block_payload.len() {
                                return;
                            }
                            let lace_data = &block_payload[lace_data_start..];
                            let frames = parse_lacing(lace_data, lace_bits).unwrap_or_default();
                            // Absolute base of lace_data in the file.
                            let lace_abs_base =
                                (block_payload.as_ptr() as usize - data_ptr) + lace_data_start;
                            for (i, (off, len)) in frames.iter().enumerate() {
                                let abs_start = lace_abs_base + off;
                                let abs_end = abs_start.saturating_add(*len);
                                if abs_end > data.len() || len == &0usize {
                                    continue;
                                }
                                // Per-frame timestamp: laced frames share the block
                                // timestamp; duration from BlockDuration/default.
                                // For video lacing this is correct (all same ts);
                                // for audio, DefaultDuration splits would need
                                // per-frame offsets — approximated by duration.
                                let ts = base_ts;
                                let dur = block_duration.unwrap_or(0);
                                // DTS==PTS here; B-frame ReferenceBlock delay is
                                // applied as DTS skew below when ref present.
                                let (pts, dts) = if ref_block.is_some_and(|r| r != 0) {
                                    // Backward reference: decode earlier than present.
                                    // Approximate DTS = PTS + ref (ref negative).
                                    (ts, ts.saturating_add(ref_block.unwrap_or(0)))
                                } else {
                                    (ts, ts)
                                };
                                let _ = (i, pts, dts);
                                samples.push(MatroskaSample {
                                    track_num: trk,
                                    timestamp_ticks: ts,
                                    duration_ticks: dur,
                                    is_keyframe: if i == 0 { is_keyframe } else { false },
                                    data_range: (abs_start, *len),
                                });
                            }
                        };

                    for cluster_res in EbmlIter::new(elem.payload, elem.offset) {
                        let c_elem = match cluster_res {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        if c_elem.id == ID_CLUSTER_TIMESTAMP {
                            cluster_time =
                                super::element::try_read_uint(c_elem.payload).unwrap_or(0) as i64;
                        } else if c_elem.id == ID_SIMPLE_BLOCK {
                            push_block(
                                c_elem.payload,
                                c_elem.offset,
                                true,
                                None,
                                None,
                                cluster_time,
                                &mut samples,
                            );
                        } else if c_elem.id == ID_BLOCK_GROUP {
                            let mut blk: Option<&[u8]> = None;
                            let mut blk_off = 0u64;
                            let mut blk_dur: Option<u32> = None;
                            let mut blk_ref: Option<i64> = None;
                            for bg_res in EbmlIter::new(c_elem.payload, c_elem.offset) {
                                let bg_elem = match bg_res {
                                    Ok(e) => e,
                                    Err(_) => continue,
                                };
                                match bg_elem.id {
                                    ID_BLOCK => {
                                        blk = Some(bg_elem.payload);
                                        blk_off = bg_elem.offset;
                                    }
                                    ID_BLOCK_DURATION => {
                                        blk_dur = super::element::try_read_uint(bg_elem.payload)
                                            .ok()
                                            .map(|v| v as u32);
                                    }
                                    ID_REFERENCE_BLOCK
                                        // Signed integer (EBML signed).
                                        if !bg_elem.payload.is_empty() => {
                                            // Decode as signed big-endian.
                                            let mut v: i64 = 0;
                                            let neg = (bg_elem.payload[0] & 0x80) != 0;
                                            for &b in bg_elem.payload {
                                                v = (v << 8) | b as i64;
                                            }
                                            if neg {
                                                // Sign-extend from payload len.
                                                let bits = bg_elem.payload.len() * 8;
                                                v -= 1i64 << bits;
                                            }
                                            blk_ref = Some(v);
                                        }
                                    _ => {}
                                }
                            }
                            if let Some(b) = blk {
                                push_block(
                                    b,
                                    blk_off,
                                    false,
                                    blk_ref,
                                    blk_dur,
                                    cluster_time,
                                    &mut samples,
                                );
                            }
                        } else if c_elem.id == ID_VOID || c_elem.id == ID_CRC32 {
                            continue;
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
                                    ID_CUE_TIME => {
                                        cue_time = super::element::try_read_uint(cp_elem.payload)
                                            .unwrap_or(0)
                                    }
                                    ID_CUE_TRACK_POSITIONS => {
                                        for pos_res in
                                            EbmlIter::new(cp_elem.payload, cp_elem.offset)
                                        {
                                            let pos_elem = pos_res?;
                                            match pos_elem.id {
                                                ID_CUE_TRACK => {
                                                    cue_track = super::element::try_read_uint(
                                                        pos_elem.payload,
                                                    )
                                                    .unwrap_or(1)
                                                }
                                                ID_CUE_CLUSTER_POSITION => {
                                                    cluster_pos = super::element::try_read_uint(
                                                        pos_elem.payload,
                                                    )
                                                    .unwrap_or(0);
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

        // Matroska timebase without u64->u32 truncation.
        let time_base = matroska_timebase(timestamp_scale_ns);

        // Drop samples referencing unknown tracks (previously misattributed to stream 0).
        samples.retain(|s| tracks.iter().any(|t| t.track_num == s.track_num));
        // Sort cues by time for seeking.
        cues.sort_by_key(|c| c.time_ticks);
        // Fill per-sample duration from BlockDuration or DefaultDuration.
        for s in &mut samples {
            if s.duration_ticks == 0
                && let Some(tr) = tracks.iter().find(|t| t.track_num == s.track_num)
                && let Some(dd) = tr.default_duration_ticks
            {
                // DefaultDuration is in nanoseconds; convert to ticks.
                // ticks = ns / timestamp_scale_ns.
                if timestamp_scale_ns > 0 {
                    s.duration_ticks = (dd / timestamp_scale_ns.max(1)) as u32;
                }
            }
        }

        // Segment duration (float ticks) -> duration_ticks in timebase units.
        let segment_duration_as_ticks = segment_duration_ticks;

        let stream_infos: Vec<StreamInfo> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| StreamInfo {
                index: i,
                codec: t.codec_id,
                time_base,
                duration_ticks: segment_duration_as_ticks.map(|v| v as i64),
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

    /// Number of tracks.
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }
}

impl Demuxer for MatroskaDemuxer {
    fn streams(&self) -> &[StreamInfo] {
        &self.stream_infos
    }

    fn read_packet(&mut self) -> Result<Option<Packet>, DemuxError> {
        while self.cursor < self.samples.len() {
            let s = &self.samples[self.cursor];
            self.cursor += 1;

            let stream_index = match self.tracks.iter().position(|t| t.track_num == s.track_num) {
                Some(i) => i,
                None => continue, // skip unknown-track samples instead of misattributing to 0
            };

            let (start, len) = s.data_range;
            let total = self.source.as_slice().len();
            let end = start
                .checked_add(len)
                .ok_or_else(|| DemuxError::InvalidBox {
                    offset: start as u64,
                    detail: "matroska sample end overflows".into(),
                })?;
            if end > total {
                return Err(DemuxError::UnexpectedEof {
                    offset: start as u64,
                });
            }
            let data = self.source.extract(start, len);

            return Ok(Some(Packet {
                stream_index,
                pts: Some(s.timestamp_ticks),
                dts: s.timestamp_ticks,
                duration: s.duration_ticks,
                is_keyframe: s.is_keyframe,
                data,
            }));
        }
        Ok(None)
    }

    fn seek_to_keyframe(&mut self, stream_index: usize, timestamp: i64) -> Result<(), DemuxError> {
        let track_num = self.tracks.get(stream_index).map(|t| t.track_num).ok_or(
            DemuxError::SampleOutOfRange {
                index: stream_index as u32,
                total: self.tracks.len() as u32,
            },
        )?;

        // Prefer cues if available: find the greatest cue time <= timestamp for this track.
        let mut cue_hint: Option<i64> = None;
        for c in &self.cues {
            if c.track_num == track_num && (c.time_ticks as i64) <= timestamp {
                cue_hint = Some(c.time_ticks as i64);
            }
        }
        let search_ts = cue_hint.unwrap_or(timestamp);

        // Samples are in file (cluster) order, not globally sorted when
        // tracks interleave. Scan all for the keyframe with greatest PTS <= ts.
        let mut best_idx: Option<usize> = None;
        let mut best_ts = i64::MIN;
        let mut first_key_idx: Option<usize> = None;
        for (i, s) in self.samples.iter().enumerate() {
            if s.track_num != track_num || !s.is_keyframe {
                continue;
            }
            if first_key_idx.is_none() {
                first_key_idx = Some(i);
            }
            if s.timestamp_ticks <= search_ts && s.timestamp_ticks >= best_ts {
                best_ts = s.timestamp_ticks;
                best_idx = Some(i);
            }
        }
        // If cue_hint was used and found nothing <= hint, fall back to full timestamp search.
        if best_idx.is_none() && cue_hint.is_some() {
            for (i, s) in self.samples.iter().enumerate() {
                if s.track_num != track_num || !s.is_keyframe {
                    continue;
                }
                if s.timestamp_ticks <= timestamp && s.timestamp_ticks >= best_ts {
                    best_ts = s.timestamp_ticks;
                    best_idx = Some(i);
                }
            }
        }

        self.cursor = best_idx.or(first_key_idx).unwrap_or(0);
        Ok(())
    }

    fn estimated_frame_duration_seconds(&self, stream_index: usize) -> Option<f64> {
        let tr = self.tracks.get(stream_index)?;
        if let Some(dd_ns) = tr.default_duration_ticks.filter(|&d| d > 0) {
            let sec = dd_ns as f64 / 1_000_000_000.0;
            if sec > 0.0 && sec.is_finite() {
                return Some(sec);
            }
        }
        let stream_info = self.stream_infos.get(stream_index)?;
        let mut sample_count = 0usize;
        let mut total_duration = 0u64;
        for s in &self.samples {
            if s.track_num == tr.track_num {
                sample_count += 1;
                if s.duration_ticks > 0 {
                    total_duration += s.duration_ticks as u64;
                }
            }
        }
        if sample_count > 0 && total_duration > 0 {
            let avg_ticks = (total_duration as f64) / (sample_count as f64);
            let sec = stream_info.time_base.to_seconds(avg_ticks.round() as i64);
            if sec > 0.0 && sec.is_finite() {
                return Some(sec);
            }
        }
        None
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

        let track_entry = make_ebml_master(
            ID_TRACK_ENTRY,
            &[
                // TrackNumber = 1
                0xD7, 0x81, 0x01, // TrackType = 1 (video)
                0x83, 0x81, 0x01, // CodecID = "V_MPEG4/ISO/AVC"
                0x86, 0x8F, b'V', b'_', b'M', b'P', b'E', b'G', b'4', b'/', b'I', b'S', b'O', b'/',
                b'A', b'V', b'C',
            ],
        );
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
        let cue_track_pos = make_ebml_master(
            ID_CUE_TRACK_POSITIONS,
            &[
                0xF7, 0x81, 0x01, // CueTrack = 1
                0xF1, 0x81, 0x50, // CueClusterPosition = 0x50
            ],
        );
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

        let mut demuxer =
            MatroskaDemuxer::from_bytes(Bytes::from(mkv_bytes)).expect("Failed to demux MKV");
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
