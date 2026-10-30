// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
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
