# Changelog

All notable changes to Project Poppy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.5.0] - 2026-10-11

### Added
- **Synchronized Companion Audio**: Interactive player automatically detects container audio streams and plays synchronized audio with zero-latency `SIGSTOP`/`SIGCONT` pausing and seeking.
- **Letterboxing & Pillarboxing**: Direct viewport calculations preserving native video aspect ratio with clean letterboxing on arbitrary window dimensions.
- **Strict VSync Presentation**: Enforced `wgpu::PresentMode::Fifo` presentation to eliminate screen tearing across Vulkan, Wayland, and X11.
- **Clock-Paced Frame Pacing**: Converted event loop from busy-polling to `ControlFlow::WaitUntil` timestamp scheduling, reducing CPU utilization to ~4%.

### Fixed
- **VA-API Reference Surface Overwrites**: Expanded decode surface pool from 16 to 32 and implemented DPB reference-aware surface allocation, resolving corrupted frame concealment warnings and macroblocking artifacts.
- **VA-API H.264 FFI Struct Alignment**: Corrected bitfield offsets and reserved padding fields in `VAPictureParameterBufferH264` and `VASliceParameterBufferH264`.

---

## [1.0.0] - 2026-10-09

### Added
- **Production API Freeze**: Stabilized the public interface across all workspace crates with SemVer 1.0.0 guarantees under Rust Edition 2024.
- **Universal C ABI (`libpoppy.so`)**:
  - Dual-target `cdylib` and `rlib` library output in `crates/poppy` emitting `target/release/libpoppy.so`.
  - Canonical C header [`include/poppy.h`](include/poppy.h) providing complete null-checked C functions.
  - Lifecycle management: `poppy_version`, `poppy_open`, `poppy_close`, `poppy_last_error`.
  - Stream metadata: `poppy_get_stream_count`, `poppy_get_stream_info`, `poppy_get_duration_seconds`, `poppy_get_codec_name`.
  - Seeking and frame extraction: `poppy_read_frame`, `poppy_seek`, `poppy_seek_seconds`, `poppy_free_frame`.
  - Direct zero-copy GPU handle exports: `poppy_frame_export_dmabuf`, `poppy_frame_export_dxgi`, `poppy_frame_export_cvpixelbuffer`.
  - Exception safety: All entry points wrapped with `catch_unwind(AssertUnwindSafe(...))`.
- **Architectural Specification**: Comprehensive technical architectural guide in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## [0.9.0] - 2026-10-09

### Added
- Comprehensive libFuzzer and AddressSanitizer (ASan) suite with 6 dedicated targets in `fuzz/`.
- Property-based testing (`proptest`) for bitstream readers, VINT, LEB128, and Exp-Golomb codecs.

### Fixed
- Fixed allocator OOM in ISOBMFF `StszTable` by enforcing maximum sample bounds.
- Fixed 367 GB allocation panic on malformed HEVC SPS `num_short_term_ref_pic_sets`.
- Fixed integer overflow in H.264 PPS `skip_scaling_list`.
- Fixed underflow in HEVC slice segment address calculations.
- Fixed channel deadlock in `AsyncMediaStream` with cooperative reservation.

---

## [0.8.0] - 2026-10-09

### Added
- Ergonomic Tokio `Stream<Item = Result<DecodedFrame, PoppyError>>` in `poppy::AsyncMediaStream`.
- `poppy::MediaStreamBuilder` with buffer capacity tuning and asynchronous hardware decoder detection.
- Sub-millisecond timeline seeking and bidirectional frame scrubbing (`step_forward`, `step_backward`).
- Bounded asynchronous backpressure preventing unbounded memory growth.

---

## [0.7.0] - 2026-10-09

### Added
- Windows Direct3D 12 Video (`D3D12_VIDEO_DECODE`) hardware decoder HAL backend.
- DXGI Shared Handle zero-copy texture export for D3D12 and wgpu.
- macOS Apple VideoToolbox hardware decode backend with CoreVideo `CVPixelBuffer` export for Metal `IOSurface` textures.

---

## [0.6.0] - 2026-10-09

### Added
- Direct Vulkan zero-copy GPU pipeline: DRM PRIME DMA-BUF fd to `wgpu::Texture` via `wgpu-hal` and `VK_EXT_image_drm_format_modifier`.
- Zero staging memory copies on Linux DRM PRIME.

---

## [0.5.0] - 2026-10-09

### Added
- Interactive desktop GUI video player (`poppy play`) built on `winit 0.30` and `wgpu 24`.
- Real-time PTS clock synchronization, keyboard navigation (Space, Arrows, R, Q).
- Multi-slice picture batching in `poppy-hw`.
- Bitstream error concealment and reference frame freeze on dropped packets.
- MBAFF and PAFF interlacing detection with progressive fallback.

---

## [0.4.0] - 2026-10-09

### Added
- AV1 Open Bitstream Unit (OBU) parser (`crates/poppy-av1`) with Sequence Header and Frame Header parsing.
- Linux VA-API AV1 hardware decoding engine (`VAProfileAV1Profile0`).
- Audio packet pass-through demuxing for AAC, Opus, and FLAC tracks.

---

## [0.3.0] - 2026-10-09

### Added
- HEVC / H.265 bitstream parser (`crates/poppy-hevc`) supporting VPS, SPS, PPS, and Slice Headers.
- VA-API HEVC Main and Main 10 (10-bit HDR) hardware decoding.
- `wgpu` rendering subsystem with WGSL shaders for NV12 and P010 color space conversion (BT.601, BT.709, BT.2020).

---

## [0.2.0] - 2026-10-09

### Added
- Linux VA-API hardware acceleration engine (`crates/poppy-hw`) with DRM render node discovery.
- Decoded Picture Buffer (DPB) state machine with sliding-window reference frame tracking.
- Matroska / EBML demuxer with cue-based seeking (`crates/poppy-demux`).
- H.264 NAL parser and slice header decoder (`crates/poppy-h264`).

---

## [0.1.0] - 2026-10-09

### Added
- Pure safe-Rust ISOBMFF / MP4 container box parser (`crates/poppy-demux`).
- AVCC to Annex-B bitstream filter.
- `poppy-cli probe` and `poppy-cli dump-es` inspection utilities.
