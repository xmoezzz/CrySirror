// SPDX-License-Identifier: MPL-2.0
//! Particle effects of the `part-*.bin` DC modules and their presentation
//! through the shared VFX service.
//!
//! An `effect-group-desc` (+0xc emitter count, +0x14 emitter `StringId`s)
//! names `particle-effect-desc`s. Trigger regions name the module to load
//! (`part-module` entry; `menu-temple-ingame10`'s `part-menu`), and
//! `PartSpawner` spawners place a group (`particle-group` string) with
//! `color`, `alpha`, `sprite-scale` and `effect-scale` tags (`sub_7616E8`).
//!
//! Descriptor (version 8, magic 0xde5c; offsets from the object):
//! curves (+0x24 count, +0x30 `{u16 count, u16 first}` headers, +0x34
//! 96-byte bodies of four cubic segments `t0[4] t1[4] a[4] b[4] c[4]
//! d[4]`), ramps (+0x28, +0x38 `{u8 input, u8 count, u16 interp << 13 |
//! first}`, +0x3c `{v0, v1, p0, p1}` segments), flags (+0x44; 0x4 burst),
//! render orientation (+0x4c), material `StringId` (+0x60), the attribute
//! table (+0x9b count, +0xa4 `{StringId, slot}`), the spawn and update
//! programs (+0x9c/+0xa8, +0x9d/+0xac, 4-byte instructions), constants
//! (+0x9e, +0xb0), emission (+0xc8: a burst count or particles/s) and the
//! emitter shape (+0xec). A float parameter at or above 2⁶³ names ramp
//! `⌊v/2⁶³ − ½⌋`, at or above 2⁶⁴ curve `⌊v/2⁶⁴ − ½⌋` (`sub_FE8` of the
//! SPU particle job). The programs run on the interpreter of the SPU job
//! (`sub_12E0`, 23 operations; see [`Effect::run`]).
//!
//! Presentation: the native programs run here on sampled particles; their
//! results become the shared emitter's birth ranges and colour-over-life
//! curve, and the shared CPU simulation moves and draws the particles.
//! Not reproduced (SATD-nd-retail-runtime): per-particle random sizes (the
//! sample mean is used), the random atlas frame (frame 0), the volume
//! shape's per-particle outward/around-axis directions (approximated by
//! their dominant direction), force nodes of kinds 4, 6 and 7, and the
//! godray shader's fog-texture term (it reaches only alpha, which the
//! additive blend ignores).
use crate::dc::{Dc, sid};
use anyhow::{Result, ensure};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;

const MAGIC: u16 = 0xde5c;
const VERSION: u16 = 8;
const FLAG_BURST: u32 = 0x4;

#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    /// Attribute slot whose fraction drives the ramp; `None` normalised age.
    pub input: Option<usize>,
    pub interpolation: u8,
    /// `{v0, v1, p0, p1}` segments.
    pub segments: Vec<[f32; 4]>,
}

impl Ramp {
    /// `v0 + (v1 − v0)·f(s)`, `s = (x − p0)/(p1 − p0)`; past the last
    /// segment its `v1` (`sub_438`, `sub_BB0`).
    pub fn eval(&self, x: f32) -> f32 {
        let Some(last) = self.segments.last() else { return 0. };
        for &[v0, v1, p0, p1] in &self.segments {
            if x < p1 {
                let s = if p1 == p0 { 0. } else { ((x - p0) / (p1 - p0)).clamp(0., 1.) };
                let f = match self.interpolation {
                    0 => 0.,
                    2 => s * s,
                    3 => 1. - (1. - s) * (1. - s),
                    4 => 0.5 * (1. - (std::f32::consts::PI * s).cos()),
                    _ => s,
                };
                return v0 + (v1 - v0) * f;
            }
        }
        last[1]
    }
}

/// A looping piecewise cubic, segments `[t0, t1, a, b, c, d]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve(pub Vec<[f32; 6]>);

impl Curve {
    pub fn period(&self) -> f32 {
        self.0.last().map_or(0., |s| s[1])
    }

    /// `((a·u + b)·u + c)·u + d` with `u = t − t0`, `t` wrapped by the last
    /// segment's end (`loc_7C8`).
    pub fn eval(&self, t: f32) -> f32 {
        let period = self.period();
        let t = if period > 0. { t.rem_euclid(period) } else { t };
        let Some(last) = self.0.last() else { return 0. };
        let s = self.0.iter().find(|s| t < s[1]).unwrap_or(last);
        let u = t - s[0];
        ((s[2] * u + s[3]) * u + s[4]) * u + s[5]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Life {
    Infinite,
    Fixed(f32),
    /// `L + U[0, 1)·R`.
    Random(f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Kind 0: a point at `offset` (spawner-local).
    Point { offset: Vec3, life: Life },
    /// Kind 1 volume emitter: `size` (full extents) about `offset`.
    Volume { kind: u8, offset: Vec3, size: Vec3 },
}

/// Birth velocity terms of the emitter shape (Maya emitter attributes,
/// exported field by field). Point shape (kind 0, `#1d9d708e`): +0x04 mode
/// (0 omni, 1 directional), +0x30 direction, +0x40 speed, +0x44 speed
/// random, +0x48 spread (fraction of 180°). Volume shape (kind 1,
/// `#aee75cf5`): +0x40 direction, +0x50 speed (a multiplier of the terms
/// that follow), +0x54 speed random, +0x58 away-from-centre, +0x5c
/// away-from-axis, +0x60 along-axis, +0x64 around-axis, +0x68 random
/// direction. Fields may be curve or ramp references (`sub_FE8`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launch {
    pub speed: f32,
    pub speed_random: f32,
    pub direction: Vec3,
    /// Point shape: spread as a fraction of 180°; 1 for omni.
    pub spread: f32,
    /// Volume shape terms.
    pub away_from_center: f32,
    pub away_from_axis: f32,
    pub along_axis: f32,
    pub around_axis: f32,
    pub random_direction: f32,
}

/// A force node (+0xf0.., the first +0x9f; Maya fields): `{kind, 0,
/// magnitude +0x8, …, max distance +0x10 (−1 unlimited), position +0x20,
/// …, direction or frequency +0x50}`.
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// Kind 2: drag of `magnitude`.
    Drag(f32),
    /// Kind 3: gravity along `direction`.
    Gravity { magnitude: f32, direction: Vec3 },
    /// Kind 8: turbulence of `magnitude` and `frequency`.
    Turbulence { magnitude: f32, frequency: f32 },
    /// Kind 5: a radial field about `position` within `max_distance`.
    Radial { magnitude: f32, position: Vec3, max_distance: f32 },
    /// Kinds 0, 1 (the always-first `{1, 1.0}` node), 4, 6, 7: not
    /// mapped.
    Other(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub name: String,
    pub flags: u32,
    /// Render orientation 0–7 (`sub_4710` of the cull job).
    pub orientation: u8,
    pub material: u32,
    /// Attribute name `StringId` → slot.
    pub slots: HashMap<u32, usize>,
    pub constants: Vec<f32>,
    pub curves: Vec<Curve>,
    pub ramps: Vec<Ramp>,
    pub spawn: Vec<[u8; 4]>,
    pub update: Vec<[u8; 4]>,
    /// Raw +0xc8 parameter.
    pub emission: f32,
    pub shape: Shape,
    pub launch: Launch,
    pub fields: Vec<Field>,
}

/// Per-particle state of the interpreter.
#[derive(Clone, Debug)]
pub struct Particle {
    pub attrs: Vec<f32>,
    pub seed: u16,
}

/// Times an evaluation reads (seconds).
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    pub age: f32,
    pub effect_time: f32,
    pub dt: f32,
}

impl Effect {
    pub fn slot(&self, name: &str) -> Option<usize> {
        self.slots.get(&sid(name)).copied()
    }

    /// A float parameter: a constant or a ramp/curve reference.
    pub fn param(&self, v: f32, time: f32) -> f32 {
        if v >= 2f32.powi(64) {
            self.curves.get((v / 2f32.powi(64) - 0.5) as usize).map_or(0., |c| c.eval(time))
        } else if v >= 2f32.powi(63) {
            self.ramps.get((v / 2f32.powi(63) - 0.5) as usize).map_or(0., |r| r.eval(0.))
        } else {
            v
        }
    }

    /// Run `program` on a particle (`sub_12E0`). The random state restarts
    /// from the particle's seed on every run, so the update program
    /// re-draws the spawn program's per-particle values in order.
    pub fn run(&self, program: &[[u8; 4]], p: &mut Particle, clock: Clock) {
        let mut r = [0f32; 16];
        let mut rng = p.seed;
        let lifespan = self.slot("lifespan").and_then(|s| p.attrs.get(s)).copied().unwrap_or(-1.);
        let t = if lifespan > 0. { clock.age / lifespan } else { 0. };
        let attr = |p: &Particle, i: usize| p.attrs.get(i).copied().unwrap_or(0.);
        for &[op, dest, a, c] in program {
            let (d, a, b, lo) = (usize::from(dest & 15), usize::from(a), usize::from(c >> 4), usize::from(c & 15));
            let ra = r[a & 15];
            let v = match op {
                0 => attr(p, a),
                1 => self.ramps.get(a + b).map_or(0., |ramp| ramp.eval(ramp.input.map_or(t, |i| attr(p, i).rem_euclid(1.)))),
                2 => {
                    if let Some(x) = p.attrs.get_mut(usize::from(dest)) {
                        *x = ra;
                    }
                    continue;
                }
                3 => ra,
                4 => ra + r[b],
                5 => ra - r[b],
                6 => ra * r[b],
                7 => ra / r[b],
                8 => {
                    if r[b] == 0. { 0. } else { ra % r[b] }
                }
                9 => self.constants.get(a).copied().unwrap_or(0.),
                10 => match a {
                    0 => clock.effect_time,
                    1 => clock.dt,
                    2 => clock.age,
                    _ => 0.,
                },
                11 | 12 => 0.,
                13 => clock.effect_time,
                14 => clock.age,
                15 => {
                    let u = f32::from(rng) / 65536.;
                    rng = rng.wrapping_mul(0x6255).wrapping_add(0x3619);
                    ra + u * (r[b] - ra)
                }
                16 | 17 => {
                    let span = r[b] - ra;
                    let s = if span == 0. { 0. } else { ((r[lo] - ra) / span).clamp(0., 1.) };
                    if op == 16 { s * s * (3. - 2. * s) } else { s }
                }
                18 => ra.max(r[lo]).min(r[b]),
                19 => (ra * ra + r[b] * r[b] + r[lo] * r[lo]).sqrt(),
                20 => {
                    if ra == 0. { 0. } else { ra.signum() }
                }
                21 => ra.sin(),
                22 => ra.cos(),
                _ => continue,
            };
            r[d] = if v.is_finite() { v } else { 0. };
        }
        p.seed = rng;
    }

    /// A new particle after the shape's lifespan and the spawn program.
    pub fn birth(&self, seed: u16) -> Particle {
        let slots = self.slots.values().copied().max().map_or(0, |m| m + 1);
        let mut p = Particle { attrs: vec![0.; slots], seed };
        let lifespan = match &self.shape {
            Shape::Point { life: Life::Fixed(l), .. } => *l,
            Shape::Point { life: Life::Random(l, r), .. } => l + f32::from(seed) / 65536. * r,
            _ => -1.,
        };
        if let Some(s) = self.slot("lifespan") {
            p.attrs[s] = lifespan;
        }
        self.run(&self.spawn, &mut p, Clock::default());
        p.seed = seed;
        p
    }
}

/// Every effect group and descriptor of a particle module.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub groups: HashMap<u32, Vec<u32>>,
    pub effects: HashMap<u32, Effect>,
}

impl Library {
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self> {
        let mut dc = Dc::default();
        let module = dc.load(sid(name), bytes)?.clone();
        let m = &dc.mem;
        let mut out = Self::default();
        for (n, a) in &module.entries {
            let kind = m.type_of(*a).ok();
            if kind == Some(sid("effect-group-desc")) {
                let count = m.u32(a + 0xc)?;
                ensure!(count <= 256, "ND effect group size");
                let array = m.u32(a + 0x14)?;
                out.groups.insert(*n, (0..count).map(|i| m.u32(array + 4 * i)).collect::<Result<_>>()?);
            } else if kind == Some(sid("particle-effect-desc")) {
                if let Some(e) = effect(&dc, *a)? {
                    out.effects.insert(*n, e);
                }
            }
        }
        Ok(out)
    }

    pub fn extend(&mut self, other: Self) {
        self.groups.extend(other.groups);
        self.effects.extend(other.effects);
    }
}

fn u8_at(dc: &Dc, at: u32) -> Result<u8> {
    Ok(dc.mem.bytes(at, 1)?[0])
}

fn u16_at(dc: &Dc, at: u32) -> Result<u16> {
    let b = dc.mem.bytes(at, 2)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn vec3(dc: &Dc, at: u32) -> Result<Vec3> {
    Ok(Vec3::new(dc.mem.f32(at)?, dc.mem.f32(at + 4)?, dc.mem.f32(at + 8)?))
}

/// A version-8 descriptor; `None` for other versions.
fn effect(dc: &Dc, p: u32) -> Result<Option<Effect>> {
    let m = &dc.mem;
    ensure!(u16_at(dc, p)? == MAGIC, "ND particle descriptor magic");
    if u16_at(dc, p + 2)? != VERSION {
        return Ok(None);
    }
    let curves = {
        let (heads, bodies) = (m.u32(p + 0x30)?, m.u32(p + 0x34)?);
        (0..u32::from(u16_at(dc, p + 0x24)?))
            .map(|i| {
                let (n, first) = (u32::from(u16_at(dc, heads + 4 * i)?), u32::from(u16_at(dc, heads + 4 * i + 2)?));
                ensure!(n <= 64, "ND particle curve size");
                let mut segments = Vec::new();
                for k in first..first + n {
                    let w = |j: u32| m.f32(bodies + 96 * k + 4 * j);
                    for j in 0..4 {
                        // Bodies pad unused segments with empty ranges.
                        if w(4 + j)? > w(j)? || (k == first && j == 0) {
                            segments.push([w(j)?, w(4 + j)?, w(8 + j)?, w(12 + j)?, w(16 + j)?, w(20 + j)?]);
                        }
                    }
                }
                Ok(Curve(segments))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let ramps = {
        let (heads, keys) = (m.u32(p + 0x38)?, m.u32(p + 0x3c)?);
        (0..u32::from(u16_at(dc, p + 0x28)?))
            .map(|i| {
                let (input, n, h) = (u8_at(dc, heads + 4 * i)?, u32::from(u8_at(dc, heads + 4 * i + 1)?), u16_at(dc, heads + 4 * i + 2)?);
                let first = u32::from(h & 0x1fff);
                let segments = (first..first + n).map(|k| Ok([m.f32(keys + 16 * k)?, m.f32(keys + 16 * k + 4)?, m.f32(keys + 16 * k + 8)?, m.f32(keys + 16 * k + 12)?])).collect::<Result<Vec<_>>>()?;
                Ok(Ramp { input: (input != 0xff).then_some(usize::from(input)), interpolation: (h >> 13) as u8, segments })
            })
            .collect::<Result<Vec<_>>>()?
    };
    let table = m.u32(p + 0xa4)?;
    let mut slots = HashMap::new();
    for i in 0..u32::from(u8_at(dc, p + 0x9b)?) {
        let slot = m.u32(table + 8 * i + 4)? as i32;
        if (0..256).contains(&slot) {
            slots.insert(m.u32(table + 8 * i)?, slot as usize);
        }
    }
    let program = |count: u32, at: u32| -> Result<Vec<[u8; 4]>> {
        if count == 0 {
            return Ok(Vec::new());
        }
        let base = m.u32(at)?;
        (0..count).map(|i| Ok(m.bytes(base + 4 * i, 4)?.try_into()?)).collect()
    };
    let constants = {
        let base = m.u32(p + 0xb0)?;
        (0..u32::from(u8_at(dc, p + 0x9e)?)).map(|i| m.f32(base + 4 * i)).collect::<Result<Vec<_>>>()?
    };
    let sh = m.u32(p + 0xec)?;
    let shape = if m.u32(sh)? == 0 {
        let life = match u8_at(dc, sh + 5)? {
            0 => Life::Infinite,
            1 => Life::Random(m.f32(sh + 0x54)?, m.f32(sh + 0x58)?),
            _ => Life::Fixed(m.f32(sh + 0x54)?),
        };
        Shape::Point { offset: vec3(dc, sh + 0x10)?, life }
    } else {
        Shape::Volume { kind: u8_at(dc, sh + 4)?, offset: vec3(dc, sh + 0x10)?, size: vec3(dc, sh + 0x30)? }
    };
    let launch = if m.u32(sh)? == 0 {
        let omni = u8_at(dc, sh + 4)? == 0;
        Launch { speed: m.f32(sh + 0x40)?, speed_random: m.f32(sh + 0x44)?, direction: vec3(dc, sh + 0x30)?, spread: if omni { 1. } else { m.f32(sh + 0x48)? }, ..Default::default() }
    } else {
        let f = |o: u32| m.f32(sh + o);
        Launch {
            speed: f(0x50)?,
            speed_random: f(0x54)?,
            direction: vec3(dc, sh + 0x40)?,
            spread: 0.,
            away_from_center: f(0x58)?,
            away_from_axis: f(0x5c)?,
            along_axis: f(0x60)?,
            around_axis: f(0x64)?,
            random_direction: f(0x68)?,
        }
    };
    let mut fields = Vec::new();
    for k in 0..u32::from(u8_at(dc, p + 0x9f)?).min(7) {
        let q = m.u32(p + 0xf0 + 4 * k)?;
        if q == 0 {
            continue;
        }
        let magnitude = m.f32(q + 8)?;
        fields.push(match m.u32(q)? {
            2 => Field::Drag(magnitude),
            3 => Field::Gravity { magnitude, direction: vec3(dc, q + 0x50)? },
            8 => Field::Turbulence { magnitude, frequency: m.f32(q + 0x50)? },
            5 => Field::Radial { magnitude, position: vec3(dc, q + 0x20)?, max_distance: m.f32(q + 0x10)? },
            other => Field::Other(other),
        });
    }
    Ok(Some(Effect {
        launch,
        fields,
        name: m.cstr(m.u32(p + 0x10)?).unwrap_or_default(),
        flags: m.u32(p + 0x44)?,
        orientation: u8_at(dc, p + 0x4c)?,
        material: m.u32(p + 0x60)?,
        slots,
        constants,
        curves,
        ramps,
        spawn: program(u32::from(u8_at(dc, p + 0x9c)?), p + 0xa8)?,
        update: program(u32::from(u8_at(dc, p + 0x9d)?), p + 0xac)?,
        emission: m.f32(p + 0xc8)?,
        shape,
    }))
}

/// A particle material of `particle.pak`'s `MATERIAL_TABLE_1` (+0xc record
/// count, +0x20 record pointers, record +0x18 `StringId`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleMaterial {
    pub name: String,
    /// Pass 0's sampler texture IDs.
    pub samplers: Vec<u32>,
    /// Pass 0's colour blend factors (pass data +0x20, `u16` source and
    /// destination in GL enumerants: 1 ONE, 0x303 ONE_MINUS_SRC_ALPHA).
    pub blend: (u16, u16),
    /// Soft-particle distance: the fragment program divides the scene
    /// depth difference by `g_fDepthBlend` (`DIV_SAT`; 0.5 m on
    /// `godrays-fog`).
    pub depth_blend: Option<f32>,
}

pub fn materials(p: &crate::mesh::Loaded, payload: usize) -> Result<HashMap<u32, ParticleMaterial>> {
    let count = p.u32(payload + 0xc)? as usize;
    ensure!(count <= 4096, "ND material table size");
    let array = p.ptr(payload + 0x20)?;
    let mut out = HashMap::new();
    for i in 0..count {
        let Ok(m) = p.ptr(array + 4 * i) else { continue };
        let Ok(material) = crate::mesh::material(p, m) else { continue };
        let data = p.ptr(p.ptr(m + 0x10)?).and_then(|pass| p.ptr(pass + 0x10))?;
        let depth_blend = crate::rsx::material_constant(p, data, "g_fDepthBlend").ok().flatten().map(|v| v[0]).filter(|v| v.is_finite() && *v > 0.);
        out.insert(p.u32(m + 0x18)?, ParticleMaterial { name: material.name, samplers: material.samplers, blend: (p.u16(data + 0x20)?, p.u16(data + 0x22)?), depth_blend });
    }
    Ok(out)
}

/// The shared blend of native factors. The dust shader writes alpha 0, so
/// `ONE, ONE_MINUS_SRC_ALPHA` adds like `ONE, ONE` (its fade is the
/// vertex alpha, kept here).
pub fn blend(factors: (u16, u16)) -> crysirror_vfx::BlendMode {
    match factors {
        (0x302, 0x303) => crysirror_vfx::BlendMode::Alpha,
        (0x306, 0) | (0, 0x300) => crysirror_vfx::BlendMode::Multiply,
        _ => crysirror_vfx::BlendMode::Additive,
    }
}

/// A `PartSpawner`'s tags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Vec3,
    pub rotation: Quat,
    pub color: [f32; 3],
    pub alpha: f32,
    pub sprite_scale: f32,
    pub effect_scale: Vec3,
}

/// The quad plane of render orientations 5–7 in the spawner frame, as
/// the columns (quad X, quad Y, normal) of a rotation: 5 the plane of
/// spawner Y and Z (rows −Z, Y, X), 6 of X and Z (rows X, −Z, Y), 7 the
/// spawner frame (`sub_4710`).
fn plane(orientation: u8) -> Option<Mat4> {
    let m = |x: Vec3, y: Vec3, z: Vec3| Mat4::from_cols(x.extend(0.), y.extend(0.), z.extend(0.), glam::Vec4::W);
    match orientation {
        5 => Some(m(-Vec3::Z, Vec3::Y, Vec3::X)),
        6 => Some(m(Vec3::X, -Vec3::Z, Vec3::Y)),
        7 => Some(Mat4::IDENTITY),
        _ => None,
    }
}

/// Sampled particles per descriptor.
const SAMPLES: u16 = 64;
const LIFE_KEYS: usize = 16;
/// Longest shared-VFX lifetime; infinite native particles are re-born
/// after it.
const MAX_LIFE: f32 = 3600.;

/// The shared emitter for `effect` at `placement` and its world transform.
pub fn emitter(effect: &Effect, id: &str, placement: &Placement, texture: Option<(String, u32)>, blend: crysirror_vfx::BlendMode, soft_distance: f32, seed: u32) -> (crysirror_vfx::EmitterSpec, Mat4) {
    use crysirror_vfx::{Burst, ColorCurve, ColorKey, EmitterSpec, FacingMode, InitSpec, Interpolation, Module, RenderSpec, SpawnShape, SpawnSpec, Vec2Curve, Vec2Key};
    let seeds = (0..SAMPLES).map(|i| (i as u16).wrapping_mul(0x9e37).wrapping_add(0x7f4a));
    let born: Vec<Particle> = seeds.map(|s| effect.birth(s)).collect();
    let read = |p: &Particle, name: &str| effect.slot(name).and_then(|s| p.attrs.get(s)).copied().unwrap_or(0.);
    let lives: Vec<f32> = born.iter().map(|p| read(p, "lifespan")).collect();
    let infinite = lives.iter().any(|l| *l <= 0.);
    let lifetime = if infinite {
        [MAX_LIFE, MAX_LIFE]
    } else {
        let lo = lives.iter().copied().fold(f32::MAX, f32::min).clamp(0.01, MAX_LIFE);
        [lo, lives.iter().copied().fold(0., f32::max).clamp(lo, MAX_LIFE)]
    };
    let twist: Vec<f32> = born.iter().map(|p| read(p, "twist").to_radians()).filter(|v| v.is_finite()).collect();
    let rotation = if twist.is_empty() { [0., 0.] } else { [twist.iter().copied().fold(f32::MAX, f32::min), twist.iter().copied().fold(f32::MIN, f32::max)] };
    // Mean colour, alpha and quad size at normalised age `t`, × the
    // spawner's colour, alpha and sprite scale.
    let state_at = |t: f32| -> [f32; 6] {
        let mut sum = [0f32; 6];
        for (p, life) in born.iter().zip(&lives) {
            let mut q = p.clone();
            if !effect.update.is_empty() {
                let age = if *life > 0. { t * life } else { t * 10. };
                effect.run(&effect.update, &mut q, Clock { age, effect_time: age, dt: 1. / 30. });
            }
            for (k, name) in ["colorR", "colorG", "colorB", "alpha", "scaleX", "scaleY"].iter().enumerate() {
                sum[k] += if k >= 4 { read(&q, name).abs() } else { read(&q, name) };
            }
        }
        let n = born.len() as f32;
        let tint = [placement.color[0], placement.color[1], placement.color[2], placement.alpha, placement.sprite_scale, placement.sprite_scale];
        let v: [f32; 6] = std::array::from_fn(|k| sum[k] / n * tint[k]);
        let mut v = v.map(|v| if v.is_finite() { v.clamp(0., 1e5) } else { 0. });
        v[3] = v[3].min(1.);
        v
    };
    let keys: Vec<[f32; 6]> = (0..LIFE_KEYS).map(|k| state_at(k as f32 / (LIFE_KEYS - 1) as f32)).collect();
    let mut update = Vec::new();
    let first = if infinite {
        // Infinite particles: the update program's mean over 10 s.
        let n = keys.len() as f32;
        std::array::from_fn(|c| keys.iter().map(|k| k[c]).sum::<f32>() / n)
    } else {
        let time = |k: usize| k as f32 / (LIFE_KEYS - 1) as f32;
        let varies = |c: std::ops::Range<usize>| keys.iter().any(|k| c.clone().any(|i| (k[i] - keys[0][i]).abs() > 0.02 * keys[0][i].abs().max(1e-3)));
        if varies(0..4) {
            update.push(Module::ColorOverLife { curve: ColorCurve { keys: keys.iter().enumerate().map(|(k, v)| ColorKey { time: time(k), value: [v[0], v[1], v[2], v[3]] }).collect(), interpolation: Interpolation::Linear } });
        }
        if varies(4..6) {
            update.push(Module::SizeOverLife { curve: Vec2Curve { keys: keys.iter().enumerate().map(|(k, v)| Vec2Key { time: time(k), value: [v[4], v[5]] }).collect(), interpolation: Interpolation::Linear } });
        }
        keys[0]
    };
    let (color, size) = ([first[0], first[1], first[2], first[3]], [first[4], first[5]]);
    let scale = placement.effect_scale;
    // Shape fields may be curve or ramp references.
    let resolve = |v: Vec3| Vec3::new(effect.param(v.x, 0.), effect.param(v.y, 0.), effect.param(v.z, 0.)).clamp(Vec3::splat(-1e4), Vec3::splat(1e4));
    let (offset, shape) = match &effect.shape {
        Shape::Point { offset, .. } => (resolve(*offset), SpawnShape::Point),
        Shape::Volume { offset, size, .. } => (resolve(*offset), SpawnShape::Box { half_extent: (resolve(*size) * scale * 0.5).abs().to_array() }),
    };
    let rate_param = |t: f32| effect.param(effect.emission, t).max(0.);
    let (spawn, capacity) = if effect.flags & FLAG_BURST != 0 {
        let count = (rate_param(0.) as u32).min(4096);
        (SpawnSpec { rate: 0f32.into(), bursts: vec![Burst { time: 0., count }], duration: lifetime[1], looping: infinite, burst_looping: infinite, auto_start: true }, count.max(1))
    } else {
        // The mean of a rate curve over its period.
        let period = match effect.emission {
            v if v >= 2f32.powi(64) => effect.curves.get((v / 2f32.powi(64) - 0.5) as usize).map_or(1., |c| c.period().max(0.01)),
            _ => 1.,
        };
        let rate = (0..64).map(|i| rate_param(period * i as f32 / 64.)).sum::<f32>() / 64.;
        let rate = rate.min(1000.);
        (SpawnSpec { rate: rate.into(), bursts: vec![], duration: period, looping: true, burst_looping: true, auto_start: true }, ((rate * lifetime[1] * 1.25) as u32 + 4).clamp(1, 16384))
    };
    // Birth velocity: the shape's speed terms (the volume terms by their
    // dominant direction) and the force nodes as shared modules.
    let l = &effect.launch;
    let p0 = |v: f32| effect.param(v, 0.);
    let (speed_scale, speed_random) = (p0(l.speed), p0(l.speed_random).abs());
    let (direction, spread, base) = match &effect.shape {
        Shape::Point { .. } => (l.direction, p0(l.spread).clamp(0., 1.) * 180., speed_scale),
        Shape::Volume { .. } => {
            let (away, along, random) = (p0(l.away_from_center) + p0(l.away_from_axis), p0(l.along_axis), p0(l.random_direction));
            // Outward and random terms point anywhere; along-axis is +Y.
            if along.abs() >= away.abs() + random.abs() { (Vec3::Y * along.signum(), 0., speed_scale * along.abs()) } else { (Vec3::Y, 180., speed_scale * (away.abs() + random.abs())) }
        }
    };
    let speed = [(base - speed_random * 0.5).max(0.), base.max(0.) + speed_random * 0.5];
    let direction = direction.try_normalize().unwrap_or(Vec3::Y);
    for f in &effect.fields {
        match *f {
            Field::Drag(m) if p0(m) > 0. => update.insert(0, crysirror_vfx::Module::Drag { coefficient: p0(m).into() }),
            Field::Gravity { magnitude, direction } => {
                let a = direction.try_normalize().unwrap_or(-Vec3::Y) * p0(magnitude);
                update.insert(0, crysirror_vfx::Module::Acceleration { value: a.to_array().into() });
            }
            Field::Turbulence { magnitude, frequency } if p0(magnitude) != 0. => update.insert(0, crysirror_vfx::Module::CurlNoise { strength: p0(magnitude).abs().into(), frequency: p0(frequency).abs().max(0.01).into() }),
            Field::Radial { magnitude, position, max_distance } if p0(magnitude) != 0. => update.insert(0, crysirror_vfx::Module::Attractor { point: position.to_array().into(), strength: (-p0(magnitude)).into(), radius: (if max_distance > 0. { max_distance } else { 1000. }).into() }),
            _ => {}
        }
    }
    let facing = match effect.orientation {
        1 => FacingMode::Axis { axis: [0., 1., 0.] },
        2..=4 => FacingMode::Velocity,
        5..=7 => FacingMode::Local,
        _ => FacingMode::Camera,
    };
    let (texture, columns) = texture.map_or((None, 1), |(t, c)| (Some(t), c));
    let spec = EmitterSpec {
        id: id.to_owned(),
        seed,
        capacity,
        spawn,
        initialize: InitSpec { shape, lifetime, speed, direction: direction.to_array().into(), spread_degrees: spread, color, size, rotation, ..Default::default() },
        update,
        render: RenderSpec::Sprite { texture, columns, rows: 1, frames: 1, fps: 0., soft_distance, blend, facing },
        ..Default::default()
    };
    let frame = Mat4::from_rotation_translation(placement.rotation, placement.position) * Mat4::from_translation(offset * scale);
    (spec, frame * plane(effect.orientation).unwrap_or(Mat4::IDENTITY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_and_curve() {
        let ramp = Ramp { input: None, interpolation: 1, segments: vec![[0., 1., 0., 0.15], [1., 1., 0.15, 0.845], [1., 0., 0.845, 1.]] };
        assert!((ramp.eval(0.075) - 0.5).abs() < 1e-6);
        assert_eq!(ramp.eval(0.5), 1.);
        assert_eq!(ramp.eval(2.), 0.);
        let curve = Curve(vec![[0., 1., 0., 0., 1., 2.], [1., 2., 0., 0., 0., 3.]]);
        assert_eq!(curve.eval(0.5), 2.5);
        assert_eq!(curve.eval(2.5), 2.5);
    }

    #[test]
    fn launch_and_fields_become_shared_motion() {
        let e = Effect {
            name: "dust".into(),
            flags: 0,
            orientation: 0,
            material: 0,
            slots: HashMap::new(),
            constants: vec![],
            curves: vec![],
            ramps: vec![],
            spawn: vec![],
            update: vec![],
            emission: 10.,
            shape: Shape::Point { offset: Vec3::ZERO, life: Life::Fixed(2.) },
            launch: Launch { speed: 2., speed_random: 1., direction: Vec3::Y, spread: 0.25, ..Default::default() },
            fields: vec![Field::Drag(3.), Field::Gravity { magnitude: 0.5, direction: Vec3::new(0., -1., 0.) }, Field::Other(1)],
        };
        let placement = Placement { position: Vec3::ZERO, rotation: Quat::IDENTITY, color: [1.; 3], alpha: 1., sprite_scale: 1., effect_scale: Vec3::ONE };
        let (spec, _) = emitter(&e, "dust", &placement, None, crysirror_vfx::BlendMode::Additive, 0., 1);
        assert_eq!(spec.initialize.speed, [1.5, 2.5]);
        assert_eq!(spec.initialize.spread_degrees, 45.);
        assert!(spec.update.iter().any(|m| matches!(m, crysirror_vfx::Module::Drag { .. })));
        assert!(spec.update.iter().any(|m| matches!(m, crysirror_vfx::Module::Acceleration { value } if *value == [0., -0.5, 0.].into())));
        crysirror_vfx::SystemSpec { emitters: vec![spec], ..Default::default() }.compile().unwrap();
    }

    #[test]
    fn program_random_repeats_per_particle() {
        // r0 = 1, r1 = 3, r2 = rand(r0, r1), attr[0] = r2.
        let e = Effect {
            name: String::new(),
            flags: 0,
            orientation: 0,
            material: 0,
            slots: HashMap::from([(sid("scaleX"), 0)]),
            constants: vec![1., 3.],
            curves: vec![],
            ramps: vec![],
            spawn: vec![[9, 0, 0, 0], [9, 1, 1, 0], [15, 2, 0, 0x10], [2, 0, 2, 0]],
            update: vec![],
            emission: 0.,
            shape: Shape::Point { offset: Vec3::ZERO, life: Life::Infinite },
            launch: Launch::default(),
            fields: vec![],
        };
        let a = e.birth(1234);
        let b = e.birth(1234);
        assert_eq!(a.attrs, b.attrs);
        assert!((1.0..3.0).contains(&a.attrs[0]));
    }
}
