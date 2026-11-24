// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! VideoOverlay transport over the shared media service: pause/resume,
//! rewind and frame seeks keep krkrz's immediate status events. The movie
//! is the engine's own synthetic MPEG fixture.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

#[test]
fn pause_resume_rewind_and_frame_seek_keep_native_status() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(
        d.path().join("assets/movie.mpg"),
        include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg"),
    )
    .unwrap();
    std::fs::write(
        d.path().join("assets/startup.tjs"),
        r#"
class V extends VideoOverlay {
    function V(w) { super.VideoOverlay(w); }
    function onStatusChanged(s) { Debug.message("status " + s); }
}
var w = new Window();
var v = new V(w);
v.open("movie.mpg");
v.play();
v.pause();
Debug.message("now " + v.status);
v.frame = 5;
v.rewind();
v.play();
Debug.message("now " + v.status);
v.stop();
"#,
    )
    .unwrap();
    let p = serde_json::from_value(json!({"schema":2,"name":"video","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-video","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let mut r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    for _ in 0..4 {
        let result = r.tick(16, InputFrame::default());
        assert!(result.is_ok(), "{result:?}");
    }
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    // log_tail is newest first.
    let order: Vec<&str> = ["status stop", "status play", "status pause", "now pause", "status play", "now play", "status stop"]
        .into_iter()
        .collect();
    let mut at = log.len();
    for line in order {
        let found = log[..at].rfind(line).unwrap_or_else(|| panic!("missing `{line}` in order: {log}"));
        at = found;
    }
}

#[test]
fn overlay_uses_the_games_mpeg_copy_only_for_unsupported_asf_audio() {
    let project = tempfile::tempdir().unwrap();
    let assets = project.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    let wmv = include_bytes!("../../crysirror-video/tests/fixtures/wmv_wma.asf");
    let mpeg = include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg");
    let mut unsupported = wmv.to_vec();
    // WAVEFORMATEX starts 54 bytes after the ASF audio stream type GUID.
    let audio_guid = [0x40, 0x9e, 0x69, 0xf8, 0x4d, 0x5b, 0xcf, 0x11,
                      0xa8, 0xfd, 0x00, 0x80, 0x5f, 0x5c, 0x44, 0x2b];
    let at = unsupported.windows(audio_guid.len()).position(|w| w == audio_guid).unwrap() + 54;
    assert_eq!(&unsupported[at..at + 2], &[0x61, 0x01]);
    unsupported[at..at + 2].copy_from_slice(&[0x63, 0x01]);
    std::fs::write(assets.join("high.wmv"), &unsupported).unwrap();
    std::fs::write(assets.join("high_sd.mpg"), mpeg).unwrap();
    std::fs::write(assets.join("supported.wmv"), wmv).unwrap();
    std::fs::write(assets.join("supported_sd.mpg"), mpeg).unwrap();
    std::fs::write(assets.join("startup.tjs"), r#"
var w = new Window();
var v = new VideoOverlay(w);
v.open("high.wmv");
v.prepare();
v.open("supported.wmv");
v.prepare();
"#).unwrap();
    let spec = serde_json::from_value(json!({
        "schema":2,"name":"video","content_revision":"1","canvas":[32,32],"assets":"assets",
        "clients":[{"name":"native","adapter":"krkr","game":"synthetic-video","config":{"canvas":[32,32]}}]
    })).unwrap();
    let path = project.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&spec).unwrap()).unwrap();
    let mut adapters = AdapterRegistry::default();
    crysirror_krkr::register(&mut adapters).unwrap();
    let mut runtime = Runtime::with_registry(spec, &path, &project.path().join("state"), adapters).unwrap();
    runtime.tick(16, InputFrame::default()).unwrap();
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("VideoOverlay.prepare high_sd.mpg"), "{log}");
    assert!(log.contains("VideoOverlay.prepare supported.wmv"), "{log}");
}
