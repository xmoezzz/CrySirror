// SPDX-License-Identifier: MPL-2.0
//! NPC script natives (`DcNpc*`, PPU 0x221000–0x22E200). Each looks its
//! NPC up through the NPC manager (`sub_1E2968`); without a name argument
//! the calling process is the NPC. Failures print the native's message
//! and return 0, as the original does.
use super::{Ctx, Wait};
use crate::dc::{Dc, sid};
use crate::npc::{self, Agent, Goal, Motion, Order};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

/// Spawner tags `sub_22CE80` reads for a zone marker (and
/// `npc-move-to-spawner` for the arrival radius); names not recovered.
const TAG_ZONE_RADIUS: u32 = 0xa28a_0aec;
const TAG_ZONE_Y_THRESHOLD: u32 = 0xac43_d55d;
/// The object type `npc-use-scripted-ap` requires of its spawner.
const TYPE_ACTION_PACK: u32 = 0x7dbc_3381;
/// Scripted skill index (`sub_2A10B0(npc+0x1864, 2)`).
const SKILL_SCRIPT: u32 = 2;

type Reply = Option<(u32, Status)>;

impl Ctx<'_> {
    fn self_name(&self) -> u32 {
        self.proc.map_or(0, |p| self.inner.procs[p].name)
    }
    /// Argument `i`, or the caller when absent (the natives' fallback to
    /// the current process).
    fn arg_or_self(&self, a: &[u32; MAX_ARGS], n: usize, i: usize) -> u32 {
        if n > i { a[i] } else { self.self_name() }
    }
    fn agent(&mut self, name: u32) -> Option<&mut Agent> {
        self.world.agents()?.get_mut(name)
    }
    fn has_agent(&self, name: u32) -> bool {
        self.world.agents_ref().is_some_and(|a| a.get(name).is_some())
    }
    /// Whether the NPC has the scripted skill (`sub_2A10B0`). The skill
    /// list (Npc +0x1864) comes from the character; every soldier
    /// character carries it, so a live NPC has it.
    fn has_script_skill(&self, name: u32) -> bool {
        let _ = SKILL_SCRIPT;
        self.has_agent(name)
    }
    fn vec3(dc: &Dc, at: u32) -> Result<[f32; 3]> {
        Ok([dc.mem.f32(at)?, dc.mem.f32(at + 4)?, dc.mem.f32(at + 8)?])
    }
    fn missing_npc(&mut self, native: &str, name: u32) -> Reply {
        self.inner.error(format!("{native}: Unable to find npc {name:#010x}"));
        Some((0, Status::Continue))
    }
    /// Start a move order (`sub_29A170`/`sub_29A2C8`); suspends the
    /// caller for the `wait-` forms. 1 when the target is on the nav mesh.
    fn order_move(&mut self, native: &str, who: u32, target: [f32; 3], motion: Motion, goal: Goal, face: Option<f32>, radius: f32, wait: bool) -> Reply {
        let Some(from) = self.world.agents_ref().and_then(|a| a.get(who)).map(|a| a.position) else { return self.missing_npc(native, who) };
        if !self.has_script_skill(who) {
            self.inner.error("wait-npc-move-to: Npc doesn't have the scripted skill!!".into());
            return Some((0, Status::Continue));
        }
        let Some(path) = self.world.nav_path(from, target) else {
            self.inner.error(format!("{native}: Npc {who:#010x} told to move off the nav mesh/map! ({target:?})"));
            return Some((0, Status::Continue));
        };
        if let Some(agent) = self.agent(who) {
            agent.ap = None;
            agent.order = Some(Order::Move { path, motion, goal, face, radius });
        }
        if wait {
            if self.proc.is_none() {
                self.inner.error(format!("{native}: Called outside script context -- will not wait"));
            } else {
                return Some(self.suspend(Wait::Npc(who)));
            }
        }
        Some((1, Status::Continue))
    }

    /// The NPC natives; `None` for other names.
    pub(super) fn npc_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let f = |i: usize| f32::from_bits(a[i]);
        // Boolean script arguments (`((x>>31) - ((x>>31)^x)) >> 31`):
        // any non-zero value.
        let flag = |i: usize| n > i && a[i] != 0;
        match name {
            "npc-set-accuracy-multiplier" => {
                let who = self.arg_or_self(a, n, 0);
                let value = if n > 1 { f(1) } else { 1. };
                match self.agent(who) {
                    Some(agent) => {
                        agent.accuracy_multiplier = value;
                        ok(1)
                    }
                    None => Ok(self.missing_npc(name, who)),
                }
            }
            // (npc-set-zone-marker npc marker): zone 0 follows the marker
            // spawner (`sub_22CE80`): its frame, update method 3, and the
            // marker's radius and y-threshold tags when not negative.
            "npc-set-zone-marker" => {
                let who = self.arg_or_self(a, n, 0);
                let marker = self.arg_or_self(a, n, 1);
                if !self.has_agent(who) {
                    return ok(0);
                }
                let Some((center, _)) = self.world.locator(marker) else { return ok(0) };
                let float = |w: &dyn super::World, tag| match w.property(marker, tag) {
                    Some(super::Property::Float(v)) => v,
                    Some(super::Property::Int(v)) => v as f32,
                    _ => -1.,
                };
                let (radius, y) = (float(&*self.world, TAG_ZONE_RADIUS), float(&*self.world, TAG_ZONE_Y_THRESHOLD));
                if let Some(agent) = self.agent(who) {
                    agent.zones[0].center = Some(center);
                    agent.config.zone_marker = Some(center);
                    agent.config.zone_update_method = 3;
                    if radius >= 0. {
                        agent.zones[0].radius = radius.abs();
                    }
                    if y >= 0. {
                        agent.zones[0].y_threshold = y.abs();
                    }
                }
                ok(0)
            }
            // (npc-set-zone-center npc spawner), `sub_1B2DA0(npc, 0)`.
            "npc-set-zone-center" => {
                let who = self.arg_or_self(a, n, 0);
                let at = self.arg_or_self(a, n, 1);
                if let Some((center, _)) = self.world.locator(at) {
                    if let Some(agent) = self.agent(who) {
                        agent.zones[0].center = Some(center);
                    }
                }
                ok(0)
            }
            // Radius and y-threshold of zone 0 (absolute values; the
            // default argument is 0.0 at TOC-0x7FA4).
            "npc-set-zone-radius" | "npc-set-zone-y-threshold" => {
                let who = self.arg_or_self(a, n, 0);
                let value = if n > 1 { f(1).abs() } else { 0. };
                if let Some(agent) = self.agent(who) {
                    if name == "npc-set-zone-radius" { agent.zones[0].radius = value } else { agent.zones[0].y_threshold = value }
                }
                ok(0)
            }
            "npc-set-zone-update-method" => {
                if n <= 1 {
                    self.inner.error(format!("{name} got incorrect number of arguments ({n})"));
                    return ok(0);
                }
                if let Some(agent) = self.agent(a[0]) {
                    agent.config.zone_update_method = a[1];
                }
                ok(0)
            }
            "npc-is-player-in-front?" => {
                let who = self.arg_or_self(a, n, 0);
                let player = self.world.locator(sid("player")).map(|(p, _)| p);
                match self.world.agents_ref().and_then(|x| x.get(who)) {
                    Some(agent) => ok(u32::from(player.is_some_and(|p| agent.in_front(p)))),
                    None => {
                        self.inner.error(format!("{name}: Can't find npc: {who:#010x}"));
                        ok(0)
                    }
                }
            }
            // (npc-override-contact-status npc contact status).
            "npc-override-contact-status" => {
                let who = self.arg_or_self(a, n, 0);
                if !self.has_agent(who) {
                    self.inner.error(format!("{name}: Can't find npc: {who:#010x}"));
                    return ok(0);
                }
                let contact = self.arg_or_self(a, n, 1);
                let known = contact == sid("player") || self.inner.process(contact).is_some() || self.world.object_spawned(contact);
                if !known {
                    self.inner.error(format!("{name}: Can't find contact process: {contact:#010x}"));
                    return ok(0);
                }
                if let Some(status) = npc::contact_status(if n > 2 { a[2] } else { 0 }) {
                    if let Some(agent) = self.agent(who) {
                        agent.contacts.insert(contact, status);
                    }
                }
                ok(0)
            }
            // (npc-set-demeanor npc d): 1 and 2 select those demeanours,
            // anything else 0 (`sub_227118`).
            "npc-set-demeanor" => {
                let who = self.arg_or_self(a, n, 0);
                let demeanor = match (n > 1).then(|| a[1]) {
                    Some(1) => 1,
                    Some(2) => 2,
                    _ => 0,
                };
                match self.agent(who) {
                    Some(agent) => {
                        agent.demeanor = demeanor;
                        ok(0)
                    }
                    None => Ok(self.missing_npc(name, who)),
                }
            }
            // Knowledge state 4 (combat) and 1 (preoccupied), `sub_1D6580`.
            "npc-set-in-combat" | "npc-set-preoccupied" => {
                let who = self.arg_or_self(a, n, 0);
                if who == 0 {
                    self.inner.error(format!("{name}: No NPC name specified"));
                    return ok(0);
                }
                let state = if name == "npc-set-in-combat" { npc::KNOWLEDGE_COMBAT } else { npc::KNOWLEDGE_PREOCCUPIED };
                match self.agent(who) {
                    Some(agent) => {
                        agent.knowledge = state;
                        ok(u32::from(name == "npc-set-in-combat"))
                    }
                    None => Ok(self.missing_npc(name, who)),
                }
            }
            "npc-in-combat?" => {
                let who = self.arg_or_self(a, n, 0);
                ok(u32::from(self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(Agent::in_combat)))
            }
            // Configuration +0x20 skill bits.
            "npc-add-skill" | "npc-remove-skill" => {
                let who = self.arg_or_self(a, n, 0);
                let bit = 1u64 << ((if n > 1 { a[1] } else { 0 }) & 63);
                if let Some(agent) = self.agent(who) {
                    if name == "npc-add-skill" { agent.config.skills |= bit } else { agent.config.skills &= !bit }
                }
                ok(0)
            }
            "npc-set-combat-behavior" => {
                let who = self.arg_or_self(a, n, 0);
                let behavior = self.arg_or_self(a, n, 1);
                if let Some(agent) = self.agent(who) {
                    agent.combat_behavior = behavior;
                }
                ok(0)
            }
            "npc-enable-shooting" | "npc-set-shot-damage-multiplier" => {
                if n <= 1 {
                    self.inner.error(format!("{name} got incorrect number of arguments ({n})"));
                    return ok(0);
                }
                match self.agent(a[0]) {
                    Some(agent) if name == "npc-enable-shooting" => agent.config.shooting_disabled = a[1] == 0,
                    Some(agent) => agent.config.shot_damage_multiplier = f(1),
                    None => self.inner.error(format!("{name}: NPC {:#010x} not found", a[0])),
                }
                ok(0)
            }
            "npc-disable-flinch" => {
                let who = self.arg_or_self(a, n, 0);
                if who == 0 {
                    self.inner.error(format!("{name}: No NPC name specified"));
                    return ok(0);
                }
                match self.agent(who) {
                    Some(agent) => agent.config.flinch = false,
                    None => {
                        self.missing_npc(name, who);
                    }
                }
                ok(0)
            }
            "npc-set-can-die" => {
                let who = self.arg_or_self(a, n, 0);
                if let Some(agent) = self.agent(who) {
                    agent.can_die = flag(1);
                }
                ok(0)
            }
            // The same health toggle, then the NPC is sent event
            // `ignore-all-fire-no-blood` with the inverted flag.
            "npc-set-can-be-hit" => {
                let who = self.arg_or_self(a, n, 0);
                let on = flag(1);
                if let Some(agent) = self.agent(who) {
                    agent.can_die = on;
                    agent.can_be_hit = on;
                    self.send(sid("ignore-all-fire-no-blood"), who, vec![u32::from(!on)]);
                }
                ok(0)
            }
            "npc-set-ignore" => {
                let who = self.arg_or_self(a, n, 0);
                if let Some(agent) = self.agent(who) {
                    agent.ignored = flag(1);
                }
                ok(0)
            }
            "npc-mark-as-defunct" if n > 0 => {
                if let Some(agent) = self.agent(a[0]) {
                    agent.defunct = true;
                }
                ok(0)
            }
            "npc-prefer-target" => {
                let who = self.arg_or_self(a, n, 0);
                let target = self.arg_or_self(a, n, 1);
                match self.agent(who) {
                    Some(agent) => {
                        agent.prefer_target = (target != 0).then_some(target);
                        ok(0)
                    }
                    None => {
                        self.inner.error(format!("{name}: Unable to find NPC {who:#010x}"));
                        ok(0)
                    }
                }
            }
            // (set-object-faction object faction): factions 0–11
            // (`sub_85A40` maps others to the invalid 12).
            "set-object-faction" => {
                if n <= 1 {
                    self.inner.error(format!("{name}: needs two arguments. got {n}"));
                    return ok(0);
                }
                if a[1] > 11 {
                    self.inner.error(format!("{name}: Invalid faction type {} specified", a[1] as i32));
                    return ok(0);
                }
                if let Some(agent) = self.agent(a[0]) {
                    agent.faction = Some(a[1]);
                }
                ok(0)
            }
            "npc-set-weapon-visible" => {
                if n != 2 {
                    self.inner.error(format!("{name}: Expected two args (object-name string) (visible? boolean)"));
                    return ok(0);
                }
                let on = a[1] != 0;
                match self.agent(a[0]) {
                    Some(agent) => agent.weapon_visible = on,
                    None => self.inner.error(format!("{name}: Cannot make {:#010x} {}visible - no such spawner, or not a ProcessGameObject", a[0], if on { "" } else { "in" })),
                }
                ok(0)
            }
            // (npc-play-facial-anim npc clip): 1 when started.
            "npc-play-facial-anim" => {
                let who = self.arg_or_self(a, n, 0);
                let clip = self.arg_or_self(a, n, 1);
                match self.agent(who) {
                    Some(agent) => {
                        agent.facial = Some(clip);
                        ok(1)
                    }
                    None => {
                        self.inner.error(format!("{name}: Can't find npc: {who:#010x}"));
                        ok(0)
                    }
                }
            }
            // (broadcast-event-to-npcs event args…): the event goes to
            // every NPC manager slot's process (`sub_1DDD08`, `sub_83430`).
            "broadcast-event-to-npcs" => {
                if n == 0 {
                    self.inner.error(format!("{name}: Expected at least one argument (event-id)."));
                    return ok(0);
                }
                if a[0] == 0 {
                    self.inner.error(format!("{name}: Empty/null event id."));
                    return ok(0);
                }
                let mut names: Vec<u32> = self.world.agents_ref().map(|x| x.agents.keys().copied().collect()).unwrap_or_default();
                names.sort_unstable();
                let args = a[1..n.min(MAX_ARGS)].to_vec();
                for target in names {
                    self.send(a[0], target, args.clone());
                }
                ok(0)
            }
            // (npc-move-to-spawner npc spawner [motion] [goal]) and the wait
            // form; the spawner's radius tag is the arrival radius.
            "npc-move-to-spawner" | "wait-npc-move-to-spawner" => {
                let who = self.arg_or_self(a, n, 0);
                let spawner = self.arg_or_self(a, n, 1);
                let motion = Motion::from_arg(if n > 2 { a[2] } else { 1 });
                let goal = Goal::from_arg(if n > 3 { a[3] } else { 0 });
                if !self.has_agent(who) {
                    return Ok(self.missing_npc("npc-move-to-spawner", who));
                }
                let Some((target, rotation)) = self.world.locator(spawner) else {
                    self.inner.error("npc-move-to-spawner: Expected at least two arguments (npc-name string) (spawner-name string)".into());
                    return ok(0);
                };
                let radius = match self.world.property(spawner, TAG_ZONE_RADIUS) {
                    Some(super::Property::Float(r)) if r > 0. => r,
                    _ => 0.,
                };
                let forward = glam::Quat::from_array(rotation).normalize() * glam::Vec3::Z;
                Ok(self.order_move(name, who, target, motion, goal, Some(forward.x.atan2(forward.z)), radius, name.starts_with("wait-")))
            }
            // (npc-move-to npc position [motion]): run by default.
            "npc-move-to" | "wait-npc-move-to" => {
                let who = self.arg_or_self(a, n, 0);
                if !self.has_agent(who) {
                    return Ok(self.missing_npc("npc-move-to", who));
                }
                if n < 2 || a[1] == 0 {
                    self.inner.error("npc-move-to: Expected at least two arguments (npc-name string) (position vector)".into());
                    return ok(0);
                }
                let target = Self::vec3(dc, a[1])?;
                let motion = if n > 2 { Motion::from_arg(a[2]) } else { Motion::Run };
                Ok(self.order_move(name, who, target, motion, Goal::Stop, None, 0., name.starts_with("wait-")))
            }
            // (npc-turn-to npc position) / wait form (`sub_299E34`).
            "npc-turn-to" | "wait-npc-turn-to" => {
                let who = self.arg_or_self(a, n, 0);
                let Some(from) = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position) else {
                    if name == "npc-turn-to" {
                        self.inner.error(format!("{name}: Unable to find NPC {who:#010x}"));
                    }
                    return ok(0);
                };
                if n < 2 || a[1] == 0 {
                    if name == "npc-turn-to" {
                        self.inner.error(format!("{name}: Expected two arguments (npc-id symbol) (pos vector)"));
                    }
                    return ok(0);
                }
                let to = Self::vec3(dc, a[1])?;
                let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
                if !self.has_script_skill(who) {
                    return ok(0);
                }
                if let Some(agent) = self.agent(who) {
                    if dx * dx + dz * dz > 1e-8 {
                        agent.order = Some(Order::Turn { yaw: dx.atan2(dz) });
                    }
                }
                if name == "wait-npc-turn-to" {
                    if self.proc.is_none() {
                        self.inner.error(format!("{name}: Called outside script context -- will not wait"));
                        return ok(0);
                    }
                    return Ok(Some(self.suspend(Wait::Npc(who))));
                }
                ok(0)
            }
            // (wait-npc-bind npc): the scripted skill takes the NPC for the
            // calling process (`sub_299FAC`); binding is immediate here.
            "wait-npc-bind" => {
                let who = self.arg_or_self(a, n, 0);
                if !self.has_agent(who) {
                    self.inner.error(format!("{name}: Unable to find NPC {who:#010x}"));
                    return ok(0);
                }
                if !self.has_script_skill(who) {
                    self.inner.error(format!("{name}: Npc {who:#010x} has no script skill"));
                    return ok(0);
                }
                let me = self.self_name();
                if let Some(agent) = self.agent(who) {
                    agent.bound = Some(me);
                }
                ok(0)
            }
            // (npc-unbind npc): release the scripted skill and its order.
            "npc-unbind" => {
                let who = self.arg_or_self(a, n, 0);
                match self.agent(who) {
                    Some(agent) => {
                        agent.bound = None;
                        agent.order = None;
                        agent.ap = None;
                    }
                    None => self.inner.error(format!("{name}: Unable to find NPC {who:#010x}")),
                }
                ok(0)
            }
            // (npc-use-scripted-ap npc spawner): the spawner must hold an
            // `action-pack` object; the NPC walks there and occupies it.
            "npc-use-scripted-ap" => {
                let who = self.arg_or_self(a, n, 0);
                let ap = self.arg_or_self(a, n, 1);
                if !self.has_agent(who) || !self.has_script_skill(who) {
                    return ok(0);
                }
                let is_ap = self.world.object_type(ap) == Some(TYPE_ACTION_PACK);
                let Some((target, _)) = self.world.locator(ap).filter(|_| is_ap) else {
                    self.inner.error(format!("{name}: Can't find scripted ap: {ap:#010x}"));
                    return ok(0);
                };
                let from = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position).unwrap_or(target);
                let path = self.world.nav_path(from, target).unwrap_or_else(|| vec![target]);
                if let Some(agent) = self.agent(who) {
                    agent.order = Some(Order::Move { path, motion: Motion::Walk, goal: Goal::Stop, face: None, radius: 0. });
                    agent.ap = Some(ap);
                }
                ok(0)
            }
            // (wait-npc-leave-scripted-ap npc): `sub_29BA40` exits the AP;
            // the exit clip is not played, so it ends with the next frame.
            "wait-npc-leave-scripted-ap" => {
                let who = self.arg_or_self(a, n, 0);
                if let Some(agent) = self.agent(who) {
                    agent.ap = None;
                    if matches!(agent.order, Some(Order::UseAp { .. })) {
                        agent.order = None;
                    }
                    let frame = self.inner.frame + 1;
                    return Ok(Some(self.suspend(Wait::Frame(frame))));
                }
                ok(0)
            }
            // (npc-use-closest-cover npc frame [threat]): move to the cover
            // point nearest the bound frame (`sub_299C14`).
            "npc-use-closest-cover" => {
                let who = self.arg_or_self(a, n, 0);
                let Some(from) = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position) else {
                    return Ok(self.missing_npc(name, who));
                };
                if !self.has_script_skill(who) {
                    self.inner.error(format!("{name}: Npc {who:#010x} has no script skill"));
                    return ok(0);
                }
                if n < 2 || a[1] == 0 {
                    self.inner.error(format!("{name}: Can't find bound frame"));
                    return ok(0);
                }
                let near = Self::vec3(dc, a[1])?;
                let threat = self.world.locator(self.arg_or_self(a, n, 2)).map(|(p, _)| p);
                let Some(cover) = self.world.closest_cover(near, threat) else { return ok(0) };
                let path = self.world.nav_path(from, cover).unwrap_or_else(|| vec![cover]);
                if let Some(agent) = self.agent(who) {
                    agent.order = Some(Order::Cover { path });
                }
                ok(1)
            }
            // (npc-set-patrol-route npc route [start]): walk the route's
            // spline while unaware.
            "npc-set-patrol-route" => {
                let who = self.arg_or_self(a, n, 0);
                let route = self.arg_or_self(a, n, 1);
                let start = if n > 2 { a[2] as i32 } else { -1 };
                let distance = if start > 0 { self.world.spline_distance(route, self.world.locator(who).map_or([0.; 3], |l| l.0)).unwrap_or(0.) } else { 0. };
                if let Some(agent) = self.agent(who) {
                    agent.patrol = (route != 0).then_some((route, distance, 1.));
                }
                ok(0)
            }
            // (npc-stop npc): stop the current motion (`sub_1A5F90`).
            "npc-stop" => {
                let who = self.arg_or_self(a, n, 0);
                match self.agent(who) {
                    Some(agent) => {
                        agent.order = None;
                        agent.patrol = None;
                        ok(1)
                    }
                    None => Ok(self.missing_npc(name, who)),
                }
            }
            _ => self.npc_native2(dc, name, a, n),
        }
    }

    /// Record a parameter on the NPC that nothing consumes yet.
    fn npc_param(&mut self, native: &str, who: u32, value: u32) -> Reply {
        match self.agent(who) {
            Some(agent) => {
                agent.params.insert(sid(native), value);
                Some((1, Status::Continue))
            }
            None => self.missing_npc(native, who),
        }
    }

    fn npc_native2(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let who = self.arg_or_self(a, n, 0);
        let arg = |i: usize| if n > i { a[i] } else { 0 };
        match name {
            // (wait-npc-use-closest-cover npc [threat] [radius]): walk into
            // the nearest cover (`sub_299C14`) and wait.
            "wait-npc-use-closest-cover" => {
                let Some(from) = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position) else {
                    self.inner.error(format!("wait-npc-use-closest-cover: Cannot find npc '{who:#010x}'"));
                    return ok(0);
                };
                let threat = if n > 1 && a[1] != 0 { self.world.locator(a[1]).map(|(p, _)| p) } else { self.world.locator(sid("player")).map(|(p, _)| p) };
                let Some(cover) = self.world.closest_cover(from, threat) else {
                    self.inner.error("wait-npc-use-closest-cover: Can't find bound frame".into());
                    return ok(0);
                };
                let path = self.world.nav_path(from, cover).unwrap_or_else(|| vec![cover]);
                if let Some(agent) = self.agent(who) {
                    agent.order = Some(Order::Cover { path });
                }
                if self.proc.is_some() {
                    return Ok(Some(self.suspend(Wait::Npc(who))));
                }
                ok(1)
            }
            // (wait-npc-use-scripted-ap npc ap): as `npc-use-scripted-ap`,
            // waiting until the NPC arrives.
            "wait-npc-use-scripted-ap" => {
                let reply = self.npc_native(dc, "npc-use-scripted-ap", a, n)?;
                if self.proc.is_some() && self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(|x| x.order.is_some()) {
                    return Ok(Some(self.suspend(Wait::Npc(who))));
                }
                Ok(reply)
            }
            // (wait-for-npc-not-busy npc): until its scripted order ends.
            "wait-for-npc-not-busy" => {
                if !self.has_agent(who) {
                    self.inner.error(format!("wait-for-npc-not-busy: Can't find npc: {who:#010x}"));
                    return ok(0);
                }
                if self.proc.is_none() {
                    self.inner.error("wait-for-npc-not-busy: Called outside script context -- will not wait".into());
                    return ok(0);
                }
                Ok(Some(self.suspend(Wait::Npc(who))))
            }
            // (npc-update-move-dest npc point): re-path a move order.
            "npc-update-move-dest" => {
                if n < 2 || a[1] == 0 {
                    return ok(0);
                }
                let to = Self::vec3(dc, a[1])?;
                let from = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position);
                let path = from.and_then(|f| self.world.nav_path(f, to)).unwrap_or_else(|| vec![to]);
                match self.agent(who) {
                    Some(agent) => {
                        if let Some(Order::Move { path: p, .. }) = agent.order.as_mut() {
                            *p = path;
                        }
                    }
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(0)
            }
            // (npc-hunt npc target location): run to the location, aware of
            // the target.
            "npc-hunt" => {
                if n < 3 || a[2] == 0 {
                    self.inner.error("npc-hunt: No location specified".into());
                    return ok(0);
                }
                let to = match self.world.locator(a[2]) {
                    Some((p, _)) => p,
                    None => Self::vec3(dc, a[2])?,
                };
                let reply = self.order_move(name, who, to, Motion::Run, Goal::Stop, None, 1., false);
                if let Some(agent) = self.agent(who) {
                    agent.knowledge = npc::KNOWLEDGE_COMBAT;
                    agent.prefer_target = Some(a[1]);
                }
                Ok(reply)
            }
            // (npc-aim-at npc point): turn toward the point.
            "npc-aim-at" => {
                if n < 2 || a[1] == 0 {
                    self.inner.error("npc-aim-at: Expected two arguments (npc-id symbol) (pos vector)".into());
                    return ok(0);
                }
                let to = Self::vec3(dc, a[1])?;
                match self.agent(who) {
                    Some(agent) => {
                        let d = glam::Vec3::from_array(to) - glam::Vec3::from_array(agent.position);
                        if d.x * d.x + d.z * d.z > 1e-6 {
                            agent.yaw = d.x.atan2(d.z);
                        }
                    }
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(0)
            }
            "npc-set-perfect-aim" => {
                match self.agent(who) {
                    Some(agent) => agent.perfect_aim = n < 2 || a[1] != 0,
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(0)
            }
            "npc-lower-weapon" => {
                match self.agent(who) {
                    Some(agent) => agent.weapon_lowered = true,
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(0)
            }
            // Unaware (ambient knowledge).
            "npc-set-ambient" => {
                match self.agent(who) {
                    Some(agent) => agent.knowledge = npc::KNOWLEDGE_UNAWARE,
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(0)
            }
            // The player's faction (`set-object-faction` 0).
            "npc-set-to-player-faction" => {
                if let Some(agent) = self.agent(who) {
                    agent.faction = Some(0);
                }
                ok(0)
            }
            // Fired within the last second.
            "npc-fired-recently?" => {
                let now = self.world.now();
                ok(u32::from(self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(|x| now - x.last_shot < 1.)))
            }
            "npc-reload-weapon" => {
                match self.agent(who) {
                    Some(agent) => agent.gun.reloading = 2.,
                    None => return Ok(self.missing_npc(name, who)),
                }
                ok(1)
            }
            // (npc-has-confirmed-status? npc status): in combat.
            "npc-has-confirmed-status?" => ok(u32::from(self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(|x| x.in_combat()))),
            // (npc-using-skill? npc skill): only the scripted skill runs
            // orders here.
            "npc-using-skill?" => ok(u32::from(self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(|x| x.order.is_some()))),
            "npc-entered-action-pack?" => ok(u32::from(self.world.agents_ref().and_then(|x| x.get(who)).is_some_and(|x| x.ap.is_some() && x.order.is_none()))),
            "npc-is-in-scripted-anim?" | "npc-playing-scripted-animation?" => ok(u32::from(self.world.animation_phase(who).is_some())),
            "npc-in-melee-with-player?" => ok(0),
            // (npc-is-nav-map-position-blocked? npc frame): no nav path to
            // the point.
            "npc-is-nav-map-position-blocked?" => {
                if n < 2 || a[1] == 0 {
                    self.inner.error("npc-is-nav-map-position-blocked?: Expected two arguments (npc-name symbol) (pos bound-frame)".into());
                    return ok(0);
                }
                let to = Self::vec3(dc, a[1])?;
                let from = self.world.agents_ref().and_then(|x| x.get(who)).map(|x| x.position);
                ok(u32::from(from.is_some_and(|f| self.world.nav_path(f, to).is_none())))
            }
            // (npcs-get-player-position): the player's frame as the NPCs
            // know it (their shared knowledge; the player's own here).
            "npcs-get-player-position" => match self.world.locator(sid("player")) {
                Some((p, q)) => ok(crate::ss::rail_natives::alloc_frame(dc, p, q)?),
                None => ok(0),
            },
            // Shots: (fire-weapon-at-target shooter target …),
            // (fire-weapon shooter …), (npc-fire-weapon npc),
            // (emit-bullet-from-spawner spawner target …).
            "fire-weapon-at-target" | "emit-bullet-from-spawner" => {
                if n < 2 || a[0] == 0 || a[1] == 0 {
                    return ok(0);
                }
                let target = if self.world.locator(a[1]).is_some() { crate::vehicle::Target::Object(a[1]) } else { crate::vehicle::Target::Point(Self::vec3(dc, a[1])?) };
                self.world.scripted_shot(a[0], target);
                ok(1)
            }
            "fire-weapon" | "npc-fire-weapon" => {
                let target = self.world.agents_ref().and_then(|x| x.get(who)).and_then(|x| x.prefer_target).unwrap_or(sid("player"));
                self.world.scripted_shot(who, crate::vehicle::Target::Object(target));
                ok(1)
            }
            // Grenades: (throw-grenade source destination …) between two
            // points; NPCs throw at the player.
            "throw-grenade" => {
                if n < 3 || a[1] == 0 || a[2] == 0 {
                    self.inner.error("throw-grenade: no destination position specified".into());
                    return ok(0);
                }
                let (from, to) = (Self::vec3(dc, a[1])?, Self::vec3(dc, a[2])?);
                self.world.throw_grenade(from, to);
                ok(1)
            }
            "npc-throw-grenade-at-player" | "npc-cover-throw-grenade-anim" | "throw-sla-grenade" => {
                let (Some((from, _)), Some((to, _))) = (self.world.locator(who), self.world.locator(sid("player"))) else { return ok(0) };
                self.world.throw_grenade([from[0], from[1] + 1.6, from[2]], to);
                ok(1)
            }
            // (spawn-explosion-npc-only kind locator …).
            "spawn-explosion-npc-only" => {
                if n < 2 || a[1] == 0 {
                    self.inner.error("spawn-explosion failed! -- no valid position".into());
                    return ok(0);
                }
                let at = Self::vec3(dc, a[1])?;
                self.world.npc_blast(at);
                ok(0)
            }
            // (npc-swap-with-ragdoll npc): the NPC dies (no ragdoll).
            "npc-swap-with-ragdoll" => {
                if !self.world.npc_die(who) {
                    return Ok(self.missing_npc(name, who));
                }
                self.world.set_visible(who, false);
                ok(1)
            }
            // Every NPC not of the player's faction dies.
            "all-enemy-npcs-die" => {
                let enemies: Vec<u32> = self.world.agents_ref().map(|x| x.agents.iter().filter(|(_, ag)| ag.faction != Some(0)).map(|(k, _)| *k).collect()).unwrap_or_default();
                for e in enemies {
                    self.world.npc_die(e);
                    self.world.set_visible(e, false);
                }
                ok(0)
            }
            // (set-object-health object fraction): hit points = fraction ×
            // the maximum (health system virtuals +0x28, +0x10).
            "set-object-health" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("set-object-health: Expected: (object-name string)".into());
                    return ok(0);
                }
                let fraction = if n > 1 { f32::from_bits(a[1]) } else { 1. };
                match self.agent(who) {
                    Some(agent) => {
                        agent.hit_points = agent.max_hit_points * fraction.clamp(0., 1.);
                        agent.health = fraction.clamp(0., 1.);
                    }
                    None => {
                        self.inner.error(format!("set-object-health: Object '{who:#010x}' was not of type npc"));
                        return ok(0);
                    }
                }
                ok(0)
            }
            // Recorded without a consumer.
            "npc-laz-set-param" | "npc-change-vision-settings" | "npc-sniper-set-watch-area" | "npc-set-guard-radio-delay" | "npc-use-cap" | "npc-attach-to-battle-manager"
            | "npc-change-sniper-params" | "npc-enable-hit-reactions" | "npc-set-buddy-sniper-hide-time" | "npc-show-hit-effects" | "npc-set-cover-advance-spline"
            | "npc-remove-traversal-skill" | "npc-set-comm-group-id" | "npc-set-lead-drake-spline" | "npc-take-weapon" => Ok(self.npc_param(name, who, arg(1))),
            "npc-disable-deal-no-damage-when-offscreen" => ok(0),
            _ => Ok(None),
        }
    }
}
