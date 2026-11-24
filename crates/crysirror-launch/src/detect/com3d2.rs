// SPDX-License-Identifier: MPL-2.0
//! COM3D2's standalone 2.1 archive stack (GameUty.UpdateFileSystemPath).
use super::*;
use crysirror_cm3d2::binio::Reader;
use std::io::Read;

fn paths(dir: &SourcePath) -> Result<Vec<String>> {
    let mut bytes = Vec::new();
    crate::source_path::SourceFile::open(dir.join("paths.dat"))?
        .take(1 << 20)
        .read_to_end(&mut bytes)?;
    let mut r = Reader::new(&bytes);
    r.expect_str("CM3D2_PATHS")?;
    let _version = r.i32()?;
    let count = r.count()?;
    ensure!(count <= 4096, "COM3D2 path list limit");
    let mut out = Vec::new();
    for _ in 0..count {
        let suffix = r.str()?;
        ensure!(
            !suffix.contains(['/', '\\', '\0']),
            "invalid COM3D2 archive suffix"
        );
        out.push(suffix);
    }
    Ok(out)
}

fn archive_name<'a>(names: &'a [String], stem: &str) -> Option<&'a String> {
    find(names, &format!("{stem}.arc")).or_else(|| find(names, &format!("{stem}.arc.bz2")))
}

fn series(stems: &mut Vec<String>, base: &str, suffixes: &[String], names: &[String]) {
    for suffix in suffixes {
        let stem = format!("{base}_{suffix}");
        if archive_name(names, &stem).is_some() {
            stems.extend([stem.clone(), format!("{stem}_2"), format!("{stem}_3")]);
        }
    }
}

pub(super) fn detect(dir: &SourcePath, names: &[String], name: &str) -> Result<Option<Plan>> {
    let Some(data) = find(names, "COM3D2x64_Data") else {
        return Ok(None);
    };
    let player = dir.join(data);
    if !player.join("globalgamemanagers").is_file() {
        return Ok(None);
    }
    let mut info = String::new();
    crate::source_path::SourceFile::open(player.join("app.info"))?
        .take(512)
        .read_to_string(&mut info)?;
    if !info.lines().eq(["KISS", "CUSTOM ORDER MAID 3D 2"]) {
        return Ok(None);
    }
    let mut header = [0; 28];
    crate::source_path::SourceFile::open(player.join("globalgamemanagers"))?
        .read_exact(&mut header)?;
    ensure!(
        header[8..12] == 17u32.to_be_bytes() && &header[20..28] == b"5.6.4p2\0",
        "unsupported COM3D2 Unity profile"
    );
    let root = dir.canonicalize()?;
    let mut mounts = Vec::new();
    let mut targets = Vec::new();
    for folder in ["GameData_20", "GameData"] {
        let Some(folder_name) = find(names, folder) else {
            ensure!(folder != "GameData", "missing COM3D2 GameData");
            continue;
        };
        let source = dir.join(folder_name);
        let names = listing(&source)?;
        let list = if source.join("paths.dat").is_file() {
            paths(&source)?
        } else {
            Vec::new()
        };
        let mut stems = Vec::new();
        if folder == "GameData_20" {
            if list.is_empty() {
                continue;
            }
            for base in ["material", "menu", "model", "texture"] {
                series(&mut stems, base, &list, &names);
            }
            stems.push("prioritymaterial".into());
        } else {
            for (base, extras) in [
                ("csv", &["prioritymaterial"][..]),
                ("motion", &["motion2"]),
                ("script", &[]),
                ("script_share", &["script_share2"]),
                ("sound", &["sound2"]),
                ("texture", &["texture2", "texture3"]),
                ("system", &["system2"]),
                ("bg", &[]),
                ("voice", &["voice2", "voice3"]),
                ("parts", &["parts2"]),
            ] {
                if base != "bg" {
                    stems.push(base.to_owned());
                }
                if base == "voice" {
                    stems.extend((b'a'..=b'y').map(|c| format!("voice_{}", c as char)));
                }
                series(&mut stems, base, &list, &names);
                stems.extend(extras.iter().map(|s| s.to_string()));
            }
        }
        for stem in stems {
            let Some(file) = archive_name(&names, &stem) else {
                continue;
            };
            let path = source.join(file).canonicalize()?;
            ensure!(path.starts_with(&root), "archive escapes COM3D2 root");
            let mut magic = [0; 8];
            crate::source_path::SourceFile::open(path)?.read_exact(&mut magic)?;
            ensure!(
                if file.to_ascii_lowercase().ends_with(".bz2") {
                    &magic[..3] == b"BZh" && matches!(magic[3], b'1'..=b'9')
                } else {
                    magic == *b"warc\xff\xaa\x45\xf1"
                },
                "invalid KISS archive {folder}/{file}"
            );
            let target = format!("__com3d2/{folder}/{stem}");
            mounts.push(ArchiveMount {
                fallback: false,
                source: format!("{folder_name}/{file}"),
                target: target.clone(),
                options: json!({"format":"kiss/warc","profile":"game"}),
            });
            targets.push(target);
        }
    }
    for required in ["script", "system", "parts", "sound"] {
        ensure!(
            targets.contains(&format!("__com3d2/GameData/{required}")),
            "missing COM3D2 {required}.arc"
        );
    }
    Ok(Some(Plan {
        engine: "COM3D2",
        project: project(
            name,
            [1920, 1080],
            "com3d2",
            json!({"data_dir":data,"archives":targets}),
        ),
        mounts,
        profiles: std::collections::BTreeMap::from([(
            "game".into(),
            json!({"limits":{"max_entry_size":1073741824u64}}),
        )]),
        evidence: Vec::new(),
    }))
}
