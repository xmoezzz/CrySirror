//! Shared streaming helper compared with the pinned original helper, with
//! synthetic ordinary fixtures and an opt-in read-only local movie prefix.
use crysirror_video::mpeg::{self, AudioChunk, VideoFrame};
use std::io::{Cursor, Read, Seek, SeekFrom};

fn oracle_video(bytes: &[u8], limit: usize) -> Vec<VideoFrame> {
    let header = crysirror_video::mpeg_probe::find_sequence_header(&bytes[..bytes.len().min(65536)]);
    let duration = header
        .as_ref()
        .and_then(|h| crysirror_video::mpeg_probe::fps_from_frame_rate_code(h.frame_rate_code))
        .map_or(1000. / 29.97, |fps| 1000. / f64::from(fps));
    let mut pipe = na_mpeg2_decoder::MpegVideoPipeline::new();
    let mut frames = Vec::new();
    let mut emit = |frame: std::sync::Arc<na_mpeg2_decoder::Frame>| {
        if frames.len() >= limit {
            return;
        }
        let mut rgba = vec![0; frame.width * frame.height * 4];
        na_mpeg2_decoder::frame_to_rgba_bt601_limited(&frame, &mut rgba);
        let (width, height) = header
            .as_ref()
            .map(|h| (usize::from(h.width), usize::from(h.height)))
            .filter(|&(w, h)| w < frame.width || h < frame.height)
            .unwrap_or((frame.width, frame.height));
        let mut cropped = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            cropped.extend_from_slice(&rgba[y * frame.width * 4..y * frame.width * 4 + width * 4]);
        }
        frames.push(VideoFrame {
            pts_ms: frame
                .pts_90k
                .map_or((frames.len() as f64 * duration) as i64, |p| p / 90),
            width: width as u32,
            height: height as u32,
            rgba: cropped,
        });
    };
    for bytes in bytes.chunks(mpeg::CHUNK_BYTES) {
        pipe.push_with(bytes, None, &mut emit).unwrap();
    }
    pipe.flush_with(&mut emit).unwrap();
    frames
}
fn fixture() -> Vec<u8> {
    include_bytes!("fixtures/red_blue.mpg").to_vec()
}
#[test]
fn streaming_video_matches_original_pts_and_pixels_including_delayed_flush() {
    let bytes = fixture();
    let expected = oracle_video(&bytes, usize::MAX);
    let mut actual = Vec::new();
    let stats = mpeg::decode_video(
        Cursor::new(bytes),
        || false,
        |f| {
            actual.push(f);
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(stats.outputs, 20);
    assert!(!stats.cancelled);
}
struct Broken;
impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("synthetic read failure"))
    }
}
impl Seek for Broken {
    fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
        Ok(0)
    }
}
#[test]
fn read_callback_empty_stream_and_cancellation_are_distinct() {
    assert!(
        mpeg::decode_video(Broken, || false, |_| Ok(true))
            .unwrap_err()
            .to_string()
            .contains("sequence read")
    );
    assert!(
        mpeg::decode_audio(Broken, || false, |_| Ok(true))
            .unwrap_err()
            .to_string()
            .contains("audio read")
    );
    assert!(mpeg::decode_video(Cursor::new(b"invalid"), || false, |_| Ok(true)).is_err());
    assert!(
        mpeg::decode_video(Broken, || true, |_| Ok(true))
            .unwrap()
            .cancelled
    );
    let error = mpeg::decode_video(
        Cursor::new(fixture()),
        || false,
        |_| anyhow::bail!("synthetic callback failure"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("synthetic callback failure"));
    let stats = mpeg::decode_video(Cursor::new(fixture()), || false, |_| Ok(false)).unwrap();
    assert_eq!(stats.outputs, 1);
    assert!(stats.cancelled);
}
#[test]
#[ignore = "requires CRYSIRROR_MPEG_FIXTURE; reads only an existing local movie prefix"]
fn local_movie_prefix_matches_pinned_video_and_audio_helpers() {
    let path = std::env::var("CRYSIRROR_MPEG_FIXTURE").expect("absolute local MPEG path");
    // Read a bounded prefix, not a whole OP. The shared service itself gets a
    // capability-owned reader from its caller in production.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .unwrap()
        .take(2 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .unwrap();
    let expected = oracle_video(&bytes, 12);
    assert_eq!(expected.len(), 12);
    let mut actual = Vec::new();
    let video_stats = mpeg::decode_video(
        Cursor::new(&bytes),
        || false,
        |f| {
            actual.push(f);
            Ok(actual.len() < 12)
        },
    )
    .unwrap();
    assert!(video_stats.cancelled);
    assert_eq!(actual, expected);
    let mut expected = Vec::new();
    let mut pipe = na_mpeg2_decoder::MpegAudioPipeline::new();
    for b in bytes.chunks(mpeg::CHUNK_BYTES) {
        pipe.push_with(b, None, |c| {
            if expected.len() < 2 {
                expected.push(AudioChunk {
                    pts_ms: c.pts_ms,
                    sample_rate: c.sample_rate,
                    channels: c.channels,
                    samples: c
                        .samples
                        .iter()
                        .map(|s| (s.clamp(-1., 1.) * 32767.) as i16)
                        .collect(),
                });
            }
        })
        .unwrap();
        if expected.len() >= 2 {
            break;
        }
    }
    assert_eq!(expected.len(), 2, "local fixture must include MPEG audio");
    let mut actual = Vec::new();
    let s = mpeg::decode_audio(
        Cursor::new(&bytes),
        || false,
        |c| {
            actual.push(c);
            Ok(actual.len() < 2)
        },
    )
    .unwrap();
    assert!(s.cancelled);
    assert_eq!(actual, expected);
    eprintln!(
        "exact local prefix: 12 RGBA/PTS frames, 2 PCM/PTS chunks; video bytes={} audio bytes={}",
        video_stats.encoded_bytes, s.encoded_bytes
    );
}
