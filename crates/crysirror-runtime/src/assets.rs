// SPDX-License-Identifier: MPL-2.0
use crate::vfs::read_limited;
use anyhow::{ensure, Context, Result};
use cap_std::{ambient_authority, fs::Dir};
use crysirror_core::{
    path::{Mount, VfsPath},
    state::ClientId, service::ClientStorage,
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[path = "assets_async.rs"]
mod asynchronous;
pub use asynchronous::{AssetKind, AssetState, AssetTicket, AssetStatus};

/// Immutable assets with a capability root per client, not merely a string prefix.
/// Explicit reload constructs a new store/runtime. This is not a native-code sandbox.
pub struct Assets {
    asynchronous: asynchronous::AsyncAssets,
    /// CPU text resources belong to engine resources, not a GPU/device lifetime.
    pub text: crysirror_text::TextSystem,
    pub decoders: crysirror_image::DecoderRegistry,
    root: Dir,
    clients: BTreeMap<ClientId, Dir>,
    storage: BTreeMap<ClientId, Arc<dyn ClientStorage>>,
    bytes: BTreeMap<String, Arc<[u8]>>,
    total: usize,
    limit: usize,
    surfaces: BTreeMap<(ClientId, u32), Surface>,
}
/// A replaceable RGBA render product (ADR 0008). Never saved.
#[derive(Clone)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub revision: u64,
}
pub const SURFACE_PREFIX: &str = "__surface/";
pub const MAX_SURFACES_PER_CLIENT: usize = 16;
impl Assets {
    pub fn new(path: &Path) -> Result<Self> {
        #[allow(unused_mut)]
        let mut decoders=crysirror_image::DecoderRegistry::default();
        #[cfg(feature="arc-formats")]
        crysirror_formats_arc::register_images(&mut decoders)?;
        Ok(Self {
            asynchronous: asynchronous::AsyncAssets::new()?,
            text: Default::default(),
            decoders,
            root: Dir::open_ambient_dir(path, ambient_authority()).context("open asset root")?,
            clients: BTreeMap::new(),
            storage: BTreeMap::new(),
            bytes: BTreeMap::new(),
            total: 0,
            limit: 256 * 1024 * 1024,
            surfaces: BTreeMap::new(),
        })
    }
    /// Decode through the same registry and scoped dependency resolver used by
    /// Bitmap, UI and renderer texture uploads.
    pub fn decode_image(&self,client:ClientId,path:&str,bytes:&[u8])->Result<crysirror_image::ImageSurface>{
        self.decode_image_with(client,path,bytes,None,None)
    }
    pub fn decode_image_with(&self,client:ClientId,path:&str,bytes:&[u8],resources:Option<&crysirror_resource::DecodeContext>,format:Option<&str>)->Result<crysirror_image::ImageSurface>{
        let source=|name:&str|->crysirror_core::Result<Vec<u8>>{
            let logical=VfsPath::new(Mount::Asset,name)?;
            if let Some(storage)=self.storage.get(&client){storage.read(&logical)}
            else if let Some(dir)=self.clients.get(&client){read_limited(dir,logical.relative())}
            else if client.0==0 {read_limited(&self.root,logical.relative())}
            else{Err(crysirror_core::Error::NotFound)}
        };
        self.decoders.decode_with(path,bytes,Some(&source),resources,format)
    }
    fn insert(&mut self, key: String, bytes: Vec<u8>) -> Result<Arc<[u8]>> {
        ensure!(
            self.total
                .checked_add(bytes.len())
                .is_some_and(|n| n <= self.limit),
            "asset cache exceeds 256 MiB"
        );
        self.total += bytes.len();
        let bytes: Arc<[u8]> = Arc::from(bytes);
        self.bytes.insert(key, bytes.clone());
        Ok(bytes)
    }
    /// Project-wide assets, such as fonts, use a separate cache namespace.
    pub fn read(&mut self, path: &str) -> Result<Arc<[u8]>> {
        let path = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let key = format!("project:{}", path.relative());
        if let Some(bytes) = self.bytes.get(&key) {
            return Ok(bytes.clone());
        }
        let bytes = read_limited(&self.root, path.relative())
            .map_err(|e| anyhow::anyhow!("asset {}: {e}", path.relative()))?;
        self.insert(key, bytes)
    }
    pub fn register(&mut self, client: ClientId, prefix: &str) -> Result<()> {
        ensure!(
            !self.clients.contains_key(&client),
            "duplicate asset client"
        );
        let dir = if prefix.is_empty() {
            self.root.try_clone()?
        } else {
            let path = VfsPath::new(Mount::Asset, prefix).map_err(|e| anyhow::anyhow!("{e}"))?;
            self.root
                .open_dir(path.relative())
                .context("client asset root")?
        };
        self.clients.insert(client, dir);
        Ok(())
    }
    pub fn key(&self, client: ClientId, path: &str) -> Result<String> {
        let path = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(self.clients.contains_key(&client), "unknown asset client");
        Ok(format!("client:{}:{}", client.0, path.relative()))
    }
    pub fn read_for(&mut self, client: ClientId, path: &str) -> Result<Arc<[u8]>> {
        let key = self.key(client, path)?;
        if let Some(bytes) = self.bytes.get(&key) {
            return Ok(bytes.clone());
        }
        self.poll_tasks();
        if let Some(bytes)=self.prepared_bytes(client,path){return Ok(bytes);}
        let bytes = if let Some(storage)=self.storage.get(&client) {
            let logical=VfsPath::new(Mount::Asset,path).map_err(|e|anyhow::anyhow!("{e}"))?;
            storage.read(&logical).map_err(|e|anyhow::anyhow!("client asset {path}: {e}"))?
        } else {
            let dir = self.clients.get(&client).context("unknown asset client")?;
            read_limited(dir, path).map_err(|e| anyhow::anyhow!("client asset {path}: {e}"))?
        };
        self.insert(key, bytes)
    }
    /// Refresh the same capability namespace used by native services after a
    /// mount. Ordinary cached files are invalidated; generated assets retain
    /// their separate immutable namespace.
    pub fn refresh_storage(&mut self,client:ClientId,storage:Arc<dyn ClientStorage>)->Result<()> {
        ensure!(self.clients.contains_key(&client),"unknown asset client");
        let prefix=format!("client:{}:",client.0);
        let generated=format!("{prefix}__crysirror/");
        self.bytes.retain(|key,bytes|{
            if key.starts_with(&prefix) && !key.starts_with(&generated) {
                self.total-=bytes.len();false
            } else {true}
        });
        self.asynchronous.invalidate(client);
        self.storage.insert(client,storage);Ok(())
    }
    pub fn register_generated(
        &mut self,
        client: ClientId,
        path: &str,
        data: Vec<u8>,
    ) -> Result<()> {
        ensure!(
            path.starts_with("__crysirror/"),
            "generated resource namespace"
        );
        let key = self.key(client, path)?;
        if let Some(old) = self.bytes.get(&key) {
            ensure!(
                old.as_ref() == data.as_slice(),
                "immutable generated resource changed"
            );
            return Ok(());
        }
        self.insert(key, data)?;
        Ok(())
    }
    /// Read an already-published immutable engine resource, scoped to its owner.
    /// Unlike read_for this never falls back to disk or bypasses provider mounts.
    pub fn generated_for(&self, client: ClientId, path: &str) -> Result<Option<Arc<[u8]>>> {
        ensure!(path.starts_with("__crysirror/"), "generated resource namespace");
        let key = self.key(client, path)?;
        Ok(self.bytes.get(&key).cloned())
    }
    pub fn put_surface(
        &mut self,
        client: ClientId,
        slot: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<String> {
        ensure!(self.clients.contains_key(&client), "unknown asset client");
        ensure!(
            (1..=8192).contains(&width) && (1..=8192).contains(&height),
            "surface dimensions"
        );
        ensure!(
            rgba.len() as u64 == u64::from(width) * u64::from(height) * 4,
            "surface RGBA length"
        );
        let count = self.surfaces.keys().filter(|(c, _)| *c == client).count();
        let revision = match self.surfaces.get(&(client, slot)) {
            Some(old) => old
                .revision
                .checked_add(1)
                .context("surface revision overflow")?,
            None => {
                ensure!(count < MAX_SURFACES_PER_CLIENT, "surface slot limit");
                1
            }
        };
        let old_bytes = self
            .surfaces
            .get(&(client, slot))
            .map_or(0, |surface| surface.rgba.len());
        let total = self
            .surfaces
            .values()
            .map(|surface| surface.rgba.len())
            .sum::<usize>()
            - old_bytes;
        ensure!(
            total
                .checked_add(rgba.len())
                .is_some_and(|n| n <= 256 * 1024 * 1024),
            "surfaces exceed 256 MiB"
        );
        self.surfaces.insert(
            (client, slot),
            Surface {
                width,
                height,
                rgba: Arc::from(rgba),
                revision,
            },
        );
        Ok(format!("{SURFACE_PREFIX}{slot}"))
    }
    pub fn remove_surface(&mut self, client: ClientId, slot: u32) -> bool {
        self.surfaces.remove(&(client, slot)).is_some()
    }
    /// Resolve an image asset path that names a surface.
    pub fn surface(&self, client: ClientId, asset: &str) -> Option<&Surface> {
        let slot = asset.strip_prefix(SURFACE_PREFIX)?.parse().ok()?;
        self.surfaces.get(&(client, slot))
    }
    pub fn clear(&mut self) {
        let clients: Vec<_> = self.clients.keys().copied().collect();
        for client in clients { self.asynchronous.invalidate(client); }
        self.bytes.clear();
        self.total = 0;
        self.surfaces.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_names_use_separate_client_roots() {
        let tmp = tempfile::tempdir().unwrap();
        for (dir, bytes) in [("a", b"left".as_slice()), ("b", b"right".as_slice())] {
            std::fs::create_dir(tmp.path().join(dir)).unwrap();
            std::fs::write(tmp.path().join(dir).join("panel"), bytes).unwrap();
        }
        let mut assets = Assets::new(tmp.path()).unwrap();
        assets.register(ClientId(1), "a").unwrap();
        assets.register(ClientId(2), "b").unwrap();
        assert_eq!(
            assets.read_for(ClientId(1), "panel").unwrap().as_ref(),
            b"left"
        );
        assert_eq!(
            assets.read_for(ClientId(2), "panel").unwrap().as_ref(),
            b"right"
        );
    }
    #[cfg(unix)]
    #[test]
    fn relative_symlink_cannot_cross_client_root_even_if_project_can_read_it() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("a")).unwrap();
        std::fs::create_dir_all(tmp.path().join("b")).unwrap();
        std::fs::write(tmp.path().join("b/secret"), b"other client").unwrap();
        std::os::unix::fs::symlink("../b/secret", tmp.path().join("a/link")).unwrap();
        let mut assets = Assets::new(tmp.path()).unwrap();
        assets.register(ClientId(1), "a").unwrap();
        assert!(assets.read("a/link").is_ok());
        assert!(assets.read_for(ClientId(1), "link").is_err());
    }
}
