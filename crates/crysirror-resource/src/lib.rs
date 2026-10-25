// SPDX-License-Identifier: MPL-2.0
//! Shared archive and transform contracts. No VM, window or device dependency.
#![forbid(unsafe_code)]
mod context;
pub mod io;
pub mod keys;
pub mod registry;
pub mod transforms;

pub use context::{DecodeContext, DecodeLimits, PipelineRules, PipelineSelector};
pub use io::{concat, read_all, read_exact_at, subrange, AssetIo};
pub use keys::{KeyMaterial, KeyProvider, MemoryKeyProvider};
pub use registry::{ArchiveMount, FormatRegistry, Probe};
pub use transforms::{ByteTransform, TransformRegistry, TransformSpec};

use anyhow::Result;
use crysirror_core::service::AssetReader;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Public entry metadata. Native offsets, segment plans and filename codecs
/// remain inside a format plugin; no native executable pointers are exposed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveEntry {
    pub id: u32,
    pub path: String,
    pub size: u64,
    pub stored_size: u64,
    #[serde(default)]
    pub metadata: serde_json::Value,
}
pub trait Archive: Send + Sync {
    fn entries(&self) -> &[ArchiveEntry];
    fn open(&self, entry_id: u32, context: &DecodeContext) -> Result<Arc<dyn AssetReader>>;
}
pub trait ArchiveFormat: Send + Sync {
    fn id(&self) -> &'static str;
    fn probe(&self, source: &dyn AssetReader) -> Result<Probe>;
    fn open(
        &self,
        source: Arc<dyn AssetReader>,
        context: &DecodeContext,
    ) -> Result<Box<dyn Archive>>;
}
