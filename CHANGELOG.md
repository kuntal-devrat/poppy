# Changelog

All notable changes to Project Poppy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.6.1] - 2026-10-16

### Added
- **Audio Stream Selection**: Added `audio_stream_index` override to `MediaStreamBuilder` and filtered stream discovery by `s.enabled`.

### Optimized
- **True Zero-Copy Matroska Demuxing**: Wrapped `memmap2::Mmap` in `Bytes::from_owner` in `poppy-demux`, replacing `copy_from_slice` with $O(1)$ `Bytes::slice` calls for MKV and WebM containers.
- **MP4 Seek Optimization**: Eliminated redundant consecutive keyframe searches in `Mp4Demuxer::seek_to_keyframe`.

### Fixed
- **Render Handle Isolation**: Removed hardcoded mock handles from `poppy-render` production code paths, isolating test mocks to test suites.
- **Audio Config Validation**: Validated sample rate and channel count prior to Symphonia decoder construction.
- **Version Alignment**: Made CLI version dynamic via clap derive and aligned C ABI test assertions with package version.

---

## [0.6.0] - 2026-10-12

### Added
- **MPEG-2 Transport Stream (MPEG-TS) Demuxer**:
  - Auto-detection of 188-byte (standard), 192-byte (M2TS BDAV), and 204-byte (FEC) packet cadence.
  - PAT (Program Association Table) and PMT (Program Map Table) parsing to identify elementary video/audio streams.
  - Multi-packet PES reassembly across TS packets with continuity counter monitoring.
  - 33-bit rollover-aware PTS/DTS timestamp parsing (90 kHz timebase).
  - Fast keyframe seeking via adaptation field random access indicators and NAL unit IDR inspection.
  - Automatic container dispatch in `MediaStream::open` for `.ts`, `.mts`, and `.m2ts` files.
- **Asynchronous Tokio Stream**:
  - `Stream<Item = Result<DecodedFrame, PoppyError>>` implementation in `poppy::AsyncMediaStream`.
  - `poppy::MediaStreamBuilder` with configurable buffer capacity for bounded backpressure.
  - Sub-millisecond timeline seeking and bidirectional frame stepping (`step_forward`, `step_backward`).
- **Universal C ABI (`libpoppy.so`)**:
  - C-compatible shared library output in `crates/poppy` and canonical header [`include/poppy.h`](include/poppy.h).
  - Lifecycle management: `poppy_open`, `poppy_close`, `poppy_last_error`.
  - Stream metadata and frame extraction: `poppy_read_frame`, `poppy_read_audio_frame`, `poppy_seek`, `poppy_seek_seconds`.
  - Zero-copy GPU handle exports: `poppy_frame_export_dmabuf`, `poppy_frame_export_dxgi`, `poppy_frame_export_cvpixelbuffer`.
  - Panic barriers wrapping all FFI entry points with `catch_unwind`.
- **Dynamic Software Decode Fallback**:
  - Integrated OpenH264 dynamic loading backend with automatic runtime resolution.
  - I420 to NV12 software color plane packaging for direct `wgpu` rendering.
  - Added `software_fallback(bool)` option to `MediaStreamBuilder`.
- **Edit List (`elst` / `edts`) Mapping**:
  - ISO/IEC 14496-12 edit list parser with dwell edit detection, initial sample offset trimming, and gapless audio playback.
- **Security & Property Tests**:
  - 6 dedicated `cargo-fuzz` targets with AddressSanitizer in `fuzz/`.
  - Property-based testing (`proptest`) for bitstream readers, VINT, LEB128, and Exp-Golomb codecs.

### Fixed
- Fixed allocator OOM in ISOBMFF `StszTable` by enforcing maximum sample bounds.
- Fixed allocation panic on malformed HEVC SPS `num_short_term_ref_pic_sets`.
- Fixed integer overflow in H.264 PPS `skip_scaling_list`.
- Fixed underflow in HEVC slice segment address calculations.
- Fixed channel deadlock in `AsyncMediaStream` with cooperative reservation.
- Checked `libc::dup()` return codes in `DmaBufDescriptor::clone()` and `DmaBufImage::try_from()`.
- Propagated SPS/PPS parameter set submission errors in pipeline initialization instead of swallowing errors.

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
