// SPDX-License-Identifier: MPL-2.0
//! NPC agents (PPU `game/ai/agent/npc.cpp`): the fields script natives
//! write on the `Npc` object, its configuration (`m_pConfiguration`,
//! Npc +0x2660) and knowledge (`m_pKnowledge`, +0x2664), and the orders
//! of the scripted skill (skill 2, `sub_2A10B0(npc+0x1864, 2)`).
//!
//! The natives look the NPC up in the NPC manager (`sub_1E2968`), which
//! holds only live NPCs. Engine motion (path following, turning) is
//! stepped by the host each frame; the native skill drives animation
//! controllers with root motion, so the speeds here are approximations
//! (SATD-nd-retail-runtime).
use glam::Vec3;
use std::collections::HashMap;

/// Movement types of `npc-move-to(-spawner)`: argument 0 walk, 1 run,
/// 2 sprint map to the native motion codes 1, 3 and 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Walk = 1,
    Run = 3,
    Sprint = 5,
}

impl Motion {
    /// The script argument's motion (`sub_226530`: 0 walk, 2 sprint,
    /// anything else run).
    pub fn from_arg(value: u32) -> Self {
        match value {
            0 => Self::Walk,
            2 => Self::Sprint,
            _ => Self::Run,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Run => "run",
            Self::Sprint => "sprint",
        }
    }
    /// Ground speed, metres per second. Not native: the scripted skill
    /// moves by the locomotion clips' root motion.
    pub fn speed(self) -> f32 {
        match self {
            Self::Walk => 1.5,
            Self::Run => 3.8,
            Self::Sprint => 5.5,
        }
    }
}

/// What the NPC does on arrival (`npc-move-to-spawner`'s fourth
/// argument: 1 stop and face the spawner's direction, 2 continue, else
/// stop).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    Stop,
    StopAndFace,
    Continue,
}

impl Goal {
    pub fn from_arg(value: u32) -> Self {
        match value {
            1 => Self::StopAndFace,
            2 => Self::Continue,
            _ => Self::Stop,
        }
    }
}

/// Knowledge states written by natives (`sub_1D6580`, Knowledge +0x1450).
pub const KNOWLEDGE_UNAWARE: u32 = 0;
pub const KNOWLEDGE_PREOCCUPIED: u32 = 1;
pub const KNOWLEDGE_COMBAT: u32 = 4;

/// Contact status codes of `npc-override-contact-status` (argument 0, 1,
/// 2 → `sub_1D6748` status 1, 2, 3).
pub fn contact_status(arg: u32) -> Option<u32> {
    (arg <= 2).then_some(arg + 1)
}

/// `m_pConfiguration` fields scripts set.
#[derive(Clone, Debug, PartialEq)]
pub struct Configuration {
    /// Word +0 bit 58: flinching on hits (`npc-disable-flinch` clears it).
    pub flinch: bool,
    /// +0x20: bit mask of granted skills (`npc-add-skill`/`-remove-skill`).
    pub skills: u64,
    /// +0x30 bit 29: shooting disabled (`npc-enable-shooting`).
    pub shooting_disabled: bool,
    /// +0x38: damage dealt by this NPC's shots.
    pub shot_damage_multiplier: f32,
    /// +0xCC: zone update method (3 follows a zone marker spawner).
    pub zone_update_method: u32,
    /// +0xD0: the zone marker's frame when following one.
    pub zone_marker: Option<[f32; 3]>,
}

impl Default for Configuration {
    fn default() -> Self {
        Self { flinch: true, skills: 0, shooting_disabled: false, shot_damage_multiplier: 1., zone_update_method: 0, zone_marker: None }
    }
}

/// One of the two zones at Npc +0x2680 (`sub_1B0EC0`/`sub_1B0E90`/
/// `sub_1B2DA0`, 32 bytes each): where the NPC may fight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Zone {
    pub center: Option<[f32; 3]>,
    pub radius: f32,
    pub y_threshold: f32,
}

/// An order of the scripted skill.
#[derive(Clone, Debug, PartialEq)]
pub enum Order {
    /// `sub_29A170`/`sub_29A2C8`: follow `path` (nav corners ending at
    /// the target) at `motion`'s speed, then `goal` (with the facing).
    Move { path: Vec<[f32; 3]>, motion: Motion, goal: Goal, face: Option<f32>, radius: f32 },
    /// `sub_299E34`: turn in place to face a yaw (radians).
    Turn { yaw: f32 },
    /// `sub_299994`: occupy scripted action pack `spawner`.
    UseAp { spawner: u32 },
    /// `sub_299C14`: move into the nearest cover.
    Cover { path: Vec<[f32; 3]> },
}

/// Turn rate for scripted turns, radians per second (not native).
const TURN_RATE: f32 = std::f32::consts::PI * 1.5;
/// Yaw within which a turn is complete.
const TURN_DONE: f32 = 0.05;

#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub position: [f32; 3],
    /// Heading about +Y, radians; zero faces +Z.
    pub yaw: f32,
    /// Npc +0x26E4 (`npc-set-accuracy-multiplier`, default 1).
    pub accuracy_multiplier: f32,
    pub config: Configuration,
    /// Knowledge +0x1450.
    pub knowledge: u32,
    /// Contact statuses by process name (`npc-override-contact-status`).
    pub contacts: HashMap<u32, u32>,
    /// Knowledge +0x1190 preferred target (`npc-prefer-target`).
    pub prefer_target: Option<u32>,
    /// Requested demeanour 0–2 (`sub_1A3EE4`).
    pub demeanor: u8,
    /// `npc-set-combat-behavior` (a behaviour `StringId`).
    pub combat_behavior: u32,
    /// Health component's invulnerability toggle (+0x15C vtable +0x4C).
    pub can_die: bool,
    pub can_be_hit: bool,
    /// Targeting flag bit 14 (`npc-set-ignore`).
    pub ignored: bool,
    /// Npc +0x1858 bit 42 (`npc-mark-as-defunct`).
    pub defunct: bool,
    /// Faction 0–11 (`set-object-faction`, `sub_F36C4`).
    pub faction: Option<u32>,
    pub zones: [Zone; 2],
    pub weapon_visible: bool,
    /// The facial clip playing (`npc-play-facial-anim`).
    pub facial: Option<u32>,
    /// Script process the scripted skill is bound to (`wait-npc-bind`).
    pub bound: Option<u32>,
    pub order: Option<Order>,
    /// Scripted action pack occupied after `UseAp` arrives.
    pub ap: Option<u32>,
    /// Current ground speed (presentation).
    pub speed: f32,
}

impl Agent {
    pub fn new(position: [f32; 3], rotation: [f32; 4]) -> Self {
        let forward = glam::Quat::from_array(rotation).normalize() * Vec3::Z;
        Self {
            position,
            yaw: forward.x.atan2(forward.z),
            accuracy_multiplier: 1.,
            config: Configuration::default(),
            knowledge: KNOWLEDGE_UNAWARE,
            contacts: HashMap::new(),
            prefer_target: None,
            demeanor: 0,
            combat_behavior: 0,
            can_die: true,
            can_be_hit: true,
            ignored: false,
            defunct: false,
            faction: None,
            zones: [Zone::default(); 2],
            weapon_visible: true,
            facial: None,
            bound: None,
            order: None,
            ap: None,
            speed: 0.,
        }
    }

    pub fn rotation(&self) -> [f32; 4] {
        glam::Quat::from_rotation_y(self.yaw).to_array()
    }

    /// `npc-in-combat?` (`sub_1BB62C`: knowledge state 4).
    pub fn in_combat(&self) -> bool {
        self.knowledge == KNOWLEDGE_COMBAT
    }

    /// `npc-is-player-in-front?` (`sub_228A40`): the player is within 2 m
    /// and the unit offset's dot with the NPC's forward is at least 0.5.
    pub fn in_front(&self, point: [f32; 3]) -> bool {
        let d = Vec3::from_array(point) - Vec3::from_array(self.position);
        let distance = d.length();
        if !(distance < 2.) {
            return false;
        }
        let forward = glam::Quat::from_rotation_y(self.yaw) * Vec3::Z;
        let unit = if distance > f32::EPSILON { d / distance } else { Vec3::ZERO };
        unit.dot(forward) >= 0.5
    }

    /// Advance the current order by `dt` seconds; true when it completed
    /// this step.
    pub fn step(&mut self, dt: f32) -> bool {
        let dt = dt.clamp(0., 0.25);
        self.speed = 0.;
        let Some(order) = self.order.as_mut() else { return false };
        match order {
            Order::Move { path, motion, goal, face, radius } => {
                let speed = motion.speed();
                let done = follow(&mut self.position, &mut self.yaw, path, speed * dt, *radius);
                self.speed = if done && *goal != Goal::Continue { 0. } else { speed };
                if done {
                    if let (Goal::StopAndFace, Some(f)) = (*goal, *face) {
                        self.order = Some(Order::Turn { yaw: f });
                        return false;
                    }
                    self.order = None;
                    return true;
                }
                false
            }
            Order::Cover { path } => {
                let done = follow(&mut self.position, &mut self.yaw, path, Motion::Run.speed() * dt, 0.);
                self.speed = if done { 0. } else { Motion::Run.speed() };
                if done {
                    self.order = None;
                }
                done
            }
            Order::Turn { yaw } => {
                let delta = wrap(*yaw - self.yaw);
                let step = TURN_RATE * dt;
                if delta.abs() <= step.max(TURN_DONE) {
                    self.yaw = *yaw;
                    self.order = None;
                    return true;
                }
                self.yaw = wrap(self.yaw + step.copysign(delta));
                false
            }
            // The AP's entry animation is not played: the NPC occupies it.
            Order::UseAp { spawner } => {
                self.ap = Some(*spawner);
                self.order = None;
                true
            }
        }
    }
}

fn wrap(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI
}

/// Move `position` up to `distance` along `path` (consumed corner by
/// corner), facing the motion; true once within `radius` of the end.
fn follow(position: &mut [f32; 3], yaw: &mut f32, path: &mut Vec<[f32; 3]>, mut distance: f32, radius: f32) -> bool {
    let mut p = Vec3::from_array(*position);
    while let Some(&next) = path.first() {
        let target = Vec3::from_array(next);
        let d = target - p;
        let length = d.length();
        let last = path.len() == 1;
        if last && length <= radius.max(0.05) {
            path.clear();
            break;
        }
        if length <= distance {
            p = target;
            distance -= length;
            path.remove(0);
            continue;
        }
        let flat = Vec3::new(d.x, 0., d.z);
        if flat.length_squared() > 1e-6 {
            *yaw = flat.x.atan2(flat.z);
        }
        p += d / length * distance;
        break;
    }
    *position = p.to_array();
    path.is_empty()
}

/// The live NPCs by spawner name.
#[derive(Clone, Debug, Default)]
pub struct Agents {
    pub agents: HashMap<u32, Agent>,
}

impl Agents {
    pub fn get(&self, name: u32) -> Option<&Agent> {
        self.agents.get(&name)
    }
    pub fn get_mut(&mut self, name: u32) -> Option<&mut Agent> {
        self.agents.get_mut(&name)
    }
    /// Step every order; returns the agents whose order completed.
    pub fn step(&mut self, dt: f32) -> Vec<u32> {
        let mut done = Vec::new();
        for (&name, agent) in &mut self.agents {
            if agent.step(dt) {
                done.push(name);
            }
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_and_goal_codes_follow_the_native_argument_decoding() {
        assert_eq!(Motion::from_arg(0), Motion::Walk);
        assert_eq!(Motion::from_arg(1), Motion::Run);
        assert_eq!(Motion::from_arg(2), Motion::Sprint);
        assert_eq!(Motion::from_arg(7), Motion::Run);
        assert_eq!(Goal::from_arg(1), Goal::StopAndFace);
        assert_eq!(Goal::from_arg(2), Goal::Continue);
        assert_eq!(Goal::from_arg(0), Goal::Stop);
        assert_eq!(contact_status(2), Some(3));
        assert_eq!(contact_status(3), None);
    }

    #[test]
    fn move_order_follows_corners_then_faces() {
        let mut a = Agent::new([0.; 3], [0., 0., 0., 1.]);
        a.order = Some(Order::Move { path: vec![[0., 0., 2.], [2., 0., 2.]], motion: Motion::Walk, goal: Goal::StopAndFace, face: Some(1.0), radius: 0. });
        let mut steps = 0;
        while a.order.is_some() && steps < 1000 {
            a.step(0.1);
            steps += 1;
        }
        assert!(a.order.is_none());
        assert!((Vec3::from_array(a.position) - Vec3::new(2., 0., 2.)).length() < 1e-4);
        assert!((a.yaw - 1.0).abs() < 1e-6);
        // 4 m at 1.5 m/s plus the turn.
        assert!((27..40).contains(&steps), "{steps}");
    }

    #[test]
    fn player_in_front_needs_distance_and_cone() {
        let a = Agent::new([0.; 3], [0., 0., 0., 1.]);
        assert!(a.in_front([0., 0., 1.5]));
        assert!(!a.in_front([0., 0., 2.5]));
        assert!(!a.in_front([1.5, 0., 0.5]));
        assert!(!a.in_front([0., 0., -1.]));
    }
}
