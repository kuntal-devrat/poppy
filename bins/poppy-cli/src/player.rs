//! Real-time interactive desktop video player using `winit` and `wgpu`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use poppy::MediaStream;
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
        }
    }

    /// Read next frame from the hardware decoder, or loop/stay on pause.
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
        #[allow(clippy::collapsible_if)]
        if let Some(stream) = &mut self.stream {
            if let Err(e) = stream.seek_seconds(target) {
                eprintln!("⚠️ Seek error: {e}");
            }
        }

        self.paused_at_position = target;
        self.playback_start = Some(Instant::now());
        let _ = self.advance_frame();
        if let Some(w) = &self.window {
            w.request_redraw();
        }
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

        println!(
            "[{state_str}] {cur_str} / {dur_str} | Size: {}x{} | Format: {fmt_str} | Frame #{:04}",
            self.video_width, self.video_height, self.frame_count
        );
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

        let inner_size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: inner_size.width.max(1),
            height: inner_size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&renderer.device, &config);

        println!(
            "📺 Window created: {}x{} (Surface: {:?})",
            inner_size.width, inner_size.height, surface_format
        );
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
                        println!("⏸ Paused at {:.2}s", self.paused_at_position);
                    } else {
                        self.playback_start = Some(Instant::now());
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
                    if let Some(stream) = &mut self.stream {
                        let _ = stream.seek_seconds(0.0);
                    }
                    self.paused_at_position = 0.0;
                    self.playback_start = Some(Instant::now());
                    let _ = self.advance_frame();
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
                KeyCode::Escape | KeyCode::KeyQ => {
                    println!("\n👋 Exiting.");
                    event_loop.exit();
                }
                _ => {}
            },

            WindowEvent::RedrawRequested => {
                let start = match self.playback_start {
                    Some(s) => s,
                    None => {
                        let now = Instant::now();
                        self.playback_start = Some(now);
                        now
                    }
                };

                if !self.is_paused {
                    let elapsed = start.elapsed().as_secs_f64();
                    let target_pos = self.paused_at_position + elapsed;

                    // Fetch next frame if current relative PTS is behind target playback position
                    if self.relative_position() <= target_pos {
                        match self.advance_frame() {
                            Ok(true) => {}
                            Ok(false) => {
                                // End of stream reached
                                println!(
                                    "🏁 End of stream reached. [Press 'R' to Replay, 'Q' to Exit]"
                                );
                                self.is_paused = true;
                            }
                            Err(e) => {
                                eprintln!("⚠️ Decode error during playback: {e}");
                            }
                        }
                    }
                }

                // Render current frame to surface texture
                if let (Some(surface), Some(renderer), Some(frame)) =
                    (&self.surface, &self.renderer, &self.current_frame)
                {
                    match surface.get_current_texture() {
                        Ok(output) => {
                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            if let Err(e) = renderer.render_frame_to_view(frame, &view) {
                                eprintln!("⚠️ Render error: {e}");
                            }

                            output.present();
                        }
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
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
                        Err(wgpu::SurfaceError::Timeout | wgpu::SurfaceError::Other) => {}
                    }
                }

                if self.last_status_print.elapsed() >= Duration::from_millis(500) {
                    self.print_hud();
                    self.last_status_print = Instant::now();
                }

                // Schedule next frame redraw
                if !self.is_paused {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    event_loop.set_control_flow(ControlFlow::Poll);
                } else {
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }

            _ => {}
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
