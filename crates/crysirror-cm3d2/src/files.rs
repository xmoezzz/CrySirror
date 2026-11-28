// SPDX-License-Identifier: MPL-2.0
//! `GameUty.FileSystem`: the archives of `GameUty.Init` with every folder
//! registered through `AddAutoPath`, so game code opens files by name alone
//! (`FileOpen("body001.model")`, `"allmain.ks"`). Names compare case-
//! insensitively. A later archive replaces an earlier entry of the same name
//! (inferred from the patch archives being added after their base).
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::collections::HashMap;

/// Upper bound on indexed files (CM3D2 1.00 ships about 54k).
const MAX_FILES: usize = 1 << 20;

#[derive(Clone, Debug, Default)]
pub struct GameFiles {
    by_name: HashMap<String, String>,
    csv_paths: Vec<String>,
    /// Full paths under `menu/` (`GetList("menu", AllFile)`).
    pub menu_files: Vec<String>,
}

/// Whether an archive entry name is a file (`ft_0001.ks`) rather than a
/// folder: an extension of 1–8 ASCII alphanumerics.
fn is_file_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()))
}

impl GameFiles {
    pub fn index(storage: &dyn ClientStorage, archives: &[String]) -> Result<Self> {
        let mut files = Self::default();
        for root in archives {
            if let Some(suffix) = root.rsplit('/').next().and_then(|name| name.strip_prefix("csv_"))
                && !suffix.rsplit_once('_').is_some_and(|(_, n)| matches!(n, "2" | "3"))
                && !files.csv_paths.iter().any(|old| old == suffix)
            {
                files.csv_paths.push(suffix.to_owned());
            }
            let mut pending = vec![root.clone()];
            while let Some(dir) = pending.pop() {
                for name in storage.list(Mount::Asset, &dir).with_context(|| format!("list game archive directory {dir:?}"))? {
                    let path = format!("{dir}/{name}");
                    // Every file has a short ASCII extension; folders may
                    // contain dots too (`01.処女喪失セックス`).
                    if !is_file_name(&name) {
                        pending.push(path);
                        continue;
                    }
                    let inner = path.strip_prefix(root.as_str()).unwrap_or(&path).trim_start_matches('/');
                    if inner.to_ascii_lowercase().starts_with("menu/") {
                        files.menu_files.push(path.clone());
                    }
                    files.by_name.insert(name.to_ascii_lowercase(), path);
                    ensure!(files.by_name.len() <= MAX_FILES, "CM3D2 file index limit");
                }
            }
        }
        Ok(files)
    }

    pub fn path(&self, name: &str) -> Option<&str> {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        self.by_name.get(&base.to_ascii_lowercase()).map(String::as_str)
    }

    pub(crate) fn csv_paths(&self) -> &[String] {
        &self.csv_paths
    }

    /// Every indexed file name (lower case) with extension `ext` (`.nei`).
    pub fn names_with_ext(&self, ext: &str) -> Vec<String> {
        let mut v: Vec<String> = self.by_name.keys().filter(|n| n.ends_with(ext)).cloned().collect();
        v.sort();
        v
    }

    pub fn exists(&self, name: &str) -> bool {
        self.path(name).is_some()
    }

    pub fn read(&self, storage: &dyn ClientStorage, name: &str) -> Result<Vec<u8>> {
        let path = self.path(name).with_context(|| format!("CM3D2 file {name} is not in the game archives"))?;
        let path = VfsPath::new(Mount::Asset, path).with_context(|| format!("invalid game archive entry {path:?}"))?;
        storage.read(&path).with_context(|| format!("read game archive entry {}", path.relative()))
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dotted_folders_are_not_files() {
        assert!(super::is_file_name("c_ft_0001.ks"));
        assert!(super::is_file_name("body001.model"));
        assert!(!super::is_file_name("01.処女喪失セックス"));
        assert!(!super::is_file_name("00_ft"));
    }
}
