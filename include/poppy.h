/**
 * @file poppy.h
 * @brief Project Poppy C Application Binary Interface (C ABI)
 * 
 * High-performance, zero-copy video demuxing and hardware decoding library.
 * Copyright (c) 2026 Project Poppy Contributors.
 * Licensed under MIT OR Apache-2.0.
 */

#ifndef POPPY_H
#define POPPY_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

/* ========================================================================= */
/* Status and Error Codes                                                    */
/* ========================================================================= */

#define POPPY_OK                   0
#define POPPY_ERR_NULL_PTR        -1
#define POPPY_ERR_IO              -2
#define POPPY_ERR_INVALID_DATA    -3
#define POPPY_ERR_HARDWARE        -4
#define POPPY_ERR_EOF             -5
#define POPPY_ERR_BUFFER_TOO_SMALL -6
#define POPPY_ERR_UNSUPPORTED     -7
#define POPPY_ERR_PANIC           -8

/* ========================================================================= */
/* Codec Identification Constants                                            */
/* ========================================================================= */

#define POPPY_CODEC_UNKNOWN        0
#define POPPY_CODEC_H264           1
#define POPPY_CODEC_HEVC           2
#define POPPY_CODEC_AV1            3
#define POPPY_CODEC_VP9            4
#define POPPY_CODEC_AAC           10
#define POPPY_CODEC_OPUS          11
#define POPPY_CODEC_MP3           12
#define POPPY_CODEC_FLAC          13
#define POPPY_CODEC_VORBIS        14

/* ========================================================================= */
/* Pixel Format Enums                                                        */
/* ========================================================================= */

typedef enum {
    POPPY_PIXEL_FORMAT_NV12    = 0,
    POPPY_PIXEL_FORMAT_P010    = 1,
    POPPY_PIXEL_FORMAT_YUV420P = 2,
    POPPY_PIXEL_FORMAT_RGBA32  = 3
} PoppyPixelFormat;

/* ========================================================================= */
/* Opaque Types                                                              */
/* ========================================================================= */

typedef struct PoppyStream     PoppyStream;
typedef struct PoppyFrame      PoppyFrame;
typedef struct PoppyAudioFrame PoppyAudioFrame;

/* ========================================================================= */
/* Metadata Structures                                                       */
/* ========================================================================= */

typedef struct {
    size_t   stream_index;
    int32_t  codec_id;
    uint32_t time_base_num;
    uint32_t time_base_den;
    int64_t  duration_ticks;
} PoppyStreamInfo;

typedef struct {
    uint32_t sample_rate;
    uint16_t channels;
    uint16_t bits_per_sample;
} PoppyAudioConfig;

/* ========================================================================= */
/* Library Lifecycle and Information                                         */
/* ========================================================================= */

/**
 * @brief Returns the Poppy library semver version string ("0.6.0").
 */
const char* poppy_version(void);

/**
 * @brief Copies the last error message on the calling thread into buffer.
 */
int32_t poppy_last_error(char* buffer, size_t buffer_size);

/* ========================================================================= */
/* Stream Lifecycle and Demuxing                                             */
/* ========================================================================= */

/**
 * @brief Opens a media file and initializes hardware decode context.
 *
 * Thread-safe: the returned handle is internally synchronized; concurrent
 * poppy_read_frame/popppy_seek calls are serialized via a mutex.
 * On error, *out_stream is set to NULL.
 *
 * @note No audio C API is currently exposed (Rust facade has AudioDecoder/Sink;
 * C users get video-only). Audio support via C is planned.
 *
 * @param file_path Null-terminated UTF-8 file path.
 * @param out_stream Output pointer receiving the stream handle.
 * @return POPPY_OK on success, or negative error code.
 */
int32_t poppy_open(const char* file_path, PoppyStream** out_stream);

/**
 * @brief Closes and deallocates an open media stream.
 */
int32_t poppy_close(PoppyStream* stream);

/**
 * @brief Returns the total count of media tracks in the container.
 */
int32_t poppy_get_stream_count(const PoppyStream* stream, size_t* out_count);

/**
 * @brief Retrieves metadata for a specific track index.
 */
int32_t poppy_get_stream_info(const PoppyStream* stream, size_t index, PoppyStreamInfo* out_info);

/**
 * @brief Retrieves total stream duration in seconds.
 */
int32_t poppy_get_duration_seconds(const PoppyStream* stream, double* out_seconds);

/**
 * @brief Copies the active video track's codec name into buffer.
 */
int32_t poppy_get_codec_name(const PoppyStream* stream, char* buffer, size_t buffer_size);

/**
 * @brief Returns whether the stream has an active hardware decoder bound.
 *
 * @param stream Valid PoppyStream handle.
 * @return 1 if hardware accelerated, 0 if demuxing-only/no GPU, or negative error code.
 */
int32_t poppy_has_hardware_decoder(const PoppyStream* stream);

/* ========================================================================= */
/* Decoding and Seeking                                                      */
/* ========================================================================= */

/**
 * @brief Decodes the next video frame from the stream.
 *
 * @param stream Active stream handle.
 * @param out_frame Output pointer receiving the newly decoded frame.
 * @return POPPY_OK on success, POPPY_ERR_EOF at end of stream, or negative error.
 */
int32_t poppy_read_frame(PoppyStream* stream, PoppyFrame** out_frame);

/**
 * @brief Seeks to the nearest keyframe at or before timestamp_ticks.
 */
int32_t poppy_seek(PoppyStream* stream, int64_t timestamp_ticks);

/**
 * @brief Seeks to the nearest keyframe at or before seconds.
 */
int32_t poppy_seek_seconds(PoppyStream* stream, double seconds);

/* ========================================================================= */
/* Frame Inspection and GPU Zero-Copy Export                                 */
/* ========================================================================= */

/**
 * @brief Retrieves frame width and height in pixels.
 */
int32_t poppy_frame_get_dimensions(const PoppyFrame* frame, uint32_t* out_width, uint32_t* out_height);

/**
 * @brief Retrieves Presentation Timestamp (PTS) in timebase ticks.
 */
int32_t poppy_frame_get_pts(const PoppyFrame* frame, int64_t* out_pts);

/**
 * @brief Computes Presentation Timestamp (PTS) in seconds.
 */
int32_t poppy_frame_get_pts_seconds(const PoppyFrame* frame, const PoppyStream* stream, double* out_seconds);

/**
 * @brief Returns true if the frame was concealed due to a bitstream error.
 *
 * @note Thread-safe: handles are internally synchronized with a mutex;
 * concurrent calls from multiple threads are serialized.
 */
int32_t poppy_frame_is_concealed(const PoppyFrame* frame, bool* out_is_concealed);

/**
 * @brief Retrieves the pixel format (0=NV12, 1=P010, 2=YUV420P, 3=RGBA32).
 */
int32_t poppy_frame_get_pixel_format(const PoppyFrame* frame, int32_t* out_format);

/**
 * @brief Exports a Linux DRM PRIME DMA-BUF file descriptor for direct Vulkan/OpenGL texture import.
 *
 * Note: Caller must call close() on the returned file descriptor when done.
 */
int32_t poppy_frame_export_dmabuf(
    const PoppyFrame* frame,
    int32_t* out_fd,
    uint32_t* out_stride,
    uint64_t* out_modifier
);

/**
 * @brief Exports a Windows DXGI shared handle for Direct3D 12 texture binding.
 */
int32_t poppy_frame_export_dxgi(const PoppyFrame* frame, intptr_t* out_handle);

/**
 * @brief Exports a macOS CVPixelBuffer pointer for Metal texture binding.
 */
int32_t poppy_frame_export_cvpixelbuffer(const PoppyFrame* frame, uintptr_t* out_ptr);

/**
 * @brief Copies raw frame payload if a software fallback buffer is present.
 */
int32_t poppy_frame_copy_data(
    const PoppyFrame* frame,
    uint8_t* out_buffer,
    size_t buffer_size,
    size_t* out_bytes_copied
);

/**
 * @brief Frees a decoded frame and releases its hardware surface back to the pool.
 */
int32_t poppy_free_frame(PoppyFrame* frame);

/* ========================================================================= */
/* Audio Operations                                                          */
/* ========================================================================= */

/**
 * @brief Returns 1 if the stream has an audio track, 0 if not, or negative error.
 */
int32_t poppy_has_audio(PoppyStream* stream);

/**
 * @brief Retrieves the audio stream configuration.
 */
int32_t poppy_audio_get_config(PoppyStream* stream, PoppyAudioConfig* config_out);

/**
 * @brief Reads and decodes the next PCM audio frame from the stream.
 *
 * Returns POPPY_OK on success, POPPY_ERR_EOF at end-of-stream.
 */
int32_t poppy_read_audio_frame(PoppyStream* stream, PoppyAudioFrame** frame_out);

/**
 * @brief Frees a decoded audio frame.
 */
int32_t poppy_free_audio_frame(PoppyAudioFrame* frame);

/**
 * @brief Returns the sample rate of the audio frame in Hz (e.g. 44100, 48000).
 */
uint32_t poppy_audio_frame_get_sample_rate(const PoppyAudioFrame* frame);

/**
 * @brief Returns the channel count of the audio frame (e.g. 1 for mono, 2 for stereo).
 */
uint16_t poppy_audio_frame_get_channels(const PoppyAudioFrame* frame);

/**
 * @brief Returns the total count of f32 samples across all channels.
 */
size_t poppy_audio_frame_get_sample_count(const PoppyAudioFrame* frame);

/**
 * @brief Returns the presentation timestamp of the audio frame in seconds.
 */
double poppy_audio_frame_get_pts_seconds(const PoppyAudioFrame* frame);

/**
 * @brief Copies interleaved normalized f32 PCM samples into a caller-provided buffer.
 *
 * Returns POPPY_ERR_BUFFER_TOO_SMALL if buffer_len < sample count.
 */
int32_t poppy_audio_frame_get_samples(
    const PoppyAudioFrame* frame,
    float* buffer,
    size_t buffer_len
);

#ifdef __cplusplus
}
#endif

#endif /* POPPY_H */
