//! VA-API H.264 decode submission engine.
//!
//! Submits parsed parameter buffers and slice data directly to the GPU's fixed-function
//! video decode ASIC.

use std::collections::{HashMap, VecDeque};

use poppy_core::{Packet, PixelFormat};
use poppy_h264::{find_annex_b_nals, NalHeader, NalUnitType, Pps, SliceHeader, Sps};

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
}

impl HwDecoder for VaapiH264Decoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        // Handle both Annex-B (start codes) and AVCC length-prefixed NALs
        let nals = if packet.data.starts_with(&[0, 0, 0, 1]) || packet.data.starts_with(&[0, 0, 1]) {
            find_annex_b_nals(&packet.data)
        } else {
            // Parse AVCC length-prefixed
            let mut list = Vec::new();
            let mut cursor = 0;
            let d = &packet.data;
            while cursor + 4 <= d.len() {
                let len = u32::from_be_bytes([d[cursor], d[cursor + 1], d[cursor + 2], d[cursor + 3]]) as usize;
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
                        self.sps_map.insert(sps_id, sps);
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
                    let slice = match SliceHeader::parse(header.unit_type, slice_payload, sps, pps) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    let pool = match &self.pool {
                        Some(p) => p,
                        None => continue,
                    };

                    let target_surface = pool.surfaces[self.cur_surface_idx];
                    let cur_poc = slice.pic_order_cnt_lsb.unwrap_or(0) as i32;

                    // Clear DPB on IDR
                    if header.unit_type.is_idr() {
                        let released = self.dpb.clear_on_idr();
                        for rf in released {
                            self.output_queue.push_back(DecodedFrame {
                                pts: rf.pts,
                                width: sps.display_width(),
                                height: sps.display_height(),
                                pixel_format: PixelFormat::Nv12,
                                surface_id: rf.surface_id,
                                data: None,
                            });
                        }
                    }

                    // Build VAPictureParameterBufferH264
                    let mut pic_param: VAPictureParameterBufferH264 = unsafe { std::mem::zeroed() };
                    pic_param.curr_pic = VAPictureH264 {
                        picture_id: target_surface,
                        frame_idx: slice.frame_num,
                        flags: 0,
                        top_field_order_cnt: cur_poc,
                        bottom_field_order_cnt: cur_poc,
                    };
                    self.dpb.fill_reference_frames(&mut pic_param.reference_frames);
                    pic_param.picture_width_in_mbs_minus1 = sps.pic_width_in_mbs_minus1 as u16;
                    pic_param.picture_height_in_mbs_minus1 = sps.pic_height_in_map_units_minus1 as u16;
                    pic_param.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8;
                    pic_param.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8;
                    pic_param.num_ref_frames = sps.max_num_ref_frames as u8;
                    pic_param.frame_num = slice.frame_num as u16;

                    // Build VAIQMatrixBufferH264
                    let iq_matrix = VAIQMatrixBufferH264::default();

                    // Build VASliceParameterBufferH264
                    let mut slice_param = VASliceParameterBufferH264 {
                        slice_data_size: nal.len() as u32,
                        slice_data_offset: 0,
                        slice_data_bit_offset: slice.slice_data_bit_offset as u16,
                        first_mb_in_slice: slice.first_mb_in_slice as u16,
                        slice_type: slice.raw_slice_type as u8,
                        slice_qp_delta: slice.slice_qp_delta as i8,
                        ..Default::default()
                    };
                    self.dpb.fill_ref_pic_list0(cur_poc, &mut slice_param.ref_pic_list0);

                    // Submit buffers to hardware decoder ASIC
                    unsafe {
                        let va = &self.device.va;
                        let d = self.device.display;
                        let ctx = pool.context_id;

                        let mut buf_pic = 0;
                        let mut buf_iq = 0;
                        let mut buf_slice_param = 0;
                        let mut buf_slice_data = 0;

                        (va.vaBeginPicture)(d, ctx, target_surface);

                        (va.vaCreateBuffer)(
                            d,
                            ctx,
                            VA_PICTURE_PARAMETER_BUFFER_TYPE,
                            std::mem::size_of::<VAPictureParameterBufferH264>() as u32,
                            1,
                            &mut pic_param as *mut _ as *mut std::ffi::c_void,
                            &mut buf_pic,
                        );

                        (va.vaCreateBuffer)(
                            d,
                            ctx,
                            VA_IQMATRIX_BUFFER_TYPE,
                            std::mem::size_of::<VAIQMatrixBufferH264>() as u32,
                            1,
                            &iq_matrix as *const _ as *mut std::ffi::c_void,
                            &mut buf_iq,
                        );

                        (va.vaCreateBuffer)(
                            d,
                            ctx,
                            VA_SLICE_PARAMETER_BUFFER_TYPE,
                            std::mem::size_of::<VASliceParameterBufferH264>() as u32,
                            1,
                            &mut slice_param as *mut _ as *mut std::ffi::c_void,
                            &mut buf_slice_param,
                        );

                        (va.vaCreateBuffer)(
                            d,
                            ctx,
                            VA_SLICE_DATA_BUFFER_TYPE,
                            nal.len() as u32,
                            1,
                            nal.as_ptr() as *mut std::ffi::c_void,
                            &mut buf_slice_data,
                        );

                        let mut bufs = [buf_pic, buf_iq, buf_slice_param, buf_slice_data];
                        (va.vaRenderPicture)(d, ctx, bufs.as_mut_ptr(), 4);
                        (va.vaEndPicture)(d, ctx);

                        for &b in &bufs {
                            (va.vaDestroyBuffer)(d, b);
                        }

                        // Sync surface: hardware ASIC executes decode
                        let sync_status = (va.vaSyncSurface)(d, target_surface);
                        if sync_status != VA_STATUS_SUCCESS {
                            let err_msg = va.err_str(sync_status);
                            if err_msg.contains("decoding error") {
                                eprintln!("VA-API warning: {} (error concealment active)", err_msg);
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
                        frame_num: slice.frame_num,
                        poc: cur_poc,
                        is_reference: header.nal_ref_idc > 0,
                        is_long_term: false,
                        pts: packet.pts.or(Some(packet.dts)),
                        needed_for_output: true,
                    });

                    for rf in released {
                        self.output_queue.push_back(DecodedFrame {
                            pts: rf.pts,
                            width: sps.display_width(),
                            height: sps.display_height(),
                            pixel_format: PixelFormat::Nv12,
                            surface_id: rf.surface_id,
                            data: None,
                        });
                    }

                    // If stream has no B-frames (no reordering delay), emit frame directly
                    if sps.max_num_ref_frames <= 1 {
                        self.output_queue.push_back(DecodedFrame {
                            pts: packet.pts.or(Some(packet.dts)),
                            width: sps.display_width(),
                            height: sps.display_height(),
                            pixel_format: PixelFormat::Nv12,
                            surface_id: target_surface,
                            data: None,
                        });
                    }

                    self.cur_surface_idx = (self.cur_surface_idx + 1) % pool.surfaces.len();
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
        let remaining = self.dpb.drain();
        for rf in remaining {
            self.output_queue.push_back(DecodedFrame {
                pts: rf.pts,
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::Nv12,
                surface_id: rf.surface_id,
                data: None,
            });
        }
        Ok(())
    }
}
