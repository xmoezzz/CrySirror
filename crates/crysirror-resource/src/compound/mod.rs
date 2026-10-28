// SPDX-License-Identifier: MPL-2.0
//! Compound storage media: hashed-name domains routed onto mounted containers.
//!
//! A media owns numbered archive mounts, named archive domains and an ordered
//! routing table from logical domains (`"."` is the media root) to archive
//! domains. Lookups produce container/inner-path candidates, newest route first;
//! bytes are still read through the caller's scoped VFS. No VM is involved.
//!
//! Protocol review: cxdec-hxv4-static-analysis at
//! 3c6dee001df011fa7ed557c7463635c6f67698f6 (`docs/core/hxv4/03-resource-hash.md`,
//! `sample/startup.tjs`). File-level routing remaps only directory hashes this
//! media produced, so every remap has an exact source spelling.
mod hash;
pub use hash::{file_hash, path_hash};

use anyhow::{bail, ensure, Context, Result};
use std::collections::BTreeMap;

/// Upper bounds of one media's mutable state.
pub const MAX_MOUNTS: usize = 256;
pub const MAX_ROUTES: usize = 4096;
const MAX_KNOWN_HASHES: usize = 65_536;
const MAX_NAME: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    Domain {
        domain: String,
        target: String,
    },
    Files {
        domain: String,
        source: [u8; 8],
        target: String,
        target_hash: [u8; 8],
    },
}

#[derive(Debug)]
pub struct CompoundStorage {
    media: String,
    key: Vec<u8>,
    mounts: Vec<Option<String>>,
    domains: BTreeMap<String, usize>,
    known: BTreeMap<[u8; 8], String>,
    routes: Vec<Route>,
}

/// One routed lookup: read `inner` from the container mounted from `container`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub container: String,
    pub inner: String,
}

fn name(value: &str, what: &str) -> Result<()> {
    ensure!(
        value.len() <= MAX_NAME && !value.contains('\0'),
        "compound storage {what} name"
    );
    Ok(())
}

/// Split a media-relative path into its directory (no trailing slash) and name.
fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

impl CompoundStorage {
    /// `media` is the hash salt (`mediaName`), `key` the bootstrap hash key.
    pub fn new(media: &str, key: &[u8]) -> Result<Self> {
        ensure!(!media.is_empty(), "compound storage media name required");
        name(media, "media")?;
        ensure!(key.len() <= 64, "compound storage hash key length");
        Ok(Self {
            media: media.into(),
            key: key.into(),
            mounts: Vec::new(),
            domains: BTreeMap::new(),
            known: BTreeMap::new(),
            routes: Vec::new(),
        })
    }
    pub fn media(&self) -> &str {
        &self.media
    }
    pub fn key(&self) -> &[u8] {
        &self.key
    }
    /// Hash of exactly `path`; remembered so later file routes keep its spelling.
    pub fn path_hash(&mut self, path: &str) -> Result<[u8; 8]> {
        name(path, "path")?;
        let hash = path_hash(path, &self.media);
        if !self.known.contains_key(&hash) {
            ensure!(
                self.known.len() < MAX_KNOWN_HASHES,
                "compound storage hash table limit"
            );
            self.known.insert(hash, path.into());
        }
        Ok(hash)
    }
    pub fn file_hash(&self, name: &str) -> [u8; 32] {
        file_hash(name, &self.media)
    }
    /// Mount an already-opened container and return its stable index.
    pub fn mount(&mut self, container: &str) -> Result<usize> {
        name(container, "container")?;
        ensure!(!container.is_empty(), "compound storage container required");
        if let Some(i) = self
            .mounts
            .iter()
            .position(|m| m.as_deref() == Some(container))
        {
            return Ok(i);
        }
        let slot = self.mounts.iter().position(Option::is_none);
        let i = match slot {
            Some(i) => i,
            None => {
                ensure!(self.mounts.len() < MAX_MOUNTS, "compound storage mount limit");
                self.mounts.push(None);
                self.mounts.len() - 1
            }
        };
        self.mounts[i] = Some(container.into());
        Ok(i)
    }
    pub fn mounted(&self, index: usize) -> Option<&str> {
        self.mounts.get(index)?.as_deref()
    }
    pub fn mount_index(&self, container: &str) -> Option<usize> {
        self.mounts
            .iter()
            .position(|m| m.as_deref() == Some(container))
    }
    /// Unmount drops the domains naming this mount and every route through them.
    pub fn unmount(&mut self, index: usize) -> Result<()> {
        let slot = self
            .mounts
            .get_mut(index)
            .filter(|m| m.is_some())
            .context("compound storage mount index not mounted")?;
        *slot = None;
        let gone: Vec<String> = self
            .domains
            .iter()
            .filter(|(_, m)| **m == index)
            .map(|(d, _)| d.clone())
            .collect();
        for d in gone {
            self.domains.remove(&d);
            self.routes.retain(|r| match r {
                Route::Domain { domain, .. } | Route::Files { domain, .. } => *domain != d,
            });
        }
        Ok(())
    }
    pub fn unmount_all(&mut self) {
        self.mounts.clear();
        self.domains.clear();
        self.routes.clear();
    }
    /// Name the entries of mount `index` as archive domain `domain`.
    pub fn entry_domain(&mut self, domain: &str, index: usize) -> Result<()> {
        name(domain, "domain")?;
        ensure!(self.mounted(index).is_some(), "compound storage domain mount");
        if let Some(previous) = self.domains.get(domain) {
            ensure!(*previous == index, "compound storage domain already bound");
        }
        self.domains.insert(domain.into(), index);
        Ok(())
    }
    fn push(&mut self, route: Route) -> Result<()> {
        self.routes.retain(|r| *r != route);
        ensure!(self.routes.len() < MAX_ROUTES, "compound storage route limit");
        self.routes.push(route);
        Ok(())
    }
    /// Expose a whole archive domain under logical domain `target`.
    pub fn assign_domain(&mut self, domain: &str, target: &str) -> Result<()> {
        name(target, "target")?;
        ensure!(self.domains.contains_key(domain), "compound storage unknown domain");
        self.push(Route::Domain {
            domain: domain.into(),
            target: target.into(),
        })
    }
    /// Expose files of `domain` in directory `source` as directory `target_hash`
    /// of logical domain `target`. Both hashes must come from [`Self::path_hash`].
    pub fn assign_files(
        &mut self,
        domain: &str,
        source: [u8; 8],
        target: &str,
        target_hash: [u8; 8],
    ) -> Result<()> {
        name(target, "target")?;
        ensure!(self.domains.contains_key(domain), "compound storage unknown domain");
        ensure!(
            self.known.contains_key(&source) && self.known.contains_key(&target_hash),
            "compound storage file route uses a hash this media did not produce"
        );
        self.push(Route::Files {
            domain: domain.into(),
            source,
            target: target.into(),
            target_hash,
        })
    }
    /// Remove every route into logical domain `target`.
    pub fn clear_domain(&mut self, target: &str) {
        self.routes.retain(|r| match r {
            Route::Domain { target: t, .. } | Route::Files { target: t, .. } => t != target,
        });
    }
    pub fn clear_all(&mut self) {
        self.routes.clear();
    }
    /// Lookup candidates for `path` inside logical domain `target`, newest first.
    pub fn candidates(&self, target: &str, path: &str) -> Vec<Candidate> {
        let path = path.trim_start_matches('/');
        let (dir, file) = split(path);
        // A directory may be spelled with or without its trailing delimiter.
        let dir_hashes = [
            path_hash(dir, &self.media),
            path_hash(&format!("{dir}/"), &self.media),
        ];
        let mut out = Vec::new();
        for route in self.routes.iter().rev() {
            let (domain, inner) = match route {
                Route::Domain { domain, target: t } if t == target => (domain, path.to_owned()),
                Route::Files {
                    domain,
                    source,
                    target: t,
                    target_hash,
                } if t == target && dir_hashes.contains(target_hash) => {
                    let Some(source) = self.known.get(source) else {
                        continue;
                    };
                    let source = source.trim_end_matches('/');
                    let inner = if source.is_empty() {
                        file.to_owned()
                    } else {
                        format!("{source}/{file}")
                    };
                    (domain, inner)
                }
                _ => continue,
            };
            let Some(container) = self
                .domains
                .get(domain)
                .and_then(|m| self.mounted(*m))
            else {
                continue;
            };
            let candidate = Candidate {
                container: container.into(),
                inner,
            };
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        }
        out
    }
    /// A known spelling of a hash produced by this media.
    pub fn spelling(&self, hash: &[u8; 8]) -> Option<&str> {
        self.known.get(hash).map(String::as_str)
    }
    pub fn domain_names(&self) -> Vec<String> {
        self.domains.keys().cloned().collect()
    }
    pub fn require_domain(&self, domain: &str) -> Result<()> {
        if !self.domains.contains_key(domain) {
            bail!("compound storage unknown domain `{domain}`");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_domains_and_files_newest_first() {
        let mut s = CompoundStorage::new("xp3hnp", &[0; 32]).unwrap();
        let root = s.mount("data.xp3").unwrap();
        let patch = s.mount("patch.xp3").unwrap();
        assert_eq!((root, patch), (0, 1));
        assert_eq!(s.mount("data.xp3").unwrap(), 0);
        s.entry_domain("0", root).unwrap();
        s.entry_domain("1", patch).unwrap();
        s.assign_domain("0", ".").unwrap();
        let zpath = s.path_hash("").unwrap();
        let sub = s.path_hash("sub/").unwrap();
        s.assign_files("1", sub, ".", zpath).unwrap();
        assert_eq!(
            s.candidates(".", "a.tjs"),
            vec![
                Candidate { container: "patch.xp3".into(), inner: "sub/a.tjs".into() },
                Candidate { container: "data.xp3".into(), inner: "a.tjs".into() },
            ]
        );
        assert_eq!(s.candidates(".", "x/a.tjs").len(), 1);
        assert!(s.assign_files("1", [9; 8], ".", zpath).is_err());
        assert!(s.assign_domain("7", ".").is_err());
        s.unmount(patch).unwrap();
        assert_eq!(s.candidates(".", "a.tjs").len(), 1);
        assert!(s.unmount(patch).is_err());
        s.clear_domain(".");
        assert!(s.candidates(".", "a.tjs").is_empty());
    }
    #[test]
    fn path_hash_is_little_endian_over_exact_spelling() {
        // Documented root value (03-resource-hash.md) and directory-with-slash
        // spelling observed in an authenticated archive's directory table.
        assert_eq!(path_hash("", "xp3hnp"), [0x94, 0xd4, 0xa9, 0x7c, 0x61, 0x49, 0x86, 0x21]);
        assert_eq!(path_hash("system/", "xp3hnp"), [0xbc, 0x67, 0x92, 0xf8, 0x31, 0xd5, 0xfb, 0xd5]);
        assert_ne!(path_hash("system", "xp3hnp"), path_hash("system/", "xp3hnp"));
        assert_ne!(path_hash("", "xp3hnp"), path_hash("", "other"));
    }
}
