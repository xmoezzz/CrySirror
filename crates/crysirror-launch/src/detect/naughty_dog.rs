//! Uncharted 2: Among Thieves (PS3 disc BCAS20097): the installed
//! `PS3_GAME` tree with `USRDIR/build/main/pak22.psarc`. Opens no
//! executable; the encrypted or decrypted EBOOT is never read.
use super::*;

pub(super) fn detect(dir: &SourcePath, names: &[String], name: &str) -> Result<Option<Plan>> {
    let Some(game) = find(names, "PS3_GAME").filter(|d| dir.join(d).is_dir()).cloned() else {
        return Ok(None);
    };
    let main = dir.join(&game).join("USRDIR").join("build").join("main");
    if !main.join("pak22.psarc").is_file() || !main.join("pak22.txt").is_file() {
        return Ok(None);
    }
    ensure!(game == "PS3_GAME", "Naughty Dog data must be under PS3_GAME");
    let mounts = ["sound1/music", "sound1/streams", "sound1/speech/english"]
        .into_iter()
        .filter(|stem| main.join(&format!("{stem}.psarc")).is_file())
        .map(|stem| {
            let target = format!("PS3_GAME/USRDIR/build/main/{stem}");
            ArchiveMount {
                source: format!("{target}.psarc"),
                target,
                fallback: false,
                options: json!({"format": "sony/psarc", "profile": "nd-soundtrack"}),
            }
        })
        .collect();
    Ok(Some(Plan {
        engine: "Naughty Dog (Uncharted 2)",
        project: project(name, [1280, 720], crysirror_naughty_dog::ENGINE_ID, json!({})),
        // The credits mix is 75 MiB; the largest music entry is 96 MiB.
        // PSARC serves both by block, independent of the PCM working set.
        profiles: std::collections::BTreeMap::from([(
            "nd-soundtrack".into(),
            json!({"limits": {"max_entry_size": 128u64 << 20}}),
        )]),
        mounts,
        evidence: Vec::new(),
    }))
}
