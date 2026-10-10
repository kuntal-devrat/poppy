use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

use poppy::ffi::*;

#[test]
fn test_c_abi_version() {
    let ver_ptr = poppy_version();
    assert!(!ver_ptr.is_null());
    let ver = unsafe { CStr::from_ptr(ver_ptr) }.to_str().unwrap();
    assert_eq!(ver, "1.0.0");
}

#[test]
fn test_c_abi_null_and_error_handling() {
    unsafe {
        let res = poppy_open(std::ptr::null(), std::ptr::null_mut());
        assert_eq!(res, POPPY_ERR_NULL_PTR);

        let mut err_buf = [0 as c_char; 256];
        let err_res = poppy_last_error(err_buf.as_mut_ptr(), err_buf.len());
        assert_eq!(err_res, POPPY_OK);
        let err_msg = CStr::from_ptr(err_buf.as_ptr()).to_str().unwrap();
        assert!(err_msg.contains("Null pointer"));

        let bad_path = CString::new("nonexistent_path_to_video.mp4").unwrap();
        let mut stream = std::ptr::null_mut();
        let open_res = poppy_open(bad_path.as_ptr(), &mut stream);
        assert_ne!(open_res, POPPY_OK);
    }
}

#[test]
fn test_c_abi_full_pipeline_on_real_video() {
    let candidates = [
        "tests/samples/test_h264.mp4",
        "../../tests/samples/test_h264.mp4",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/samples/test_h264.mp4"
        ),
    ];
    let path = candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("tests/samples/test_h264.mp4");

    if !Path::new(path).exists() {
        println!("Sample not found, skipping C ABI test");
        return;
    }

    let c_path = CString::new(path).unwrap();
    let mut stream: *mut PoppyStream = std::ptr::null_mut();

    unsafe {
        let res = poppy_open(c_path.as_ptr(), &mut stream);
        assert_eq!(res, POPPY_OK, "poppy_open must succeed");
        assert!(!stream.is_null());

        // Stream count
        let mut count = 0usize;
        assert_eq!(poppy_get_stream_count(stream, &mut count), POPPY_OK);
        assert!(count > 0);

        // Stream info
        let mut info = PoppyStreamInfo {
            stream_index: 0,
            codec_id: 0,
            time_base_num: 0,
            time_base_den: 0,
            duration_ticks: 0,
        };
        assert_eq!(poppy_get_stream_info(stream, 0, &mut info), POPPY_OK);
        assert_eq!(info.stream_index, 0);
        assert!(info.time_base_den > 0);

        // Codec name
        let mut codec_buf = [0 as c_char; 64];
        assert_eq!(
            poppy_get_codec_name(stream, codec_buf.as_mut_ptr(), codec_buf.len()),
            POPPY_OK
        );
        let codec_str = CStr::from_ptr(codec_buf.as_ptr()).to_str().unwrap();
        assert!(codec_str.contains("H264"));

        // Duration
        let mut dur = 0.0f64;
        assert_eq!(poppy_get_duration_seconds(stream, &mut dur), POPPY_OK);
        assert!(dur > 0.0);

        let has_hw = poppy_has_hardware_decoder(stream);
        assert!(has_hw >= 0);
        if has_hw == 0 {
            println!(
                "Skipping C ABI frame decode assertions: no hardware decoder available on this system"
            );
            assert_eq!(poppy_close(stream), POPPY_OK);
            return;
        }

        // Read first frame
        let mut frame: *mut PoppyFrame = std::ptr::null_mut();
        let frame_res = poppy_read_frame(stream, &mut frame);
        assert_eq!(frame_res, POPPY_OK);
        assert!(!frame.is_null());

        // Frame dimensions
        let mut w = 0u32;
        let mut h = 0u32;
        assert_eq!(poppy_frame_get_dimensions(frame, &mut w, &mut h), POPPY_OK);
        assert!(w > 0);
        assert!(h > 0);

        // Frame PTS
        let mut pts = 0i64;
        assert_eq!(poppy_frame_get_pts(frame, &mut pts), POPPY_OK);
        assert!(pts >= 0);

        let mut pts_sec = 0.0f64;
        assert_eq!(
            poppy_frame_get_pts_seconds(frame, stream, &mut pts_sec),
            POPPY_OK
        );

        // Pixel format
        let mut pix_fmt = -1;
        assert_eq!(poppy_frame_get_pixel_format(frame, &mut pix_fmt), POPPY_OK);
        assert!(pix_fmt >= 0);

        // Zero-copy DMA-BUF export (on Linux VA-API)
        #[cfg(target_os = "linux")]
        {
            let mut dmabuf_fd = -1;
            let mut stride = 0u32;
            let mut modifier = 0u64;
            let dma_res =
                poppy_frame_export_dmabuf(frame, &mut dmabuf_fd, &mut stride, &mut modifier);
            if dma_res == POPPY_OK {
                assert!(dmabuf_fd >= 0);
                assert!(stride >= w);
                println!(
                    "✅ Exported DMA-BUF fd={} stride={} mod={:#x}",
                    dmabuf_fd, stride, modifier
                );
                libc::close(dmabuf_fd);
            }
        }

        // Seek
        assert_eq!(poppy_seek_seconds(stream, 0.0), POPPY_OK);

        // Free frame
        assert_eq!(poppy_free_frame(frame), POPPY_OK);

        // Close stream
        assert_eq!(poppy_close(stream), POPPY_OK);
    }
}
