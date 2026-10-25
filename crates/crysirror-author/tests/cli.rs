// SPDX-License-Identifier: MPL-2.0
use std::{path::Path, process::Command};

#[test]
fn cli_emits_a_valid_project() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/arena.ts");
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror-author"))
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project: crysirror_runtime::Project = serde_json::from_slice(&output.stdout).unwrap();
    project.validate().unwrap();
    assert_eq!(project.scene3d.unwrap().meshes.len(), 10);
}

#[test]
fn cli_failure_emits_no_project() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("bad.ts");
    std::fs::write(&source, "function buildProject() { return {}; }").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror-author"))
        .arg(source)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("generated project JSON"));
}
