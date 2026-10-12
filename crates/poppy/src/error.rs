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

    #[error("audio error: {0}")]
    Audio(#[from] poppy_audio::AudioError),

    #[error("unsupported container format for '{0}'")]
    UnsupportedFormat(String),

    #[error("no streams found in media file")]
    NoStreams,

    #[error("async channel closed or worker terminated: {0}")]
    ChannelClosed(String),

    #[error("seek failed: {0}")]
    SeekFailed(String),

    #[error("invalid seek target: {0}")]
    InvalidSeek(String),
}
