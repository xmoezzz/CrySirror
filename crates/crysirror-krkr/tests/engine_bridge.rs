// SPDX-License-Identifier: AGPL-3.0-or-later
//! Original-style TJS2 scripts -> public engine services. Self-authored
//! scripts and generated images only; no game data, fonts or GPU.
use crysirror_core::{
    service::{InputFrame, buttons},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use crysirror_scene::layer::Paint;
use serde_json::json;

fn png(path: &std::path::Path, w: u32, h: u32, rgba: [u8; 4]) {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    img.save(path).unwrap();
}

fn fixture(startup: &str, extra: &[(&str, &str)]) -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    let game = t.path().join("assets/game");
    std::fs::create_dir_all(game.join("system")).unwrap();
    std::fs::create_dir_all(game.join("image")).unwrap();
    std::fs::write(game.join("startup.tjs"), startup).unwrap();
    for (name, body) in extra {
        std::fs::write(game.join(name), body).unwrap();
    }
    png(&game.join("image/Chara.png"), 40, 20, [200, 30, 30, 255]);
    let p: crysirror_runtime::Project = serde_json::from_value(json!({
        "schema": 2, "name": "krkr bridge", "content_revision": "1", "canvas": [640, 480],
        "assets": "assets",
        "clients": [{"name": "native", "adapter": "krkr", "game": "synthetic",
            "config": {"root": "game", "canvas": [640, 480]}}]
    }))
    .unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    (t, r)
}

const SCENE: &str = r#"
Debug.message("boot", System.getTickCount());
Storages.addAutoPath("image/");
Scripts.execStorage("system/lib.tjs");
var win = new Window();
win.setInnerSize(640, 480);
var base = new Layer(win, null);
base.fillRect(0, 0, base.imageWidth, base.imageHeight, 0xff203040);
class Sprite extends Layer {
    var clicks = 0;
    function Sprite(w, p) { super.Layer(w, p); }
    function onClick(x, y) { clicks++; Debug.message("click", x, y, clicks); }
}
var chara = new Sprite(win, base);
chara.loadImages("chara");
chara.setPos(100, 50);
chara.visible = true;
chara.opacity = 200;
var panel = new Layer(win, base);
panel.setImageSize(120, 60);
panel.setSizeToImageSize();
panel.colorRect(0, 0, 120, 60, 0x00ff00, 128);
panel.setPos(300, 300);
panel.visible = true;
global.ticks = 0;
var t = new Timer(function() { ticks++; chara.left += 10; if (ticks == 3) t.enabled = false; }, "");
t.interval = 100;
t.enabled = true;
win.visible = true;
Debug.message("ready", helper(2));
"#;

#[test]
fn tjs_layers_timers_and_clicks_drive_engine_owned_nodes() {
    let (_t, mut r) = fixture(
        SCENE,
        &[("system/lib.tjs", "function helper(x) { return x * 21; }")],
    );
    r.tick(0, InputFrame::default()).unwrap();
    let diag = r.inspect()["sessions"][0]["native"].clone();
    let log = diag["log_tail"].to_string();
    assert!(log.contains("ready 42"), "{diag}");
    assert_eq!(diag["layers"], 3);

    let client = ClientId(1);
    let layers = &r.host.engine.layers;
    // Primary container covers the window; chara and panel are its children.
    let primary = layers.get(client, 2).unwrap();
    assert_eq!(primary.rect, [0., 0., 640., 480.]);
    assert!(primary.visible);
    let chara = layers.get(client, 4).unwrap();
    assert_eq!(chara.parent, Some(2));
    assert_eq!(chara.rect, [100., 50., 40., 20.]);
    assert_eq!(chara.opacity, 200);
    assert!(chara.order[0] > primary.order[0]);
    // The image lives in an engine canvas surface, not a VM framebuffer.
    let surface = r.host.engine.canvases.surface(client, 2).unwrap();
    assert_eq!([surface.width, surface.height], [40, 20]);
    let json = serde_json::to_string(&surface.nodes).unwrap();
    assert!(json.contains("image/Chara.png"), "{json}");

    // Timer fires on the engine clock and is disabled by its own handler.
    for ms in [100, 200, 300, 400, 500] {
        r.tick(ms, InputFrame::default()).unwrap();
    }
    assert_eq!(r.host.engine.layers.get(client, 4).unwrap().rect[0], 130.);

    // A click inside the sprite reaches the script subclass with local coordinates.
    let at = |down| InputFrame {
        pointer_x: 135,
        pointer_y: 55,
        buttons_down: if down { buttons::MOUSE_LEFT } else { 0 },
        buttons_pressed: if down { buttons::MOUSE_LEFT } else { 0 },
        ..Default::default()
    };
    r.tick(600, at(true)).unwrap();
    r.tick(616, at(false)).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("click 5 5 1"), "{log}");

    // Engine-owned state is editable without the VM.
    let fill = r.host.engine.layers.frame([640, 480]).unwrap();
    assert!(
        fill.iter()
            .any(|p| matches!(p.paint, Paint::Solid { color } if color == [0, 255, 0, 128]))
    );
}

fn failure(r: &mut Runtime) -> String {
    assert!(r.tick(0, InputFrame::default()).is_err());
    r.inspect()["sessions"][0]["native"]["error"].to_string()
}

#[test]
fn unsupported_natives_fail_visibly() {
    let (_t, mut r) = fixture(
        r#"var win = new Window(); var base = new Layer(win, null);
           var l = new Layer(win, base); l.loadImages("image/chara");
           l.fillRect(1, 1, 4, 4, 0x80ffffff);"#,
        &[],
    );
    let e = failure(&mut r);
    assert!(e.contains("partial translucent Layer.fillRect"), "{e}");

    let (_t, mut r) = fixture(r#"Plugins.link("extrans.dll");"#, &[]);
    let e = failure(&mut r);
    assert!(e.contains("native DLLs are never loaded"), "{e}");

    let (_t, mut r) = fixture(r#"var w = new Window(); w.setInnerSize(800, 600);"#, &[]);
    let e = failure(&mut r);
    assert!(e.contains("project canvas"), "{e}");
}

#[test]
fn shift_jis_scripts_and_case_insensitive_storage() {
    let (sjis, _, _) = encoding_rs::SHIFT_JIS
        .encode("Debug.message(\"吉里吉里\", Storages.isExistentStorage(\"IMAGE/chara.PNG\"));");
    let t = tempfile::tempdir().unwrap();
    let game = t.path().join("assets/game");
    std::fs::create_dir_all(game.join("image")).unwrap();
    std::fs::write(game.join("startup.tjs"), &*sjis).unwrap();
    png(&game.join("image/Chara.png"), 2, 2, [0, 0, 0, 255]);
    let p: crysirror_runtime::Project = serde_json::from_value(json!({
        "schema": 2, "name": "krkr sjis", "content_revision": "1", "canvas": [640, 480], "assets": "assets",
        "clients": [{"name": "native", "adapter": "krkr", "game": "s", "config": {"root": "game", "canvas": [640, 480]}}]
    }))
    .unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("吉里吉里 1"), "{log}");
}
