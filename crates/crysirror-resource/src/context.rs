// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
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
    /// Maximum number of published entries per archive.
    pub max_entries: usize,
    /// Maximum logical bytes per decoded resource.
    pub max_entry_size: u64,
    /// Maximum bytes read or inflated for an index.
    pub max_index_size: u64,
    /// Retained decoded bytes shared by this client's profiles.
    pub max_resident_bytes: u64,
    /// Maximum read bytes per format probe.
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
thread_local! {
    // Resolving a mounted entry can synchronously decode another entry before
    // returning its reader. Bound that call graph independently of format keys.
    static DEPENDENCY_STACK: std::cell::RefCell<Vec<(ClientId, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}
struct DependencyGuard;
impl DependencyGuard {
    fn enter(client: ClientId, path: &str) -> Result<Self> {
        DEPENDENCY_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            ensure!(stack.len() < 16, "resource companion recursion limit");
            ensure!(
                !stack.iter().any(|(c, p)| *c == client && p == path),
                "resource companion dependency cycle"
            );
            stack.push((client, path.to_owned()));
            Ok(Self)
        })
    }
}
impl Drop for DependencyGuard {
    fn drop(&mut self) {
        DEPENDENCY_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
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
    /// Logical source name relative to this client's capability root.
    pub source_name: Option<String>,
    /// Explicit format-specific options. Secret bytes belong in KeyProvider;
    /// options may refer to key aliases, never ambient paths/device pointers.
    pub parameters: BTreeMap<String, serde_json::Value>,
    /// Scoped companion resources; workers capture the same immutable authority.
    pub resolver: Option<Arc<dyn crate::DependencyResolver>>,
    budget: Arc<AtomicU64>,
    cancellation: crysirror_tasks::Cancellation,
}
impl DecodeContext {
    pub fn new(client: ClientId) -> Self {
        Self {
            client,
            limits: Default::default(),
            transforms: Arc::new(TransformRegistry::default()),
            keys: Arc::new(MemoryKeyProvider::default()),
            pipelines: Arc::new(PipelineRules::default()),
            source_name: None,
            parameters: BTreeMap::new(),
            resolver: None,
            budget: Arc::new(AtomicU64::new(0)),
            cancellation: Default::default(),
        }
    }
    /// Attach a worker cancellation token to bounded CPU/stream decode loops.
    pub fn with_cancellation(mut self, cancellation: crysirror_tasks::Cancellation) -> Self {
        self.cancellation = cancellation;
        self
    }
    pub(crate) fn published(mut self) -> Self {
        self.cancellation = Default::default();
        self
    }
    /// Check cooperative cancellation without publishing partial resources.
    pub fn check_cancelled(&self) -> Result<()> {
        self.cancellation.check()
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
    pub fn open_dependency(&self, path: &str) -> Result<Arc<dyn AssetReader>> {
        let canonical =
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, path)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            canonical.relative() == path,
            "resource dependency path must be canonical"
        );
        let _dependency = DependencyGuard::enter(self.client, path)?;
        self.resolver
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!("format requires an explicit client-scoped companion resolver")
            })?
            .open(path)
    }
    pub fn open_sibling(&self, name: &str) -> Result<Arc<dyn AssetReader>> {
        ensure!(
            !name.is_empty()
                && !name.starts_with('/')
                && !name.contains(['\\', ':', '\0'])
                && name
                    .split('/')
                    .all(|s| !s.is_empty() && s != "." && s != ".."),
            "invalid companion resource name"
        );
        let source = self
            .source_name
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("companion resource requires a logical source_name"))?;
        let path = source
            .rsplit_once('/')
            .map_or_else(|| name.to_owned(), |(parent, _)| format!("{parent}/{name}"));
        self.open_dependency(&path)
    }
    pub fn memory(&self, bytes: Vec<u8>) -> Result<Arc<dyn AssetReader>> {
        self.memory_shared(Arc::from(bytes))
    }
    /// Charge an already shared immutable buffer without copying its contents.
    /// The returned reader keeps its charge until its last reference is dropped.
    pub fn memory_shared(&self, bytes: Arc<[u8]>) -> Result<Arc<dyn AssetReader>> {
        ensure!(
            bytes.len() as u64 <= self.limits.max_entry_size,
            "decoded entry byte limit"
        );
        let reservation = self.reserve(bytes.len() as u64)?;
        Ok(Arc::new(Resident {
            reader: MemoryAssetReader(bytes),
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
    /// Admit the maximum before running a variable-size codec, then retain
    /// only its actual output charge. The codec must enforce this bound while
    /// producing data, not allocate unbounded output and truncate afterward.
    pub fn decode_bounded(
        &self,
        limit: u64,
        decode: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<Arc<dyn AssetReader>> {
        ensure!(
            limit <= self.limits.max_entry_size && limit <= usize::MAX as u64,
            "decoded entry byte limit"
        );
        let mut reservation = self.reserve(limit)?;
        let bytes = decode()?;
        ensure!(
            bytes.len() as u64 <= limit,
            "codec exceeded declared output bound"
        );
        reservation.shrink(bytes.len() as u64);
        Ok(Arc::new(Resident {
            reader: MemoryAssetReader(Arc::from(bytes)),
            _reservation: reservation,
        }))
    }
    /// Admit transient codec working storage before allocation and release its
    /// charge on return/error. The closure must not let that allocation escape;
    /// retained output uses decode_with/decode_bounded instead. This accounts
    /// caller-declared storage, not all allocations inside third-party codecs.
    pub fn with_temporary_bytes<T>(
        &self,
        bytes: u64,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        ensure!(
            bytes <= usize::MAX as u64,
            "temporary codec allocation size"
        );
        let _reservation = self.reserve(bytes)?;
        work()
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
