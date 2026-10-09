use poppy_h264::bitstream::{BitReader, remove_emulation_prevention};
use poppy_h264::nal::{NalHeader, NalUnit, find_annex_b_nals};
use proptest::prelude::*;

proptest! {
    /// Property: remove_emulation_prevention never panics and never increases output length.
    #[test]
    fn prop_emulation_prevention_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let rbsp = remove_emulation_prevention(&data);
        prop_assert!(rbsp.len() <= data.len());
    }

    /// Property: BitReader operations never panic on arbitrary byte inputs.
    #[test]
    fn prop_bitreader_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..128)) {
        let mut reader = BitReader::new(&data);
        while reader.bits_remaining() > 0 {
            // Read either 1 bit or ue
            if reader.read_bit().is_err() {
                break;
            }
            let _ = reader.read_ue();
            let _ = reader.read_se();
        }
    }

    /// Property: find_annex_b_nals never panics on arbitrary data and slices returned are valid.
    #[test]
    fn prop_find_annex_b_nals_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let nals = find_annex_b_nals(&data);
        for nal in nals {
            prop_assert!(!nal.is_empty());
            if let Some(unit) = NalUnit::parse(nal) {
                prop_assert_eq!(unit.payload.len() + 1, nal.len());
            }
        }
    }

    /// Property: NalHeader parsing is consistent and never panics for any byte.
    #[test]
    fn prop_nal_header_parse_all_bytes(byte in any::<u8>()) {
        let header = NalHeader::parse(byte);
        prop_assert_eq!(header.forbidden_zero_bit, (byte & 0x80) != 0);
        prop_assert_eq!(header.nal_ref_idc, (byte >> 5) & 0x03);
    }

    /// Property: Exp-Golomb ue(v) encoder / decoder round-trip for values 0..100_000.
    #[test]
    fn prop_ue_roundtrip(val in 0u32..100_000) {
        // Encode ue(v): code_num has k leading zeros followed by '1' and (code_num + 1 - 2^k) in k bits
        let code_num = val;
        let val_plus_1 = code_num + 1;
        let k = 31 - val_plus_1.leading_zeros() as usize; // floor(log2(val + 1))
        let total_bits = 2 * k + 1;

        let mut bits = Vec::new();
        // k zeros
        bits.resize(k, false);
        // '1' bit
        bits.push(true);
        // k suffix bits
        let suffix = val_plus_1 - (1 << k);
        for bit_idx in (0..k).rev() {
            bits.push(((suffix >> bit_idx) & 1) == 1);
        }

        // Pack bits into bytes
        let mut bytes = Vec::new();
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (idx, &b) in chunk.iter().enumerate() {
                if b {
                    byte |= 1 << (7 - idx);
                }
            }
            bytes.push(byte);
        }

        let mut reader = BitReader::new(&bytes);
        let decoded = reader.read_ue().expect("decode ue");
        prop_assert_eq!(decoded, val);
        prop_assert_eq!(reader.bits_read(), total_bits);
    }
}
