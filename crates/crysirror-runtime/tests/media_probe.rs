//! Device-free media header preparation, independent of native VMs/playback.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, MediaProbeInfo, MediaProbeJob, MediaProbeState},
    state::ClientId,
};
use crysirror_runtime::Runtime;
use std::time::{Duration, Instant};
fn runtime() -> (tempfile::TempDir, Runtime) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(root.path().join("project.json"),br#"{"schema":2,"name":"media-probe","content_revision":"1","assets":"assets","canvas":[16,16],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"probe"}]}"#).unwrap();
    let rt = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    (root, rt)
}
fn finish(job: &mut MediaProbeJob) -> MediaProbeInfo {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            MediaProbeState::Ready(info) => return info,
            MediaProbeState::Failed(error) => panic!("{error}"),
            MediaProbeState::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    }
}
#[test]
fn mpeg_header_probe_does_not_create_playback_decode_pcm_or_read_whole_movie() {
    let (root, mut rt) = runtime();
    let head = [
        0, 0, 1, 0xb3, 0x01, 0x00, 0x10, 0x13, 0xff, 0xff, 0xe0, 0x18,
    ];
    std::fs::write(root.path().join("assets/probe.mpg"), head).unwrap();
    // A large sparse payload after the header must not be copied as a movie.
    std::fs::OpenOptions::new()
        .write(true)
        .open(root.path().join("assets/probe.mpg"))
        .unwrap()
        .set_len(100 * 1024 * 1024)
        .unwrap();
    let scene = serde_json::to_value(&rt.host.engine).unwrap();
    let media = rt.host.media.inspect();
    let voices = rt.host.scene.voices().clone();
    let path = VfsPath::new(Mount::Asset, "probe.mpg").unwrap();
    assert!(rt.host.begin_media_probe(ClientId(99), &path).is_err());
    assert!(rt
        .host
        .begin_media_probe(
            ClientId(1),
            &VfsPath::new(Mount::Save, "probe.mpg").unwrap()
        )
        .is_err());
    let mut job = rt.host.begin_media_probe(ClientId(1), &path).unwrap();
    assert_eq!(job.client(), ClientId(1));
    let info = finish(&mut job);
    assert_eq!(info.client, ClientId(1));
    assert_eq!(info.size, [16, 16]);
    assert_eq!(info.container, "mpeg");
    assert_eq!(info.video_decoder, "mpeg1/mpeg2");
    assert!(info.header_validated);
    assert_eq!(info.frame_ms, Some(40.));
    assert_eq!(info.duration_ms, None);
    assert_eq!(info.audio_decoder, None);
    assert_eq!(info.has_audio, None);
    assert_eq!(job.poll().unwrap(), MediaProbeState::Ready(info));
    assert_eq!(serde_json::to_value(&rt.host.engine).unwrap(), scene);
    assert_eq!(rt.host.media.inspect(), media);
    assert_eq!(rt.host.scene.voices(), &voices);
    assert_eq!(rt.host.scene.frame(), 0);
}
#[test]
fn malformed_probe_is_retained_failure_and_does_not_poison_later_admission() {
    let (root, mut rt) = runtime();
    std::fs::write(root.path().join("assets/bad.bin"), [0xaa; 128]).unwrap();
    let path = VfsPath::new(Mount::Asset, "bad.bin").unwrap();
    for _ in 0..2 {
        let mut job = rt.host.begin_media_probe(ClientId(1), &path).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match job.poll().unwrap() {
                MediaProbeState::Failed(error) => {
                    assert!(!error.is_empty());
                    assert_eq!(job.poll().unwrap(), MediaProbeState::Failed(error));
                    break;
                }
                MediaProbeState::Ready(_) => panic!("bad header succeeded"),
                MediaProbeState::Pending => {
                    assert!(Instant::now() < until);
                    std::thread::yield_now();
                }
            }
        }
    }
    assert!(!rt.host.media.has_resources());
}
