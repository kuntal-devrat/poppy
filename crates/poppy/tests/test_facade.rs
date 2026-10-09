use poppy::core::CodecId;

#[test]
fn test_poppy_facade_end_to_end() {
    let mut stream = poppy::open("../../tests/samples/test_clip.mp4")
        .or_else(|_| poppy::open("tests/samples/test_clip.mp4"))
        .expect("Failed to open test_clip.mp4 via poppy facade");

    let streams = stream.streams();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec, CodecId::H264);

    let pkt0 = stream
        .next_packet()
        .expect("Failed to read packet")
        .expect("Expected packet 0");
    assert_eq!(pkt0.dts, 0);
    assert!(pkt0.is_keyframe);

    let pkt1 = stream
        .next_packet()
        .expect("Failed to read packet")
        .expect("Expected packet 1");
    assert_eq!(pkt1.dts, 1000);
    assert!(!pkt1.is_keyframe);

    let pkt_none = stream.next_packet().expect("Failed to read packet");
    assert!(pkt_none.is_none());

    // Seek back to 0
    stream.seek(0).expect("Seek to 0 failed");
    let pkt_rewind = stream
        .next_packet()
        .expect("Failed to read packet")
        .expect("Expected rewind packet");
    assert_eq!(pkt_rewind.dts, 0);
}

#[test]
fn test_h264_decode_frame_count() {
    let mut stream = poppy::open("../../tests/samples/test_h264.mp4")
        .or_else(|_| poppy::open("tests/samples/test_h264.mp4"))
        .expect("Failed to open test_h264.mp4");

    let mut count = 0;
    while let Some(frame) = stream.next_frame().expect("next_frame failed") {
        count += 1;
        println!("Frame #{count}: pts={:?}", frame.pts);
    }
    println!("Total decoded frames from test_h264.mp4: {count}");
    assert_eq!(count, 30);
}
