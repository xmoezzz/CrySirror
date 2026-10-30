//! Real shared codec registry and metadata policy, independent of a native VM.
use crysirror_audio::decoder::{self, AudioSource, Stream};
#[test]
fn capability_names_are_codecs_and_use_the_actual_stream_registry() {
    assert!(decoder::codec_supported("opus"));
    assert!(decoder::codec_supported("vorbis"));
    assert!(!decoder::codec_supported("ogg"));
    assert!(!decoder::codec_supported("missing"));
}
#[test]
#[ignore = "requires CRYSIRROR_CODEC_FIXTURES with authored tone.opus, tone.ogg, gain.ogg"]
fn generated_codecs_decode_seek_and_publish_replay_gain_metadata_without_applying_it() {
    let root = std::path::PathBuf::from(
        std::env::var_os("CRYSIRROR_CODEC_FIXTURES").expect("explicit authored fixture path"),
    );
    for (name, extension, gain) in [
        ("tone.opus", "opus", false),
        ("tone.ogg", "ogg", false),
        ("gain.ogg", "ogg", true),
    ] {
        let mut source = Stream::from_bytes(
            std::fs::read(root.join(name)).unwrap().into(),
            Some(extension),
        )
        .unwrap();
        assert_eq!(source.sample_rate(), 48000);
        assert_eq!(source.has_replay_gain(), gain, "{name}");
        let mut out = vec![[0.; 2]; 4800];
        assert_eq!(source.read(&mut out).unwrap(), 4800);
        let energy: f32 = out.iter().map(|v| v[0] * v[0] + v[1] * v[1]).sum();
        assert!(energy > 1.0, "{name}: {energy}");
        source.seek(48000).unwrap();
        assert_eq!(source.read(&mut out).unwrap(), 4800);
        assert_eq!(source.has_replay_gain(), gain);
    }
}
