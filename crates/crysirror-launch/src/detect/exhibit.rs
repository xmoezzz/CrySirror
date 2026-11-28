// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::source_path::SourceFile;
use crysirror_core::path::{Mount, VfsPath};
use std::io::Read;

pub(super) fn detect(dir: &SourcePath, names: &[String], name: &str) -> Result<Option<Plan>> {
    let Some(ini) = find(names, "ExHIBIT.ini") else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    SourceFile::open(dir.join(ini))?
        .take(65537)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "oversized ExHibit configuration");
    // The four structural fields are ASCII even in the CP932 configuration.
    let text = String::from_utf8_lossy(&bytes);
    let mut section = String::new();
    let mut values = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or_default().trim();
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.to_ascii_lowercase();
        } else if let Some((k, v)) = line.split_once('=') {
            values.insert((section.clone(), k.trim().to_ascii_lowercase()), v.trim());
        }
    }
    let field = |section: &str, key: &str| {
        values
            .get(&(section.into(), key.into()))
            .copied()
            .with_context(|| format!("ExHibit configuration missing [{section}] {key}"))
    };
    let scenario = field("release", "scenario")?.replace('\\', "/");
    let dll = VfsPath::new(Mount::Asset, &scenario)?;
    let mut signature = [0; 2];
    SourceFile::open(dir.join(dll.relative()))?.read_exact(&mut signature)?;
    ensure!(&signature == b"MZ", "invalid ExHibit scenario module");
    let entry = field("release", "entry")?;
    let path = crysirror_exhibit::script_path(entry)?;
    let mut header = [0; 8];
    SourceFile::open(dir.join(path.relative()))?.read_exact(&mut header)?;
    ensure!(&header[..4] == b"\0DLR", "invalid ExHibit RLD signature");
    let abi = u32::from_le_bytes(header[4..].try_into()?);
    let canvas = [
        field("setting", "w_view")?.parse()?,
        field("setting", "h_view")?.parse()?,
    ];
    ensure!(
        canvas.iter().all(|v| (1..=16384).contains(v)),
        "invalid ExHibit canvas"
    );
    let capacity = |key: &str| -> Result<usize> { Ok(field("setting", key)?.parse()?) };
    let optional_count = |key: &str, default: usize| -> Result<usize> {
        Ok(values
            .get(&("setting".into(), key.into()))
            .map(|n| n.parse())
            .transpose()?
            .unwrap_or(default))
    };
    let limits = json!({"regs":capacity("n_reg")?,"sys":capacity("n_sysreg")?,"cg":capacity("n_cg")?, "scene":capacity("n_scene")?, "sound":capacity("n_sound")?, "strings":capacity("n_strreg")?, "messages":optional_count("n_message",48000)?, "characters":optional_count("characters", 0)?, "locals":optional_count("n_locreg", 20)?});
    let slots = [
        optional_count("n_usave", 225)?,
        optional_count("n_asave", 25)?,
        optional_count("n_qsave", 3)?,
    ];
    let sources = if abi == 3 {
        let exe = main_exe(dir, names).context("ExHibit executable missing")?;
        let resident = find(names, "resident.dll").context("ExHibit resident module missing")?;
        Some(json!({"exe":exe,"ini":ini,"resident":resident}))
    } else {
        None
    };
    let native_bootstrap = if abi == 3 && entry.eq_ignore_ascii_case("start") {
        let mut module = Vec::new();
        SourceFile::open(dir.join(dll.relative()))?.take(4 << 20).read_to_end(&mut module)?;
        crysirror_resource::pe::PeData::parse(&module).is_ok_and(|pe| {
            matches!((pe.export_rva("start"), pe.export_rva("ExHIBIT")),
                (Ok(0x2440), Ok(0x2880)) | (Ok(0x32710), Ok(0x31de0)))
        })
    } else {
        false
    };
    // INIT_ENTRY passes [setting] CLASS to FCScenario::create, which
    // appends .system.dat beside the game's user directory.
    let save_class = (abi == 1).then(|| values
        .get(&("setting".into(), "class".into())).copied().unwrap_or("ExHIBIT"));
    let mounts = if dir.join("res/res2000.grp").is_file() {
        vec![ArchiveMount {
            fallback: false,
            source: "res/res2000.grp".into(),
            target: "res/s/v".into(),
            options: json!({"format":"exhibit/aifs-voice"}),
        }]
    } else {
        vec![]
    };
    Ok(Some(Plan {
        engine: "ExHibit",
        project: project(
            name,
            canvas,
            "exhibit",
            json!({ "entry":entry,"abi":abi,"canvas":canvas,"sources":sources,"native_bootstrap":native_bootstrap,"limits":limits,"slots":slots,"save_class":save_class }),
        ),
        profiles: Default::default(),
        mounts,
        evidence: vec![],
    }))
}
