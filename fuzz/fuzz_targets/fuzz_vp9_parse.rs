#![no_main]

use libfuzzer_sys::fuzz_target;
use poppy_vp9::{Vp9FrameHeader, parse_superframe_frames};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    // Superframe index parsing (validates exact-fit + size checks).
    if let Ok(frames) = parse_superframe_frames(data) {
        for f in frames {
            // Uncompressed header parsing with and without reference dims.
            let _ = Vp9FrameHeader::parse(f, None);
            let dims = [(1920, 1080); 8];
            let _ = Vp9FrameHeader::parse(f, Some(&dims));
        }
    } else {
        let _ = Vp9FrameHeader::parse(data, None);
    }
});
