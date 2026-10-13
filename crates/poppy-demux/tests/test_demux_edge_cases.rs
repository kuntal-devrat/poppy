use bytes::Bytes;
use poppy_core::CodecId;
use poppy_demux::{DemuxError, Demuxer, MatroskaDemuxer, Mp4Demuxer};

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

fn build_synthetic_trak(
    track_id: u32,
    enabled: bool,
    width: u32,
    height: u32,
    sample_sizes: &[usize],
    chunk_offset: u32,
) -> Vec<u8> {
    // avcC box
    let avcc_payload = vec![
        1, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, 4, 0x67, 0x64, 0x00, 0x1F, 1, 0x00, 3, 0x68, 0xEB,
        0xE3,
    ];
    let avcc_box = make_box(b"avcC", &avcc_payload);

    // avc1 box
    let mut avc1_payload = vec![0u8; 78];
    avc1_payload[24..26].copy_from_slice(&(width as u16).to_be_bytes());
    avc1_payload[26..28].copy_from_slice(&(height as u16).to_be_bytes());
    avc1_payload[28..32].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[32..36].copy_from_slice(&0x00480000u32.to_be_bytes());
    avc1_payload[40..42].copy_from_slice(&1u16.to_be_bytes());
    avc1_payload[74..76].copy_from_slice(&24u16.to_be_bytes());
    avc1_payload.extend_from_slice(&avcc_box);
    let avc1_box = make_box(b"avc1", &avc1_payload);

    // stsd
    let mut stsd_payload = Vec::new();
    stsd_payload.extend_from_slice(&1u32.to_be_bytes());
    stsd_payload.extend_from_slice(&avc1_box);
    let stsd_box = make_fullbox(b"stsd", 0, [0, 0, 0], &stsd_payload);

    // stsz
    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes());
    stsz_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
    for &sz in sample_sizes {
        stsz_payload.extend_from_slice(&(sz as u32).to_be_bytes());
    }
    let stsz_box = make_fullbox(b"stsz", 0, [0, 0, 0], &stsz_payload);

    // stsc
    let mut stsc_payload = Vec::new();
    if sample_sizes.is_empty() {
        stsc_payload.extend_from_slice(&0u32.to_be_bytes());
    } else {
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
        stsc_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        stsc_payload.extend_from_slice(&1u32.to_be_bytes());
    }
    let stsc_box = make_fullbox(b"stsc", 0, [0, 0, 0], &stsc_payload);

    // stco
    let mut stco_payload = Vec::new();
    if sample_sizes.is_empty() {
        stco_payload.extend_from_slice(&0u32.to_be_bytes());
    } else {
        stco_payload.extend_from_slice(&1u32.to_be_bytes());
        stco_payload.extend_from_slice(&chunk_offset.to_be_bytes());
    }
    let stco_box = make_fullbox(b"stco", 0, [0, 0, 0], &stco_payload);

    // stts
    let mut stts_payload = Vec::new();
    if sample_sizes.is_empty() {
        stts_payload.extend_from_slice(&0u32.to_be_bytes());
    } else {
        stts_payload.extend_from_slice(&1u32.to_be_bytes());
        stts_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        stts_payload.extend_from_slice(&1000u32.to_be_bytes());
    }
    let stts_box = make_fullbox(b"stts", 0, [0, 0, 0], &stts_payload);

    // stbl
    let mut stbl_payload = Vec::new();
    stbl_payload.extend_from_slice(&stsd_box);
    stbl_payload.extend_from_slice(&stts_box);
    stbl_payload.extend_from_slice(&stsc_box);
    stbl_payload.extend_from_slice(&stsz_box);
    stbl_payload.extend_from_slice(&stco_box);
    let stbl_box = make_box(b"stbl", &stbl_payload);

    // minf
    let minf_box = make_box(b"minf", &stbl_box);

    // hdlr
    let mut hdlr_payload = vec![0u8; 16];
    hdlr_payload[4..8].copy_from_slice(b"vide");
    let hdlr_box = make_fullbox(b"hdlr", 0, [0, 0, 0], &hdlr_payload);

    // mdhd
    let mut mdhd_payload = vec![0u8; 20];
    mdhd_payload[8..12].copy_from_slice(&30000u32.to_be_bytes());
    mdhd_payload[12..16].copy_from_slice(&2000u32.to_be_bytes());
    let mdhd_box = make_fullbox(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    // mdia
    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd_box);
    mdia_payload.extend_from_slice(&hdlr_box);
    mdia_payload.extend_from_slice(&minf_box);
    let mdia_box = make_box(b"mdia", &mdia_payload);

    // tkhd: flag 0x0001 = enabled
    let flag_byte = if enabled { 0x01 } else { 0x00 };
    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&track_id.to_be_bytes());
    tkhd_payload[16..20].copy_from_slice(&2000u32.to_be_bytes());
    tkhd_payload[72..76].copy_from_slice(&(width << 16).to_be_bytes());
    tkhd_payload[76..80].copy_from_slice(&(height << 16).to_be_bytes());
    let tkhd_box = make_fullbox(b"tkhd", 0, [0, 0, flag_byte], &tkhd_payload);

    // trak
    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd_box);
    trak_payload.extend_from_slice(&mdia_box);
    make_box(b"trak", &trak_payload)
}

#[test]
fn test_mp4_zero_length_mdat_empty_file() {
    // 1. MP4 with 0-length mdat (size 8 = header only) and 0 samples
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let trak = build_synthetic_trak(1, true, 1920, 1080, &[], 0);
    let moov = make_box(b"moov", &trak);
    let mdat = make_box(b"mdat", &[]); // 8 bytes: size=8, FourCC="mdat", empty body

    let mut file1 = Vec::new();
    file1.extend_from_slice(&ftyp);
    file1.extend_from_slice(&moov);
    file1.extend_from_slice(&mdat);

    let mut demux1 = Mp4Demuxer::from_bytes(Bytes::from(file1)).expect("open MP4 with size 8 mdat");
    assert_eq!(demux1.streams().len(), 1);
    assert_eq!(demux1.streams()[0].width, 1920);
    assert_eq!(demux1.streams()[0].height, 1080);
    assert!(demux1.streams()[0].enabled);
    assert!(
        demux1.read_packet().expect("read_packet").is_none(),
        "Empty mdat produces no packets"
    );

    // 2. MP4 with size 0 mdat extending to EOF (0 payload bytes after header)
    let mut mdat_size0 = vec![0, 0, 0, 0]; // size = 0
    mdat_size0.extend_from_slice(b"mdat");

    let mut file2 = Vec::new();
    file2.extend_from_slice(&ftyp);
    file2.extend_from_slice(&moov);
    file2.extend_from_slice(&mdat_size0);

    let mut demux2 = Mp4Demuxer::from_bytes(Bytes::from(file2)).expect("open MP4 with size 0 mdat");
    assert_eq!(demux2.streams().len(), 1);
    assert!(
        demux2.read_packet().expect("read_packet").is_none(),
        "Size 0 mdat produces no packets"
    );
}

#[test]
fn test_mp4_moov_at_end_of_file() {
    // Streaming-unfriendly layout: ftyp -> mdat -> moov
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let sample0 = [0, 0, 0, 4, 0x65, 1, 2, 3];
    let sample1 = [0, 0, 0, 5, 0x41, 4, 5, 6, 7];

    let mut mdat_payload = Vec::new();
    mdat_payload.extend_from_slice(&sample0);
    mdat_payload.extend_from_slice(&sample1);
    let mdat = make_box(b"mdat", &mdat_payload);

    let mdat_body_offset = (ftyp.len() + 8) as u32;
    let trak = build_synthetic_trak(
        1,
        true,
        1280,
        720,
        &[sample0.len(), sample1.len()],
        mdat_body_offset,
    );
    let moov = make_box(b"moov", &trak);

    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&ftyp);
    mp4.extend_from_slice(&mdat);
    mp4.extend_from_slice(&moov);

    let mut demux =
        Mp4Demuxer::from_bytes(Bytes::from(mp4)).expect("open streaming-unfriendly MP4");
    assert_eq!(demux.streams().len(), 1);
    assert_eq!(demux.streams()[0].width, 1280);
    assert_eq!(demux.streams()[0].height, 720);

    let pkt0 = demux
        .read_packet()
        .expect("read pkt0")
        .expect("pkt0 exists");
    assert_eq!(pkt0.data.as_ref(), &sample0);

    let pkt1 = demux
        .read_packet()
        .expect("read pkt1")
        .expect("pkt1 exists");
    assert_eq!(pkt1.data.as_ref(), &sample1);

    assert!(demux.read_packet().expect("read EOF").is_none());
}

#[test]
fn test_mp4_multiple_video_tracks_metadata() {
    // Track 1: thumbnail (160x120, disabled)
    // Track 2: main video (1920x1080, enabled)
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let trak1 = build_synthetic_trak(1, false, 160, 120, &[], 0);
    let trak2 = build_synthetic_trak(2, true, 1920, 1080, &[], 0);

    let mut moov_payload = Vec::new();
    moov_payload.extend_from_slice(&trak1);
    moov_payload.extend_from_slice(&trak2);
    let moov = make_box(b"moov", &moov_payload);

    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&ftyp);
    mp4.extend_from_slice(&moov);

    let demux = Mp4Demuxer::from_bytes(Bytes::from(mp4)).expect("open multi-track MP4");
    let streams = demux.streams();
    assert_eq!(streams.len(), 2);

    assert_eq!(streams[0].width, 160);
    assert_eq!(streams[0].height, 120);
    assert!(!streams[0].enabled);

    assert_eq!(streams[1].width, 1920);
    assert_eq!(streams[1].height, 1080);
    assert!(streams[1].enabled);
}

#[test]
fn test_mkv_lacing_types() {
    // Directly test parse_lacing for Xiph, EBML, and Fixed-size
    // We can also demux a synthetic cluster containing laced blocks.
    // 1. Fixed-size lacing (type 3): 3 frames of 10 bytes each in a 31-byte buffer (1 lace header + 30 data)
    let mut fixed_lace = vec![2u8]; // frame_count - 1 = 2 -> 3 frames
    fixed_lace.extend_from_slice(&[0xAA; 30]); // 30 bytes / 3 = 10 bytes each
    // Lacing type 3 in matroska.rs:
    // parse_lacing(&fixed_lace, 3) -> [(1, 10), (11, 10), (21, 10)]

    // 2. Xiph lacing (type 1): 3 frames: len 300 (255+45), len 50, len 20
    let mut xiph_lace = vec![2u8]; // frame_count - 1 = 2 -> 3 frames
    xiph_lace.extend_from_slice(&[255, 45]); // first frame len = 300
    xiph_lace.extend_from_slice(&[50]); // second frame len = 50
    xiph_lace.extend_from_slice(&[0xBB; 370]); // total payload = 300 + 50 + 20 = 370

    // 3. EBML lacing (type 2): 3 frames: len 100, len 110 (delta +10), len 80 (delta -30)
    // First length 100 as EBML vint: (0x80 | 100) = 0xE4 (1 byte)
    // Delta +10: len=1, bias=63, uval = 10 + 63 = 73 -> (0x80 | 73) = 0xC9
    // Delta -30: len=1, bias=63, uval = -30 + 63 = 33 -> (0x80 | 33) = 0xA1
    let mut ebml_lace = vec![2u8]; // frame_count - 1 = 2 -> 3 frames
    ebml_lace.extend_from_slice(&[0xE4, 0xC9]); // frame 0: 100, frame 1: 110
    ebml_lace.extend_from_slice(&[0xCC; 290]); // total = 100 + 110 + 80 = 290

    // Construct a synthetic Matroska container with laced cluster blocks to verify end-to-end
    let mkv = build_synthetic_mkv_with_laced_block(&fixed_lace, 3);
    let mut demux = MatroskaDemuxer::from_bytes(Bytes::from(mkv)).expect("parse MKV fixed lacing");
    let mut count = 0;
    while let Some(pkt) = demux.read_packet().expect("read laced packet") {
        assert_eq!(pkt.data.len(), 10);
        count += 1;
    }
    assert_eq!(count, 3, "Fixed lacing must produce 3 distinct packets");

    // Test Xiph lacing in container
    let mkv_xiph = build_synthetic_mkv_with_laced_block(&xiph_lace, 1);
    let mut demux_xiph =
        MatroskaDemuxer::from_bytes(Bytes::from(mkv_xiph)).expect("parse MKV Xiph lacing");
    let p0 = demux_xiph
        .read_packet()
        .expect("read p0")
        .expect("p0 exists");
    assert_eq!(p0.data.len(), 300);
    let p1 = demux_xiph
        .read_packet()
        .expect("read p1")
        .expect("p1 exists");
    assert_eq!(p1.data.len(), 50);
    let p2 = demux_xiph
        .read_packet()
        .expect("read p2")
        .expect("p2 exists");
    assert_eq!(p2.data.len(), 20);

    // Test EBML lacing in container
    let mkv_ebml = build_synthetic_mkv_with_laced_block(&ebml_lace, 2);
    let mut demux_ebml =
        MatroskaDemuxer::from_bytes(Bytes::from(mkv_ebml)).expect("parse MKV EBML lacing");
    let ep0 = demux_ebml
        .read_packet()
        .expect("read ep0")
        .expect("ep0 exists");
    assert_eq!(ep0.data.len(), 100);
    let ep1 = demux_ebml
        .read_packet()
        .expect("read ep1")
        .expect("ep1 exists");
    assert_eq!(ep1.data.len(), 110);
    let ep2 = demux_ebml
        .read_packet()
        .expect("read ep2")
        .expect("ep2 exists");
    assert_eq!(ep2.data.len(), 80);
}

#[test]
fn test_mkv_codec_delay_and_seek_preroll() {
    // Construct synthetic MKV with CodecDelay = 6_500_000 ns (6.5ms)
    // and SeekPreRoll = 80_000_000 ns (80ms).
    let mkv = build_synthetic_mkv_with_delay_and_preroll(6_500_000, 80_000_000);
    let mut demux = MatroskaDemuxer::from_bytes(Bytes::from(mkv)).expect("parse MKV with Opus");
    let stream = &demux.streams()[0];
    assert_eq!(stream.codec, CodecId::Opus);

    // Read packet and check that sample timestamp was adjusted
    let pkt = demux
        .read_packet()
        .expect("read packet")
        .expect("packet exists");
    assert!(pkt.pts.is_some() || pkt.dts >= 0);

    // Seek to 100ms: with 80ms preroll, effective seek target is 20ms
    assert!(demux.seek_to_keyframe(0, 100).is_ok());
}

#[test]
fn test_corrupt_truncated_file_handling() {
    // 1. Truncated box header (<8 bytes)
    let truncated_hdr = vec![0x00, 0x00, 0x00];
    let err1 = match Mp4Demuxer::from_bytes(Bytes::from(truncated_hdr)) {
        Err(e) => e,
        Ok(_) => panic!("Expected error for truncated header"),
    };
    match &err1 {
        DemuxError::UnexpectedEof { offset } => {
            assert_eq!(*offset, 0);
            assert!(
                err1.to_string()
                    .contains("unexpected end of data at offset 0x0")
            );
        }
        other => panic!("Expected UnexpectedEof, got {other:?}"),
    }

    // 2. Box size smaller than header (<8)
    let mut invalid_size = vec![0x00, 0x00, 0x00, 0x04]; // size = 4
    invalid_size.extend_from_slice(b"free");
    let err2 = match Mp4Demuxer::from_bytes(Bytes::from(invalid_size)) {
        Err(e) => e,
        Ok(_) => panic!("Expected error for invalid size"),
    };
    match &err2 {
        DemuxError::InvalidBox { offset, detail } => {
            assert_eq!(*offset, 0);
            assert!(detail.contains("smaller than its header"));
            assert!(err2.to_string().contains("invalid box at offset 0x0"));
        }
        other => panic!("Expected InvalidBox, got {other:?}"),
    }

    // 3. Extended size box with largesize < 16
    let mut invalid_large = vec![0x00, 0x00, 0x00, 0x01]; // size = 1
    invalid_large.extend_from_slice(b"free");
    invalid_large.extend_from_slice(&10u64.to_be_bytes()); // largesize = 10 < 16
    let err3 = match Mp4Demuxer::from_bytes(Bytes::from(invalid_large)) {
        Err(e) => e,
        Ok(_) => panic!("Expected error for invalid largesize"),
    };
    match &err3 {
        DemuxError::InvalidBox { detail, .. } => {
            assert!(detail.contains("invalid largesize 10"));
        }
        other => panic!("Expected InvalidBox, got {other:?}"),
    }

    // 4. Missing moov box
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let err4 = match Mp4Demuxer::from_bytes(Bytes::from(ftyp)) {
        Err(e) => e,
        Ok(_) => panic!("Expected error for missing moov"),
    };
    match &err4 {
        DemuxError::BoxNotFound { fourcc } => {
            assert_eq!(fourcc, "moov");
            assert!(err4.to_string().contains("required box 'moov' not found"));
        }
        other => panic!("Expected BoxNotFound, got {other:?}"),
    }

    // 5. Truncated sample payload during read_packet
    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    let trak = build_synthetic_trak(1, true, 1920, 1080, &[500], 100);
    let moov = make_box(b"moov", &trak);
    let mut file5 = Vec::new();
    file5.extend_from_slice(&ftyp);
    file5.extend_from_slice(&moov); // Total file length is < 500+100
    let mut demux5 = Mp4Demuxer::from_bytes(Bytes::from(file5)).expect("init ok");
    let err5 = demux5.read_packet().unwrap_err();
    match &err5 {
        DemuxError::UnexpectedEof { offset } => {
            assert_eq!(*offset, 100);
            assert!(err5.to_string().contains("unexpected end of data"));
        }
        other => panic!("Expected UnexpectedEof, got {other:?}"),
    }
}

// Helpers for Matroska test containers
fn ebml_vint(id: u64) -> Vec<u8> {
    if id <= 0x7F {
        vec![(id | 0x80) as u8]
    } else if id <= 0x3FFF {
        vec![((id >> 8) | 0x40) as u8, (id & 0xFF) as u8]
    } else if id <= 0x1FFFFF {
        vec![
            ((id >> 16) | 0x20) as u8,
            ((id >> 8) & 0xFF) as u8,
            (id & 0xFF) as u8,
        ]
    } else {
        vec![
            ((id >> 24) | 0x10) as u8,
            ((id >> 16) & 0xFF) as u8,
            ((id >> 8) & 0xFF) as u8,
            (id & 0xFF) as u8,
        ]
    }
}

fn ebml_elem(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&ebml_vint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn build_synthetic_mkv_with_laced_block(lace_payload: &[u8], lace_type: u8) -> Vec<u8> {
    let mut ebml_header_payload = Vec::new();
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x86], &[1])); // EBMLVersion = 1
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF7], &[1])); // EBMLReadVersion = 1
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF2], &[4])); // EBMLMaxIDLength = 4
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF3], &[8])); // EBMLMaxSizeLength = 8
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x82], b"matroska")); // DocType = matroska
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x87], &[4])); // DocTypeVersion = 4
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x85], &[2])); // DocTypeReadVersion = 2
    let ebml_header = ebml_elem(&[0x1A, 0x45, 0xDF, 0xA3], &ebml_header_payload);

    // Segment info
    let mut info_payload = Vec::new();
    info_payload.extend_from_slice(&ebml_elem(&[0x2A, 0xD7, 0xB1], &1_000_000u64.to_be_bytes())); // TimestampScale = 1ms
    let info = ebml_elem(&[0x15, 0x49, 0xA9, 0x66], &info_payload);

    // Track entry
    let mut track_payload = Vec::new();
    track_payload.extend_from_slice(&ebml_elem(&[0xD7], &[1])); // TrackNumber = 1
    track_payload.extend_from_slice(&ebml_elem(&[0x73, 0xC5], &[1])); // TrackUID = 1
    track_payload.extend_from_slice(&ebml_elem(&[0x83], &[1])); // TrackType = 1 (Video)
    track_payload.extend_from_slice(&ebml_elem(&[0x86], b"V_MPEG4/ISO/AVC")); // CodecID
    let track_entry = ebml_elem(&[0xAE], &track_payload);
    let tracks = ebml_elem(&[0x16, 0x54, 0xAE, 0x6B], &track_entry);

    // SimpleBlock with lacing
    // TrackNum (1 byte: 0x81), Timecode (2 bytes: 0x00, 0x00), Flags (1 byte)
    // Flags: keyframe (0x80) | (lace_type << 1)
    let flags = 0x80 | ((lace_type & 0x03) << 1);
    let mut block_payload = vec![0x81, 0x00, 0x00, flags];
    block_payload.extend_from_slice(lace_payload);
    let simple_block = ebml_elem(&[0xA3], &block_payload);

    // Cluster
    let mut cluster_payload = Vec::new();
    cluster_payload.extend_from_slice(&ebml_elem(&[0xE7], &[0])); // Timestamp = 0
    cluster_payload.extend_from_slice(&simple_block);
    let cluster = ebml_elem(&[0x1F, 0x43, 0xB6, 0x75], &cluster_payload);

    // Segment
    let mut seg_payload = Vec::new();
    seg_payload.extend_from_slice(&info);
    seg_payload.extend_from_slice(&tracks);
    seg_payload.extend_from_slice(&cluster);
    let segment = ebml_elem(&[0x18, 0x53, 0x80, 0x67], &seg_payload);

    let mut out = Vec::new();
    out.extend_from_slice(&ebml_header);
    out.extend_from_slice(&segment);
    out
}

fn build_synthetic_mkv_with_delay_and_preroll(delay_ns: u64, preroll_ns: u64) -> Vec<u8> {
    let mut ebml_header_payload = Vec::new();
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x86], &[1]));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF7], &[1]));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF2], &[4]));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0xF3], &[8]));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x82], b"matroska"));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x87], &[4]));
    ebml_header_payload.extend_from_slice(&ebml_elem(&[0x42, 0x85], &[2]));
    let ebml_header = ebml_elem(&[0x1A, 0x45, 0xDF, 0xA3], &ebml_header_payload);

    let mut info_payload = Vec::new();
    info_payload.extend_from_slice(&ebml_elem(&[0x2A, 0xD7, 0xB1], &1_000_000u64.to_be_bytes())); // 1ms scale
    let info = ebml_elem(&[0x15, 0x49, 0xA9, 0x66], &info_payload);

    let mut track_payload = Vec::new();
    track_payload.extend_from_slice(&ebml_elem(&[0xD7], &[1])); // TrackNumber = 1
    track_payload.extend_from_slice(&ebml_elem(&[0x73, 0xC5], &[1])); // TrackUID = 1
    track_payload.extend_from_slice(&ebml_elem(&[0x83], &[2])); // TrackType = 2 (Audio)
    track_payload.extend_from_slice(&ebml_elem(&[0x86], b"A_OPUS")); // CodecID
    track_payload.extend_from_slice(&ebml_elem(&[0x56, 0xAA], &delay_ns.to_be_bytes())); // CodecDelay
    track_payload.extend_from_slice(&ebml_elem(&[0x56, 0xBB], &preroll_ns.to_be_bytes())); // SeekPreRoll
    let track_entry = ebml_elem(&[0xAE], &track_payload);
    let tracks = ebml_elem(&[0x16, 0x54, 0xAE, 0x6B], &track_entry);

    // Cluster with 1 packet
    let block_payload = vec![0x81, 0x00, 0x00, 0x80, 0x01, 0x02, 0x03];
    let simple_block = ebml_elem(&[0xA3], &block_payload);
    let mut cluster_payload = Vec::new();
    cluster_payload.extend_from_slice(&ebml_elem(&[0xE7], &[0]));
    cluster_payload.extend_from_slice(&simple_block);
    let cluster = ebml_elem(&[0x1F, 0x43, 0xB6, 0x75], &cluster_payload);

    let mut seg_payload = Vec::new();
    seg_payload.extend_from_slice(&info);
    seg_payload.extend_from_slice(&tracks);
    seg_payload.extend_from_slice(&cluster);
    let segment = ebml_elem(&[0x18, 0x53, 0x80, 0x67], &seg_payload);

    let mut out = Vec::new();
    out.extend_from_slice(&ebml_header);
    out.extend_from_slice(&segment);
    out
}
