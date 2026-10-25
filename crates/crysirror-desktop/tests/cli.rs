// SPDX-License-Identifier: MPL-2.0
use std::{path::Path, process::Command};
fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/project.json")
}
#[test]
fn validation_uses_real_cli() {
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
        .arg("validate")
        .arg(fixture())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["schema_valid"], true);
}
#[test]
fn inspection_reports_only_actually_registered_native_vms() {
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
        .arg("inspect")
        .arg(fixture())
        .output()
        .unwrap();
    assert!(output.status.success());
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["rfvp_native_vm"], cfg!(feature = "rfvp"));
    assert_eq!(v["bgi_native_vm"], cfg!(feature = "bgi"));
    assert_eq!(v["siglus_native_vm"], true);
    let mut expected = vec!["crysirror-script-v1", "siglus"];
    if cfg!(feature = "rfvp") { expected.push("rfvp"); }
    if cfg!(feature = "bgi") { expected.push("bgi"); }
    expected.sort_unstable();
    assert_eq!(v["registered_adapters"], serde_json::json!(expected));
}
#[cfg(feature = "bgi")]
#[test]
fn bgi_inspection_validates_config_without_booting_or_opening_native_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.json");
    let mut project = serde_json::json!({"schema":2,"name":"BGI dormant",
        "content_revision":"1","canvas":[320,180],"assets":"missing-assets",
        "clients":[{"name":"bgi","adapter":"bgi","game":"fixture",
            "config":{"font":"missing-font.ttf"}}]});
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let run = || Command::new(env!("CARGO_BIN_EXE_crysirror")).arg("inspect").arg(&path).output().unwrap();
    let output = run();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["bgi_native_vm"], true);
    assert!(!dir.path().join("missing-assets").exists());
    project["clients"][0]["config"]["strict"] = false.into();
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let output = run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("strict native operation handling"));
}
#[test]
fn trace_is_executed_without_a_display() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
        .arg("trace")
        .arg(fixture())
        .args(["--frames", "3", "--press-at", "1", "--data-dir"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["runtime"]["visuals"], 4);
    assert_eq!(v["pixels_rendered"], false);
}
#[test]
fn shader_check_validates_all_passes_without_creating_gpu_and_rejects_escape() {
    let project =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shader-effects/project.json");
    for (client, asset, valid) in [
        ("game", "waves.wgsl", true),
        ("game", "../Cargo.toml", false),
        ("other", "waves.wgsl", false),
    ] {
        let o = Command::new(env!("CARGO_BIN_EXE_crysirror"))
            .arg("shader-check")
            .arg(&project)
            .args(["--client", client, "--asset", asset])
            .output()
            .unwrap();
        assert_eq!(
            o.status.success(),
            valid,
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        if valid {
            let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
            assert_eq!(v["gpu_created"], false);
            assert_eq!(
                v["stages"],
                serde_json::json!(["color", "prepass", "shadow"])
            );
        }
    }
}
