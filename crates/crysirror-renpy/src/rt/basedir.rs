// SPDX-License-Identifier: MPL-2.0
//! Writable view of the game's base directory (`config.basedir`).
//!
//! Games write next to themselves with plain `open()`: DDLC records
//! `game/firstrun` and rewrites or deletes `characters/*.chr`, then probes
//! them with `renpy.file("../characters/…")`. The installed files stay
//! read-only; changes live in the client's Save mount as an overlay
//! (`basedir/<path>`), and deletions of installed files in the
//! `basedir.removed` list. Reads see overlay, then removal, then the
//! installed file.
use super::state::Files;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{cell::RefCell, collections::BTreeSet, sync::Arc};

const OVERLAY: &str = "basedir";
const REMOVED: &str = "basedir.removed";
const MAX_FILE: usize = 64 * 1024 * 1024;

/// A base-relative path ("game/firstrun", "characters/x.chr") of a
/// script-visible path, or None when it leaves the base directory.
pub fn relative(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split(['/', '\\']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub struct BaseDir {
    files: Arc<dyn Files>,
    removed: RefCell<Option<BTreeSet<String>>>,
}

impl BaseDir {
    pub fn new(files: Arc<dyn Files>) -> BaseDir {
        BaseDir { files, removed: RefCell::new(None) }
    }

    fn storage(&self) -> Option<&dyn ClientStorage> {
        self.files.temporary_storage()
    }

    fn overlay_path(rel: &str) -> Option<VfsPath> {
        VfsPath::new(Mount::Save, &format!("{OVERLAY}/{rel}")).ok()
    }

    fn with_removed<T>(&self, f: impl FnOnce(&mut BTreeSet<String>) -> T) -> T {
        let mut r = self.removed.borrow_mut();
        let set = r.get_or_insert_with(|| {
            let data = self
                .storage()
                .zip(VfsPath::new(Mount::Save, REMOVED).ok())
                .and_then(|(s, p)| s.read(&p).ok())
                .unwrap_or_default();
            String::from_utf8_lossy(&data).lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()
        });
        f(set)
    }

    fn save_removed(&self) -> bool {
        let text = self.with_removed(|s| s.iter().map(|l| format!("{l}\n")).collect::<String>());
        let Some((s, p)) = self.storage().zip(VfsPath::new(Mount::Save, REMOVED).ok()) else { return false };
        if text.is_empty() { s.remove(&p).is_ok() || !s.exists(&p) } else { s.write_atomic(&p, text.as_bytes()).is_ok() }
    }

    fn overlay_meta(&self, rel: &str) -> Option<bool> {
        let s = self.storage()?;
        s.metadata(Mount::Save, &format!("{OVERLAY}/{rel}")).ok()?.map(|m| m.is_directory)
    }

    /// The installed file or directory (None: missing; Some(true): directory).
    fn installed_meta(&self, rel: &str) -> Option<bool> {
        if let Some(g) = rel.strip_prefix("game/") {
            if self.files.list().iter().any(|n| n == g) {
                return Some(false);
            }
        }
        let s = self.storage()?;
        s.metadata(Mount::Asset, rel).ok()?.map(|m| m.is_directory)
    }

    fn installed_read(&self, rel: &str) -> Option<Vec<u8>> {
        if let Some(g) = rel.strip_prefix("game/")
            && let Some(d) = self.files.read(g)
        {
            return Some(d);
        }
        let s = self.storage()?;
        let m = s.metadata(Mount::Asset, rel).ok()??;
        if m.is_directory || m.size.is_some_and(|z| z > MAX_FILE as u64) {
            return None;
        }
        s.read(&VfsPath::new(Mount::Asset, rel).ok()?).ok()
    }

    fn is_removed(&self, rel: &str) -> bool {
        self.with_removed(|s| s.contains(rel))
    }

    /// Whether `rel` has a base-directory change (written or removed), so
    /// the loader's index of installed files does not decide it.
    pub fn changed(&self, rel: &str) -> bool {
        self.is_removed(rel) || self.overlay_meta(rel) == Some(false)
    }

    pub fn read(&self, rel: &str) -> Option<Vec<u8>> {
        if self.overlay_meta(rel) == Some(false) {
            return self.storage()?.read(&Self::overlay_path(rel)?).ok();
        }
        if self.is_removed(rel) {
            return None;
        }
        self.installed_read(rel)
    }

    pub fn stat(&self, rel: &str) -> Option<bool> {
        if let Some(d) = self.overlay_meta(rel) {
            return Some(d);
        }
        if self.is_removed(rel) {
            return None;
        }
        self.installed_meta(rel)
    }

    pub fn write(&self, rel: &str, data: &[u8]) -> bool {
        let (Some(s), Some(p)) = (self.storage(), Self::overlay_path(rel)) else { return false };
        if s.write_atomic(&p, data).is_err() {
            return false;
        }
        if self.with_removed(|r| r.remove(rel)) {
            return self.save_removed();
        }
        true
    }

    pub fn remove(&self, rel: &str) -> bool {
        let overlay = self.overlay_meta(rel) == Some(false);
        if overlay {
            let (Some(s), Some(p)) = (self.storage(), Self::overlay_path(rel)) else { return false };
            if s.remove(&p).is_err() {
                return false;
            }
        }
        if !self.is_removed(rel) && self.installed_meta(rel) == Some(false) {
            self.with_removed(|r| r.insert(rel.to_owned()));
            return self.save_removed();
        }
        overlay
    }

    pub fn list_dir(&self, rel: &str) -> Option<Vec<String>> {
        let mut names = BTreeSet::new();
        let mut found = false;
        if let Some(s) = self.storage() {
            let dir = if rel.is_empty() { OVERLAY.to_owned() } else { format!("{OVERLAY}/{rel}") };
            if let Ok(l) = s.list(Mount::Save, &dir) {
                found |= self.overlay_meta(rel) == Some(true) || rel.is_empty();
                names.extend(l);
            }
            let installed = if rel.is_empty() { s.list(Mount::Asset, "") } else { s.list(Mount::Asset, rel) };
            if let Ok(l) = installed {
                found = true;
                let prefix = if rel.is_empty() { String::new() } else { format!("{rel}/") };
                names.extend(l.into_iter().filter(|n| !self.is_removed(&format!("{prefix}{n}"))));
            }
        }
        found.then(|| names.into_iter().collect())
    }
}
