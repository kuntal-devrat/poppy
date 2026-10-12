# Changelog

All notable changes to Project Poppy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.6.0] - 2026-10-12

### Added
- **Full MPEG-2 Transport Stream (MPEG-TS) Demuxer**:
  - Auto-detection of 188-byte (standard), 192-byte (M2TS BDAV), and 204-byte (FEC) packet cadence.
  - Full PAT (Program Association Table) and PMT (Program Map Table) parsing to identify elementary video/audio streams.
  - Multi-packet Packetized Elementary Stream (PES) reassembly across TS packets with continuity counter monitoring.
  - 33-bit rollover-aware PTS/DTS timestamp parsing (90 kHz timebase).
  - Fast keyframe seeking via adaptation field random access indicators and NAL unit IDR inspection.
  - Seamless container dispatch in `MediaStream::open` for `.ts`, `.mts`, and `.m2ts` files.
- **Real macOS VideoToolbox Hardware Decoder**:
  - Dynamic `dlopen` loading of CoreMedia, VideoToolbox, and CoreVideo frameworks with zero static C build dependencies.
  - Creation of real `VTDecompressionSession` from parameter sets (`avcC` / `hvcC`).
  - Submission of `CMSampleBuffer`s and asynchronous frame extraction into decoded `CVPixelBuffer`s with zero-copy Metal `IOSurface` descriptors.
- **Real Windows Direct3D 12 Video Decoder**:
  - Dynamic COM runtime loading of `D3D12CreateDevice` and query for `ID3D12VideoDevice`.
  - Multi-slice picture batching with DXVA picture parameter assembly (`DXVA_PicParams_H264`) and Texture2D surface pools.
  - Zero-copy DXGI shared handle export (`ID3D12Device::CreateSharedHandle`).
- **Dynamic Software Decode Fallback**:
  - Integrated OpenH264 dynamic loading backend with automatic runtime resolution.
  - I420 to NV12 software color plane packaging for direct `wgpu` rendering.
  - Added `software_fallback(bool)` option to `MediaStreamBuilder` for headless CI and GPU-less environments.
- **Full Audio C ABI**:
  - Added `poppy_has_audio`, `poppy_audio_get_config`, `poppy_read_audio_frame`, and `poppy_audio_frame_get_samples` to `include/poppy.h` and `libpoppy`.
  - Thread-safe multi-threaded concurrent access tests in `test_c_abi.rs`.
- **Edit List (`elst` / `edts`) Gapless Mapping**:
  - Full ISO/IEC 14496-12 edit list parser with dwell edit detection, initial sample offset trimming, and gapless audio playback.
- **Vorbis Audio Support**:
  - Enabled Vorbis decoding across `poppy-core`, `poppy-demux`, and `poppy-audio` (Symphonia backend).
- **Comprehensive Integration Tests & Property Tests**:
  - fMP4 / DASH fragment parsing tests (`test_fmp4_demux.rs`).
  - Matroska / WebM integration tests (`test_mkv_demux.rs`).
  - MPEG-TS integration tests (`test_mpegts_demux.rs`).
  - Software fallback tests (`test_software_fallback.rs`).
  - A/V sync verification tests (`test_av_sync.rs`).
  - Full doc-tests across all 10 workspace crates.

### Optimized
- **Zero-Copy Memory-Mapped Packet Extraction**: Wrapped mmap buffers in `Bytes::from_owner` to eliminate heap copies during demuxing.
- **O(log N) Binary Search Track Synchronization**: Replaced linear O(N) sample scanning on multi-track seeks with binary search DTS synchronization.

### Fixed
- **BUG-1**: Fixed proptest facade compilation error on integer bounds assertion.
- **BUG-2 & BUG-3**: Checked `libc::dup()` return codes in `DmaBufDescriptor::clone()` and `DmaBufImage::try_from()`, adding `try_clone()` to prevent silent fd table exhaustion errors.
- **BUG-5**: Propagated SPS/PPS parameter set submission errors in pipeline initialization instead of silently swallowing errors.
- **BUG-6**: Rejected `sample_rate == 0` configs instead of silently setting 1 Hz.
- **BUG-7**: Implemented real frame duration estimation in `estimate_frame_duration_seconds()` for accurate backward stepping.
- **Safety**: Added `#![deny(unsafe_op_in_unsafe_fn)]` across hardware and render crates.

---

## [0.5.5] - 2026-10-12

### Fixed
- **Pixel-Perfect H.264 & HEVC Continuous POC MSB Tracking**: Implemented full `PicOrderCntMsb` state machine per ITU-T H.264 §8.2.1.1 / §8.2.1.3 and ITU-T H.265 §8.3.1. Eliminates `pic_order_cnt_lsb` rollover corruptions where wrapped frames were bumped out of order and inverted reference lists caused macroblock tearing.
- **H.264 Weighted Prediction (`pred_weight_table`) Support**: Added complete prediction weight table parsing in `poppy-h264` and passed luma/chroma weights, denoms, and offsets into `VASliceParameterBufferH264` to eliminate brightness/color flashing during video cross-fades and transitions.
- **Reference Picture List Modification (RPLR)**: Implemented H.264 §8.2.4.3 list modification in DPB `fill_ref_pic_list0` and `fill_ref_pic_list1` to properly support reordered motion prediction on retimed/slow-motion effects.

### Added
- **Monotonic Presentation Regression Test**: Added `test_hw_decode_verify_monotonic_pts` in `crates/poppy-hw` asserting strict monotonic timestamp order across multi-B-frame sequences.

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
