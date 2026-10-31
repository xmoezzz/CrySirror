//! Majiro native boot over shared archives. Never launches install/executable.
use super::*;
use std::io::Read;
pub(super) fn detect(dir: &Path, names: &[String], name: &str) -> Result<Option<Plan>> {
    let root = dir.canonicalize()?;
    ensure!(names.len() <= 32768, "Majiro directory metadata budget");
    ensure!(
        with_ext(names, "arc").count() <= 64,
        "Majiro archive probe budget"
    );
    let mut archives = Vec::new();
    for file in with_ext(names, "arc") {
        let full = dir.join(file).canonicalize()?;
        ensure!(
            full.starts_with(&root),
            "Majiro archive probe escapes game root"
        );
        let mut f = std::fs::File::open(full)?;
        let mut h = [0; 16];
        if f.read_exact(&mut h).is_ok()
            && (h == *b"MajiroArcV2.000\0" || h == *b"MajiroArcV3.000\0")
        {
            archives.push(file.clone());
        }
    }
    if archives.is_empty() {
        return Ok(None);
    }
    #[cfg(not(feature = "majiro"))]
    {
        let _ = (name, archives);
        bail!("recognized Majiro ARC in {}; native adapter 'majiro' is not linked; enable feature majiro",dir.display());
    }
    #[cfg(feature = "majiro")]
    {
        native_plan(dir, name, archives).map(Some)
    }
}
#[cfg(feature = "majiro")]
fn native_plan(dir: &Path, name: &str, archives: Vec<String>) -> Result<Plan> {
    use crysirror_core::{service::AssetReader, state::ClientId};
    use crysirror_resource::{read_all, ArchiveFormat, DecodeContext};
    use std::sync::{Arc, Mutex};
    struct File(Mutex<std::fs::File>);
    impl AssetReader for File {
        fn len(&self) -> crysirror_core::Result<u64> {
            self.0
                .lock()
                .map_err(|_| crysirror_core::Error::Invalid("poisoned Majiro reader"))?
                .metadata()
                .map(|m| m.len())
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
        fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            use std::io::{Seek, SeekFrom};
            let mut f = self
                .0
                .lock()
                .map_err(|_| crysirror_core::Error::Invalid("poisoned Majiro reader"))?;
            f.seek(SeekFrom::Start(at))
                .and_then(|_| f.read(out))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    ensure!(archives.len() <= 64, "Majiro archive mount budget");
    let mut cx = DecodeContext::new(ClientId(1));
    cx.limits.max_entry_size = 1024 * 1024 * 1024;
    let root = dir.canonicalize()?;
    let mut mounts = Vec::new();
    let mut env = None;
    let mut environment = None;
    let mut script = None;
    for (index, file) in archives.into_iter().enumerate() {
        let full = dir.join(&file).canonicalize()?;
        ensure!(full.starts_with(&root), "Majiro archive escapes game root");
        let a = crysirror_formats_arc::MajiroArc
            .open(Arc::new(File(Mutex::new(std::fs::File::open(full)?))), &cx)?;
        let target = format!("__majiro/{index}");
        for e in a.entries() {
            if e.path.eq_ignore_ascii_case("majiro.env")
                && (file.to_ascii_lowercase().starts_with("fastdata")
                    || file.to_ascii_lowercase().starts_with("update"))
            {
                ensure!(
                    env.is_none(),
                    "multiple Majiro ENV providers require native patch precedence mapping"
                );
                env = Some(read_all(a.open(e.id, &cx)?.as_ref(), 256 * 1024)?);
                environment = Some(format!("{target}/{}", e.path));
            }
            if e.path.eq_ignore_ascii_case("start.mjo") {
                ensure!(
                    script.is_none(),
                    "multiple Majiro entry providers require native patch precedence mapping"
                );
                script = Some(format!("{target}/{}", e.path));
            }
        }
        mounts.push(ArchiveMount {
            fallback:false,
            source: file,
            target,
            options: json!({"format":"majiro/arc","profile":"game"}),
        });
    }
    let env = env.context("Majiro essential majiro.env absent from fastdata/update")?;
    let env = crysirror_majiro::environment::parse(&env)?;
    let mut config = json!({"script":script.context("original Majiro start.mjo absent")?,"run_mode":1,"keyboard_lock_policy":"engine_logical","environment":environment.context("original Majiro ENV provider")?,"settings_profile":"portable_client"});
    // Native program lookup may include bin/, as on original installation media.
    if let Some(exe) = env.get("EXENAME") {
        let candidates = [dir.join(exe), dir.join("bin").join(exe)];
        if let Some(file) = candidates.iter().find(|p| p.is_file()) {
            let full = file.canonicalize()?;
            ensure!(
                full.starts_with(&root),
                "Majiro executable metadata escapes game root"
            );
            config["executable"] = json!(file
                .strip_prefix(dir)?
                .to_str()
                .context("Majiro executable name encoding")?);
        }
    }
    let width = env
        .get("CLIENT_WIDTH")
        .context("Majiro CLIENT_WIDTH")?
        .parse::<u32>()?;
    let height = env
        .get("CLIENT_HEIGHT")
        .context("Majiro CLIENT_HEIGHT")?
        .parse::<u32>()?;
    ensure!(
        (1..=8192).contains(&width) && (1..=8192).contains(&height),
        "Majiro canvas dimensions"
    );
    Ok(Plan {
        engine: "Majiro",
        project: project(name, [width, height], "majiro", config),
        profiles: std::collections::BTreeMap::from([(
            "game".into(),
            json!({"limits":{"max_entry_size":1024u64*1024*1024}}),
        )]),
        mounts,
        evidence: vec![],
    })
}
