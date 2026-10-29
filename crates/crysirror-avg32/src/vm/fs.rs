// SPDX-License-Identifier: MPL-2.0
//! CrySirror seam replacing upstream's `game_fs`: the original code keeps
//! calling `read`/`read_dir`/`write`, which resolve through the engine's
//! client-scoped capability storage installed for the current VM step.
//! Paths are game-root relative (`DAT/SEEN.TXT`); `@save/<name>` addresses
//! the client's save mount. No ambient filesystem path is ever opened.
use anyhow::{Context, Result, bail};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, StorageMetadata},
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const SAVE_ROOT: &str = "@save";

#[derive(Clone)]
pub struct Storage {
    pub storage: Arc<dyn ClientStorage>,
    /// Asset-relative directory holding the installed game.
    pub root: String,
}

thread_local! {
    static CURRENT: RefCell<Option<Storage>> = const { RefCell::new(None) };
}

/// Installs `storage` for the duration of `body` (nesting restores the outer one).
pub fn with<T>(storage: &Storage, body: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|c| c.replace(Some(storage.clone())));
    struct Restore(Option<Storage>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CURRENT.with(|c| *c.borrow_mut() = previous);
        }
    }
    let _restore = Restore(previous);
    body()
}

fn current() -> Result<Storage> {
    CURRENT
        .with(|c| c.borrow().clone())
        .context("AVG32 file access outside a CrySirror VM step")
}

fn vfs(path: &Path) -> Result<VfsPath> {
    let text = path
        .to_str()
        .context("AVG32 path is not UTF-8")?
        .replace('\\', "/");
    let text = text.trim_start_matches("./").trim_matches('/');
    if let Some(rest) = text.strip_prefix(SAVE_ROOT) {
        let rest = rest.trim_start_matches('/');
        return Ok(VfsPath::new(Mount::Save, rest)?);
    }
    let root = current()?.root;
    let joined = match (root.is_empty(), text.is_empty()) {
        (true, _) => text.to_owned(),
        (false, true) => root,
        (false, false) => format!("{root}/{text}"),
    };
    Ok(VfsPath::new(Mount::Asset, &joined)?)
}

pub fn read(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let vfs = vfs(path)?;
    current()?
        .storage
        .read(&vfs)
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("failed to read {}", path.display()))
}

pub fn file_len(path: impl AsRef<Path>) -> Result<u64> {
    let vfs = vfs(path.as_ref())?;
    let storage = current()?.storage;
    match storage.metadata(vfs.mount(), vfs.relative()) {
        Ok(Some(StorageMetadata { size: Some(size), .. })) => Ok(size),
        Ok(Some(_)) => Ok(storage.read(&vfs).map_err(|e| anyhow::anyhow!("{e}"))?.len() as u64),
        Ok(None) => bail!("{} not found", path.as_ref().display()),
        Err(_) => Ok(storage.read(&vfs).map_err(|e| anyhow::anyhow!("{e}"))?.len() as u64),
    }
}

/// Directory entry names (not paths) under a game-relative directory.
pub fn list(directory: impl AsRef<Path>) -> Result<Vec<String>> {
    let storage = current()?;
    let text = directory.as_ref().to_str().unwrap_or_default();
    let (mount, relative) = if text.trim_matches(['/', '.']).is_empty() {
        // The game root itself (which may be the asset root).
        (Mount::Asset, storage.root.clone())
    } else {
        let vfs = vfs(directory.as_ref())?;
        (vfs.mount(), vfs.relative().to_owned())
    };
    storage
        .storage
        .list(mount, &relative)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Upstream `read_dir` shape: `(name, path)` pairs.
pub fn read_dir(directory: impl AsRef<Path>) -> Result<Vec<(String, PathBuf)>> {
    let directory = directory.as_ref();
    Ok(list(directory)?
        .into_iter()
        .map(|name| {
            let path = directory.join(&name);
            (name, path)
        })
        .collect())
}

/// Atomic write; only the save mount is writable.
pub fn write(path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
    let vfs = vfs(path.as_ref())?;
    if vfs.mount() != Mount::Save {
        bail!("AVG32 writes are limited to the save mount");
    }
    current()?
        .storage
        .write_atomic(&vfs, bytes)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

pub fn metadata(path: impl AsRef<Path>) -> Option<StorageMetadata> {
    let vfs = vfs(path.as_ref()).ok()?;
    current()
        .ok()?
        .storage
        .metadata(vfs.mount(), vfs.relative())
        .ok()
        .flatten()
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    metadata(path).is_some_and(|m| m.is_directory)
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    metadata(path).is_some_and(|m| !m.is_directory)
}

pub fn read_to_string(path: impl AsRef<Path>) -> Result<String> {
    Ok(String::from_utf8_lossy(&read(path)?).into_owned())
}

/// Random-access asset stream (CloneCD images, AFS voice archives).
pub struct File {
    reader: Arc<dyn crysirror_core::service::AssetReader>,
    position: u64,
}

pub fn open(path: impl AsRef<Path>) -> Result<File> {
    let vfs = vfs(path.as_ref())?;
    let reader = current()?
        .storage
        .open_asset(&vfs)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(File {
        reader,
        position: 0,
    })
}

impl std::io::Read for File {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let count = self
            .reader
            .read_at(self.position, output)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.position += count as u64;
        Ok(count)
    }
}

impl std::io::Seek for File {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        let length = self
            .reader
            .len()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let target = match position {
            std::io::SeekFrom::Start(at) => Some(at),
            std::io::SeekFrom::End(delta) => length.checked_add_signed(delta),
            std::io::SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        self.position = target
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek"))?;
        Ok(self.position)
    }
}
