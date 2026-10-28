// SPDX-License-Identifier: MPL-2.0
//! Capability-based roots. No adapter can open an ambient absolute path here.
use crate::model::identity;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use crysirror_core::{
    Error, Result,
    path::{Mount, VfsPath},
    service::{AssetReader, ClientStorage, FileSystem, StorageLease},
    state::ClientId,
    vfs::{VfsProvider, strip_mount},
};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MAX_FILE: u64 = 64 * 1024 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
fn io(e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::NotFound
    } else {
        Error::Io(e.to_string())
    }
}
pub fn read_limited(dir: &Dir, path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let file = dir.open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(Error::Invalid("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::Limit("file size"));
    }
    Ok(bytes)
}
pub fn write_atomic(dir: &Dir, relative: &str, bytes: &[u8]) -> Result<()> {
    VfsPath::new(Mount::Save, relative)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::Limit("file size"));
    }
    let target = Path::new(relative);
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    if !parent.as_os_str().is_empty() {
        dir.create_dir_all(parent).map_err(io)?;
    }
    let mut selected: Option<(PathBuf, cap_std::fs::File)> = None;
    for _ in 0..32 {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(".crysirror-{}-{id}.tmp", std::process::id()));
        match dir.open_with(&temp, OpenOptions::new().write(true).create_new(true)) {
            Ok(file) => {
                selected = Some((temp, file));
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        }
    }
    let (temp, mut file) = selected.ok_or(Error::Limit("temporary filenames"))?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = dir.remove_file(&temp);
        return Err(io(e));
    }
    if let Err(e) = dir.rename(&temp, dir, target) {
        let _ = dir.remove_file(&temp);
        return Err(io(e));
    }
    // Atomic replacement, not a promise of parent-directory durability on power loss.
    Ok(())
}
struct Mounts {
    asset: Dir,
    save: Dir,
    cache: Dir,
}
type ProviderMount = (String, Arc<dyn VfsProvider>);
type ProviderMounts = Arc<RwLock<Vec<ProviderMount>>>;

// No provider callback runs under this lock. A poisoned writer cannot leave
// partially valid Rust values, so retain existing mounts rather than hiding
// overlays after a recovered panic.
fn provider_snapshot(mounts: &ProviderMounts) -> Vec<ProviderMount> {
    mounts
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub struct DiskVfs {
    writes: crysirror_tasks::TaskPool,
    pub assets: Dir,
    writable_root: Dir,
    mounts: BTreeMap<ClientId, Mounts>,
    /// Per-client provider mounts over the asset namespace, newest first.
    providers: BTreeMap<ClientId, ProviderMounts>,
}
impl DiskVfs {
    pub fn new(assets: &Path, writable: &Path) -> Result<Self> {
        std::fs::create_dir_all(writable).map_err(io)?;
        Ok(Self {
            writes: crysirror_tasks::TaskPool::new("vfs-write", 1, 64)
                .map_err(|e| Error::Io(e.to_string()))?,
            assets: Dir::open_ambient_dir(assets, ambient_authority()).map_err(io)?,
            writable_root: Dir::open_ambient_dir(writable, ambient_authority()).map_err(io)?,
            mounts: BTreeMap::new(),
            providers: BTreeMap::new(),
        })
    }
    pub fn register(&mut self, client: ClientId, engine: &str, game: &str) -> Result<()> {
        self.register_with_assets(client, engine, game, "")
    }
    pub fn register_with_assets(
        &mut self,
        client: ClientId,
        engine: &str,
        game: &str,
        prefix: &str,
    ) -> Result<()> {
        if !identity(engine) || !identity(game) || self.mounts.contains_key(&client) {
            return Err(Error::Invalid("VFS identity"));
        }
        let mut dirs = Vec::new();
        for kind in ["save", "cache"] {
            let path = format!("{engine}/{game}/{kind}");
            self.writable_root.create_dir_all(&path).map_err(io)?;
            dirs.push(self.writable_root.open_dir(&path).map_err(io)?);
        }
        let cache = dirs.pop().ok_or(Error::NotFound)?;
        let save = dirs.pop().ok_or(Error::NotFound)?;
        let asset = if prefix.is_empty() {
            self.assets.try_clone().map_err(io)?
        } else {
            let p = VfsPath::new(Mount::Asset, prefix)?;
            self.assets.open_dir(p.relative()).map_err(io)?
        };
        self.mounts.insert(client, Mounts { asset, save, cache });
        self.providers
            .insert(client, Arc::new(RwLock::new(Vec::new())));
        Ok(())
    }
    /// Overlay a read-only provider at `mount_point` in one client's asset
    /// namespace. Later mounts shadow earlier mounts and the disk root.
    /// Mounting does not grant access to any other client's namespace.
    pub fn mount_provider(
        &mut self,
        client: ClientId,
        mount_point: &str,
        provider: Arc<dyn VfsProvider>,
    ) -> Result<()> {
        if !self.mounts.contains_key(&client) {
            return Err(Error::NotFound);
        }
        let point = if mount_point.is_empty() {
            String::new()
        } else {
            VfsPath::new(Mount::Asset, mount_point)?
                .relative()
                .to_owned()
        };
        let mounts = self.providers.get(&client).ok_or(Error::NotFound)?;
        let mut list = mounts.write().unwrap_or_else(|error| error.into_inner());
        if list.len() >= 64 {
            return Err(Error::Limit("provider mounts"));
        }
        list.insert(0, (point, provider));
        Ok(())
    }
    /// A thread-safe handle with this client's authority only (ADR 0008).
    /// Later provider mounts are visible to existing handles on their next
    /// operation; an already opened reader retains its original data source.
    pub fn storage(&self, client: ClientId) -> Result<Arc<dyn ClientStorage>> {
        let m = self.mounts.get(&client).ok_or(Error::NotFound)?;
        Ok(Arc::new(CapStorage {
            writes: self.writes.clone(),
            asset: m.asset.try_clone().map_err(io)?,
            save: m.save.try_clone().map_err(io)?,
            cache: m.cache.try_clone().map_err(io)?,
            providers: self
                .providers
                .get(&client)
                .cloned()
                .ok_or(Error::NotFound)?,
        }))
    }
    pub fn provider_mounts(&self, client: ClientId) -> Vec<(String, &'static str)> {
        self.providers
            .get(&client)
            .map(|mounts| {
                provider_snapshot(mounts)
                    .into_iter()
                    .map(|(p, v)| (p, v.kind()))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Open a read-only, seekable disk asset under this client's capability.
    /// A matching provider can shadow the file, so callers must fall back to
    /// normal provider-aware reads in that case. No ambient path is returned.
    pub fn open_asset_file(
        &self,
        client: ClientId,
        path: &VfsPath,
    ) -> Result<Option<std::fs::File>> {
        if path.mount() != Mount::Asset {
            return Err(Error::Invalid("stream source must be an asset"));
        }
        let dir = self.dir(client, Mount::Asset)?;
        let mounts = self.providers.get(&client).ok_or(Error::NotFound)?;
        if provider_snapshot(mounts)
            .iter()
            .any(|(point, _)| strip_mount(&point, path.relative()).is_some())
        {
            return Ok(None);
        }
        let file = dir.open(path.relative()).map_err(io)?;
        if !file.metadata().map_err(io)?.is_file() {
            return Err(Error::Invalid("not a regular asset file"));
        }
        Ok(Some(file.into_std()))
    }
    fn dir(&self, client: ClientId, mount: Mount) -> Result<&Dir> {
        let m = self.mounts.get(&client).ok_or(Error::NotFound)?;
        Ok(match mount {
            Mount::Asset => &m.asset,
            Mount::Save => &m.save,
            Mount::Cache => &m.cache,
        })
    }
}
struct CapStorage {
    writes: crysirror_tasks::TaskPool,
    asset: Dir,
    save: Dir,
    cache: Dir,
    providers: ProviderMounts,
}
struct CapAssetReader {
    file: Mutex<cap_std::fs::File>,
    len: u64,
}
struct CapStorageLease {
    _file: std::fs::File,
}
impl StorageLease for CapStorageLease {}
impl AssetReader for CapAssetReader {
    fn len(&self) -> Result<u64> {
        Ok(self.len)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let mut file = self
            .file
            .lock()
            .map_err(|_| Error::Io("asset stream lock poisoned".into()))?;
        file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let count = output
            .len()
            .min((self.len - offset).min(usize::MAX as u64) as usize);
        file.read(&mut output[..count]).map_err(io)
    }
}
impl CapStorage {
    fn dir(&self, mount: Mount) -> &Dir {
        match mount {
            Mount::Asset => &self.asset,
            Mount::Save => &self.save,
            Mount::Cache => &self.cache,
        }
    }
}
struct PendingWrite {
    task: Option<crysirror_tasks::Task<()>>,
    state: crysirror_core::service::StorageWriteState,
}
impl crysirror_core::service::StorageWrite for PendingWrite {
    fn poll(&mut self) -> Result<crysirror_core::service::StorageWriteState> {
        if let Some(task) = &mut self.task {
            match task.try_take() {
                Ok(None) => (),
                Ok(Some(())) => {
                    self.task = None;
                    self.state = crysirror_core::service::StorageWriteState::Ready;
                }
                Err(e) => {
                    self.task = None;
                    self.state = crysirror_core::service::StorageWriteState::Failed(e.to_string());
                }
            }
        }
        Ok(self.state.clone())
    }
}
impl ClientStorage for CapStorage {
    fn metadata(&self, mount: Mount, relative: &str) -> Result<Option<crysirror_core::service::StorageMetadata>> {
        use crysirror_core::service::StorageMetadata;
        if !relative.is_empty() && VfsPath::new(mount, relative)?.relative() != relative {
            return Err(Error::Invalid("metadata canonical path"));
        }
        let virtual_info = |size: Option<u64>| StorageMetadata { is_directory: size.is_none(), size,
            created_ms: None, modified_ms: None, accessed_ms: None };
        if mount == Mount::Asset {
            for (point, provider) in provider_snapshot(&self.providers) {
                if relative == point || strip_mount(relative, &point).is_some() {
                    return Ok(Some(virtual_info(None)));
                }
                let Some(rest) = strip_mount(&point, relative) else { continue; };
                match provider.open(rest) {
                    Ok(reader) => return Ok(Some(virtual_info(Some(reader.len()?)))),
                    Err(Error::NotFound) => {},
                    Err(e) => return Err(e),
                }
                if !provider.list_dir(rest)?.is_empty() { return Ok(Some(virtual_info(None))); }
            }
        }
        let metadata = match self.dir(mount).metadata(if relative.is_empty() { "." } else { relative }) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        if !metadata.is_file() && !metadata.is_dir() { return Err(Error::Unsupported("special storage metadata")); }
        let millis = |t: std::io::Result<cap_std::time::SystemTime>| t.ok().and_then(|t| {
            let t = t.into_std();
            let n = match t.duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => i128::try_from(d.as_millis()).ok()?,
                Err(e) => -i128::try_from(e.duration().as_millis()).ok()?,
            };
            i64::try_from(n).ok()
        });
        Ok(Some(StorageMetadata { is_directory: metadata.is_dir(), size: metadata.is_file().then(|| metadata.len()),
            created_ms: millis(metadata.created()), modified_ms: millis(metadata.modified()), accessed_ms: millis(metadata.accessed()) }))
    }

    fn container_mount(&self,source:&str)->Result<Option<String>> {
        let path=VfsPath::new(Mount::Asset,source)?;
        if path.relative()!=source {return Err(Error::Invalid("container source path"))}
        Ok(provider_snapshot(&self.providers).into_iter().find(|(_,p)|p.container_source()==Some(source)).map(|(point,_)|point))
    }

    fn begin_write_atomic(
        &self,
        path: &VfsPath,
        data: Arc<[u8]>,
    ) -> Result<Box<dyn crysirror_core::service::StorageWrite>> {
        if path.mount() == Mount::Asset {
            return Err(Error::ReadOnly);
        }
        if data.len() > 4 * 1024 * 1024 {
            return Err(Error::Limit("async atomic write bytes"));
        }
        let permit = self
            .writes
            .reserve()
            .map_err(|e| Error::Io(e.to_string()))?;
        let dir = self.dir(path.mount()).try_clone().map_err(io)?;
        let relative = path.relative().to_owned();
        let task = permit
            .spawn(move |cancel| {
                cancel.check()?;
                write_atomic(&dir, &relative, &data).map_err(|e| anyhow::anyhow!("{e}"))
            })
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(Box::new(PendingWrite {
            task: Some(task),
            state: crysirror_core::service::StorageWriteState::Pending,
        }))
    }
    fn acquire_lease(&self, name: &str) -> Result<Box<dyn StorageLease>> {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::Invalid("storage lease name"));
        }
        self.cache.create_dir_all("__leases").map_err(io)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        let file = self
            .cache
            .open_with(format!("__leases/{name}"), &options)
            .map_err(io)?
            .into_std();
        file.try_lock()
            .map_err(|e| Error::Io(format!("storage lease {name} unavailable: {e}")))?;
        Ok(Box::new(CapStorageLease { _file: file }))
    }
    fn open_asset(&self, path: &VfsPath) -> Result<Arc<dyn AssetReader>> {
        if path.mount() != Mount::Asset {
            return Err(Error::ReadOnly);
        }
        for (point, provider) in provider_snapshot(&self.providers) {
            let Some(rest) = strip_mount(&point, path.relative()) else {
                continue;
            };
            match provider.open(rest) {
                Err(Error::NotFound) => continue,
                Ok(reader) => return Ok(reader),
                Err(error) => return Err(error),
            }
        }
        let file = self.asset.open(path.relative()).map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        if !metadata.is_file() {
            return Err(Error::Invalid("asset stream must be a file"));
        }
        Ok(Arc::new(CapAssetReader {
            file: Mutex::new(file),
            len: metadata.len(),
        }))
    }
    fn open_read(&self, path: &VfsPath) -> Result<Arc<dyn AssetReader>> {
        if path.mount()==Mount::Asset {return self.open_asset(path)}
        let file=self.dir(path.mount()).open(path.relative()).map_err(io)?;
        let metadata=file.metadata().map_err(io)?;
        if !metadata.is_file(){return Err(Error::Invalid("read stream must be a file"))}
        Ok(Arc::new(CapAssetReader{file:Mutex::new(file),len:metadata.len()}))
    }
    fn read(&self, path: &VfsPath) -> Result<Vec<u8>> {
        if path.mount() == Mount::Asset {
            for (point, provider) in provider_snapshot(&self.providers) {
                let Some(rest) = strip_mount(&point, path.relative()) else {
                    continue;
                };
                match provider.read(rest) {
                    Err(Error::NotFound) => continue,
                    Ok(b) if b.len() as u64 > MAX_FILE => return Err(Error::Limit("file size")),
                    other => return other,
                }
            }
        }
        read_limited(self.dir(path.mount()), path.relative())
    }
    fn write_atomic(&self, path: &VfsPath, data: &[u8]) -> Result<()> {
        if path.mount() == Mount::Asset {
            return Err(Error::ReadOnly);
        }
        write_atomic(self.dir(path.mount()), path.relative(), data)
    }
    fn remove(&self, path: &VfsPath) -> Result<()> {
        if path.mount() == Mount::Asset {
            return Err(Error::ReadOnly);
        }
        self.dir(path.mount())
            .remove_file(path.relative())
            .map_err(io)
    }
    fn list(&self, mount: Mount, dir: &str) -> Result<Vec<String>> {
        let rel = if dir.is_empty() {
            String::new()
        } else {
            VfsPath::new(mount, dir)?.relative().to_owned()
        };
        let mut names = std::collections::BTreeSet::new();
        let d = self.dir(mount);
        let entries = if rel.is_empty() {
            d.entries()
        } else {
            d.read_dir(&rel)
        };
        match entries {
            Ok(entries) => {
                for e in entries {
                    let e = e.map_err(io)?;
                    if let Ok(n) = e.file_name().into_string() {
                        if !n.starts_with(".crysirror-") {
                            names.insert(n);
                        }
                    }
                    if names.len() > 65536 {
                        return Err(Error::Limit("directory listing"));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        if mount == Mount::Asset {
            for (point, provider) in provider_snapshot(&self.providers) {
                if rel==point {
                    names.extend(provider.list_dir("")?);
                } else if let Some(rest)=strip_mount(&point,&rel) {
                    names.extend(provider.list_dir(rest)?);
                } else if let Some(rest)=strip_mount(&rel,&point) {
                    if let Some(child)=rest.split('/').next().filter(|v|!v.is_empty()){names.insert(child.into());}
                }
                if names.len()>65536{return Err(Error::Limit("directory listing"))}
            }
        }
        Ok(names.into_iter().collect())
    }
}
impl FileSystem for DiskVfs {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> Result<Vec<u8>> {
        let dir = self.dir(client, path.mount())?;
        if path.mount() == Mount::Asset {
            let mounts = self.providers.get(&client).ok_or(Error::NotFound)?;
            for (point, provider) in provider_snapshot(mounts) {
                let Some(rest) = strip_mount(&point, path.relative()) else {
                    continue;
                };
                match provider.read(rest) {
                    Err(Error::NotFound) => continue,
                    Ok(b) if b.len() as u64 > MAX_FILE => return Err(Error::Limit("file size")),
                    other => return other,
                }
            }
        }
        read_limited(dir, path.relative())
    }
    fn write_atomic(&mut self, client: ClientId, path: &VfsPath, data: &[u8]) -> Result<()> {
        if path.mount() == Mount::Asset {
            return Err(Error::ReadOnly);
        }
        write_atomic(self.dir(client, path.mount())?, path.relative(), data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_preserves_capabilities_and_does_not_invent_archive_timestamps() {
        use crysirror_core::vfs::MemoryProvider;
        let assets = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::create_dir(assets.path().join("empty")).unwrap();
        std::fs::write(assets.path().join("same.bin"), b"disk").unwrap();
        let mut fs = DiskVfs::new(assets.path(), state.path()).unwrap();
        fs.register(ClientId(1), "test", "one").unwrap();
        fs.register(ClientId(2), "test", "two").unwrap();
        let mut provider = MemoryProvider::new();
        provider.insert("same.bin", b"archive".to_vec()).unwrap();
        provider.insert("folder/child", b"x".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "", Arc::new(provider)).unwrap();
        let one = fs.storage(ClientId(1)).unwrap();
        let archive = one.metadata(Mount::Asset, "same.bin").unwrap().unwrap();
        assert_eq!(archive.size, Some(7));
        assert_eq!((archive.created_ms, archive.modified_ms, archive.accessed_ms), (None, None, None));
        assert!(one.metadata(Mount::Asset, "folder").unwrap().unwrap().is_directory);
        assert!(one.metadata(Mount::Asset, "empty").unwrap().unwrap().is_directory);
        assert!(one.metadata(Mount::Asset, "absent").unwrap().is_none());
        assert!(one.metadata(Mount::Asset, "../outside").is_err());
        let save = VfsPath::new(Mount::Save, "private.bin").unwrap();
        one.write_atomic(&save, b"private").unwrap();
        assert_eq!(one.metadata(Mount::Save, "private.bin").unwrap().unwrap().size, Some(7));
        assert!(fs.storage(ClientId(2)).unwrap().metadata(Mount::Save, "private.bin").unwrap().is_none());
    }
    #[test]
    fn read_streams_cover_scoped_save_and_cache_without_reading_foreign_clients() {
        let a=tempfile::tempdir().unwrap();let w=tempfile::tempdir().unwrap();
        let mut fs=DiskVfs::new(a.path(),w.path()).unwrap();
        fs.register(ClientId(1),"krkr","a").unwrap();fs.register(ClientId(2),"krkr","b").unwrap();
        let one=fs.storage(ClientId(1)).unwrap();let two=fs.storage(ClientId(2)).unwrap();
        for mount in [Mount::Save,Mount::Cache] {
            let path=VfsPath::new(mount,"small.json").unwrap();one.write_atomic(&path,b"{\"ok\":true}").unwrap();
            let reader=one.open_read(&path).unwrap();assert_eq!(reader.len().unwrap(),11);
            let mut bytes=[0;11];assert_eq!(reader.read_at(0,&mut bytes).unwrap(),11);assert_eq!(&bytes,b"{\"ok\":true}");
            assert!(two.open_read(&path).is_err());
        }
    }
    #[test]
    fn disk_namespaces_are_separate() {
        let a = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let mut fs = DiskVfs::new(a.path(), w.path()).unwrap();
        fs.register(ClientId(1), "rfvp", "game").unwrap();
        fs.register(ClientId(2), "siglus", "game").unwrap();
        let p = VfsPath::new(Mount::Save, "slot/1.sav").unwrap();
        fs.write_atomic(ClientId(1), &p, b"one").unwrap();
        fs.write_atomic(ClientId(2), &p, b"two").unwrap();
        assert_eq!(fs.read(ClientId(1), &p).unwrap(), b"one");
        assert_eq!(fs.read(ClientId(2), &p).unwrap(), b"two");
        fs.write_atomic(ClientId(1), &p, b"replace").unwrap();
        assert_eq!(fs.read(ClientId(1), &p).unwrap(), b"replace");
    }
    #[test]
    fn asset_mount_is_read_only() {
        let a = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        let mut fs = DiskVfs::new(a.path(), w.path()).unwrap();
        fs.register(ClientId(1), "test", "game").unwrap();
        assert_eq!(
            fs.write_atomic(ClientId(1), &VfsPath::new(Mount::Asset, "x").unwrap(), b"x"),
            Err(Error::ReadOnly)
        );
    }
    #[test]
    fn seekable_assets_keep_client_roots_and_provider_precedence() {
        use crysirror_core::vfs::MemoryProvider;
        let assets = tempfile::tempdir().unwrap();
        let writable = tempfile::tempdir().unwrap();
        for (name, data) in [("a", b"one"), ("b", b"two")] {
            std::fs::create_dir_all(assets.path().join(name)).unwrap();
            std::fs::write(assets.path().join(name).join("song.wav"), data).unwrap();
        }
        let mut fs = DiskVfs::new(assets.path(), writable.path()).unwrap();
        fs.register_with_assets(ClientId(1), "test", "one", "a")
            .unwrap();
        fs.register_with_assets(ClientId(2), "test", "two", "b")
            .unwrap();
        let path = VfsPath::new(Mount::Asset, "song.wav").unwrap();
        for (client, expected) in [(1, b"one"), (2, b"two")] {
            let mut file = fs
                .open_asset_file(ClientId(client), &path)
                .unwrap()
                .unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            assert_eq!(data, expected);
        }
        assert!(fs.open_asset_file(ClientId(9), &path).is_err());
        assert!(
            fs.open_asset_file(ClientId(1), &VfsPath::new(Mount::Save, "song.wav").unwrap())
                .is_err()
        );
        let mut provider = MemoryProvider::new();
        provider.insert("song.wav", b"overlay".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "", Arc::new(provider))
            .unwrap();
        assert!(fs.open_asset_file(ClientId(1), &path).unwrap().is_none());
        assert_eq!(fs.read(ClientId(1), &path).unwrap(), b"overlay");
        assert!(fs.open_asset_file(ClientId(2), &path).unwrap().is_some());
    }
    #[test]
    fn provider_mounts_shadow_disk_per_client_only() {
        use crysirror_core::vfs::MemoryProvider;
        let a = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(a.path().join("pack")).unwrap();
        std::fs::write(a.path().join("pack/a.txt"), b"disk").unwrap();
        std::fs::write(a.path().join("b.txt"), b"disk-b").unwrap();
        let mut fs = DiskVfs::new(a.path(), w.path()).unwrap();
        fs.register(ClientId(1), "siglus", "game").unwrap();
        fs.register(ClientId(2), "siglus", "other").unwrap();
        let mut m = MemoryProvider::new();
        m.insert("a.txt", b"memory".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "pack", Arc::new(m)).unwrap();
        let a_txt = VfsPath::new(Mount::Asset, "pack/a.txt").unwrap();
        assert_eq!(fs.read(ClientId(1), &a_txt).unwrap(), b"memory");
        assert_eq!(fs.read(ClientId(2), &a_txt).unwrap(), b"disk");
        // Paths the provider does not hold fall through to the disk root.
        let b = VfsPath::new(Mount::Asset, "b.txt").unwrap();
        assert_eq!(fs.read(ClientId(1), &b).unwrap(), b"disk-b");
        assert_eq!(
            fs.provider_mounts(ClientId(1)),
            vec![("pack".into(), "memory")]
        );
        assert!(
            fs.mount_provider(ClientId(9), "", Arc::new(MemoryProvider::new()))
                .is_err()
        );
    }
    #[test]
    fn storage_handle_is_client_scoped_thread_safe_and_lists() {
        use crysirror_core::vfs::MemoryProvider;
        let a = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("disk.bin"), b"d").unwrap();
        let mut fs = DiskVfs::new(a.path(), w.path()).unwrap();
        fs.register(ClientId(1), "rfvp", "game").unwrap();
        fs.register(ClientId(2), "rfvp", "other").unwrap();
        let mut m = MemoryProvider::new();
        m.insert("graph/a", b"m".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "", Arc::new(m)).unwrap();
        let s = fs.storage(ClientId(1)).unwrap();
        let t = std::thread::spawn(move || {
            assert_eq!(s.list(Mount::Asset, "").unwrap(), vec!["disk.bin", "graph"]);
            assert_eq!(s.list(Mount::Asset, "graph").unwrap(), vec!["a"]);
            let save = VfsPath::new(Mount::Save, "save/s1").unwrap();
            s.write_atomic(&save, b"x").unwrap();
            assert_eq!(s.list(Mount::Save, "save").unwrap(), vec!["s1"]);
            assert_eq!(
                s.write_atomic(&VfsPath::new(Mount::Asset, "disk.bin").unwrap(), b"x"),
                Err(Error::ReadOnly)
            );
            s.remove(&save).unwrap();
            assert!(!s.exists(&save));
        });
        t.join().unwrap();
        let other = fs.storage(ClientId(2)).unwrap();
        assert!(
            other
                .read(&VfsPath::new(Mount::Asset, "graph/a").unwrap())
                .is_err()
        );
    }
    #[test]
    fn held_storage_observes_later_mounts_without_crossing_clients() {
        use crysirror_core::vfs::MemoryProvider;
        let assets = tempfile::tempdir().unwrap();
        let writable = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("shadow.txt"), b"disk").unwrap();
        std::fs::write(assets.path().join("fallback.txt"), b"fallback").unwrap();
        let mut fs = DiskVfs::new(assets.path(), writable.path()).unwrap();
        fs.register(ClientId(1), "test", "one").unwrap();
        fs.register(ClientId(2), "test", "two").unwrap();
        let held_a = fs.storage(ClientId(1)).unwrap();
        let held_b = fs.storage(ClientId(2)).unwrap();
        let new = VfsPath::new(Mount::Asset, "new/added.txt").unwrap();
        let shadow = VfsPath::new(Mount::Asset, "shadow.txt").unwrap();
        let fallback = VfsPath::new(Mount::Asset, "fallback.txt").unwrap();
        assert_eq!(held_a.read(&new), Err(Error::NotFound));
        assert!(fs.open_asset_file(ClientId(1), &shadow).unwrap().is_some());
        let mut first = MemoryProvider::new();
        first.insert("new/added.txt", b"added".to_vec()).unwrap();
        first.insert("shadow.txt", b"overlay".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "", Arc::new(first)).unwrap();
        // The handles predate the mount, including a handle used on a worker.
        let worker_a = held_a.clone();
        let worker_new = new.clone();
        std::thread::spawn(move || {
            assert_eq!(worker_a.read(&worker_new).unwrap(), b"added");
            let reader = worker_a.open_asset(&worker_new).unwrap();
            let mut bytes = [0; 5];
            assert_eq!(reader.read_at(0, &mut bytes).unwrap(), 5);
            assert_eq!(&bytes, b"added");
            assert_eq!(
                worker_a.list(Mount::Asset, "new").unwrap(),
                vec!["added.txt"]
            );
        })
        .join()
        .unwrap();
        assert_eq!(
            held_a.list(Mount::Asset, "").unwrap(),
            vec!["fallback.txt", "new", "shadow.txt"]
        );
        assert_eq!(
            held_b.list(Mount::Asset, "").unwrap(),
            vec!["fallback.txt", "shadow.txt"]
        );
        assert_eq!(held_b.read(&new), Err(Error::NotFound));
        assert!(matches!(held_b.open_asset(&new), Err(Error::NotFound)));
        assert_eq!(held_b.read(&shadow).unwrap(), b"disk");
        assert!(fs.open_asset_file(ClientId(1), &shadow).unwrap().is_none());
        assert!(fs.open_asset_file(ClientId(2), &shadow).unwrap().is_some());
        // A matching mount disables the raw-disk shortcut even for a missing
        // provider entry; provider-aware storage still falls through to disk.
        assert!(
            fs.open_asset_file(ClientId(1), &fallback)
                .unwrap()
                .is_none()
        );
        assert_eq!(held_a.read(&fallback).unwrap(), b"fallback");
        let mut bytes = [0; 8];
        assert_eq!(
            held_a
                .open_asset(&fallback)
                .unwrap()
                .read_at(0, &mut bytes)
                .unwrap(),
            8
        );
        assert_eq!(&bytes, b"fallback");
        let mut newest = MemoryProvider::new();
        newest.insert("shadow.txt", b"newest".to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "", Arc::new(newest))
            .unwrap();
        assert_eq!(held_a.read(&shadow).unwrap(), b"newest");
        assert_eq!(held_a.read(&new).unwrap(), b"added");
        assert_eq!(fs.read(ClientId(1), &shadow).unwrap(), b"newest");
        assert_eq!(fs.provider_mounts(ClientId(1)).len(), 2);
        assert!(fs.provider_mounts(ClientId(2)).is_empty());
    }
    #[test]
    fn provider_callbacks_run_after_mount_snapshot_unlocks() {
        use crysirror_core::vfs::MemoryProvider;
        struct CheckingProvider {
            mounts: std::sync::Weak<RwLock<Vec<ProviderMount>>>,
            memory: MemoryProvider,
        }
        impl CheckingProvider {
            fn check(&self) {
                let mounts = self.mounts.upgrade().unwrap();
                assert!(
                    mounts.try_write().is_ok(),
                    "provider callback retained the mount lock"
                );
            }
        }
        impl VfsProvider for CheckingProvider {
            fn kind(&self) -> &'static str {
                self.check();
                "checking"
            }
            fn read(&self, relative: &str) -> Result<Vec<u8>> {
                self.check();
                self.memory.read(relative)
            }
            fn list(&self) -> Result<Vec<String>> {
                self.check();
                self.memory.list()
            }
        }
        let assets = tempfile::tempdir().unwrap();
        let writable = tempfile::tempdir().unwrap();
        let mut fs = DiskVfs::new(assets.path(), writable.path()).unwrap();
        fs.register(ClientId(1), "test", "game").unwrap();
        let held = fs.storage(ClientId(1)).unwrap();
        let mut memory = MemoryProvider::new();
        memory.insert("item", b"value".to_vec()).unwrap();
        let provider = CheckingProvider {
            mounts: Arc::downgrade(&fs.providers[&ClientId(1)]),
            memory,
        };
        fs.mount_provider(ClientId(1), "", Arc::new(provider))
            .unwrap();
        let item = VfsPath::new(Mount::Asset, "item").unwrap();
        assert_eq!(held.read(&item).unwrap(), b"value");
        assert_eq!(held.open_asset(&item).unwrap().len().unwrap(), 5);
        assert_eq!(held.list(Mount::Asset, "").unwrap(), vec!["item"]);
        assert_eq!(fs.read(ClientId(1), &item).unwrap(), b"value");
        assert_eq!(
            fs.provider_mounts(ClientId(1)),
            vec![(String::new(), "checking")]
        );
    }
    #[test]
    fn shared_provider_mounts_keep_the_per_client_limit() {
        use crysirror_core::vfs::MemoryProvider;
        let assets = tempfile::tempdir().unwrap();
        let writable = tempfile::tempdir().unwrap();
        let mut fs = DiskVfs::new(assets.path(), writable.path()).unwrap();
        fs.register(ClientId(1), "test", "one").unwrap();
        fs.register(ClientId(2), "test", "two").unwrap();
        let _held = fs.storage(ClientId(1)).unwrap();
        for _ in 0..64 {
            fs.mount_provider(ClientId(1), "", Arc::new(MemoryProvider::new()))
                .unwrap();
        }
        assert_eq!(
            fs.mount_provider(ClientId(1), "", Arc::new(MemoryProvider::new())),
            Err(Error::Limit("provider mounts"))
        );
        assert_eq!(fs.provider_mounts(ClientId(1)).len(), 64);
        fs.mount_provider(ClientId(2), "", Arc::new(MemoryProvider::new()))
            .unwrap();
        assert_eq!(fs.provider_mounts(ClientId(2)).len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_cannot_escape_assets() {
        let a = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let w = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"not an asset").unwrap();
        std::os::unix::fs::symlink(outside.path(), a.path().join("escape")).unwrap();
        let mut fs = DiskVfs::new(a.path(), w.path()).unwrap();
        fs.register(ClientId(1), "test", "game").unwrap();
        assert!(
            fs.open_asset_file(
                ClientId(1),
                &VfsPath::new(Mount::Asset, "escape/secret").unwrap()
            )
            .is_err()
        );
        assert!(
            fs.read(
                ClientId(1),
                &VfsPath::new(Mount::Asset, "escape/secret").unwrap()
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod atomic_write_tests {
    use super::*;
    use crysirror_core::service::{StorageWrite, StorageWriteState};
    fn poll(op: &mut dyn StorageWrite) -> StorageWriteState {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let s = op.poll().unwrap();
            if s != StorageWriteState::Pending {
                return s;
            }
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
    }
    #[test]
    fn worker_commits_are_atomic_owner_scoped_and_retain_success_or_failure() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("assets")).unwrap();
        let mut fs = DiskVfs::new(&d.path().join("assets"), &d.path().join("state")).unwrap();
        fs.register(ClientId(1), "test", "a").unwrap();
        fs.register(ClientId(2), "test", "b").unwrap();
        let a = fs.storage(ClientId(1)).unwrap();
        let b = fs.storage(ClientId(2)).unwrap();
        let path = VfsPath::new(Mount::Save, "data/state.ksd").unwrap();
        a.write_atomic(&path, b"old").unwrap();
        let mut op = a
            .begin_write_atomic(&path, Arc::from(&b"replacement"[..]))
            .unwrap();
        assert_eq!(poll(&mut *op), StorageWriteState::Ready);
        assert_eq!(op.poll().unwrap(), StorageWriteState::Ready);
        assert_eq!(a.read(&path).unwrap(), b"replacement");
        assert!(b.read(&path).is_err());
        assert!(
            a.begin_write_atomic(
                &VfsPath::new(Mount::Asset, "forbidden").unwrap(),
                Arc::from(&b"x"[..])
            )
            .is_err()
        );
        let bad = VfsPath::new(Mount::Save, "data/state.ksd/child").unwrap();
        let mut failed = a.begin_write_atomic(&bad, Arc::from(&b"bad"[..])).unwrap();
        let result = poll(&mut *failed);
        assert!(matches!(result, StorageWriteState::Failed(_)));
        assert_eq!(failed.poll().unwrap(), result);
        assert_eq!(a.read(&path).unwrap(), b"replacement");
    }
    #[test]
    fn rejected_worker_admission_does_not_change_existing_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("assets")).unwrap();
        let mut fs = DiskVfs::new(&d.path().join("assets"), &d.path().join("state")).unwrap();
        fs.register(ClientId(1), "test", "a").unwrap();
        let a = fs.storage(ClientId(1)).unwrap();
        let path = VfsPath::new(Mount::Save, "value").unwrap();
        a.write_atomic(&path, b"old").unwrap();
        let permits = (0..fs.writes.stats().capacity)
            .map(|_| fs.writes.reserve().unwrap())
            .collect::<Vec<_>>();
        assert!(a.begin_write_atomic(&path, Arc::from(&b"new"[..])).is_err());
        assert_eq!(a.read(&path).unwrap(), b"old");
        drop(permits);
        let mut op = a.begin_write_atomic(&path, Arc::from(&b"new"[..])).unwrap();
        assert_eq!(poll(&mut *op), StorageWriteState::Ready);
        assert_eq!(a.read(&path).unwrap(), b"new");
    }
}
