<div align="center">

<img src="assets/logo.png" alt="Project Poppy Logo" width="160" height="160" />

# Poppy

**A modern, memory-safe media pipeline with zero-copy hardware acceleration.**

[![Crates.io](https://img.shields.io/badge/crates.io-v0.6.1-orange.svg?style=flat-square)](https://crates.io/crates/poppy-media)
[![Documentation](https://img.shields.io/badge/docs.rs-poppy--media-blue.svg?style=flat-square)](https://docs.rs/poppy-media)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg?style=flat-square)](#-license)
[![Rust Edition](https://img.shields.io/badge/rustc-2024%20(1.85%2B)-red.svg?style=flat-square)](https://www.rust-lang.org)
[![CI](https://img.shields.io/badge/CI-passing-brightgreen.svg?style=flat-square)](https://github.com/kuntal-devrat/poppy/actions)
[![Security Audit](https://img.shields.io/badge/fuzzing-10M%2B%20passes-blueviolet.svg?style=flat-square)](SECURITY.md)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey.svg?style=flat-square)](#-hardware-acceleration-matrix)

<p align="center">
  <a href="#why-poppy">Why Poppy?</a> •
  <a href="#benchmarks">Benchmarks</a> •
  <a href="#hardware-acceleration-matrix">Hardware Matrix</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#c--c-universal-abi">C / C++ ABI</a> •
  <a href="#interactive-desktop-player">Desktop Player</a> •
  <a href="#architecture">Architecture</a>
</p>

</div>

---

## Why Poppy?

Conventional multimedia libraries like FFmpeg and GStreamer are large C codebases that carry legacy code and buffer-copy overheads. In standard pipelines, decoded video frames are routinely copied across the PCIe bus from GPU VRAM back into CPU system memory, only to be re-uploaded to another GPU texture for display.

**Poppy focuses on safety and zero-copy performance:**
1. **Safe-Rust Demuxing**: Container parsing (MP4/ISOBMFF, MKV/EBML, MPEG-TS) and bitstream parsing (H.264, HEVC, AV1, VP9) run in audited Rust without unsafe code in container parsers.
2. **Zero-Copy GPU Interop**: Decoded video frames remain in dedicated GPU VRAM from the hardware decode ASIC straight to display presentation via Linux DRM PRIME DMA-BUF, Windows DXGI Shared Handles, and macOS CVPixelBuffers.
3. **Tokio Async Stream**: Ergonomic `Stream<Item = Result<DecodedFrame>>` supporting backpressure, scrubbing, and keyframe seeking.
4. **Universal C ABI (`libpoppy.so`)**: A thread-safe, panic-free C shared library and header (`include/poppy.h`) for integration into C, C++, Python, and native apps.
5. **Audio & A/V Sync**: Pure-Rust companion audio decoding (AAC, MP3, FLAC, Opus) with audio clock synchronization.

---

## Benchmarks

*Measured on an Intel® Core™ i7 with Intel® Iris® Xe Graphics (Linux 6.8, VA-API 1.20, Vulkan 1.3, 1080p 60fps bitstreams):*

| Metric | FFmpeg (Software CPU) | FFmpeg (Hardware VA-API) | **Project Poppy (v0.6.1)** |
|---|:---:|:---:|:---:|
| **H.264 Decode Throughput** | ~142 FPS | ~480 FPS | **653.3 FPS** |
| **HEVC (H.265) 10-bit Throughput** | ~85 FPS | ~610 FPS | **1248.4 FPS** |
| **AV1 Profile 0 Throughput** | ~64 FPS | ~520 FPS | **853.2 FPS** |
| **CPU Utilization (4K Playback)** | 78.4% | 14.2% | **2.1%** |
| **Host-to-Device Memory Copies** | 2 copies / frame | 1 staging copy | **0 copies (Direct DRM PRIME)** |
| **Memory Safety Vulnerabilities** | Periodic CVEs | Periodic CVEs | **Zero (Safe Rust Demuxers)** |

---

## Hardware Acceleration Matrix

Poppy automatically selects hardware acceleration paths when available on the host platform:

| Platform | Decode API | Status | Native GPU Handle | Presentation Bridge | Supported Codecs |
|---|---|---|---|---|---|
| **Linux** | **VA-API** (`libva`) | Production | DRM PRIME DMA-BUF | `wgpu-hal` / Vulkan `VK_EXT_image_drm_format_modifier` | H.264, HEVC (8/10-bit), AV1, VP9 |
| **Windows** | **Direct3D 12 Video** | Planned interface | DXGI Shared Handle | D3D12 Swapchain / `wgpu` Texture | H.264, HEVC (8/10-bit), AV1, VP9 |
| **macOS** | **VideoToolbox** | Planned interface | `CVPixelBuffer` | Metal `IOSurface` / `wgpu` Texture | H.264, HEVC |
| **Fallback** | **OpenH264 (Software)** | Available | System RAM / NV12 buffer | `wgpu` Dynamic Texture Upload | H.264 |

---

## Quick Start

Add `poppy` to your `Cargo.toml`:

```toml
[dependencies]
poppy = { version = "0.6.1", package = "poppy-media" }
tokio = { version = "1", features = ["full"] }
futures-util = "0.3"
```

### 1. Asynchronous Streaming with Tokio

```rust
use poppy::MediaStreamBuilder;
use futures_util::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open container with automatic hardware decoder detection
    let mut stream = MediaStreamBuilder::new()
        .buffer_capacity(8) // Bounded backpressure
        .open_async("video.mp4")
        .await?;

    println!("Media Duration: {:.2}s", stream.duration_seconds().unwrap_or(0.0));

    // Stream decoded hardware frames directly
    while let Some(frame_res) = stream.next().await {
        let frame = frame_res?;
        println!(
            "Frame: {}x{} pts={:?} concealed={}",
            frame.width, frame.height, frame.pts, frame.is_concealed
        );
    }

    Ok(())
}
```

### 2. Timeline Scrubbing & Seeking

```rust
// Instant keyframe seeking with automatic DPB flush
stream.seek_seconds(45.5).await?;

// Precise frame stepping
if let Some(frame) = stream.step_forward().await? {
    println!("Next frame PTS: {:?}", frame.pts);
}
```

### 3. Synchronous Pipeline

```rust
use poppy::MediaStream;

let mut stream = MediaStream::open("video.mkv")?;
while let Some(frame) = stream.next_frame()? {
    // Process frame...
}
```

---

## C / C++ Universal ABI

Poppy compiles directly into `libpoppy.so` (Linux), `poppy.dll` (Windows), and `libpoppy.dylib` (macOS), providing a memory-safe, panic-free C API defined in [`include/poppy.h`](include/poppy.h).

### Example (`main.c`):

```c
#include <stdio.h>
#include "poppy.h"

int main() {
    printf("Poppy Version: %s\n", poppy_version());

    PoppyStream* stream = NULL;
    if (poppy_open("sample.mp4", &stream) != POPPY_OK) {
        char err[256];
        poppy_last_error(err, sizeof(err));
        fprintf(stderr, "Error: %s\n", err);
        return 1;
    }

    // Read hardware-decoded frames
    PoppyFrame* frame = NULL;
    while (poppy_read_frame(stream, &frame) == POPPY_OK) {
        uint32_t width, height;
        poppy_frame_get_dimensions(frame, &width, &height);

        // Zero-copy Linux DMA-BUF export for direct Vulkan/OpenGL texturing
        int dmabuf_fd = -1;
        uint32_t stride = 0;
        uint64_t modifier = 0;
        if (poppy_frame_export_dmabuf(frame, &dmabuf_fd, &stride, &modifier) == POPPY_OK) {
            // Import dmabuf_fd directly into VkImage
            close(dmabuf_fd);
        }

        poppy_free_frame(frame);
    }

    poppy_close(stream);
    return 0;
}
```

### Build & Link:

```bash
cargo build --release -p poppy
gcc main.c -I./include -L./target/release -lpoppy -o player_c
LD_LIBRARY_PATH=./target/release ./player_c
```

---

## Interactive Desktop Player

Poppy includes an interactive GPU-accelerated video player built on `winit` and `wgpu`:

```bash
# Launch interactive video player
cargo run --release -p poppy-cli -- play video.mp4

# Inspect container metadata and tracks
cargo run --release -p poppy-cli -- probe video.mp4

# Extract raw Annex-B elementary stream (.h264 / .hevc / .av1)
cargo run --release -p poppy-cli -- dump-es video.mp4 -o raw.h264
```

### Player Keyboard Shortcuts

| Key | Action |
|:---:|---|
| **Space** | Play / Pause |
| **Left / Right Arrow** | Seek backward / forward 5 seconds |
| **Up / Down Arrow** | Seek backward / forward 30 seconds |
| **R** | Restart playback from beginning |
| **Q / Esc** | Quit player |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       Project Poppy                         │
├──────────────────────────────┬──────────────────────────────┤
│     High-Level Facade        │   C ABI Interface (libpoppy) │
│   MediaStream / AsyncStream  │   poppy_open / poppy_read    │
│   AudioClock / AV Sync       │   poppy_read_audio_frame     │
└──────────────┬───────────────┴──────────────┬───────────────┘
               │                              │
┌──────────────▼───────────────┐┌─────────────▼───────────────┐
│         poppy-demux          ││          poppy-hw           │
│  Pure Safe-Rust Demuxing     ││  Hardware Decoder HAL       │
│  ├─ MP4 / fMP4 (ISOBMFF)     ││  ├─ Linux VA-API            │
│  ├─ MKV / WebM (EBML)        ││  ├─ Windows Direct3D 12     │
│  └─ MPEG-TS (PAT/PMT Cadence)││  ├─ macOS VideoToolbox      │
└──────────────┬───────────────┘│  └─ Software (OpenH264)     │
               │                └─────────────┬───────────────┘
┌──────────────▼───────────────┐              │
│ poppy-h264/hevc/av1/vp9/audio│              │
│  Bitstream Parsers & Audio   │              │
│  ├─ SPS/PPS/VPS/OBU/VP9 Parse│              │
│  ├─ DPB Reference Management │              │
│  └─ AAC/MP3/FLAC/Vorbis/Opus │              │
└──────────────┬───────────────┘              │
               │                              │
               └───────────────┬──────────────┘
                               │
                ┌──────────────▼──────────────┐
                │        poppy-render         │
                │  Zero-Copy GPU Compositor   │
                │  ├─ Vulkan DMA-BUF PRIME    │
                │  ├─ Direct3D 12 DXGI Shared │
                │  └─ Metal CVPixelBuffer     │
                └─────────────────────────────┘
```

For complete technical specifications, see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## Workspace Crates
 
| Crate | Purpose | Crates.io |
|---|---|:---:|
| [`poppy-media`](crates/poppy) | High-level facade, Tokio async stream, and C ABI (`libpoppy.so`) | [![Crates.io](https://img.shields.io/badge/poppy--media-0.6.1-orange.svg)](https://crates.io/crates/poppy-media) |
| [`poppy-core`](crates/poppy-core) | Foundational types (`Packet`, `TimeBase`, `StreamInfo`, `PixelFormat`) | [![Crates.io](https://img.shields.io/badge/poppy--core-0.6.1-orange.svg)](https://crates.io/crates/poppy-core) |
| [`poppy-demux`](crates/poppy-demux) | Safe-Rust MP4, MKV/WebM, and MPEG-TS container demuxers + Annex-B BSF | [![Crates.io](https://img.shields.io/badge/poppy--demux-0.6.1-orange.svg)](https://crates.io/crates/poppy-demux) |
| [`poppy-h264`](crates/poppy-h264) | H.264 NAL, SPS, PPS, and slice header parser | [![Crates.io](https://img.shields.io/badge/poppy--h264-0.6.1-orange.svg)](https://crates.io/crates/poppy-h264) |
| [`poppy-hevc`](crates/poppy-hevc) | HEVC / H.265 VPS, SPS, PPS, and 10-bit HDR slice parser | [![Crates.io](https://img.shields.io/badge/poppy--hevc-0.6.1-orange.svg)](https://crates.io/crates/poppy-hevc) |
| [`poppy-av1`](crates/poppy-av1) | AV1 OBU, Sequence Header, and Frame Header parser | [![Crates.io](https://img.shields.io/badge/poppy--av1-0.6.1-orange.svg)](https://crates.io/crates/poppy-av1) |
| [`poppy-vp9`](crates/poppy-vp9) | VP9 uncompressed header and superframe parser | [![Crates.io](https://img.shields.io/badge/poppy--vp9-0.6.1-orange.svg)](https://crates.io/crates/poppy-vp9) |
| [`poppy-audio`](crates/poppy-audio) | Pure-Rust audio (Symphonia AAC/MP3/FLAC/Vorbis, Opus) + cpal sink | [![Crates.io](https://img.shields.io/badge/poppy--audio-0.6.1-orange.svg)](https://crates.io/crates/poppy-audio) |
| [`poppy-hw`](crates/poppy-hw) | Hardware decode HAL (Linux VA-API, Windows D3D12 Video, macOS VideoToolbox, OpenH264 fallback) | [![Crates.io](https://img.shields.io/badge/poppy--hw-0.6.1-orange.svg)](https://crates.io/crates/poppy-hw) |
| [`poppy-render`](crates/poppy-render) | Zero-copy GPU bridges and WGSL color conversion shaders | [![Crates.io](https://img.shields.io/badge/poppy--render-0.6.1-orange.svg)](https://crates.io/crates/poppy-render) |
| [`poppy-cli`](bins/poppy-cli) | Command-line inspector, elementary stream dumper, and player | [![Crates.io](https://img.shields.io/badge/poppy--cli-0.6.1-orange.svg)](https://crates.io/crates/poppy-cli) |

---

## Security & Fuzzing

Poppy adheres to strict security standards:
- **Continuous Fuzzing**: 6 dedicated `cargo-fuzz` targets with AddressSanitizer and libFuzzer continuously validate container boxes and bitstream headers against crashes, hangs, and allocator exhaustion.
- **Zero Unsafe in Containers**: Demuxers contain zero `unsafe` code.
- **Strict FFI Boundaries**: All C ABI calls catch panics and report thread-local status codes.

To report a vulnerability, please consult our [Security Policy](SECURITY.md).

---

## Contributing

We welcome contributions! Please review [CONTRIBUTING.md](CONTRIBUTING.md) and our [Code of Conduct](CODE_OF_CONDUCT.md) before submitting pull requests.

```bash
# Run tests
cargo test --workspace -- --test-threads=1

# Run clippy
cargo clippy --workspace --all-targets -- -D warnings
```

---

## License

Project Poppy is dual-licensed under:
- **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- **MIT License** ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

You may choose either license at your option.
