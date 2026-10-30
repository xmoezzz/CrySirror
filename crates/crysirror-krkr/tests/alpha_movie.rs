// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `AlphaMovie.dll` facade over the shared media service. Movie data is
//! local only (`CRYSIRROR_AMV_SAMPLE`); nothing is committed.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn runtime(files: &[(&str, Vec<u8>)], source: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    for (name, bytes) in files {
        std::fs::write(d.path().join("assets").join(name), bytes).unwrap();
    }
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let p = serde_json::from_value(json!({"schema":2,"name":"amv","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-amv","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    (d, r)
}

fn log(r: &Runtime) -> String {
    r.inspect()["sessions"][0]["native"]["log_tail"].to_string()
}

#[test]
fn linking_installs_the_class_and_non_movies_fail_visibly() {
    let (_d, mut r) = runtime(
        &[("bad.amv", b"not a movie".to_vec())],
        r#"
Plugins.link("AlphaMovie.dll");
var m = new AlphaMovie();
Debug.message("class " + (typeof global.AlphaMovie));
try { m.open("bad.amv"); } catch (e) { Debug.message("open failed: " + e.message); }
"#,
    );
    let result = r.tick(1, InputFrame::default());
    let log = log(&r);
    assert!(result.is_ok(), "{result:?} {log}");
    assert!(log.contains("class Object"), "{log}");
    assert!(log.contains("open failed") && log.contains("AlphaMovie header"), "{log}");
}

#[test]
#[ignore = "needs CRYSIRROR_AMV_SAMPLE (a local .amv file)"]
fn local_sample_plays_through_shared_media() {
    let bytes = std::fs::read(std::env::var_os("CRYSIRROR_AMV_SAMPLE").unwrap()).unwrap();
    let (_d, mut r) = runtime(
        &[("op.amv", bytes)],
        r#"
Plugins.link("AlphaMovie.dll");
var w = new Window();
var target = new Layer(w, null);
var m = new AlphaMovie();
m.open("op.amv");
Debug.message("info " + m.numOfFrame + " " + m.FPSRate + "/" + m.FPSScale + " " + m.screenWidth + "x" + m.screenHeight);
m.play();
global.last = -1;
System.addContinuousHandler(function(tick) {
    var f = m.showNextImage(target);
    if (f >= 0 && f \ 30 != global.last \ 30) Debug.message("frame " + f + " layer " + target.imageWidth + "x" + target.imageHeight);
    if (f >= 0) global.last = f;
});
"#,
    );
    for _ in 0..400 {
        r.tick(17, InputFrame::default()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let log = log(&r);
    assert!(log.contains("info "), "{log}");
    assert!(log.contains("frame 30 layer 800x600"), "{log}");
    assert!(log.contains("frame 120 layer"), "picture index never advanced: {log}");
    println!("{log}");
}
