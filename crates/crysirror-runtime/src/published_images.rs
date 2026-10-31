// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Owner-published immutable worker images. No encoding, hashing, pixel copy or
//! mutable slot reuse at publication. Frozen frames retain the resource lease.
use crate::assets::Surface;
use crysirror_core::{
    image_processing::ImageResource,
    path::{Mount, VfsPath},
    service::UploadedImage,
    state::ClientId,
    Error, Result,
};
use std::{
    collections::{BTreeMap,BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
static NEXT_IMAGE: AtomicU64 = AtomicU64::new(1);
#[derive(Default)]
struct State {
    entries: BTreeMap<(ClientId, String), Surface>,
    bytes: usize,
    retired: BTreeSet<(ClientId,String)>,
}
#[derive(Clone, Default)]
pub struct PublishedImages(Arc<Mutex<State>>);
impl PublishedImages {
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
        if s.entries.len() >= 4096
            || s.bytes
                .checked_add(bytes)
                .is_none_or(|v| v > 256 * 1024 * 1024)
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
        self.retire(client,path,false)
    }
    /// End caller ownership while preserving engine-retained logical paths.
    pub(crate) fn retire(&self, client: ClientId, path: &str, retained: bool) -> Result<()> {
        if !path.starts_with("__image_resource/")
            || VfsPath::new(Mount::Asset, path)?.relative() != path
        {
            return Err(Error::Invalid("published image path"));
        }
        let mut s = self
            .0
            .lock()
            .map_err(|_| Error::Invalid("image publication owner lock"))?;
        let key=(client,path.to_owned());
        if s.retired.contains(&key) || !s.entries.contains_key(&key) {return Err(Error::NotFound);}
        if retained {s.retired.insert(key);return Ok(());}
        let item = s
            .entries
            .remove(&key)
            .ok_or(Error::NotFound)?;
        s.bytes -= item.rgba.len();
        Ok(())
    }
    pub(crate) fn has_retired(&self)->bool {self.0.lock().map(|s|!s.retired.is_empty()).unwrap_or(true)}
    pub(crate) fn collect_retired(&self, referenced:&BTreeSet<(ClientId,String)>)->Result<()> {
        let mut state=self.0.lock().map_err(|_|Error::Invalid("image publication owner lock"))?;
        let unused:Vec<_>=state.retired.iter().filter(|key|!referenced.contains(*key)).cloned().collect();
        for key in unused {state.retired.remove(&key);if let Some(surface)=state.entries.remove(&key){state.bytes-=surface.rgba.len();}}
        Ok(())
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
