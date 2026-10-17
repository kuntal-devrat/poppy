use poppy_demux::Mp4Demuxer;
use std::path::PathBuf;

#[test]
fn test_real_video_open() {
    let path = std::env::var("POPPY_TEST_VIDEO")
        .map(PathBuf::from)
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            let candidates = [
                "tests/samples/test_av_aac.mp4",
                "../../tests/samples/test_av_aac.mp4",
            ];
            candidates.iter().map(PathBuf::from).find(|p| p.exists())
        });

    let path = match path {
        Some(p) => p,
        None => {
            eprintln!("Test video file not found, skipping.");
            return;
        }
    };
    let demuxer = Mp4Demuxer::open(&path).expect("Failed to open real video file");
    assert_eq!(demuxer.tracks().len(), 2);
}
