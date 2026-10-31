//! Public reflective Media header jobs do not need a native VM/device/frame.
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use std::time::{Duration, Instant};
#[test]
fn media_probe_facade_publishes_without_playback_tick_and_invalidates_restore_tickets() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(
        root.path().join("assets/probe.mpg"),
        [
            0, 0, 1, 0xb3, 0x01, 0x00, 0x10, 0x13, 0xff, 0xff, 0xe0, 0x18,
        ],
    )
    .unwrap();
    std::fs::write(root.path().join("project.json"),br#"{"schema":2,"name":"media-probe","content_revision":"1","assets":"assets","canvas":[16,16],"clients":[{"name":"app","adapter":"crysirror-script-v1","game":"probe"}]}"#).unwrap();
    let runtime = Runtime::load(
        &root.path().join("project.json"),
        &root.path().join("state"),
    )
    .unwrap();
    let mut app = App::new(runtime, "app").unwrap();
    let media = app.context().singleton("Media").unwrap();
    let id = app
        .context()
        .call(media, "beginProbe", &[Value::Str("probe.mpg".into())])
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ready = loop {
        let value = app
            .context()
            .call(media, "probeStatus", &[id.clone()])
            .unwrap()
            .to_json();
        match value["state"].as_str() {
            Some("ready") => break value,
            Some("failed") => panic!("{value}"),
            _ => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    };
    assert_eq!(ready["info"]["size"], serde_json::json!([16, 16]));
    assert_eq!(ready["info"]["frame_ms"], 40.);
    assert_eq!(ready["info"]["audio_decoder"], serde_json::Value::Null);
    assert!(!app.runtime().unwrap().host.media.has_resources());
    assert_eq!(app.runtime().unwrap().now_ms(), 0);
    let saved = app.runtime().unwrap().snapshot().unwrap();
    app.runtime_mut().unwrap().restore(&saved).unwrap();
    assert!(app.context().call(media, "probeStatus", &[id]).is_err());
    let next = app
        .context()
        .call(media, "beginProbe", &[Value::Str("probe.mpg".into())])
        .unwrap();
    app.context()
        .call(media, "releaseProbe", &[next.clone()])
        .unwrap();
    assert!(app.context().call(media, "probeStatus", &[next]).is_err());
}
