use crysirror_video::isobmff::Mp4Stream;
use std::io::Cursor;

#[test]
fn malformed_mp4_is_an_error() {
    for bytes in [
        vec![],
        b"\0\0\0\x18ftypisom".to_vec(),
        b"\0\0\0\x04ftyp".to_vec(),
    ] {
        assert!(Mp4Stream::new(Cursor::new(bytes)).is_err());
    }
}

/// Self-generated clips plus independently decoded FFmpeg reference samples.
/// No decoder process, external asset or device is needed to run this test.
#[test]
fn synthetic_three_codec_decode_seek_and_cancel() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for codec in ["h264", "h264_main", "h265", "h265_10", "av1", "av1_10"] {
        let mut decoder =
            Mp4Stream::new(std::fs::File::open(dir.join(format!("{codec}.mp4"))).unwrap())
                .unwrap_or_else(|e| panic!("{codec} open: {e:#}"));
        assert_eq!((decoder.info().width, decoder.info().height), (64, 64));
        assert!(!decoder.has_audio());
        assert!(decoder.next_frame(|| true).unwrap().is_none());
        let mut frames = Vec::new();
        while let Some(frame) = decoder
            .next_frame(|| false)
            .unwrap_or_else(|e| panic!("{codec} decode: {e:#}"))
        {
            assert_eq!((frame.width, frame.height), (64, 64));
            assert_eq!(frame.rgba.len(), 64 * 64 * 4);
            assert!(frame.rgba.chunks_exact(4).all(|p| p[3] == 255));
            frames.push(frame);
        }
        assert_eq!(
            frames.iter().map(|f| f.pts_ms).collect::<Vec<_>>(),
            [0, 200, 400, 600, 800],
            "{codec} timestamps"
        );
        assert!(decoder.next_frame(|| false).unwrap().is_none());
        decoder
            .seek(400)
            .unwrap_or_else(|e| panic!("{codec} seek: {e:#}"));
        let sought = decoder.next_frame(|| false).unwrap().expect("seek result");
        assert_eq!(
            sought, frames[2],
            "{codec} seek must reconstruct same pixels and PTS"
        );
        decoder.seek(0).unwrap();
        assert_eq!(decoder.next_frame(|| false).unwrap().unwrap(), frames[0]);
        let oracle = std::fs::read(dir.join(format!("{codec}.oracle.rgb"))).unwrap();
        let tolerance = if codec.ends_with("_10") { 4 } else { 1 };
        for (f, frame) in frames.iter().enumerate() {
            for sample in 0..64 {
                let pixel = (sample * 1297 + 113) % 4096;
                for channel in 0..3 {
                    let actual = frame.rgba[pixel * 4 + channel];
                    let expected = oracle[(f * 64 + sample) * 3 + channel];
                    assert!(
                        actual.abs_diff(expected) <= tolerance,
                        "{codec} frame {f} pixel {pixel} channel {channel}: {actual} vs independent oracle {expected}"
                    );
                }
            }
        }
        decoder.seek(10000).unwrap();
        assert!(decoder.next_frame(|| false).unwrap().is_none());
        eprintln!(
            "{codec}: 5 frames, RGBA64x64, PTS[0,200,400,600,800], seek/rewind/cancel/EOF passed"
        );
    }
}

#[test]
fn corrupt_sample_is_a_terminal_decode_error_not_eof() {
    let mut bytes = include_bytes!("fixtures/h264.mp4").to_vec();
    let mdat = bytes.windows(4).position(|b| b == b"mdat").unwrap();
    bytes[mdat + 4..mdat + 8].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut decoder = Mp4Stream::new(Cursor::new(bytes)).unwrap();
    assert!(decoder.next_frame(|| false).is_err());
    assert!(
        decoder
            .next_frame(|| false)
            .unwrap_err()
            .to_string()
            .contains("failed")
    );
}

#[test]
fn common_video_dispatch_and_rewind_use_native_mp4() {
    let bytes = include_bytes!("fixtures/h264_main.mp4").to_vec();
    assert_eq!(
        crysirror_video::sniff(&bytes).unwrap(),
        crysirror_video::Container::Mp4
    );
    let mut stream = crysirror_video::open(bytes).unwrap();
    let first = stream.next_frame().unwrap().unwrap();
    assert_eq!(first.pts_ms, 0);
    stream.next_frame().unwrap().unwrap();
    stream.rewind().unwrap();
    assert_eq!(stream.next_frame().unwrap().unwrap(), first);
}
