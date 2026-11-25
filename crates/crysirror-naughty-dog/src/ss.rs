// SPDX-License-Identifier: MPL-2.0
//! State-script processes (PPU `game/state-script/ss-*.cpp`) and the
//! script natives that act on them.
//!
//! DC layout: `state-script {name, ?, initial-state, ?, options, state
//! count, states}`; a state is `{name, block count, blocks}`; a block is
//! `{kind, event, 0, track-group}` with kinds 0 `begin`, 1 `end`, 2 event
//! (name in `event`) and 3 `update` (names from `sub_7DEB70`); a track
//! group is `{track count, tracks}`; a track is `{name, lambda count,
//! [(label, script-lambda)]}`. A track runs its lambdas in order; a wait
//! native suspends the lambda mid-way and the track resumes it when the
//! wait completes (`sub_153618` status 1). `go` posts the `go` event to
//! the calling process and abandons the lambda (status 2): the old
//! state's `end` block runs, then the new state's `begin` block.
//!
//! Engine effects go through [`World`]. Natives without an implementation
//! return 0 and are counted in [`Runtime::missing`] so callers see every
//! gap (SATD-nd-retail-runtime).
use crate::dc::{Dc, TYPE_STATE_SCRIPT, sid};
use crate::native_names::NATIVES;
use crate::script::{MAX_ARGS, Natives, Outcome, Status, Thread};
use crate::task::Graph;
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[path = "ss_npc.rs"]
mod npc_natives;
#[path = "ss_player.rs"]
pub mod player_natives;
#[path = "ss_object.rs"]
pub mod object_natives;
#[path = "ss_sound.rs"]
pub mod sound_natives;
#[path = "ss_vehicle.rs"]
pub mod vehicle_natives;
#[path = "ss_misc.rs"]
pub mod misc_natives;

/// A script setting override (`sub_15F480`): its value and the frame it
/// expires after.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub value: u32,
    pub until: u64,
}

pub const KIND_BEGIN: u32 = 0;
pub const KIND_END: u32 = 1;
pub const KIND_EVENT: u32 = 2;
pub const KIND_UPDATE: u32 = 3;
/// `wait-frames` converts frames at 1/30 s (`*(toc-0x7cb4)` = 0.0333).
const FRAME_SECONDS: f32 = 1. / 30.;
const MAX_ERRORS: usize = 256;
/// State changes applied per process and frame.
const MAX_TRANSITIONS: usize = 8;

/// A spawner schema property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    Int(i32),
    Float(f32),
    Sid(u32),
    Str(String),
    Vec4([f32; 4]),
}

/// Parameter names of `set-single-shader-instance-param` and the
/// `(vector, component)` of `g_optionalInstanceParameters` each sets
/// (`sub_13DBD8` → `sub_F65D0`; the names are not in the `StringId`
/// database).
const INSTANCE_PARAMS: [(u32, usize, usize); 16] = [
    (0xccb3_9f64, 0, 0), (0xc872_82d3, 0, 1), (0xc531_a40a, 0, 2), (0xf4fc_22d9, 0, 3),
    (0xcd6b_33e3, 1, 0), (0xc9aa_2e54, 1, 1), (0xc4e9_088d, 1, 2), (0xf524_8e5e, 1, 3),
    (0xcf02_c66a, 2, 0), (0xcbc3_dbdd, 2, 1), (0xc680_fd04, 2, 2), (0xf74d_7bd7, 2, 3),
    (0xceda_6aed, 3, 0), (0xca1b_775a, 3, 1), (0xc758_5183, 3, 2), (0xf695_d750, 3, 3),
];

const INSTANCE_VECTORS: [u32; 4] = [0x80b9_5849, 0x8478_45fe, 0x893b_6327, 0x8dfa_7e90];

/// How a script light obtains its locator on each frame.
#[derive(Clone, Copy, Debug)]
pub enum LightLocator {
    Frame([f32; 3], [f32; 4]),
    Joint { object: u32, joint: u32 },
}

/// A script particle request, served by the engine after the frame's
/// scripts.
#[derive(Clone, Debug, PartialEq)]
pub enum ParticleRequest {
    Spawn { handle: u32, group: u32, position: [f32; 3], rotation: [f32; 4] },
    Kill { handle: u32 },
}

/// Engine services natives use. Names are `StringId`s.
pub trait World {
    /// Seconds since the level started.
    fn now(&self) -> f64;
    fn print(&mut self, text: &str);
    fn level_spawned(&self, level: u32) -> bool;
    fn object_spawned(&self, name: u32) -> bool;
    fn spawn(&mut self, name: u32) -> bool;
    fn kill(&mut self, name: u32) -> bool;
    /// Spawner position and rotation quaternion (x, y, z, w).
    fn locator(&self, name: u32) -> Option<([f32; 3], [f32; 4])>;
    fn property(&self, spawner: u32, prop: u32) -> Option<Property>;
    /// Fade the screen to `color` at `alpha` (1 opaque) over `seconds`.
    fn fade(&mut self, color: [f32; 3], alpha: f32, seconds: f32);
    fn camera(&mut self, name: u32, seconds: f32);
    /// Release scripted camera `name` (`None`: every scripted camera).
    fn camera_disable(&mut self, _name: Option<u32>) {}
    /// Keep camera animation `name` (clip `clip`, blended in over
    /// `seconds`) offsetting the active camera; enabling a running one
    /// keeps its phase.
    fn camera_animation(&mut self, _name: u32, _clip: u32, _seconds: f32) {}
    /// Blend camera animation `name` out (`None`: every one).
    fn camera_animation_disable(&mut self, _name: Option<u32>) {}
    fn player_controls(&mut self, enabled: bool);
    /// Left and right stick (x, y in −1…1) of a controller.
    fn sticks(&self, pad: u32) -> [f32; 4];
    /// Play clip `clip` once on `object` (a spawner name or `player`).
    /// False when the object is unknown.
    fn animate(&mut self, _object: u32, _clip: u32, _options: &Animate) -> bool {
        false
    }
    /// Phase (0–1) of the script animation `object` is playing; `None`
    /// once it has ended (or none plays).
    fn animation_phase(&self, _object: u32) -> Option<f32> {
        None
    }
    /// Whether `object` (`player`, a spawner) stands in trigger region
    /// `region` (a region name `StringId`).
    fn in_region(&self, _object: u32, _region: u32) -> bool {
        false
    }
    /// Show or hide a spawned object's entities. False when unknown.
    fn set_visible(&mut self, _object: u32, _visible: bool) -> bool {
        false
    }
    /// Whether `object` (`player`, a character) stands on `support`'s
    /// collision body.
    fn object_on_object(&self, _object: u32, _support: u32) -> bool {
        false
    }
    /// Spawner `name`'s NPC while its process exists: `Some(dead)`.
    fn npc(&self, _name: u32) -> Option<bool> {
        None
    }
    /// Whether spawner `name`'s NPC was killed; kept after its process
    /// is gone (the spawner's flag 0x2000_0000).
    fn npc_killed(&self, _name: u32) -> bool {
        false
    }
    /// Kill spawner `name`'s NPC. False when it has none.
    fn npc_die(&mut self, _name: u32) -> bool {
        false
    }
    /// Live NPC processes.
    fn npc_alive_count(&self) -> u32 {
        0
    }
    /// World frame of joint `joint` of `object` (`player` or a spawner).
    fn joint_locator(&self, _object: u32, _joint: u32) -> Option<([f32; 3], [f32; 4])> {
        None
    }
    /// Add a point light (zero cone) or spot light (cone angle in degrees).
    fn add_dynamic_light(&mut self, _name: u32, _locator: LightLocator, _radius: f32, _cone: f32, _enabled: bool) {}
    /// Move an existing light to a fixed locator, keeping its type and gain.
    fn update_dynamic_light(&mut self, _name: u32, _locator: LightLocator, _radius: f32, _cone: f32) {}
    /// Recolour dynamic light `name`. False when it does not exist.
    fn dynamic_light_color(&mut self, _name: u32, _color: [f32; 3], _gain: f32) -> bool {
        false
    }
    fn remove_dynamic_light(&mut self, _name: u32) {}
    /// Start effect group `group` at a frame; returns its handle (0 when
    /// unsupported).
    fn spawn_particles(&mut self, _group: u32, _position: [f32; 3], _rotation: [f32; 4]) -> u32 {
        0
    }
    fn kill_particles(&mut self, _handle: u32) {}
    /// Set component `component` of instance parameter `vector` of
    /// `object`'s shaders. False when unknown.
    fn set_instance_param(&mut self, _object: u32, _vector: usize, _component: usize, _value: f32) -> bool {
        false
    }
    fn set_instance_vector(&mut self, _object: u32, _vector: usize, _value: [f32; 4]) -> bool {
        false
    }
    /// Whether a world point lies in the presented camera's view frustum.
    fn on_screen(&self, _point: [f32; 3]) -> bool {
        false
    }
    /// Move water object `name` to its spawner plus `offset`. False when
    /// unknown.
    fn set_water_offset(&mut self, _name: u32, _offset: [f32; 3]) -> bool {
        false
    }
    /// `speaker` says vox `id`; false when no line plays.
    fn say(&mut self, _speaker: u32, _id: u32) -> bool {
        false
    }
    fn speaking(&self, _speaker: u32) -> bool {
        false
    }
    fn stop_speaking(&mut self, _speaker: u32) {}
    /// The script's render-settings record (and a second one with its
    /// blend), or `None` for the regions' choice.
    fn set_render_settings(&mut self, _choice: Option<(u32, Option<(u32, f32)>)>) {}
    /// Seconds of the current frame.
    fn frame_seconds(&self) -> f32 {
        FRAME_SECONDS
    }
    /// An explosion of `kind` at a frame.
    fn spawn_explosion(&mut self, _kind: u32, _position: [f32; 3], _rotation: [f32; 4]) {}
    /// Show text `id` as an on-screen message.
    fn message(&mut self, _id: u32) {}
    fn kill_all_breakables(&mut self) {}
    /// The vehicles (tanks, helicopters) by object name.
    fn vehicles(&mut self) -> Option<&mut crate::vehicle::Vehicles> {
        None
    }
    fn vehicles_ref(&self) -> Option<&crate::vehicle::Vehicles> {
        None
    }
    /// Start sound `id`; returns its handle (0 when nothing plays).
    fn play_sound(&mut self, _id: u32, _at: sound_natives::SoundAt, _volume: f32, _pitch: f32) -> u32 {
        0
    }
    fn stop_sound(&mut self, _handle: u32) {}
    fn set_mixer(&mut self, _mixer: u32) {}
    /// Block or unblock the nav polygon under object `name`.
    fn nav_block(&mut self, _name: u32, _block: bool) -> bool {
        false
    }
    /// Remove (false) or restore the cover points on object `name`.
    fn object_cover(&mut self, _name: u32, _enabled: bool) -> bool {
        false
    }
    fn enable_action_pack(&mut self, _ap: u32, _enabled: bool) {}
    /// Every value spawner `spawner` has for tag `prop`.
    fn properties(&self, spawner: u32, prop: u32) -> Vec<Property> {
        self.property(spawner, prop).into_iter().collect()
    }
    /// Move spawned object `name` to a frame.
    fn place(&mut self, _name: u32, _position: [f32; 3], _rotation: [f32; 4]) {}
    /// Stop object `name` acting; false when unknown.
    fn make_inert(&mut self, _name: u32) -> bool {
        false
    }
    /// Point `character`'s head at `(target, attach point, blend)`, or
    /// stop (`None`).
    fn look_at(&mut self, _character: u32, _target: Option<(u32, u32, u32)>) {}
    fn set_mesh_visible(&mut self, _object: u32, _mesh: u32, _visible: bool) -> bool {
        false
    }
    fn camera_shake(&mut self, _shake: object_natives::Shake) {}
    fn set_collision_layer(&mut self, _object: u32, _layer: u32) -> bool {
        false
    }
    /// Health fraction of a breakable or rail vehicle.
    fn object_health(&self, _object: u32) -> Option<f32> {
        None
    }
    fn player_health(&self) -> f32 {
        1.
    }
    fn set_animation_speed(&mut self, _object: u32, _speed: f32) -> bool {
        false
    }
    fn activate_spline(&mut self, _spline: u32, _active: bool) -> bool {
        false
    }
    /// The id of the object the player carries.
    fn player_carried(&self) -> Option<u32> {
        None
    }
    /// The player process's native state class (`StringId`).
    fn player_state(&self) -> Option<u32> {
        None
    }
    fn kill_player(&mut self, _ragdoll: bool, _death_camera: bool, _delay: f32) {}
    /// Angle (radians) between a joint's current local rotation and its
    /// bind rotation; `None` when the object has no such joint.
    fn joint_bind_angle(&self, _object: u32, _joint: u32) -> Option<f32> {
        None
    }
    /// The object type (process type) of spawner `name`.
    fn object_type(&self, _name: u32) -> Option<u32> {
        None
    }
    /// The live NPC agents (the NPC manager, `sub_1E2968`).
    fn agents(&mut self) -> Option<&mut crate::npc::Agents> {
        None
    }
    fn agents_ref(&self) -> Option<&crate::npc::Agents> {
        None
    }
    /// Corners of a navigation path from `from` to `to` ending at `to`;
    /// `None` when either point is off the nav meshes.
    fn nav_path(&self, _from: [f32; 3], _to: [f32; 3]) -> Option<Vec<[f32; 3]>> {
        None
    }
    /// The cover point nearest `point` (facing away from `threat` when
    /// given).
    fn closest_cover(&self, _point: [f32; 3], _threat: Option<[f32; 3]>) -> Option<[f32; 3]> {
        None
    }
    /// Position and unit tangent of spline `spline` at arc distance
    /// `distance` (`sub_35A20`).
    fn spline_at(&self, _spline: u32, _distance: f32) -> Option<([f32; 3], [f32; 3])> {
        None
    }
    /// Arc distance along spline `spline` of its point closest to `point`.
    fn spline_distance(&self, _spline: u32, _point: [f32; 3]) -> Option<f32> {
        None
    }
}

/// Options of `(animate object clip key value …)`. The keys seen in the
/// level scripts are 2 (blend-in seconds), 4 (blend-out seconds) and 7 (an
/// action-pack reference locator, `get-ap-ref-locator` /
/// `get-spawner-locator`); others are ignored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animate {
    pub blend_in: Option<f32>,
    pub blend_out: Option<f32>,
    /// Position and rotation quaternion (x, y, z, w) to play at.
    pub locator: Option<([f32; 3], [f32; 4])>,
}

#[derive(Clone, Debug, PartialEq)]
enum Wait {
    None,
    Until(f64),
    Frame(u64),
    Signal(u32),
    /// Until process variable `var` of `process` equals `value`.
    Var { process: usize, var: u32, value: u32 },
    /// Until the script animation of an object ends.
    Animation(u32),
    /// Until NPC `name`'s scripted-skill order completes (or the NPC is
    /// gone).
    Npc(u32),
    /// Until `speaker` stops speaking.
    Speech(u32),
    /// Until vehicle `name`'s order completes.
    Vehicle(u32),
}

struct Track {
    lambdas: u32,
    count: u32,
    next: u32,
    thread: Option<Thread>,
    wait: Wait,
    /// A `jump` made by the suspended lambda.
    jump: Option<u32>,
}

struct Group {
    kind: u32,
    event: u32,
    args: Vec<u32>,
    /// The process that sent the event (`event-sender`).
    sender: Option<u32>,
    tracks: Vec<Track>,
}

pub struct Process {
    pub name: u32,
    pub script: u32,
    /// Spawner whose properties `tag-*` read.
    pub spawner: Option<u32>,
    pub state: Option<u32>,
    pub vars: HashMap<u32, u32>,
    groups: Vec<Group>,
    events: VecDeque<(u32, Vec<u32>, Option<u32>)>,
    pending_go: Option<u32>,
    pub dead: bool,
    /// When the current state began (`wait-until-frame` counts from it).
    pub state_start: f64,
}

/// Runtime state outside the DC memory (so natives can borrow both).
pub struct Inner {
    pub procs: Vec<Process>,
    native_ids: HashMap<u32, u32>,
    signals: HashSet<u32>,
    pub frame: u64,
    pub tasks: Graph,
    /// Current `(node, subnode)`.
    pub task: Option<(usize, usize)>,
    pub missing: BTreeMap<&'static str, u32>,
    pub errors: Vec<String>,
    /// A task change requested by `task-subnode-complete` (current task)
    /// or `task-subnode-play`.
    pub task_change: Option<(usize, usize)>,
    /// Module names requested by `add-load-module`/`add-import-module`.
    pub module_requests: Vec<u32>,
    /// `pad-get-info` records by pad (one 28-byte heap record each).
    pads: HashMap<u32, u32>,
    /// Player movement types from `set-walk-type`/`set-run-type`.
    pub movement: [u32; 2],
    pub prints: Vec<String>,
    /// State of the scripts' random numbers (xorshift).
    rng: u32,
    /// Game statistics by `(stat, qualifier)` (`stat-value?`).
    stats: HashMap<(u32, u32), i32>,
    /// Names of `StringId`s the runtime has seen (natives' names, loaded
    /// symbols), for `string<-string-id`.
    pub names: HashMap<u32, String>,
    /// Script setting overrides by setting id.
    pub settings: HashMap<u32, Setting>,
    /// The player's weapons and their ammo (-1: the weapon's full load).
    pub weapons: BTreeMap<u32, i32>,
    /// Prone settings record chosen by `player-set-prone-mode`.
    pub prone: Option<u32>,
    /// World time at the last tick.
    now: f64,
    /// The `-disc` command-line id and the FMA flag of the game state.
    pub disc: Option<u32>,
    pub fma: bool,
    /// Task graphs logged in by `game-task-graph-login` (addresses).
    pub task_graphs: Vec<u32>,
    /// The chapter menu needs rebuilding.
    pub task_menu_dirty: bool,
    /// The game difficulty (`sub_57040`, profile +0x1DE0): 0 easy, 1
    /// normal, 2 hard, 3 crushing.
    pub difficulty: u32,
    /// Game state natives write (journal, HUD, weather, ropes, …).
    pub game: misc_natives::GameState,
}

pub struct Runtime {
    pub dc: Dc,
    pub inner: Inner,
}

/// Native call context: the calling process and the wait it requests.
struct Ctx<'a> {
    inner: &'a mut Inner,
    world: &'a mut dyn World,
    proc: Option<usize>,
    wait: Wait,
    /// The running track's `(label, lambda)` pairs and count.
    track: Option<(u32, u32)>,
    /// Entry index a `jump` continues the track at.
    jump: Option<u32>,
}

fn state_by_name(dc: &Dc, script: u32, name: u32) -> Result<Option<u32>> {
    let (n, states) = (dc.mem.u32(script + 0x14)?, dc.mem.u32(script + 0x18)?);
    ensure!(n < 4096, "state count");
    for i in 0..n {
        let s = states + 12 * i;
        if name == 0 || dc.mem.u32(s)? == name {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// The track group of `state`'s block of `kind` (and `event`).
fn block(dc: &Dc, state: u32, kind: u32, event: u32) -> Result<Option<u32>> {
    let (n, blocks) = (dc.mem.u32(state + 4)?, dc.mem.u32(state + 8)?);
    ensure!(n < 4096, "block count");
    for i in 0..n {
        let b = blocks + 16 * i;
        if dc.mem.u32(b)? == kind && (kind != KIND_EVENT || dc.mem.u32(b + 4)? == event) {
            let g = dc.mem.u32(b + 12)?;
            return Ok((g != 0).then_some(g));
        }
    }
    Ok(None)
}

fn group(dc: &Dc, kind: u32, event: u32, args: Vec<u32>, g: u32) -> Result<Group> {
    let (n, tracks) = (dc.mem.u32(g)?, dc.mem.u32(g + 4)?);
    ensure!(n <= 64, "track count");
    let tracks = (0..n)
        .map(|i| {
            let t = tracks + 12 * i;
            Ok(Track { lambdas: dc.mem.u32(t + 8)?, count: dc.mem.u32(t + 4)?, next: 0, thread: None, wait: Wait::None, jump: None })
        })
        .collect::<Result<_>>()?;
    Ok(Group { kind, event, args, sender: None, tracks })
}

impl Runtime {
    pub fn new(dc: Dc) -> Self {
        let native_ids = NATIVES.iter().enumerate().map(|(i, (n, _))| (sid(n), i as u32)).collect();
        Self {
            dc,
            inner: Inner {
                procs: Vec::new(),
                native_ids,
                signals: HashSet::new(),
                frame: 0,
                tasks: Graph::default(),
                task: None,
                missing: BTreeMap::new(),
                errors: Vec::new(),
                task_change: None,
                module_requests: Vec::new(),
                prints: Vec::new(),
                pads: HashMap::new(),
                movement: [0; 2],
                rng: 0x2545_f491,
                stats: HashMap::new(),
                names: NATIVES.iter().map(|(n, _)| (sid(n), (*n).to_owned())).collect(),
                settings: HashMap::new(),
                weapons: BTreeMap::new(),
                prone: None,
                now: 0.,
                disc: None,
                fma: false,
                task_graphs: Vec::new(),
                task_menu_dirty: false,
                difficulty: 1,
                game: misc_natives::GameState { journal_page: -1, ..Default::default() },
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn call_for_test(&mut self, world: &mut dyn World, name: &str, args: &[u32]) -> Result<u32> {
        let mut ctx = Ctx { inner: &mut self.inner, world, proc: None, wait: Wait::None, track: None, jump: None };
        let id = ctx.lookup(sid(name)).context("test native id")?;
        let mut padded = [0; MAX_ARGS];
        padded[..args.len()].copy_from_slice(args);
        let (value, _) = ctx.call(&mut self.dc, id, &padded, args.len() as u32)?;
        Ok(value)
    }

    /// Run a top-level lambda (for example a module's `top-level`) to
    /// completion outside any process. A suspension is an error.
    pub fn run_lambda(&mut self, world: &mut dyn World, lambda: u32, args: &[u32]) -> Result<u32> {
        let mut t = Thread::new(&self.dc, lambda, args)?;
        let mut ctx = Ctx { inner: &mut self.inner, world, proc: None, wait: Wait::None, track: None, jump: None };
        match t.run(&mut self.dc, &mut ctx)? {
            Outcome::Returned(v) => Ok(v),
            other => anyhow::bail!("top-level lambda ended with {other:?}"),
        }
    }

    /// Start a process running the state script named `script`.
    pub fn spawn(&mut self, name: u32, script: u32, spawner: Option<u32>) -> Result<usize> {
        let world_now = self.inner.now;
        let addr = *self.dc.symbols.get(&script).with_context(|| format!("state script {script:#010x} is not loaded"))?;
        ensure!(self.dc.mem.type_of(addr)? == TYPE_STATE_SCRIPT, "{script:#010x} is not a state-script");
        let initial = self.dc.mem.u32(addr + 8)?;
        let state = state_by_name(&self.dc, addr, initial)?.or(state_by_name(&self.dc, addr, 0)?);
        let mut p = Process { name, script: addr, spawner, state: None, vars: HashMap::new(), groups: Vec::new(), events: VecDeque::new(), pending_go: None, dead: false, state_start: world_now };
        if let Some(s) = state {
            p.state = Some(s);
            if let Some(g) = block(&self.dc, s, KIND_BEGIN, 0)? {
                p.groups.push(group(&self.dc, KIND_BEGIN, 0, Vec::new(), g)?);
            }
        }
        self.inner.procs.push(p);
        Ok(self.inner.procs.len() - 1)
    }

    /// Advance every process by one frame.
    pub fn tick(&mut self, world: &mut dyn World) -> Result<()> {
        self.inner.frame += 1;
        self.inner.now = world.now();
        let frame = self.inner.frame;
        self.inner.settings.retain(|_, s| s.until >= frame);
        let mut i = 0;
        while i < self.inner.procs.len() {
            self.step_process(world, i)?;
            i += 1;
        }
        Ok(())
    }

    fn step_process(&mut self, world: &mut dyn World, i: usize) -> Result<()> {
        if self.inner.procs[i].dead {
            return Ok(());
        }
        // Events queued for this process start their handler groups.
        while let Some((event, args, sender)) = self.inner.procs[i].events.pop_front() {
            if let Some(state) = self.inner.procs[i].state {
                if let Some(g) = block(&self.dc, state, KIND_EVENT, event)? {
                    let mut g = group(&self.dc, KIND_EVENT, event, args, g)?;
                    g.sender = sender;
                    self.inner.procs[i].groups.retain(|x| !(x.kind == KIND_EVENT && x.event == event));
                    self.inner.procs[i].groups.push(g);
                }
            }
        }
        // `update` runs every frame from its first lambda once done.
        if let Some(state) = self.inner.procs[i].state {
            if !self.inner.procs[i].groups.iter().any(|g| g.kind == KIND_UPDATE) {
                if let Some(g) = block(&self.dc, state, KIND_UPDATE, 0)? {
                    let g = group(&self.dc, KIND_UPDATE, 0, Vec::new(), g)?;
                    self.inner.procs[i].groups.push(g);
                }
            }
        }
        // A state change runs the new state's blocks in the same frame;
        // states that keep changing continue on the next frame.
        for _ in 0..MAX_TRANSITIONS {
            self.run_groups(world, i)?;
            let Some(next) = self.inner.procs[i].pending_go.take() else { return Ok(()) };
            self.transition(world, i, next)?;
        }
        Ok(())
    }

    fn run_groups(&mut self, world: &mut dyn World, i: usize) -> Result<()> {
        let now = world.now();
        let mut gi = 0;
        while gi < self.inner.procs[i].groups.len() {
            let mut ti = 0;
            while ti < self.inner.procs[i].groups[gi].tracks.len() {
                if self.inner.procs[i].pending_go.is_some() || self.inner.procs[i].dead {
                    return Ok(());
                }
                self.run_track(world, i, gi, ti, now)?;
                ti += 1;
            }
            let g = &self.inner.procs[i].groups[gi];
            if g.tracks.iter().all(|t| t.next >= t.count && t.thread.is_none()) {
                self.inner.procs[i].groups.remove(gi);
            } else {
                gi += 1;
            }
        }
        Ok(())
    }

    fn wait_done(&self, wait: &Wait, now: f64) -> bool {
        match wait {
            Wait::None => true,
            Wait::Until(t) => now >= *t,
            Wait::Frame(f) => self.inner.frame >= *f,
            Wait::Signal(s) => self.inner.signals.contains(s),
            Wait::Var { process, var, value } => self.inner.procs.get(*process).is_none_or(|p| p.vars.get(var).copied().unwrap_or(0) == *value),
            // Resolved against the world in `run_track`.
            Wait::Animation(_) | Wait::Npc(_) | Wait::Speech(_) | Wait::Vehicle(_) => false,
        }
    }

    fn run_track(&mut self, world: &mut dyn World, i: usize, gi: usize, ti: usize, now: f64) -> Result<()> {
        let mut guard = 0;
        loop {
            let t = &self.inner.procs[i].groups[gi].tracks[ti];
            let done = match t.wait {
                Wait::Animation(object) => world.animation_phase(object).is_none(),
                Wait::Speech(speaker) => !world.speaking(speaker),
                Wait::Vehicle(name) => world.vehicles_ref().and_then(|v| v.vehicles.get(&name)).is_none_or(|v| v.order.is_none()),
                Wait::Npc(name) => world.agents_ref().and_then(|a| a.get(name)).is_none_or(|a| a.order.is_none()),
                ref w => self.wait_done(w, now),
            };
            if !done {
                return Ok(());
            }
            let mut thread = match self.inner.procs[i].groups[gi].tracks[ti].thread.take() {
                Some(th) => th,
                None => {
                    let t = &self.inner.procs[i].groups[gi].tracks[ti];
                    if t.next >= t.count {
                        return Ok(());
                    }
                    guard += 1;
                    ensure!(guard <= 4096, "track lambda loop");
                    let lambda = self.dc.mem.u32(t.lambdas + 8 * t.next + 4)?;
                    match Thread::new(&self.dc, lambda, &[]) {
                        Ok(th) => th,
                        Err(e) => {
                            // A track entry without a lambda (label only)
                            // has nothing to run.
                            if lambda != 0 {
                                self.inner.error(format!("process {:#010x}: {e:#}", self.inner.procs[i].name));
                            }
                            self.inner.procs[i].groups[gi].tracks[ti].next += 1;
                            continue;
                        }
                    }
                }
            };
            let pairs = {
                let t = &self.inner.procs[i].groups[gi].tracks[ti];
                (t.lambdas, t.count)
            };
            let mut ctx = Ctx { inner: &mut self.inner, world, proc: Some(i), wait: Wait::None, track: Some(pairs), jump: None };
            let outcome = thread.run(&mut self.dc, &mut ctx);
            let wait = std::mem::replace(&mut ctx.wait, Wait::None);
            let jump = ctx.jump.take();
            let frame = self.inner.frame;
            let outcome = match outcome {
                Ok(o) => o,
                Err(e) => {
                    self.inner.error(format!("process {:#010x}: {e:#}", self.inner.procs[i].name));
                    Outcome::Aborted
                }
            };
            let Some(track) = self.inner.procs.get_mut(i).and_then(|p| p.groups.get_mut(gi)).and_then(|g| g.tracks.get_mut(ti)) else { return Ok(()) };
            match outcome {
                // After a `jump` the track continues at the label's entry
                // (`sub_7DACBC` stores entry − 1; the runner advances).
                // A backward jump waits for the next frame: loops whose
                // waits complete at once here would otherwise spin.
                Outcome::Returned(_) => {
                    let jump = jump.or(track.jump.take());
                    let current = track.next;
                    track.next = jump.unwrap_or(current + 1);
                    track.wait = if jump.is_some_and(|k| k <= current) { Wait::Frame(frame + 1) } else { Wait::None };
                }
                Outcome::Suspended => {
                    track.jump = jump.or(track.jump.take());
                    track.thread = Some(thread);
                    track.wait = wait;
                    return Ok(());
                }
                Outcome::Aborted => {
                    track.jump = None;
                    track.next = track.count;
                    return Ok(());
                }
            }
        }
    }

    fn transition(&mut self, world: &mut dyn World, i: usize, next: u32) -> Result<()> {
        let script = self.inner.procs[i].script;
        let Some(target) = state_by_name(&self.dc, script, next)? else {
            self.inner.error(format!("process {:#010x}: go to unknown state {next:#010x}", self.inner.procs[i].name));
            return Ok(());
        };
        // The old state's `end` block runs until it waits; then the new
        // state begins.
        if let Some(old) = self.inner.procs[i].state {
            self.inner.procs[i].groups.clear();
            if let Some(g) = block(&self.dc, old, KIND_END, 0)? {
                self.inner.procs[i].groups.push(group(&self.dc, KIND_END, 0, Vec::new(), g)?);
                self.run_groups(world, i)?;
            }
        }
        let now = world.now();
        let p = &mut self.inner.procs[i];
        p.groups.clear();
        p.state = Some(target);
        p.state_start = now;
        if let Some(g) = block(&self.dc, target, KIND_BEGIN, 0)? {
            self.inner.procs[i].groups.push(group(&self.dc, KIND_BEGIN, 0, Vec::new(), g)?);
        }
        Ok(())
    }

    /// Make `at` the current task subnode. Completion is native runtime
    /// state in each subnode's +0x1c record: word 0 is 3 once complete
    /// (`task-subnode-complete?` in `task-defines.bin`). Every subnode
    /// ordered before `at` is marked complete, the others not.
    pub fn select_task(&mut self, at: (usize, usize)) -> Result<()> {
        self.inner.task = Some(at);
        for i in 0..self.inner.tasks.nodes.len() {
            for j in 0..self.inner.tasks.nodes[i].subnodes.len() {
                let record = self.dc.mem.u32(self.inner.tasks.nodes[i].subnodes[j].address + 0x1c)?;
                if record != 0 {
                    let done = self.inner.tasks.before((i, j), at);
                    self.dc.mem.write_u32(record, if done { 3 } else { 0 })?;
                }
            }
        }
        Ok(())
    }

    /// Native names called but not implemented, with call counts.
    /// A live script setting override.
    pub fn setting(&self, id: u32) -> Option<u32> {
        self.inner.settings.get(&id).filter(|s| s.until >= self.inner.frame).map(|s| s.value)
    }

    pub fn missing(&self) -> &BTreeMap<&'static str, u32> {
        &self.inner.missing
    }

    pub fn process_by_name(&self, name: u32) -> Option<usize> {
        self.inner.procs.iter().position(|p| p.name == name && !p.dead)
    }

    /// The current state name of a process.
    pub fn state_name(&self, i: usize) -> Option<u32> {
        self.inner.procs[i].state.and_then(|s| self.dc.mem.u32(s).ok())
    }
}

impl Inner {
    fn error(&mut self, message: String) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(message);
        }
    }
    fn process(&self, name: u32) -> Option<usize> {
        self.procs.iter().position(|p| p.name == name && !p.dead)
    }
}

impl Ctx<'_> {
    fn arg_f(args: &[u32; MAX_ARGS], i: usize) -> f32 {
        f32::from_bits(args[i])
    }
    /// The process `args[i]` names (`self` and absent arguments are the
    /// caller).
    fn target(&self, args: &[u32; MAX_ARGS], argc: u32, i: usize) -> Option<usize> {
        if (argc as usize) > i && args[i] != sid("self") {
            self.inner.process(args[i])
        } else {
            self.proc
        }
    }
    fn set_var(&mut self, args: &[u32; MAX_ARGS], argc: u32) -> u32 {
        if argc < 2 {
            return 0;
        }
        match self.target(args, argc, 2) {
            Some(p) => {
                self.inner.procs[p].vars.insert(args[0], args[1]);
                1
            }
            None => 0,
        }
    }
    fn get_var(&mut self, args: &[u32; MAX_ARGS], argc: u32) -> u32 {
        if argc < 1 {
            return 0;
        }
        match self.target(args, argc, 1) {
            Some(p) => self.inner.procs[p].vars.get(&args[0]).copied().unwrap_or(0),
            None => 0,
        }
    }
    fn tag(&mut self, prop: u32) -> Option<Property> {
        let spawner = self.inner.procs.get(self.proc?)?.spawner?;
        self.world.property(spawner, prop)
    }
    fn suspend(&mut self, wait: Wait) -> (u32, Status) {
        if self.proc.is_none() {
            self.inner.error("wait outside a state-script track".into());
            return (0, Status::Continue);
        }
        self.wait = wait;
        (1, Status::Suspend)
    }
    fn send(&mut self, event: u32, target: u32, args: Vec<u32>) -> u32 {
        let Some(p) = self.inner.process(target) else { return 0 };
        if event == sid("go") {
            self.inner.procs[p].pending_go = args.first().copied();
        } else {
            let sender = self.proc.map(|q| self.inner.procs[q].name);
            self.inner.procs[p].events.push_back((event, args, sender));
        }
        1
    }
}

/// The NPC manager's built-in groups (`sub_A0E58`): `Some(None)` counts
/// every NPC, `Some(Some(type))` those of one process type.
fn builtin_group(group: u32) -> Option<Option<u32>> {
    match group {
        0x3766_5f0e => Some(Some(0x88db_cf2f)), // all-mercs
        0xfe8e_71e9 => Some(Some(0x88db_cf2f)),
        0x49e5_675b => Some(Some(0x7726_a8bc)), // all-villagers: Villager
        0x1a4e_b1ef => Some(Some(0xd86d_38aa)),
        0x26d8_ac33 => Some(None),
        0xab7a_1939 => Some(Some(0xfd2c_9c09)),
        _ => None,
    }
}

/// `ND_TRACE_NATIVES=substring` prints matching native calls (diagnostic).
fn trace_natives() -> Option<&'static String> {
    static FILTER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FILTER.get_or_init(|| std::env::var("ND_TRACE_NATIVES").ok()).as_ref()
}

impl Ctx<'_> {
    fn next_random(&mut self) -> u32 {
        let r = &mut self.inner.rng;
        *r ^= *r << 13;
        *r ^= *r >> 17;
        *r ^= *r << 5;
        *r
    }

    /// Members of an `entity-group` symbol (empty when it is none).
    /// The NPC a `npc-*` native names, or the calling process.
    fn npc_name(&self, a: &[u32], n: usize) -> u32 {
        if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) }
    }

    fn group_members(dc: &Dc, name: u32) -> Result<Vec<u32>> {
        let Some(&group) = dc.symbols.get(&name) else { return Ok(Vec::new()) };
        if dc.mem.type_of(group).ok() != Some(sid("entity-group")) {
            return Ok(Vec::new());
        }
        let (count, array) = (dc.mem.u32(group)?, dc.mem.u32(group + 4)?);
        ensure!(count <= 4096, "entity-group size");
        (0..count).map(|i| dc.mem.u32(array + 8 * i + 4)).collect()
    }

    /// Call a native (by its script value) or a script lambda with `args`;
    /// a lambda must not suspend.
    fn invoke(&mut self, dc: &mut Dc, function: u32, args: &[u32]) -> Result<u32> {
        let mut regs = [0u32; MAX_ARGS];
        regs[..args.len().min(MAX_ARGS)].copy_from_slice(&args[..args.len().min(MAX_ARGS)]);
        if function >= crate::script::NATIVE_BASE && function != crate::script::NULL_SYMBOL {
            return Ok(self.call(dc, function - crate::script::NATIVE_BASE, &regs, args.len() as u32)?.0);
        }
        if function == 0 || function == crate::script::NULL_SYMBOL {
            return Ok(0);
        }
        let mut thread = Thread::new(dc, function, args)?;
        match thread.run(dc, self)? {
            Outcome::Returned(v) => Ok(v),
            other => {
                self.inner.error(format!("group lambda ended with {other:?}"));
                Ok(0)
            }
        }
    }
}

impl Natives for Ctx<'_> {
    fn lookup(&self, s: u32) -> Option<u32> {
        self.inner.native_ids.get(&s).copied()
    }
    fn error(&mut self, message: String) {
        self.inner.error(message);
    }
    fn call(&mut self, dc: &mut Dc, id: u32, a: &[u32; MAX_ARGS], argc: u32) -> Result<(u32, Status)> {
        let name = NATIVES.get(id as usize).context("native id")?.0;
        let n = argc as usize;
        let ok = |v: u32| Ok((v, Status::Continue));
        let b = |v: bool| Ok((u32::from(v), Status::Continue));
        let now = self.world.now();
        if trace_natives().is_some_and(|f| name.contains(f.as_str())) {
            let proc = self.proc.map(|p| (self.inner.procs[p].name, self.inner.procs[p].state.unwrap_or(0)));
            eprintln!("[native {now:.2}] {proc:08x?}: ({name} {:08x?})", &a[..n.min(MAX_ARGS)]);
        }
        if let Some(reply) = self.npc_native(dc, name, a, n)? {
            return Ok(reply);
        }
        if let Some(reply) = self.player_native(dc, name, a, n)? {
            return Ok(reply);
        }
        if let Some(reply) = self.object_native(dc, name, a, n)? {
            return Ok(reply);
        }
        if let Some(reply) = self.sound_native(dc, name, a, n)? {
            return Ok(reply);
        }
        if let Some(reply) = self.vehicle_native(dc, name, a, n)? {
            return Ok(reply);
        }
        if let Some(reply) = self.misc_native(dc, name, a, n)? {
            return Ok(reply);
        }
        match name {
            // (add-load-module 'name 'script): the host loads the module
            // after the calling lambda (`sub_1545A8` queues it).
            "add-load-module" | "add-import-module" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                self.inner.module_requests.push(a[0]);
                ok(1)
            }
            "go" => {
                let Some(p) = self.proc else { return ok(0) };
                self.inner.procs[p].pending_go = Some(a[0]);
                Ok((1, Status::Abort))
            }
            "wait" => {
                let s = Self::arg_f(a, 0);
                if n == 0 || s <= 0. { ok(0) } else { Ok(self.suspend(Wait::Until(now + f64::from(s)))) }
            }
            "wait-frames" => {
                let s = a[0] as i32 as f32 * FRAME_SECONDS;
                if n == 0 || s <= 0. { ok(0) } else { Ok(self.suspend(Wait::Until(now + f64::from(s)))) }
            }
            "wait-one-frame" => Ok(self.suspend(Wait::Frame(self.inner.frame + 1))),
            "wait-for" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                if self.inner.signals.contains(&a[0]) { ok(0) } else { Ok(self.suspend(Wait::Signal(a[0]))) }
            }
            "signal" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                self.inner.signals.insert(a[0]);
                ok(1)
            }
            "wait-boolean" | "wait-int32" => {
                let Some(p) = self.target(a, argc, 2) else { return ok(0) };
                if n < 2 || self.inner.procs[p].vars.get(&a[0]).copied().unwrap_or(0) == a[1] {
                    return ok(0);
                }
                Ok(self.suspend(Wait::Var { process: p, var: a[0], value: a[1] }))
            }
            "set-boolean" | "set-int32" | "set-float" | "set-symbol" => ok(self.set_var(a, argc)),
            "get-boolean" | "get-int32" | "get-float" | "get-symbol" => ok(self.get_var(a, argc)),
            "tag-symbol" | "tag-int32" | "tag-boolean" | "tag-float" => {
                let v = match self.tag(a[0]) {
                    Some(Property::Sid(s)) => s,
                    Some(Property::Int(i)) => i as u32,
                    Some(Property::Float(f)) => if name == "tag-float" { f.to_bits() } else { f as i32 as u32 },
                    Some(Property::Str(s)) => sid(&s),
                    Some(Property::Vec4(_)) | None => {
                        self.inner.error(format!("{name}: no tag {:#010x}", a[0]));
                        0
                    }
                };
                ok(if name == "tag-float" && matches!(self.tag(a[0]), Some(Property::Int(_))) { (v as i32 as f32).to_bits() } else { v })
            }
            "tag-specified?" => b(self.tag(a[0]).is_some()),
            "send-event" => {
                if n < 2 {
                    return ok(0);
                }
                let args = a[2..n.min(MAX_ARGS)].to_vec();
                ok(self.send(a[0], a[1], args))
            }
            "event-arg-int32" | "event-arg-float" | "event-arg-symbol" | "event-arg-boolean" => {
                let i = a[0] as usize;
                let v = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == KIND_EVENT)).and_then(|g| g.args.get(i).copied());
                ok(v.unwrap_or(0))
            }
            "spawn-script" => {
                // The process starts with the next frame's tick.
                let Some(&addr) = dc.symbols.get(&a[0]) else {
                    self.inner.error(format!("spawn-script: {:#010x} not loaded", a[0]));
                    return ok(0);
                };
                if dc.mem.type_of(addr)? != TYPE_STATE_SCRIPT {
                    return ok(0);
                }
                let initial = dc.mem.u32(addr + 8)?;
                let state = state_by_name(dc, addr, initial)?.or(state_by_name(dc, addr, 0)?);
                let mut p = Process { name: a[0], script: addr, spawner: None, state, vars: HashMap::new(), groups: Vec::new(), events: VecDeque::new(), pending_go: None, dead: false, state_start: now };
                if let Some(s) = state {
                    if let Some(g) = block(dc, s, KIND_BEGIN, 0)? {
                        p.groups.push(group(dc, KIND_BEGIN, 0, Vec::new(), g)?);
                    }
                }
                self.inner.procs.push(p);
                ok(1)
            }
            "end-script" => {
                if let Some(p) = self.proc {
                    self.inner.procs[p].dead = true;
                }
                Ok((0, Status::Abort))
            }
            // (player-in-region? region) / (object-in-region? object region):
            // `sub_F7D94`, see `crate::trigger`.
            "player-in-region?" if n > 0 => b(self.world.in_region(sid("player"), a[0])),
            "object-in-region?" if n > 1 => {
                let object = if a[0] == sid("player") { a[0] } else { self.inner.process(a[0]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[0]) };
                b(self.world.in_region(object, a[1]))
            }
            // NPC bookkeeping. (npc-alive? name) asks the NPC manager's
            // process (`sub_1E2968`) for not-dead (vtable +0x128, ^1);
            // (npc-killed? name) for dead, else the spawner's killed flag
            // (`sub_22A900`). Without a name they use the caller.
            "npc-alive?" => {
                let who = self.npc_name(a, n);
                b(self.world.npc(who) == Some(false))
            }
            "npc-killed?" => {
                let who = self.npc_name(a, n);
                b(self.world.npc(who).unwrap_or(false) || self.world.npc_killed(who))
            }
            "npc-die" => {
                let who = self.npc_name(a, n);
                b(self.world.npc_die(who))
            }
            // The manager's live count (`sub_221708`) and its fixed
            // capacity, 15 (`sub_221070`).
            "npc-alive-count" => ok(self.world.npc_alive_count()),
            "npc-max-count" => ok(15),
            // Group forms over an entity group's members (inferred from
            // the names; the natives are not decompiled).
            "npc-group-alive-count" | "npc-group-dead-count" | "npc-group-find-first-alive" if n > 0 => {
                let members = Self::group_members(dc, a[0])?;
                let alive = |w: &dyn World, m: u32| w.npc(m) == Some(false);
                let dead = |w: &dyn World, m: u32| w.npc(m).unwrap_or(false) || w.npc_killed(m);
                match name {
                    "npc-group-alive-count" => ok(members.iter().filter(|m| alive(&*self.world, **m)).count() as u32),
                    "npc-group-dead-count" => ok(members.iter().filter(|m| dead(&*self.world, **m)).count() as u32),
                    _ => ok(members.iter().copied().find(|m| alive(&*self.world, *m)).unwrap_or(0)),
                }
            }
            // (part-module-loaded? module): whether the particle module is
            // in the loaded-module table (`sub_765C50`).
            "part-module-loaded?" if n > 0 => b(dc.module(a[0]).is_some()),
            // Entity groups (`entity-group` symbols: `{count, {ptr,
            // member}…}`). (for-group group fn args…) calls `fn` (a native
            // or lambda) with each member and the args (`sub_A1BF0`);
            // (group-and group fn args…) is true while every call is
            // (`sub_A1A40`).
            "for-group" | "group-and" | "group-or" if n > 1 => {
                let members = Self::group_members(dc, a[0])?;
                let extra: Vec<u32> = a[2..n.min(MAX_ARGS)].to_vec();
                let mut all = true;
                let mut any = false;
                for m in members {
                    let args: Vec<u32> = std::iter::once(m).chain(extra.iter().copied()).collect();
                    let v = self.invoke(dc, a[1], &args)?;
                    all &= v != 0;
                    any |= v != 0;
                }
                match name {
                    "group-and" => b(all),
                    "group-or" => b(any),
                    _ => ok(0),
                }
            }
            // (group-is-member-symbol? group symbol), `sub_A13BC`.
            "group-is-member-symbol?" if n > 1 => b(Self::group_members(dc, a[0])?.contains(&a[1])),
            // (group-random group): a member chosen at random.
            "group-random" if n > 0 => {
                let members = Self::group_members(dc, a[0])?;
                if members.is_empty() {
                    return ok(0);
                }
                let r = self.next_random();
                ok(members[(r as usize) % members.len()])
            }
            // (symbol-eq? a b).
            "symbol-eq?" => b(n > 1 && a[0] == a[1]),
            // (lerp-scale a b c d x): `t = clamp((x − a) / (b − a), 0, 1)`,
            // `c (1 − t) + d t` (`sub_1416CC`, shared with the trees).
            "lerp-scale" if n == 5 => {
                let f = |i: usize| Self::arg_f(a, i);
                let t = ((f(4) - f(0)) / (f(1) - f(0))).max(0.);
                let t = if t.is_nan() { 0. } else { t.min(1.) };
                ok(((1. - t) * f(2) + t * f(3)).to_bits())
            }
            // (random-float): uniform in [0, 1) (`sub_90A9F0`).
            "random-float" => {
                let r = self.next_random();
                ok(((r >> 8) as f32 / (1u32 << 24) as f32).to_bits())
            }
            // (get-locator-distance a b): distance between two locator
            // records `{x, y, z, 1, …}`.
            "get-locator-distance" => {
                if n < 2 || a[0] == 0 || a[1] == 0 {
                    return ok(0f32.to_bits());
                }
                let v = |at: u32| -> Result<[f32; 3]> { Ok([f32::from_bits(dc.mem.u32(at)?), f32::from_bits(dc.mem.u32(at + 4)?), f32::from_bits(dc.mem.u32(at + 8)?)]) };
                let (p, q) = (v(a[0])?, v(a[1])?);
                ok(((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt().to_bits())
            }
            // (set-visible object on?) (`sub_13AE98`: any non-zero shows).
            "set-visible" if n == 2 => {
                let object = self.inner.process(a[0]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[0]);
                b(self.world.set_visible(object, a[1] != 0))
            }
            // (object-on-object? object support): the object stands on the
            // support's rigid body (`sub_143C54`).
            "object-on-object?" if n == 2 => {
                let object = |x: u32| self.inner.process(x).and_then(|p| self.inner.procs[p].spawner).unwrap_or(x);
                b(self.world.object_on_object(object(a[0]), object(a[1])))
            }
            // (get-object-id object): the process's id word (+76); here the
            // object's name.
            "get-object-id" => ok(if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) }),
            // (get-object-state object): its state-script state, like
            // `get-script-state` (`sub_1429E8`).
            "get-object-state" => ok(self.target(a, argc, 0).and_then(|p| self.inner.procs[p].state).map(|s| dc.mem.u32(s)).transpose()?.unwrap_or(0)),
            "get-script-state" => ok(self.target(a, argc, 0).and_then(|p| self.inner.procs[p].state).map(|s| dc.mem.u32(s)).transpose()?.unwrap_or(0)),
            "get-current-time" => ok((now as f32).to_bits()),
            // (pad-get-info pad) → `{held, pressed, released, left x, y,
            // right x, y}` (`sub_1459E8`). Button words go through
            // `sub_137B28`, whose bit mapping is not recovered: they are 0.
            "pad-get-info" => {
                let pad = if n > 0 { a[0] } else { 0 };
                let at = match self.inner.pads.get(&pad) {
                    Some(at) => *at,
                    None => {
                        let at = dc.mem.alloc(28)?;
                        self.inner.pads.insert(pad, at);
                        at
                    }
                };
                for (k, v) in self.world.sticks(pad).iter().enumerate() {
                    dc.mem.write_u32(at + 12 + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // The movement-type table entry (`sub_678348`) is recorded; the
            // player movement does not use it yet.
            "set-walk-type" | "set-run-type" => {
                self.inner.movement[usize::from(name == "set-run-type")] = a[0];
                ok(0)
            }
            "print-string" => {
                let text = if a[0] == 0 { String::new() } else { dc.mem.cstr(a[0]).unwrap_or_default() };
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            "levels-fully-spawned?" | "levels-fully-spawned-ignore-player?" => b((0..n.max(1)).all(|i| self.world.level_spawned(a[i]))),
            "object-spawned?" => b(n > 0 && (self.world.object_spawned(a[0]) || self.inner.process(a[0]).is_some())),
            "spawn-entity" => b(n > 0 && self.world.spawn(a[0])),
            "kill-entity" => {
                let killed = n > 0 && self.world.kill(a[0]);
                if let Some(p) = self.inner.process(a[0]) {
                    self.inner.procs[p].dead = true;
                }
                b(killed)
            }
            // (animate object clip key value …) / (wait-animate …): the
            // object is a process (its spawner) or a spawner name.
            "animate" | "wait-animate" => {
                if n < 2 {
                    return ok(0);
                }
                let object = self.inner.process(a[0]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[0]);
                let mut options = Animate::default();
                for k in (2..n.saturating_sub(1)).step_by(2) {
                    match a[k] {
                        2 => options.blend_in = Some(Self::arg_f(a, k + 1)),
                        4 => options.blend_out = Some(Self::arg_f(a, k + 1)),
                        7 if a[k + 1] != 0 => {
                            let f = |i: u32| dc.mem.f32(a[k + 1] + 4 * i).unwrap_or(f32::NAN);
                            let (pos, rot) = ([f(0), f(1), f(2)], [f(4), f(5), f(6), f(7)]);
                            if pos.iter().chain(&rot).all(|v| v.is_finite()) {
                                options.locator = Some((pos, rot));
                            }
                        }
                        _ => {}
                    }
                }
                let started = self.world.animate(object, a[1], &options);
                if name == "wait-animate" && started { Ok(self.suspend(Wait::Animation(object))) } else { b(started) }
            }
            // (get-animation-phase object): 0–1, 0 when nothing plays.
            "get-animation-phase" => {
                let object = self.inner.process(a[0]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[0]);
                ok(self.world.animation_phase(object).unwrap_or(0.).to_bits())
            }
            // (get-object-locator object) / (get-ap-ref-locator ? object):
            // the object's locator; a process stands at its spawner and
            // `player` where the player is.
            "get-object-locator" | "get-ap-ref-locator" => {
                let i = usize::from(name == "get-ap-ref-locator" && n > 1);
                let object = self.inner.process(a[i]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[i]);
                let Some((pos, rot)) = self.world.locator(object) else { return ok(0) };
                let at = dc.mem.alloc(32)?;
                for (k, v) in pos.iter().chain(&[1.]).chain(&rot).enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // (spline-locator-at-distance spline distance): a locator at the
            // point, facing along the tangent (`sub_1666F8`).
            "spline-locator-at-distance" if n > 1 => {
                let Some((pos, tangent)) = self.world.spline_at(a[0], Self::arg_f(a, 1)) else {
                    self.inner.error(format!("{name}: no spline {:#010x}", a[0]));
                    return ok(0);
                };
                let forward = glam::Vec3::from_array(tangent);
                let rot = glam::Quat::from_rotation_arc(glam::Vec3::Z, forward.normalize_or(glam::Vec3::Z));
                let at = dc.mem.alloc(32)?;
                for (k, v) in pos.iter().chain(&[1.]).chain(&rot.to_array()).enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // (spline-distance-locator spline locator) /
            // (spline-distance-spawner spline spawner): arc distance of the
            // closest point (`sub_165BE0` → `sub_37224`).
            "spline-distance-locator" | "spline-distance-spawner" if n > 1 && a[1] != 0 => {
                let point = if name == "spline-distance-locator" {
                    Some([dc.mem.f32(a[1])?, dc.mem.f32(a[1] + 4)?, dc.mem.f32(a[1] + 8)?])
                } else {
                    self.world.locator(a[1]).map(|(p, _)| p)
                };
                ok(point.and_then(|p| self.world.spline_distance(a[0], p)).unwrap_or(0.).to_bits())
            }
            // (get-joint-locator object joint) / (get-joint-position object
            // joint) / (get-object-position object): heap locator or
            // position records (`sub_138AC0`, `sub_14F708`, `sub_143240`).
            "get-joint-locator" | "get-joint-position" | "get-object-position" => {
                let object = self.npc_name(a, n);
                let frame = if name == "get-object-position" { self.world.locator(object) } else if n > 1 { self.world.joint_locator(object, a[1]) } else { None };
                let Some((pos, rot)) = frame else { return ok(0) };
                let at = dc.mem.alloc(32)?;
                let words: Vec<f32> = if name == "get-joint-locator" { pos.iter().chain(&[1.]).chain(&rot).copied().collect() } else { pos.iter().chain(&[1.]).copied().collect() };
                for (k, v) in words.iter().enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // The optional float is the cone angle: sub_7B998 and
            // sub_7AE48 multiply it by π/180; zero on add selects a point.
            "add-dynamic-light" | "update-dynamic-light" if n > 2 && a[1] != 0 => {
                let mut frame = [0.; 8];
                for (i, value) in frame.iter_mut().enumerate() {
                    *value = dc.mem.f32(a[1] + 4 * i as u32)?;
                }
                let locator = LightLocator::Frame(
                    [frame[0], frame[1], frame[2]],
                    [frame[4], frame[5], frame[6], frame[7]],
                );
                let radius = Self::arg_f(a, 2);
                let cone = if n > 3 { Self::arg_f(a, 3) } else { 0. };
                if name == "add-dynamic-light" {
                    self.world.add_dynamic_light(a[0], locator, radius, cone, n < 5 || a[4] != 0);
                } else {
                    self.world.update_dynamic_light(a[0], locator, radius, cone);
                }
                ok(0)
            }
            "add-dynamic-light-joint" if n > 3 => {
                let object = self.inner.process(a[1]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(a[1]);
                let joint = a[2];
                if self.world.joint_locator(object, joint).is_some() {
                    let cone = if n > 4 { Self::arg_f(a, 4) } else { 0. };
                    self.world.add_dynamic_light(a[0], LightLocator::Joint { object, joint }, Self::arg_f(a, 3), cone, n < 6 || a[5] != 0);
                }
                ok(0)
            }
            // (set-dynamic-light-color name r g b [gain]), sub_143678.
            "set-dynamic-light-color" if n > 3 => b(self.world.dynamic_light_color(
                a[0],
                [Self::arg_f(a, 1), Self::arg_f(a, 2), Self::arg_f(a, 3)],
                if n > 4 { Self::arg_f(a, 4) } else { 1. },
            )),
            "remove-dynamic-light" if n > 0 => {
                self.world.remove_dynamic_light(a[0]);
                ok(0)
            }
            // (spawn-particles-at-joint group object joint [parent] [flag]),
            // (spawn-particles-attached group object …) and
            // (spawn-particles-at-locator group locator …): start an effect
            // group, returning its handle (`sub_1392E0` sends the object a
            // spawn-particles event and returns the effect's id).
            "spawn-particles-at-joint" | "spawn-particles-attached" if n > 1 => {
                let frame = if name == "spawn-particles-at-joint" && n > 2 { self.world.joint_locator(a[1], a[2]).or_else(|| self.world.locator(a[1])) } else { self.world.locator(a[1]) };
                ok(frame.map_or(0, |(p, q)| self.world.spawn_particles(a[0], p, q)))
            }
            "spawn-particles-at-locator" if n > 1 && a[1] != 0 => {
                let f = |k: u32| dc.mem.f32(a[1] + 4 * k);
                let (p, q) = ([f(0)?, f(1)?, f(2)?], [f(4)?, f(5)?, f(6)?, f(7)?]);
                let q = if q.iter().all(|v| *v == 0.) { [0., 0., 0., 1.] } else { q };
                ok(self.world.spawn_particles(a[0], p, q))
            }
            // (kill-particles handle [immediately]) (`sub_13D7C8`); a
            // gradual stop (live particles finishing) is not reproduced.
            "kill-particles" if n > 0 => {
                self.world.kill_particles(a[0]);
                ok(0)
            }
            // Statistics (`sub_67D230` and neighbours): a table of
            // `{stat, qualifier}` counters; one argument means qualifier 0.
            // (increment-stat stat qualifier) adds 1; (reset-stat …) clears.
            "stat-value?" if n > 0 => ok(*self.inner.stats.get(&(a[0], if n > 1 { a[1] } else { 0 })).unwrap_or(&0) as u32),
            "increment-stat" | "increment-task-stat" | "increment-stat-difficulty" if n > 0 => {
                *self.inner.stats.entry((a[0], if n > 1 { a[1] } else { 0 })).or_default() += 1;
                b(true)
            }
            "reset-stat" if n > 0 => b(self.inner.stats.remove(&(a[0], if n > 1 { a[1] } else { 0 })).is_some()),
            // (get-dda): the dynamic difficulty value (`sub_57598`): the
            // least of the players' values below the global cap, else the
            // cap, 5.0 (the float at 0xe12ca0). Per-player adjustment is
            // not modelled, so the cap is returned.
            "get-dda" => ok(5f32.to_bits()),
            // Rumble needs a host vibration service.
            "start-rumble" | "stop-rumble" | "do-rumble-ptr" | "stop-script-rumbles" => ok(0),
            // (set-single-shader-instance-param object name value): one
            // component of the object's `g_optionalInstanceParameters`.
            "set-single-shader-instance-param" if n > 2 => {
                let object = self.npc_name(a, n);
                match INSTANCE_PARAMS.iter().find(|(k, _, _)| *k == a[1]) {
                    Some(&(_, v, c)) => b(self.world.set_instance_param(object, v, c, Self::arg_f(a, 2))),
                    None => {
                        self.inner.error(format!("{name}: unknown parameter {:#010x}", a[1]));
                        ok(0)
                    }
                }
            }
            // (set-shader-instance-param object name x y z w), sub_145BB8.
            "set-shader-instance-param" if n > 5 => {
                let object = self.npc_name(a, n);
                match INSTANCE_VECTORS.iter().position(|&selector| selector == a[1]) {
                    Some(vector) => b(self.world.set_instance_vector(
                        object, vector, std::array::from_fn(|i| Self::arg_f(a, i + 2)),
                    )),
                    None => {
                        self.inner.error(format!("{name}: unknown parameter {:#010x}", a[1]));
                        ok(0)
                    }
                }
            }
            // (is-on-screen? locator): the point against the camera frustum
            // (`sub_14108C`).
            "is-on-screen?" if n > 0 && a[0] != 0 => b(self.world.on_screen([dc.mem.f32(a[0])?, dc.mem.f32(a[0] + 4)?, dc.mem.f32(a[0] + 8)?])),
            // (set-water-offset name vector): the named water body moves to
            // its locator plus the offset (`sub_13F068` → `sub_850BD0`,
            // `sub_8507E8`).
            "set-water-offset" if n > 1 && a[1] != 0 => {
                let offset = [dc.mem.f32(a[1])?, dc.mem.f32(a[1] + 4)?, dc.mem.f32(a[1] + 8)?];
                b(self.world.set_water_offset(a[0], offset))
            }
            // (camera-local-space): switches three camera settings through
            // the settings table (`sub_142788` → `sub_15F934`, `sub_15F480`);
            // the follow camera here has no such settings.
            "camera-local-space" => ok(0),
            "get-spawner-locator" | "get-spawner-position" => {
                let Some((pos, rot)) = self.world.locator(a[0]) else {
                    self.inner.error(format!("{name}: no spawner {:#010x}", a[0]));
                    return ok(0);
                };
                // A heap `{x, y, z, 1, qx, qy, qz, qw}` record.
                let at = dc.mem.alloc(32)?;
                for (k, v) in pos.iter().chain(&[1.]).chain(&rot).enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // (fade-to-black alpha seconds), `sub_87D20(160, alpha, seconds)`.
            "fade-to-black" | "fade-to-white" => {
                let white = name == "fade-to-white";
                self.world.fade(if white { [1.; 3] } else { [0.; 3] }, Self::arg_f(a, 0), if n > 1 { Self::arg_f(a, 1) } else { 0. });
                ok(0)
            }
            // (set-black-fade seconds on?): fade fully in or out.
            "set-black-fade" | "set-white-fade" => {
                let white = name == "set-white-fade";
                let on = n > 1 && a[1] != 0;
                self.world.fade(if white { [1.; 3] } else { [0.; 3] }, if on { 1. } else { 0. }, Self::arg_f(a, 0));
                ok(0)
            }
            "camera-enable" => {
                self.world.camera(a[0], if n > 2 { Self::arg_f(a, 2) } else { 0. });
                ok(0)
            }
            "camera-disable" | "camera-abandon" => {
                self.world.camera_disable((n > 0 && a[0] != 0).then_some(a[0]));
                ok(0)
            }
            "camera-disable-all" => {
                self.world.camera_disable(None);
                ok(0)
            }
            // (camera-animation-enable name clip blend): `DcCameraAnimationEnable`
            // (`sub_366730`) keeps up to 10 named camera animations; the
            // scripts call it every frame while one should run.
            "camera-animation-enable" => {
                let clip = if n > 1 && a[1] != 0 { a[1] } else { a[0] };
                self.world.camera_animation(a[0], clip, if n > 2 { Self::arg_f(a, 2) } else { 0. });
                ok(0)
            }
            // (camera-animation-disable name), `sub_364550`.
            "camera-animation-disable" => {
                self.world.camera_animation_disable((n > 0 && a[0] != 0).then_some(a[0]));
                ok(0)
            }
            // The argument is a control mask (`player-controls-all` returns
            // 0xffff, `player-controls-none` 0): no mask leaves controls on.
            "player-disable-controls" => {
                self.world.player_controls(n > 0 && a[0] == 0);
                ok(0)
            }
            "task-find" => {
                // (task-find node subnode) → subnode record.
                let found = self.inner.tasks.find(if n > 1 { a[1] } else { a[0] });
                ok(found.map(|(i, j)| self.inner.tasks.nodes[i].subnodes[j].address).unwrap_or(0))
            }
            "task-subnode-complete" | "task-subnode-play" => {
                let Some(at) = self.inner.tasks.nodes.iter().enumerate().find_map(|(i, nd)| nd.subnodes.iter().position(|s| s.address == a[0]).map(|j| (i, j))) else { return b(false) };
                if name == "task-subnode-play" {
                    self.inner.task_change = Some(at);
                    return b(true);
                }
                let record = dc.mem.u32(a[0] + 0x1c)?;
                if record != 0 {
                    dc.mem.write_u32(record, 3)?;
                }
                if self.inner.task == Some(at) {
                    self.inner.task_change = self.inner.tasks.next(at);
                }
                b(true)
            }
            "task-node-active?" => b(self.inner.task.is_some_and(|(i, _)| self.inner.tasks.nodes[i].id == a[0])),
            "task-subnode-active?" => {
                let (Some(current), Some(at)) = (self.inner.task, self.inner.tasks.nodes.iter().enumerate().find_map(|(i, nd)| nd.subnodes.iter().position(|s| s.address == a[0]).map(|j| (i, j)))) else { return b(false) };
                b(at == current)
            }
            // (jump label): continue the current track at the entry
            // labelled `label` (`sub_7D4678`).
            "jump" => {
                if n == 0 {
                    self.inner.error("jump: not enough arguments".into());
                    return ok(0);
                }
                let Some((pairs, count)) = self.track.filter(|_| self.proc.is_some()) else {
                    self.inner.error("jump: Called outside state script context - ignored".into());
                    return ok(0);
                };
                if a[0] == 0 {
                    self.inner.error("jump: Null or empty label name".into());
                    return ok(0);
                }
                for k in 0..count {
                    if dc.mem.u32(pairs + 8 * k)? == a[0] {
                        self.jump = Some(k);
                        return ok(1);
                    }
                }
                self.inner.error(format!("jump: Unable to jump to non-existent label {:#010x}", a[0]));
                ok(0)
            }
            // (wait-until-frame frame): until `frame`/30 s into the state
            // (`sub_7D63B8`).
            "wait-until-frame" => {
                if n == 0 {
                    self.inner.error("wait-until-frame: Expected at least one argument: (abs-frame int32)".into());
                    return ok(0);
                }
                let Some(p) = self.proc else { return ok(1) };
                let at = self.inner.procs[p].state_start + f64::from(a[0] as i32 as f32 * FRAME_SECONDS);
                if now >= at {
                    return ok(1);
                }
                let (value, status) = self.suspend(Wait::Until(at));
                Ok((value, status))
            }
            // (tag-string tag): the calling process's spawner string tag
            // as a script string (`sub_7C9B24`).
            "tag-string" => {
                if n != 1 {
                    self.inner.error("tag-string: expected one argument".into());
                    return ok(0);
                }
                if self.proc.is_none() {
                    self.inner.error("tag-string: Called outside state script context - ignored".into());
                    return ok(0);
                }
                match self.tag(a[0]) {
                    Some(Property::Str(text)) => ok(dc.mem.alloc_str(&text)?),
                    _ => ok(0),
                }
            }
            // (string<-string-id id): the id's name from the `StringId`
            // table (`sub_84A2EC`); names the table lacks read "(#f)".
            "string<-string-id" => {
                let text = if n == 0 || a[0] == 0 { "(null)".to_owned() } else { self.inner.names.get(&a[0]).cloned().unwrap_or_else(|| "(#f)".into()) };
                ok(dc.mem.alloc_str(&text)?)
            }
            "print-symbol" => {
                let id = if n > 0 { a[0] } else { 0 };
                let text = format!("print-symbol: {} ({id})", self.inner.names.get(&id).map_or("(#f)", String::as_str));
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            "print-float" => {
                let text = format!("print-float: {:4.2}", if n > 0 { Self::arg_f(a, 0) } else { 0. });
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            // (ss-error format args…): a script error with up to two
            // arguments substituted (`sub_AC6504`).
            "ss-error" => {
                let format = if n > 0 && a[0] != 0 { dc.mem.cstr(a[0]).unwrap_or_default() } else { String::new() };
                let mut text = String::new();
                let mut args = a[1..n.clamp(1, 3)].iter();
                let mut chars = format.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '%' && chars.peek().is_some_and(|c| *c != '%') {
                        let spec = chars.next().unwrap_or('d');
                        let v = args.next().copied().unwrap_or(0);
                        match spec {
                            's' => text.push_str(&dc.mem.cstr(v).unwrap_or_default()),
                            'f' | 'g' => text.push_str(&f32::from_bits(v).to_string()),
                            'x' => text.push_str(&format!("{v:x}")),
                            _ => text.push_str(&(v as i32).to_string()),
                        }
                    } else if c == '%' {
                        chars.next();
                        text.push('%');
                    } else {
                        text.push(c);
                    }
                }
                self.inner.error(text);
                ok(0)
            }
            // (make-vector3 x y z): a heap vector, components default 0.
            "make-vector3" => {
                let at = dc.mem.alloc(12)?;
                for k in 0..3 {
                    dc.mem.write_u32(at + 4 * k as u32, if n > k { a[k] } else { 0 })?;
                }
                ok(at)
            }
            // (random-integer n): ⌊random-float × n⌋.
            "random-integer" => {
                if n == 0 {
                    self.inner.error("random-integer: Expected at least one argument - returning zero always".into());
                    return ok(0);
                }
                let r = (self.next_random() >> 8) as f32 / (1u32 << 24) as f32;
                ok((r * a[0] as i32 as f32).floor() as i32 as u32)
            }
            // (group-count group) / (group-at group index): entity groups;
            // the built-in groups count live NPCs by type (`sub_A0E58`).
            "group-count" | "group-at" => {
                let group = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
                if name == "group-count" {
                    if let Some(kind) = builtin_group(group) {
                        let count = self.world.agents_ref().map_or(0, |x| x.agents.keys().filter(|k| kind.is_none_or(|t| self.world.object_type(**k) == Some(t))).count());
                        return ok(count as u32);
                    }
                }
                let known = dc.symbols.get(&group).is_some_and(|g| dc.mem.type_of(*g).ok() == Some(sid("entity-group")));
                if !known {
                    self.inner.error(format!("group-count: Unknown group {group:#010x}"));
                    return ok(0);
                }
                let members = Self::group_members(dc, group)?;
                if name == "group-count" {
                    return ok(members.len() as u32);
                }
                let index = if n > 1 { a[1] as i32 } else { -1 };
                match usize::try_from(index).ok().and_then(|i| members.get(i)) {
                    Some(m) => ok(*m),
                    None => {
                        self.inner.error(format!("group-count: {group:#010x} Index out of bounds {index}"));
                        ok(0)
                    }
                }
            }
            "get-difficulty" => ok(self.inner.difficulty),
            // (disc? id): the `-disc` command-line id (game state +0x1DA0,
            // e.g. `e3`); unset on the retail disc.
            "disc?" => b(n > 0 && self.inner.disc.is_some_and(|d| d == a[0])),
            // (is-fma): the game state's flag byte +0x1DC5.
            "is-fma" => b(self.inner.fma),
            // (game-task-graph-login graph): `sub_7E4518` logs a task graph
            // in; the graph of the boot module is already loaded.
            "game-task-graph-login" => {
                if n > 0 && a[0] != 0 && !self.inner.task_graphs.contains(&a[0]) {
                    self.inner.task_graphs.push(a[0]);
                }
                ok(0)
            }
            // (update-task-menu): refresh the chapter menu (`sub_7EC0F8`).
            "update-task-menu" => {
                self.inner.task_menu_dirty = true;
                ok(0)
            }
            // (camera-fade-out name): the named camera object fades out of
            // the blend (`sub_3564E0`); the shared camera releases it.
            "camera-fade-out" => {
                let camera = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
                self.world.camera_disable(Some(camera));
                ok(0)
            }
            // The guarded natives above with missing or null arguments
            // print the original message and return 0.
            "add-dynamic-light" | "update-dynamic-light" | "add-dynamic-light-joint" | "set-dynamic-light-color" | "remove-dynamic-light"
            | "spline-distance-locator" | "spline-distance-spawner" | "spline-locator-at-distance" | "is-on-screen?" | "set-water-offset"
            | "spawn-particles-at-locator" | "spawn-particles-at-joint" | "spawn-particles-attached" | "kill-particles"
            | "set-single-shader-instance-param" | "set-shader-instance-param" | "stat-value?" | "increment-stat" | "increment-task-stat"
            | "increment-stat-difficulty" | "reset-stat" | "player-in-region?" | "object-in-region?" | "set-visible" | "object-on-object?"
            | "for-group" | "group-and" | "group-or" | "group-is-member-symbol?" | "group-random" | "lerp-scale" | "part-module-loaded?"
            | "npc-group-alive-count" | "npc-group-dead-count" | "npc-group-find-first-alive" | "npc-mark-as-defunct" => {
                let detail = match name {
                    "add-dynamic-light" | "update-dynamic-light" if n > 2 => "boundframe not specified",
                    "spline-distance-locator" | "spline-distance-spawner" if n > 1 => "NULL or empty spline name.",
                    "is-on-screen?" => "passed NULL location.",
                    "set-water-offset" if n > 1 => "NULL offset vector",
                    "spawn-particles-at-locator" => "Expected two args: (effect-name string) (where locator)",
                    _ => "not enough arguments",
                };
                self.inner.error(format!("{name}: {detail}"));
                ok(0)
            }
            _ => {
                *self.inner.missing.entry(name).or_default() += 1;
                ok(0)
            }
        }
    }
}
