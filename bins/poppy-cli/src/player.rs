//! Real-time interactive desktop video player using `winit` and `wgpu`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use poppy::MediaStream;
use poppy_audio::AudioSink;
use poppy_hw::DecodedFrame;
use poppy_render::GpuRenderer;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

pub struct VideoPlayerApp {
    file_path: PathBuf,
    stream: Option<MediaStream>,
    renderer: Option<GpuRenderer>,
    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,

    // Playback clock & state
    current_frame: Option<DecodedFrame>,
    is_paused: bool,
    playback_start: Option<Instant>,
    paused_at_position: f64,
    current_pts_sec: f64,
    first_pts_sec: f64,
    duration_sec: Option<f64>,
    video_width: u32,
    video_height: u32,
    frame_count: u64,
    last_status_print: Instant,

    // In-process pure-Rust audio playback sink
    audio_sink: Option<AudioSink>,
    has_audio_stream: bool,

    // Seek display synchronization
    seek_frame_pending_display: bool,
}

impl VideoPlayerApp {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            file_path: path.as_ref().to_path_buf(),
            stream: None,
            renderer: None,
            window: None,
            surface: None,
            surface_config: None,
            current_frame: None,
            is_paused: false,
            playback_start: None,
            paused_at_position: 0.0,
            current_pts_sec: 0.0,
            first_pts_sec: 0.0,
            duration_sec: None,
            video_width: 1280,
            video_height: 720,
            frame_count: 0,
            last_status_print: Instant::now(),
            audio_sink: None,
            has_audio_stream: false,
            seek_frame_pending_display: false,
        }
    }

    /// Read next frame from the hardware decoder.
    fn advance_frame(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        let stream = match &mut self.stream {
            Some(s) => s,
            None => return Ok(false),
        };

        if let Some(frame) = stream.next_frame()? {
            self.video_width = frame.width;
            self.video_height = frame.height;
            if let Some(pts) = frame.pts {
                self.current_pts_sec = stream.time_base().to_seconds(pts);
            }
            self.current_frame = Some(frame);
            self.frame_count += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Current playback position relative to stream start.
    fn relative_position(&self) -> f64 {
        (self.current_pts_sec - self.first_pts_sec).max(0.0)
    }

    /// Current master presentation clock in seconds.
    fn current_master_time(&self) -> f64 {
        if let Some(sink) = &self.audio_sink {
            sink.clock().current_pts_seconds()
        } else if let Some(start) = self.playback_start {
            self.paused_at_position + start.elapsed().as_secs_f64()
        } else {
            self.paused_at_position
        }
    }

    /// Keep the in-process audio ring buffer filled with decoded PCM frames.
    fn pump_audio(&mut self) {
        if let (Some(stream), Some(sink)) = (&mut self.stream, &self.audio_sink) {
            // Buffer up to ~1 second of audio in the device ring buffer
            let target_samples =
                (sink.device_sample_rate() as usize) * (sink.device_channels() as usize);
            let mut frames_pumped = 0;
            while sink.queued_samples() < target_samples && frames_pumped < 32 {
                match stream.next_audio_frame() {
                    Ok(Some(af)) => {
                        sink.push_frame(&af);
                        frames_pumped += 1;
                    }
                    _ => break,
                }
            }
        }
    }

    /// Pauses audio playback.
    fn pause_audio(&mut self) {
        if let Some(sink) = &self.audio_sink {
            sink.pause();
        }
    }

    /// Resumes audio playback.
    fn resume_audio(&mut self) {
        if let Some(sink) = &self.audio_sink {
            sink.play();
        }
        for _ in 0..4 {
            self.pump_audio();
        }
    }

    /// Seek forward or backward by `delta_seconds`.
    fn seek_relative(&mut self, delta_seconds: f64) {
        let current_rel = self.relative_position();
        let target = (current_rel + delta_seconds).max(0.0);
        let target = if let Some(dur) = self.duration_sec {
            target.min(dur)
        } else {
            target
        };

        println!("⏩ Seeking to {:.2}s...", target);
        let stream_target = self.first_pts_sec + target;
        if let Some(stream) = &mut self.stream {
            match stream.seek_exact(stream_target) {
                Ok(Some(frame)) => {
                    self.video_width = frame.width;
                    self.video_height = frame.height;
                    if let Some(pts) = frame.pts {
                        self.current_pts_sec = stream.time_base().to_seconds(pts);
                    }
                    self.current_frame = Some(frame);
                    self.frame_count += 1;
                }
                Ok(None) => {}
                Err(e) => eprintln!("⚠️ Seek error: {e}"),
            }
        }

        // Snap both the audio sink clock and master playback anchor to the landed frame timestamp
        let actual_pos = self.relative_position();
        if let Some(sink) = &self.audio_sink {
            sink.reset_to(actual_pos);
        }

        self.paused_at_position = actual_pos;
        self.playback_start = Some(Instant::now());
        // Fully pre-buffer up to 1 second of audio at seek target before resuming display
        for _ in 0..4 {
            self.pump_audio();
        }

        self.seek_frame_pending_display = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// Compute letterboxed/pillarboxed viewport preserving video native aspect ratio.
    fn compute_viewport(&self) -> Option<(f32, f32, f32, f32)> {
        let config = self.surface_config.as_ref()?;
        let win_w = config.width as f32;
        let win_h = config.height as f32;
        if win_w <= 0.0 || win_h <= 0.0 || self.video_width == 0 || self.video_height == 0 {
            return None;
        }

        let video_aspect = self.video_width as f32 / self.video_height as f32;
        let window_aspect = win_w / win_h;

        let (vp_x, vp_y, vp_w, vp_h) = if window_aspect > video_aspect {
            // Window is wider than video -> pillarbox (black bars left and right)
            let w = win_h * video_aspect;
            let x = (win_w - w) * 0.5;
            (x, 0.0, w, win_h)
        } else {
            // Window is taller than video -> letterbox (black bars top and bottom)
            let h = win_w / video_aspect;
            let y = (win_h - h) * 0.5;
            (0.0, y, win_w, h)
        };

        Some((vp_x.round(), vp_y.round(), vp_w.round(), vp_h.round()))
    }

    fn print_hud(&mut self) {
        let dur_str = if let Some(dur) = self.duration_sec {
            format!("{:02}:{:05.2}", (dur as u64) / 60, dur % 60.0)
        } else {
            "--:--.--".to_string()
        };
        let rel_sec = self.relative_position();
        let cur_str = format!("{:02}:{:05.2}", (rel_sec as u64) / 60, rel_sec % 60.0);

        let state_str = if self.is_paused {
            "⏸ PAUSED"
        } else {
            "▶ PLAYING"
        };
        let fmt_str = self
            .current_frame
            .as_ref()
            .map(|f| format!("{}", f.pixel_format))
            .unwrap_or_else(|| "Unknown".to_string());

        let audio_indicator = if self.has_audio_stream {
            "🔊 Audio: Sync"
        } else {
            "🔇 No Audio"
        };

        println!(
            "[{state_str}] {cur_str} / {dur_str} | Size: {}x{} | Format: {fmt_str} | {audio_indicator} | Frame #{:04}",
            self.video_width, self.video_height, self.frame_count
        );
    }

    /// Schedule event loop wake-up for the next video frame timestamp.
    fn schedule_next_wake(&self, event_loop: &ActiveEventLoop) {
        if self.is_paused {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        if self.seek_frame_pending_display {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }

        let target_pos = self.current_master_time();
        let cur_rel = self.relative_position();

        if cur_rel <= target_pos {
            // Next frame is due right now
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            // Sleep until the current frame presentation duration expires
            let wait_secs = cur_rel - target_pos;
            let wake_at = Instant::now() + Duration::from_secs_f64(wait_secs.clamp(0.001, 0.1));
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake_at));
        }
    }
}

impl ApplicationHandler for VideoPlayerApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        println!("🎬 Initializing hardware-accelerated playback pipeline...");
        let mut stream = match poppy::open(&self.file_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("❌ Failed to open media: {e}");
                event_loop.exit();
                return;
            }
        };

        self.duration_sec = stream.duration_seconds();
        self.has_audio_stream = stream.streams().iter().any(|s| s.is_audio());

        // Peek first frame to get natural dimensions
        let (initial_w, initial_h) = match stream.next_frame() {
            Ok(Some(frame)) => {
                let w = frame.width;
                let h = frame.height;
                if let Some(pts) = frame.pts {
                    let pts_sec = stream.time_base().to_seconds(pts);
                    self.current_pts_sec = pts_sec;
                    self.first_pts_sec = pts_sec;
                }
                self.current_frame = Some(frame);
                self.frame_count += 1;
                (w, h)
            }
            Ok(None) => (1280, 720),
            Err(e) => {
                eprintln!("⚠️ Warning reading initial frame: {e}");
                (1280, 720)
            }
        };

        self.video_width = initial_w;
        self.video_height = initial_h;
        self.stream = Some(stream);

        // Limit window display to reasonable default while preserving aspect ratio
        let display_w = initial_w.clamp(640, 1280);
        let display_h = (display_w as f64 * (initial_h as f64 / initial_w as f64)) as u32;

        let filename = self
            .file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Poppy Player");

        let window_attrs = WindowAttributes::default()
            .with_title(format!("🌸 Poppy Player — {filename}"))
            .with_inner_size(LogicalSize::new(display_w, display_h));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("❌ Failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let mut renderer = match GpuRenderer::new_sync() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("❌ Failed to create wgpu renderer: {e}");
                event_loop.exit();
                return;
            }
        };

        let surface = match renderer.instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("❌ Failed to create wgpu surface: {e}");
                event_loop.exit();
                return;
            }
        };

        let caps = surface.get_capabilities(&renderer.adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        if let Err(e) = renderer.set_output_format(surface_format) {
            eprintln!("⚠️ Failed to match surface output format: {e}");
        }

        // Select platform-optimal vsync present mode to eliminate screen tearing while avoiding
        // Wayland compositor presentation deadlocks / VK_TIMEOUT stalls on GNOME
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::AutoVsync) {
            wgpu::PresentMode::AutoVsync
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if caps.present_modes.contains(&wgpu::PresentMode::FifoRelaxed) {
            wgpu::PresentMode::FifoRelaxed
        } else if caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
            wgpu::PresentMode::Fifo
        } else {
            caps.present_modes[0]
        };

        let inner_size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: inner_size.width.max(1),
            height: inner_size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&renderer.device, &config);

        println!(
            "📺 Window created: {}x{} (Surface: {:?}, VSync: {:?})",
            inner_size.width, inner_size.height, surface_format, present_mode
        );
        if self.has_audio_stream
            && let Some(stream) = &self.stream
            && let Some(cfg) = stream.audio_config()
        {
            match AudioSink::try_new(cfg.sample_rate, cfg.channels) {
                Ok(sink) => {
                    println!(
                        "🔊 Audio stream detected ({} Hz, {} channels): Pure-Rust in-process playback active",
                        cfg.sample_rate, cfg.channels
                    );
                    self.audio_sink = Some(sink);
                    self.pump_audio();
                }
                Err(e) => {
                    eprintln!("⚠️ Failed to initialize audio sink: {e}");
                }
            }
        } else if !self.has_audio_stream {
            println!("🔇 No audio stream present in container");
        }
        println!(
            "⌨️  Controls: [Space] Pause/Play | [Left/Right] Seek ±5s | [R] Replay | [Q/Esc] Quit\n"
        );

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.surface_config = Some(config);
        self.playback_start = Some(Instant::now());

        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                println!("\n👋 Closing Poppy Video Player.");
                event_loop.exit();
            }

            WindowEvent::Resized(new_size) => {
                #[allow(clippy::collapsible_if)]
                if let (Some(surface), Some(renderer), Some(config)) =
                    (&self.surface, &self.renderer, &mut self.surface_config)
                {
                    if new_size.width > 0 && new_size.height > 0 {
                        config.width = new_size.width;
                        config.height = new_size.height;
                        surface.configure(&renderer.device, config);
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }

            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => match key {
                KeyCode::Space => {
                    self.is_paused = !self.is_paused;
                    if self.is_paused {
                        self.paused_at_position = self.relative_position();
                        self.pause_audio();
                        println!("⏸ Paused at {:.2}s", self.paused_at_position);
                    } else {
                        self.playback_start = Some(Instant::now());
                        self.resume_audio();
                        println!("▶ Resumed playback");
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                KeyCode::ArrowRight => {
                    self.seek_relative(5.0);
                }
                KeyCode::ArrowLeft => {
                    self.seek_relative(-5.0);
                }
                KeyCode::KeyR => {
                    println!("🔄 Replaying from start...");
                    self.seek_relative(-self.relative_position());
                }
                KeyCode::Escape | KeyCode::KeyQ => {
                    println!("\n👋 Exiting.");
                    event_loop.exit();
                }
                _ => {}
            },

            WindowEvent::RedrawRequested => {
                if self.playback_start.is_none() {
                    self.playback_start = Some(Instant::now());
                }

                self.pump_audio();

                if self.seek_frame_pending_display {
                    self.seek_frame_pending_display = false;
                    // Render the landed seek frame without advancing forward.
                    // Snap the master audio clock and wall anchor to this frame timestamp at presentation
                    let landed_pos = self.relative_position();
                    if let Some(sink) = &self.audio_sink {
                        sink.clock().reset_to(landed_pos);
                    }
                    self.paused_at_position = landed_pos;
                    self.playback_start = Some(Instant::now());
                } else if !self.is_paused {
                    let target_pos = self.current_master_time();

                    // Advance frame if playback clock is ahead
                    // Allow catch-up up to 4 frames if noticeably behind, preventing stutter spirals
                    let max_advance = if target_pos - self.relative_position() > 0.08 {
                        4
                    } else {
                        2
                    };
                    let mut frames_advanced = 0;
                    while !self.is_paused
                        && self.relative_position() < target_pos
                        && frames_advanced < max_advance
                    {
                        match self.advance_frame() {
                            Ok(true) => {
                                frames_advanced += 1;
                            }
                            Ok(false) => {
                                // End of stream reached
                                println!(
                                    "🏁 End of stream reached. [Press 'R' to Replay, 'Q' to Exit]"
                                );
                                self.is_paused = true;
                                self.pause_audio();
                                break;
                            }
                            Err(e) => {
                                eprintln!("⚠️ Decode error during playback: {e}");
                                break;
                            }
                        }
                    }
                }

                // Render current frame to surface texture with letterboxing / pillarboxing
                if let (Some(surface), Some(renderer), Some(frame)) =
                    (&self.surface, &self.renderer, &self.current_frame)
                {
                    match surface.get_current_texture() {
                        Ok(output) => {
                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let viewport = self.compute_viewport();
                            if let Err(e) =
                                renderer.render_frame_to_view_with_viewport(frame, &view, viewport)
                            {
                                eprintln!("⚠️ Render error: {e}");
                            }

                            output.present();
                        }
                        Err(
                            wgpu::SurfaceError::Lost
                            | wgpu::SurfaceError::Outdated
                            | wgpu::SurfaceError::Timeout,
                        ) => {
                            if let (Some(config), Some(window)) =
                                (&self.surface_config, &self.window)
                            {
                                surface.configure(&renderer.device, config);
                                window.request_redraw();
                            }
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            eprintln!("❌ Out of GPU memory!");
                            event_loop.exit();
                            return;
                        }
                        Err(wgpu::SurfaceError::Other) => {}
                    }
                }

                if self.last_status_print.elapsed() >= Duration::from_millis(500) {
                    self.print_hud();
                    self.last_status_print = Instant::now();
                }

                // Schedule next frame wake-up
                self.schedule_next_wake(event_loop);
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.seek_frame_pending_display {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }

        if !self.is_paused {
            self.pump_audio();
            let target_pos = self.current_master_time();
            let cur_rel = self.relative_position();
            if cur_rel <= target_pos {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                let wait_secs = cur_rel - target_pos;
                let wake_at = Instant::now() + Duration::from_secs_f64(wait_secs.clamp(0.001, 0.1));
                event_loop.set_control_flow(ControlFlow::WaitUntil(wake_at));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Launches the interactive desktop video player.
pub fn run_player(file_path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    let mut app = VideoPlayerApp::new(file_path);
    event_loop.run_app(&mut app)?;
    Ok(())
}
