// SPDX-License-Identifier: MPL-2.0
//! The player's locomotion controller over the `anim-player` state machine
//! and blend trees.
//!
//! The PPU controller is not decompiled; its requests are reproduced from
//! the transition table: from rest, input relative to the facing direction
//! requests `idle-run-f`/`-r`/`-l`/`-b` (front within 45°, sides to 135°,
//! else back); a reversal while running requests `run-180`; releasing the
//! stick requests `run-idle` (or `idle` from states that name it); ten
//! seconds at rest requests an ambient idle. Jumps, falls and landings
//! have no incoming transitions and are entered directly: Space enters
//! `run-jump-long` (moving, with a 4.5 m/s lift) or `jump-in-place`;
//! leaving the ground by more than 0.5 m enters `fall`; touching down
//! from `fall` enters `fall-land-soft`/`-medium`/`-hard` (drops under
//! 1.5 m, under 4 m, beyond) `-run` or `-idle`, and from `jump-long` the
//! `jump-long-land-run` state or the `idle` request. `auto` transitions
//! chain the rest (`run-jump-long` → `jump-long`, `jump-in-place` →
//! `jump-in-place-land` → `idle`).
//!
//! Each frame the current state is presented by an [`Animator`]. Ground
//! speed is the blended clips' root motion (`align`) along the facing
//! direction. Thresholds and rates in this module are not native values
//! (SATD-nd-retail-runtime).
//!
//! Hanging and climbing ([`Mode::Hang`], [`Mode::Climb`]) follow an edge:
//! the hang and climb clips' `apReference` channel locates the edge (the
//! action-pack reference) relative to their `align`, so with `AP =
//! align(0) · apReference(0)` the root stands at `edge · AP⁻¹ · align(t)`
//! (`ledge-grab-idle-hurt`: the edge 1.53 m above and 0.5 m ahead of the
//! root; the chest and knee climb steps agree on the root 0.6 m below and
//! 0.34 m behind the edge where they meet). From the hang, sideways input
//! requests `ledge-shimmy-left`/`-right`, releasing it `ledge-grab`;
//! forward input over a standable top enters `ledge-climb-up` (→ `-2` by
//! `auto`, then the `idle`/`run` request picks `climb-onto-1-75m^…`);
//! the drop input enters `fall`. Falling or jumping with an edge in reach
//! enters `fall-grab-edge`, then `ledge-grab`. Jumping from the hang to
//! another edge plays `ledge-jump-begin` → `-mid` → `-end`; the native
//! controller picks `ledge-jump-up-short`/`-long`/`-right-…` clips by name
//! (their `apReference` places the target edge 2.43 m or 3.90 m above
//! the root), the tree here plays its default.
use crate::anim_state::{Machine, StateTable};
use crate::anim_tree::{Graph, Inputs};
use crate::dc::sid;
use anyhow::Result;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_4, PI};

/// States the controller drives; their trees' clips are the player rig's.
pub const STATES: &[&str] = &[
    "idle",
    "idle-run-f",
    "idle-run-fr",
    "idle-run-fl",
    "idle-run-r",
    "idle-run-l",
    "idle-run-b",
    "idle-run-br",
    "idle-run-bl",
    "run",
    "run-idle",
    "run-180",
    "idle-turn-r",
    "idle-turn-l",
    "idle-turn-br",
    "idle-turn-bl",
    "ambient-casual",
    "ambient-stand-a1",
    "ambient-stand-a2",
    "ambient-stand-a3",
    "jump-in-place",
    "jump-in-place-land",
    "run-jump-long",
    "jump-long",
    "jump-long-land-run",
    "jump-long-land-idle",
    "fall",
    "fall-land-soft-idle",
    "fall-land-soft-run",
    "fall-land-medium-idle",
    "fall-land-medium-run",
    "fall-land-hard-idle",
    "fall-land-hard-run",
    "ledge-grab",
    "ledge-shimmy-left",
    "ledge-shimmy-right",
    "ledge-climb-up",
    "ledge-climb-up-2",
    "climb-onto-1-75m^idle",
    "climb-onto-1-75m^run",
    "fall-grab-edge",
    "ledge-jump-begin",
    "ledge-jump-mid",
    "ledge-jump-end",
    "aim-idle",
    "aim-run-forward",
    "aim-run-backward",
    "throw-grenade",
    "air-punch",
    "hero-generic-hit-reaction",
    "hero-generic-hit-reaction-left",
    "hero-generic-hit-reaction-right",
    "hero-multi-death-front",
    "hero-multi-death-back",
    "death-stand-explosion-front",
    "death-stand-explosion-back",
    "fall-death",
    "free-rope-grab",
    "free-rope-idle",
    "free-rope-swing-mid^front",
    "free-rope-swing-mid^back",
    "free-rope-swing-front^mid",
    "free-rope-swing-back^mid",
    "free-rope^jump-long",
    "ledge-overhead-swing-enter",
    "ledge-overhead-swing",
    "ledge-overhead-swing-jump",
    "ledge-overhead-forward",
    "wall-jump-ant-90",
    "wall-jump-90-long-jump",
    "wall-rope-enter-bottom-grab",
    "jump^wall-rope-idle",
    "wall-rope-idle",
    "wall-rope-climb-up",
    "wall-rope-climb-down",
    "wall-rope-exit-bottom^idle",
    "wall-rope-climb-onto-1-75m^idle",
    "wall-rope-jump",
    "wall-rope-idle^swing",
    "wall-rope-swing",
    "wall-rope-swing^idle",
];
const WALL_ROPE: &[&str] = &["wall-rope-enter-bottom-grab", "jump^wall-rope-idle", "wall-rope-idle", "wall-rope-climb-up", "wall-rope-climb-down", "wall-rope-idle^swing", "wall-rope-swing", "wall-rope-swing^idle"];
const ROPE: &[&str] = &["free-rope-grab", "free-rope-idle", "free-rope-swing-mid^front", "free-rope-swing-mid^back", "free-rope-swing-front^mid", "free-rope-swing-back^mid"];
/// Airborne states besides the jumps and the fall: rope and bar
/// releases, wall jumps.
/// Swing rate (rad/s) above which a wall rope plays its swing states.
const WALL_ROPE_SWING: f32 = 0.6;
const FLIGHT: &[&str] = &["free-rope^jump-long", "ledge-overhead-swing-jump", "wall-jump-90-long-jump", "wall-rope-jump"];

/// One-shot ground actions that return to `idle` when their clip ends,
/// and the death states, which hold.
const ACTIONS: &[&str] = &["wall-rope-exit-bottom^idle", "wall-rope-climb-onto-1-75m^idle", "throw-grenade", "air-punch", "hero-generic-hit-reaction", "hero-generic-hit-reaction-left", "hero-generic-hit-reaction-right"];
const DEATHS: &[&str] = &["hero-multi-death-front", "hero-multi-death-back", "death-stand-explosion-front", "death-stand-explosion-back", "fall-death"];
const AIM: &[&str] = &["aim-idle", "aim-run-forward", "aim-run-backward"];

const HANG: &[&str] = &["ledge-grab", "ledge-shimmy-left", "ledge-shimmy-right", "fall-grab-edge", "ledge-overhead-swing-enter", "ledge-overhead-swing"];
const LEAP: &[&str] = &["ledge-jump-begin", "ledge-jump-mid", "ledge-jump-end", "climb-up-lh", "climb-up-rh", "climb-down-lh", "climb-down-rh", "ledge-overhead-forward"];
const CLIMB: &[&str] = &["ledge-climb-up", "ledge-climb-up-2", "climb-onto-1-75m^idle", "climb-onto-1-75m^run"];

/// How the adapter places the player for the current state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mode {
    #[default]
    Ground,
    /// On an edge: the root follows the grab point.
    Hang,
    /// Climbing onto an edge's top: the root follows the clip from the
    /// edge where the climb began.
    Climb,
    /// Jumping from one edge to another (`ledge-jump-begin` → `-mid` →
    /// `-end`, `auto`): the adapter moves the root between the grips.
    Leap,
    /// Swimming at a water surface (`swim-…`).
    Swim,
    /// In cover at a cover point (`edge-cover-…` low, `corner-cover-…`
    /// high): the root stays at the cover.
    Cover,
    /// On a free rope (`free-rope-…`): the adapter swings the root about
    /// the rope's anchor.
    Rope,
    /// On a wall rope (`wall-rope-…`): the adapter holds the root on the
    /// rope at the climbed height.
    WallRope,
}

const AMBIENT: &[&str] = &["ambient-casual", "ambient-stand-a1", "ambient-stand-a2", "ambient-stand-a3"];
const AMBIENT_AFTER: f32 = 10.;
const JUMP_LIFT: f32 = 4.5;
const FALL_AFTER: f32 = 0.5;
const TURN_RATE: f32 = 6.;
const LEAN_RATE: f32 = 8.;

/// What the controller knows of a decoded clip.
#[derive(Clone, Debug, Default)]
pub struct ClipInfo {
    pub name: String,
    pub duration: f32,
    /// `align` root speed, metres per second.
    pub speed: f32,
    pub additive: bool,
    pub fps: f32,
    /// `align` per frame (clip space).
    pub align: Vec<(Vec3, Quat)>,
    /// The action-pack reference in clip space, `align(0) ·
    /// apReference(0)`, when the clip has one.
    pub ap: Option<(Vec3, Quat)>,
}

impl ClipInfo {
    /// `align` at `time` seconds (clamped, interpolated).
    pub fn align_at(&self, time: f32) -> (Vec3, Quat) {
        let Some(&first) = self.align.first() else { return (Vec3::ZERO, Quat::IDENTITY) };
        let x = (time * self.fps.max(1.)).clamp(0., (self.align.len() - 1) as f32);
        let (i, f) = (x.floor() as usize, x.fract());
        let (a, b) = (self.align.get(i).copied().unwrap_or(first), self.align.get(i + 1).copied().unwrap_or(self.align[self.align.len() - 1]));
        (a.0.lerp(b.0, f), a.1.slerp(b.1, f))
    }
}

/// The root of the heaviest clip now: its name, `align` and reference.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub clip: String,
    pub align: (Vec3, Quat),
    /// `align` one frame earlier.
    pub before: (Vec3, Quat),
    pub ap: Option<(Vec3, Quat)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub clip: String,
    pub time: f32,
    pub weight: f32,
    pub additive: bool,
}

/// One frame of input.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    /// Desired world heading (radians, as `yaw`) while the stick is held.
    pub heading: Option<f32>,
    pub jump: bool,
    pub grounded: bool,
    /// Metres dropped since leaving the ground.
    pub dropped: f32,
    /// Airborne with an edge in reach of the hands.
    pub grab: bool,
    /// While hanging: sideways input (+1 right), forward input over a
    /// standable top, the drop input, and a jump to another edge.
    pub lateral: f32,
    pub climb: bool,
    pub drop: bool,
    pub leap: bool,
    /// Ground rise per metre ahead.
    pub slope: f32,
    pub movement: u32,
    pub weapon: u32,
    pub seconds: f32,
    /// Take cover of this kind (0 high-left, 1 high-right, 2 low-left, 3
    /// low-right), or leave cover.
    pub cover: Option<u8>,
    pub uncover: bool,
    /// While hanging: step to the hand hold above (+1) or below (−1)
    /// (wall and ladder climbing, `climb-up-lh/rh`, `climb-down-lh/rh`).
    pub step: i8,
    /// Deep water at the player's position, and the sprint input.
    pub swim: bool,
    pub movement_fast: bool,
    /// Aiming (the camera's yaw to face), throwing a grenade, a melee
    /// strike, a hit taken (side: −1 left, 0 front, 1 right) and death
    /// (the death state to play).
    pub aim: Option<f32>,
    pub throw: bool,
    pub melee: bool,
    pub hurt: Option<i8>,
    pub death: Option<&'static str>,
    /// A free rope within reach of the hands while airborne; on a rope,
    /// the swing's angular velocity (rad/s, + forward) and the release.
    pub rope_grab: bool,
    pub rope_swing: f32,
    /// The held edge is a free-standing bar (no wall under it).
    pub bar: bool,
    /// A wall right ahead during a long jump.
    pub wall_ahead: bool,
    /// A wall rope in reach: grab it from the air (`jump^wall-rope-idle`)
    /// or from the ground at its foot (`wall-rope-enter-bottom-grab`); on
    /// it, the climb input (+1 up, −1 down) and whether the top or the
    /// bottom is reached.
    pub wall_rope_grab: Option<bool>,
    pub wall_rope_climb: i8,
    pub wall_rope_top: bool,
    pub wall_rope_bottom: bool,
    /// The sideways swing rate on a wall rope (rad/s, + toward the
    /// player's right).
    pub wall_rope_swing: f32,
    /// Land hard whatever the drop (`player-force-hard-land`).
    pub hard_land: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Step {
    pub layers: Vec<Layer>,
    /// Ground speed along `yaw`.
    pub speed: f32,
    /// Upward velocity to apply (a jump take-off).
    pub lift: Option<f32>,
    pub state: Option<String>,
    pub mode: Mode,
    pub root: Option<Root>,
}

/// Presents an `anim-actor` state machine: the current state's tree
/// evaluated each frame, its clips phase-locked to the heaviest one (whose
/// length times the state for `auto`), and the previous state's clips
/// faded out over the transition's fade time. Shared by the player and
/// the NPCs; `map` turns tree leaves into clips (an NPC's anim set).
pub struct Animator {
    pub machine: Machine,
    current: Vec<Layer>,
    previous: Vec<Layer>,
    fade_time: f32,
    duration: f32,
    /// The heaviest non-additive clip and its time.
    main: Option<(u32, f32)>,
}

impl Animator {
    pub fn new(machine: Machine) -> Self {
        Self { machine, current: Vec::new(), previous: Vec::new(), fade_time: 0., duration: 0., main: None }
    }

    /// The root of the heaviest clip at its current time, and one
    /// `seconds` step earlier (not before the clip's start).
    pub fn root(&self, clips: &HashMap<u32, ClipInfo>, seconds: f32) -> Option<Root> {
        let (clip, time) = self.main?;
        let info = clips.get(&clip)?;
        Some(Root { clip: info.name.clone(), align: info.align_at(time), before: info.align_at((time - seconds).max(0.)), ap: info.ap })
    }

    /// Advance by `seconds` (taking `auto` transitions) and present.
    /// `before` is the state at the start of the frame, before any request.
    /// Returns the layers and the blended root speed.
    #[allow(clippy::too_many_arguments)]
    pub fn present(&mut self, graph: &mut Graph, table: &StateTable, clips: &HashMap<u32, ClipInfo>, map: &dyn Fn(u32) -> u32, mut inputs: Inputs, before: u32, seconds: f32) -> Result<(Vec<Layer>, f32)> {
        let duration = self.duration;
        self.machine.advance(table, seconds, duration);
        if self.machine.state != before {
            self.previous = std::mem::take(&mut self.current).into_iter().filter(|l| l.weight > 1e-3).collect();
            self.fade_time = 0.;
        }
        inputs.phase = if duration > 0. { (self.machine.time / duration).min(1.) } else { 0. };
        let tree = table.get(self.machine.state).map_or(0, |s| s.tree);
        let leaves = graph.evaluate(tree, &inputs, &|c| clips.get(&map(c)).is_some_and(|i| i.additive))?;
        // Mirrored states play the mirrored clips where they were built.
        let mirrored = table.get(self.machine.state).is_some_and(|s| graph.mirrored(s.mirror));
        let mirror = |c: u32| clips.get(&c).map(|i| sid(&format!("{}{}", i.name, crate::anim::MIRROR_SUFFIX))).filter(|m| clips.contains_key(m)).unwrap_or(c);
        let map = |c: u32| if mirrored { mirror(map(c)) } else { map(c) };
        let present: Vec<_> = leaves.into_iter().filter_map(|l| clips.get(&map(l.clip)).map(|i| (l, i))).collect();
        let base: f32 = present.iter().filter(|(l, _)| !l.additive).map(|(l, _)| l.weight).sum();
        let heaviest = present.iter().filter(|(l, _)| !l.additive).max_by(|a, b| a.0.weight.total_cmp(&b.0.weight));
        let main = heaviest.map(|(_, i)| i.duration).unwrap_or(0.);
        self.duration = main;
        let phase = if main > 0. { self.machine.time / main } else { 0. };
        self.main = heaviest.map(|(l, i)| (map(l.clip), l.phase.unwrap_or(phase) * i.duration));
        self.current = present
            .iter()
            .map(|(l, i)| Layer { clip: i.name.clone(), time: l.phase.unwrap_or(phase) * i.duration, weight: if l.additive || base <= 1e-4 { l.weight } else { l.weight / base }, additive: l.additive })
            .collect();
        self.fade_time += seconds;
        let fade = self.machine.fade.max(0.05);
        let t = (self.fade_time / fade).min(1.);
        for l in &mut self.previous {
            l.time += seconds;
        }
        if t >= 1. {
            self.previous.clear();
        }
        let mut layers: Vec<Layer> = self.previous.iter().map(|l| Layer { weight: l.weight * (1. - t), ..l.clone() }).collect();
        layers.extend(self.current.iter().map(|l| Layer { weight: if self.previous.is_empty() { l.weight } else { l.weight * t }, ..l.clone() }));
        layers.retain(|l| l.weight > 1e-4);
        let by_name = |n: &str| clips.values().find(|i| i.name == n);
        let speed = layers.iter().filter(|l| !l.additive).map(|l| l.weight * by_name(&l.clip).map_or(0., |i| i.speed)).sum::<f32>();
        Ok((layers, speed))
    }
}

pub struct Controller {
    pub anim: Animator,
    pub yaw: f32,
    rest_time: f32,
    yaw_rate: f32,
    ambient: usize,
    /// The hand that led the last climbing step.
    hand: bool,
}

fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(2. * PI) - PI
}

impl Controller {
    pub fn new(table: &StateTable, yaw: f32) -> Self {
        Self { anim: Animator::new(Machine::new(table)), yaw, rest_time: 0., yaw_rate: 0., ambient: 0, hand: false }
    }

    fn name(&self, table: &StateTable) -> String {
        table.get(self.anim.machine.state).map(|s| s.name.clone()).unwrap_or_default()
    }

    /// Take transition `request`, else enter `fallback` directly.
    fn go(&mut self, table: &StateTable, request: &str, fallback: Option<&str>, fade: f32) {
        if !self.anim.machine.request(table, sid(request)) {
            if let Some(target) = fallback.filter(|t| table.get(sid(t)).is_some()) {
                self.anim.machine.force(sid(target), fade);
            }
        }
    }

    /// The placement mode of the current state.
    pub fn mode(&self, table: &StateTable) -> Mode {
        let state = self.name(table);
        if HANG.contains(&state.as_str()) {
            Mode::Hang
        } else if CLIMB.contains(&state.as_str()) {
            Mode::Climb
        } else if LEAP.contains(&state.as_str()) {
            Mode::Leap
        } else if ROPE.contains(&state.as_str()) {
            Mode::Rope
        } else if WALL_ROPE.contains(&state.as_str()) {
            Mode::WallRope
        } else if state.starts_with("swim-") {
            Mode::Swim
        } else if state.starts_with("edge-cover-") || state.starts_with("corner-cover-") {
            Mode::Cover
        } else {
            Mode::Ground
        }
    }

    /// Enter the hang on an edge (a hanging start, a grab).
    pub fn hang(&mut self, fade: f32) {
        self.anim.machine.force(sid("ledge-grab"), fade);
    }

    pub fn step(&mut self, graph: &mut Graph, table: &StateTable, clips: &HashMap<u32, ClipInfo>, f: &Frame) -> Result<Step> {
        let before = self.anim.machine.state;
        let state = self.name(table);
        let finished = self.anim.duration > 0. && self.anim.machine.time >= self.anim.duration;
        match self.mode(table) {
            Mode::WallRope => {
                let busy = matches!(state.as_str(), "wall-rope-enter-bottom-grab" | "jump^wall-rope-idle" | "wall-rope-climb-up" | "wall-rope-climb-down" | "wall-rope-idle^swing" | "wall-rope-swing^idle") && !finished;
                let swinging = f.wall_rope_swing.abs() > WALL_ROPE_SWING;
                if f.jump {
                    // Away from the wall, or sideways with a swing.
                    self.yaw = if swinging { wrap(self.yaw - f.wall_rope_swing.signum() * PI / 2.) } else { wrap(self.yaw + PI) };
                    self.anim.machine.force(sid("wall-rope-jump"), 0.1);
                    return self.finish(graph, table, clips, f, before, Some(JUMP_LIFT), 0.);
                }
                if f.drop {
                    self.anim.machine.force(sid("fall"), 0.2);
                } else if !busy {
                    let on_swing = state.starts_with("wall-rope-swing") || state == "wall-rope-idle^swing";
                    let target = match f.wall_rope_climb {
                        _ if swinging && on_swing => "wall-rope-swing",
                        _ if swinging => "wall-rope-idle^swing",
                        _ if on_swing && state != "wall-rope-swing^idle" => "wall-rope-swing^idle",
                        c if c > 0 && f.wall_rope_top => "wall-rope-climb-onto-1-75m^idle",
                        c if c > 0 => "wall-rope-climb-up",
                        c if c < 0 && f.wall_rope_bottom => "wall-rope-exit-bottom^idle",
                        c if c < 0 => "wall-rope-climb-down",
                        _ => "wall-rope-idle",
                    };
                    if state != target || finished {
                        self.anim.machine.force(sid(target), 0.15);
                    }
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Rope => {
                if f.jump || f.drop {
                    self.anim.machine.force(sid("free-rope^jump-long"), 0.1);
                    let lift = (f.jump).then_some(JUMP_LIFT);
                    return self.finish(graph, table, clips, f, before, lift, 0.);
                }
                let target = if state == "free-rope-grab" && !finished {
                    "free-rope-grab"
                } else if f.rope_swing > 0.8 {
                    "free-rope-swing-mid^front"
                } else if f.rope_swing < -0.8 {
                    "free-rope-swing-mid^back"
                } else {
                    "free-rope-idle"
                };
                if state != target && (state == "free-rope-grab" || !state.starts_with("free-rope-swing") || finished || target == "free-rope-idle") {
                    self.go(table, target, Some(target), 0.25);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Hang if f.bar && state == "ledge-grab" => {
                self.anim.machine.force(sid("ledge-overhead-swing-enter"), 0.15);
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Hang if state.starts_with("ledge-overhead-swing") => {
                if f.drop {
                    self.anim.machine.force(sid("fall"), 0.2);
                } else if f.jump {
                    self.anim.machine.force(sid("ledge-overhead-swing-jump"), 0.1);
                    return self.finish(graph, table, clips, f, before, Some(JUMP_LIFT * 0.6), 0.);
                } else if f.leap && state == "ledge-overhead-swing" {
                    // Hand over hand to the next bar ahead.
                    self.anim.machine.force(sid("ledge-overhead-forward"), 0.1);
                } else if state == "ledge-overhead-swing-enter" && finished {
                    self.anim.machine.force(sid("ledge-overhead-swing"), 0.15);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Hang => {
                if f.drop {
                    self.anim.machine.force(sid("fall"), 0.2);
                } else if state == "fall-grab-edge" {
                    if finished {
                        self.anim.machine.force(sid("ledge-grab"), 0.15);
                    }
                } else if f.step != 0 {
                    self.hand = !self.hand;
                    let target = format!("climb-{}-{}", if f.step > 0 { "up" } else { "down" }, if self.hand { "lh" } else { "rh" });
                    self.anim.machine.force(sid(&target), 0.1);
                } else if f.leap {
                    self.anim.machine.force(sid("ledge-jump-begin"), 0.1);
                } else if f.climb {
                    self.anim.machine.force(sid("ledge-climb-up"), 0.15);
                } else if f.lateral > 0.5 {
                    self.go(table, "ledge-shimmy-right", None, 0.15);
                } else if f.lateral < -0.5 {
                    self.go(table, "ledge-shimmy-left", None, 0.15);
                } else if state != "ledge-grab" {
                    self.go(table, "ledge-grab", Some("ledge-grab"), 0.15);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Leap => {
                if (state == "ledge-jump-end" || state.starts_with("climb-up-") || state.starts_with("climb-down-")) && finished {
                    self.anim.machine.force(sid("ledge-grab"), 0.15);
                } else if state == "ledge-overhead-forward" && finished {
                    self.anim.machine.force(sid("ledge-overhead-swing"), 0.15);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Climb => {
                // The last climb step takes the `idle`/`run` request.
                if state == "ledge-climb-up-2" && finished {
                    let target = if f.heading.is_some() { "run" } else { "idle" };
                    self.go(table, target, Some(&format!("climb-onto-1-75m^{target}")), 0.1);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Swim => {
                if !f.swim {
                    self.go(table, "idle", Some("idle"), 0.3);
                } else {
                    let target = match f.heading {
                        None => "swim-idle",
                        Some(_) if f.movement_fast => "swim-surface-crawl",
                        Some(_) => "swim-surface-breast",
                    };
                    if state != target {
                        self.anim.machine.force(sid(target), 0.3);
                    }
                }
                if let Some(h) = f.heading {
                    let d = wrap(h - self.yaw);
                    self.yaw = wrap(self.yaw + d.clamp(-TURN_RATE * 0.5 * f.seconds, TURN_RATE * 0.5 * f.seconds));
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Cover => {
                let side = if state.contains("left") { "l" } else { "r" };
                if state.ends_with("-exit-l") || state.ends_with("-exit-r") {
                    if finished {
                        self.go(table, "idle", Some("idle"), 0.2);
                    }
                } else if f.uncover {
                    let exit = if f.heading.is_some() { format!("edge-cover-run-exit-{side}") } else { format!("edge-cover-idle-exit-{side}") };
                    if state.starts_with("edge-cover-") {
                        self.anim.machine.force(sid(&exit), 0.15);
                    } else {
                        self.go(table, "idle", Some("idle"), 0.2);
                    }
                } else if state.contains("-enter-") && finished {
                    let base = state.split("-enter-").next().unwrap_or(&state).to_owned();
                    self.anim.machine.force(sid(&base), 0.15);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            Mode::Ground => {}
        }
        if f.swim {
            self.anim.machine.force(sid("swim-idle"), 0.3);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        if let Some(kind) = f.cover {
            let (family, side) = (if kind >= 2 { "edge-cover" } else { "corner-cover" }, if kind % 2 == 0 { "left" } else { "right" });
            let enter = format!("{family}-{side}-enter-close");
            let target = if table.get(sid(&enter)).is_some() { enter } else { format!("{family}-{side}") };
            self.anim.machine.force(sid(&target), 0.15);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        // Death holds its state; one-shot actions play out; aiming walks
        // facing the camera.
        if DEATHS.contains(&state.as_str()) {
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        if let Some(d) = f.death {
            self.anim.machine.force(sid(d), 0.1);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        if ACTIONS.contains(&state.as_str()) {
            if finished {
                self.go(table, "idle", Some("idle"), 0.2);
            }
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        let grounded_state = !matches!(state.as_str(), "run-jump-long" | "jump-long" | "fall" | "jump-in-place") && !FLIGHT.contains(&state.as_str());
        if grounded_state && f.grounded {
            if f.throw {
                self.anim.machine.force(sid("throw-grenade"), 0.1);
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            if f.melee {
                self.anim.machine.force(sid("air-punch"), 0.1);
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            if let Some(side) = f.hurt {
                let target = match side {
                    s if s < 0 => "hero-generic-hit-reaction-left",
                    s if s > 0 => "hero-generic-hit-reaction-right",
                    _ => "hero-generic-hit-reaction",
                };
                self.anim.machine.force(sid(target), 0.1);
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            if let Some(yaw) = f.aim {
                self.yaw = yaw;
                let target = match f.heading.map(|h| wrap(h - yaw)) {
                    Some(a) if a.abs() > FRAC_PI_4 * 3. => "aim-run-backward",
                    Some(_) => "aim-run-forward",
                    None => "aim-idle",
                };
                if state != target {
                    self.go(table, target, Some(target), 0.15);
                }
                return self.finish(graph, table, clips, f, before, None, 0.);
            }
            if AIM.contains(&state.as_str()) {
                self.go(table, "idle", Some("idle"), 0.2);
            }
        }
        let airborne = matches!(state.as_str(), "run-jump-long" | "jump-long" | "fall" | "jump-in-place") || FLIGHT.contains(&state.as_str());
        if let Some(from_air) = f.wall_rope_grab.filter(|air| *air == airborne) {
            self.anim.machine.force(sid(if from_air { "jump^wall-rope-idle" } else { "wall-rope-enter-bottom-grab" }), 0.1);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        if airborne && f.rope_grab {
            self.anim.machine.force(sid("free-rope-grab"), 0.1);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        // A wall jump: jump again against a wall during a long jump; the
        // player turns away from the wall.
        if matches!(state.as_str(), "run-jump-long" | "jump-long") && f.wall_ahead && f.jump {
            self.yaw = wrap(self.yaw + PI);
            self.anim.machine.force(sid("wall-jump-90-long-jump"), 0.05);
            return self.finish(graph, table, clips, f, before, Some(JUMP_LIFT), 0.);
        }
        if airborne && f.grab {
            self.anim.machine.force(sid("fall-grab-edge"), 0.1);
            return self.finish(graph, table, clips, f, before, None, 0.);
        }
        let at_rest = state == "idle" || state == "run-idle" || state.starts_with("ambient-") || state.starts_with("idle-turn-") || state.ends_with("-land-idle") || state == "jump-in-place-land" || state.starts_with("fall-land-") && state.ends_with("-idle");
        let moving_state = state == "run" || state.starts_with("idle-run-") || state == "run-180" || state.ends_with("-land-run");
        let mut lift = None;
        let relative = f.heading.map(|h| wrap(h - self.yaw));
        if airborne {
            if f.grounded && state != "jump-in-place" && state != "run-jump-long" {
                if state == "fall" || FLIGHT.contains(&state.as_str()) {
                    let grade = if f.hard_land {
                        "hard"
                    } else if f.dropped < 1.5 {
                        "soft"
                    } else if f.dropped < 4. {
                        "medium"
                    } else {
                        "hard"
                    };
                    let target = format!("fall-land-{grade}-{}", if f.heading.is_some() { "run" } else { "idle" });
                    self.anim.machine.force(sid(&target), 0.1);
                } else if f.heading.is_some() {
                    self.anim.machine.force(sid("jump-long-land-run"), 0.1);
                } else {
                    self.go(table, "idle", Some("jump-long-land-idle"), 0.1);
                }
            }
        } else if !f.grounded && f.dropped > FALL_AFTER {
            self.anim.machine.force(sid("fall"), 0.2);
        } else if f.jump {
            if f.heading.is_some() {
                self.anim.machine.force(sid("run-jump-long"), 0.1);
                lift = Some(JUMP_LIFT);
            } else {
                self.anim.machine.force(sid("jump-in-place"), 0.1);
            }
        } else if let Some(a) = relative {
            if at_rest {
                let dir = if a.abs() < FRAC_PI_4 {
                    "idle-run-f"
                } else if a.abs() < 3. * FRAC_PI_4 {
                    if a > 0. { "idle-run-r" } else { "idle-run-l" }
                } else {
                    "idle-run-b"
                };
                self.go(table, dir, Some(dir), 0.2);
            } else if state == "run" && a.abs() > 3. * FRAC_PI_4 {
                self.go(table, "run-180", Some("run-180"), 0.2);
            }
        } else if moving_state {
            if state == "run" || state.starts_with("idle-run-") {
                self.go(table, "run-idle", Some("run-idle"), 0.2);
            } else {
                self.go(table, "idle", Some("idle"), 0.2);
            }
        } else if state == "idle" {
            self.rest_time += f.seconds;
            if self.rest_time > AMBIENT_AFTER {
                self.rest_time = 0.;
                let target = AMBIENT[self.ambient % AMBIENT.len()];
                self.ambient += 1;
                self.go(table, target, Some(target), 0.5);
            }
        }
        if state != "idle" || f.heading.is_some() {
            self.rest_time = 0.;
        }
        // Facing: turn toward the stick, quickly while running and over the
        // start and reversal clips.
        let old_yaw = self.yaw;
        if let Some(h) = f.heading {
            let name = self.name(table);
            if !matches!(name.as_str(), "jump-long" | "run-jump-long" | "fall") {
                let rate = if name == "run" { TURN_RATE } else { TURN_RATE * 0.75 };
                let d = wrap(h - self.yaw);
                self.yaw = wrap(self.yaw + d.clamp(-rate * f.seconds, rate * f.seconds));
            }
        }
        let rate = if f.seconds > 0. { wrap(self.yaw - old_yaw) / f.seconds } else { 0. };
        self.yaw_rate += (rate - self.yaw_rate) * (f.seconds * 8.).min(1.);
        let lean = (self.yaw_rate / LEAN_RATE).clamp(-1., 1.);
        self.finish(graph, table, clips, f, before, lift, lean)
    }

    /// Present the state and report the step.
    #[allow(clippy::too_many_arguments)]
    fn finish(&mut self, graph: &mut Graph, table: &StateTable, clips: &HashMap<u32, ClipInfo>, f: &Frame, before: u32, lift: Option<f32>, lean: f32) -> Result<Step> {
        let inputs = Inputs {
            info: vec![(28, lean.max(0.)), (32, (-lean).max(0.)), (40, (f.slope * 2.).clamp(0., 1.)), (44, (-f.slope * 2.).clamp(0., 1.))],
            instance: vec![(48, f.movement), (52, f.weapon)],
            phase: 0.,
        };
        let (layers, speed) = self.anim.present(graph, table, clips, &|c| c, inputs, before, f.seconds)?;
        let changed = self.anim.machine.state != before;
        let root = self.anim.root(clips, f.seconds);
        Ok(Step { layers, speed, lift, state: changed.then(|| self.name(table)), mode: self.mode(table), root })
    }
}
