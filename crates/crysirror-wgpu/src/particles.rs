// SPDX-License-Identifier: MPL-2.0
//! CPU particle simulation and instanced billboard rendering (ADR 0014).
//! Visual only: deterministic per emitter seed, rebuilt after restore.
use crysirror_scene::view::ParticleEmitter;
use glam::{Mat4, Quat, Vec3, Vec4};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

#[derive(Clone, Copy)]
struct Particle {
    pos: Vec3,
    vel: Vec3,
    age: f32,
    life: f32,
}
struct Sim {
    parts: Vec<Particle>,
    last: f32,
    acc: f32,
    rng: u64,
    burst_done: bool,
}
fn rnd(s: &mut u64) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    (*s >> 40) as f32 / (1u64 << 24) as f32
}

fn step_sim(sim: &mut Sim, s: &crysirror_scene::components::EmitterSpec, m: &Mat4, dt: f32) {
    let origin = m.w_axis.truncate();
    let axis = m
        .transform_vector3(Vec3::from_array(s.direction))
        .normalize_or(Vec3::Y);
    let mut spawn = 0usize;
    if !sim.burst_done {
        spawn += s.burst as usize;
        sim.burst_done = true;
    }
    sim.acc += s.rate * dt;
    spawn += sim.acc.floor() as usize;
    sim.acc = sim.acc.fract();
    let room = (s.max as usize).saturating_sub(sim.parts.len());
    for _ in 0..spawn.min(room) {
        let theta = (rnd(&mut sim.rng) * s.spread_degrees).to_radians();
        let phi = rnd(&mut sim.rng) * std::f32::consts::TAU;
        let local = Vec3::new(
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        );
        let dir = Quat::from_rotation_arc(Vec3::Y, axis) * local;
        let speed = s.speed[0] + (s.speed[1] - s.speed[0]) * rnd(&mut sim.rng);
        let life = s.lifetime[0] + (s.lifetime[1] - s.lifetime[0]) * rnd(&mut sim.rng);
        // Spread births across the step to avoid pulsing.
        let f = rnd(&mut sim.rng) * dt;
        sim.parts.push(Particle {
            pos: origin + dir * speed * f,
            vel: dir * speed,
            age: f,
            life,
        });
    }
    let g = Vec3::from_array(s.gravity);
    for p in &mut sim.parts {
        p.vel += g * dt;
        p.vel *= (1. - s.drag * dt).max(0.);
        p.pos += p.vel * dt;
        p.age += dt;
    }
    sim.parts.retain(|p| p.age < p.life);
}

pub struct Particles {
    sims: HashMap<u64, Sim>,
    additive: wgpu::RenderPipeline,
    alpha: wgpu::RenderPipeline,
    buffer: Option<wgpu::Buffer>,
    add_range: std::ops::Range<u32>,
    alpha_range: std::ops::Range<u32>,
}
pub const MAX_LIVE: usize = 200_000;

impl Particles {
    pub fn new(
        d: &wgpu::Device,
        frame_bgl: &wgpu::BindGroupLayout,
        common: &str,
        msaa: u32,
    ) -> Self {
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{common}\n{}", include_str!("shaders/particle.wgsl")).into(),
            ),
        });
        let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particles"),
            bind_group_layouts: &[Some(frame_bgl)],
            immediate_size: 0,
        });
        const ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4];
        let make = |entry: &str, blend: wgpu::BlendState| {
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor { cache: None,
                label: Some("particles"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: 32,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ATTRS,
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::LessEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: msaa,
                    ..Default::default()
                },
                multiview_mask: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        Self {
            sims: HashMap::new(),
            additive: make("fs_add", add),
            alpha: make("fs_alpha", wgpu::BlendState::ALPHA_BLENDING),
            buffer: None,
            add_range: 0..0,
            alpha_range: 0..0,
        }
    }

    /// Advance every emitter to `time` and build the instance buffer.
    pub fn prepare(
        &mut self,
        d: &wgpu::Device,
        emitters: &[ParticleEmitter],
        time: f32,
        eye: Vec3,
    ) -> usize {
        let live: std::collections::HashSet<u64> = emitters.iter().map(|e| e.key).collect();
        self.sims.retain(|k, _| live.contains(k));
        let (mut add, mut alpha): (Vec<[f32; 8]>, Vec<(f32, [f32; 8])>) = (Vec::new(), Vec::new());
        for e in emitters {
            let s = &e.spec;
            let sim = self.sims.entry(e.key).or_insert_with(|| Sim {
                parts: Vec::new(),
                last: time,
                acc: 0.,
                rng: 0x9E37_79B9_7F4A_7C15 ^ (u64::from(s.seed) << 1 | 1),
                burst_done: false,
            });
            if time < sim.last {
                // Time went backwards (restore): restart deterministically.
                *sim = Sim {
                    parts: Vec::new(),
                    last: time,
                    acc: 0.,
                    rng: 0x9E37_79B9_7F4A_7C15 ^ (u64::from(s.seed) << 1 | 1),
                    burst_done: false,
                };
            }
            let fresh = sim.last == time && sim.parts.is_empty() && !sim.burst_done;
            let m = Mat4::from_cols_array_2d(&e.matrix);
            if fresh && s.rate > 0. {
                // Warm up continuous emitters to their steady state (one max lifetime).
                let mut t = 0.;
                while t < s.lifetime[1] {
                    step_sim(sim, s, &m, 1. / 30.);
                    t += 1. / 30.;
                }
            }
            let dt = (time - sim.last).min(0.1);
            sim.last = time;
            step_sim(sim, s, &m, dt);
            let (c0, c1) = (
                Vec4::from_array(s.color_start),
                Vec4::from_array(s.color_end),
            );
            for p in &sim.parts {
                let t = (p.age / p.life).clamp(0., 1.);
                let c = c0.lerp(c1, t);
                let size = s.size[0] + (s.size[1] - s.size[0]) * t;
                let inst = [p.pos.x, p.pos.y, p.pos.z, size, c.x, c.y, c.z, c.w];
                if s.additive {
                    add.push(inst);
                } else {
                    alpha.push(((p.pos - eye).length_squared(), inst));
                }
            }
        }
        alpha.sort_by(|a, b| b.0.total_cmp(&a.0));
        let total = (add.len() + alpha.len()).min(MAX_LIVE);
        let mut data: Vec<[f32; 8]> = add;
        data.extend(alpha.into_iter().map(|x| x.1));
        data.truncate(total);
        let add_count = emitters
            .iter()
            .filter(|e| e.spec.additive)
            .map(|e| self.sims.get(&e.key).map_or(0, |s| s.parts.len()))
            .sum::<usize>()
            .min(total);
        self.add_range = 0..add_count as u32;
        self.alpha_range = add_count as u32..total as u32;
        self.buffer = (!data.is_empty()).then(|| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("particles"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        total
    }
    /// Draw inside the HDR pass (frame bind group already set at 0).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, frame: &'a wgpu::BindGroup) {
        let Some(buf) = &self.buffer else { return };
        pass.set_bind_group(0, frame, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        if !self.alpha_range.is_empty() {
            pass.set_pipeline(&self.alpha);
            pass.draw(0..4, self.alpha_range.clone());
        }
        if !self.add_range.is_empty() {
            pass.set_pipeline(&self.additive);
            pass.draw(0..4, self.add_range.clone());
        }
    }
    pub fn live(&self) -> usize {
        self.sims.values().map(|s| s.parts.len()).sum()
    }
}
