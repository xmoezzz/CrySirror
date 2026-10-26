// SPDX-License-Identifier: MPL-2.0
//! Resident GPU particle simulation. Encoding, successful submission and offline
//! observation are distinct operations; interactive rendering never maps state.
use anyhow::{ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use crysirror_vfx::{
    CompiledEmitter, Interpolation, Module, ParameterMap, ParameterValue, SimulationSpace,
    SpawnShape,
};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use wgpu::util::DeviceExt;

pub const MAX_CAPACITY: u32 = 200_000;
pub const MAX_BATCH_STEPS: usize = 32;
pub const MAX_BATCH_SPAWNS: u32 = 200_000;
pub const MAX_GPU_EVENTS: u32 = 1024;
const CONTROL_HEADER_WORDS: u32 = 12;
const HEADER_WORDS: usize = 8;
const FREE_SCAN_GROUP: u32 = 128;
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Shared resident layout used directly by sprite and mesh vertex shaders.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuParticle {
    pub position_age: [f32; 4],
    pub velocity_lifetime: [f32; 4],
    pub previous_position_rotation: [f32; 4],
    pub color: [f32; 4],
    pub size: [f32; 4],
    /// Native particle ID low/high, ribbon ID, active flag.
    pub metadata: [u32; 4],
}
impl GpuParticle {
    pub fn id(&self) -> u64 {
        u64::from(self.metadata[0]) | (u64::from(self.metadata[1]) << 32)
    }
    pub fn active(&self) -> bool {
        self.metadata[3] != 0
    }
}

/// Owner-produced immutable input for exactly one fixed simulation tick.
#[derive(Clone, Debug)]
pub struct SimulationStep {
    pub tick: u64,
    pub dt: f32,
    pub time: f32,
    pub spawn_count: u32,
    pub first_id: u64,
    pub seed: u32,
    pub transform: [[f32; 4]; 4],
    pub eye: [f32; 3],
    pub vertex_count: u32,
    pub sort_alpha: bool,
    /// Parameters at this tick, not the latest frame's parameter values.
    pub parameters: Option<ParameterMap>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    transform: [[f32; 4]; 4],
    eye: [f32; 4],
    step: [f32; 4],
    counts: [u32; 4],
    flags: [u32; 4],
    tick_sort: [u32; 4],
    limits: [u32; 4],
}

/// Proof that a specific encoded batch is pending. It is not a GPU fence.
#[derive(Debug)]
pub struct EncodedBatch {
    instance: u64,
    nonce: u64,
    last_tick: u64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct GpuSimulationStats {
    pub capacity: u32,
    pub committed_tick: Option<u64>,
    pub submitted_steps: u64,
    pub encoded_steps: u64,
    pub submitted_spawn_requests: u64,
    pub visual_restarts: u64,
    pub unreplayed_ticks: u64,
}

/// Debug/offline result. Fetching it is an explicit GPU completion fence.
#[derive(Debug)]
pub struct OfflineSnapshot {
    pub particles: Vec<GpuParticle>,
    pub indices: Vec<u32>,
    pub draw_args: [u32; 4],
    pub event_count: u32,
    pub dropped_events: u32,
    pub dropped_births: u32,
    pub events: Vec<GpuEvent>,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuEvent {
    /// ID low/high, event kind (0 birth, 1 death, 2 collision), tick low.
    pub identity: [u32; 4],
    pub position: [f32; 4],
    pub velocity: [f32; 4],
}

struct PackedProgram {
    words: Vec<[u32; 4]>,
    modules: u32,
    local: bool,
}
fn bits(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}
fn interpolation(i: Interpolation) -> u32 {
    match i {
        Interpolation::Constant => 0,
        Interpolation::Linear => 1,
        Interpolation::Smooth => 2,
    }
}
impl PackedProgram {
    fn compile(plan: &CompiledEmitter) -> Result<Self> {
        plan.spec.validate(&plan.parameters)?;
        let s = &plan.spec;
        crysirror_vfx::validate_gpu_profile(s)?;
        let p = &plan.parameters;
        let init = &s.initialize;
        let direction = init.direction.resolve(p)?;
        let mut words = vec![[0; 4]; HEADER_WORDS + s.update.len() * 4];
        words[0] = bits([
            init.lifetime[0],
            init.lifetime[1],
            init.speed[0],
            init.speed[1],
        ]);
        words[1] = bits([
            direction[0],
            direction[1],
            direction[2],
            init.spread_degrees.to_radians(),
        ]);
        let color = match init.color_parameter.as_ref().and_then(|n| p.get(n)) {
            Some(ParameterValue::Color(c)) => *c,
            _ => init.color,
        };
        let size = match init.size_parameter.as_ref().and_then(|n| p.get(n)) {
            Some(ParameterValue::Vector(v)) => [v[0], v[1]],
            _ => init.size,
        };
        words[2] = bits(color);
        words[3] = bits([size[0], size[1], init.rotation[0], init.rotation[1]]);
        let (shape, a, b, c, extent): (u32, f32, f32, f32, [f32; 3]) = match &init.shape {
            SpawnShape::Point => (0, 0., 0., 0., [0.; 3]),
            SpawnShape::Sphere { radius, surface } => {
                (1, *radius, 0., if *surface { 1. } else { 0. }, [0.; 3])
            }
            SpawnShape::Box { half_extent } => (2, 0., 0., 0., *half_extent),
            SpawnShape::Cone { radius, height } => (3, *radius, *height, 0., [0.; 3]),
            SpawnShape::Line { end } => (4, 0., 0., 0., *end),
        };
        words[4] = [shape, a.to_bits(), b.to_bits(), c.to_bits()];
        words[5] = [
            extent[0].to_bits(),
            extent[1].to_bits(),
            extent[2].to_bits(),
            init.ribbon_id,
        ];
        for (i, m) in s.update.iter().enumerate() {
            let at = HEADER_WORDS + i * 4;
            let vector = |op, v: [f32; 3]| [op, v[0].to_bits(), v[1].to_bits(), v[2].to_bits()];
            match m {
                Module::Acceleration { value } => words[at] = vector(1, value.resolve(p)?),
                Module::Drag { coefficient } => {
                    words[at] = [2, coefficient.resolve(p)?.to_bits(), 0, 0]
                }
                Module::CurlNoise {
                    strength,
                    frequency,
                } => {
                    words[at] = [
                        3,
                        strength.resolve(p)?.to_bits(),
                        frequency.resolve(p)?.to_bits(),
                        0,
                    ]
                }
                Module::Attractor {
                    point,
                    strength,
                    radius,
                } => {
                    words[at] = vector(4, point.resolve(p)?);
                    words[at + 1] = bits([strength.resolve(p)?, radius.resolve(p)?, 0., 0.]);
                }
                Module::Orbit { axis, speed } => {
                    words[at] = vector(5, axis.resolve(p)?);
                    words[at + 1] = bits([speed.resolve(p)?, 0., 0., 0.]);
                }
                Module::Rotate { radians_per_second } => {
                    words[at] = [6, radians_per_second.resolve(p)?.to_bits(), 0, 0]
                }
                Module::ColorOverLife { curve } => {
                    words[at] = [
                        7,
                        words.len() as u32,
                        curve.keys.len() as u32,
                        interpolation(curve.interpolation),
                    ];
                    for k in &curve.keys {
                        words.push(bits([k.time, k.value[0], k.value[1], k.value[2]]));
                        words.push(bits([k.value[3], 0., 0., 0.]));
                    }
                }
                Module::SizeOverLife { curve } => {
                    words[at] = [
                        8,
                        words.len() as u32,
                        curve.keys.len() as u32,
                        interpolation(curve.interpolation),
                    ];
                    for k in &curve.keys {
                        words.push(bits([k.time, k.value[0], k.value[1], 0.]));
                    }
                }
                Module::CollisionPlane {
                    normal,
                    offset,
                    restitution,
                    friction,
                    kill,
                } => {
                    words[at] = vector(9, normal.resolve(p)?);
                    words[at + 1] = bits([
                        offset.resolve(p)?,
                        *restitution,
                        *friction,
                        if *kill { 1. } else { 0. },
                    ]);
                }
                Module::CollisionSphere {
                    center,
                    radius,
                    restitution,
                    kill,
                } => {
                    words[at] = vector(10, center.resolve(p)?);
                    words[at + 1] = bits([
                        radius.resolve(p)?,
                        *restitution,
                        if *kill { 1. } else { 0. },
                        0.,
                    ]);
                }
            }
        }
        Ok(Self {
            words,
            modules: s.update.len() as u32,
            local: s.space == SimulationSpace::Local,
        })
    }
}

/// One emitter's derived resident state. The engine owns its key, client and
/// generation; replacing any of those requires constructing a new simulation.
pub struct GpuSimulation {
    instance: u64,
    particles: wgpu::Buffer,
    indices: wgpu::Buffer,
    control: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    clear: wgpu::ComputePipeline,
    scan_free: wgpu::ComputePipeline,
    prefix_free: wgpu::ComputePipeline,
    spawn: wgpu::ComputePipeline,
    update: wgpu::ComputePipeline,
    compact: wgpu::ComputePipeline,
    sort: wgpu::ComputePipeline,
    capacity: u32,
    padded: u32,
    stats: GpuSimulationStats,
    nonce: u64,
    pending: Option<(u64, u64, u64, u64)>,
}
impl GpuSimulation {
    pub fn validate_program(plan: &CompiledEmitter) -> Result<()> {
        PackedProgram::compile(plan).map(|_| ())
    }
    pub fn new(d: &wgpu::Device, capacity: u32) -> Result<Self> {
        ensure!(
            (1..=MAX_CAPACITY).contains(&capacity),
            "GPU VFX capacity must be 1..={MAX_CAPACITY}"
        );
        let limits = d.limits();
        ensure!(
            limits.max_storage_buffers_per_shader_stage >= 4,
            "GPU VFX needs four compute storage-buffer bindings"
        );
        ensure!(
            u64::from(capacity) * 96 <= u64::from(limits.max_storage_buffer_binding_size),
            "GPU VFX particle storage exceeds device binding limit"
        );
        let padded = capacity.next_power_of_two();
        ensure!(
            limits.max_compute_invocations_per_workgroup >= FREE_SCAN_GROUP
                && limits.max_compute_workgroup_size_x >= FREE_SCAN_GROUP
                && limits.max_compute_workgroups_per_dimension >= padded.div_ceil(64),
            "GPU VFX compute dispatch limits are insufficient"
        );
        for bytes in [
            u64::from(capacity) * 96,
            u64::from(padded + capacity + capacity.div_ceil(FREE_SCAN_GROUP)) * 4,
            u64::from(CONTROL_HEADER_WORDS) * 4 + u64::from(MAX_GPU_EVENTS) * 48,
        ] {
            ensure!(
                bytes <= limits.max_buffer_size
                    && bytes <= u64::from(limits.max_storage_buffer_binding_size),
                "GPU VFX resident buffer exceeds actual device limits"
            );
        }
        let storage = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        let buffer = |name, size, usage| {
            d.create_buffer(&wgpu::BufferDescriptor {
                label: Some(name),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let particles = buffer("vfx resident particles", u64::from(capacity) * 96, storage);
        let indices = buffer(
            "vfx active slots and allocator scratch",
            u64::from(padded + capacity + capacity.div_ceil(FREE_SCAN_GROUP)) * 4,
            storage,
        );
        let control = buffer(
            "vfx indirect and events",
            u64::from(CONTROL_HEADER_WORDS) * 4 + u64::from(MAX_GPU_EVENTS) * 48,
            storage | wgpu::BufferUsages::INDIRECT,
        );
        let storage_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vfx compute"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<Uniform>() as u64),
                    },
                    count: None,
                },
                storage_entry(1, false),
                storage_entry(2, true),
                storage_entry(3, false),
                storage_entry(4, false),
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vfx compute"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vfx compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vfx_compute.wgsl").into()),
        });
        let pipeline = |entry| {
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let instance = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
        ensure!(instance != u64::MAX, "GPU VFX instance identity exhausted");
        Ok(Self {
            instance,
            particles,
            indices,
            control,
            clear: pipeline("clear"),
            scan_free: pipeline("scan_free"),
            prefix_free: pipeline("prefix_free"),
            spawn: pipeline("spawn"),
            update: pipeline("update"),
            compact: pipeline("compact"),
            sort: pipeline("sort_indices"),
            layout,
            capacity,
            padded,
            stats: GpuSimulationStats {
                capacity,
                ..Default::default()
            },
            nonce: 0,
            pending: None,
        })
    }
    pub fn draw_particles(&self) -> &wgpu::Buffer {
        &self.particles
    }
    pub fn draw_indices(&self) -> &wgpu::Buffer {
        &self.indices
    }
    pub fn draw_indirect(&self) -> &wgpu::Buffer {
        &self.control
    }
    pub fn stats(&self) -> GpuSimulationStats {
        self.stats
    }
    /// Explicitly start a freshly cleared visual simulation after history was
    /// unavailable. This is not a restored/checkpoint-equivalent particle state.
    pub fn acknowledge_visual_restart(&mut self, first_tick: u64) -> Result<()> {
        ensure!(
            self.pending.is_none()
                && self.stats.submitted_steps == 0
                && self.stats.committed_tick.is_none(),
            "GPU VFX visual restart requires a newly cleared simulation"
        );
        self.stats.visual_restarts = 1;
        self.stats.unreplayed_ticks = first_tick.saturating_sub(1);
        self.stats.committed_tick = first_tick.checked_sub(1);
        Ok(())
    }
    pub fn encode_batch(
        &mut self,
        d: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        plan: &CompiledEmitter,
        steps: &[SimulationStep],
    ) -> Result<EncodedBatch> {
        ensure!(
            self.pending.is_none(),
            "GPU VFX batch still pending submission or cancellation"
        );
        ensure!(
            !steps.is_empty() && steps.len() <= MAX_BATCH_STEPS,
            "GPU VFX batch must contain 1..={MAX_BATCH_STEPS} fixed steps"
        );
        ensure!(
            self.stats.committed_tick.is_some() || steps[0].tick <= 1,
            "GPU VFX initial history unavailable; reset and acknowledge_visual_restart explicitly"
        );
        ensure!(
            plan.spec.capacity == self.capacity,
            "GPU VFX capacity changed without a new generation"
        );
        let programs = steps
            .iter()
            .map(|s| {
                let at_tick = CompiledEmitter {
                    spec: plan.spec.clone(),
                    parameters: s
                        .parameters
                        .clone()
                        .unwrap_or_else(|| plan.parameters.clone()),
                };
                PackedProgram::compile(&at_tick)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut expected = match self.stats.committed_tick {
            Some(t) => Some(t.checked_add(1).context("GPU VFX tick overflow")?),
            None => None,
        };
        let mut births = 0u64;
        for s in steps {
            if let Some(t) = expected {
                ensure!(
                    s.tick == t,
                    "GPU VFX history gap: expected tick {t}, got {}; reset/replay explicitly",
                    s.tick
                );
            }
            expected = Some(s.tick.checked_add(1).context("GPU VFX tick overflow")?);
            ensure!(
                s.dt.is_finite()
                    && (0.0..=1.0).contains(&s.dt)
                    && s.time.is_finite()
                    && s.time >= 0.,
                "GPU VFX invalid fixed dt/time"
            );
            ensure!(
                s.transform
                    .iter()
                    .flatten()
                    .chain(s.eye.iter())
                    .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                "GPU VFX invalid transform/camera"
            );
            ensure!(
                s.vertex_count > 0 && s.vertex_count <= 1_000_000,
                "GPU VFX invalid indirect geometry vertex count"
            );
            ensure!(
                s.first_id.checked_add(u64::from(s.spawn_count)).is_some(),
                "GPU VFX particle ID overflow"
            );
            births += u64::from(s.spawn_count);
        }
        ensure!(
            births <= u64::from(MAX_BATCH_SPAWNS),
            "GPU VFX batch spawn budget exceeded"
        );
        let align = d.limits().min_uniform_buffer_offset_alignment.max(1) as usize;
        let stride = std::mem::size_of::<Uniform>().div_ceil(align) * align;
        let mut uniforms = vec![];
        let mut passes = vec![];
        for (s, program) in steps.iter().zip(&programs) {
            let u = Uniform {
                transform: s.transform,
                eye: [s.eye[0], s.eye[1], s.eye[2], 0.],
                step: [s.dt, s.time, 0., 0.],
                counts: [
                    self.capacity,
                    s.spawn_count,
                    s.first_id as u32,
                    (s.first_id >> 32) as u32,
                ],
                flags: [
                    s.seed,
                    u32::from(program.local),
                    program.modules,
                    s.vertex_count,
                ],
                tick_sort: [s.tick as u32, (s.tick >> 32) as u32, 0, 0],
                limits: [
                    self.padded,
                    MAX_GPU_EVENTS,
                    u32::from(s.sort_alpha),
                    self.capacity.div_ceil(FREE_SCAN_GROUP),
                ],
            };
            let ui = uniforms.len();
            uniforms.push(u);
            passes.push((0, ui, 1));
            if s.spawn_count > 0 {
                passes.extend([
                    (1, ui, self.capacity.div_ceil(FREE_SCAN_GROUP)),
                    (2, ui, 1),
                    (3, ui, self.capacity.div_ceil(64)),
                ]);
            }
            passes.extend([
                (4, ui, self.capacity.div_ceil(64)),
                (5, ui, self.padded.div_ceil(64)),
            ]);
            if s.sort_alpha {
                let mut k = 2;
                while k <= self.padded {
                    let mut j = k / 2;
                    while j > 0 {
                        let mut sort = u;
                        sort.tick_sort[2] = j;
                        sort.tick_sort[3] = k;
                        let si = uniforms.len();
                        uniforms.push(sort);
                        passes.push((6, si, self.padded.div_ceil(64)));
                        j /= 2;
                    }
                    k *= 2;
                }
            }
        }
        let mut uniform_bytes = vec![0u8; uniforms.len() * stride];
        ensure!(
            uniform_bytes.len() as u64 <= d.limits().max_buffer_size,
            "GPU VFX tick uniform buffer exceeds actual device limits"
        );
        ensure!(
            programs.iter().all(|p| p.words.len() as u64 * 16
                <= d.limits()
                    .max_buffer_size
                    .min(u64::from(d.limits().max_storage_buffer_binding_size))),
            "GPU VFX compiled program exceeds actual device limits"
        );
        for (i, u) in uniforms.iter().enumerate() {
            uniform_bytes[i * stride..i * stride + std::mem::size_of::<Uniform>()]
                .copy_from_slice(bytemuck::bytes_of(u));
        }
        let ub = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vfx immutable tick uniforms"),
            contents: &uniform_bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let binds = programs
            .iter()
            .map(|program| {
                let program_buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vfx checked program"),
                    contents: bytemuck::cast_slice(&program.words),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                d.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vfx compute"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &ub,
                                offset: 0,
                                size: NonZeroU64::new(std::mem::size_of::<Uniform>() as u64),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: self.particles.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: program_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.indices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: self.control.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect::<Vec<_>>();
        let mut current_step = 0;
        for (kind, ui, groups) in passes {
            if kind == 0 && ui != 0 {
                current_step += 1;
            }
            let pipeline = match kind {
                0 => &self.clear,
                1 => &self.scan_free,
                2 => &self.prefix_free,
                3 => &self.spawn,
                4 => &self.update,
                5 => &self.compact,
                _ => &self.sort,
            };
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("vfx fixed tick"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &binds[current_step], &[(ui * stride) as u32]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        self.nonce = self.nonce.wrapping_add(1);
        let last_tick = steps.last().unwrap().tick;
        self.pending = Some((self.nonce, last_tick, steps.len() as u64, births));
        self.stats.encoded_steps += steps.len() as u64;
        Ok(EncodedBatch {
            instance: self.instance,
            nonce: self.nonce,
            last_tick,
        })
    }
    pub fn mark_submitted(&mut self, token: EncodedBatch) -> Result<()> {
        let (n, t, steps, births) = self.pending.context("GPU VFX no pending batch")?;
        ensure!(
            self.instance == token.instance && n == token.nonce && t == token.last_tick,
            "GPU VFX stale or foreign submission token"
        );
        self.stats.committed_tick = Some(t);
        self.stats.submitted_steps += steps;
        self.stats.submitted_spawn_requests += births;
        self.pending = None;
        Ok(())
    }
    /// Only use when the owning encoder was abandoned without submission.
    pub fn cancel_pending(&mut self, token: EncodedBatch) -> Result<()> {
        ensure!(
            self.instance == token.instance
                && self
                    .pending
                    .is_some_and(|p| p.0 == token.nonce && p.1 == token.last_tick),
            "GPU VFX stale or foreign cancellation token"
        );
        self.pending = None;
        Ok(())
    }
    pub fn readback_offline(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<OfflineSnapshot> {
        ensure!(
            self.pending.is_none(),
            "submit or cancel GPU VFX batch before offline observation"
        );
        let pbytes = u64::from(self.capacity) * 96;
        let ibytes = u64::from(self.padded) * 4;
        let cbytes = u64::from(CONTROL_HEADER_WORDS) * 4 + u64::from(MAX_GPU_EVENTS) * 48;
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vfx explicit offline readback"),
            size: pbytes + ibytes + cbytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vfx offline observation"),
        });
        encoder.copy_buffer_to_buffer(&self.particles, 0, &read, 0, pbytes);
        encoder.copy_buffer_to_buffer(&self.indices, 0, &read, pbytes, ibytes);
        encoder.copy_buffer_to_buffer(&self.control, 0, &read, pbytes + ibytes, cbytes);
        queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = send.send(r);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(20)),
            })
            .map_err(|e| anyhow::anyhow!("VFX offline completion: {e}"))?;
        recv.recv_timeout(std::time::Duration::from_secs(20))
            .context("VFX offline map timeout")??;
        let mapped = read.slice(..).get_mapped_range();
        let particles =
            bytemuck::cast_slice::<u8, GpuParticle>(&mapped[..pbytes as usize]).to_vec();
        let all_indices =
            bytemuck::cast_slice::<u8, u32>(&mapped[pbytes as usize..(pbytes + ibytes) as usize]);
        let ctrl = &mapped[(pbytes + ibytes) as usize..];
        let header = bytemuck::cast_slice::<u8, u32>(&ctrl[..CONTROL_HEADER_WORDS as usize * 4]);
        let draw_args: [u32; 4] = header[..4].try_into().unwrap();
        ensure!(draw_args[1] <= self.capacity, "GPU VFX invalid draw count");
        let event_count = header[4];
        let take = event_count.min(MAX_GPU_EVENTS) as usize;
        let events =
            bytemuck::cast_slice::<u8, GpuEvent>(&ctrl[CONTROL_HEADER_WORDS as usize * 4..])
                [..take]
                .to_vec();
        let out = OfflineSnapshot {
            particles,
            indices: all_indices[..draw_args[1] as usize].to_vec(),
            draw_args,
            event_count,
            dropped_events: header[5],
            dropped_births: header[7],
            events,
        };
        drop(mapped);
        read.unmap();
        Ok(out)
    }
}
