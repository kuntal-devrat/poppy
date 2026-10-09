use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use poppy_core::codec::CodecConfig;
use poppy_demux::bsf::avcc_to_annex_b;
use poppy_demux::{Demuxer, Mp4Demuxer};

#[derive(Parser, Debug)]
#[command(
    name = "poppy",
    version = "0.1.0",
    about = "🌸 Poppy — Modern media pipeline inspector and elementary stream dumper"
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

        /// Output file path (e.g. output.h264)
        #[arg(short, long)]
        output: PathBuf,

        /// Stream index to extract (defaults to first video stream)
        #[arg(short, long)]
        stream: Option<usize>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Probe { file } => {
            println!("🔍 Probing: {}", file.display());
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
                    CodecConfig::Hevc(_) => {
                        println!("  HEVC Config present");
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
            let streams = demuxer.streams();
            if streams.is_empty() {
                return Err("No streams found in container".into());
            }

            // Find target stream
            let target_stream_idx = match stream {
                Some(idx) => {
                    if idx >= streams.len() {
                        return Err(format!("Stream index {} out of range (total {})", idx, streams.len()).into());
                    }
                    idx
                }
                None => {
                    // Default to first video track
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

            let stream_info = &streams[target_stream_idx];
            println!(
                "📥 Extracting Stream #{} ({}) from {}",
                target_stream_idx,
                stream_info.codec,
                input.display()
            );

            let (nal_length_size, sps_pps) = match &stream_info.config {
                CodecConfig::H264(avcc) => {
                    let mut params = Vec::new();
                    params.extend(avcc.sps_list.clone());
                    params.extend(avcc.pps_list.clone());
                    (avcc.nal_length_size, Some(params))
                }
                _ => (4, None),
            };

            let mut out_file = File::create(&output)?;
            let mut packet_count = 0usize;
            let mut total_bytes = 0usize;
            let mut annex_b_buf = Vec::with_capacity(256 * 1024);

            while let Some(packet) = demuxer.read_packet()? {
                if packet.stream_index != target_stream_idx {
                    continue;
                }

                annex_b_buf.clear();

                let params_ref = if packet.is_keyframe {
                    sps_pps.as_deref()
                } else {
                    None
                };

                avcc_to_annex_b(
                    &packet.data,
                    nal_length_size,
                    params_ref,
                    &mut annex_b_buf,
                )?;

                out_file.write_all(&annex_b_buf)?;
                total_bytes += annex_b_buf.len();
                packet_count += 1;
            }

            println!(
                "✨ Wrote {} packets ({} bytes / {:.2} MB) to {}",
                packet_count,
                total_bytes,
                total_bytes as f64 / (1024.0 * 1024.0),
                output.display()
            );
        }
    }

    Ok(())
}
