// SPDX-License-Identifier: MPL-2.0
//! Shared archive and transform contracts. No VM, window or device dependency.
#![forbid(unsafe_code)]
pub mod compound;
pub mod dsc;
pub mod capability_lookup;
pub mod capability_tree;
pub mod circus_pck;
pub mod circus_voice;
pub mod exhibit_voice;
pub mod exhibit_pcm;
pub mod exhibit_dlt;
pub mod circus_crm;
pub mod circus_frames;
pub mod cg_catalog;
mod context;
mod crypto;
mod jobs;
pub mod krkr_pbd;
pub mod signature;
pub mod npf;
pub mod sniff;
pub mod pe;
pub mod checksum;
pub mod pe_menu;
pub mod private_profile;
pub mod xml;
pub mod sli;
pub mod text_stream;
mod wave;
pub use jobs::{ResourceResult, ResourceTasks, MAX_RESOURCE_JOBS, MAX_RESOURCE_JOBS_PER_CLIENT};
pub mod backref_lzss;
pub mod byte_pair;
pub mod byte_json;
pub mod byte_table;
pub mod record_sequence;
pub mod word_run_length;
pub mod sentinel_runs;
mod file;
pub mod io;
pub mod keys;
pub mod lzss;
pub mod lz4_frame;
pub mod legacy_stream;
pub mod lane_feedback;
pub mod installer;
pub mod registry;
pub mod redirect;
pub mod retained_assets;
pub mod transforms;
pub mod xor_recovery;

pub use context::{DecodeContext, DecodeLimits, PipelineRules, PipelineSelector};
pub use crypto::rsa_public_decrypt;
pub use crypto::{
    aes256_cbc_decrypt, aes_cbc_decrypt, blowfish_le_decrypt, blowfish_le_encrypt, md5_with_iv, rc4_crypt,
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
/// `metadata` holds facts the format stores as JSON; formats with large
/// indexes keep per-entry facts typed and publish them through
/// [`Archive::entry_metadata`], which is the complete view.
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
    /// Complete JSON facts of one entry. A JSON object per entry costs about
    /// 1-2 KiB of heap, so large indexes (tens of thousands of voice entries)
    /// keep their facts typed and build this view on request. The default
    /// returns the stored `ArchiveEntry::metadata`.
    fn entry_metadata(&self, entry_id: u32) -> serde_json::Value {
        self.entries()
            .iter()
            .find(|e| e.id == entry_id)
            .map(|e| e.metadata.clone())
            .unwrap_or_default()
    }
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

pub mod dos_glob;

pub mod bwef;
