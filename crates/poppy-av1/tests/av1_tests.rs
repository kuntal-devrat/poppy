use poppy_av1::{ObuType, SequenceHeader, split_obus};

#[test]
fn test_parse_synthetic_sequence_header() {
    // Construct a minimal valid SequenceHeader OBU:
    // OBU Header: type=1 (SequenceHeader, 0b00001000 = 0x08), has_size=1 (bit 1) -> 0x0A
    // Payload size: LEB128 of payload len
    // Payload:
    // seq_profile = 0 (3 bits: 000)
    // still_picture = 0 (1 bit: 0)
    // reduced_still_picture_header = 1 (1 bit: 1)
    // seq_level_idx = 0 (5 bits: 00000)
    // frame_width_bits_minus_1 = 3 (4 bits: 0011 -> 4 bits for width)
    // frame_height_bits_minus_1 = 3 (4 bits: 0011 -> 4 bits for height)
    // max_frame_width_minus_1 = 15 (4 bits: 1111 -> 16px)
    // max_frame_height_minus_1 = 15 (4 bits: 1111 -> 16px)
    // use_128x128_superblock = 0 (1 bit: 0)
    // enable_filter_intra = 0 (1 bit: 0)
    // enable_intra_edge_filter = 0 (1 bit: 0)
    // enable_superres = 0 (1 bit: 0)
    // enable_cdef = 0 (1 bit: 0)
    // enable_restoration = 0 (1 bit: 0)
    // high_bitdepth = 0 (1 bit: 0)
    // mono_chrome = 0 (1 bit: 0)
    // color_description_present_flag = 0 (1 bit: 0)
    // color_range = 0 (1 bit: 0)
    // separate_uv_delta_q = 0 (1 bit: 0)
    // film_grain_params_present = 0 (1 bit: 0)
    let payload = [
        0b00001000, 0b00001100, 0b11111111, 0b11000000, 0b00000000, 0b00000000,
    ];

    let header = SequenceHeader::parse(&payload).unwrap();
    assert_eq!(header.seq_profile, 0);
    assert!(header.reduced_still_picture_header);
    assert_eq!(header.bit_depth(), 8);
}

#[test]
fn test_split_obus() {
    // Temporal delimiter OBU (type 2, has_size_field=1, size=0)
    // 0x12, 0x00
    let data = [0x12, 0x00];
    let obus = split_obus(&data).unwrap();
    assert_eq!(obus.len(), 1);
    assert_eq!(obus[0].header.obu_type, ObuType::TemporalDelimiter);
    assert_eq!(obus[0].payload.len(), 0);
}
