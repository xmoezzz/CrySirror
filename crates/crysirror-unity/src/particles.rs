// SPDX-License-Identifier: MPL-2.0
//! Unity `ParticleSystem` (class 198) and `ParticleSystemRenderer` (199)
//! as engine VFX systems ([`crysirror_vfx::SystemSpec`]).
//!
//! Unity emits along the shape's local +Z; engine spawn shapes and the
//! emission axis use local +Y, so [`ParticleImport::rotation`] carries
//! the Y→Z turn. Modules without an engine equivalent are reported in
//! [`ParticleImport::notes`] instead of being dropped silently.
use crate::{
    PPtr,
    managed::{Fields, Value},
    scene6::Scene6,
    world6::{UnityMaterial, world_matrix},
};
use anyhow::{Result, bail};
use crysirror_vfx::{BlendMode, Burst, ColorCurve, ColorKey, EmitterSpec, FacingMode, InitSpec, Interpolation, Module, RenderSpec, SimulationSpace, SpawnShape, SpawnSpec, SystemSpec, Vec2Curve, Vec2Key};
use glam::{Mat4, Quat, Vec3};

pub const PARTICLE_SYSTEM: i32 = 198;
pub const PARTICLE_SYSTEM_RENDERER: i32 = 199;

/// Unity `Physics.gravity` magnitude (project default).
const GRAVITY: f32 = 9.81;
/// Samples of baked lifetime curves (engine curves take 2..32 keys).
const CURVE_SAMPLES: usize = 9;

/// A particle system of an active scene node.
pub struct SceneParticles<'a> {
    pub node: usize,
    pub system: &'a Fields,
    pub renderer: Option<&'a Fields>,
}

/// The particle systems of nodes active in the hierarchy.
pub fn scene_particles(scene: &Scene6) -> Vec<SceneParticles<'_>> {
    let mut out = Vec::new();
    for (i, n) in scene.nodes.iter().enumerate() {
        let mut active = n.active;
        let mut p = n.parent;
        while let Some(q) = p {
            active &= scene.nodes[q].active;
            p = scene.nodes[q].parent;
        }
        if !active {
            continue;
        }
        let renderer = n.components.iter().find(|c| c.class_id == PARTICLE_SYSTEM_RENDERER).map(|c| &c.fields);
        for c in n.components.iter().filter(|c| c.class_id == PARTICLE_SYSTEM) {
            out.push(SceneParticles { node: i, system: &c.fields, renderer });
        }
    }
    out
}

/// The first material of a particle renderer, if it draws.
pub fn renderer_material(renderer: &Fields) -> Option<PPtr> {
    if renderer.bool("m_Enabled") == Some(false) {
        return None;
    }
    match renderer.array("m_Materials")?.first()? {
        Value::PPtr(p) if !p.is_null() => Some(*p),
        _ => None,
    }
}

/// A converted particle system.
#[derive(Clone, Debug)]
pub struct ParticleImport {
    pub system: SystemSpec,
    /// Unity world position of the emitter (shape offset applied).
    pub position: [f32; 3],
    /// Unity world rotation of the engine emitter frame (local +Y is the
    /// Unity shape's +Z).
    pub rotation: [f32; 4],
    /// `_MainTex` of the material; the caller fills the render texture.
    pub texture: Option<PPtr>,
    /// Behaviour approximated or not converted.
    pub notes: Vec<String>,
}

fn f(v: &Fields, k: &str) -> f32 {
    v.float(k).unwrap_or(0.) as f32
}

fn vec3(v: Option<&Fields>, default: f32) -> Vec3 {
    v.map_or(Vec3::splat(default), |v| Vec3::new(v.float("x").unwrap_or(default as f64) as f32, v.float("y").unwrap_or(default as f64) as f32, v.float("z").unwrap_or(default as f64) as f32))
}

fn color(v: Option<&Fields>) -> [f32; 4] {
    v.map_or([1.; 4], |c| ["r", "g", "b", "a"].map(|k| c.float(k).unwrap_or(1.) as f32))
}

/// Evaluate a type-tree `AnimationCurve` (keyframes `time`, `value`,
/// `inSlope`, `outSlope`; cubic Hermite, infinite slopes step).
pub fn curve(c: &Fields, t: f32) -> Option<f32> {
    let keys: Vec<[f32; 4]> = c
        .array("m_Curve")?
        .iter()
        .filter_map(|k| match k {
            Value::Struct(k) => Some([f(k, "time"), f(k, "value"), f(k, "inSlope"), f(k, "outSlope")]),
            _ => None,
        })
        .collect();
    let first = keys.first()?;
    if t <= first[0] || keys.len() == 1 {
        return Some(first[1]);
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b[0] {
            let dt = b[0] - a[0];
            if dt <= 0. || !a[3].is_finite() || !b[2].is_finite() {
                return Some(if t < b[0] { a[1] } else { b[1] });
            }
            let s = (t - a[0]) / dt;
            let (s2, s3) = (s * s, s * s * s);
            return Some((2. * s3 - 3. * s2 + 1.) * a[1] + (s3 - 2. * s2 + s) * dt * a[3] + (-2. * s3 + 3. * s2) * b[1] + (s3 - s2) * dt * b[2]);
        }
    }
    keys.last().map(|k| k[1])
}

/// A `MinMaxCurve`: `minMaxState` 0 constant, 1 curve, 2 between two
/// curves, 3 between two constants.
#[derive(Clone, Copy)]
struct MinMax<'a>(&'a Fields);

impl MinMax<'_> {
    fn state(&self) -> i64 {
        self.0.int("minMaxState").unwrap_or(0)
    }
    /// The value range at normalized time `t`.
    fn range(&self, t: f32) -> [f32; 2] {
        let s = f(self.0, "scalar");
        match self.state() {
            1 => {
                let v = self.0.fields("maxCurve").and_then(|c| curve(c, t)).unwrap_or(1.) * s;
                [v, v]
            }
            2 => {
                let a = self.0.fields("minCurve").and_then(|c| curve(c, t)).unwrap_or(1.) * s;
                let b = self.0.fields("maxCurve").and_then(|c| curve(c, t)).unwrap_or(1.) * s;
                [a.min(b), a.max(b)]
            }
            3 => {
                let a = f(self.0, "minScalar");
                [a.min(s), a.max(s)]
            }
            _ => [s, s],
        }
    }
    fn mean(&self, t: f32) -> f32 {
        let r = self.range(t);
        (r[0] + r[1]) * 0.5
    }
}

fn minmax<'a>(m: &'a Fields, k: &str) -> Option<MinMax<'a>> {
    m.fields(k).map(MinMax)
}

/// A serialized `Gradient` with alpha keys, at `t`.
fn gradient(g: &Fields, t: f32) -> [f32; 4] {
    let rgb = crate::world6::gradient(g, t).unwrap_or([1.; 3]);
    let n = g.int("m_NumAlphaKeys").unwrap_or(0).clamp(0, 8) as usize;
    let keys: Vec<(f32, f32)> = (0..n).map(|i| (g.int(&format!("atime{i}")).unwrap_or(0) as f32 / 65535., g.fields(&format!("key{i}")).map_or(1., |c| f(c, "a")))).collect();
    let a = match keys.as_slice() {
        [] => 1.,
        [k, ..] if t <= k.0 => k.1,
        _ => keys
            .windows(2)
            .find(|w| t <= w[1].0)
            .map(|w| {
                if g.int("m_Mode").unwrap_or(0) == 1 {
                    return w[1].1;
                }
                let d = w[1].0 - w[0].0;
                w[0].1 + (w[1].1 - w[0].1) * if d > 0. { (t - w[0].0) / d } else { 1. }
            })
            .unwrap_or(keys.last().unwrap().1),
    };
    [rgb[0], rgb[1], rgb[2], a]
}

/// A `MinMaxGradient` (state 0 colour, 1 gradient, 2 two colours, 3 two
/// gradients, 4 random colour) as its mean at `t`.
fn minmax_gradient(g: &Fields, t: f32) -> [f32; 4] {
    let max = || g.fields("maxGradient").map_or([1.; 4], |x| gradient(x, t));
    let min = || g.fields("minGradient").map_or([1.; 4], |x| gradient(x, t));
    let mix = |a: [f32; 4], b: [f32; 4]| [0, 1, 2, 3].map(|i| (a[i] + b[i]) * 0.5);
    match g.int("minMaxState").unwrap_or(0) {
        1 | 4 => max(),
        2 => mix(color(g.fields("minColor")), color(g.fields("maxColor"))),
        3 => mix(min(), max()),
        _ => color(g.fields("maxColor")),
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    let c = c.clamp(0., 1.);
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// How a particle shader blends and tints. `None`: not convertible.
fn shader_blend(m: &UnityMaterial) -> Option<(BlendMode, [f32; 4], bool)> {
    let s = m.shader.as_str();
    // Legacy particle shaders: `2 · vertex colour · _TintColor · _MainTex`.
    let tint = |k: &str| {
        let c = m.color(k).unwrap_or([0.5, 0.5, 0.5, 0.5]);
        [srgb_to_linear(c[0]) * 2., srgb_to_linear(c[1]) * 2., srgb_to_linear(c[2]) * 2., (c[3] * 2.).min(1.)]
    };
    if !(s.starts_with("Legacy Shaders/Particles/") || s.starts_with("Particles/") || s.contains("[Sky]") && s.contains("Particles")) {
        return None;
    }
    let approximated = s.ends_with("(Soft)");
    let blend = if s.contains("Additive") {
        BlendMode::Additive
    } else if s.contains("Premultiply") {
        BlendMode::Premultiplied
    } else if s.contains("Multiply") {
        BlendMode::Multiply
    } else if s.contains("Alpha Blended") || s.contains("Blended") {
        BlendMode::Alpha
    } else {
        return None;
    };
    let tint = if m.color("_TintColor").is_some() { tint("_TintColor") } else { [1.; 4] };
    Some((blend, tint, approximated))
}

/// Convert one particle system. `material` is the renderer's first
/// material; `seed` replaces Unity's random seed.
pub fn particle_system(scene: &Scene6, p: &SceneParticles<'_>, material: Option<&UnityMaterial>, seed: u32) -> Result<ParticleImport> {
    let ps = p.system;
    let mut notes = Vec::new();
    let module = |k: &str| ps.fields(k).filter(|m| m.bool("enabled").unwrap_or(m.int("enabled").unwrap_or(0) != 0));
    let Some(init) = ps.fields("InitialModule") else { bail!("ParticleSystem without InitialModule") };
    let Some(material) = material else { bail!("particle renderer without a material") };
    let Some((blend, tint, soft)) = shader_blend(material) else { bail!("particle shader {} is not converted", material.shader) };
    if soft {
        notes.push(format!("{}: soft additive (One OneMinusSrcColor) drawn additive", material.shader));
    }

    // Transform: node world × shape offset, then the Y→Z emission turn.
    let world = world_matrix(scene, p.node);
    let (scale, rot, pos) = world.to_scale_rotation_translation();
    let shape = module("ShapeModule");
    let (shape_pos, shape_rot, shape_scale) = shape.map_or((Vec3::ZERO, Quat::IDENTITY, Vec3::ONE), |s| {
        let r = vec3(s.fields("m_Rotation"), 0.);
        (vec3(s.fields("m_Position"), 0.), Quat::from_euler(glam::EulerRot::ZXY, r.z.to_radians(), r.x.to_radians(), r.y.to_radians()), vec3(s.fields("m_Scale"), 1.))
    });
    let scaling = ps.int("scalingMode").unwrap_or(1);
    // Hierarchy (0) and Local (1) scale sizes and shapes; Shape (2) shapes only.
    let size_scale = if scaling == 2 { 1. } else { scale.max_element() };
    let emitter = Mat4::from_scale_rotation_translation(Vec3::ONE, rot, pos) * Mat4::from_translation(shape_pos * scale);
    let frame = rot * shape_rot * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);

    // Shape (in the engine frame: engine y = Unity z).
    let mut spread = 0.;
    let shape_spec = match shape.map(|s| (s.int("type").unwrap_or(4), s)) {
        None => SpawnShape::Point,
        Some((t, s)) => {
            let radius = s.fields("radius").map_or(1., |r| f(r, "value"));
            let thickness = s.float("radiusThickness").unwrap_or(1.) as f32;
            let k = scale * shape_scale;
            match t {
                0..=3 => {
                    spread = if t <= 1 { 180. } else { 90. };
                    notes.push("sphere shapes emit within a cone instead of radially".into());
                    SpawnShape::Sphere { radius: radius * k.max_element(), surface: thickness == 0. || t == 1 || t == 3 }
                }
                4 | 7 | 8 | 9 => {
                    spread = f(s, "angle").clamp(0., 90.);
                    if t >= 8 {
                        notes.push("cone volume emission from the base disc".into());
                    }
                    SpawnShape::Cone { radius: radius * k.x.max(k.y), height: 0. }
                }
                5 | 15 | 16 => {
                    let h = k * 0.5;
                    SpawnShape::Box { half_extent: [h.x.abs(), h.z.abs(), h.y.abs()] }
                }
                10 | 11 => {
                    spread = 90.;
                    notes.push("circle shapes emit within a cone instead of radially".into());
                    SpawnShape::Cone { radius: radius * k.x.max(k.y), height: 0. }
                }
                12 => SpawnShape::Line { end: [2. * radius * k.x, 0., 0.] },
                18 => SpawnShape::Box { half_extent: [k.x * 0.5, 0., k.y * 0.5] },
                other => {
                    notes.push(format!("shape type {other} emits from a point"));
                    SpawnShape::Point
                }
            }
        }
    };

    // Start values.
    let range = |k: &str| minmax(init, k).map_or([1., 1.], |m| m.range(0.));
    let lifetime = range("startLifetime");
    let speed = range("startSpeed");
    let start_size = range("startSize").map(|v| (v * size_scale).max(0.));
    let rotation = range("startRotation");
    let start_color = init.fields("startColor").map_or([1.; 4], |g| minmax_gradient(g, 0.));
    for k in ["startLifetime", "startSpeed", "startSize"] {
        if minmax(init, k).is_some_and(|c| matches!(c.state(), 1 | 2)) {
            notes.push(format!("{k} over emitter time uses its start value"));
        }
    }
    if init.int("size3D").unwrap_or(0) != 0 || init.int("rotation3D").unwrap_or(0) != 0 {
        notes.push("3D start size/rotation use the X axis".into());
    }
    let tinted = |c: [f32; 4]| [0, 1, 2, 3].map(|i| (c[i] * start_color[i] * tint[i]).max(0.)).map(|v| v.min(64.));
    let clamp_a = |mut c: [f32; 4]| {
        c[3] = c[3].clamp(0., 1.);
        c
    };

    // Lifetime modules.
    let mut update = Vec::new();
    let gravity = minmax(init, "gravityModifier").map_or(0., |m| m.mean(0.));
    if gravity != 0. {
        update.push(Module::Acceleration { value: [0., -GRAVITY * gravity, 0.].into() });
    }
    if let Some(force) = module("ForceModule") {
        let v = Vec3::new(minmax(force, "x").map_or(0., |m| m.mean(0.)), minmax(force, "y").map_or(0., |m| m.mean(0.)), minmax(force, "z").map_or(0., |m| m.mean(0.)));
        let local = force.int("inWorldSpace").unwrap_or(0) == 0;
        let w = if local { rot * v } else { v };
        // The engine's Unity-space mirror (x) of world vectors.
        update.push(Module::Acceleration { value: [-w.x, w.y, w.z].into() });
        notes.push("force over lifetime uses its start value".into());
    }
    if let Some(clamp) = module("ClampVelocityModule") {
        let dampen = f(clamp, "dampen");
        if dampen > 0. {
            update.push(Module::Drag { coefficient: (dampen * 10.).into() });
        }
        notes.push("limit velocity drawn as drag".into());
    }
    if let Some(noise) = module("NoiseModule") {
        let strength = minmax(noise, "strength").map_or(1., |m| m.mean(0.));
        update.push(Module::CurlNoise { strength: strength.into(), frequency: f(noise, "frequency").into() });
    }
    if let Some(r) = module("RotationModule") {
        let v = minmax(r, "curve").map_or(0., |m| m.mean(0.));
        if v != 0. {
            update.push(Module::Rotate { radians_per_second: v.into() });
        }
    }
    let samples = |g: &dyn Fn(f32) -> [f32; 4]| -> ColorCurve {
        ColorCurve { keys: (0..CURVE_SAMPLES).map(|i| i as f32 / (CURVE_SAMPLES - 1) as f32).map(|t| ColorKey { time: t, value: clamp_a(tinted(g(t))) }).collect(), interpolation: Interpolation::Linear }
    };
    if let Some(c) = module("ColorModule").and_then(|m| m.fields("gradient")) {
        update.push(Module::ColorOverLife { curve: samples(&|t| minmax_gradient(c, t)) });
    }
    let size_mean = (start_size[0] + start_size[1]) * 0.5;
    if let Some(s) = module("SizeModule").and_then(|m| minmax(m, "curve")) {
        if s.state() != 0 || f(s.0, "scalar") != 1. {
            let keys = (0..CURVE_SAMPLES)
                .map(|i| i as f32 / (CURVE_SAMPLES - 1) as f32)
                .map(|t| {
                    let v = (s.mean(t) * size_mean).max(0.);
                    Vec2Key { time: t, value: [v, v] }
                })
                .collect();
            update.push(Module::SizeOverLife { curve: Vec2Curve { keys, interpolation: Interpolation::Linear } });
            if start_size[0] != start_size[1] {
                notes.push("size over lifetime uses the mean start size".into());
            }
        }
    }
    for (k, what) in [("VelocityModule", "velocity over lifetime"), ("InheritVelocityModule", "inherit velocity"), ("CollisionModule", "collision"), ("SubModule", "sub emitters"), ("TrailModule", "trails"), ("LightsModule", "particle lights"), ("SizeBySpeedModule", "size by speed"), ("ColorBySpeedModule", "colour by speed"), ("RotationBySpeedModule", "rotation by speed"), ("ExternalForcesModule", "external forces"), ("LifetimeByEmitterSpeedModule", "lifetime by emitter speed")] {
        if module(k).is_some() {
            notes.push(format!("{what} not converted"));
        }
    }

    // Emission.
    let emission = module("EmissionModule");
    let rate = emission.and_then(|e| minmax(e, "rateOverTime")).map_or(0., |m| m.mean(0.)).max(0.);
    let bursts: Vec<Burst> = emission
        .and_then(|e| e.array("m_Bursts"))
        .unwrap_or(&[])
        .iter()
        .filter_map(|b| match b {
            Value::Struct(b) => Some(Burst { time: f(b, "time").max(0.), count: minmax(b, "countCurve").map_or(f(b, "minCount"), |m| m.mean(0.)).round().clamp(0., 65_536.) as u32 }),
            _ => None,
        })
        .collect();
    if emission.and_then(|e| minmax(e, "rateOverDistance")).is_some_and(|m| m.mean(0.) > 0.) {
        notes.push("rate over distance not converted".into());
    }
    if minmax(ps, "startDelay").is_some_and(|m| m.mean(0.) > 0.) {
        notes.push("start delay not converted".into());
    }
    let duration = f(ps, "lengthInSec").max(0.05);
    let looping = ps.bool("looping").unwrap_or(true);

    // Renderer.
    let r = p.renderer;
    let facing = match r.and_then(|r| r.int("m_RenderMode")).unwrap_or(0) {
        1 => FacingMode::Velocity,
        2 => FacingMode::Axis { axis: [0., 1., 0.] },
        0 => FacingMode::Camera,
        other => {
            notes.push(format!("render mode {other} drawn as camera billboards"));
            FacingMode::Camera
        }
    };
    let (mut columns, mut rows, mut frames, mut fps) = (1, 1, 1, 0.);
    if let Some(uv) = module("UVModule") {
        columns = uv.int("tilesX").unwrap_or(1).clamp(1, 64) as u32;
        rows = uv.int("tilesY").unwrap_or(1).clamp(1, 64) as u32;
        frames = if uv.int("animationType").unwrap_or(0) == 1 { columns } else { columns * rows };
        let cycles = f(uv, "cycles").max(1.);
        fps = frames as f32 * cycles / ((lifetime[0] + lifetime[1]) * 0.5).max(0.01);
        notes.push("texture sheet animation at a fixed rate over the mean lifetime".into());
    }
    let render = RenderSpec::Sprite { texture: None, columns, rows, frames, fps, soft_distance: if soft { 0.5 } else { 0. }, triplanar_coverage: None, blend, facing };

    let capacity = init.int("maxNumParticles").unwrap_or(1000).clamp(1, 16_384) as u32;
    let max_life = lifetime[1].max(0.01);
    let needed = ((rate * max_life).ceil() as u32 + bursts.iter().map(|b| b.count).sum::<u32>()).max(1);
    let simulation_speed = f(ps, "simulationSpeed").clamp(0., 16.);
    let emitter_spec = EmitterSpec {
        id: "unity".into(),
        space: if ps.int("moveWithTransform").unwrap_or(0) == 1 { SimulationSpace::World } else { SimulationSpace::Local },
        capacity: capacity.min(needed.saturating_mul(2)).max(1),
        seed: seed.max(1),
        spawn: SpawnSpec { rate: rate.into(), bursts, duration, looping, burst_looping: looping, auto_start: ps.bool("playOnAwake").unwrap_or(true) },
        initialize: InitSpec {
            shape: shape_spec,
            lifetime: [lifetime[0].max(0.01), max_life],
            speed: [speed[0], speed[1].max(speed[0])],
            direction: [0., 1., 0.].into(),
            spread_degrees: spread,
            color: clamp_a(tinted([1.; 4])),
            size: [size_mean, size_mean],
            rotation: [rotation[0], rotation[1].max(rotation[0])],
            ..Default::default()
        },
        update,
        render,
        ..Default::default()
    };
    if start_size[0] != start_size[1] {
        notes.push("start size range drawn at its mean".into());
    }
    let system = SystemSpec { emitters: vec![emitter_spec], warmup_seconds: if ps.bool("prewarm").unwrap_or(false) { duration.min(10.) } else { 0. }, time_scale: simulation_speed, ..Default::default() };
    Ok(ParticleImport { system, position: emitter.w_axis.truncate().to_array(), rotation: frame.to_array(), texture: material.texture("_MainTex").map(|t| t.texture), notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, slope: f32) -> Value {
        Value::Struct(Fields(vec![("time".into(), Value::Float(time.into())), ("value".into(), Value::Float(value.into())), ("inSlope".into(), Value::Float(slope.into())), ("outSlope".into(), Value::Float(slope.into()))]))
    }

    #[test]
    fn hermite_curve_and_steps() {
        let c = Fields(vec![("m_Curve".into(), Value::Array(vec![key(0., 0., 1.), key(1., 1., 1.)]))]);
        assert!((curve(&c, 0.5).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(curve(&c, -1.), Some(0.));
        assert_eq!(curve(&c, 2.), Some(1.));
        let step = Fields(vec![("m_Curve".into(), Value::Array(vec![key(0., 2., f32::INFINITY), key(1., 3., f32::INFINITY)]))]);
        assert_eq!(curve(&step, 0.5), Some(2.));
    }

    #[test]
    fn minmax_states() {
        let m = Fields(vec![("minMaxState".into(), Value::Int(3)), ("scalar".into(), Value::Float(4.)), ("minScalar".into(), Value::Float(2.))]);
        assert_eq!(MinMax(&m).range(0.), [2., 4.]);
        assert_eq!(MinMax(&m).mean(0.), 3.);
    }
}
