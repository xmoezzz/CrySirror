// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free native adapter probe using the ordinary Runtime/SharedHost.
//! A detected plan and explicit game root are supplied by the normal detector.
//! No original executable, private renderer or simulated-success host is used.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{FileSystem, InputFrame},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::path::PathBuf;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 5,
        "native_probe <detected-plan.json> <game-root> <state-root> <report.json> <frames>"
    );
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let project: Project =
        serde_json::from_value(plan.get("project").context("detected project")?.clone())?;
    ensure!(
        project.clients.iter().all(|c| c.adapter == "siglus"),
        "probe currently supports the default Siglus registry only"
    );
    ensure!(
        plan["mounts"].as_array().is_some_and(|m| m.is_empty()),
        "probe requires an archive-free detected plan"
    );
    let frames: u32 = args[4].to_str().context("frames UTF-8")?.parse()?;
    ensure!((1..=10_000).contains(&frames), "frame budget");
    let mut rt = Runtime::with_asset_root(
        project,
        &PathBuf::from(&args[1]),
        &PathBuf::from(&args[2]),
        AdapterRegistry::default(),
    )?;
    // Optional bounded static inspection of original scene data. This does
    // not change a VM entry point, instruction, resource or execution result.
    if let Some(names) = std::env::var_os("CRYSIRROR_PROBE_SCENES") {
        let config: crysirror_siglus_vm::adapter::Config =
            serde_json::from_value(rt.project.clients[0].config.clone())?;
        ensure!(
            config.format == "pck",
            "scene inspection requires a native pack"
        );
        let materials = if let Some(path) = config.key_file.as_ref() {
            let bytes = rt
                .host
                .read(ClientId(1), &VfsPath::new(Mount::Asset, path)?)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            crysirror_siglus_vm::key_toml::parse_key_toml(std::str::from_utf8(&bytes)?)?
        } else {
            Default::default()
        };
        let mut options = crysirror_siglus_vm::scene_pck::ScenePckDecodeOptions {
            exe_angou_element: materials.exe_key16.map(|key| key.to_vec()),
            easy_angou_code: materials.scene_angou_code,
            ..Default::default()
        };
        options.string_encryption_override = if config.string_codec == "plain" {
            crysirror_siglus_vm::key_toml::StringEncryptionOverride::None
        } else {
            crysirror_siglus_vm::key_toml::StringEncryptionOverride::Xor
        };
        let bytes = rt
            .host
            .read(ClientId(1), &VfsPath::new(Mount::Asset, &config.scene)?)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let pack = crysirror_siglus_vm::scene_pck::ScenePck::load_lazy_from_bytes(bytes, &options)?;
        let names = names
            .to_str()
            .context("inspection scene names UTF-8")?
            .split(',')
            .collect::<Vec<_>>();
        ensure!(names.len() <= 16, "inspection scene budget");
        for name in names {
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "inspection scene name"
            );
            let no = pack
                .find_scene_no(name)
                .context("inspection scene not found")?;
            let data = pack.scn_data_slice(no)?;
            ensure!(
                data.len() <= 16 * 1024 * 1024,
                "inspection scene byte budget"
            );
            let path = PathBuf::from(&args[3]).with_file_name(format!("{name}.scn"));
            std::fs::write(path, &*data)?;
        }
    }
    if let Some(font) = std::env::var_os("CRYSIRROR_TEST_FONT") {
        let bytes = std::fs::read(font)?;
        rt.host.text.add_font("default", bytes.clone())?;
        rt.assets.text.add_font("default", bytes)?;
    }
    let mut error = None;
    let mut completed = 0;
    for _ in 0..frames {
        match rt.tick(16, InputFrame::default()) {
            Ok(()) => completed += 1,
            Err(e) => {
                error = Some(format!("{e:#}"));
                break;
            }
        }
    }
    let report = serde_json::json!({"frames_completed":completed,"requested_frames":frames,"gpu_rendered":false,"error":error,"runtime":rt.inspect()});
    std::fs::write(&args[3], serde_json::to_vec_pretty(&report)?)?;
    println!(
        "completed={completed}/{frames}; {}",
        error.as_deref().unwrap_or("no native error")
    );
    if error.is_some() {
        std::process::exit(1);
    }
    Ok(())
}
