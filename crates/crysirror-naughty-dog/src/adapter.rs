// SPDX-License-Identifier: MPL-2.0
//! Player session for the installed BCAS20097 disc.
//!
//! Task mode (the default) boots the native DC scripts from `dc1.psarc`
//! (`startup` and the modules it requests), selects a task subnode of the
//! game task graph, loads that subnode's level with its `cache` levels and
//! the node's world level from `pak22.psarc`, places the player at the
//! subnode's continue spawner and runs the state-script processes of the
//! level's spawners every frame. Spawners with flag 0x40000000 (continue
//! points, companions, cameras: inferred as no-auto-birth) start hidden
//! and appear on `spawn-entity`. The player moves on the walkable level
//! surfaces (WASD, Shift runs, arrows turn the camera); Page Down/Page Up
//! step to the next/previous task subnode in task order.
//!
//! `{"level": NAME}` without a task opens the free-camera level viewer
//! (WASD/QE move, arrows turn, Page Down/Up switch `pak22.txt` levels).
//! Native animation, collision, AI and saves are not executed
//! (SATD-nd-retail-runtime).
use crate::{
    dc::sid,
    game::Game,
    ground::Ground,
    levels,
    package::Directory,
    scene::{self, SpawnerRecord},
    ss::{LightLocator, Property, World},
    task::Manager,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{AssetReader, HostServices, InputFrame, buttons as b},
    state::ClientId,
    vfs::VfsProvider,
    vm::{VmAdapter, VmYield},
};
use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry, read_all};
use crysirror_scene::{components::{Falloff, LightShape, LightSpec}, world::Transform};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const ENGINE_ID: &str = "naughty-dog";
const MAIN: &str = "PS3_GAME/USRDIR/build/main";
/// Largest decoded package and base-colour image edge for the viewer.
const PACKAGE_LIMIT: u64 = 256 << 20;
const MAX_TEXTURE: u32 = 512;
const FRAME_MS: u64 = 16;
const SCREEN: [u32; 2] = [1280, 720];
const FADE_LAYER: u32 = 0x4e44_0001;
const NO_AUTO_BIRTH: u32 = 0x4000_0000;
const MOVIE_LAYER: u32 = 0x4e44_0002;
/// Message dialog and main-menu text layers.
const UI_LAYER: u32 = 0x4e44_0010;
const MENU_LEVEL: &str = "menu-temple";
/// Front-end layers: logo, title and up to ten buttons.
const UI_LAYERS: u32 = 12;
/// The world assigns the first video the first video surface slot.
const MOVIE_SURFACE: &str = "__surface/1000";
/// World videos are read whole (`crysirror_runtime::vfs::MAX_FILE`).
const MAX_WORLD_VIDEO: u64 = crysirror_core::limits::LARGE_ASSET_BYTES;
const MAX_LOG: usize = 64;
/// The BCAS20097 light pool has 32 records (sub_7A88C).
const DYNAMIC_LIGHT_IDS: std::ops::Range<u32> = 0x4e4c_0000..0x4e4c_0020;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// `level-name` of `pak22.txt` for the level viewer.
    #[serde(default)]
    pub level: Option<String>,
    /// Task subnode to start (default: the first task of the boot sequence).
    #[serde(default)]
    pub task: Option<String>,
    /// `text1.psarc` catalog number (`N.common`); 2 is English.
    #[serde(default)]
    pub language: Option<u32>,
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 128 && n.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.level.as_deref().is_none_or(valid_name), "Naughty Dog level name");
        ensure!(self.task.as_deref().is_none_or(valid_name), "Naughty Dog task name");
        Ok(())
    }
    fn viewer(&self) -> bool {
        self.level.is_some() && self.task.is_none()
    }
}

/// Free camera in metres; yaw/pitch in radians.
#[derive(Clone, Copy, Debug)]
struct Camera {
    eye: [f32; 3],
    yaw: f32,
    pitch: f32,
    speed: f32,
}

impl Camera {
    fn forward(&self) -> [f32; 3] {
        [self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos()]
    }
    fn op(&self) -> serde_json::Value {
        let f = self.forward();
        let target: [f32; 3] = std::array::from_fn(|k| self.eye[k] + f[k]);
        json!({"op": "set_camera", "camera": {"eye": self.eye, "target": target, "up": [0.0, 1.0, 0.0], "fov_y_degrees": 60.0, "near": 0.1, "far": 20000.0}})
    }
    fn step(&mut self, input: &InputFrame, seconds: f32) -> bool {
        let down = |m| input.buttons_down & m != 0;
        let turn = 1.5 * seconds;
        let mut moved = false;
        let mut adjust = |cond: bool, f: &mut dyn FnMut()| {
            if cond {
                f();
                moved = true;
            }
        };
        adjust(down(b::LEFT), &mut || self.yaw -= turn);
        adjust(down(b::RIGHT), &mut || self.yaw += turn);
        adjust(down(b::UP), &mut || self.pitch = (self.pitch + turn).min(1.5));
        adjust(down(b::DOWN), &mut || self.pitch = (self.pitch - turn).max(-1.5));
        let speed = self.speed * seconds * if down(b::SHIFT) { 4. } else { 1. };
        let (f, r) = (self.forward(), [self.yaw.cos(), 0., self.yaw.sin()]);
        let mut delta = [0f32; 3];
        for (key, dir, sign) in [(b::KEY_W, f, 1.), (b::KEY_S, f, -1.), (b::KEY_D, r, 1.), (b::KEY_A, r, -1.), (b::KEY_E, [0., 1., 0.], 1.), (b::KEY_Q, [0., 1., 0.], -1.)] {
            if down(key) {
                moved = true;
                for k in 0..3 {
                    delta[k] += dir[k] * sign * speed;
                }
            }
        }
        for k in 0..3 {
            self.eye[k] += delta[k];
        }
        moved
    }
}

/// The archive path of a declared package: the exact name, else its only
/// numbered sibling (`con-1-start-ingame` → `con-1-start-ingame10.pak`).
/// The native variant rule is unknown (SATD-nd-retail-runtime).
fn package_path(mount: &ArchiveMount, name: &str) -> Option<String> {
    let exact = format!("{name}.pak");
    if mount.exists(&exact) {
        return Some(exact);
    }
    let mut numbered = (0..100).map(|n| format!("{name}{n}.pak")).filter(|p| mount.exists(p));
    let first = numbered.next()?;
    numbered.next().is_none().then_some(first)
}

/// One queued load step.
enum Load {
    /// A level package of `pak22.psarc`.
    Package(String),
    /// An art group of `actor27.psarc`, placed at each spawner.
    Actor(String, Vec<SpawnerRecord>),
    /// Part of a package's meshes whose model exceeded the host's
    /// per-model limit: `(package, archive path, meshes)`.
    Part(String, String, Vec<crate::mesh::Mesh>),
    /// `PartSpawner`s of a package, presented once the particle modules
    /// its regions name are loaded.
    Particles(Vec<SpawnerRecord>),
}

/// Largest generated model: the host's large-asset limit less a margin.
const MODEL_LIMIT: usize = (crysirror_core::limits::LARGE_ASSET_BYTES as usize) / 16 * 15;

/// The player character placed at the continue spawner.
struct Player {
    entity: Option<u32>,
    position: [f32; 3],
    yaw: f32,
    fall: f32,
    /// Camera orbit yaw and pitch around the player.
    view_yaw: f32,
    view_pitch: f32,
    controls: bool,
    /// Native animation state machine and blend-tree controller.
    anim: Option<crate::player::Controller>,
    /// Highest height since leaving the ground.
    air_top: Option<f32>,
    /// Current follow-camera distance (eases back out after obstruction).
    view_distance: f32,
    /// The body the player stands on and its transform then: the player
    /// rides its motion (the hanging car).
    platform: Option<(u32, glam::Mat4)>,
    /// The edge held while hanging or climbing.
    grip: Option<Grip>,
    /// A jump between edges in progress: the root it started from, the
    /// target grip and the seconds elapsed.
    leap: Option<(glam::Vec3, Grip, f32)>,
    /// Seconds before an airborne player may grab again (after a drop).
    regrab: f32,
}

/// Engine side of the script natives: spawner entities, prints and fades.
#[derive(Default)]
struct Stage {
    now: f64,
    /// Spawner records of the loaded packages by name `StringId`.
    spawners: HashMap<u32, SpawnerRecord>,
    /// Entities of each spawner.
    entities: HashMap<u32, Vec<u32>>,
    visible: HashSet<u32>,
    levels: HashSet<u32>,
    ops: Vec<serde_json::Value>,
    log: Vec<String>,
    fade: f32,
    fade_color: [u8; 3],
    fade_target: f32,
    fade_rate: f32,
    controls: Option<bool>,
    camera: Option<u32>,
    /// Left stick from the movement keys.
    stick: [f32; 2],
    /// Script animations by object (spawner name or `player`).
    animations: HashMap<u32, ScriptAnimation>,
    /// Where the player stands, for `player` locators.
    player_locator: Option<([f32; 3], [f32; 4])>,
    /// Props without an animation controller hold the last frame of their
    /// finished script animation: asset, clip and time.
    held: HashMap<u32, (String, String, f32)>,
    /// Camera animations by name: clip, start, blend-in seconds and the
    /// time a disable began blending it out.
    camera_animations: Vec<CameraAnimation>,
    /// Current world transforms of spawned objects moved away from their
    /// spawner (script root motion, or a moved parent spawner).
    poses: HashMap<u32, glam::Mat4>,
    /// The body (entity) the player stands on or hangs from.
    player_support: Option<u32>,
    /// The eye of the camera last presented (world).
    camera_eye: Option<[f32; 3]>,
    /// Trigger regions of the loaded level packages, with their enter
    /// and exit lambdas' script addresses.
    regions: Vec<(crate::trigger::Region, Option<u32>, Option<u32>)>,
    /// The regions the player is in (indices into `regions`).
    player_regions: HashSet<usize>,
    /// Spawners whose objects are NPC processes (`Soldier` process
    /// type, or a character driven by the NPC state machine).
    npc_spawners: HashSet<u32>,
    /// NPCs that died; their spawners keep the killed flag.
    npc_dead: HashSet<u32>,
    /// Splines of the loaded levels by name `StringId`.
    splines: HashMap<u32, crate::nav::Spline>,
    /// Nav meshes of the loaded levels.
    navigation: crate::nav::Navigation,
    /// Cover points derived from the levels' corners.
    cover: Vec<crate::nav::Cover>,
    /// Bind-pose joint frames (object space) by object and joint name.
    joints: HashMap<u32, std::sync::Arc<HashMap<u32, glam::Mat4>>>,
    /// Named runtime light records and their last published scene state.
    lights: HashMap<u32, DynamicLight>,
    light_order: u64,
    /// The presented camera: eye, unit forward, up, vertical field of
    /// view (degrees) and aspect.
    camera_view: Option<([f32; 3], [f32; 3], [f32; 3], f32)>,
    /// Per-object `g_optionalInstanceParameters` set by scripts. The
    /// materials that read them (box cars, the pistol) do not yet apply
    /// them.
    instance_params: HashMap<u32, [[f32; 4]; 4]>,
    /// Particle spawns and kills the scripts requested this frame.
    particle_requests: Vec<crate::ss::ParticleRequest>,
    next_particle_handle: u32,
}

/// A script dynamic light (`add-dynamic-light`, `sub_7B998`).
#[derive(Clone, Debug)]
struct DynamicLight {
    id: u32,
    order: u64,
    locator: LightLocator,
    radius: f32,
    color: [f32; 3],
    gain: f32,
    /// Shape is chosen when adding the light. Update writes the cone
    /// angles without changing the native shape flag.
    spot: bool,
    cone: f32,
    enabled: bool,
    published: Option<(Transform, LightSpec)>,
}

fn light_input_is_finite(locator: LightLocator, radius: f32, cone: f32) -> bool {
    radius.is_finite()
        && cone.is_finite()
        && match locator {
            LightLocator::Frame(position, rotation) => position.into_iter().chain(rotation).all(f32::is_finite),
            LightLocator::Joint { .. } => true,
        }
}

/// Object-space bind frames of a skeleton's joints by name `StringId`.
fn joint_frames(s: &crate::skeleton::Skeleton) -> std::sync::Arc<HashMap<u32, glam::Mat4>> {
    std::sync::Arc::new(s.names.iter().zip(&s.inverse_bind).map(|(n, m)| (sid(n), glam::Mat4::from_cols_array(m).inverse())).collect())
}

struct CameraAnimation {
    name: u32,
    clip: u32,
    start: f64,
    blend: f32,
    stop: Option<f64>,
}

/// A camera clip's camera channel: frames per second and per frame the
/// optional field of view (degrees), rotation and translation.
struct CameraClip {
    fps: f32,
    samples: Vec<(Option<f32>, glam::Quat, glam::Vec3)>,
}

/// The camera channel of camera clips (`camera-train-wreck.pak`): per
/// frame `[fov°, ?, ?, quat, translation]` (components 7) or
/// `[quat, translation]` (components 6). No string with this `StringId`
/// is stored in the EBOOT or the packages.
const CAMERA_CHANNEL: u32 = 0x306c_3455;

/// A clip a script plays on an object (`animate`). The clip is found and
/// decoded by the adapter after the script tick; until then it reports
/// phase 0.
struct ScriptAnimation {
    clip: u32,
    options: crate::ss::Animate,
    start: f64,
    /// Seconds, once the clip is loaded.
    duration: Option<f32>,
    /// Library asset and clip name, once loaded.
    source: Option<(String, String)>,
    /// Clip-space root motion (`align` per frame at `fps`) and the world
    /// transform of clip space, when the clip has an `align` channel.
    root: Option<(Vec<(glam::Vec3, glam::Quat)>, f32, glam::Mat4)>,
    /// The clip could not be found or decoded: the animation ends at once.
    failed: bool,
}

/// Default blend time of script animations without a blend option.
const SCRIPT_BLEND: f32 = 0.2;

impl Stage {
    /// Whether trigger region `r` holds `object`: the player only in
    /// `player` regions; a parented region in its parent's live frame and
    /// never while the parent is not spawned.
    fn holds(&self, r: &crate::trigger::Region, object: u32) -> bool {
        let Some((at, _)) = World::locator(self, object) else { return false };
        let player = sid("player");
        if object == player && !r.categories.is_empty() && !r.categories.contains(&player) {
            return false;
        }
        self.holds_point(r, at)
    }

    /// Whether region `r` holds world point `at` (its category unchecked).
    fn holds_point(&self, r: &crate::trigger::Region, at: [f32; 3]) -> bool {
        let at = glam::Vec3::from_array(at);
        let local = match r.parent {
            Some(parent) => {
                if self.entities.contains_key(&parent) && !self.visible.contains(&parent) {
                    return false;
                }
                let Some((t, q)) = World::locator(self, parent) else { return false };
                glam::Quat::from_array(q).normalize().inverse() * (at - glam::Vec3::from_array(t))
            }
            None => at,
        };
        r.contains_local(local)
    }

    /// The transform carrying a spline's stored (level-frame) points with
    /// its parent: `parent now · parent spawner⁻¹`.
    fn spline_frame(&self, spline: &crate::nav::Spline) -> glam::Mat4 {
        let Some(parent) = spline.parent else { return glam::Mat4::IDENTITY };
        let (Some((t, q)), Some(s)) = (World::locator(self, parent), self.spawners.get(&parent)) else { return glam::Mat4::IDENTITY };
        let now = glam::Mat4::from_rotation_translation(glam::Quat::from_array(q).normalize(), glam::Vec3::from_array(t));
        let placed = glam::Mat4::from_rotation_translation(glam::Quat::from_array(s.rotation).normalize(), glam::Vec3::from_array(s.position));
        now * placed.inverse()
    }

    fn set_visible(&mut self, name: u32, visible: bool) -> bool {
        let Some(ids) = self.entities.get(&name) else { return false };
        for id in ids {
            self.ops.push(json!({"op": "set_visible", "id": id, "visible": visible}));
        }
        if visible { self.visible.insert(name) } else { self.visible.remove(&name) }
    }
    fn light_frame(&self, locator: LightLocator) -> Option<([f32; 3], [f32; 4])> {
        match locator {
            LightLocator::Frame(position, rotation) => Some((position, rotation)),
            LightLocator::Joint { object, joint } => self.joint_locator(object, joint),
        }
    }

    /// sub_7AFB8 scans the active list newest first: recycle the first
    /// light behind the camera, otherwise the one farthest from the eye.
    fn evict_dynamic_light(&mut self) -> Option<u32> {
        let (eye, forward, _, _) = self.camera_view?;
        let eye = glam::Vec3::from_array(eye);
        let forward = glam::Vec3::from_array(forward);
        let (name, _, _, _) = self.lights.iter().map(|(&name, light)| {
            let (behind, distance) = match self.light_frame(light.locator) {
                Some((position, _)) => {
                    let delta = glam::Vec3::from_array(position) - eye;
                    (delta.dot(forward) < 0., delta.length_squared())
                }
                None => (true, 0.),
            };
            (name, behind, distance, light.order)
        }).max_by(|(_, behind_a, distance_a, order_a), (_, behind_b, distance_b, order_b)| {
            behind_a.cmp(behind_b).then_with(|| {
                if *behind_a { order_a.cmp(order_b) } else { distance_a.total_cmp(distance_b).then(order_a.cmp(order_b)) }
            })
        })?;
        let id = self.lights[&name].id;
        self.remove_dynamic_light(name);
        Some(id)
    }

    fn publish_lights(&mut self) {
        let mut lights = std::mem::take(&mut self.lights);
        for (&name, light) in &mut lights {
            let Some((position, rotation)) = self.light_frame(light.locator)
                .filter(|(position, rotation)| position.iter().chain(rotation).all(|value| value.is_finite()))
            else {
                // The native renderer retires a light when the attached
                // object's handle has gone away.
                if light.published.take().is_some() {
                    self.ops.push(json!({"op": "despawn", "id": light.id}));
                }
                continue;
            };
            // Locators look along local +Z; the shared spot emits along -Z.
            let rotation = glam::Quat::from_array(rotation);
            let length = rotation.length_squared();
            let orientation = if length.is_finite() && length > f32::MIN_POSITIVE {
                rotation.normalize()
            } else {
                glam::Quat::IDENTITY
            }
                * glam::Quat::from_rotation_y(std::f32::consts::PI);
            let (x, y, z) = orientation.to_euler(glam::EulerRot::XYZ);
            let transform = Transform {
                translation: position,
                rotation_degrees: [x.to_degrees(), y.to_degrees(), z.to_degrees()],
                ..Default::default()
            };
            let outer = (light.cone * 0.5).clamp(0.1, 89.);
            let shape = if light.spot {
                LightShape::Spot { inner_degrees: outer * 0.9, outer_degrees: outer }
            } else {
                LightShape::Point
            };
            let spec = LightSpec {
                shape,
                color: light.color.map(|value| value.clamp(0., 1000.)),
                intensity: light.gain.clamp(0., 1e9),
                range: light.radius.clamp(0.01, 100_000.),
                falloff: Falloff::Exponent { exponent: 2.0 },
                enabled: light.enabled && light.radius > 0. && (!light.spot || (0. < light.cone && light.cone < 180.)),
                ..Default::default()
            };
            let id = light.id;
            if let Some((last_transform, last_spec)) = &light.published {
                if last_transform != &transform {
                    self.ops.push(json!({"op": "set_transform", "id": id, "transform": transform}));
                }
                if last_spec != &spec {
                    self.ops.push(json!({"op": "set_light", "id": id, "light": spec}));
                }
            } else {
                self.ops.push(json!({"op": "spawn", "entity": {"id": id, "name": format!("dynamic-light:{name:08x}"), "transform": transform, "light": spec}}));
            }
            light.published = Some((transform, spec));
        }
        lights.retain(|_, light| light.published.is_some());
        self.lights = lights;
    }

    fn log(&mut self, line: String) {
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(line);
    }
}

impl World for Stage {
    fn now(&self) -> f64 {
        self.now
    }
    fn print(&mut self, text: &str) {
        self.log(format!("print: {text}"));
    }
    fn level_spawned(&self, level: u32) -> bool {
        self.levels.contains(&level)
    }
    fn object_spawned(&self, name: u32) -> bool {
        self.visible.contains(&name)
    }
    fn spawn(&mut self, name: u32) -> bool {
        self.set_visible(name, true);
        self.spawners.contains_key(&name)
    }
    fn kill(&mut self, name: u32) -> bool {
        self.set_visible(name, false)
    }
    fn npc(&self, name: u32) -> Option<bool> {
        (self.npc_spawners.contains(&name) && self.visible.contains(&name)).then(|| self.npc_dead.contains(&name))
    }
    fn npc_killed(&self, name: u32) -> bool {
        self.npc_dead.contains(&name)
    }
    fn npc_die(&mut self, name: u32) -> bool {
        self.npc(name).is_some() && self.npc_dead.insert(name)
    }
    fn npc_alive_count(&self) -> u32 {
        self.npc_spawners.iter().filter(|n| self.npc(**n) == Some(false)).count() as u32
    }
    fn joint_locator(&self, object: u32, joint: u32) -> Option<([f32; 3], [f32; 4])> {
        let local = *self.joints.get(&object)?.get(&joint)?;
        let (t, q) = World::locator(self, object)?;
        let m = glam::Mat4::from_rotation_translation(glam::Quat::from_array(q).normalize(), glam::Vec3::from_array(t)) * local;
        let (_, r, p) = m.to_scale_rotation_translation();
        Some((p.to_array(), r.normalize().to_array()))
    }
    fn add_dynamic_light(&mut self, name: u32, locator: LightLocator, radius: f32, cone: f32, enabled: bool) {
        if !light_input_is_finite(locator, radius, cone) {
            return;
        }
        if let Some(light) = self.lights.get_mut(&name) {
            // Direct add leaves the attachment flag of a reused record
            // intact. update-dynamic-light explicitly clears it.
            if !matches!(light.locator, LightLocator::Joint { .. }) || matches!(locator, LightLocator::Joint { .. }) {
                light.locator = locator;
            }
            light.radius = radius;
            light.color = [1.; 3];
            light.spot = cone != 0.;
            light.cone = cone;
            light.enabled = enabled;
            return;
        }
        let id = DYNAMIC_LIGHT_IDS.into_iter().find(|id| self.lights.values().all(|light| light.id != *id))
            .or_else(|| self.evict_dynamic_light());
        if let Some(id) = id {
            self.light_order += 1;
            self.lights.insert(name, DynamicLight {
                id, order: self.light_order, locator, radius, color: [1.; 3], gain: 1., spot: cone != 0., cone, enabled, published: None,
            });
        } else {
            self.log(format!("dynamic-light {name:08x}: light pool full"));
        }
    }
    fn update_dynamic_light(&mut self, name: u32, locator: LightLocator, radius: f32, cone: f32) {
        if !light_input_is_finite(locator, radius, cone) {
            return;
        }
        if let Some(light) = self.lights.get_mut(&name) {
            light.locator = locator;
            light.radius = radius;
            light.color = [1.; 3];
            light.cone = cone;
        }
    }
    fn dynamic_light_color(&mut self, name: u32, color: [f32; 3], gain: f32) -> bool {
        if !gain.is_finite() || !color.iter().all(|value| value.is_finite()) {
            return false;
        }
        let Some(light) = self.lights.get_mut(&name) else { return false };
        light.color = color;
        light.gain = gain;
        true
    }
    fn set_instance_param(&mut self, object: u32, vector: usize, component: usize, value: f32) -> bool {
        if !self.entities.contains_key(&object) || vector > 3 || component > 3 {
            return false;
        }
        self.instance_params.entry(object).or_default()[vector][component] = value;
        true
    }
    fn set_instance_vector(&mut self, object: u32, vector: usize, value: [f32; 4]) -> bool {
        if !self.entities.contains_key(&object) || vector > 3 {
            return false;
        }
        self.instance_params.entry(object).or_default()[vector] = value;
        true
    }
    fn on_screen(&self, point: [f32; 3]) -> bool {
        let Some((eye, forward, up, fov)) = self.camera_view else { return false };
        let (f, u) = (glam::Vec3::from_array(forward).normalize_or(glam::Vec3::Z), glam::Vec3::from_array(up));
        let r = f.cross(u).normalize_or(glam::Vec3::X);
        let u = r.cross(f);
        let d = glam::Vec3::from_array(point) - glam::Vec3::from_array(eye);
        let z = d.dot(f);
        if z <= 0.01 {
            return false;
        }
        let t = (fov.to_radians() * 0.5).tan();
        let aspect = SCREEN[0] as f32 / SCREEN[1] as f32;
        (d.dot(r) / z).abs() <= t * aspect && (d.dot(u) / z).abs() <= t
    }
    fn set_water_offset(&mut self, name: u32, offset: [f32; 3]) -> bool {
        let Some(s) = self.spawners.get(&name) else { return false };
        let q = glam::Quat::from_array(s.rotation).normalize();
        let t = glam::Vec3::from_array(s.position) + glam::Vec3::from_array(offset);
        self.poses.insert(name, glam::Mat4::from_rotation_translation(q, t));
        let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
        for e in self.entities.get(&name).into_iter().flatten() {
            self.ops.push(json!({"op": "set_transform", "id": e, "transform": {"translation": t.to_array(), "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1.0, 1.0, 1.0]}}));
        }
        true
    }
    fn spawn_particles(&mut self, group: u32, position: [f32; 3], rotation: [f32; 4]) -> u32 {
        self.next_particle_handle += 1;
        // Handles live in their own range, apart from name `StringId`s
        // only by convention.
        let handle = 0x5000_0000 | self.next_particle_handle;
        self.particle_requests.push(crate::ss::ParticleRequest::Spawn { handle, group, position, rotation });
        handle
    }
    fn kill_particles(&mut self, handle: u32) {
        self.particle_requests.push(crate::ss::ParticleRequest::Kill { handle });
    }
    fn remove_dynamic_light(&mut self, name: u32) {
        if let Some(light) = self.lights.remove(&name).filter(|light| light.published.is_some()) {
            self.ops.push(json!({"op": "despawn", "id": light.id}));
        }
    }
    fn spline_at(&self, spline: u32, distance: f32) -> Option<([f32; 3], [f32; 3])> {
        let s = self.splines.get(&spline)?;
        let (p, t) = s.at(distance);
        let m = self.spline_frame(s);
        Some((m.transform_point3(p).to_array(), m.transform_vector3(t).to_array()))
    }
    fn spline_distance(&self, spline: u32, point: [f32; 3]) -> Option<f32> {
        let s = self.splines.get(&spline)?;
        Some(s.closest_distance(self.spline_frame(s).inverse().transform_point3(glam::Vec3::from_array(point))))
    }
    fn locator(&self, name: u32) -> Option<([f32; 3], [f32; 4])> {
        if name == sid("player") {
            return self.player_locator;
        }
        if let Some(m) = self.poses.get(&name) {
            let (_, r, t) = m.to_scale_rotation_translation();
            return Some((t.to_array(), r.to_array()));
        }
        self.spawners.get(&name).map(|s| (s.position, s.rotation))
    }
    fn property(&self, spawner: u32, prop: u32) -> Option<Property> {
        self.spawners.get(&spawner)?.properties.iter().find(|(k, _)| *k == prop).map(|(_, v)| v.clone())
    }
    fn fade(&mut self, color: [f32; 3], alpha: f32, seconds: f32) {
        self.fade_color = color.map(|c| (c.clamp(0., 1.) * 255.) as u8);
        self.fade_target = alpha.clamp(0., 1.);
        if seconds <= 0. {
            self.fade = self.fade_target;
            self.fade_rate = 0.;
        } else {
            self.fade_rate = 1. / seconds;
        }
    }
    fn camera(&mut self, name: u32, _seconds: f32) {
        self.camera = Some(name);
        self.log(format!("camera {name:08x}{}", if self.spawners.contains_key(&name) { "" } else { " (no spawner)" }));
    }
    fn camera_disable(&mut self, name: Option<u32>) {
        self.log(format!("camera release {:?}", name.map(|n| format!("{n:08x}"))));
        if name.is_none() || name == self.camera {
            self.camera = None;
        }
    }
    fn set_visible(&mut self, object: u32, visible: bool) -> bool {
        Stage::set_visible(self, object, visible)
    }
    fn in_region(&self, object: u32, region: u32) -> bool {
        self.regions.iter().any(|(r, _, _)| r.id == region && self.holds(r, object))
    }
    fn object_on_object(&self, object: u32, support: u32) -> bool {
        object == sid("player") && self.player_support.is_some_and(|body| self.entities.get(&support).is_some_and(|ids| ids.contains(&body)))
    }
    fn camera_animation(&mut self, name: u32, clip: u32, seconds: f32) {
        if let Some(c) = self.camera_animations.iter_mut().find(|c| c.name == name) {
            c.stop = None;
        } else if self.camera_animations.len() < 10 {
            self.log(format!("camera animation {name:08x}"));
            self.camera_animations.push(CameraAnimation { name, clip, start: self.now, blend: seconds.max(0.), stop: None });
        }
    }
    fn camera_animation_disable(&mut self, name: Option<u32>) {
        let now = self.now;
        for c in self.camera_animations.iter_mut().filter(|c| name.is_none_or(|n| n == c.name)) {
            c.stop.get_or_insert(now);
        }
    }
    fn player_controls(&mut self, enabled: bool) {
        self.controls = Some(enabled);
    }
    fn sticks(&self, pad: u32) -> [f32; 4] {
        if pad == 0 { [self.stick[0], self.stick[1], 0., 0.] } else { [0.; 4] }
    }
    fn animate(&mut self, object: u32, clip: u32, options: &crate::ss::Animate) -> bool {
        if object != sid("player") && !self.entities.contains_key(&object) {
            return false;
        }
        self.held.remove(&object);
        self.animations.insert(object, ScriptAnimation { clip, options: options.clone(), start: self.now, duration: None, source: None, root: None, failed: false });
        true
    }
    fn animation_phase(&self, object: u32) -> Option<f32> {
        let a = self.animations.get(&object)?;
        if a.failed {
            return None;
        }
        match a.duration {
            None => Some(0.),
            Some(d) => {
                let t = (self.now - a.start) as f32;
                (t < d).then(|| if d > 0. { t / d } else { 1. })
            }
        }
    }
}

/// A selected task subnode with its levels.
struct TaskPlan {
    name: String,
    level: String,
    continue_player: Option<String>,
    /// Allies' continue spawners (`continue-flynn-steam-vault-1`) and the
    /// camera's.
    continue_allies: Vec<String>,
    continue_camera: Option<String>,
    /// Level names whose packages load (subnode level, caches, world,
    /// then the displayed levels of the world's load regions).
    levels: Vec<String>,
    /// Load regions evaluated: `(region, scheme, displayed, not displayed)`.
    regions: Vec<(String, u32, Vec<String>, Vec<String>)>,
    /// `TaskManagerMainMenu`: no player; the menu camera looks at
    /// `menu-temple`.
    menu: bool,
}

/// The mounted disc data and the presented level.
struct Session {
    mount: ArchiveMount,
    /// Level names of `pak22.txt` in native order, with their packages.
    levels: Vec<(String, Vec<String>)>,
    all_levels: std::collections::BTreeMap<String, crate::Level>,
    current: usize,
    camera: Camera,
    last_ms: u64,
    start_ms: u64,
    triangles: usize,
    packages: Vec<String>,
    /// Spawned entities and their uploaded models.
    entities: Vec<(u32, VfsPath)>,
    next_model: u32,
    /// Packages of the current level still to load, one per tick (the host
    /// bounds generated model bytes per tick).
    pending: std::collections::VecDeque<Load>,
    /// `actor27.psarc`, for spawned art groups.
    actors: ArchiveMount,
    bounds: Option<([f32; 3], [f32; 3])>,
    start: Option<scene::StartCamera>,
    /// Whether a level light table has set the environment.
    lit: bool,
    // Task mode.
    modules: Option<ArchiveMount>,
    game: Option<Game>,
    /// Task subnode names in task order.
    tasks: Vec<String>,
    plan: Option<TaskPlan>,
    stage: Stage,
    records: Vec<SpawnerRecord>,
    ground: Ground,
    player: Option<Player>,
    started: bool,
    /// Baked light probes of the loaded levels.
    probes: crate::probes::ProbeField,
    /// The level light table's directional light.
    light: Option<scene::LevelLight>,
    script_error: Option<String>,
    /// End time (stage seconds) of the playing task movie, and whether a
    /// world video and its layer present it.
    movie: Option<f64>,
    movie_video: bool,
    ui: Option<Ui>,
    /// The `text1.psarc` common catalog of the configured language.
    catalog: Option<Vec<u8>>,
    /// Developer free camera in task mode (F1), detached from the player.
    free: bool,
    /// Camera clips by name (`None`: not found in the level's packages).
    camera_clips: HashMap<u32, Option<CameraClip>>,
    /// The level's sky (`sky-…` package) as an uploaded environment image.
    sky: Option<VfsPath>,
    /// Front-end pages of `menu.bin` and the title logo (`menu.pak`
    /// `logo2` with its `logo2-alpha` mask) as an uploaded image.
    menu_pages: Vec<crate::menu::Page>,
    logo: Option<(VfsPath, u32, u32)>,
    /// The player rig (`actor27.psarc/proto.pak` skeleton and the clips the
    /// player uses), loaded with the first skinned model that has no
    /// skeleton of its own; `Some(None)` when unavailable.
    player_rig: Option<Option<(crate::skeleton::Skeleton, Vec<crate::anim::Clip>)>>,
    /// `anim-player.bin` states and blend trees.
    anim_table: Option<crate::anim_state::StateTable>,
    anim_graph: Option<crate::anim_tree::Graph>,
    /// The player rig's clips by name `StringId`.
    player_clips: HashMap<u32, crate::player::ClipInfo>,
    /// `spawn-character.bin` package lists, and the NPC rigs loaded from
    /// them by skeleton package.
    characters: Vec<crate::characters::Character>,
    npc_rigs: HashMap<String, Option<(crate::skeleton::Skeleton, Vec<crate::anim::Clip>)>>,
    /// `anim-npc.bin` states and trees, `anim-sets.bin`, the NPCs they
    /// drive and each NPC rig's clips by name `StringId`.
    npc_anim: Option<(crate::anim_tree::Graph, crate::anim_state::StateTable)>,
    anim_sets: crate::anim_sets::AnimSets,
    npcs: Vec<Npc>,
    npc_clips: HashMap<String, HashMap<u32, crate::player::ClipInfo>>,
    /// Each NPC rig's packages (skeleton base, animation packages).
    npc_packages: HashMap<String, Vec<String>>,
    /// Effect groups and descriptors of the loaded particle modules.
    particles: crate::particle::Library,
    part_modules: HashSet<u32>,
    /// `particle.pak` materials, read on first use.
    particle_materials: Option<HashMap<u32, crate::particle::ParticleMaterial>>,
    /// Uploaded particle textures by texture ID: path and atlas columns.
    particle_textures: HashMap<u32, Option<(String, u32)>>,
    /// Particle effects of the presented level; the nearest to the camera
    /// are live entities.
    effects: Vec<Effect>,
    /// The rig each spawned entity is skinned to.
    entity_rigs: HashMap<u32, RigRef>,
    /// Skeletons of skinned art groups that carry their own.
    own_rigs: HashMap<String, crate::skeleton::Skeleton>,
    /// Animation packages read for script animations.
    anim_packages: HashMap<String, Arc<crate::mesh::Loaded>>,
    /// Skeletons by `JOINT_HIERARCHY` hash, for clips authored on another
    /// rig than the entity's (`None`: package without one).
    skeletons: HashMap<String, Option<crate::skeleton::Skeleton>>,
    /// Each rig's controller clips as one animation library, uploaded once
    /// per session (models carry only joints and skins), by `RigRef` key.
    rig_libraries: HashMap<String, String>,
    /// Uploaded single-clip libraries by (rig, clip).
    libraries: HashMap<(String, u32), Option<LibraryClip>>,
}

/// A clip uploaded as a single-clip animation library.
#[derive(Clone, Debug)]
struct LibraryClip {
    asset: String,
    name: String,
    seconds: f32,
    fps: f32,
    align: Vec<(glam::Vec3, glam::Quat)>,
    ap_reference: Vec<(glam::Vec3, glam::Quat)>,
}

/// The skeleton an entity is skinned to.
#[derive(Clone, Debug, PartialEq)]
enum RigRef {
    Player,
    /// An NPC rig key (`flynn-base:[…]`).
    Npc(String),
    /// An art group's own `JOINT_HIERARCHY`.
    Own(String),
}

/// A particle emitter placed by a spawner: its entity (spawned while
/// near the camera) and world position.
struct Effect {
    id: u32,
    spawner: u32,
    at: glam::Vec3,
    entity: serde_json::Value,
    live: bool,
}

/// Live particle systems at once (the shared VFX budget is 128) and the
/// distance beyond which effects sleep. The native particle pass culls
/// spawners per frame (`sub_76BC28`); its criteria are not decoded.
const MAX_EFFECTS: usize = 96;
const EFFECT_RANGE: f32 = 80.;
/// Particle modules resident in game (the effect groups of 
/// `train-wreck`'s fire spawners live in `particle-u2-gameparts`; the
/// native loader of these modules is not located).
const GLOBAL_PART_MODULES: &[&str] = &["particle-u2-base", "particle-u2-gameparts"];

/// An NPC entity presented by the `anim-npc` state machine.
struct Npc {
    entity: u32,
    animator: crate::player::Animator,
    sets: Vec<u32>,
    rig: String,
}

/// NPC states presented without AI: the ambient idle (the state the
/// demeanour and move natives would leave is not modelled).
const NPC_STATES: &[&str] = &["s_ambient-idle"];

/// Packages holding the player rig's clips: `proto.pak` and the hurt
/// movement types of `anim-proto-hurt.pak`.
/// Follow-camera distance behind the player's head, metres.
const FOLLOW_DISTANCE: f32 = 4.;

const PLAYER_CLIP_PACKAGES: &[&str] = &["proto.pak", "anim-proto-hurt.pak"];

/// The world environment: the level's directional light and ambient term
/// when known. Native intensities (0.9–19 on the installed levels) are
/// clamped to the renderer's sun range; the mapping is not calibrated
/// against the original renderer (SATD-nd-retail-runtime).
fn environment(light: Option<&scene::LevelLight>, sky: Option<&VfsPath>) -> serde_json::Value {
    let mut env = match light {
        None => json!({"op": "set_environment", "environment": {"show_sky": true, "intensity": 1.0, "ambient": [0.55, 0.55, 0.55], "sun": {"direction": [0.3, -0.8, -0.4], "color": [1.0, 0.97, 0.9], "intensity": 0.6, "shadow_strength": 0.0}}}),
        Some(l) => {
            let ambient = l.ambient.map_or([0.45; 3], |a| a.map(|v| (v * 1.6).clamp(0.05, 1.5)));
            json!({"op": "set_environment", "environment": {"show_sky": true, "intensity": 1.0, "ambient": ambient, "sun": {"direction": l.direction, "color": l.color.map(|c| c.clamp(0., 4.)), "intensity": l.intensity.clamp(0.2, 4.0), "shadow_strength": 0.6}}})
        }
    };
    if let Some(sky) = sky {
        env["environment"]["hdr"] = json!(sky.relative());
    }
    env
}

fn yaw_of(q: [f32; 4]) -> f32 {
    let f = glam::Quat::from_array(q).normalize() * glam::Vec3::Z;
    f.x.atan2(f.z)
}

/// The duration of an MPEG-2 video elementary stream: picture start codes
/// over the sequence header's frame rate.
fn movie_seconds(reader: &dyn AssetReader) -> Result<f64> {
    let len = reader.len()?;
    let mut head = [0u8; 8];
    crysirror_resource::read_exact_at(reader, 0, &mut head)?;
    ensure!(head[..4] == [0, 0, 1, 0xb3], "not an MPEG video sequence");
    let fps = match head[7] & 15 {
        1 => 24000. / 1001.,
        2 => 24.,
        3 => 25.,
        4 => 30000. / 1001.,
        5 => 30.,
        6 => 50.,
        7 => 60000. / 1001.,
        8 => 60.,
        c => anyhow::bail!("MPEG frame rate code {c}"),
    };
    let (mut pictures, mut at, mut buf) = (0u64, 0u64, vec![0u8; 4 << 20]);
    while at < len {
        let n = ((len - at) as usize).min(buf.len());
        crysirror_resource::read_exact_at(reader, at, &mut buf[..n])?;
        pictures += buf[..n].windows(4).filter(|w| *w == [0, 0, 1, 0]).count() as u64;
        // Overlap three bytes so codes across chunks are seen once.
        at += if at + (n as u64) < len { n as u64 - 3 } else { n as u64 };
    }
    Ok(pictures as f64 / fps)
}

/// Boot-sequence screens drawn by the player.
enum Ui {
    /// `TaskManagerMessageDialog`: dismissed with Enter.
    Message(String),
    /// `TaskManagerMainMenu`: the `menu.bin` page stack (indices into
    /// `Session::menu_pages`) and the selected visible button.
    Menu { stack: Vec<usize>, selected: usize },
}

/// What a front-end button does in the player. Pages are entered by the
/// button names the EBOOT dispatches on; buttons without a player action
/// (multiplayer, saves) are drawn dimmed.
enum MenuAction {
    Page(&'static str),
    NewGame,
    None,
}

fn menu_action(button: &str) -> MenuAction {
    match button {
        "BUTTON_TITLE_START" => MenuAction::Page("PAGE_NEWPROFILE"),
        "BUTTON_NEWGAME" => MenuAction::Page("PAGE_NEWGAME"),
        "BUTTON_OPTIONS" => MenuAction::Page("PAGE_OPTIONS"),
        b if b.starts_with("BUTTON_NEWGAME_") => MenuAction::NewGame,
        _ => MenuAction::None,
    }
}

/// The task after `at` in the session's task graph.
fn game_next(session: &Session, at: (usize, usize)) -> Option<(usize, usize)> {
    session.game.as_ref()?.rt.inner.tasks.next(at)
}

impl Session {
    /// A text catalog entry by key (its `StringId`), or the key itself.
    /// Catalog text marks line breaks with `\`.
    fn text(&self, key: &str) -> String {
        self.catalog.as_deref().and_then(|b| crate::Strings::parse(b).ok()).and_then(|s| s.get(sid(key)).map(|t| t.replace('\\', "\n"))).unwrap_or_else(|| key.to_owned())
    }
}

pub struct NaughtyDogVm {
    client: ClientId,
    config: Config,
    state: Option<Session>,
}

impl NaughtyDogVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self { client, config, state: None })
    }

    fn boot(&mut self, host: &mut dyn HostServices) -> Result<Session> {
        let storage = host.storage(self.client)?;
        let open = |p: &str| -> Result<Arc<dyn AssetReader>> { Ok(storage.open_asset(&VfsPath::new(Mount::Asset, &format!("{MAIN}/{p}"))?)?) };
        let manifest = read_all(open("pak22.txt")?.as_ref(), 4 << 20)?;
        let text = std::str::from_utf8(&manifest)?;
        let all = levels(text)?;
        // Native order: the order of `level-name` directives.
        // A level presents its own packages followed by those of the levels
        // it names with `cache` (for example the menu's train-wreck views).
        let level_list: Vec<(String, Vec<String>)> = text
            .lines()
            .filter_map(|l| l.strip_prefix("level-name "))
            .filter_map(|n| {
                let level = all.get(n)?;
                let mut packages = level.packages.clone();
                for cache in &level.caches {
                    for p in all.get(cache).map(|l| l.packages.as_slice()).unwrap_or_default() {
                        if !packages.contains(p) {
                            packages.push(p.clone());
                        }
                    }
                }
                Some((n.to_owned(), packages))
            })
            .collect();
        let mut registry = FormatRegistry::default();
        crysirror_formats_arc::register(&mut registry)?;
        let mut context = DecodeContext::new(self.client);
        context.limits.max_entry_size = PACKAGE_LIMIT;
        let (_, archive) = registry.open(open("pak22.psarc")?, Some("sony/psarc"), &context)?;
        let mount = ArchiveMount::new(archive, context.clone())?;
        let (_, actor_archive) = registry.open(open("actor27.psarc")?, Some("sony/psarc"), &context)?;
        let actors = ArchiveMount::new(actor_archive, context.clone())?;
        let camera = Camera { eye: [0.; 3], yaw: 0., pitch: 0., speed: 5. };
        let now = host.now_ms();
        let mut session = Session {
            mount,
            levels: level_list,
            all_levels: all,
            current: 0,
            camera,
            last_ms: now,
            start_ms: now,
            triangles: 0,
            packages: Vec::new(),
            entities: Vec::new(),
            next_model: 0,
            pending: Default::default(),
            actors,
            bounds: None,
            start: None,
            lit: false,
            modules: None,
            game: None,
            tasks: Vec::new(),
            plan: None,
            stage: Stage::default(),
            records: Vec::new(),
            ground: Ground::default(),
            player: None,
            started: false,
            probes: Default::default(),
            light: None,
            script_error: None,
            movie: None,
            movie_video: false,
            ui: None,
            catalog: None,
            free: false,
            camera_clips: HashMap::new(),
            sky: None,
            menu_pages: Vec::new(),
            logo: None,
            player_rig: None,
            anim_table: None,
            anim_graph: None,
            player_clips: HashMap::new(),
            characters: Vec::new(),
            npc_rigs: HashMap::new(),
            npc_anim: None,
            anim_sets: Default::default(),
            npcs: Vec::new(),
            npc_clips: HashMap::new(),
            npc_packages: HashMap::new(),
            particles: Default::default(),
            part_modules: HashSet::new(),
            particle_materials: None,
            particle_textures: HashMap::new(),
            effects: Vec::new(),
            entity_rigs: HashMap::new(),
            own_rigs: HashMap::new(),
            anim_packages: HashMap::new(),
            skeletons: HashMap::new(),
            rig_libraries: HashMap::new(),
            libraries: HashMap::new(),
        };
        if self.config.viewer() {
            let level = self.config.level.as_deref().unwrap_or("menu-temple");
            session.current = session.levels.iter().position(|(n, _)| n == level).with_context(|| format!("unknown Naughty Dog level {level}"))?;
        } else {
            let (_, text) = registry.open(open("text1.psarc")?, Some("sony/psarc"), &context)?;
            let text = ArchiveMount::new(text, context.clone())?;
            let catalog = format!("{}.common", self.config.language.unwrap_or(2));
            session.catalog = text.open(&catalog).ok().map(|r| read_all(r.as_ref(), 16 << 20)).transpose()?;
            let (_, dc) = registry.open(open("dc1.psarc")?, Some("sony/psarc"), &context)?;
            let modules = ArchiveMount::new(dc, context)?;
            let game = Game::boot(&modules, &mut session.stage)?;
            session.characters = crate::game::ModuleSource::read(&modules, "spawn-character").and_then(|b| crate::characters::parse(&b)).unwrap_or_default();
            match crate::game::ModuleSource::read(&modules, "anim-npc").and_then(|b| {
                let graph = crate::anim_tree::Graph::load("anim-npc", &b)?;
                Ok((crate::anim_state::parse_module(&graph.dc, graph.module)?, graph))
            }) {
                Ok((table, graph)) => session.npc_anim = Some((graph, table)),
                Err(e) => session.stage.log(format!("anim-npc.bin: {e:#}")),
            }
            session.anim_sets = crate::game::ModuleSource::read(&modules, "anim-sets").and_then(|b| crate::anim_sets::AnimSets::parse(&b)).unwrap_or_else(|e| {
                session.stage.log(format!("anim-sets.bin: {e:#}"));
                Default::default()
            });
            match crate::game::ModuleSource::read(&modules, "anim-player").and_then(|b| {
                let graph = crate::anim_tree::Graph::new(&b)?;
                Ok((crate::anim_state::parse_loaded(&graph.dc)?, graph))
            }) {
                Ok((table, graph)) => {
                    session.anim_table = Some(table);
                    session.anim_graph = Some(graph);
                }
                Err(e) => session.stage.log(format!("anim-player.bin: {e:#}")),
            }
            session.menu_pages = crate::game::ModuleSource::read(&modules, "menu").and_then(|b| crate::menu::pages(&b)).unwrap_or_else(|e| {
                session.stage.log(format!("menu.bin: {e:#}"));
                Vec::new()
            });
            session.logo = match Self::menu_logo(host, self.client, &session.mount) {
                Ok(logo) => logo,
                Err(e) => {
                    session.stage.log(format!("menu logo: {e:#}"));
                    None
                }
            };
            let g = &game.rt.inner.tasks;
            session.tasks = g.order.iter().filter_map(|id| g.node(*id)).flat_map(|n| n.order.iter().filter_map(|s| n.subnodes.iter().find(|x| x.id == *s)).map(|s| s.name.clone())).collect();
            let task = self.config.task.clone().or_else(|| game.default_task()).context("the task graph has no story task")?;
            session.current = session.tasks.iter().position(|t| *t == task).with_context(|| format!("unknown Naughty Dog task {task}"))?;
            session.modules = Some(modules);
            session.game = Some(game);
        }
        self.begin(host, &mut session)?;
        Ok(session)
    }

    /// Select the task subnode `session.current` and its levels.
    fn plan(session: &mut Session) -> Result<TaskPlan> {
        let game = session.game.as_mut().context("task mode")?;
        let name = session.tasks[session.current].clone();
        let (node, sub) = game.select(&name)?;
        let n = &game.rt.inner.tasks.nodes[node];
        let s = &n.subnodes[sub];
        // A subnode without a level (most movies) loads nothing.
        if s.level.is_empty() {
            return Ok(TaskPlan { name, level: String::new(), continue_player: None, continue_allies: Vec::new(), continue_camera: None, levels: Vec::new(), regions: Vec::new(), menu: false });
        }
        // The EBOOT's main-menu manager (`sub_98478`) loads `menu-temple`
        // and views it through a fixed camera (`sub_98634`,
        // `CameraControlFixed`); the subnode's own level `menu` only holds
        // the localized logo board drawn as 2D art. `menu-temple`'s `cache`
        // list preloads the first story level and is not displayed.
        if matches!(s.manager, crate::task::Manager::MainMenu) {
            return Ok(TaskPlan { name, level: MENU_LEVEL.into(), continue_player: None, continue_allies: Vec::new(), continue_camera: None, levels: vec![MENU_LEVEL.into()], regions: Vec::new(), menu: true });
        }
        let mut levels = vec![s.level.clone()];
        for c in session.all_levels.get(&s.level).map(|l| l.caches.as_slice()).unwrap_or_default() {
            if !levels.contains(c) {
                levels.push(c.clone());
            }
        }
        if !n.world.is_empty() && !levels.contains(&n.world) {
            levels.push(n.world.clone());
        }
        Ok(TaskPlan { name, level: s.level.clone(), continue_player: s.continue_player.clone(), continue_allies: s.continue_allies.clone(), continue_camera: s.continue_camera.clone(), levels, regions: Vec::new(), menu: false })
    }

    /// Unload the presented level and queue the packages of the current
    /// level (viewer) or task subnode.
    fn begin(&self, host: &mut dyn HostServices, session: &mut Session) -> Result<()> {
        let mut world = vec![environment(None, None)];
        for (id, asset) in session.entities.drain(..) {
            world.push(json!({"op": "despawn", "id": id}));
            host.release_model(self.client, &asset)?;
        }
        for e in session.effects.drain(..) {
            if e.live {
                world.push(json!({"op": "despawn", "id": e.id}));
            }
        }
        session.triangles = 0;
        session.packages.clear();
        session.bounds = None;
        session.start = None;
        session.lit = false;
        session.records.clear();
        session.ground = Ground::default();
        session.player = None;
        session.started = false;
        session.probes = Default::default();
        session.light = None;
        if let Some(sky) = session.sky.take() {
            host.release_model(self.client, &sky)?;
        }
        session.script_error = None;
        session.movie = None;
        // A task left during its movie stops the video and its layer.
        let mut layers = vec![json!({"op": "put", "layer": {"id": FADE_LAYER, "rect": [0, 0, SCREEN[0], SCREEN[1]], "paint": {"kind": "solid", "color": [0, 0, 0, 0]}}})];
        if std::mem::take(&mut session.movie_video) {
            world.push(json!({"op": "set_video", "name": "movie", "video": null}));
            layers.push(json!({"op": "remove", "id": MOVIE_LAYER}));
        }
        session.ui = None;
        session.stage.spawners.clear();
        session.stage.npc_spawners.clear();
        session.stage.npc_dead.clear();
        session.stage.splines.clear();
        for (_, light) in session.stage.lights.drain() {
            if light.published.is_some() {
                world.push(json!({"op": "despawn", "id": light.id}));
            }
        }
        session.stage.joints.retain(|k, _| *k == sid("player"));
        session.stage.navigation.clear();
        session.stage.cover.clear();
        session.stage.entities.clear();
        session.stage.instance_params.clear();
        session.npcs.clear();
        session.stage.animations.clear();
        session.stage.held.clear();
        session.stage.camera = None;
        session.stage.camera_animations.clear();
        session.stage.poses.clear();
        session.stage.regions.clear();
        session.stage.player_regions.clear();
        session.entity_rigs.clear();
        session.stage.visible.clear();
        session.stage.ops.clear();
        session.stage.fade = 0.;
        session.stage.fade_target = 0.;
        if session.game.is_some() {
            // A task restarts the script runtime from the booted modules.
            if let (Some(modules), Some(game)) = (&session.modules, &mut session.game) {
                if !game.rt.inner.procs.is_empty() {
                    *game = Game::boot(modules, &mut session.stage)?;
                }
            }
            let plan = Self::plan(session)?;
            let mut packages = Vec::new();
            for l in &plan.levels {
                for p in session.all_levels.get(l).map(|l| l.packages.as_slice()).unwrap_or_default() {
                    if !packages.contains(p) {
                        packages.push(p.clone());
                    }
                }
            }
            session.stage.levels = plan.levels.iter().map(|l| sid(l)).collect();
            session.pending = packages.into_iter().map(Load::Package).collect();
            session.plan = Some(plan);
        } else {
            session.pending = session.levels[session.current].1.iter().cloned().map(Load::Package).collect();
        }
        host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": layers, "world": world}))?)?;
        Ok(())
    }

    /// Load and present the next queued package; frame the camera once the
    /// level's start camera is known or the last package is in.
    fn load_next(&self, host: &mut dyn HostServices, session: &mut Session) -> Result<()> {
        let Some(step) = session.pending.pop_front() else { return Ok(()) };
        let mut world = Vec::new();
        let package_name = match step {
            Load::Package(name) => name,
            Load::Actor(art, spawners) => {
                self.load_actor(host, session, &art, &spawners, &mut world)?;
                String::new()
            }
            Load::Particles(spawners) => {
                self.load_particles(host, session, &spawners, &mut world)?;
                String::new()
            }
            Load::Part(name, path, meshes) => {
                let package = session.mount.open(&path)?;
                let directory = Directory::read(package.as_ref())?;
                self.upload_meshes(host, session, &name, &path, meshes, package.as_ref(), &directory, &mut world)?;
                String::new()
            }
        };
        if let Some(path) = (!package_name.is_empty()).then(|| package_path(&session.mount, &package_name)).flatten() {
            // Read and relocate the package once; every item below parses
            // the same copy.
            let package: Arc<dyn AssetReader> = Arc::new(crysirror_core::service::MemoryAssetReader(read_all(session.mount.open(&path)?.as_ref(), PACKAGE_LIMIT)?.into()));
            let directory = Directory::read(package.as_ref())?;
            let loaded = crate::mesh::Loaded::read(package.as_ref(), PACKAGE_LIMIT)?;
            let meshes = scene::meshes_of(&loaded, &directory)?;
            if session.game.is_some() {
                if let Some(item) = directory.items.iter().find(|i| i.kind == "LIGHT_TABLE_1") {
                    session.probes.add(crate::probes::read(&loaded, item.payload_offset as usize)?);
                }
            }
            if !session.lit {
                if let Some(item) = directory.items.iter().find(|i| i.kind == "LIGHT_TABLE_1") {
                    if let Some(light) = scene::level_light(&loaded, item.payload_offset as usize)? {
                        world.push(environment(Some(&light), session.sky.as_ref()));
                        session.light = Some(light);
                        session.lit = true;
                    }
                }
            }
            // Spawned art groups follow the level packages.
            let mut groups: std::collections::BTreeMap<String, Vec<SpawnerRecord>> = Default::default();
            let mut emitters = Vec::new();
            for item in directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP") {
                for s in scene::spawner_records(&loaded, item.payload_offset as usize)? {
                    session.stage.spawners.insert(sid(&s.name), s.clone());
                    if s.process_type == sid("Soldier") {
                        session.stage.npc_spawners.insert(sid(&s.name));
                    }
                    if let Some(art) = s.art_group.clone() {
                        groups.entry(art).or_default().push(s.clone());
                    }
                    if s.process_type == sid("PartSpawner") {
                        emitters.push(s.clone());
                    }
                    session.records.push(s);
                }
            }
            for (art, list) in groups {
                session.pending.push_back(Load::Actor(art, list));
            }
            if !emitters.is_empty() {
                session.pending.push_back(Load::Particles(emitters));
            }
            if session.start.is_none() {
                if let Some(item) = directory.items.iter().find(|i| i.kind == "CAMERA_TABLE_1") {
                    if let Some(c) = scene::start_camera(&loaded, item.payload_offset as usize)? {
                        // The level's start camera (`CAMERA_TABLE_1`, metres).
                        session.camera = Camera { eye: c.position, yaw: c.forward[0].atan2(-c.forward[2]), pitch: c.forward[1].clamp(-1., 1.).asin(), speed: 5. };
                        if session.game.is_none() {
                            world.push(session.camera.op());
                        }
                        session.start = Some(c);
                    }
                }
            }
            if session.game.is_some() {
                // Native collision and climb edges where the package has
                // them; otherwise its render triangles.
                let native = crate::collision::level_object(&loaded).unwrap_or_else(|e| {
                    session.stage.log(format!("{package_name}: collision: {e:#}"));
                    crate::collision::Object::default()
                });
                if native.triangles.is_empty() {
                    session.ground.add(&meshes);
                } else {
                    session.ground.add_collision(&native.triangles);
                }
                session.ground.add_edges(&native.features.edges);
                session.stage.cover.extend(native.features.corners.iter().filter_map(crate::nav::cover));
                match crate::nav::nav_meshes(&loaded, &directory) {
                    Ok(meshes) => meshes.into_iter().for_each(|m| session.stage.navigation.add(m)),
                    Err(e) => session.stage.log(format!("{package_name}: nav meshes: {e:#}")),
                }
                match crate::nav::splines(&loaded, &directory) {
                    Ok(splines) => session.stage.splines.extend(splines.into_iter().map(|s| (s.id, s))),
                    Err(e) => session.stage.log(format!("{package_name}: splines: {e:#}")),
                }
                for item in directory.items.iter().filter(|i| i.kind == "REGION_TREE") {
                    match crate::trigger::regions(&loaded, item.payload_offset as usize) {
                        Ok(regions) => {
                            // Their enter/exit lambdas run in the script VM
                            // with the package mapped.
                            let base = match session.game.as_mut() {
                                Some(game) if regions.iter().any(|r| r.enter.is_some() || r.exit.is_some()) => game.rt.dc.map_package(sid(&package_name), &loaded.data, loaded.pointers()).ok(),
                                _ => None,
                            };
                            let at = |o: Option<usize>| base.zip(o).and_then(|(b, o)| u32::try_from(o).ok().map(|o| b + o));
                            session.stage.regions.extend(regions.into_iter().map(|r| {
                                let (enter, exit) = (at(r.enter), at(r.exit));
                                (r, enter, exit)
                            }));
                        }
                        Err(e) => session.stage.log(format!("{package_name}: trigger regions: {e:#}")),
                    }
                }
                if let Some(item) = directory.items.iter().find(|i| i.kind == "REGION_TREE") {
                    let regions = crate::region::load_regions(&loaded, item.payload_offset as usize)?;
                    if !regions.is_empty() {
                        self.apply_load_regions(session, &package_name, &loaded, &regions)?;
                    }
                }
            }
            let sky = if package_name.starts_with("sky-") { scene::sky_texture(&meshes, package.as_ref(), &directory)? } else { None };
            if let Some(image) = sky {
                // The sky is drawn behind everything at infinity, as the
                // environment image, not as a 50 m mesh around the origin.
                let rendered = crysirror_asset::sky::ImageSky { width: image.width, height: image.height, rgba: &image.rgba, u_offset: 0., zenith_span: std::f32::consts::FRAC_PI_2, scale: 1. }.render(2048, 1024)?;
                session.next_model += 1;
                let path = host.upload_hdr(self.client, 0x4e44_0000 + session.next_model, rendered.width, rendered.height, &rendered.rgb)?;
                if let Some(old) = session.sky.replace(path) {
                    host.release_model(self.client, &old)?;
                }
                if session.lit && session.game.is_none() {
                    world.push(environment(session.light.as_ref(), session.sky.as_ref()));
                }
            } else if let Some((l, h)) = scene::placed_bounds(&meshes) {
                session.bounds = Some(match session.bounds {
                    None => (l, h),
                    Some((a, b)) => (std::array::from_fn(|k| a[k].min(l[k])), std::array::from_fn(|k| b[k].max(h[k]))),
                });
                session.triangles += meshes.iter().flat_map(|m| &m.fragments).map(|f| f.triangles.len()).sum::<usize>();
                session.packages.push(package_name.clone());
                self.upload_meshes(host, session, &package_name, &path, meshes, package.as_ref(), &directory, &mut world)?;
            }
        }
        if session.pending.is_empty() {
            let (lo, hi) = session.bounds.unwrap_or(([-5.; 3], [5.; 3]));
            let (center, size): ([f32; 3], f32) = (std::array::from_fn(|k| (lo[k] + hi[k]) * 0.5), (0..3).map(|k| hi[k] - lo[k]).fold(0., f32::max));
            session.camera.speed = (size * 0.05).clamp(2., 50.);
            if session.game.is_some() {
                self.start_task(session, &mut world)?;
                host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": [], "world": std::mem::take(&mut world)}))?)?;
                self.enter_task(host, session)?;
            } else if session.start.is_none() {
                // South of the placed bounds, slightly above, facing them.
                session.camera = Camera { eye: [center[0], center[1] + size * 0.15, center[2] + size * 0.6], yaw: 0., pitch: -0.15, speed: session.camera.speed };
                world.push(session.camera.op());
            }
        }
        if !world.is_empty() {
            host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": [], "world": world}))?)?;
        }
        Ok(())
    }

    /// Upload a package's meshes as one model and spawn it; a model over
    /// the host limit is split into two halves queued as separate steps.
    #[allow(clippy::too_many_arguments)]
    fn upload_meshes(&self, host: &mut dyn HostServices, session: &mut Session, name: &str, path: &str, mut meshes: Vec<crate::mesh::Mesh>, package: &dyn AssetReader, directory: &Directory, world: &mut Vec<serde_json::Value>) -> Result<()> {
        // `to_glb` refuses a model over the import limit; split on either.
        let probes = (!session.probes.is_empty()).then_some(&session.probes);
        let glb = scene::document_lit(name, &meshes, package, directory, MAX_TEXTURE, probes, None)?.to_glb().ok().filter(|g| g.len() <= MODEL_LIMIT);
        let Some(glb) = glb else {
            ensure!(meshes.len() > 1, "{name}: one mesh exceeds the model limit");
            let second = meshes.split_off(meshes.len() / 2);
            session.pending.push_front(Load::Part(name.to_owned(), path.to_owned(), second));
            session.pending.push_front(Load::Part(name.to_owned(), path.to_owned(), meshes));
            return Ok(());
        };
        session.next_model += 1;
        let asset = host.upload_model(self.client, 0x4e44_0000 + session.next_model, &glb)?;
        let id = 0x100 + session.next_model;
        world.push(json!({"op": "spawn", "entity": {"id": id, "name": name, "geometry": {"kind": "scene", "asset": asset.relative()}}}));
        session.entities.push((id, asset));
        Ok(())
    }

    /// The environment of a loaded level: the light table's sun, ambient
    /// from the baked probes' mean irradiance and the level's sky image
    /// (`sky-…` package) as background and image-based light. Without a
    /// sky image the environment is black and its light scaled to the
    /// probes' brightness (the procedural daylight sky would otherwise
    /// light night and cave levels).
    /// The render settings in force: those of the render-settings region
    /// holding the camera, or without a player the first a level region
    /// names (`menu-temple-ingame10`: `*menu-temple-render-settings-01*`).
    /// Regions choosing their record by script lambda (`train-wreck`) are
    /// not evaluated.
    fn render_settings_name(session: &Session) -> Option<u32> {
        let regions = &session.stage.regions;
        let eye = session.stage.camera_eye?;
        regions.iter().find(|(r, _, _)| r.render_settings.is_some() && session.stage.holds_point(r, eye)).and_then(|(r, _, _)| r.render_settings)
    }

    fn render_settings(session: &Session) -> Option<crate::render_settings::RenderSettings> {
        let dc = &session.game.as_ref()?.rt.dc;
        let regions = &session.stage.regions;
        // Render-settings regions are `camera` regions: tested at the
        // camera (the eye of the last presented camera).
        let held = regions.iter().filter(|(r, _, _)| r.render_settings.is_some()).find(|(r, _, _)| session.player.is_some() && session.stage.camera_eye.is_some_and(|eye| session.stage.holds_point(r, eye)));
        // Without a player (the menu) the level's first record applies;
        // with one, only a region holding it selects a record.
        let region = held.or_else(|| if session.player.is_none() { regions.iter().find(|(r, _, _)| r.render_settings.is_some()) } else { None })?;
        let name = region.0.render_settings?;
        let settings = crate::render_settings::read(dc, name).ok().flatten();
        settings
    }

    fn level_environment(session: &Session) -> Option<serde_json::Value> {
        if session.probes.is_empty() {
            return None;
        }
        let settings = Self::render_settings(session);
        let e = session.probes.mean_irradiance();
        let lum = 0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2];
        let scale = settings.map_or(1., |s| s.light_scale.iter().sum::<f32>() / 3.);
        let sun = session.light.as_ref().map_or(json!(null), |l| json!({"direction": l.direction, "color": l.color.map(|c| c.clamp(0., 4.)), "intensity": (l.intensity * scale).clamp(0.05, 4.0), "shadow_strength": 1.0}));
        let ambient_scale = settings.map_or(1., |s| s.ambient_scale) * UNBAKED_AMBIENT;
        let mut env = json!({"op": "set_environment", "environment": {"show_sky": session.sky.is_some(), "intensity": if session.sky.is_some() { 1. } else { (lum * 4.).clamp(0.02, 1.) }, "ambient": e.map(|v| (v * ambient_scale).clamp(0., 16.)), "sun": sun, "tonemap": "filmic"}});
        match settings {
            // The area's tonemap: Hable's curve with its constants and
            // bias; the camera adapts (`tonemap-adaption-time`, light range
            // ±4 stops) with a constant compensation calibrated against the
            // original title screen. The record's key (`tonemap-manual-
            // exposure-value`) and the native exposure modes (`sub_ABB33C`
            // caller) are not applied.
            Some(s) => {
                let t = s.tonemap;
                env["environment"]["filmic_curve"] = json!({"shoulder": t.shoulder, "linear": t.linear, "angle": t.angle, "toe": t.toe, "toe_numerator": t.toe_numerator, "toe_denominator": t.toe_denominator, "white": t.white, "exposure_bias": t.exposure_bias});
                env["environment"]["auto_exposure"] = json!({"min_log2": -8.0, "max_log2": 8.0, "speed_up": 3.0, "speed_down": 1.0, "compensation": EXPOSURE_CALIBRATION});
                env["environment"]["grading"] = json!({"color_filter": s.tint.map(|v| v.clamp(0., 4.))});
                // The record's Gaussian fog (see `crate::render_settings`)
                // mapped to the shared exponential fog: the same maximum
                // opacity `density`, and the same opacity `1 − e⁻¹` of it
                // one `distance-range` past `distance-start`. Height
                // thinning maps to the exponential falloff when the range
                // is positive; fog thickening with height (a negative
                // range, the menu temple) stays uniform.
                let f = s.fog;
                if f.density > 0. && f.distance_range > 0. && f.multiplier >= 1. {
                    let colour = f.colour.map(|c| c.clamp(0., 16.));
                    env["environment"]["fog"] = json!({"color": colour, "density": 1. / f.distance_range, "height_falloff": if f.height_range > 0. { 1. / f.height_range } else { 0. }, "base_height": f.height_start, "start_distance": f.distance_start.max(0.), "max_opacity": f.density.clamp(0., 1.), "sun_scattering": 0.0});
                }
            }
            None => {
                env["environment"]["auto_exposure"] = json!({"min_log2": -6.0, "max_log2": 2.0, "speed_up": 3.0, "speed_down": 1.0, "compensation": 0.5});
            }
        }
        if let Some(sky) = &session.sky {
            env["environment"]["hdr"] = json!(sky.relative());
        }
        Some(env)
    }

    /// The main menu's fixed camera: `menu-temple`'s `CAMERA_TABLE_1` start
    /// camera (`|_cam_start_1`, at the `menu-camera-1` spawner) with the
    /// spawner's `fov` property (75°), taken as horizontal for 16:9.
    fn menu_camera(session: &Session) -> serde_json::Value {
        let fov = session.stage.spawners.values().find(|s| s.name.starts_with("menu-camera")).and_then(|s| s.properties.iter().find(|(k, _)| *k == sid("fov"))).and_then(|(_, v)| match v {
            crate::ss::Property::Float(f) => Some(*f),
            crate::ss::Property::Int(i) => Some(*i as f32),
            _ => None,
        });
        let vertical = fov.map_or(46.8, |h| (2. * ((h.to_radians() * 0.5).tan() * SCREEN[1] as f32 / SCREEN[0] as f32).atan()).to_degrees());
        let (eye, forward) = session.start.as_ref().map_or(([0.; 3], [0., 0., 1.]), |c| (c.position, c.forward));
        let target: [f32; 3] = std::array::from_fn(|k| eye[k] + forward[k]);
        json!({"op": "set_camera", "camera": {"eye": eye, "target": target, "up": [0.0, 1.0, 0.0], "fov_y_degrees": vertical, "near": 0.1, "far": 20000.0}})
    }

    /// A scripted camera spawner: the eye at the spawner looking along its
    /// local +z (`cam-railing-center-intro-1` faces the climb start), up
    /// along its local +y, with its horizontal `fov` (75° without one).
    /// Blend times and camera types are not modelled.
    fn spawner_camera(cam: &SpawnerRecord) -> serde_json::Value {
        let q = glam::Quat::from_array(cam.rotation).normalize();
        let (forward, up) = (q * glam::Vec3::Z, q * glam::Vec3::Y);
        let fov = cam.properties.iter().find(|(k, _)| *k == sid("fov")).and_then(|(_, v)| match v {
            crate::ss::Property::Float(f) => Some(*f),
            crate::ss::Property::Int(i) => Some(*i as f32),
            _ => None,
        });
        let h = fov.filter(|f| *f > 1. && *f < 170.).unwrap_or(75.);
        let vertical = (2. * ((h.to_radians() * 0.5).tan() * SCREEN[1] as f32 / SCREEN[0] as f32).atan()).to_degrees();
        let eye = glam::Vec3::from_array(cam.position);
        json!({"op": "set_camera", "camera": {"eye": eye.to_array(), "target": (eye + forward).to_array(), "up": up.to_array(), "fov_y_degrees": vertical, "near": 0.1, "far": 20000.0}})
    }

    /// The camera clip `clip` from the level's `actor` packages
    /// (`camera-train-wreck.pak` for `train-wreck`).
    fn camera_clip(session: &mut Session, clip: u32) -> Option<&CameraClip> {
        if !session.camera_clips.contains_key(&clip) {
            let mut found = None;
            let actors: Vec<String> = session.plan.iter().flat_map(|p| &p.levels).filter_map(|l| session.all_levels.get(l)).flat_map(|l| l.actors.iter().filter(|a| a.starts_with("camera-")).cloned()).collect();
            for actor in actors {
                let package = format!("{actor}.pak");
                if !session.actors.exists(&package) {
                    continue;
                }
                if !session.anim_packages.contains_key(&package) {
                    let Ok(bytes) = session.actors.open(&package).map_err(anyhow::Error::from).and_then(|r| Ok(read_all(r.as_ref(), PACKAGE_LIMIT)?)) else { continue };
                    let Ok(loaded) = crate::mesh::Loaded::read(&crysirror_core::service::MemoryAssetReader(bytes.into()), PACKAGE_LIMIT) else { continue };
                    session.anim_packages.insert(package.clone(), Arc::new(loaded));
                }
                let loaded = session.anim_packages[&package].clone();
                let Some((name, at)) = crate::anim::items(&loaded).find(|(n, _)| sid(n) == clip) else { continue };
                let fps = crate::anim::decode(&loaded, at, name, 1).map_or(30., |c| c.fps);
                let samples: Vec<_> = crate::anim::raw_samples(&loaded, at, CAMERA_CHANNEL)
                    .into_iter()
                    .filter_map(|v| {
                        let (fov, r) = match v.len() {
                            10 => (Some(v[0]), &v[3..]),
                            7 => (None, &v[..]),
                            _ => return None,
                        };
                        Some((fov, glam::Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize(), glam::Vec3::new(r[4], r[5], r[6])))
                    })
                    .collect();
                session.stage.log(format!("camera clip {name}: {} frames", samples.len()));
                found = (!samples.is_empty()).then_some(CameraClip { fps, samples });
                break;
            }
            session.camera_clips.insert(clip, found);
        }
        session.camera_clips.get(&clip).and_then(Option::as_ref)
    }

    /// Offset a `set_camera` op by the running camera animations: each
    /// loops its camera channel relative to its first frame, in camera
    /// space (+z forward, +y up), weighted by its blend (inferred
    /// application, SATD-nd-retail-runtime).
    fn animate_camera(session: &mut Session, op: &mut serde_json::Value) {
        let now = session.stage.now;
        session.stage.camera_animations.retain(|c| c.stop.is_none_or(|s| ((now - s) as f32) < c.blend.max(1e-3)));
        let running: Vec<(u32, f32, f32)> = session
            .stage
            .camera_animations
            .iter()
            .map(|c| {
                let t = (now - c.start) as f32;
                let w_in = if c.blend > 0. { t / c.blend } else { 1. };
                let w_out = c.stop.map_or(1., |s| if c.blend > 0. { 1. - (now - s) as f32 / c.blend } else { 0. });
                (c.clip, t, w_in.min(w_out).clamp(0., 1.))
            })
            .collect();
        let v3 = |v: &serde_json::Value| v.as_array().map(|a| glam::Vec3::new(a[0].as_f64().unwrap_or(0.) as f32, a[1].as_f64().unwrap_or(0.) as f32, a[2].as_f64().unwrap_or(0.) as f32));
        let cam = &op["camera"];
        let (Some(mut eye), Some(target), Some(mut up)) = (v3(&cam["eye"]), v3(&cam["target"]), v3(&cam["up"])) else { return };
        let mut forward = (target - eye).normalize_or_zero();
        let mut fov = cam["fov_y_degrees"].as_f64().unwrap_or(60.) as f32;
        let mut changed = false;
        for (clip, t, w) in running {
            let Some(c) = Self::camera_clip(session, clip) else { continue };
            let x = (t * c.fps).rem_euclid(c.samples.len() as f32);
            let (i, f) = (x.floor() as usize % c.samples.len(), x.fract());
            let (fa, ra, ta) = c.samples[i];
            let (fb, rb, tb) = c.samples[(i + 1) % c.samples.len()];
            let (f0, r0, t0) = c.samples[0];
            let rot = r0.inverse() * ra.slerp(rb, f);
            let offset = r0.inverse() * (ta.lerp(tb, f) - t0);
            let basis = glam::Mat3::from_cols(up.cross(forward).normalize_or_zero(), up, forward);
            let rot = glam::Quat::IDENTITY.slerp(rot, w);
            eye += basis * (offset * w);
            forward = basis * (rot * glam::Vec3::Z);
            up = basis * (rot * glam::Vec3::Y);
            if let (Some(a), Some(b), Some(base)) = (fa, fb, f0) {
                let half = |d: f32| (d.to_radians() * 0.5).tan();
                let zoom = (half(a + (b - a) * f) / half(base)).powf(w);
                fov = (2. * (half(fov) * zoom).atan()).to_degrees();
            }
            changed = true;
        }
        if changed {
            op["camera"]["eye"] = json!(eye.to_array());
            op["camera"]["target"] = json!((eye + forward).to_array());
            op["camera"]["up"] = json!(up.to_array());
            op["camera"]["fov_y_degrees"] = json!(fov);
        }
    }

    /// Place the player and start the level's state-script processes.
    fn start_task(&self, session: &mut Session, world: &mut Vec<serde_json::Value>) -> Result<()> {
        session.ground.prepare();
        // The environment follows the player's render-settings region, so
        // it is set once the player stands (the menu has none).
        session.player = None;
        if session.plan.as_ref().is_some_and(|p| p.menu) {
            if let Some(env) = Self::level_environment(session) {
                world.push(env);
            }
        }
        let plan = session.plan.as_ref().context("task plan")?;
        if plan.menu {
            world.push(Self::menu_camera(session));
            session.stage.log(format!("task {} level {}: menu camera", plan.name, plan.level));
            let (Some(game), Some(modules)) = (session.game.as_mut(), session.modules.as_ref()) else { return Ok(()) };
            if let Err(e) = game.start(modules, &mut session.stage, &session.records) {
                session.script_error = Some(format!("{e:#}"));
            }
            world.append(&mut session.stage.ops);
            session.started = true;
            return Ok(());
        }
        let spawner = plan.continue_player.as_deref().and_then(|n| session.stage.spawners.get(&sid(n))).cloned();
        let (position, yaw) = match &spawner {
            Some(s) => (s.position, yaw_of(s.rotation)),
            None => (session.start.as_ref().map_or([0.; 3], |c| c.position), session.camera.yaw),
        };
        let entity = spawner.as_ref().and_then(|s| session.stage.entities.get(&sid(&s.name))).and_then(|v| v.first().copied());
        if let Some(s) = &spawner {
            session.stage.set_visible(sid(&s.name), true);
        }
        // The task's allies (Flynn in the museum) appear at their continue
        // spawners.
        for ally in &plan.continue_allies {
            if session.stage.spawners.contains_key(&sid(ally)) {
                session.stage.set_visible(sid(ally), true);
                session.stage.log(format!("ally {ally}"));
            }
        }
        let camera = plan.continue_camera.as_deref().and_then(|n| session.stage.spawners.get(&sid(n))).map(|c| c.position);
        let anim = session.anim_table.as_ref().filter(|_| entity.is_some()).map(|t| crate::player::Controller::new(t, yaw));
        // The camera starts behind the player (eye offset `(-sin, cos)` of
        // the view yaw opposite the facing `(sin, cos)` of the yaw).
        let mut player = Player { entity, position, yaw, fall: 0., view_yaw: std::f32::consts::PI - yaw, view_pitch: -0.25, controls: true, anim, air_top: None, view_distance: FOLLOW_DISTANCE, platform: None, grip: None, leap: None, regrab: 0. };
        // The continue camera spawner sets the starting view direction
        // (eye offset `(-sin y · cos p, -sin p, cos y · cos p)` from the head).
        if let Some(c) = camera {
            let d = [c[0] - position[0], c[1] - position[1] - 1.5, c[2] - position[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len > 0.5 {
                player.view_yaw = (-d[0]).atan2(d[2]);
                player.view_pitch = (-d[1] / len).asin().clamp(-1.2, 0.6);
            }
        }
        // A spawner on a climb edge (within 0.3 m; `continue-player-climb-1`
        // on the hanging car's rail) starts hanging from it: the spawner
        // marks the grab point.
        let at = glam::Vec3::from_array(position);
        let edge = session.ground.edges_near(at, 0.3).into_iter().min_by(|a, b| crate::ground::segment_distance(at, a.1.p0, a.1.p1).total_cmp(&crate::ground::segment_distance(at, b.1.p0, b.1.p1)));
        match (edge, player.anim.as_mut()) {
            (Some((index, e, body)), Some(c)) => {
                player.grip = Some(Grip { body, edge: index, s: (at - e.p0).dot((e.p1 - e.p0).normalize_or_zero()).clamp(0., (e.p1 - e.p0).length()), climb: None });
                c.hang(0.);
                session.stage.log(format!("player hangs from edge {index} of {body:?}"));
            }
            _ => {
                let below = session.ground.height(position[0], position[2], position[1] + 1.);
                // A spawn more than 8 m above the next surface keeps its
                // height.
                if let Some(h) = below.filter(|h| position[1] - h <= 8.) {
                    player.position[1] = h;
                }
            }
        }
        session.stage.player_locator = Some((player.position, glam::Quat::from_rotation_y(player.yaw).to_array()));
        session.stage.camera_eye = Some([player.position[0], player.position[1] + 1.5, player.position[2]]);
        session.player = Some(player);
        if let Some(env) = Self::level_environment(session) {
            world.push(env);
        }
        session.stage.log(format!("task {} level {}: player at {:?}", plan.name, plan.level, position));
        let (Some(game), Some(modules)) = (session.game.as_mut(), session.modules.as_ref()) else { return Ok(()) };
        match game.start(modules, &mut session.stage, &session.records) {
            Ok(n) => session.stage.log(format!("{n} state-script processes")),
            Err(e) => session.script_error = Some(format!("{e:#}")),
        }
        world.append(&mut session.stage.ops);
        world.push(Self::follow(session.player.as_mut().expect("player"), &session.ground, 0.));
        session.started = true;
        Ok(())
    }

    /// A `TaskManagerPlayMovie` subnode plays `movie1/NAME.m2v` full
    /// screen; it completes when the stream ends or Enter/Escape skips it.
    fn start_movie(&self, host: &mut dyn HostServices, session: &mut Session, name: &str) -> Result<Option<(serde_json::Value, serde_json::Value)>> {
        let path = format!("{MAIN}/movie1/{name}.m2v");
        let storage = host.storage(self.client)?;
        let Ok(reader) = storage.open_asset(&VfsPath::new(Mount::Asset, &path)?) else { return Ok(None) };
        // World videos are read whole, within the runtime's per-file limit;
        // a longer movie is skipped and its task completes.
        if reader.len()? > MAX_WORLD_VIDEO {
            session.movie = Some(session.stage.now);
            session.stage.log(format!("movie {name}.m2v exceeds the {} MiB world-video read limit; skipped", MAX_WORLD_VIDEO >> 20));
            return Ok(None);
        }
        let seconds = movie_seconds(reader.as_ref())?;
        session.movie = Some(session.stage.now + seconds);
        session.movie_video = true;
        session.stage.log(format!("movie {name}.m2v {seconds:.1} s"));
        let video = json!({"op": "set_video", "name": "movie", "video": {"asset": path, "playing": true}});
        let layer = json!({"op": "put", "layer": {"id": MOVIE_LAYER, "rect": [0, 0, SCREEN[0], SCREEN[1]], "paint": {"kind": "image", "asset": MOVIE_SURFACE}}});
        Ok(Some((video, layer)))
    }

    /// Begin the current subnode's own behaviour: its start lambda (for
    /// example the retail skip of the kiosk trailer, which plays another
    /// task), then what its task-manager record presents. A subnode that
    /// presents nothing and has no level completes at once.
    fn enter_task(&self, host: &mut dyn HostServices, session: &mut Session) -> Result<()> {
        let Some(game) = session.game.as_mut() else { return Ok(()) };
        let Some(at) = game.rt.inner.task else { return Ok(()) };
        let sub = game.rt.inner.tasks.nodes[at.0].subnodes[at.1].clone();
        if let Some(l) = sub.on_start {
            if let Err(e) = game.rt.run_lambda(&mut session.stage, l, &[]) {
                session.stage.log(format!("start lambda of {}: {e:#}", sub.name));
            }
            if game.rt.inner.task_change.is_some() {
                return Ok(());
            }
        }
        let mut layers = Vec::new();
        let mut world = Vec::new();
        match &sub.manager {
            Manager::PlayMovie(name) => match self.start_movie(host, session, name)? {
                Some((video, layer)) => {
                    world.push(video);
                    layers.push(layer);
                }
                None if session.movie.is_none() => session.movie = Some(session.stage.now),
                None => {}
            },
            Manager::MessageDialog(key) => {
                let text = session.text(key);
                session.ui = Some(Ui::Message(text));
            }
            Manager::MainMenu => {
                let root = session.menu_pages.iter().position(|p| p.name == "PAGE_ROOT_MAIN");
                session.ui = Some(Ui::Menu { stack: root.into_iter().collect(), selected: 0 });
            }
            Manager::None | Manager::Other(_) => {
                if sub.level.is_empty() {
                    let next = game_next(session, at);
                    if let Some(game) = session.game.as_mut() {
                        game.rt.inner.task_change = next;
                    }
                }
            }
        }
        layers.extend(Self::ui_layers(session));
        host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": layers, "world": world}))?)?;
        Ok(())
    }

    /// Message dialog or main-menu layers (text from the disc catalog).
    fn ui_layers(session: &Session) -> Vec<serde_json::Value> {
        let text = |id: u32, rect: [i32; 4], line: &str, color: [u8; 4], size: u16| {
            json!({"op": "put", "layer": {"id": id, "rect": rect, "paint": {"kind": "text", "text": line, "font": "default", "size": size, "color": color, "layout": crysirror_core::service::SIMPLE_TEXT_LAYOUT}}})
        };
        // A fixed set of layers always exists; unused ones are transparent.
        let mut out: Vec<serde_json::Value> = (0..UI_LAYERS).map(|i| json!({"op": "put", "layer": {"id": UI_LAYER + i, "rect": [0, 0, 1, 1], "paint": {"kind": "solid", "color": [0, 0, 0, 0]}}})).collect();
        match session.ui.as_ref() {
            None => {}
            Some(Ui::Message(t)) => out[0] = text(UI_LAYER, [160, 260, SCREEN[0] as i32 - 320, 200], t, [255, 255, 255, 255], 28),
            Some(Ui::Menu { stack, selected }) => {
                let Some(page) = stack.last().and_then(|i| session.menu_pages.get(*i)) else { return out };
                if let Some((asset, w, h)) = &session.logo {
                    // Title logo, upper left, at half its texture width.
                    let (w, h) = (*w as i32 / 2, *h as i32 / 2);
                    out[0] = json!({"op": "put", "layer": {"id": UI_LAYER, "rect": [96, 64, w, h], "paint": {"kind": "image", "asset": asset.relative()}}});
                }
                let root = page.name == "PAGE_ROOT_MAIN";
                if let Some(t) = page.title.as_deref().filter(|_| !root) {
                    out[1] = text(UI_LAYER + 1, [128, 340, 900, 60], &session.text(t), [235, 225, 200, 255], 30);
                }
                if root {
                    if let Some(t) = page.title.as_deref() {
                        out[1] = text(UI_LAYER + 1, [128, 650, 1100, 40], &session.text(t), [200, 200, 200, 255], 14);
                    }
                }
                let visible: Vec<&crate::menu::Button> = page.buttons.iter().filter(|b| b.condition == 0).collect();
                for (i, button) in visible.iter().enumerate().take(UI_LAYERS as usize - 2) {
                    let on = i == *selected;
                    let enabled = !matches!(menu_action(&button.name), MenuAction::None);
                    let color = match (on, enabled) {
                        (true, true) => [255, 230, 160, 255],
                        (true, false) => [190, 170, 130, 255],
                        (false, true) => [225, 225, 225, 255],
                        (false, false) => [130, 130, 130, 255],
                    };
                    let y = if root { 520 } else { 400 + 46 * i as i32 };
                    let x = if root { 520 } else { 128 };
                    out[2 + i] = text(UI_LAYER + 2 + i as u32, [x, y, 700, 50], &format!("{}{}", if on && !root { "> " } else { "" }, session.text(&button.text)), color, if root { 30 } else { 32 });
                }
            }
        }
        out
    }

    /// An NPC skeleton from its `*-base` package and the clips named
    /// `wanted` (and `idle`) in it and the character's animation packages
    /// that fit the skeleton; clips the decoder cannot read are skipped.
    /// Present `PartSpawner`s: load the particle modules the level's
    /// regions name (`part-module`), then spawn one shared VFX entity per
    /// emitter of each spawner's `particle-group` (see `crate::particle`).
    fn load_particles(&self, host: &mut dyn HostServices, session: &mut Session, spawners: &[SpawnerRecord], world: &mut Vec<serde_json::Value>) -> Result<()> {
        let globals = GLOBAL_PART_MODULES.iter().map(|n| sid(n)).filter(|_| session.game.is_some());
        let wanted: Vec<u32> = session.stage.regions.iter().filter_map(|(r, _, _)| r.part_module).chain(globals).filter(|m| !session.part_modules.contains(m)).collect();
        if let Some(modules) = &session.modules {
            for module in wanted {
                session.part_modules.insert(module);
                let Some(name) = crate::game::ModuleSource::names(modules).into_iter().find(|n| sid(n) == module) else {
                    session.stage.log(format!("particle module {module:#010x}: not installed"));
                    continue;
                };
                match crate::game::ModuleSource::read(modules, &name).and_then(|b| crate::particle::Library::parse(&name, &b)) {
                    Ok(library) => {
                        session.stage.log(format!("particle module {name}: {} groups, {} effects", library.groups.len(), library.effects.len()));
                        session.particles.extend(library);
                    }
                    Err(e) => session.stage.log(format!("particle module {name}: {e:#}")),
                }
            }
        }
        if session.particle_materials.is_none() {
            let materials = package_path(&session.mount, "particle").map(|path| -> Result<_> {
                let reader = crysirror_core::service::MemoryAssetReader(read_all(session.mount.open(&path)?.as_ref(), PACKAGE_LIMIT)?.into());
                let directory = Directory::read(&reader)?;
                let loaded = crate::mesh::Loaded::read(&reader, PACKAGE_LIMIT)?;
                let item = directory.items.iter().find(|i| i.kind == "MATERIAL_TABLE_1").context("particle.pak: no material table")?;
                Ok((crate::particle::materials(&loaded, item.payload_offset as usize)?, reader, directory))
            });
            match materials {
                Some(Ok((materials, reader, directory))) => {
                    // Decode each material's first sampler that is not the
                    // shared placeholder; atlases (width a multiple of the
                    // height) are frame strips.
                    let mut textures = HashMap::new();
                    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
                        if let Ok(desc) = crate::texture::VramDesc::read(&reader, item.payload_offset) {
                            textures.insert(desc.id, desc);
                        }
                    }
                    for m in materials.values() {
                        let Some(desc) = m.samplers.iter().filter_map(|id| textures.get(id)).find(|d| !d.name.contains("missingtexture")) else { continue };
                        if session.particle_textures.contains_key(&desc.id) {
                            continue;
                        }
                        let uploaded = desc.decode(&reader, directory.texture_base).and_then(|image| {
                            session.next_model += 1;
                            let path = host.upload_rgba(self.client, 0x4e50_0000 + session.next_model, image.width, image.height, &image.rgba)?;
                            let columns = if image.width > image.height && image.width % image.height == 0 { image.width / image.height } else { 1 };
                            Ok((path.relative().to_string(), columns))
                        });
                        match uploaded {
                            Ok(t) => session.particle_textures.insert(desc.id, Some(t)),
                            Err(e) => {
                                session.stage.log(format!("particle texture {}: {e:#}", desc.name));
                                session.particle_textures.insert(desc.id, None)
                            }
                        };
                    }
                    let resolved: HashMap<u32, crate::particle::ParticleMaterial> = materials
                        .into_iter()
                        .map(|(k, mut m)| {
                            m.samplers.retain(|id| session.particle_textures.get(id).is_some_and(Option::is_some));
                            (k, m)
                        })
                        .collect();
                    session.particle_materials = Some(resolved);
                }
                Some(Err(e)) => {
                    session.stage.log(format!("particle.pak: {e:#}"));
                    session.particle_materials = Some(HashMap::new());
                }
                None => session.particle_materials = Some(HashMap::new()),
            }
        }
        let mut presented = 0;
        for s in spawners {
            let property = |name: &str| s.properties.iter().find(|(k, _)| *k == sid(name)).map(|(_, v)| v.clone());
            let Some(crate::ss::Property::Str(group)) = property("particle-group") else { continue };
            let Some(emitters) = session.particles.groups.get(&sid(&group)).cloned() else {
                session.stage.log(format!("{}: particle group {group} not loaded", s.name));
                continue;
            };
            let float = |name: &str, default: f32| match property(name) {
                Some(crate::ss::Property::Float(f)) => f,
                Some(crate::ss::Property::Int(i)) => i as f32,
                _ => default,
            };
            let vector = |name: &str| match property(name) {
                Some(crate::ss::Property::Vec4(v)) => Some([v[0], v[1], v[2]]),
                _ => None,
            };
            let placement = crate::particle::Placement {
                position: glam::Vec3::from_array(s.position),
                rotation: glam::Quat::from_array(s.rotation).normalize(),
                color: vector("color").unwrap_or([1.; 3]),
                alpha: float("alpha", 1.),
                sprite_scale: float("sprite-scale", 1.),
                effect_scale: vector("effect-scale").map_or(glam::Vec3::ONE, glam::Vec3::from_array),
            };
            // The particle manager births particle spawners from their
            // `particle-group` tag each frame (`sub_76BC28` → `sub_761B30`),
            // without the spawner process, so `noAutoBirth` does not hold
            // them back; scripts can still hide them.
            presented += Self::place_effect(session, &emitters, &placement, sid(&s.name), &s.name);
            session.stage.visible.insert(sid(&s.name));
        }
        session.packages.push(format!("particles×{presented}"));
        Self::update_effects(session, world);
        Ok(())
    }

    /// Queue the shared emitters of an effect group's descriptors at
    /// `placement` under `owner` (a spawner or script handle); returns how
    /// many were placed.
    fn place_effect(session: &mut Session, emitters: &[u32], placement: &crate::particle::Placement, owner: u32, label: &str) -> usize {
        let mut placed = 0;
        for (k, emitter) in emitters.iter().enumerate() {
            let Some(effect) = session.particles.effects.get(emitter) else { continue };
            let material = session.particle_materials.as_ref().and_then(|m| m.get(&effect.material));
            let texture = material.and_then(|m| m.samplers.first()).and_then(|id| session.particle_textures.get(id).cloned().flatten());
            let blend = material.map_or(crysirror_vfx::BlendMode::Additive, |m| crate::particle::blend(m.blend));
            let soft = material.and_then(|m| m.depth_blend).unwrap_or(0.);
            let (spec, frame) = crate::particle::emitter(effect, &effect.name, placement, texture, blend, soft, owner ^ k as u32);
            let system = crysirror_vfx::SystemSpec { emitters: vec![spec], ..Default::default() };
            let (scale, rotation, translation) = frame.to_scale_rotation_translation();
            let (x, y, z) = rotation.to_euler(glam::EulerRot::XYZ);
            session.next_model += 1;
            let id = 0x100 + session.next_model;
            let entity = json!({"id": id, "name": format!("{label}:{}", effect.name), "visible": true, "transform": {"translation": translation.to_array(), "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": scale.to_array()}, "vfx": system});
            session.effects.push(Effect { id, spawner: owner, at: translation, entity, live: false });
            placed += 1;
        }
        placed
    }

    /// Serve the scripts' particle requests (`spawn-particles-at-…`,
    /// `kill-particles`): effects are placed under their handle at the
    /// requested frame; attached effects do not follow the joint.
    fn serve_particle_requests(session: &mut Session, world: &mut Vec<serde_json::Value>) {
        for request in std::mem::take(&mut session.stage.particle_requests) {
            match request {
                crate::ss::ParticleRequest::Spawn { handle, group, position, rotation } => {
                    let Some(emitters) = session.particles.groups.get(&group).cloned() else {
                        session.stage.log(format!("script particles {group:#010x}: group not loaded"));
                        continue;
                    };
                    let placement = crate::particle::Placement { position: glam::Vec3::from_array(position), rotation: glam::Quat::from_array(rotation).normalize(), color: [1.; 3], alpha: 1., sprite_scale: 1., effect_scale: glam::Vec3::ONE };
                    Self::place_effect(session, &emitters, &placement, handle, &format!("script-particles-{handle:x}"));
                    session.stage.visible.insert(handle);
                }
                crate::ss::ParticleRequest::Kill { handle } => {
                    session.effects.retain(|e| {
                        if e.spawner != handle {
                            return true;
                        }
                        if e.live {
                            world.push(json!({"op": "despawn", "id": e.id}));
                        }
                        false
                    });
                    session.stage.visible.remove(&handle);
                }
            }
        }
    }

    /// Keep the `MAX_EFFECTS` nearest particle effects within
    /// `EFFECT_RANGE` of the camera live (spawned) and the rest asleep.
    fn update_effects(session: &mut Session, world: &mut Vec<serde_json::Value>) {
        let Some(eye) = session.stage.camera_eye.map(glam::Vec3::from_array).or_else(|| session.start.as_ref().map(|c| glam::Vec3::from_array(c.position))) else { return };
        let mut order: Vec<(f32, usize)> = session.effects.iter().enumerate().filter(|(_, e)| session.stage.visible.contains(&e.spawner)).map(|(i, e)| (e.at.distance(eye), i)).filter(|(d, _)| *d <= EFFECT_RANGE).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wanted: HashSet<usize> = order.into_iter().take(MAX_EFFECTS).map(|(_, i)| i).collect();
        for (i, e) in session.effects.iter_mut().enumerate() {
            match (e.live, wanted.contains(&i)) {
                (false, true) => world.push(json!({"op": "spawn", "entity": e.entity})),
                (true, false) => world.push(json!({"op": "despawn", "id": e.id})),
                _ => continue,
            }
            e.live = !e.live;
        }
    }

    fn npc_rig(actors: &ArchiveMount, base: &str, anims: &[String], wanted: &HashSet<u32>) -> Result<Option<(crate::skeleton::Skeleton, Vec<crate::anim::Clip>)>> {
        let read = |name: &str| -> Result<crate::mesh::Loaded> {
            let package = crysirror_core::service::MemoryAssetReader(read_all(actors.open(name)?.as_ref(), PACKAGE_LIMIT)?.into());
            crate::mesh::Loaded::read(&package, PACKAGE_LIMIT)
        };
        let path = format!("{base}.pak");
        if !actors.exists(&path) {
            return Ok(None);
        }
        let Some(skeleton) = crate::skeleton::of_package(&read(&path)?)? else { return Ok(None) };
        let mut clips = Vec::new();
        let packages = std::iter::once(path.clone()).chain(anims.iter().map(|a| format!("{a}.pak")));
        for package in packages {
            if !actors.exists(&package) {
                continue;
            }
            let loaded = read(&package)?;
            for (n, at) in crate::anim::items(&loaded).filter(|(n, _)| *n == "idle" || wanted.contains(&sid(n))) {
                if clips.iter().any(|c: &crate::anim::Clip| c.name == n) {
                    continue;
                }
                if let Ok(clip) = crate::anim::decode(&loaded, at, n, skeleton.animated) {
                    if clip.hierarchy == skeleton.hash {
                        clips.push(clip);
                    }
                }
            }
        }
        Ok(Some((skeleton, clips)))
    }

    /// Load the clips scripts started this frame and blend every running
    /// script animation over its entities' controller layers (weight ramps
    /// over the blend-in and blend-out times); an animation with a
    /// locator places its entities there when it starts. Animations whose
    /// clip cannot be found end at once (logged).
    fn script_animations(&self, host: &mut dyn HostServices, session: &mut Session, world: &mut Vec<serde_json::Value>) {
        let objects: Vec<u32> = session.stage.animations.iter().filter(|(_, a)| a.source.is_none() && !a.failed).map(|(o, _)| *o).collect();
        for object in objects {
            let clip = session.stage.animations[&object].clip;
            let entities = Self::animation_entities(session, object);
            let rig = entities.iter().find_map(|e| session.entity_rigs.get(e).cloned());
            let loaded = match rig {
                Some(rig) => self.animation_library(host, session, &rig, clip),
                None => Err(anyhow::anyhow!("object has no skinned entity")),
            };
            // The entity's transform now: the player's, else its spawner's.
            let current = if object == sid("player") {
                session.player.as_ref().map(|p| glam::Mat4::from_rotation_translation(glam::Quat::from_rotation_y(p.yaw), glam::Vec3::from_array(p.position)))
            } else {
                session.stage.spawners.get(&object).map(|s| glam::Mat4::from_rotation_translation(glam::Quat::from_array(s.rotation).normalize(), glam::Vec3::from_array(s.position)))
            };
            let a = session.stage.animations.get_mut(&object).expect("animation");
            match loaded {
                Ok(lib) => {
                    a.duration = Some(lib.seconds);
                    a.source = Some((lib.asset.clone(), lib.name.clone()));
                    a.start = session.stage.now;
                    // Clip space in the world: the locator stands where the
                    // clip's `apReference` starts; without one, the root
                    // starts where the entity stands.
                    let frame = |(t, r): (glam::Vec3, glam::Quat)| glam::Mat4::from_rotation_translation(r, t);
                    let base = match (a.options.locator, lib.ap_reference.first(), lib.align.first(), current) {
                        (Some((pos, rot)), ap, _, _) => Some(glam::Mat4::from_rotation_translation(glam::Quat::from_array(rot).normalize(), glam::Vec3::from_array(pos)) * ap.map_or(glam::Mat4::IDENTITY, |a| frame(*a).inverse())),
                        (None, _, Some(first), Some(now)) => Some(now * frame(*first).inverse()),
                        _ => None,
                    };
                    if let Some(base) = base {
                        let align = if lib.align.is_empty() { vec![(glam::Vec3::ZERO, glam::Quat::IDENTITY)] } else { lib.align.clone() };
                        a.root = Some((align, lib.fps, base));
                    }
                    session.stage.log(format!("animate {object:08x} {} {:.2}s", lib.name, lib.seconds));
                }
                Err(e) => {
                    a.failed = true;
                    session.stage.log(format!("animate {object:08x} clip {clip:08x}: {e:#}"));
                }
            }
        }
        let now = session.stage.now;
        let mut finished = Vec::new();
        let mut held = Vec::new();
        let mut moves = Vec::new();
        for (object, a) in &session.stage.animations {
            let (Some((asset, name)), Some(d)) = (&a.source, a.duration) else {
                if a.failed {
                    finished.push(*object);
                }
                continue;
            };
            let t = (now - a.start) as f32;
            if t >= d {
                finished.push(*object);
                let entities = Self::animation_entities(session, *object);
                let controlled = entities.iter().any(|e| session.player.as_ref().is_some_and(|p| p.entity == Some(*e)) || session.npcs.iter().any(|n| n.entity == *e));
                if !controlled {
                    held.push((*object, (asset.clone(), name.clone(), d)));
                }
                continue;
            }
            let (bin, bout) = (a.options.blend_in.unwrap_or(SCRIPT_BLEND), a.options.blend_out.unwrap_or(SCRIPT_BLEND));
            let w_in = if bin > 0. { t / bin } else { 1. };
            let w_out = if bout > 0. { (d - t) / bout } else { 1. };
            let w = w_in.min(w_out).clamp(0., 1.);
            let layer = json!({"clip": name, "source": asset, "time": t, "speed": 0.0, "weight": w.max(1e-3), "target_weight": w.max(1e-3), "looping": false});
            if let Some((align, fps, base)) = &a.root {
                let x = (t * fps).clamp(0., (align.len() - 1) as f32);
                let (i, f) = (x.floor() as usize, x.fract());
                let (t0, r0) = align[i];
                let (t1, r1) = align[(i + 1).min(align.len() - 1)];
                let m = *base * glam::Mat4::from_rotation_translation(r0.slerp(r1, f), t0.lerp(t1, f));
                let (_, r, pos) = m.to_scale_rotation_translation();
                moves.push((*object, pos.to_array(), r));
            }
            for e in Self::animation_entities(session, *object) {
                let found = world.iter_mut().rev().find(|op| op["op"] == "set_animation" && op["id"] == e);
                match found {
                    Some(op) => {
                        if let Some(layers) = op["animation"]["layers"].as_array_mut() {
                            for l in layers.iter_mut() {
                                for k in ["weight", "target_weight"] {
                                    if let Some(v) = l[k].as_f64() {
                                        l[k] = json!(v * f64::from(1. - w));
                                    }
                                }
                            }
                            layers.push(layer.clone());
                        }
                    }
                    None => world.push(json!({"op": "set_animation", "id": e, "animation": {"layers": [layer.clone()]}})),
                }
            }
        }
        // Root motion: entities follow the clip's `align` channel; the
        // player keeps the position for its controller.
        for (object, pos, r) in moves {
            let yaw = yaw_of(r.to_array());
            if object == sid("player") {
                if let Some(p) = session.player.as_mut() {
                    p.position = pos;
                    p.yaw = yaw;
                    if let Some(c) = p.anim.as_mut() {
                        c.yaw = yaw;
                    }
                }
            }
            let (x, y, z) = r.to_euler(glam::EulerRot::XYZ);
            if object != sid("player") {
                session.stage.poses.insert(object, glam::Mat4::from_rotation_translation(r, glam::Vec3::from_array(pos)));
            }
            let m = glam::Mat4::from_rotation_translation(r, glam::Vec3::from_array(pos)).to_cols_array();
            for e in Self::animation_entities(session, object) {
                session.ground.move_body(e, m);
                world.push(json!({"op": "set_transform", "id": e, "transform": {"translation": pos, "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1.0, 1.0, 1.0]}}));
            }
        }
        // Spawners placed in a moved object's frame (the rail, seats and
        // child cameras of `train-wreck-car-b-1`) move with it, unless they
        // play their own animation.
        let moved: Vec<u32> = session.stage.poses.keys().copied().filter(|o| session.stage.animations.contains_key(o)).collect();
        for parent in moved {
            Self::move_children(session, parent, world, 0);
        }
        for o in finished {
            session.stage.animations.remove(&o);
        }
        // A held pose is set once; the entity keeps it until replaced.
        for (object, (asset, name, t)) in held {
            for e in Self::animation_entities(session, object) {
                world.push(json!({"op": "set_animation", "id": e, "animation": {"layers": [{"clip": name, "source": asset, "time": t, "speed": 0.0, "weight": 1.0, "target_weight": 1.0, "looping": false}]}}));
            }
            session.stage.held.insert(object, (asset, name, t));
        }
    }

    /// The skeleton with JH hash `hash` among the human base rigs and the
    /// loaded characters' skeleton packages.
    fn skeleton_for(session: &mut Session, hash: u32) -> Result<Option<crate::skeleton::Skeleton>> {
        let mut candidates: Vec<String> = vec!["npc-normal-base.pak".into(), "proto.pak".into()];
        for packages in session.npc_packages.values() {
            if let Some(base) = packages.first() {
                if !candidates.contains(base) {
                    candidates.push(base.clone());
                }
            }
        }
        for package in candidates {
            if !session.skeletons.contains_key(&package) {
                let skeleton = if session.actors.exists(&package) {
                    let reader = crysirror_core::service::MemoryAssetReader(read_all(session.actors.open(&package)?.as_ref(), PACKAGE_LIMIT)?.into());
                    crate::skeleton::of_package(&crate::mesh::Loaded::read(&reader, PACKAGE_LIMIT)?)?
                } else {
                    None
                };
                session.skeletons.insert(package.clone(), skeleton);
            }
            if let Some(Some(s)) = session.skeletons.get(&package) {
                if s.hash == hash {
                    return Ok(Some(s.clone()));
                }
            }
        }
        Ok(None)
    }

    /// The entities an animated object stands for: the player's, or its
    /// spawner's.
    /// Place a hanging or climbing player from its grip. Hanging: the
    /// clip's sideways root motion slides the grab point along the edge
    /// (onto the linked edge past an end) and the root stands at `edge ·
    /// AP⁻¹`. Climbing: the root follows `edge · AP⁻¹ · align(t)` from the
    /// edge frame where the climb began, or the clip's `align` motion for
    /// clips without a reference. A grip whose edge is gone is released.
    fn follow_grip(p: &mut Player, ground: &Ground, mode: crate::player::Mode, root: Option<&crate::player::Root>) {
        let Some(mut grip) = p.grip else { return };
        let Some(mut edge) = ground.edge(grip.body, grip.edge) else {
            p.grip = None;
            return;
        };
        let ap = |r: Option<&crate::player::Root>| r.and_then(|r| r.ap).map_or(glam::Mat4::from_translation(HANG_REACH), |(t, q)| glam::Mat4::from_rotation_translation(q, t));
        match mode {
            crate::player::Mode::Hang => {
                grip.climb = None;
                if let Some(r) = root {
                    let delta = r.align.0 - r.before.0;
                    if delta.length() < 0.2 {
                        let frame = edge_frame(&edge, grip.s);
                        grip.s += frame.transform_vector3(glam::Vec3::new(delta.x, 0., 0.)).dot((edge.p1 - edge.p0).normalize_or_zero());
                    }
                }
                // Past an end: the linked edge, else the end.
                for _ in 0..4 {
                    let len = (edge.p1 - edge.p0).length();
                    let link = if grip.s < 0. { edge.prev } else if grip.s > len { edge.next } else { None };
                    let Some(next) = link.and_then(|i| ground.edge(grip.body, usize::from(i)).map(|e| (usize::from(i), e))) else {
                        grip.s = grip.s.clamp(0., len);
                        break;
                    };
                    grip.s = if grip.s < 0. { (next.1.p1 - next.1.p0).length() + grip.s } else { grip.s - len };
                    (grip.edge, edge) = next;
                }
                let m = edge_frame(&edge, grip.s) * ap(root).inverse();
                let (_, _, t) = m.to_scale_rotation_translation();
                p.position = t.to_array();
            }
            crate::player::Mode::Climb => {
                let start = *grip.climb.get_or_insert_with(|| edge_frame(&edge, grip.s));
                match root {
                    Some(r) if r.ap.is_some() => {
                        let m = start * ap(Some(r)).inverse() * glam::Mat4::from_rotation_translation(r.align.1, r.align.0);
                        p.position = m.to_scale_rotation_translation().2.to_array();
                    }
                    Some(r) => {
                        let (_, q, _) = start.to_scale_rotation_translation();
                        p.position = (glam::Vec3::from_array(p.position) + q * (r.align.0 - r.before.0)).to_array();
                    }
                    None => {}
                }
            }
            crate::player::Mode::Ground | crate::player::Mode::Leap => return,
        }
        let f = edge_frame(&edge, grip.s).transform_vector3(glam::Vec3::Z);
        p.yaw = f.x.atan2(f.z);
        if let Some(c) = p.anim.as_mut() {
            c.yaw = p.yaw;
        }
        p.fall = 0.;
        p.air_top = None;
        p.grip = Some(grip);
    }

    /// The edge a hanging player can jump to: above (`toward` up: 0.4–2.5 m
    /// higher, within 0.8 m sideways) or to a side (0.5–2.5 m along
    /// `toward`, within 1 m of the height), facing the same way. Distances
    /// are not native (SATD-nd-retail-runtime).
    fn leap_target(ground: &Ground, grip: Grip, toward: glam::Vec3) -> Option<Grip> {
        let edge = ground.edge(grip.body, grip.edge)?;
        let frame = edge_frame(&edge, grip.s);
        let at = frame.transform_point3(glam::Vec3::ZERO);
        let facing = frame.transform_vector3(glam::Vec3::Z);
        let up = toward.y > 0.5;
        let probe = at + toward * 1.4;
        let closest = |e: &crate::collision::Edge| {
            let d = (e.p1 - e.p0).normalize_or_zero();
            let s = (at - e.p0).dot(d).clamp(0., (e.p1 - e.p0).length());
            (e.p0 + d * s, s)
        };
        ground
            .edges_near(probe, 1.6)
            .into_iter()
            .filter(|(i, e, body)| !(*body == grip.body && *i == grip.edge) && glam::Vec3::new(-e.n1.x, 0., -e.n1.z).normalize_or_zero().dot(facing) > 0.7)
            .filter_map(|(i, e, body)| {
                let (q, s) = closest(&e);
                let d = q - at;
                let flat = glam::Vec3::new(d.x, 0., d.z);
                let ok = if up { (0.4..=2.5).contains(&d.y) && flat.length() < 0.8 } else { d.y.abs() < 1. && (0.5..=2.5).contains(&flat.dot(toward)) };
                ok.then_some((d.length(), Grip { body, edge: i, s, climb: None }))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, g)| g)
    }

    /// Place the spawners parented to `parent` (`SpawnerRecord::parent`)
    /// in its current frame: `child = parent now · parent spawner⁻¹ · child
    /// spawner`, recursively; their entities and bodies follow.
    fn move_children(session: &mut Session, parent: u32, world: &mut Vec<serde_json::Value>, depth: usize) {
        let (Some(now), Some(spawned)) = (session.stage.poses.get(&parent).copied(), session.stage.spawners.get(&parent)) else { return };
        if depth > 8 {
            return;
        }
        let frame = |s: &SpawnerRecord| glam::Mat4::from_rotation_translation(glam::Quat::from_array(s.rotation).normalize(), glam::Vec3::from_array(s.position));
        let delta = now * frame(spawned).inverse();
        let name = spawned.name.clone();
        let children: Vec<(u32, glam::Mat4)> = session.stage.spawners.iter().filter(|(c, s)| s.parent.as_deref() == Some(name.as_str()) && !session.stage.animations.contains_key(c)).map(|(c, s)| (*c, delta * frame(s))).collect();
        for (child, m) in children {
            session.stage.poses.insert(child, m);
            let (_, r, t) = m.to_scale_rotation_translation();
            let (x, y, z) = r.to_euler(glam::EulerRot::XYZ);
            for e in session.stage.entities.get(&child).cloned().unwrap_or_default() {
                session.ground.move_body(e, m.to_cols_array());
                world.push(json!({"op": "set_transform", "id": e, "transform": {"translation": t.to_array(), "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1.0, 1.0, 1.0]}}));
            }
            Self::move_children(session, child, world, depth + 1);
        }
    }

    fn animation_entities(session: &Session, object: u32) -> Vec<u32> {
        if object == sid("player") {
            return session.player.as_ref().and_then(|p| p.entity).into_iter().collect();
        }
        session.stage.entities.get(&object).cloned().unwrap_or_default()
    }

    /// Upload (once per rig and clip) a library with `clip` decoded for
    /// `rig`'s skeleton, searched in the rig's packages and then the current
    /// task's level animation packages. Returns the asset, clip name and
    /// length.
    fn animation_library(&self, host: &mut dyn HostServices, session: &mut Session, rig: &RigRef, clip: u32) -> Result<LibraryClip> {
        let key = (format!("{rig:?}"), clip);
        if let Some(found) = session.libraries.get(&key) {
            return found.clone().context("clip not found earlier");
        }
        let mut packages: Vec<String> = match rig {
            RigRef::Player => PLAYER_CLIP_PACKAGES.iter().map(|p| p.to_string()).collect(),
            RigRef::Npc(k) => session.npc_packages.get(k).cloned().unwrap_or_default(),
            RigRef::Own(art) => vec![format!("{art}.pak")],
        };
        if let Some(plan) = &session.plan {
            for level in &plan.levels {
                // `pak22.txt` `actor` lines: the level's animation packages.
                for actor in session.all_levels.get(level).map(|l| l.actors.as_slice()).unwrap_or_default() {
                    let p = format!("{actor}.pak");
                    if !packages.contains(&p) {
                        packages.push(p);
                    }
                }
            }
        }
        let mut result = None;
        for package in &packages {
            if !session.actors.exists(package) {
                continue;
            }
            if !session.anim_packages.contains_key(package) {
                let reader = crysirror_core::service::MemoryAssetReader(read_all(session.actors.open(package)?.as_ref(), PACKAGE_LIMIT)?.into());
                session.anim_packages.insert(package.clone(), Arc::new(crate::mesh::Loaded::read(&reader, PACKAGE_LIMIT)?));
            }
            let loaded = session.anim_packages[package].clone();
            let Some((name, at)) = crate::anim::items(&loaded).find(|(n, _)| sid(n) == clip) else { continue };
            let skeleton = match rig {
                RigRef::Player => session.player_rig.as_ref().and_then(|r| r.as_ref()).map(|(s, _)| s),
                RigRef::Npc(k) => session.npc_rigs.get(k).and_then(|r| r.as_ref()).map(|(s, _)| s),
                RigRef::Own(art) => session.own_rigs.get(art),
            }
            .context("rig skeleton not loaded")?;
            let mut skeleton = skeleton.clone();
            // A clip authored on another rig (the story's player clips name
            // the `npc-normal-base` hierarchy, the player models the
            // `proto.pak` rig) plays with that rig's joints; the entity binds
            // them by joint name.
            let hierarchy = crate::anim::hierarchy(&loaded, at)?;
            if hierarchy != skeleton.hash {
                match Self::skeleton_for(session, hierarchy)? {
                    Some(other) => skeleton = other,
                    None => continue,
                }
            }
            let skeleton = &skeleton;
            let decoded = crate::anim::decode(&loaded, at, name, skeleton.animated)?;
            let channels = decoded.joints.iter().take(skeleton.names.len()).filter(|j| j.rotation.is_some() || j.translation.is_some() || j.scale.is_some()).count();
            anyhow::ensure!(channels > 0, "clip {name} animates none of the {} joints (unsupported {:?})", skeleton.names.len(), decoded.unsupported);
            let seconds = if decoded.frames > 1 { decoded.frames as f32 / decoded.fps.max(1.) } else { 0. };
            let glb = scene::animation_library(&scene::Rig { skeleton, clips: std::slice::from_ref(&decoded) }).to_glb()?;
            session.next_model += 1;
            let asset = host.upload_model(self.client, 0x4e44_0000 + session.next_model, &glb)?;
            result = Some(LibraryClip { asset: asset.relative().to_string(), name: decoded.name.clone(), seconds, fps: decoded.fps.max(1.), align: decoded.align.clone(), ap_reference: decoded.ap_reference.clone() });
            break;
        }
        session.libraries.insert(key, result.clone());
        result.with_context(|| format!("clip not in {}", packages.join(", ")))
    }

    /// The player skeleton from `actor27.psarc/proto.pak` and the clips
    /// named `wanted` (the controller states' blend-tree clips).
    fn player_rig(actors: &ArchiveMount, wanted: &HashSet<u32>, mirrored: &HashSet<u32>) -> Result<Option<(crate::skeleton::Skeleton, Vec<crate::anim::Clip>)>> {
        if !actors.exists("proto.pak") {
            return Ok(None);
        }
        let read = |name: &str| -> Result<crate::mesh::Loaded> {
            let package = crysirror_core::service::MemoryAssetReader(read_all(actors.open(name)?.as_ref(), PACKAGE_LIMIT)?.into());
            crate::mesh::Loaded::read(&package, PACKAGE_LIMIT)
        };
        let Some(skeleton) = crate::skeleton::of_package(&read("proto.pak")?)? else { return Ok(None) };
        let mut clips = Vec::new();
        for package in PLAYER_CLIP_PACKAGES {
            if !actors.exists(package) {
                continue;
            }
            let loaded = read(package)?;
            for (n, at) in crate::anim::items(&loaded).filter(|(n, _)| wanted.contains(&sid(n))) {
                if clips.iter().any(|c: &crate::anim::Clip| c.name == n) {
                    continue;
                }
                let clip = crate::anim::decode(&loaded, at, n, skeleton.animated)?;
                if clip.hierarchy == skeleton.hash {
                    if mirrored.contains(&sid(n)) {
                        clips.push(crate::anim::mirror(&clip, &skeleton));
                    }
                    clips.push(clip);
                }
            }
        }
        Ok(Some((skeleton, clips)))
    }

    /// The English title logo of `menu.pak` (`logo2`, alpha from
    /// `logo2-alpha`), uploaded once.
    fn menu_logo(host: &mut dyn HostServices, client: ClientId, mount: &ArchiveMount) -> Result<Option<(VfsPath, u32, u32)>> {
        let Some(path) = package_path(mount, "menu") else { return Ok(None) };
        let package = crysirror_core::service::MemoryAssetReader(read_all(mount.open(&path)?.as_ref(), PACKAGE_LIMIT)?.into());
        let directory = Directory::read(&package)?;
        let mut colour = None;
        let mut alpha = None;
        for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
            let d = crate::texture::VramDesc::read(&package, item.payload_offset)?;
            if d.name.contains("/menu/logo2.tga") {
                colour = Some(d.decode(&package, directory.texture_base)?);
            } else if d.name.contains("/menu/logo2-alpha.tga") {
                alpha = Some(d.decode(&package, directory.texture_base)?);
            }
        }
        let (Some(mut c), Some(a)) = (colour, alpha) else { return Ok(None) };
        ensure!(c.width == a.width && c.height == a.height, "logo mask size");
        for (px, m) in c.rgba.chunks_exact_mut(4).zip(a.rgba.chunks_exact(4)) {
            px[3] = m[0];
        }
        // The logo is stored bottom row first (the menu board's texture
        // coordinates flip it): "AMONG THIEVES" sits above the title
        // otherwise.
        let row = c.width as usize * 4;
        let flipped: Vec<u8> = c.rgba.chunks_exact(row).rev().flatten().copied().collect();
        c.rgba = flipped;
        let asset = host.upload_rgba(client, 0x4e44_4c00, c.width, c.height, &c.rgba)?;
        Ok(Some((asset, c.width, c.height)))
    }

    fn change_task(&self, host: &mut dyn HostServices, session: &mut Session, at: (usize, usize)) -> Result<()> {
        let Some(game) = session.game.as_mut() else { return Ok(()) };
        let name = game.rt.inner.tasks.nodes[at.0].subnodes[at.1].name.clone();
        session.stage.log(format!("task change to {name}"));
        let Some(index) = session.tasks.iter().position(|t| *t == name) else { return Ok(()) };
        session.current = index;
        let plan = Self::plan(session)?;
        if session.plan.as_ref().is_some_and(|p| p.levels == plan.levels) {
            session.plan = Some(plan);
            session.ui = None;
            return self.enter_task(host, session);
        }
        self.begin(host, session)
    }

    /// Evaluate load regions for the current task: each region's scheme
    /// lambda runs in the script VM (the package is mapped into its address
    /// space) and the scheme's displayed levels are queued.
    fn apply_load_regions(&self, session: &mut Session, package: &str, loaded: &crate::mesh::Loaded, regions: &[crate::region::LoadRegion]) -> Result<()> {
        let Some(game) = session.game.as_mut() else { return Ok(()) };
        let base = game.rt.dc.map_package(sid(package), &loaded.data, loaded.pointers())?;
        for region in regions {
            let scheme = match region.scheme {
                Some(off) => game.rt.run_lambda(&mut session.stage, base + u32::try_from(off)?, &[])?,
                None => 1,
            };
            let (shown, hidden) = crate::region::levels(region, scheme);
            let plan = session.plan.as_mut().context("task plan")?;
            for level in shown.iter().chain(&hidden) {
                session.stage.levels.insert(sid(level));
            }
            for level in &shown {
                if plan.levels.contains(level) {
                    continue;
                }
                plan.levels.push(level.clone());
                for p in session.all_levels.get(level).map(|l| l.packages.as_slice()).unwrap_or_default() {
                    let queued = session.pending.iter().any(|l| matches!(l, Load::Package(x) if x == p));
                    if !queued && !session.packages.contains(p) {
                        session.pending.push_back(Load::Package(p.clone()));
                    }
                }
            }
            session.stage.log(format!("load region {} scheme {scheme}: displayed {shown:?}, not displayed {hidden:?}", region.name));
            plan.regions.push((region.name.clone(), scheme, shown, hidden));
        }
        Ok(())
    }

    /// Third-person camera behind and above the player, pulled in front of
    /// the first surface between the player's head and the camera at once
    /// and easing back out (2 m/s).
    fn follow(p: &mut Player, ground: &Ground, seconds: f32) -> serde_json::Value {
        let target = [p.position[0], p.position[1] + 1.5, p.position[2]];
        let far = FOLLOW_DISTANCE;
        // Blocked closer than 2.5 m (inside rooms or the hanging car), the
        // camera rises over the player toward the clearest pitch.
        let free_at = |yaw: f32, pitch: f32| {
            let cy = pitch.cos();
            let dir = [-yaw.sin() * cy, -pitch.sin(), yaw.cos() * cy];
            let want: [f32; 3] = std::array::from_fn(|k| target[k] + dir[k] * far);
            (dir, ground.raycast(target, want).map_or(far, |t| (t * far - 0.3).max(0.5)))
        };
        let (mut dir, mut free) = free_at(p.view_yaw, p.view_pitch);
        // Then over the shoulders, then rising; the view yaw follows a
        // sideways choice so the camera does not swing back next frame.
        if free < 2.5 {
            let mut best = (0., p.view_pitch);
            'search: for k in 0..=4 {
                let pitch = (p.view_pitch - 0.3 * k as f32).max(-1.4);
                for side in [0., 0.5, -0.5, 1., -1.] {
                    let (d, f) = free_at(p.view_yaw + side, pitch);
                    if f > free + 0.25 {
                        (dir, free, best) = (d, f, (side, pitch));
                    }
                    if free >= 2.5 {
                        break 'search;
                    }
                }
            }
            p.view_yaw += best.0 * seconds.min(1.) * 2.;
        }
        p.view_distance = if free < p.view_distance { free } else { (p.view_distance + 2. * seconds).min(free) };
        let d = p.view_distance;
        let eye: [f32; 3] = std::array::from_fn(|k| target[k] + dir[k] * d);
        json!({"op": "set_camera", "camera": {"eye": eye, "target": target, "up": [0.0, 1.0, 0.0], "fov_y_degrees": 60.0, "near": 0.1, "far": 20000.0}})
    }

    /// Player movement on the walkable surfaces, the script tick and the
    /// presentation of their effects.
    fn play(&self, host: &mut dyn HostServices, session: &mut Session, input: &InputFrame, seconds: f32) -> Result<()> {
        let mut world = Vec::new();
        let mut layers = Vec::new();
        session.stage.now = (host.now_ms().saturating_sub(session.start_ms)) as f64 / 1000.;
        let key = |m: u64| f32::from(u8::from(input.buttons_down & m != 0));
        session.stage.stick = [key(b::KEY_D) - key(b::KEY_A), key(b::KEY_W) - key(b::KEY_S)];
        if let Some(c) = session.stage.controls.take() {
            if let Some(p) = &mut session.player {
                p.controls = c;
            }
        }
        if input.buttons_pressed & b::F1 != 0 && session.player.is_some() {
            session.free = !session.free;
            if session.free {
                let p = session.player.as_ref().expect("player");
                session.camera = Camera { eye: [p.position[0], p.position[1] + 2., p.position[2]], yaw: p.view_yaw, pitch: p.view_pitch, speed: 5. };
            }
        }
        if session.free {
            session.camera.step(input, seconds);
            world.push(session.camera.op());
        } else if let Some(p) = &mut session.player {
            let down = |m| input.buttons_down & m != 0;
            let turn = 1.8 * seconds;
            if down(b::LEFT) {
                p.view_yaw -= turn;
            }
            if down(b::RIGHT) {
                p.view_yaw += turn;
            }
            if down(b::UP) {
                p.view_pitch = (p.view_pitch - turn).max(-1.2);
            }
            if down(b::DOWN) {
                p.view_pitch = (p.view_pitch + turn).min(0.6);
            }
            let (fx, fz) = (p.view_yaw.sin(), -p.view_yaw.cos());
            let mut mv = [0f32; 2];
            if p.controls {
                for (key, dx, dz) in [(b::KEY_W, fx, fz), (b::KEY_S, -fx, -fz), (b::KEY_D, -fz, fx), (b::KEY_A, fz, -fx)] {
                    if down(key) {
                        mv[0] += dx;
                        mv[1] += dz;
                    }
                }
            }
            let len = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
            let heading = (len > 0.).then(|| mv[0].atan2(mv[1]));
            // Ride the platform's motion since the last frame.
            if let Some((id, before)) = p.platform {
                if let Some(now) = session.ground.body_transform(id) {
                    let delta = now * before.inverse();
                    p.position = delta.transform_point3(glam::Vec3::from_array(p.position)).to_array();
                    let f = delta.transform_vector3(glam::Vec3::new(p.yaw.sin(), 0., p.yaw.cos()));
                    let turn = f.x.atan2(f.z) - p.yaw;
                    if turn.abs() > 1e-6 {
                        p.yaw += turn;
                        p.view_yaw -= turn;
                        if let Some(c) = p.anim.as_mut() {
                            c.yaw = p.yaw;
                        }
                    }
                    p.platform = Some((id, now));
                }
            }
            let movement = session.game.as_ref().map_or([0; 2], |g| g.rt.inner.movement);
            let below = session.ground.height(p.position[0], p.position[2], p.position[1] + 0.6);
            let grounded = p.fall >= 0. && below.is_some_and(|h| h >= p.position[1] - p.fall.max(0.) * seconds - 0.05);
            let dropped = p.air_top.map_or(0., |top| top - p.position[1]);
            let ahead = |d: f32| session.ground.height(p.position[0] + p.yaw.sin() * d, p.position[2] + p.yaw.cos() * d, p.position[1] + 0.6);
            let slope = match (below, ahead(0.5)) {
                (Some(h0), Some(h1)) if grounded => (h1 - h0) / 0.5,
                _ => 0.,
            };
            // Hanging: sideways input along the edge, forward input over a
            // standable top (within 0.35 m of the edge height, 0.45 m in),
            // the drop key (C). Airborne: an edge in reach of the hands.
            let mode = p.anim.as_ref().zip(session.anim_table.as_ref()).map_or(crate::player::Mode::Ground, |(a, t)| a.mode(t));
            let facing = glam::Vec3::new(p.yaw.sin(), 0., p.yaw.cos());
            let right = glam::Vec3::new(-facing.z, 0., facing.x);
            let stick = if len > 0. { glam::Vec3::new(mv[0], 0., mv[1]) / len } else { glam::Vec3::ZERO };
            let (mut lateral, mut climb, mut release) = (0., false, false);
            let mut leap_to = None;
            p.regrab = (p.regrab - seconds).max(0.);
            if mode == crate::player::Mode::Hang {
                if let Some(edge) = p.grip.and_then(|g| session.ground.edge(g.body, g.edge).map(|e| (g, e))).map(|(g, e)| edge_frame(&e, g.s)) {
                    let probe = edge.transform_point3(glam::Vec3::new(0., 0., 0.45));
                    let top = session.ground.height(probe.x, probe.z, probe.y + 0.35).filter(|h| (h - probe.y).abs() < 0.35);
                    lateral = stick.dot(right);
                    climb = stick.dot(facing) > 0.7 && top.is_some();
                    // Jump: to an edge above, or to the side the stick
                    // points.
                    if input.buttons_pressed & b::SPACE != 0 {
                        let toward = if lateral.abs() > 0.5 { right * lateral.signum() } else { glam::Vec3::Y };
                        leap_to = p.grip.and_then(|g| Self::leap_target(&session.ground, g, toward));
                    }
                }
                release = input.buttons_pressed & b::KEY_C != 0 || p.grip.is_some_and(|g| session.ground.edge(g.body, g.edge).is_none());
            }
            let reach = glam::Vec3::from_array(p.position) + glam::Quat::from_rotation_y(p.yaw) * HANG_REACH;
            let candidate = if mode == crate::player::Mode::Ground && p.regrab <= 0. && p.fall > -1. && (p.air_top.is_some() || p.fall > 0.) {
                session
                    .ground
                    .edges_near(reach, 0.45)
                    .into_iter()
                    .filter(|(_, e, _)| glam::Vec3::new(-e.n1.x, 0., -e.n1.z).normalize_or_zero().dot(facing) > 0.3)
                    .min_by(|a, b| crate::ground::segment_distance(reach, a.1.p0, a.1.p1).total_cmp(&crate::ground::segment_distance(reach, b.1.p0, b.1.p1)))
            } else {
                None
            };
            let mut ground_speed = None;
            let mut step_mode = mode;
            let mut step_root = None;
            if let (Some(anim), Some(table), Some(graph), Some(id)) = (p.anim.as_mut(), session.anim_table.as_ref(), session.anim_graph.as_mut(), p.entity) {
                let running = down(b::SHIFT);
                let wanted = movement[usize::from(running)];
                let wanted = if wanted == 0 { sid(if running { "run" } else { "walk" }) } else { wanted };
                let frame = crate::player::Frame {
                    heading,
                    jump: input.buttons_pressed & b::SPACE != 0 && p.controls,
                    grounded,
                    dropped,
                    slope,
                    movement: graph.movement_index(wanted).or_else(|| graph.movement_index(sid("run"))).unwrap_or(0),
                    weapon: graph.weapon_index(sid("no-weapon")).unwrap_or(1),
                    grab: candidate.is_some(),
                    lateral,
                    climb,
                    drop: release,
                    leap: leap_to.is_some(),
                    seconds,
                };
                match anim.step(graph, table, &session.player_clips, &frame) {
                    Ok(step) => {
                        step_mode = step.mode;
                        step_root = step.root.clone();
                        if let Some(state) = &step.state {
                            session.stage.log(format!("player state {state} at {:?} yaw {:.2}: {}", p.position.map(|v| (v * 100.).round() / 100.), anim.yaw, step.layers.iter().map(|l| format!("{}×{:.2}", l.clip, l.weight)).collect::<Vec<_>>().join(" ")));
                        }
                        if let Some(lift) = step.lift {
                            p.fall = -lift;
                        }
                        ground_speed = Some(step.speed);
                        p.yaw = anim.yaw;
                        let source = session.rig_libraries.get(&format!("{:?}", RigRef::Player));
                        let layers: Vec<_> = step.layers.iter().map(|l| json!({"clip": l.clip, "source": source, "time": l.time, "speed": 0.0, "weight": l.weight, "target_weight": l.weight, "looping": true, "additive": l.additive})).collect();
                        world.push(json!({"op": "set_animation", "id": id, "animation": {"layers": layers}}));
                    }
                    Err(e) => session.stage.log(format!("player animation: {e:#}")),
                }
            }
            // A jump between edges eases the root from where it started
            // to the target edge's hang (over `LEAP_SECONDS`).
            if step_mode == crate::player::Mode::Leap {
                if p.leap.is_none() {
                    p.leap = leap_to.map(|g| (glam::Vec3::from_array(p.position), g, 0.));
                }
                if let Some((from, to, t)) = p.leap.as_mut() {
                    *t += seconds;
                    if let Some(edge) = session.ground.edge(to.body, to.edge) {
                        let target = edge_frame(&edge, to.s) * glam::Mat4::from_translation(HANG_REACH).inverse();
                        let x = (*t / LEAP_SECONDS).clamp(0., 1.);
                        let ease = x * x * (3. - 2. * x);
                        p.position = from.lerp(target.to_scale_rotation_translation().2, ease).to_array();
                    }
                }
            } else if let Some((_, to, _)) = p.leap.take() {
                p.grip = Some(to);
            }
            if step_mode == crate::player::Mode::Ground && p.grip.is_some() && mode == crate::player::Mode::Hang {
                p.regrab = 0.4;
            }
            // On an edge the root follows the grip, not the ground.
            if step_mode != crate::player::Mode::Ground && step_mode != crate::player::Mode::Leap {
                if p.grip.is_none() {
                    // A grab (or a hang without a grip): the nearest edge
                    // in reach.
                    let found = candidate.or_else(|| session.ground.edges_near(reach, 0.6).into_iter().min_by(|a, b| crate::ground::segment_distance(reach, a.1.p0, a.1.p1).total_cmp(&crate::ground::segment_distance(reach, b.1.p0, b.1.p1))));
                    p.grip = found.map(|(edge, e, body)| Grip { body, edge, s: (reach - e.p0).dot((e.p1 - e.p0).normalize_or_zero()).clamp(0., (e.p1 - e.p0).length()), climb: None });
                }
                Self::follow_grip(p, &session.ground, step_mode, step_root.as_ref());
            } else if p.grip.take().is_some() {
                p.air_top = None;
            }
            let on_edge = step_mode != crate::player::Mode::Ground && (p.grip.is_some() || p.leap.is_some());
            let speed = match ground_speed {
                _ if on_edge => 0.,
                Some(v) => v,
                None if len > 0. => {
                    p.yaw = mv[0].atan2(mv[1]);
                    if down(b::SHIFT) { 6. } else { 2.5 }
                }
                None => 0.,
            };
            if speed > 0.01 {
                let step = speed * seconds;
                let (dx, dz) = (p.yaw.sin() * step, p.yaw.cos() * step);
                // Walls stop the player (rays at knee and chest height,
                // 0.3 m ahead); blocked moves slide along one axis. Step up
                // at most 0.6 m; a missing surface ahead blocks.
                for (mx, mz) in [(dx, dz), (dx, 0.), (0., dz)] {
                    if mx == 0. && mz == 0. {
                        continue;
                    }
                    let next = [p.position[0] + mx, p.position[2] + mz];
                    let len = (mx * mx + mz * mz).sqrt();
                    let reach = (len + 0.3) / len;
                    let wall = [0.4, 1.2].iter().any(|h| {
                        let from = [p.position[0], p.position[1] + h, p.position[2]];
                        session.ground.raycast(from, [from[0] + mx * reach, from[1], from[2] + mz * reach]).is_some()
                    });
                    let floor = session.ground.height(next[0], next[1], p.position[1] + 0.6);
                    if !wall && (floor.is_some_and(|h| h > p.position[1] - 3. || !grounded) || session.ground.is_empty()) {
                        p.position[0] = next[0];
                        p.position[2] = next[1];
                        break;
                    }
                }
            }
            let support = if on_edge { None } else { session.ground.support(p.position[0], p.position[2], p.position[1] + 0.6) };
            p.platform = None;
            match support.map(|(h, _)| h) {
                Some(h) if p.fall >= 0. && h >= p.position[1] - p.fall * seconds - 0.05 => {
                    p.position[1] = h;
                    p.fall = 0.;
                    p.air_top = None;
                    p.platform = support.and_then(|(_, body)| body).and_then(|b| session.ground.body_transform(b).map(|m| (b, m)));
                }
                // Climbing and hanging are not implemented: standing still
                // above a drop of more than 8 m (the hanging car at the
                // start of `train-wreck-climb`) holds the player in place.
                Some(h) if p.fall == 0. && p.air_top.is_none() && p.position[1] - h > 8. => {}
                Some(_) => {
                    let top = p.air_top.get_or_insert(p.position[1]);
                    *top = top.max(p.position[1]);
                    p.fall = (p.fall + 9.8 * seconds).min(30.);
                    p.position[1] -= p.fall * seconds;
                }
                None => {}
            }
            if let Some(id) = p.entity {
                world.push(json!({"op": "set_transform", "id": id, "transform": {"translation": p.position, "rotation_degrees": [0.0, p.yaw.to_degrees(), 0.0], "scale": [1.0, 1.0, 1.0]}}));
            }
            // The camera drifts behind a player moving away from it unless
            // the view keys turn it.
            if heading.is_some() && !(down(b::LEFT) || down(b::RIGHT)) {
                let behind = std::f32::consts::PI - p.yaw;
                let d = (behind - p.view_yaw + std::f32::consts::PI).rem_euclid(2. * std::f32::consts::PI) - std::f32::consts::PI;
                if d.abs() < 2. {
                    p.view_yaw += d * (1.5 * seconds).min(1.);
                }
            }
            // A camera the scripts enabled (`cam-…` spawners) replaces the
            // follow camera.
            // Cameras parented to a moved object follow its pose.
            let scripted = session.stage.camera.and_then(|c| {
                let mut cam = session.stage.spawners.get(&c)?.clone();
                if let Some(m) = session.stage.poses.get(&c) {
                    let (_, r, t) = m.to_scale_rotation_translation();
                    (cam.position, cam.rotation) = (t.to_array(), r.to_array());
                }
                Some(cam)
            });
            let mut camera = match scripted.as_ref() {
                Some(cam) => Self::spawner_camera(cam),
                None => Self::follow(p, &session.ground, seconds),
            };
            Self::animate_camera(session, &mut camera);
            let eye = camera["camera"]["eye"].as_array().map(|a| [0, 1, 2].map(|k| a.get(k).and_then(|v| v.as_f64()).unwrap_or(0.) as f32));
            // Crossing into another render-settings region changes the
            // environment.
            let before = Self::render_settings_name(session);
            session.stage.camera_eye = eye;
            let vector = |k: &str| camera["camera"][k].as_array().map(|a| [0, 1, 2].map(|i| a.get(i).and_then(|v| v.as_f64()).unwrap_or(0.) as f32));
            session.stage.camera_view = match (eye, vector("target"), vector("up"), camera["camera"]["fov_y_degrees"].as_f64()) {
                (Some(e), Some(t), Some(u), Some(fov)) => Some((e, [t[0] - e[0], t[1] - e[1], t[2] - e[2]], u, fov as f32)),
                _ => None,
            };
            Self::serve_particle_requests(session, &mut world);
            Self::update_effects(session, &mut world);
            if Self::render_settings_name(session) != before {
                if let Some(env) = Self::level_environment(session) {
                    world.push(env);
                }
            }
            world.push(camera);
        }
        session.stage.player_locator = session.player.as_ref().map(|p| (p.position, glam::Quat::from_rotation_y(p.yaw).to_array()));
        session.stage.player_support = session.player.as_ref().and_then(|p| p.platform.map(|(b, _)| b).or(p.grip.and_then(|g| g.body)));
        if let Some((graph, table)) = session.npc_anim.as_mut() {
            for npc in &mut session.npcs {
                let Some(clips) = session.npc_clips.get(&npc.rig) else { continue };
                let before = npc.animator.machine.state;
                let sets = &session.anim_sets;
                let chain = &npc.sets;
                match npc.animator.present(graph, table, clips, &|c| sets.resolve(chain, c), npc_inputs(), before, seconds) {
                    Ok((layers, _)) => {
                        let source = session.rig_libraries.get(&format!("{:?}", RigRef::Npc(npc.rig.clone())));
                        let layers: Vec<_> = layers.iter().map(|l| json!({"clip": l.clip, "source": source, "time": l.time, "speed": 0.0, "weight": l.weight, "target_weight": l.weight, "looping": true, "additive": l.additive})).collect();
                        world.push(json!({"op": "set_animation", "id": npc.entity, "animation": {"layers": layers}}));
                    }
                    Err(e) => session.stage.log(format!("npc animation: {e:#}")),
                }
            }
        }
        // Region enter/exit scripts for the player (`sub_115828`).
        if session.player.is_some() {
            let player = sid("player");
            let now: HashSet<usize> = (0..session.stage.regions.len()).filter(|i| session.stage.holds(&session.stage.regions[*i].0, player)).collect();
            let mut runs = Vec::new();
            for i in now.difference(&session.stage.player_regions) {
                let (r, enter, _) = &session.stage.regions[*i];
                runs.push((format!("enter {}", r.name), *enter));
            }
            for i in session.stage.player_regions.difference(&now) {
                let (r, _, exit) = &session.stage.regions[*i];
                runs.push((format!("exit {}", r.name), *exit));
            }
            session.stage.player_regions = now;
            if let Some(game) = session.game.as_mut() {
                for (what, lambda) in runs {
                    session.stage.log(format!("region {what}"));
                    if let Some(lambda) = lambda {
                        if let Err(e) = game.rt.run_lambda(&mut session.stage, lambda, &[]) {
                            session.stage.log(format!("region {what}: {e:#}"));
                        }
                    }
                }
            }
        }
        if let (Some(game), Some(modules)) = (session.game.as_mut(), session.modules.as_ref()) {
            if session.script_error.is_none() {
                let ticked = game.rt.tick(&mut session.stage).and_then(|_| game.load_requested(modules, &mut session.stage));
                if let Err(e) = ticked {
                    session.script_error = Some(format!("{e:#}"));
                }
            }
        }
        self.script_animations(host, session, &mut world);
        session.stage.player_locator = session.player.as_ref().map(|p| (p.position, glam::Quat::from_rotation_y(p.yaw).to_array()));
        session.stage.publish_lights();
        // Props collide while spawned.
        for (name, ids) in &session.stage.entities {
            let on = session.stage.visible.contains(name);
            for id in ids {
                session.ground.enable_body(*id, on);
            }
        }
        world.append(&mut session.stage.ops);
        let skip = input.buttons_pressed & (b::ENTER | b::ESCAPE) != 0;
        let mut change = session.game.as_mut().and_then(|g| g.rt.inner.task_change.take());
        // Boot screens: Enter dismisses a message; the menu moves with
        // Up/Down and New Game starts the story.
        let pressed = |m: u64| input.buttons_pressed & m != 0;
        let mut ui_changed = false;
        match &mut session.ui {
            Some(Ui::Message(_)) if pressed(b::ENTER) => {
                session.ui = None;
                ui_changed = true;
                let at = session.game.as_ref().and_then(|g| g.rt.inner.task);
                change = change.or_else(|| at.and_then(|at| game_next(session, at)));
            }
            Some(Ui::Menu { stack, selected }) => {
                let page = stack.last().and_then(|i| session.menu_pages.get(*i));
                let visible: Vec<String> = page.map(|p| p.buttons.iter().filter(|b| b.condition == 0).map(|b| b.name.clone()).collect()).unwrap_or_default();
                let n = visible.len().max(1);
                if pressed(b::DOWN) {
                    *selected = (*selected + 1) % n;
                    ui_changed = true;
                }
                if pressed(b::UP) {
                    *selected = (*selected + n - 1) % n;
                    ui_changed = true;
                }
                if pressed(b::ESCAPE) && stack.len() > 1 {
                    stack.pop();
                    *selected = 0;
                    ui_changed = true;
                }
                if pressed(b::ENTER) {
                    match visible.get(*selected).map(|b| menu_action(b)) {
                        Some(MenuAction::Page(target)) => {
                            if let Some(i) = session.menu_pages.iter().position(|p| p.name == target) {
                                stack.push(i);
                                // The difficulty page opens on Normal.
                                *selected = session.menu_pages[i].buttons.iter().filter(|b| b.condition == 0).position(|b| b.name.ends_with("_NORMAL")).unwrap_or(0);
                                ui_changed = true;
                            }
                        }
                        Some(MenuAction::NewGame) => {
                            session.ui = None;
                            ui_changed = true;
                            if let Some(game) = session.game.as_ref() {
                                change = game.story_task().and_then(|t| game.rt.inner.tasks.find(sid(&t)));
                            }
                        }
                        Some(MenuAction::None) | None => {}
                    }
                }
            }
            _ => {}
        }
        if ui_changed {
            layers.extend(Self::ui_layers(session));
        }
        if let Some(end) = session.movie {
            if skip || session.stage.now >= end {
                session.movie = None;
                if std::mem::take(&mut session.movie_video) {
                    world.push(json!({"op": "set_video", "name": "movie", "video": null}));
                    layers.push(json!({"op": "remove", "id": MOVIE_LAYER}));
                }
                if let Some(game) = session.game.as_mut() {
                    if let Some(at) = game.rt.inner.task {
                        let record = game.rt.dc.mem.u32(game.rt.inner.tasks.nodes[at.0].subnodes[at.1].address + 0x1c)?;
                        if record != 0 {
                            game.rt.dc.mem.write_u32(record, 3)?;
                        }
                        change = change.or(game.rt.inner.tasks.next(at));
                    }
                }
            }
        }
        let st = &mut session.stage;
        if st.fade != st.fade_target {
            let step = if st.fade_rate > 0. { st.fade_rate * seconds } else { 1. };
            st.fade = if st.fade < st.fade_target { (st.fade + step).min(st.fade_target) } else { (st.fade - step).max(st.fade_target) };
            let [r, g, b] = st.fade_color;
            layers.push(json!({"op": "set_solid_color", "id": FADE_LAYER, "color": [r, g, b, (st.fade * 255.).round() as u8]}));
        }
        host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": layers, "world": world}))?)?;
        if let Some(at) = change {
            self.change_task(host, session, at)?;
        }
        Ok(())
    }
}

impl NaughtyDogVm {
    /// Upload an art group's meshes once and spawn an entity per spawner.
    /// An art group without an installed package or render meshes is
    /// skipped. In task mode no-auto-birth spawners start hidden.
    fn load_actor(&self, host: &mut dyn HostServices, session: &mut Session, art: &str, spawners: &[SpawnerRecord], world: &mut Vec<serde_json::Value>) -> Result<()> {
        let path = format!("{art}.pak");
        if !session.actors.exists(&path) {
            return Ok(());
        }
        let package: Arc<dyn AssetReader> = Arc::new(crysirror_core::service::MemoryAssetReader(read_all(session.actors.open(&path)?.as_ref(), PACKAGE_LIMIT)?.into()));
        let directory = Directory::read(package.as_ref())?;
        let loaded = crate::mesh::Loaded::read(package.as_ref(), PACKAGE_LIMIT)?;
        let mut meshes = scene::meshes_of(&loaded, &directory)?;
        if meshes.iter().all(|m| m.fragments.iter().all(|f| f.triangles.is_empty())) {
            return Ok(());
        }
        scene::place_art_group(&mut meshes);
        // Skinned models without their own skeleton bind to their
        // character's skeleton package (`spawn-character.bin`): the player
        // models to the `proto.pak` rig, NPCs to their `*-base` package.
        let skinned = meshes.iter().any(|m| m.skinned);
        let own = crate::skeleton::of_package(&loaded)?;
        let binding = if skinned && own.is_none() { crate::characters::rig_of(&session.characters, art) } else { None };
        if matches!(binding, Some((None, _))) && session.player_rig.is_none() {
            let mut wanted: HashSet<u32> = [sid("idle")].into();
            // Clips of states whose mirror lambda holds are also built
            // mirrored.
            let mut mirrored: HashSet<u32> = HashSet::new();
            if let (Some(graph), Some(table)) = (session.anim_graph.as_mut(), session.anim_table.as_ref()) {
                for state in crate::player::STATES {
                    if let Some(s) = table.get(sid(state)) {
                        wanted.extend(graph.clips(s.tree));
                        if graph.mirrored(s.mirror) {
                            mirrored.extend(graph.clips(s.tree));
                        }
                    }
                }
            }
            session.player_rig = Some(Self::player_rig(&session.actors, &wanted, &mirrored).unwrap_or_else(|e| {
                session.stage.log(format!("player rig: {e:#}"));
                None
            }));
            if let Some(Some((skeleton, clips))) = &session.player_rig {
                session.stage.joints.insert(sid("player"), joint_frames(skeleton));
                session.player_clips = clip_infos(clips);
                session.stage.log(format!("player rig: {} of {} blend-tree clips", clips.len(), wanted.len()));
            }
        }
        // An NPC's clips are its anim set's mapping of the NPC states'
        // tree leaves (`*NAME-anim-set*` by art group, its dash prefixes, then
        // character: `flynn-heist`, `flynn`).
        let character = crate::characters::character_of(&session.characters, art).map(|c| c.name.clone());
        let prefixes: Vec<&str> = art.match_indices('-').map(|(i, _)| &art[..i]).rev().collect();
        let npc_sets = session.anim_sets.sets_for([art].into_iter().chain(prefixes).chain(character.as_deref()));
        let npc_key = binding.as_ref().and_then(|b| b.0.as_ref()).map(|base| format!("{base}:{:08x?}", npc_sets));
        if let (Some((Some(base), anims)), Some(key)) = (&binding, &npc_key) {
            if !session.npc_rigs.contains_key(key) {
                let mut wanted = HashSet::new();
                if let Some((graph, table)) = &session.npc_anim {
                    for state in NPC_STATES {
                        if let Some(s) = table.get(sid(state)) {
                            wanted.extend(graph.clips(s.tree).into_iter().map(|c| session.anim_sets.resolve(&npc_sets, c)));
                        }
                    }
                }
                let rig = Self::npc_rig(&session.actors, base, anims, &wanted).unwrap_or_else(|e| {
                    session.stage.log(format!("rig {base}: {e:#}"));
                    None
                });
                if let Some((_, clips)) = &rig {
                    session.npc_clips.insert(key.clone(), clip_infos(clips));
                }
                session.npc_packages.insert(key.clone(), std::iter::once(format!("{base}.pak")).chain(anims.iter().map(|a| format!("{a}.pak"))).collect());
                session.npc_rigs.insert(key.clone(), rig);
            }
        }
        if let (Some(skeleton), true) = (&own, skinned) {
            session.own_rigs.insert(art.to_owned(), skeleton.clone());
        }
        let rig_ref = match (&binding, &own) {
            (Some((None, _)), _) => Some(RigRef::Player),
            (Some((Some(_), _)), _) => npc_key.clone().map(RigRef::Npc),
            (None, Some(_)) if skinned => Some(RigRef::Own(art.to_owned())),
            _ => None,
        };
        let rig = match (&binding, &session.player_rig) {
            (Some((None, _)), Some(Some((skeleton, clips)))) => Some(scene::Rig { skeleton, clips }),
            (Some((Some(_), _)), _) => npc_key.as_ref().and_then(|k| session.npc_rigs.get(k)).and_then(|r| r.as_ref()).map(|(skeleton, clips)| scene::Rig { skeleton, clips }),
            // Skinned art groups with their own skeleton (doors, props)
            // get its joints for script animations.
            _ => own.as_ref().filter(|_| skinned).map(|skeleton| scene::Rig { skeleton, clips: &[] }),
        };
        let idle = rig.as_ref().and_then(|r| r.clips.iter().find(|c| c.name == "idle")).map(|c| c.name.clone());
        // The rig's clips go to a shared library once; the model gets the
        // joints and skin only.
        let library = match (&rig, &rig_ref) {
            (Some(r), Some(rr)) if !r.clips.is_empty() => {
                let key = format!("{rr:?}");
                if !session.rig_libraries.contains_key(&key) {
                    let glb = scene::animation_library(r).to_glb()?;
                    session.next_model += 1;
                    let asset = host.upload_model(self.client, 0x4e44_0000 + session.next_model, &glb)?;
                    session.stage.log(format!("rig library {key}: {} clips, {} KiB", r.clips.len(), glb.len() / 1024));
                    session.rig_libraries.insert(key.clone(), asset.relative().to_string());
                }
                session.rig_libraries.get(&key).cloned()
            }
            _ => None,
        };
        let rig = rig.map(|r| scene::Rig { skeleton: r.skeleton, clips: &[] });
        if skinned && own.is_none() {
            session.stage.log(format!("art {art}: rig {:?}, {} clips, anim set {:?}", binding.as_ref().map(|b| b.0.clone().unwrap_or_else(|| "proto".into())), rig.as_ref().map_or(0, |r| r.clips.len()), npc_sets.iter().map(|s| format!("{s:08x}")).collect::<Vec<_>>()));
        }
        let glb = scene::document_lit(art, &meshes, package.as_ref(), &directory, MAX_TEXTURE, None, rig.as_ref())?.to_glb()?;
        session.next_model += 1;
        let asset = host.upload_model(self.client, 0x4e44_0000 + session.next_model, &glb)?;
        let triangles: usize = meshes.iter().flat_map(|m| &m.fragments).map(|f| f.triangles.len()).sum();
        let task = session.game.is_some();
        // Native prop collision in the art-group frame (bind pose of the
        // collision joints) and its climb edges.
        let bind = |j: usize| own.as_ref().and_then(|s| s.inverse_bind.get(j)).map(|m| glam::Mat4::from_cols_array(m).inverse());
        let collision = if task && binding.is_none() {
            crate::collision::prop_objects(&loaded, &bind).unwrap_or_else(|e| {
                session.stage.log(format!("art {art}: collision: {e:#}"));
                Vec::new()
            })
        } else {
            Vec::new()
        };
        let collision_triangles: Vec<_> = collision.iter().flat_map(|o| o.triangles.iter().copied()).collect();
        let collision_edges: Vec<_> = collision.iter().flat_map(|o| o.features.edges.iter().copied()).collect();
        for s in spawners {
            session.next_model += 1;
            let id = 0x100 + session.next_model;
            let q = glam::Quat::from_array(s.rotation).normalize();
            let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
            let visible = !task || s.flags & NO_AUTO_BIRTH == 0;
            // Props collide (as movable bodies); characters do not.
            if task && binding.is_none() {
                let m = glam::Mat4::from_rotation_translation(q, glam::Vec3::from_array(s.position));
                if collision_triangles.is_empty() {
                    session.ground.add_body(id, &meshes, m.to_cols_array(), visible);
                } else {
                    session.ground.add_body_collision(id, &collision_triangles, &collision_edges, m.to_cols_array(), visible);
                }
            }
            world.push(json!({"op": "spawn", "entity": {"id": id, "name": format!("{art}:{}", s.name), "visible": visible, "transform": {"translation": s.position, "rotation_degrees": [x.to_degrees(), y.to_degrees(), z.to_degrees()], "scale": [1.0, 1.0, 1.0]}, "geometry": {"kind": "scene", "asset": asset.relative()}}}));
            let driven = npc_key.as_ref().filter(|k| session.npc_clips.get(*k).is_some_and(|c| !c.is_empty()) && session.npc_anim.is_some());
            if let Some(key) = driven {
                let machine = crate::anim_state::Machine { state: sid(NPC_STATES[0]), time: 0., previous: None, fade: 0. };
                session.npcs.push(Npc { entity: id, animator: crate::player::Animator::new(machine), sets: npc_sets.clone(), rig: key.clone() });
                session.stage.npc_spawners.insert(sid(&s.name));
            } else if let (Some(clip), Some(source)) = (&idle, &library) {
                world.push(json!({"op": "set_animation", "id": id, "animation": {"layers": [{"clip": clip, "source": source, "looping": true}]}}));
            }
            session.entities.push((id, asset.clone()));
            if let Some(r) = &rig_ref {
                session.entity_rigs.insert(id, r.clone());
            }
            let name = sid(&s.name);
            if let Some(r) = &rig {
                session.stage.joints.insert(name, joint_frames(r.skeleton));
            }
            session.stage.entities.entry(name).or_default().push(id);
            if visible {
                session.stage.visible.insert(name);
            }
            session.triangles += triangles;
        }
        session.packages.push(format!("{art}×{}", spawners.len()));
        Ok(())
    }
}

impl VmAdapter for NaughtyDogVm {
    fn engine_id(&self) -> &'static str {
        ENGINE_ID
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(&mut self, host: &mut dyn HostServices, input: InputFrame, _budget: u32) -> crysirror_core::Result<VmYield> {
        let fail = |e: anyhow::Error| Error::Io(format!("{e:#}"));
        if self.state.is_none() {
            let session = self.boot(host).map_err(fail)?;
            self.state = Some(session);
            return Ok(VmYield::WaitUntil(host.now_ms() + FRAME_MS));
        }
        let mut session = self.state.take().expect("booted");
        // Page Down / Page Up: next / previous level or task subnode.
        let step = i32::from(input.buttons_pressed & b::PAGE_DOWN != 0) - i32::from(input.buttons_pressed & b::PAGE_UP != 0);
        if step != 0 {
            let n = if session.game.is_some() { session.tasks.len() } else { session.levels.len() } as i32;
            session.current = (session.current as i32 + step).rem_euclid(n) as usize;
            let begun = self.begin(host, &mut session);
            self.state = Some(session);
            begun.map_err(fail)?;
            return Ok(VmYield::WaitUntil(host.now_ms() + FRAME_MS));
        }
        if !session.pending.is_empty() {
            let loaded = self.load_next(host, &mut session);
            self.state = Some(session);
            loaded.map_err(fail)?;
            return Ok(VmYield::WaitUntil(host.now_ms() + FRAME_MS));
        }
        let now = host.now_ms();
        let seconds = (now.saturating_sub(session.last_ms)).min(100) as f32 / 1000.;
        session.last_ms = now;
        // A subnode without levels starts as soon as its (empty) load is
        // done.
        if !session.started && session.game.is_some() {
            let mut world = Vec::new();
            let started = self.start_task(&mut session, &mut world).and_then(|_| {
                host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": [], "world": world}))?)?;
                self.enter_task(host, &mut session)
            });
            self.state = Some(session);
            started.map_err(fail)?;
            return Ok(VmYield::WaitUntil(now + FRAME_MS));
        }
        if session.started {
            let played = self.play(host, &mut session, &input, seconds);
            self.state = Some(session);
            played.map_err(fail)?;
            return Ok(VmYield::WaitUntil(now + FRAME_MS));
        }
        let moved = session.camera.step(&input, seconds);
        let op = session.camera.op();
        self.state = Some(session);
        if moved {
            host.submit_engine(self.client, 1, &serde_json::to_vec(&json!({"layers": [], "world": [op]})).map_err(|e| Error::Io(e.to_string()))?)?;
        }
        Ok(VmYield::WaitUntil(now + FRAME_MS))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("Naughty Dog session snapshots"))
    }
    fn restore(&mut self, _bytes: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("Naughty Dog session snapshots"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        let v = match &self.state {
            None => json!({"loaded": false, "config": {"level": self.config.level, "task": self.config.task}}),
            Some(s) => {
                let mut v = json!({"loaded": s.pending.is_empty(), "pending": s.pending.len(), "packages": s.packages, "triangles": s.triangles, "eye": s.camera.eye});
                match (&s.game, &s.plan) {
                    (Some(game), Some(plan)) => {
                        let rt = &game.rt;
                        let states: Vec<_> = rt.inner.procs.iter().enumerate().filter(|(_, p)| !p.dead).take(64).map(|(i, p)| format!("{:08x}:{:08x}", p.name, rt.state_name(i).unwrap_or(0))).collect();
                        v["task"] = json!(plan.name);
                        v["level"] = json!(plan.level);
                        v["levels"] = json!(plan.levels);
                        v["load_regions"] = json!(plan.regions);
                        v["modules"] = json!(game.loaded.len());
                        v["processes"] = json!(rt.inner.procs.iter().filter(|p| !p.dead).count());
                        v["states"] = json!(states);
                        v["missing_natives"] = json!(rt.missing());
                        v["script_errors"] = json!(rt.inner.errors.iter().take(32).collect::<Vec<_>>());
                        v["script_error"] = json!(s.script_error);
                        v["log"] = json!(s.stage.log);
                        v["player"] = json!(s.player.as_ref().map(|p| p.position));
                        v["ground_triangles"] = json!(s.ground.len());
                        v["movie"] = json!(s.movie);
                    }
                    _ => v["level"] = json!(s.levels[s.current].0),
                }
                v
            }
        };
        serde_json::to_vec(&v).unwrap_or_default()
    }
}

#[cfg(feature = "runtime")]
pub fn register(registry: &mut crysirror_runtime::AdapterRegistry) -> Result<()> {
    registry.register(ENGINE_ID, Arc::new(Factory))
}

#[cfg(feature = "runtime")]
struct Factory;

#[cfg(feature = "runtime")]
impl crysirror_runtime::runtime::VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &crysirror_runtime::model::ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(spec.program.is_empty(), "the Naughty Dog player executes no data actions");
        Ok(Box::new(NaughtyDogVm::new(client, serde_json::from_value(spec.config.clone())?)?))
    }
}

/// What the animation controllers need of decoded clips, by name `StringId`.
fn clip_infos(clips: &[crate::anim::Clip]) -> HashMap<u32, crate::player::ClipInfo> {
    clips
        .iter()
        .map(|c| {
            // The action-pack reference in clip space, `align(0) ·
            // apReference(0)`; clips without one store zeros.
            let ap = match (c.align.first(), c.ap_reference.first()) {
                (Some(&(at, ar)), Some(&(pt, pr))) if pt.length_squared() > 1e-8 => Some((at + ar * pt, ar * pr)),
                _ => None,
            };
            let info = crate::player::ClipInfo { name: c.name.clone(), duration: if c.frames > 1 { c.frames as f32 / c.fps.max(1.) } else { 0. }, speed: c.root_speed, additive: is_additive(&c.name), fps: c.fps, align: c.align.clone(), ap };
            (sid(&c.name), info)
        })
        .collect()
}

/// A grip on a climb edge: the edge (a body's or the level's), the grab
/// point's distance from `p0`, and the edge frame a climb began from.
#[derive(Clone, Copy, Debug)]
struct Grip {
    body: Option<u32>,
    edge: usize,
    s: f32,
    climb: Option<glam::Mat4>,
}

/// The frame of an edge at `s` metres from `p0`: at the grab point, +z
/// into the wall (against the wall normal `n1`), +y up.
fn edge_frame(e: &crate::collision::Edge, s: f32) -> glam::Mat4 {
    let d = (e.p1 - e.p0).normalize_or_zero();
    let mut f = glam::Vec3::new(-e.n1.x, 0., -e.n1.z);
    if f.length_squared() < 1e-6 {
        f = d.cross(glam::Vec3::Y);
    }
    let f = f.normalize_or_zero();
    glam::Mat4::from_rotation_translation(glam::Quat::from_rotation_y(f.x.atan2(f.z)), e.p0 + d * s)
}

/// Auto-exposure compensation (stops) under a render-settings record:
/// the menu temple matches the original title screen at 0.46 and the
/// prologue's earlier adaptation used 0.5.
const EXPOSURE_CALIBRATION: f32 = 0.5;

/// Scale of the environment ambient for meshes without baked probes
/// (instanced meshes, characters), relative to the record's
/// `ambient-scale` (calibrated with [`EXPOSURE_CALIBRATION`]).
const UNBAKED_AMBIENT: f32 = 0.25;

/// Seconds a jump between edges takes to move the root (the
/// `ledge-jump-up-long` begin, mid and end clips last about 1.2 s).
const LEAP_SECONDS: f32 = 0.9;

/// The root of the default hang (`ledge-grab-idle-hurt`): the edge 1.53 m
/// above and 0.5 m ahead.
const HANG_REACH: glam::Vec3 = glam::Vec3::new(-0.046, 1.525, 0.5);

/// Additive clips carry an `add` name segment (`idle-base-big-add`,
/// `look-add--up-down`, `ef-walk-add-look--up-down`); their channels are
/// local deltas.
fn is_additive(name: &str) -> bool {
    name.split('-').any(|p| p == "add")
}

/// NPC tree inputs without AI: the look (info +140 left–right, +144
/// up–down) and aim (+92, +96) sweeps the phase lambdas read sit at their
/// centre phase.
fn npc_inputs() -> crate::anim_tree::Inputs {
    crate::anim_tree::Inputs { info: vec![(92, 0.5), (96, 0.5), (140, 0.5), (144, 0.5)], ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dc::Dc, ss::Runtime};
    use crysirror_scene::world::{World as Scene, WorldCommand};

    const CLIENT: ClientId = ClientId(1);

    fn publish(stage: &mut Stage, scene: &mut Scene) {
        stage.publish_lights();
        let ops: Vec<WorldCommand> = stage.ops.drain(..)
            .map(|op| serde_json::from_value(op).unwrap())
            .collect();
        scene.apply_batch(CLIENT, &ops).unwrap();
    }

    fn locator(rt: &mut Runtime, position: [f32; 3]) -> u32 {
        let address = rt.dc.mem.alloc(32).unwrap();
        let words = position.into_iter().chain([1., 0., 0., 0., 1.]);
        for (index, value) in words.enumerate() {
            rt.dc.mem.write_u32(address + (4 * index) as u32, value.to_bits()).unwrap();
        }
        address
    }

    #[test]
    fn dynamic_light_natives_change_the_existing_scene_light() {
        let mut rt = Runtime::new(Dc::default());
        let mut stage = Stage::default();
        let mut scene = Scene::default();
        let pos = locator(&mut rt, [1., 2., 3.]);
        let new_pos = locator(&mut rt, [4., 5., 6.]);
        let (name, other_name) = (0xabcd_0012, 0x1234_0012);
        let radius = 8f32.to_bits();
        let angle = 60f32.to_bits();
        rt.call_for_test(&mut stage, "update-dynamic-light", &[name, pos, radius, angle]).unwrap();
        rt.call_for_test(&mut stage, "add-dynamic-light", &[name, pos, radius, angle, 0]).unwrap();
        rt.call_for_test(&mut stage, "add-dynamic-light", &[other_name, pos, radius]).unwrap();
        publish(&mut stage, &mut scene);
        assert_eq!(scene.len(), 2);
        let spot = scene.handle(CLIENT, stage.lights[&name].id).unwrap();
        let point = scene.handle(CLIENT, stage.lights[&other_name].id).unwrap();
        let light = scene.entity(spot).unwrap().light.as_ref().unwrap();
        assert!(!light.enabled);
        assert_eq!(light.shape, LightShape::Spot { inner_degrees: 27., outer_degrees: 30. });
        assert_eq!(scene.entity(point).unwrap().light.as_ref().unwrap().shape, LightShape::Point);

        rt.call_for_test(&mut stage, "add-dynamic-light", &[name, pos, radius, angle, 1]).unwrap();
        let color = [0.25f32, 0.5, 0.75].map(f32::to_bits);
        let value = rt.call_for_test(&mut stage, "set-dynamic-light-color", &[name, color[0], color[1], color[2], 2f32.to_bits()]).unwrap();
        assert_eq!(value, 1);
        publish(&mut stage, &mut scene);
        let light = scene.entity(spot).unwrap().light.as_ref().unwrap();
        assert!(light.enabled);
        assert_eq!(light.color, [0.25, 0.5, 0.75]);
        assert_eq!(light.intensity, 2.);

        rt.call_for_test(&mut stage, "update-dynamic-light", &[name, new_pos, radius, angle]).unwrap();
        publish(&mut stage, &mut scene);
        let entity = scene.entity(spot).unwrap();
        assert_eq!(entity.transform.translation, [4., 5., 6.]);
        assert_eq!(entity.light.as_ref().unwrap().color, [1.; 3]);
        assert_eq!(entity.light.as_ref().unwrap().intensity, 2.);

        rt.call_for_test(&mut stage, "remove-dynamic-light", &[name]).unwrap();
        publish(&mut stage, &mut scene);
        assert!(scene.entity(spot).is_err());
        assert!(scene.entity(point).is_ok());
    }

    #[test]
    fn joint_light_moves_and_is_released_when_the_joint_is_gone() {
        let mut rt = Runtime::new(Dc::default());
        let mut stage = Stage::default();
        let mut scene = Scene::default();
        let (name, object, joint) = (7, 8, 9);
        stage.poses.insert(object, glam::Mat4::IDENTITY);
        stage.joints.insert(object, Arc::new(HashMap::from([(joint, glam::Mat4::from_translation(glam::Vec3::Y))])));
        rt.call_for_test(&mut stage, "add-dynamic-light-joint", &[name, object, joint, 8f32.to_bits()]).unwrap();
        publish(&mut stage, &mut scene);
        let handle = scene.handle(CLIENT, stage.lights[&name].id).unwrap();
        assert_eq!(scene.entity(handle).unwrap().transform.translation, [0., 1., 0.]);
        stage.poses.insert(object, glam::Mat4::from_translation(glam::Vec3::X));
        publish(&mut stage, &mut scene);
        assert_eq!(scene.entity(handle).unwrap().transform.translation, [1., 1., 0.]);
        stage.joints.remove(&object);
        publish(&mut stage, &mut scene);
        assert!(scene.is_empty() && stage.lights.is_empty());
    }

    #[test]
    fn full_light_pool_reuses_a_light_behind_the_camera() {
        let mut stage = Stage {
            camera_view: Some(([0.; 3], [0., 0., 1.], [0., 1., 0.], 60.)),
            ..Default::default()
        };
        let mut scene = Scene::default();
        for name in 0..32 {
            let z = if name == 0 { -1. } else { name as f32 };
            let frame = LightLocator::Frame([0., 0., z], glam::Quat::IDENTITY.to_array());
            stage.add_dynamic_light(name, frame, 10., 0., true);
        }
        publish(&mut stage, &mut scene);
        let id = stage.lights[&0].id;
        let evicted = scene.handle(CLIENT, id).unwrap();
        let frame = LightLocator::Frame([0.; 3], glam::Quat::IDENTITY.to_array());
        stage.add_dynamic_light(32, frame, 10., 0., true);
        publish(&mut stage, &mut scene);
        assert_eq!(scene.len(), 32);
        assert!(!stage.lights.contains_key(&0));
        assert_eq!(stage.lights[&32].id, id);
        assert!(scene.entity(evicted).is_err());
    }

    #[test]
    fn shader_instance_natives_share_the_native_component_layout() {
        let mut rt = Runtime::new(Dc::default());
        let mut stage = Stage::default();
        let object = 1;
        stage.entities.insert(object, vec![2]);
        let values = [1f32, 2., 3., 4.].map(f32::to_bits);
        let result = rt.call_for_test(&mut stage, "set-shader-instance-param", &[
            object, 0x8478_45fe, values[0], values[1], values[2], values[3],
        ]).unwrap();
        assert_eq!(result, 1);
        rt.call_for_test(&mut stage, "set-single-shader-instance-param", &[object, 0xc4e9_088d, 5f32.to_bits()]).unwrap();
        rt.call_for_test(&mut stage, "set-single-shader-instance-param", &[object, 0xcbc3_dbdd, 6f32.to_bits()]).unwrap();
        assert_eq!(stage.instance_params[&object][1], [1., 2., 5., 4.]);
        assert_eq!(stage.instance_params[&object][2], [0., 6., 0., 0.]);
    }
}
