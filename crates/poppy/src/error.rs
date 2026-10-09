//! Unified error types for the Poppy media pipeline.

/// Root error enum for high-level Poppy operations.
#[derive(Debug, thiserror::Error)]
pub enum PoppyError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("demux error: {0}")]
    Demux(#[from] poppy_demux::DemuxError),

    #[error("hardware decoder error: {0}")]
    Hardware(#[from] poppy_hw::HwError),

    #[error("unsupported container format for '{0}'")]
    UnsupportedFormat(String),

    #[error("no streams found in media file")]
    NoStreams,
}
