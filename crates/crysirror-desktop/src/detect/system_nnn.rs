// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Reviewed System-NNN install media are mounted as data, never installed or run.
use super::*;
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};
struct Reader {
    file: Mutex<std::fs::File>,
    length: u64,
}
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.length)
    }
    fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| crysirror_core::Error::Invalid("NNN detection reader lock"))?;
        file.seek(SeekFrom::Start(at))
            .and_then(|_| file.read(out))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
fn source(root: &Path, path: &str) -> Result<Arc<dyn AssetReader>> {
    let full = root.join(path).canonicalize()?;
    ensure!(
        full.starts_with(root),
        "NNN detection escapes capability root"
    );
    let file = std::fs::File::open(full)?;
    let length = file.metadata()?.len();
    Ok(Arc::new(Reader {
        file: Mutex::new(file),
        length,
    }))
}
pub(super) fn detect(dir: &Path, names: &[String], name: &str) -> Result<Option<Plan>> {
    let root = dir.canonicalize()?;
    ensure!(
        with_ext(names, "msi").count() <= 4,
        "installer detection candidate budget"
    );
    let mut cx = DecodeContext::new(ClientId(1));
    cx.limits.max_entry_size = 1 << 30;
    for filename in with_ext(names, "msi") {
        let body = source(&root, filename)?;
        let mut magic = [0; 8];
        if body.read_at(0, &mut magic)? != 8
            || magic != [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
        {
            continue;
        }
        let catalog = InstallerCatalog::open(body, &cx)
            .with_context(|| format!("installer catalog {filename}"))?;
        for configuration in catalog
            .files()
            .iter()
            .filter(|file| file.path.ends_with("/init2/game.fxf") || file.path == "init2/game.fxf")
        {
            let prefix = configuration
                .path
                .strip_suffix("init2/game.fxf")
                .unwrap()
                .trim_end_matches('/');
            let logical = |tail: &str| {
                if prefix.is_empty() {
                    tail.to_owned()
                } else {
                    format!("{prefix}/{tail}")
                }
            };
            if !catalog
                .files()
                .iter()
                .any(|file| file.path == logical("spt/sys.spt"))
            {
                continue;
            }
            // Candidate recognition uses original data tables. The execution ABI
            // still needs a reviewed module; it is never inferred from filenames.
            let mut module = None;
            for (index, file) in catalog.files().iter().enumerate() {
                let (parent, leaf) = file
                    .path
                    .rsplit_once('/')
                    .unwrap_or(("", file.path.as_str()));
                if parent != prefix || !leaf.to_ascii_lowercase().ends_with(".exe") {
                    continue;
                }
                let bytes = read_all(catalog.read(index, &cx)?.as_ref(), 32 << 20)?;
                let pe = crysirror_resource::pe::PeData::parse(&bytes)?;
                let hash = pe
                    .sha256()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                #[cfg(feature = "system-nnn")]
                if hash == crysirror_system_nnn::native_bootstrap::MUGEN2_MODULE_SHA256 {
                    ensure!(module.is_none(), "multiple reviewed NNN callback modules");
                    module = Some(leaf.to_owned());
                }
                #[cfg(not(feature = "system-nnn"))]
                {
                    let _ = hash;
                    module = Some(leaf.to_owned());
                }
            }
            let module=module.context("recognized System-NNN SPT install media, but its callback module ABI is not reviewed")?;
            #[cfg(not(feature = "system-nnn"))]
            {
                let _ = (module, name);
                bail!("recognized System-NNN install media; native adapter is not linked; enable feature system-nnn");
            }
            #[cfg(feature = "system-nnn")]
            return Ok(Some(plan(name, &module, Some((filename, prefix)))));
        }
    }
    // Already-installed copies use the same capability paths and adapter.
    if dir.join("spt/sys.spt").is_file()
        && (dir.join("init2/game.fxf").is_file() || dir.join("init/game.xtx").is_file())
    {
        #[cfg(not(feature = "system-nnn"))]
        bail!("recognized System-NNN installation; enable feature system-nnn");
        #[cfg(feature = "system-nnn")]
        for filename in with_ext(names, "exe") {
            let bytes = read_all(source(&root, filename)?.as_ref(), 32 << 20)?;
            let pe = crysirror_resource::pe::PeData::parse(&bytes)?;
            let hash = pe
                .sha256()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if hash == crysirror_system_nnn::native_bootstrap::MUGEN2_MODULE_SHA256 {
                return Ok(Some(plan(name, filename, None)));
            }
        }
        #[cfg(feature = "system-nnn")]
        bail!("recognized System-NNN installation has an unreviewed callback module ABI");
    }
    Ok(None)
}
#[cfg(feature = "system-nnn")]
fn plan(name: &str, module: &str, installer: Option<(&str, &str)>) -> Plan {
    let mut profiles = std::collections::BTreeMap::new();
    let mounts=installer.map(|(source,prefix)|{
        profiles.insert("nnn-installer".into(),json!({"limits":{"max_entry_size":1073741824},"parameters":{"windows/msi":{"root":prefix}}}));
        vec![ArchiveMount{fallback:true,source:source.into(),target:"".into(),options:json!({"format":"windows/msi","profile":"nnn-installer"})}]
    }).unwrap_or_default();
    Plan {
        engine: "System-NNN",
        project: json!({"schema":2,"name":name,"content_revision":"system-nnn-native-v1","assets":"game","canvas":[800,600],"clients":[{"name":"game","adapter":"system-nnn","game":name,"asset_prefix":"","config":{"root":"","module":module,"font":"default","entry":"controller"}}]}),
        profiles,
        mounts,
        evidence: vec![],
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_native_tables_or_installer_header_means_no_nnn_match() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("renamed.msi"), b"not an installer").unwrap();
        assert!(detect(root.path(), &["renamed.msi".into()], "authored")
            .unwrap()
            .is_none());
    }
    #[test]
    #[cfg(feature = "system-nnn")]
    #[ignore = "reads owner installation media; no original executable is run"]
    fn owner_media_plan_selects_real_adapter_and_scoped_installed_archive_root() {
        let root = PathBuf::from(std::env::var("CRYSIRROR_TEST_NNN_ROOT").unwrap());
        let names = listing(&root).unwrap();
        let plan = detect(&root, &names, "NNN owner media").unwrap().unwrap();
        assert_eq!(plan.engine, "System-NNN");
        assert_eq!(plan.project["clients"][0]["adapter"], "system-nnn");
        let config: crysirror_system_nnn::installed::Config =
            serde_json::from_value(plan.project["clients"][0]["config"].clone()).unwrap();
        config.validate().unwrap();
        assert_eq!(plan.mounts.len(), 1);
        assert_eq!(plan.mounts[0].target, "");
        assert_eq!(plan.mounts[0].options["format"], "windows/msi");
        assert_eq!(
            plan.profiles["nnn-installer"]["parameters"]["windows/msi"]["root"],
            "mugen2"
        );
        println!("{}", serde_json::to_string(&plan).unwrap());
    }
}
