//! VA-API H.264 decode submission engine.
//!
//! Submits parsed parameter buffers and slice data directly to the GPU's fixed-function
//! video decode ASIC.

use std::collections::{HashMap, VecDeque};

use poppy_core::{InterlaceMode, Packet, PixelFormat};
use poppy_h264::{NalHeader, NalUnitType, Pps, SliceHeader, Sps, find_annex_b_nals};

use super::device::VaDevice;
use super::dpb::{Dpb, DpbFrame};
use super::ffi::*;
use super::surface::SurfacePool;
use crate::{DecodedFrame, HwDecoder, HwError};

/// Hardware-accelerated VA-API H.264 video decoder.
pub struct VaapiH264Decoder {
    device: VaDevice,
    pool: Option<SurfacePool>,
    dpb: Dpb,
    sps_map: HashMap<u32, Sps>,
    pps_map: HashMap<u32, Pps>,
    active_sps_id: Option<u32>,
    active_pps_id: Option<u32>,
    output_queue: VecDeque<DecodedFrame>,
    cur_surface_idx: usize,
}

impl VaapiH264Decoder {
    /// Initializes a new hardware decoder using an auto-detected DRM device.
    pub fn new() -> Result<Self, HwError> {
        let device = VaDevice::open_auto()?;
        Ok(Self {
            device,
            pool: None,
            dpb: Dpb::new(16),
            sps_map: HashMap::new(),
            pps_map: HashMap::new(),
            active_sps_id: None,
            active_pps_id: None,
            output_queue: VecDeque::new(),
            cur_surface_idx: 0,
        })
    }

    /// Access the underlying SurfacePool for zero-copy DMA-BUF export.
    pub fn surface_pool(&self) -> Option<&SurfacePool> {
        self.pool.as_ref()
    }

    /// Set parameter sets directly from extracted SPS and PPS slices.
    pub fn set_parameter_sets(
        &mut self,
        sps_list: &[bytes::Bytes],
        pps_list: &[bytes::Bytes],
    ) -> Result<(), HwError> {
        for sps_bytes in sps_list {
            if let Ok(sps) = Sps::parse(sps_bytes) {
                let id = sps.seq_parameter_set_id;
                self.ensure_pool(&sps)?;
                self.sps_map.insert(id, sps);
                self.active_sps_id = Some(id);
            }
        }
        for pps_bytes in pps_list {
            if let Ok(pps) = Pps::parse(pps_bytes) {
                let id = pps.pic_parameter_set_id;
                self.pps_map.insert(id, pps);
                self.active_pps_id = Some(id);
            }
        }
        Ok(())
    }

    pub fn ensure_pool(&mut self, sps: &Sps) -> Result<(), HwError> {
        let coded_w = sps.coded_width();
        let coded_h = sps.coded_height();
        let need_alloc = match &self.pool {
            Some(p) => p.width != coded_w || p.height != coded_h,
            None => true,
        };
        if need_alloc {
            let pool = SurfacePool::new(
                self.device.va.clone(),
                self.device.display,
                self.device.config_id,
                coded_w,
                coded_h,
                16,
            )?;
            self.pool = Some(pool);
            self.cur_surface_idx = 0;
            self.dpb.max_ref_frames = sps.max_num_ref_frames as usize;
            self.dpb.max_reorder_frames =
                if sps.pic_order_cnt_type == 2 || sps.max_num_ref_frames <= 1 {
                    0
                } else {
                    sps.max_num_ref_frames as usize
                };
        }
        Ok(())
    }

    /// Provide out-of-band codec extradata (avcC record or Annex-B parameter sets).
    pub fn set_extradata(&mut self, extradata: &[u8]) -> Result<(), HwError> {
        if let Ok(record) = poppy_core::codec::AvcCRecord::parse(extradata) {
            self.set_parameter_sets(&record.sps_list, &record.pps_list)?;
        } else {
            let nals = find_annex_b_nals(extradata);
            for nal in nals {
                let header = NalHeader::parse(nal[0]);
                match header.unit_type {
                    NalUnitType::Sps => {
                        if let Ok(sps) = Sps::parse(nal) {
                            let id = sps.seq_parameter_set_id;
                            self.sps_map.insert(id, sps);
                            self.active_sps_id = Some(id);
                        }
                    }
                    NalUnitType::Pps => {
                        if let Ok(pps) = Pps::parse(nal) {
                            let id = pps.pic_parameter_set_id;
                            self.pps_map.insert(id, pps);
                            self.active_pps_id = Some(id);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl HwDecoder for VaapiH264Decoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        // Handle both Annex-B (start codes) and AVCC length-prefixed NALs
        let is_avcc = if packet.data.starts_with(&[0, 0, 0, 1]) {
            false
        } else if packet.data.len() >= 4 {
            let len = u32::from_be_bytes([
                packet.data[0],
                packet.data[1],
                packet.data[2],
                packet.data[3],
            ]) as usize;
            if packet.data.starts_with(&[0, 0, 1]) {
                // If starts with 00 00 01, distinguish 3-byte Annex-B from AVCC (len 256..511)
                len > 0
                    && len + 4 <= packet.data.len()
                    && packet.data.len() > 4
                    && (packet.data[4] & 0x80 == 0)
            } else {
                len > 0 && len + 4 <= packet.data.len()
            }
        } else {
            false
        };

        let nals = if !is_avcc {
            find_annex_b_nals(&packet.data)
        } else {
            // Parse AVCC length-prefixed
            let mut list = Vec::new();
            let mut cursor = 0;
            let d = &packet.data;
            while cursor + 4 <= d.len() {
                let len =
                    u32::from_be_bytes([d[cursor], d[cursor + 1], d[cursor + 2], d[cursor + 3]])
                        as usize;
                cursor += 4;
                if cursor + len <= d.len() {
                    list.push(&d[cursor..cursor + len]);
                    cursor += len;
                } else {
                    break;
                }
            }
            list
        };

        let mut slice_nals = Vec::new();

        for nal in nals {
            if nal.is_empty() {
                continue;
            }

            let header = NalHeader::parse(nal[0]);
            match header.unit_type {
                NalUnitType::Sps => {
                    if let Ok(sps) = Sps::parse(nal) {
                        let sps_id = sps.seq_parameter_set_id;
                        let coded_w = sps.coded_width();
                        let coded_h = sps.coded_height();
                        self.sps_map.insert(sps_id, sps.clone());
                        self.active_sps_id = Some(sps_id);

                        // Reallocate surface pool if dimensions change or unallocated
                        let need_alloc = match &self.pool {
                            Some(p) => p.width != coded_w || p.height != coded_h,
                            None => true,
                        };

                        if need_alloc {
                            let pool = SurfacePool::new(
                                self.device.va.clone(),
                                self.device.display,
                                self.device.config_id,
                                coded_w,
                                coded_h,
                                16,
                            )?;
                            self.pool = Some(pool);
                            self.cur_surface_idx = 0;
                            self.dpb.max_ref_frames = sps.max_num_ref_frames as usize;
                            self.dpb.max_reorder_frames =
                                if sps.pic_order_cnt_type == 2 || sps.max_num_ref_frames <= 1 {
                                    0
                                } else {
                                    sps.max_num_ref_frames as usize
                                };
                        }
                    }
                }
                NalUnitType::Pps => {
                    if let Ok(pps) = Pps::parse(nal) {
                        let pps_id = pps.pic_parameter_set_id;
                        self.pps_map.insert(pps_id, pps);
                        self.active_pps_id = Some(pps_id);
                    }
                }
                NalUnitType::SliceIdr | NalUnitType::SliceNonIdr => {
                    let sps = match self.active_sps_id.and_then(|id| self.sps_map.get(&id)) {
                        Some(s) => s,
                        None => continue,
                    };
                    let pps = match self.active_pps_id.and_then(|id| self.pps_map.get(&id)) {
                        Some(p) => p,
                        None => continue,
                    };

                    let slice_payload = &nal[1..];
                    match SliceHeader::parse(
                        header.unit_type,
                        header.nal_ref_idc,
                        slice_payload,
                        sps,
                        pps,
                    ) {
                        Ok(slice) => {
                            slice_nals.push((nal, header, slice));
                        }
                        Err(e) => {
                            // Resilient packet error concealment: log and continue to next slice/NAL
                            eprintln!(
                                "VA-API H.264 slice parse warning: {e} (error concealment active)"
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        // Multi-slice picture batching: group slices belonging to the same picture
        let mut picture_batches: Vec<Vec<(&[u8], NalHeader, SliceHeader)>> = Vec::new();
        for (nal, header, slice) in slice_nals {
            let is_new_picture = match picture_batches.last() {
                None => true,
                Some(current_batch) => {
                    let (_, prev_header, prev_slice) = &current_batch[0];
                    slice.first_mb_in_slice == 0
                        || slice.frame_num != prev_slice.frame_num
                        || slice.pic_order_cnt_lsb != prev_slice.pic_order_cnt_lsb
                        || slice.field_pic_flag != prev_slice.field_pic_flag
                        || slice.bottom_field_flag != prev_slice.bottom_field_flag
                        || header.unit_type.is_idr() != prev_header.unit_type.is_idr()
                        || slice.idr_pic_id != prev_slice.idr_pic_id
                }
            };

            if is_new_picture {
                picture_batches.push(vec![(nal, header, slice)]);
            } else {
                picture_batches
                    .last_mut()
                    .unwrap()
                    .push((nal, header, slice));
            }
        }

        for slices in picture_batches {
            if slices.is_empty() {
                continue;
            }
            let sps = match self.active_sps_id.and_then(|id| self.sps_map.get(&id)) {
                Some(s) => s,
                None => continue,
            };
            let pps = match self.active_pps_id.and_then(|id| self.pps_map.get(&id)) {
                Some(p) => p,
                None => continue,
            };
            let pool = match &self.pool {
                Some(p) => p,
                None => continue,
            };

            let (_first_nal, first_header, first_slice) = &slices[0];
            let target_surface = pool.surfaces[self.cur_surface_idx];
            let cur_poc = match sps.pic_order_cnt_type {
                0 => first_slice.pic_order_cnt_lsb.unwrap_or(0) as i32,
                2 => (first_slice.frame_num as i32) * 2,
                _ => first_slice
                    .pic_order_cnt_lsb
                    .unwrap_or(first_slice.frame_num * 2) as i32,
            };

            // Clear DPB on IDR
            if first_header.unit_type.is_idr() {
                let released = self.dpb.clear_on_idr();
                for rf in released {
                    let dmabuf = pool.export_dmabuf_descriptor(rf.surface_id).ok();
                    let data = pool
                        .read_surface_data(rf.surface_id, rf.width, rf.height)
                        .ok();
                    self.output_queue.push_back(DecodedFrame {
                        pts: rf.pts,
                        width: rf.width,
                        height: rf.height,
                        pixel_format: PixelFormat::Nv12,
                        surface_id: rf.surface_id,
                        data,
                        interlace_mode: rf.interlace_mode,
                        dmabuf,
                        dxgi_handle: None,
                        cv_pixel_buffer: None,
                        is_concealed: rf.is_concealed,
                    });
                }
            }

            // Detect interlaced stream format (MBAFF / PAFF detection & progressive fallback)
            let interlace_mode = if sps.frame_mbs_only_flag {
                InterlaceMode::Progressive
            } else if first_slice.field_pic_flag {
                if first_slice.bottom_field_flag {
                    InterlaceMode::BottomField
                } else {
                    InterlaceMode::TopField
                }
            } else if sps.mb_adaptive_frame_field_flag {
                InterlaceMode::TopFieldFirst
            } else {
                InterlaceMode::Progressive
            };

            // Build VAPictureParameterBufferH264
            let mut pic_param: VAPictureParameterBufferH264 = unsafe { std::mem::zeroed() };
            let pic_flags = if first_slice.field_pic_flag {
                if first_slice.bottom_field_flag {
                    VA_PICTURE_H264_BOTTOM_FIELD
                } else {
                    VA_PICTURE_H264_TOP_FIELD
                }
            } else {
                0
            };
            pic_param.curr_pic = VAPictureH264 {
                picture_id: target_surface,
                frame_idx: first_slice.frame_num,
                flags: pic_flags,
                top_field_order_cnt: cur_poc,
                bottom_field_order_cnt: cur_poc,
            };

            // Resilient reference frame handling & concealment
            let is_p_or_b = !first_slice.slice_type.is_intra();
            let missing_refs = is_p_or_b && self.dpb.frames.is_empty();
            let mut is_concealed = missing_refs;

            self.dpb
                .fill_reference_frames(&mut pic_param.reference_frames);
            pic_param.picture_width_in_mbs_minus1 = sps.pic_width_in_mbs_minus1 as u16;
            pic_param.picture_height_in_mbs_minus1 = sps.pic_height_in_map_units_minus1 as u16;
            pic_param.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8;
            pic_param.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8;
            pic_param.num_ref_frames = sps.max_num_ref_frames as u8;
            pic_param.frame_num = first_slice.frame_num as u16;

            // Set seq_fields & pic_fields
            let mut seq_fields: u32 = 0;
            seq_fields |= (sps.chroma_format_idc as u32) & 3;
            if sps.gaps_in_frame_num_value_allowed_flag {
                seq_fields |= 1 << 3;
            }
            if sps.frame_mbs_only_flag {
                seq_fields |= 1 << 4;
            }
            if sps.mb_adaptive_frame_field_flag {
                seq_fields |= 1 << 5;
            }
            if sps.direct_8x8_inference_flag {
                seq_fields |= 1 << 6;
            }
            seq_fields |= (sps.log2_max_frame_num_minus4 & 0xF) << 8;
            seq_fields |= (sps.pic_order_cnt_type & 3) << 12;
            seq_fields |= (sps.log2_max_pic_order_cnt_lsb_minus4 & 0xF) << 14;
            if sps.delta_pic_order_always_zero_flag {
                seq_fields |= 1 << 18;
            }
            pic_param.seq_fields = seq_fields;

            let mut pic_fields: u32 = 0;
            if pps.entropy_coding_mode_flag {
                pic_fields |= 1 << 0;
            }
            if pps.weighted_pred_flag {
                pic_fields |= 1 << 1;
            }
            pic_fields |= ((pps.weighted_bipred_idc & 3) as u32) << 2;
            if pps.transform_8x8_mode_flag {
                pic_fields |= 1 << 4;
            }
            if first_slice.field_pic_flag {
                pic_fields |= 1 << 5;
            }
            if first_slice.bottom_field_flag {
                pic_fields |= 1 << 6;
            }
            if pps.bottom_field_pic_order_in_frame_present_flag {
                pic_fields |= 1 << 7;
            }
            if first_header.nal_ref_idc > 0 {
                pic_fields |= 1 << 9;
            }
            pic_param.pic_fields = pic_fields;

            // Build VAIQMatrixBufferH264
            let iq_matrix = VAIQMatrixBufferH264::default();

            // Submit buffers to hardware decoder ASIC
            unsafe {
                let va = &self.device.va;
                let d = self.device.display;
                let ctx = pool.context_id;

                let mut bufs_to_render = Vec::with_capacity(2 + slices.len() * 2);

                (va.vaBeginPicture)(d, ctx, target_surface);

                let mut buf_pic = 0;
                (va.vaCreateBuffer)(
                    d,
                    ctx,
                    VA_PICTURE_PARAMETER_BUFFER_TYPE,
                    std::mem::size_of::<VAPictureParameterBufferH264>() as u32,
                    1,
                    &mut pic_param as *mut _ as *mut std::ffi::c_void,
                    &mut buf_pic,
                );
                bufs_to_render.push(buf_pic);

                let mut buf_iq = 0;
                (va.vaCreateBuffer)(
                    d,
                    ctx,
                    VA_IQMATRIX_BUFFER_TYPE,
                    std::mem::size_of::<VAIQMatrixBufferH264>() as u32,
                    1,
                    &iq_matrix as *const _ as *mut std::ffi::c_void,
                    &mut buf_iq,
                );
                bufs_to_render.push(buf_iq);

                // Multi-slice batching: append all slice parameter and slice data buffers together
                for (nal, _header, slice) in &slices {
                    let mut slice_param = VASliceParameterBufferH264 {
                        slice_data_size: nal.len() as u32,
                        slice_data_offset: 0,
                        slice_data_bit_offset: (slice.slice_data_bit_offset + 8) as u16,
                        first_mb_in_slice: slice.first_mb_in_slice as u16,
                        slice_type: slice.slice_type as u8,
                        direct_spatial_mv_pred_flag: slice.direct_spatial_mv_pred_flag as u8,
                        num_ref_idx_l0_active_minus1: slice.num_ref_idx_l0_active_minus1 as u8,
                        num_ref_idx_l1_active_minus1: slice.num_ref_idx_l1_active_minus1 as u8,
                        cabac_init_idc: slice.cabac_init_idc as u8,
                        slice_qp_delta: slice.slice_qp_delta as i8,
                        disable_deblocking_filter_idc: slice.disable_deblocking_filter_idc as u8,
                        slice_alpha_c0_offset_div2: slice.slice_alpha_c0_offset_div2 as i8,
                        slice_beta_offset_div2: slice.slice_beta_offset_div2 as i8,
                        ..Default::default()
                    };
                    self.dpb
                        .fill_ref_pic_list0(cur_poc, &mut slice_param.ref_pic_list0);
                    if slice.slice_type == poppy_h264::SliceType::B {
                        self.dpb
                            .fill_ref_pic_list1(cur_poc, &mut slice_param.ref_pic_list1);
                    }

                    let mut buf_slice_param = 0;
                    (va.vaCreateBuffer)(
                        d,
                        ctx,
                        VA_SLICE_PARAMETER_BUFFER_TYPE,
                        std::mem::size_of::<VASliceParameterBufferH264>() as u32,
                        1,
                        &mut slice_param as *mut _ as *mut std::ffi::c_void,
                        &mut buf_slice_param,
                    );
                    bufs_to_render.push(buf_slice_param);

                    let mut buf_slice_data = 0;
                    (va.vaCreateBuffer)(
                        d,
                        ctx,
                        VA_SLICE_DATA_BUFFER_TYPE,
                        nal.len() as u32,
                        1,
                        nal.as_ptr() as *mut std::ffi::c_void,
                        &mut buf_slice_data,
                    );
                    bufs_to_render.push(buf_slice_data);
                }

                (va.vaRenderPicture)(
                    d,
                    ctx,
                    bufs_to_render.as_mut_ptr(),
                    bufs_to_render.len() as i32,
                );
                (va.vaEndPicture)(d, ctx);

                for &b in &bufs_to_render {
                    (va.vaDestroyBuffer)(d, b);
                }

                // Sync surface: hardware ASIC executes decode
                let mut sync_status = (va.vaSyncSurface)(d, target_surface);
                let mut retries = 0;
                while sync_status == 34 && retries < 1000 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    sync_status = (va.vaSyncSurface)(d, target_surface);
                    retries += 1;
                }
                if sync_status != VA_STATUS_SUCCESS {
                    let err_msg = va.err_str(sync_status);
                    if err_msg.contains("decoding error") || err_msg.contains("error") {
                        is_concealed = true;
                        eprintln!("VA-API warning: {} (concealed corrupted frame)", err_msg);
                    } else {
                        return Err(HwError::DecodeError(format!(
                            "vaSyncSurface failed: {}",
                            err_msg
                        )));
                    }
                }
            }

            // Update DPB with decoded frame
            let released = self.dpb.insert(DpbFrame {
                surface_id: target_surface,
                frame_num: first_slice.frame_num,
                poc: cur_poc,
                is_reference: first_header.nal_ref_idc > 0,
                is_long_term: false,
                pts: packet.pts.or(Some(packet.dts)),
                needed_for_output: true,
                width: sps.display_width(),
                height: sps.display_height(),
                interlace_mode,
                is_concealed,
            });

            for rf in released {
                let dmabuf = pool.export_dmabuf_descriptor(rf.surface_id).ok();
                let data = pool
                    .read_surface_data(rf.surface_id, rf.width, rf.height)
                    .ok();
                self.output_queue.push_back(DecodedFrame {
                    pts: rf.pts,
                    width: rf.width,
                    height: rf.height,
                    pixel_format: PixelFormat::Nv12,
                    surface_id: rf.surface_id,
                    data,
                    interlace_mode: rf.interlace_mode,
                    dmabuf,
                    dxgi_handle: None,
                    cv_pixel_buffer: None,
                    is_concealed: rf.is_concealed,
                });
            }

            self.cur_surface_idx = (self.cur_surface_idx + 1) % pool.surfaces.len();
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Option<DecodedFrame>, HwError> {
        Ok(self.output_queue.pop_front())
    }

    fn flush(&mut self) -> Result<(), HwError> {
        let remaining = self.dpb.drain();
        for rf in remaining {
            let dmabuf = self
                .pool
                .as_ref()
                .and_then(|p| p.export_dmabuf_descriptor(rf.surface_id).ok());
            let data = self
                .pool
                .as_ref()
                .and_then(|p| p.read_surface_data(rf.surface_id, rf.width, rf.height).ok());
            self.output_queue.push_back(DecodedFrame {
                pts: rf.pts,
                width: rf.width,
                height: rf.height,
                pixel_format: PixelFormat::Nv12,
                surface_id: rf.surface_id,
                data,
                interlace_mode: rf.interlace_mode,
                dmabuf,
                dxgi_handle: None,
                cv_pixel_buffer: None,
                is_concealed: rf.is_concealed,
            });
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.dpb.clear();
        self.output_queue.clear();
        Ok(())
    }
}
