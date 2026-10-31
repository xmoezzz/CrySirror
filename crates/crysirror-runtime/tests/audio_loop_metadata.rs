//! Public shared host catalog: no native VM, device, original game or renderer.
use crysirror_core::{
    service::{AudioLoopMetadata, AudioLoopRange, AudioLoopTimebase, HostServices},
    state::ClientId,
};
use crysirror_runtime::Runtime;
use serde_json::json;
fn runtime() -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"loop-catalog","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    (t, rt)
}
fn entry(asset: &str, start: u64, end: Option<u64>) -> AudioLoopMetadata {
    AudioLoopMetadata {
        asset: asset.into(),
        range: AudioLoopRange {
            timebase: AudioLoopTimebase::ReferenceFrames { sample_rate: 44100 },
            start_frame: start,
            end_frame: end,
        },
    }
}
#[test]
fn registered_clients_receive_atomic_scoped_owner_commits() {
    let (_t, mut rt) = runtime();
    let a = ClientId(1);
    let b = ClientId(2);
    let foreign = ClientId(999);
    let e = entry("music/a.ogg", 100, Some(300));
    assert!(rt
        .host
        .publish_audio_loop_metadata(foreign, &[e.clone()])
        .is_err());
    assert!(rt.host.audio_loop_metadata(foreign, &e.asset).is_err());
    assert!(rt.host.clear_audio_loop_metadata(foreign).is_err());
    let receipt = rt
        .host
        .publish_audio_loop_metadata(a, &[e.clone()])
        .unwrap();
    assert_eq!(receipt.client, a);
    assert_eq!(receipt.revision, 1);
    assert_eq!(receipt.entries, 1);
    assert_eq!(
        rt.host.audio_loop_metadata(a, &e.asset).unwrap(),
        Some(e.clone())
    );
    assert_eq!(rt.host.audio_loop_metadata(b, &e.asset).unwrap(), None);
    for batch in [
        vec![
            entry("music/new.ogg", 0, None),
            entry("music/bad.ogg", 2, Some(1)),
        ],
        vec![e.clone(), e.clone()],
        vec![entry("../escape", 0, None)],
        vec![entry("x", 0, None); 4097],
    ] {
        assert!(rt.host.publish_audio_loop_metadata(a, &batch).is_err());
        assert_eq!(
            rt.host.publish_audio_loop_metadata(a, &[]).unwrap(),
            receipt
        );
        assert_eq!(
            rt.host.audio_loop_metadata(a, "music/new.ogg").unwrap(),
            None
        );
    }
    assert_eq!(rt.host.clear_audio_loop_metadata(b).unwrap().entries, 0);
    assert_eq!(rt.host.audio_loop_metadata(a, &e.asset).unwrap(), Some(e));
}
#[test]
fn live_catalog_requires_explicit_resource_reconstruction_before_restore() {
    let (_t, mut rt) = runtime();
    let saved = rt.snapshot().unwrap();
    let prepared = rt.restore_plan().decode(&saved).unwrap();
    let a = ClientId(1);
    let e = entry("music/a.ogg", 0, None);
    rt.host
        .publish_audio_loop_metadata(a, &[e.clone()])
        .unwrap();
    assert!(rt
        .capture_snapshot()
        .err()
        .unwrap()
        .to_string()
        .contains("audio loop catalog"));
    assert!(rt
        .commit_restore(prepared)
        .unwrap_err()
        .to_string()
        .contains("audio loop catalog"));
    assert_eq!(rt.host.audio_loop_metadata(a, &e.asset).unwrap(), Some(e));
    assert_eq!(rt.host.clear_audio_loop_metadata(a).unwrap().revision, 2);
    rt.restore(&saved).unwrap();
    assert_eq!(rt.host.audio_loop_metadata(a, "music/a.ogg").unwrap(), None);
}
