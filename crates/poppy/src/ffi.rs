//! Production C Application Binary Interface (C ABI) and shared library (`libpoppy.so`)
//! bindings for Project Poppy.
//!
//! Exposes a memory-safe, null-checked, exception-safe FFI interface for C, C++, Python,
//! and other native environments requiring high-performance demuxing and hardware decoding.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_int};
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{CodecId, DecodedFrame, MediaStream, PixelFormat, StreamInfo};

// Result status codes
pub const POPPY_OK: c_int = 0;
pub const POPPY_ERR_NULL_PTR: c_int = -1;
pub const POPPY_ERR_IO: c_int = -2;
pub const POPPY_ERR_INVALID_DATA: c_int = -3;
pub const POPPY_ERR_HARDWARE: c_int = -4;
pub const POPPY_ERR_EOF: c_int = -5;
pub const POPPY_ERR_BUFFER_TOO_SMALL: c_int = -6;
pub const POPPY_ERR_UNSUPPORTED: c_int = -7;
pub const POPPY_ERR_PANIC: c_int = -8;

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_last_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = msg.to_string();
    });
}

/// Opaque handle representing an open Poppy media stream.
pub struct PoppyStream {
    inner: MediaStream,
}

/// Opaque handle representing a decoded video frame.
pub struct PoppyFrame {
    inner: DecodedFrame,
}

/// C-compatible representation of stream information.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PoppyStreamInfo {
    pub stream_index: usize,
    pub codec_id: c_int,
    pub time_base_num: u32,
    pub time_base_den: u32,
    pub duration_ticks: i64,
}

fn codec_to_c_int(c: CodecId) -> c_int {
    match c {
        CodecId::H264 => 1,
        CodecId::Hevc => 2,
        CodecId::Av1 => 3,
        CodecId::Vp9 => 4,
        CodecId::Aac => 10,
        CodecId::Opus => 11,
        CodecId::Mp3 => 12,
        CodecId::Flac => 13,
        CodecId::Unknown(_) => 0,
    }
}

/// Returns the semver version string of Poppy ("0.5.5").
#[unsafe(no_mangle)]
pub extern "C" fn poppy_version() -> *const c_char {
    static VERSION: &[u8] = b"0.5.5\0";
    VERSION.as_ptr() as *const c_char
}

/// Retrieves the most recent error message on the calling thread.
///
/// # Safety
/// If non-null, `buffer` must point to a writable byte array of at least `buffer_size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_last_error(buffer: *mut c_char, buffer_size: usize) -> c_int {
    if buffer.is_null() || buffer_size == 0 {
        return POPPY_ERR_NULL_PTR;
    }

    LAST_ERROR.with(|e| {
        let err = e.borrow();
        let bytes = err.as_bytes();
        let copy_len = bytes.len().min(buffer_size.saturating_sub(1));
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, copy_len);
            *buffer.add(copy_len) = 0;
        }
        POPPY_OK
    })
}

/// Opens a media stream from a file path.
///
/// # Safety
/// - `file_path` must be a valid, null-terminated C string pointer.
/// - `out_stream` must be a valid, writable pointer to a `*mut PoppyStream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_open(
    file_path: *const c_char,
    out_stream: *mut *mut PoppyStream,
) -> c_int {
    if file_path.is_null() || out_stream.is_null() {
        set_last_error("Null pointer argument to poppy_open");
        return POPPY_ERR_NULL_PTR;
    }

    let res = catch_unwind(AssertUnwindSafe(|| {
        let c_str = unsafe { CStr::from_ptr(file_path) };
        let path_str = match c_str.to_str() {
            Ok(s) => s,
            Err(e) => {
                set_last_error(&format!("Invalid UTF-8 path: {e}"));
                return POPPY_ERR_INVALID_DATA;
            }
        };

        match MediaStream::open(path_str) {
            Ok(stream) => {
                let boxed = Box::new(PoppyStream { inner: stream });
                unsafe {
                    *out_stream = Box::into_raw(boxed);
                }
                POPPY_OK
            }
            Err(e) => {
                set_last_error(&e.to_string());
                POPPY_ERR_IO
            }
        }
    }));

    match res {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in poppy_open");
            POPPY_ERR_PANIC
        }
    }
}

/// Closes and deallocates an open media stream.
///
/// # Safety
/// If non-null, `stream` must be a valid pointer returned by `poppy_open`.
/// After this call, `stream` must not be dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_close(stream: *mut PoppyStream) -> c_int {
    if stream.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(stream) });
    }));

    POPPY_OK
}

/// Returns the total number of streams in the media container.
///
/// # Safety
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `out_count` must be a writable pointer to a `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_get_stream_count(
    stream: *const PoppyStream,
    out_count: *mut usize,
) -> c_int {
    if stream.is_null() || out_count.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &*stream };
    unsafe {
        *out_count = s.inner.streams().len();
    }
    POPPY_OK
}

/// Retrieves metadata for a specific stream index.
///
/// # Safety
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `out_info` must be a writable pointer to a `PoppyStreamInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_get_stream_info(
    stream: *const PoppyStream,
    index: usize,
    out_info: *mut PoppyStreamInfo,
) -> c_int {
    if stream.is_null() || out_info.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &*stream };
    let streams = s.inner.streams();
    if index >= streams.len() {
        set_last_error(&format!("Stream index {index} out of bounds"));
        return POPPY_ERR_INVALID_DATA;
    }

    let info: &StreamInfo = &streams[index];
    unsafe {
        *out_info = PoppyStreamInfo {
            stream_index: info.index,
            codec_id: codec_to_c_int(info.codec),
            time_base_num: info.time_base.num,
            time_base_den: info.time_base.den,
            duration_ticks: info.duration_ticks.unwrap_or(0),
        };
    }

    POPPY_OK
}

/// Returns the total stream duration in seconds, if available.
///
/// # Safety
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `out_seconds` must be a writable pointer to a `c_double`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_get_duration_seconds(
    stream: *const PoppyStream,
    out_seconds: *mut c_double,
) -> c_int {
    if stream.is_null() || out_seconds.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &*stream };
    unsafe {
        match s.inner.duration_seconds() {
            Some(dur) => {
                *out_seconds = dur;
            }
            None => {
                *out_seconds = 0.0;
            }
        }
    }
    POPPY_OK
}

/// Copies the active codec name into `buffer`.
///
/// # Safety
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `buffer` must point to a writable memory region of at least `buffer_size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_get_codec_name(
    stream: *const PoppyStream,
    buffer: *mut c_char,
    buffer_size: usize,
) -> c_int {
    if stream.is_null() || buffer.is_null() || buffer_size == 0 {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &*stream };
    let codec_name = format!("{:?}", s.inner.codec());
    let c_name = match CString::new(codec_name) {
        Ok(c) => c,
        Err(_) => return POPPY_ERR_INVALID_DATA,
    };

    let bytes = c_name.as_bytes_with_nul();
    if bytes.len() > buffer_size {
        return POPPY_ERR_BUFFER_TOO_SMALL;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, bytes.len());
    }
    POPPY_OK
}

/// Returns whether the stream has an active hardware decoder bound.
/// Returns 1 if true, 0 if false, or a negative error code if null.
///
/// # Safety
/// `stream` must point to an open, valid `PoppyStream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_has_hardware_decoder(stream: *const PoppyStream) -> c_int {
    if stream.is_null() {
        return POPPY_ERR_NULL_PTR;
    }
    let s = unsafe { &*stream };
    if s.inner.has_hardware_decoder() { 1 } else { 0 }
}

/// Decodes the next hardware-accelerated video frame from the stream.
///
/// Returns `POPPY_OK` on success, or `POPPY_ERR_EOF` when the stream has ended.
///
/// # Safety
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `out_frame` must be a writable pointer to a `*mut PoppyFrame`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_read_frame(
    stream: *mut PoppyStream,
    out_frame: *mut *mut PoppyFrame,
) -> c_int {
    if stream.is_null() || out_frame.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &mut *stream };
        match s.inner.next_frame() {
            Ok(Some(frame)) => {
                let boxed = Box::new(PoppyFrame { inner: frame });
                unsafe {
                    *out_frame = Box::into_raw(boxed);
                }
                POPPY_OK
            }
            Ok(None) => {
                unsafe {
                    *out_frame = std::ptr::null_mut();
                }
                POPPY_ERR_EOF
            }
            Err(e) => {
                set_last_error(&e.to_string());
                POPPY_ERR_HARDWARE
            }
        }
    }));

    match res {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in poppy_read_frame");
            POPPY_ERR_PANIC
        }
    }
}

/// Seeks the stream to the nearest keyframe at or before `timestamp_ticks`.
///
/// # Safety
/// `stream` must point to an open, valid `PoppyStream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_seek(stream: *mut PoppyStream, timestamp_ticks: i64) -> c_int {
    if stream.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &mut *stream };
    match s.inner.seek(timestamp_ticks) {
        Ok(()) => POPPY_OK,
        Err(e) => {
            set_last_error(&e.to_string());
            POPPY_ERR_IO
        }
    }
}

/// Seeks the stream to the nearest keyframe at or before `seconds`.
///
/// # Safety
/// `stream` must point to an open, valid `PoppyStream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_seek_seconds(stream: *mut PoppyStream, seconds: c_double) -> c_int {
    if stream.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let s = unsafe { &mut *stream };
    match s.inner.seek_seconds(seconds) {
        Ok(()) => POPPY_OK,
        Err(e) => {
            set_last_error(&e.to_string());
            POPPY_ERR_IO
        }
    }
}

/// Retrieves frame dimensions (width and height in pixels).
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_width` and `out_height` must be writable pointers to `u32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_get_dimensions(
    frame: *const PoppyFrame,
    out_width: *mut u32,
    out_height: *mut u32,
) -> c_int {
    if frame.is_null() || out_width.is_null() || out_height.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    unsafe {
        *out_width = f.inner.width;
        *out_height = f.inner.height;
    }
    POPPY_OK
}

/// Retrieves the Presentation Timestamp (PTS) of the decoded frame in timebase ticks.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_pts` must be a writable pointer to an `i64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_get_pts(frame: *const PoppyFrame, out_pts: *mut i64) -> c_int {
    if frame.is_null() || out_pts.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    unsafe {
        *out_pts = f.inner.pts.unwrap_or(-1);
    }
    POPPY_OK
}

/// Calculates frame presentation timestamp in seconds relative to the given stream.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `stream` must point to an open, valid `PoppyStream`.
/// - `out_seconds` must be a writable pointer to a `c_double`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_get_pts_seconds(
    frame: *const PoppyFrame,
    stream: *const PoppyStream,
    out_seconds: *mut c_double,
) -> c_int {
    if frame.is_null() || stream.is_null() || out_seconds.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    let s = unsafe { &*stream };
    unsafe {
        match f.inner.pts {
            Some(pts) => {
                *out_seconds = s.inner.time_base().to_seconds(pts);
                POPPY_OK
            }
            None => {
                *out_seconds = 0.0;
                POPPY_OK
            }
        }
    }
}

/// Returns whether this frame was concealed due to a bitstream corruption or packet drop.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_is_concealed` must be a writable pointer to a `bool`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_is_concealed(
    frame: *const PoppyFrame,
    out_is_concealed: *mut bool,
) -> c_int {
    if frame.is_null() || out_is_concealed.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    unsafe {
        *out_is_concealed = f.inner.is_concealed;
    }
    POPPY_OK
}

/// Returns the pixel format of the frame (0 for NV12 8-bit, 1 for P010 10-bit HDR, 2 for YUV420P, 3 for RGBA32).
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_format` must be a writable pointer to a `c_int`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_get_pixel_format(
    frame: *const PoppyFrame,
    out_format: *mut c_int,
) -> c_int {
    if frame.is_null() || out_format.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    unsafe {
        *out_format = match f.inner.pixel_format {
            PixelFormat::Nv12 => 0,
            PixelFormat::P010 => 1,
            PixelFormat::Yuv420p => 2,
            PixelFormat::Rgba32 => 3,
        };
    }
    POPPY_OK
}

/// Exports a Linux DRM PRIME DMA-BUF file descriptor for direct Vulkan/OpenGL texture import.
///
/// On success, duplicates and returns an open file descriptor in `out_fd`.
/// The caller is responsible for closing the returned file descriptor using `close()`.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_fd`, `out_stride`, and `out_modifier` must be writable pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_export_dmabuf(
    frame: *const PoppyFrame,
    out_fd: *mut c_int,
    out_stride: *mut u32,
    out_modifier: *mut u64,
) -> c_int {
    if frame.is_null() || out_fd.is_null() || out_stride.is_null() || out_modifier.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    if let Some(ref dma) = f.inner.dmabuf {
        let dup_fd = unsafe { libc::dup(dma.fd) };
        if dup_fd < 0 {
            set_last_error("Failed to dup DMA-BUF file descriptor");
            return POPPY_ERR_IO;
        }
        unsafe {
            *out_fd = dup_fd;
            *out_stride = dma.pitches.first().copied().unwrap_or(0);
            *out_modifier = dma.drm_modifier;
        }
        POPPY_OK
    } else {
        set_last_error("Frame does not contain an active DMA-BUF surface");
        POPPY_ERR_UNSUPPORTED
    }
}

/// Exports a Windows DXGI shared handle for Direct3D 12 texture binding.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_handle` must be a writable pointer to an `isize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_export_dxgi(
    frame: *const PoppyFrame,
    out_handle: *mut isize,
) -> c_int {
    if frame.is_null() || out_handle.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    if let Some(ref dxgi) = f.inner.dxgi_handle {
        unsafe {
            *out_handle = dxgi.handle;
        }
        POPPY_OK
    } else {
        set_last_error("Frame does not contain a DXGI handle");
        POPPY_ERR_UNSUPPORTED
    }
}

/// Exports a macOS CVPixelBuffer pointer for Metal texture binding.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_ptr` must be a writable pointer to a `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_export_cvpixelbuffer(
    frame: *const PoppyFrame,
    out_ptr: *mut usize,
) -> c_int {
    if frame.is_null() || out_ptr.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    if let Some(ref cv) = f.inner.cv_pixel_buffer {
        unsafe {
            *out_ptr = cv.pixel_buffer_ptr;
        }
        POPPY_OK
    } else {
        set_last_error("Frame does not contain a CVPixelBuffer reference");
        POPPY_ERR_UNSUPPORTED
    }
}

/// Copies raw frame data if software CPU fallback data is available.
///
/// # Safety
/// - `frame` must point to a valid `PoppyFrame`.
/// - `out_buffer` must point to a writable byte array of at least `buffer_size` bytes.
/// - `out_bytes_copied` must be a writable pointer to a `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_frame_copy_data(
    frame: *const PoppyFrame,
    out_buffer: *mut u8,
    buffer_size: usize,
    out_bytes_copied: *mut usize,
) -> c_int {
    if frame.is_null() || out_buffer.is_null() || out_bytes_copied.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let f = unsafe { &*frame };
    if let Some(ref data) = f.inner.data {
        if buffer_size < data.len() {
            set_last_error("Buffer size is smaller than frame payload");
            return POPPY_ERR_BUFFER_TOO_SMALL;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), out_buffer, data.len());
            *out_bytes_copied = data.len();
        }
        POPPY_OK
    } else {
        set_last_error("No software staging buffer available for this GPU frame");
        POPPY_ERR_UNSUPPORTED
    }
}

/// Frees a decoded video frame.
///
/// # Safety
/// If non-null, `frame` must be a valid pointer returned by `poppy_read_frame`.
/// After this call, `frame` must not be dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_free_frame(frame: *mut PoppyFrame) -> c_int {
    if frame.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(frame) });
    }));

    POPPY_OK
}
