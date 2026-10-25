// SPDX-License-Identifier: MPL-2.0
//! Reproducible, self-authored original-ABI scenes, kept outside the source tree.
use anyhow::{Result, ensure};
use crysirror_siglus_vm::fixture;
use std::path::{Path, PathBuf};
fn main() -> Result<()> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(args.len() == 1, "native_fixture <new external directory>");
    let out = PathBuf::from(&args[0]);
    ensure!(
        out.is_absolute() && !out.exists(),
        "choose a new absolute output directory"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()?;
    let parent = out.parent().unwrap().canonicalize()?;
    ensure!(
        !parent.starts_with(&root),
        "fixture output must remain external"
    );
    std::fs::create_dir(&out)?;
    for (name, scene) in [
        ("rect", fixture::demo_scene()),
        ("image", fixture::image_scene()),
        ("input", fixture::key_wait_scene(true)),
        ("message", fixture::message_scene()),
    ] {
        let dir = out.join(name);
        std::fs::create_dir(&dir)?;
        std::fs::create_dir(dir.join("assets"))?;
        std::fs::write(dir.join("assets/Scene.pck"), fixture::pack_scene(&scene))?;
        if name == "image" {
            std::fs::write(dir.join("assets/native.g00"), fixture::pixel_g00())?;
        }
        let p = serde_json::json!({"schema":2,"name":format!("Native Siglus {name} fixture"),"content_revision":"native-verification-1","canvas":[320,180],"background":[18,22,34,255],"assets":"assets","clients":[{"name":"siglus","adapter":"siglus","game":"native-verification","config":{"scene":"Scene.pck","string_codec":"plain"}}]});
        std::fs::write(dir.join("project.json"), serde_json::to_vec_pretty(&p)?)?;
    }
    println!("{}", out.display());
    Ok(())
}
