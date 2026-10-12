//! MPEG-2 Transport Stream (MPEG-TS / ISO/IEC 13818-1) demuxer.
//!
//! Provides streaming and file-based demuxing for broadcast, IPTV, and HLS
//! MPEG-TS container files (`.ts`, `.mts`, `.m2ts`).

pub mod demuxer;

pub use demuxer::MpegTsDemuxer;
