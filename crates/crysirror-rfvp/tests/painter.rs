// SPDX-License-Identifier: MPL-2.0
//! Runs rfvp's own win95_painter_demo HCB in the vendored original engine
//! through the CrySirror runtime. Needs a host font (none is bundled):
//! `CRYSIRROR_TEST_FONT=/path/font.ttf cargo test -p crysirror-rfvp -- --ignored`
//! It prints FNV-1a framebuffer hashes so a run with the upstream MS Gothic
//! file can be compared with the unmodified upstream `rfvp_os_host`
//! (docs/rfvp-differential.json).
use crysirror_core::service::{buttons, InputFrame};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::path::{Path, PathBuf};

fn font() -> PathBuf {
    PathBuf::from(std::env::var("CRYSIRROR_TEST_FONT").expect("set CRYSIRROR_TEST_FONT"))
}
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}
fn runtime(tmp: &Path) -> Runtime {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rfvp-painter");
    // Fetched by `tools/fetch_demo_assets.py rfvp-painter` (kept out of the tree).
    let game = std::env::var_os("CRYSIRROR_RFVP_PAINTER_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../local/demos/rfvp-painter/assets")
        });
    copy_dir(&game.join("game"), &tmp.join("assets/game"));
    std::fs::create_dir_all(tmp.join("assets/font")).unwrap();
    std::fs::copy(font(), tmp.join("assets/font/host-font.ttf")).unwrap();
    std::fs::copy(example.join("project.json"), tmp.join("project.json")).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_rfvp::register(&mut registry).unwrap();
    let path = tmp.join("project.json");
    Runtime::with_registry(
        Project::load(&path).unwrap(),
        &path,
        &tmp.join("state"),
        registry,
    )
    .unwrap()
}
fn surface(rt: &Runtime) -> (u32, Vec<u8>) {
    let id = rt.client_id("painter").unwrap();
    let s = rt.assets.surface(id, "__surface/0").expect("RFVP surface");
    (s.width, s.rgba.to_vec())
}
fn pixel(rt: &Runtime, x: u32, y: u32) -> [u8; 4] {
    let (w, rgba) = surface(rt);
    let i = ((y * w + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}
fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325u64, |h, x| {
        (h ^ u64::from(*x)).wrapping_mul(0x100000001b3)
    })
}
fn native(rt: &Runtime) -> serde_json::Value {
    rt.inspect()["sessions"][0]["native"].clone()
}

#[test]
#[ignore = "requires a caller-supplied font via CRYSIRROR_TEST_FONT; none bundled"]
fn original_rfvp_painter_boots_renders_and_receives_input() {
    let tmp = tempfile::tempdir().unwrap();
    let mut rt = runtime(tmp.path());
    for _ in 0..120 {
        rt.tick(16, InputFrame::default()).unwrap();
    }
    let n = native(&rt);
    assert_eq!(n["vm"], "original-rfvp");
    assert_eq!(n["title"], "Win95-Painter");
    assert_eq!(n["screen"], serde_json::json!([1024, 640]));
    assert_eq!(n["threads"][0], 1, "main script context running");
    // The original software renderer drew its UI; the shared Layer shows it.
    assert_eq!(pixel(&rt, 300, 10), [0, 0, 128, 255], "navy title bar");
    assert_eq!(pixel(&rt, 300, 300), [255, 255, 255, 255], "white canvas");
    assert_eq!(rt.host.engine.frame([1024, 640]).unwrap().len(), 1);
    println!("boot_hash={:016x}", fnv(&surface(&rt).1));

    // Same input sequence as the upstream differential harness.
    let mut f = InputFrame {
        pointer_x: 250,
        pointer_y: 300,
        ..Default::default()
    };
    rt.tick(16, f).unwrap();
    f.buttons_down = buttons::MOUSE_LEFT;
    f.buttons_pressed = buttons::MOUSE_LEFT;
    rt.tick(16, f).unwrap();
    // The edge reached the original InputManager: LeftClick(4) | MouseLeft(16).
    assert_eq!(native(&rt)["input"], serde_json::json!([20, 20, 250, 300]));
    f.buttons_pressed = 0;
    for x in (250..=420).step_by(10) {
        f.pointer_x = x;
        rt.tick(16, f).unwrap();
    }
    f.buttons_down = 0;
    for _ in 0..5 {
        rt.tick(16, f).unwrap();
    }
    println!("final_hash={:016x}", fnv(&surface(&rt).1));
    // Runtime snapshots are refused visibly rather than silently partial.
    assert!(rt.snapshot().is_err());
}
