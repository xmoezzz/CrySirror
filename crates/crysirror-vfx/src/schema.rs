// SPDX-License-Identifier: MPL-2.0
use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ParameterMap = BTreeMap<String, ParameterValue>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
/// Typed authored parameter value. Updates must preserve the declared variant.
pub enum ParameterValue {
    Float(f32),
    Vector([f32; 3]),
    Color([f32; 4]),
    Bool(bool),
}
impl ParameterValue {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Float(v) => finite(&[*v])?,
            Self::Vector(v) => finite(v)?,
            Self::Color(v) => {
                finite(v)?;
                ensure!(
                    v[..3].iter().all(|x| *x >= 0.) && (0.0..=1.).contains(&v[3]),
                    "invalid VFX color parameter"
                );
            }
            Self::Bool(_) => {}
        }
        Ok(())
    }
    pub fn same_type(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
/// Finite scalar constant or checked float-parameter binding.
pub enum ScalarValue {
    Constant(f32),
    Parameter(ScalarBinding),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Affine float binding: parameter * scale + offset.
pub struct ScalarBinding {
    pub parameter: String,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub offset: f32,
}
impl From<f32> for ScalarValue {
    fn from(v: f32) -> Self {
        Self::Constant(v)
    }
}
impl ScalarValue {
    pub fn resolve(&self, p: &ParameterMap) -> Result<f32> {
        let value = match self {
            Self::Constant(v) => *v,
            Self::Parameter(b) => {
                let Some(ParameterValue::Float(v)) = p.get(&b.parameter) else {
                    bail!(
                        "VFX float parameter '{}' missing or wrong type",
                        b.parameter
                    )
                };
                finite(&[b.scale, b.offset])?;
                *v * b.scale + b.offset
            }
        };
        finite(&[value])?;
        Ok(value)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
/// Finite simulation-space vector or checked vector-parameter binding.
pub enum VectorValue {
    Constant([f32; 3]),
    Parameter(VectorBinding),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Vector binding with an explicit scalar multiplier.
pub struct VectorBinding {
    pub parameter: String,
    #[serde(default = "one")]
    pub scale: f32,
}
impl From<[f32; 3]> for VectorValue {
    fn from(v: [f32; 3]) -> Self {
        Self::Constant(v)
    }
}
impl VectorValue {
    pub fn resolve(&self, p: &ParameterMap) -> Result<[f32; 3]> {
        let value = match self {
            Self::Constant(v) => *v,
            Self::Parameter(b) => {
                finite(&[b.scale])?;
                let Some(ParameterValue::Vector(v)) = p.get(&b.parameter) else {
                    bail!(
                        "VFX vector parameter '{}' missing or wrong type",
                        b.parameter
                    )
                };
                v.map(|x| x * b.scale)
            }
        };
        finite(&value)?;
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Bounded curve segment interpolation; Smooth is cubic smoothstep.
pub enum Interpolation {
    Constant,
    #[default]
    Linear,
    Smooth,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Scalar value at a normalized lifetime coordinate.
pub struct ScalarKey {
    pub time: f32,
    pub value: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Full-width/full-height value at a normalized lifetime coordinate.
pub struct Vec2Key {
    pub time: f32,
    pub value: [f32; 2],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Linear HDR RGB and opacity at a normalized lifetime coordinate.
pub struct ColorKey {
    pub time: f32,
    pub value: [f32; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Checked scalar lifetime curve with 2..32 increasing keys from 0 to 1.
pub struct ScalarCurve {
    pub keys: Vec<ScalarKey>,
    #[serde(default)]
    pub interpolation: Interpolation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Checked full-size lifetime curve with 2..32 normalized keys.
pub struct Vec2Curve {
    pub keys: Vec<Vec2Key>,
    #[serde(default)]
    pub interpolation: Interpolation,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Checked linear HDR color/opacity lifetime curve.
pub struct ColorCurve {
    pub keys: Vec<ColorKey>,
    #[serde(default)]
    pub interpolation: Interpolation,
}
fn curve_segment(times: impl Iterator<Item = f32>, t: f32) -> (usize, f32) {
    let t = t.clamp(0., 1.);
    let mut values = times.enumerate();
    let (_, mut previous) = values.next().expect("sample requires a validated curve");
    let mut last = 0;
    for (index, next) in values {
        last = index;
        if t <= next {
            return (
                index - 1,
                ((t - previous) / (next - previous)).clamp(0., 1.),
            );
        }
        previous = next;
    }
    (last.saturating_sub(1), 1.)
}
fn weight(t: f32, i: Interpolation) -> f32 {
    match i {
        Interpolation::Constant => {
            if t >= 1. {
                1.
            } else {
                0.
            }
        }
        Interpolation::Linear => t,
        Interpolation::Smooth => t * t * (3. - 2. * t),
    }
}
fn check_times(times: impl Iterator<Item = f32>) -> Result<()> {
    let times = times.collect::<Vec<_>>();
    ensure!(
        (2..=32).contains(&times.len()),
        "VFX curve requires 2..32 keys"
    );
    ensure!(
        times[0] == 0.
            && *times.last().unwrap() == 1.
            && times
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.).contains(v))
            && times.windows(2).all(|w| w[0] < w[1]),
        "VFX normalized curve keys must increase from 0 to 1"
    );
    Ok(())
}
impl ScalarCurve {
    pub fn linear(a: f32, b: f32) -> Self {
        Self {
            keys: vec![
                ScalarKey { time: 0., value: a },
                ScalarKey { time: 1., value: b },
            ],
            interpolation: Interpolation::Linear,
        }
    }
    pub fn validate(&self) -> Result<()> {
        check_times(self.keys.iter().map(|k| k.time))?;
        for k in &self.keys {
            finite(&[k.value])?;
        }
        Ok(())
    }
    pub fn sample(&self, t: f32) -> f32 {
        let (i, f) = curve_segment(self.keys.iter().map(|k| k.time), t);
        let a = self.keys[i].value;
        a + (self.keys[i + 1].value - a) * weight(f, self.interpolation)
    }
}
impl Vec2Curve {
    pub fn linear(a: [f32; 2], b: [f32; 2]) -> Self {
        Self {
            keys: vec![
                Vec2Key { time: 0., value: a },
                Vec2Key { time: 1., value: b },
            ],
            interpolation: Interpolation::Linear,
        }
    }
    pub fn validate(&self) -> Result<()> {
        check_times(self.keys.iter().map(|k| k.time))?;
        for k in &self.keys {
            finite(&k.value)?;
            ensure!(k.value.iter().all(|v| *v >= 0.), "negative particle size");
        }
        Ok(())
    }
    pub fn sample(&self, t: f32) -> [f32; 2] {
        let (i, f) = curve_segment(self.keys.iter().map(|k| k.time), t);
        std::array::from_fn(|c| {
            let a = self.keys[i].value[c];
            a + (self.keys[i + 1].value[c] - a) * weight(f, self.interpolation)
        })
    }
}
impl ColorCurve {
    pub fn linear(a: [f32; 4], b: [f32; 4]) -> Self {
        Self {
            keys: vec![
                ColorKey { time: 0., value: a },
                ColorKey { time: 1., value: b },
            ],
            interpolation: Interpolation::Linear,
        }
    }
    pub fn validate(&self) -> Result<()> {
        check_times(self.keys.iter().map(|k| k.time))?;
        for k in &self.keys {
            finite(&k.value)?;
            ensure!(
                k.value[..3].iter().all(|v| *v >= 0.) && (0.0..=1.).contains(&k.value[3]),
                "invalid VFX curve color"
            );
        }
        Ok(())
    }
    pub fn sample(&self, t: f32) -> [f32; 4] {
        let (i, f) = curve_segment(self.keys.iter().map(|k| k.time), t);
        std::array::from_fn(|c| {
            let a = self.keys[i].value[c];
            a + (self.keys[i + 1].value[c] - a) * weight(f, self.interpolation)
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Explicit execution backend; unsupported GPU features fail at compilation.
pub enum SimulationTarget {
    #[default]
    Cpu,
    Gpu,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// World trajectories stay fixed; Local trajectories follow the current transform.
pub enum SimulationSpace {
    #[default]
    World,
    Local,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Explicit CPU birth admission policy; GPU currently supports DropNew.
pub enum OverflowPolicy {
    #[default]
    DropNew,
    KillOldest,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// Birth positions in emitter-local coordinates before simulation-space conversion.
pub enum SpawnShape {
    Point,
    Sphere { radius: f32, surface: bool },
    Box { half_extent: [f32; 3] },
    Cone { radius: f32, height: f32 },
    Line { end: [f32; 3] },
}
impl Default for SpawnShape {
    fn default() -> Self {
        Self::Point
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One scheduled automatic burst, in seconds within the emitter cycle.
pub struct Burst {
    pub time: f32,
    pub count: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Automatic emission schedule; event-only targets use auto_start=false.
pub struct SpawnSpec {
    pub rate: ScalarValue,
    pub bursts: Vec<Burst>,
    pub duration: f32,
    pub looping: bool,
    /// Repeat burst schedules per cycle, independently of continuous emission.
    pub burst_looping: bool,
    /// Event-only emitters do not create automatic births.
    pub auto_start: bool,
}
impl Default for SpawnSpec {
    fn default() -> Self {
        Self {
            rate: 20.0.into(),
            bursts: vec![],
            duration: 5.,
            looping: true,
            burst_looping: true,
            auto_start: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Birth attributes. Size is full world width/height; rotation is radians.
pub struct InitSpec {
    pub shape: SpawnShape,
    pub lifetime: [f32; 2],
    pub speed: [f32; 2],
    pub direction: VectorValue,
    pub spread_degrees: f32,
    pub color: [f32; 4],
    pub size: [f32; 2],
    pub rotation: [f32; 2],
    pub ribbon_id: u32,
    /// Optional typed user color binding sampled at birth.
    pub color_parameter: Option<String>,
    /// Optional vector parameter whose XY components define full birth width/height.
    pub size_parameter: Option<String>,
}
impl Default for InitSpec {
    fn default() -> Self {
        Self {
            shape: SpawnShape::Point,
            lifetime: [1., 2.],
            speed: [1., 2.],
            direction: [0., 1., 0.].into(),
            spread_degrees: 20.,
            color: [1.; 4],
            size: [0.1; 2],
            rotation: [0.; 2],
            ribbon_id: 0,
            color_parameter: None,
            size_parameter: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
/// Typed instruction stack: ordered forces, integration, then responses and appearance.
pub enum Module {
    Acceleration {
        value: VectorValue,
    },
    Drag {
        coefficient: ScalarValue,
    },
    CurlNoise {
        strength: ScalarValue,
        frequency: ScalarValue,
    },
    Attractor {
        point: VectorValue,
        strength: ScalarValue,
        radius: ScalarValue,
    },
    Orbit {
        axis: VectorValue,
        speed: ScalarValue,
    },
    Rotate {
        radians_per_second: ScalarValue,
    },
    ColorOverLife {
        curve: ColorCurve,
    },
    SizeOverLife {
        curve: Vec2Curve,
    },
    CollisionPlane {
        normal: VectorValue,
        offset: ScalarValue,
        restitution: f32,
        friction: f32,
        kill: bool,
    },
    CollisionSphere {
        center: VectorValue,
        radius: ScalarValue,
        restitution: f32,
        kill: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Renderer blend contract, independent of the execution backend.
pub enum BlendMode {
    Alpha,
    #[default]
    Additive,
    Premultiplied,
    Multiply,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// Sprite orientation: camera-facing, velocity-aligned or fixed local axis.
pub enum FacingMode {
    #[default]
    Camera,
    Velocity,
    Axis {
        axis: [f32; 3],
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// Built-in or client-relative static mesh resource for particle rendering.
pub enum MeshShape {
    #[default]
    Cube,
    Quad,
    Asset {
        path: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
/// Device-independent renderer configuration. Ribbon width is explicit full width.
pub enum RenderSpec {
    Sprite {
        #[serde(default)]
        texture: Option<String>,
        #[serde(default = "one_u")]
        columns: u32,
        #[serde(default = "one_u")]
        rows: u32,
        #[serde(default = "one_u")]
        frames: u32,
        #[serde(default)]
        fps: f32,
        #[serde(default)]
        soft_distance: f32,
        #[serde(default)]
        blend: BlendMode,
        #[serde(default)]
        facing: FacingMode,
    },
    Mesh {
        #[serde(default)]
        mesh: MeshShape,
        #[serde(default)]
        texture: Option<String>,
        #[serde(default)]
        blend: BlendMode,
    },
    Ribbon {
        width: f32,
        #[serde(default)]
        texture: Option<String>,
        #[serde(default = "one")]
        uv_scale: f32,
        #[serde(default = "segments")]
        max_segments: u32,
        #[serde(default)]
        blend: BlendMode,
    },
}
impl Default for RenderSpec {
    fn default() -> Self {
        Self::Sprite {
            texture: None,
            columns: 1,
            rows: 1,
            frames: 1,
            fps: 0.,
            soft_distance: 0.,
            blend: BlendMode::Additive,
            facing: FacingMode::Camera,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// CPU event kinds and GPU debug record categories.
pub enum EventKind {
    Birth,
    Death,
    Collision,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Bounded CPU sub-emitter request at a parent event, with optional velocity inheritance.
pub struct EventHandler {
    pub event: EventKind,
    pub emitter: String,
    pub count: u32,
    #[serde(default)]
    pub inherit_velocity: f32,
    #[serde(default = "one")]
    pub probability: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Named data-driven emitter: schedule, initialization, typed module stack and renderer.
pub struct EmitterSpec {
    pub id: String,
    pub enabled: bool,
    pub enabled_parameter: Option<String>,
    pub target: SimulationTarget,
    pub space: SimulationSpace,
    pub capacity: u32,
    pub seed: u32,
    pub overflow: OverflowPolicy,
    pub spawn: SpawnSpec,
    pub initialize: InitSpec,
    pub update: Vec<Module>,
    pub render: RenderSpec,
    /// Optional CPU pixel-plane mesh publication, independently of a native VM.
    pub screen: Option<crate::screen::ScreenSpriteTarget>,
    /// Optional CPU falling/periodic-drift/nonuniform-atlas node.
    pub falling_atlas: Option<crate::falling_atlas::FallingAtlasSpec>,
    pub events: Vec<EventHandler>,
}
impl Default for EmitterSpec {
    fn default() -> Self {
        Self {
            id: "emitter".into(),
            enabled: true,
            enabled_parameter: None,
            target: SimulationTarget::Cpu,
            space: SimulationSpace::World,
            capacity: 1024,
            seed: 1,
            overflow: OverflowPolicy::DropNew,
            spawn: Default::default(),
            initialize: Default::default(),
            update: vec![],
            render: Default::default(),
            screen: None,
            falling_atlas: None,
            events: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// A scoped multi-emitter effect with typed parameters and a fixed simulation clock.
pub struct SystemSpec {
    /// Explicit external render-tick/wall-clock source, or engine fixed time.
    pub clock: crate::falling_atlas::SimulationClock,
    /// Initial single-stream random state; changing it restarts this revision.
    pub sequence_rng: Option<crate::falling_atlas::SequenceRng>,
    /// Fenced stream replacement without particle-pool reinitialization.
    pub sequence_exchange: Option<crate::falling_atlas::SequenceExchange>,
    /// Explicit restart epoch; parameter/control changes do not increment it.
    pub revision: u64,
    pub emitters: Vec<EmitterSpec>,
    pub parameters: ParameterMap,
    pub fixed_step_hz: u32,
    pub warmup_seconds: f32,
    pub max_steps_per_advance: u32,
    pub enabled: bool,
    pub paused: bool,
    pub time_scale: f32,
}
impl Default for SystemSpec {
    fn default() -> Self {
        Self {
            clock: Default::default(),
            sequence_rng: None,
            sequence_exchange: None,
            revision: 0,
            emitters: vec![EmitterSpec::default()],
            parameters: BTreeMap::new(),
            fixed_step_hz: 60,
            warmup_seconds: 0.,
            max_steps_per_advance: 64,
            enabled: true,
            paused: false,
            time_scale: 1.,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
/// Validated immutable authored system, ready for owner scheduling.
pub struct CompiledSystem {
    pub spec: SystemSpec,
}
#[derive(Clone, Debug, PartialEq)]
/// Checked emitter and parameter environment; specialize folds bindings for CPU ticks.
pub struct CompiledEmitter {
    pub spec: EmitterSpec,
    pub parameters: ParameterMap,
}

pub(crate) fn finite(v: &[f32]) -> Result<()> {
    ensure!(
        v.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.),
        "VFX value must be finite and bounded"
    );
    Ok(())
}
pub fn logical_asset(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && path.len() <= 1024
            && !path.starts_with('/')
            && !path.contains(['\\', '\0', ':'])
            && path
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".."),
        "VFX asset path must be capability relative"
    );
    Ok(())
}
fn name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control),
        "invalid VFX name"
    );
    Ok(())
}
fn one() -> f32 {
    1.
}
fn one_u() -> u32 {
    1
}
fn segments() -> u32 {
    2048
}

impl RenderSpec {
    pub fn validate(&self) -> Result<()> {
        let texture = match self {
            Self::Sprite {
                texture,
                columns,
                rows,
                frames,
                fps,
                soft_distance,
                facing,
                ..
            } => {
                ensure!(
                    (1..=256).contains(columns)
                        && (1..=256).contains(rows)
                        && (1..=columns * rows).contains(frames),
                    "VFX flipbook grid/frame limits"
                );
                ensure!(
                    texture.is_some() || (*columns == 1 && *rows == 1 && *frames == 1),
                    "VFX flipbooks require an explicit texture asset"
                );
                ensure!(
                    fps.is_finite()
                        && (0.0..=4096.).contains(fps)
                        && soft_distance.is_finite()
                        && (0.0..=10000.).contains(soft_distance),
                    "VFX sprite parameters"
                );
                if let FacingMode::Axis { axis } = facing {
                    finite(axis)?;
                    ensure!(
                        glam::Vec3::from_array(*axis).length_squared() > 1e-12,
                        "VFX facing axis is zero"
                    );
                }
                texture
            }
            Self::Mesh { mesh, texture, .. } => {
                if let MeshShape::Asset { path } = mesh {
                    logical_asset(path)?;
                }
                texture
            }
            Self::Ribbon {
                width,
                texture,
                uv_scale,
                max_segments,
                ..
            } => {
                ensure!(
                    width.is_finite()
                        && (0.0001..=10000.).contains(width)
                        && uv_scale.is_finite()
                        && (0.0001..=10000.).contains(uv_scale)
                        && (1..=65536).contains(max_segments),
                    "VFX ribbon parameters"
                );
                texture
            }
        };
        if let Some(path) = texture {
            logical_asset(path)?;
        }
        Ok(())
    }
}
impl EmitterSpec {
    pub fn validate(&self, parameters: &ParameterMap) -> Result<()> {
        if self.target == SimulationTarget::Gpu {
            validate_gpu_profile(self)?;
        }
        name(&self.id)?;
        ensure!(
            (1..=200_000).contains(&self.capacity),
            "VFX emitter capacity"
        );
        if let Some(name) = &self.enabled_parameter {
            ensure!(
                matches!(parameters.get(name), Some(ParameterValue::Bool(_))),
                "VFX enabled parameter must be bool"
            );
        }
        ensure!(
            (0.0..=1_000_000.).contains(&self.spawn.rate.resolve(parameters)?),
            "VFX spawn rate"
        );
        ensure!(
            self.spawn.duration.is_finite()
                && (0.0001..=86400.).contains(&self.spawn.duration)
                && self.spawn.bursts.len() <= 256,
            "VFX duration/bursts"
        );
        for burst in &self.spawn.bursts {
            ensure!(
                burst.time.is_finite()
                    && (0.0..self.spawn.duration).contains(&burst.time)
                    && burst.count <= 200_000,
                "VFX burst bounds"
            );
        }
        let s = &self.initialize;
        finite(&s.lifetime)?;
        finite(&s.speed)?;
        finite(&s.color)?;
        finite(&s.size)?;
        finite(&s.rotation)?;
        if let Some(name) = &s.color_parameter {
            ensure!(
                matches!(parameters.get(name), Some(ParameterValue::Color(_))),
                "VFX birth color parameter must be color"
            );
        }
        if let Some(name) = &s.size_parameter {
            ensure!(
                matches!(parameters.get(name), Some(ParameterValue::Vector(_))),
                "VFX birth size parameter must be vector"
            );
            if let Some(ParameterValue::Vector(v)) = parameters.get(name) {
                ensure!(
                    v[0] >= 0. && v[1] >= 0.,
                    "VFX birth size parameter is negative"
                );
            }
        }
        ensure!(
            s.lifetime[0] > 0.
                && s.lifetime[1] >= s.lifetime[0]
                && s.lifetime[1] <= 3600.
                && s.speed[0] >= 0.
                && s.speed[1] >= s.speed[0]
                && s.size.iter().all(|v| *v >= 0.)
                && s.rotation[1] >= s.rotation[0]
                && s.spread_degrees.is_finite()
                && (0.0..=180.).contains(&s.spread_degrees)
                && s.color[..3].iter().all(|x| *x >= 0.)
                && (0.0..=1.).contains(&s.color[3]),
            "VFX initialization ranges"
        );
        let direction = s.direction.resolve(parameters)?;
        ensure!(
            glam::Vec3::from_array(direction).length_squared() > 1e-12,
            "VFX emission direction is zero"
        );
        match &s.shape {
            SpawnShape::Point => {}
            SpawnShape::Sphere { radius, .. } => {
                ensure!(
                    radius.is_finite() && (0.0..=10000.).contains(radius),
                    "VFX sphere radius"
                )
            }
            SpawnShape::Box { half_extent } => {
                finite(half_extent)?;
                ensure!(half_extent.iter().all(|v| *v >= 0.), "VFX box extent")
            }
            SpawnShape::Cone { radius, height } => {
                finite(&[*radius, *height])?;
                ensure!(*radius >= 0. && *height >= 0., "VFX cone extent")
            }
            SpawnShape::Line { end } => finite(end)?,
        }
        ensure!(
            self.update.len() <= 32 && self.events.len() <= 16,
            "VFX module/event limit"
        );
        let mut collided = false;
        for module in &self.update {
            let force = matches!(
                module,
                Module::Acceleration { .. }
                    | Module::Drag { .. }
                    | Module::CurlNoise { .. }
                    | Module::Attractor { .. }
                    | Module::Orbit { .. }
            );
            ensure!(
                !collided || !force,
                "VFX force modules must precede collision responses"
            );
            collided |= matches!(
                module,
                Module::CollisionPlane { .. } | Module::CollisionSphere { .. }
            );
            match module {
                Module::Acceleration { value } => {
                    value.resolve(parameters)?;
                }
                Module::Drag { coefficient } => {
                    ensure!(coefficient.resolve(parameters)? >= 0., "VFX negative drag");
                }
                Module::CurlNoise {
                    strength,
                    frequency,
                } => {
                    strength.resolve(parameters)?;
                    ensure!(
                        frequency.resolve(parameters)? >= 0.,
                        "VFX negative frequency"
                    );
                }
                Module::Attractor {
                    point,
                    strength,
                    radius,
                } => {
                    point.resolve(parameters)?;
                    strength.resolve(parameters)?;
                    ensure!(radius.resolve(parameters)? > 0., "VFX attractor radius");
                }
                Module::Orbit { axis, speed } => {
                    ensure!(
                        glam::Vec3::from_array(axis.resolve(parameters)?).length_squared() > 1e-12,
                        "VFX orbit axis"
                    );
                    speed.resolve(parameters)?;
                }
                Module::Rotate { radians_per_second } => {
                    radians_per_second.resolve(parameters)?;
                }
                Module::ColorOverLife { curve } => curve.validate()?,
                Module::SizeOverLife { curve } => curve.validate()?,
                Module::CollisionPlane {
                    normal,
                    offset,
                    restitution,
                    friction,
                    ..
                } => {
                    ensure!(
                        glam::Vec3::from_array(normal.resolve(parameters)?).length_squared()
                            > 1e-12,
                        "VFX collision normal"
                    );
                    offset.resolve(parameters)?;
                    ensure!(
                        restitution.is_finite()
                            && (0.0..=1.).contains(restitution)
                            && friction.is_finite()
                            && (0.0..=1.).contains(friction),
                        "VFX plane response"
                    );
                }
                Module::CollisionSphere {
                    center,
                    radius,
                    restitution,
                    ..
                } => {
                    center.resolve(parameters)?;
                    ensure!(
                        radius.resolve(parameters)? > 0.
                            && restitution.is_finite()
                            && (0.0..=1.).contains(restitution),
                        "VFX sphere response"
                    );
                }
            }
        }
        for h in &self.events {
            name(&h.emitter)?;
            ensure!(
                h.count <= 4096
                    && h.inherit_velocity.is_finite()
                    && (0.0..=16.).contains(&h.inherit_velocity)
                    && h.probability.is_finite()
                    && (0.0..=1.).contains(&h.probability),
                "VFX event handler"
            );
        }
        self.render.validate()?;
        if let Some(node) = &self.falling_atlas {
            node.validate()?;
            ensure!(self.target==SimulationTarget::Cpu&&self.screen.is_some()&&self.space==SimulationSpace::Local&&self.update.is_empty()&&self.events.is_empty(),"VFX falling node requires CPU local screen sprites without conflicting updates/events");
        }
        if let Some(screen) = &self.screen {
            screen.validate()?;
            ensure!(
                self.target == SimulationTarget::Cpu && self.capacity <= 4096,
                "VFX screen profile requires bounded CPU particles"
            );
            ensure!(
                matches!(
                    &self.render,
                    RenderSpec::Sprite {
                        texture: Some(_),
                        facing: FacingMode::Camera,
                        blend: BlendMode::Alpha | BlendMode::Additive | BlendMode::Multiply,
                        ..
                    }
                ),
                "VFX screen render profile unsupported"
            );
        }
        Ok(())
    }
    pub fn compile(&self, parameters: &ParameterMap) -> Result<CompiledEmitter> {
        self.validate(parameters)?;
        Ok(CompiledEmitter {
            spec: self.clone(),
            parameters: parameters.clone(),
        })
    }
}
/// Features currently supported by the resident GPU executor. Both public
/// compilation and backend packing use this single capability boundary.
pub fn validate_gpu_profile(spec: &EmitterSpec) -> Result<()> {
    ensure!(
        spec.overflow == OverflowPolicy::DropNew,
        "GPU VFX does not implement kill_oldest allocation"
    );
    ensure!(spec.events.is_empty(),"GPU VFX child-emitter handlers require a GPU event-consumer mapping; choose CPU explicitly");
    ensure!(
        !matches!(spec.render, RenderSpec::Ribbon { .. }),
        "GPU ribbons require resident trail history; choose CPU explicitly"
    );
    Ok(())
}
impl SystemSpec {
    pub fn compile(&self) -> Result<CompiledSystem> {
        if let Some(rng) = &self.sequence_rng {
            rng.validate()?;
        }
        if self.sequence_rng.is_some() {
            ensure!(
                matches!(
                    self.clock,
                    crate::falling_atlas::SimulationClock::External { .. }
                ) && self
                    .emitters
                    .iter()
                    .all(|e| e.target == SimulationTarget::Cpu),
                "VFX sequence streams require explicit external CPU clock"
            );
        }
        if let Some(exchange) = &self.sequence_exchange {
            exchange.state.validate()?;
            ensure!(
                exchange.epoch > 0 && self.sequence_rng.is_some(),
                "VFX sequence exchange requires an initialized stream and epoch"
            );
        }
        ensure!(
            !self.emitters.is_empty()
                && self.emitters.len() <= 64
                && self.parameters.len() <= 128
                && (1..=240).contains(&self.fixed_step_hz)
                && self.warmup_seconds.is_finite()
                && (0.0..=10.).contains(&self.warmup_seconds)
                && (1..=4096).contains(&self.max_steps_per_advance)
                && self.time_scale.is_finite()
                && (0.0..=16.).contains(&self.time_scale),
            "VFX system limits"
        );
        for (k, v) in &self.parameters {
            name(k)?;
            v.validate()?;
        }
        let mut names = std::collections::BTreeSet::new();
        let mut screen_layers = std::collections::BTreeSet::new();
        for e in &self.emitters {
            ensure!(names.insert(e.id.clone()), "duplicate VFX emitter id");
            e.validate(&self.parameters)?;
            if let Some(node) = &e.falling_atlas {
                ensure!(matches!(self.clock,crate::falling_atlas::SimulationClock::External{..})&&self.sequence_rng.is_some()&&self.warmup_seconds==0.&&self.time_scale==1.&&node.tick_hz==self.fixed_step_hz,"VFX falling node requires explicit external clocks, one stream, matching fixed ticks and no warmup/time scaling");
            }
            if let Some(screen) = &e.screen {
                ensure!(
                    screen_layers.insert(screen.layer),
                    "duplicate VFX screen target layer"
                );
            }
        }
        for e in &self.emitters {
            for h in &e.events {
                ensure!(
                    !self
                        .emitters
                        .iter()
                        .any(|target| target.id == h.emitter && target.falling_atlas.is_some()),
                    "VFX falling child event targets are not implemented"
                );
                ensure!(
                    names.contains(&h.emitter),
                    "VFX event target '{}' absent",
                    h.emitter
                );
                ensure!(
                    e.target == SimulationTarget::Cpu
                        && self
                            .emitters
                            .iter()
                            .find(|t| t.id == h.emitter)
                            .is_some_and(|t| t.target == SimulationTarget::Cpu),
                    "VFX child events currently require CPU source and target emitters"
                );
            }
        }
        Ok(CompiledSystem { spec: self.clone() })
    }
    pub fn validate(&self) -> Result<()> {
        self.compile().map(|_| ())
    }
    pub fn with_parameters(&self, overrides: &ParameterMap) -> Result<Self> {
        let mut out = self.clone();
        for (k, v) in overrides {
            let original = out
                .parameters
                .get(k)
                .ok_or_else(|| anyhow::anyhow!("unknown VFX parameter '{k}'"))?;
            ensure!(original.same_type(v), "VFX parameter type mismatch");
            v.validate()?;
            out.parameters.insert(k.clone(), v.clone());
        }
        out.compile()?;
        Ok(out)
    }
}

impl CompiledEmitter {
    /// Constant-fold typed bindings once per tick. The original compiled plan
    /// stays intact for GPU history replay with older parameter maps.
    pub fn specialize(&self) -> Result<Self> {
        let mut out = self.clone();
        let p = &self.parameters;
        out.spec.spawn.rate = ScalarValue::Constant(out.spec.spawn.rate.resolve(p)?);
        out.spec.initialize.direction =
            VectorValue::Constant(out.spec.initialize.direction.resolve(p)?);
        if let Some(name) = out.spec.initialize.color_parameter.take() {
            let Some(ParameterValue::Color(value)) = p.get(&name) else {
                bail!("VFX birth color parameter missing/type");
            };
            out.spec.initialize.color = *value;
        }
        if let Some(name) = out.spec.initialize.size_parameter.take() {
            let Some(ParameterValue::Vector(value)) = p.get(&name) else {
                bail!("VFX birth size parameter missing/type");
            };
            out.spec.initialize.size = [value[0], value[1]];
        }
        fn scalar(v: &mut ScalarValue, p: &ParameterMap) -> Result<()> {
            *v = ScalarValue::Constant(v.resolve(p)?);
            Ok(())
        }
        fn vector(v: &mut VectorValue, p: &ParameterMap) -> Result<()> {
            *v = VectorValue::Constant(v.resolve(p)?);
            Ok(())
        }
        for m in &mut out.spec.update {
            match m {
                Module::Acceleration { value } => vector(value, p)?,
                Module::Drag { coefficient } => scalar(coefficient, p)?,
                Module::CurlNoise {
                    strength,
                    frequency,
                } => {
                    scalar(strength, p)?;
                    scalar(frequency, p)?;
                }
                Module::Attractor {
                    point,
                    strength,
                    radius,
                } => {
                    vector(point, p)?;
                    scalar(strength, p)?;
                    scalar(radius, p)?;
                }
                Module::Orbit { axis, speed } => {
                    vector(axis, p)?;
                    scalar(speed, p)?;
                }
                Module::Rotate { radians_per_second } => scalar(radians_per_second, p)?,
                Module::CollisionPlane { normal, offset, .. } => {
                    vector(normal, p)?;
                    scalar(offset, p)?;
                }
                Module::CollisionSphere { center, radius, .. } => {
                    vector(center, p)?;
                    scalar(radius, p)?;
                }
                Module::ColorOverLife { .. } | Module::SizeOverLife { .. } => {}
            }
        }
        Ok(out)
    }
}
