// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Background scoped reads/imports. Only poll/resolve commits results to Assets.
use super::Assets;
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
    state::ClientId,
};
use crysirror_tasks::{Cancellation, Task, TaskPool};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
static NEXT_STORE: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Bytes,
    Image,
    Model,
    /// Baked CPU world entities for non-PBR level streaming.
    Scene,
    /// PBR level hierarchy and decoded model prepared together.
    Level,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetState {
    Loading,
    Ready,
    Failed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AssetTicket {
    pub store: u64,
    pub client: u32,
    pub id: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct AssetStatus {
    pub ticket: AssetTicket,
    pub path: String,
    pub kind: AssetKind,
    pub state: AssetState,
    pub error: Option<String>,
    pub bytes: usize,
}
enum Prepared {
    Level(
        Arc<crysirror_asset::ImportedModel>,
        Arc<crysirror_asset::pbr::PbrModel>,
        usize,
    ),
    Scene(Arc<crysirror_asset::ImportedModel>, usize),
    Bytes(Arc<[u8]>),
    Image(Arc<crysirror_image::ImageSurface>),
    Model(Arc<crysirror_asset::pbr::PbrModel>, usize),
}
impl Prepared {
    fn bytes(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::Image(i) => i.rgba.len(),
            Self::Model(_, bytes) | Self::Scene(_, bytes) | Self::Level(_, _, bytes) => *bytes,
        }
    }
}
/// A scoped pin; releasing the final pin allows managed streaming cache eviction.
pub struct AssetLease {
    ticket: AssetTicket,
    pins: Arc<AtomicUsize>,
}
impl AssetLease {
    pub fn ticket(&self) -> AssetTicket {
        self.ticket
    }
}
impl Drop for AssetLease {
    fn drop(&mut self) {
        self.pins.fetch_sub(1, Ordering::AcqRel);
    }
}
struct Entry {
    pins: Arc<AtomicUsize>,
    automatic: bool,
    leased: bool,
    touched: std::time::Instant,
    status: AssetStatus,
    job: Option<Task<Prepared>>,
    value: Option<Prepared>,
}
pub(super) struct AsyncAssets {
    budget: usize,
    store: u64,
    next: u64,
    io: TaskPool,
    cpu: TaskPool,
    entries: BTreeMap<u64, Entry>,
    keys: BTreeMap<(ClientId, AssetKind, String), u64>,
    epochs: BTreeMap<ClientId, u64>,
    pub nonblocking: bool,
    resident: usize,
}
impl AsyncAssets {
    pub fn new() -> Result<Self> {
        let cpu = std::thread::available_parallelism()
            .map_or(2, usize::from)
            .saturating_sub(2)
            .clamp(2, 4);
        Ok(Self {
            budget: 256 * 1024 * 1024,
            store: NEXT_STORE.fetch_add(1, Ordering::Relaxed),
            next: 1,
            io: TaskPool::new("assets", 2, 64)?,
            cpu: TaskPool::new("cpu", cpu, 256)?,
            entries: BTreeMap::new(),
            keys: BTreeMap::new(),
            epochs: BTreeMap::new(),
            nonblocking: false,
            resident: 0,
        })
    }
    fn entry(&self, client: ClientId, t: AssetTicket) -> Result<&Entry> {
        ensure!(
            t.store == self.store && t.client == client.0,
            "foreign asset ticket"
        );
        let entry = self.entries.get(&t.id).context("asset ticket released")?;
        ensure!(entry.status.ticket == t, "foreign asset ticket");
        Ok(entry)
    }
    fn finish(&mut self, id: u64, result: Result<Prepared>) {
        let e = self.entries.get_mut(&id).unwrap();
        match result {
            Ok(value) => {
                let bytes = value.bytes();
                if bytes > self.budget
                    || self
                        .resident
                        .checked_add(bytes)
                        .is_none_or(|n| n > self.budget)
                {
                    e.status.state = AssetState::Failed;
                    e.status.error = Some(format!(
                        "prepared assets exceed {} MiB budget",
                        self.budget / (1024 * 1024)
                    ));
                } else {
                    self.resident += bytes;
                    e.status.bytes = bytes;
                    e.status.state = AssetState::Ready;
                    e.value = Some(value);
                }
            }
            Err(error) => {
                e.status.state = AssetState::Failed;
                e.status.error = Some(format!("{error:#}"));
            }
        }
    }
    fn poll(&mut self) {
        let ids: Vec<_> = self.entries.keys().copied().collect();
        for id in ids {
            let entry = self.entries.get_mut(&id).unwrap();
            let Some(job) = &mut entry.job else { continue };
            let result = job.try_take();
            if !matches!(result, Ok(None)) {
                entry.job = None;
                self.finish(id, result.and_then(|v| v.context("asset result absent")));
            }
        }
    }
    pub fn invalidate(&mut self, client: ClientId) {
        let epoch = self.epochs.entry(client).or_default();
        *epoch = epoch.saturating_add(1);
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.status.ticket.client == client.0)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(mut e) = self.entries.remove(&id) {
                if let Some(job) = e.job.take() {
                    job.cancel();
                }
                self.resident -= e.status.bytes;
            }
        }
        self.keys.retain(|(c, _, _), _| *c != client);
    }
}
impl Drop for AsyncAssets {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            if let Some(job) = &entry.job {
                job.cancel();
            }
        }
    }
}
#[derive(Clone)]
struct Source {
    storage: Option<Arc<dyn ClientStorage>>,
    dir: Arc<cap_std::fs::Dir>,
    cache: BTreeMap<String, Arc<[u8]>>,
}
impl Source {
    fn read(&self, path: &str, cancel: &Cancellation) -> Result<Arc<[u8]>> {
        cancel.check()?;
        let p = VfsPath::new(Mount::Asset, path)?;
        if let Some(data) = self.cache.get(p.relative()) {
            ensure!(data.len() <= 64 * 1024 * 1024, "async asset exceeds 64 MiB");
            return Ok(data.clone());
        }
        let bytes = if let Some(storage) = &self.storage {
            let reader = storage.open_asset(&p)?;
            let len = reader.len()?;
            ensure!(len <= 64 * 1024 * 1024, "async asset exceeds 64 MiB");
            let mut out = vec![0; len as usize];
            let mut at = 0;
            while at < out.len() {
                cancel.check()?;
                let end = (at + 256 * 1024).min(out.len());
                let n = reader.read_at(at as u64, &mut out[at..end])?;
                ensure!(
                    n > 0 && n <= end - at,
                    "asset reader truncated or exceeded requested bytes"
                );
                at += n;
            }
            out
        } else {
            super::read_limited(&self.dir, p.relative())?
        };
        cancel.check()?;
        Ok(Arc::from(bytes))
    }
}
fn load_model(
    source: &Source,
    name: &str,
    bytes: &[u8],
    cancel: &Cancellation,
    decoders: &crysirror_image::DecoderRegistry,
) -> Result<crysirror_asset::pbr::PbrModel> {
    let dir = name
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    let model = crysirror_asset::pbr::load_pbr_with_decoder(
        bytes,
        |uri| {
            source
                .read(&format!("{dir}{uri}"), cancel)
                .map(|b| b.to_vec())
        },
        |uri, data| {
            let dependency = |p: &str| {
                source
                    .read(p, cancel)
                    .map(|b| b.to_vec())
                    .map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))
            };
            decoders.decode_with(&format!("{dir}{uri}"), data, Some(&dependency), None, None)
        },
    )?;
    Ok(model)
}
impl Assets {
    /// Desktop uses nonblocking loading; captures/offline tools explicitly wait.
    pub fn set_nonblocking(&mut self, value: bool) {
        self.asynchronous.nonblocking = value;
    }
    pub fn is_nonblocking(&self) -> bool {
        self.asynchronous.nonblocking
    }
    /// Host policy only: bounded CPU prepared-cache budget, not total RAM/VRAM.
    pub fn set_prepared_budget(&mut self, bytes: usize) -> Result<()> {
        ensure!(
            (16 * 1024 * 1024..=4u64 * 1024 * 1024 * 1024).contains(&(bytes as u64))
                && bytes >= self.asynchronous.resident,
            "invalid prepared asset budget or below live residency"
        );
        self.asynchronous.budget = bytes;
        Ok(())
    }
    pub fn store_id(&self) -> u64 {
        self.asynchronous.store
    }
    /// Bounded blocking-I/O executor; CPU continuations are polled by owners.
    pub fn io_tasks(&self) -> TaskPool {
        self.asynchronous.io.clone()
    }
    pub fn cpu_tasks(&self) -> TaskPool {
        self.asynchronous.cpu.clone()
    }
    pub fn revision(&self, client: ClientId) -> u64 {
        self.asynchronous.epochs.get(&client).copied().unwrap_or(0)
    }
    fn source(&self, client: ClientId) -> Result<Source> {
        let dir = self
            .clients
            .get(&client)
            .context("unknown asset client")?
            .try_clone()?;
        let prefix = format!("client:{}:", client.0);
        Ok(Source {
            storage: self.storage.get(&client).cloned(),
            dir: Arc::new(dir),
            cache: self
                .bytes
                .iter()
                .filter_map(|(key, value)| {
                    key.strip_prefix(&prefix).map(|p| (p.into(), value.clone()))
                })
                .collect(),
        })
    }
    /// Spawn an independently awaitable capability-scoped read. No main-thread I/O.
    pub fn read_async(&self, client: ClientId, path: &str) -> Result<Task<Arc<[u8]>>> {
        let source = self.source(client)?;
        let path = VfsPath::new(Mount::Asset, path)?.relative().to_owned();
        self.asynchronous
            .io
            .spawn(move |cancel| source.read(&path, &cancel))
    }
    /// Request deduplicated background bytes, image decoding or glTF import.
    pub fn request(
        &mut self,
        client: ClientId,
        path: &str,
        kind: AssetKind,
    ) -> Result<AssetTicket> {
        self.request_inner(client, path, kind, true)
    }
    pub fn lease(&mut self, client: ClientId, path: &str, kind: AssetKind) -> Result<AssetLease> {
        let ticket = self.request_inner(client, path, kind, false)?;
        let e = self.asynchronous.entries.get_mut(&ticket.id).unwrap();
        e.leased = true;
        e.pins.fetch_add(1, Ordering::AcqRel);
        Ok(AssetLease {
            ticket,
            pins: e.pins.clone(),
        })
    }
    fn request_inner(
        &mut self,
        client: ClientId,
        path: &str,
        kind: AssetKind,
        explicit: bool,
    ) -> Result<AssetTicket> {
        let path = VfsPath::new(Mount::Asset, path)?.relative().to_owned();
        self.key(client, &path)?;
        let key = (client, kind, path.clone());
        if let Some(id) = self.asynchronous.keys.get(&key) {
            let entry = self.asynchronous.entries.get_mut(id).unwrap();
            entry.touched = std::time::Instant::now();
            if explicit {
                entry.automatic = false;
            }
            return Ok(entry.status.ticket);
        }
        ensure!(
            self.asynchronous.entries.len() < 4096,
            "async asset request limit; release unused tickets"
        );
        if self
            .asynchronous
            .entries
            .values()
            .filter(|e| e.status.ticket.client == client.0 && e.status.state == AssetState::Loading)
            .count()
            >= 32
        {
            return Err(crysirror_tasks::CapacityExceeded.into());
        }
        let source = self.source(client)?;
        let decoders = self.decoders.clone();
        let name = path.clone();
        let profiler = self.profiler.clone();
        let frame = profiler.frame();
        let job = self.asynchronous.io.spawn(move |cancel| {
            let _span = profiler.span(frame, format!("asset.{kind:?}.read_prepare"));
            let bytes = source.read(&name, &cancel)?;
            match kind {
                AssetKind::Bytes => Ok(Prepared::Bytes(bytes)),
                AssetKind::Image => {
                    let dependencies = |uri: &str| {
                        source
                            .read(uri, &cancel)
                            .map(|b| b.to_vec())
                            .map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))
                    };
                    let image =
                        decoders.decode_with(&name, &bytes, Some(&dependencies), None, None)?;
                    cancel.check()?;
                    Ok(Prepared::Image(Arc::new(image)))
                }
                AssetKind::Scene | AssetKind::Level => {
                    let dir = name
                        .rsplit_once('/')
                        .map(|(d, _)| format!("{d}/"))
                        .unwrap_or_default();
                    let options = crysirror_asset::ImportOptions {
                        reference_asset: (kind == AssetKind::Level).then(|| name.clone()),
                        ..Default::default()
                    };
                    let scene = crysirror_asset::import_gltf(&bytes, &options, |uri| {
                        source
                            .read(&format!("{dir}{uri}"), &cancel)
                            .map(|b| b.to_vec())
                    })?;
                    cancel.check()?;
                    let accounted = serde_json::to_vec(&scene.entities)?.len();
                    if kind == AssetKind::Level {
                        let model = load_model(&source, &name, &bytes, &cancel, &decoders)?;
                        let size = accounted + model_bytes(&model);
                        Ok(Prepared::Level(Arc::new(scene), Arc::new(model), size))
                    } else {
                        Ok(Prepared::Scene(Arc::new(scene), accounted))
                    }
                }
                AssetKind::Model => {
                    let model = load_model(&source, &name, &bytes, &cancel, &decoders)?;
                    cancel.check()?;
                    let accounted = model_bytes(&model);
                    Ok(Prepared::Model(Arc::new(model), accounted))
                }
            }
        })?;
        let id = self.asynchronous.next;
        self.asynchronous.next = id.checked_add(1).context("asset ticket overflow")?;
        let ticket = AssetTicket {
            store: self.asynchronous.store,
            client: client.0,
            id,
        };
        self.asynchronous.entries.insert(
            id,
            Entry {
                pins: Arc::new(AtomicUsize::new(0)),
                automatic: !explicit,
                leased: false,
                touched: std::time::Instant::now(),
                status: AssetStatus {
                    ticket,
                    path,
                    kind,
                    state: AssetState::Loading,
                    error: None,
                    bytes: 0,
                },
                job: Some(job),
                value: None,
            },
        );
        self.asynchronous.keys.insert(key, id);
        Ok(ticket)
    }
    pub(super) fn prepared_bytes(&self, client: ClientId, path: &str) -> Option<Arc<[u8]>> {
        let id = self
            .asynchronous
            .keys
            .get(&(client, AssetKind::Bytes, path.to_owned()))?;
        match self.asynchronous.entries.get(id)?.value.as_ref()? {
            Prepared::Bytes(b) => Some(b.clone()),
            _ => None,
        }
    }
    pub fn poll_tasks(&mut self) {
        self.asynchronous.poll()
    }
    /// Explicit offline readiness fence. Interactive hosts use task_status.
    pub fn wait_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<AssetStatus> {
        self.asynchronous.entry(client, ticket)?;
        if let Some(job) = self
            .asynchronous
            .entries
            .get_mut(&ticket.id)
            .unwrap()
            .job
            .take()
        {
            let result = job.wait();
            self.asynchronous.finish(ticket.id, result);
        }
        self.task_status(client, ticket)
    }
    pub fn task_status(&mut self, client: ClientId, ticket: AssetTicket) -> Result<AssetStatus> {
        self.poll_tasks();
        Ok(self.asynchronous.entry(client, ticket)?.status.clone())
    }
    pub fn ticket(&self, client: ClientId, id: u64) -> Result<AssetTicket> {
        let ticket = self
            .asynchronous
            .entries
            .get(&id)
            .context("unknown asset task")?
            .status
            .ticket;
        self.asynchronous.entry(client, ticket)?;
        Ok(ticket)
    }
    pub fn cancel_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<()> {
        self.asynchronous.entry(client, ticket)?;
        let e = self.asynchronous.entries.get_mut(&ticket.id).unwrap();
        ensure!(
            e.status.state == AssetState::Loading,
            "only loading tasks can be cancelled"
        );
        if let Some(job) = e.job.take() {
            job.cancel();
        }
        e.status.state = AssetState::Cancelled;
        Ok(())
    }
    pub fn release_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<()> {
        self.asynchronous.entry(client, ticket)?;
        ensure!(
            self.asynchronous.entries[&ticket.id]
                .pins
                .load(Ordering::Acquire)
                == 0,
            "asset is pinned by a streaming lease"
        );
        let e = self.asynchronous.entries.remove(&ticket.id).unwrap();
        self.asynchronous.resident -= e.status.bytes;
        self.asynchronous
            .keys
            .remove(&(client, e.status.kind, e.status.path));
        Ok(())
    }
    fn resolve(&mut self, client: ClientId, ticket: AssetTicket) -> Result<Option<&Prepared>> {
        self.poll_tasks();
        self.asynchronous.entry(client, ticket)?;
        if !self.asynchronous.nonblocking {
            if let Some(job) = self
                .asynchronous
                .entries
                .get_mut(&ticket.id)
                .unwrap()
                .job
                .take()
            {
                let result = job.wait();
                self.asynchronous.finish(ticket.id, result);
            }
        }
        let entry = self.asynchronous.entry(client, ticket)?;
        match entry.status.state {
            AssetState::Ready => Ok(entry.value.as_ref()),
            AssetState::Loading => Ok(None),
            _ => anyhow::bail!(
                "asset {} {:?}: {}",
                entry.status.path,
                entry.status.state,
                entry.status.error.as_deref().unwrap_or("cancelled")
            ),
        }
    }
    pub fn image_for(
        &mut self,
        client: ClientId,
        path: &str,
    ) -> Result<Option<Arc<crysirror_image::ImageSurface>>> {
        let ticket = match self.request_inner(client, path, AssetKind::Image, false) {
            Ok(t) => t,
            Err(e) if self.is_nonblocking() && e.is::<crysirror_tasks::CapacityExceeded>() => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        match self.resolve(client, ticket)? {
            Some(Prepared::Image(v)) => Ok(Some(v.clone())),
            None => Ok(None),
            _ => unreachable!(),
        }
    }
    pub fn model_for(
        &mut self,
        client: ClientId,
        path: &str,
    ) -> Result<Option<Arc<crysirror_asset::pbr::PbrModel>>> {
        self.poll_tasks();
        if let Some(id) = self
            .asynchronous
            .keys
            .get(&(client, AssetKind::Level, path.to_owned()))
        {
            if let Some(Prepared::Level(_, model, _)) = self.asynchronous.entries[id].value.as_ref()
            {
                return Ok(Some(model.clone()));
            }
        }
        let ticket = match self.request_inner(client, path, AssetKind::Model, false) {
            Ok(t) => t,
            Err(e) if self.is_nonblocking() && e.is::<crysirror_tasks::CapacityExceeded>() => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        match self.resolve(client, ticket)? {
            Some(Prepared::Model(v, _)) => Ok(Some(v.clone())),
            None => Ok(None),
            _ => unreachable!(),
        }
    }
    /// Consume only a ready CPU import; this never joins a worker.
    pub fn ready_scene(
        &mut self,
        client: ClientId,
        ticket: AssetTicket,
    ) -> Result<Arc<crysirror_asset::ImportedModel>> {
        self.poll_tasks();
        let entry = self.asynchronous.entry(client, ticket)?;
        match entry.value.as_ref() {
            Some(Prepared::Scene(v, _)) | Some(Prepared::Level(v, _, _)) => Ok(v.clone()),
            _ => anyhow::bail!("scene import not ready"),
        }
    }
    /// Evict only automatically managed, unpinned and unshared payloads.
    /// Explicit request IDs remain owned until their caller releases them.
    pub fn collect_unused(&mut self) -> usize {
        let ids: Vec<_> = self
            .asynchronous
            .entries
            .iter()
            .filter(|(_, e)| {
                let shared = match &e.value {
                    Some(Prepared::Bytes(v)) => Arc::strong_count(v) > 1,
                    Some(Prepared::Image(v)) => Arc::strong_count(v) > 1,
                    Some(Prepared::Model(v, _)) => Arc::strong_count(v) > 1,
                    Some(Prepared::Scene(v, _)) => Arc::strong_count(v) > 1,
                    Some(Prepared::Level(v, m, _)) => {
                        Arc::strong_count(v) > 1 || Arc::strong_count(m) > 1
                    }
                    None => false,
                };
                e.automatic
                    && e.pins.load(Ordering::Acquire) == 0
                    && !shared
                    && (e.leased
                        || (e.status.state != AssetState::Loading
                            && e.touched.elapsed().as_secs() >= 5))
            })
            .map(|(id, _)| *id)
            .collect();
        let count = ids.len();
        for id in ids {
            let ticket = self.asynchronous.entries[&id].status.ticket;
            let _ = self.release_task(ClientId(ticket.client), ticket);
        }
        count
    }
    pub fn tasks_inspect(&self) -> serde_json::Value {
        serde_json::json!({"budget_bytes":self.asynchronous.budget,"nonblocking":self.asynchronous.nonblocking,"resident_bytes":self.asynchronous.resident,"io":self.asynchronous.io.stats(),"cpu":self.asynchronous.cpu.stats(),"assets":self.asynchronous.entries.values().map(|e|&e.status).collect::<Vec<_>>()})
    }
}

// Account retained CPU payloads before publishing on the main thread. This is
// cache accounting, not a bound on all transient decoder/driver allocations.
fn model_bytes(m: &crysirror_asset::pbr::PbrModel) -> usize {
    use std::mem::size_of;
    size_of::<crysirror_asset::pbr::PbrModel>()
        + m.images
            .iter()
            .map(|i| i.levels.iter().map(Vec::capacity).sum::<usize>())
            .sum::<usize>()
        + m.meshes
            .iter()
            .flatten()
            .map(|p| {
                p.vertices.capacity() * size_of::<crysirror_asset::pbr::PbrVertex>()
                    + p.indices.capacity() * 4
                    + p.skin_attrs
                        .as_ref()
                        .map_or(0, |a| a.capacity() * size_of::<([u16; 4], [f32; 4])>())
            })
            .sum::<usize>()
        + m.nodes
            .iter()
            .map(|n| {
                size_of::<crysirror_asset::pbr::SceneNode>()
                    + n.name.capacity()
                    + n.children.capacity() * size_of::<usize>()
            })
            .sum::<usize>()
        + m.roots.capacity() * size_of::<usize>()
        + m.skins
            .iter()
            .map(|s| {
                size_of::<crysirror_asset::pbr::Skin>()
                    + s.joints.capacity() * size_of::<usize>()
                    + s.inverse_bind.capacity() * 64
            })
            .sum::<usize>()
        + m.animations
            .iter()
            .map(|c| {
                size_of::<crysirror_asset::pbr::Clip>()
                    + c.name.capacity()
                    + c.channels
                        .iter()
                        .map(|v| {
                            size_of::<crysirror_asset::pbr::Channel>()
                                + v.times.capacity() * 4
                                + v.values.capacity() * 16
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
        + serde_json::to_vec(&m.materials).map_or(0, |v| v.len().saturating_mul(4))
}
