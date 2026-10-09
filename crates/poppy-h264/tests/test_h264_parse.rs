use poppy_h264::{
    find_annex_b_nals, NalHeader, NalUnitType, Pps, SliceHeader, SliceType, Sps,
};

#[test]
fn test_h264_full_stream_parsing() {
    // Canonical H.264 stream:
    // SPS + PPS + IDR Slice with emulation prevention bytes
    let stream: &[u8] = &[
        // SPS: High profile, 1920x1080
        0x00, 0x00, 0x00, 0x01,
        0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x78,
        0x02, 0x27, 0xE5, 0xC0, 0x44, 0x00, 0x00, 0x03,
        0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x36,
        0x82, 0x21, 0x1B,
        // PPS: CABAC, pic_id 0
        0x00, 0x00, 0x00, 0x01,
        0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0,
        // IDR Slice (NAL type 5 = 0x65)
        // first_mb = 0 (1 bit '1'), slice_type = 7 (I-slice, ue 7 = 0001000), pps_id = 0 (1 bit '1'), frame_num = 0 (4 bits '0000'), idr_pic_id = 0 (1 bit '1'), poc_lsb = 0 (6 bits '000000')...
        0x00, 0x00, 0x00, 0x01,
        0x65, 0x88, 0x84, 0x00, 0x10, 0xFF,
    ];

    let nals = find_annex_b_nals(stream);
    assert_eq!(nals.len(), 3);

    // 1. Verify SPS
    let sps_nal = nals[0];
    let sps_h = NalHeader::parse(sps_nal[0]);
    assert_eq!(sps_h.unit_type, NalUnitType::Sps);
    let sps = Sps::parse(sps_nal).expect("Failed to parse SPS");
    assert_eq!(sps.profile_idc, 100);
    assert_eq!(sps.level_idc, 31);
    assert_eq!(sps.coded_width(), 1920);
    assert_eq!(sps.display_height(), 1080);

    // 2. Verify PPS
    let pps_nal = nals[1];
    let pps_h = NalHeader::parse(pps_nal[0]);
    assert_eq!(pps_h.unit_type, NalUnitType::Pps);
    let pps = Pps::parse(pps_nal).expect("Failed to parse PPS");
    assert_eq!(pps.pic_parameter_set_id, 0);
    assert!(pps.entropy_coding_mode_flag); // CABAC

    // 3. Verify IDR Slice Header
    let idr_nal = nals[2];
    let idr_h = NalHeader::parse(idr_nal[0]);
    assert_eq!(idr_h.unit_type, NalUnitType::SliceIdr);
    assert!(idr_h.unit_type.is_idr());

    // Parse slice header from payload (after 1-byte NAL header)
    let slice = SliceHeader::parse(idr_h.unit_type, &idr_nal[1..], &sps, &pps)
        .expect("Failed to parse IDR Slice Header");

    assert_eq!(slice.first_mb_in_slice, 0);
    assert_eq!(slice.slice_type, SliceType::I);
    assert!(slice.slice_type.is_intra());
    assert_eq!(slice.frame_num, 0);
    assert_eq!(slice.idr_pic_id, Some(0));
    assert!(slice.slice_data_bit_offset > 0);
}
