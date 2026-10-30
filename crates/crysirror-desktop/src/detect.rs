// SPDX-License-Identifier: MPL-2.0
//! Engine detection for launching an installed game in place
//! (`crysirror play [dir]`, or the binary double-clicked inside the game
//! folder). It recognizes the engine from the folder's own files, opens the
//! protected archives through the shared resource formats to choose their
//! profile, and produces a typed engine resource plan. Only data is read;
//! no native executable or external launcher script runs.
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What `play` runs.
#[derive(Debug, serde::Serialize)]
pub struct Plan {
    pub engine: &'static str,
    pub project: Value,
    pub profiles: std::collections::BTreeMap<String, Value>,
    pub mounts: Vec<ArchiveMount>,
    pub evidence: Vec<FileEvidence>,
}

#[derive(Debug, serde::Serialize)]
pub struct FileEvidence {
    pub path: String,
    pub identity: Option<crysirror_resource::sniff::FileIdentity>,
}

#[derive(Debug, serde::Serialize)]
pub struct ArchiveMount {
    pub source: String,
    pub target: String,
    pub options: Value,
}

/// File names in `dir`, as found.
fn listing(dir: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .with_context(|| format!("read {}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    Ok(names)
}

fn find<'a>(names: &'a [String], wanted: &str) -> Option<&'a String> {
    names.iter().find(|n| n.eq_ignore_ascii_case(wanted))
}

fn with_ext<'a>(names: &'a [String], ext: &str) -> impl Iterator<Item = &'a String> {
    let ext = ext.to_owned();
    names.iter().filter(move |n| {
        Path::new(n)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(&ext))
    })
}

/// The game's main executable: signed/configured companions first, then the
/// largest, skipping obvious tools.
fn main_exe(dir: &Path, names: &[String]) -> Option<String> {
    let tool = |n: &str| {
        let l = n.to_ascii_lowercase();
        [
            "support", "setup", "config", "uninst", "launcher", "update", "crash",
        ]
        .iter()
        .any(|t| l.contains(t))
    };
    let exes: Vec<&String> = with_ext(names, "exe").filter(|n| !tool(n)).collect();
    if let Some(e) = exes.iter().find(|e| {
        find(names, &format!("{e}.sig")).is_some() || find(names, &format!("{e}.emb")).is_some()
    }) {
        return Some((*e).clone());
    }
    exes.into_iter()
        .max_by_key(|e| std::fs::metadata(dir.join(e)).map(|m| m.len()).unwrap_or(0))
        .cloned()
}

/// The bounded magic classifier supplies content evidence. Native engine
/// parsers still decide layout/profile support, including encrypted scripts.
pub fn detect(dir: &Path) -> Result<Plan> {
    let mut plan = detect_layout(dir)?;
    let prefix = plan.project["clients"][0]["asset_prefix"].as_str().unwrap_or("");
    if !prefix.is_empty() {
        crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, prefix)?;
    }
    let client_dir = dir.join(prefix).canonicalize()?;
    ensure!(client_dir.starts_with(dir.canonicalize()?), "detection asset prefix escapes game folder");
    let names = listing(&client_dir)?;
    let scoped = |path: &str| if prefix.is_empty() { path.to_owned() } else { format!("{prefix}/{path}") };
    let detector = crysirror_resource::sniff::FileDetector::new()?;
    let mut inputs = std::collections::BTreeSet::new();
    for mount in &plan.mounts {
        inputs.insert(scoped(&mount.source));
    }
    let config = &plan.project["clients"][0]["config"];
    for key in ["script", "scene", "gameexe", "executable"] {
        if let Some(path) = config[key].as_str() {
            if client_dir.join(path).is_file() {
                inputs.insert(scoped(path));
            }
        }
    }
    if let Some(exe) = main_exe(&client_dir, &names) {
        inputs.insert(scoped(&exe));
    }
    if plan.engine == "RFVP" {
        inputs.extend(with_ext(&names, "hcb").map(|p|scoped(p)));
    }
    if plan.engine == "RealLive" || plan.engine == "AVG32" {
        for key in ["Gameexe.ini", "Seen.txt"] {
            if let Some(path) = find(&names, key) {
                inputs.insert(scoped(path));
            }
        }
    }
    for path in inputs {
        let full = dir.join(&path).canonicalize()?;
        ensure!(
            full.starts_with(&client_dir),
            "detection input escapes client capability root: {path}"
        );
        use std::io::Read;
        let mut file = std::fs::File::open(full)?;
        let length = file.metadata()?.len();
        let expected = length.min(crysirror_resource::sniff::MAX_FILE_PREFIX_BYTES as u64);
        let mut prefix = Vec::new();
        file.by_ref().take(expected).read_to_end(&mut prefix)?;
        ensure!(
            prefix.len() as u64 == expected && file.metadata()?.len() == length,
            "detection input changed or ended during prefix read: {path}"
        );
        // Script bytes are classified before decode only as evidence. Validate
        // the selected NS source after its native XOR transform as well.
        let mut identity = detector.identify_bytes(&prefix)?;
        if let Some(identity) = &mut identity {
            identity.prefix_truncated |= length > prefix.len() as u64;
        }
        plan.evidence.push(FileEvidence { path, identity });
    }
    Ok(plan)
}

fn detect_layout(dir: &Path) -> Result<Plan> {
    let names = listing(dir)?;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "game".into());
    // CatSystem2 installers can carry the actual capability root in data/.
    // Select it from its own startup/INT content, without executing the wrapper.
    for prefix in std::iter::once("").chain(find(&names,"data").map(String::as_str)) {
        let root = dir.join(prefix).canonicalize()?;
        ensure!(root.starts_with(dir.canonicalize()?), "nested game root escapes selected directory");
        if !root.is_dir() { continue; }
        let local = listing(&root)?;
        let Some(config) = find(&local,"config").filter(|n|root.join(n).is_dir()) else { continue; };
        let settings = root.join(config).canonicalize()?;
        ensure!(settings.starts_with(&root), "game config escapes capability root");
        let files = listing(&settings)?;
        let Some(startup) = find(&files,"startup.xml") else { continue; };
        let Some(archive) = with_ext(&local,"int").next() else { continue; };
        let path = root.join(archive).canonicalize()?;
        ensure!(path.starts_with(&root), "native archive escapes capability root");
        use std::io::Read;
        let mut signature = [0;4];
        if std::fs::File::open(path)?.read_exact(&mut signature).is_err() || signature != *b"KIF\0" { continue; }
        return catsystem2(dir,&local,&name,prefix,&format!("{config}/{startup}"));
    }
    if find(&names, "data.xp3").is_some() {
        return krkr(dir, &names, &name);
    }
    if find(&names, "gameexe.ini").is_some() && find(&names, "seen.txt").is_some() {
        return reallive(dir, &names, &name);
    }
    if find(&names, "gameexe.ini").is_some()
        && (find(&names, "pdt").is_some()
            || with_ext(&names, "exe").any(|e| e.to_ascii_lowercase().starts_with("avg32")))
    {
        return Ok(Plan {
            engine: "AVG32",
            project: project(
                &name,
                [640, 480],
                "avg32",
                json!({"root": "", "font": "default"}),
            ),
            evidence: Vec::new(),
            profiles: Default::default(),
            mounts: Vec::new(),
        });
    }
    if ["nscript.dat", "pscript.dat", "0.txt", "00.txt"]
        .iter()
        .any(|s| find(&names, s).is_some())
    {
        return nscripter(dir, &names, &name);
    }
    if find(&names, "scene.pck").is_some() {
        return siglus(dir, &names, &name);
    }
    if with_ext(&names, "hcb").next().is_some() {
        return rfvp(dir, &names, &name);
    }
    if let Some(adv) = find(&names, "Advdata") {
        let root = dir.join(adv);
        let entries = listing(&root)?;
        if let Some(mes) = find(&entries, "MES") {
            let scripts = listing(&root.join(mes))?;
            if let Some(start) = find(&scripts, "start.mes") {
                // Select only the bounded native execution ABI implemented
                // by the first-class adapter; images alone never establish it.
                let path = root.join(mes).join(start).canonicalize()?;
                ensure!(
                    path.starts_with(dir.canonicalize()?),
                    "Circus script escapes game folder"
                );
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() >= 8 && bytes.len() <= 16 * 1024 * 1024 {
                    let count = u32::from_le_bytes(bytes[..4].try_into()?) as usize;
                    let marker = u32::from_le_bytes(bytes[4..8].try_into()?);
                    if count <= 65535 && marker == 3 {
                        let offset = 4 + count * 6;
                        if let Some(version) = bytes.get(offset..offset + 3) {
                            let abi = match version {
                                [0xb6, 0xaa, 0] => Some("dc4 / 0xaab6"),
                                [0xb6, 0xab, 0] => Some("dc4ph / 0xabb6"),
                                _ => None,
                            };
                            if let Some(abi) = abi {
                                if version == [0xb6, 0xab, 0] {
                                    #[cfg(feature = "circus")]
                                    {
                                        crysirror_circus::Program::parse(&bytes)?;
                                        return Ok(Plan {
                                            engine:"circus",
                                            project:project(&name,[1280,720],"circus",json!({"script":"start","font":"default"})),
                                            evidence:vec![FileEvidence {path:format!("{adv}/{mes}/{start}"),identity:None}],
                                            profiles:Default::default(),mounts:Vec::new(),
                                        });
                                    }
                                    #[cfg(not(feature = "circus"))]
                                    bail!("recognized Circus MES ({abi}) in {}; native adapter 'circus' is not linked; enable feature circus",dir.display());
                                }
                                bail!("recognized Circus MES ({abi}) in {}; native execution for this ABI is not implemented",dir.display());
                            }
                        }
                    }
                }
            }
        }
    }
    bail!("no supported game engine recognized in {}", dir.display())
}

fn project(name: &str, canvas: [u32; 2], adapter: &str, config: Value) -> Value {
    json!({
        "schema": 2,
        "name": name,
        "content_revision": "1",
        // Informational: `play` passes the game folder as the asset root.
        "assets": "game",
        "canvas": canvas,
        "clients": [{"name": "game", "game": "local-installed", "adapter": adapter, "config": config}],
    })
}

fn catsystem2(dir:&Path,names:&[String],name:&str,prefix:&str,startup_path:&str)->Result<Plan> {
    #[cfg(not(feature="catsystem2"))]
    {
        let _=(names,name,prefix,startup_path);
        bail!("recognized CatSystem2 startup/INT in {}; native adapter 'catsystem2' is not linked; enable feature catsystem2",dir.display());
    }
    #[cfg(feature="catsystem2")]
    {
        use std::io::Read;
        let root=dir.join(prefix).canonicalize()?;
        let startup_file=root.join(startup_path).canonicalize()?;
        ensure!(startup_file.starts_with(&root),"CatSystem2 startup escapes capability root");
        let mut bytes=Vec::new();
        std::fs::File::open(startup_file)?.take(1024*1024+1).read_to_end(&mut bytes)?;
        let startup=crysirror_catsystem2::parse_startup(&bytes)?;
        let executable=find(names,"cs2.exe").cloned().or_else(||main_exe(&root,names)).context("CatSystem2 archive profile executable missing")?;
        let archive=startup.script.split('/').next().context("CatSystem2 script path missing")?;
        ensure!(find(names,archive).is_some_and(|p|Path::new(p).extension().and_then(|e|e.to_str()).is_some_and(|e|e.eq_ignore_ascii_case("int"))),"CatSystem2 startup archive is absent");
        let mut p=project(name,startup.canvas,"catsystem2",json!({"script":startup.script,"font":"default","heap_bytes":startup.heap_bytes,"executable":executable.clone(),"canvas":startup.canvas}));
        if !prefix.is_empty(){p["clients"][0]["asset_prefix"]=json!(prefix);}
        let mounts=with_ext(names,"int").map(|source|ArchiveMount{source:source.clone(),target:source.clone(),options:json!({"format":"cat-system/int","profile":"game"})}).collect();
        Ok(Plan{engine:"CatSystem2",project:p,profiles:std::collections::BTreeMap::from([("game".into(),json!({"limits":{"max_entry_size":1024u64*1024*1024},"parameters":{"cat-system/int":{"executable":executable}}}))]),mounts,evidence:vec![FileEvidence{path:if prefix.is_empty(){startup_path.to_owned()}else{format!("{prefix}/{startup_path}")},identity:None}]})
    }
}

/// RealLive: `#SCREENSIZE_MOD` selects the canvas (0 = 640x480, 1 = 800x600,
/// 999 = explicit size).
fn reallive(dir: &Path, names: &[String], name: &str) -> Result<Plan> {
    let ini = std::fs::read(dir.join(find(names, "gameexe.ini").unwrap()))?;
    let text = String::from_utf8_lossy(&ini);
    let mut canvas = [640, 480];
    if let Some(line) = text
        .lines()
        .find(|l| l.trim_start().starts_with("#SCREENSIZE_MOD"))
    {
        let nums: Vec<u32> = line
            .split(['=', '.', ',', ' '])
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        canvas = match nums.as_slice() {
            [0, ..] => [640, 480],
            [1, ..] => [800, 600],
            [999, w, h, ..] => [*w, *h],
            _ => bail!("unrecognized RealLive #SCREENSIZE_MOD: {line}"),
        };
    }
    Ok(Plan {
        engine: "RealLive",
        project: project(name, canvas, "reallive", json!({"root": ""})),
        evidence: Vec::new(),
        profiles: Default::default(),
        mounts: Vec::new(),
    })
}

#[cfg(all(feature = "krkr", feature = "arc-formats"))]
fn krkr(dir: &Path, names: &[String], name: &str) -> Result<Plan> {
    use krkr_probe::*;
    let exe = main_exe(dir, names).context("KiriKiri game without an executable")?;
    let tpm: Vec<String> = listing(&dir.join("plugin"))
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".tpm"))
        .map(|n| format!("plugin/{n}"))
        .collect();
    // Profiles in order of evidence; the first that opens data.xp3 and
    // finds its startup wins.
    let mut tried = Vec::new();
    let mut candidates: Vec<Protection> = Vec::new();
    if !tpm.is_empty() {
        candidates.push(Protection::AdlerXor {
            modules: tpm.clone(),
        });
        candidates.push(Protection::Cxdec {
            modules: tpm.clone(),
            setup: false,
        });
        // Some classic filters keep the boundary config in the executable
        // and an encoded generator/table in a companion. Recovery must see
        // both capabilities; trying each module in isolation cannot combine
        // their structurally validated facts. Keep the shared module bound.
        if tpm.len() < 16 {
            let mut modules = vec![exe.clone()];
            modules.extend(tpm.iter().cloned());
            candidates.push(Protection::Cxdec { modules, setup: false });
        }
    }
    candidates.push(Protection::Hxv4);
    candidates.push(Protection::Cxdec {
        modules: vec![exe.clone()],
        setup: true,
    });
    candidates.push(Protection::Plain);
    for protection in candidates {
        match open(dir, &exe, &protection) {
            Ok(archive) => match canvas(dir, &exe, &archive, &protection) {
                Ok(canvas) => return Ok(plan(dir, names, name, &exe, protection, canvas)),
                Err(e) => tried.push(format!("{} canvas: {e:#}", protection.label())),
            },
            Err(e) => tried.push(format!("{}: {e:#}", protection.label())),
        }
    }
    bail!(
        "KiriKiri archive protection not recognized:\n  {}",
        tried.join("\n  ")
    )
}

#[cfg(not(all(feature = "krkr", feature = "arc-formats")))]
fn krkr(_: &Path, _: &[String], _: &str) -> Result<Plan> {
    bail!("KiriKiri game found, but this build lacks the `krkr` and `arc-formats` features")
}

#[cfg(all(feature = "krkr", feature = "arc-formats"))]
mod krkr_probe {
    use super::*;
    use crysirror_core::{service::AssetReader, state::ClientId};
    use crysirror_resource::{read_all, Archive, ArchiveFormat, DecodeContext, DependencyResolver};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub enum Protection {
        Hxv4,
        Cxdec { modules: Vec<String>, setup: bool },
        AdlerXor { modules: Vec<String> },
        Plain,
    }
    impl Protection {
        pub fn label(&self) -> &'static str {
            match self {
                Self::Hxv4 => "HXV4",
                Self::AdlerXor { .. } => "rotating Adler XOR",
                Self::Cxdec { modules, setup: false } if modules.len() > 1 => "CXDEC (combined modules)",
                Self::Cxdec { setup: false, .. } => "CXDEC (classic module)",
                Self::Cxdec { .. } => "CXDEC",
                Self::Plain => "plain XP3",
            }
        }
        /// `kirikiri/xp3` profile parameters, as the mount script declares.
        pub fn parameters(&self, exe: &str) -> Value {
            match self {
                Self::AdlerXor { modules } => json!({"adler_xor_modules": modules}),
                Self::Hxv4 => {
                    json!({"hxv4_executable": exe, "hxv4_startup_target": "__bootstrap/startup.tjs",
                    "hxv4_bootstrap_profile": "__bootstrap/protection.json", "hxv4_hashed_root": "__hxv4"})
                }
                Self::Cxdec {
                    modules,
                    setup: true,
                } => json!({"cxdec_modules": modules,
                    "cxdec_bootstrap_profile": "__cxdec/protection.json"}),
                Self::Cxdec { modules, .. } => json!({"cxdec_modules": modules}),
                Self::Plain => json!({}),
            }
        }
    }

    struct File(Mutex<std::fs::File>);
    impl AssetReader for File {
        fn len(&self) -> crysirror_core::Result<u64> {
            let f = self
                .0
                .lock()
                .map_err(|_| crysirror_core::Error::Io("poisoned".into()))?;
            f.metadata()
                .map(|m| m.len())
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
        fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = self
                .0
                .lock()
                .map_err(|_| crysirror_core::Error::Io("poisoned".into()))?;
            f.seek(SeekFrom::Start(offset))
                .and_then(|_| f.read(out))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    /// Companion modules, confined to the game folder.
    struct Companions(PathBuf);
    impl DependencyResolver for Companions {
        fn open(&self, path: &str) -> anyhow::Result<Arc<dyn AssetReader>> {
            anyhow::ensure!(
                !path.split(['/', '\\']).any(|s| s == ".."),
                "companion escapes the game folder"
            );
            Ok(Arc::new(File(Mutex::new(std::fs::File::open(
                self.0.join(path),
            )?))))
        }
    }

    pub struct Opened {
        archive: Box<dyn Archive>,
        context: DecodeContext,
    }

    pub fn open(dir: &Path, exe: &str, protection: &Protection) -> Result<Opened> {
        let opened = open_archive(dir, "data.xp3", exe, protection)?;
        // The profile must reach the game's startup script.
        let startup = match protection {
            Protection::Hxv4 => "__bootstrap/protection.json",
            _ => "startup.tjs",
        };
        anyhow::ensure!(
            opened.find(startup).is_some(),
            "{startup} not found in data.xp3"
        );
        Ok(opened)
    }

    pub fn open_archive(
        dir: &Path,
        name: &str,
        exe: &str,
        protection: &Protection,
    ) -> Result<Opened> {
        let mut context = DecodeContext::new(ClientId(1));
        context.source_name = Some(name.into());
        context.resolver = Some(Arc::new(Companions(dir.to_path_buf())));
        context.limits.max_entry_size = 256 * 1024 * 1024;
        context.limits.max_index_size = 64 * 1024 * 1024;
        context.limits.max_entries = 1_000_000;
        context
            .parameters
            .insert("kirikiri/xp3".into(), protection.parameters(exe));
        let data = names_ci(dir, name)?;
        let source = Arc::new(File(Mutex::new(std::fs::File::open(&data)?)));
        let archive = crysirror_formats_arc::Xp3.open(source, &context)?;
        Ok(Opened { archive, context })
    }

    fn names_ci(dir: &Path, name: &str) -> Result<PathBuf> {
        let names = listing(dir)?;
        Ok(dir.join(find(&names, name).with_context(|| format!("{name} missing"))?))
    }

    impl Opened {
        fn find(&self, path: &str) -> Option<u32> {
            self.archive.resolve(path).or_else(|| {
                self.archive
                    .entries()
                    .iter()
                    .find(|e| e.path.eq_ignore_ascii_case(path))
                    .map(|e| e.id)
            })
        }
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            let id = self.find(path)?;
            read_all(
                self.archive.open(id, &self.context).ok()?.as_ref(),
                4 * 1024 * 1024,
            )
            .ok()
        }
    }

    /// `scWidth`/`scHeight` from the game's Config.tjs (source text, krkr
    /// encrypted text, or compiled TJS2 decompiled by the shared VM).
    /// `scWidth`/`scHeight` from Config.tjs in data.xp3 or, failing that, the
    /// other archives (some titles keep `main/` in its own `main.xp3`).
    pub fn canvas(
        dir: &Path,
        exe: &str,
        archive: &Opened,
        protection: &Protection,
    ) -> Result<[u32; 2]> {
        if let Ok(c) = canvas_in(archive, protection) {
            return Ok(c);
        }
        for name in listing(dir)?
            .iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".xp3"))
        {
            if name.eq_ignore_ascii_case("data.xp3") {
                continue;
            }
            if let Some(c) = open_archive(dir, name, exe, protection)
                .ok()
                .and_then(|other| canvas_in(&other, protection).ok())
            {
                return Ok(c);
            }
        }
        bail!("Config.tjs with scWidth/scHeight not found")
    }

    fn canvas_in(archive: &Opened, protection: &Protection) -> Result<[u32; 2]> {
        let prefix = if matches!(protection, Protection::Hxv4) {
            "__hxv4/"
        } else {
            ""
        };
        for path in [
            "system/Config.tjs",
            "Config.tjs",
            "main/Config.tjs",
            "data/system/Config.tjs",
        ] {
            let Some(bytes) = archive.read(&format!("{prefix}{path}")) else {
                continue;
            };
            let text = if bytes.starts_with(b"TJS2") {
                let file = crysirror_tjs2::bytecode::BytecodeFile::parse(&bytes)?;
                let out = crysirror_tjs2::decompile::decompile(&file, &Default::default())?;
                out.sources
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                crysirror_resource::text_stream::decode_auto(&bytes, 4 * 1024 * 1024)?
            };
            if std::env::var_os("CRYSIRROR_DETECT_TRACE").is_some() {
                for l in text
                    .lines()
                    .filter(|l| l.contains("scWidth") || l.contains("scHeight"))
                {
                    eprintln!("[detect] {path}: {}", l.trim());
                }
            }
            if let (Some(w), Some(h)) =
                (assignment(&text, "scWidth"), assignment(&text, "scHeight"))
            {
                return Ok([w, h]);
            }
        }
        bail!("Config.tjs with scWidth/scHeight not found")
    }

    /// The largest `name = <integer>` assignment outside line comments. A
    /// Config.tjs may choose between sizes at run time (`if (FullHDMode)`);
    /// the largest declared size is the canvas the window scales from.
    fn assignment(text: &str, name: &str) -> Option<u32> {
        text.lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .filter_map(|l| {
                let at = l.find(name)?;
                let rest = l[at + name.len()..]
                    .trim_start()
                    .strip_prefix('=')?
                    .trim_start();
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            })
            .filter(|v| (64..=8192).contains(v))
            .max()
    }

    pub fn plan(
        dir: &Path,
        names: &[String],
        name: &str,
        exe: &str,
        protection: Protection,
        canvas: [u32; 2],
    ) -> Plan {
        let _ = dir;
        // Ordinary player launches select the game's own production path.
        // Without this explicit native option, some packed titles compile
        // DEBUG=1 and lazily open Win32-only developer consoles during play.
        let mut config = json!({"canvas": canvas, "executable": exe,
            "arguments":{"-debugwin":"no"}});
        match &protection {
            Protection::Hxv4 => {
                config["startup"] = "__bootstrap/startup.tjs".into();
                config["bootstrap_profile"] = "__bootstrap/protection.json".into();
                config.as_object_mut().unwrap().remove("executable");
            }
            Protection::Cxdec { setup: true, .. } => {
                config["startup"] = "startup.tjs".into();
                config["bootstrap_profile"] = "__cxdec/protection.json".into();
            }
            _ => config["startup"] = "startup.tjs".into(),
        }
        let profile = json!({"limits": {"max_entry_size": 268435456u64, "max_index_size": 67108864, "max_entries": 1000000},
            "parameters": {"kirikiri/xp3": protection.parameters(exe)}});
        let data = find(names, "data.xp3")
            .cloned()
            .unwrap_or_else(|| "data.xp3".into());
        let mut mounts = vec![(data.clone(), String::new())];
        if !matches!(protection, Protection::Hxv4) {
            for x in with_ext(names, "xp3").filter(|n| **n != data) {
                let stem = x[..x.len() - 4].to_ascii_lowercase();
                mounts.push((x.clone(), format!("__xp3/{stem}")));
            }
        }
        Plan {
            engine: "KiriKiri",
            project: project(name, canvas, "krkr", config),
            evidence: Vec::new(),
            profiles: [("game".into(), profile)].into(),
            mounts: mounts
                .into_iter()
                .map(|(source, target)| ArchiveMount {
                    source,
                    target,
                    options: json!({"format":"kirikiri/xp3","profile":"game"}),
                })
                .collect(),
        }
    }
}

/// Read bounded launch metadata, never an entire archive. A symlink companion
/// still has to resolve inside the selected game's capability root.
fn read_bounded(dir: &Path, path: &str, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let full = dir.join(path).canonicalize()?;
    let root = dir.canonicalize()?;
    ensure!(
        full.starts_with(root),
        "launch metadata escapes game folder: {path}"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(full)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "launch metadata exceeds {limit} bytes: {path}"
    );
    Ok(bytes)
}

fn nscripter(dir: &Path, names: &[String], name: &str) -> Result<Plan> {
    ensure!(
        cfg!(feature = "nscripter"),
        "NScripter found; this build needs the nscripter feature"
    );
    let script = ["0.txt", "00.txt", "nscript.dat", "pscript.dat"]
        .into_iter()
        .find_map(|s| find(names, s))
        .context("NScripter native script missing")?;
    let mut bytes = read_bounded(dir, script, 16 * 1024 * 1024)?;
    if script.eq_ignore_ascii_case("nscript.dat") || script.eq_ignore_ascii_case("pscript.dat") {
        crysirror_resource::transforms::xor_cycle_in_place(&mut bytes, &[0x84], 0);
    }
    let detector = crysirror_resource::sniff::FileDetector::new()?;
    if let Some(identity) = detector.identify_bytes(&bytes)? {
        let mime = &identity.mime_type;
        ensure!(
            !mime.starts_with("image/")
                && !mime.starts_with("audio/")
                && !mime.starts_with("video/")
                && !matches!(
                    mime.as_str(),
                    "application/zip"
                        | "application/x-7z-compressed"
                        | "application/x-rar"
                        | "application/x-dosexec"
                        | "application/x-executable"
                ),
            "NScripter source {script} contains {mime}, not a supported native script"
        );
    }
    let canvas = nscript_canvas(&bytes)?;
    let mut project = project(
        name,
        canvas,
        "nscripter",
        json!({
            "script":script,"font":"default","encoding":if script.eq_ignore_ascii_case("pscript.dat") {"utf8"} else {"sjis"}, "canvas":canvas,
        }),
    );
    if let Some(font) = find(names, "default.ttf").or_else(|| find(names, "default.otf")) {
        project["fonts"] = json!({"default":font});
    }
    let mut archives: Vec<(u32, String)> = names
        .iter()
        .filter_map(|n| {
            let lower = n.to_ascii_lowercase();
            let stem = lower.strip_prefix("arc")?.strip_suffix(".nsa")?;
            let rank = if stem.is_empty() {
                0
            } else {
                stem.parse::<u32>().ok()?
            };
            Some((rank, n.clone()))
        })
        .collect();
    archives.sort();
    let mut sources: Vec<String> = find(names, "arc.sar").cloned().into_iter().collect();
    sources.extend(archives.into_iter().map(|(_, n)| n));
    ensure!(sources.len() <= 64, "NScripter archive mount limit");
    ensure!(
        sources.is_empty() || cfg!(feature = "arc-formats"),
        "NScripter archives need the arc-formats feature"
    );
    let mounts: Vec<ArchiveMount> = sources
        .into_iter()
        .map(|source| {
            let format = if source.to_ascii_lowercase().ends_with(".sar") {
                "nscripter/sar"
            } else {
                "nscripter/nsa"
            };
            ArchiveMount {
                target: format!("__nscript/{source}"),
                source,
                options: json!({"format":format}),
            }
        })
        .collect();
    project["clients"][0]["config"]["archives"] =
        json!(mounts.iter().rev().map(|m| &m.target).collect::<Vec<_>>());
    Ok(Plan {
        engine: "NScripter",
        project,
        evidence: Vec::new(),
        profiles: Default::default(),
        mounts,
    })
}

/// Native ONS header convention: the first semicolon configuration line,
/// with optional '$', selects logical resolution (ScriptHandler.cpp 1175+).
fn nscript_canvas(bytes: &[u8]) -> Result<[u32; 2]> {
    let prefix = &bytes[..bytes.len().min(65536)];
    let Some(start) = prefix.iter().position(|b| *b == b';') else {
        return Ok([640, 480]);
    };
    let line = prefix[start + 1..]
        .split(|b| *b == b'\n' || *b == b'\r')
        .next()
        .unwrap_or_default();
    let text = String::from_utf8_lossy(line);
    let mut rest = text.trim_start().trim_start_matches('$');
    let mut canvas = [640, 480];
    fn number(rest: &mut &str) -> Option<u32> {
        *rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        let len = rest.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return None;
        }
        let value = rest[..len].parse().unwrap_or(u32::MAX);
        *rest = &rest[len..];
        Some(value)
    }
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if let Some(mode) = rest.strip_prefix("mode") {
            let (size, consumed) = if mode.starts_with("800") {
                ([800, 600], 3)
            } else if mode.starts_with("400") {
                ([400, 300], 3)
            } else if mode.starts_with("320") {
                ([320, 240], 3)
            } else if mode.starts_with("w720") {
                ([1280, 720], 4)
            } else {
                bail!("unsupported NScripter screen declaration: {rest}")
            };
            canvas = size;
            rest = &mode[consumed..];
        } else if rest.starts_with(['s', 'S']) {
            let mut size = &rest[1..];
            let Some(w) = number(&mut size) else { break };
            let h = number(&mut size).context("NScripter header height missing")?;
            canvas = [w, h];
            rest = size;
        } else if let Some(value) = rest.strip_prefix("value") {
            rest = value;
            if number(&mut rest).is_none() {
                break;
            }
        } else if rest.starts_with(['v', 'V', 'g', 'G', 'l', 'L']) {
            rest = &rest[1..];
            if number(&mut rest).is_none() {
                break;
            }
        } else {
            // Ordinary comments end configuration parsing, as in ONS. A
            // later word such as "mode800" is not another directive.
            break;
        }
    }
    ensure!(
        canvas.iter().all(|n| (1..=8192).contains(n)),
        "NScripter canvas limit"
    );
    Ok(canvas)
}

#[cfg(feature = "rfvp")]
fn rfvp(dir: &Path, names: &[String], name: &str) -> Result<Plan> {
    let scripts: Vec<_> = with_ext(names, "hcb").collect();
    ensure!(
        scripts.len() == 1,
        "RFVP HCB selection is ambiguous ({} files)",
        scripts.len()
    );
    let bytes = read_bounded(dir, scripts[0], 64 * 1024 * 1024)?;
    let parser = crysirror_rfvp_vm::script::parser::Parser::from_bytes(
        bytes,
        crysirror_rfvp_vm::script::parser::Nls::ShiftJIS,
    )?;
    ensure!(
        parser.get_game_mode() <= 15,
        "unknown RFVP screen mode {}",
        parser.get_game_mode()
    );
    let (w, h) = parser.get_screen_size();
    let fonts = listing(&dir.join("font")).unwrap_or_default();
    let font = find(names, "default.ttf")
        .cloned()
        .or_else(|| find(&fonts, "default.ttf").map(|f| format!("font/{f}")))
        .or_else(|| with_ext(&fonts, "ttf").next().map(|f| format!("font/{f}")))
        .context("RFVP requires a game-supplied font (default.ttf or font/*.ttf)")?;
    Ok(Plan {
        engine: "RFVP",
        project: project(name, [w, h], "rfvp", json!({"default_font":font,"screen":[w,h]})),
        evidence: Vec::new(),
        profiles: Default::default(),
        mounts: vec![],
    })
}
#[cfg(not(feature = "rfvp"))]
fn rfvp(_: &Path, _: &[String], _: &str) -> Result<Plan> {
    bail!("RFVP found; this build needs the rfvp feature")
}

fn siglus(dir: &Path, names: &[String], name: &str) -> Result<Plan> {
    use crysirror_siglus_vm::{
        gameexe::{decode_gameexe_dat_bytes, GameexeConfig, GameexeDecodeOptions},
        key_toml,
    };
    let scene = find(names, "Scene.pck").context("Siglus Scene.pck missing")?;
    let gameexe = find(names, "Gameexe.dat")
        .or_else(|| find(names, "Gameexe.ini"))
        .context("Siglus Gameexe.dat/ini missing")?;
    let key = find(names, "key.toml").or_else(|| find(names, ".crysirror-validation-key.toml"));
    let options = if let Some(path) = key {
        let bytes = read_bounded(dir, path, 64 * 1024)?;
        let parsed = key_toml::parse_key_toml(std::str::from_utf8(&bytes)?)?;
        GameexeDecodeOptions {
            exe_key16: parsed.exe_key16,
            base_angou_code: parsed.base_angou_code,
            game_angou_code: parsed.game_angou_code,
            chain_order: parsed
                .chain_order
                .unwrap_or_else(|| GameexeDecodeOptions::default().chain_order),
            ..Default::default()
        }
    } else {
        GameexeDecodeOptions::default()
    };
    let bytes = read_bounded(dir, gameexe, 16 * 1024 * 1024)?;
    let (text, _) = decode_gameexe_dat_bytes(&bytes, &options)
        .context("Siglus Gameexe decode needs caller-owned material in key.toml")?;
    let ini = GameexeConfig::from_text(&text);
    let size = ini
        .get_entry("SCREEN_SIZE")
        .context("Siglus SCREEN_SIZE missing")?;
    let canvas: [u32; 2] = [
        size.item_unquoted(0)
            .context("SCREEN_SIZE width")?
            .parse()?,
        size.item_unquoted(1)
            .context("SCREEN_SIZE height")?
            .parse()?,
    ];
    ensure!(
        canvas.iter().all(|n| (1..=8192).contains(n)),
        "Siglus screen size limit"
    );
    let mut config = json!({"scene":scene,"gameexe":gameexe});
    if let Some(key) = key {
        config["key_file"] = key.clone().into();
    }
    Ok(Plan {
        engine: "Siglus",
        project: project(name, canvas, "siglus", config),
        evidence: Vec::new(),
        profiles: Default::default(),
        mounts: vec![],
    })
}

/// A system font for engines that need one (the game's own embedded fonts
/// are loaded by the game itself).
pub fn system_font() -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "C:\\Windows\\Fonts\\msgothic.ttc",
            "C:\\Windows\\Fonts\\YuGothR.ttc",
            "C:\\Windows\\Fonts\\meiryo.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat_startup_fixture(root:&Path) {
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/startup.xml"),b"<document><SCREEN><width>640</width><height>360</height><v_width>1280</v_width><v_height>720</v_height></SCREEN><SCRIPT><start>kcs.int/main.kcs</start></SCRIPT><ETC><heap>4096</heap></ETC></document>").unwrap();
        std::fs::write(root.join("kcs.int"),b"KIF\0\0\0\0\0").unwrap();
        std::fs::write(root.join("cs2.exe"),b"MZ").unwrap();
    }

    #[test]
    fn catsystem2_detection_selects_original_nested_capability_and_startup() {
        let dir=tempfile::tempdir().unwrap();
        cat_startup_fixture(&dir.path().join("data"));
        std::fs::write(dir.path().join("installer.exe"),vec![0;1024]).unwrap();
        #[cfg(feature="catsystem2")]
        {
            let p=detect(dir.path()).unwrap();
            assert_eq!(p.engine,"CatSystem2");
            assert_eq!(p.project["canvas"],json!([1280,720]));
            assert_eq!(p.project["clients"][0]["asset_prefix"],"data");
            let config=&p.project["clients"][0]["config"];
            assert_eq!(config["script"],"kcs.int/main.kcs");
            assert_eq!(config["heap_bytes"],4096);
            assert_eq!(config["executable"],"cs2.exe");
            assert_eq!(p.mounts[0].source,"kcs.int");
            assert_eq!(p.mounts[0].target,"kcs.int");
            assert!(p.evidence.iter().all(|e|e.path.starts_with("data/")));
            let direct=detect(&dir.path().join("data")).unwrap();
            assert!(direct.project["clients"][0].get("asset_prefix").is_none());
        }
        #[cfg(not(feature="catsystem2"))]
        assert!(detect(dir.path()).unwrap_err().to_string().contains("native adapter 'catsystem2' is not linked"));
    }

    #[test]
    #[cfg(all(unix,feature="catsystem2"))]
    fn catsystem2_evidence_cannot_escape_its_selected_client_root() {
        let dir=tempfile::tempdir().unwrap();
        cat_startup_fixture(&dir.path().join("data"));
        std::fs::remove_file(dir.path().join("data/cs2.exe")).unwrap();
        std::fs::write(dir.path().join("outside-client.exe"),b"MZ").unwrap();
        std::os::unix::fs::symlink("../outside-client.exe",dir.path().join("data/cs2.exe")).unwrap();
        assert!(detect(dir.path()).unwrap_err().to_string().contains("escapes client capability root"));
    }

    #[test]
    fn circus_recognition_selects_only_the_available_native_profile() {
        let dir = tempfile::tempdir().unwrap();
        let scripts = dir.path().join("Advdata/MES");
        std::fs::create_dir_all(&scripts).unwrap();
        let mut bytes = Vec::new();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend(3u32.to_le_bytes());
        bytes.extend([0; 8]);
        bytes.extend([0xb6, 0xab, 0, 0, 0, 0]);
        std::fs::write(scripts.join("start.mes"), &bytes).unwrap();
        #[cfg(feature = "circus")]
        {
            let plan = detect(dir.path()).unwrap();
            assert_eq!(plan.engine, "circus");
            assert_eq!(plan.project["canvas"], json!([1280, 720]));
            assert_eq!(
                plan.project["clients"][0]["config"],
                json!({"script":"start","font":"default"})
            );
        }
        #[cfg(not(feature = "circus"))]
        {
            let error = detect(dir.path()).unwrap_err().to_string();
            assert!(error.contains("recognized Circus MES (dc4ph / 0xabb6)"));
            assert!(error.contains("native adapter 'circus' is not linked"));
        }
        bytes[17] = 0xaa;
        std::fs::write(scripts.join("start.mes"), &bytes).unwrap();
        assert!(detect(dir.path())
            .unwrap_err()
            .to_string()
            .contains("this ABI is not implemented"));
        bytes[16] = 0;
        std::fs::write(scripts.join("start.mes"), &bytes).unwrap();
        assert!(detect(dir.path())
            .unwrap_err()
            .to_string()
            .contains("no supported game engine"));
    }

    #[test]
    fn native_header_and_unicode_paths_are_bounded() {
        assert_eq!(
            nscript_canvas(b";$mode800,value200\n*define\n").unwrap(),
            [800, 600]
        );
        assert_eq!(
            nscript_canvas(b";$S1920,1080\n*define\n").unwrap(),
            [1920, 1080]
        );
        assert_eq!(
            nscript_canvas(b"; generated script\n*define\n").unwrap(),
            [640, 480]
        );
        assert!(nscript_canvas(b";$s99999,480\n").is_err());
        assert!(nscript_canvas(b";$mode_unknown\n").is_err());
        assert!(nscript_canvas(b";$s9999999999999999999999,480\n").is_err());
        assert_eq!(
            nscript_canvas(b"; description mentions mode800\n").unwrap(),
            [640, 480]
        );
        let names = vec!["日本語".into(), "ゲーム.EXE".into(), "テスト.xp3".into()];
        assert_eq!(with_ext(&names, "exe").count(), 1);
        assert_eq!(with_ext(&names, "xp3").count(), 1);
    }

    #[cfg(all(feature = "nscripter", feature = "arc-formats"))]
    #[test]
    fn nscripter_plan_mounts_original_archives_and_keeps_native_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let mut script = b";$mode800\n*define\ngame\n*start\nend\n".to_vec();
        crysirror_resource::transforms::xor_cycle_in_place(&mut script, &[0x84], 0);
        std::fs::write(dir.path().join("nscript.dat"), script).unwrap();
        for n in ["arc.sar", "arc.nsa", "arc1.nsa", "arc10.nsa", "default.ttf"] {
            std::fs::write(dir.path().join(n), []).unwrap();
        }
        let plan = detect(dir.path()).unwrap();
        assert_eq!(plan.engine, "NScripter");
        assert_eq!(plan.project["canvas"], json!([800, 600]));
        assert_eq!(plan.project["fonts"]["default"], "default.ttf");
        assert_eq!(
            plan.project["clients"][0]["config"]["archives"],
            json!([
                "__nscript/arc10.nsa",
                "__nscript/arc1.nsa",
                "__nscript/arc.nsa",
                "__nscript/arc.sar"
            ])
        );
        assert_eq!(
            plan.mounts
                .iter()
                .map(|m| m.source.as_str())
                .collect::<Vec<_>>(),
            vec!["arc.sar", "arc.nsa", "arc1.nsa", "arc10.nsa"]
        );
        // Plaintext has native precedence over nscript.dat, independent of
        // the folder name. Detection is not a game-name whitelist.
        std::fs::write(dir.path().join("0.txt"), b";plain\n*define\ngame\n").unwrap();
        assert_eq!(
            detect(dir.path()).unwrap().project["clients"][0]["config"]["script"],
            "0.txt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_metadata_cannot_escape_selected_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("source"), b"private").unwrap();
        std::os::unix::fs::symlink(outside.path().join("source"), dir.path().join("0.txt"))
            .unwrap();
        assert!(read_bounded(dir.path(), "0.txt", 1024).is_err());
        std::fs::write(dir.path().join("large"), [0; 16]).unwrap();
        assert!(read_bounded(dir.path(), "large", 8).is_err());
    }

    #[cfg(feature = "nscripter")]
    #[test]
    fn renamed_image_cannot_be_selected_as_native_script() {
        let dir = tempfile::tempdir().unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.path().join("0.txt"), png.into_inner()).unwrap();
        let error = detect(dir.path()).unwrap_err().to_string();
        assert!(error.contains("image/png"), "{error}");
    }

    #[cfg(all(feature = "krkr", feature = "arc-formats"))]
    #[test]
    fn installed_player_selects_native_production_mode_before_game_scripts() {
        let names = vec!["data.xp3".to_owned()];
        let plan = krkr_probe::plan(
            Path::new("."),
            &names,
            "authored",
            "game.exe",
            krkr_probe::Protection::Plain,
            [640, 480],
        );
        assert_eq!(
            plan.project["clients"][0]["config"]["arguments"],
            json!({"-debugwin":"no"})
        );
        assert_eq!(
            plan.project["clients"][0]["config"]["startup"],
            "startup.tjs"
        );
    }
}
