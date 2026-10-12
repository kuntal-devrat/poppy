//! VA-API H.265 / HEVC decode submission engine.
//!
//! Submits parsed HEVC parameter buffers (VPS, SPS, PPS, Slice) directly to the GPU's
//! fixed-function hardware video decode ASIC.

use std::collections::{HashMap, VecDeque};

use poppy_core::{InterlaceMode, Packet, PixelFormat};
use poppy_hevc::{
    HevcNalHeader, HevcNalType, PictureParameterSet, SequenceParameterSet, SliceSegmentHeader,
    VideoParameterSet, split_annex_b_nalus,
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
    prev_poc_msb: i32,
    prev_poc_lsb: i32,
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
            prev_poc_msb: 0,
            prev_poc_lsb: 0,
        })
    }

    /// Access the underlying SurfacePool for zero-copy DMA-BUF export.
    pub fn surface_pool(&self) -> Option<&SurfacePool> {
        self.pool.as_ref()
    }
}

impl HwDecoder for VaapiHevcDecoder {
    fn submit_packet(&mut self, packet: &Packet) -> Result<(), HwError> {
        let nalu_slices: Vec<&[u8]> = if packet.data.starts_with(&[0, 0, 0, 1])
            || packet.data.starts_with(&[0, 0, 1])
        {
            let units = split_annex_b_nalus(&packet.data);
            units.into_iter().map(|u| u.data).collect()
        } else {
            // Parse length-prefixed HVCC NAL units
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

        let mut vcl_slices = Vec::new();

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
                        let max_dec_pic = sps.max_dec_pic_buffering[0];
                        let max_dpb = (max_dec_pic as usize).max(1);
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
                            self.dpb.max_reorder_frames =
                                if max_dec_pic <= 1 { 0 } else { max_dpb };
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

                    match SliceSegmentHeader::parse(&rbsp, nut, sps, pps) {
                        Ok(slice) => {
                            vcl_slices.push((nal, nut, slice));
                        }
                        Err(e) => {
                            eprintln!(
                                "VA-API HEVC slice parse warning: {e} (error concealment active)"
                            );
                        }
                    }
                }
                _ => {}
            }
        }

        // Multi-slice picture batching: group slice segments belonging to the same picture
        let mut picture_batches: Vec<Vec<(&[u8], HevcNalType, SliceSegmentHeader)>> = Vec::new();
        for item in vcl_slices {
            let is_new_picture = match picture_batches.last() {
                None => true,
                Some(current_batch) => {
                    let (_, prev_nut, prev_slice) = &current_batch[0];
                    item.2.first_slice_segment_in_pic_flag
                        || item.2.slice_pic_order_cnt_lsb != prev_slice.slice_pic_order_cnt_lsb
                        || item.1.is_idr() != prev_nut.is_idr()
                }
            };
            if is_new_picture {
                picture_batches.push(vec![item]);
            } else {
                picture_batches.last_mut().unwrap().push(item);
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

            let (_first_nal, first_nut, first_slice) = &slices[0];

            // Clear DPB on IDR
            if first_nut.is_idr() {
                self.prev_poc_msb = 0;
                self.prev_poc_lsb = 0;
                let released = self.dpb.clear_on_idr();
                for rf in released {
                    let w = sps.cropped_width();
                    let h = sps.cropped_height();
                    let dmabuf = pool.export_dmabuf_descriptor(rf.surface_id).ok();
                    let data = if dmabuf.is_none() {
                        pool.read_surface_data(rf.surface_id, w, h).ok()
                    } else {
                        None
                    }; // lazy: skip CPU readback on zero-copy path
                    self.output_queue.push_back(DecodedFrame {
                        pts: rf.pts,
                        width: w,
                        height: h,
                        pixel_format: if self.is_10bit {
                            PixelFormat::P010
                        } else {
                            PixelFormat::Nv12
                        },
                        surface_id: rf.surface_id,
                        data,
                        interlace_mode: InterlaceMode::Progressive,
                        dmabuf,
                        dxgi_handle: None,
                        cv_pixel_buffer: None,
                        is_concealed: false,
                    });
                }
            }

            // Pre-bump if DPB reaches max_dec_pic_buffering.
            // Respect reference status: evict non-reference, already-output frames first;
            // only bump needed_for_output frames (in POC order) when forced.
            let max_dpb = (sps.max_dec_pic_buffering[0] as usize).max(1);
            while self.dpb.frames.len() >= max_dpb {
                // 1. Drop non-reference, already-output frames (no output needed).
                if let Some(idx) = self
                    .dpb
                    .frames
                    .iter()
                    .position(|f| !f.is_reference && !f.needed_for_output)
                {
                    self.dpb.frames.remove(idx);
                    continue;
                }
                // 2. Bump oldest needed_for_output frame (min POC).
                if let Some((min_idx, _)) = self
                    .dpb
                    .frames
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.needed_for_output)
                    .min_by_key(|(_, f)| f.poc)
                {
                    let bumped = self.dpb.frames.remove(min_idx);
                    let dmabuf = pool.export_dmabuf_descriptor(bumped.surface_id).ok();
                    let data = if dmabuf.is_none() {
                        pool.read_surface_data(bumped.surface_id, bumped.width, bumped.height)
                            .ok()
                    } else {
                        None
                    }; // lazy zero-copy
                    self.output_queue.push_back(DecodedFrame {
                        pts: bumped.pts,
                        width: bumped.width,
                        height: bumped.height,
                        pixel_format: if self.is_10bit {
                            PixelFormat::P010
                        } else {
                            PixelFormat::Nv12
                        },
                        surface_id: bumped.surface_id,
                        data,
                        interlace_mode: bumped.interlace_mode,
                        dmabuf,
                        dxgi_handle: None,
                        cv_pixel_buffer: None,
                        is_concealed: bumped.is_concealed,
                    });
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

            let max_poc_lsb = 1i32 << (sps.log2_max_pic_order_cnt_lsb_minus4 + 4);
            let poc_lsb = first_slice.slice_pic_order_cnt_lsb as i32;
            let (poc_msb, cur_poc) = if first_nut.is_idr() {
                self.prev_poc_msb = 0;
                self.prev_poc_lsb = 0;
                (0, 0)
            } else {
                let msb = if (poc_lsb < self.prev_poc_lsb)
                    && (self.prev_poc_lsb - poc_lsb >= max_poc_lsb / 2)
                {
                    self.prev_poc_msb + max_poc_lsb
                } else if (poc_lsb > self.prev_poc_lsb)
                    && (poc_lsb - self.prev_poc_lsb > max_poc_lsb / 2)
                {
                    self.prev_poc_msb - max_poc_lsb
                } else {
                    self.prev_poc_msb
                };
                (msb, msb + poc_lsb)
            };
            self.prev_poc_msb = poc_msb;
            self.prev_poc_lsb = poc_lsb;

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
                let flags = VA_PICTURE_HEVC_SHORT_TERM_REFERENCE
                    | if dpb_f.poc < cur_poc {
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

            let is_intra = first_slice.slice_type == poppy_hevc::HevcSliceType::I;
            let missing_refs = !is_intra && self.dpb.frames.is_empty();
            let mut is_concealed = missing_refs;

            pic_param.st_rps_bits = first_slice.st_rps_bits;
            pic_param.sps_max_dec_pic_buffering_minus1 =
                sps.max_dec_pic_buffering[0].saturating_sub(1) as u8;
            pic_param.pic_width_in_luma_samples = sps.pic_width_in_luma_samples as u16;
            pic_param.pic_height_in_luma_samples = sps.pic_height_in_luma_samples as u16;
            pic_param.bit_depth_luma_minus8 = sps.bit_depth_luma_minus8 as u8;
            pic_param.bit_depth_chroma_minus8 = sps.bit_depth_chroma_minus8 as u8;
            pic_param.pcm_sample_bit_depth_luma_minus1 = 0;
            pic_param.pcm_sample_bit_depth_chroma_minus1 = 0;
            pic_param.log2_min_pcm_luma_coding_block_size_minus3 = 0;
            pic_param.log2_diff_max_min_pcm_luma_coding_block_size = 0;
            pic_param.log2_max_pic_order_cnt_lsb_minus4 =
                sps.log2_max_pic_order_cnt_lsb_minus4 as u8;
            pic_param.log2_min_luma_coding_block_size_minus3 =
                sps.log2_min_luma_coding_block_size_minus3 as u8;
            pic_param.log2_diff_max_min_luma_coding_block_size =
                sps.log2_diff_max_min_luma_coding_block_size as u8;
            pic_param.log2_min_transform_block_size_minus2 =
                sps.log2_min_luma_transform_block_size_minus2 as u8;
            pic_param.log2_diff_max_min_transform_block_size =
                sps.log2_diff_max_min_luma_transform_block_size as u8;
            pic_param.max_transform_hierarchy_depth_intra =
                sps.max_transform_hierarchy_depth_intra as u8;
            pic_param.max_transform_hierarchy_depth_inter =
                sps.max_transform_hierarchy_depth_inter as u8;
            pic_param.init_qp_minus26 = pps.init_qp_minus26 as i8;
            pic_param.diff_cu_qp_delta_depth = pps.diff_cu_qp_delta_depth as u8;
            pic_param.pps_cb_qp_offset = pps.pps_cb_qp_offset as i8;
            pic_param.pps_cr_qp_offset = pps.pps_cr_qp_offset as i8;
            pic_param.log2_parallel_merge_level_minus2 = pps.log2_parallel_merge_level_minus2 as u8;

            let mut pic_fields = 0u32;
            pic_fields |= sps.chroma_format_idc & 0x3;
            if sps.separate_colour_plane_flag {
                pic_fields |= 1 << 2;
            }
            if sps.pcm_enabled_flag {
                pic_fields |= 1 << 3;
            }
            if sps.scaling_list_enabled_flag {
                pic_fields |= 1 << 4;
            }
            if pps.transform_skip_enabled_flag {
                pic_fields |= 1 << 5;
            }
            if sps.amp_enabled_flag {
                pic_fields |= 1 << 6;
            }
            if sps.strong_intra_smoothing_enabled_flag {
                pic_fields |= 1 << 7;
            }
            if pps.sign_data_hiding_enabled_flag {
                pic_fields |= 1 << 8;
            }
            if pps.constrained_intra_pred_flag {
                pic_fields |= 1 << 9;
            }
            if pps.cu_qp_delta_enabled_flag {
                pic_fields |= 1 << 10;
            }
            if pps.weighted_pred_flag {
                pic_fields |= 1 << 11;
            }
            if pps.weighted_bipred_flag {
                pic_fields |= 1 << 12;
            }
            if pps.transquant_bypass_enabled_flag {
                pic_fields |= 1 << 13;
            }
            if pps.tiles_enabled_flag {
                pic_fields |= 1 << 14;
            }
            if pps.entropy_coding_sync_enabled_flag {
                pic_fields |= 1 << 15;
            }
            if pps.pps_loop_filter_across_slices_enabled_flag {
                pic_fields |= 1 << 16;
            }
            if pps.loop_filter_across_tiles_enabled_flag {
                pic_fields |= 1 << 17;
            }
            if sps.max_num_reorder_pics[0] == 0 {
                pic_fields |= 1 << 19; // NoPicReorderingFlag
            }
            pic_param.pic_fields = pic_fields;

            let mut slice_parsing_fields = 0u32;
            if pps.lists_modification_present_flag {
                slice_parsing_fields |= 1 << 0;
            }
            if sps.long_term_ref_pics_present_flag {
                slice_parsing_fields |= 1 << 1;
            }
            if sps.sps_temporal_mvp_enabled_flag {
                slice_parsing_fields |= 1 << 2;
            }
            if pps.cabac_init_present_flag {
                slice_parsing_fields |= 1 << 3;
            }
            if pps.output_flag_present_flag {
                slice_parsing_fields |= 1 << 4;
            }
            if pps.dependent_slice_segments_enabled_flag {
                slice_parsing_fields |= 1 << 5;
            }
            if pps.pps_slice_chroma_qp_offsets_present_flag {
                slice_parsing_fields |= 1 << 6;
            }
            if sps.sample_adaptive_offset_enabled_flag {
                slice_parsing_fields |= 1 << 7;
            }
            if pps.deblocking_filter_override_enabled_flag {
                slice_parsing_fields |= 1 << 8;
            }
            if pps.pps_deblocking_filter_disabled_flag {
                slice_parsing_fields |= 1 << 9;
            }
            if pps.slice_segment_header_extension_present_flag {
                slice_parsing_fields |= 1 << 10;
            }
            if first_nut.is_irap() {
                slice_parsing_fields |= 1 << 11; // RapPicFlag
            }
            if first_nut.is_idr() {
                slice_parsing_fields |= 1 << 12; // IdrPicFlag
            }
            if is_intra {
                slice_parsing_fields |= 1 << 13; // IntraPicFlag
            }
            pic_param.slice_parsing_fields = slice_parsing_fields;

            pic_param.num_tile_columns_minus1 = pps.num_tile_columns_minus1 as u8;
            pic_param.num_tile_rows_minus1 = pps.num_tile_rows_minus1 as u8;
            for (idx, &w) in pps.column_width_minus1.iter().enumerate() {
                if idx < 19 {
                    pic_param.column_width_minus1[idx] = w as u16;
                }
            }
            for (idx, &h) in pps.row_height_minus1.iter().enumerate() {
                if idx < 21 {
                    pic_param.row_height_minus1[idx] = h as u16;
                }
            }
            pic_param.num_short_term_ref_pic_sets = sps.short_term_ref_pic_sets.len() as u8;
            pic_param.num_long_term_ref_pic_sps = 0;
            pic_param.num_ref_idx_l0_default_active_minus1 =
                pps.num_ref_idx_l0_default_active_minus1 as u8;
            pic_param.num_ref_idx_l1_default_active_minus1 =
                pps.num_ref_idx_l1_default_active_minus1 as u8;
            pic_param.num_extra_slice_header_bits = pps.num_extra_slice_header_bits;
            pic_param.pps_beta_offset_div2 = pps.pps_beta_offset_div2 as i8;
            pic_param.pps_tc_offset_div2 = pps.pps_tc_offset_div2 as i8;

            let mut ref_pic_list = [[0xffu8; 15]; 2];
            if !is_intra {
                for (slot_idx, slot) in ref_pic_list[0].iter_mut().enumerate() {
                    if slot_idx < ref_indices.len() {
                        *slot = ref_indices[slot_idx];
                    }
                }
                if first_slice.slice_type == poppy_hevc::HevcSliceType::B {
                    for (slot_idx, slot) in ref_pic_list[1].iter_mut().enumerate() {
                        if slot_idx < ref_indices.len() {
                            *slot = ref_indices[slot_idx];
                        }
                    }
                }
            }

            // Submit to VA-API hardware pipeline with multi-slice batching
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

                let mut bufs_to_render = Vec::with_capacity(2 + slices.len() * 2);

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
                    return Err(HwError::DecodeError(format!(
                        "vaCreateBuffer(pic_param): {}",
                        va.err_str(status)
                    )));
                }
                bufs_to_render.push(pic_param_buf);

                let scaling_list_needed =
                    sps.scaling_list_enabled_flag || pps.pps_scaling_list_data_present_flag;
                if scaling_list_needed {
                    let mut iq_matrix = VAIQMatrixBufferHEVC::default();
                    let mut iq_param_buf = 0;
                    let status = (va.vaCreateBuffer)(
                        dpy,
                        ctx,
                        VA_IQMATRIX_BUFFER_TYPE,
                        std::mem::size_of::<VAIQMatrixBufferHEVC>() as u32,
                        1,
                        &mut iq_matrix as *mut _ as *mut std::ffi::c_void,
                        &mut iq_param_buf,
                    );
                    if status == VA_STATUS_SUCCESS {
                        bufs_to_render.push(iq_param_buf);
                    }
                }

                // Append each slice segment parameter & data buffer
                for (slice_idx, (nal, _nut, slice)) in slices.iter().enumerate() {
                    let mut long_slice_flags = 0u32;
                    if slice_idx == slices.len() - 1 {
                        long_slice_flags |= 1 << 0; // LastSliceOfPic
                    }
                    if slice.dependent_slice_segment_flag {
                        long_slice_flags |= 1 << 1; // dependent_slice_segment_flag
                    }
                    long_slice_flags |= (slice.slice_type as u32 & 0x3) << 2; // slice_type
                    if slice.slice_sao_luma_flag {
                        long_slice_flags |= 1 << 6;
                    }
                    if slice.slice_sao_chroma_flag {
                        long_slice_flags |= 1 << 7;
                    }
                    if slice.mvd_l1_zero_flag {
                        long_slice_flags |= 1 << 8;
                    }
                    if slice.cabac_init_flag {
                        long_slice_flags |= 1 << 9;
                    }
                    if slice.slice_temporal_mvp_enabled_flag {
                        long_slice_flags |= 1 << 10;
                    }
                    if slice.slice_deblocking_filter_disabled_flag {
                        long_slice_flags |= 1 << 11;
                    }
                    if slice.collocated_from_l0_flag {
                        long_slice_flags |= 1 << 12; // collocated_from_l0_flag
                    }
                    if slice.slice_loop_filter_across_slices_enabled_flag {
                        long_slice_flags |= 1 << 13;
                    }

                    let (num_l0, num_l1) = if is_intra {
                        (0, 0)
                    } else if slice.slice_type == poppy_hevc::HevcSliceType::B {
                        (
                            slice.num_ref_idx_l0_active.saturating_sub(1) as u8,
                            slice.num_ref_idx_l1_active.saturating_sub(1) as u8,
                        )
                    } else {
                        (slice.num_ref_idx_l0_active.saturating_sub(1) as u8, 0)
                    };

                    let collocated_ref_idx = if slice.slice_temporal_mvp_enabled_flag
                        && !is_intra
                        && ref_pic_list[0][0] != 0xff
                    {
                        slice.collocated_ref_idx
                    } else {
                        0xff
                    };

                    let mut slice_param: VASliceParameterBufferHEVC = std::mem::zeroed();
                    slice_param.slice_data_size = nal.len() as u32;
                    slice_param.slice_data_offset = 0;
                    slice_param.slice_data_flag = 0;
                    slice_param.slice_data_byte_offset = slice.slice_data_byte_offset as u32;
                    slice_param.slice_segment_address = slice.slice_segment_address;
                    slice_param.ref_pic_list = ref_pic_list;
                    slice_param.long_slice_flags = long_slice_flags;
                    slice_param.collocated_ref_idx = collocated_ref_idx;
                    slice_param.num_ref_idx_l0_active_minus1 = num_l0;
                    slice_param.num_ref_idx_l1_active_minus1 = num_l1;
                    slice_param.slice_qp_delta = slice.slice_qp_delta as i8;
                    slice_param.five_minus_max_num_merge_cand = if !is_intra {
                        slice.five_minus_max_num_merge_cand
                    } else {
                        0
                    };
                    // Prediction weights (previously parsed-and-discarded; now mapped).
                    if let Some(pw) = &slice.pred_weights {
                        slice_param.luma_log2_weight_denom = pw.luma_log2_weight_denom.min(7) as u8;
                        slice_param.delta_chroma_log2_weight_denom =
                            pw.delta_chroma_log2_weight_denom.clamp(-7, 7) as i8;
                        for i in 0..(slice.num_ref_idx_l0_active.min(15) as usize) {
                            if pw.luma_weight_l0_flag.get(i).copied().unwrap_or(false) {
                                slice_param.delta_luma_weight_l0[i] =
                                    pw.delta_luma_weight_l0.get(i).copied().unwrap_or(0).clamp(-127, 127) as i8;
                                slice_param.luma_offset_l0[i] =
                                    pw.luma_offset_l0.get(i).copied().unwrap_or(0).clamp(-127, 127) as i8;
                            }
                            if pw.chroma_weight_l0_flag.get(i).copied().unwrap_or(false) {
                                for c in 0..2 {
                                    slice_param.delta_chroma_weight_l0[i][c] = pw
                                        .delta_chroma_weight_l0
                                        .get(i)
                                        .map(|a| a[c])
                                        .unwrap_or(0)
                                        .clamp(-127, 127)
                                        as i8;
                                    slice_param.chroma_offset_l0[i][c] = pw
                                        .chroma_offset_l0
                                        .get(i)
                                        .map(|a| a[c])
                                        .unwrap_or(0)
                                        .clamp(-127, 127)
                                        as i8;
                                }
                            }
                        }
                        for i in 0..(slice.num_ref_idx_l1_active.min(15) as usize) {
                            if pw.luma_weight_l1_flag.get(i).copied().unwrap_or(false) {
                                slice_param.delta_luma_weight_l1[i] =
                                    pw.delta_luma_weight_l1.get(i).copied().unwrap_or(0).clamp(-127, 127) as i8;
                                slice_param.luma_offset_l1[i] =
                                    pw.luma_offset_l1.get(i).copied().unwrap_or(0).clamp(-127, 127) as i8;
                            }
                            if pw.chroma_weight_l1_flag.get(i).copied().unwrap_or(false) {
                                for c in 0..2 {
                                    slice_param.delta_chroma_weight_l1[i][c] = pw
                                        .delta_chroma_weight_l1
                                        .get(i)
                                        .map(|a| a[c])
                                        .unwrap_or(0)
                                        .clamp(-127, 127)
                                        as i8;
                                    slice_param.chroma_offset_l1[i][c] = pw
                                        .chroma_offset_l1
                                        .get(i)
                                        .map(|a| a[c])
                                        .unwrap_or(0)
                                        .clamp(-127, 127)
                                        as i8;
                                }
                            }
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
                    if status == VA_STATUS_SUCCESS {
                        bufs_to_render.push(slice_param_buf);
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
                    if status == VA_STATUS_SUCCESS {
                        bufs_to_render.push(slice_data_buf);
                    }
                }

                let status = (va.vaRenderPicture)(
                    dpy,
                    ctx,
                    bufs_to_render.as_mut_ptr(),
                    bufs_to_render.len() as i32,
                );
                for b in &bufs_to_render {
                    let _ = (va.vaDestroyBuffer)(dpy, *b);
                }

                if status != VA_STATUS_SUCCESS {
                    return Err(HwError::DecodeError(format!(
                        "vaRenderPicture: {}",
                        va.err_str(status)
                    )));
                }

                let end_status = (va.vaEndPicture)(dpy, ctx);
                if end_status != VA_STATUS_SUCCESS {
                    let err_msg = va.err_str(end_status);
                    if end_status == crate::vaapi::ffi::VA_STATUS_ERROR_DECODING_ERROR {
                        is_concealed = true;
                        eprintln!(
                            "VA-API HEVC warning: {} (concealed corrupted frame)",
                            err_msg
                        );
                    } else {
                        return Err(HwError::DecodeError(format!("vaEndPicture: {}", err_msg)));
                    }
                }

                let mut sync_status = (va.vaSyncSurface)(dpy, target_surface);
                let mut retries = 0;
                while sync_status == crate::vaapi::ffi::VA_STATUS_ERROR_HW_BUSY && retries < 50 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    sync_status = (va.vaSyncSurface)(dpy, target_surface);
                    retries += 1;
                }
                if sync_status != VA_STATUS_SUCCESS {
                    let err_msg = va.err_str(sync_status);
                    if sync_status == crate::vaapi::ffi::VA_STATUS_ERROR_DECODING_ERROR {
                        is_concealed = true;
                        eprintln!(
                            "VA-API HEVC warning: {} (concealed corrupted frame)",
                            err_msg
                        );
                    } else {
                        return Err(HwError::DecodeError(format!(
                            "vaSyncSurface failed: {}",
                            err_msg
                        )));
                    }
                }
            }

            // Non-reference pictures (TRAIL_N/TSA_N/STSA_N/RADL_N/RASL_N) must not be retained as references.
            let is_ref = first_nut.is_reference();
            let released = self.dpb.insert(DpbFrame {
                surface_id: target_surface,
                frame_num: 0,
                poc: cur_poc,
                is_reference: is_ref,
                is_long_term: false,
                pts: packet.pts.or(Some(packet.dts)),
                needed_for_output: true,
                width: sps.cropped_width(),
                height: sps.cropped_height(),
                interlace_mode: InterlaceMode::Progressive,
                is_concealed,
            });

            for rf in released {
                let dmabuf = pool.export_dmabuf_descriptor(rf.surface_id).ok();
                let data = if dmabuf.is_none() {
                    pool.read_surface_data(rf.surface_id, rf.width, rf.height)
                        .ok()
                } else {
                    None
                }; // lazy zero-copy
                self.output_queue.push_back(DecodedFrame {
                    pts: rf.pts,
                    width: rf.width,
                    height: rf.height,
                    pixel_format: if self.is_10bit {
                        PixelFormat::P010
                    } else {
                        PixelFormat::Nv12
                    },
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
            let data = if dmabuf.is_none() {
                self.pool
                    .as_ref()
                    .and_then(|p| p.read_surface_data(rf.surface_id, rf.width, rf.height).ok())
            } else {
                None
            }; // lazy zero-copy
            self.output_queue.push_back(DecodedFrame {
                pts: rf.pts,
                width: rf.width,
                height: rf.height,
                pixel_format: if self.is_10bit {
                    PixelFormat::P010
                } else {
                    PixelFormat::Nv12
                },
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
