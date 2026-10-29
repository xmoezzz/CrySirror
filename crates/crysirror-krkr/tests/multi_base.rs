//! simpleanim.tjs GUIAnimButtonTerminator shape (no game data): a script
//! class with a script base and Layer as its second base, whose Layer part
//! is constructed through `_super[_ctor](...)` from the first base.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

#[test]
fn second_native_base_members_are_reachable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("assets/startup.tjs"),
        r#"
class Wrapper {
    function Wrapper() { _super[_ctor](...); }
    var _super = global.Layer;
    var _ctor = "Layer";
}
class Terminator extends Wrapper, Layer {
    function Terminator(win, parent) {
        var w = global.Wrapper;
        w.Wrapper(win, parent);
        setPos(1, 2, 30, 40);
        imageLeft = 3;
        imageTop = 4;
        _super.visible = 0;
    }
    var _wrap = global.Wrapper;
}
var win = new Window();
var base = new Layer(win, null);
var t = new Terminator(win, base);
Debug.message("ok " + t.imageLeft + " " + t.imageTop + " " + t.width + " " + (t instanceof "Layer"));
"#,
    )
    .unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"mb","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-mb","config":{"canvas":[64,64]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut r = Runtime::with_registry(project, &path, &dir.path().join("state"), registry).unwrap();
    let res = r.tick(0, InputFrame::default());
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(res.is_ok() && log.contains("ok 3 4 30 1"), "{res:?} {log}");
}
