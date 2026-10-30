// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Self-authored Yuzusoft-style property hook on `Layer.font` (Utils.tjs
//! objectHookInjection + msghack shape): the hook rebinds to the Font object,
//! where the script stored a helper. No game data.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn log_with(files: &[(&str, &[u8])], source: &str) -> String {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    for (name, bytes) in files {
        std::fs::write(d.path().join("assets").join(name), bytes).unwrap();
    }
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let p = serde_json::from_value(json!({"schema":2,"name":"font hook","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-hook","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let mut r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    let result = r.tick(1, InputFrame::default());
    format!("{:?} {}", result.err(), r.inspect()["sessions"][0]["native"]["log_tail"])
}

#[test]
#[ignore = "diagnostic; needs UTILS_TJS (local game bytecode, never committed)"]
fn real_utils_hook_on_layer_font() {
    let utils = std::fs::read(std::env::var("UTILS_TJS").unwrap()).unwrap();
    let out = log_with(&[("Utils.tjs", &utils)], r#"
global.kirikiriz = true;
try { Scripts.execStorage("Utils.tjs"); } catch (e) { Debug.message("utils load: " + e.message); }
class Hack {
    var layer;
    function Hack(w, root) {
        layer = new Layer(w, root);
        layer.font.fontSetter = fontSetter;
        var d = %[]; d["target"] = layer.font; d["prop"] = "face"; d["get"] = fontGetter; d["set"] = fontSetterFace;
        objectHookInjection(d);
    }
    function fontGetter(ref) { return *ref; }
    function fontSetter(name, ref, v) { *ref = v; global.__seen = name + "=" + v; }
    function fontSetterFace(ref, v) { fontSetter("face", ref, v); }
}
var w = new Window();
var root = new Layer(w, null);
var h = new Hack(w, root);
h.layer.font.face = "Hook";
Debug.message("hooked " + global.__seen + " face=" + h.layer.font.face);
"#);
    assert!(out.contains("hooked face=Hook face=Hook"), "{out}");
}

#[test]
fn layer_font_writes_run_unbound_property_setters_on_the_font() {
    // `Layer.font` is self-bound (krkr `tTJSVariant(dsp, dsp)`), so an
    // unbound property stored on it runs with `this` = the Font, where the
    // script put its helper -- not with the caller's `this`.
    let out = log_with(&[], r#"
class Hack {
    var layer;
    function Hack(w, root) {
        layer = new Layer(w, root);
        layer.font.helper = helper;
        var holder = new Dictionary();
        var make = function() {
            ("property prop { setter(v) { helper(v); } getter { return 0; } }")!;
            return &this.prop;
        } incontextof holder;
        var p = make();
        &layer.font.face = p incontextof null;
    }
    function helper(v) { global.__seen = v; }
}
var w = new Window();
var h = new Hack(w, new Layer(w, null));
h.layer.font.face = "Hook";
Debug.message("hooked " + global.__seen);
"#);
    assert!(out.contains("hooked Hook"), "{out}");
}

#[test]
fn window_subclass_starts_with_an_engine_draw_device() {
    // krkrz: `new KAGWindow()` (a Window subclass) has a BasicDrawDevice
    // whose `recreate()` scripts call from resetScreenMode.
    let out = log_with(&[], r#"
class KW extends Window { function KW() { super.Window(); } }
var k = new KW();
k.drawDevice.recreate();
Debug.message("device " + (k.drawDevice instanceof "BasicDrawDevice"));
"#);
    assert!(out.contains("device 1"), "{out}");
}
