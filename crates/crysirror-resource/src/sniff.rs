// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Content identification from bounded capability prefixes, independent of VMs.
//!
//! The embedded pure-Rust magic database is shared read-only after one validated
//! initialization. There is no libmagic FFI, filename hint, file/mmap reader or
//! external rules file in this interface. Identification is evidence about a
//! sampled prefix, not structural validation or a promise that a decoder exists.
//! Call this synchronous CPU operation from a resource/boot worker, not a draw loop.

use anyhow::{Context, Result, ensure};
use crysirror_core::service::AssetReader;
use pure_magic::MagicDb;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Maximum input inspected by one identification call, irrespective of file size.
pub const MAX_FILE_PREFIX_BYTES: usize = 64 * 1024;

/// A positive content-rule match. Generic text/data/empty fallback is `None`.
/// A match never establishes complete file validity or engine decoder support.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    /// Content MIME from the matched rule; may remain application/octet-stream
    /// for a descriptive rule without a more specific MIME declaration.
    pub mime_type: String,
    /// Human-readable description produced by the rule database.
    pub description: String,
    /// Rule origin in the embedded database, when recorded by that database.
    pub rule_source: Option<String>,
    /// Database-specific ranking score, not a probability or validation result.
    pub strength: u64,
    /// Input bytes available to the matching rules, never exceeding 64 KiB.
    pub bytes_examined: u32,
    /// The original input was longer than the inspected prefix. Rules requiring
    /// data outside that prefix cannot establish an identity through this API.
    pub prefix_truncated: bool,
}

/// Cloneable reader-independent detector; clones share the validated database.
#[derive(Clone)]
pub struct FileDetector {
    db: Arc<MagicDb>,
}

impl std::fmt::Debug for FileDetector {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("FileDetector")
            .field("rules", &self.rule_count())
            .field("max_prefix_bytes", &MAX_FILE_PREFIX_BYTES)
            .finish()
    }
}

// Cache either the immutable rules or their initialization failure. Each caller
// gets an owned detector/error without repeated database decoding or a poisoned
// mutable singleton. No asset, VM, file or task handles enter this global cache.
static DATABASE: OnceLock<std::result::Result<Arc<MagicDb>, String>> = OnceLock::new();

fn load_database() -> Result<Arc<MagicDb>> {
    let mut db = magic_db::load().context("load embedded Rust magic database")?;
    db.verify().context("verify embedded Rust magic rules")?;
    ensure!(
        db.rules().len() >= 100,
        "embedded magic rule inventory is incomplete"
    );
    // Self-authored PNG IHDR prefix checks the pinned peer's rule dispatch and
    // MIME metadata as well as successful deserialization. It is not a decoder.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";
    let probe = db.first_magic_slice(PNG, None)?;
    ensure!(
        !probe.is_default() && probe.mime_type() == "image/png",
        "embedded magic database failed its PNG rule check"
    );
    Ok(Arc::new(db))
}

impl FileDetector {
    /// Initialize or acquire the process-wide immutable embedded rule database.
    /// Initialization/verification failures remain visible to every caller.
    pub fn new() -> Result<Self> {
        let result = DATABASE.get_or_init(|| load_database().map_err(|e| format!("{e:#}")));
        let db = result.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self { db: db.clone() })
    }

    /// Number of loaded top-level rules (not a count of supported file formats).
    pub fn rule_count(&self) -> usize {
        self.db.rules().len()
    }

    /// Read at most 64 KiB from a caller-owned capability, handling partial reads.
    /// A premature zero read, invalid read count or changed source length fails;
    /// missing bytes are never padded or mistaken for an intentionally short file.
    /// This cannot authenticate an input or detect arbitrary same-size mutation.
    pub fn identify(&self, source: &dyn AssetReader) -> Result<Option<FileIdentity>> {
        let length = source.len().context("file identification source length")?;
        let count = length.min(MAX_FILE_PREFIX_BYTES as u64) as usize;
        let mut prefix = vec![0; count];
        crate::read_exact_at(source, 0, &mut prefix)?;
        ensure!(
            source
                .len()
                .context("file identification source length after read")?
                == length,
            "file identification source length changed during prefix read"
        );
        self.identify_prefix(&prefix, length > count as u64)
    }

    /// Identify a caller-owned byte slice by its first 64 KiB. No filename or
    /// extension can affect rule selection; larger input is explicitly reported.
    pub fn identify_bytes(&self, bytes: &[u8]) -> Result<Option<FileIdentity>> {
        let count = bytes.len().min(MAX_FILE_PREFIX_BYTES);
        self.identify_prefix(&bytes[..count], bytes.len() > count)
    }

    fn identify_prefix(
        &self,
        bytes: &[u8],
        prefix_truncated: bool,
    ) -> Result<Option<FileIdentity>> {
        if bytes.is_empty() {
            return Ok(None);
        }
        let identity = self
            .db
            .first_magic_slice(bytes, None)
            .context("identify resource prefix using embedded Rust magic rules")?;
        if identity.is_default() {
            return Ok(None);
        }
        Ok(Some(FileIdentity {
            mime_type: identity.mime_type().to_owned(),
            description: identity.message(),
            rule_source: identity.source().map(str::to_owned),
            strength: identity.strength(),
            bytes_examined: bytes.len() as u32,
            prefix_truncated,
        }))
    }
}
