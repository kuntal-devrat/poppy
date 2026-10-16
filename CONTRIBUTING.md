# Contributing to Project Poppy

Thank you for your interest in contributing to Poppy! Whether you are reporting a bug, proposing an optimization, adding a hardware backend, or improving documentation, your help is welcome.

---

## Development Setup

### Prerequisites
- **Rust 1.85+** (Rust Edition 2024 support required).
- **Cargo**.
- Linux (for VA-API / Vulkan testing): `libva-dev`, `vainfo`, `libdrm-dev`, Vulkan SDK / Mesa drivers.
- Windows (optional): Direct3D 12 Video SDK.
- macOS (optional): macOS 13+ with VideoToolbox.

### Clone and Build
```bash
git clone https://github.com/kuntal-devrat/poppy.git
cd poppy
cargo build --workspace
```

---

## Testing Guidelines

Before opening a pull request, ensure all workspace tests pass:

```bash
# Run unit and integration tests (single-threaded for GPU device exclusivity)
cargo test --workspace -- --test-threads=1

# Verify strict Clippy compliance (0 warnings allowed)
cargo clippy --workspace --all-targets -- -D warnings

# Verify formatting
cargo fmt --all -- --check

# Verify documentation builds cleanly
cargo doc --workspace --no-deps
```

---

## Fuzz Testing

Poppy enforces structure-aware fuzzing on all container demuxers and bitstream parsers. If you modify parser code in `poppy-demux`, `poppy-h264`, `poppy-hevc`, or `poppy-av1`, you must run the relevant fuzz targets:

```bash
# Install cargo-fuzz
cargo install cargo-fuzz

# Run fuzzer with AddressSanitizer
cargo fuzz run fuzz_isobmff -- -max_total_time=60
cargo fuzz run fuzz_ebml -- -max_total_time=60
cargo fuzz run fuzz_h264_parse -- -max_total_time=60
cargo fuzz run fuzz_hevc_parse -- -max_total_time=60
cargo fuzz run fuzz_av1_parse -- -max_total_time=60
```

---

## Pull Request Process

1. **Fork and Branch**: Create a feature branch from `main` (e.g. `feat/vulkan-modifier-fallback`).
2. **Commit Conventions**: Use clear, concise commit messages adhering to Conventional Commits:
   - `feat: add support for AV1 Tile Group multi-threading`
   - `fix: prevent integer overflow in HEVC scaling list parser`
   - `perf: optimize DMA-BUF fd duplicate calls in Linux render bridge`
3. **Keep Unsafe Minimal**: Any `unsafe` blocks must be accompanied by explicit `// SAFETY:` comments detailing preconditions and invariant guarantees.
4. **Documentation**: Public APIs must contain docstrings with `# Example` and `# Errors` sections.
5. **Continuous Integration**: Ensure all CI gates pass green on GitHub Actions.
