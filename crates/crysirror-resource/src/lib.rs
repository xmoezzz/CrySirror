// SPDX-License-Identifier: MPL-2.0
//! Shared archive and transform contracts. No VM, window or device dependency.
#![forbid(unsafe_code)]
pub mod compound;
pub mod capability_lookup;
pub mod circus_pck;
mod context;
mod crypto;
mod jobs;
pub mod krkr_pbd;
pub mod signature;
pub mod sniff;
pub mod pe;
pub mod pe_menu;
pub mod xml;
pub mod sli;
pub mod text_stream;
mod wave;
pub use jobs::{ResourceResult, ResourceTasks, MAX_RESOURCE_JOBS, MAX_RESOURCE_JOBS_PER_CLIENT};
pub mod backref_lzss;
mod file;
pub mod io;
pub mod keys;
pub mod lzss;
pub mod registry;
pub mod transforms;

pub use context::{DecodeContext, DecodeLimits, PipelineRules, PipelineSelector};
pub use crypto::rsa_public_decrypt;
pub use crypto::{
    aes256_cbc_decrypt, aes_cbc_decrypt, blowfish_le_decrypt, md5_with_iv, rc4_crypt,
};
pub use file::{DecodedFile, DependencyResolver, FileFormat, FileKind, FileRegistry};
pub use io::{concat, read_all, read_exact_at, subrange, AssetIo};
pub use keys::{KeyMaterial, KeyProvider, MemoryKeyProvider};
pub use lzss::{lzss_decompress, LzssMode, LzssParams};
pub use registry::{ArchiveMount, FormatRegistry, Probe};
pub use transforms::{ByteTransform, TransformRegistry, TransformSpec};
pub use wave::{WaveLoop, WaveSpec};

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
    /// Optional authenticated lookup for formats whose directory stores hashes.
    /// Returning an ID never bypasses the mount capability or decode limits.
    fn resolve(&self, _path: &str) -> Option<u32> {
        None
    }
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
