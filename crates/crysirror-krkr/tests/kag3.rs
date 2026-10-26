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
    std::fs::write(assets.join("scenario/first.ks"),"*start\n[iscript]\nDebug.message(\"CRYSIRROR_KAG3_SCENARIO_STARTED\");\n[endscript]\nHello CrySirror.[l][r]\nA second line.[p]\n[s]\n").unwrap();
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
    for _ in 0..300 {
        if let Err(error) = runtime.tick(16, InputFrame::default()) {
            panic!("{error:#}\n{}", runtime.inspect());
        }
        let state = runtime.inspect();
        if state["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("CRYSIRROR_KAG3_SCENARIO_STARTED")
        {
            assert!(
                runtime
                    .host
                    .engine
                    .layers
                    .get(crysirror_core::state::ClientId(1), 2)
                    .is_some()
            );
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "original KAG3 scenario did not start: {}",
        runtime.inspect()
    );
}
