// SPDX-License-Identifier: MPL-2.0
use crate::{read_all, Archive, ArchiveEntry, ArchiveFormat, DecodeContext};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::AssetReader,
    vfs::VfsProvider,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Probe {
    pub confidence: u8,
}
impl Probe {
    pub const NO: Self = Self { confidence: 0 };
    pub const YES: Self = Self { confidence: 100 };
}
#[derive(Default)]
pub struct FormatRegistry {
    formats: BTreeMap<&'static str, Arc<dyn ArchiveFormat>>,
}
impl FormatRegistry {
    pub fn register(&mut self, format: Arc<dyn ArchiveFormat>) -> Result<()> {
        let id = format.id();
        ensure!(
            !id.is_empty() && id.len() <= 128 && !self.formats.contains_key(id),
            "invalid or duplicate archive format"
        );
        self.formats.insert(id, format);
        Ok(())
    }
    pub fn names(&self) -> Vec<&'static str> {
        self.formats.keys().copied().collect()
    }
    pub fn probe(
        &self,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Vec<(&'static str, Probe)>> {
        let mut matches = Vec::new();
        for (id, format) in &self.formats {
            let reader = ProbeReader {
                source: source.clone(),
                remaining: AtomicU64::new(context.limits.max_probe_bytes),
            };
            let probe = format
                .probe(&reader)
                .with_context(|| format!("probe archive format {id}"))?;
            ensure!(probe.confidence <= 100, "invalid format confidence");
            if probe.confidence > 0 {
                matches.push((*id, probe));
            }
        }
        Ok(matches)
    }
    pub fn open(
        &self,
        source: Arc<dyn AssetReader>,
        format: Option<&str>,
        context: &DecodeContext,
    ) -> Result<(&'static str, Box<dyn Archive>)> {
        let id = if let Some(id) = format {
            *self
                .formats
                .get_key_value(id)
                .context("archive format not registered")?
                .0
        } else {
            let matches = self.probe(source.clone(), context)?;
            ensure!(
                matches.len() == 1,
                "archive detection needs one match, found {:?}; specify format",
                matches.iter().map(|(id, _)| *id).collect::<Vec<_>>()
            );
            matches[0].0
        };
        let archive = self.formats[id]
            .open(source, context)
            .with_context(|| format!("open {id}"))?;
        ensure!(
            archive.entries().len() <= context.limits.max_entries,
            "archive entry limit"
        );
        Ok((id, archive))
    }
}
struct ProbeReader {
    source: Arc<dyn AssetReader>,
    remaining: AtomicU64,
}
impl AssetReader for ProbeReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        if self
            .remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_sub(out.len() as u64)
            })
            .is_err()
        {
            return Err(crysirror_core::Error::Limit("archive probe bytes"));
        }
        self.source.read_at(offset, out)
    }
}
/// A mounted archive exposes canonical entry paths through the same VFS used
/// by every VM. It never writes extracted game data to the filesystem.
pub struct ArchiveMount {
    archive: Box<dyn Archive>,
    context: DecodeContext,
    paths: BTreeMap<String, u32>,
}
impl ArchiveMount {
    pub fn new(archive: Box<dyn Archive>, context: DecodeContext) -> Result<Self> {
        ensure!(
            archive.entries().len() <= context.limits.max_entries,
            "archive entry limit"
        );
        let mut paths = BTreeMap::new();
        let mut ids = std::collections::BTreeSet::new();
        for entry in archive.entries() {
            let native = entry.path.replace('\\', "/");
            let path = VfsPath::new(Mount::Asset, &native)
                .map_err(|e| anyhow::anyhow!("archive entry path: {e}"))?;
            ensure!(
                paths.insert(path.relative().into(), entry.id).is_none() && ids.insert(entry.id),
                "duplicate archive path or id"
            );
        }
        Ok(Self {
            archive,
            context,
            paths,
        })
    }
    pub fn entries(&self) -> &[ArchiveEntry] {
        self.archive.entries()
    }
    pub fn open_entry(&self, path: &str) -> Result<Arc<dyn AssetReader>> {
        let id = *self.paths.get(path).context("archive entry not found")?;
        self.archive.open(id, &self.context)
    }
}
impl VfsProvider for ArchiveMount {
    fn kind(&self) -> &'static str {
        "archive"
    }
    fn read(&self, path: &str) -> crysirror_core::Result<Vec<u8>> {
        let Some(&id) = self.paths.get(path) else {
            return Err(crysirror_core::Error::NotFound);
        };
        let source = self
            .archive
            .open(id, &self.context)
            .map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))?;
        read_all(source.as_ref(), self.context.limits.max_entry_size)
            .map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))
    }
    fn exists(&self, path: &str) -> bool {
        self.paths.contains_key(path)
    }
    fn list(&self) -> crysirror_core::Result<Vec<String>> {
        Ok(self.paths.keys().cloned().collect())
    }
    fn open(&self, path: &str) -> crysirror_core::Result<Arc<dyn AssetReader>> {
        let Some(&id) = self.paths.get(path) else {
            return Err(crysirror_core::Error::NotFound);
        };
        self.archive
            .open(id, &self.context)
            .map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))
    }
}
