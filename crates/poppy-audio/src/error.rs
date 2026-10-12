use poppy_core::CodecId;
use thiserror::Error;

/// Errors produced during audio decoding or playback operations.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("unsupported audio codec: {0:?}")]
    UnsupportedCodec(CodecId),

    #[error("decoder initialization failed: {0}")]
    InitFailed(String),

    #[error("audio decode error: {0}")]
    DecodeError(String),

    #[error("playback device error: {0}")]
    DeviceError(String),

    #[error("unsupported audio format or configuration: {0}")]
    UnsupportedFormat(String),
}
