use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

use poppy::ffi::*;

#[test]
fn test_c_abi_version() {
    let ver_ptr = poppy_version();
    assert!(!ver_ptr.is_null());
    let ver = unsafe { CStr::from_ptr(ver_ptr) }.to_str().unwrap();
    assert_eq!(ver, "0.6.0");
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
        let mut exact_frame: *mut PoppyFrame = std::ptr::null_mut();
        assert_eq!(poppy_seek_exact(stream, 0.5, &mut exact_frame), POPPY_OK);
        if !exact_frame.is_null() {
            assert_eq!(poppy_free_frame(exact_frame), POPPY_OK);
        }

        // Free frame
        assert_eq!(poppy_free_frame(frame), POPPY_OK);

        // Close stream
        assert_eq!(poppy_close(stream), POPPY_OK);
    }
}

#[test]
fn test_c_abi_concurrent_access() {
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
        return;
    }

    let c_path = CString::new(path).unwrap();
    let mut stream: *mut PoppyStream = std::ptr::null_mut();
    unsafe {
        assert_eq!(poppy_open(c_path.as_ptr(), &mut stream), POPPY_OK);
        assert!(!stream.is_null());
    }

    let stream_addr = stream as usize;
    let mut handles = Vec::new();

    for thread_idx in 0..8 {
        let handle = std::thread::spawn(move || {
            let s = stream_addr as *mut PoppyStream;
            for i in 0..40 {
                unsafe {
                    let mut count = 0usize;
                    assert_eq!(poppy_get_stream_count(s, &mut count), POPPY_OK);
                    assert!(count > 0);

                    let mut dur = 0.0f64;
                    assert_eq!(poppy_get_duration_seconds(s, &mut dur), POPPY_OK);
                    assert!(dur > 0.0);

                    let mut info = PoppyStreamInfo {
                        stream_index: 0,
                        codec_id: 0,
                        time_base_num: 0,
                        time_base_den: 0,
                        duration_ticks: 0,
                    };
                    assert_eq!(poppy_get_stream_info(s, 0, &mut info), POPPY_OK);

                    let mut codec_buf = [0 as c_char; 32];
                    assert_eq!(
                        poppy_get_codec_name(s, codec_buf.as_mut_ptr(), codec_buf.len()),
                        POPPY_OK
                    );

                    if (i + thread_idx) % 8 == 0 {
                        let _ = poppy_seek_seconds(s, 0.0);
                    }

                    let mut frame: *mut PoppyFrame = std::ptr::null_mut();
                    let res = poppy_read_frame(s, &mut frame);
                    if res == POPPY_OK && !frame.is_null() {
                        let mut w = 0u32;
                        let mut h = 0u32;
                        assert_eq!(poppy_frame_get_dimensions(frame, &mut w, &mut h), POPPY_OK);
                        let mut pts = 0i64;
                        assert_eq!(poppy_frame_get_pts(frame, &mut pts), POPPY_OK);
                        assert_eq!(poppy_free_frame(frame), POPPY_OK);
                    }
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join()
            .expect("Thread panicked during concurrent FFI access");
    }

    unsafe {
        assert_eq!(poppy_close(stream), POPPY_OK);
    }
}

#[test]
fn test_c_abi_audio_pipeline_on_real_media() {
    let samples = [
        "tests/samples/test_av_aac.mp4",
        "tests/samples/test_av_opus.webm",
        "../../tests/samples/test_av_aac.mp4",
        "../../tests/samples/test_av_opus.webm",
    ];

    for s_path in &samples {
        if !Path::new(s_path).exists() {
            continue;
        }

        let c_path = CString::new(*s_path).unwrap();
        let mut stream: *mut PoppyStream = std::ptr::null_mut();

        unsafe {
            let res = poppy_open(c_path.as_ptr(), &mut stream);
            assert_eq!(res, POPPY_OK, "poppy_open must succeed for {s_path}");
            assert!(!stream.is_null());

            let has_audio = poppy_has_audio(stream);
            assert_eq!(has_audio, 1, "Expected audio track in {s_path}");

            let mut config = PoppyAudioConfig {
                sample_rate: 0,
                channels: 0,
                bits_per_sample: 0,
            };
            let cfg_res = poppy_audio_get_config(stream, &mut config);
            assert_eq!(cfg_res, POPPY_OK);
            assert!(config.sample_rate >= 8000);
            assert!(config.channels >= 1);

            let mut audio_frame: *mut PoppyAudioFrame = std::ptr::null_mut();
            let mut frames_read = 0;

            while frames_read < 10 {
                let read_res = poppy_read_audio_frame(stream, &mut audio_frame);
                if read_res == POPPY_ERR_EOF {
                    break;
                }
                assert_eq!(read_res, POPPY_OK, "read audio frame should succeed");
                assert!(!audio_frame.is_null());

                let rate = poppy_audio_frame_get_sample_rate(audio_frame);
                let channels = poppy_audio_frame_get_channels(audio_frame);
                let sample_count = poppy_audio_frame_get_sample_count(audio_frame);
                let pts_sec = poppy_audio_frame_get_pts_seconds(audio_frame);

                assert_eq!(rate, config.sample_rate);
                assert_eq!(channels, config.channels);
                assert!(sample_count > 0);
                assert!(pts_sec >= 0.0);

                let mut pcm_buf = vec![0.0f32; sample_count];
                let copy_res =
                    poppy_audio_frame_get_samples(audio_frame, pcm_buf.as_mut_ptr(), pcm_buf.len());
                assert_eq!(copy_res, POPPY_OK);

                // Samples must be within normalized [-1.0, 1.0] range
                for &sample in &pcm_buf {
                    assert!(
                        (-1.5..=1.5).contains(&sample),
                        "Sample out of range: {sample}"
                    );
                }

                assert_eq!(poppy_free_audio_frame(audio_frame), POPPY_OK);
                frames_read += 1;
            }

            assert!(frames_read > 0, "Should have read at least 1 audio frame");
            assert_eq!(poppy_close(stream), POPPY_OK);
        }
    }
}

#[test]
fn test_c_abi_header_parity() {
    let header_paths = [
        "include/poppy.h",
        "../../include/poppy.h",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../include/poppy.h"),
    ];

    let header_path = header_paths
        .iter()
        .find(|p| Path::new(p).exists())
        .expect("include/poppy.h must exist");

    let header_content = std::fs::read_to_string(header_path).expect("read poppy.h");

    // All public C functions in poppy::ffi
    let exported_functions = [
        "poppy_version",
        "poppy_last_error",
        "poppy_open",
        "poppy_close",
        "poppy_get_stream_count",
        "poppy_get_stream_info",
        "poppy_get_duration_seconds",
        "poppy_get_codec_name",
        "poppy_has_hardware_decoder",
        "poppy_read_frame",
        "poppy_seek",
        "poppy_seek_seconds",
        "poppy_seek_exact",
        "poppy_frame_get_dimensions",
        "poppy_frame_get_pts",
        "poppy_frame_get_pts_seconds",
        "poppy_frame_is_concealed",
        "poppy_frame_get_pixel_format",
        "poppy_frame_export_dmabuf",
        "poppy_frame_export_dxgi",
        "poppy_frame_export_cvpixelbuffer",
        "poppy_frame_copy_data",
        "poppy_free_frame",
        "poppy_has_audio",
        "poppy_audio_get_config",
        "poppy_read_audio_frame",
        "poppy_free_audio_frame",
        "poppy_audio_frame_get_sample_rate",
        "poppy_audio_frame_get_channels",
        "poppy_audio_frame_get_sample_count",
        "poppy_audio_frame_get_pts_seconds",
        "poppy_audio_frame_get_samples",
    ];

    for func in &exported_functions {
        assert!(
            header_content.contains(func),
            "C header poppy.h is missing declaration for exported function: {func}"
        );
    }
}
