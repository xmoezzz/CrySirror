// SPDX-License-Identifier: MPL-2.0
//! Engine detection for launching an installed game in place
//! (`crysirror play [dir]`, or the binary double-clicked inside the game
//! folder). It recognizes the engine from the folder's own files, opens the
//! protected archives through the shared resource formats to choose their
//! profile, and produces a typed engine resource plan. Only data is read;
//! no native executable or external launcher script runs.
mod majiro;
mod system_nnn;
mod system4;
mod bgi;
mod softpal;
mod qlie;
mod sugarpot;
mod nsystem;
mod n2system;
mod malie;
mod rscript;
mod willplus;
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use crate::source_path::SourcePath;

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
    #[serde(skip_serializing_if="std::ops::Not::not")]
    pub fallback: bool,
    pub source: String,
    pub target: String,
    pub options: Value,
}

// The bootstrap script selects its active revision and domains at runtime.
// A sibling discovered by extension can be an unused or unsupported pack.
const HXV4_COMPANION_PROFILE: &str = "hxv4-companion";

impl ArchiveMount {
    pub(crate) fn is_hxv4_companion(&self) -> bool {
        self.options["profile"].as_str() == Some(HXV4_COMPANION_PROFILE)
    }
}

/// File names in `dir`, as found.
fn listing(dir: &SourcePath) -> Result<Vec<String>> {
    let mut names: Vec<String> = dir.read_dir()
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
fn main_exe(dir: &SourcePath, names: &[String]) -> Option<String> {
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
        .max_by_key(|e| dir.join(e).metadata().map(|m| m.len()).unwrap_or(0))
        .cloned()
}

/// The bounded magic classifier supplies content evidence. Native engine
/// parsers still decide layout/profile support, including encrypted scripts.
pub fn detect(dir: &Path) -> Result<Plan> {
    detect_source(&SourcePath::local(dir))
}

pub fn detect_source(dir: &SourcePath) -> Result<Plan> {
    detect_source_with_entry(dir, None)
}

/// Preserve a file explicitly selected by the host. Directory discovery keeps
/// the existing ranking. The selected entry must belong to this directory.
pub fn detect_source_with_entry(dir: &SourcePath, entry: Option<&str>) -> Result<Plan> {
    let explicit_exe = entry.filter(|s| s.to_ascii_lowercase().ends_with(".exe"));
    if let Some(entry) = explicit_exe {
        ensure!(!entry.contains(['/', '\\', ':']) && !entry.contains('\0'), "invalid executable name");
    }
    let mut plan = detect_layout(dir, explicit_exe)?;
    // Original engines clear the game screen to black before their first
    // draw; the authored-project default is a demo colour.
    if plan.project.get("background").is_none() {
        plan.project["background"] = serde_json::json!([0, 0, 0, 255]);
    }
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
    if plan.engine == "KiriKiri" {
        if let Some(entry) = explicit_exe {
            inputs.insert(scoped(entry));
        }
    }
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
        let mut file = crate::source_path::SourceFile::open(full)?;
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

fn detect_layout(dir: &SourcePath, explicit_exe: Option<&str>) -> Result<Plan> {
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
        if crate::source_path::SourceFile::open(path)?.read_exact(&mut signature).is_err() || signature != *b"KIF\0" { continue; }
        return catsystem2(dir,&local,&name,prefix,&format!("{config}/{startup}"));
    }
    if let Some(plan) = system4::detect(dir, &names, &name)? { return Ok(plan); }
    if let Some(plan) = majiro::detect(dir, &names, &name)? { return Ok(plan); }
    if let Some(plan) = system_nnn::detect(dir, &names, &name)? { return Ok(plan); }
    if let Some(plan) = bgi::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = softpal::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = qlie::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = sugarpot::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = nsystem::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = malie::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = n2system::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = rscript::detect(dir,&names,&name)? {return Ok(plan);}
    if let Some(plan) = willplus::detect(dir,&names,&name)? {return Ok(plan);}
    if find(&names, "data.xp3").is_some() {
        return krkr(dir, &names, &name, explicit_exe);
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
                crate::source_path::SourceFile::open(path)?
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
                                [0x9f, 0xa0, 0] => Some("dc3wy / 0xa09f"),
                                [0x71, 0x8a, 0] => Some("dc3 / 0x8a71"),
                                _ => None,
                            };
                            if let Some(abi) = abi {
                                if matches!(version,[0xb6,0xab,0]|[0x9f,0xa0,0]|[0x71,0x8a,0]) {
                                    {
                                        let program=crysirror_circus::Program::parse(&bytes)?;
                                        let (canvas,config)=match program.profile {
                                            crysirror_circus::Profile::Dc4PlusHarmony=>([1280,720],json!({"script":"start","font":"default"})),
                                            crysirror_circus::Profile::Dc3WithYou=>([1024,576],json!({"script":"start","font":"default","abi":"dc3wy-a09f","installation_policy":"mounted-resources"})),
                                            //D.C.III runs on the DC3WY interpreter through
                                            //crysirror_circus::dc3_original's opcode translation.
                                            crysirror_circus::Profile::Dc3=>([1024,576],json!({"script":"start","font":"default","abi":"dc3-8a71","installation_policy":"mounted-resources"})),
                                            _=>bail!("Circus execution/header profile mismatch"),
                                        };
                                        return Ok(Plan {
                                            engine:"circus",
                                            project:project(&name,canvas,"circus",config),
                                            evidence:vec![FileEvidence {path:format!("{adv}/{mes}/{start}"),identity:None}],
                                            profiles:Default::default(),mounts:Vec::new(),
                                        });
                                    }
                                }
                                bail!("recognized Circus MES ({abi}) in {}; native execution for this ABI is not implemented",dir.display());
                            }
                            if version[2]==0 {
                                let abi=u16::from_le_bytes([version[0],version[1]]);
                                bail!("recognized Circus MES ABI {abi:#06x} in {}; this script version has no native interpreter yet (the player currently executes DC4 Plus Harmony / 0xabb6)",dir.display());
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(system)=find(&names,"System.MES") {
        use std::io::Read;
        let mut signature=[0;9];
        let mut file=crate::source_path::SourceFile::open(dir.join(system))?;
        if file.read_exact(&mut signature).is_ok() && &signature==b"#SYS_SET\0" {
            ensure!(find(&names,"DC.EXE").is_some(),"recognized early Circus System.MES in {} without the reviewed DC.EXE front end",dir.display());
            return Ok(Plan {
                engine:"circus",
                project:project(&name,[800,600],"circus",json!({"script":"start","font":"default","abi":"dc1-advtxt","installation_policy":"mounted-resources"})),
                evidence:vec![FileEvidence{path:system.clone(),identity:None}],
                profiles:Default::default(),mounts:Vec::new(),
            });
        }
    }
    if let Some(mes)=find(&names,"MES") {
        let scripts=listing(&dir.join(mes))?;
        if let Some(start)=find(&scripts,"start.mes") {
            use std::io::Read;
            let mut bytes=Vec::new();
            crate::source_path::SourceFile::open(dir.join(mes).join(start))?.take(16*1024*1024+1).read_to_end(&mut bytes)?;
            if bytes.len()>=4 && bytes.len()<=16*1024*1024 {
                let count=u32::from_le_bytes(bytes[..4].try_into()?) as usize;
                if count<=65535 && bytes.get(4+count*4..6+count*4)==Some(&[0x69,0x57]) {
                    let program=crysirror_circus::Program::parse(&bytes)?;
                    ensure!(program.profile==crysirror_circus::Profile::Dc2PlusCommunication,"Circus root MES execution/header mismatch");
                    return Ok(Plan {engine:"circus",project:project(&name,[800,600],"circus",json!({"script":"start","font":"default","abi":"dc2pc-5769","installation_policy":"mounted-resources"})),evidence:vec![FileEvidence{path:format!("{mes}/{start}"),identity:None}],profiles:Default::default(),mounts:Vec::new()});
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

fn catsystem2(dir:&SourcePath,names:&[String],name:&str,prefix:&str,startup_path:&str)->Result<Plan> {
    {
        use std::io::Read;
        let root=dir.join(prefix).canonicalize()?;
        let startup_file=root.join(startup_path).canonicalize()?;
        ensure!(startup_file.starts_with(&root),"CatSystem2 startup escapes capability root");
        let mut bytes=Vec::new();
        crate::source_path::SourceFile::open(startup_file)?.take(1024*1024+1).read_to_end(&mut bytes)?;
        let startup=crysirror_catsystem2::parse_startup(&bytes)?;
        let executable=find(names,"cs2.exe").cloned().or_else(||main_exe(&root,names)).context("CatSystem2 archive profile executable missing")?;
        let archive=startup.script.split('/').next().context("CatSystem2 script path missing")?;
        ensure!(find(names,archive).is_some_and(|p|Path::new(p).extension().and_then(|e|e.to_str()).is_some_and(|e|e.eq_ignore_ascii_case("int"))),"CatSystem2 startup archive is absent");
        let mut p=project(name,startup.canvas,"catsystem2",json!({"script":startup.script,"font":"default","heap_bytes":startup.heap_bytes,"executable":executable.clone(),"canvas":startup.canvas}));
        if !prefix.is_empty(){p["clients"][0]["asset_prefix"]=json!(prefix);}
        // Some titles ship an ordinary archive as `<name>.int.FWP`; it serves as `<name>.int`.
        let renamed=with_ext(names,"fwp").filter(|n|n.len()>4&&n[..n.len()-4].to_ascii_lowercase().ends_with(".int")).map(|source|(source.clone(),source[..source.len()-4].to_owned()));
        let mounts=with_ext(names,"int").map(|source|(source.clone(),source.clone())).chain(renamed).map(|(source,target)|ArchiveMount{fallback:false,source,target,options:json!({"format":"cat-system/int","profile":"game"})}).collect();
        Ok(Plan{engine:"CatSystem2",project:p,profiles:std::collections::BTreeMap::from([("game".into(),json!({"limits":{"max_entry_size":1024u64*1024*1024},"parameters":{"cat-system/int":{"executable":executable}}}))]),mounts,evidence:vec![FileEvidence{path:if prefix.is_empty(){startup_path.to_owned()}else{format!("{prefix}/{startup_path}")},identity:None}]})
    }
}

/// RealLive: `#SCREENSIZE_MOD` selects the canvas (0 = 640x480, 1 = 800x600,
/// 999 = explicit size).
fn reallive(dir: &SourcePath, names: &[String], name: &str) -> Result<Plan> {
    let ini = dir.join(find(names, "gameexe.ini").unwrap()).read()?;
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
        mounts: reallive_voice_mounts(dir, &ini)?,
    })
}

// Keep the original voice packs intact. The common resource registry indexes
// their numbered audio entries; the VM only selects the native pack/sample ID.
fn reallive_voice_mounts(dir: &SourcePath, ini: &[u8]) -> Result<Vec<ArchiveMount>> {
    use crysirror_reallive::vm::{Gameexe, Nls};
    let exe = Gameexe::from_bytes(ini, Nls::Sjis);
    let requested = exe.get("FOLDNAME.KOE").and_then(|e| e.str(0)).unwrap_or("koe").replace('\\', "/");
    let root = dir.canonicalize()?;
    let mut folder = dir.to_path_buf();
    if !requested.is_empty() && requested != "." {
        let path = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &requested)?;
        for component in path.relative().split('/') {
            let names = listing(&folder)?;
            let Some(actual) = find(&names, component) else { return voice_packs_in(dir, &root, &[dir.to_path_buf()]); };
            folder = folder.join(actual);
            ensure!(folder.canonicalize()?.starts_with(&root), "RealLive voice directory escapes game capability root");
        }
    }
    voice_packs_in(dir, &root, &[dir.to_path_buf(), folder])
}

fn voice_packs_in(dir: &SourcePath, root: &SourcePath, folders: &[SourcePath]) -> Result<Vec<ArchiveMount>> {
    let mut packs = std::collections::BTreeMap::new();
    for folder in folders {
        if !folder.is_dir() { continue; }
        let names = listing(folder)?;
        ensure!(names.len() <= 32768, "RealLive voice directory entry budget");
        for name in names {
            let format = match Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                Some("koe") => "real-live/koepac",
                Some("ovk") => "real-live/ovk",
                Some("nwk") => "real-live/nwk",
                _ => continue,
            };
            let file = folder.join(&name);
            ensure!(file.is_file() && file.canonicalize()?.starts_with(root), "RealLive voice pack escapes game capability root");
            let relative = file.strip_prefix(dir)?.to_str().context("RealLive voice path is not UTF-8")?.replace('\\', "/");
            packs.insert(relative, format);
            ensure!(packs.len() <= crysirror_runtime::vfs::MAX_PROVIDER_MOUNTS, "RealLive voice archive mount budget");
        }
    }
    Ok(packs.into_iter().map(|(source, format)| ArchiveMount { fallback:false, target: source.clone(), source, options: json!({"format":format}) }).collect())
}

struct KrkrEntry {
    executable: String,
    overlay: Option<crysirror_krkr::storage::LauncherOverlay>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct KrkrPatchManifest {
    game_executable_file: String,
    #[serde(default)]
    patch_archives: Vec<String>,
    #[serde(default)]
    patch_protocols: Vec<String>,
    #[serde(default)]
    patch_no_protocol: bool,
}

fn krkr_entry(dir: &SourcePath, names: &[String], entry: Option<&str>) -> Result<KrkrEntry> {
    let Some(entry) = entry else {
        let executable = main_exe(dir, names).context("KiriKiri game without an executable")?;
        return Ok(KrkrEntry { executable, overlay: None });
    };
    let selected = find(names, entry).context("selected KiriKiri executable missing")?;
    let root = dir.canonicalize()?;
    let scoped_file = |name: &str| -> Result<SourcePath> {
        let file = root.join(name).canonicalize()?;
        ensure!(file.starts_with(&root) && file.is_file(), "KiriKiri entry outside game directory");
        Ok(file)
    };
    let selected_file = scoped_file(selected)?;
    if let Some(manifest) = find(names, "KrkrPatch.json") {
        // Identify the launcher before consulting its manifest. A second
        // standalone game EXE in the directory remains selectable on its own.
        use std::io::Read;
        let mut prefix = Vec::new();
        crate::source_path::SourceFile::open(selected_file)?.take(1024 * 1024).read_to_end(&mut prefix)?;
        let manifest_name = b"KrkrPatch.json";
        let utf16: Vec<u8> = manifest_name.iter().flat_map(|&c| [c, 0]).collect();
        if prefix.starts_with(b"MZ") && [manifest_name.as_slice(), utf16.as_slice()].iter()
            .any(|needle| prefix.windows(needle.len()).any(|s| s.eq_ignore_ascii_case(needle))) {
            scoped_file(manifest)?;
            let bytes = read_bounded(dir, manifest, 64 * 1024)?;
            let config: KrkrPatchManifest = serde_json::from_slice(
                bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes),
            ).with_context(|| format!("parse {manifest}"))?;
            let runtime = &config.game_executable_file;
            ensure!(runtime.to_ascii_lowercase().ends_with(".exe") && !runtime.contains(['/', '\\', ':', '\0']), "invalid launcher runtime name");
            let runtime = find(names, runtime).context("launcher runtime executable missing")?;
            scoped_file(runtime)?;
            log::info!("KiriKiri selected launcher {selected}; runtime {runtime}");
            let overlay = if config.patch_archives.is_empty() {
                None
            } else {
                let protocols = config.patch_protocols.iter().map(|protocol| {
                    protocol.strip_suffix("://").map(str::to_ascii_lowercase)
                        .with_context(|| format!("invalid KiriKiri launcher protocol: {protocol}"))
                }).collect::<Result<Vec<_>>>()?;
                let mut overlay = crysirror_krkr::storage::LauncherOverlay {
                    archives: config.patch_archives,
                    protocols,
                    no_protocol: config.patch_no_protocol,
                };
                overlay.validate()?;
                for archive in &mut overlay.archives {
                    let file = find(names, archive).with_context(|| format!("KiriKiri launcher XP3 `{archive}` is missing"))?;
                    scoped_file(file)?;
                    archive.clone_from(file);
                }
                Some(overlay)
            };
            return Ok(KrkrEntry { executable: runtime.to_owned(), overlay });
        }
    }
    log::info!("KiriKiri selected runtime {selected}");
    Ok(KrkrEntry { executable: selected.to_owned(), overlay: None })
}

fn krkr(dir: &SourcePath, names: &[String], name: &str, explicit_exe: Option<&str>) -> Result<Plan> {
    use krkr_probe::*;
    let KrkrEntry { executable: exe, overlay } = krkr_entry(dir, names, explicit_exe)?;
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
    if let Some(profile) = find(names, "crysirror-nana.local.json") {
        let setup_file = exe.rsplit_once('.')
            .and_then(|(stem, _)| find(names, &format!("{stem}.key")))
            .filter(|name| dir.join(name).is_file()).cloned();
        candidates.push(Protection::Nana { profile: profile.clone(), setup_file });
    }
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
    candidates.push(Protection::HashFoldXor { key_bias: None, zero_key: None });
    candidates.push(Protection::Plain);
    for protection in candidates {
        match open(dir, &exe, &protection) {
            Ok(archive) => {
                let protection = protection.authenticated(&archive)?;
                match canvas(dir, &exe, &archive, &protection) {
                    Ok(canvas) => {
                        let archives: Vec<_> = with_ext(names, "xp3")
                            .filter(|n| !dir.join(n).is_dir()).cloned().collect();
                        let mut plan = plan(&archives, name, &exe, protection, canvas)?;
                        if let Some(overlay) = overlay {
                            configure_krkr_overlay(&mut plan, overlay)?;
                        }
                        return Ok(plan);
                    }
                    Err(e) => tried.push(format!("{} canvas: {e:#}", protection.label())),
                }
            },
            Err(e) => tried.push(format!("{}: {e:#}", protection.label())),
        }
    }
    bail!(
        "KiriKiri startup detection failed (archive protection or initial canvas):\n  {}",
        tried.join("\n  ")
    )
}

fn configure_krkr_overlay(
    plan: &mut Plan,
    overlay: crysirror_krkr::storage::LauncherOverlay,
) -> Result<()> {
    let profile = "launcher-overlay";
    for archive in &overlay.archives {
        let mount = plan.mounts.iter_mut().find(|mount| &mount.source == archive)
            .with_context(|| format!("missing KiriKiri launcher mount for `{archive}`"))?;
        // Explicit launcher archives are ordinary XP3. An HXV4 companion
        // profile would leak the base executable's protection into this mount.
        mount.options["profile"] = json!(profile);
    }
    let limits = plan.profiles["game"]["limits"].clone();
    plan.profiles.insert(profile.into(), json!({ "limits": limits }));
    log::info!("KiriKiri launcher override archives (first match wins): {}", overlay.archives.join(", "));
    plan.project["clients"][0]["config"]["launcher_overlay"] = serde_json::to_value(overlay)?;
    Ok(())
}


/// Developer search over a KiriKiri game's own script text: opens every XP3
/// with the protection profile that startup detection selects and returns
/// `archive:entry: line` for each script line (or compiled-TJS2 UTF-16
/// constant) containing `needle`. Reads only; nothing leaves the caller.
pub fn kirikiri_script_search(dir: &Path, needle: &str, limit: usize) -> Result<Vec<String>> {
    use crysirror_resource::read_all;
    use krkr_probe::*;
    let dir = SourcePath::local(dir);
    let names = listing(&dir)?;
    let exe = main_exe(&dir, &names).context("KiriKiri game without an executable")?;
    let tpm: Vec<String> = listing(&dir.join("plugin")).unwrap_or_default().into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".tpm")).map(|n| format!("plugin/{n}")).collect();
    let mut candidates = Vec::new();
    if !tpm.is_empty() {
        candidates.push(Protection::AdlerXor { modules: tpm.clone() });
        candidates.push(Protection::Cxdec { modules: tpm.clone(), setup: false });
    }
    candidates.extend([
        Protection::Hxv4,
        Protection::Cxdec { modules: vec![exe.clone()], setup: true },
        Protection::HashFoldXor { key_bias: None, zero_key: None },
        Protection::Plain,
    ]);
    let protection = candidates.into_iter()
        .find_map(|p| open(&dir, &exe, &p).ok().and_then(|a| p.authenticated(&a).ok()))
        .context("no protection profile opens data.xp3")?;
    let context: usize = std::env::var("KRKR_GREP_CONTEXT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = Vec::new();
    let mut archives: Vec<String> = names.iter().filter(|n| n.to_ascii_lowercase().ends_with(".xp3")).cloned().collect();
    archives.sort();
    for name in archives {
        let Ok(opened) = open_archive(&dir, &name, &exe, &protection) else { continue };
        for entry in opened.archive.entries() {
            let lower = entry.path.to_ascii_lowercase();
            // `ext:.psd` lists entry names instead of searching text.
            if let Some(ext) = needle.strip_prefix("ext:") {
                if lower.ends_with(&ext.to_ascii_lowercase()) {
                    out.push(format!("{name}:{} ({} bytes)", entry.path, entry.size));
                    if out.len() >= limit { return Ok(out); }
                }
                continue;
            }
            // `file:path` prints one whole entry instead of searching.
            let whole = needle.strip_prefix("file:");
            if whole.is_some_and(|f| !lower.ends_with(&f.to_ascii_lowercase())) {
                continue;
            }
            let named = [".tjs", ".ks", ".txt", ".csv", ".ini", ".json", ".func", ".scn"].iter().any(|e| lower.ends_with(e));
            // Hash-named archives (HXV4) carry no extension: sniff content.
            let hashed = !lower.contains('.') && entry.size <= 8 << 20;
            if !named && !hashed {
                continue;
            }
            let Ok(reader) = opened.archive.open(entry.id, &opened.context) else { continue };
            let Ok(bytes) = read_all(reader.as_ref(), 64 * 1024 * 1024) else { continue };
            if hashed && !(bytes.starts_with(b"TJS2") || bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xfe])) {
                continue;
            }
            let text = if bytes.starts_with(b"TJS2") {
                let decompiled = crysirror_tjs2::bytecode::BytecodeFile::parse(&bytes).ok().and_then(|file| {
                    // KRKR_GREP_OBJECTS lists code objects; KRKR_GREP_OBJECT=n
                    // decompiles only object n.
                    if whole.is_some() && std::env::var_os("KRKR_GREP_OBJECTS").is_some() {
                        let text = file.objects.iter().enumerate().map(|(i, o)| format!(
                            "#{i} parent={:?} {:?} {}", o.parent, o.context_type, o.name(&file).unwrap_or(""),
                        )).collect::<Vec<_>>().join("\n");
                        return Some(text);
                    }
                    let object_index = std::env::var("KRKR_GREP_OBJECT").ok().and_then(|v| v.parse().ok());
                    if let (Some(n), Some(_)) = (object_index, std::env::var_os("KRKR_GREP_DISASM")) {
                        return file.disassemble_object_full(n).ok();
                    }
                    crysirror_tjs2::decompile::decompile(&file, &crysirror_tjs2::decompile::DecompileOptions { object_index, ..Default::default() }).ok()
                        .map(|out| out.sources.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n"))
                });
                match decompiled {
                    Some(text) => text,
                    None => continue,
                }
            } else {
                let Ok(text) = crysirror_resource::text_stream::decode_auto(&bytes, 64 * 1024 * 1024) else { continue };
                text
            };
            let lines: Vec<&str> = text.lines().collect();
            if whole.is_some() {
                out.extend(lines.iter().enumerate().map(|(k, l)| format!("{name}:{}:{}: {}", entry.path, k + 1, l.trim_end())));
                return Ok(out);
            }
            for (line_no, line) in lines.iter().enumerate() {
                if line.contains(needle) {
                    for (k, shown) in lines.iter().enumerate().skip(line_no).take(context + 1) {
                        out.push(format!("{name}:{}:{}: {}", entry.path, k + 1, shown.trim_end()));
                    }
                    if out.len() >= limit { return Ok(out); }
                }
            }
        }
    }
    Ok(out)
}

mod krkr_probe {
    use super::*;
    use crysirror_core::{service::AssetReader, state::ClientId};
    use crysirror_resource::{read_all, Archive, ArchiveFormat, DecodeContext, DependencyResolver};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    pub enum Protection {
        Hxv4,
        Nana { profile: String, setup_file: Option<String> },
        Cxdec { modules: Vec<String>, setup: bool },
        AdlerXor { modules: Vec<String> },
        HashFoldXor { key_bias: Option<u8>, zero_key: Option<u8> },
        Plain,
    }
    impl Protection {
        pub fn label(&self) -> &'static str {
            match self {
                Self::Hxv4 => "HXV4",
                Self::Nana { .. } => "Nana (local dls profile)",
                Self::HashFoldXor { .. } => "checksum-authenticated folded Adler XOR",
                Self::AdlerXor { .. } => "rotating Adler XOR",
                Self::Cxdec { modules, setup: false } if modules.len() > 1 => "CXDEC (combined modules)",
                Self::Cxdec { setup: false, .. } => "CXDEC (classic module)",
                Self::Cxdec { .. } => "CXDEC",
                Self::Plain => "plain XP3",
            }
        }
        pub(super) fn authenticated(&self, opened: &Opened) -> Result<Self> {
            if matches!(self, Self::Nana { setup_file: Some(_), .. }) {
                ensure!(opened.find("__cxdec/protection.json").is_some(),
                    "Nana startup input has no matching archive bootstrap");
            }
            if let Self::Cxdec{modules,setup:true}=self {
                // The format publishes a bootstrap only for setupArchiveData
                // startup redirects. Classic CXDEC uses ordinary startup.tjs
                // and must not require a synthetic protection.json resource.
                if opened.find("__cxdec/protection.json").is_none() {
                    return Ok(Self::Cxdec{modules:modules.clone(),setup:false});
                }
            }
            if !matches!(self, Self::HashFoldXor { .. }) { return Ok(self.clone()); }
            let profile = opened.archive.entries().iter()
                .find_map(|e| opened.archive.entry_metadata(e.id).get("hash_fold_xor").cloned())
                .context("folded Adler XOR profile has no authenticated entry metadata")?;
            let bias = u8::try_from(profile["key_bias"].as_u64().context("folded Adler XOR key bias")?)?;
            let zero_key = profile["zero_key"].as_u64().map(u8::try_from).transpose()?;
            Ok(Self::HashFoldXor { key_bias: Some(bias), zero_key })
        }
        /// `kirikiri/xp3` profile parameters, as the mount script declares.
        pub fn parameters(&self, exe: &str) -> Value {
            match self {
                Self::AdlerXor { modules } => json!({"adler_xor_modules": modules}),
                Self::HashFoldXor { key_bias, zero_key } => {
                    let mut profile = serde_json::Map::new();
                    if let Some(bias) = key_bias { profile.insert("key_bias".into(), json!(bias)); }
                    if let Some(key) = zero_key { profile.insert("zero_key".into(), json!(key)); }
                    json!({"hash_fold_xor": profile})
                },
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
                Self::Nana { profile, setup_file: Some(setup_file) } => json!({
                    "cxdec_nana_profile": profile, "cxdec_nana_setup_file": setup_file,
                    "cxdec_bootstrap_profile": "__cxdec/protection.json",
                }),
                Self::Nana { profile, setup_file: None } => json!({"cxdec_nana_profile": profile}),
                Self::Plain => json!({}),
            }
        }
    }

    struct File(Mutex<crate::source_path::SourceFile>);
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
    struct Companions(SourcePath);
    impl DependencyResolver for Companions {
        fn open(&self, path: &str) -> anyhow::Result<Arc<dyn AssetReader>> {
            anyhow::ensure!(
                !path.split(['/', '\\']).any(|s| s == ".."),
                "companion escapes the game folder"
            );
            let root = self.0.canonicalize()?;
            let file = root.join(path).canonicalize()?;
            ensure!(file.starts_with(&root) && file.is_file(), "companion escapes the game folder");
            Ok(Arc::new(File(Mutex::new(crate::source_path::SourceFile::open(file)?))))
        }
    }

    pub struct Opened {
        pub(super) archive: Box<dyn Archive>,
        pub(super) context: DecodeContext,
    }

    pub fn open(dir: &SourcePath, exe: &str, protection: &Protection) -> Result<Opened> {
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
        dir: &SourcePath,
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
        let source = Arc::new(File(Mutex::new(crate::source_path::SourceFile::open(&data)?)));
        let archive = crysirror_formats_arc::Xp3.open(source, &context)?;
        Ok(Opened { archive, context })
    }

    fn names_ci(dir: &SourcePath, name: &str) -> Result<SourcePath> {
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
        dir: &SourcePath,
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
        bail!("initial canvas constants not found in supported KiriKiri startup scripts")
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
            "sys_source/FixNum.tjs",
            "sys_source/Init.tjs",
            "sys_source/MainWindow.tjs",
            "macro/userFix.ks",
            "macro/userMacro.ks",
            "startup.tjs",
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
                    .filter(|l| {let lower=l.to_ascii_lowercase();lower.contains("width") || lower.contains("height") || lower.contains("size")})
                {
                    eprintln!("[detect] {path}: {}", l.trim());
                }
            }
            for (width,height) in [("scWidth","scHeight"),("nWND_WIDTH","nWND_HEIGHT")] {
                if let Some(canvas)=canvas_assignments(&text,width,height) {
                    return Ok(canvas);
                }
            }
        }
        bail!("initial canvas constants scWidth/scHeight or nWND_WIDTH/nWND_HEIGHT not found")
    }

    fn canvas_assignments(text:&str,width:&str,height:&str)->Option<[u32;2]> {
        let (mut w,mut h)=(None,None);
        let mut pairs=Vec::new();
        for line in text.lines() {
            if let Some(value)=assignment(line,width){w=Some(value);}
            if let Some(value)=assignment(line,height){h=Some(value);}
            if let (Some(x),Some(y))=(w,h) {
                pairs.push([x,y]);w=None;h=None;
            }
        }
        // Keep width/height from the same declaration pair. Independently
        // taking maxima invents1280x768 for1280x720 vs1024x768 branches.
        pairs.into_iter().max_by_key(|[w,h]|(u64::from(*w)*u64::from(*h),*w,*h))
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
        names: &[String],
        name: &str,
        exe: &str,
        protection: Protection,
        canvas: [u32; 2],
    ) -> Result<Plan> {
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
            Protection::Cxdec { setup: true, .. } | Protection::Nana { setup_file: Some(_), .. } => {
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
        let mut companions = with_ext(names, "xp3").filter(|n| **n != data).peekable();
        let mut profiles = std::collections::BTreeMap::new();
        let companion_profile = if matches!(protection, Protection::Hxv4) && companions.peek().is_some() {
            let mut companion = profile.clone();
            companion["parameters"]["kirikiri/xp3"] = json!({
                "hxv4_executable": exe, "hxv4_hashed_root": "__hxv4",
            });
            profiles.insert(HXV4_COMPANION_PROFILE.into(), companion);
            HXV4_COMPANION_PROFILE
        } else {
            "game"
        };
        profiles.insert("game".into(), profile);
        let mut mounts = vec![ArchiveMount {
            source: data.clone(), target: String::new(), fallback: false,
            options: json!({"format":"kirikiri/xp3","profile":"game"}),
        }];
        let mut points = std::collections::BTreeSet::new();
        for source in companions {
            ensure!(mounts.len() < crysirror_runtime::vfs::MAX_PROVIDER_MOUNTS, "KiriKiri archive mount limit");
            let stem = source[..source.len() - 4].to_ascii_lowercase();
            ensure!(stem != "data" && points.insert(stem.clone()), "ambiguous KiriKiri archive: {source}");
            mounts.push(ArchiveMount {
                source: source.clone(), target: format!("__xp3/{stem}"), fallback: false,
                options: json!({"format":"kirikiri/xp3","profile":companion_profile}),
            });
        }
        Ok(Plan {
            engine: "KiriKiri",
            project: project(name, canvas, "krkr", config),
            evidence: Vec::new(),
            profiles,
            mounts,
        })
    }
}

/// Read bounded launch metadata, never an entire archive. A symlink companion
/// still has to resolve inside the selected game's capability root.
fn read_bounded(dir: &SourcePath, path: &str, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let full = dir.join(path).canonicalize()?;
    let root = dir.canonicalize()?;
    ensure!(
        full.starts_with(root),
        "launch metadata escapes game folder: {path}"
    );
    let mut bytes = Vec::new();
    crate::source_path::SourceFile::open(full)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "launch metadata exceeds {limit} bytes: {path}"
    );
    Ok(bytes)
}

fn nscripter(dir: &SourcePath, names: &[String], name: &str) -> Result<Plan> {
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
    let mounts: Vec<ArchiveMount> = sources
        .into_iter()
        .map(|source| {
            let format = if source.to_ascii_lowercase().ends_with(".sar") {
                "nscripter/sar"
            } else {
                "nscripter/nsa"
            };
            ArchiveMount {
                fallback:false,
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

fn rfvp(dir: &SourcePath, names: &[String], name: &str) -> Result<Plan> {
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

fn siglus(dir: &SourcePath, names: &[String], name: &str) -> Result<Plan> {
    use crysirror_siglus_integrated::{
        gameexe::{decode_gameexe_dat_bytes, GameexeConfig},
        decode_material,
        key_toml,
    };
    let scene = find(names, "Scene.pck").context("Siglus Scene.pck missing")?;
    let gameexe = find(names, "Gameexe.dat")
        .or_else(|| find(names, "Gameexe.ini"))
        .context("Siglus Gameexe.dat/ini missing")?;
    let key = find(names, "key.toml").or_else(|| find(names, ".crysirror-validation-key.toml"));
    let materials = if let Some(path) = key {
        let bytes = read_bounded(dir, path, 64 * 1024)?;
        key_toml::parse_key_toml(std::str::from_utf8(&bytes)?)?
    } else {
        key_toml::KeyTomlConfig::default()
    };
    let bytes = read_bounded(dir, gameexe, 16 * 1024 * 1024)?;
    let scene_bytes = read_bounded(dir, scene, 64 * 1024 * 1024)?;
    let materials = decode_material::resolve(&bytes, &scene_bytes, materials)
        .context("Siglus automatic Gameexe/Scene decode material")?;
    let options = decode_material::game_options(&materials);
    let (text, _) = decode_gameexe_dat_bytes(&bytes, &options)
        .context("Siglus Gameexe decode after material resolution")?;
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
        // The integrated runtime opens its original voice packs through the
        // scoped native root. Shared archive mounts belong to the extraction
        // path; adding one per pack here duplicates that work and prevents
        // games with more packs than MAX_PROVIDER_MOUNTS from launching.
        mounts: Vec::new(),
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
    } else if cfg!(target_os = "ios") {
        &["/System/Library/Fonts/LanguageSupport/PingFang.ttc",
          "/System/Library/Fonts/Core/PingFang.ttc"]
    } else if cfg!(target_os = "android") {
        &["/system/fonts/NotoSansCJK-Regular.ttc", "/system/fonts/Roboto-Regular.ttf"]
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
#[path = "detect/krkr_overlay_tests.rs"]
mod krkr_overlay_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_krkr_entry_uses_its_manifest_only_for_the_launcher() {
        let temp = tempfile::tempdir().unwrap();
        let dir = super::SourcePath::local(temp.path());
        let names = ["play.exe", "runtime.exe", "another.exe", "KrkrPatch.json"].map(String::from);
        let mut stub = b"MZ".to_vec();
        stub.extend("KrkrPatch.json".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(temp.path().join("play.exe"), stub).unwrap();
        std::fs::write(temp.path().join("runtime.exe"), b"MZruntime").unwrap();
        std::fs::write(temp.path().join("another.exe"), b"MZanother").unwrap();
        let manifest = temp.path().join("KrkrPatch.json");
        std::fs::write(&manifest, r#"{"gameExecutableFile":"runtime.exe"}"#).unwrap();
        assert_eq!(super::krkr_entry(&dir, &names, Some("play.exe")).unwrap().executable, "runtime.exe");
        assert_eq!(super::krkr_entry(&dir, &names, Some("another.exe")).unwrap().executable, "another.exe");
        std::fs::write(&manifest, r#"{"gameExecutableFile":"../other.exe"}"#).unwrap();
        assert!(super::krkr_entry(&dir, &names, Some("play.exe")).is_err());
    }
    use super::*;

    #[test]
    fn reallive_voice_archives_use_shared_mounts_without_extraction() {
        let dir=tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("KOE")).unwrap();
        for file in ["KOE/z0629.nwk", "KOE/z0751.ovk", "z0001.koe"] {
            std::fs::write(dir.path().join(file), []).unwrap();
        }
        let mounts=reallive_voice_mounts(&SourcePath::local(dir.path()), b"#FOLDNAME.KOE=\"koe\"\n").unwrap();
        assert_eq!(mounts.len(),3);
        assert_eq!(mounts[0].source,"KOE/z0629.nwk");
        assert_eq!(mounts[0].target,mounts[0].source);
        assert_eq!(mounts[0].options["format"],"real-live/nwk");
        assert_eq!(mounts[1].options["format"],"real-live/ovk");
        assert_eq!(mounts[2].options["format"],"real-live/koepac");
        #[cfg(unix)] {
            let outside=tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("z0002.nwk"), []).unwrap();
            std::os::unix::fs::symlink(outside.path().join("z0002.nwk"),dir.path().join("KOE/z0002.nwk")).unwrap();
            assert!(reallive_voice_mounts(&SourcePath::local(dir.path()), b"").unwrap_err().to_string().contains("escapes"));
        }
    }

    fn cat_startup_fixture(root:&Path) {
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/startup.xml"),b"<document><SCREEN><width>640</width><height>360</height><v_width>1280</v_width><v_height>720</v_height></SCREEN><SCRIPT><start>kcs.int/main.kcs</start></SCRIPT><ETC><heap>4096</heap></ETC></document>").unwrap();
        std::fs::write(root.join("kcs.int"),b"KIF\0\0\0\0\0").unwrap();
        std::fs::write(root.join("cs2.exe"),b"MZ").unwrap();
    }

    #[test]
    fn system4_detection_builds_original_nested_ini_plan_without_unpacking() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("GAMEDATA");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("System40.ini"), b"CodeName=\"Game.ain\" GameName=\"Example\" ViewWidth=800 ViewHeight=600").unwrap();
        std::fs::write(root.join("Game.ain"), b"AI2\0").unwrap();
        std::fs::write(root.join("GameGA.ald"), []).unwrap();
        std::fs::write(root.join("GameGB.ald"), []).unwrap();
        {
            let plan = detect(dir.path()).unwrap();
            assert_eq!(plan.engine, "System4");
            assert_eq!(plan.project["clients"][0]["adapter"], "system4");
            assert_eq!(plan.project["clients"][0]["asset_prefix"], "GAMEDATA");
            assert_eq!(plan.project["clients"][0]["config"]["program"], "Game.ain");
            assert_eq!(plan.mounts.len(), 2);
            assert!(plan.mounts.iter().all(|m| m.options["format"] == "alice-soft/ald"));
            let typed: crysirror_runtime::Project = serde_json::from_value(plan.project.clone()).unwrap();
            assert!(typed.clients[0].program.is_empty());
            assert_eq!(detect(&root).unwrap().project["clients"][0]["config"]["ini"], "System40.ini");
        }
    }

    #[test]
    fn catsystem2_detection_selects_original_nested_capability_and_startup() {
        let dir=tempfile::tempdir().unwrap();
        cat_startup_fixture(&dir.path().join("data"));
        std::fs::write(dir.path().join("installer.exe"),vec![0;1024]).unwrap();
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
    }

    #[test]
    #[cfg(unix)]
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
        {
            let plan = detect(dir.path()).unwrap();
            assert_eq!(plan.engine, "circus");
            assert_eq!(plan.project["canvas"], json!([1280, 720]));
            assert_eq!(
                plan.project["clients"][0]["config"],
                json!({"script":"start","font":"default"})
            );
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
        assert!(read_bounded(&SourcePath::local(dir.path()), "0.txt", 1024).is_err());
        std::fs::write(dir.path().join("large"), [0; 16]).unwrap();
        assert!(read_bounded(&SourcePath::local(dir.path()), "large", 8).is_err());
    }

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

    #[test]
    fn hxv4_companions_keep_the_bootstrap_and_root_on_data_only() {
        let names = ["DATA.XP3", "Voice.Xp3", "patch@r10.xp3", "game.exe"].map(String::from);
        let plan = krkr_probe::plan(
            &names, "authored", "game.exe",
            krkr_probe::Protection::Hxv4, [640, 480],
        ).unwrap();
        let mounts: Vec<_> = plan.mounts.iter()
            .map(|m| (m.source.as_str(), m.target.as_str())).collect();
        assert_eq!(mounts, [
            ("DATA.XP3", ""), ("Voice.Xp3", "__xp3/voice"), ("patch@r10.xp3", "__xp3/patch@r10"),
        ]);
        for (index, mount) in plan.mounts.iter().enumerate() {
            assert_eq!(mount.is_hxv4_companion(), index != 0);
            let profile = &plan.profiles[mount.options["profile"].as_str().unwrap()];
            let params = &profile["parameters"]["kirikiri/xp3"];
            assert_eq!(params["hxv4_executable"], "game.exe");
            assert_eq!(params["hxv4_hashed_root"], "__hxv4");
            assert_eq!(params.get("hxv4_startup_target").is_some(), index == 0);
            assert_eq!(params.get("hxv4_bootstrap_profile").is_some(), index == 0);
        }
    }

    #[test]
    fn krkr_archive_plans_reject_collisions_and_provider_overflow() {
        let ambiguous = ["data.xp3", "Voice.xp3", "voice.XP3"].map(String::from);
        let make_plan = |names: &[String]| {
            krkr_probe::plan(names, "authored", "game.exe", krkr_probe::Protection::Hxv4, [640, 480])
        };
        assert!(make_plan(&ambiguous).unwrap_err().to_string().contains("ambiguous"));
        let mut names = vec!["data.xp3".into()];
        names.extend((1..crysirror_runtime::vfs::MAX_PROVIDER_MOUNTS).map(|i| format!("voice{i}.xp3")));
        assert!(make_plan(&names).is_ok());
        names.push("excess.xp3".into());
        assert!(make_plan(&names).unwrap_err().to_string().contains("mount limit"));
    }

    #[test]
    fn installed_player_selects_native_production_mode_before_game_scripts() {
        let names = vec!["data.xp3".to_owned(), "voice.xp3".to_owned()];
        let plan = krkr_probe::plan(
            &names,
            "authored",
            "game.exe",
            krkr_probe::Protection::Plain,
            [640, 480],
        ).unwrap();
        assert_eq!(plan.mounts.len(), 2);
        assert!(plan.mounts.iter().all(|m| m.options["profile"] == "game" && !m.is_hxv4_companion()));
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
