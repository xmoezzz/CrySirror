// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Read-only MSI installed-file views through capability readers. No installer
//! actions, registry changes, native code, filesystem extraction or COM.
use crate::{AssetIo, DecodeContext};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::AssetReader,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::Arc,
};
const MAX_ROWS: usize = 32768;
/// Immutable logical installed file, usable by any adapter or asset tool.
#[derive(Clone, Debug)]
pub struct InstallerFile {
    pub path: String,
    pub size: u64,
    pub sequence: u32,
    pub cabinet_stream: String,
    pub cabinet_key: String,
}
/// A catalog owns only its scoped source and immutable data. The third-party
/// MSI/CAB readers are created and destroyed inside each worker invocation;
/// neither !Send MSI objects nor decoder pointers cross task boundaries.
pub struct InstallerCatalog {
    source: Arc<dyn AssetReader>,
    files: Vec<InstallerFile>,
}
fn text<'a>(row: &'a msi::Row, column: &str) -> Result<&'a str> {
    ensure!(row.has_column(column), "MSI missing column {column}");
    row[column]
        .as_str()
        .with_context(|| format!("MSI string {column}"))
}
fn number(row: &msi::Row, column: &str) -> Result<u32> {
    ensure!(row.has_column(column), "MSI missing column {column}");
    let n = row[column]
        .as_int()
        .with_context(|| format!("MSI integer {column}"))?;
    ensure!(n >= 0, "MSI negative {column}");
    Ok(n as u32)
}
fn long_name(value: &str) -> &str {
    value
        .rsplit_once('|')
        .map(|(_, long)| long)
        .unwrap_or(value)
}
fn safe_component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && !value.contains(['/', '\\', ':']) && value != "." && value != "..",
        "MSI unsafe path component"
    );
    Ok(())
}
impl InstallerCatalog {
    /// Index only relational metadata. All work belongs on an I/O worker.
    pub fn open(source: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<Self> {
        cx.check_cancelled()?;
        let mut package = msi::Package::open(AssetIo::new(source.clone())?)?;
        let mut dirs = BTreeMap::<String, (Option<String>, String)>::new();
        for row in package.select_rows(msi::Select::table("Directory"))? {
            cx.check_cancelled()?;
            ensure!(dirs.len() < MAX_ROWS, "MSI directory budget");
            let key = text(&row, "Directory")?.to_owned();
            ensure!(
                row.has_column("Directory_Parent"),
                "MSI missing parent column"
            );
            let parent = row["Directory_Parent"].as_str().map(str::to_owned);
            let raw = text(&row, "DefaultDir")?.split(':').next().unwrap();
            let name = long_name(raw).to_owned();
            if parent.is_some() && name != "." {
                safe_component(&name)?;
            }
            ensure!(
                dirs.insert(key, (parent, name)).is_none(),
                "duplicate MSI directory"
            );
        }
        let mut components = BTreeMap::new();
        for row in package.select_rows(msi::Select::table("Component"))? {
            cx.check_cancelled()?;
            ensure!(components.len() < MAX_ROWS, "MSI component budget");
            ensure!(
                components
                    .insert(
                        text(&row, "Component")?.to_owned(),
                        text(&row, "Directory_")?.to_owned()
                    )
                    .is_none(),
                "duplicate MSI component"
            );
        }
        let mut media = BTreeMap::new();
        for row in package.select_rows(msi::Select::table("Media"))? {
            ensure!(media.len() < 256, "MSI media budget");
            let cabinet = text(&row, "Cabinet")?;
            let stream = cabinet
                .strip_prefix('#')
                .context("external MSI cabinets need an explicit scoped dependency mapping")?;
            ensure!(
                !stream.is_empty() && stream.len() <= 512,
                "MSI cabinet stream name"
            );
            ensure!(
                media
                    .insert(number(&row, "LastSequence")?, stream.to_owned())
                    .is_none(),
                "duplicate MSI media range"
            );
        }
        let mut files = Vec::new();
        let mut paths = BTreeSet::new();
        for row in package.select_rows(msi::Select::table("File"))? {
            cx.check_cancelled()?;
            ensure!(
                files.len() < MAX_ROWS.min(cx.limits.max_entries),
                "MSI file budget"
            );
            let name = long_name(text(&row, "FileName")?);
            safe_component(name)?;
            let mut folder = components
                .get(text(&row, "Component_")?)
                .context("MSI missing component")?
                .as_str();
            let mut parts = Vec::new();
            let mut visited = BTreeSet::new();
            loop {
                ensure!(
                    visited.len() < 64 && visited.insert(folder),
                    "MSI directory cycle/depth"
                );
                let (parent, name) = dirs.get(folder).context("MSI missing directory")?;
                let Some(parent) = parent else {
                    break;
                };
                if name != "." {
                    parts.push(name.as_str());
                }
                folder = parent;
            }
            parts.reverse();
            parts.push(name);
            let path = parts.join("/");
            VfsPath::new(Mount::Asset, &path)?;
            ensure!(
                paths.insert(path.to_ascii_lowercase()),
                "duplicate MSI installed path"
            );
            let sequence = number(&row, "Sequence")?;
            ensure!(sequence > 0, "MSI file sequence");
            let cabinet_stream = media
                .range(sequence..)
                .next()
                .context("MSI file missing media range")?
                .1
                .clone();
            let cabinet_key = text(&row, "File")?.to_owned();
            ensure!(
                !cabinet_key.is_empty() && cabinet_key.len() <= 512,
                "MSI cabinet key budget"
            );
            let size = number(&row, "FileSize")? as u64;
            ensure!(size <= cx.limits.max_entry_size, "MSI entry size budget");
            files.push(InstallerFile {
                path,
                size,
                sequence,
                cabinet_stream,
                cabinet_key,
            });
        }
        ensure!(!files.is_empty(), "MSI has no installed files");
        Ok(Self { source, files })
    }
    pub fn files(&self) -> &[InstallerFile] {
        &self.files
    }
    /// Decode one selected member, retaining the common decoded-byte lease.
    /// No extraction or installation side effect. Invoke on a resource worker.
    pub fn read(&self, index: usize, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        let file = self.files.get(index).context("MSI file index")?;
        cx.check_cancelled()?;
        cx.decode_with(file.size, || {
            let mut package = msi::Package::open(AssetIo::new(self.source.clone())?)?;
            let stream = package.read_stream(&file.cabinet_stream)?;
            let mut cabinet = cab::Cabinet::new(stream)?;
            let mut reader = cabinet.read_file(&file.cabinet_key)?;
            let mut bytes = Vec::new();
            let mut block = [0; 32768];
            loop {
                cx.check_cancelled()?;
                let n = reader.read(&mut block)?;
                if n == 0 {
                    break;
                }
                ensure!(
                    (bytes.len() as u64).saturating_add(n as u64) <= file.size,
                    "MSI cabinet output exceeds declared file size"
                );
                bytes.extend_from_slice(&block[..n]);
            }
            ensure!(
                bytes.len() as u64 == file.size,
                "MSI cabinet file size mismatch"
            );
            Ok(bytes)
        })
    }
}
/// Shared `windows/msi` archive adapter. A CFB header is only a candidate;
/// opening must validate the MSI database and supported embedded media profile.
pub struct MsiArchiveFormat;
impl crate::ArchiveFormat for MsiArchiveFormat {
    fn id(&self) -> &'static str {
        "windows/msi"
    }
    fn probe(&self, source: &dyn AssetReader) -> Result<crate::Probe> {
        if source.len().map_err(|e| anyhow::anyhow!("{e}"))? < 8 {
            return Ok(crate::Probe::NO);
        }
        let mut magic = [0; 8];
        crate::read_exact_at(source, 0, &mut magic)?;
        Ok(
            if magic == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
                crate::Probe { confidence: 10 }
            } else {
                crate::Probe::NO
            },
        )
    }
    fn open(
        &self,
        source: Arc<dyn AssetReader>,
        cx: &DecodeContext,
    ) -> Result<Box<dyn crate::Archive>> {
        let catalog = InstallerCatalog::open(source, cx)?;
        // A selected installed directory can be mounted as a capability root
        // alongside loose distribution media, without extracting either one.
        let root=if let Some(options)=cx.parameters.get("windows/msi") {
            let object=options.as_object().context("MSI options must be an object")?;
            ensure!(object.keys().all(|key|key=="root"),"unknown MSI archive option");
            object.get("root").map(|value|value.as_str().context("MSI root must be a string")).transpose()?.unwrap_or("")
        } else {""};
        if !root.is_empty() {
            let checked=crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset,root)?;
            ensure!(checked.relative()==root,"MSI root must be canonical");
        }
        let prefix=format!("{root}/");
        let entries:Vec<_>=catalog.files.iter().enumerate().filter_map(|(id,f)|{
            let path=if root.is_empty(){f.path.as_str()}else{f.path.strip_prefix(&prefix)?};
            Some(crate::ArchiveEntry{
                id:id as u32,path:path.to_owned(),size:f.size,stored_size:f.size,
                metadata:serde_json::json!({"installed_path":f.path,"sequence":f.sequence,"cabinet_stream":f.cabinet_stream,"cabinet_key":f.cabinet_key,"stored_size_is_logical":true}),
            })
        }).collect();
        ensure!(root.is_empty()||!entries.is_empty(),"MSI installed root not found: {root}");
        Ok(Box::new(InstalledArchive { catalog, entries }))
    }
}
struct InstalledArchive {
    catalog: InstallerCatalog,
    entries: Vec<crate::ArchiveEntry>,
}
impl crate::Archive for InstalledArchive {
    fn entries(&self) -> &[crate::ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        self.catalog.read(id as usize, cx)
    }
}
