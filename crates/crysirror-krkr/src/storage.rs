// SPDX-License-Identifier: AGPL-3.0-or-later
//! KiriKiri storage naming over the client's engine-owned capability storage.
//!
//! KiriKiri resolves bare storage names through an ordered "auto path" list
//! with case-insensitive matching. Only that naming policy lives here; bytes
//! come from the shared `ClientStorage` handle and images are decoded by the
//! engine's shared decoders.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{collections::BTreeMap, sync::Arc};

/// Upper bound on indexed files; a project larger than this fails visibly.
const MAX_FILES: usize = 65_536;

pub struct Storage {
    files: Arc<dyn ClientStorage>,
    root: String,
    /// Lowercased path relative to `root` → provider spelling relative to the
    /// asset mount.
    index: BTreeMap<String, String>,
    /// Lowercased auto-path directories, searched after a direct match.
    auto: Vec<String>,
}

impl Storage {
    pub fn new(files: Arc<dyn ClientStorage>, root: &str) -> Result<Self> {
        let mut this = Self {
            files,
            root: root.trim_matches('/').to_owned(),
            index: BTreeMap::new(),
            auto: Vec::new(),
        };
        let mut stack = vec![this.root.clone()];
        while let Some(dir) = stack.pop() {
            for name in this.files.list(Mount::Asset, &dir).map_err(err)? {
                let full = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                };
                // Providers list directories and files alike; a directory has
                // children, a file reads. Probe by listing first.
                match this.files.list(Mount::Asset, &full) {
                    Ok(children) if !children.is_empty() => stack.push(full),
                    _ => {
                        let relative = full
                            .strip_prefix(&this.root)
                            .unwrap_or(&full)
                            .trim_start_matches('/')
                            .to_lowercase();
                        this.index.insert(relative, full);
                        ensure!(
                            this.index.len() <= MAX_FILES,
                            "KiriKiri storage index limit"
                        );
                    }
                }
            }
        }
        Ok(this)
    }

    /// `Storages.addAutoPath`: a directory (trailing `/`) searched for bare names.
    /// Archive auto paths (`foo.xp3>`) are not mounted by this adapter yet.
    pub fn add_auto_path(&mut self, path: &str) -> Result<()> {
        ensure!(
            !path.contains('>'),
            "KiriKiri archive auto path `{path}` requires an engine archive mount"
        );
        let dir = normalize(path).trim_end_matches('/').to_lowercase();
        if !self.auto.contains(&dir) {
            self.auto.push(dir);
        }
        Ok(())
    }

    pub fn remove_auto_path(&mut self, path: &str) {
        let dir = normalize(path).trim_end_matches('/').to_lowercase();
        self.auto.retain(|d| *d != dir);
    }

    /// Resolve a storage name to an asset-relative path, or `None`.
    pub fn resolve(&self, name: &str) -> Option<String> {
        if let Ok(Some(path)) = save_path(name) {
            return self
                .files
                .exists(&path)
                .then(|| format!("savedata/{}", path.relative()));
        }
        let key = normalize(name).to_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Some(found) = self.index.get(&key) {
            return Some(found.clone());
        }
        let base = key.rsplit('/').next().unwrap_or(&key);
        for dir in self.auto.iter().rev() {
            let candidate = if dir.is_empty() {
                base.to_owned()
            } else {
                format!("{dir}/{base}")
            };
            if let Some(found) = self.index.get(&candidate) {
                return Some(found.clone());
            }
        }
        None
    }

    /// Resolve trying KiriKiri's implicit extensions in order.
    pub fn resolve_with(&self, name: &str, extensions: &[&str]) -> Option<String> {
        if let Some(found) = self.resolve(name) {
            return Some(found);
        }
        extensions
            .iter()
            .find_map(|ext| self.resolve(&format!("{name}{ext}")))
    }

    pub fn begin_write(
        &self,
        name: &str,
        data: std::sync::Arc<[u8]>,
    ) -> Result<Box<dyn crysirror_core::service::StorageWrite>> {
        let path =
            save_path(name)?.context("writes require the virtual savedata/ capability root")?;
        self.files.begin_write_atomic(&path, data).map_err(err)
    }
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        if let Some(path) = save_path(path)? {
            return self.files.read(&path).map_err(err);
        }
        self.files
            .read(&VfsPath::new(Mount::Asset, path).map_err(err)?)
            .map_err(err)
            .with_context(|| format!("read storage `{path}`"))
    }

    pub fn read_named(&self, name: &str) -> Result<Vec<u8>> {
        if let Some(path) = save_path(name)? {
            return self.files.read(&path).map_err(err);
        }
        let path = self
            .resolve(name)
            .with_context(|| format!("storage `{name}` not found"))?;
        self.read(&path)
    }
}

/// Only this virtual prefix grants native writes; it never denotes an OS path.
pub fn save_path(name: &str) -> Result<Option<VfsPath>> {
    let normalized = normalize(name);
    let Some(relative) = normalized.strip_prefix("savedata/") else {
        return Ok(None);
    };
    let relative = relative
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/");
    Ok(Some(VfsPath::new(Mount::Save, &relative).map_err(err)?))
}

/// Strip KiriKiri's `file://./` and archive prefixes into a slash path.
pub fn normalize(name: &str) -> String {
    let mut s = name.replace('\\', "/");
    for prefix in ["file://./", "file://", "./"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
        }
    }
    s.trim_start_matches('/').to_owned()
}

/// Decode a script text the way KiriKiri does: a UTF-16LE/UTF-8 BOM wins,
/// then valid UTF-8, then Shift_JIS (the platform default of most titles).
pub fn decode_text(bytes: &[u8]) -> Result<String> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        ensure!(rest.len() % 2 == 0, "odd UTF-16LE script length");
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).context("invalid UTF-16LE script");
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).context("invalid UTF-8 script");
    }
    ensure!(
        !bytes.starts_with(b"TJS2"),
        "compiled TJS2 bytecode must be executed, not decoded as text"
    );
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    let (text, _, errors) = encoding_rs::SHIFT_JIS.decode(bytes);
    ensure!(!errors, "script is neither UTF-8 nor Shift_JIS");
    Ok(text.into_owned())
}

fn err(e: crysirror_core::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_prefixes_and_decodes_boms() {
        assert_eq!(normalize("file://./data\\bg.png"), "data/bg.png");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, b'b', 0]).unwrap(), "ab");
        assert_eq!(decode_text(b"\xEF\xBB\xBFok").unwrap(), "ok");
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("吉里吉里");
        assert_eq!(decode_text(&sjis).unwrap(), "吉里吉里");
    }
}
