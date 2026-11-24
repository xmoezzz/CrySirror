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
        Arc, Mutex, MutexGuard, Weak,
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
pub(crate) type ImageFacts = ([u32; 2], bool, [u8; 32]);

pub(crate) fn is_tlg(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("tlg"))
}

#[derive(Clone)]
enum Prepared {
    Level(
        Arc<crysirror_asset::ImportedModel>,
        Arc<crysirror_asset::pbr::PbrModel>,
        usize,
    ),
    Scene(Arc<crysirror_asset::ImportedModel>, usize),
    Bytes(Arc<[u8]>),
    Image(Arc<crysirror_image::ImageSurface>, Option<ImageFacts>),
    Model(Arc<crysirror_asset::pbr::PbrModel>, usize),
}
impl Prepared {
    fn bytes(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::Image(i, _) => i.rgba.len(),
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
    touched: web_time::Instant,
    status: AssetStatus,
    // A decoded payload that would fit once other owners release residency.
    // Keep only its size, never the rejected allocation.
    blocked_bytes: Option<usize>,
    job: Option<Task<Prepared>>,
    value: Option<Prepared>,
    image_facts: Option<ImageFacts>,
}
impl Entry {
    fn cache_owned(&self) -> bool {
        self.automatic
            && self.pins.load(Ordering::Acquire) == 0
            && match &self.value {
                Some(Prepared::Bytes(v)) => Arc::strong_count(v) == 1,
                Some(Prepared::Image(v, _)) => Arc::strong_count(v) == 1,
                Some(Prepared::Model(v, _)) => Arc::strong_count(v) == 1,
                Some(Prepared::Scene(v, _)) => Arc::strong_count(v) == 1,
                Some(Prepared::Level(v, m, _)) => {
                    Arc::strong_count(v) == 1 && Arc::strong_count(m) == 1
                }
                None => true,
            }
    }
}
// Host policy for the aggregate prepared payload. Format-specific decode
// limits and residency accounting apply independently.
const DEFAULT_PREPARED_BUDGET: Option<usize> = if cfg!(all(
    target_pointer_width = "64",
    not(target_family = "wasm"),
    any(target_os = "windows", target_os = "macos", target_os = "linux",
        target_os = "ios", target_os = "android")
)) {
    None
} else {
    Some(256 * 1024 * 1024)
};

pub(super) struct AsyncAssets {
    budget: Option<usize>,
    store: u64,
    next: u64,
    io: TaskPool,
    cpu: TaskPool,
    entries: BTreeMap<u64, Entry>,
    keys: BTreeMap<(ClientId, AssetKind, String), u64>,
    epochs: BTreeMap<ClientId, u64>,
    pub(super) storage: BTreeMap<ClientId, Arc<dyn ClientStorage>>,
    image_memory: Arc<AtomicUsize>,
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
            budget: DEFAULT_PREPARED_BUDGET,
            store: NEXT_STORE.fetch_add(1, Ordering::Relaxed),
            next: 1,
            io: TaskPool::new("assets", 2, 64)?,
            cpu: TaskPool::new("cpu", cpu, 256)?,
            entries: BTreeMap::new(),
            keys: BTreeMap::new(),
            epochs: BTreeMap::new(),
            storage: BTreeMap::new(),
            image_memory: Arc::new(AtomicUsize::new(0)),
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
    fn remove_entry(&mut self, id: u64) {
        let entry = self.entries.remove(&id).expect("existing asset entry");
        let status = entry.status;
        self.resident -= status.bytes;
        self.keys
            .remove(&(ClientId(status.ticket.client), status.kind, status.path));
    }

    fn eviction_candidates(&self) -> impl Iterator<Item = (&u64, &Entry)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.status.bytes != 0 && entry.cache_owned())
    }

    fn make_room(&mut self, bytes: usize) -> bool {
        let Some(budget) = self.budget else {
            return self.resident.checked_add(bytes).is_some();
        };
        let Some(limit) = budget.checked_sub(bytes) else {
            return false;
        };
        if self.resident <= limit {
            return true;
        }
        let candidates = self.eviction_candidates();
        let mut victims: Vec<_> = candidates
            .map(|(&id, e)| (e.touched, id, e.status.bytes))
            .collect();
        if victims.iter().map(|v| v.2).sum::<usize>() < self.resident - limit {
            return false;
        }
        victims.sort_unstable();
        for (_, id, _) in victims {
            self.remove_entry(id);
            if self.resident <= limit {
                break;
            }
        }
        true
    }

    fn finish(&mut self, id: u64, result: Result<Prepared>) {
        match result {
            Ok(mut value) => {
                let bytes = value.bytes();
                let fits = self.make_room(bytes);
                let e = self.entries.get_mut(&id).unwrap();
                if let Prepared::Image(_, facts) = &mut value {
                    e.image_facts = facts.take();
                }
                if !fits {
                    e.status.state = AssetState::Failed;
                    e.blocked_bytes = self.budget.filter(|limit| bytes <= *limit).map(|_| bytes);
                    e.status.error = Some(match self.budget {
                        Some(limit) => format!(
                            "prepared assets exceed {} MiB budget (resident={} bytes, incoming={} bytes)",
                            limit / (1024 * 1024),
                            self.resident,
                            bytes
                        ),
                        None => "prepared asset byte accounting overflow".into(),
                    });
                } else {
                    self.resident += bytes;
                    e.status.bytes = bytes;
                    e.status.state = AssetState::Ready;
                    e.value = Some(value);
                }
            }
            Err(error) => {
                let e = self.entries.get_mut(&id).unwrap();
                e.status.state = AssetState::Failed;
                e.status.error = Some(format!("{error:#}"));
            }
        }
    }
    // A lookup must not publish an unrelated completion that evicts the very
    // ticket being inspected. The regular runtime poll services all workers.
    fn poll_one(&mut self, id: u64) {
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        let Some(job) = &mut entry.job else { return };
        let result = job.try_take();
        if !matches!(result, Ok(None)) {
            entry.job = None;
            self.finish(id, result.and_then(|v| v.context("asset result absent")));
        }
    }

    fn poll(&mut self) {
        let ids: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(&id, e)| e.job.as_ref().map(|_| id))
            .collect();
        for id in ids {
            self.poll_one(id);
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
    fn request(
        &mut self,
        client: ClientId,
        path: &str,
        kind: AssetKind,
        explicit: bool,
        decoders: &crysirror_image::DecoderRegistry,
        profiler: &crysirror_tasks::profile::Profiler,
        source: impl FnOnce(Option<Arc<dyn ClientStorage>>) -> Result<Source>,
    ) -> Result<AssetTicket> {
        let path = VfsPath::new(Mount::Asset, path)?.relative().to_owned();
        let key = (client, kind, path.clone());
        if let Some(&id) = self.keys.get(&key) {
            let cache = &mut *self;
            let entry = &cache.entries[&id];
            let retry = entry.blocked_bytes.zip(cache.budget).is_some_and(|(bytes, budget)| {
                let required = bytes.saturating_sub(budget - cache.resident);
                entry.cache_owned()
                    && bytes <= budget
                    && (required == 0
                        || cache
                            .eviction_candidates()
                            .map(|(_, e)| e.status.bytes)
                            .sum::<usize>()
                            >= required)
            });
            if !retry {
                let entry = cache.entries.get_mut(&id).unwrap();
                entry.touched = web_time::Instant::now();
                if explicit {
                    entry.automatic = false;
                }
                return Ok(entry.status.ticket);
            }
            cache.remove_entry(id);
        }
        ensure!(
            self.entries.len() < 4096,
            "async asset request limit; release unused tickets"
        );
        if self.entries
            .values()
            .filter(|e| e.status.ticket.client == client.0 && e.status.state == AssetState::Loading)
            .count()
            >= 32
        {
            return Err(crysirror_tasks::CapacityExceeded.into());
        }
        let source = source(self.storage.get(&client).cloned())?;
        let decoders = decoders.clone();
        let name = path.clone();
        let profiler = profiler.clone();
        let image_memory = self.image_memory.clone();
        let frame = profiler.frame();
        let job = self.io.spawn(move |cancel| {
            let _span = profiler.span(frame, format!("asset.{kind:?}.read_prepare"));
            let bytes = source.read(&name, &cancel)?;
            match kind {
                AssetKind::Bytes => Ok(Prepared::Bytes(bytes)),
                AssetKind::Image => {
                    let dependencies = |uri: &str| {
                        source
                            .read(uri, &cancel)
                            .map(|b| b.to_vec())
                            .map_err(|e| {
                                // Format decoders probe optional companions
                                // (e.g. per-tile extensions); keep absence typed.
                                if e.chain().any(|c| {
                                    matches!(
                                        c.downcast_ref::<crysirror_core::Error>(),
                                        Some(crysirror_core::Error::NotFound)
                                    )
                                }) {
                                    crysirror_core::Error::NotFound
                                } else {
                                    crysirror_core::Error::Io(format!("{e:#}"))
                                }
                            })
                    };
                    let tlg = is_tlg(&name);
                    let scratch = if tlg {
                        let size = crysirror_image::encoded_dimensions_hint(&bytes)
                            .map(|[w, h]| crysirror_image::rgba_len(w, h))
                            .transpose()?
                            .unwrap_or(crysirror_image::MAX_DECODED_BYTES);
                        Some(crate::image_processing::Charge::new(
                            image_memory,
                            2 * size + bytes.len(),
                        )?)
                    } else {
                        None
                    };
                    let resources = tlg.then(|| {
                        crysirror_resource::DecodeContext::new(client).with_cancellation(cancel.clone())
                    });
                    let image = decoders.decode_with(
                        &name, &bytes, Some(&dependencies), resources.as_ref(), None,
                    )?;
                    cancel.check()?;
                    let facts = tlg.then(|| (
                        [image.width, image.height],
                        image.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255),
                        *blake3::hash(&bytes).as_bytes(),
                    ));
                    drop(scratch);
                    Ok(Prepared::Image(Arc::new(image), facts))
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
        let id = self.next;
        self.next = id.checked_add(1).context("asset ticket overflow")?;
        let ticket = AssetTicket {
            store: self.store,
            client: client.0,
            id,
        };
        self.entries.insert(
            id,
            Entry {
                pins: Arc::new(AtomicUsize::new(0)),
                automatic: !explicit,
                leased: false,
                blocked_bytes: None,
                touched: web_time::Instant::now(),
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
                image_facts: None,
            },
        );
        self.keys.insert(key, id);
        Ok(ticket)
    }
    /// Only an explicitly synchronous consumer may join a worker. The worker
    /// uses a scoped source and does not acquire the prepared cache lock.
    fn wait_one(&mut self, id: u64) {
        if let Some(job) = self.entries.get_mut(&id).and_then(|entry| entry.job.take()) {
            self.finish(id, job.wait());
        }
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
    dir: Arc<dyn ClientStorage>,
    cache: BTreeMap<String, Arc<[u8]>>,
}
impl Source {
    fn read(&self, path: &str, cancel: &Cancellation) -> Result<Arc<[u8]>> {
        cancel.check()?;
        let p = VfsPath::new(Mount::Asset, path)?;
        if let Some(data) = self.cache.get(p.relative()) {
            ensure!(data.len() as u64 <= crysirror_core::limits::LARGE_ASSET_BYTES, "async asset exceeds the asset limit");
            return Ok(data.clone());
        }
        let bytes = if let Some(storage) = &self.storage {
            let reader = storage.open_asset(&p)?;
            let len = reader.len()?;
            ensure!(len <= crysirror_core::limits::LARGE_ASSET_BYTES, "async asset exceeds the asset limit");
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
            self.dir.read(&p)?
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
/// A metadata handle cannot keep a replaced asset store alive. TLG lookups
/// enter the same cache and worker queue as owner-prefetch and rendering.
#[derive(Clone)]
pub(crate) struct ImagePreparation {
    cache: Weak<Mutex<AsyncAssets>>,
    profiler: crysirror_tasks::profile::Profiler,
}
impl ImagePreparation {
    pub(crate) fn scoped_storage(&self, client: ClientId) -> Option<(u64, Arc<dyn ClientStorage>)> {
        let cache = self.cache.upgrade()?;
        let cache = cache.lock().expect("prepared asset cache poisoned");
        let storage = cache.storage.get(&client)?.clone();
        Some((cache.epochs.get(&client).copied().unwrap_or(0), storage))
    }

    pub(crate) fn facts(
        &self,
        client: ClientId,
        path: &str,
        decoders: &crysirror_image::DecoderRegistry,
        wait: bool,
    ) -> crysirror_core::Result<Option<ImageFacts>> {
        self.read_facts(client, path, decoders, wait).map_err(|error| {
            if error.is::<crysirror_tasks::CapacityExceeded>() {
                crysirror_core::Error::Limit("TLG metadata preparation capacity")
            } else {
                crysirror_core::Error::Io(format!("{error:#}"))
            }
        })
    }

    fn read_facts(
        &self,
        client: ClientId,
        path: &str,
        decoders: &crysirror_image::DecoderRegistry,
        wait: bool,
    ) -> Result<Option<ImageFacts>> {
        let cache = self.cache.upgrade().context("image asset store released")?;
        let mut cache = cache.lock().expect("prepared asset cache poisoned");
        let ticket = cache.request(client, path, AssetKind::Image, false, decoders, &self.profiler, |storage| {
            let storage = storage.context("image metadata client storage absent")?;
            Ok(Source { dir: storage.clone(), storage: Some(storage), cache: BTreeMap::new() })
        });
        let ticket = match ticket {
            Ok(ticket) => ticket,
            Err(error) if !wait && error.is::<crysirror_tasks::CapacityExceeded>() => return Ok(None),
            Err(error) => return Err(error),
        };
        if wait {
            cache.wait_one(ticket.id);
        } else {
            cache.poll_one(ticket.id);
        }
        let entry = cache.entry(client, ticket)?;
        // Metadata is still usable when the pixels cannot be retained under
        // pressure. The normal image lookup may retry admission later.
        if let Some(facts) = entry.image_facts {
            return Ok(Some(facts));
        }
        if entry.status.state == AssetState::Loading {
            return Ok(None);
        }
        anyhow::bail!("asset {}: {}", entry.status.path,
            entry.status.error.as_deref().unwrap_or("image metadata absent"))
    }
}
impl Assets {
    pub(super) fn prepared(&self) -> MutexGuard<'_, AsyncAssets> {
        self.asynchronous.lock().expect("prepared asset cache poisoned")
    }

    pub(super) fn asset_storage(&self, client: ClientId) -> Option<Arc<dyn ClientStorage>> {
        self.prepared().storage.get(&client).cloned()
    }

    pub(crate) fn image_preparation(&self) -> ImagePreparation {
        ImagePreparation { cache: Arc::downgrade(&self.asynchronous), profiler: self.profiler.clone() }
    }

    pub(crate) fn image_memory(&self) -> Arc<AtomicUsize> {
        self.prepared().image_memory.clone()
    }

    /// Desktop uses nonblocking loading; captures/offline tools explicitly wait.
    pub fn set_nonblocking(&mut self, value: bool) {
        self.prepared().nonblocking = value;
    }
    pub fn is_nonblocking(&self) -> bool {
        self.prepared().nonblocking
    }
    /// Host policy only: bounded CPU prepared-cache budget, not total RAM/VRAM.
    pub fn set_prepared_budget(&mut self, bytes: usize) -> Result<()> {
        let mut prepared = self.prepared();
        ensure!(
            (16 * 1024 * 1024..=4u64 * 1024 * 1024 * 1024).contains(&(bytes as u64))
                && bytes >= prepared.resident,
            "invalid prepared asset budget or below live residency"
        );
        prepared.budget = Some(bytes);
        Ok(())
    }
    pub fn store_id(&self) -> u64 {
        self.prepared().store
    }
    /// Bounded blocking-I/O executor; CPU continuations are polled by owners.
    pub fn io_tasks(&self) -> TaskPool {
        self.prepared().io.clone()
    }
    pub fn cpu_tasks(&self) -> TaskPool {
        self.prepared().cpu.clone()
    }
    pub fn revision(&self, client: ClientId) -> u64 {
        self.prepared().epochs.get(&client).copied().unwrap_or(0)
    }
    fn source(&self, client: ClientId, storage: Option<Arc<dyn ClientStorage>>) -> Result<Source> {
        let dir = self
            .clients
            .get(&client)
            .context("unknown asset client")?
            .clone();
        let prefix = format!("client:{}:", client.0);
        Ok(Source {
            storage,
            dir,
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
        let source = self.source(client, self.asset_storage(client))?;
        let path = VfsPath::new(Mount::Asset, path)?.relative().to_owned();
        self.io_tasks().spawn(move |cancel| source.read(&path, &cancel))
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
        let mut prepared = self.prepared();
        let e = prepared.entries.get_mut(&ticket.id).unwrap();
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
        self.key(client, path)?;
        self.prepared().request(client, path, kind, explicit, &self.decoders, &self.profiler,
                                |storage| self.source(client, storage))
    }
    pub(super) fn prepared_bytes(&self, client: ClientId, path: &str) -> Option<Arc<[u8]>> {
        let prepared = self.prepared();
        let id = prepared.keys.get(&(client, AssetKind::Bytes, path.to_owned()))?;
        match prepared.entries.get(id)?.value.as_ref()? {
            Prepared::Bytes(b) => Some(b.clone()),
            _ => None,
        }
    }
    pub fn poll_tasks(&mut self) {
        self.prepared().poll()
    }
    /// Explicit offline readiness fence. Interactive hosts use task_status.
    pub fn wait_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<AssetStatus> {
        let mut prepared = self.prepared();
        prepared.entry(client, ticket)?;
        prepared.wait_one(ticket.id);
        Ok(prepared.entry(client, ticket)?.status.clone())
    }
    pub fn task_status(&mut self, client: ClientId, ticket: AssetTicket) -> Result<AssetStatus> {
        let mut prepared = self.prepared();
        prepared.entry(client, ticket)?;
        prepared.poll_one(ticket.id);
        Ok(prepared.entry(client, ticket)?.status.clone())
    }
    pub fn ticket(&self, client: ClientId, id: u64) -> Result<AssetTicket> {
        let prepared = self.prepared();
        let ticket = prepared.entries.get(&id).context("unknown asset task")?.status.ticket;
        prepared.entry(client, ticket)?;
        Ok(ticket)
    }
    pub fn cancel_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<()> {
        let mut prepared = self.prepared();
        prepared.entry(client, ticket)?;
        let entry = prepared.entries.get_mut(&ticket.id).unwrap();
        ensure!(
            entry.status.state == AssetState::Loading,
            "only loading tasks can be cancelled"
        );
        if let Some(job) = entry.job.take() {
            job.cancel();
        }
        entry.status.state = AssetState::Cancelled;
        Ok(())
    }
    /// Drop every unpinned prepared entry of `path` (any kind) for the
    /// client: a released generated model must also leave the prepared
    /// budget, or repeated scene changes accumulate until loads fail.
    pub fn release_prepared_path(&mut self, client: ClientId, path: &str) {
        let mut prepared = self.prepared();
        let ids: Vec<u64> = prepared.entries
            .iter()
            .filter(|(_, e)| e.status.ticket.client == client.0 && e.status.path == path && e.pins.load(Ordering::Acquire) == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(job) = prepared.entries.get_mut(&id).and_then(|entry| entry.job.take()) {
                job.cancel();
            }
            prepared.remove_entry(id);
        }
    }
    pub fn release_task(&mut self, client: ClientId, ticket: AssetTicket) -> Result<()> {
        let mut prepared = self.prepared();
        let entry = prepared.entry(client, ticket)?;
        ensure!(entry.pins.load(Ordering::Acquire) == 0, "asset is pinned by a streaming lease");
        prepared.remove_entry(ticket.id);
        Ok(())
    }
    fn resolve(&mut self, client: ClientId, ticket: AssetTicket) -> Result<Option<Prepared>> {
        let mut prepared = self.prepared();
        prepared.entry(client, ticket)?;
        prepared.poll_one(ticket.id);
        if !prepared.nonblocking {
            prepared.wait_one(ticket.id);
        }
        let entry = prepared.entry(client, ticket)?;
        match entry.status.state {
            AssetState::Ready => Ok(entry.value.clone()),
            AssetState::Loading => Ok(None),
            _ => anyhow::bail!(
                "asset {} {:?}: {}",
                entry.status.path,
                entry.status.state,
                entry.status.error.as_deref().unwrap_or("cancelled")
            ),
        }
    }
    /// Start the ordinary, evictable image decode without waiting or pinning
    /// its result. The first visible draw uses the same client-scoped entry.
    /// A saturated worker queue leaves the normal first-use path available.
    pub fn prefetch_image(&mut self, client: ClientId, path: &str) -> Result<bool> {
        if self.surface(client, path).is_some() {
            return Ok(true);
        }
        match self.request_inner(client, path, AssetKind::Image, false) {
            Ok(_) => Ok(true),
            Err(error) if error.is::<crysirror_tasks::CapacityExceeded>() => Ok(false),
            Err(error) => Err(error),
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
            Some(Prepared::Image(v, _)) => Ok(Some(v)),
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
        {
            let prepared = self.prepared();
            if let Some(id) = prepared.keys.get(&(client, AssetKind::Level, path.to_owned())) {
                if let Some(Prepared::Level(_, model, _)) = prepared.entries[id].value.as_ref() {
                    return Ok(Some(model.clone()));
                }
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
            Some(Prepared::Model(v, _)) => Ok(Some(v)),
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
        let mut prepared = self.prepared();
        prepared.entry(client, ticket)?;
        prepared.poll_one(ticket.id);
        let entry = prepared.entry(client, ticket)?;
        match entry.value.as_ref() {
            Some(Prepared::Scene(v, _)) | Some(Prepared::Level(v, _, _)) => Ok(v.clone()),
            _ => anyhow::bail!("scene import not ready"),
        }
    }
    /// Evict only automatically managed, unpinned and unshared payloads.
    /// Explicit request IDs remain owned until their caller releases them.
    pub fn collect_unused(&mut self) -> usize {
        let mut prepared = self.prepared();
        let ids: Vec<_> = prepared.entries
            .iter()
            .filter(|(_, e)| {
                e.cache_owned()
                    && (e.leased
                        || (e.status.state != AssetState::Loading
                            && e.touched.elapsed().as_secs() >= 5))
            })
            .map(|(id, _)| *id)
            .collect();
        let count = ids.len();
        for id in ids {
            prepared.remove_entry(id);
        }
        count
    }
    pub fn tasks_inspect(&self) -> serde_json::Value {
        let prepared = self.prepared();
        serde_json::json!({"budget_bytes":prepared.budget,"nonblocking":prepared.nonblocking,
            "resident_bytes":prepared.resident,"io":prepared.io.stats(),"cpu":prepared.cpu.stats(),
            "assets":prepared.entries.values().map(|e|&e.status).collect::<Vec<_>>()})
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
