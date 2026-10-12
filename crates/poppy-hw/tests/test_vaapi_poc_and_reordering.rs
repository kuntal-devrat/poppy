use poppy_core::codec::CodecConfig;
use poppy_demux::bsf::avcc_to_annex_b;
use poppy_demux::{Demuxer, Mp4Demuxer};
use poppy_hw::{HwDecoder, VaapiH264Decoder};
use std::path::Path;

#[test]
fn test_hw_decode_verify_monotonic_pts() {
    let path = Path::new(
        "/home/pikasu/Downloads/𝗠𝗢𝗡𝗧𝗔𝗚𝗘𝗠 𝗨𝗥𝗔𝗡𝗜𝗨𝗠 (𝗦𝗨𝗣𝗘𝗥 𝗦𝗟𝗢𝗪𝗘𝗗) - weekkse (1080p, h264).mp4",
    );
    if !path.exists() {
        return;
    }

    let mut demuxer = match Mp4Demuxer::open(path) {
        Ok(d) => d,
        Err(_) => return,
    };
    let video_stream_idx = match demuxer
        .streams()
        .iter()
        .position(|s| s.codec == poppy_core::codec::CodecId::H264)
    {
        Some(idx) => idx,
        None => return,
    };
    let stream_config = demuxer.streams()[video_stream_idx].config.clone();

    let avcc = match stream_config {
        CodecConfig::H264(a) => a,
        _ => return,
    };

    let mut decoder = match VaapiH264Decoder::new() {
        Ok(d) => d,
        Err(_) => return,
    };

    let mut params = Vec::new();
    params.extend(avcc.sps_list.clone());
    params.extend(avcc.pps_list.clone());
    let mut annex_b_buf = Vec::new();

    let mut frame_count = 0;
    let mut prev_pts = -1i64;
    let mut non_monotonic = 0;

    while let Ok(Some(packet)) = demuxer.read_packet() {
        if packet.stream_index != video_stream_idx {
            continue;
        }

        annex_b_buf.clear();
        let params_ref = if packet.is_keyframe {
            Some(&params[..])
        } else {
            None
        };
        if avcc_to_annex_b(
            &packet.data,
            avcc.nal_length_size,
            params_ref,
            &mut annex_b_buf,
        )
        .is_err()
        {
            continue;
        }

        let mut pkt = packet.clone();
        pkt.data = bytes::Bytes::from(annex_b_buf.clone());

        if decoder.submit_packet(&pkt).is_err() {
            continue;
        }
        while let Ok(Some(f)) = decoder.receive_frame() {
            let pts = f.pts.unwrap_or(0);
            if prev_pts >= 0 {
                let diff = pts - prev_pts;
                if diff <= 0 {
                    non_monotonic += 1;
                }
            }
            prev_pts = pts;
            frame_count += 1;
        }

        if frame_count >= 100 {
            break;
        }
    }

    assert_eq!(
        non_monotonic, 0,
        "All frames must be strictly monotonic in display order!"
    );
}
