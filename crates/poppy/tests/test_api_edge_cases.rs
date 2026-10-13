use poppy::{MediaStream, PoppyError};
use std::path::PathBuf;

fn make_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = (8 + payload.len()) as u32;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(&size.to_be_bytes());
    b.extend_from_slice(fourcc);
    b.extend_from_slice(payload);
    b
}

fn make_fullbox(fourcc: &[u8; 4], version: u8, flags: [u8; 3], payload: &[u8]) -> Vec<u8> {
    let size = (12 + payload.len()) as u32;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(&size.to_be_bytes());
    b.extend_from_slice(fourcc);
    b.push(version);
    b.extend_from_slice(&flags);
    b.extend_from_slice(payload);
    b
}

fn build_synthetic_trak(
    track_id: u32,
    enabled: bool,
    width: u32,
    height: u32,
    sample_sizes: &[usize],
    chunk_offset: u32,
) -> Vec<u8> {
    let avcc_payload = vec![
        1, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, 4, 0x67, 0x64, 0x00, 0x1F, 1, 0x00, 3, 0x68, 0xEB,
        0xE3,
    ];
    let avcc_box = make_box(b"avcC", &avcc_payload);

    let mut avc1_payload = vec![0u8; 78];
    avc1_payload[24..26].copy_from_slice(&(width as u16).to_be_bytes());
    avc1_payload[26..28].copy_from_slice(&(height as u16).to_be_bytes());
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

    let mut stsz_payload = Vec::new();
    stsz_payload.extend_from_slice(&0u32.to_be_bytes());
    stsz_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
    for &sz in sample_sizes {
        stsz_payload.extend_from_slice(&(sz as u32).to_be_bytes());
    }
    let stsz_box = make_fullbox(b"stsz", 0, [0, 0, 0], &stsz_payload);

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

    let mut stco_payload = Vec::new();
    if sample_sizes.is_empty() {
        stco_payload.extend_from_slice(&0u32.to_be_bytes());
    } else {
        stco_payload.extend_from_slice(&1u32.to_be_bytes());
        stco_payload.extend_from_slice(&chunk_offset.to_be_bytes());
    }
    let stco_box = make_fullbox(b"stco", 0, [0, 0, 0], &stco_payload);

    let mut stts_payload = Vec::new();
    if sample_sizes.is_empty() {
        stts_payload.extend_from_slice(&0u32.to_be_bytes());
    } else {
        stts_payload.extend_from_slice(&1u32.to_be_bytes());
        stts_payload.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
        stts_payload.extend_from_slice(&1000u32.to_be_bytes());
    }
    let stts_box = make_fullbox(b"stts", 0, [0, 0, 0], &stts_payload);

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
    mdhd_payload[8..12].copy_from_slice(&30000u32.to_be_bytes());
    mdhd_payload[12..16].copy_from_slice(&2000u32.to_be_bytes());
    let mdhd_box = make_fullbox(b"mdhd", 0, [0, 0, 0], &mdhd_payload);

    let mut mdia_payload = Vec::new();
    mdia_payload.extend_from_slice(&mdhd_box);
    mdia_payload.extend_from_slice(&hdlr_box);
    mdia_payload.extend_from_slice(&minf_box);
    let mdia_box = make_box(b"mdia", &mdia_payload);

    let flag_byte = if enabled { 0x01 } else { 0x00 };
    let mut tkhd_payload = vec![0u8; 80];
    tkhd_payload[8..12].copy_from_slice(&track_id.to_be_bytes());
    tkhd_payload[16..20].copy_from_slice(&2000u32.to_be_bytes());
    tkhd_payload[72..76].copy_from_slice(&(width << 16).to_be_bytes());
    tkhd_payload[76..80].copy_from_slice(&(height << 16).to_be_bytes());
    let tkhd_box = make_fullbox(b"tkhd", 0, [0, 0, flag_byte], &tkhd_payload);

    let mut trak_payload = Vec::new();
    trak_payload.extend_from_slice(&tkhd_box);
    trak_payload.extend_from_slice(&mdia_box);
    make_box(b"trak", &trak_payload)
}

#[test]
fn test_seek_seconds_non_finite_rejection() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !sample_path.exists() {
        return;
    }

    let mut stream = MediaStream::builder()
        .hardware_acceleration(false)
        .open_sync(&sample_path)
        .expect("open video stream");

    // 1. Sync seek_seconds non-finite assertions
    for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = stream
            .seek_seconds(non_finite)
            .expect_err("must reject non-finite seek");
        match err {
            PoppyError::InvalidSeek(msg) => {
                assert!(
                    msg.contains("non-finite seek target"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("Expected PoppyError::InvalidSeek, got: {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_async_seek_seconds_non_finite_rejection() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !sample_path.exists() {
        return;
    }

    let mut async_stream = MediaStream::builder()
        .hardware_acceleration(false)
        .open_async(&sample_path)
        .await
        .expect("open async video stream");

    for non_finite in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = async_stream
            .seek_seconds(non_finite)
            .await
            .expect_err("must reject non-finite async seek");
        match err {
            PoppyError::InvalidSeek(msg) => {
                assert!(
                    msg.contains("non-finite seek target"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("Expected PoppyError::InvalidSeek, got: {other:?}"),
        }
    }
}

#[test]
fn test_c_ffi_seek_seconds_non_finite_rejection() {
    let sample_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests/samples/test_h264.mp4");

    if !sample_path.exists() {
        return;
    }

    use std::ffi::CString;
    let c_path = CString::new(sample_path.to_str().unwrap()).unwrap();
    let mut stream_ptr: *mut poppy::ffi::PoppyStream = std::ptr::null_mut();
    let ret = unsafe { poppy::ffi::poppy_open(c_path.as_ptr(), &mut stream_ptr) };
    assert_eq!(ret, poppy::ffi::POPPY_OK);
    assert!(!stream_ptr.is_null());

    for non_finite in [
        std::os::raw::c_double::NAN,
        std::os::raw::c_double::INFINITY,
        std::os::raw::c_double::NEG_INFINITY,
    ] {
        let ret = unsafe { poppy::ffi::poppy_seek_seconds(stream_ptr, non_finite) };
        assert_eq!(ret, poppy::ffi::POPPY_ERR_INVALID_DATA);
    }

    unsafe {
        poppy::ffi::poppy_close(stream_ptr);
    }
}

#[test]
fn test_multi_video_track_selection_pipeline() {
    let temp_dir = std::env::temp_dir();
    let mp4_path = temp_dir.join("poppy_test_multi_track.mp4");

    let ftyp = make_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
    // Track 1: disabled, 720p
    let trak1 = build_synthetic_trak(1, false, 1280, 720, &[], 0);
    // Track 2: enabled, 1080p
    let trak2 = build_synthetic_trak(2, true, 1920, 1080, &[], 0);

    let mut moov_payload = Vec::new();
    moov_payload.extend_from_slice(&trak1);
    moov_payload.extend_from_slice(&trak2);
    let moov = make_box(b"moov", &moov_payload);
    let mdat = make_box(b"mdat", &[]);

    let mut file_data = Vec::new();
    file_data.extend_from_slice(&ftyp);
    file_data.extend_from_slice(&moov);
    file_data.extend_from_slice(&mdat);

    std::fs::write(&mp4_path, &file_data).expect("write synthetic multi-track MP4");

    // 1. Default selection selects Track 2 (enabled 1080p) over Track 1 (disabled 720p)
    let stream_default = MediaStream::builder()
        .hardware_acceleration(false)
        .open_sync(&mp4_path)
        .expect("open multi-track MP4");

    assert_eq!(
        stream_default.target_stream_index(),
        1,
        "Default pipeline selection must choose enabled primary 1080p track (index 1)"
    );

    // 2. Explicit track selection override
    let stream_override = MediaStream::builder()
        .hardware_acceleration(false)
        .stream_index(0)
        .open_sync(&mp4_path)
        .expect("open multi-track MP4 with index override");

    assert_eq!(
        stream_override.target_stream_index(),
        0,
        "Explicit builder index selection must choose track index 0"
    );

    let _ = std::fs::remove_file(&mp4_path);
}
