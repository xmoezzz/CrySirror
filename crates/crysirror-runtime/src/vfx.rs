// SPDX-License-Identifier: MPL-2.0
//! Owner publication of worker-owned VFX simulation and bounded GPU timelines.
//! No window/device pointers, renderer callbacks or VM-specific algorithms.
use anyhow::{ensure, Context, Result};
use crysirror_core::state::ClientId;
use crysirror_scene::{
    components::EmitterSpec,
    world::{EntityHandle, World},
};
use crysirror_tasks::{CapacityExceeded, Task, TaskPool};
use crysirror_vfx::{
    CpuWorld, FrameSnapshot, GpuEmitterFrame, ParticleEvent, SystemInstance, SystemSpec,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
#[derive(Clone)]
struct ScreenPublication {
    system: u64,
    emitter: String,
    declaration: serde_json::Value,
    frame: Arc<crysirror_puppet::PuppetFrame>,
}

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
const MAX_GPU_PACKETS: usize = 65536;
const MAX_GPU_HISTORY_BYTES: usize = 32 * 1024 * 1024;
const MAX_EVENT_QUEUE: usize = 8192;
const MAX_INPUT_SAMPLES: usize = 2048;
const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;

fn packet_bytes(parameters: &crysirror_vfx::ParameterMap) -> usize {
    std::mem::size_of::<crysirror_vfx::TickSpawn>()
        + parameters
            .keys()
            .map(|name| name.len() * 2 + 128)
            .sum::<usize>()
}

/// Compatibility lowering only: simulation is implemented by crysirror-vfx.
pub fn lower_legacy(s: &EmitterSpec) -> Result<SystemSpec> {
    use crysirror_vfx::*;
    s.validate()?;
    let mut emitter = crysirror_vfx::EmitterSpec::default();
    emitter.id = "legacy".into();
    emitter.capacity = s.max;
    emitter.seed = s.seed;
    emitter.enabled = s.enabled;
    emitter.spawn.rate = s.rate.into();
    emitter.spawn.burst_looping = false;
    emitter.spawn.duration = s.lifetime[1].max(1.);
    if s.burst > 0 {
        emitter.spawn.bursts.push(Burst {
            time: 0.,
            count: s.burst,
        });
    }
    emitter.initialize.lifetime = s.lifetime;
    emitter.initialize.speed = s.speed;
    emitter.initialize.direction = s.direction.into();
    emitter.initialize.spread_degrees = s.spread_degrees;
    emitter.initialize.color = s.color_start;
    emitter.initialize.size = [s.size[0] * 2.; 2];
    emitter.update = vec![
        Module::Acceleration {
            value: s.gravity.into(),
        },
        Module::Drag {
            coefficient: s.drag.into(),
        },
        Module::ColorOverLife {
            curve: ColorCurve::linear(s.color_start, s.color_end),
        },
        Module::SizeOverLife {
            curve: Vec2Curve::linear([s.size[0] * 2.; 2], [s.size[1] * 2.; 2]),
        },
    ];
    emitter.render = RenderSpec::Sprite {
        texture: None,
        columns: 1,
        rows: 1,
        frames: 1,
        fps: 0.,
        soft_distance: 0.,
        facing: FacingMode::Camera,
        blend: if s.additive {
            BlendMode::Additive
        } else {
            BlendMode::Alpha
        },
    };
    let system = SystemSpec {
        emitters: vec![emitter],
        fixed_step_hz: 30,
        warmup_seconds: if s.rate > 0. { s.lifetime[1] } else { 0. },
        ..Default::default()
    };
    system
        .compile()
        .context("legacy emitter exceeds the modular VFX profile")?;
    Ok(system)
}

struct Completed {
    world: CpuWorld,
    frame: Result<FrameSnapshot>,
    time_ms: u64,
    epoch: u64,
    inputs: Vec<SystemInstance>,
}
struct InputSample {
    time_ms: u64,
    inputs: Vec<SystemInstance>,
    bytes: usize,
}
pub struct VfxRuntime {
    pending_screen: bool,
    screen_layers: BTreeMap<(u32, u32), ScreenPublication>,
    screen_frame: Option<Arc<FrameSnapshot>>,
    world: Option<CpuWorld>,
    pending: Option<Task<Completed>>,
    frame: Arc<FrameSnapshot>,
    owners: BTreeMap<(u32, u32, u32), u64>,
    visible: BTreeSet<u64>,
    inputs: Vec<SystemInstance>,
    accepted_inputs: Vec<SystemInstance>,
    input_history: VecDeque<InputSample>,
    input_bytes: usize,
    recorded_ms: u64,
    recorded_inputs: Vec<SystemInstance>,
    serial: u32,
    epoch: u64,
    sampled_ms: u64,
    gpu_history: BTreeMap<u64, GpuEmitterFrame>,
    events: VecDeque<ParticleEvent>,
    dropped_events: u64,
    error: Option<String>,
    backpressured: bool,
}
impl Default for VfxRuntime {
    fn default() -> Self {
        Self {
            pending_screen: false,
            screen_layers: BTreeMap::new(),
            screen_frame: None,
            world: Some(CpuWorld::default()),
            pending: None,
            frame: Arc::new(FrameSnapshot::default()),
            owners: BTreeMap::new(),
            visible: BTreeSet::new(),
            inputs: vec![],
            accepted_inputs: vec![],
            serial: 0,
            input_history: VecDeque::new(),
            input_bytes: 0,
            recorded_ms: 0,
            recorded_inputs: vec![],
            epoch: NEXT_EPOCH.fetch_add(1, Ordering::Relaxed),
            sampled_ms: 0,
            gpu_history: BTreeMap::new(),
            events: VecDeque::new(),
            dropped_events: 0,
            error: None,
            backpressured: false,
        }
    }
}
impl VfxRuntime {
    /// Completed single-stream receipts mapped to the current client/entity
    /// lifetime. No entry is produced from declarations or worker admission.
    /// The fingerprint lets a host reject a cached receipt if a same-revision
    /// declaration was edited before its worker acknowledgement arrives.
    pub fn sequence_receipts(
        &self,
        world: &World,
    ) -> Result<BTreeMap<(u32, u32), serde_json::Value>> {
        let mut out = BTreeMap::new();
        for client in world.client_ids() {
            for (entity, generation, spec) in world.entities(ClientId(client)) {
                let Some(system) = &spec.vfx else { continue };
                if system.sequence_rng.is_none() {
                    continue;
                }
                let Some(key) = self.owners.get(&(client, entity, generation)) else {
                    continue;
                };
                let Some(accepted) = self
                    .accepted_inputs
                    .iter()
                    .find(|input| input.client == client && input.key == *key)
                else {
                    continue;
                };
                let mut normalized = system.clone();
                normalized.paused |= world.client_paused(client);
                normalized.max_steps_per_advance = normalized.max_steps_per_advance.min(32);
                if accepted.spec != normalized {
                    continue;
                }
                let Some(receipt) = self.frame.sequence_states.iter().find(|receipt| {
                    receipt.client == client
                        && receipt.system_key == *key
                        && receipt.revision == system.revision
                }) else {
                    continue;
                };
                let fingerprint = blake3::hash(&serde_json::to_vec(system)?)
                    .to_hex()
                    .to_string();
                out.insert((client,entity),serde_json::json!({"phase":"ready","system_key":key,"entity_generation":generation,"revision":receipt.revision,"completed_tick":receipt.completed_tick,"exchange_epoch":receipt.exchange_epoch,"sequence_rng":receipt.sequence_rng,"source_fingerprint":fingerprint}));
            }
        }
        Ok(out)
    }
    /// Sequence exchanges require worker-state reconstruction, even without a
    /// screen target. A declared seed cannot restore a completed borrowed stream.
    pub fn has_sequence_state(&self, world: &World) -> bool {
        let needs = |spec: &SystemSpec| spec.sequence_rng.is_some() || spec.sequence_exchange.is_some();
        let contains = |inputs: &[SystemInstance]| inputs.iter().any(|input| needs(&input.spec));
        !self.frame.sequence_states.is_empty()
            || contains(&self.inputs) || contains(&self.accepted_inputs) || contains(&self.recorded_inputs)
            || self.input_history.iter().any(|sample| contains(&sample.inputs))
            || world.client_ids().any(|client| world.entities(ClientId(client)).into_iter()
                .any(|(_, _, entity)| entity.vfx.as_ref().is_some_and(needs)))
    }
    /// True when an active, queued or declared screen producer requires
    /// explicit producer/resource reconstruction to support a snapshot.
    /// A transparent layer shape never acts as a producer identity.
    pub fn has_screen_state(&self, world: &World) -> bool {
        let contains = |inputs: &[SystemInstance]| {
            inputs.iter().any(|system| {
                system
                    .spec
                    .emitters
                    .iter()
                    .any(|emitter| emitter.screen.is_some())
            })
        };
        !self.screen_layers.is_empty()
            || self.pending_screen
            || contains(&self.inputs)
            || contains(&self.accepted_inputs)
            || contains(&self.recorded_inputs)
            || self
                .input_history
                .iter()
                .any(|sample| contains(&sample.inputs))
            || self
                .frame
                .emitters
                .iter()
                .any(|emitter| emitter.screen.is_some())
            || world.client_ids().any(|client| {
                world
                    .entities(ClientId(client))
                    .into_iter()
                    .any(|(_, _, entity)| {
                        entity.vfx.as_ref().is_some_and(|system| {
                            system
                                .emitters
                                .iter()
                                .any(|emitter| emitter.screen.is_some())
                        })
                    })
            })
    }

    /// Publish completed CPU screen sprites through ordinary owner-scoped
    /// dynamic mesh layers. An unchanged frame/visibility set is idempotent.
    /// Declarations are leases: foreign/existing layers and foreign geometry
    /// are never overwritten or removed, including during cleanup/reset.
    pub fn publish_screen(
        &mut self,
        layers: &mut crysirror_scene::layer::LayerSystem,
    ) -> Result<()> {
        use crysirror_scene::layer::{LayerCommand as C, LayerSpec, Paint, SpriteTransform};
        let mut desired = BTreeMap::new();
        for system in &self.inputs {
            for emitter in &system.spec.emitters {
                if let Some(target) = &emitter.screen {
                    target.validate()?;
                    ensure!(
                        desired
                            .insert(
                                (system.client, target.layer),
                                (
                                    system.key,
                                    emitter.id.clone(),
                                    target.clone(),
                                    emitter.render.clone()
                                )
                            )
                            .is_none(),
                        "VFX screen target declared by multiple emitters"
                    );
                }
            }
        }
        if desired.is_empty() && self.screen_layers.is_empty() {
            self.screen_frame = Some(self.frame.clone());
            return Ok(());
        }
        let same_frame = self
            .screen_frame
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &self.frame));
        let mut staged = layers.clone();
        let mut publications = self.screen_layers.clone();
        for (&(owner, id), old) in &self.screen_layers {
            let declaration_matches =
                desired
                    .get(&(owner, id))
                    .is_some_and(|(system, emitter, _, _)| {
                        *system == old.system && emitter == &old.emitter
                    });
            let current = staged.get(ClientId(owner), id);
            let current_missing = current.is_none();
            if let Some(current) = current {
                ensure!(
                    serde_json::to_value(current)? == old.declaration,
                    "VFX screen layer was replaced or edited outside its declaration"
                );
                ensure!(
                    staged
                        .dynamic_puppet_frame(owner, id)
                        .is_some_and(|frame| Arc::ptr_eq(frame, &old.frame)),
                    "VFX screen layer mesh producer changed"
                );
                ensure!(
                    !staged.has_children(ClientId(owner), id),
                    "VFX screen target must remain a leaf layer"
                );
                if !declaration_matches {
                    staged.apply_batch(ClientId(owner), &[C::Remove { id }])?;
                }
            }
            if !declaration_matches || current_missing {
                publications.remove(&(owner, id));
            }
        }
        for (&(owner, id), _) in &desired {
            ensure!(
                staged.get(ClientId(owner), id).is_none()
                    || publications.contains_key(&(owner, id)),
                "VFX screen target conflicts with an existing layer"
            );
        }
        for emitter in &self.frame.emitters {
            let Some(target) = &emitter.screen else {
                continue;
            };
            let Some((system, name, current, render)) =
                desired.get(&(emitter.client, target.layer))
            else {
                continue;
            };
            if *system != emitter.system_key
                || name != &emitter.emitter_id
                || current != target
                || render != &emitter.render
            {
                continue;
            }
            let key = (emitter.client, target.layer);
            if same_frame && publications.contains_key(&key) {
                continue;
            }
            let frame = Arc::new(crysirror_vfx::screen::pack_screen_emitter(
                emitter.client,
                emitter,
                &crysirror_vfx::screen::ScreenSpriteOptions {
                    canvas: target.canvas,
                    anchor: target.anchor,
                },
                emitter.screen_uvs.as_deref(),
            )?);
            let spec = LayerSpec {
                id: target.layer,
                parent: target.parent,
                rect: [0., 0., target.canvas[0] as f32, target.canvas[1] as f32],
                paint: Paint::Solid { color: [0; 4] },
                transform: SpriteTransform::default(),
                order: target.order,
                clip: Some([0., 0., target.canvas[0] as f32, target.canvas[1] as f32]),
                opacity: 255,
                visible: self.visible.contains(system),
                blend: Default::default(),
                uv: None,
                sampling: Default::default(),
                filter: None,
            };
            let declaration = serde_json::to_value(&spec)?;
            staged.apply_batch(ClientId(emitter.client), &[C::Put { layer: spec }])?;
            staged.set_dynamic_puppet(emitter.client, target.layer, frame.clone())?;
            publications.insert(
                key,
                ScreenPublication {
                    system: *system,
                    emitter: name.clone(),
                    declaration,
                    frame,
                },
            );
        }
        // Visibility can change before worker completion; do not expose stale
        // particles for a hidden/removed owner while awaiting the next frame.
        for (&(owner, id), entry) in &mut publications {
            let visible = self.visible.contains(&entry.system);
            if staged
                .get(ClientId(owner), id)
                .is_some_and(|layer| layer.visible != visible)
            {
                staged.apply_batch(ClientId(owner), &[C::SetVisible { id, visible }])?;
                entry.declaration = serde_json::to_value(
                    staged
                        .get(ClientId(owner), id)
                        .context("VFX screen published layer missing")?,
                )?;
            }
        }
        *layers = staged;
        self.screen_layers = publications;
        self.screen_frame = Some(self.frame.clone());
        Ok(())
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn reset(&mut self, now_ms: u64) {
        self.pending_screen = false;
        // Keep leases until publish_screen can clean them against the newly
        // restored layer system. Reset must not leave orphan derived layers.
        self.screen_frame = None;
        if let Some(task) = self.pending.take() {
            task.cancel();
        }
        self.world = Some(CpuWorld::default());
        self.frame = Arc::new(FrameSnapshot::default());
        self.owners.clear();
        self.visible.clear();
        self.inputs.clear();
        self.accepted_inputs.clear();
        self.input_history.clear();
        self.input_bytes = 0;
        self.recorded_ms = now_ms;
        self.recorded_inputs.clear();
        self.gpu_history.clear();
        self.events.clear();
        self.error = None;
        self.sampled_ms = now_ms;
        self.epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        self.backpressured = false;
    }
    fn collect(&mut self, world: &World) -> Result<()> {
        let mut live = BTreeSet::new();
        let mut inputs = vec![];
        let mut visible = BTreeSet::new();
        for client in world.client_ids() {
            for (id, generation, spec) in world.entities(ClientId(client)) {
                let mut system = match (&spec.vfx, &spec.particles) {
                    (Some(s), None) => s.clone(),
                    (None, Some(s)) => lower_legacy(s)?,
                    (None, None) => continue,
                    _ => anyhow::bail!("entity has both legacy and modular VFX"),
                };
                system.paused |= world.client_paused(client);
                // The renderer's GPU command batches are bounded to 32 ticks.
                system.max_steps_per_advance = system.max_steps_per_advance.min(32);
                system.compile()?;
                let owner = (client, id, generation);
                live.insert(owner);
                let key = if let Some(k) = self.owners.get(&owner) {
                    *k
                } else {
                    self.serial = self
                        .serial
                        .checked_add(1)
                        .context("VFX lifetime handle exhausted")?;
                    let key = (u64::from(client) << 32) | u64::from(self.serial);
                    self.owners.insert(owner, key);
                    key
                };
                if world.effective_visible(client, id) {
                    visible.insert(key);
                }
                inputs.push(SystemInstance {
                    key,
                    client,
                    transform: world
                        .global_matrix(EntityHandle {
                            client,
                            id,
                            generation,
                        })?
                        .to_cols_array_2d(),
                    spec: system,
                });
            }
        }
        self.owners.retain(|owner, _| live.contains(owner));
        self.inputs = inputs;
        self.visible = visible;
        Ok(())
    }
    fn trim_gpu(&mut self) {
        let valid = self.owners.values().copied().collect::<BTreeSet<_>>();
        self.gpu_history
            .retain(|_, g| valid.contains(&g.system_key));
        for g in self.gpu_history.values_mut() {
            let tick = g.progress.submitted_tick();
            g.steps.retain(|step| step.tick > tick);
        }
    }
    fn record_inputs(&mut self, now_ms: u64) -> Result<()> {
        ensure!(
            now_ms >= self.recorded_ms,
            "VFX clock moved backwards without reset"
        );
        if now_ms == self.recorded_ms && self.inputs == self.recorded_inputs {
            return Ok(());
        }
        // Identical adjacent intervals may coalesce. Changed parameters and
        // transforms must retain their original boundary while a worker runs.
        if let Some(last) = self.input_history.back_mut() {
            if last.inputs == self.inputs {
                last.time_ms = now_ms;
                self.recorded_ms = now_ms;
                return Ok(());
            }
        }
        let bytes = self
            .inputs
            .iter()
            .try_fold(0usize, |total, s| -> Result<usize> {
                Ok(total
                    + std::mem::size_of::<SystemInstance>()
                    + serde_json::to_vec(&s.spec)?.len() * 4
                    + 256)
            })?;
        ensure!(self.input_history.len()<MAX_INPUT_SAMPLES && self.input_bytes.saturating_add(bytes)<=MAX_INPUT_BYTES,
            "VFX owner input history is full; producer must fence/throttle or explicitly reset rather than discard temporal controls");
        self.input_history.push_back(InputSample {
            time_ms: now_ms,
            inputs: self.inputs.clone(),
            bytes,
        });
        self.input_bytes += bytes;
        self.recorded_ms = now_ms;
        self.recorded_inputs = self.inputs.clone();
        Ok(())
    }
    fn publish(&mut self, completed: Completed) -> Result<()> {
        if completed.epoch != self.epoch {
            return Ok(());
        }
        self.world = Some(completed.world);
        let mut frame = match completed.frame {
            Ok(frame) => frame,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                return Err(e);
            }
        };
        self.sampled_ms = completed.time_ms;
        self.accepted_inputs = completed.inputs;
        let valid = self.owners.values().copied().collect::<BTreeSet<_>>();
        frame.emitters.retain(|e| valid.contains(&e.system_key));
        frame.gpu_emitters.retain(|e| valid.contains(&e.system_key));
        self.trim_gpu();
        let mut alive = BTreeSet::new();
        for gpu in &mut frame.gpu_emitters {
            alive.insert(gpu.key);
            let cached = self.gpu_history.entry(gpu.key).or_insert_with(|| {
                let mut g = gpu.clone();
                g.steps.clear();
                g
            });
            if cached.generation != gpu.generation || cached.progress != gpu.progress {
                *cached = gpu.clone();
                cached.steps.clear();
            }
            let submitted = gpu.progress.submitted_tick();
            let last = cached.steps.last().map_or(submitted, |s| s.tick);
            for step in gpu
                .steps
                .iter()
                .filter(|s| s.tick > last && s.tick > submitted)
            {
                ensure!(
                    step.tick == cached.steps.last().map_or(submitted + 1, |s| s.tick + 1),
                    "VFX GPU timeline gap before owner publication"
                );
                cached.steps.push(step.clone());
            }
            cached.tick = gpu.tick;
            cached.emitter = gpu.emitter.clone();
            cached.transform = gpu.transform;
            cached.steps.retain(|s| s.tick > submitted);
            *gpu = cached.clone();
        }
        self.gpu_history.retain(|key, _| alive.contains(key));
        ensure!(
            self.gpu_history
                .values()
                .map(|g| g.steps.len())
                .sum::<usize>()
                <= MAX_GPU_PACKETS,
            "VFX GPU timeline budget"
        );
        ensure!(
            self.gpu_history
                .values()
                .flat_map(|g| &g.steps)
                .map(|s| packet_bytes(&s.parameters))
                .sum::<usize>()
                <= MAX_GPU_HISTORY_BYTES,
            "VFX GPU timeline byte budget"
        );
        // A local core history eviction is not a lost command: the owner retained it.
        frame.stats.gpu_history_dropped = 0;
        for event in &frame.events {
            if valid.contains(&event.system_key) {
                if self.events.len() < MAX_EVENT_QUEUE {
                    self.events.push_back(event.clone());
                } else {
                    self.dropped_events += 1;
                }
            }
        }
        self.frame = Arc::new(frame);
        self.error = None;
        Ok(())
    }
    fn poll(&mut self) -> Result<()> {
        if let Some(task) = &mut self.pending {
            match task.try_take() {
                Ok(Some(value)) => {
                    self.pending = None;
                    self.pending_screen = false;
                    self.publish(value)?;
                }
                Ok(None) => {}
                Err(e) => {
                    self.pending = None;
                    self.pending_screen = false;
                    self.error = Some(format!("VFX worker failed; reset required: {e:#}"));
                    return Err(e).context("VFX worker failed; reset required");
                }
            }
        }
        Ok(())
    }
    pub fn update(&mut self, world: &World, now_ms: u64, pool: &TaskPool) -> Result<()> {
        self.update_inner(world, now_ms, pool, true)
    }
    fn update_inner(
        &mut self,
        world: &World,
        now_ms: u64,
        pool: &TaskPool,
        record: bool,
    ) -> Result<()> {
        self.collect(world)?;
        self.poll()?;
        if record {
            self.record_inputs(now_ms)?;
        }
        self.trim_gpu();
        if self.pending.is_some() {
            return Ok(());
        }
        let caught = self.world.as_ref().is_none_or(CpuWorld::is_caught_up);
        let (inputs, time_ms, queued) = if !caught {
            (self.accepted_inputs.clone(), self.sampled_ms, false)
        } else if let Some(next) = self.input_history.front() {
            (next.inputs.clone(), next.time_ms, true)
        } else {
            return Ok(());
        };
        let gpu_count = inputs
            .iter()
            .map(|s| {
                s.spec
                    .emitters
                    .iter()
                    .filter(|e| e.target == crysirror_vfx::SimulationTarget::Gpu)
                    .count()
            })
            .sum::<usize>();
        let retained = self
            .gpu_history
            .values()
            .map(|g| g.steps.len())
            .sum::<usize>();
        let retained_bytes = self
            .gpu_history
            .values()
            .flat_map(|g| &g.steps)
            .map(|s| packet_bytes(&s.parameters))
            .sum::<usize>();
        let per_tick = inputs
            .iter()
            .map(|s| {
                s.spec
                    .emitters
                    .iter()
                    .filter(|e| e.target == crysirror_vfx::SimulationTarget::Gpu)
                    .count()
                    * packet_bytes(&s.spec.parameters)
            })
            .sum::<usize>();
        let packet_limit = if gpu_count == 0 {
            32
        } else {
            MAX_GPU_PACKETS.saturating_sub(retained) / gpu_count
        };
        let byte_limit = if per_tick == 0 {
            32
        } else {
            MAX_GPU_HISTORY_BYTES.saturating_sub(retained_bytes) / per_tick
        };
        // Leave room for a possible zero-time bootstrap in addition to work ticks.
        let budget = packet_limit.min(byte_limit).saturating_sub(1).min(32);
        self.backpressured = gpu_count > 0 && budget == 0;
        if self.backpressured {
            return Ok(());
        }
        let mut work_inputs = inputs.clone();
        if gpu_count > 0 {
            for input in &mut work_inputs {
                if input
                    .spec
                    .emitters
                    .iter()
                    .any(|e| e.target == crysirror_vfx::SimulationTarget::Gpu)
                {
                    input.spec.max_steps_per_advance =
                        input.spec.max_steps_per_advance.min(budget as u32);
                }
            }
        }
        if inputs.is_empty() && self.frame.emitters.is_empty() && self.frame.gpu_emitters.is_empty()
        {
            self.sampled_ms = time_ms;
            self.accepted_inputs = inputs;
            if queued {
                self.input_bytes -= self.input_history.pop_front().unwrap().bytes;
            }
            return Ok(());
        }
        let delta = time_ms
            .checked_sub(self.sampled_ms)
            .context("VFX clock moved backwards without reset")?;
        let Some(cpu) = self.world.take() else {
            anyhow::bail!("VFX worker state is unavailable")
        };
        let slot = Arc::new(Mutex::new(Some(cpu)));
        let work = slot.clone();
        let epoch = self.epoch;
        let pending_screen = work_inputs.iter().any(|system| {
            system
                .spec
                .emitters
                .iter()
                .any(|emitter| emitter.screen.is_some())
        });
        match pool.spawn(move |cancel| {
            cancel.check()?;
            let mut world = work
                .lock()
                .map_err(|_| anyhow::anyhow!("VFX work lock"))?
                .take()
                .context("VFX work already taken")?;
            let frame = world.advance(&work_inputs, delta);
            cancel.check()?;
            Ok(Completed {
                world,
                frame,
                time_ms,
                epoch,
                inputs,
            })
        }) {
            Ok(task) => {
                self.pending = Some(task);
                self.pending_screen = pending_screen;
                if queued {
                    self.input_bytes -= self.input_history.pop_front().unwrap().bytes;
                }
            }
            Err(e) => {
                self.world = slot
                    .lock()
                    .map_err(|_| anyhow::anyhow!("VFX work lock"))?
                    .take();
                if e.downcast_ref::<CapacityExceeded>().is_none() {
                    return Err(e);
                }
                self.backpressured = true;
            }
        }
        Ok(())
    }
    /// Offline-only completion fence. Does not advance VM/world/input clocks.
    pub fn finish_offline(&mut self, world: &World, now_ms: u64, pool: &TaskPool) -> Result<()> {
        for _ in 0..65536 {
            // Drain admitted historical samples before recording the current
            // owner state. This lets an explicit fence recover a full journal
            // without overwriting the first sample that failed admission.
            let record = self.pending.is_none()
                && self.input_history.is_empty()
                && self.world.as_ref().is_some_and(CpuWorld::is_caught_up);
            self.update_inner(world, now_ms, pool, record)?;
            if let Some(task) = self.pending.take() {
                self.pending_screen = false;
                self.publish(task.wait()?)?;
                continue;
            }
            ensure!(!self.backpressured,"VFX worker admission or GPU timeline is backpressured; complete queued work/submit the renderer before retrying the offline fence");
            if self.input_history.is_empty()
                && self.world.as_ref().is_some_and(CpuWorld::is_caught_up)
                && self.recorded_ms == now_ms
                && self.recorded_inputs == self.inputs
            {
                return Ok(());
            }
            std::thread::yield_now();
        }
        anyhow::bail!("VFX offline fence exceeded its work budget")
    }
    pub fn frame(&self) -> Arc<FrameSnapshot> {
        let mut frame = (*self.frame).clone();
        frame
            .emitters
            .retain(|e| self.visible.contains(&e.system_key));
        Arc::new(frame)
    }
    pub fn visible_systems(&self) -> Arc<BTreeSet<u64>> {
        Arc::new(self.visible.clone())
    }
    pub fn inspect(&self, client: Option<u32>) -> serde_json::Value {
        let cpu = self
            .frame
            .emitters
            .iter()
            .filter(|e| client.is_none_or(|c| e.client == c))
            .map(|e| e.particles.len())
            .sum::<usize>();
        let gpu = self
            .frame
            .gpu_emitters
            .iter()
            .filter(|e| client.is_none_or(|c| e.client == c))
            .count();
        let emitters=self.frame.emitters.iter().filter(|e|client.is_none_or(|c|e.client==c)).map(|e|{
            let bounds=if e.particles.is_empty(){serde_json::Value::Null}else{
                let (mut min,mut max)=([f32::INFINITY;3],[f32::NEG_INFINITY;3]);
                for p in e.particles.iter(){for axis in 0..3 {min[axis]=min[axis].min(p.position[axis]);max[axis]=max[axis].max(p.position[axis]);}}
                serde_json::json!({"min":min,"max":max,"space":e.space})
            };
            serde_json::json!({"system_key":e.system_key,"emitter_id":e.emitter_id,"client":e.client,"target":"cpu","live_particles":e.particles.len(),"bounds":bounds})
        }).chain(self.frame.gpu_emitters.iter().filter(|e|client.is_none_or(|c|e.client==c)).map(|e|serde_json::json!({"system_key":e.system_key,"emitter_id":e.emitter.spec.id,"client":e.client,"target":"gpu","capacity":e.emitter.spec.capacity,"live_particles":null,"planned_tick":e.tick,"submitted_tick":e.progress.submitted_tick()}))).collect::<Vec<_>>();
        let mut value = serde_json::json!({"epoch":self.epoch,"sampled_at_ms":self.sampled_ms,"pending":self.pending.is_some(),"backpressured":self.backpressured,
            "emitters":emitters,
            "pending_input_samples":self.input_history.len(),"pending_input_bytes":self.input_bytes,
            "cpu_live_particles":cpu,"gpu_emitters":gpu,"gpu_live_particles":null,"gpu_pending_packets":self.gpu_history.values().filter(|g|client.is_none_or(|c|g.client==c)).map(|g|g.steps.iter().filter(|s|s.tick>g.progress.submitted_tick()).count()).sum::<usize>(),
            "gpu_timeline_bytes":self.gpu_history.values().filter(|g|client.is_none_or(|c|g.client==c)).map(|g|g.steps.iter().filter(|s|s.tick>g.progress.submitted_tick()).map(|s|packet_bytes(&s.parameters)).sum::<usize>()).sum::<usize>(),
            "dropped_events":self.dropped_events,"error":self.error,"simulation":self.frame.stats});
        if client.is_some() {
            value.as_object_mut().unwrap().remove("simulation");
            value.as_object_mut().unwrap().remove("error");
        }
        value
    }
    pub fn drain_events(&mut self, client: u32) -> Vec<ParticleEvent> {
        let mut out = vec![];
        self.events.retain(|e| {
            if e.client == client {
                out.push(e.clone());
                false
            } else {
                true
            }
        });
        out
    }
}
