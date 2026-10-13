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
/// Internally synchronized with a Mutex so concurrent C calls are serialized
/// (PoppyStream is Send but not lock-free; see header docs).
pub struct PoppyStream {
    inner: std::sync::Mutex<MediaStream>,
}
/// PoppyStream is Send + Sync via the internal Mutex.
unsafe impl Send for PoppyStream {}
unsafe impl Sync for PoppyStream {}

/// Opaque handle representing a decoded video frame.
pub struct PoppyFrame {
    inner: DecodedFrame,
}

/// Opaque handle representing a decoded audio frame.
pub struct PoppyAudioFrame {
    inner: poppy_core::AudioFrame,
    pts_seconds: f64,
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

/// C-compatible representation of audio stream configuration.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PoppyAudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
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
        CodecId::Vorbis => 14,
        CodecId::Unknown(_) => 0,
    }
}

/// Returns the semver version string of Poppy (from Cargo.toml).
#[unsafe(no_mangle)]
pub extern "C" fn poppy_version() -> *const c_char {
    // CARGO_PKG_VERSION is baked at compile time, never drifts from Cargo.toml.
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
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
        let need = bytes.len() + 1;
        let copy_len = bytes.len().min(buffer_size.saturating_sub(1));
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, copy_len);
            *buffer.add(copy_len) = 0;
        }
        if need > buffer_size {
            POPPY_ERR_BUFFER_TOO_SMALL
        } else {
            POPPY_OK
        }
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
                let boxed = Box::new(PoppyStream {
                    inner: std::sync::Mutex::new(stream),
                });
                unsafe {
                    *out_stream = Box::into_raw(boxed);
                }
                POPPY_OK
            }
            Err(e) => {
                set_last_error(&e.to_string());
                unsafe {
                    *out_stream = std::ptr::null_mut();
                }
                POPPY_ERR_IO
            }
        }
    }));

    match res {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in poppy_open");
            unsafe {
                if !out_stream.is_null() {
                    *out_stream = std::ptr::null_mut();
                }
            }
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
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        unsafe {
            *out_count = guard.streams().len();
        }
        POPPY_OK
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_get_stream_count");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        let streams = guard.streams();
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
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_get_stream_info");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        unsafe {
            match guard.duration_seconds() {
                Some(dur) => {
                    *out_seconds = dur;
                }
                None => {
                    *out_seconds = 0.0;
                }
            }
        }
        POPPY_OK
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_get_duration_seconds");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        let codec_name = format!("{:?}", guard.codec());
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
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_get_codec_name");
            POPPY_ERR_PANIC
        }
    }
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
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => return POPPY_ERR_PANIC,
        };
        if guard.has_hardware_decoder() { 1 } else { 0 }
    }));
    match res {
        Ok(c) => c,
        Err(_) => POPPY_ERR_PANIC,
    }
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
        let s = unsafe { &*stream };
        let mut guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                unsafe {
                    *out_frame = std::ptr::null_mut();
                }
                return POPPY_ERR_PANIC;
            }
        };
        match guard.next_frame() {
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
                unsafe {
                    *out_frame = std::ptr::null_mut();
                }
                POPPY_ERR_HARDWARE
            }
        }
    }));

    match res {
        Ok(code) => code,
        Err(_) => {
            set_last_error("Panic occurred in poppy_read_frame");
            unsafe {
                if !out_frame.is_null() {
                    *out_frame = std::ptr::null_mut();
                }
            }
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
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let mut guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        match guard.seek(timestamp_ticks) {
            Ok(()) => POPPY_OK,
            Err(e) => {
                set_last_error(&e.to_string());
                POPPY_ERR_IO
            }
        }
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_seek");
            POPPY_ERR_PANIC
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
    if !seconds.is_finite() {
        set_last_error("seek_seconds: non-finite timestamp");
        return POPPY_ERR_INVALID_DATA;
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let mut guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        match guard.seek_seconds(seconds) {
            Ok(()) => POPPY_OK,
            Err(e) => {
                set_last_error(&e.to_string());
                POPPY_ERR_IO
            }
        }
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_seek_seconds");
            POPPY_ERR_PANIC
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        unsafe {
            *out_width = f.inner.width;
            *out_height = f.inner.height;
        }
        POPPY_OK
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_get_dimensions");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        unsafe {
            *out_pts = f.inner.pts.unwrap_or(-1);
        }
        POPPY_OK
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_get_pts");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                set_last_error("PoppyStream mutex poisoned");
                return POPPY_ERR_PANIC;
            }
        };
        unsafe {
            match f.inner.pts {
                Some(pts) => {
                    let tb = guard.time_base();
                    // Guard against zero denominator (corrupt timebase).
                    if tb.den == 0 {
                        *out_seconds = 0.0;
                    } else {
                        *out_seconds = tb.to_seconds(pts);
                    }
                    POPPY_OK
                }
                None => {
                    *out_seconds = 0.0;
                    POPPY_OK
                }
            }
        }
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_get_pts_seconds");
            POPPY_ERR_PANIC
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        unsafe {
            *out_is_concealed = f.inner.is_concealed;
        }
        POPPY_OK
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_is_concealed");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
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
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_get_pixel_format");
            POPPY_ERR_PANIC
        }
    }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        if let Some(ref dma) = f.inner.dmabuf {
            // Validate fd before dup to avoid EBADF panic across FFI.
            if dma.fd < 0 {
                set_last_error("Invalid DMA-BUF fd");
                return POPPY_ERR_INVALID_DATA;
            }
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
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_export_dmabuf");
            POPPY_ERR_PANIC
        }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
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
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_export_dxgi");
            POPPY_ERR_PANIC
        }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        if let Some(ref cv) = f.inner.cv_pixel_buffer {
            if cv.pixel_buffer_ptr == 0 {
                set_last_error("Invalid CVPixelBuffer pointer (simulated backend)");
                return POPPY_ERR_INVALID_DATA;
            }
            unsafe {
                *out_ptr = cv.pixel_buffer_ptr;
            }
            POPPY_OK
        } else {
            set_last_error("Frame does not contain a CVPixelBuffer reference");
            POPPY_ERR_UNSUPPORTED
        }
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_export_cvpixelbuffer");
            POPPY_ERR_PANIC
        }
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

    let res = catch_unwind(AssertUnwindSafe(|| {
        let f = unsafe { &*frame };
        if let Some(ref data) = f.inner.data {
            if buffer_size < data.len() {
                set_last_error("Buffer size is smaller than frame payload");
                return POPPY_ERR_BUFFER_TOO_SMALL;
            }
            // Defensive: data len could be 0? copy of 0 is fine.
            unsafe {
                if !data.is_empty() {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), out_buffer, data.len());
                }
                *out_bytes_copied = data.len();
            }
            POPPY_OK
        } else {
            set_last_error("No software staging buffer available for this GPU frame");
            POPPY_ERR_UNSUPPORTED
        }
    }));
    match res {
        Ok(c) => c,
        Err(_) => {
            set_last_error("Panic in poppy_frame_copy_data");
            POPPY_ERR_PANIC
        }
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

/// Returns 1 if the media stream contains an initialized audio stream, 0 otherwise.
/// Returns `POPPY_ERR_NULL_PTR` if `stream` is null.
///
/// # Safety
///
/// `stream` must point to a valid `PoppyStream` allocated by `poppy_open()`, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_has_audio(stream: *mut PoppyStream) -> c_int {
    if stream.is_null() {
        return POPPY_ERR_NULL_PTR;
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => return POPPY_ERR_PANIC,
        };
        if guard.has_audio() { 1 } else { 0 }
    }));
    res.unwrap_or(POPPY_ERR_PANIC)
}

/// Retrieves the audio configuration for the stream.
///
/// # Safety
/// `stream` and `config_out` must be non-null valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_get_config(
    stream: *mut PoppyStream,
    config_out: *mut PoppyAudioConfig,
) -> c_int {
    if stream.is_null() || config_out.is_null() {
        return POPPY_ERR_NULL_PTR;
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => return POPPY_ERR_PANIC,
        };
        match guard.audio_config() {
            Some(cfg) => {
                unsafe {
                    *config_out = PoppyAudioConfig {
                        sample_rate: cfg.sample_rate,
                        channels: cfg.channels,
                        bits_per_sample: cfg.bits_per_sample,
                    };
                }
                POPPY_OK
            }
            None => {
                set_last_error("Stream has no active audio track");
                POPPY_ERR_UNSUPPORTED
            }
        }
    }));
    res.unwrap_or(POPPY_ERR_PANIC)
}

/// Reads and decodes the next PCM audio frame from the stream.
///
/// Returns `POPPY_OK` on success, `POPPY_ERR_EOF` at end-of-stream.
///
/// # Safety
/// `stream` and `frame_out` must be non-null valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_read_audio_frame(
    stream: *mut PoppyStream,
    frame_out: *mut *mut PoppyAudioFrame,
) -> c_int {
    if stream.is_null() || frame_out.is_null() {
        return POPPY_ERR_NULL_PTR;
    }
    let res = catch_unwind(AssertUnwindSafe(|| {
        let s = unsafe { &*stream };
        let mut guard = match s.inner.lock() {
            Ok(g) => g,
            Err(_) => return POPPY_ERR_PANIC,
        };
        let audio_tb = guard.audio_time_base();
        match guard.next_audio_frame() {
            Ok(Some(frame)) => {
                let pts_seconds = match (frame.pts, audio_tb) {
                    (Some(ticks), Some(tb)) => tb.to_seconds(ticks).max(0.0),
                    _ => 0.0,
                };
                let boxed = Box::new(PoppyAudioFrame {
                    inner: frame,
                    pts_seconds,
                });
                unsafe {
                    *frame_out = Box::into_raw(boxed);
                }
                POPPY_OK
            }
            Ok(None) => POPPY_ERR_EOF,
            Err(e) => {
                set_last_error(&format!("Audio decode error: {e}"));
                POPPY_ERR_IO
            }
        }
    }));
    res.unwrap_or(POPPY_ERR_PANIC)
}

/// Frees a decoded audio frame.
///
/// # Safety
/// If non-null, `frame` must be a valid pointer returned by `poppy_read_audio_frame`.
/// After this call, `frame` must not be dereferenced.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_free_audio_frame(frame: *mut PoppyAudioFrame) -> c_int {
    if frame.is_null() {
        return POPPY_ERR_NULL_PTR;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(frame) });
    }));

    POPPY_OK
}

/// Returns the sample rate of the audio frame in Hz (e.g. 44100, 48000).
///
/// # Safety
///
/// `frame` must point to a valid `PoppyAudioFrame` allocated by `poppy_read_audio_frame()`, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_frame_get_sample_rate(frame: *const PoppyAudioFrame) -> u32 {
    if frame.is_null() {
        return 0;
    }
    let f = unsafe { &*frame };
    f.inner.sample_rate
}

/// Returns the channel count of the audio frame (e.g. 1 for mono, 2 for stereo).
///
/// # Safety
///
/// `frame` must point to a valid `PoppyAudioFrame` allocated by `poppy_read_audio_frame()`, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_frame_get_channels(frame: *const PoppyAudioFrame) -> u16 {
    if frame.is_null() {
        return 0;
    }
    let f = unsafe { &*frame };
    f.inner.channels
}

/// Returns the total number of f32 samples across all channels in the audio frame.
///
/// # Safety
///
/// `frame` must point to a valid `PoppyAudioFrame` allocated by `poppy_read_audio_frame()`, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_frame_get_sample_count(
    frame: *const PoppyAudioFrame,
) -> usize {
    if frame.is_null() {
        return 0;
    }
    let f = unsafe { &*frame };
    f.inner.samples.len()
}

/// Returns the presentation timestamp of the audio frame in seconds.
///
/// # Safety
///
/// `frame` must point to a valid `PoppyAudioFrame` allocated by `poppy_read_audio_frame()`, or be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_frame_get_pts_seconds(
    frame: *const PoppyAudioFrame,
) -> c_double {
    if frame.is_null() {
        return 0.0;
    }
    let f = unsafe { &*frame };
    f.pts_seconds
}

/// Copies interleaved f32 PCM samples into the caller-provided buffer.
///
/// # Safety
/// `frame` and `buffer` must be valid non-null pointers. `buffer` must point to at least `buffer_len` f32 values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poppy_audio_frame_get_samples(
    frame: *const PoppyAudioFrame,
    buffer: *mut f32,
    buffer_len: usize,
) -> c_int {
    if frame.is_null() || buffer.is_null() {
        return POPPY_ERR_NULL_PTR;
    }
    let f = unsafe { &*frame };
    if buffer_len < f.inner.samples.len() {
        set_last_error("Buffer length is smaller than audio sample count");
        return POPPY_ERR_BUFFER_TOO_SMALL;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(f.inner.samples.as_ptr(), buffer, f.inner.samples.len());
    }
    POPPY_OK
}
