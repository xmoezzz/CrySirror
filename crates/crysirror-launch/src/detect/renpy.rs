// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::source_path::SourceFile;
use crysirror_renpy::{adapter::ENGINE_ID, rt::state::Profile};
use std::io::Read;

pub(super) fn detect(dir: &SourcePath, name: &str) -> Result<Option<Plan>> {
    let game = dir.join("game");
    let source = dir.join("renpy/__init__.py");
    if !game.is_dir() || !source.is_file() {
        return Ok(None);
    }
    let root = dir.canonicalize()?;
    let source = source.canonicalize()?;
    let game = game.canonicalize()?;
    ensure!(
        source.starts_with(&root) && game.starts_with(&root),
        "Ren'Py installation escapes source root"
    );
    let read = |path: SourcePath| -> Result<String> {
        let mut bytes = Vec::new();
        SourceFile::open(path)?
            .take(128 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 128 * 1024,
            "oversized Ren'Py version manifest"
        );
        Ok(String::from_utf8(bytes)?)
    };
    let init = read(source)?;
    // Built releases record the build in renpy/vc_version.py (8.x: the
    // whole version string).
    let vc = dir.join("renpy/vc_version.py");
    let vc = if vc.is_file() {
        let vc = vc.canonicalize()?;
        ensure!(vc.starts_with(&root), "Ren'Py installation escapes source root");
        Some(read(vc)?)
    } else {
        None
    };
    let Some(version) = Profile::detect(&init, vc.as_deref()) else {
        return Ok(None);
    };
    ensure!(
        Profile::supported(version),
        "unsupported Ren'Py runtime {}.{}.{}.{}",
        version[0],
        version[1],
        version[2],
        version[3]
    );

    let names = listing(&game)?;
    // Ren'Py 6.99 main.py sorts configured archives in reverse name order.
    let mounts: Vec<_> = names
        .iter()
        .rev()
        .filter(|name| name.ends_with(".rpa"))
        .enumerate()
        .map(|(i, name)| ArchiveMount {
            fallback: false,
            source: format!("game/{name}"),
            target: format!("renpy-archives/{i}"),
            options: json!({"format":"renpy/rpa"}),
        })
        .collect();
    ensure!(mounts.len() <= 64, "too many Ren'Py archives");
    let mounted: Vec<_> = mounts.iter().map(|mount| &mount.target).collect();

    // The screen size the game's top-level sources set (gui.rpy,
    // options.rpy); otherwise 800x600, upstream's default. The runtime
    // fits the virtual screen into this canvas either way.
    let mut sources = Vec::new();
    for name in names.iter().filter(|n| n.ends_with(".rpy")).take(64) {
        let path = game.join(name);
        let mut text = String::new();
        if SourceFile::open(path)?.take(4 << 20).read_to_string(&mut text).is_ok() {
            sources.push(text);
        }
    }
    let canvas = crysirror_renpy::rt::state::screen_size_hint(sources.iter().map(String::as_str)).map_or([800, 600], |[w, h]| [w as u32, h as u32]);
    Ok(Some(Plan {
        engine: "Ren'Py",
        project: project(
            name,
            canvas,
            ENGINE_ID,
            json!({"version":version,"canvas":canvas,"archives":mounted}),
        ),
        profiles: Default::default(),
        mounts,
        evidence: vec![],
    }))
}
