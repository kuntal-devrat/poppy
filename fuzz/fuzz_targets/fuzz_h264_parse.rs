#![no_main]

use libfuzzer_sys::fuzz_target;
use poppy_h264::{find_annex_b_nals, NalHeader, Pps, SliceHeader, Sps};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // Fuzz Annex-B NAL splitting
    let nals = find_annex_b_nals(data);
    for nal in &nals {
        if !nal.is_empty() {
            let _ = NalHeader::parse(nal[0]);
        }
    }

    // Fuzz SPS parsing
    let sps_res = Sps::parse(data);

    // Fuzz PPS parsing
    let pps_res = Pps::parse(data);

    // If both SPS and PPS somehow succeed on the fuzz input, fuzz slice parsing
    if let (Ok(sps), Ok(pps)) = (sps_res, pps_res) {
        for nal in nals {
            if nal.len() > 1 {
                let h = NalHeader::parse(nal[0]);
                if h.unit_type.is_slice() {
                    let _ = SliceHeader::parse(h.unit_type, h.nal_ref_idc, &nal[1..], &sps, &pps);
                }
            }
        }
    }
});
