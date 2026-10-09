# 🌸 Poppy

> **Crisp, memory-safe media pipeline with zero-copy hardware decode.**

Poppy is a pure safe-Rust media container and zero-copy hardware acceleration pipeline. It bypasses legacy C library overhead and archaic function pointers, feeding raw compressed NAL units directly into GPU video decode ASICs via DMA buffers.

---

## ✨ Features

- **Pure Safe-Rust Container Demuxers**:
  - **ISOBMFF (MP4 / MOV)**: Full parsing of `ftyp`, `moov`, `mdat`, `trak`, `mdia`, `minf`, `stbl` sample tables (`stts`, `ctts`, `stss`, `stsc`, `stsz`, `stco`, `co64`), and codec configuration (`avcC`, `hvcC`).
  - **Matroska / WebM (EBML)**: Spec-compliant RFC 8794 variable-size integer (VINT) parser, segment tree walker, cluster, and `SimpleBlock` sample demuxing.
- **Minimal H.264 Bitstream Parser (`poppy-h264`)**:
  - Emulation prevention byte removal (`0x000003` stripping).
  - Bitstream Exponential-Golomb decoders (`ue(v)`, `se(v)`, `te(v)`).
  - SPS & PPS parameter extractors.
  - Slice header parser that calculates `slice_data_bit_offset` and macroblock offsets required by hardware decoders.
- **Hardware Decoder HAL (`poppy-hw`)**:
  - Linux **VA-API** backend talking directly to DRM render nodes (`/dev/dri/renderD*`).
  - Fixed-function ASIC submission (`VAPictureParameterBufferH264`, `VAIQMatrixBufferH264`, `VASliceParameterBufferH264`).
  - Decoded Picture Buffer (DPB) state machine with sliding-window reference frame management.
- **Zero-Copy DMA-BUF Interop (`poppy-render`)**:
  - Exports decoded surfaces directly as Linux DMA-BUF PRIME file descriptors (`vaDeriveImage` / `vaExportSurfaceHandle`) without CPU memory copies.
  - Ready for direct import into Vulkan (`VkImportMemoryFdInfoKHR`) or OpenGL / EGL.
- **Ergonomic Facade API (`poppy`)**:
  - Clean `poppy::open("video.mp4")` builder interface.
  - Unified typed errors.
- **Fuzz Testing Suite (`poppy-fuzz`)**:
  - `libFuzzer` targets for MP4 box parsing, EBML parsing, and H.264 bitstream parsing.

---

## 🏗️ Workspace Structure

```text
poppy/
├── Cargo.toml                       # Monorepo workspace
├── bins/
│   └── poppy-cli/                   # Command-line tool: probe & dump-es
├── crates/
│   ├── poppy-core/                  # Zero-dependency foundational types (Packet, TimeBase, CodecId)
│   ├── poppy-demux/                 # Safe MP4 & MKV container demuxers + Annex-B BSF
│   ├── poppy-h264/                  # SPS/PPS and slice header bitstream parser
│   ├── poppy-hw/                    # VA-API hardware decode submission engine & DPB
│   ├── poppy-render/                # DMA-BUF PRIME descriptors & GPU texture bridges
│   └── poppy/                       # High-level ergonomic facade crate
└── fuzz/                            # cargo-fuzz targets for libFuzzer
```

---

## 🚀 Quickstart

Add Poppy to your `Cargo.toml`:

```toml
[dependencies]
poppy = "0.1.0"
```

### High-Level API

```rust
use poppy;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Automatically probes container format and attaches hardware GPU decoder
    let mut stream = poppy::open("input.mp4")?;

    for s in stream.streams() {
        println!("Stream #{}: codec={:?}, timebase={}", s.index, s.codec, s.time_base);
    }

    // Demux and hardware-decode frames directly on the GPU
    while let Some(frame) = stream.next_frame()? {
        println!(
            "Hardware decoded frame: {}x{}, format={:?}, pts={:?}",
            frame.width, frame.height, frame.pixel_format, frame.pts
        );
    }

    Ok(())
}
```

### Demuxing Directly

```rust
use poppy_demux::{Demuxer, Mp4Demuxer};

let mut demuxer = Mp4Demuxer::open("input.mp4")?;
while let Some(packet) = demuxer.read_packet()? {
    println!(
        "Packet: stream={}, dts={}, size={} bytes, keyframe={}",
        packet.stream_index, packet.dts, packet.data.len(), packet.is_keyframe
    );
}
```

### Hardware Decoding (Linux VA-API)

```rust
use poppy_hw::{HwDecoder, VaapiH264Decoder};

let mut decoder = VaapiH264Decoder::new()?;
decoder.submit_packet(&packet)?;

if let Some(frame) = decoder.receive_frame()? {
    println!("GPU Frame: {}x{}", frame.width, frame.height);
}
```

---

## 🛠️ CLI Usage

```bash
# Inspect container metadata, codecs, and keyframe statistics
poppy-cli probe video.mp4

# Dump elementary Annex-B stream (.h264) from MP4 container
poppy-cli dump-es video.mp4 -o stream.h264
```

---

## 🧪 Testing & Quality

Run the test suite across all workspace crates:

```bash
cargo test --workspace
```

Run libFuzzer targets:

```bash
cargo check --manifest-path fuzz/Cargo.toml
```

---

## 📄 License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
