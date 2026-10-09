use poppy_hevc::bitstream::{BitReader, remove_emulation_prevention};
use poppy_hevc::nal::{HevcNalHeader, split_annex_b_nalus};
use proptest::prelude::*;

proptest! {
    /// Property: remove_emulation_prevention never panics on arbitrary data.
    #[test]
    fn prop_hevc_emulation_prevention_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let rbsp = remove_emulation_prevention(&data);
        prop_assert!(rbsp.len() <= data.len());
    }

    /// Property: BitReader operations never panic on arbitrary byte inputs.
    #[test]
    fn prop_hevc_bitreader_never_panics(data in proptest::collection::vec(any::<u8>(), 0..128)) {
        let mut reader = BitReader::new(&data);
        while reader.bits_remaining() > 0 {
            if reader.read_bit().is_err() {
                break;
            }
            let _ = reader.read_ue();
            let _ = reader.read_se();
        }
    }

    /// Property: HevcNalHeader::parse never panics for any 2-byte slice.
    #[test]
    fn prop_hevc_header_parse_never_panics(b0 in any::<u8>(), b1 in any::<u8>()) {
        let bytes = [b0, b1];
        let res = HevcNalHeader::parse(&bytes);
        if (b0 & 0x80) != 0 {
            prop_assert!(res.is_err());
        } else if (b1 & 0x07) == 0 {
            prop_assert!(res.is_err());
        } else {
            prop_assert!(res.is_ok());
            let hdr = res.unwrap();
            prop_assert_eq!(hdr.nuh_temporal_id_plus1, b1 & 0x07);
        }
    }

    /// Property: split_annex_b_nalus never panics on arbitrary bitstream.
    #[test]
    fn prop_split_annex_b_nalus_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let nals = split_annex_b_nalus(&data);
        for nal in nals {
            prop_assert!(!nal.data.is_empty());
        }
    }
}
