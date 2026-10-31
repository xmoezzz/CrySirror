// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Presenter-independent Canvas capture admission and worker-owned readback.
//! The VM holds only an ImageJob. GPU capabilities stay with the presenter.
use crate::{
    assets::{Assets, Surface},
    image_processing::{self, Charge},
    Runtime,
};
use crysirror_core::{image_processing::*, service::AssetReader, state::ClientId, Error, Result};
use crysirror_scene::{
    canvas::MaterializeRequest,
    layer::{LayerSpec, Paint},
};
use crysirror_tasks::{Task, TaskPermit, TaskPool};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};

type Reply = std::result::Result<Arc<dyn AssetReader>, String>;
type Completion = Mutex<Option<Reply>>;
const MAX_CAPTURES: usize = 16;
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
fn error(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}

/// Frozen logical drawing request. Resource lookup during presenter preparation
/// is scoped by Runtime to the image/font generations admitted with this request.
pub struct CanvasImageCapture {
    request: MaterializeRequest,
    store: u64,
    published: BTreeMap<(ClientId, String), Surface>,
    text: crysirror_text::TextSystem,
    response: Weak<Completion>,
}
impl CanvasImageCapture {
    pub fn request(&self) -> &MaterializeRequest {
        &self.request
    }
    fn complete(&self, reply: Reply) {
        if let Some(response) = self.response.upgrade() {
            if let Ok(mut value) = response.lock() {
                if value.is_none() {
                    *value = Some(reply);
                }
            }
        }
    }
    fn alive(&self) -> bool {
        self.response.strong_count() != 0
    }
}
impl Drop for CanvasImageCapture {
    fn drop(&mut self) {
        self.complete(Err("Canvas image presenter abandoned request".into()));
    }
}

pub(crate) struct Broker {
    enabled: bool,
    store: u64,
    pending: VecDeque<CanvasImageCapture>,
    count: Arc<AtomicUsize>,
}
impl Broker {
    pub(crate) fn new(store: u64) -> Self {
        Self {
            enabled: false,
            store,
            pending: VecDeque::new(),
            count: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub(crate) fn begin(
        &mut self,
        owner: ClientId,
        request: MaterializeRequest,
        published: BTreeMap<(ClientId, String), Surface>,
        text: &crysirror_text::TextSystem,
        tasks: &TaskPool,
        memory: Arc<AtomicUsize>,
    ) -> Result<ImageJob> {
        if !self.enabled {
            return Err(Error::Unsupported("Canvas image capture presenter"));
        }
        self.pending.retain(CanvasImageCapture::alive);
        let [width, height] = request.size;
        let pixels = u64::from(width) * u64::from(height);
        if request.client != owner.0 || width == 0 || height == 0 || pixels > MAX_PIXELS {
            return Err(Error::Limit("Canvas capture geometry"));
        }
        let mut nodes = 0;
        validate_layers(&request.layers, &mut nodes, 0)?;
        let bytes = usize::try_from(pixels * 4).map_err(error)?;
        let output = Charge::new(memory.clone(), bytes)?;
        // GPU readback (including row padding), staging Vec and final Arc can
        // coexist. The common image budget bounds outstanding captures too.
        let scratch = Charge::new(memory, bytes * 4 + height as usize * 256)?;
        let permit = tasks.reserve().map_err(error)?;
        let text = text.frozen().map_err(error)?;
        self.count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CAPTURES).then_some(n + 1)
            })
            .map_err(|_| Error::Limit("Canvas capture jobs"))?;
        let lease = Admission(self.count.clone());
        let response = Arc::new(Mutex::new(None));
        self.pending.push_back(CanvasImageCapture {
            request,
            store: self.store,
            published,
            text,
            response: Arc::downgrade(&response),
        });
        Ok(Box::new(Job {
            owner,
            size: [width, height],
            response,
            admission: Some((permit, output, scratch)),
            task: None,
            state: ImageProcessState::Pending,
            _lease: lease,
        }))
    }
}

// Snapshot and Mesh are immutable; live Puppet evaluation and replaceable
// surface/frame-history names have no admission-time resource freeze here.
// Reject them explicitly instead of capturing a later mutable generation.
fn validate_asset(asset: &str) -> Result<()> {
    if asset.starts_with("__surface/")
        || asset.starts_with("__crysirror/frame-history/")
        || asset.starts_with("__frame_history/")
        || asset.starts_with("__mesh_history/")
    {
        return Err(Error::Unsupported(
            "mutable Canvas capture image dependency",
        ));
    }
    Ok(())
}
fn validate_layers(layers: &[LayerSpec], nodes: &mut usize, depth: usize) -> Result<()> {
    if depth > 16 {
        return Err(Error::Limit("Canvas capture nesting"));
    }
    for layer in layers {
        *nodes += 1;
        if *nodes > 4096 {
            return Err(Error::Limit("Canvas capture nodes"));
        }
        if let Some(filter) = &layer.filter {
            if let Some(tone) = &filter.tone_curve {
                validate_asset(&tone.asset)?;
            }
            if let Some(mask) = &filter.mask {
                validate_asset(&mask.asset)?;
            }
        }
        match &layer.paint {
            Paint::Image { asset, .. } => validate_asset(asset)?,
            Paint::Snapshot { layers, .. } => validate_layers(layers, nodes, depth + 1)?,
            Paint::Mesh { frame, .. } => {
                for asset in &frame.textures {
                    validate_asset(asset)?;
                }
            }
            Paint::Puppet { .. } => return Err(Error::Unsupported("live Puppet Canvas capture")),
            Paint::Solid { .. } | Paint::Text { .. } | Paint::GlyphRun { .. } => (),
        }
    }
    Ok(())
}
struct Admission(Arc<AtomicUsize>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Job {
    owner: ClientId,
    size: [u32; 2],
    response: Arc<Completion>,
    admission: Option<(TaskPermit, Charge, Charge)>,
    task: Option<Task<Arc<dyn ImageResource>>>,
    state: ImageProcessState,
    _lease: Admission,
}
impl ImageProcess for Job {
    fn poll(&mut self) -> Result<ImageProcessState> {
        if self.admission.is_some() {
            let reply = self
                .response
                .lock()
                .map_err(|_| Error::Invalid("Canvas capture completion lock"))?
                .take();
            if let Some(reply) = reply {
                let (permit, output, scratch) = self.admission.take().unwrap();
                match reply {
                    Err(reason) => {
                        self.state = ImageProcessState::Failed(reason);
                    }
                    Ok(reader) => {
                        let owner = self.owner;
                        let size = self.size;
                        match permit.spawn(move |cancel| {
                            let _scratch = scratch;
                            cancel.check()?;
                            let length = crysirror_image::rgba_len(size[0], size[1])?;
                            anyhow::ensure!(
                                reader.len().map_err(error)? == length as u64,
                                "Canvas readback byte length"
                            );
                            let mut rgba = vec![0; length];
                            let mut offset = 0;
                            while offset < length {
                                cancel.check()?;
                                let end = (offset + 65536).min(length);
                                let n = reader
                                    .read_at(offset as u64, &mut rgba[offset..end])
                                    .map_err(error)?;
                                anyhow::ensure!(
                                    n > 0 && n <= end - offset,
                                    "Canvas readback short or invalid read"
                                );
                                offset += n;
                            }
                            cancel.check()?;
                            let surface = crysirror_image::ImageSurface {
                                width: size[0],
                                height: size[1],
                                rgba: rgba.into(),
                            };
                            image_processing::resource_from_surface(owner, surface, output, &|| {
                                cancel.check()
                            })
                        }) {
                            Ok(task) => self.task = Some(task),
                            Err(reason) => {
                                self.state = ImageProcessState::Failed(reason.to_string())
                            }
                        }
                    }
                }
            }
        }
        if let Some(task) = &mut self.task {
            match task.try_take() {
                Ok(None) => (),
                Ok(Some(resource)) => {
                    self.state = ImageProcessState::Ready(resource);
                    self.task = None;
                }
                Err(reason) => {
                    self.state = ImageProcessState::Failed(reason.to_string());
                    self.task = None;
                }
            }
        }
        Ok(self.state.clone())
    }
}
impl Runtime {
    /// Install/remove the host presenter capability before ticking VMs. A
    /// device-free host defaults to an explicit unsupported service.
    pub fn set_canvas_image_presenter(&mut self, enabled: bool) {
        self.host.canvas_images.enabled = enabled;
        if !enabled {
            for work in self.host.canvas_images.pending.drain(..) {
                work.complete(Err("Canvas image presenter removed".into()));
            }
        }
    }
    /// Prepare admitted immutable requests on the presenter owner. Returning
    /// None retries loading dependencies next frame; a reader is consumed only
    /// on an engine worker. The closure must not perform a blocking readback.
    pub fn prepare_canvas_images(
        &mut self,
        mut prepare: impl FnMut(
            &MaterializeRequest,
            &mut Assets,
        ) -> anyhow::Result<Option<Arc<dyn AssetReader>>>,
    ) -> anyhow::Result<()> {
        let requests = std::mem::take(&mut self.host.canvas_images.pending);
        for work in requests {
            if !work.alive() {
                continue;
            }
            let result = self.assets.with_capture_resources(
                work.store,
                work.published.clone(),
                work.text.clone(),
                |assets| prepare(&work.request, assets),
            );
            match result {
                Ok(Some(reader)) => work.complete(Ok(reader)),
                Ok(None) => {
                    self.host.canvas_images.pending.push_back(work);
                }
                Err(reason) => work.complete(Err(format!("Canvas capture: {reason:#}"))),
            }
        }
        Ok(())
    }
}
