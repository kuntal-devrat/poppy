use poppy_av1::leb128::read_leb128;
use poppy_av1::obu::{ObuHeader, split_obus};
use proptest::prelude::*;

proptest! {
    /// Property: read_leb128 never panics on arbitrary byte inputs.
    #[test]
    fn prop_leb128_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok((_val, bytes_read)) = read_leb128(&data) {
            prop_assert!((1..=8).contains(&bytes_read));
            prop_assert!(bytes_read <= data.len());
        }
    }

    /// Property: LEB128 encode / decode roundtrip for arbitrary u64 values.
    #[test]
    fn prop_leb128_roundtrip(val in any::<u64>()) {
        let mut encoded = Vec::new();
        let mut n = val;
        loop {
            let mut byte = (n & 0x7F) as u8;
            n >>= 7;
            if n != 0 {
                byte |= 0x80;
            }
            encoded.push(byte);
            if n == 0 {
                break;
            }
        }

        // Only values that fit in <= 8 LEB128 bytes are supported by the AV1 spec parser
        if encoded.len() <= 8 {
            let (decoded, bytes_read) = read_leb128(&encoded).expect("decode valid leb128");
            prop_assert_eq!(decoded, val);
            prop_assert_eq!(bytes_read, encoded.len());
        }
    }

    /// Property: ObuHeader::parse never panics on arbitrary byte sequences.
    #[test]
    fn prop_obu_header_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64)) {
        if let Ok(hdr) = ObuHeader::parse(&data) {
            prop_assert!(hdr.header_len <= data.len());
            prop_assert!(hdr.temporal_id <= 7);
            prop_assert!(hdr.spatial_id <= 3);
        }
    }

    /// Property: split_obus never panics on arbitrary byte sequences.
    #[test]
    fn prop_split_obus_never_panics(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        let _ = split_obus(&data);
    }
}
