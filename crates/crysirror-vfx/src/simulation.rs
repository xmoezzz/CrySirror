// SPDX-License-Identifier: MPL-2.0
//! Deterministic fixed-step CPU oracle and engine-owned particle frames.
use crate::*;
use anyhow::{ensure, Result};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// Device-independent particle attributes; size is full world width/height.
pub struct Particle {
    pub id: u64,
    pub position: [f32; 3],
    pub previous_position: [f32; 3],
    pub velocity: [f32; 3],
    pub age: f32,
    pub lifetime: f32,
    pub color: [f32; 4],
    pub size: [f32; 2],
    pub rotation: f32,
    pub ribbon_id: u32,
}
#[derive(Clone, Debug, PartialEq)]
/// One client-authorized authored system at an immutable affine transform.
pub struct SystemInstance {
    pub key: u64,
    pub client: u32,
    pub transform: [[f32; 4]; 4],
    pub spec: SystemSpec,
}
#[derive(Clone, Debug, PartialEq)]
/// Immutable CPU particle snapshot. The renderer owns all device resources.
pub struct EmitterFrame {
    pub system_key: u64,
    pub key: u64,
    pub client: u32,
    pub emitter_id: String,
    pub transform: [[f32; 4]; 4],
    pub space: SimulationSpace,
    pub render: RenderSpec,
    pub particles: Arc<[Particle]>,
}
#[derive(Clone, Debug, PartialEq)]
/// One fixed-clock GPU command, retaining historical parameters and transform.
pub struct TickSpawn {
    pub tick: u64,
    pub count: u32,
    pub first_id: u64,
    pub seed: u32,
    pub dt: f32,
    pub time: f32,
    pub transform: [[f32; 4]; 4],
    pub parameters: ParameterMap,
}
#[derive(Clone, Debug, PartialEq)]
/// GPU execution plan and submitted-progress receipt for one emitter lifetime.
pub struct GpuEmitterFrame {
    pub system_key: u64,
    pub key: u64,
    pub client: u32,
    pub generation: u64,
    pub emitter: CompiledEmitter,
    pub transform: [[f32; 4]; 4],
    pub steps: Vec<TickSpawn>,
    pub tick: u64,
    pub progress: GpuProgress,
}
/// Submission receipt, never project/save state or a device pointer. The owner
/// uses it to release only commands actually submitted by a renderer.
#[derive(Clone, Debug, Default)]
pub struct GpuProgress(Arc<AtomicU64>);
impl PartialEq for GpuProgress {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl GpuProgress {
    pub fn submitted_tick(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
    pub fn mark_submitted(&self, tick: u64) {
        self.0.fetch_max(tick, Ordering::Release);
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
/// Client-scoped CPU event with world-space coordinates and logical identities.
pub struct ParticleEvent {
    pub client: u32,
    pub system_key: u64,
    pub key: u64,
    pub emitter_id: String,
    pub particle_id: u64,
    pub kind: EventKind,
    /// Event coordinates are always world space, even for local simulations.
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub tick: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
/// Bounded simulation diagnostics; GPU live counts are reported by its executor.
pub struct VfxStats {
    pub systems: usize,
    pub emitters: usize,
    pub reserved_particles: usize,
    pub live_particles: usize,
    pub spawned: u64,
    pub dropped_spawns: u64,
    pub events: usize,
    pub dropped_events: u64,
    pub simulated_ticks: u64,
    pub pending_seconds: f64,
    pub gpu_history_dropped: u64,
}
#[derive(Clone, Debug, Default, PartialEq)]
/// Published immutable CPU particle and GPU command resources for one owner result.
pub struct FrameSnapshot {
    pub emitters: Vec<EmitterFrame>,
    pub gpu_emitters: Vec<GpuEmitterFrame>,
    pub events: Vec<ParticleEvent>,
    pub stats: VfxStats,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Strict reservation and queue limits checked before new particle pools allocate.
pub struct WorldBudget {
    pub max_particles: usize,
    pub max_emitters: usize,
    pub max_systems: usize,
    pub max_events: usize,
    pub max_pending_seconds: f64,
}
impl Default for WorldBudget {
    fn default() -> Self {
        Self {
            max_particles: 200_000,
            max_emitters: 256,
            max_systems: 128,
            max_events: 4096,
            max_pending_seconds: 60.,
        }
    }
}
impl WorldBudget {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=2_000_000).contains(&self.max_particles)
                && (1..=4096).contains(&self.max_emitters)
                && (1..=1024).contains(&self.max_systems)
                && (1..=65536).contains(&self.max_events)
                && self.max_pending_seconds.is_finite()
                && (0.0..=86400.).contains(&self.max_pending_seconds),
            "VFX world budget limits"
        );
        Ok(())
    }
}

/// Pure integer random oracle; dimensions are independent of allocation order.
pub fn random_u32(seed: u32, id: u64, dimension: u32) -> u32 {
    let mut x = seed
        ^ (id as u32).wrapping_mul(747796405)
        ^ ((id >> 32) as u32).wrapping_mul(2891336453)
        ^ dimension.wrapping_mul(277803737);
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    (x >> 22) ^ x
}
pub fn random_f32(seed: u32, id: u64, dimension: u32) -> f32 {
    (random_u32(seed, id, dimension) >> 8) as f32 / 16777216.
}
pub fn sample_shape(shape: &SpawnShape, seed: u32, id: u64) -> [f32; 3] {
    let a = random_f32(seed, id, 4);
    let b = random_f32(seed, id, 5);
    let c = random_f32(seed, id, 6);
    let out = match shape {
        SpawnShape::Point => Vec3::ZERO,
        SpawnShape::Sphere { radius, surface } => {
            let y = 1. - 2. * a;
            let r = (1. - y * y).max(0.).sqrt();
            let phi = std::f32::consts::TAU * b;
            let length = radius * if *surface { 1. } else { c.cbrt() };
            Vec3::new(r * phi.cos(), y, r * phi.sin()) * length
        }
        SpawnShape::Box { half_extent } => {
            Vec3::from_array(*half_extent) * Vec3::new(2. * a - 1., 2. * b - 1., 2. * c - 1.)
        }
        SpawnShape::Cone { radius, height } => {
            let y = height * a.cbrt();
            let r = radius * if *height > 0. { y / height } else { 1. } * b.sqrt();
            let phi = std::f32::consts::TAU * c;
            Vec3::new(r * phi.cos(), y, r * phi.sin())
        }
        SpawnShape::Line { end } => Vec3::from_array(*end) * a,
    };
    out.to_array()
}
/// Birth dimensions: life 0, speed 1, cone 2/3, shape 4/5/6, rotation 7.
pub fn spawn_particle(
    emitter: &CompiledEmitter,
    id: u64,
    transform: [[f32; 4]; 4],
) -> Result<Particle> {
    let s = &emitter.spec;
    let p = &emitter.parameters;
    let init = &s.initialize;
    let life = init.lifetime[0] + (init.lifetime[1] - init.lifetime[0]) * random_f32(s.seed, id, 0);
    let speed = init.speed[0] + (init.speed[1] - init.speed[0]) * random_f32(s.seed, id, 1);
    let cosine = 1. - random_f32(s.seed, id, 2) * (1. - init.spread_degrees.to_radians().cos());
    let sine = (1. - cosine * cosine).max(0.).sqrt();
    let phi = std::f32::consts::TAU * random_f32(s.seed, id, 3);
    let axis = Vec3::from_array(init.direction.resolve(p)?).normalize();
    let direction = Quat::from_rotation_arc(Vec3::Y, axis)
        * Vec3::new(sine * phi.cos(), cosine, sine * phi.sin());
    let mut position = Vec3::from_array(sample_shape(&init.shape, s.seed, id));
    let mut velocity = direction * speed;
    if s.space == SimulationSpace::World {
        let matrix = Mat4::from_cols_array_2d(&transform);
        position = matrix.transform_point3(position);
        velocity = matrix.transform_vector3(direction).normalize_or(Vec3::Y) * speed;
    }
    let color = init
        .color_parameter
        .as_ref()
        .and_then(|n| p.get(n))
        .and_then(|p| {
            if let ParameterValue::Color(v) = p {
                Some(*v)
            } else {
                None
            }
        })
        .unwrap_or(init.color);
    let size = init
        .size_parameter
        .as_ref()
        .and_then(|n| p.get(n))
        .and_then(|p| {
            if let ParameterValue::Vector(v) = p {
                Some([v[0], v[1]])
            } else {
                None
            }
        })
        .unwrap_or(init.size);
    Ok(Particle {
        id,
        position: position.to_array(),
        previous_position: position.to_array(),
        velocity: velocity.to_array(),
        age: 0.,
        lifetime: life,
        color,
        size,
        rotation: init.rotation[0]
            + (init.rotation[1] - init.rotation[0]) * random_f32(s.seed, id, 7),
        ribbon_id: init.ribbon_id,
    })
}
/// One simulation tick: ordered forces, integration, then ordered responses and
/// attributes. Newborn particles take this same tick; dead slots free next tick.
pub fn update_particle(
    particle: &mut Particle,
    emitter: &CompiledEmitter,
    transform: [[f32; 4]; 4],
    dt: f32,
    time: f32,
) -> Result<(bool, bool)> {
    ensure!(
        dt.is_finite() && (0.0..=1.).contains(&dt) && time.is_finite(),
        "invalid VFX update clock"
    );
    if dt == 0. {
        return Ok((particle.age < particle.lifetime, false));
    }
    let mut p = Vec3::from_array(particle.position);
    let mut v = Vec3::from_array(particle.velocity);
    particle.previous_position = particle.position;
    let origin = if emitter.spec.space == SimulationSpace::World {
        Mat4::from_cols_array_2d(&transform).w_axis.truncate()
    } else {
        Vec3::ZERO
    };
    for module in &emitter.spec.update {
        match module {
            Module::Acceleration { value } => {
                v += Vec3::from_array(value.resolve(&emitter.parameters)?) * dt
            }
            Module::Drag { coefficient } => {
                v *= (-coefficient.resolve(&emitter.parameters)? * dt).exp()
            }
            Module::CurlNoise {
                strength,
                frequency,
            } => {
                let f = frequency.resolve(&emitter.parameters)?;
                let a = strength.resolve(&emitter.parameters)?;
                let phase = random_f32(emitter.spec.seed, particle.id, 8) * std::f32::consts::TAU;
                v += Vec3::new(
                    -(p.z * f + time * 1.37 + phase).cos(),
                    -(p.x * f + time * 0.73 + phase).cos(),
                    -(p.y * f + time + phase).cos(),
                ) * (f * a * dt);
            }
            Module::Attractor {
                point,
                strength,
                radius,
            } => {
                let delta = Vec3::from_array(point.resolve(&emitter.parameters)?) - p;
                let distance = delta.length();
                let r = radius.resolve(&emitter.parameters)?;
                if distance < r && distance > 1e-6 {
                    v += delta / distance
                        * strength.resolve(&emitter.parameters)?
                        * (1. - distance / r).powi(2)
                        * dt;
                }
            }
            Module::Orbit { axis, speed } => {
                let q = Quat::from_axis_angle(
                    Vec3::from_array(axis.resolve(&emitter.parameters)?).normalize(),
                    speed.resolve(&emitter.parameters)? * dt,
                );
                p = origin + q * (p - origin);
                v = q * v;
            }
            Module::Rotate { radians_per_second } => {
                particle.rotation += radians_per_second.resolve(&emitter.parameters)? * dt
            }
            _ => {}
        }
    }
    p += v * dt;
    let mut collision = false;
    let mut killed = false;
    let age = particle.age + dt;
    let normalized = (age / particle.lifetime).clamp(0., 1.);
    for module in &emitter.spec.update {
        match module {
            Module::ColorOverLife { curve } => particle.color = curve.sample(normalized),
            Module::SizeOverLife { curve } => particle.size = curve.sample(normalized),
            Module::CollisionPlane {
                normal,
                offset,
                restitution,
                friction,
                kill,
            } => {
                let n = Vec3::from_array(normal.resolve(&emitter.parameters)?).normalize();
                let depth = offset.resolve(&emitter.parameters)? - n.dot(p);
                if depth > 0. {
                    collision = true;
                    p += n * depth;
                    let vn = v.dot(n);
                    if vn < 0. {
                        v = (v - n * vn) * (1. - friction) - n * vn * *restitution;
                    }
                    killed |= *kill;
                }
            }
            Module::CollisionSphere {
                center,
                radius,
                restitution,
                kill,
            } => {
                let center = Vec3::from_array(center.resolve(&emitter.parameters)?);
                let r = radius.resolve(&emitter.parameters)?;
                let delta = p - center;
                if delta.length_squared() < r * r {
                    collision = true;
                    let n = delta.normalize_or(Vec3::Y);
                    p = center + n * r;
                    let vn = v.dot(n);
                    if vn < 0. {
                        v -= n * vn * (1. + restitution);
                    }
                    killed |= *kill;
                }
            }
            _ => {}
        }
    }
    particle.age = age;
    particle.position = p.to_array();
    particle.velocity = v.to_array();
    ensure!(
        p.is_finite() && v.is_finite() && particle.rotation.is_finite(),
        "nonfinite VFX particle after update"
    );
    Ok((!killed && age < particle.lifetime, collision))
}

#[derive(Clone, Debug)]
struct EmitterState {
    client: u32,
    transform: [[f32; 4]; 4],
    system_key: u64,
    key: u64,
    generation: u64,
    spec: EmitterSpec,
    slots: Vec<Option<Particle>>,
    next_id: u64,
    spawn_fraction: f64,
    age: f64,
    tick: u64,
    history: VecDeque<TickSpawn>,
    progress: GpuProgress,
    initialized: bool,
}
#[derive(Clone, Debug)]
struct SystemState {
    client: u32,
    spec: SystemSpec,
    pending: f64,
    emitters: BTreeMap<String, EmitterState>,
    pending_events: Vec<(String, Particle, EventKind)>,
}
#[derive(Clone, Debug)]
/// Movable safe CPU reference simulator and GPU fixed-tick command planner.
pub struct CpuWorld {
    budget: WorldBudget,
    systems: BTreeMap<u64, SystemState>,
    next_emitter_key: u32,
    last: FrameSnapshot,
}
impl Default for CpuWorld {
    fn default() -> Self {
        Self::new(WorldBudget::default()).expect("default VFX budget")
    }
}
impl CpuWorld {
    pub fn new(budget: WorldBudget) -> Result<Self> {
        budget.validate()?;
        Ok(Self {
            budget,
            systems: BTreeMap::new(),
            next_emitter_key: 1,
            last: Default::default(),
        })
    }
    pub fn clear(&mut self) {
        self.systems.clear();
        self.last = Default::default();
    }
    pub fn snapshot(&self) -> &FrameSnapshot {
        &self.last
    }
    pub fn is_caught_up(&self) -> bool {
        self.systems.values().all(|s| {
            s.spec.paused
                || s.spec.time_scale == 0.
                || s.pending + 1e-10 < 1. / s.spec.fixed_step_hz as f64
        })
    }
    pub fn pending_seconds(&self) -> f64 {
        self.systems
            .values()
            .filter(|s| !s.spec.paused && s.spec.time_scale > 0.)
            .map(|s| s.pending)
            .sum()
    }
    pub fn budget(&self) -> &WorldBudget {
        &self.budget
    }
    pub fn advance(
        &mut self,
        instances: &[SystemInstance],
        delta_ms: u64,
    ) -> Result<FrameSnapshot> {
        self.advance_seconds(instances, delta_ms as f64 / 1000.)
    }
    pub fn advance_seconds(
        &mut self,
        instances: &[SystemInstance],
        delta: f64,
    ) -> Result<FrameSnapshot> {
        ensure!(
            delta.is_finite() && delta >= 0. && delta <= self.budget.max_pending_seconds,
            "VFX input delta exceeds explicit backlog budget"
        );
        ensure!(
            instances.len() <= self.budget.max_systems,
            "VFX global system budget"
        );
        let mut keys = std::collections::BTreeSet::new();
        let mut emitters = 0usize;
        let mut reserved = 0usize;
        for input in instances {
            ensure!(keys.insert(input.key), "duplicate VFX system key");
            input.spec.validate()?;
            if let Some(old) = self.systems.get(&input.key) {
                ensure!(
                    old.client == input.client,
                    "VFX system cannot transfer client authority"
                );
            }
            let matrix = Mat4::from_cols_array_2d(&input.transform);
            ensure!(
                matrix.is_finite() && matrix.determinant().abs() > 1e-12,
                "VFX transform is nonfinite/singular"
            );
            ensure!(
                matrix.x_axis.w.abs() < 1e-6
                    && matrix.y_axis.w.abs() < 1e-6
                    && matrix.z_axis.w.abs() < 1e-6
                    && (matrix.w_axis.w - 1.).abs() < 1e-6,
                "VFX simulation transform must be affine"
            );
            emitters = emitters
                .checked_add(input.spec.emitters.len())
                .ok_or_else(|| anyhow::anyhow!("VFX emitter budget overflow"))?;
            for e in &input.spec.emitters {
                reserved = reserved
                    .checked_add(e.capacity as usize)
                    .ok_or_else(|| anyhow::anyhow!("VFX capacity overflow"))?;
            }
            let old_pending = self
                .systems
                .get(&input.key)
                .map_or(input.spec.warmup_seconds as f64, |s| s.pending);
            let added = if input.spec.paused {
                0.
            } else {
                delta * input.spec.time_scale as f64
            };
            ensure!(
                old_pending + added <= self.budget.max_pending_seconds,
                "VFX backlog budget exceeded; explicit reset/seek required"
            );
        }
        ensure!(
            emitters <= self.budget.max_emitters && reserved <= self.budget.max_particles,
            "VFX global emitter/particle reservation budget exceeded before allocation"
        );
        self.systems.retain(|key, _| keys.contains(key));
        let mut frame = FrameSnapshot::default();
        frame.stats.systems = instances.len();
        frame.stats.emitters = emitters;
        frame.stats.reserved_particles = reserved;
        for input in instances {
            let state = self
                .systems
                .entry(input.key)
                .or_insert_with(|| SystemState {
                    client: input.client,
                    spec: input.spec.clone(),
                    pending: input.spec.warmup_seconds as f64,
                    emitters: BTreeMap::new(),
                    pending_events: Vec::new(),
                });
            ensure!(
                state.client == input.client,
                "VFX system cannot transfer client authority"
            );
            if state.spec.fixed_step_hz != input.spec.fixed_step_hz
                || state.spec.revision != input.spec.revision
            {
                state.emitters.clear();
                state.pending_events.clear();
                state.pending = input.spec.warmup_seconds as f64;
            }
            state.spec = input.spec.clone();
            let names = input
                .spec
                .emitters
                .iter()
                .map(|e| e.id.clone())
                .collect::<std::collections::BTreeSet<_>>();
            state.emitters.retain(|name, _| names.contains(name));
            state
                .pending_events
                .retain(|(source, _, _)| names.contains(source));
            for spec in &input.spec.emitters {
                let reset = state
                    .emitters
                    .get(&spec.id)
                    .is_some_and(|old| !same_template(&old.spec, spec));
                let previous = if reset {
                    state
                        .pending_events
                        .retain(|(source, _, _)| source != &spec.id);
                    state.emitters.remove(&spec.id)
                } else {
                    None
                };
                if !state.emitters.contains_key(&spec.id) {
                    let serial = self.next_emitter_key;
                    self.next_emitter_key = self
                        .next_emitter_key
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("VFX identity space exhausted"))?;
                    let (key, generation) = previous.map_or(
                        ((u64::from(input.client) << 32) | u64::from(serial), 1),
                        |old| (old.key, old.generation + 1),
                    );
                    state.emitters.insert(
                        spec.id.clone(),
                        EmitterState {
                            client: input.client,
                            transform: input.transform,
                            system_key: input.key,
                            key,
                            generation,
                            spec: spec.clone(),
                            slots: if spec.target == SimulationTarget::Cpu {
                                vec![None; spec.capacity as usize]
                            } else {
                                vec![]
                            },
                            next_id: 0,
                            spawn_fraction: 0.,
                            age: 0.,
                            tick: 0,
                            history: VecDeque::new(),
                            progress: GpuProgress::default(),
                            initialized: false,
                        },
                    );
                }
                state.emitters.get_mut(&spec.id).unwrap().spec = spec.clone();
                state.emitters.get_mut(&spec.id).unwrap().transform = input.transform;
            }
            let gpu_bootstrap = initialize_system(state, input, &self.budget, &mut frame)?;
            if !input.spec.paused && input.spec.time_scale > 0. {
                state.pending += delta * input.spec.time_scale as f64;
            }
            let dt = 1. / input.spec.fixed_step_hz as f64;
            let mut steps = 0;
            let gpu_budget = if input
                .spec
                .emitters
                .iter()
                .any(|e| e.target == SimulationTarget::Gpu)
            {
                32 - u32::from(gpu_bootstrap)
            } else {
                4096
            };
            while !input.spec.paused
                && input.spec.time_scale > 0.
                && state.pending + 1e-10 >= dt
                && steps < input.spec.max_steps_per_advance.min(gpu_budget)
            {
                tick_system(state, input, dt as f32, &self.budget, &mut frame)?;
                state.pending = (state.pending - dt).max(0.);
                steps += 1;
                frame.stats.simulated_ticks += 1;
            }
            if !input.spec.paused && input.spec.time_scale > 0. {
                frame.stats.pending_seconds += state.pending;
            }
            for spec in &input.spec.emitters {
                let emitter = state.emitters.get(&spec.id).unwrap();
                if spec.target == SimulationTarget::Cpu {
                    let particles = emitter.slots.iter().flatten().cloned().collect::<Vec<_>>();
                    frame.stats.live_particles += particles.len();
                    frame.emitters.push(EmitterFrame {
                        system_key: input.key,
                        key: emitter.key,
                        client: input.client,
                        emitter_id: spec.id.clone(),
                        transform: input.transform,
                        space: spec.space,
                        render: spec.render.clone(),
                        particles: Arc::from(particles),
                    });
                } else {
                    frame.gpu_emitters.push(GpuEmitterFrame {
                        system_key: input.key,
                        key: emitter.key,
                        client: input.client,
                        generation: emitter.generation,
                        emitter: spec.compile(&input.spec.parameters)?,
                        transform: input.transform,
                        steps: emitter.history.iter().cloned().collect(),
                        tick: emitter.tick,
                        progress: emitter.progress.clone(),
                    });
                }
            }
        }
        frame.stats.events = frame.events.len();
        self.last = frame.clone();
        Ok(frame)
    }
}
fn same_template(a: &EmitterSpec, b: &EmitterSpec) -> bool {
    let (mut a, mut b) = (a.clone(), b.clone());
    a.enabled = true;
    b.enabled = true;
    a.enabled_parameter = None;
    b.enabled_parameter = None;
    a.spawn.rate = 0.0.into();
    b.spawn.rate = 0.0.into();
    a == b
}
fn event(
    frame: &mut FrameSnapshot,
    budget: &WorldBudget,
    e: &EmitterState,
    particle: &Particle,
    kind: EventKind,
) {
    if frame.events.len() < budget.max_events {
        let (position, velocity) = if e.spec.space == SimulationSpace::Local {
            let matrix = Mat4::from_cols_array_2d(&e.transform);
            (
                matrix
                    .transform_point3(Vec3::from_array(particle.position))
                    .to_array(),
                matrix
                    .transform_vector3(Vec3::from_array(particle.velocity))
                    .to_array(),
            )
        } else {
            (particle.position, particle.velocity)
        };
        frame.events.push(ParticleEvent {
            client: e.client,
            system_key: e.system_key,
            key: e.key,
            emitter_id: e.spec.id.clone(),
            particle_id: particle.id,
            kind,
            position,
            velocity,
            tick: e.tick,
        });
    } else {
        frame.stats.dropped_events += 1;
    }
}
fn spawn_count(e: &mut EmitterState, spec: &SystemSpec, dt: f64) -> Result<u32> {
    let enabled = emission_enabled(spec, &e.spec);
    let end = e.age + dt;
    let mut count = 0u64;
    if enabled && e.spec.spawn.auto_start {
        let duration = e.spec.spawn.duration as f64;
        let active = if e.spec.spawn.looping {
            dt
        } else {
            (duration - e.age).max(0.).min(dt)
        };
        e.spawn_fraction += e.spec.spawn.rate.resolve(&spec.parameters)? as f64 * active;
        count = e.spawn_fraction.floor() as u64;
        e.spawn_fraction = e.spawn_fraction.fract();
        for b in &e.spec.spawn.bursts {
            let time = b.time as f64;
            let first = (((e.age - time) / duration).floor() + 1.).max(0.) as u64;
            let last = if end < time {
                None
            } else {
                Some(((end - time) / duration).floor() as u64)
            };
            if let Some(last) = last {
                let repetitions = if e.spec.spawn.looping && e.spec.spawn.burst_looping {
                    if last >= first {
                        last.saturating_sub(first).saturating_add(1)
                    } else {
                        0
                    }
                } else {
                    u64::from(first == 0 && last >= first)
                };
                count = count.saturating_add(repetitions.saturating_mul(u64::from(b.count)));
            }
        }
    }
    e.age = end;
    Ok(count.min(u64::from(u32::MAX)) as u32)
}
fn initialize_system(
    state: &mut SystemState,
    input: &SystemInstance,
    budget: &WorldBudget,
    frame: &mut FrameSnapshot,
) -> Result<bool> {
    if input.spec.paused {
        return Ok(false);
    }
    let mut gpu_bootstrap = false;
    for spec in &input.spec.emitters {
        let e = state.emitters.get_mut(&spec.id).unwrap();
        if e.initialized {
            continue;
        }
        e.initialized = true;
        let enabled = emission_enabled(&input.spec, spec) && spec.spawn.auto_start;
        let count = if enabled {
            spec.spawn
                .bursts
                .iter()
                .filter(|b| b.time == 0.)
                .map(|b| u64::from(b.count))
                .sum::<u64>()
                .min(u64::from(u32::MAX)) as u32
        } else {
            0
        };
        if count == 0 {
            continue;
        }
        let first = e.next_id;
        e.next_id = e
            .next_id
            .checked_add(u64::from(count))
            .ok_or_else(|| anyhow::anyhow!("VFX particle identity exhausted"))?;
        e.tick += 1;
        if spec.target == SimulationTarget::Gpu {
            gpu_bootstrap = true;
            let bounded = count.min(spec.capacity);
            frame.stats.dropped_spawns += u64::from(count - bounded);
            e.history.push_back(TickSpawn {
                tick: e.tick,
                count: bounded,
                first_id: first,
                seed: spec.seed,
                dt: 0.,
                time: 0.,
                transform: input.transform,
                parameters: input.spec.parameters.clone(),
            });
        } else {
            let compiled = spec.compile(&input.spec.parameters)?.specialize()?;
            let mut next = vec![];
            spawn_into(
                e,
                &compiled,
                input.transform,
                count,
                first,
                None,
                budget,
                frame,
                &mut next,
            )?;
            for event in next {
                if state.pending_events.len() < budget.max_events {
                    state.pending_events.push(event);
                } else {
                    frame.stats.dropped_events += 1;
                }
            }
        }
    }
    Ok(gpu_bootstrap)
}
fn tick_system(
    state: &mut SystemState,
    input: &SystemInstance,
    dt: f32,
    budget: &WorldBudget,
    frame: &mut FrameSnapshot,
) -> Result<()> {
    let mut spawned_events = std::mem::take(&mut state.pending_events);
    for spec in &input.spec.emitters {
        let e = state.emitters.get_mut(&spec.id).unwrap();
        e.tick += 1;
        let requested = spawn_count(e, &input.spec, dt as f64)?;
        let first = e.next_id;
        e.next_id = e
            .next_id
            .checked_add(u64::from(requested))
            .ok_or_else(|| anyhow::anyhow!("VFX particle identity exhausted"))?;
        if spec.target == SimulationTarget::Gpu {
            if e.history.len() == 32 {
                e.history.pop_front();
                frame.stats.gpu_history_dropped += 1;
            }
            e.history.push_back(TickSpawn {
                tick: e.tick,
                count: requested.min(spec.capacity),
                first_id: first,
                seed: spec.seed,
                dt,
                time: e.age as f32,
                transform: input.transform,
                parameters: input.spec.parameters.clone(),
            });
            frame.stats.dropped_spawns += u64::from(requested.saturating_sub(spec.capacity));
            continue;
        }
        let compiled = spec.compile(&input.spec.parameters)?.specialize()?;
        spawn_into(
            e,
            &compiled,
            input.transform,
            requested,
            first,
            None,
            budget,
            frame,
            &mut spawned_events,
        )?;
        for slot in 0..e.slots.len() {
            if let Some(mut particle) = e.slots[slot].take() {
                let (alive, collision) =
                    update_particle(&mut particle, &compiled, input.transform, dt, e.age as f32)?;
                if collision {
                    defer(
                        &mut spawned_events,
                        spec,
                        &particle,
                        EventKind::Collision,
                        budget,
                        frame,
                    );
                    event(frame, budget, e, &particle, EventKind::Collision);
                }
                if alive {
                    e.slots[slot] = Some(particle);
                } else {
                    defer(
                        &mut spawned_events,
                        spec,
                        &particle,
                        EventKind::Death,
                        budget,
                        frame,
                    );
                    event(frame, budget, e, &particle, EventKind::Death);
                }
            }
        }
    }
    // Child births are deferred until parent integration is complete. A bounded
    // event breadth prevents cycles from recursively running in the same tick.
    for (source, particle, kind) in spawned_events.into_iter().take(budget.max_events) {
        let Some(source_spec) = input.spec.emitters.iter().find(|e| e.id == source) else {
            continue;
        };
        for (handler_index, handler) in source_spec
            .events
            .iter()
            .enumerate()
            .filter(|(_, h)| h.event == kind)
        {
            if handler.probability <= 0.
                || random_f32(source_spec.seed, particle.id, 9 + handler_index as u32)
                    >= handler.probability
            {
                continue;
            }
            let target_spec = input
                .spec
                .emitters
                .iter()
                .find(|e| e.id == handler.emitter)
                .unwrap();
            if !emission_enabled(&input.spec, target_spec) {
                continue;
            }
            let target = state.emitters.get_mut(&handler.emitter).unwrap();
            ensure!(
                target_spec.target == SimulationTarget::Cpu,
                "VFX CPU events cannot target a GPU emitter without a device event interface"
            );
            let first = target.next_id;
            target.next_id = target
                .next_id
                .checked_add(u64::from(handler.count))
                .ok_or_else(|| anyhow::anyhow!("VFX particle identity exhausted"))?;
            let compiled = target_spec.compile(&input.spec.parameters)?.specialize()?;
            let mut next_tick = vec![];
            let mut parent = particle.clone();
            let matrix = Mat4::from_cols_array_2d(&input.transform);
            if source_spec.space == SimulationSpace::Local {
                parent.position = matrix
                    .transform_point3(Vec3::from_array(parent.position))
                    .to_array();
                parent.velocity = matrix
                    .transform_vector3(Vec3::from_array(parent.velocity))
                    .to_array();
            }
            if target_spec.space == SimulationSpace::Local {
                let inverse = matrix.inverse();
                parent.position = inverse
                    .transform_point3(Vec3::from_array(parent.position))
                    .to_array();
                parent.velocity = inverse
                    .transform_vector3(Vec3::from_array(parent.velocity))
                    .to_array();
            }
            spawn_into(
                target,
                &compiled,
                input.transform,
                handler.count,
                first,
                Some((&parent, handler.inherit_velocity)),
                budget,
                frame,
                &mut next_tick,
            )?;
            for event in next_tick {
                if state.pending_events.len() < budget.max_events {
                    state.pending_events.push(event);
                } else {
                    frame.stats.dropped_events += 1;
                }
            }
        }
    }
    Ok(())
}
fn defer(
    events: &mut Vec<(String, Particle, EventKind)>,
    spec: &EmitterSpec,
    particle: &Particle,
    kind: EventKind,
    budget: &WorldBudget,
    frame: &mut FrameSnapshot,
) {
    if spec.events.iter().any(|h| h.event == kind) {
        if events.len() < budget.max_events {
            events.push((spec.id.clone(), particle.clone(), kind));
        } else {
            frame.stats.dropped_events += 1;
        }
    }
}
fn emission_enabled(system: &SystemSpec, emitter: &EmitterSpec) -> bool {
    system.enabled
        && emitter.enabled
        && emitter.enabled_parameter.as_ref().map_or(true, |name| {
            matches!(
                system.parameters.get(name),
                Some(ParameterValue::Bool(true))
            )
        })
}
#[allow(clippy::too_many_arguments)]
fn spawn_into(
    e: &mut EmitterState,
    compiled: &CompiledEmitter,
    transform: [[f32; 4]; 4],
    requested: u32,
    first: u64,
    parent: Option<(&Particle, f32)>,
    budget: &WorldBudget,
    frame: &mut FrameSnapshot,
    new_events: &mut Vec<(String, Particle, EventKind)>,
) -> Result<()> {
    if requested == 0 {
        return Ok(());
    }
    let mut free = e
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.is_none().then_some(i))
        .collect::<VecDeque<_>>();
    let admitted = (requested as usize).min(e.slots.len());
    let first = if e.spec.overflow == OverflowPolicy::KillOldest {
        first + u64::from(requested) - admitted as u64
    } else {
        first
    };
    let mut oldest = if e.spec.overflow == OverflowPolicy::KillOldest {
        e.slots
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().map(|p| (p.id, i)))
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };
    frame.stats.dropped_spawns += requested as u64 - admitted as u64;
    for k in 0..admitted {
        let slot = if let Some(slot) = free.pop_front() {
            slot
        } else if e.spec.overflow == OverflowPolicy::KillOldest {
            oldest.pop_first().unwrap().1
        } else {
            frame.stats.dropped_spawns += 1;
            continue;
        };
        if let Some(old) = e.slots[slot].take() {
            oldest.remove(&(old.id, slot));
            event(frame, budget, e, &old, EventKind::Death);
            defer(new_events, &e.spec, &old, EventKind::Death, budget, frame);
        }
        let mut particle = spawn_particle(compiled, first + k as u64, transform)?;
        if let Some((parent, inherit)) = parent {
            let origin = if compiled.spec.space == SimulationSpace::World {
                Mat4::from_cols_array_2d(&transform).w_axis.truncate()
            } else {
                Vec3::ZERO
            };
            particle.position =
                (Vec3::from_array(particle.position) + Vec3::from_array(parent.position) - origin)
                    .to_array();
            particle.previous_position = particle.position;
            particle.velocity = (Vec3::from_array(particle.velocity)
                + Vec3::from_array(parent.velocity) * inherit)
                .to_array();
        }
        event(frame, budget, e, &particle, EventKind::Birth);
        defer(
            new_events,
            &e.spec,
            &particle,
            EventKind::Birth,
            budget,
            frame,
        );
        if e.spec.overflow == OverflowPolicy::KillOldest {
            oldest.insert((particle.id, slot));
        }
        e.slots[slot] = Some(particle);
        frame.stats.spawned += 1;
    }
    Ok(())
}
