use poppy_hevc::{
    HevcNalHeader, HevcNalType, HevcNalUnit, NalError, PictureParameterSet, SequenceParameterSet,
    VideoParameterSet, split_annex_b_nalus,
};

#[test]
fn test_nal_header_parsing() {
    // VPS header: byte0 = (32 << 1) = 64 (0x40), byte1 = 1 (temporal id 1)
    let header_bytes = [0x40, 0x01];
    let header = HevcNalHeader::parse(&header_bytes).expect("parse VPS header");
    assert_eq!(header.nal_unit_type, HevcNalType::VpsNut);
    assert_eq!(header.nuh_layer_id, 0);
    assert_eq!(header.nuh_temporal_id_plus1, 1);
    assert!(header.nal_unit_type.is_parameter_set());

    // SPS header: byte0 = (33 << 1) = 66 (0x42), byte1 = 1
    let sps_header = HevcNalHeader::parse(&[0x42, 0x01]).unwrap();
    assert_eq!(sps_header.nal_unit_type, HevcNalType::SpsNut);

    // PPS header: byte0 = (34 << 1) = 68 (0x44), byte1 = 1
    let pps_header = HevcNalHeader::parse(&[0x44, 0x01]).unwrap();
    assert_eq!(pps_header.nal_unit_type, HevcNalType::PpsNut);

    // IDR header: byte0 = (19 << 1) = 38 (0x26), byte1 = 1
    let idr_header = HevcNalHeader::parse(&[0x26, 0x01]).unwrap();
    assert_eq!(idr_header.nal_unit_type, HevcNalType::IdrWRadl);
    assert!(idr_header.nal_unit_type.is_irap());
    assert!(idr_header.nal_unit_type.is_idr());
    assert!(idr_header.nal_unit_type.is_vcl());

    // Forbidden zero bit error
    assert_eq!(
        HevcNalHeader::parse(&[0x80, 0x01]),
        Err(NalError::ForbiddenZeroBitSet)
    );

    // Zero temporal ID error
    assert_eq!(
        HevcNalHeader::parse(&[0x40, 0x00]),
        Err(NalError::ZeroTemporalId)
    );
}

#[test]
fn test_annex_b_nalu_splitting() {
    let mut stream = Vec::new();
    // 4-byte start code + VPS NALU (2 header bytes + 4 payload bytes)
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0xAA, 0xBB, 0xCC, 0xDD]);
    // 3-byte start code + SPS NALU
    stream.extend_from_slice(&[0x00, 0x00, 0x01, 0x42, 0x01, 0x11, 0x22]);
    // 4-byte start code + IDR slice NALU
    stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x26, 0x01, 0x33, 0x44, 0x55]);

    let nalus = split_annex_b_nalus(&stream);
    assert_eq!(nalus.len(), 3);
    assert_eq!(nalus[0].header.nal_unit_type, HevcNalType::VpsNut);
    assert_eq!(nalus[1].header.nal_unit_type, HevcNalType::SpsNut);
    assert_eq!(nalus[2].header.nal_unit_type, HevcNalType::IdrWRadl);
}

#[test]
fn test_emulation_prevention_removal() {
    // 0x00 0x00 0x03 0x00 should become 0x00 0x00 0x00
    let payload = [0x42, 0x01, 0x00, 0x00, 0x03, 0x00, 0xFF];
    let nalu = HevcNalUnit::parse(&payload).unwrap();
    let rbsp = nalu.rbsp();
    assert_eq!(rbsp, vec![0x00, 0x00, 0x00, 0xFF]);
}

#[test]
fn test_real_hevc_sps_vps_pps() {
    // Standard test vectors for HEVC 1080p Main Profile
    // Real HEVC VPS RBSP
    let vps_data = [
        0x0c, 0x01, 0xff, 0xff, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0xb0, 0x00, 0x00, 0x03, 0x00,
        0x00, 0x03, 0x00, 0x7b, 0xac, 0x09,
    ];
    let vps_rbsp = poppy_hevc::remove_emulation_prevention(&vps_data);
    let vps = VideoParameterSet::parse(&vps_rbsp).expect("parse VPS");
    assert_eq!(vps.vps_video_parameter_set_id, 0);
    assert_eq!(vps.profile_tier_level.profile_idc, 1); // Main Profile

    // Real HEVC SPS RBSP (1920x1080 Main profile)
    let sps_data = [
        0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0xb0, 0x00, 0x00, 0x03, 0x00, 0x00,
        0x03, 0x00, 0x7b, 0xa0, 0x03, 0xc0, 0x80, 0x10, 0xe5, 0x96, 0x6e, 0x49, 0x32, 0xb8, 0x04,
        0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0x50, 0x20,
    ];
    let sps_rbsp = poppy_hevc::remove_emulation_prevention(&sps_data[2..]);
    let sps = SequenceParameterSet::parse(&sps_rbsp).expect("parse SPS");
    assert_eq!(sps.profile_tier_level.profile_idc, 1);
    assert_eq!(sps.pic_width_in_luma_samples, 1920);
    assert_eq!(sps.pic_height_in_luma_samples, 1080);
    assert_eq!(sps.cropped_width(), 1920);
    assert_eq!(sps.cropped_height(), 1080);
    assert_eq!(sps.bit_depth(), 8);
    assert!(!sps.is_10bit());

    // Real HEVC PPS RBSP
    let pps_data = [0xc0, 0xf3, 0xc0, 0xc2, 0x44, 0x04];
    let pps = PictureParameterSet::parse(&pps_data).expect("parse PPS");
    assert_eq!(pps.pps_pic_parameter_set_id, 0);
    assert_eq!(pps.pps_seq_parameter_set_id, 0);
}
