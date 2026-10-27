// Copyright (c) 2026 CrySirror contributors.
// This file is distributed under the GNU Affero General Public License,
// version 3 or (at your option) any later version; see ../krkr-upstream/LICENSE.
//! External original KAG3 scripts are read only at runtime, never distributed.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn copy_tree(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}
#[test]
#[ignore = "requires complete pinned KAG3 checkout and explicitly supplied system font"]
fn original_kag3_startup_executes_scenario_through_engine_services() {
    let source = std::path::PathBuf::from(
        std::env::var_os("CRYSIRROR_KAG3_SOURCE").expect("CRYSIRROR_KAG3_SOURCE"),
    );
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&source)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(head.stdout).unwrap().trim(),
        "1f3ab309106d210e3169bbbe0fb4e066ae463b42"
    );
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    copy_tree(&source.join("data"), &assets);
    // The unmodified stock system loads a self-authored scenario and test images.
    std::fs::write(
        assets.join("scenario/first.ks"),
        r#"*start
[iscript]
Debug.message("CRYSIRROR_KAG3_SCENARIO_STARTED");
[endscript]
Hello CrySirror.[l]
[iscript]
Debug.message("CRYSIRROR_KAG3_AFTER_LINE");
[endscript]
[r]A second line.[p]
[iscript]
Debug.message("CRYSIRROR_KAG3_AFTER_PAGE");
[endscript]
[s]
"#,
    )
    .unwrap();
    for name in ["LineBreak.png", "PageBreak.png"] {
        image::RgbaImage::from_pixel(384, 24, image::Rgba([255, 255, 255, 255]))
            .save(assets.join("image").join(name))
            .unwrap();
    }
    let project:crysirror_runtime::Project=serde_json::from_value(json!({"schema":2,"name":"original KAG3","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"test-kag3","config":{"canvas":[640,480]}}]})).unwrap();
    std::fs::write(
        temp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime = Runtime::with_registry(
        project,
        &temp.path().join("project.json"),
        &temp.path().join("state"),
        registry,
    )
    .unwrap();
    let font = std::env::var_os("CRYSIRROR_TEST_FONT").expect("explicit system font path");
    runtime
        .assets
        .text
        .add_font("default", std::fs::read(font).unwrap())
        .unwrap();
    runtime.host.text = runtime.assets.text.clone();
    fn native(r: &Runtime) -> serde_json::Value {
        r.inspect()["sessions"][0]["native"].clone()
    }
    fn wait(r: &mut Runtime, tag: &str) {
        for _ in 0..600 {
            if let Err(e) = r.tick(16, InputFrame::default()) {
                panic!("{e:#}\n{}", r.inspect());
            }
            let n = native(r);
            if n["kag"]["click_waiting"] == true && n["kag"]["last_tag"] == tag {
                return;
            }
            std::thread::yield_now();
        }
        panic!("KAG did not wait at {tag}: {}", r.inspect());
    }
    wait(&mut runtime, "l");
    let before = native(&runtime);
    assert!(
        before["log_tail"]
            .to_string()
            .contains("CRYSIRROR_KAG3_SCENARIO_STARTED")
    );
    assert!(
        !before["log_tail"]
            .to_string()
            .contains("CRYSIRROR_KAG3_AFTER_LINE")
    );
    let canvases = runtime.inspect()["engine"]["canvases"].to_string();
    assert!(
        canvases.contains("glyph_run"),
        "no retained text: {canvases}"
    );
    assert!(canvases.contains("character"));
    for _ in 0..5 {
        runtime.tick(16, InputFrame::default()).unwrap();
        assert_eq!(native(&runtime)["kag"]["last_tag"], "l");
    }
    runtime
        .tick(
            16,
            InputFrame {
                buttons_pressed: 1,
                ..Default::default()
            },
        )
        .unwrap();
    wait(&mut runtime, "p");
    assert!(
        native(&runtime)["log_tail"]
            .to_string()
            .contains("CRYSIRROR_KAG3_AFTER_LINE")
    );
    assert!(
        !native(&runtime)["log_tail"]
            .to_string()
            .contains("CRYSIRROR_KAG3_AFTER_PAGE")
    );
    runtime
        .tick(
            16,
            InputFrame {
                buttons_pressed: crysirror_core::service::buttons::MOUSE_LEFT,
                pointer_x: 320,
                pointer_y: 240,
                ..Default::default()
            },
        )
        .unwrap();
    for _ in 0..300 {
        runtime.tick(16, InputFrame::default()).unwrap();
        if native(&runtime)["kag"]["sleeping"] == true {
            assert!(
                native(&runtime)["log_tail"]
                    .to_string()
                    .contains("CRYSIRROR_KAG3_AFTER_PAGE")
            );
            return;
        }
    }
    panic!("KAG did not reach stop: {}", runtime.inspect());
}
