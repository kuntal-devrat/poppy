//! Fluent configuration builder for high-level media streams.

use std::path::Path;

use crate::async_stream::AsyncMediaStream;
use crate::error::PoppyError;
use crate::pipeline::MediaStream;
use poppy_render::ColorSpace;

/// Configuration builder for synchronous and asynchronous `MediaStream` instances.
#[derive(Debug, Clone)]
pub struct MediaStreamBuilder {
    pub(crate) stream_index: Option<usize>,
    pub(crate) hardware_acceleration: bool,
    pub(crate) software_fallback: bool,
    pub(crate) buffer_capacity: usize,
    pub(crate) color_space: Option<ColorSpace>,
}

impl Default for MediaStreamBuilder {
    fn default() -> Self {
        Self {
            stream_index: None,
            hardware_acceleration: true,
            software_fallback: true,
            buffer_capacity: 16,
            color_space: None,
        }
    }
}

impl MediaStreamBuilder {
    /// Creates a new builder with default settings:
    /// - Hardware acceleration enabled
    /// - Auto-selected primary video stream
    /// - Backpressure queue capacity of 16 frames
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a specific stream track index to demux and decode.
    pub fn stream_index(mut self, index: usize) -> Self {
        self.stream_index = Some(index);
        self
    }

    /// Enable or disable hardware decoder acceleration.
    ///
    /// When disabled, compressed packets are passed through without GPU decoding
    /// unless software fallback is enabled.
    pub fn hardware_acceleration(mut self, enabled: bool) -> Self {
        self.hardware_acceleration = enabled;
        self
    }

    /// Enable or disable software decoder fallback when hardware acceleration is unavailable.
    pub fn software_fallback(mut self, enabled: bool) -> Self {
        self.software_fallback = enabled;
        self
    }

    /// Set the bounded buffer capacity for the asynchronous frame stream.
    ///
    /// Controls backpressure: if the downstream consumer cannot keep up with
    /// decoding speed, the background worker task will wait once `capacity`
    /// unconsumed frames are buffered. Default is 16 frames.
    pub fn buffer_capacity(mut self, capacity: usize) -> Self {
        self.buffer_capacity = capacity.max(1);
        self
    }

    /// Set the target color space for downstream rendering conversion.
    pub fn color_space(mut self, color_space: ColorSpace) -> Self {
        self.color_space = Some(color_space);
        self
    }

    /// Open a synchronous `MediaStream` using the configured settings.
    pub fn open_sync<P: AsRef<Path>>(&self, path: P) -> Result<MediaStream, PoppyError> {
        MediaStream::open_with_builder(path, self)
    }

    /// Open a synchronous `MediaStream` using the configured settings.
    pub fn open<P: AsRef<Path>>(&self, path: P) -> Result<MediaStream, PoppyError> {
        self.open_sync(path)
    }

    /// Open an asynchronous `AsyncMediaStream` using the configured settings.
    ///
    /// Runs demuxing and hardware decoding on a dedicated worker task with
    /// backpressure flow control and precise frame scrubbing capabilities.
    pub async fn open_async<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<AsyncMediaStream, PoppyError> {
        AsyncMediaStream::open_with_builder(path, self).await
    }
}
