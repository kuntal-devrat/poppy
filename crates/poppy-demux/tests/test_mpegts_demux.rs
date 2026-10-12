use bytes::Bytes;
use poppy_core::CodecId;
use poppy_demux::mpegts::MpegTsDemuxer;
use poppy_demux::Demuxer;

/// Helper function to construct a valid PAT (PID 0) TS packet (188 bytes).
fn make_pat_packet(pmt_pid: u16) -> [u8; 188] {
    let mut pkt = [0xFF; 188];
    pkt[0] = 0x47; // Sync byte
    pkt[1] = 0x40; // PUSI = 1, PID = 0
    pkt[2] = 0x00; // PID = 0
    pkt[3] = 0x10; // AFC = 01 (payload only), CC = 0

    // Pointer field
    pkt[4] = 0x00;

    // PAT Table Header
    pkt[5] = 0x00; // Table ID = 0 (PAT)
    pkt[6] = 0xB0; // Section syntax indicator = 1, Section length MSB
    pkt[7] = 0x0D; // Section length = 13 bytes (5 header + 4 program + 4 CRC)
    pkt[8] = 0x00; // Transport stream ID MSB
    pkt[9] = 0x01; // Transport stream ID LSB
    pkt[10] = 0xC1; // Version 0, current/next = 1
    pkt[11] = 0x00; // Section number 0
    pkt[12] = 0x00; // Last section number 0

    // Program 1 entry
    pkt[13] = 0x00;
    pkt[14] = 0x01; // Program number = 1
    pkt[15] = 0xE0 | ((pmt_pid >> 8) as u8 & 0x1F); // PMT PID MSB
    pkt[16] = (pmt_pid & 0xFF) as u8; // PMT PID LSB

    // 4-byte mock CRC32
    pkt[17] = 0xAA;
    pkt[18] = 0xBB;
    pkt[19] = 0xCC;
    pkt[20] = 0xDD;

    pkt
}

/// Helper function to construct a valid PMT TS packet (188 bytes) with H.264 video and AAC audio.
fn make_pmt_packet(pmt_pid: u16, video_pid: u16, audio_pid: u16) -> [u8; 188] {
    let mut pkt = [0xFF; 188];
    pkt[0] = 0x47; // Sync byte
    pkt[1] = 0x40 | ((pmt_pid >> 8) as u8 & 0x1F); // PUSI = 1, PID MSB
    pkt[2] = (pmt_pid & 0xFF) as u8; // PID LSB
    pkt[3] = 0x10; // AFC = 01 (payload only), CC = 0

    // Pointer field
    pkt[4] = 0x00;

    // PMT Table Header
    pkt[5] = 0x02; // Table ID = 2 (PMT)
    pkt[6] = 0xB0; // Section syntax indicator = 1
    pkt[7] = 0x17; // Section length = 23 bytes (9 header + 5 ES1 + 5 ES2 + 4 CRC)
    pkt[8] = 0x00;
    pkt[9] = 0x01; // Program number = 1
    pkt[10] = 0xC1; // Version 0, current/next = 1
    pkt[11] = 0x00; // Section number 0
    pkt[12] = 0x00; // Last section number 0
    pkt[13] = 0xE0 | ((video_pid >> 8) as u8 & 0x1F); // PCR PID MSB
    pkt[14] = (video_pid & 0xFF) as u8; // PCR PID LSB
    pkt[15] = 0xF0; // Program info length MSB = 0
    pkt[16] = 0x00; // Program info length LSB = 0

    // ES 1: H.264 Video (stream type 0x1B)
    pkt[17] = 0x1B; // Stream type: H.264
    pkt[18] = 0xE0 | ((video_pid >> 8) as u8 & 0x1F);
    pkt[19] = (video_pid & 0xFF) as u8;
    pkt[20] = 0xF0; // ES info length MSB = 0
    pkt[21] = 0x00; // ES info length LSB = 0

    // ES 2: AAC Audio (stream type 0x0F)
    pkt[22] = 0x0F; // Stream type: AAC ADTS
    pkt[23] = 0xE0 | ((audio_pid >> 8) as u8 & 0x1F);
    pkt[24] = (audio_pid & 0xFF) as u8;
    pkt[25] = 0xF0;
    pkt[26] = 0x00;

    // 4-byte mock CRC32
    pkt[27] = 0x11;
    pkt[28] = 0x22;
    pkt[29] = 0x33;
    pkt[30] = 0x44;

    pkt
}

/// Helper function to construct a PES video TS packet with PTS.
fn make_pes_packet_part1(
    pid: u16,
    cc: u8,
    pts: i64,
    is_random_access: bool,
    payload_prefix: &[u8],
) -> [u8; 188] {
    let mut pkt = [0xFF; 188];
    pkt[0] = 0x47; // Sync byte
    pkt[1] = 0x40 | ((pid >> 8) as u8 & 0x1F); // PUSI = 1
    pkt[2] = (pid & 0xFF) as u8;

    let payload_start = if is_random_access {
        pkt[3] = 0x30 | (cc & 0x0F); // AFC = 11 (Adaptation + Payload)
        pkt[4] = 0x01; // Adaptation field length = 1
        pkt[5] = 0x40; // random_access_indicator = 1
        6
    } else {
        pkt[3] = 0x10 | (cc & 0x0F); // AFC = 01 (Payload only)
        4
    };

    // PES Header: 00 00 01 E0
    pkt[payload_start] = 0x00;
    pkt[payload_start + 1] = 0x00;
    pkt[payload_start + 2] = 0x01;
    pkt[payload_start + 3] = 0xE0; // Video stream ID
    pkt[payload_start + 4] = 0x00; // Unbounded PES length
    pkt[payload_start + 5] = 0x00;
    pkt[payload_start + 6] = 0x80; // Flags 1
    pkt[payload_start + 7] = 0x80; // Flags 2 (PTS present, PTS_DTS_flags = 10)
    pkt[payload_start + 8] = 0x05; // PES header data length = 5 bytes

    // 5-byte 33-bit PTS encoding: 0010 PTS[32..30] 1 PTS[29..15] 1 PTS[14..0] 1
    let b0 = 0x21 | (((pts >> 29) as u8 & 0x0E));
    let b1 = ((pts >> 22) as u8) & 0xFF;
    let b2 = 0x01 | (((pts >> 14) as u8 & 0xFE));
    let b3 = ((pts >> 7) as u8) & 0xFF;
    let b4 = 0x01 | (((pts << 1) as u8 & 0xFE));

    pkt[payload_start + 9] = b0;
    pkt[payload_start + 10] = b1;
    pkt[payload_start + 11] = b2;
    pkt[payload_start + 12] = b3;
    pkt[payload_start + 13] = b4;

    let data_offset = payload_start + 14;
    let copy_len = payload_prefix.len().min(188 - data_offset);
    pkt[data_offset..data_offset + copy_len].copy_from_slice(&payload_prefix[..copy_len]);

    pkt
}

/// Helper function to construct a continuation PES TS packet (PUSI = 0).
fn make_pes_packet_part2(pid: u16, cc: u8, data: &[u8]) -> [u8; 188] {
    let mut pkt = [0xFF; 188];
    pkt[0] = 0x47;
    pkt[1] = (pid >> 8) as u8 & 0x1F; // PUSI = 0
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F); // AFC = 01

    let copy_len = data.len().min(184);
    pkt[4..4 + copy_len].copy_from_slice(&data[..copy_len]);
    pkt
}

#[test]
fn test_mpegts_demuxer_pat_pmt_and_stream_discovery() {
    let pmt_pid = 0x1000;
    let video_pid = 0x0100;
    let audio_pid = 0x0101;

    let mut stream_bytes = Vec::new();
    stream_bytes.extend_from_slice(&make_pat_packet(pmt_pid));
    stream_bytes.extend_from_slice(&make_pmt_packet(pmt_pid, video_pid, audio_pid));

    let demuxer = MpegTsDemuxer::from_bytes(Bytes::from(stream_bytes))
        .expect("demuxer initialization from PAT/PMT");

    assert_eq!(demuxer.streams().len(), 2);
    let streams = demuxer.streams();

    assert_eq!(streams[0].codec, CodecId::H264);
    assert_eq!(streams[0].time_base.den, 90_000);
    assert!(streams[0].is_video());

    assert_eq!(streams[1].codec, CodecId::Aac);
    assert_eq!(streams[1].time_base.den, 90_000);
    assert!(streams[1].is_audio());
}

#[test]
fn test_mpegts_pes_reassembly_multi_packet_and_pts() {
    let pmt_pid = 0x1000;
    let video_pid = 0x0100;
    let audio_pid = 0x0101;

    let mut stream_bytes = Vec::new();
    stream_bytes.extend_from_slice(&make_pat_packet(pmt_pid));
    stream_bytes.extend_from_slice(&make_pmt_packet(pmt_pid, video_pid, audio_pid));

    // NAL unit payload: Annex-B IDR frame (00 00 00 01 65 ...)
    let mut nalu_payload = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x00];
    nalu_payload.resize(300, 0xAA); // 300-byte frame spanning across 2 TS packets

    // Packet 1: PUSI = 1 with PTS = 90000 (1.0 second)
    let pts_target = 90_000;
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        0,
        pts_target,
        true, // random access indicator
        &nalu_payload[..150],
    ));

    // Packet 2: Continuation packet with PUSI = 0
    stream_bytes.extend_from_slice(&make_pes_packet_part2(
        video_pid,
        1,
        &nalu_payload[150..],
    ));

    // Frame 2: PUSI = 1 with PTS = 93000 to trigger finalization of Frame 1
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        2,
        93_000,
        false,
        &[0x00, 0x00, 0x00, 0x01, 0x41, 0x9A],
    ));

    let mut demuxer = MpegTsDemuxer::from_bytes(Bytes::from(stream_bytes))
        .expect("demuxer initialization");

    let pkt1 = demuxer.read_packet().expect("read packet 1").expect("first packet");
    assert_eq!(pkt1.stream_index, 0);
    assert_eq!(pkt1.pts, Some(pts_target));
    assert!(pkt1.is_keyframe);
    assert!(pkt1.data.len() >= 300);
    assert!(pkt1.data.starts_with(&[0x00, 0x00, 0x00, 0x01, 0x65]));

    let pkt2 = demuxer.read_packet().expect("read packet 2").expect("second packet");
    assert_eq!(pkt2.stream_index, 0);
    assert_eq!(pkt2.pts, Some(93_000));
}

#[test]
fn test_mpegts_m2ts_192_byte_cadence_detection() {
    let pmt_pid = 0x1000;
    let video_pid = 0x0100;
    let audio_pid = 0x0101;

    // Create 192-byte M2TS packets with 4-byte TP_extra_header prefix
    let pat = make_pat_packet(pmt_pid);
    let pmt = make_pmt_packet(pmt_pid, video_pid, audio_pid);

    let mut m2ts_bytes = Vec::new();
    // Packet 1: 4 bytes prefix + 188 bytes PAT
    m2ts_bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    m2ts_bytes.extend_from_slice(&pat);

    // Packet 2: 4 bytes prefix + 188 bytes PMT
    m2ts_bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x02]);
    m2ts_bytes.extend_from_slice(&pmt);

    let demuxer = MpegTsDemuxer::from_bytes(Bytes::from(m2ts_bytes))
        .expect("M2TS 192-byte cadence should be detected");

    assert_eq!(demuxer.streams().len(), 2);
}

#[test]
fn test_mpegts_seeking_to_keyframe() {
    let pmt_pid = 0x1000;
    let video_pid = 0x0100;
    let audio_pid = 0x0101;

    let mut stream_bytes = Vec::new();
    stream_bytes.extend_from_slice(&make_pat_packet(pmt_pid));
    stream_bytes.extend_from_slice(&make_pmt_packet(pmt_pid, video_pid, audio_pid));

    // Keyframe 1: PTS 0
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        0,
        0,
        true,
        &[0x00, 0x00, 0x00, 0x01, 0x65, 0x01],
    ));

    // Non-keyframe: PTS 3000
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        1,
        3_000,
        false,
        &[0x00, 0x00, 0x00, 0x01, 0x41, 0x02],
    ));

    // Keyframe 2: PTS 6000
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        2,
        6_000,
        true,
        &[0x00, 0x00, 0x00, 0x01, 0x65, 0x03],
    ));

    // End sentinel
    stream_bytes.extend_from_slice(&make_pes_packet_part1(
        video_pid,
        3,
        9_000,
        false,
        &[0x00, 0x00, 0x00, 0x01, 0x41, 0x04],
    ));

    let mut demuxer = MpegTsDemuxer::from_bytes(Bytes::from(stream_bytes))
        .expect("demuxer initialization");

    // Seek to PTS 6000
    demuxer.seek_to_keyframe(0, 6_000).expect("seek to keyframe");
    let pkt = demuxer.read_packet().expect("read packet").expect("packet after seek");
    assert_eq!(pkt.pts, Some(6_000));
    assert!(pkt.is_keyframe);
}

#[test]
fn test_mpegts_resilience_to_garbage_bytes() {
    let pmt_pid = 0x1000;
    let video_pid = 0x0100;
    let audio_pid = 0x0101;

    let mut stream_bytes = Vec::new();
    // 50 bytes of garbage prefix before sync byte
    stream_bytes.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF].repeat(12));
    stream_bytes.extend_from_slice(&[0x00, 0x00]);

    stream_bytes.extend_from_slice(&make_pat_packet(pmt_pid));
    stream_bytes.extend_from_slice(&make_pmt_packet(pmt_pid, video_pid, audio_pid));

    let demuxer = MpegTsDemuxer::from_bytes(Bytes::from(stream_bytes))
        .expect("resynchronization with prefix garbage");
    assert_eq!(demuxer.streams().len(), 2);
}
