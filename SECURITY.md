# Security Policy

Project Poppy considers memory safety, bitstream parsing robustness, and driver sandboxing to be critical core principles.

## Supported Versions

Only the latest major release receives active security patches.

| Version | Supported          |
| ------- | ------------------ |
| 0.6.x   | :white_check_mark: |
| < 0.6.0 | :x:                |

## Security Architecture & Philosophy

1. **Pure Safe Rust Demuxers**:
   All container parsers (ISOBMFF for MP4, EBML for Matroska) and bitstream headers (H.264, HEVC, AV1) are written in 100% safe Rust. No untrusted input can trigger buffer overflows, use-after-free, or arbitrary code execution in the demuxing phase.
2. **Continuous Fuzzing**:
   Every parser is hardened with continuous AddressSanitizer (ASan) and libFuzzer suites in `fuzz/`.
3. **Hardware Driver Isolation**:
   Hardware decode calls (VA-API on Linux, D3D12 Video on Windows, VideoToolbox on macOS) are strictly validated before hardware buffer submission.

## Reporting a Vulnerability

If you discover a potential vulnerability in Project Poppy, please do **NOT** open a public issue.

Please report vulnerabilities privately via email to:
**`security@projectpoppy.dev`** (or create a Private Vulnerability Advisory on GitHub).

Include:
- A description of the vulnerability and attack vector.
- A minimal reproducing sample file or test case.
- Impact assessment (denial-of-service, GPU driver panic, memory leak).

### Response SLA
- Initial response within **48 hours**.
- Patch timeline agreed upon within **7 business days**.
- Coordinated public disclosure after a patch is released to crates.io.
