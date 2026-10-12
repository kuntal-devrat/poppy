use bytes::Bytes;
use poppy_core::CodecId;
use poppy_core::codec::CodecConfig;
use poppy_demux::{Demuxer, Mp4Demuxer};

/// Helper to construct a box with 4-byte size and 4-byte FourCC.
fn make_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(payload);
    out
}

/// Helper to construct a FullBox (version + flags + payload).
fn make_fullbox(fourcc: &[u8; 4], version: u8, flags: [u8; 3], payload: &[u8]) -> Vec<u8> {
    let mut full_payload = Vec::with_capacity(4 + payload.len());
    full_payload.push(version);
    full_payload.extend_from_slice(&flags);
    full_payload.extend_from_slice(payload);
    make_box(fourcc, &full_payload)
}

#[test]
fn test_mp4_demux_end_to_end() {
    // Construct a synthetic compliant MP4 in memory:
    // ftyp, mdat, moov

    // 1. ftyp box
    let ftyp_payload = b"isom\0\0\x02\0isomiso2mp41";
    let ftyp = make_box(b"ftyp", ftyp_payload);

    // 2. Sample data to put in mdat
    // Sample 0: NAL length (4 bytes) = 4, followed by 4 bytes of NAL
    // Sample 1: NAL length (4 bytes) = 6, followed by 6 bytes of NAL
    let sample0 = [0, 0, 0, 4, 0x65, 1, 2, 3]; // Keyframe (IDR slice)
    let sample1 = [0, 0, 0, 6, 0x41, 4, 5, 6, 7, 8]; // Non-keyframe (P-slice)
    let mut mdat_payload = Vec::new();
    mdat_payload.extend_from_slice(&sample0);
    mdat_payload.extend_from_slice(&sample1);

    let mdat = make_box(b"mdat", &mdat_payload);

    // Byte offset of mdat body in the file:
    // ftyp size + 8 (mdat box header)
    let mdat_body_offset = (ftyp.len() + 8) as u64;

    // 3. Construct moov tree:
    // avcC box
    let avcc_payload = vec![
        1,    // configurationVersion = 1
        0x64, // AVCProfileIndication (High = 100 = 0x64)
        0x00, // profile_compatibility
        0x1F, // AVCLevelIndication (3.1 = 31 = 0x1F)
        0xFF, // lengthSizeMinusOne | 0xFC = 3 | 0xFC = 0xFF -> 4 bytes length
        0xE1, // numOfSequenceParameterSets | 0xE0 = 1 | 0xE0 = 0xE1
        0x00, 4, // sps length = 4
        0x67, 0x64, 0x00, 0x1F, // sps bytes
        1,    // numOfPictureParameterSets = 1
        0x00, 3, // pps length = 3
        0x68, 0xEB, 0xE3, // pps bytes
    ];
    let avcc_box = make_box(b"avcC", &avcc_payload);

    // avc1 box (VisualSampleEntry)
    // 78 bytes total after size & fourcc:
    // 6 reserved + 2 data_ref_idx + 16 pre_defined/reserved + 2 width + 2 height + 8 res + 4 res + 2 frame_count + 32 compressor + 2 depth + 2 pre_defined
    let mut avc1_payload = vec![0u8; 78];
    // width at offset 24..26 (1920)
    avc1_payload[24..26].copy_from_slice(&1920u16.to_be_bytes());
    // height at offset 26..28 (1080)
    avc1_payload[26..28].copy_from_slice(&1080u16.to_be_bytes());
    // horiz / vert resolution: 72 dpi (0x00480000)
    avc1_payload[28..32].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[32..36].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[40..42].copy_from_slice(&1u16.to_be_bytes()); // frame_count = 1
    avc1_payload[74..76].copy_from_slice(&24u16.to_be_bytes()); // depth = 24
    avc1_payload.extend_from_slice(&avcc_box);
    let avc1_box = make_box(b"avc1", &avc1_payload);

    // stsd box: entry_count = 1
    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stsd_payload.extend_from_slice(&avc1_box);
    let stsd_box = make_fullbox(b"stsd", 0, [0, 0, 0], &stsd_payload);

    // stsz box: default_sample_size = 0, count = 2, sizes = [sample0.len(), sample1.len()]
    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes()); // default size = 0
    stsz_payload.extend_from_slice(&2u32.to_be_bytes()); // count = 2
    stsz_payload.extend_from_slice(&(sample0.len() as u32).to_be_bytes());
    stsz_payload.extend_from_slice(&(sample1.len() as u32).to_be_bytes());
    let stsz_box = make_fullbox(b"stsz", 0, [0, 0, 0], &stsz_payload);

    // stsc box: 1 entry: first_chunk = 1, samples_per_chunk = 2, desc_idx = 1
    let mut stsc_payload = Vec::new();
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // first_chunk = 1
    stsc_payload.extend_from_slice(&2u32.to_be_bytes()); // samples_per_chunk = 2
    stsc_payload.extend_from_slice(&1u32.to_be_bytes()); // desc_idx = 1
    let stsc_box = make_fullbox(b"stsc", 0, [0, 0, 0], &stsc_payload);

    // stco box: 1 chunk at mdat_body_offset
    let mut stco_payload = Vec::new();
    stco_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stco_payload.extend_from_slice(&(mdat_body_offset as u32).to_be_bytes());
    let stco_box = make_fullbox(b"stco", 0, [0, 0, 0], &stco_payload);

    // stts box: 2 samples each duration 1000
    let mut stts_payload = Vec::new();
    stts_payload.extend_from_slice(&1u32.to_be_bytes()); // 1 entry
    stts_payload.extend_from_slice(&2u32.to_be_bytes()); // sample_count = 2
    stts_payload.extend_from_slice(&1000u32.to_be_bytes()); // delta = 1000
    let stts_box = make_fullbox(b"stts", 0, [0, 0, 0], &stts_payload);

    // stss box: sample 1 (1-based) is sync sample
    let mut stss_payload = Vec::new();
    stss_payload.extend_from_slice(&1u32.to_be_bytes()); // 1 entry
    stss_payload.extend_from_slice(&1u32.to_be_bytes()); // sample_num = 1
    let stss_box = make_fullbox(b"stss", 0, [0, 0, 0], &stss_payload);

    // stbl box
    let mut stbl_payload = Vec::new();
    stbl_payload.extend_from_slice(&stsd_box);
    stbl_payload.extend_from_slice(&stts_box);
    stbl_payload.extend_from_slice(&stss_box);
    stbl_payload.extend_from_slice(&stsc_box);
    stbl_payload.extend_from_slice(&stsz_box);
    stbl_payload.extend_from_slice(&stco_box);
    let stbl_box = make_box(b"stbl", &stbl_payload);

    // minf box
    let minf_box = make_box(b"minf", &stbl_box);

    // hdlr box: handler_type = "vide"
    let mut hdlr_payload = vec![0u8; 16];
    hdlr_payload[4..8].copy_from_slice(b"vide");
    let hdlr_box = make_fullbox(b"hdlr", 0, [0, 0, 0], &hdlr_payload);

    // mdhd box: timescale = 30000, duration = 2000
    let mut mdhd_payload = vec![0u8; 20];
    mdhd_payload[8..12].copy_from_slice(&30000u32.to_be_bytes()); // timescale = 30000
    mdhd_payload[12..16].copy_from_slice(&2000u32.to_be_bytes()); // duration = 2000
    let mdhd_box = make_fullbox(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    // mdia box
    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd_box);
    mdia_payload.extend_from_slice(&hdlr_box);
    mdia_payload.extend_from_slice(&minf_box);
    let mdia_box = make_box(b"mdia", &mdia_payload);

    // tkhd box: track_id = 1, duration = 2000, width = 1920 << 16, height = 1080 << 16
    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&1u32.to_be_bytes()); // track_id = 1
    tkhd_payload[16..20].copy_from_slice(&2000u32.to_be_bytes()); // duration = 2000
    tkhd_payload[72..76].copy_from_slice(&(1920u32 << 16).to_be_bytes()); // width
    tkhd_payload[76..80].copy_from_slice(&(1080u32 << 16).to_be_bytes()); // height
    let tkhd_box = make_fullbox(b"tkhd", 0, [0, 0, 0], &tkhd_payload);

    // trak box
    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd_box);
    trak_payload.extend_from_slice(&mdia_box);
    let trak_box = make_box(b"trak", &trak_payload);

    // moov box
    let moov_box = make_box(b"moov", &trak_box);

    // Assemble complete MP4 byte buffer
    let mut mp4_file = Vec::new();
    mp4_file.extend_from_slice(&ftyp);
    mp4_file.extend_from_slice(&mdat);
    mp4_file.extend_from_slice(&moov_box);

    let _ = std::fs::write("../../tests/samples/test_clip.mp4", &mp4_file);
    let _ = std::fs::write("tests/samples/test_clip.mp4", &mp4_file);

    // Test both from_bytes and open(file)
    let bytes = Bytes::from(mp4_file);
    let mut demuxer = Mp4Demuxer::from_bytes(bytes).expect("Failed to initialize Mp4Demuxer");

    // Check streams
    let streams = demuxer.streams();
    assert_eq!(streams.len(), 1);
    let s0 = &streams[0];
    assert_eq!(s0.codec, CodecId::H264);
    assert_eq!(s0.time_base.num, 1);
    assert_eq!(s0.time_base.den, 30000);
    assert_eq!(s0.duration_ticks, Some(2000));

    // Verify AvcCRecord
    match &s0.config {
        CodecConfig::H264(avcc) => {
            assert_eq!(avcc.profile_idc, 100);
            assert_eq!(avcc.level_idc, 31);
            assert_eq!(avcc.nal_length_size, 4);
            assert_eq!(avcc.sps_list.len(), 1);
            assert_eq!(avcc.pps_list.len(), 1);
            assert_eq!(&avcc.sps_list[0][..], &[0x67, 0x64, 0x00, 0x1F]);
            assert_eq!(&avcc.pps_list[0][..], &[0x68, 0xEB, 0xE3]);
        }
        _ => panic!("Expected H.264 CodecConfig"),
    }

    // Read Packet 0
    let pkt0 = demuxer.read_packet().unwrap().expect("Expected packet 0");
    assert_eq!(pkt0.stream_index, 0);
    assert_eq!(pkt0.dts, 0);
    assert_eq!(pkt0.duration, 1000);
    assert!(pkt0.is_keyframe);
    assert_eq!(&pkt0.data[..], &sample0[..]);

    // Read Packet 1
    let pkt1 = demuxer.read_packet().unwrap().expect("Expected packet 1");
    assert_eq!(pkt1.stream_index, 0);
    assert_eq!(pkt1.dts, 1000);
    assert_eq!(pkt1.duration, 1000);
    assert!(!pkt1.is_keyframe);
    assert_eq!(&pkt1.data[..], &sample1[..]);

    // End of stream
    let pkt_end = demuxer.read_packet().unwrap();
    assert!(pkt_end.is_none());

    // Seeking test: seek back to 0
    demuxer.seek_to_keyframe(0, 0).unwrap();
    let pkt_rewind = demuxer.read_packet().unwrap().expect("Expected rewind pkt");
    assert_eq!(pkt_rewind.dts, 0);
    assert!(pkt_rewind.is_keyframe);
}

#[test]
fn test_mp4_edit_list_timestamp_offset() {
    // 1. ftyp box
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");

    // 2. mdat
    let sample0 = [0, 0, 0, 4, 0x65, 1, 2, 3];
    let sample1 = [0, 0, 0, 6, 0x41, 4, 5, 6, 7, 8];
    let mut mdat_payload = Vec::new();
    mdat_payload.extend_from_slice(&sample0);
    mdat_payload.extend_from_slice(&sample1);
    let mdat = make_box(b"mdat", &mdat_payload);
    let mdat_body_offset = (ftyp.len() + 8) as u64;

    // 3. avcC box
    let avcc_payload = vec![
        1, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, 4,
        0x67, 0x64, 0x00, 0x1F, 1, 0x00, 3, 0x68, 0xEB, 0xE3,
    ];
    let avcc_box = make_box(b"avcC", &avcc_payload);

    // avc1 box
    let mut avc1_payload = vec![0u8; 78];
    avc1_payload[24..26].copy_from_slice(&1920u16.to_be_bytes());
    avc1_payload[26..28].copy_from_slice(&1080u16.to_be_bytes());
    avc1_payload.extend_from_slice(&avcc_box);
    let avc1_box = make_box(b"avc1", &avc1_payload);

    // stsd box
    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&avc1_box);
    let stsd_box = make_fullbox(b"stsd", 0, [0, 0, 0], &stsd_payload);

    // stts box: 2 samples, each 1000 duration
    let mut stts_payload = Vec::new();
    stts_payload.extend_from_slice(&1u32.to_be_bytes());
    stts_payload.extend_from_slice(&2u32.to_be_bytes());
    stts_payload.extend_from_slice(&1000u32.to_be_bytes());
    let stts_box = make_fullbox(b"stts", 0, [0, 0, 0], &stts_payload);

    // stss box: keyframe at sample 1 (1-indexed)
    let mut stss_payload = Vec::new();
    stss_payload.extend_from_slice(&1u32.to_be_bytes());
    stss_payload.extend_from_slice(&1u32.to_be_bytes());
    let stss_box = make_fullbox(b"stss", 0, [0, 0, 0], &stss_payload);

    // stsc box
    let mut stsc_payload = Vec::new();
    stsc_payload.extend_from_slice(&1u32.to_be_bytes());
    stsc_payload.extend_from_slice(&1u32.to_be_bytes());
    stsc_payload.extend_from_slice(&2u32.to_be_bytes());
    stsc_payload.extend_from_slice(&1u32.to_be_bytes());
    let stsc_box = make_fullbox(b"stsc", 0, [0, 0, 0], &stsc_payload);

    // stsz box
    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes());
    stsz_payload.extend_from_slice(&2u32.to_be_bytes());
    stsz_payload.extend_from_slice(&(sample0.len() as u32).to_be_bytes());
    stsz_payload.extend_from_slice(&(sample1.len() as u32).to_be_bytes());
    let stsz_box = make_fullbox(b"stsz", 0, [0, 0, 0], &stsz_payload);

    // stco box
    let mut stco_payload = Vec::new();
    stco_payload.extend_from_slice(&1u32.to_be_bytes());
    stco_payload.extend_from_slice(&(mdat_body_offset as u32).to_be_bytes());
    let stco_box = make_fullbox(b"stco", 0, [0, 0, 0], &stco_payload);

    // stbl box
    let mut stbl_payload = Vec::new();
    stbl_payload.extend_from_slice(&stsd_box);
    stbl_payload.extend_from_slice(&stts_box);
    stbl_payload.extend_from_slice(&stss_box);
    stbl_payload.extend_from_slice(&stsc_box);
    stbl_payload.extend_from_slice(&stsz_box);
    stbl_payload.extend_from_slice(&stco_box);
    let stbl_box = make_box(b"stbl", &stbl_payload);

    let minf_box = make_box(b"minf", &stbl_box);

    let mut hdlr_payload = vec![0u8; 16];
    hdlr_payload[4..8].copy_from_slice(b"vide");
    let hdlr_box = make_fullbox(b"hdlr", 0, [0, 0, 0], &hdlr_payload);

    let mut mdhd_payload = vec![0u8; 20];
    mdhd_payload[8..12].copy_from_slice(&30000u32.to_be_bytes());
    mdhd_payload[12..16].copy_from_slice(&2000u32.to_be_bytes());
    let mdhd_box = make_fullbox(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd_box);
    mdia_payload.extend_from_slice(&hdlr_box);
    mdia_payload.extend_from_slice(&minf_box);
    let mdia_box = make_box(b"mdia", &mdia_payload);

    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&1u32.to_be_bytes());
    tkhd_payload[16..20].copy_from_slice(&2000u32.to_be_bytes());
    tkhd_payload[72..76].copy_from_slice(&(1920u32 << 16).to_be_bytes());
    tkhd_payload[76..80].copy_from_slice(&(1080u32 << 16).to_be_bytes());
    let tkhd_box = make_fullbox(b"tkhd", 0, [0, 0, 0], &tkhd_payload);

    // edts + elst box: media_time = 1000 (shifts timestamps by -1000)
    let mut elst_payload = Vec::new();
    elst_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    elst_payload.extend_from_slice(&2000u32.to_be_bytes()); // segment_duration = 2000
    elst_payload.extend_from_slice(&1000i32.to_be_bytes()); // media_time = 1000
    elst_payload.extend_from_slice(&0x00010000u32.to_be_bytes()); // media_rate = 1.0
    let elst_box = make_fullbox(b"elst", 0, [0, 0, 0], &elst_payload);
    let edts_box = make_box(b"edts", &elst_box);

    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd_box);
    trak_payload.extend_from_slice(&edts_box);
    trak_payload.extend_from_slice(&mdia_box);
    let trak_box = make_box(b"trak", &trak_payload);

    let moov_box = make_box(b"moov", &trak_box);

    let mut mp4_file = Vec::new();
    mp4_file.extend_from_slice(&ftyp);
    mp4_file.extend_from_slice(&mdat);
    mp4_file.extend_from_slice(&moov_box);

    let mut demuxer = Mp4Demuxer::from_bytes(Bytes::from(mp4_file)).expect("open MP4 with edts");

    let pkt0 = demuxer.read_packet().unwrap().expect("packet 0");
    // Sample 0 original dts was 0; with media_time = 1000, shifted to -1000
    assert_eq!(pkt0.dts, -1000);

    let pkt1 = demuxer.read_packet().unwrap().expect("packet 1");
    // Sample 1 original dts was 1000; with media_time = 1000, shifted to 0
    assert_eq!(pkt1.dts, 0);
}
