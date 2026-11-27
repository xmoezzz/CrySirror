// SPDX-License-Identifier: MPL-2.0
//! Ren'Py's loose-first search order over client-scoped Resources mounts.
use crysirror_core::{
    Error, Result,
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use std::{collections::BTreeMap, sync::Arc};

pub trait GameFiles {
    fn list(&self) -> Vec<String>;
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    fn exists(&self, name: &str) -> bool {
        self.read(name).is_some() || self.common_read(name).is_some()
    }
    fn common_list(&self) -> Vec<String>;
    fn common_read(&self, name: &str) -> Option<Vec<u8>>;
    fn temporary_storage(&self) -> Option<&dyn ClientStorage> {
        None
    }
}

pub struct ScopedFiles {
    storage: Arc<dyn ClientStorage>,
    game: BTreeMap<String, VfsPath>,
    common: BTreeMap<String, VfsPath>,
}

impl ScopedFiles {
    pub fn new(storage: Arc<dyn ClientStorage>, archive_mounts: &[String]) -> Result<Self> {
        Self::load(storage, archive_mounts, || Ok(()))
    }

    pub(crate) fn load(
        storage: Arc<dyn ClientStorage>,
        archive_mounts: &[String],
        check: impl Fn() -> Result<()>,
    ) -> Result<Self> {
        let mut game = BTreeMap::new();
        index(storage.as_ref(), "game", "", &mut game, &check)?;
        for mount in archive_mounts {
            index(storage.as_ref(), mount, "", &mut game, &check)?;
        }
        let mut common = BTreeMap::new();
        if storage
            .metadata(Mount::Asset, "renpy/common")?
            .is_some_and(|m| m.is_directory)
        {
            index(storage.as_ref(), "renpy/common", "", &mut common, &check)?;
        }
        Ok(Self {
            storage,
            game,
            common,
        })
    }

    pub fn asset_path(&self, name: &str) -> Option<&VfsPath> {
        self.game.get(name).or_else(|| self.common.get(name))
    }
}

fn index(
    storage: &dyn ClientStorage,
    root: &str,
    relative: &str,
    entries: &mut BTreeMap<String, VfsPath>,
    check: &impl Fn() -> Result<()>,
) -> Result<()> {
    if relative.bytes().filter(|&c| c == b'/').count() >= 32 {
        return Err(Error::Limit("Ren'Py resource nesting"));
    }
    let dir = if relative.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    };
    check()?;
    for name in storage.list(Mount::Asset, &dir)? {
        check()?;
        if relative.is_empty()
            && root == "game"
            && (matches!(name.as_str(), "saves" | "cache" | "old-game") || name.ends_with(".rpa"))
        {
            continue;
        }
        let relative = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let path = VfsPath::new(Mount::Asset, &format!("{root}/{relative}"))?;
        match storage.metadata(Mount::Asset, path.relative())? {
            Some(m) if m.is_directory => index(storage, root, &relative, entries, check)?,
            Some(_) => {
                if entries.len() >= 100_000 {
                    return Err(Error::Limit("Ren'Py resource entries"));
                }
                entries.entry(relative).or_insert(path);
            }
            None => return Err(Error::NotFound),
        }
    }
    Ok(())
}

fn read_script_asset(storage: &dyn ClientStorage, path: &VfsPath) -> Option<Vec<u8>> {
    let reader = storage.open_asset(path).ok()?;
    crysirror_resource::read_all(reader.as_ref(), 32 * 1024 * 1024).ok()
}

impl GameFiles for ScopedFiles {
    fn list(&self) -> Vec<String> {
        self.game.keys().cloned().collect()
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.game
            .get(name)
            .and_then(|path| read_script_asset(self.storage.as_ref(), path))
    }
    fn exists(&self, name: &str) -> bool {
        self.asset_path(name).is_some()
    }
    fn temporary_storage(&self) -> Option<&dyn ClientStorage> {
        Some(self.storage.as_ref())
    }
    fn common_list(&self) -> Vec<String> {
        self.common.keys().cloned().collect()
    }
    fn common_read(&self, name: &str) -> Option<Vec<u8>> {
        self.common
            .get(name)
            .and_then(|path| read_script_asset(self.storage.as_ref(), path))
    }
}
