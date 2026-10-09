#![no_main]

use libfuzzer_sys::fuzz_target;
use poppy_av1::{FrameHeader, ObuHeader, ObuType, SequenceHeader, read_leb128, split_obus};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Fuzz LEB128 decoding
    let _ = read_leb128(data);

    // 2. Fuzz OBU Header parsing
    let _ = ObuHeader::parse(data);

    // 3. Fuzz OBU Stream splitting
    if let Ok(obus) = split_obus(data) {
        let mut active_seq: Option<SequenceHeader> = None;

        for obu in obus {
            match obu.header.obu_type {
                ObuType::SequenceHeader => {
                    if let Ok(seq) = SequenceHeader::parse(obu.payload) {
                        active_seq = Some(seq);
                    }
                }
                ObuType::FrameHeader | ObuType::Frame => {
                    if let Some(ref seq) = active_seq {
                        let _ = FrameHeader::parse(obu.payload, seq);
                    }
                }
                _ => {}
            }
        }
    }

    // 4. Direct Sequence Header fuzzing on raw input
    let _ = SequenceHeader::parse(data);
});
