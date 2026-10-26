// SPDX-License-Identifier: MPL-2.0
//! Shared encoded-file/audio conversion contracts. Parsers never own a VM,
//! device or ambient filesystem. Results can feed images, sound or mounted VFS.
use crate::{DecodeContext, Probe};
use anyhow::{ensure, Context, Result};
use crysirror_core::service::AssetReader;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

pub trait DependencyResolver: Send + Sync {
    fn open(&self, path: &str) -> Result<Arc<dyn AssetReader>>;
}
impl<F> DependencyResolver for F
where
    F: Fn(&str) -> Result<Arc<dyn AssetReader>> + Send + Sync,
{
    fn open(&self, path: &str) -> Result<Arc<dyn AssetReader>> {
        self(path)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Audio,
}
#[derive(Clone)]
pub struct DecodedFile {
    pub reader: Arc<dyn AssetReader>,
    /// Suggested lower-case extension, without a leading dot. Metadata-only:
    /// no implicit writes or renaming of original resources occur.
    pub extension: String,
    pub metadata: serde_json::Value,
}
pub trait FileFormat: Send + Sync {
    fn id(&self) -> &'static str;
    fn kind(&self) -> FileKind;
    fn probe(&self, path: &str, source: &dyn AssetReader) -> Result<Probe>;
    fn decode(
        &self,
        path: &str,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<DecodedFile>;
}
#[derive(Clone, Default)]
pub struct FileRegistry {
    formats: BTreeMap<&'static str, Arc<dyn FileFormat>>,
}
impl FileRegistry {
    pub fn register(&mut self, format: Arc<dyn FileFormat>) -> Result<()> {
        let id = format.id();
        ensure!(
            !id.is_empty() && id.len() <= 128 && !self.formats.contains_key(id),
            "invalid or duplicate file decoder"
        );
        self.formats.insert(id, format);
        Ok(())
    }
    pub fn names(&self) -> Vec<&'static str> {
        self.formats.keys().copied().collect()
    }
    pub fn formats(&self, kind: FileKind) -> Vec<&'static str> {
        self.formats
            .iter()
            .filter(|(_, f)| f.kind() == kind)
            .map(|(id, _)| *id)
            .collect()
    }
    pub fn probe(
        &self,
        path: &str,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Vec<(&'static str, Probe)>> {
        let mut matches = vec![];
        for (id, format) in &self.formats {
            let reader = BoundedProbe {
                source: source.clone(),
                remaining: AtomicU64::new(context.limits.max_probe_bytes),
            };
            let probe = format
                .probe(path, &reader)
                .with_context(|| format!("probe file decoder {id}"))?;
            ensure!(probe.confidence <= 100, "invalid file probe confidence");
            if probe.confidence > 0 {
                matches.push((*id, probe));
            }
        }
        Ok(matches)
    }
    pub fn decode(
        &self,
        path: &str,
        source: Arc<dyn AssetReader>,
        format: Option<&str>,
        context: &DecodeContext,
    ) -> Result<(&'static str, DecodedFile)> {
        let id = if let Some(id) = format {
            self.formats
                .get_key_value(id)
                .context("file decoder not registered")?
                .0
                .to_owned()
        } else {
            let candidates = self.probe(path, source.clone(), context)?;
            let best = candidates
                .iter()
                .map(|(_, p)| p.confidence)
                .max()
                .unwrap_or(0);
            let candidates = candidates
                .iter()
                .filter(|(_, p)| p.confidence == best)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            ensure!(candidates.len()==1,"file detection needs one highest-confidence match, found {candidates:?}; specify format");
            candidates[0]
        };
        let mut scoped = context.clone();
        scoped.source_name = Some(path.into());
        let output = self.formats[id]
            .decode(path, source, &scoped)
            .with_context(|| format!("decode file {id}"))?;
        ensure!(
            output.reader.len().map_err(|e| anyhow::anyhow!("{e}"))?
                <= context.limits.max_entry_size,
            "decoded file byte limit"
        );
        ensure!(
            output.extension.len() <= 16
                && output
                    .extension
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "invalid decoded-file extension"
        );
        Ok((id, output))
    }
}
struct BoundedProbe {
    source: Arc<dyn AssetReader>,
    remaining: AtomicU64,
}
impl AssetReader for BoundedProbe {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.source.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        self.remaining
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_sub(out.len() as u64)
            })
            .map_err(|_| crysirror_core::Error::Limit("file probe bytes"))?;
        self.source.read_at(offset, out)
    }
}
