use poppy_vp9::{Vp9BitReader, parse_superframe_frames};
use proptest::prelude::*;

proptest! {
    #[test]
    fn prop_vp9_bitreader_never_panics(data in prop::collection::vec(any::<u8>(), 0..256), n in 0usize..40) {
        let mut r = Vp9BitReader::new(&data);
        let _ = r.read_bits(n);
    }

    #[test]
    fn prop_superframe_never_panics(data in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = parse_superframe_frames(&data);
    }

    #[test]
    fn prop_vp9_header_never_panics(data in prop::collection::vec(any::<u8>(), 0..512)) {
        let dims = [(320u32, 240u32); 8];
        let _ = poppy_vp9::Vp9FrameHeader::parse(&data, None);
        let _ = poppy_vp9::Vp9FrameHeader::parse(&data, Some(&dims));
    }
}
