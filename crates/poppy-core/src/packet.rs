//! Packet and timestamp types for demuxed media data.

use bytes::Bytes;

use crate::CodecId;
use crate::codec::CodecConfig;

/// Rational timebase for timestamp arithmetic.
///
/// A timestamp `ts` in this timebase represents `ts * (num / den)` seconds.
///
/// # Common timebases
/// - `(1, 90_000)` — MPEG-TS standard (each tick = ~11.1 µs)
/// - `(1, 1_000)` — millisecond precision
/// - `(1, 48_000)` — 48kHz audio sample clock
/// - `(1, 1_000_000)` — microsecond precision
///
/// # Examples
/// ```
/// use poppy_core::TimeBase;
///
/// let tb = TimeBase::new(1, 90_000);
/// assert!((tb.to_seconds(90_000) - 1.0).abs() < 1e-9);
/// assert!((tb.to_seconds(45_000) - 0.5).abs() < 1e-9);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeBase {
    pub num: u32,
    pub den: u32,
}

impl TimeBase {
    /// Create a new timebase. Panics if `den` is zero.
    pub const fn new(num: u32, den: u32) -> Self {
        assert!(den != 0, "TimeBase denominator must not be zero");
        Self { num, den }
    }

    /// Convert a raw timestamp in this timebase to fractional seconds.
    pub fn to_seconds(self, ts: i64) -> f64 {
        (ts as f64) * (self.num as f64) / (self.den as f64)
    }

    /// Convert a raw timestamp in this timebase to whole microseconds.
    pub fn to_micros(self, ts: i64) -> i64 {
        // Use i128 intermediate to avoid overflow for large timestamps
        let micros = (ts as i128) * (self.num as i128) * 1_000_000 / (self.den as i128);
        micros as i64
    }

    /// Convert a duration in seconds to a timestamp in this timebase.
    pub fn from_seconds(self, seconds: f64) -> i64 {
        (seconds * (self.den as f64) / (self.num as f64)).round() as i64
    }
}

impl std::fmt::Display for TimeBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

/// A single demuxed access unit (one compressed video frame or audio packet).
///
/// The `data` field holds a reference-counted byte slice ([`Bytes`]) that points
/// directly into the memory-mapped file or read buffer, avoiding copies.
///
/// For H.264 video in MP4 containers, `data` contains AVCC-framed NAL units
/// (length-prefixed). Use the bitstream filter in `poppy-demux` to convert
/// to Annex-B format (start-code-prefixed) before feeding to hardware decoders.
#[derive(Debug, Clone)]
pub struct Packet {
    /// Index of the stream (track) this packet belongs to.
    /// Corresponds to the index in `Demuxer::streams()`.
    pub stream_index: usize,

    /// Presentation Timestamp — when this frame should be DISPLAYED.
    ///
    /// In units of the stream's [`TimeBase`].
    /// `None` if the container does not provide PTS for this sample
    /// (rare but technically possible in broken files).
    ///
    /// For streams with B-frames, PTS differs from DTS because frames
    /// are decoded out of display order.
    pub pts: Option<i64>,

    /// Decode Timestamp — when this frame should be FED to the decoder.
    ///
    /// In units of the stream's [`TimeBase`].
    /// For streams without B-frames (no `ctts` box), DTS == PTS.
    pub dts: i64,

    /// Duration of this sample in timebase units. 0 if unknown.
    pub duration: u32,

    /// `true` if this is a sync sample (random access point).
    /// For H.264: this means an IDR (Instantaneous Decoder Refresh) NAL unit.
    /// Determined by the `stss` box, or by absence of `stss` (all samples sync).
    pub is_keyframe: bool,

    /// The raw compressed payload bytes.
    ///
    /// Uses [`bytes::Bytes`] for cheap cloning (reference-counted, not copied).
    pub data: Bytes,
}

impl Packet {
    /// Returns the presentation timestamp, falling back to DTS if PTS is absent.
    pub fn pts_or_dts(&self) -> i64 {
        self.pts.unwrap_or(self.dts)
    }

    /// Returns the size of the compressed payload in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// Metadata describing a single track (stream) in a media container.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Zero-based index of this stream in the container.
    pub index: usize,

    /// The codec used for this stream.
    pub codec: CodecId,

    /// The timebase used for all timestamps in this stream.
    pub time_base: TimeBase,

    /// Total duration of this stream in timebase units.
    /// `None` if the duration is unknown (e.g., live streams).
    pub duration_ticks: Option<i64>,

    /// Codec-specific configuration extracted from the container.
    pub config: CodecConfig,
}

impl StreamInfo {
    /// Returns the total duration of this stream in seconds, if known.
    pub fn duration_seconds(&self) -> Option<f64> {
        self.duration_ticks
            .map(|ticks| self.time_base.to_seconds(ticks))
    }

    /// Returns true if this stream contains video.
    pub fn is_video(&self) -> bool {
        matches!(
            self.codec,
            CodecId::H264 | CodecId::Hevc | CodecId::Av1 | CodecId::Vp9
        )
    }

    /// Returns true if this stream contains audio.
    pub fn is_audio(&self) -> bool {
        matches!(
            self.codec,
            CodecId::Aac | CodecId::Opus | CodecId::Mp3 | CodecId::Flac
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timebase_to_seconds() {
        let tb = TimeBase::new(1, 90_000);
        let secs = tb.to_seconds(90_000);
        assert!((secs - 1.0).abs() < 1e-9);
    }

    #[test]
    fn timebase_to_seconds_half() {
        let tb = TimeBase::new(1, 90_000);
        let secs = tb.to_seconds(45_000);
        assert!((secs - 0.5).abs() < 1e-9);
    }

    #[test]
    fn timebase_to_micros() {
        let tb = TimeBase::new(1, 1_000);
        assert_eq!(tb.to_micros(500), 500_000);
    }

    #[test]
    fn timebase_from_seconds() {
        let tb = TimeBase::new(1, 90_000);
        assert_eq!(tb.from_seconds(1.0), 90_000);
    }

    #[test]
    fn timebase_display() {
        let tb = TimeBase::new(1, 48_000);
        assert_eq!(format!("{tb}"), "1/48000");
    }

    #[test]
    fn packet_pts_or_dts_prefers_pts() {
        let pkt = Packet {
            stream_index: 0,
            pts: Some(100),
            dts: 90,
            duration: 10,
            is_keyframe: false,
            data: Bytes::new(),
        };
        assert_eq!(pkt.pts_or_dts(), 100);
    }

    #[test]
    fn packet_pts_or_dts_falls_back_to_dts() {
        let pkt = Packet {
            stream_index: 0,
            pts: None,
            dts: 90,
            duration: 10,
            is_keyframe: false,
            data: Bytes::new(),
        };
        assert_eq!(pkt.pts_or_dts(), 90);
    }
}
