// SPDX-License-Identifier: MPL-2.0
use crate::{ArchiveEntry, KeyProvider, MemoryKeyProvider, TransformRegistry, TransformSpec};
use anyhow::{ensure, Result};
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DecodeLimits {
    pub max_entries: usize,
    pub max_entry_size: u64,
    pub max_index_size: u64,
    pub max_resident_bytes: u64,
    pub max_probe_bytes: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_entry_size: 64 * 1024 * 1024,
            max_index_size: 16 * 1024 * 1024,
            max_resident_bytes: 256 * 1024 * 1024,
            max_probe_bytes: 1024 * 1024,
        }
    }
}
pub trait PipelineSelector: Send + Sync {
    fn pipeline(&self, format: &str, entry: &ArchiveEntry) -> Result<Vec<TransformSpec>>;
}
#[derive(Default)]
pub struct PipelineRules {
    pub rules: BTreeMap<(String, Option<String>), Vec<TransformSpec>>,
}
impl PipelineSelector for PipelineRules {
    fn pipeline(&self, format: &str, entry: &ArchiveEntry) -> Result<Vec<TransformSpec>> {
        Ok(self
            .rules
            .get(&(format.into(), Some(entry.path.clone())))
            .or_else(|| self.rules.get(&(format.into(), None)))
            .cloned()
            .unwrap_or_default())
    }
}
#[derive(Clone)]
pub struct DecodeContext {
    pub client: ClientId,
    pub limits: DecodeLimits,
    pub transforms: Arc<TransformRegistry>,
    pub keys: Arc<dyn KeyProvider>,
    pub pipelines: Arc<dyn PipelineSelector>,
    budget: Arc<AtomicU64>,
}
impl DecodeContext {
    pub fn new(client: ClientId) -> Self {
        Self {
            client,
            limits: Default::default(),
            transforms: Arc::new(TransformRegistry::default()),
            keys: Arc::new(MemoryKeyProvider::default()),
            pipelines: Arc::new(PipelineRules::default()),
            budget: Arc::new(AtomicU64::new(0)),
        }
    }
    pub fn pipeline(&self, format: &str, entry: &ArchiveEntry) -> Result<Vec<TransformSpec>> {
        self.pipelines.pipeline(format, entry)
    }
    pub fn apply(
        &self,
        spec: &TransformSpec,
        input: Arc<dyn AssetReader>,
        metadata: &serde_json::Value,
    ) -> Result<Arc<dyn AssetReader>> {
        let out = self.transforms.apply(spec, input, metadata, self)?;
        if let Some(n) = spec.expected_size {
            ensure!(
                out.len().map_err(|e| anyhow::anyhow!("{e}"))? == n,
                "transform output size mismatch"
            );
        }
        Ok(out)
    }
    pub fn apply_pipeline(
        &self,
        format: &str,
        entry: &ArchiveEntry,
        mut input: Arc<dyn AssetReader>,
        required: bool,
    ) -> Result<Arc<dyn AssetReader>> {
        let pipeline = self.pipeline(format, entry)?;
        ensure!(
            !required || !pipeline.is_empty(),
            "encrypted entry needs an explicit transform/key profile: {format}:{}",
            entry.path
        );
        for spec in pipeline {
            input = self.apply(&spec, input, &entry.metadata)?;
        }
        ensure!(
            input.len().map_err(|e| anyhow::anyhow!("{e}"))? == entry.size,
            "entry decoded size mismatch"
        );
        Ok(input)
    }
    pub fn resident_bytes(&self) -> u64 {
        self.budget.load(Ordering::Relaxed)
    }
    pub fn memory(&self, bytes: Vec<u8>) -> Result<Arc<dyn AssetReader>> {
        ensure!(
            bytes.len() as u64 <= self.limits.max_entry_size,
            "decoded entry byte limit"
        );
        let reservation = self.reserve(bytes.len() as u64)?;
        Ok(Arc::new(Resident {
            reader: MemoryAssetReader(Arc::from(bytes)),
            _reservation: reservation,
        }))
    }
    /// Reserve a declared decoded size before invoking a format-specific codec.
    /// The closure must itself obey that output size and reject malformed data.
    pub fn decode_with(
        &self,
        size: u64,
        decode: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<Arc<dyn AssetReader>> {
        ensure!(
            size <= self.limits.max_entry_size && size <= usize::MAX as u64,
            "decoded entry byte limit"
        );
        let reservation = self.reserve(size)?;
        let bytes = decode()?;
        ensure!(bytes.len() as u64 == size, "codec decoded size mismatch");
        Ok(Arc::new(Resident {
            reader: MemoryAssetReader(Arc::from(bytes)),
            _reservation: reservation,
        }))
    }
    pub(crate) fn reserve(&self, bytes: u64) -> Result<Reservation> {
        let limit = self.limits.max_resident_bytes;
        ensure!(
            self.budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| old
                    .checked_add(bytes)
                    .filter(|new| *new <= limit))
                .is_ok(),
            "resident decoded-resource budget exceeded"
        );
        Ok(Reservation {
            budget: self.budget.clone(),
            bytes,
        })
    }
}
pub(crate) struct Reservation {
    budget: Arc<AtomicU64>,
    bytes: u64,
}
impl Reservation {
    pub(crate) fn shrink(&mut self, bytes: u64) {
        if bytes < self.bytes {
            self.budget.fetch_sub(self.bytes - bytes, Ordering::Relaxed);
            self.bytes = bytes;
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
pub(crate) struct Resident {
    pub reader: MemoryAssetReader,
    pub _reservation: Reservation,
}
impl AssetReader for Resident {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.reader.len()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        self.reader.read_at(offset, out)
    }
}
