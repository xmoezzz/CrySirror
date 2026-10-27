// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Client-owned acoustic scenes and bounded background propagation. No device.
mod instances;
use crate::{EngineState, Value};
use anyhow::{Context, Result, ensure};
use crysirror_acoustics::{
    AcousticMeshUpdate, AcousticQuery, AcousticScene, AcousticSceneSpec, DirectPath,
};
use crysirror_audio::mixer::{Mixer, VoiceHandle};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_tasks::{
    Task,
    table::{OperationState, OperationTable},
};
pub use instances::AcousticMeshBindingSpec;
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
type Key = (u32, u32, u64);
fn key(h: VoiceHandle) -> Key {
    (h.client, h.slot, h.generation)
}
const RESERVATION: usize = 128 * 1024 * 1024;
const HOST_BUDGET: usize = 512 * 1024 * 1024;
struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(budget: Arc<AtomicUsize>) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| old
                    .checked_add(RESERVATION)
                    .filter(|next| *next <= HOST_BUDGET))
                .is_ok(),
            "acoustic geometry staging budget exceeded"
        );
        Ok(Self {
            budget,
            bytes: RESERVATION,
        })
    }
    fn shrink(&mut self, bytes: usize) {
        let bytes = bytes.min(self.bytes);
        self.budget.fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        self.bytes = bytes;
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
pub(crate) struct PreparedScene {
    pub(crate) scene: AcousticScene,
    _charge: Charge,
}
enum Product {
    Scene {
        scene: Arc<PreparedScene>,
        base: Option<u64>,
    },
    Summary(Json),
    Probe {
        revision: u64,
        path: DirectPath,
    },
}
struct Flight {
    handle: VoiceHandle,
    revision: u64,
    scene_generation: u64,
    task: Task<DirectPath>,
}
/// Derived, nonserializable state. Geometry and queries are namespaced by client.
pub struct AcousticsRuntime {
    instances: instances::InstanceRuntime,
    epoch: Option<u64>,
    jobs: OperationTable<Product>,
    budget: Arc<AtomicUsize>,
    scenes: BTreeMap<u32, (u64, Arc<PreparedScene>)>,
    scene_jobs: BTreeMap<u32, u64>,
    generations: BTreeMap<u32, u64>,
    flight: BTreeMap<Key, Flight>,
    waiting: BTreeMap<Key, (VoiceHandle, AcousticQuery)>,
    order: VecDeque<Key>,
    results: BTreeMap<Key, Json>,
}
impl Default for AcousticsRuntime {
    fn default() -> Self {
        Self {
            instances: Default::default(),
            epoch: None,
            jobs: OperationTable::new(16, 64).unwrap(),
            budget: Arc::new(AtomicUsize::new(0)),
            scenes: BTreeMap::new(),
            scene_jobs: BTreeMap::new(),
            generations: BTreeMap::new(),
            flight: BTreeMap::new(),
            waiting: BTreeMap::new(),
            order: VecDeque::new(),
            results: BTreeMap::new(),
        }
    }
}
impl AcousticsRuntime {
    pub fn has_scene(&self, owner: u32) -> bool {
        self.scenes.contains_key(&owner)
    }
    pub(crate) fn revision(&self, owner: u32) -> Option<u64> {
        self.scenes.get(&owner).map(|s| s.0)
    }
    pub(crate) fn generation(&self, owner: u32) -> Option<u64> {
        self.generations.get(&owner).copied()
    }
    pub fn reset(&mut self) {
        self.instances = Default::default();
        self.jobs.clear();
        self.scenes.clear();
        self.scene_jobs.clear();
        self.generations.clear();
        self.flight.clear();
        self.waiting.clear();
        self.order.clear();
        self.results.clear();
        self.epoch = None;
    }
    pub(crate) fn scene(&self, owner: u32) -> Result<(u64, Arc<PreparedScene>)> {
        self.scenes
            .get(&owner)
            .cloned()
            .context("client acoustic scene is not prepared")
    }
    fn begin_update(
        &mut self,
        owner: u32,
        epoch: u64,
        pool: &crysirror_tasks::TaskPool,
        updates: Vec<AcousticMeshUpdate>,
    ) -> Result<u64> {
        ensure!(
            !self.scene_jobs.contains_key(&owner),
            "acoustic scene preparation pending; wait before another explicit commit"
        );
        ensure!(
            !updates.is_empty() && updates.len() <= 4096,
            "acoustic instance update count"
        );
        let mut ids = std::collections::BTreeSet::new();
        let (base, previous) = self.scene(owner)?;
        for update in &updates {
            update.validate()?;
            ensure!(ids.insert(update.id), "duplicate acoustic mesh update");
            ensure!(
                previous.scene.mesh_pose(update.id).is_some(),
                "unknown acoustic mesh update ID"
            );
        }
        self.jobs.admit(owner)?;
        let mut charge = Charge::new(self.budget.clone())?;
        let ticket =
            self.jobs
                .submit_deferred(owner, epoch, "acoustic_update", pool, move |cancel| {
                    cancel.check()?;
                    let scene = previous
                        .scene
                        .updated_cancellable(&updates, || cancel.check())?;
                    charge.shrink(previous._charge.bytes);
                    cancel.check()?;
                    Ok(Product::Scene {
                        scene: Arc::new(PreparedScene {
                            scene,
                            _charge: charge,
                        }),
                        base: Some(base),
                    })
                })?;
        self.scene_jobs.insert(owner, ticket);
        Ok(ticket)
    }
    fn poll(&mut self, mixer: &mut Mixer, epoch: u64) -> Result<()> {
        if self.epoch.is_some_and(|old| old != epoch) {
            self.reset();
        }
        self.epoch = Some(epoch);
        for (owner, _, ticket) in self.jobs.tickets() {
            let status = self.jobs.status(owner, epoch, ticket)?;
            if status.state == OperationState::Pending {
                continue;
            }
            let automatic = self.instances.is_automatic(owner, ticket);
            if status.state == OperationState::Failed {
                if self.scene_jobs.get(&owner) == Some(&ticket) {
                    self.scene_jobs.remove(&owner);
                }
                self.instances.finished(owner, ticket, None, status.error);
                if automatic {
                    self.jobs.cancel(owner, ticket)?;
                }
                continue;
            }
            let generations = &mut self.generations;
            let scenes = &mut self.scenes;
            let pending = &mut self.scene_jobs;
            let mut replaced = None;
            let publication=self.jobs.publish(owner,epoch,ticket,|product|{
                let Product::Scene{scene,base}=product else{return Ok(product)};
                ensure!(pending.get(&owner)==Some(&ticket),"superseded acoustic scene");
                ensure!(base.is_none_or(|base|scenes.get(&owner).is_some_and(|s|s.0==base)),"acoustic update base revision changed");
                if base.is_none(){generations.insert(owner,ticket);}
                let info=json!({"revision":ticket,"scene_generation":generations.get(&owner),"triangles":scene.scene.triangle_count(),"backend":scene.scene.backend,"geometry":scene.scene.geometry_stats()});
                scenes.insert(owner,(ticket,scene));pending.remove(&owner);replaced=Some(base.is_none());Ok(Product::Summary(info))
            });
            if let Err(error) = publication {
                if self.scene_jobs.get(&owner) == Some(&ticket) {
                    self.scene_jobs.remove(&owner);
                }
                self.instances
                    .finished(owner, ticket, None, Some(format!("{error:#}")));
                if automatic {
                    self.jobs.cancel(owner, ticket)?;
                }
                continue;
            }
            if let Some(full) = replaced {
                if full {
                    self.flight.retain(|key, _| key.0 != owner);
                    self.waiting.retain(|key, _| key.0 != owner);
                    self.order.retain(|key| key.0 != owner);
                    self.results.retain(|key, _| key.0 != owner);
                }
                self.instances.finished(owner, ticket, Some(ticket), None);
                if full {
                    self.instances.clear(owner);
                }
            }
            if automatic {
                self.jobs.cancel(owner, ticket)?;
            }
        }
        let keys = self.flight.keys().copied().collect::<Vec<_>>();
        for key in keys {
            let job = self.flight.get_mut(&key).unwrap();
            if !mixer.is_playing(key.0, job.handle)
                || self.generations.get(&key.0) != Some(&job.scene_generation)
            {
                self.flight.remove(&key);
                continue;
            }
            let result = job.task.try_take();
            match result {
                Ok(None) => {}
                Ok(Some(path)) => {
                    mixer.set_acoustic_gains(key.0, job.handle, Some(path.gains))?;
                    self.results.insert(
                        key,
                        json!({"revision":job.revision,"path":path,"error":null}),
                    );
                    self.flight.remove(&key);
                }
                Err(error) => {
                    self.results.insert(
                        key,
                        json!({"revision":job.revision,"error":format!("{error:#}")}),
                    );
                    self.flight.remove(&key);
                }
            }
        }
        self.results.retain(|key, _| {
            mixer.is_playing(
                key.0,
                VoiceHandle {
                    client: key.0,
                    slot: key.1,
                    generation: key.2,
                },
            )
        });
        self.waiting
            .retain(|_, (handle, _)| mixer.is_playing(handle.client, *handle));
        self.order.retain(|key| self.waiting.contains_key(key));
        Ok(())
    }
}
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    let epoch = state.runtime.restored;
    let a = &mut state.mixer_audio;
    a.acoustics.poll(&mut a.mixer, epoch)
}
pub(crate) fn queue(
    state: &mut EngineState,
    handle: VoiceHandle,
    query: AcousticQuery,
) -> Result<()> {
    let runtime = &mut state.mixer_audio.acoustics;
    runtime.scene(handle.client)?;
    let key = key(handle);
    if !runtime.waiting.contains_key(&key) {
        ensure!(runtime.waiting.len() < 1024, "acoustic source queue full");
        runtime.order.push_back(key);
    }
    runtime.waiting.insert(key, (handle, query));
    Ok(())
}
pub(crate) fn dispatch(state: &mut EngineState) -> Result<()> {
    let pool = state.runtime.assets.cpu_tasks();
    let runtime = &mut state.mixer_audio.acoustics;
    let count = runtime.order.len();
    for _ in 0..count {
        if runtime.flight.len() >= 32 {
            break;
        }
        let key = runtime.order.pop_front().unwrap();
        if runtime.flight.contains_key(&key) {
            runtime.order.push_back(key);
            continue;
        }
        let Some((handle, query)) = runtime.waiting.get(&key).cloned() else {
            continue;
        };
        let (revision, scene) = runtime.scene(handle.client)?;
        let scene_generation = runtime
            .generation(handle.client)
            .context("acoustic scene generation missing")?;
        match pool.spawn(move |cancel| {
            cancel.check()?;
            let result = crysirror_acoustics::simulate(&scene.scene, &query)?;
            cancel.check()?;
            Ok(result)
        }) {
            Ok(task) => {
                runtime.waiting.remove(&key);
                runtime.flight.insert(
                    key,
                    Flight {
                        handle,
                        revision,
                        scene_generation,
                        task,
                    },
                );
            }
            Err(error) if error.is::<crysirror_tasks::CapacityExceeded>() => {
                runtime.order.push_front(key);
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn arg(args: &[Value], n: usize) -> Result<&Value> {
    args.get(n).context("acoustic argument required")
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginAcousticScene"
            | "beginUpdateAcousticScene"
            | "bindAcousticMesh"
            | "unbindAcousticMesh"
            | "beginAcousticProbe"
            | "acousticStatus"
            | "cancelAcousticJob"
            | "clearAcousticScene"
            | "acousticStats"
            | "acousticVoice"
    ) {
        return Ok(None);
    }
    poll(state)?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    let pool = state.runtime.assets.cpu_tasks();
    if matches!(method, "bindAcousticMesh" | "unbindAcousticMesh") {
        return Ok(Some(instances::call(state, method, args)?));
    }
    if method == "beginUpdateAcousticScene" {
        ensure!(
            matches!(arg(args,0)?,Value::List(v) if !v.is_empty()&&v.len()<=4096),
            "acoustic instance update count"
        );
        let updates: Vec<AcousticMeshUpdate> = serde_json::from_value(arg(args, 0)?.to_json())?;
        let runtime = &mut state.mixer_audio.acoustics;
        ensure!(
            updates
                .iter()
                .all(|u| !runtime.instances.is_bound(owner, u.id)),
            "unbind an entity-driven acoustic mesh before explicit editing"
        );
        return Ok(Some(Value::Int(
            runtime
                .begin_update(owner, epoch, &pool, updates)?
                .try_into()?,
        )));
    }
    if method == "beginAcousticScene" {
        let name = arg(args, 0)?.as_str()?;
        let path = VfsPath::new(Mount::Asset, name).map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            path.relative() == name,
            "canonical acoustic scene asset required"
        );
        state.mixer_audio.acoustics.jobs.admit(owner)?;
        let storage = state
            .runtime
            .host
            .fs
            .storage(state.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let runtime = &mut state.mixer_audio.acoustics;
        let mut charge = Charge::new(runtime.budget.clone())?;
        let ticket = runtime
            .jobs
            .submit(owner, epoch, "acoustic_scene", &pool, move |cancel| {
                cancel.check()?;
                let reader = storage
                    .open_asset(&path)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let bytes = crysirror_resource::read_all(reader.as_ref(), 32 * 1024 * 1024)?;
                cancel.check()?;
                let spec: AcousticSceneSpec = serde_json::from_slice(&bytes)?;
                drop(bytes);
                cancel.check()?;
                let estimate = 8192
                    + spec.meshes.len() * 512
                    + spec
                        .meshes
                        .iter()
                        .map(|mesh| mesh.triangles.len() * 384 + mesh.vertices.len() * 16)
                        .sum::<usize>();
                ensure!(
                    estimate <= RESERVATION,
                    "acoustic scene reservation exceeded"
                );
                let scene = AcousticScene::build(spec)?;
                charge.shrink(estimate);
                cancel.check()?;
                Ok(Product::Scene {
                    scene: Arc::new(PreparedScene {
                        scene,
                        _charge: charge,
                    }),
                    base: None,
                })
            })?;
        if let Some(old) = runtime.scene_jobs.insert(owner, ticket) {
            let _ = runtime.jobs.cancel(owner, old);
            runtime.instances.canceled(owner, old);
        }
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if method == "beginAcousticProbe" {
        let query: AcousticQuery = serde_json::from_value(arg(args, 0)?.to_json())?;
        query.settings.validate()?;
        let runtime = &mut state.mixer_audio.acoustics;
        let (revision, scene) = runtime.scene(owner)?;
        let ticket = runtime
            .jobs
            .submit(owner, epoch, "acoustic_probe", &pool, move |cancel| {
                cancel.check()?;
                let path = crysirror_acoustics::simulate(&scene.scene, &query)?;
                cancel.check()?;
                Ok(Product::Probe { revision, path })
            })?;
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if matches!(method, "acousticStatus" | "cancelAcousticJob") {
        let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
        let runtime = &mut state.mixer_audio.acoustics;
        if method == "cancelAcousticJob" {
            runtime.jobs.cancel(owner, ticket)?;
            runtime.scene_jobs.retain(|_, id| *id != ticket);
            runtime.instances.canceled(owner, ticket);
            return Ok(Some(Value::Null));
        }
        // Completion may race the initial poll above. Publish scene ownership
        // before exposing Ready, including a worker that completed just now.
        let status = runtime.jobs.status(owner, epoch, ticket)?;
        if status.state == OperationState::Ready {
            poll(state)?;
        }
        let runtime = &mut state.mixer_audio.acoustics;
        let mut value = serde_json::to_value(runtime.jobs.status(owner, epoch, ticket)?)?;
        if value["state"] == "ready" {
            if let Some(product) = runtime.jobs.poll(owner, epoch, ticket)? {
                value["result"] = match product {
                    Product::Summary(info) => info.clone(),
                    Product::Probe { revision, path } => {
                        let mut value = serde_json::to_value(path)?;
                        value["revision"] = json!(revision);
                        value
                    }
                    Product::Scene { .. } => anyhow::bail!("acoustic scene has not been published"),
                };
            }
        }
        return Ok(Some(Value::from_json(value)));
    }
    if method == "acousticVoice" {
        let handle: VoiceHandle = serde_json::from_value(arg(args, 0)?.to_json())?;
        ensure!(handle.client == owner, "foreign acoustic voice handle");
        ensure!(
            state.mixer_audio.mixer.is_playing(owner, handle),
            "stale acoustic voice handle"
        );
        let runtime = &state.mixer_audio.acoustics;
        return Ok(Some(Value::from_json(
            json!({"pending":runtime.flight.contains_key(&key(handle)) || runtime.waiting.contains_key(&key(handle)),"result":runtime.results.get(&key(handle)),"current_revision":runtime.revision(owner),"geometry_stale":runtime.results.get(&key(handle)).and_then(|v|v["revision"].as_u64()).is_some_and(|r|Some(r)!=runtime.revision(owner))}),
        )));
    }
    if method == "clearAcousticScene" {
        let a = &mut state.mixer_audio;
        let runtime = &mut a.acoustics;
        runtime.instances.clear(owner);
        runtime.scenes.remove(&owner);
        runtime.generations.remove(&owner);
        if let Some(ticket) = runtime.scene_jobs.remove(&owner) {
            let _ = runtime.jobs.cancel(owner, ticket);
        }
        runtime.flight.retain(|key, _| key.0 != owner);
        runtime.waiting.retain(|key, _| key.0 != owner);
        runtime.order.retain(|key| key.0 != owner);
        runtime.results.retain(|key, _| key.0 != owner);
        for attachment in a
            .attachments
            .values_mut()
            .filter(|a| a.handle.client == owner)
        {
            if attachment.acoustics.take().is_some() && a.mixer.is_playing(owner, attachment.handle)
            {
                a.mixer.set_acoustic_gains(owner, attachment.handle, None)?;
            }
        }
        return Ok(Some(Value::Null));
    }
    let runtime = &state.mixer_audio.acoustics;
    Ok(Some(Value::from_json(
        json!({"scene":runtime.scenes.get(&owner).map(|(revision,scene)|json!({"revision":revision,"scene_generation":runtime.generation(owner),"triangles":scene.scene.triangle_count(),"backend":scene.scene.backend,"geometry":scene.scene.geometry_stats()})),"instances":runtime.instances.status(owner),"preparing":runtime.scene_jobs.contains_key(&owner),"pending_sources":runtime.flight.keys().filter(|key|key.0==owner).count(),"queued_sources":runtime.waiting.keys().filter(|key|key.0==owner).count(),"host_geometry_reservation":runtime.budget.load(Ordering::Relaxed),"host_geometry_budget":HOST_BUDGET}),
    )))
}

pub(crate) fn update_instances(state: &mut EngineState, dt_ms: u64) -> Result<()> {
    instances::update(state, dt_ms)
}
