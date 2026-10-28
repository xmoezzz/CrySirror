//! Ogg Opus through the shared stream decoder. Real files are local only.
use crysirror_audio::decoder::{AudioSource, Stream};

#[test]
fn registry_includes_opus() {
    let r = crysirror_audio::opus::codecs();
    assert!(r.get_codec(symphonia::core::codecs::CODEC_TYPE_OPUS).is_some());
    assert!(r.get_codec(symphonia::core::codecs::CODEC_TYPE_VORBIS).is_some());
}

#[test]
#[ignore = "requires a local Ogg Opus file in CRYSIRROR_OPUS_FILE; none shipped"]
fn decodes_and_seeks_local_ogg_opus() {
    let path = std::env::var("CRYSIRROR_OPUS_FILE").unwrap();
    let bytes: std::sync::Arc<[u8]> = std::fs::read(path).unwrap().into();
    let mut s = Stream::from_bytes(bytes, Some("opus")).unwrap();
    assert_eq!(s.sample_rate(), 48_000);
    let mut buf = vec![[0f32; 2]; 48_000];
    assert_eq!(s.read(&mut buf).unwrap(), 48_000);
    let energy: f32 = buf.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum();
    assert!(energy > 0.);
    s.seek(96_000).unwrap();
    assert!(s.read(&mut buf).unwrap() > 0);
}
