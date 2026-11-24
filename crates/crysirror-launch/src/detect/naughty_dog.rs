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
    Ok(Some(Plan {
        engine: "Naughty Dog (Uncharted 2)",
        project: project(name, [1280, 720], crysirror_naughty_dog::ENGINE_ID, json!({})),
        profiles: std::collections::BTreeMap::new(),
        mounts: Vec::new(),
        evidence: Vec::new(),
    }))
}
