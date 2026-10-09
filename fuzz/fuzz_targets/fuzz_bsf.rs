#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use poppy_demux::bsf::{avcc_to_annex_b, hevc_to_annex_b};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let nls_options = [1, 2, 4];
    let nls = nls_options[(data[0] as usize) % nls_options.len()];
    let payload = &data[1..];

    // Fuzz AVCC to Annex-B
    let mut out_avc = Vec::new();
    let _ = avcc_to_annex_b(payload, nls, None, &mut out_avc);

    // Fuzz with parameter sets
    let fake_sps = Bytes::copy_from_slice(&[0x67, 0x42, 0x00, 0x1f]);
    let fake_pps = Bytes::copy_from_slice(&[0x68, 0xce, 0x38, 0x80]);
    let params = [fake_sps.clone(), fake_pps.clone()];
    let mut out_avc_params = Vec::new();
    let _ = avcc_to_annex_b(payload, nls, Some(&params), &mut out_avc_params);

    // Fuzz HEVC to Annex-B
    let mut out_hevc = Vec::new();
    let _ = hevc_to_annex_b(payload, nls, None, None, None, &mut out_hevc);

    let fake_vps = Bytes::copy_from_slice(&[0x40, 0x01, 0x0c, 0x01]);
    let mut out_hevc_params = Vec::new();
    let _ = hevc_to_annex_b(
        payload,
        nls,
        Some(&[fake_vps]),
        Some(&[fake_sps]),
        Some(&[fake_pps]),
        &mut out_hevc_params,
    );
});
