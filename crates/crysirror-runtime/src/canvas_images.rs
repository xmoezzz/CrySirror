// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Presenter-independent Canvas capture admission and worker-owned readback.
//! The VM holds only an ImageJob. GPU capabilities stay with the presenter.
use crate::{
    Runtime,
    assets::{Assets, Surface},
    image_processing::{self, Charge},
};
use crysirror_core::{Error, Result, image_processing::*, service::AssetReader, state::ClientId};
use crysirror_scene::{
    canvas::MaterializeRequest,
    layer::{LayerSpec, Paint},
};
use crysirror_tasks::{Task, TaskPermit, TaskPool};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

type Reply = std::result::Result<Arc<dyn AssetReader>, String>;
type Completion = Mutex<Option<Reply>>;
const MAX_CAPTURES: usize = 16;
// The shared image byte budget still admits every output/staging allocation;
// this geometry cap allows 4K regions without promising unbounded readbacks.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
struct TargetTransfer {
    counter: Arc<AtomicUsize>,
    bytes: usize,
}
impl TargetTransfer {
    fn reserve(counter:Arc<AtomicUsize>,bytes:usize)->Result<Self> {
        let limit = if cfg!(target_os="vita") {32} else {128} *1024*1024;
        counter.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|n.checked_add(bytes).filter(|n|*n<=limit))
            .map_err(|_|Error::Limit("target capture working set"))?;
        Ok(Self{counter,bytes})
    }
}
impl Drop for TargetTransfer {
    fn drop(&mut self) {self.counter.fetch_sub(self.bytes,Ordering::AcqRel);}
}
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

struct TargetImageCapture {
    target: crysirror_scene::layer::render_target::RenderTargetBinding,
    store: u64,
    published: BTreeMap<(ClientId, String), Surface>,
    text: crysirror_text::TextSystem,
    response: Weak<Completion>,
}
impl TargetImageCapture {
    fn complete(&self, reply: Reply) {
        if let Some(response) = self.response.upgrade() {
            if let Ok(mut value) = response.lock() {
                if value.is_none() { *value = Some(reply); }
            }
        }
    }
    fn alive(&self) -> bool { self.response.strong_count() != 0 }
}
impl Drop for TargetImageCapture {
    fn drop(&mut self) { self.complete(Err("target image presenter abandoned request".into())); }
}

struct FrameCapture {
    response: Weak<Completion>,
}
impl Drop for FrameCapture {
    fn drop(&mut self) {
        if let Some(response) = self.response.upgrade() {
            if let Ok(mut value) = response.lock() {
                if value.is_none() {
                    *value = Some(Err("frame capture presenter abandoned request".into()));
                }
            }
        }
    }
}
/// Presenter pixels, read on the engine worker like a Canvas capture.
struct FrameReader(Vec<u8>);
impl AssetReader for FrameReader {
    fn len(&self) -> Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<usize> {
        let start = usize::try_from(offset).map_err(error)?.min(self.0.len());
        let count = out.len().min(self.0.len() - start);
        out[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

pub(crate) struct Broker {
    enabled: bool,
    store: u64,
    pending: VecDeque<CanvasImageCapture>,
    frames: Vec<FrameCapture>,
    targets: VecDeque<TargetImageCapture>,
    target_transfers: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
}
impl Broker {
    pub(crate) fn new(store: u64) -> Self {
        Self {
            enabled: false,
            store,
            pending: VecDeque::new(),
            frames: Vec::new(),
            targets: VecDeque::new(),
            target_transfers: Arc::new(AtomicUsize::new(0)),
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
        validate_layers(&request.layers, &mut BTreeMap::new(), 0)?;
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
            target_transfer: None,
        }))
    }
}

impl Broker {
    pub(crate) fn begin_target(
        &mut self,
        owner: ClientId,
        target: crysirror_scene::layer::render_target::RenderTargetBinding,
        published: BTreeMap<(ClientId, String), Surface>,
        text: &crysirror_text::TextSystem,
        tasks: &TaskPool,
        memory: Arc<AtomicUsize>,
    ) -> Result<ImageJob> {
        if !self.enabled { return Err(Error::Unsupported("target image presenter")); }
        if target.owner != owner.0 || target.generation == 0 { return Err(Error::Invalid("target image owner/generation")); }
        target.spec.validate().map_err(error)?;
        self.targets.retain(TargetImageCapture::alive);
        let size = target.spec.size;
        let bytes = usize::try_from(u64::from(size[0])*u64::from(size[1])*4).map_err(error)?;
        let transfer = TargetTransfer::reserve(self.target_transfers.clone(),bytes*6+size[1] as usize*256)?;
        let output = Charge::new(memory.clone(),bytes)?;
        // Frozen GPU copy, padded mapping and CPU readback/worker buffers can
        // coexist. This is additional to the live target's own reservation.
        let scratch = Charge::new(memory,bytes*5+size[1] as usize*256)?;
        let permit = tasks.reserve().map_err(error)?;
        let text = text.frozen().map_err(error)?;
        self.count.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<MAX_CAPTURES).then_some(n+1))
            .map_err(|_|Error::Limit("Canvas capture jobs"))?;
        let lease = Admission(self.count.clone());
        let response = Arc::new(Mutex::new(None));
        self.targets.push_back(TargetImageCapture {target,store:self.store,published,text,response:Arc::downgrade(&response)});
        Ok(Box::new(Job {owner,size,response,admission:Some((permit,output,scratch)),task:None,state:ImageProcessState::Pending,_lease:lease,target_transfer:Some(transfer)}))
    }
    pub(crate) fn begin_frame(
        &mut self,
        owner: ClientId,
        size: [u32; 2],
        tasks: &TaskPool,
        memory: Arc<AtomicUsize>,
    ) -> Result<ImageJob> {
        if !self.enabled {
            return Err(Error::Unsupported("frame capture presenter"));
        }
        let pixels = u64::from(size[0]) * u64::from(size[1]);
        if size[0] == 0 || size[1] == 0 || pixels > MAX_PIXELS {
            return Err(Error::Limit("frame capture geometry"));
        }
        let bytes = usize::try_from(pixels * 4).map_err(error)?;
        let output = Charge::new(memory.clone(), bytes)?;
        let scratch = Charge::new(memory, bytes * 2)?;
        let permit = tasks.reserve().map_err(error)?;
        self.count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CAPTURES).then_some(n + 1)
            })
            .map_err(|_| Error::Limit("Canvas capture jobs"))?;
        let lease = Admission(self.count.clone());
        let response = Arc::new(Mutex::new(None));
        self.frames.push(FrameCapture {
            response: Arc::downgrade(&response),
        });
        Ok(Box::new(Job {
            owner,
            size,
            response,
            admission: Some((permit, output, scratch)),
            task: None,
            state: ImageProcessState::Pending,
            _lease: lease,
            target_transfer: None,
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
fn validate_layers(layers: &[LayerSpec], pages: &mut BTreeMap<usize, usize>, depth: usize) -> Result<usize> {
    if depth > 16 {
        return Err(Error::Limit("Canvas capture nesting"));
    }
    if layers.len() > crysirror_scene::layer::MAX_LAYERS {
        return Err(Error::Limit("Canvas capture page nodes"));
    }
    let mut child_depth = 0;
    for layer in layers {
        if let Some(filter) = &layer.filter {
            if let Some(tone) = &filter.tone_curve {
                validate_asset(&tone.asset)?;
            }
            if let Some(mask) = &filter.mask {
                validate_asset(&mask.asset)?;
            }
        }
        let paint=match &layer.paint { Paint::CoverageWrite{paint}=>paint.as_ref(), other=>other };
        match paint {
            Paint::CoverageWrite{..}=>return Err(Error::Unsupported("nested coverage write in Canvas capture")),
            Paint::GlyphView{..}=>return Err(Error::Unsupported("unresolved glyph view in Canvas capture")),
            Paint::CoverageReset{..}=>(),
            Paint::Image { asset, .. } => validate_asset(asset)?,
            Paint::Snapshot { layers, .. } => {
                let id = Arc::as_ptr(layers) as usize;
                let height = match pages.get(&id) {
                    Some(&height) => height,
                    None => {
                        let height = validate_layers(layers, pages, depth + 1)?;
                        if pages.len() >= crysirror_scene::layer::MAX_LAYERS { return Err(Error::Limit("distinct Canvas capture pages")); }
                        pages.insert(id, height);
                        height
                    }
                };
                child_depth = child_depth.max(1 + height);
                if depth + child_depth > 16 { return Err(Error::Limit("Canvas capture nesting")); }
            }
            Paint::Mesh { frame, .. } => {
                for asset in &frame.textures {
                    validate_asset(asset)?;
                }
            }
            Paint::ProjectedMesh { frame, .. } => {
                for part in &frame.parts {
                    if let Some(asset) = &part.texture { validate_asset(asset)?; }
                }
            }
            Paint::LegacyMesh { spec } => {
                for asset in spec.textures.iter().flatten() { validate_asset(asset)?; }
            }
            Paint::Backdrop { .. } => return Err(Error::Unsupported("Backdrop in a Canvas capture")),
            Paint::Puppet { .. } => return Err(Error::Unsupported("live Puppet Canvas capture")),
            Paint::Group { .. } | Paint::OrderedState { .. } | Paint::Solid { .. } | Paint::Text { .. } | Paint::GlyphRun { .. } => (),
        }
    }
    Ok(child_depth)
}

#[cfg(test)]
mod capture_budget_tests {
    use super::*;
    #[test]
    fn nested_canvas_pages_share_a_bounded_scene_budget() {
        let leaf: LayerSpec = serde_json::from_value(serde_json::json!({
            "id": 1, "rect":[0,0,1,1], "paint":{"kind":"solid","color":[0,0,0,0]}
        })).unwrap();
        let mut page = leaf.clone();
        let mut children = vec![leaf.clone()];
        children.extend((2..=2100).map(|id| {
            let mut child = leaf.clone();
            child.id = id;
            child.parent = Some(1);
            child
        }));
        page.paint = Paint::Snapshot { size:[1,1], layers:children.into() };
        let mut visited = BTreeMap::new();
        validate_layers(&vec![page.clone(); 12], &mut visited, 0).unwrap();
        assert_eq!(visited.len(), 1);
        assert!(validate_layers(&[page], &mut visited, 16).is_err());
        assert!(validate_layers(&vec![leaf; crysirror_scene::layer::MAX_LAYERS+1], &mut visited, 0).is_err());
    }
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
    target_transfer: Option<TargetTransfer>,
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
            match task.poll_owner() {
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
        // The completed immutable output keeps its ordinary resource charge;
        // transfer staging and GPU-copy admission end at this worker fence.
        if !matches!(self.state,ImageProcessState::Pending) {self.target_transfer=None;}
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
            for work in self.host.canvas_images.targets.drain(..) {
                work.complete(Err("target image presenter removed".into()));
            }
        }
    }
    /// Presenter work only; pruning cancelled jobs also releases their frozen
    /// resource leases even if no subsequent target capture is requested.
    pub fn has_pending_target_images(&mut self) -> bool {
        self.host.canvas_images.targets.retain(TargetImageCapture::alive);
        !self.host.canvas_images.targets.is_empty()
    }
    /// Freeze GPU copies before a later frame can overwrite the source. The
    /// returned reader maps on the ImageJob worker, never on the VM owner.
    pub fn prepare_target_images(
        &mut self,
        mut prepare: impl FnMut(&crysirror_scene::layer::render_target::RenderTargetBinding,&mut Assets)
            -> anyhow::Result<Option<Arc<dyn AssetReader>>>,
    ) -> anyhow::Result<()> {
        let requests = std::mem::take(&mut self.host.canvas_images.targets);
        for work in requests {
            if !work.alive() { continue; }
            let current = self.host.engine.layers.render_target(ClientId(work.target.owner),work.target.id);
            if current.is_none_or(|t|t.generation!=work.target.generation) {
                work.complete(Err("target image source deleted or replaced".into()));
                continue;
            }
            let result = self.assets.with_capture_resources(work.store,work.published.clone(),work.text.clone(),
                |assets|prepare(&work.target,assets));
            match result {
                Ok(Some(reader)) => work.complete(Ok(reader)),
                Ok(None) => self.host.canvas_images.targets.push_back(work),
                Err(error) => work.complete(Err(format!("target image: {error:#}"))),
            }
        }
        Ok(())
    }
    /// Complete pending whole-frame captures. `capture` renders the current frame
    /// and returns tightly packed canvas-sized RGBA; it runs only when requested.
    pub fn prepare_frame_captures(
        &mut self,
        capture: impl FnOnce(&mut Self) -> anyhow::Result<Vec<u8>>,
    ) -> anyhow::Result<()> {
        self.host
            .canvas_images
            .frames
            .retain(|f| f.response.strong_count() != 0);
        if self.host.canvas_images.frames.is_empty() {
            return Ok(());
        }
        let result = capture(self);
        let frames = std::mem::take(&mut self.host.canvas_images.frames);
        let reader: Reply = match result {
            Ok(rgba) => Ok(Arc::new(FrameReader(rgba))),
            Err(reason) => Err(format!("frame capture: {reason:#}")),
        };
        for frame in &frames {
            if let Some(response) = frame.response.upgrade() {
                if let Ok(mut value) = response.lock() {
                    *value = Some(reader.clone());
                }
            }
        }
        Ok(())
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
