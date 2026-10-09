#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use poppy_demux::{Demuxer, MatroskaDemuxer};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let bytes = Bytes::copy_from_slice(data);
    if let Ok(mut demuxer) = MatroskaDemuxer::from_bytes(bytes) {
        let _ = demuxer.streams();

        let mut count = 0;
        while let Ok(Some(_pkt)) = demuxer.read_packet() {
            count += 1;
            if count > 100 {
                break;
            }
        }

        let _ = demuxer.seek_to_keyframe(0, 0);
        let _ = demuxer.seek_to_keyframe(0, 5000);
    }
});
