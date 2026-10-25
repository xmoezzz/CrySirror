// SPDX-License-Identifier: MPL-2.0
//! Read-only filesystem providers mounted under a client's asset namespace.
//! A provider sees only paths relative to its mount point; it never receives a
//! host path or another client's identity. Native engines (Siglus, RFVP) keep
//! their own archive/name semantics by supplying a provider or by decoding
//! through these reads; the host never reinterprets their formats.
use crate::{path::VfsPath, Error, Result};
use alloc::{collections::BTreeMap, string::String, sync::Arc, vec::Vec};
use crate::service::{AssetReader, MemoryAssetReader};

pub trait VfsProvider: Send + Sync {
    /// Short diagnostic kind, e.g. "memory" or "siglus-pack".
    fn kind(&self) -> &'static str;
    /// `relative` is canonical (validated by [`VfsPath`]) and mount-relative.
    fn read(&self, relative: &str) -> Result<Vec<u8>>;
    /// Open random-access data without materializing an entire archive entry.
    /// Legacy providers retain a bounded in-memory fallback.
    fn open(&self, relative: &str) -> Result<Arc<dyn AssetReader>> {
        let bytes = self.read(relative)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(Error::Limit("provider file size"));
        }
        Ok(Arc::new(MemoryAssetReader(Arc::from(bytes))))
    }
    fn exists(&self, relative: &str) -> bool {
        self.read(relative).is_ok()
    }
    /// Bounded listing for inspection tools; `Unsupported` when unknowable.
    fn list(&self) -> Result<Vec<String>> {
        Err(Error::Unsupported("provider listing"))
    }
}

/// An immutable in-memory tree, useful for generated, embedded or test assets.
#[derive(Clone, Debug, Default)]
pub struct MemoryProvider {
    files: BTreeMap<String, Vec<u8>>,
}
impl MemoryProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, relative: &str, bytes: Vec<u8>) -> Result<()> {
        let p = VfsPath::new(crate::path::Mount::Asset, relative)?;
        self.files.insert(p.relative().into(), bytes);
        Ok(())
    }
}
impl VfsProvider for MemoryProvider {
    fn kind(&self) -> &'static str {
        "memory"
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>> {
        self.files.get(relative).cloned().ok_or(Error::NotFound)
    }
    fn exists(&self, relative: &str) -> bool {
        self.files.contains_key(relative)
    }
    fn list(&self) -> Result<Vec<String>> {
        Ok(self.files.keys().cloned().collect())
    }
}

/// Splits a canonical asset-relative path at a mount point. `""` mounts at root.
pub fn strip_mount<'a>(mount_point: &str, relative: &'a str) -> Option<&'a str> {
    if mount_point.is_empty() {
        return Some(relative);
    }
    relative
        .strip_prefix(mount_point)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mount_prefix_is_component_exact() {
        assert_eq!(strip_mount("pack", "pack/a.g00"), Some("a.g00"));
        assert_eq!(strip_mount("pack", "package/a.g00"), None);
        assert_eq!(strip_mount("pack", "pack"), None);
        assert_eq!(strip_mount("", "x/y"), Some("x/y"));
    }
    #[test]
    fn memory_provider_canonicalizes_and_rejects_traversal() {
        let mut m = MemoryProvider::new();
        m.insert("a/./b.txt", b"x".to_vec()).unwrap();
        assert_eq!(m.read("a/b.txt").unwrap(), b"x");
        assert!(m.insert("../escape", Vec::new()).is_err());
        assert_eq!(m.read("missing"), Err(Error::NotFound));
    }
}
