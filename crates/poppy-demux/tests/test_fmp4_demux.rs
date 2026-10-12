use bytes::Bytes;
use poppy_core::CodecId;
use poppy_demux::{Demuxer, Mp4Demuxer};

fn make_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(size as usize);
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(payload);
    out
}

fn make_fullbox(fourcc: &[u8; 4], version: u8, flags: [u8; 3], payload: &[u8]) -> Vec<u8> {
    let mut full_payload = Vec::with_capacity(4 + payload.len());
    full_payload.push(version);
    full_payload.extend_from_slice(&flags);
    full_payload.extend_from_slice(payload);
    make_box(fourcc, &full_payload)
}

/// Constructs a synthetic multi-fragment fMP4 stream with moov/mvex, sidx, two moof/mdat fragments, and mfra.
fn build_synthetic_fmp4() -> Vec<u8> {
    let mut fmp4 = Vec::new();

    // 1. ftyp
    let ftyp = make_box(b"ftyp", b"iso6\0\0\x02\0iso6dashmp41");
    fmp4.extend_from_slice(&ftyp);

    // 2. moov
    // avcC box
    let avcc_payload = vec![
        1, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, 4, 0x67, 0x64, 0x00, 0x1F, 1, 0x00, 3, 0x68, 0xEB,
        0xE3,
    ];
    let avcc_box = make_box(b"avcC", &avcc_payload);

    let mut avc1_payload = vec![0u8; 78];
    avc1_payload[24..26].copy_from_slice(&1920u16.to_be_bytes());
    avc1_payload[26..28].copy_from_slice(&1080u16.to_be_bytes());
    avc1_payload[28..32].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[32..36].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[40..42].copy_from_slice(&1u16.to_be_bytes());
    avc1_payload[74..76].copy_from_slice(&24u16.to_be_bytes());
    avc1_payload.extend_from_slice(&avcc_box);
    let avc1_box = make_box(b"avc1", &avc1_payload);

    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&avc1_box);
    let stsd_box = make_fullbox(b"stsd", 0, [0, 0, 0], &stsd_payload);

    // In fMP4, stts, stsc, stsz, stco in moov are empty
    let stts_box = make_fullbox(b"stts", 0, [0, 0, 0], &0u32.to_be_bytes());
    let stsc_box = make_fullbox(b"stsc", 0, [0, 0, 0], &0u32.to_be_bytes());
    let stsz_box = make_fullbox(b"stsz", 0, [0, 0, 0], &[0, 0, 0, 0, 0, 0, 0, 0]);
    let stco_box = make_fullbox(b"stco", 0, [0, 0, 0], &0u32.to_be_bytes());

    let mut stbl_payload = Vec::new();
    stbl_payload.extend_from_slice(&stsd_box);
    stbl_payload.extend_from_slice(&stts_box);
    stbl_payload.extend_from_slice(&stsc_box);
    stbl_payload.extend_from_slice(&stsz_box);
    stbl_payload.extend_from_slice(&stco_box);
    let stbl_box = make_box(b"stbl", &stbl_payload);
    let minf_box = make_box(b"minf", &stbl_box);

    let mut hdlr_payload = vec![0u8; 16];
    hdlr_payload[4..8].copy_from_slice(b"vide");
    let hdlr_box = make_fullbox(b"hdlr", 0, [0, 0, 0], &hdlr_payload);

    let mut mdhd_payload = vec![0u8; 20];
    mdhd_payload[8..12].copy_from_slice(&30000u32.to_be_bytes()); // timescale = 30000
    mdhd_payload[12..16].copy_from_slice(&0u32.to_be_bytes()); // duration = 0 (fragmented)
    let mdhd_box = make_fullbox(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd_box);
    mdia_payload.extend_from_slice(&hdlr_box);
    mdia_payload.extend_from_slice(&minf_box);
    let mdia_box = make_box(b"mdia", &mdia_payload);

    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&1u32.to_be_bytes()); // track_id = 1
    tkhd_payload[72..76].copy_from_slice(&(1920u32 << 16).to_be_bytes());
    tkhd_payload[76..80].copy_from_slice(&(1080u32 << 16).to_be_bytes());
    let tkhd_box = make_fullbox(b"tkhd", 0, [0, 0, 0], &tkhd_payload);

    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd_box);
    trak_payload.extend_from_slice(&mdia_box);
    let trak_box = make_box(b"trak", &trak_payload);

    // mvex box with trex
    let mut trex_payload = vec![0u8; 20];
    trex_payload[0..4].copy_from_slice(&1u32.to_be_bytes()); // track_id = 1
    trex_payload[4..8].copy_from_slice(&1u32.to_be_bytes()); // default_sample_description_index = 1
    trex_payload[8..12].copy_from_slice(&1001u32.to_be_bytes()); // default_sample_duration = 1001
    trex_payload[12..16].copy_from_slice(&0u32.to_be_bytes()); // default_sample_size = 0
    trex_payload[16..20].copy_from_slice(&0x01010000u32.to_be_bytes()); // non-sync by default
    let trex_box = make_fullbox(b"trex", 0, [0, 0, 0], &trex_payload);

    let mut mvex_payload = Vec::new();
    mvex_payload.extend_from_slice(&trex_box);
    let mvex_box = make_box(b"mvex", &mvex_payload);

    let mut mvhd_payload = vec![0u8; 96];
    mvhd_payload[12..16].copy_from_slice(&30000u32.to_be_bytes()); // timescale = 30000
    mvhd_payload[16..20].copy_from_slice(&0u32.to_be_bytes()); // duration = 0
    let mvhd_box = make_fullbox(b"mvhd", 0, [0, 0, 0], &mvhd_payload);

    let mut moov_payload = Vec::new();
    moov_payload.extend_from_slice(&mvhd_box);
    moov_payload.extend_from_slice(&trak_box);
    moov_payload.extend_from_slice(&mvex_box);
    let moov = make_box(b"moov", &moov_payload);
    fmp4.extend_from_slice(&moov);

    // 3. sidx (Segment Index)
    let mut sidx_payload = Vec::new();
    sidx_payload.extend_from_slice(&1u32.to_be_bytes()); // reference_id = 1
    sidx_payload.extend_from_slice(&30000u32.to_be_bytes()); // timescale = 30000
    sidx_payload.extend_from_slice(&0u32.to_be_bytes()); // earliest_presentation_time = 0
    sidx_payload.extend_from_slice(&0u32.to_be_bytes()); // first_offset = 0
    sidx_payload.extend_from_slice(&0u16.to_be_bytes()); // reserved = 0
    sidx_payload.extend_from_slice(&2u16.to_be_bytes()); // reference_count = 2 references

    // Reference 1: size=1000, duration=2002, starts_with_sap=1, sap_type=1
    let ref1_size_type = 1000u32;
    let ref1_dur = 2002u32;
    let ref1_sap = (1u32 << 31) | (1u32 << 28);
    sidx_payload.extend_from_slice(&ref1_size_type.to_be_bytes());
    sidx_payload.extend_from_slice(&ref1_dur.to_be_bytes());
    sidx_payload.extend_from_slice(&ref1_sap.to_be_bytes());

    // Reference 2: size=1000, duration=2002, starts_with_sap=1, sap_type=1
    let ref2_size_type = 1000u32;
    let ref2_dur = 2002u32;
    let ref2_sap = (1u32 << 31) | (1u32 << 28);
    sidx_payload.extend_from_slice(&ref2_size_type.to_be_bytes());
    sidx_payload.extend_from_slice(&ref2_dur.to_be_bytes());
    sidx_payload.extend_from_slice(&ref2_sap.to_be_bytes());

    let sidx = make_fullbox(b"sidx", 0, [0, 0, 0], &sidx_payload);
    fmp4.extend_from_slice(&sidx);

    // Helper to build a fragment (moof + mdat)
    let build_fragment = |seq: u32,
                          base_dt: u64,
                          samples: Vec<(Vec<u8>, u32, bool, Option<i64>)>|
     -> (Vec<u8>, Vec<u8>) {
        let mut mdat_data = Vec::new();
        for (data, _, _, _) in &samples {
            mdat_data.extend_from_slice(data);
        }
        let mdat = make_box(b"mdat", &mdat_data);

        // mfhd
        let mfhd = make_fullbox(b"mfhd", 0, [0, 0, 0], &seq.to_be_bytes());

        // tfhd: flags: default-base-is-moof (0x020000)
        let mut tfhd_payload = Vec::new();
        tfhd_payload.extend_from_slice(&1u32.to_be_bytes()); // track_id = 1
        let tfhd = make_fullbox(b"tfhd", 0, [0x02, 0x00, 0x00], &tfhd_payload);

        // tfdt: version 1, 64-bit base decode time
        let tfdt = make_fullbox(b"tfdt", 1, [0, 0, 0], &base_dt.to_be_bytes());

        // trun:
        // flags:
        // 0x000001: data-offset-present
        // 0x000100: sample-duration-present
        // 0x000200: sample-size-present
        // 0x000400: sample-flags-present
        // 0x000800: sample-composition-time-offsets-present
        let trun_flags = [0x00, 0x0f, 0x01];
        let mut trun_payload = Vec::new();
        trun_payload.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        // Placeholder data_offset (will be calculated: moof total size + 8 header of mdat)
        // We will calculate exact offset below.
        let data_offset_pos = trun_payload.len();
        trun_payload.extend_from_slice(&0i32.to_be_bytes());

        for (data, dur, is_key, c_offset) in &samples {
            trun_payload.extend_from_slice(&dur.to_be_bytes());
            trun_payload.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let flags = if *is_key {
                0x00000000u32
            } else {
                0x01010000u32
            };
            trun_payload.extend_from_slice(&flags.to_be_bytes());
            trun_payload.extend_from_slice(&(c_offset.unwrap_or(0) as u32).to_be_bytes());
        }

        // Calculate moof size to set accurate data_offset
        // traf = tfhd + tfdt + trun
        // moof = mfhd + traf
        let dummy_trun = make_fullbox(b"trun", 0, trun_flags, &trun_payload);
        let mut traf_payload = Vec::new();
        traf_payload.extend_from_slice(&tfhd);
        traf_payload.extend_from_slice(&tfdt);
        traf_payload.extend_from_slice(&dummy_trun);
        let dummy_traf = make_box(b"traf", &traf_payload);

        let mut moof_payload = Vec::new();
        moof_payload.extend_from_slice(&mfhd);
        moof_payload.extend_from_slice(&dummy_traf);
        let dummy_moof_size = (8 + moof_payload.len()) as i32;

        // data_offset = distance from moof start to mdat payload = moof_size + 8 (mdat header)
        let exact_data_offset = dummy_moof_size + 8;
        trun_payload[data_offset_pos..data_offset_pos + 4]
            .copy_from_slice(&exact_data_offset.to_be_bytes());

        let final_trun = make_fullbox(b"trun", 0, trun_flags, &trun_payload);
        let mut final_traf_payload = Vec::new();
        final_traf_payload.extend_from_slice(&tfhd);
        final_traf_payload.extend_from_slice(&tfdt);
        final_traf_payload.extend_from_slice(&final_trun);
        let final_traf = make_box(b"traf", &final_traf_payload);

        let mut final_moof_payload = Vec::new();
        final_moof_payload.extend_from_slice(&mfhd);
        final_moof_payload.extend_from_slice(&final_traf);
        let final_moof = make_box(b"moof", &final_moof_payload);

        (final_moof, mdat)
    };

    // Fragment 1: 2 samples, keyframe at dts=0, non-keyframe at dts=1001
    let sample0 = vec![0, 0, 0, 4, 0x65, 0xAA, 0xBB, 0xCC]; // IDR
    let sample1 = vec![0, 0, 0, 4, 0x41, 0x11, 0x22, 0x33]; // Non-IDR
    let frag1_samples = vec![
        (sample0, 1001, true, Some(0)),
        (sample1, 1001, false, Some(1001)),
    ];
    let (moof1, mdat1) = build_fragment(1, 0, frag1_samples);
    fmp4.extend_from_slice(&moof1);
    fmp4.extend_from_slice(&mdat1);

    // Fragment 2: 2 samples, keyframe at dts=2002, non-keyframe at dts=3003
    let sample2 = vec![0, 0, 0, 4, 0x65, 0xDD, 0xEE, 0xFF]; // IDR
    let sample3 = vec![0, 0, 0, 4, 0x41, 0x44, 0x55, 0x66]; // Non-IDR
    let frag2_samples = vec![
        (sample2, 1001, true, Some(0)),
        (sample3, 1001, false, Some(1001)),
    ];
    let (moof2, mdat2) = build_fragment(2, 2002, frag2_samples);
    fmp4.extend_from_slice(&moof2);
    fmp4.extend_from_slice(&mdat2);

    // 4. mfra (Movie Fragment Random Access)
    // tfra box for track 1
    let mut tfra_payload = Vec::new();
    tfra_payload.extend_from_slice(&1u32.to_be_bytes()); // track_id = 1
    tfra_payload.extend_from_slice(&0u32.to_be_bytes()); // length_size_of_traf_num/trun_num/sample_num
    tfra_payload.extend_from_slice(&2u32.to_be_bytes()); // number of entries = 2

    // Entry 1: time=0, moof_offset=...
    let moof1_offset = (ftyp.len() + moov.len() + sidx.len()) as u32;
    tfra_payload.extend_from_slice(&0u32.to_be_bytes()); // time = 0
    tfra_payload.extend_from_slice(&moof1_offset.to_be_bytes()); // moof_offset
    tfra_payload.extend_from_slice(&[1, 1, 1]); // traf=1, trun=1, sample=1

    // Entry 2: time=2002, moof_offset=...
    let moof2_offset = (moof1_offset as usize + moof1.len() + mdat1.len()) as u32;
    tfra_payload.extend_from_slice(&2002u32.to_be_bytes()); // time = 2002
    tfra_payload.extend_from_slice(&moof2_offset.to_be_bytes()); // moof_offset
    tfra_payload.extend_from_slice(&[1, 1, 1]); // traf=1, trun=1, sample=1

    let tfra = make_fullbox(b"tfra", 0, [0, 0, 0], &tfra_payload);

    // mfro
    let mfro_size = 16u32; // size(4) + fourcc(4) + fullbox(4) + parent_size(4)
    let mfra_total_size = (8 + tfra.len() + mfro_size as usize) as u32;
    let mfro = make_fullbox(b"mfro", 0, [0, 0, 0], &mfra_total_size.to_be_bytes());

    let mut mfra_payload = Vec::new();
    mfra_payload.extend_from_slice(&tfra);
    mfra_payload.extend_from_slice(&mfro);
    let mfra = make_box(b"mfra", &mfra_payload);
    fmp4.extend_from_slice(&mfra);

    fmp4
}

#[test]
fn test_fmp4_demux_full_integration() {
    let fmp4_bytes = build_synthetic_fmp4();
    let mut demuxer = Mp4Demuxer::from_bytes(Bytes::from(fmp4_bytes))
        .expect("Mp4Demuxer::from_bytes must parse valid synthetic fMP4");

    // 1. Verify stream information
    assert_eq!(demuxer.streams().len(), 1);
    let stream = &demuxer.streams()[0];
    assert_eq!(stream.codec, CodecId::H264);
    assert_eq!(stream.time_base.den, 30000);
    assert!(stream.is_video());
    assert!(!stream.is_audio());

    // Duration should reflect decoded samples from fragments: 4 samples * 1001 ticks = 4004
    assert_eq!(stream.duration_ticks, Some(4004));

    // 2. Verify DASH sidx segments
    assert_eq!(demuxer.sidx_segments().len(), 1);
    let sidx = &demuxer.sidx_segments()[0];
    assert_eq!(sidx.reference_id, 1);
    assert_eq!(sidx.timescale, 30000);
    assert_eq!(sidx.references.len(), 2);
    assert_eq!(sidx.references[0].subsegment_duration, 2002);
    assert_eq!(sidx.references[1].subsegment_duration, 2002);

    // 3. Verify mfra / tfra entries
    assert_eq!(demuxer.mfra_entries().len(), 1);
    assert_eq!(demuxer.mfra_entries()[0].track_id, 1);
    assert_eq!(demuxer.mfra_entries()[0].entries.len(), 2);
    assert_eq!(demuxer.mfra_entries()[0].entries[0].0, 0);
    assert_eq!(demuxer.mfra_entries()[0].entries[1].0, 2002);

    // 4. Demux all packets across both fragments
    let p0 = demuxer.read_packet().unwrap().expect("packet 0");
    assert_eq!(p0.dts, 0);
    assert_eq!(p0.pts, Some(0));
    assert_eq!(p0.duration, 1001);
    assert!(p0.is_keyframe);
    assert_eq!(p0.data.as_ref(), &[0, 0, 0, 4, 0x65, 0xAA, 0xBB, 0xCC]);

    let p1 = demuxer.read_packet().unwrap().expect("packet 1");
    assert_eq!(p1.dts, 1001);
    assert_eq!(p1.pts, Some(2002));
    assert_eq!(p1.duration, 1001);
    assert!(!p1.is_keyframe);
    assert_eq!(p1.data.as_ref(), &[0, 0, 0, 4, 0x41, 0x11, 0x22, 0x33]);

    let p2 = demuxer.read_packet().unwrap().expect("packet 2");
    assert_eq!(p2.dts, 2002);
    assert_eq!(p2.pts, Some(2002));
    assert_eq!(p2.duration, 1001);
    assert!(p2.is_keyframe);
    assert_eq!(p2.data.as_ref(), &[0, 0, 0, 4, 0x65, 0xDD, 0xEE, 0xFF]);

    let p3 = demuxer.read_packet().unwrap().expect("packet 3");
    assert_eq!(p3.dts, 3003);
    assert_eq!(p3.pts, Some(4004));
    assert_eq!(p3.duration, 1001);
    assert!(!p3.is_keyframe);
    assert_eq!(p3.data.as_ref(), &[0, 0, 0, 4, 0x41, 0x44, 0x55, 0x66]);

    // End of stream
    assert!(demuxer.read_packet().unwrap().is_none());

    // 5. Seek across fragment boundary to keyframe in fragment 2 (dts = 2002)
    demuxer
        .seek_to_keyframe(0, 2002)
        .expect("seek to fragment 2 keyframe");
    let p_seek = demuxer.read_packet().unwrap().expect("packet after seek");
    assert_eq!(p_seek.dts, 2002);
    assert!(p_seek.is_keyframe);

    // Seek back to fragment 1 keyframe (dts = 0)
    demuxer
        .seek_to_keyframe(0, 0)
        .expect("seek to fragment 1 keyframe");
    let p_seek0 = demuxer
        .read_packet()
        .unwrap()
        .expect("packet after seek to 0");
    assert_eq!(p_seek0.dts, 0);
    assert!(p_seek0.is_keyframe);
}

#[test]
fn test_fmp4_streaming_reader() {
    let fmp4_bytes = build_synthetic_fmp4();
    let cursor = std::io::Cursor::new(fmp4_bytes);
    let mut demuxer = Mp4Demuxer::open_streaming(cursor).expect("open_streaming on fMP4");

    assert_eq!(demuxer.streams().len(), 1);
    let p = demuxer.read_packet().unwrap().expect("first packet");
    assert_eq!(p.dts, 0);
    assert!(p.is_keyframe);
}

#[test]
fn test_fmp4_corrupt_moof_handling() {
    let mut fmp4_bytes = build_synthetic_fmp4();
    // Corrupt the moof body by truncating mid-box
    let len = fmp4_bytes.len();
    fmp4_bytes.truncate(len / 2);
    // Demuxer should gracefully report DemuxError instead of panicking
    let res = Mp4Demuxer::from_bytes(Bytes::from(fmp4_bytes));
    let _ = res; // May be Err or partial Ok, but must never panic
}
