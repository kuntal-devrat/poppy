//! ISOBMFF (MP4) container parser.
//!
//! Implements parsing of ISO Base Media File Format containers as defined in
//! ISO 14496-12, with codec-specific extensions from ISO 14496-15 (AVC/HEVC).

pub mod box_reader;
pub mod fragment;
pub mod moov;
pub mod mp4;
pub mod sample_table;

pub use box_reader::{BoxHeader, parse_fullbox_header};
pub use fragment::parse_moof;
pub use moov::ParsedTrack;
pub use mp4::Mp4Demuxer;
pub use sample_table::{SampleInfo, SampleTable, Stz2Table};
