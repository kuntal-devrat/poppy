use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use poppy_core::codec::CodecConfig;
use poppy_demux::bsf::{avcc_to_annex_b, hevc_to_annex_b};
use poppy_demux::{Demuxer, Mp4Demuxer};

#[derive(Parser, Debug)]
#[command(
    name = "poppy",
    version = "0.3.0",
    about = "🌸 Poppy — Modern media pipeline inspector, zero-copy GPU decoder, and wgpu renderer"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Probe media container metadata, streams, codecs, and sample statistics
    Probe {
        /// Path to the media file
        file: PathBuf,
    },
    /// Dump raw elementary bitstream (Annex-B format) from container
    DumpEs {
        /// Path to the media container file
        input: PathBuf,

        /// Output file path (e.g. output.h264 or output.h265)
        #[arg(short, long)]
        output: PathBuf,

        /// Stream index to extract (defaults to first video stream)
        #[arg(short, long)]
        stream: Option<usize>,
    },
    /// Decode video frames using GPU hardware acceleration (VA-API) and wgpu rendering
    Decode {
        /// Path to the media file
        file: PathBuf,

        /// Limit maximum number of frames to decode
        #[arg(short, long)]
        limit: Option<usize>,

        /// Enable wgpu zero-copy color conversion rendering pipeline
        #[arg(short, long)]
        render: bool,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Probe { file } => {
            println!("🔍 Probing: {}", file.display());
            let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let is_mkv = ext == "mkv" || ext == "webm";

            if is_mkv {
                let demuxer = poppy_demux::MatroskaDemuxer::open(&file)?;
                let streams = demuxer.streams();
                println!("📦 Container: Matroska / WebM ({} stream(s) found)\n", streams.len());

                for stream in streams {
                    println!("▶ Stream #{}:", stream.index);
                    println!("  Codec:      {}", stream.codec);
                    println!("  Timebase:   {}", stream.time_base);
                    match &stream.config {
                        CodecConfig::H264(avcc) => {
                            println!("  H.264 Config:");
                            println!("    Profile:           {}", avcc.profile_idc);
                            println!("    Level:             {}", avcc.level_idc);
                            println!("    SPS Count:         {}", avcc.sps_list.len());
                            println!("    PPS Count:         {}", avcc.pps_list.len());
                        }
                        CodecConfig::Hevc(hvcc) => {
                            println!("  H.265 / HEVC Config:");
                            println!("    Profile IDC:       {} ({})", hvcc.profile_idc, if hvcc.is_10bit() { "10-bit HDR" } else { "8-bit Main" });
                            println!("    Bit Depth (Luma):  {}-bit", hvcc.bit_depth_luma);
                            println!("    Level:             {}", hvcc.level_idc);
                            println!("    VPS Count:         {}", hvcc.vps_list.len());
                            println!("    SPS Count:         {}", hvcc.sps_list.len());
                            println!("    PPS Count:         {}", hvcc.pps_list.len());
                        }
                        _ => {}
                    }
                    println!();
                }
                return Ok(());
            }

            let demuxer = Mp4Demuxer::open(&file)?;
            let streams = demuxer.streams();
            let tracks = demuxer.tracks();

            println!("📦 Container: ISOBMFF / MP4 ({} stream(s) found)\n", streams.len());

            for (i, stream) in streams.iter().enumerate() {
                let track = &tracks[i];
                let handler = String::from_utf8_lossy(&track.handler_type);
                let duration_sec = stream
                    .duration_ticks
                    .map(|d| stream.time_base.to_seconds(d))
                    .unwrap_or(0.0);

                let keyframes = track
                    .sample_table
                    .samples
                    .iter()
                    .filter(|s| s.is_keyframe)
                    .count();

                println!("▶ Stream #{}:", stream.index);
                println!("  Handler:    {}", handler.trim());
                println!("  Codec:      {}", stream.codec);
                if track.width > 0 && track.height > 0 {
                    println!("  Resolution: {}x{}", track.width, track.height);
                }
                println!("  Timebase:   {}", stream.time_base);
                println!(
                    "  Duration:   {:.3} s ({} ticks)",
                    duration_sec,
                    stream.duration_ticks.unwrap_or(0)
                );
                println!("  Samples:    {}", track.sample_table.samples.len());
                println!("  Keyframes:  {}", keyframes);

                match &stream.config {
                    CodecConfig::H264(avcc) => {
                        println!("  H.264 Config:");
                        println!("    Profile:           {}", avcc.profile_idc);
                        println!("    Level:             {}", avcc.level_idc);
                        println!("    NAL Length Size:   {} byte(s)", avcc.nal_length_size);
                        println!("    SPS Count:         {}", avcc.sps_list.len());
                        println!("    PPS Count:         {}", avcc.pps_list.len());
                    }
                    CodecConfig::Hevc(hvcc) => {
                        println!("  H.265 / HEVC Config:");
                        println!("    Profile IDC:       {} ({})", hvcc.profile_idc, if hvcc.is_10bit() { "10-bit HDR" } else { "8-bit Main" });
                        println!("    Bit Depth (Luma):  {}-bit", hvcc.bit_depth_luma);
                        println!("    Level:             {}", hvcc.level_idc);
                        println!("    NAL Length Size:   {} byte(s)", hvcc.nal_length_size);
                        println!("    VPS Count:         {}", hvcc.vps_list.len());
                        println!("    SPS Count:         {}", hvcc.sps_list.len());
                        println!("    PPS Count:         {}", hvcc.pps_list.len());
                    }
                    CodecConfig::Av1(_) => {
                        println!("  AV1 Config present");
                    }
                    CodecConfig::Audio(_) => {
                        println!("  Audio Config present");
                    }
                    CodecConfig::Unknown => {}
                }
                println!();
            }
        }

        Commands::DumpEs {
            input,
            output,
            stream,
        } => {
            let mut demuxer = Mp4Demuxer::open(&input)?;
            let num_streams = demuxer.streams().len();
            if num_streams == 0 {
                return Err("No streams found in container".into());
            }

            let target_stream_idx = match stream {
                Some(idx) => {
                    if idx >= num_streams {
                        return Err(format!("Stream index {} out of range (total {})", idx, num_streams).into());
                    }
                    idx
                }
                None => {
                    let mut found = 0;
                    for (i, t) in demuxer.tracks().iter().enumerate() {
                        if &t.handler_type == b"vide" {
                            found = i;
                            break;
                        }
                    }
                    found
                }
            };

            let stream_codec = demuxer.streams()[target_stream_idx].codec;
            let stream_config = demuxer.streams()[target_stream_idx].config.clone();

            println!(
                "📥 Extracting Stream #{} ({}) from {}",
                target_stream_idx,
                stream_codec,
                input.display()
            );

            let mut out_file = File::create(&output)?;
            let mut packet_count = 0usize;
            let mut total_bytes = 0usize;
            let mut annex_b_buf = Vec::with_capacity(256 * 1024);

            match stream_config {
                CodecConfig::H264(avcc) => {
                    let mut params = Vec::new();
                    params.extend(avcc.sps_list.clone());
                    params.extend(avcc.pps_list.clone());

                    while let Some(packet) = demuxer.read_packet()? {
                        if packet.stream_index != target_stream_idx {
                            continue;
                        }

                        annex_b_buf.clear();
                        let params_ref = if packet.is_keyframe { Some(&params[..]) } else { None };
                        avcc_to_annex_b(&packet.data, avcc.nal_length_size, params_ref, &mut annex_b_buf)?;
                        out_file.write_all(&annex_b_buf)?;
                        total_bytes += annex_b_buf.len();
                        packet_count += 1;
                    }
                }
                CodecConfig::Hevc(hvcc) => {
                    while let Some(packet) = demuxer.read_packet()? {
                        if packet.stream_index != target_stream_idx {
                            continue;
                        }

                        annex_b_buf.clear();
                        let (vps, sps, pps) = if packet.is_keyframe {
                            (Some(&hvcc.vps_list[..]), Some(&hvcc.sps_list[..]), Some(&hvcc.pps_list[..]))
                        } else {
                            (None, None, None)
                        };

                        hevc_to_annex_b(
                            &packet.data,
                            hvcc.nal_length_size,
                            vps,
                            sps,
                            pps,
                            &mut annex_b_buf,
                        )?;
                        out_file.write_all(&annex_b_buf)?;
                        total_bytes += annex_b_buf.len();
                        packet_count += 1;
                    }
                }
                _ => {
                    return Err(format!("Unsupported codec for ES dump: {}", stream_codec).into());
                }
            }

            println!(
                "✨ Wrote {} packets ({} bytes / {:.2} MB) to {}",
                packet_count,
                total_bytes,
                total_bytes as f64 / (1024.0 * 1024.0),
                output.display()
            );
        }

        Commands::Decode { file, limit, render } => {
            println!("🚀 Initializing hardware decoding pipeline for: {}", file.display());
            let mut stream = poppy::open(&file)?;

            let renderer = if render {
                match poppy_render::GpuRenderer::new_sync() {
                    Ok(r) => {
                        let info = r.adapter.get_info();
                        println!("🎨 wgpu Zero-Copy Renderer: Active ({:?}, {})", info.backend, info.name);
                        Some(r)
                    }
                    Err(e) => {
                        println!("⚠️ wgpu renderer initialization skipped: {e}");
                        None
                    }
                }
            } else {
                None
            };

            let mut frame_count = 0;
            let start = std::time::Instant::now();

            while let Some(frame) = stream.next_frame()? {
                frame_count += 1;

                if let Some(r) = &renderer {
                    let _ = r.render_frame(&frame);
                }

                if frame_count == 1 || frame_count % 30 == 0 {
                    println!(
                        "  [Frame #{:04}] PTS: {:?} | Format: {} | Size: {}x{}",
                        frame_count,
                        frame.pts,
                        frame.pixel_format,
                        frame.width,
                        frame.height
                    );
                }
                if let Some(max) = limit
                    && frame_count >= max
                {
                    break;
                }
            }

            let elapsed = start.elapsed();
            let fps = if elapsed.as_secs_f64() > 0.0 {
                frame_count as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };

            println!(
                "\n✨ Decoded {} frames in {:.2?} ({:.1} FPS) via GPU hardware accelerator",
                frame_count, elapsed, fps
            );
        }
    }

    Ok(())
}
