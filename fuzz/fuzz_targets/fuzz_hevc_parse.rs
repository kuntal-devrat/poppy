#![no_main]

use libfuzzer_sys::fuzz_target;
use poppy_hevc::{
    HevcNalHeader, PictureParameterSet, SequenceParameterSet, SliceSegmentHeader,
    VideoParameterSet, remove_emulation_prevention, split_annex_b_nalus,
};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Fuzz Annex-B NAL unit splitting
    let units = split_annex_b_nalus(data);
    for u in &units {
        if !u.data.is_empty() {
            let _ = HevcNalHeader::parse(u.data);
            let rbsp = remove_emulation_prevention(u.data);
            let _ = VideoParameterSet::parse(&rbsp);
            let _ = SequenceParameterSet::parse(&rbsp);
            let _ = PictureParameterSet::parse(&rbsp);
        }
    }

    // 2. Direct parameter set fuzzing on raw input
    let rbsp = remove_emulation_prevention(data);
    let _vps_res = VideoParameterSet::parse(&rbsp);
    let sps_res = SequenceParameterSet::parse(&rbsp);
    let pps_res = PictureParameterSet::parse(&rbsp);

    // 3. Slice segment header fuzzing if valid SPS and PPS exist
    if let (Ok(sps), Ok(pps)) = (sps_res, pps_res) {
        let _ = sps.cropped_width();
        let _ = sps.cropped_height();
        let _ = sps.is_10bit();

        for u in units {
            if u.data.len() > 2 {
                if let Ok(hdr) = HevcNalHeader::parse(u.data) {
                    if hdr.nal_unit_type.is_vcl() {
                        let slice_rbsp = remove_emulation_prevention(&u.data[2..]);
                        let _ = SliceSegmentHeader::parse(&slice_rbsp, hdr.nal_unit_type, &sps, &pps);
                    }
                }
            }
        }
    }
});
