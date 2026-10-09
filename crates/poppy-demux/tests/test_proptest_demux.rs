use bytes::Bytes;
use poppy_demux::bsf::{ANNEX_B_START_CODE_4, avcc_to_annex_b, hevc_to_annex_b};
use poppy_demux::ebml::vint::{read_vint_id, read_vint_size};
use proptest::prelude::*;

proptest! {
    /// Property: read_vint_id must never panic on arbitrary byte inputs,
    /// and if successful, consumed bytes must be within 1..=4.
    #[test]
    fn prop_vint_id_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64), offset in any::<u64>()) {
        if let Ok((_id, len)) = read_vint_id(&data, offset) {
            prop_assert!((1..=4).contains(&len));
            prop_assert!(len <= data.len());
        }
    }

    /// Property: read_vint_size must never panic on arbitrary byte inputs,
    /// and if successful, consumed bytes must be within 1..=8.
    #[test]
    fn prop_vint_size_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64), offset in any::<u64>()) {
        if let Ok((_size_opt, len)) = read_vint_size(&data, offset) {
            prop_assert!((1..=8).contains(&len));
            prop_assert!(len <= data.len());
        }
    }

    /// Property: avcc_to_annex_b must never panic on arbitrary inputs and length sizes.
    #[test]
    fn prop_avcc_to_annex_b_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..512),
        nls in any::<u8>(),
        param_data in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..64), 0..4)
    ) {
        let params: Vec<Bytes> = param_data.into_iter().map(Bytes::from).collect();
        let mut out = Vec::new();
        let res = avcc_to_annex_b(&data, nls, Some(&params), &mut out);
        if nls != 1 && nls != 2 && nls != 4 {
            prop_assert!(res.is_err());
        }
        // If it succeeds, verify output starts with 4-byte start codes if params existed
        if res.is_ok() && !params.is_empty() {
            prop_assert!(out.starts_with(&ANNEX_B_START_CODE_4));
        }
    }

    /// Property: hevc_to_annex_b must never panic on arbitrary inputs and length sizes.
    #[test]
    fn prop_hevc_to_annex_b_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..512),
        nls in any::<u8>(),
        vps in proptest::collection::vec(any::<u8>(), 0..64),
        sps in proptest::collection::vec(any::<u8>(), 0..64),
        pps in proptest::collection::vec(any::<u8>(), 0..64)
    ) {
        let vps_b = [Bytes::from(vps)];
        let sps_b = [Bytes::from(sps)];
        let pps_b = [Bytes::from(pps)];
        let mut out = Vec::new();
        let res = hevc_to_annex_b(
            &data,
            nls,
            Some(&vps_b[..]),
            Some(&sps_b[..]),
            Some(&pps_b[..]),
            &mut out,
        );
        if nls != 1 && nls != 2 && nls != 4 {
            prop_assert!(res.is_err());
        }
    }

    /// Property: Well-formed AVCC packets correctly transform to Annex-B with expected NAL count.
    #[test]
    fn prop_avcc_roundtrip_structure(
        nal_units in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..60), 1..8)
    ) {
        let mut avcc_data = Vec::new();
        for nal in &nal_units {
            let len = nal.len() as u32;
            avcc_data.extend_from_slice(&len.to_be_bytes());
            avcc_data.extend_from_slice(nal);
        }

        let mut annex_b = Vec::new();
        let res = avcc_to_annex_b(&avcc_data, 4, None, &mut annex_b);
        prop_assert!(res.is_ok());

        // Count start codes
        let mut start_codes = 0;
        let mut i = 0;
        while i + 4 <= annex_b.len() {
            if annex_b[i..i + 4] == ANNEX_B_START_CODE_4 {
                start_codes += 1;
                i += 4;
            } else {
                i += 1;
            }
        }
        prop_assert_eq!(start_codes, nal_units.len());
    }
}
