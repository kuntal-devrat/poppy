use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use poppy_core::AudioFrame;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::error::AudioError;

/// Maximum buffered audio in samples: ~4 s of 48 kHz stereo.
const MAX_BUFFER_SAMPLES: usize = 48_000 * 2 * 4;

/// Inner clock state.
///
/// Both fields are always updated together under a single mutex, so readers can
/// never observe a torn combination of `base_micros` from one update and
/// `last_instant` from another (the race present when the two lived in separate
/// atomics/mutexes).
#[derive(Debug, Clone, Copy)]
struct ClockInner {
    base_micros: u64,
    last_instant: Option<Instant>,
    max_reported_micros: u64,
}

/// High-resolution master audio clock tracking exact audible playback time.
///
/// `base_micros` counts samples actually consumed by the audio hardware; the
/// reader adds bounded wall-clock extrapolation on top. The realtime audio
/// thread only ever uses non-blocking [`Mutex::try_lock`] (falling back to a
/// lock-free pending-delta stash on contention), so it can never block or
/// allocate. All other methods are for non-realtime threads and may block
/// briefly.
#[derive(Clone)]
pub struct AudioClock {
    inner: Arc<Mutex<ClockInner>>,
    /// Deltas stashed by the RT thread when [`Mutex::try_lock`] failed because
    /// a non-RT thread momentarily held the lock. Folded into `base_micros`
    /// under lock by the next successful operation, so no progress is lost.
    pending_micros: Arc<AtomicU64>,
    is_paused: Arc<AtomicBool>,
}

impl AudioClock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClockInner {
                base_micros: 0,
                last_instant: None,
                max_reported_micros: 0,
            })),
            pending_micros: Arc::new(AtomicU64::new(0)),
            is_paused: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Lock (non-RT threads only) and fold any RT-stashed deltas into the base.
    fn lock_folded(&self) -> MutexGuard<'_, ClockInner> {
        let mut guard = self.inner.lock().unwrap();
        let pending = self.pending_micros.swap(0, Ordering::AcqRel);
        if pending > 0 {
            guard.base_micros = guard.base_micros.saturating_add(pending);
            guard.max_reported_micros = guard.max_reported_micros.max(guard.base_micros);
        }
        guard
    }

    /// Current audible presentation timestamp in seconds with microsecond resolution.
    pub fn current_pts_seconds(&self) -> f64 {
        let mut guard = self.lock_folded();
        if self.is_paused.load(Ordering::Relaxed) {
            return (guard.base_micros as f64) / 1_000_000.0;
        }

        let elapsed_micros = guard
            .last_instant
            .map(|t| (t.elapsed().as_secs_f64() * 1_000_000.0) as u64)
            .unwrap_or(0);

        // Bound extrapolation to 100ms to avoid clock runaway during stalls/underruns
        const MAX_EXTRAPOLATION_MICROS: u64 = 100_000;
        let bounded_elapsed = elapsed_micros.min(MAX_EXTRAPOLATION_MICROS);

        let computed = guard.base_micros.saturating_add(bounded_elapsed);
        guard.max_reported_micros = guard.max_reported_micros.max(computed);
        (guard.max_reported_micros as f64) / 1_000_000.0
    }

    /// Record audio hardware progress: update base PTS and refresh update instant.
    ///
    /// Called from the realtime audio callback. Never blocks: uses
    /// [`Mutex::try_lock`] and stashes the delta lock-free on contention.
    /// Performs no allocation and no logging.
    fn advance_pts(&self, delta_seconds: f64) {
        let delta_micros = (delta_seconds.max(0.0) * 1_000_000.0) as u64;
        if delta_micros == 0 {
            return;
        }
        match self.inner.try_lock() {
            Ok(mut guard) => {
                let pending = self.pending_micros.swap(0, Ordering::AcqRel);
                let advance = pending.saturating_add(delta_micros);
                guard.base_micros = guard.base_micros.saturating_add(advance);
                guard.max_reported_micros = guard.max_reported_micros.max(guard.base_micros);
                guard.last_instant = Some(Instant::now());
            }
            Err(_) => {
                // A non-RT thread holds the lock briefly; stash without blocking.
                self.pending_micros
                    .fetch_add(delta_micros, Ordering::Relaxed);
            }
        }
    }

    /// Reset playback clock to a given timestamp in seconds.
    pub fn reset_to(&self, seconds: f64) {
        let micros = (seconds.max(0.0) * 1_000_000.0) as u64;
        // Discard any stashed deltas from before the reset.
        self.pending_micros.store(0, Ordering::Release);
        let mut guard = self.inner.lock().unwrap();
        guard.base_micros = micros;
        guard.max_reported_micros = micros;
        guard.last_instant = if self.is_paused.load(Ordering::Relaxed) {
            None
        } else {
            Some(Instant::now())
        };
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::Relaxed)
    }

    pub fn pause(&self) {
        let mut guard = self.lock_folded();
        if let Some(t) = guard.last_instant {
            let elapsed_micros = (t.elapsed().as_secs_f64() * 1_000_000.0) as u64;
            const MAX_EXTRAPOLATION_MICROS: u64 = 100_000;
            let bounded_elapsed = elapsed_micros.min(MAX_EXTRAPOLATION_MICROS);
            guard.base_micros = guard.base_micros.saturating_add(bounded_elapsed);
            guard.max_reported_micros = guard.max_reported_micros.max(guard.base_micros);
        }
        guard.last_instant = None;
        self.is_paused.store(true, Ordering::Release);
    }

    pub fn play(&self) {
        self.is_paused.store(false, Ordering::Release);
        let mut guard = self.lock_folded();
        guard.last_instant = Some(Instant::now());
    }
}

impl Default for AudioClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Streaming linear-resampler state, persisted across [`AudioSink::push_frame`]
/// calls so the output sample grid stays continuous.
///
/// `frac_pos` is the source position (in input frames, possibly slightly
/// negative right after a frame boundary) of the next output frame to
/// generate. Without this, each frame computed
/// `(in_frames / ratio).floor()` output frames independently and lost up to
/// one output frame per call, causing cumulative A/V drift on any non-integer
/// rate conversion (e.g. 44.1 kHz -> 48 kHz). `prev_tail` holds the last input
/// frame of the previous call so boundary interpolation can use the true
/// neighbour sample instead of clamping.
#[derive(Debug, Default)]
struct ResamplerState {
    frac_pos: f64,
    prev_tail: Vec<f32>,
}

/// Cross-platform real-time audio playback sink using `cpal`.
///
/// Sample transport uses a lock-free SPSC [`RingBuffer`]: the realtime `cpal`
/// callback owns no locks on its hot path — it pops via non-blocking
/// [`Consumer::pop`] behind [`Mutex::try_lock`] (falling back to silence on
/// contention) and never allocates, blocks, or logs. The producer side
/// ([`AudioSink::push_frame`], [`AudioSink::reset_to`],
/// [`AudioSink::queued_samples`]) runs on non-realtime threads and may block
/// briefly on its own mutexes; it never touches the RT path's critical
/// section beyond a `try_lock` boundary.
pub struct AudioSink {
    _stream: Option<cpal::Stream>,
    /// Producer half of the ring buffer. Only touched by non-RT threads.
    producer: Arc<Mutex<Producer<f32>>>,
    /// Consumer half, shared with the RT callback. The RT callback accesses it
    /// via `try_lock` only (never blocks); non-RT threads use it for
    /// drop-oldest eviction and drain-on-reset.
    consumer: Arc<Mutex<Consumer<f32>>>,
    ring_capacity: usize,
    clock: AudioClock,
    is_paused: Arc<AtomicBool>,
    device_sample_rate: u32,
    device_channels: u16,
    /// Streaming resampler state. Non-RT only (`push_frame` / `reset_to`).
    resampler: Mutex<ResamplerState>,
}

impl AudioSink {
    /// Attempts to open the default system audio output device.
    /// If no device is available (e.g. headless CI), gracefully enters headless dummy mode.
    pub fn try_new(target_sample_rate: u32, target_channels: u16) -> Result<Self, AudioError> {
        let (producer, consumer) = RingBuffer::new(MAX_BUFFER_SAMPLES);
        let producer = Arc::new(Mutex::new(producer));
        let consumer = Arc::new(Mutex::new(consumer));
        let resampler = Mutex::new(ResamplerState::default());
        let is_paused = Arc::new(AtomicBool::new(false));
        let clock = AudioClock::new();

        let host = cpal::default_host();
        let device = host.default_output_device();

        if let Some(device) = device
            && let Ok(config) = device.default_output_config()
        {
            let channels = config.channels().max(1);
            let sample_rate = config.sample_rate().0;

            let consumer_clone = Arc::clone(&consumer);
            let paused_clone = Arc::clone(&is_paused);
            let clock_clone = clock.clone();

            let err_fn = |err| {
                tracing::warn!("Audio output stream error: {err}");
            };

            let stream_res = match config.sample_format() {
                cpal::SampleFormat::F32 => {
                    let q = Arc::clone(&consumer_clone);
                    let p = Arc::clone(&paused_clone);
                    let clk = clock_clone.clone();
                    device.build_output_stream(
                        &config.into(),
                        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                            if p.load(Ordering::Relaxed) {
                                data.fill(0.0);
                                return;
                            }

                            // RT path: never block, never allocate, never log.
                            match q.try_lock() {
                                Ok(mut cons) => {
                                    let mut consumed = 0usize;
                                    for out in data.iter_mut() {
                                        match cons.pop() {
                                            Ok(s) => {
                                                *out = s;
                                                consumed += 1;
                                            }
                                            Err(_) => *out = 0.0,
                                        }
                                    }

                                    let consumed_frames = consumed / channels as usize;
                                    if consumed_frames > 0 {
                                        let delta_sec = consumed_frames as f64 / sample_rate as f64;
                                        clk.advance_pts(delta_sec);
                                    }
                                }
                                Err(_) => {
                                    // Consumer briefly held by a non-RT
                                    // drain; emit silence for this callback.
                                    data.fill(0.0);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::I16 => {
                    let q = Arc::clone(&consumer_clone);
                    let p = Arc::clone(&paused_clone);
                    let clk = clock_clone.clone();
                    device.build_output_stream(
                        &config.into(),
                        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                            if p.load(Ordering::Relaxed) {
                                data.fill(0);
                                return;
                            }

                            // RT path: never block, never allocate, never log.
                            match q.try_lock() {
                                Ok(mut cons) => {
                                    let mut consumed = 0usize;
                                    for out in data.iter_mut() {
                                        match cons.pop() {
                                            Ok(s) => {
                                                *out =
                                                    (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                                consumed += 1;
                                            }
                                            Err(_) => *out = 0,
                                        }
                                    }

                                    let consumed_frames = consumed / channels as usize;
                                    if consumed_frames > 0 {
                                        let delta_sec = consumed_frames as f64 / sample_rate as f64;
                                        clk.advance_pts(delta_sec);
                                    }
                                }
                                Err(_) => {
                                    data.fill(0);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::U16 => {
                    let q = Arc::clone(&consumer_clone);
                    let p = Arc::clone(&paused_clone);
                    let clk = clock_clone.clone();
                    device.build_output_stream(
                        &config.into(),
                        move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                            if p.load(Ordering::Relaxed) {
                                data.fill(32768);
                                return;
                            }

                            // RT path: never block, never allocate, never log.
                            match q.try_lock() {
                                Ok(mut cons) => {
                                    let mut consumed = 0usize;
                                    for out in data.iter_mut() {
                                        match cons.pop() {
                                            Ok(s) => {
                                                let s = s.clamp(-1.0, 1.0);
                                                *out = ((s * 0.5 + 0.5) * u16::MAX as f32) as u16;
                                                consumed += 1;
                                            }
                                            Err(_) => *out = 32768,
                                        }
                                    }

                                    let consumed_frames = consumed / channels as usize;
                                    if consumed_frames > 0 {
                                        let delta_sec = consumed_frames as f64 / sample_rate as f64;
                                        clk.advance_pts(delta_sec);
                                    }
                                }
                                Err(_) => {
                                    data.fill(32768);
                                }
                            }
                        },
                        err_fn,
                        None,
                    )
                }
                _ => Err(cpal::BuildStreamError::DeviceNotAvailable),
            };

            if let Ok(stream) = stream_res {
                let _ = stream.play();
                clock.play();
                return Ok(Self {
                    _stream: Some(stream),
                    producer,
                    consumer,
                    ring_capacity: MAX_BUFFER_SAMPLES,
                    clock,
                    is_paused,
                    device_sample_rate: sample_rate,
                    device_channels: channels,
                    resampler,
                });
            }
        }

        // Headless fallback mode:
        tracing::info!("Audio device not found or unavailable; using virtual headless audio clock");
        clock.play();
        Ok(Self {
            _stream: None,
            producer,
            consumer,
            ring_capacity: MAX_BUFFER_SAMPLES,
            clock,
            is_paused,
            device_sample_rate: target_sample_rate.max(1),
            device_channels: target_channels.max(1),
            resampler,
        })
    }

    /// Access the master audio clock.
    pub fn clock(&self) -> AudioClock {
        self.clock.clone()
    }

    /// Submit a decoded audio frame into the playback ring buffer, performing
    /// automatic channel mapping and streaming linear resampling if the frame
    /// does not match the target audio output hardware.
    ///
    /// Non-RT thread only: may block briefly on the producer mutex. If the
    /// buffer is full, the oldest samples are evicted to make room (with a
    /// `tracing::warn`), so a stalled device can never grow memory
    /// unboundedly.
    pub fn push_frame(&self, frame: &AudioFrame) {
        if frame.samples.is_empty() {
            return;
        }

        let in_channels = frame.channels.max(1) as usize;
        let out_channels = self.device_channels.max(1) as usize;
        let in_rate = frame.sample_rate.max(1);
        let out_rate = self.device_sample_rate.max(1);

        // 1. Channel layout adaptation
        let channel_adapted: Vec<f32> = if in_channels == out_channels {
            frame.samples.clone()
        } else if in_channels == 1 && out_channels == 2 {
            // Mono -> Stereo
            let mut out = Vec::with_capacity(frame.samples.len() * 2);
            for &s in &frame.samples {
                out.push(s);
                out.push(s);
            }
            out
        } else if in_channels == 2 && out_channels == 1 {
            // Stereo -> Mono
            let mut out = Vec::with_capacity(frame.samples.len() / 2);
            for chunk in frame.samples.as_chunks::<2>().0 {
                out.push((chunk[0] + chunk[1]) * 0.5);
            }
            out
        } else {
            // Multi-channel downmix / upmix fallback
            let in_frames = frame.samples.len() / in_channels;
            let mut out = Vec::with_capacity(in_frames * out_channels);
            for i in 0..in_frames {
                for c in 0..out_channels {
                    let s = if c < in_channels {
                        frame.samples[i * in_channels + c]
                    } else {
                        0.0
                    };
                    out.push(s);
                }
            }
            out
        };

        if channel_adapted.is_empty() {
            return;
        }

        // 2. Streaming linear resampling if sample rates differ.
        // The fractional source position persists across calls (see
        // ResamplerState), so the output grid stays continuous and no
        // cumulative drift accumulates.
        let final_samples: Vec<f32> = if in_rate == out_rate {
            // No resampling; reset streaming state so a later rate change
            // starts from a clean phase.
            *self.resampler.lock().unwrap() = ResamplerState::default();
            channel_adapted
        } else {
            let in_frames = channel_adapted.len() / out_channels;
            if in_frames == 0 {
                return;
            }
            let ratio = in_rate as f64 / out_rate as f64;
            let mut st = self.resampler.lock().unwrap();
            if st.prev_tail.len() != out_channels {
                // Layout changed mid-stream; restart the phase.
                st.prev_tail.clear();
                st.frac_pos = 0.0;
            }
            let mut pos = st.frac_pos;
            if !pos.is_finite() || pos < -1.0 {
                pos = 0.0;
            }
            // Note: a large positive `pos` (tiny input chunk under heavy
            // downsampling) is valid and self-corrects: calls that emit
            // nothing subtract `in_frames` until the phase re-enters range.
            let est_frames = (in_frames as f64 / ratio + 2.0).max(0.0);
            let mut out =
                Vec::with_capacity((est_frames.ceil() as usize).saturating_mul(out_channels));

            loop {
                let idx0 = pos.floor() as isize;
                let frac = (pos - idx0 as f64) as f32;
                let idx1 = idx0 + 1;
                // `idx1` past the end needs next frame's first sample, which
                // hasn't arrived yet: stop and resume there next call instead
                // of clamping, keeping the grid exact.
                if idx1 < 0 || idx1 >= in_frames as isize {
                    break;
                }
                for c in 0..out_channels {
                    let s0 = if idx0 < 0 {
                        st.prev_tail[c]
                    } else {
                        channel_adapted[idx0 as usize * out_channels + c]
                    };
                    let s1 = channel_adapted[idx1 as usize * out_channels + c];
                    out.push((1.0 - frac) * s0 + frac * s1);
                }
                pos += ratio;
            }
            st.frac_pos = pos - in_frames as f64;
            st.prev_tail.clear();
            st.prev_tail
                .extend_from_slice(&channel_adapted[(in_frames - 1) * out_channels..]);
            out
        };

        if final_samples.is_empty() {
            return;
        }

        // 3. Enqueue. Non-RT: blocking lock is fine here.
        let mut prod = self.producer.lock().unwrap();
        let buffered = self.ring_capacity.saturating_sub(prod.slots());
        let total = buffered.saturating_add(final_samples.len());
        if total > MAX_BUFFER_SAMPLES {
            // Prevent unbounded memory growth if playback is paused or
            // blocked: evict oldest samples first.
            let to_drop = total - MAX_BUFFER_SAMPLES;
            let mut dropped = 0usize;
            if let Ok(mut cons) = self.consumer.lock() {
                for _ in 0..to_drop {
                    match cons.pop() {
                        Ok(_) => dropped += 1,
                        Err(_) => break,
                    }
                }
            }
            if dropped > 0 {
                tracing::warn!(
                    "Audio ring buffer full ({buffered} samples buffered); dropped {dropped} oldest samples"
                );
            }
        }

        let mut dropped_new = 0usize;
        for s in final_samples {
            if prod.push(s).is_err() {
                // Still full (single frame larger than the whole buffer):
                // drop the newest remainder.
                dropped_new += 1;
            }
        }
        if dropped_new > 0 {
            tracing::warn!(
                "Audio ring buffer full; dropped {dropped_new} newest samples from oversized frame"
            );
        }
    }

    /// Pause audio playback without flushing the queued samples.
    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::Release);
        self.clock.pause();
    }

    /// Resume audio playback.
    pub fn play(&self) {
        self.is_paused.store(false, Ordering::Release);
        self.clock.play();
    }

    /// Clear all queued audio samples and reset playback clock to a target timestamp.
    pub fn reset_to(&self, seconds: f64) {
        if let Ok(mut cons) = self.consumer.lock() {
            while cons.pop().is_ok() {}
        }
        *self.resampler.lock().unwrap() = ResamplerState::default();
        self.clock.reset_to(seconds);
    }

    /// Returns the number of unplayed audio samples currently buffered in memory.
    pub fn queued_samples(&self) -> usize {
        self.producer
            .lock()
            .map(|prod| self.ring_capacity.saturating_sub(prod.slots()))
            .unwrap_or(0)
    }

    pub fn device_sample_rate(&self) -> u32 {
        self.device_sample_rate
    }

    pub fn device_channels(&self) -> u16 {
        self.device_channels
    }
}
