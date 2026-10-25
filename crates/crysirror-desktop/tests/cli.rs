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
    assert_eq!(v["rfvp_native_vm"], false);
    assert_eq!(v["bgi_native_vm"], false);
    assert_eq!(v["reallive_native_vm"], cfg!(feature = "reallive"));
    assert_eq!(
        v["registered_adapters"],
        if cfg!(feature = "reallive") {
            serde_json::json!(["crysirror-script-v1", "reallive", "siglus"])
        } else {
            serde_json::json!(["crysirror-script-v1", "siglus"])
        }
    );
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

#[test]
#[ignore = "requires a real GPU; inspect actual backend capabilities explicitly"]
fn gpu_info_separates_adapter_availability_from_default_device_enablement() {
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
        .arg("gpu-info")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["implemented"]["hardware_raytrace"], true);
    assert_eq!(value["default_device_enabled"]["hardware_raytrace"], false);
    assert_eq!(value["hardware_rt_report"]["enabled"], false);
    let adapters = value["all_adapters"].as_array().expect("adapter reports");
    assert!(!adapters.is_empty());
    for adapter in adapters {
        assert_eq!(adapter["enabled"], false);
        assert_eq!(adapter["ray_query_enabled"], false);
        assert!(adapter["dedicated_rt_hardware_verified"].is_null());
        if adapter["available"] == false {
            assert!(adapter["reason"].is_string());
        }
    }
}

#[test]
#[ignore = "requires a real GPU; exercises RT opt-in or explicit unsupported result"]
fn renderer_cli_requests_rt_only_when_selected_and_reports_encoded_work() {
    let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
        .arg("gpu-info")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let available = report["all_adapters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["available"] == true);
    let dir = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/desktop/arena.json");
    for mode in ["raster", "hardware-raytrace"] {
        let image = dir.path().join(format!("{mode}.png"));
        let output = Command::new(env!("CARGO_BIN_EXE_crysirror"))
            .arg("render")
            .arg(&project)
            .args(["--renderer", mode, "--frames", "1", "--output"])
            .arg(&image)
            .arg("--data-dir")
            .arg(dir.path().join("state"))
            .output()
            .unwrap();
        if mode == "hardware-raytrace" && !available {
            assert!(!output.status.success());
            assert!(
                !image.exists(),
                "unsupported RT must not write a raster fallback image"
            );
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("hardware ray tracing unavailable"));
            continue;
        }
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(image.exists());
        let selected = mode == "hardware-raytrace";
        assert_eq!(value["renderer"], mode);
        assert_eq!(value["hardware_raytracing"], selected);
        assert_eq!(value["hardware_rt_report"]["enabled"], selected);
        assert_eq!(value["stats"]["hardware_rt_report"]["enabled"], selected);
        if selected {
            assert!(
                value["stats"]["hardware_rt_stats"]["primary_dispatches"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            assert!(
                value["stats"]["hardware_rt_stats"]["triangles"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        } else {
            assert!(value["stats"]["hardware_rt_stats"].is_null());
        }
    }
}
