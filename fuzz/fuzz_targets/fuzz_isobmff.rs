#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use poppy_demux::{Demuxer, Mp4Demuxer};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let bytes = Bytes::copy_from_slice(data);
    if let Ok(mut demuxer) = Mp4Demuxer::from_bytes(bytes) {
        let _ = demuxer.streams();

        // Read up to 100 packets to test parsing resilience
        let mut count = 0;
        while let Ok(Some(_pkt)) = demuxer.read_packet() {
            count += 1;
            if count > 100 {
                break;
            }
        }

        // Test seeking resilience
        let _ = demuxer.seek_to_keyframe(0, 0);
        let _ = demuxer.seek_to_keyframe(0, i64::MAX / 2);
    }
});
