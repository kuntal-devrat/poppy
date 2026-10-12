use poppy_vp9::{
    DC_QLOOKUP, Vp9FrameHeader, Vp9FrameType, calculate_segment_qmul, parse_superframe_frames,
};

#[test]
fn test_tables_qlookup() {
    // 8-bit baseline tests
    assert_eq!(DC_QLOOKUP[0][0], 4);
    assert_eq!(DC_QLOOKUP[0][1], 8);

    let (ydc, yac, uvdc, uvac) = calculate_segment_qmul(8, 20, 0, 0, 0, None, false);
    assert!(ydc > 0 && yac > 0 && uvdc > 0 && uvac > 0);
}

#[test]
fn test_parse_superframe() {
    let dummy_packet = vec![0x10, 0x20, 0x30, 0x40];
    let frames = parse_superframe_frames(&dummy_packet).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0], &dummy_packet[..]);
}

#[test]
fn test_parse_synthetic_keyframe() {
    // Construct a minimal valid VP9 keyframe header bitstream
    let mut bits = Vec::new();

    // Helper closure to push bits MSB-first
    fn push_bits(vec: &mut Vec<u8>, bit_count: usize, value: u32) {
        for i in (0..bit_count).rev() {
            let bit = ((value >> i) & 1) as u8;
            vec.push(bit);
        }
    }

    push_bits(&mut bits, 2, 2); // frame_marker = 0b10
    push_bits(&mut bits, 1, 0); // profile_low = 0
    push_bits(&mut bits, 1, 0); // profile_high = 0 -> profile 0
    push_bits(&mut bits, 1, 0); // show_existing_frame = 0
    push_bits(&mut bits, 1, 0); // frame_type = KeyFrame
    push_bits(&mut bits, 1, 1); // show_frame = 1
    push_bits(&mut bits, 1, 0); // error_resilient_mode = 0

    // Sync code 0x498342
    push_bits(&mut bits, 8, 0x49);
    push_bits(&mut bits, 8, 0x83);
    push_bits(&mut bits, 8, 0x42);

    // Color config: profile 0 -> color_space (3 bits), color_range (1 bit)
    push_bits(&mut bits, 3, 1); // color_space = 1
    push_bits(&mut bits, 1, 0); // color_range = 0

    // Width - 1 (16 bits) = 1919 (1920)
    push_bits(&mut bits, 16, 1919);
    // Height - 1 (16 bits) = 1079 (1080)
    push_bits(&mut bits, 16, 1079);

    // Render size different = 0
    push_bits(&mut bits, 1, 0);

    // refresh_frame_context = 1, frame_parallel_decoding_mode = 1
    push_bits(&mut bits, 1, 1);
    push_bits(&mut bits, 1, 1);

    // frame_context_idx = 0
    push_bits(&mut bits, 2, 0);

    // Loop filter: filter_level = 10 (6 bits), sharpness = 2 (3 bits), delta_enabled = 0 (1 bit)
    push_bits(&mut bits, 6, 10);
    push_bits(&mut bits, 3, 2);
    push_bits(&mut bits, 1, 0);

    // Quantization: base_q_idx = 50 (8 bits), delta_y_dc=0, delta_uv_dc=0, delta_uv_ac=0
    push_bits(&mut bits, 8, 50);
    push_bits(&mut bits, 1, 0); // delta_y_dc present=0
    push_bits(&mut bits, 1, 0); // delta_uv_dc present=0
    push_bits(&mut bits, 1, 0); // delta_uv_ac present=0

    // Segmentation: enabled = 0
    push_bits(&mut bits, 1, 0);

    // Tile cols log2: sb64_cols for 1920 is ( (1920+7)/8 + 7 )/8 = (240+7)/8 = 30.
    // min_log2 = 0. max_log2 = 4.
    // tile_cols_log2 loop: read 0 to keep 0
    push_bits(&mut bits, 1, 0);
    // tile_rows_log2: 0
    push_bits(&mut bits, 1, 0);

    // first_partition_size = 150 (16 bits)
    push_bits(&mut bits, 16, 150);

    // Convert bits vector into bytes
    let mut bytes = Vec::new();
    let mut current_byte = 0u8;
    let mut bit_in_byte = 0;
    for bit in bits {
        current_byte = (current_byte << 1) | bit;
        bit_in_byte += 1;
        if bit_in_byte == 8 {
            bytes.push(current_byte);
            current_byte = 0;
            bit_in_byte = 0;
        }
    }
    if bit_in_byte > 0 {
        current_byte <<= 8 - bit_in_byte;
        bytes.push(current_byte);
    }

    // Append some dummy payload bytes for partition data
    bytes.resize(bytes.len() + 200, 0xAA);

    let header = Vp9FrameHeader::parse(&bytes, None).expect("Should parse synthetic keyframe");

    assert_eq!(header.frame_type, Vp9FrameType::KeyFrame);
    assert_eq!(header.profile, 0);
    assert_eq!(header.width, 1920);
    assert_eq!(header.height, 1080);
    assert!(header.show_frame);
    assert_eq!(header.refresh_frame_flags, 0xFF);
    assert_eq!(header.loop_filter.filter_level, 10);
    assert_eq!(header.loop_filter.sharpness_level, 2);
    assert_eq!(header.quant.base_q_idx, 50);
    assert_eq!(header.first_partition_size, 150);
    assert!(header.uncompressed_header_size > 0);
}
