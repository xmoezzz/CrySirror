//! Native BF handle attachment and release policy; frame codecs live in the engine.
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct BurikoMovieResource {
    pub archive: String,
    pub resource: String,
    pub bytes: Arc<[u8]>,
    pub metadata: [i32; 5],
    pub parent: Option<i32>,
    pub child: Option<i32>,
}

#[derive(Debug, Default)]
pub(crate) struct BurikoMovieRegistry {
    next_handle: i32,
    resources: BTreeMap<i32, BurikoMovieResource>,
}

impl BurikoMovieRegistry {
    pub(crate) fn load(
        &mut self,
        archive: String,
        resource: String,
        bytes: Vec<u8>,
    ) -> Result<(i32, [i32; 5]), i32> {
        let metadata = crysirror_image::cbg::bf_movie_metadata(&bytes).ok_or(2)?;
        self.next_handle = self.next_handle.saturating_add(1).max(1);
        let handle = self.next_handle;
        self.resources.insert(
            handle,
            BurikoMovieResource {
                archive,
                resource,
                bytes: Arc::from(bytes),
                metadata,
                parent: None,
                child: None,
            },
        );
        Ok((handle, metadata))
    }

    pub(crate) fn contains(&self, handle: i32) -> bool {
        self.resources.contains_key(&handle)
    }

    pub(crate) fn metadata(&self, handle: i32) -> Option<[i32; 5]> {
        self.resources
            .get(&handle)
            .map(|resource| resource.metadata)
    }

    pub(crate) fn release(&mut self, handle: i32) -> i32 {
        let Some(resource) = self.resources.remove(&handle) else {
            return 3;
        };
        if let Some(parent) = resource.parent
            && let Some(parent_resource) = self.resources.get_mut(&parent)
            && parent_resource.child == Some(handle)
        {
            parent_resource.child = resource.child;
        }
        if let Some(child) = resource.child
            && let Some(child_resource) = self.resources.get_mut(&child)
        {
            child_resource.parent = resource.parent;
        }
        0
    }

    pub(crate) fn attach(&mut self, source: i32) -> Result<i32, i32> {
        let Some(source_resource) = self.resources.get(&source) else {
            return Err(3);
        };
        if source_resource.child.is_some() {
            return Err(7);
        }
        let cloned = source_resource.clone();
        self.next_handle = self.next_handle.saturating_add(1).max(1);
        let handle = self.next_handle;
        self.resources.insert(
            handle,
            BurikoMovieResource {
                archive: cloned.archive,
                resource: cloned.resource,
                bytes: cloned.bytes,
                metadata: cloned.metadata,
                parent: Some(source),
                child: None,
            },
        );
        self.resources
            .get_mut(&source)
            .expect("source resource disappeared")
            .child = Some(handle);
        Ok(handle)
    }

    pub(crate) fn resource(&self, handle: i32) -> Option<&BurikoMovieResource> {
        self.resources.get(&handle)
    }
}
