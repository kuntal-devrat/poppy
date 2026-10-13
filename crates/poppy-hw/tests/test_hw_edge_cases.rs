use bytes::Bytes;
use poppy_core::Packet;
use poppy_h264::sps::Sps;
use poppy_hevc::pps::PictureParameterSet;
use poppy_hevc::slice::{HevcSliceType, SliceSegmentHeader};
use poppy_hevc::sps::SequenceParameterSet;
use poppy_hw::{DecodedFrame, DmaBufDescriptor, HwDecoder, HwError};

struct BitWriter {
    bytes: Vec<u8>,
    cur_byte: u8,
    bits_in_cur: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            cur_byte: 0,
            bits_in_cur: 0,
        }
    }

    fn write_bit(&mut self, bit: bool) {
        self.cur_byte = (self.cur_byte << 1) | (if bit { 1 } else { 0 });
        self.bits_in_cur += 1;
        if self.bits_in_cur == 8 {
            self.bytes.push(self.cur_byte);
            self.cur_byte = 0;
            self.bits_in_cur = 0;
        }
    }

    fn write_bits(&mut self, val: u64, count: usize) {
        for i in (0..count).rev() {
            let bit = ((val >> i) & 1) != 0;
            self.write_bit(bit);
        }
    }

    fn write_ue(&mut self, val: u32) {
        let val_plus1 = (val as u64) + 1;
        let bits = 64 - val_plus1.leading_zeros() as usize;
        for _ in 0..(bits - 1) {
            self.write_bit(false);
        }
        self.write_bits(val_plus1, bits);
    }

    fn write_se(&mut self, val: i32) {
        let mapped = if val <= 0 {
            (-val * 2) as u32
        } else {
            (val * 2 - 1) as u32
        };
        self.write_ue(mapped);
    }

    fn into_bytes(mut self) -> Vec<u8> {
        if self.bits_in_cur > 0 {
            self.cur_byte <<= 8 - self.bits_in_cur;
            self.bytes.push(self.cur_byte);
        }
        self.bytes
    }
}

#[test]
fn test_h264_sps_change_midstream_resolution() {
    // SPS 1: 1920x1080
    let sps1_data = vec![
        0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x78, 0x02, 0x27, 0xE5, 0xC0, 0x5A, 0x80, 0x80,
        0x80, 0x82, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x00, 0x03, 0x00, 0x79, 0x1E, 0x2C, 0x5C,
        0x90,
    ];
    let sps1 = Sps::parse(&sps1_data).expect("parse SPS 1080p");
    assert_eq!(sps1.display_width(), 1920);
    assert_eq!(sps1.display_height(), 1080);
    assert_eq!(sps1.seq_parameter_set_id, 0);

    // SPS 2: 1280x720
    let sps2_data = vec![
        0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50, 0x05, 0xBB, 0x01, 0x6A, 0x02, 0x02, 0x02,
        0x80, 0x00, 0x00, 0x03, 0x00, 0x80, 0x00, 0x00, 0x1E, 0x47, 0x8C, 0x18, 0xC5,
    ];
    let sps2 = Sps::parse(&sps2_data).expect("parse SPS 720p");
    assert_eq!(sps2.display_width(), 1280);
    assert_eq!(sps2.display_height(), 720);

    // Verify resolution changed mid-stream
    assert_ne!(sps1.display_width(), sps2.display_width());
    assert_ne!(sps1.display_height(), sps2.display_height());
}

#[test]
fn test_h264_cabac_and_mbaff_interlace() {
    let mut bits = Vec::new();
    bits.push(0x67); // SPS header
    bits.extend_from_slice(&[0x64, 0x00, 0x28]); // Profile High, level 40

    let mut writer = BitWriter::new();
    writer.write_ue(0); // seq_parameter_set_id = 0
    writer.write_ue(1); // chroma_format_idc = 1 (4:2:0)
    writer.write_ue(0); // bit_depth_luma_minus8 = 0
    writer.write_ue(0); // bit_depth_chroma_minus8 = 0
    writer.write_bit(false); // qpprime_y_zero_transform_bypass_flag = 0
    writer.write_bit(false); // seq_scaling_matrix_present_flag = 0

    writer.write_ue(4); // log2_max_frame_num_minus4 = 4
    writer.write_ue(0); // pic_order_cnt_type = 0
    writer.write_ue(4); // log2_max_pic_order_cnt_lsb_minus4 = 4

    writer.write_ue(4); // max_num_ref_frames = 4
    writer.write_bit(false); // gaps_in_frame_num_value_allowed_flag = 0
    writer.write_ue(44); // pic_width_in_mbs_minus1 = 44 (720px)
    writer.write_ue(17); // pic_height_in_map_units_minus1 = 17

    writer.write_bit(false); // frame_mbs_only_flag = 0 (interlaced)
    writer.write_bit(true); // mb_adaptive_frame_field_flag = 1 (MBAFF)
    writer.write_bit(true); // direct_8x8_inference_flag = 1
    writer.write_bit(false); // frame_cropping_flag = 0
    writer.write_bit(false); // vui_parameters_present_flag = 0

    bits.extend_from_slice(&writer.into_bytes());

    let sps = Sps::parse(&bits).expect("parse MBAFF SPS");
    assert!(!sps.frame_mbs_only_flag);
    assert!(sps.mb_adaptive_frame_field_flag);

    // PPS with CABAC
    let mut pps_bits = vec![0x68];
    let mut pps_writer = BitWriter::new();
    pps_writer.write_ue(0); // pic_parameter_set_id = 0
    pps_writer.write_ue(0); // seq_parameter_set_id = 0
    pps_writer.write_bit(true); // entropy_coding_mode_flag = 1 (CABAC)
    pps_writer.write_bit(false); // bottom_field_pic_order_in_frame_present_flag = 0
    pps_writer.write_ue(0); // num_slice_groups_minus1 = 0
    pps_writer.write_ue(0); // num_ref_idx_l0_default_active_minus1 = 0
    pps_writer.write_ue(0); // num_ref_idx_l1_default_active_minus1 = 0
    pps_writer.write_bit(false); // weighted_pred_flag = 0
    pps_writer.write_bits(0, 2); // weighted_bipred_idc = 0
    pps_writer.write_se(0); // pic_init_qp_minus26 = 0
    pps_writer.write_se(0); // pic_init_qs_minus26 = 0
    pps_writer.write_se(0); // chroma_qp_index_offset = 0
    pps_writer.write_bit(true); // deblocking_filter_control_present_flag = 1
    pps_writer.write_bit(false); // constrained_intra_pred_flag = 0
    pps_writer.write_bit(false); // redundant_pic_cnt_present_flag = 0

    pps_bits.extend_from_slice(&pps_writer.into_bytes());

    let pps = poppy_h264::pps::Pps::parse(&pps_bits).expect("parse CABAC PPS");
    assert!(pps.entropy_coding_mode_flag);
}

#[test]
fn test_hevc_long_term_reference_picture_sets() {
    let mut w = BitWriter::new();
    w.write_bits(0, 4); // sps_video_parameter_set_id
    w.write_bits(0, 3); // sps_max_sub_layers_minus1
    w.write_bit(true); // sps_temporal_id_nesting_flag

    // profile_tier_level
    w.write_bits(0, 2);
    w.write_bit(false);
    w.write_bits(1, 5);
    w.write_bits(0, 32);
    w.write_bit(true);
    w.write_bit(false);
    w.write_bit(true);
    w.write_bit(true);
    w.write_bits(0, 44);
    w.write_bits(120, 8);

    w.write_ue(0); // sps_seq_parameter_set_id
    w.write_ue(1); // chroma_format_idc
    w.write_ue(1920); // pic_width
    w.write_ue(1080); // pic_height
    w.write_bit(false); // conformance_window_flag
    w.write_ue(0); // bit_depth_luma_minus8
    w.write_ue(0); // bit_depth_chroma_minus8
    w.write_ue(4); // log2_max_pic_order_cnt_lsb_minus4
    w.write_bit(true); // sub_layer_ordering_info
    w.write_ue(4);
    w.write_ue(2);
    w.write_ue(4);

    w.write_ue(0);
    w.write_ue(3);
    w.write_ue(0);
    w.write_ue(3);
    w.write_ue(1);
    w.write_ue(1);
    w.write_bit(false);
    w.write_bit(false);
    w.write_bit(true);
    w.write_bit(false);

    w.write_ue(0); // num_short_term_ref_pic_sets = 0
    w.write_bit(true); // long_term_ref_pics_present_flag = 1!
    w.write_ue(2); // num_long_term_ref_pics_sps = 2!
    w.write_bits(12, 8); // poc 12
    w.write_bit(true); // used
    w.write_bits(28, 8); // poc 28
    w.write_bit(false); // not used

    w.write_bit(true); // sps_temporal_mvp_enabled_flag
    w.write_bit(false); // strong_intra_smoothing_enabled_flag
    w.write_bit(false); // vui_parameters_present_flag

    let sps_bytes = w.into_bytes();
    let sps = SequenceParameterSet::parse(&sps_bytes).expect("parse HEVC SPS with long-term RPS");
    assert!(sps.long_term_ref_pics_present_flag);
    assert_eq!(sps.num_long_term_ref_pics_sps, 2);
    assert_eq!(sps.lt_ref_pic_poc_lsb_sps, vec![12, 28]);
    assert_eq!(sps.used_by_curr_pic_lt_sps_flag, vec![true, false]);

    // Construct PPS for slice parsing
    let mut pw = BitWriter::new();
    pw.write_ue(0); // pps_pic_parameter_set_id = 0
    pw.write_ue(0); // pps_seq_parameter_set_id = 0
    pw.write_bit(false); // dependent_slice_segments_enabled_flag = 0
    pw.write_bit(false); // output_flag_present_flag = 0
    pw.write_bits(0, 3); // num_extra_slice_header_bits = 0
    pw.write_bit(false); // sign_data_hiding_enabled_flag = 0
    pw.write_bit(false); // cabac_init_present_flag = 0
    pw.write_ue(0); // num_ref_idx_l0_default_active_minus1 = 0
    pw.write_ue(0); // num_ref_idx_l1_default_active_minus1 = 0
    pw.write_se(0); // init_qp_minus26 = 0
    pw.write_bit(false); // constrained_intra_pred_flag = 0
    pw.write_bit(false); // transform_skip_enabled_flag = 0
    pw.write_bit(false); // cu_qp_delta_enabled_flag = 0
    pw.write_se(0); // pps_cb_qp_offset = 0
    pw.write_se(0); // pps_cr_qp_offset = 0
    pw.write_bit(false); // slice_chroma_qp_offsets_present = 0
    pw.write_bit(false); // weighted_pred_flag = 0
    pw.write_bit(false); // weighted_bipred_flag = 0
    pw.write_bit(false); // transquant_bypass_enabled_flag = 0
    pw.write_bit(false); // tiles_enabled_flag = 0
    pw.write_bit(false); // entropy_coding_sync_enabled_flag = 0
    pw.write_bit(false); // pps_loop_filter_across_slices_enabled_flag = 0
    pw.write_bit(false); // deblocking_filter_control_present_flag = 0
    pw.write_bit(false); // pps_scaling_list_data_present_flag = 0
    pw.write_bit(false); // lists_modification_present_flag = 0
    pw.write_ue(0); // log2_parallel_merge_level_minus2 = 0
    pw.write_bit(false); // slice_segment_header_extension_present_flag = 0
    let pps_bytes = pw.into_bytes();
    let pps = PictureParameterSet::parse(&pps_bytes).expect("parse HEVC PPS");

    // Construct slice header referencing long-term RPS
    let mut sw = BitWriter::new();
    sw.write_bit(true); // first_slice_segment_in_pic_flag = 1
    sw.write_ue(0); // slice_pic_parameter_set_id = 0
    sw.write_ue(1); // slice_type = P (1)
    sw.write_bits(15, 8); // slice_pic_order_cnt_lsb = 15
    sw.write_bit(false); // short_term_ref_pic_set_sps_flag = 0
    sw.write_ue(0); // num_negative_pics = 0
    sw.write_ue(0); // num_positive_pics = 0

    // Long-term ref pics section (§7.3.6.1):
    sw.write_ue(1); // num_long_term_sps = 1
    sw.write_ue(0); // num_long_term_pics = 0
    sw.write_bit(false); // lt_idx_sps (1 bit since num_lt_sps == 2) -> index 0
    sw.write_bit(false); // delta_poc_msb_present_flag = false

    sw.write_bit(true); // slice_temporal_mvp_enabled_flag = 1
    sw.write_bit(false); // slice_sao_luma_flag = 0
    sw.write_bit(false); // slice_sao_chroma_flag = 0
    sw.write_bit(false); // num_ref_idx_active_override_flag = 0
    sw.write_ue(0); // five_minus_max_num_merge_cand = 0
    sw.write_se(0); // slice_qp_delta = 0

    let slice_bytes = sw.into_bytes();
    let slice_hdr = SliceSegmentHeader::parse(
        &slice_bytes,
        poppy_hevc::nal::HevcNalType::TrailR,
        &sps,
        &pps,
    )
    .expect("parse HEVC slice with long-term RPS");
    assert_eq!(slice_hdr.slice_type, HevcSliceType::P);
    assert_eq!(slice_hdr.slice_pic_order_cnt_lsb, 15);
}

#[test]
fn test_av1_superres_and_film_grain() {
    let mut w = BitWriter::new();
    w.write_bits(0, 3); // seq_profile = 0
    w.write_bit(false); // still_picture = 0
    w.write_bit(false); // reduced_still_picture_header = 0
    w.write_bit(false); // timing_info_present = 0
    w.write_bit(false); // initial_display_delay_present = 0
    w.write_bits(0, 5); // operating_points_cnt_minus_1 = 0
    w.write_bits(0, 12); // operating_point_idc[0] = 0
    w.write_bits(0, 5); // seq_level_idx[0] = 0 (<= 7, so seq_tier is not read)

    w.write_bits(3, 4); // frame_width_bits_minus_1 = 3
    w.write_bits(3, 4); // frame_height_bits_minus_1 = 3
    w.write_bits(15, 4); // max_frame_width_minus_1 = 15 (16px)
    w.write_bits(15, 4); // max_frame_height_minus_1 = 15 (16px)

    w.write_bit(false); // frame_id_numbers_present = 0
    w.write_bit(false); // use_128x128_superblock = 0
    w.write_bit(false); // enable_filter_intra = 0
    w.write_bit(false); // enable_intra_edge_filter = 0
    w.write_bit(false); // enable_interintra_compound = 0
    w.write_bit(false); // enable_masked_compound = 0
    w.write_bit(false); // enable_warped_motion = 0
    w.write_bit(false); // enable_dual_filter = 0
    w.write_bit(false); // enable_order_hint = 0
    w.write_bit(false); // seq_choose_screen_content_tools = 0
    w.write_bit(false); // seq_force_screen_content_tools = 0

    w.write_bit(true); // enable_superres = 1!
    w.write_bit(false); // enable_cdef = 0
    w.write_bit(false); // enable_restoration = 0

    // color_config:
    w.write_bit(false); // high_bitdepth = 0
    w.write_bit(false); // mono_chrome = 0
    w.write_bit(false); // color_description_present = 0
    w.write_bit(false); // color_range = 0
    w.write_bits(0, 2); // chroma_sample_position = 0
    w.write_bit(false); // separate_uv_delta_q = 0

    w.write_bit(true); // film_grain_params_present = 1!

    let seq_bytes = w.into_bytes();
    let seq = poppy_av1::SequenceHeader::parse(&seq_bytes).expect("parse AV1 Sequence Header");
    assert!(seq.enable_superres);
    assert!(seq.film_grain_params_present);

    let mut fw = BitWriter::new();
    fw.write_bit(false); // show_existing_frame = 0
    fw.write_bits(0, 2); // frame_type = KEY_FRAME (0)
    fw.write_bit(true); // show_frame = 1
    // (error_resilient_mode skipped because KeyFrame && show_frame)
    fw.write_bit(false); // disable_cdf_update = 0
    // (allow_screen_content_tools skipped because seq_force_screen_content_tools == 0)
    fw.write_bit(false); // frame_size_override_flag = 0

    // Superres:
    fw.write_bit(true); // use_superres = 1!
    fw.write_bits(3, 3); // superres_denom minus 9 = 3 -> denom = 12!

    // render_size:
    fw.write_bit(false); // render_and_frame_size_different = 0 (1 bit)

    // disable_frame_end_update_cdf:
    fw.write_bit(false); // disable_frame_end_update_cdf = 0

    // Tile info:
    fw.write_bit(true); // uniform_tile_spacing_flag = 1

    // Quantization:
    fw.write_bits(50, 8); // base_q_idx = 50
    fw.write_bit(false); // y_dc_delta_q: delta_coded = 0
    fw.write_bit(false); // u_dc_delta_q: delta_coded = 0
    fw.write_bit(false); // u_ac_delta_q: delta_coded = 0
    fw.write_bit(false); // using_qmatrix = 0

    // Segmentation:
    fw.write_bit(false); // segmentation_enabled = 0

    // Delta q:
    fw.write_bit(false); // delta_q_present = 0

    // Loop filter:
    fw.write_bits(0, 6); // filter_level[0] = 0
    fw.write_bits(0, 6); // filter_level[1] = 0
    fw.write_bits(0, 3); // sharpness_level = 0
    fw.write_bit(false); // mode_ref_delta_enabled = 0

    // TX mode:
    fw.write_bit(false); // tx_mode_select = 0

    // reduced_tx_set_used:
    fw.write_bit(false); // reduced_tx_set_used = 0

    // Film grain params:
    fw.write_bit(true); // apply_grain = 1!
    fw.write_bits(0xABCD, 16); // grain_seed = 0xABCD
    // (update_grain is implicit true for KeyFrame)
    fw.write_bits(2, 4); // num_y_points = 2
    fw.write_bits(32, 8); // point 0 val
    fw.write_bits(10, 8); // point 0 scaling
    fw.write_bits(128, 8); // point 1 val
    fw.write_bits(25, 8); // point 1 scaling
    fw.write_bit(false); // chroma_scaling_from_luma = 0
    fw.write_bits(0, 4); // num_cb_points = 0
    fw.write_bits(0, 4); // num_cr_points = 0
    fw.write_bits(0, 2); // grain_scaling_minus_8 = 0
    fw.write_bits(0, 2); // ar_coeff_lag = 0
    fw.write_bits(0, 2); // ar_coeff_shift_minus_6 = 0
    fw.write_bits(0, 2); // grain_scale_shift = 0
    fw.write_bit(true); // overlap_flag = 1
    fw.write_bit(false); // clip_to_restricted_range = 0

    let frame_bytes = fw.into_bytes();
    let frame_hdr =
        poppy_av1::FrameHeader::parse(&frame_bytes, &seq).expect("parse AV1 Frame Header");
    assert!(frame_hdr.use_superres);
    assert_eq!(frame_hdr.superres_scale_denominator, 12);
    assert!(frame_hdr.film_grain.apply_grain);
    assert_eq!(frame_hdr.film_grain.grain_seed, 0xABCD);
    assert_eq!(frame_hdr.film_grain.num_y_points, 2);
    assert_eq!(frame_hdr.film_grain.point_y_value, vec![32, 128]);
    assert_eq!(frame_hdr.film_grain.point_y_scaling, vec![10, 25]);
}

#[test]
fn test_vp9_superframe_with_multiple_subframes() {
    for num_subframes in [3usize, 5, 8] {
        let mut packet = Vec::new();
        let mut expected_slices = Vec::new();

        for i in 0..num_subframes {
            let frame_data = vec![(i as u8) + 1; (i + 1) * 20];
            expected_slices.push(frame_data.clone());
            packet.extend_from_slice(&frame_data);
        }

        let marker = 0xC0 | ((num_subframes as u8 - 1) & 0x07);
        packet.push(marker);
        for slice in &expected_slices {
            packet.push(slice.len() as u8);
        }
        packet.push(marker);

        let unpacked = poppy_vp9::parse_superframe_frames(&packet).expect("unpack VP9 superframe");
        assert_eq!(
            unpacked.len(),
            num_subframes,
            "Expected {num_subframes} subframes"
        );

        for (i, slice) in unpacked.iter().enumerate() {
            assert_eq!(*slice, expected_slices[i].as_slice());
        }
    }
}

#[test]
fn test_dmabuf_dup_exhaustion_try_clone() {
    let bad_desc = DmaBufDescriptor {
        fd: 999999, // Non-existent fd that fails dup() on Unix
        drm_format: 0x3231564E,
        drm_modifier: 0,
        width: 1920,
        height: 1080,
        offsets: vec![0, 1920 * 1080],
        pitches: vec![1920, 1920],
    };

    let err = bad_desc.try_clone().unwrap_err();
    match &err {
        HwError::DecodeError(msg) => {
            assert!(
                msg.contains("dup() failed for DMA-BUF fd 999999"),
                "Error message must clearly identify fd failure: {msg}"
            );
        }
        other => panic!("Expected DecodeError, got {other:?}"),
    }

    let frame = DecodedFrame {
        width: 1920,
        height: 1080,
        dmabuf: Some(bad_desc),
        ..Default::default()
    };

    let frame_err = frame.try_clone().unwrap_err();
    match &frame_err {
        HwError::DecodeError(msg) => {
            assert!(msg.contains("dup() failed for DMA-BUF fd 999999"));
        }
        other => panic!("Expected DecodeError, got {other:?}"),
    }
}

#[test]
fn test_empty_bytes_packet_handling() {
    let empty_pkt = Packet {
        stream_index: 0,
        pts: Some(0),
        dts: 0,
        duration: 1000,
        data: Bytes::new(),
        is_keyframe: false,
    };

    assert!(empty_pkt.data.is_empty());

    if let Ok(mut d3d12) = poppy_hw::d3d12::D3D12VideoDecoder::new(poppy_core::CodecId::H264) {
        assert!(d3d12.submit_packet(&empty_pkt).is_ok());
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(mut dec) = poppy_hw::VaapiH264Decoder::new() {
            assert!(dec.submit_packet(&empty_pkt).is_ok());
        }
        if let Ok(mut dec) = poppy_hw::VaapiHevcDecoder::new(false) {
            assert!(dec.submit_packet(&empty_pkt).is_ok());
        }
        if let Ok(mut dec) = poppy_hw::VaapiAv1Decoder::new(false) {
            assert!(dec.submit_packet(&empty_pkt).is_ok());
        }
        if let Ok(mut dec) = poppy_hw::VaapiVp9Decoder::new(false) {
            assert!(dec.submit_packet(&empty_pkt).is_ok());
        }
    }
}
