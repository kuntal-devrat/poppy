//! VA-API H.265 / HEVC decode submission engine.
//!
//! Submits parsed HEVC parameter buffers (VPS, SPS, PPS, Slice) directly to the GPU's
//! fixed-function hardware video decode ASIC.

use std::collections::{HashMap, VecDeque};

use poppy_core::{Packet, PixelFormat};
use poppy_hevc::{
    split_annex_b_nalus, HevcNalHeader, HevcNalType, PictureParameterSet, SequenceParameterSet,
    SliceSegmentHeader, VideoParameterSet,
};

use super::device::VaDevice;
use super::dpb::{Dpb, DpbFrame};
use super::ffi::*;
use super::surface::SurfacePool;
use crate::{DecodedFrame, HwDecoder, HwError};

/// Hardware-accelerated VA-API H.265 / HEVC video decoder (8-bit and 10-bit HDR).
pub struct VaapiHevcDecoder {
    device: VaDevice,
    pool: Option<SurfacePool>,
    dpb: Dpb,
    vps_map: HashMap<u8, VideoParameterSet>,
    sps_map: HashMap<u32, SequenceParameterSet>,
    pps_map: HashMap<u32, PictureParameterSet>,
    active_sps_id: Option<u32>,
    active_pps_id: Option<u32>,
    output_queue: VecDeque<DecodedFrame>,
    cur_surface_idx: usize,
    is_10bit: bool,
}

impl VaapiHevcDecoder {
    /// Initializes a new hardware HEVC decoder using an auto-detected DRM device.
    pub fn new(is_10bit: bool) -> Result<Self, HwError> {
        let device = VaDevice::open_auto_hevc(is_10bit)?;
        Ok(Self {
            device,
            pool: None,
            dpb: Dpb::new(16),
            vps_map: HashMap::new(),
            sps_map: HashMap::new(),
            pps_map: HashMap::new(),
            active_sps_id: None,
            active_pps_id: None,
            output_queue: VecDeque::new(),
            cur_surface_idx: 0,
            is_10bit,
        })
    }

    /// Access the underlying SurfacePool for zero-copy DMA-BUF export.
    pub fn surface_pool(&self) -> Option<&SurfacePool> {
        self.pool.as_ref()
    }
}

impl HwDecoder for VaapiHevcDecoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        let nalu_slices: Vec<&[u8]> = if packet.data.starts_with(&[0, 0, 0, 1]) || packet.data.starts_with(&[0, 0, 1]) {
            let units = split_annex_b_nalus(&packet.data);
            units.into_iter().map(|u| u.data).collect()
        } else {
            // Parse length-prefixed HVCC NAL units
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

        for nal in nalu_slices {
            if nal.len() < 2 {
                continue;
            }

            let header = match HevcNalHeader::parse(nal) {
                Ok(h) => h,
                Err(_) => continue,
            };

            let payload = &nal[2..];
            let rbsp = poppy_hevc::remove_emulation_prevention(payload);

            match header.nal_unit_type {
                HevcNalType::VpsNut => {
                    if let Ok(vps) = VideoParameterSet::parse(&rbsp) {
                        self.vps_map.insert(vps.vps_video_parameter_set_id, vps);
                    }
                }
                HevcNalType::SpsNut => {
                    if let Ok(sps) = SequenceParameterSet::parse(&rbsp) {
                        let sps_id = sps.sps_seq_parameter_set_id;
                        let coded_w = sps.pic_width_in_luma_samples;
                        let coded_h = sps.pic_height_in_luma_samples;
                        self.is_10bit = sps.is_10bit();
                        let max_dpb = (sps.max_dec_pic_buffering[0] as usize).max(1);
                        self.sps_map.insert(sps_id, sps);
                        self.active_sps_id = Some(sps_id);

                        let rt_format = if self.is_10bit {
                            VA_RT_FORMAT_YUV420_10
                        } else {
                            VA_RT_FORMAT_YUV420
                        };

                        let need_alloc = match &self.pool {
                            Some(p) => p.width != coded_w || p.height != coded_h,
                            None => true,
                        };

                        if need_alloc {
                            let pool = SurfacePool::new_with_format(
                                self.device.va.clone(),
                                self.device.display,
                                self.device.config_id,
                                coded_w,
                                coded_h,
                                16,
                                rt_format,
                            )?;
                            self.pool = Some(pool);
                            self.cur_surface_idx = 0;
                            self.dpb.max_ref_frames = max_dpb;
                            self.dpb.max_reorder_frames = max_dpb;
                        }
                    }
                }
                HevcNalType::PpsNut => {
                    if let Ok(pps) = PictureParameterSet::parse(&rbsp) {
                        let pps_id = pps.pps_pic_parameter_set_id;
                        self.pps_map.insert(pps_id, pps);
                        self.active_pps_id = Some(pps_id);
                    }
                }
                nut if nut.is_vcl() => {
                    let sps = match self.active_sps_id.and_then(|id| self.sps_map.get(&id)) {
                        Some(s) => s,
                        None => continue,
                    };
                    let pps = match self.active_pps_id.and_then(|id| self.pps_map.get(&id)) {
                        Some(p) => p,
                        None => continue,
                    };

                    let slice = match SliceSegmentHeader::parse(&rbsp, nut, sps, pps) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    let pool = match &self.pool {
                        Some(p) => p,
                        None => continue,
                    };

                    // Clear DPB on IDR
                    if nut.is_idr() {
                        let released = self.dpb.clear_on_idr();
                        for rf in released {
                            self.output_queue.push_back(DecodedFrame {
                                pts: rf.pts,
                                width: sps.cropped_width(),
                                height: sps.cropped_height(),
                                pixel_format: if self.is_10bit { PixelFormat::P010 } else { PixelFormat::Nv12 },
                                surface_id: rf.surface_id,
                                data: None,
                            });
                        }
                    }

                    // Pre-bump if DPB reaches max_dec_pic_buffering
                    let max_dpb = (sps.max_dec_pic_buffering[0] as usize).max(1);
                    while self.dpb.frames.len() >= max_dpb {
                        if let Some((min_idx, _)) = self.dpb.frames.iter().enumerate().min_by_key(|(_, f)| f.poc) {
                            let bumped = self.dpb.frames.remove(min_idx);
                            if bumped.needed_for_output {
                                self.output_queue.push_back(DecodedFrame {
                                    pts: bumped.pts,
                                    width: sps.cropped_width(),
                                    height: sps.cropped_height(),
                                    pixel_format: if self.is_10bit { PixelFormat::P010 } else { PixelFormat::Nv12 },
                                    surface_id: bumped.surface_id,
                                    data: None,
                                });
                            }
                        } else {
                            break;
                        }
                    }

                    // Select a free surface that is not currently held in the DPB
                    let target_surface = pool
                        .surfaces
                        .iter()
                        .copied()
                        .find(|&surf| !self.dpb.frames.iter().any(|f| f.surface_id == surf))
                        .unwrap_or(pool.surfaces[self.cur_surface_idx]);
                    self.cur_surface_idx = (self.cur_surface_idx + 1) % pool.surfaces.len();

                    let cur_poc = slice.slice_pic_order_cnt_lsb as i32;

                    // Build VAPictureParameterBufferHEVC
                    let mut pic_param: VAPictureParameterBufferHEVC = unsafe { std::mem::zeroed() };
                    pic_param.curr_pic = VAPictureHEVC {
                        picture_id: target_surface,
                        pic_order_cnt: cur_poc,
                        flags: 0,
                        va_reserved: [0; 4],
                    };

                    for slot in pic_param.reference_frames.iter_mut() {
                        *slot = VAPictureHEVC::default();
                    }

                    let max_refs = max_dpb.saturating_sub(1).min(15);
                    let mut ref_indices = Vec::new();

                    for (i, dpb_f) in self.dpb.frames.iter().take(max_refs).enumerate() {
                        let flags = if dpb_f.poc < cur_poc {
                            VA_PICTURE_HEVC_RPS_ST_CURR_BEFORE
                        } else {
                            VA_PICTURE_HEVC_RPS_ST_CURR_AFTER
                        };
                        pic_param.reference_frames[i] = VAPictureHEVC {
                            picture_id: dpb_f.surface_id,
                            pic_order_cnt: dpb_f.poc,
                            flags,
                            va_reserved: [0; 4],
                        };
                        ref_indices.push(i as u8);
                    }

                    pic_param.st_rps_bits = slice.st_rps_bits;
                    pic_param.sps_max_dec_pic_buffering_minus1 = sps.max_dec_pic_buffering[0].saturating_sub(1) as u8;
                    pic_param.pic_width_in_luma_samples = sps.pic_width_in_luma_samples as u16;
                    pic_param.pic_height_in_luma_samples = sps.pic_height_in_luma_samples as u16;
                    pic_param.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8 as u8;
                    pic_param.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8 as u8;
                    pic_param.pcm_sample_bit_depth_luma_minus1 = 0xff;
                    pic_param.pcm_sample_bit_depth_chroma_minus1 = 0xff;
                    pic_param.log2_min_pcm_luma_coding_block_size_minus3 = 0xfd;
                    pic_param.log2_max_pic_order_cnt_lsb_minus4 = sps.log2_max_pic_order_cnt_lsb_minus4 as u8;
                    pic_param.log2_min_luma_coding_block_size_minus3 = sps.log2_min_luma_coding_block_size_minus3 as u8;
                    pic_param.log2_diff_max_min_luma_coding_block_size = sps.log2_diff_max_min_luma_coding_block_size as u8;
                    pic_param.log2_min_transform_block_size_minus2 = sps.log2_min_luma_transform_block_size_minus2 as u8;
                    pic_param.log2_diff_max_min_transform_block_size = sps.log2_diff_max_min_luma_transform_block_size as u8;
                    pic_param.max_transform_hierarchy_depth_intra = sps.max_transform_hierarchy_depth_intra as u8;
                    pic_param.max_transform_hierarchy_depth_inter = sps.max_transform_hierarchy_depth_inter as u8;
                    pic_param.init_qp_minus26 = pps.init_qp_minus26 as i8;
                    pic_param.diff_cu_qp_delta_depth = pps.diff_cu_qp_delta_depth as u8;
                    pic_param.pps_cb_qp_offset = pps.pps_cb_qp_offset as i8;
                    pic_param.pps_cr_qp_offset = pps.pps_cr_qp_offset as i8;
                    pic_param.log2_parallel_merge_level_minus2 = pps.log2_parallel_merge_level_minus2 as u8;

                    let mut pic_fields = 0u32;
                    pic_fields |= sps.chroma_format_idc & 0x3;
                    if sps.separate_colour_plane_flag { pic_fields |= 1 << 2; }
                    if sps.pcm_enabled_flag { pic_fields |= 1 << 3; }
                    if sps.scaling_list_enabled_flag { pic_fields |= 1 << 4; }
                    if pps.transform_skip_enabled_flag { pic_fields |= 1 << 5; }
                    if sps.amp_enabled_flag { pic_fields |= 1 << 6; }
                    if sps.strong_intra_smoothing_enabled_flag { pic_fields |= 1 << 7; }
                    if pps.sign_data_hiding_enabled_flag { pic_fields |= 1 << 8; }
                    if pps.constrained_intra_pred_flag { pic_fields |= 1 << 9; }
                    if pps.cu_qp_delta_enabled_flag { pic_fields |= 1 << 10; }
                    if pps.weighted_pred_flag { pic_fields |= 1 << 11; }
                    if pps.weighted_bipred_flag { pic_fields |= 1 << 12; }
                    if pps.transquant_bypass_enabled_flag { pic_fields |= 1 << 13; }
                    if pps.tiles_enabled_flag { pic_fields |= 1 << 14; }
                    if pps.entropy_coding_sync_enabled_flag { pic_fields |= 1 << 15; }
                    if pps.pps_loop_filter_across_slices_enabled_flag { pic_fields |= 1 << 16; }
                    if pps.loop_filter_across_tiles_enabled_flag { pic_fields |= 1 << 17; }
                    pic_param.pic_fields = pic_fields;

                    let mut slice_parsing = 0u32;
                    if pps.lists_modification_present_flag { slice_parsing |= 1 << 0; }
                    if sps.long_term_ref_pics_present_flag { slice_parsing |= 1 << 1; }
                    if sps.sps_temporal_mvp_enabled_flag { slice_parsing |= 1 << 2; }
                    if pps.cabac_init_present_flag { slice_parsing |= 1 << 3; }
                    if pps.output_flag_present_flag { slice_parsing |= 1 << 4; }
                    if pps.dependent_slice_segments_enabled_flag { slice_parsing |= 1 << 5; }
                    if pps.pps_slice_chroma_qp_offsets_present_flag { slice_parsing |= 1 << 6; }
                    if sps.sample_adaptive_offset_enabled_flag { slice_parsing |= 1 << 7; }
                    if pps.deblocking_filter_override_enabled_flag { slice_parsing |= 1 << 8; }
                    if pps.pps_deblocking_filter_disabled_flag { slice_parsing |= 1 << 9; }
                    if pps.slice_segment_header_extension_present_flag { slice_parsing |= 1 << 10; }
                    if nut.is_irap() { slice_parsing |= 1 << 11; }
                    if nut.is_idr() { slice_parsing |= 1 << 12; }
                    if nut.is_irap() { slice_parsing |= 1 << 13; }
                    pic_param.slice_parsing_fields = slice_parsing;

                    // Build VASliceParameterBufferHEVC
                    let mut ref_pic_list = [[0xff; 15]; 2];
                    let is_intra = slice.slice_type == poppy_hevc::HevcSliceType::I;
                    if !is_intra {
                        let l0_count = (slice.num_ref_idx_l0_active as usize).min(ref_indices.len()).min(15);
                        ref_pic_list[0][..l0_count].copy_from_slice(&ref_indices[..l0_count]);
                        if slice.slice_type == poppy_hevc::HevcSliceType::B {
                            let l1_count = (slice.num_ref_idx_l1_active as usize).min(ref_indices.len()).min(15);
                            for (i, slot) in ref_pic_list[1].iter_mut().take(l1_count).enumerate() {
                                let rev_idx = ref_indices.len().saturating_sub(1).saturating_sub(i);
                                *slot = ref_indices[rev_idx];
                            }
                        }
                    }

                    let mut long_slice_flags = 1u32; // LastSliceOfPic
                    long_slice_flags |= ((slice.slice_type as u32) & 0x3) << 2;
                    if slice.slice_sao_luma_flag { long_slice_flags |= 1 << 6; }
                    if slice.slice_sao_chroma_flag { long_slice_flags |= 1 << 7; }
                    if slice.slice_temporal_mvp_enabled_flag { long_slice_flags |= 1 << 10; }
                    long_slice_flags |= 1 << 12; // collocated_from_l0_flag
                    if pps.pps_loop_filter_across_slices_enabled_flag { long_slice_flags |= 1 << 13; }

                    let (num_l0, num_l1) = if is_intra {
                        (0, 0)
                    } else if slice.slice_type == poppy_hevc::HevcSliceType::B {
                        (
                            slice.num_ref_idx_l0_active.saturating_sub(1) as u8,
                            slice.num_ref_idx_l1_active.saturating_sub(1) as u8,
                        )
                    } else {
                        (
                            slice.num_ref_idx_l0_active.saturating_sub(1) as u8,
                            0,
                        )
                    };

                    let collocated_ref_idx = if slice.slice_temporal_mvp_enabled_flag && !is_intra && ref_pic_list[0][0] != 0xff {
                        0
                    } else {
                        0xff
                    };

                    let mut slice_param: VASliceParameterBufferHEVC = unsafe { std::mem::zeroed() };
                    slice_param.slice_data_size = nal.len() as u32;
                    slice_param.slice_data_offset = 0;
                    slice_param.slice_data_flag = 0;
                    slice_param.slice_data_byte_offset = slice.slice_data_byte_offset as u32;
                    slice_param.slice_segment_address = 0;
                    slice_param.ref_pic_list = ref_pic_list;
                    slice_param.long_slice_flags = long_slice_flags;
                    slice_param.collocated_ref_idx = collocated_ref_idx;
                    slice_param.num_ref_idx_l0_active_minus1 = num_l0;
                    slice_param.num_ref_idx_l1_active_minus1 = num_l1;
                    slice_param.slice_qp_delta = slice.slice_qp_delta as i8;

                    // Submit to VA-API hardware pipeline
                    unsafe {
                        let va = &self.device.va;
                        let dpy = self.device.display;
                        let ctx = pool.context_id;

                        let status = (va.vaBeginPicture)(dpy, ctx, target_surface);
                        if status != VA_STATUS_SUCCESS {
                            return Err(HwError::DecodeError(format!("vaBeginPicture: {}", va.err_str(status))));
                        }

                        let mut pic_param_buf = 0;
                        let status = (va.vaCreateBuffer)(
                            dpy,
                            ctx,
                            VA_PICTURE_PARAMETER_BUFFER_TYPE,
                            std::mem::size_of::<VAPictureParameterBufferHEVC>() as u32,
                            1,
                            &mut pic_param as *mut _ as *mut std::ffi::c_void,
                            &mut pic_param_buf,
                        );
                        if status != VA_STATUS_SUCCESS {
                            return Err(HwError::DecodeError(format!("vaCreateBuffer(pic_param): {}", va.err_str(status))));
                        }

                        let scaling_list_needed = sps.scaling_list_enabled_flag || pps.pps_scaling_list_data_present_flag;
                        let mut iq_param_buf = 0;
                        if scaling_list_needed {
                            let mut iq_matrix = VAIQMatrixBufferHEVC::default();
                            let status = (va.vaCreateBuffer)(
                                dpy,
                                ctx,
                                VA_IQMATRIX_BUFFER_TYPE,
                                std::mem::size_of::<VAIQMatrixBufferHEVC>() as u32,
                                1,
                                &mut iq_matrix as *mut _ as *mut std::ffi::c_void,
                                &mut iq_param_buf,
                            );
                            if status != VA_STATUS_SUCCESS {
                                let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                                return Err(HwError::DecodeError(format!("vaCreateBuffer(iq_param): {}", va.err_str(status))));
                            }
                        }

                        let mut slice_param_buf = 0;
                        let status = (va.vaCreateBuffer)(
                            dpy,
                            ctx,
                            VA_SLICE_PARAMETER_BUFFER_TYPE,
                            std::mem::size_of::<VASliceParameterBufferHEVC>() as u32,
                            1,
                            &mut slice_param as *mut _ as *mut std::ffi::c_void,
                            &mut slice_param_buf,
                        );
                        if status != VA_STATUS_SUCCESS {
                            let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                            if iq_param_buf != 0 {
                                let _ = (va.vaDestroyBuffer)(dpy, iq_param_buf);
                            }
                            return Err(HwError::DecodeError(format!("vaCreateBuffer(slice_param): {}", va.err_str(status))));
                        }

                        let mut slice_data_buf = 0;
                        let status = (va.vaCreateBuffer)(
                            dpy,
                            ctx,
                            VA_SLICE_DATA_BUFFER_TYPE,
                            nal.len() as u32,
                            1,
                            nal.as_ptr() as *mut std::ffi::c_void,
                            &mut slice_data_buf,
                        );
                        if status != VA_STATUS_SUCCESS {
                            let _ = (va.vaDestroyBuffer)(dpy, pic_param_buf);
                            if iq_param_buf != 0 {
                                let _ = (va.vaDestroyBuffer)(dpy, iq_param_buf);
                            }
                            let _ = (va.vaDestroyBuffer)(dpy, slice_param_buf);
                            return Err(HwError::DecodeError(format!("vaCreateBuffer(slice_data): {}", va.err_str(status))));
                        }

                        let mut bufs = Vec::with_capacity(4);
                        bufs.push(pic_param_buf);
                        if iq_param_buf != 0 {
                            bufs.push(iq_param_buf);
                        }
                        bufs.push(slice_param_buf);
                        bufs.push(slice_data_buf);

                        let status = (va.vaRenderPicture)(dpy, ctx, bufs.as_mut_ptr(), bufs.len() as i32);
                        for b in &bufs {
                            let _ = (va.vaDestroyBuffer)(dpy, *b);
                        }

                        if status != VA_STATUS_SUCCESS {
                            return Err(HwError::DecodeError(format!("vaRenderPicture: {}", va.err_str(status))));
                        }

                        let status = (va.vaEndPicture)(dpy, ctx);
                        if status != VA_STATUS_SUCCESS {
                            return Err(HwError::DecodeError(format!("vaEndPicture: {}", va.err_str(status))));
                        }
                    }

                    let released = self.dpb.insert(DpbFrame {
                        surface_id: target_surface,
                        frame_num: 0,
                        poc: cur_poc,
                        is_reference: true,
                        is_long_term: false,
                        pts: packet.pts.or(Some(packet.dts)),
                        needed_for_output: true,
                    });

                    for rf in released {
                        self.output_queue.push_back(DecodedFrame {
                            pts: rf.pts,
                            width: sps.cropped_width(),
                            height: sps.cropped_height(),
                            pixel_format: if self.is_10bit { PixelFormat::P010 } else { PixelFormat::Nv12 },
                            surface_id: rf.surface_id,
                            data: None,
                        });
                    }

                    if sps.max_dec_pic_buffering[0] <= 1 {
                        self.output_queue.push_back(DecodedFrame {
                            pts: packet.pts.or(Some(packet.dts)),
                            width: sps.cropped_width(),
                            height: sps.cropped_height(),
                            pixel_format: if self.is_10bit { PixelFormat::P010 } else { PixelFormat::Nv12 },
                            surface_id: target_surface,
                            data: None,
                        });
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
        let remaining = self.dpb.drain();
        for rf in remaining {
            self.output_queue.push_back(DecodedFrame {
                pts: rf.pts,
                width: 1920,
                height: 1080,
                pixel_format: if self.is_10bit { PixelFormat::P010 } else { PixelFormat::Nv12 },
                surface_id: rf.surface_id,
                data: None,
            });
        }
        Ok(())
    }
}
