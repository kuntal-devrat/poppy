#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use poppy_demux::{Demuxer, Mp4Demuxer};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    // Fragmented MP4 / DASH path: moof/trun/tfhd/tfdt + sidx/mfra/traf.
    let bytes = Bytes::copy_from_slice(data);
    if let Ok(demuxer) = Mp4Demuxer::from_bytes(bytes) {
        let _ = demuxer.sidx_segments();
        let _ = demuxer.mfra_entries();
        let mut d = demuxer;
        let mut count = 0;
        while let Ok(Some(_pkt)) = d.read_packet() {
            count += 1;
            if count > 100 {
                break;
            }
        }
        let _ = d.seek_to_keyframe(0, 0);
    }
});
