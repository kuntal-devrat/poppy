//! Asynchronous media stream pipeline with Tokio flow control and precise frame scrubbing.

use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use poppy_core::{CodecId, StreamInfo, TimeBase};
use poppy_hw::DecodedFrame;
use tokio::sync::{mpsc, oneshot};

use crate::builder::MediaStreamBuilder;
use crate::error::PoppyError;
use crate::pipeline::MediaStream;

type FrameMessage = (u64, Result<Option<DecodedFrame>, PoppyError>);

/// Control command sent from `AsyncMediaStream` to the background worker.
enum StreamCommand {
    /// Seek to the nearest keyframe at or before timestamp_ticks.
    Seek {
        timestamp_ticks: i64,
        epoch: u64,
        ack: oneshot::Sender<Result<(), PoppyError>>,
    },
    /// Scrub to a target timestamp in seconds: seeks to preceding keyframe and decodes until target.
    Scrub {
        target_seconds: f64,
        epoch: u64,
        ack: oneshot::Sender<Result<Option<DecodedFrame>, PoppyError>>,
    },
    /// Close the worker.
    Close,
}

/// An asynchronous media stream delivering decoded video frames over a Tokio channel.
///
/// Implements `futures_core::Stream<Item = Result<DecodedFrame, PoppyError>>` with
/// backpressure flow control (governed by `buffer_capacity`), asynchronous seeking,
/// and video editor timeline scrubbing.
pub struct AsyncMediaStream {
    frame_rx: mpsc::Receiver<FrameMessage>,
    cmd_tx: mpsc::Sender<StreamCommand>,
    epoch: u64,
    last_pts: Option<i64>,
    streams: Vec<StreamInfo>,
    target_stream_index: usize,
    codec: CodecId,
    time_base: TimeBase,
    duration_ticks: Option<i64>,
    has_hardware_decoder: bool,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

impl AsyncMediaStream {
    /// Opens an asynchronous media stream with default configuration.
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self, PoppyError> {
        Self::open_with_builder(path, &MediaStreamBuilder::default()).await
    }

    /// Opens an asynchronous media stream using explicit builder settings.
    pub async fn open_with_builder<P: AsRef<Path>>(
        path: P,
        builder: &MediaStreamBuilder,
    ) -> Result<Self, PoppyError> {
        let sync_stream = MediaStream::open_with_builder(path, builder)?;
        let streams = sync_stream.streams().to_vec();
        let target_stream_index = sync_stream.target_stream_index();
        let codec = sync_stream.codec();
        let time_base = sync_stream.time_base();
        let duration_ticks = sync_stream.duration_ticks();
        let has_hardware_decoder = sync_stream.has_hardware_decoder();

        let (frame_tx, frame_rx) = mpsc::channel(builder.buffer_capacity);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);

        let worker_handle = std::thread::Builder::new()
            .name("poppy-async-worker".into())
            .spawn(move || {
                run_async_worker(sync_stream, frame_tx, cmd_rx);
            })?;

        Ok(Self {
            frame_rx,
            cmd_tx,
            epoch: 0,
            last_pts: None,
            streams,
            target_stream_index,
            codec,
            time_base,
            duration_ticks,
            has_hardware_decoder,
            worker_handle: Some(worker_handle),
        })
    }

    /// Pull the next decoded frame asynchronously.
    ///
    /// Returns `Ok(Some(frame))` when a frame is available, `Ok(None)` on end-of-stream,
    /// or `Err(...)` if a demuxing or decode error occurs.
    pub async fn next_frame(&mut self) -> Result<Option<DecodedFrame>, PoppyError> {
        loop {
            match self.frame_rx.recv().await {
                Some((epoch, Ok(Some(frame)))) => {
                    if epoch == self.epoch {
                        self.last_pts = frame.pts;
                        return Ok(Some(frame));
                    }
                }
                Some((epoch, Ok(None))) => {
                    if epoch == self.epoch {
                        return Ok(None);
                    }
                }
                Some((epoch, Err(err))) => {
                    if epoch == self.epoch {
                        return Err(err);
                    }
                }
                None => return Ok(None),
            }
        }
    }

    /// Asynchronously seek to the nearest keyframe at or before `timestamp_ticks`.
    ///
    /// Updates epoch counter to invalidate any in-flight pre-seek frames.
    pub async fn seek(&mut self, timestamp_ticks: i64) -> Result<(), PoppyError> {
        self.epoch += 1;
        self.last_pts = None;
        while self.frame_rx.try_recv().is_ok() {}
        let epoch = self.epoch;
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(StreamCommand::Seek {
                timestamp_ticks,
                epoch,
                ack: ack_tx,
            })
            .await
            .map_err(|_| PoppyError::ChannelClosed("Async worker terminated".into()))?;

        ack_rx
            .await
            .map_err(|_| PoppyError::ChannelClosed("Worker failed to acknowledge seek".into()))?
    }

    /// Asynchronously seek to the nearest keyframe at or before `target_seconds`.
    pub async fn seek_seconds(&mut self, target_seconds: f64) -> Result<(), PoppyError> {
        let ticks = (target_seconds * self.time_base.den as f64 / self.time_base.num as f64) as i64;
        self.seek(ticks).await
    }

    /// Precise timeline scrubbing: seeks to the preceding keyframe and decodes forward
    /// until reaching the exact frame matching `target_seconds`.
    ///
    /// Ideal for interactive video players and video editing scrub bars.
    pub async fn scrub_to(
        &mut self,
        target_seconds: f64,
    ) -> Result<Option<DecodedFrame>, PoppyError> {
        self.epoch += 1;
        while self.frame_rx.try_recv().is_ok() {}
        let epoch = self.epoch;
        let (ack_tx, ack_rx) = oneshot::channel();
        self.cmd_tx
            .send(StreamCommand::Scrub {
                target_seconds,
                epoch,
                ack: ack_tx,
            })
            .await
            .map_err(|_| PoppyError::ChannelClosed("Async worker terminated".into()))?;

        let res = ack_rx
            .await
            .map_err(|_| PoppyError::ChannelClosed("Worker failed to acknowledge scrub".into()))?;

        if let Ok(Some(ref frame)) = res {
            self.last_pts = frame.pts;
        }

        res
    }

    /// Step forward by exactly one decoded frame.
    pub async fn step_forward(&mut self) -> Result<Option<DecodedFrame>, PoppyError> {
        self.next_frame().await
    }

    /// Step backward by one frame based on estimated or measured frame duration.
    pub async fn step_backward(
        &mut self,
        frame_duration_seconds: f64,
    ) -> Result<Option<DecodedFrame>, PoppyError> {
        let cur_sec = self
            .last_pts
            .map(|pts| self.time_base.to_seconds(pts))
            .unwrap_or(0.0);
        let target_sec = (cur_sec - frame_duration_seconds).max(0.0);
        self.scrub_to(target_sec).await
    }

    /// Returns metadata for all streams in the media container.
    pub fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    /// Rational time base for the active video track.
    pub fn time_base(&self) -> TimeBase {
        self.time_base
    }

    /// Codec identifier for the active video track.
    pub fn codec(&self) -> CodecId {
        self.codec
    }

    /// Total media stream duration in seconds (if available in container metadata).
    pub fn duration_seconds(&self) -> Option<f64> {
        self.duration_ticks
            .map(|ticks| self.time_base.to_seconds(ticks))
    }

    /// Total media stream duration in timebase ticks.
    pub fn duration_ticks(&self) -> Option<i64> {
        self.duration_ticks
    }

    /// Active stream index being demuxed and decoded.
    pub fn target_stream_index(&self) -> usize {
        self.target_stream_index
    }

    /// Indicates whether a hardware decoder is actively bound to this stream.
    pub fn has_hardware_decoder(&self) -> bool {
        self.has_hardware_decoder
    }

    /// Gracefully closes the stream worker.
    pub async fn close(&mut self) {
        let _ = self.cmd_tx.send(StreamCommand::Close).await;
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Stream for AsyncMediaStream {
    type Item = Result<DecodedFrame, PoppyError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.frame_rx.poll_recv(cx) {
                Poll::Ready(Some((epoch, Ok(Some(frame))))) => {
                    if epoch == self.epoch {
                        self.last_pts = frame.pts;
                        return Poll::Ready(Some(Ok(frame)));
                    }
                }
                Poll::Ready(Some((epoch, Ok(None)))) => {
                    if epoch == self.epoch {
                        return Poll::Ready(None);
                    }
                }
                Poll::Ready(Some((epoch, Err(err)))) => {
                    if epoch == self.epoch {
                        return Poll::Ready(Some(Err(err)));
                    }
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl Drop for AsyncMediaStream {
    fn drop(&mut self) {
        self.frame_rx.close();
        let _ = self.cmd_tx.try_send(StreamCommand::Close);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Background worker loop running demuxing and hardware decoding on a dedicated thread.
fn run_async_worker(
    mut stream: MediaStream,
    frame_tx: mpsc::Sender<FrameMessage>,
    mut cmd_rx: mpsc::Receiver<StreamCommand>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return,
    };

    rt.block_on(async move {
        let mut current_epoch: u64 = 0;
        let mut eos = false;

        loop {
            if eos {
                match cmd_rx.recv().await {
                    Some(StreamCommand::Seek {
                        timestamp_ticks,
                        epoch,
                        ack,
                    }) => {
                        current_epoch = epoch;
                        eos = false;
                        let res = stream.seek(timestamp_ticks);
                        let _ = ack.send(res);
                    }
                    Some(StreamCommand::Scrub {
                        target_seconds,
                        epoch,
                        ack,
                    }) => {
                        current_epoch = epoch;
                        eos = false;
                        let res = perform_scrub(&mut stream, target_seconds);
                        let _ = ack.send(res);
                    }
                    Some(StreamCommand::Close) | None => break,
                }
                continue;
            }

            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(StreamCommand::Seek {
                            timestamp_ticks,
                            epoch,
                            ack,
                        }) => {
                            current_epoch = epoch;
                            let res = stream.seek(timestamp_ticks);
                            let _ = ack.send(res);
                        }
                        Some(StreamCommand::Scrub {
                            target_seconds,
                            epoch,
                            ack,
                        }) => {
                            current_epoch = epoch;
                            let res = perform_scrub(&mut stream, target_seconds);
                            let _ = ack.send(res);
                        }
                        Some(StreamCommand::Close) | None => break,
                    }
                }
                permit_res = frame_tx.reserve() => {
                    match permit_res {
                        Ok(permit) => {
                            match stream.next_frame() {
                                Ok(Some(frame)) => {
                                    permit.send((current_epoch, Ok(Some(frame))));
                                }
                                Ok(None) => {
                                    permit.send((current_epoch, Ok(None)));
                                    eos = true;
                                }
                                Err(err) => {
                                    permit.send((current_epoch, Err(err)));
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
}

/// Internal helper for scrub operations: seeks to preceding keyframe and decodes forward.
fn perform_scrub(
    stream: &mut MediaStream,
    target_seconds: f64,
) -> Result<Option<DecodedFrame>, PoppyError> {
    stream.seek_seconds(target_seconds)?;
    let time_base = stream.time_base();
    let mut best_frame = None;
    let mut min_diff = f64::MAX;

    while let Some(frame) = stream.next_frame()? {
        if let Some(pts) = frame.pts {
            let frame_sec = time_base.to_seconds(pts);
            let diff = (frame_sec - target_seconds).abs();
            if diff <= min_diff {
                min_diff = diff;
                best_frame = Some(frame);
            }
            if frame_sec >= target_seconds {
                break;
            }
        } else {
            best_frame = Some(frame);
            break;
        }
    }

    Ok(best_frame)
}
