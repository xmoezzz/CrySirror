// SPDX-License-Identifier: MPL-2.0
//! Gameplay components carried by world entities (ADR 0012): skeletal
//! animation state and collider/body specifications. Plain, saved data;
//! poses and physics broad-phase structures are derived from it.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

/// Dynamic diffuse global illumination volume (ADR 0028): a grid of
/// irradiance probes centred on the entity, refreshed a few per frame by
/// rendering the lit scene around each probe. Each refresh sees the previous
/// result, so bounces accumulate over updates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrradianceVolumeSpec {
    /// Half size of the volume (world axes, meters).
    pub extent: [f32; 3],
    /// Probes per axis (≥ 2 each, ≤ 4096 in total).
    #[serde(default = "volume_counts")]
    pub counts: [u32; 3],
    /// Multiplier of the indirect diffuse light (0..16).
    #[serde(default = "probe_one")]
    pub intensity: f32,
    /// Probes refreshed per frame, round-robin (1..256); each costs six capture
    /// renders.
    #[serde(default = "volume_rate")]
    pub probes_per_frame: u32,
    /// Probes refreshed per frame once every probe has been refreshed
    /// three times and no light, static object, material override, sun or
    /// environment changed (0 = stop refreshing until something changes).
    #[serde(default)]
    pub idle_probes_per_frame: u32,
    /// Chebyshev visibility from per-probe distance moments against light
    /// leaking through walls (false: interpolate without occlusion).
    #[serde(default = "yes")]
    pub visibility: bool,
    /// Bump to restart accumulation.
    #[serde(default)]
    pub revision: u32,
    /// On the hardware ray-tracing path, update probes with ray queries
    /// against the scene instead of six raster face captures (ADR 0059).
    /// Ray-traced updates light hits by the sun, emission and the volume
    /// itself; local lights are not included in their bounce yet.
    #[serde(default)]
    pub ray_traced: bool,
}
fn volume_counts() -> [u32; 3] {
    [8, 4, 8]
}
fn volume_rate() -> u32 {
    8
}
impl IrradianceVolumeSpec {
    pub const MAX_PROBES: u32 = 4096;
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.extent
                .iter()
                .all(|v| v.is_finite() && (0.01..=100_000.).contains(v))
                && self.counts.iter().all(|c| (2..=64).contains(c))
                && self.counts.iter().product::<u32>() <= Self::MAX_PROBES
                && self.intensity.is_finite()
                && (0.0..=16.).contains(&self.intensity)
                && (1..=256).contains(&self.probes_per_frame)
                && self.idle_probes_per_frame <= self.probes_per_frame,
            "invalid irradiance volume"
        );
        Ok(())
    }
}

/// Box-projected reflection capture (ADR 0024). The renderer captures the
/// scene around the entity's world position into a prefiltered radiance map
/// and uses it, parallax-corrected against the box, for glossy reflections
/// of points inside the box (UE-style box reflection capture).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionProbeSpec {
    /// Half size of the influence/projection box (world axes, meters).
    pub extent: [f32; 3],
    /// Multiplier of the captured radiance (0..16).
    #[serde(default = "probe_one")]
    pub intensity: f32,
    /// Distance inside the box faces over which the probe fades in.
    #[serde(default = "probe_blend")]
    pub blend_distance: f32,
    /// Bump to request a recapture of an unchanged probe (e.g. after moving lights).
    #[serde(default)]
    pub revision: u32,
    /// Authored equirect Radiance `.hdr` radiance instead of an engine
    /// capture (e.g. converted legacy cubemaps; ADR 0025).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Box-projected (true) or infinitely distant (false) reflections.
    #[serde(default = "probe_parallax")]
    pub parallax: bool,
}
fn probe_parallax() -> bool {
    true
}
fn probe_one() -> f32 {
    1.
}
fn probe_blend() -> f32 {
    0.5
}
impl ReflectionProbeSpec {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.extent
                .iter()
                .all(|v| v.is_finite() && (0.01..=100_000.).contains(v))
                && self.intensity.is_finite()
                && (0.0..=16.).contains(&self.intensity)
                && self.blend_distance.is_finite()
                && (0.0..=100_000.).contains(&self.blend_distance),
            "invalid reflection probe"
        );
        if let Some(s) = &self.source {
            anyhow::ensure!(
                s.ends_with(".hdr")
                    && !s.starts_with('/')
                    && !s.contains("..")
                    && !s.contains('\\'),
                "reflection probe source must be a relative .hdr asset"
            );
        }
        Ok(())
    }
}

pub const MAX_ANIMATION_LAYERS: usize = 4;

fn one() -> f32 {
    1.
}
fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationLayer {
    /// Clip name in the entity's glTF scene.
    pub clip: String,
    /// Seconds into the clip (wrapped when looping).
    #[serde(default)]
    pub time: f32,
    #[serde(default = "one")]
    pub speed: f32,
    #[serde(default = "one")]
    pub weight: f32,
    #[serde(default = "one")]
    pub target_weight: f32,
    /// Weight change per second toward `target_weight`.
    #[serde(default)]
    pub fade: f32,
    #[serde(default = "yes")]
    pub looping: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationState {
    #[serde(default)]
    pub layers: Vec<AnimationLayer>,
    /// State machine instance; when set, layers are derived from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<crate::animgraph::MachineInstance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub montage: Option<crate::animgraph::MontageInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::animgraph::AnimEvent>,
    /// Morph target (blend shape) weights by target name, applied to every
    /// mesh of the model that has a target of that name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub morphs: std::collections::BTreeMap<String, f32>,
}
/// Bound on named morph weights per entity.
pub const MAX_MORPH_WEIGHTS: usize = 1024;
impl AnimationState {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.layers.len() <= MAX_ANIMATION_LAYERS,
            "too many animation layers"
        );
        for l in &self.layers {
            ensure!(
                !l.clip.is_empty()
                    && l.clip.len() <= 256
                    && [l.time, l.speed, l.weight, l.target_weight, l.fade]
                        .iter()
                        .all(|v| v.is_finite())
                    && l.speed.abs() <= 100.
                    && (0.0..=1.).contains(&l.weight)
                    && (0.0..=1.).contains(&l.target_weight)
                    && (0.0..=1000.).contains(&l.fade)
                    && l.time.abs() <= 1_000_000.,
                "invalid animation layer"
            );
        }
        ensure!(self.morphs.len() <= MAX_MORPH_WEIGHTS, "too many morph weights");
        for (name, w) in &self.morphs {
            ensure!(
                !name.is_empty() && name.len() <= 256 && w.is_finite() && w.abs() <= 16.,
                "invalid morph weight `{name}`"
            );
        }
        Ok(())
    }
    /// Cross-fade to `clip` over `fade_seconds` (0 = cut). Re-playing the
    /// active clip only updates its speed.
    pub fn play(&mut self, clip: &str, speed: f32, fade_seconds: f32, looping: bool) {
        let rate = if fade_seconds > 0. {
            (1. / fade_seconds).min(1000.)
        } else {
            1000.
        };
        let mut found = false;
        for l in &mut self.layers {
            if l.clip == clip {
                l.target_weight = 1.;
                l.speed = speed;
                l.fade = rate;
                l.looping = looping;
                found = true;
            } else {
                l.target_weight = 0.;
                l.fade = rate;
            }
        }
        if !found {
            if self.layers.len() == MAX_ANIMATION_LAYERS {
                // Drop the weakest layer to make room.
                if let Some(i) = self
                    .layers
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.weight.total_cmp(&b.1.weight))
                    .map(|(i, _)| i)
                {
                    self.layers.remove(i);
                }
            }
            self.layers.push(AnimationLayer {
                clip: clip.into(),
                time: 0.,
                speed,
                weight: if fade_seconds > 0. && !self.layers.is_empty() {
                    0.
                } else {
                    1.
                },
                target_weight: 1.,
                fade: rate,
                looping,
            });
        }
    }
    /// Advance graph-driven animation (machine + montage) or plain layers.
    pub fn advance_with(&mut self, defs: &crate::animgraph::AnimDefs, dt: f32) {
        if self.machine.is_none() && self.montage.is_none() {
            return self.advance(dt);
        }
        if let Some(mi) = self.machine.as_mut() {
            if let Some(def) = defs.machines.get(&mi.machine) {
                if let Some(entered) = mi.advance(def, dt) {
                    self.events.push(crate::animgraph::AnimEvent {
                        kind: "state_entered".into(),
                        name: entered,
                    });
                }
            }
        }
        if let Some(m) = self.montage.as_mut() {
            let alive = match defs.montages.get(&m.montage) {
                Some(def) => crate::animgraph::advance_montage(m, def, dt, &mut self.events),
                None => false,
            };
            if !alive {
                self.montage = None;
            }
        }
        if self.events.len() > 64 {
            let n = self.events.len() - 64;
            self.events.drain(..n);
        }
        if let Ok(l) = crate::animgraph::compose(defs, self.machine.as_ref(), self.montage.as_ref())
        {
            self.layers = l;
        }
    }
    /// Advance clock and fades by `dt` seconds; drops faded-out layers.
    pub fn advance(&mut self, dt: f32) {
        for l in &mut self.layers {
            l.time += dt * l.speed;
            if l.time.abs() > 100_000. {
                l.time = l.time.rem_euclid(1000.);
            }
            let step = l.fade * dt;
            l.weight = if l.weight < l.target_weight {
                (l.weight + step).min(l.target_weight)
            } else {
                (l.weight - step).max(l.target_weight)
            };
        }
        self.layers
            .retain(|l| l.weight > 0. || l.target_weight > 0.);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Immovable level geometry.
    #[default]
    Static,
    /// Simulated rigid body. Physics owns its transform and velocity.
    Dynamic,
    /// Velocity-driven kinematic body; pushes dynamic bodies without gravity.
    Kinematic,
    /// Position-driven kinematic body, moved to explicit targets by physics.
    KinematicPosition,
    /// Kinematic capsule driven by the character controller.
    Character,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColliderShape {
    /// Axis-aligned box in entity space.
    Box { half_extents: [f32; 3] },
    /// Sphere.
    Sphere { radius: f32 },
    /// Y-up capsule: total height = 2 * (half_height + radius).
    Capsule { radius: f32, half_height: f32 },
    /// Triangle mesh of the entity's own geometry (cube, triangles, model, scene).
    Mesh,
    /// Convex hull of explicit entity-local points.
    ConvexHull { points: Vec<[f32; 3]> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CollisionGroups {
    /// Membership bit mask, applied only inside this client's physics world.
    pub memberships: u32,
    /// Membership bits this collider may collide/query against.
    pub filter: u32,
}
impl Default for CollisionGroups {
    fn default() -> Self {
        Self {
            memberships: u32::MAX,
            filter: u32::MAX,
        }
    }
}
fn friction() -> f32 {
    0.5
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColliderSpec {
    /// Collision shape.
    pub shape: ColliderShape,
    /// Static, dynamic, kinematic or character body.
    #[serde(default)]
    pub body: BodyKind,
    /// Collider center relative to the entity origin.
    #[serde(default)]
    pub offset: [f32; 3],
    /// Optional total body mass in kilograms. Otherwise derived from density.
    #[serde(default)]
    pub mass: Option<f32>,
    /// Collider density in kg/m³ when total mass is not specified.
    #[serde(default = "one")]
    pub density: f32,
    /// Nonnegative Coulomb friction coefficient. Default 0.5.
    #[serde(default = "friction")]
    pub friction: f32,
    /// Restitution coefficient in 0..1.
    #[serde(default)]
    pub restitution: f32,
    /// Multiplier of client gravity. Default 1.
    #[serde(default = "one")]
    pub gravity_scale: f32,
    /// Nonnegative linear drag rate in 1/s.
    #[serde(default)]
    pub linear_damping: f32,
    /// Nonnegative angular drag rate in 1/s.
    #[serde(default)]
    pub angular_damping: f32,
    /// Enable continuous collision detection for this body.
    #[serde(default)]
    pub ccd: bool,
    /// Permit automatic and explicit sleeping.
    #[serde(default = "yes")]
    pub can_sleep: bool,
    /// Enable this rigid body and its collider.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Detect overlaps/events without a solid contact response.
    #[serde(default)]
    pub sensor: bool,
    /// Collision/query membership and filter masks; all bits by default.
    #[serde(default)]
    pub groups: CollisionGroups,
    /// Lock world X/Y/Z translation axes.
    #[serde(default)]
    pub locked_translation: [bool; 3],
    /// Lock world X/Y/Z rotation axes.
    #[serde(default)]
    pub locked_rotation: [bool; 3],
    /// Optional minimum contact-force event magnitude in newtons.
    #[serde(default)]
    pub contact_force_threshold: Option<f32>,
}
impl ColliderSpec {
    pub fn validate(&self) -> Result<()> {
        let pos = |v: f32| v.is_finite() && v > 1e-4 && v <= 100_000.;
        let ok = match &self.shape {
            ColliderShape::Box { half_extents } => half_extents.iter().all(|v| pos(*v)),
            ColliderShape::Sphere { radius } => pos(*radius),
            ColliderShape::Capsule {
                radius,
                half_height,
            } => {
                pos(*radius)
                    && half_height.is_finite()
                    && *half_height >= 0.
                    && *half_height <= 100_000.
            }
            ColliderShape::Mesh => true,
            ColliderShape::ConvexHull { points } => {
                (4..=65_536).contains(&points.len())
                    && points
                        .iter()
                        .flatten()
                        .all(|v| v.is_finite() && v.abs() <= 100_000.)
            }
        };
        ensure!(
            ok && self
                .offset
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 100_000.),
            "invalid collider"
        );
        let nonnegative = |v: f32| v.is_finite() && (0.0..=1_000_000.).contains(&v);
        ensure!(
            self.mass
                .is_none_or(|m| m.is_finite() && m > 1e-6 && m <= 1_000_000.)
                && nonnegative(self.density)
                && nonnegative(self.friction)
                && self.restitution.is_finite()
                && (0.0..=1.).contains(&self.restitution)
                && self.gravity_scale.is_finite()
                && self.gravity_scale.abs() <= 1000.
                && nonnegative(self.linear_damping)
                && nonnegative(self.angular_damping)
                && self.contact_force_threshold.is_none_or(nonnegative),
            "invalid rigid body material or mass parameters"
        );
        ensure!(
            !(self.body == BodyKind::Character
                && !matches!(self.shape, ColliderShape::Capsule { .. })),
            "character bodies must be capsules"
        );
        ensure!(
            self.body != BodyKind::Dynamic || self.mass.is_some() || self.density > 1e-6,
            "dynamic bodies require positive density or explicit mass"
        );
        ensure!(self.body!=BodyKind::Dynamic||!matches!(self.shape,ColliderShape::Mesh)||self.mass.is_some(),"dynamic triangle meshes require explicit mass; convex hulls can derive mass from density");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crossfade_moves_weights_and_drops_faded_layers() {
        let mut a = AnimationState::default();
        a.play("walk", 1., 0.2, true);
        assert_eq!(a.layers[0].weight, 1.);
        a.play("run", 1.5, 0.2, true);
        assert_eq!((a.layers[0].target_weight, a.layers[1].weight), (0., 0.));
        a.advance(0.1);
        assert!((a.layers[0].weight - 0.5).abs() < 1e-5 && (a.layers[1].weight - 0.5).abs() < 1e-5);
        assert!((a.layers[1].time - 0.15).abs() < 1e-5);
        a.advance(0.2);
        assert_eq!(a.layers.len(), 1);
        assert_eq!(a.layers[0].clip, "run");
        a.validate().unwrap();
    }
    #[test]
    fn colliders_validate() {
        let c: ColliderSpec = serde_json::from_value(serde_json::json!({"shape":{"kind":"capsule","radius":0.3,"half_height":0.6},"body":"character"})).unwrap();
        c.validate().unwrap();
        let bad: ColliderSpec = serde_json::from_value(
            serde_json::json!({"shape":{"kind":"box","half_extents":[1,1,1]},"body":"character"}),
        )
        .unwrap();
        assert!(bad.validate().is_err());
    }
}

fn rate() -> f32 {
    20.
}
fn life() -> [f32; 2] {
    [1., 1.5]
}
fn spd() -> [f32; 2] {
    [1., 2.]
}
fn up() -> [f32; 3] {
    [0., 1., 0.]
}
fn sz() -> [f32; 2] {
    [0.1, 0.02]
}
fn c0() -> [f32; 4] {
    [4., 2., 0.6, 1.]
}
fn c1() -> [f32; 4] {
    [1., 0.2, 0.05, 0.]
}
fn maxp() -> u32 {
    512
}
/// A CPU particle emitter (ADR 0014). The spec is saved; particle state is a
/// visual simulation rebuilt by the renderer and never saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterSpec {
    /// Particles per second.
    #[serde(default = "rate")]
    pub rate: f32,
    /// Particles emitted once when the emitter appears.
    #[serde(default)]
    pub burst: u32,
    /// Random lifetime range [min, max] (s).
    #[serde(default = "life")]
    pub lifetime: [f32; 2],
    /// Random initial speed range [min, max] (units/s).
    #[serde(default = "spd")]
    pub speed: [f32; 2],
    /// Emission axis in entity space.
    #[serde(default = "up")]
    pub direction: [f32; 3],
    /// Cone half angle around `direction`.
    #[serde(default)]
    pub spread_degrees: f32,
    /// Constant acceleration (world units/s²).
    #[serde(default)]
    pub gravity: [f32; 3],
    /// Linear velocity damping (1/s).
    #[serde(default)]
    pub drag: f32,
    /// Start/end size (world units).
    #[serde(default = "sz")]
    pub size: [f32; 2],
    /// Linear HDR color + alpha at birth and death.
    #[serde(default = "c0")]
    pub color_start: [f32; 4],
    /// Linear HDR color + alpha at death.
    #[serde(default = "c1")]
    pub color_end: [f32; 4],
    /// Maximum live particles.
    #[serde(default = "maxp")]
    pub max: u32,
    /// Additive blending (false: alpha blending).
    #[serde(default = "yes")]
    pub additive: bool,
    /// Emit new particles (live ones finish).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Random seed, for reproducible emission.
    #[serde(default)]
    pub seed: u32,
}
impl EmitterSpec {
    pub fn validate(&self) -> Result<()> {
        let f = |v: &[f32]| v.iter().all(|x| x.is_finite() && x.abs() <= 1e6);
        ensure!(
            f(&[self.rate, self.spread_degrees, self.drag])
                && f(&self.lifetime)
                && f(&self.speed)
                && f(&self.direction)
                && f(&self.gravity)
                && f(&self.size)
                && f(&self.color_start)
                && f(&self.color_end)
                && (0.0..=10_000.).contains(&self.rate)
                && self.lifetime[0] > 0.
                && self.lifetime[1] >= self.lifetime[0]
                && (1..=65_536).contains(&self.max)
                && self.burst <= 65_536,
            "invalid particle emitter"
        );
        Ok(())
    }
}

/// Local light shape. Point and spot lights are punctual; a rect light is a
/// one-sided area emitter facing the entity's -Z (spot axis convention).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum LightShape {
    /// Omnidirectional punctual light (candela).
    Point,
    /// Cone along local -Z with inner/outer half angles (degrees).
    Spot {
        /// Full-intensity half angle, degrees.
        #[serde(default = "spot_inner")]
        inner_degrees: f32,
        /// Zero-intensity half angle, degrees (≤ 89).
        #[serde(default = "spot_outer")]
        outer_degrees: f32,
    },
    /// One-sided area light of `size` [w, h], facing local -Z.
    Rect {
        #[serde(default = "one2")]
        size: [f32; 2],
    },
    /// Parallel light along local -Z confined to the range sphere (a local
    /// "light volume"); intensity is illuminance (lux), not candela.
    Directional,
    /// Uniform diffuse irradiance inside the range sphere (an ambient volume);
    /// intensity is illuminance (lux).
    Ambient,
}
impl LightShape {
    /// Whether intensity is luminous intensity (candela) rather than illuminance.
    pub fn punctual(self) -> bool {
        matches!(self, Self::Point | Self::Spot { .. } | Self::Rect { .. })
    }
}
impl Default for LightShape {
    fn default() -> Self {
        Self::Point
    }
}
fn spot_inner() -> f32 {
    20.
}
fn spot_outer() -> f32 {
    35.
}
fn one2() -> [f32; 2] {
    [1.; 2]
}
/// Distance attenuation inside [`LightSpec::range`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum Falloff {
    /// Physical 1/d² with a smooth window ending at the range.
    #[default]
    InverseSquare,
    /// `(1 − d²/r²)^exponent` with no 1/d² term (UE's non-inverse-squared
    /// lights, and artist-tuned lights of older engines). Intensity is then
    /// the illuminance at the light's position.
    Exponent { exponent: f32 },
}
/// Photometric unit of [`LightSpec::intensity`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightUnits {
    /// Luminous intensity (lm/sr), the renderer's native unit.
    #[default]
    Candela,
    /// Total luminous flux. Point and spot lights use lm / 4π (as UE does, so
    /// narrowing a cone does not brighten it); rect lights use lm / π.
    Lumens,
}
/// A local light on an entity (ADR 0018). Position and direction come from
/// the entity's world transform; spot and rect lights emit along local -Z.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightSpec {
    /// Point, spot, rect, directional volume or ambient volume.
    #[serde(default)]
    pub shape: LightShape,
    /// Linear RGB color.
    #[serde(default = "white")]
    pub color: [f32; 3],
    /// Brightness in `units` (illuminance for directional/ambient).
    #[serde(default = "light_intensity")]
    pub intensity: f32,
    /// Photometric unit of `intensity`.
    #[serde(default)]
    pub units: LightUnits,
    /// Distance attenuation inside `range`.
    #[serde(default)]
    pub falloff: Falloff,
    /// Attenuation radius (world units); influence ends smoothly here.
    #[serde(default = "light_range")]
    pub range: f32,
    /// Cast shadows (budgeted by the renderer).
    #[serde(default)]
    pub shadows: bool,
    /// Turn the light off without removing it.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Radius of a point/spot emitter sphere (world units). Hardware
    /// ray-traced shadows sample it for penumbrae; 0 keeps hard shadows.
    #[serde(default)]
    pub source_radius: f32,
}
fn white() -> [f32; 3] {
    [1.; 3]
}
fn light_intensity() -> f32 {
    10.
}
fn light_range() -> f32 {
    10.
}
impl Default for LightSpec {
    fn default() -> Self {
        Self {
            shape: LightShape::Point,
            color: white(),
            intensity: light_intensity(),
            units: LightUnits::Candela,
            falloff: Falloff::InverseSquare,
            range: light_range(),
            shadows: false,
            enabled: true,
            source_radius: 0.,
        }
    }
}
impl LightSpec {
    pub fn validate(&self) -> Result<()> {
        let fin = |v: &[f32]| v.iter().all(|x| x.is_finite());
        ensure!(
            fin(&self.color) && self.color.iter().all(|c| (0.0..=1000.).contains(c)),
            "invalid light color"
        );
        ensure!(
            self.intensity.is_finite() && (0.0..=1e9).contains(&self.intensity),
            "invalid light intensity"
        );
        ensure!(
            self.range.is_finite() && (0.01..=100_000.).contains(&self.range),
            "invalid light range"
        );
        ensure!(
            self.source_radius.is_finite()
                && self.source_radius >= 0.
                && self.source_radius <= self.range,
            "invalid light source radius"
        );
        match self.shape {
            LightShape::Point => {}
            LightShape::Spot {
                inner_degrees,
                outer_degrees,
            } => ensure!(
                inner_degrees.is_finite()
                    && outer_degrees.is_finite()
                    && 0.0 <= inner_degrees
                    && inner_degrees <= outer_degrees
                    && (0.1..=89.).contains(&outer_degrees),
                "invalid spot cone"
            ),
            LightShape::Rect { size } => ensure!(
                fin(&size) && size.iter().all(|s| (0.001..=10_000.).contains(s)),
                "invalid rect light size"
            ),
            LightShape::Directional | LightShape::Ambient => ensure!(
                self.units == LightUnits::Candela,
                "directional/ambient light intensity is illuminance; lumens do not apply"
            ),
        }
        if let Falloff::Exponent { exponent } = self.falloff {
            ensure!(
                exponent.is_finite() && (0.01..=64.).contains(&exponent),
                "invalid light falloff exponent"
            );
        }
        Ok(())
    }
    /// Intensity in candela (for rect lights: candela along the normal).
    pub fn candela(&self) -> f32 {
        match (self.units, self.shape) {
            (LightUnits::Candela, _) => self.intensity,
            (LightUnits::Lumens, LightShape::Rect { .. }) => self.intensity / std::f32::consts::PI,
            (LightUnits::Lumens, _) => self.intensity / (4. * std::f32::consts::PI),
        }
    }
}
