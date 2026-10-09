use poppy_demux::Mp4Demuxer;
use std::path::Path;

#[test]
fn test_real_video_open() {
    let path = "/home/pikasu/Downloads/𝗠𝗢𝗡𝗧𝗔𝗚𝗘𝗠 𝗨𝗥𝗔𝗡𝗜𝗨𝗠 (𝗦𝗨𝗣𝗘𝗥 𝗦𝗟𝗢𝗪𝗘𝗗) - weekkse (1080p, h264).mp4";
    if !Path::new(path).exists() {
        eprintln!("Real video test file not found, skipping.");
        return;
    }
    let demuxer = Mp4Demuxer::open(path).expect("Failed to open real video file");
    assert_eq!(demuxer.tracks().len(), 2);
}
