//! VA-API AV1 hardware decode submission engine.
//!
//! Submits parsed AV1 parameter buffers (Sequence Header, Frame Header, Tile Data)
//! directly to the GPU's fixed-function hardware video decode ASIC.

use std::collections::VecDeque;
use std::sync::Arc;

use poppy_av1::frame_header::{Av1FrameType, DpbState, FrameHeader};
use poppy_av1::obu::{ObuType, split_obus};
use poppy_av1::sequence_header::SequenceHeader;
use poppy_core::{InterlaceMode, Packet, PixelFormat};

use super::device::VaDevice;
use super::ffi::*;
use super::surface::SurfacePool;
use crate::{DecodedFrame, HwDecoder, HwError};

/// Hardware-accelerated VA-API AV1 video decoder (8-bit and 10-bit HDR).
pub struct VaapiAv1Decoder {
    device: VaDevice,
    pool: Option<SurfacePool>,
    dpb_slots: [VASurfaceID; 8],
    dpb_state: DpbState,
    seq_header: Option<SequenceHeader>,
    pending_frame_header: Option<FrameHeader>,
    output_queue: VecDeque<DecodedFrame>,
    cur_surface_idx: usize,
    is_10bit: bool,
}

impl VaapiAv1Decoder {
    /// Initializes a new hardware AV1 decoder using an auto-detected DRM device.
    pub fn new(is_10bit: bool) -> Result<Self, HwError> {
        let device = VaDevice::open_auto_av1(is_10bit)?;
        Ok(Self {
            device,
            pool: None,
            dpb_slots: [VA_INVALID_ID; 8],
            dpb_state: DpbState::default(),
            seq_header: None,
            pending_frame_header: None,
            output_queue: VecDeque::new(),
            cur_surface_idx: 0,
            is_10bit,
        })
    }

    /// Feeds container extradata (e.g. `av1C` or Matroska CodecPrivate) into the decoder.
    pub fn set_extradata(&mut self, extradata: &[u8]) -> Result<(), HwError> {
        if extradata.is_empty() {
            return Ok(());
        }

        // If av1C box header is present (bit 7 set in byte 0), configOBUs start at offset 4
        let candidate = if extradata.len() > 4 && (extradata[0] & 0x80) != 0 {
            &extradata[4..]
        } else {
            extradata
        };

        if let Ok(obus) = split_obus(candidate) {
            for obu in obus {
                #[allow(clippy::collapsible_if)]
                if obu.header.obu_type == ObuType::SequenceHeader {
                    if let Ok(seq) = SequenceHeader::parse(obu.payload) {
                        self.init_surface_pool(&seq)?;
                        self.seq_header = Some(seq);
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Access the underlying SurfacePool for zero-copy DMA-BUF export.
    pub fn surface_pool(&self) -> Option<&SurfacePool> {
        self.pool.as_ref()
    }

    fn init_surface_pool(&mut self, seq: &SequenceHeader) -> Result<(), HwError> {
        let width = seq.max_frame_width;
        let height = seq.max_frame_height;
        let rt_format = if seq.high_bitdepth {
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
            self.dpb_slots = [VA_INVALID_ID; 8];
            self.dpb_state = DpbState {
                widths: [width; 8],
                heights: [height; 8],
                render_widths: [width; 8],
                render_heights: [height; 8],
                ..Default::default()
            };
        }

        self.is_10bit = seq.high_bitdepth;
        Ok(())
    }

    fn decode_frame(
        &mut self,
        fh: &FrameHeader,
        tile_data: &[u8],
        pts: Option<i64>,
    ) -> Result<(), HwError> {
        let seq = match &self.seq_header {
            Some(s) => s,
            None => return Ok(()),
        };

        let pool = match &self.pool {
            Some(p) => p,
            None => return Ok(()),
        };

        // Select free surface not currently held in DPB
        let target_surface = pool
            .surfaces
            .iter()
            .copied()
            .find(|&surf| !self.dpb_slots.contains(&surf))
            .unwrap_or(pool.surfaces[self.cur_surface_idx]);
        self.cur_surface_idx = (self.cur_surface_idx + 1) % pool.surfaces.len();

        let bit_depth_idx = if seq.high_bitdepth {
            if seq.twelve_bit { 2 } else { 1 }
        } else {
            0
        };

        // Populate VADecPictureParameterBufferAV1
        let mut pic_param: VADecPictureParameterBufferAV1 = unsafe { std::mem::zeroed() };
        pic_param.profile = seq.seq_profile;
        pic_param.order_hint_bits_minus_1 = seq.order_hint_bits_minus_1;
        pic_param.bit_depth_idx = bit_depth_idx;
        pic_param.matrix_coefficients = seq.matrix_coefficients;
        pic_param.current_frame = target_surface;
        pic_param.current_display_picture = target_surface;
        pic_param.frame_width_minus1 = fh.frame_width.saturating_sub(1) as u16;
        pic_param.frame_height_minus1 = fh.frame_height.saturating_sub(1) as u16;
        pic_param.primary_ref_frame = fh.primary_ref_frame;
        pic_param.order_hint = fh.order_hint;
        pic_param.tile_cols = fh.tile_cols;
        pic_param.tile_rows = fh.tile_rows;
        pic_param.context_update_tile_id = fh.context_update_tile_id;
        pic_param.superres_scale_denominator = if fh.use_superres {
            fh.superres_scale_denominator
        } else {
            8
        };
        pic_param.interp_filter = fh.interp_filter;
        pic_param.filter_level = [fh.filter_level[0], fh.filter_level[1]];
        pic_param.filter_level_u = fh.filter_level_u;
        pic_param.filter_level_v = fh.filter_level_v;
        pic_param.base_qindex = fh.base_qindex;
        pic_param.y_dc_delta_q = fh.y_dc_delta_q;
        pic_param.u_dc_delta_q = fh.u_dc_delta_q;
        pic_param.u_ac_delta_q = fh.u_ac_delta_q;
        pic_param.v_dc_delta_q = fh.v_dc_delta_q;
        pic_param.v_ac_delta_q = fh.v_ac_delta_q;
        pic_param.cdef_damping_minus_3 = fh.cdef_damping_minus_3;
        pic_param.cdef_bits = fh.cdef_bits;
        pic_param.cdef_y_strengths = fh.cdef_y_strengths;
        pic_param.cdef_uv_strengths = fh.cdef_uv_strengths;
        pic_param.width_in_sbs_minus_1 = fh.width_in_sbs_minus_1;
        pic_param.height_in_sbs_minus_1 = fh.height_in_sbs_minus_1;
        pic_param.ref_deltas = fh.ref_deltas;
        pic_param.mode_deltas = fh.mode_deltas;

        // Sequence info fields
        let mut seq_fields = 0u32;
        if seq.still_picture {
            seq_fields |= 1 << 0;
        }
        if seq.use_128x128_superblock {
            seq_fields |= 1 << 1;
        }
        if seq.enable_filter_intra {
            seq_fields |= 1 << 2;
        }
        if seq.enable_intra_edge_filter {
            seq_fields |= 1 << 3;
        }
        if seq.enable_interintra_compound {
            seq_fields |= 1 << 4;
        }
        if seq.enable_masked_compound {
            seq_fields |= 1 << 5;
        }
        if seq.enable_dual_filter {
            seq_fields |= 1 << 6;
        }
        if seq.enable_order_hint {
            seq_fields |= 1 << 7;
        }
        if seq.enable_jnt_comp {
            seq_fields |= 1 << 8;
        }
        if seq.enable_cdef {
            seq_fields |= 1 << 9;
        }
        if seq.mono_chrome {
            seq_fields |= 1 << 10;
        }
        if seq.color_range {
            seq_fields |= 1 << 11;
        }
        if seq.subsampling_x {
            seq_fields |= 1 << 12;
        }
        if seq.subsampling_y {
            seq_fields |= 1 << 13;
        }
        if seq.film_grain_params_present {
            seq_fields |= 1 << 15;
        }
        pic_param.seq_info_fields = seq_fields;

        // Picture info fields
        let mut pic_fields = 0u32;
        pic_fields |= (fh.frame_type as u8 as u32) & 0x03;
        if fh.show_frame {
            pic_fields |= 1 << 2;
        }
        if fh.showable_frame {
            pic_fields |= 1 << 3;
        }
        if fh.error_resilient_mode {
            pic_fields |= 1 << 4;
        }
        if fh.disable_cdf_update {
            pic_fields |= 1 << 5;
        }
        if fh.allow_screen_content_tools {
            pic_fields |= 1 << 6;
        }
        if fh.force_integer_mv {
            pic_fields |= 1 << 7;
        }
        if fh.allow_intrabc {
            pic_fields |= 1 << 8;
        }
        if fh.use_superres {
            pic_fields |= 1 << 9;
        }
        if fh.allow_high_precision_mv {
            pic_fields |= 1 << 10;
        }
        if fh.is_motion_mode_switchable {
            pic_fields |= 1 << 11;
        }
        if fh.use_ref_frame_mvs {
            pic_fields |= 1 << 12;
        }
        if fh.disable_frame_end_update_cdf {
            pic_fields |= 1 << 13;
        }
        if fh.uniform_tile_spacing_flag {
            pic_fields |= 1 << 14;
        }
        if fh.allow_warped_motion {
            pic_fields |= 1 << 15;
        }
        pic_param.pic_info_fields = pic_fields;

        // Loop filter info
        let mut lf_info = 0u8;
        lf_info |= fh.sharpness_level & 0x07;
        if fh.mode_ref_delta_enabled {
            lf_info |= 1 << 3;
        }
        if fh.mode_ref_delta_update {
            lf_info |= 1 << 4;
        }
        pic_param.loop_filter_info_fields = lf_info;

        // Quantization matrix fields
        let mut qm_fields = 0u16;
        if fh.using_qmatrix {
            qm_fields |= 1 << 0;
            qm_fields |= ((fh.qm_y as u16) & 0x0F) << 1;
            qm_fields |= ((fh.qm_u as u16) & 0x0F) << 5;
            qm_fields |= ((fh.qm_v as u16) & 0x0F) << 9;
        }
        pic_param.qmatrix_fields = qm_fields;

        // Mode control fields
        let mut mode_ctrl = 0u32;
        if fh.delta_q_present_flag {
            mode_ctrl |= 1 << 0;
        }
        mode_ctrl |= ((fh.log2_delta_q_res as u32) & 0x03) << 1;
        if fh.delta_lf_present_flag {
            mode_ctrl |= 1 << 3;
        }
        mode_ctrl |= ((fh.log2_delta_lf_res as u32) & 0x03) << 4;
        if fh.delta_lf_multi {
            mode_ctrl |= 1 << 6;
        }
        mode_ctrl |= ((fh.tx_mode as u32) & 0x03) << 7;
        if fh.reference_select {
            mode_ctrl |= 1 << 9;
        }
        if fh.reduced_tx_set_used {
            mode_ctrl |= 1 << 10;
        }
        if fh.skip_mode_present {
            mode_ctrl |= 1 << 11;
        }
        pic_param.mode_control_fields = mode_ctrl;

        // Loop restoration fields
        let mut lr_fields = 0u16;
        lr_fields |= (fh.yframe_restoration_type as u16) & 0x03;
        lr_fields |= ((fh.cbframe_restoration_type as u16) & 0x03) << 2;
        lr_fields |= ((fh.crframe_restoration_type as u16) & 0x03) << 4;
        lr_fields |= ((fh.lr_unit_shift as u16) & 0x03) << 6;
        lr_fields |= ((fh.lr_uv_shift as u16) & 0x01) << 8;
        pic_param.loop_restoration_fields = lr_fields;

        // References
        if !(fh.frame_type == Av1FrameType::KeyFrame && fh.show_frame) {
            pic_param.ref_frame_map.copy_from_slice(&self.dpb_slots);
        } else {
            pic_param.ref_frame_map = [VA_INVALID_ID; 8];
        }
        pic_param.ref_frame_idx.copy_from_slice(&fh.ref_frame_idx);

        // Segmentation
        let mut seg_info_fields = 0u32;
        if fh.segmentation_enabled {
            seg_info_fields |= 1 << 0;
        }
        if fh.segmentation_update_map {
            seg_info_fields |= 1 << 1;
        }
        if fh.segmentation_temporal_update {
            seg_info_fields |= 1 << 2;
        }
        if fh.segmentation_update_data {
            seg_info_fields |= 1 << 3;
        }
        pic_param.seg_info.segment_info_fields = seg_info_fields;
        pic_param.seg_info.feature_data = fh.feature_data;
        for i in 0..8 {
            let mut mask = 0u8;
            for j in 0..8 {
                if fh.feature_enabled[i][j] {
                    mask |= 1 << j;
                }
            }
            pic_param.seg_info.feature_mask[i] = mask;
        }

        for i in 0..7 {
            pic_param.wm[i].wmtype = fh.gm_type[i] as u32;
            pic_param.wm[i].invalid = fh.gm_invalid[i];
            pic_param.wm[i].wmmat[0] = fh.gm_params[i][0];
            pic_param.wm[i].wmmat[1] = fh.gm_params[i][1];
            pic_param.wm[i].wmmat[2] = fh.gm_params[i][2];
            pic_param.wm[i].wmmat[3] = fh.gm_params[i][3];
            pic_param.wm[i].wmmat[4] = fh.gm_params[i][4];
            pic_param.wm[i].wmmat[5] = fh.gm_params[i][5];
            pic_param.wm[i].wmmat[6] = 0;
            pic_param.wm[i].wmmat[7] = 0;
        }

        // Build VASliceParameterBufferAV1
        let mut slice_param = VASliceParameterBufferAV1 {
            slice_data_size: tile_data.len() as u32,
            slice_data_offset: 0,
            slice_data_flag: 0,
            tile_row: 0,
            tile_column: 0,
            tg_start: 0,
            tg_end: 0,
            anchor_frame_idx: 0,
            pad: 0,
            tile_idx_in_tile_list: 0,
            va_reserved: [0; 4],
        };

        let mut is_concealed = false;

        // Submit to VA-API hardware
        unsafe {
            let va = &self.device.va;
            let dpy = self.device.display;
            let ctx = pool.context_id;

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
                std::mem::size_of::<VADecPictureParameterBufferAV1>() as u32,
                1,
                &mut pic_param as *mut _ as *mut std::ffi::c_void,
                &mut pic_param_buf,
            );
            if status != VA_STATUS_SUCCESS {
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
                std::mem::size_of::<VASliceParameterBufferAV1>() as u32,
                1,
                &mut slice_param as *mut _ as *mut std::ffi::c_void,
                &mut slice_param_buf,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
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
                tile_data.len() as u32,
                1,
                tile_data.as_ptr() as *mut std::ffi::c_void,
                &mut slice_data_buf,
            );
            if status != VA_STATUS_SUCCESS {
                let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                let _ = (va.vaDestroyBuffer)(dpy, slice_param_buf);
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
            while sync_status == 34 && retries < 1000 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                sync_status = (va.vaSyncSurface)(dpy, target_surface);
                retries += 1;
            }
            if sync_status != VA_STATUS_SUCCESS {
                let err_msg = va.err_str(sync_status);
                if err_msg.contains("decoding error") {
                    is_concealed = true;
                    eprintln!("VA-API AV1 warning: {} (error concealment active)", err_msg);
                } else {
                    return Err(HwError::DecodeError(format!(
                        "vaSyncSurface: {} ({})",
                        err_msg, sync_status
                    )));
                }
            }
        }

        // Update reference frame slots and DPB state
        for i in 0..8 {
            if (fh.refresh_frame_flags & (1 << i)) != 0 {
                self.dpb_slots[i] = target_surface;
                self.dpb_state.order_hints[i] = fh.order_hint as u32;
                self.dpb_state.widths[i] = fh.frame_width;
                self.dpb_state.heights[i] = fh.frame_height;
                self.dpb_state.render_widths[i] = fh.render_width;
                self.dpb_state.render_heights[i] = fh.render_height;
            }
        }

        if fh.show_frame {
            let dmabuf = self
                .pool
                .as_ref()
                .and_then(|p| p.export_dmabuf_descriptor(target_surface).ok());
            self.output_queue.push_back(DecodedFrame {
                pts,
                width: fh.frame_width,
                height: fh.frame_height,
                pixel_format: if self.is_10bit {
                    PixelFormat::P010
                } else {
                    PixelFormat::Nv12
                },
                surface_id: target_surface,
                data: None,
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

impl HwDecoder for VaapiAv1Decoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        let obus = match split_obus(&packet.data) {
            Ok(o) => o,
            Err(_) => return Ok(()),
        };

        for obu in obus {
            match obu.header.obu_type {
                ObuType::SequenceHeader => {
                    if let Ok(seq) = SequenceHeader::parse(obu.payload) {
                        self.init_surface_pool(&seq)?;
                        self.seq_header = Some(seq);
                    }
                }
                ObuType::FrameHeader => {
                    let seq = match &self.seq_header {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Ok(fh) = FrameHeader::parse_with_dpb(obu.payload, seq, &self.dpb_state) {
                        if fh.show_existing_frame {
                            let idx = fh.frame_to_show_map_idx as usize;
                            if idx < 8 && self.dpb_slots[idx] != VA_INVALID_ID {
                                let surf_id = self.dpb_slots[idx];
                                let dmabuf = self
                                    .pool
                                    .as_ref()
                                    .and_then(|p| p.export_dmabuf_descriptor(surf_id).ok());
                                self.output_queue.push_back(DecodedFrame {
                                    pts: packet.pts.or(Some(packet.dts)),
                                    width: fh.frame_width.max(seq.max_frame_width),
                                    height: fh.frame_height.max(seq.max_frame_height),
                                    pixel_format: if self.is_10bit {
                                        PixelFormat::P010
                                    } else {
                                        PixelFormat::Nv12
                                    },
                                    surface_id: surf_id,
                                    data: None,
                                    interlace_mode: InterlaceMode::Progressive,
                                    dmabuf,
                                    dxgi_handle: None,
                                    cv_pixel_buffer: None,
                                    is_concealed: false,
                                });
                            }
                        } else {
                            self.pending_frame_header = Some(fh);
                        }
                    }
                }
                ObuType::Frame => {
                    let seq = match &self.seq_header {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Ok(fh) = FrameHeader::parse_with_dpb(obu.payload, seq, &self.dpb_state) {
                        let tile_data = if fh.tile_data_offset < obu.payload.len() {
                            &obu.payload[fh.tile_data_offset..]
                        } else {
                            &[]
                        };
                        self.decode_frame(&fh, tile_data, packet.pts.or(Some(packet.dts)))?;
                    }
                }
                ObuType::TileGroup => {
                    if let Some(fh) = self.pending_frame_header.take() {
                        self.decode_frame(&fh, obu.payload, packet.pts.or(Some(packet.dts)))?;
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError> {
        Ok(self.output_queue.pop_front())
    }

    fn flush(&mut self) -> Result<(), HwError> {
        Ok(())
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.dpb_slots = [VA_INVALID_ID; 8];
        self.dpb_state = DpbState::default();
        self.output_queue.clear();
        Ok(())
    }
}
