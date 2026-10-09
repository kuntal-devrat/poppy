//! EBML / Matroska (MKV/WebM) container demuxer.

pub mod element;
pub mod matroska;
pub mod vint;

pub use matroska::MatroskaDemuxer;
