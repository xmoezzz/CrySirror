// SPDX-License-Identifier: MPL-2.0
//! AlphaMovie through the shared video service. The sample is local only
//! (`CRYSIRROR_AMV_SAMPLE`); no movie data is committed.

#[test]
fn sniffs_alphamovie_magic() {
    assert_eq!(crysirror_video::sniff(b"AJPM\0\0\0\0").unwrap(), crysirror_video::Container::Amv);
}

#[test]
#[ignore = "needs CRYSIRROR_AMV_SAMPLE (a local .amv file)"]
fn decodes_every_frame_of_a_local_sample() {
    let bytes = std::fs::read(std::env::var_os("CRYSIRROR_AMV_SAMPLE").unwrap()).unwrap();
    let mut stream = crysirror_video::open(bytes).unwrap();
    let info = stream.info().clone();
    let mut frames = 0u64;
    let mut translucent = false;
    let mut last = 0;
    while let Some(frame) = stream.next_frame().unwrap() {
        assert_eq!((frame.width, frame.height), (info.width, info.height));
        assert!(frames == 0 || frame.pts_ms >= last);
        last = frame.pts_ms;
        translucent |= frame.rgba.chunks_exact(4).any(|p| p[3] < 255);
        frames += 1;
    }
    println!("{info:?} frames={frames} translucent={translucent}");
    assert!(frames > 0);
    assert!(info.fps.is_some_and(|f| (1.0..=240.0).contains(&f)), "{:?}", info.fps);
    stream.rewind().unwrap();
    assert_eq!(stream.next_frame().unwrap().unwrap().pts_ms, 0);
}
