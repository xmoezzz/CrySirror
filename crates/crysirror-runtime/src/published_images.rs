// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Owner-published immutable worker images. No encoding, hashing, pixel copy or
//! mutable slot reuse at publication. Frozen frames retain the resource lease.
use crate::assets::Surface;
use crysirror_core::{
    Error, Result,
    image_processing::ImageResource,
    path::{Mount, VfsPath},
    service::UploadedImage,
    state::ClientId,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
/// Per-client resident CPU copies of published images. Visual-novel VMs keep
/// whole ABMP frame sets resident (QLIE scenes exceed 256 MiB of members).
const MAX_PUBLISHED_BYTES: usize = 512 * 1024 * 1024;
static NEXT_IMAGE: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct State {
    entries: BTreeMap<(ClientId, String), Surface>,
    bytes: usize,
    reserved_count: usize,
    reserved_bytes: usize,
    retired: BTreeSet<(ClientId, String)>,
    /// Checkpoint capabilities retain catalog authority and its byte budget.
    pins: BTreeMap<(ClientId, String), u32>,
    /// Released raw-upload leases whose generated asset bytes the owner's
    /// asset cache has not yet dropped.
    dropped: Vec<(ClientId, String)>,
}
#[derive(Clone, Default)]
pub struct PublishedImages(Arc<Mutex<State>>);
impl PublishedImages {
    pub(crate) fn inspect(&self)->serde_json::Value {
        self.0.lock().map(|state|serde_json::json!({
            "resources":state.entries.len(),"bytes":state.bytes,
            "retired":state.retired.len(),"reserved_count":state.reserved_count,
            "reserved_bytes":state.reserved_bytes,
        })).unwrap_or(serde_json::Value::Null)
    }
    /// Retain the immutable decoded lease of an owner upload. The generated
    ///file path still follows the ordinary asset route; CPU consumers can
    ///borrow these bytes without re-encoding/decoding that route.
    pub(crate) fn retain_upload(
        &self,
        client: ClientId,
        path: &str,
        size: [u32; 2],
        pixels: Arc<[u8]>,
        opaque: bool,
    ) -> Result<()> {
        if !path.starts_with("__crysirror/")
            || pixels.len() as u64 != u64::from(size[0]) * u64::from(size[1]) * 4
        {
            return Err(Error::Invalid("uploaded resource lease layout/path"));
        }
        let resource: Arc<dyn ImageResource> = Arc::new(UploadedLease {
            client,
            size,
            pixels: pixels.clone(),
            opaque,
        });
        let mut state = self
            .0
            .lock()
            .map_err(|_| Error::Invalid("uploaded resource lease lock"))?;
        let key = (client, path.to_owned());
        if state.entries.contains_key(&key) {
            return Ok(());
        }
        if state
            .entries
            .len()
            .checked_add(state.reserved_count)
            .is_none_or(|n| n >= 4096)
        {
            return Err(Error::Limit("uploaded resource leases (count)"));
        }
        if state.bytes
            .checked_add(state.reserved_bytes)
            .and_then(|n| n.checked_add(pixels.len()))
            .is_none_or(|n| n > MAX_PUBLISHED_BYTES)
        {
            return Err(Error::Limit("uploaded resource leases (bytes)"));
        }
        state.bytes += pixels.len();
        state.entries.insert(
            key,
            Surface {
                width: size[0],
                height: size[1],
                rgba: pixels,
                revision: 0,
                _resource: Some(resource),
            },
        );
        Ok(())
    }

    pub fn publish(
        &self,
        client: ClientId,
        id: u32,
        resource: Arc<dyn ImageResource>,
    ) -> Result<UploadedImage> {
        if resource.client() != client {
            return Err(Error::Invalid("foreign image resource"));
        }
        let size = resource.size();
        let rgba = resource.rgba();
        if !(1..=8192).contains(&size[0])
            || !(1..=8192).contains(&size[1])
            || rgba.len() as u64 != u64::from(size[0]) * u64::from(size[1]) * 4
        {
            return Err(Error::Invalid("published image geometry/layout"));
        }
        let opaque = resource.opaque();
        let bytes = rgba.len();
        let mut s = self
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        if s.entries
            .len()
            .checked_add(s.reserved_count)
            .is_none_or(|n| n >= 4096)
            || s.bytes
                .checked_add(s.reserved_bytes)
                .and_then(|n| n.checked_add(bytes))
                .is_none_or(|v| v > MAX_PUBLISHED_BYTES)
        {
            return Err(Error::Limit("published image resources"));
        }
        let generation = NEXT_IMAGE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| Error::Limit("published image generation"))?;
        let name = format!("__image_resource/{generation}-{id}");
        s.entries.insert(
            (client, name.clone()),
            Surface {
                width: size[0],
                height: size[1],
                rgba,
                revision: generation,
                _resource: Some(resource),
            },
        );
        s.bytes += bytes;
        Ok(UploadedImage {
            path: VfsPath::new(Mount::Asset, &name)?,
            size,
            opaque,
        })
    }
    pub fn release(&self, client: ClientId, path: &str) -> Result<()> {
        self.retire(client, path, false)
    }
    /// End caller ownership while preserving engine-retained logical paths.
    pub(crate) fn retire(&self, client: ClientId, path: &str, retained: bool) -> Result<()> {
        // Raw uploads now enter this same owner-scoped lease catalog. Their
        // generated asset names must retire through the same lifetime fence.
        if !(path.starts_with("__image_resource/")||path.starts_with("__crysirror/"))
            || VfsPath::new(Mount::Asset, path)?.relative() != path
        {
            return Err(Error::Invalid("published image path"));
        }
        let mut s = self
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        let key = (client, path.to_owned());
        // Multiple live aliases can end ownership before the retained layer
        // fence releases the generation. A repeat for this owned retired key
        // is already satisfied; foreign or unknown keys still fail.
        if s.retired.contains(&key){return Ok(())}
        if !s.entries.contains_key(&key){return Err(Error::NotFound)}
        if retained || s.pins.contains_key(&key) {
            s.retired.insert(key);
            return Ok(());
        }
        let item = s.entries.remove(&key).ok_or(Error::NotFound)?;
        s.bytes -= item.rgba.len();
        if key.1.starts_with("__crysirror/") {
            s.dropped.push(key);
        }
        Ok(())
    }
    pub(crate) fn has_retired(&self) -> bool {
        self.0.lock().map(|s| !s.retired.is_empty()).unwrap_or(true)
    }
    pub(crate) fn collect_retired(&self, referenced: &BTreeSet<(ClientId, String)>) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        let unused: Vec<_> = state
            .retired
            .iter()
            .filter(|key| !referenced.contains(*key) && !state.pins.contains_key(*key))
            .cloned()
            .collect();
        for key in unused {
            state.retired.remove(&key);
            if let Some(surface) = state.entries.remove(&key) {
                state.bytes -= surface.rgba.len();
                if key.1.starts_with("__crysirror/") {
                    state.dropped.push(key);
                }
            }
        }
        Ok(())
    }
    /// Released generated paths that no live lease has republished since.
    pub(crate) fn take_dropped(&self) -> Vec<(ClientId, String)> {
        let Ok(mut state) = self.0.lock() else {
            return Vec::new();
        };
        let dropped = std::mem::take(&mut state.dropped);
        dropped.into_iter().filter(|key| !state.entries.contains_key(key)).collect()
    }
    pub(crate) fn get(&self, client: ClientId, path: &str) -> Option<Surface> {
        self.0
            .lock()
            .ok()?
            .entries
            .get(&(client, path.into()))
            .cloned()
    }
    pub(crate) fn snapshot(&self) -> BTreeMap<(ClientId, String), Surface> {
        self.0.lock().map(|s| s.entries.clone()).unwrap_or_default()
    }
    pub(crate) fn has_resources(&self) -> bool {
        self.0.lock().map(|s| !s.entries.is_empty()).unwrap_or(true)
    }
}

/// Unpublished immutable resources with real shared-capacity reservations.
/// A failed/cancelled restore drops these without inserting any catalog entry.
pub(crate) struct PreparedImages {
    catalog: PublishedImages,
    client: ClientId,
    entries: Vec<(String, Surface)>,
    bytes: usize,
}
impl PreparedImages {
    pub(crate) fn new(catalog: PublishedImages, client: ClientId) -> Self {
        Self {
            catalog,
            client,
            entries: Vec::new(),
            bytes: 0,
        }
    }
    pub(crate) fn reserve(
        &mut self,
        id: u32,
        resource: Arc<dyn ImageResource>,
    ) -> Result<UploadedImage> {
        if resource.client() != self.client {
            return Err(Error::Invalid("foreign restore image resource"));
        }
        let size = resource.size();
        let rgba = resource.rgba();
        let bytes = rgba.len();
        if !(1..=8192).contains(&size[0])
            || !(1..=8192).contains(&size[1])
            || bytes as u64 != u64::from(size[0]) * u64::from(size[1]) * 4
        {
            return Err(Error::Invalid("restore image geometry/layout"));
        }
        let opaque = resource.opaque();
        let mut state = self
            .catalog
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        if state
            .entries
            .len()
            .checked_add(state.reserved_count)
            .is_none_or(|n| n >= 4096)
            || state
                .bytes
                .checked_add(state.reserved_bytes)
                .and_then(|n| n.checked_add(bytes))
                .is_none_or(|n| n > MAX_PUBLISHED_BYTES)
        {
            return Err(Error::Limit("published image resources"));
        }
        let generation = NEXT_IMAGE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Limit("published image generation"))?;
        let name = format!("__image_resource/{generation}-{id}");
        let path = VfsPath::new(Mount::Asset, &name)?;
        state.reserved_count += 1;
        state.reserved_bytes += bytes;
        self.bytes += bytes;
        self.entries.push((
            name,
            Surface {
                width: size[0],
                height: size[1],
                rgba,
                revision: generation,
                _resource: Some(resource),
            },
        ));
        Ok(UploadedImage { path, size, opaque })
    }
    pub(crate) fn stats(&self) -> (usize, usize) {
        (self.entries.len(), self.bytes)
    }
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.entries.iter().any(|(name, _)| name == path)
    }
    /// Read only this preparation's reserved immutable publications. They are
    /// not yet present in the live catalog and grant no other VM's old leases.
    pub(crate) fn resource(&self, path: &str) -> Option<Arc<dyn ImageResource>> {
        self.entries.iter().find(|(name, _)| name == path)
            .and_then(|(_, surface)| surface._resource.clone())
    }
    pub(crate) fn release(&mut self, path: &str) -> Result<()> {
        let at = self
            .entries
            .iter()
            .position(|(name, _)| name == path)
            .ok_or(Error::NotFound)?;
        let mut state = self
            .catalog
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        let (_, surface) = self.entries.remove(at);
        let bytes = surface.rgba.len();
        state.reserved_count -= 1;
        state.reserved_bytes -= bytes;
        self.bytes -= bytes;
        Ok(())
    }
    /// The sole fallible lock acquisition occurs before catalog mutation.
    /// Capacity was reserved; installing all entries cannot partially fail.
    pub(crate) fn commit(
        mut self,
        release: &[String],
        retained: &BTreeSet<(ClientId, String)>,
        retain_all: bool,
    ) -> Result<()> {
        let catalog = self.catalog.clone();
        let mut state = catalog
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        let mut unique = BTreeSet::new();
        for path in release {
            let key = (self.client, path.clone());
            if !unique.insert(key.clone())
                || !state.entries.contains_key(&key)
                || state.retired.contains(&key)
            {
                return Err(Error::Invalid("restore image release lease"));
            }
        }
        state.reserved_count -= self.entries.len();
        state.reserved_bytes -= self.bytes;
        state.bytes += self.bytes;
        for (name, surface) in self.entries.drain(..) {
            state.entries.insert((self.client, name), surface);
        }
        for key in unique {
            if retain_all || retained.contains(&key) || state.pins.contains_key(&key) {
                state.retired.insert(key);
            } else if let Some(surface) = state.entries.remove(&key) {
                state.bytes -= surface.rgba.len();
            }
        }
        self.bytes = 0;
        Ok(())
    }
}
impl Drop for PreparedImages {
    fn drop(&mut self) {
        if let Ok(mut state) = self.catalog.0.lock() {
            state.reserved_count -= self.entries.len();
            state.reserved_bytes -= self.bytes;
        }
    }
}

#[derive(Debug)]
struct UploadedLease {
    client: ClientId,
    size: [u32; 2],
    pixels: Arc<[u8]>,
    opaque: bool,
}
impl ImageResource for UploadedLease {
    fn client(&self) -> ClientId {
        self.client
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.pixels.clone()
    }
    fn opaque(&self) -> bool {
        self.opaque
    }
    fn pixel_at(&self, point: [u32; 2]) -> Result<Option<[u8; 4]>> {
        crysirror_core::image_processing::rgba_pixel_at(self.size, &self.pixels, point)
    }
}

/// Keeps original logical paths alive without releasing their shared budget.
/// Dropping a snapshot removes only its pins; scene/other snapshot leases stay.
pub(crate) struct CheckpointPins {
    catalog: PublishedImages,
    keys: Vec<(ClientId,String)>,
}
impl Drop for CheckpointPins {
    fn drop(&mut self) {
        if let Ok(mut state)=self.catalog.0.lock() {
            for key in &self.keys {
                if let Some(count)=state.pins.get_mut(key){*count-=1;if *count==0{state.pins.remove(key);}}
            }
        }
    }
}
impl PublishedImages {
    pub(crate) fn pin_checkpoint(&self,client:ClientId,paths:&BTreeSet<String>)->Result<CheckpointPins> {
        let mut state=self.0.lock().map_err(|_|Error::Invalid("image checkpoint owner lock"))?;
        let keys=paths.iter().map(|path|(client,path.clone())).collect::<Vec<_>>();
        for key in &keys {
            if !state.entries.contains_key(key){return Err(Error::NotFound)}
            if state.pins.get(key)==Some(&u32::MAX){return Err(Error::Limit("image checkpoint leases"))}
        }
        for key in &keys{*state.pins.entry(key.clone()).or_default()+=1;}
        Ok(CheckpointPins{catalog:self.clone(),keys})
    }
}
