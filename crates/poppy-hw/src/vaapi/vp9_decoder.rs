//! Hardware VP9 video decoder using Linux VA-API.
//!
//! Provides zero-copy hardware acceleration for VP9 profiles 0, 1, 2, and 3
//! (including 8-bit NV12 and 10-bit P010) with automatic superframe unpacking
//! and 8-slot reference frame pool management.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{DecodedFrame, HwError};
use poppy_core::codec::VpcCRecord;
use poppy_core::{InterlaceMode, Packet, PixelFormat};
use poppy_vp9::{
    Vp9FrameHeader, Vp9FrameType, calculate_segment_filter_levels, calculate_segment_qmul,
    parse_superframe_frames,
};

use super::device::VaDevice;
use super::ffi::*;
use super::surface::SurfacePool;
use crate::HwDecoder;

/// Hardware-accelerated VP9 decoder implementation backed by VA-API.
pub struct VaapiVp9Decoder {
    device: VaDevice,
    pool: Option<SurfacePool>,
    ref_frame_map: [VASurfaceID; 8],
    ref_dims: [(u32, u32); 8],
    output_queue: VecDeque<DecodedFrame>,
    cur_surface_idx: usize,
    is_10bit: bool,
}

impl VaapiVp9Decoder {
    /// Initializes a new hardware VP9 decoder using an auto-detected DRM device.
    pub fn new(is_10bit: bool) -> Result<Self, HwError> {
        let device = VaDevice::open_auto_vp9(is_10bit)?;
        Ok(Self {
            device,
            pool: None,
            ref_frame_map: [VA_INVALID_ID; 8],
            ref_dims: [(0, 0); 8],
            output_queue: VecDeque::new(),
            cur_surface_idx: 0,
            is_10bit,
        })
    }

    /// Feeds container extradata (e.g. `vpcC` from MP4 or Matroska CodecPrivate) into the decoder.
    pub fn set_extradata(&mut self, extradata: &[u8]) -> Result<(), HwError> {
        if extradata.is_empty() {
            return Ok(());
        }

        if let Ok(vpcc) = VpcCRecord::parse(extradata) {
            self.is_10bit = vpcc.is_10bit();
        }

        Ok(())
    }

    /// Access the underlying SurfacePool for zero-copy DMA-BUF export.
    pub fn surface_pool(&self) -> Option<&SurfacePool> {
        self.pool.as_ref()
    }

    fn init_surface_pool(
        &mut self,
        width: u32,
        height: u32,
        is_10bit: bool,
    ) -> Result<(), HwError> {
        let rt_format = if is_10bit {
            VA_RT_FORMAT_YUV420_10
        } else {
            VA_RT_FORMAT_YUV420
        };

        let need_recreate = match &self.pool {
            Some(p) => p.width != width || p.height != height,
            None => true,
        };

        if need_recreate {
            let pool = SurfacePool::new_with_format(
                Arc::clone(&self.device.va),
                self.device.display,
                self.device.config_id,
                width,
                height,
                16,
                rt_format,
            )?;
            self.pool = Some(pool);
            self.ref_frame_map = [VA_INVALID_ID; 8];
            self.ref_dims = [(width, height); 8];
        }

        self.is_10bit = is_10bit;
        Ok(())
    }

    fn decode_sub_frame(
        &mut self,
        header: &Vp9FrameHeader,
        sub_frame_bytes: &[u8],
        pts: Option<i64>,
    ) -> Result<(), HwError> {
        if header.show_existing_frame {
            let idx = header.frame_to_show_map_idx as usize;
            if idx < 8 && self.ref_frame_map[idx] != VA_INVALID_ID {
                let surf = self.ref_frame_map[idx];
                let (w, h) = self.ref_dims[idx];
                let dmabuf = self
                    .pool
                    .as_ref()
                    .and_then(|p| p.export_dmabuf_descriptor(surf).ok());
                let data = if dmabuf.is_none() {
                    self.pool
                        .as_ref()
                        .and_then(|p| p.read_surface_data(surf, w, h).ok())
                } else {
                    None
                }; // lazy zero-copy

                self.output_queue.push_back(DecodedFrame {
                    pts,
                    width: w,
                    height: h,
                    pixel_format: if self.is_10bit {
                        PixelFormat::P010
                    } else {
                        PixelFormat::Nv12
                    },
                    surface_id: surf,
                    data,
                    interlace_mode: InterlaceMode::Progressive,
                    dmabuf,
                    dxgi_handle: None,
                    cv_pixel_buffer: None,
                    is_concealed: false,
                });
            }
            return Ok(());
        }

        let is_10bit = header.bit_depth >= 10 || header.profile >= 2;
        self.init_surface_pool(header.width, header.height, is_10bit)?;

        let pool = match &self.pool {
            Some(p) => p,
            None => return Ok(()),
        };

        // Select free surface not currently held in the reference map
        let target_surface = pool
            .surfaces
            .iter()
            .copied()
            .find(|&surf| !self.ref_frame_map.contains(&surf))
            .unwrap_or(pool.surfaces[self.cur_surface_idx]);
        self.cur_surface_idx = (self.cur_surface_idx + 1) % pool.surfaces.len();

        let mut pic_param: VADecPictureParameterBufferVP9 = unsafe { std::mem::zeroed() };
        pic_param.frame_width = header.width as u16;
        pic_param.frame_height = header.height as u16;
        pic_param.reference_frames = self.ref_frame_map;

        let pic_fields = (header.subsampling_x as u32 & 1)
            | ((header.subsampling_y as u32 & 1) << 1)
            | (((header.frame_type == Vp9FrameType::InterFrame) as u32) << 2)
            | ((header.show_frame as u32) << 3)
            | ((header.error_resilient_mode as u32) << 4)
            | ((header.intra_only as u32) << 5)
            | ((header.allow_high_precision_mv as u32) << 6)
            | ((header.vaapi_mcomp_filter_type() & 7) << 7)
            | ((header.frame_parallel_decoding_mode as u32) << 10)
            | ((header.reset_frame_context as u32 & 3) << 11)
            | ((header.refresh_frame_context as u32) << 13)
            | ((header.frame_context_idx as u32 & 3) << 14)
            | ((header.segmentation.enabled as u32) << 16)
            | ((header.segmentation.temporal_update as u32) << 17)
            | ((header.segmentation.update_map as u32) << 18)
            | ((header.ref_frame_indices[0] as u32 & 7) << 19)
            | ((header.ref_frame_sign_bias[0] as u32) << 22)
            | ((header.ref_frame_indices[1] as u32 & 7) << 23)
            | ((header.ref_frame_sign_bias[1] as u32) << 26)
            | ((header.ref_frame_indices[2] as u32 & 7) << 27)
            | ((header.ref_frame_sign_bias[2] as u32) << 30)
            | ((header.quant.is_lossless() as u32) << 31);

        pic_param.pic_fields = pic_fields;
        pic_param.filter_level = header.loop_filter.filter_level;
        pic_param.sharpness_level = header.loop_filter.sharpness_level;
        pic_param.log2_tile_rows = header.tile_rows_log2;
        pic_param.log2_tile_columns = header.tile_cols_log2;
        pic_param.frame_header_length_in_bytes = header.uncompressed_header_size as u8;
        pic_param.first_partition_size = header.first_partition_size;
        pic_param.mb_segment_tree_probs = header.segmentation.tree_probs;
        pic_param.segment_pred_probs = if header.segmentation.temporal_update {
            header.segmentation.pred_probs
        } else {
            [255; 3]
        };
        pic_param.profile = header.profile;
        pic_param.bit_depth = header.bit_depth;

        let mut slice_param: VASliceParameterBufferVP9 = unsafe { std::mem::zeroed() };
        slice_param.slice_data_size = sub_frame_bytes.len() as u32;
        slice_param.slice_data_offset = 0;
        slice_param.slice_data_flag = 0;

        for seg in 0..8 {
            let seg_q_val =
                if header.segmentation.enabled && header.segmentation.features[seg].alt_q_enabled {
                    Some(header.segmentation.features[seg].alt_q_val)
                } else {
                    None
                };

            let (ydc, yac, uvdc, uvac) = calculate_segment_qmul(
                header.bit_depth,
                header.quant.base_q_idx,
                header.quant.delta_q_y_dc,
                header.quant.delta_q_uv_dc,
                header.quant.delta_q_uv_ac,
                seg_q_val,
                header.segmentation.abs_or_delta_update,
            );

            let seg_lf_val = if header.segmentation.enabled
                && header.segmentation.features[seg].alt_lf_enabled
            {
                Some(header.segmentation.features[seg].alt_lf_val)
            } else {
                None
            };

            let lflvls = calculate_segment_filter_levels(
                header.loop_filter.filter_level,
                seg_lf_val,
                header.segmentation.abs_or_delta_update,
                header.loop_filter.mode_ref_delta_enabled,
                &header.loop_filter.ref_deltas,
                &header.loop_filter.mode_deltas,
            );

            let seg_flags = (header.segmentation.features[seg].ref_frame_enabled as u16)
                | (((header.segmentation.features[seg].ref_frame_val as u16) & 3) << 1)
                | ((header.segmentation.features[seg].skip_enabled as u16) << 3);

            slice_param.seg_param[seg] = VASegmentParameterVP9 {
                segment_flags: seg_flags,
                filter_level: lflvls,
                luma_ac_quant_scale: yac,
                luma_dc_quant_scale: ydc,
                chroma_ac_quant_scale: uvac,
                chroma_dc_quant_scale: uvdc,
                pad: 0,
                va_reserved: [0; 4],
            };
        }

        let va = &self.device.va;
        let dpy = self.device.display;
        let ctx = pool.context_id;

        let mut is_concealed = false;
        unsafe {
            let status = (va.vaBeginPicture)(dpy, ctx, target_surface);
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DecodeError(format!(
                    "vaBeginPicture: {}",
                    va.err_str(status)
                )));
            }

            let mut pic_param_buf = 0;
            let status = (va.vaCreateBuffer)(
                dpy,
                ctx,
                VA_PICTURE_PARAMETER_BUFFER_TYPE,
                std::mem::size_of::<VADecPictureParameterBufferVP9>() as u32,
                1,
                &mut pic_param as *mut _ as *mut std::ffi::c_void,
                &mut pic_param_buf,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaEndPicture)(dpy, ctx);
                return Err(HwError::DecodeError(format!(
                    "vaCreateBuffer(pic_param): {}",
                    va.err_str(status)
                )));
            }

            let mut slice_param_buf = 0;
            let status = (va.vaCreateBuffer)(
                dpy,
                ctx,
                VA_SLICE_PARAMETER_BUFFER_TYPE,
                std::mem::size_of::<VASliceParameterBufferVP9>() as u32,
                1,
                &mut slice_param as *mut _ as *mut std::ffi::c_void,
                &mut slice_param_buf,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                let _ = (va.vaEndPicture)(dpy, ctx);
                return Err(HwError::DecodeError(format!(
                    "vaCreateBuffer(slice_param): {}",
                    va.err_str(status)
                )));
            }

            let mut slice_data_buf = 0;
            let status = (va.vaCreateBuffer)(
                dpy,
                ctx,
                VA_SLICE_DATA_BUFFER_TYPE,
                sub_frame_bytes.len() as u32,
                1,
                sub_frame_bytes.as_ptr() as *mut std::ffi::c_void,
                &mut slice_data_buf,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                let _ = (va.vaDestroyBuffer)(dpy, slice_param_buf);
                let _ = (va.vaEndPicture)(dpy, ctx);
                return Err(HwError::DecodeError(format!(
                    "vaCreateBuffer(slice_data): {}",
                    va.err_str(status)
                )));
            }

            let mut bufs = [pic_param_buf, slice_param_buf, slice_data_buf];
            let status = (va.vaRenderPicture)(dpy, ctx, bufs.as_mut_ptr(), bufs.len() as i32);
            for b in bufs {
                let _ = (va.vaDestroyBuffer)(dpy, b);
            }
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaEndPicture)(dpy, ctx);
                return Err(HwError::DecodeError(format!(
                    "vaRenderPicture: {}",
                    va.err_str(status)
                )));
            }

            let status = (va.vaEndPicture)(dpy, ctx);
            if status != VA_STATUS_SUCCESS {
                return Err(HwError::DecodeError(format!(
                    "vaEndPicture: {}",
                    va.err_str(status)
                )));
            }

            let mut sync_status = (va.vaSyncSurface)(dpy, target_surface);
            let mut retries = 0;
            while sync_status == crate::vaapi::ffi::VA_STATUS_ERROR_HW_BUSY && retries < 1000 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                sync_status = (va.vaSyncSurface)(dpy, target_surface);
                retries += 1;
            }
            if sync_status != VA_STATUS_SUCCESS {
                let err_msg = va.err_str(sync_status);
                if sync_status == crate::vaapi::ffi::VA_STATUS_ERROR_DECODING_ERROR {
                    is_concealed = true;
                    eprintln!("VA-API VP9 warning: {} (concealment active)", err_msg);
                } else {
                    return Err(HwError::DecodeError(format!(
                        "vaSyncSurface: {} ({})",
                        err_msg, sync_status
                    )));
                }
            }
        }

        // Update reference frame slots
        for i in 0..8 {
            if (header.refresh_frame_flags & (1 << i)) != 0 {
                self.ref_frame_map[i] = target_surface;
                self.ref_dims[i] = (header.width, header.height);
            }
        }

        if header.show_frame {
            let dmabuf = self
                .pool
                .as_ref()
                .and_then(|p| p.export_dmabuf_descriptor(target_surface).ok());
            let data = if dmabuf.is_none() {
                self.pool.as_ref().and_then(|p| {
                    p.read_surface_data(target_surface, header.width, header.height)
                        .ok()
                })
            } else {
                None
            }; // lazy zero-copy

            self.output_queue.push_back(DecodedFrame {
                pts,
                width: header.width,
                height: header.height,
                pixel_format: if self.is_10bit {
                    PixelFormat::P010
                } else {
                    PixelFormat::Nv12
                },
                surface_id: target_surface,
                data,
                interlace_mode: InterlaceMode::Progressive,
                dmabuf,
                dxgi_handle: None,
                cv_pixel_buffer: None,
                is_concealed,
            });
        }

        Ok(())
    }
}

impl HwDecoder for VaapiVp9Decoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        if packet.data.is_empty() {
            return Ok(());
        }

        let sub_frames = match parse_superframe_frames(&packet.data) {
            Ok(frames) => frames,
            Err(_) => vec![&packet.data[..]],
        };

        for sub_frame in sub_frames {
            let header = match Vp9FrameHeader::parse(sub_frame, Some(&self.ref_dims)) {
                Ok(h) => h,
                Err(_) => continue,
            };

            self.decode_sub_frame(&header, sub_frame, packet.pts)?;
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError> {
        Ok(self.output_queue.pop_front())
    }

    fn flush(&mut self) -> Result<(), HwError> {
        self.output_queue.clear();
        Ok(())
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.flush()?;
        self.ref_frame_map = [VA_INVALID_ID; 8];
        self.ref_dims = [(0, 0); 8];
        Ok(())
    }
}
