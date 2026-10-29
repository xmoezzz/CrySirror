//! system/world.tjs + Action.tjs (real game shape, no game data): a method
//! stores `function() { _stop(); }` in a dictionary; Action.tjs later runs it
//! through `isNullContext(f) ? (f incontextof target) : f`.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

#[test]
fn completion_literal_stored_in_dictionary_rebinds_to_action_target() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("assets/startup.tjs"),
        r#"
Plugins.link("scriptsEx.dll");
function getCompleteFunc(f, target) { return Scripts.isNullContext(f) ? (f incontextof target) : f; }
class Snd {
    var stopped = 0;
    function _stop() { stopped = 1; }
    function stop() {
        var a = %[], d = %[];
        d["complete"] = function() { _stop(); };
        a["visvalue"] = d;
        return a;
    }
}
// Action.tjs PropAction: copies the callback into its own member first.
class PropAction {
    var complete;
    function PropAction(dict) { complete = dict.complete; }
    function doComplete(target) { var f = getCompleteFunc(complete, target); f(); }
}
var s2 = new Snd();
(new PropAction(s2.stop().visvalue)).doComplete(s2);
Debug.message("member-copied stopped " + s2.stopped);
var s = new Snd();
var d = s.stop().visvalue;
var f = d.complete;
Debug.message("null-context " + Scripts.isNullContext(f) + " direct " + Scripts.isNullContext(d["complete"]));
var g = getCompleteFunc(f, s);
g();
Debug.message("stopped " + s.stopped);
"#,
    )
    .unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"closure","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-closure","config":{"canvas":[64,64]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime = Runtime::with_registry(project, &path, &dir.path().join("state"), registry).unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("member-copied stopped 1"), "{log}");
    assert!(log.contains("\nstopped 1") || log.contains("\"stopped 1"), "{log}");
}

/// yuzu_default.tjs `propDrawTextParamMapTable`: a table written with
/// `saveStruct(.., "b")` is read back through `Scripts.evalStorage`.
#[test]
fn eval_storage_returns_a_binary_struct_pack() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("assets/startup.tjs"),
        r#"
var d = %[ font: "x", size: 26, list: [1, 2] ];
(Dictionary.saveStruct incontextof d)(System.dataPath + "prefs.tjs", "b");
var r = Scripts.evalStorage(System.dataPath + "prefs.tjs");
Debug.message("pack " + r.font + r.size + r.list[1]);
"#,
    )
    .unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"pack","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-pack","config":{"canvas":[64,64]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime = Runtime::with_registry(project, &path, &dir.path().join("state"), registry).unwrap();
    // The save completes on a worker; the script resumes after it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        runtime.tick(1, InputFrame::default()).unwrap();
        let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        if log.contains("pack ") {
            assert!(log.contains("pack x262"), "{log}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{log}");
        std::thread::yield_now();
    }
}
