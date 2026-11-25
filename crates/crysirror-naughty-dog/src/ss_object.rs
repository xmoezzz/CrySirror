// SPDX-License-Identifier: MPL-2.0
//! Object and entity script natives (`game/script-funcs.cpp`).
use super::{Ctx, Property};
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

/// The spawner tag listing schema spawner candidates (`sub_14CBF0`;
/// the hard and crushing lists apply only in multiplayer).
const TAG_SCHEMA_SPAWNER: u32 = 0x618b_73b8; // `schema-spawner`
/// Camera shake types of `camera-shake` (`sub_13FFAC`).
const SHAKE_STOP: u32 = 0x94f8_66ac;
const SHAKE_PROCEDURAL: u32 = 0xcf48_9dc4;
const SHAKE_GRENADE: u32 = 0x409c_ff29;
/// `grenade-shake` plays this camera animation (`sub_3669C8`).
pub const GRENADE_SHAKE_CLIP: u32 = 0x7f36_e1fe;
/// Object types `get-object-health` reads.
const TYPE_BREAKABLE: u32 = 0x8b7d_6f1a;
const TYPE_RAIL_VEHICLE: u32 = 0xdc98_4a89;

type Reply = Option<(u32, Status)>;

/// A camera shake request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shake {
    /// Stop the shakes (`sub_364F60` advances the shake stack).
    Stop,
    /// A procedural shake of `amount` (`sub_36611C`).
    Procedural { amount: f32 },
    /// The grenade camera animation, scaled by `amount`.
    Grenade { amount: f32 },
}

impl Ctx<'_> {
    /// A process name stands for its spawner; `self` for the caller's.
    fn object(&self, name: u32) -> u32 {
        let name = if name == sid("self") { self.proc.map_or(name, |p| self.inner.procs[p].name) } else { name };
        self.inner.process(name).and_then(|p| self.inner.procs[p].spawner).unwrap_or(name)
    }
    fn object_arg(&self, a: &[u32; MAX_ARGS], n: usize, i: usize) -> u32 {
        if n > i { self.object(a[i]) } else { self.object(sid("self")) }
    }
    pub(super) fn frame(dc: &Dc, at: u32) -> Result<([f32; 3], [f32; 4])> {
        let f = |k: u32| dc.mem.f32(at + 4 * k);
        let q = [f(4)?, f(5)?, f(6)?, f(7)?];
        Ok(([f(0)?, f(1)?, f(2)?], if q.iter().all(|v| *v == 0.) { [0., 0., 0., 1.] } else { q }))
    }

    pub(super) fn object_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let flag = |i: usize| n > i && a[i] != 0;
        match name {
            // (make-inert): the calling object stops acting (vtable +0x94):
            // no collision, no script animation.
            "make-inert" => {
                let me = self.object(sid("self"));
                if !self.world.make_inert(me) {
                    self.inner.error("make-inert: Object 'self' was not found".into());
                }
                ok(0)
            }
            // (wait-spawn-entity name) / (spawn-entity-at name locator) /
            // (wait-spawn-entity-at …): `sub_150008`; spawning completes
            // within the frame here, so the wait forms do not suspend.
            "wait-spawn-entity" | "spawn-entity-at" | "wait-spawn-entity-at" => {
                let who = if n > 0 { a[0] } else { self.self_name_object() };
                if who == 0 {
                    self.inner.error(format!("{}: Expected at least one argument", name.trim_start_matches("wait-")));
                    return ok(0);
                }
                let frame = if name.ends_with("-at") && n > 1 && a[1] != 0 { Some(Self::frame(dc, a[1])?) } else { None };
                let spawned = self.world.spawn(who) || self.world.object_spawned(who);
                if let (true, Some((p, q))) = (spawned, frame) {
                    self.world.place(who, p, q);
                }
                ok(u32::from(spawned))
            }
            // (entity-auto-spawn-once name): the spawner spawns its object
            // once (`sub_7E5CC`); 1 when the spawner exists.
            "entity-auto-spawn-once" => {
                let who = if n > 0 { a[0] } else { self.self_name_object() };
                if self.world.locator(who).is_none() {
                    return ok(0);
                }
                if !self.world.object_spawned(who) {
                    self.world.spawn(who);
                }
                ok(1)
            }
            // (select-best-schema-spawner spawner index): the spawners its
            // schema tags name, the `index`-th modulo their count.
            "select-best-schema-spawner" => {
                let who = if n > 0 { a[0] } else { self.self_name_object() };
                let names = self.world.properties(who, TAG_SCHEMA_SPAWNER);
                let candidates: Vec<u32> = names.into_iter().filter_map(|p| match p {
                    Property::Sid(s) => Some(s),
                    Property::Str(s) => Some(sid(&s)),
                    _ => None,
                }).filter(|s| self.world.locator(*s).is_some()).collect();
                if candidates.is_empty() {
                    return ok(0);
                }
                let index = if n > 1 { a[1] as i32 } else { 0 };
                ok(candidates[index.rem_euclid(candidates.len() as i32) as usize])
            }
            // (look-at-object character target attach-point [blend]).
            "look-at-object" => {
                if n <= 2 {
                    self.inner.error("look-at: Expected three arguments (character-name target-name attach-point-name)".into());
                    return ok(0);
                }
                if a[0] == 0 || a[1] == 0 || a[2] == 0 {
                    self.inner.error("look-at-object: NULL or empty spawner name or world-space point".into());
                    return ok(0);
                }
                let (who, target) = (self.object(a[0]), self.object(a[1]));
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("look-at-object: Cannot find character {who:#010x}"));
                    return ok(0);
                }
                if self.world.locator(target).is_none() {
                    self.inner.error(format!("look-at-object: Cannot find target object {target:#010x}"));
                    return ok(0);
                }
                let blend = if n > 3 { a[3] } else { 1 };
                self.world.look_at(who, Some((target, a[2], blend)));
                ok(1)
            }
            "clear-look-at" => {
                if n == 0 {
                    self.inner.error("clear-look-at: Expected one argument (character-name)".into());
                    return ok(0);
                }
                if a[0] == 0 {
                    self.inner.error("clear-look-at: NULL or empty character name".into());
                    return ok(0);
                }
                let who = self.object(a[0]);
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("clear-look-at: Unable to find object {who:#010x}"));
                    return ok(0);
                }
                self.world.look_at(who, None);
                ok(1)
            }
            // (set-mesh-visible object mesh on?).
            "set-mesh-visible" => {
                if n != 3 {
                    self.inner.error("set-mesh-visible: Expected three args (object-name string) (mesh-name string) (visible? boolean)".into());
                    return ok(0);
                }
                let who = self.object(a[0]);
                if !self.world.set_mesh_visible(who, a[1], a[2] != 0) {
                    self.inner.error(format!("set-mesh-visible: Cannot make {who:#010x} {}visible - no such spawner, or not a ProcessGameObject", if a[2] != 0 { "" } else { "in" }));
                }
                ok(0)
            }
            // (camera-shake type amount [name]).
            "camera-shake" => {
                if n <= 1 {
                    self.inner.error("camera-shake: Expected two arguments (shake-type string) (amount float)".into());
                    return ok(0);
                }
                let amount = f32::from_bits(a[1]);
                let shake = match a[0] {
                    SHAKE_STOP => Shake::Stop,
                    SHAKE_PROCEDURAL => Shake::Procedural { amount },
                    SHAKE_GRENADE => Shake::Grenade { amount },
                    other => {
                        self.inner.error(format!("camera-shake: Unrecognized shake type {other:#010x}"));
                        return ok(0);
                    }
                };
                self.world.camera_shake(shake);
                ok(1)
            }
            // (set-collision-layer object layer): the layer index from its
            // name (`sub_377F48`, 16 invalid).
            "set-collision-layer" => {
                let who = self.object_arg(a, n, 0);
                let layer = if n > 1 { a[1] } else { self.self_name_object() };
                if layer == 0 || self.world.locator(who).is_none() {
                    return ok(0);
                }
                if !self.world.set_collision_layer(who, layer) {
                    self.inner.error(format!("set-collision-layer: Object {who:#010x} has no collision"));
                }
                ok(0)
            }
            // (get-object-health object): a fraction for breakables (current
            // / maximum hit points), the health component for characters,
            // the rail vehicle's value (`sub_146670`).
            "get-object-health" => {
                let who = self.object_arg(a, n, 0);
                if who == 0 {
                    self.inner.error("get-object-state: Expected: (object-name string)".into());
                    return ok(0);
                }
                if let Some(agent) = self.world.agents_ref().and_then(|x| x.get(who)) {
                    return ok(agent.health.to_bits());
                }
                match self.world.object_type(who) {
                    None if who == sid("player") => ok(self.world.player_health().to_bits()),
                    None => {
                        self.inner.error(format!("get-object-health: Object {who:#010x} was not found"));
                        ok(0)
                    }
                    Some(TYPE_BREAKABLE) | Some(TYPE_RAIL_VEHICLE) => ok(self.world.object_health(who).unwrap_or(1.).to_bits()),
                    Some(_) => {
                        self.inner.error(format!("get-object-health: Object {who:#010x} was not of type breakable, npc or railvehicle"));
                        ok(0)
                    }
                }
            }
            // (set-animation-speed object speed [layer]): the speed of the
            // object's animation (event 0xAF4A581F, layer default 6).
            "set-animation-speed" => {
                if n <= 1 {
                    self.inner.error("set-animation-speed: Expected two arguments (object-name speed).".into());
                    return ok(0);
                }
                if a[0] == 0 {
                    self.inner.error("set-animation-speed: NULL or empty object name.".into());
                    return ok(0);
                }
                let who = self.object(a[0]);
                if !self.world.set_animation_speed(who, f32::from_bits(a[1])) {
                    self.inner.error(format!("set-animation-speed: Unable to find object {who:#010x}."));
                }
                ok(0)
            }
            // (stop-animating object [layer 6] [blend]): event 0xD9CB0C6D
            // ends the object's script animation.
            "stop-animating" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("stop-animating: NULL or empty object name.".into());
                    return ok(0);
                }
                let who = self.object(a[0]);
                self.world.stop_animation(who);
                ok(0)
            }
            // (spline-activate spline on?): bit 61 of the spline's flags
            // marks it inactive.
            "spline-activate" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("spline-activate: Argument (spline-name string) is NULL".into());
                    return ok(0);
                }
                if !self.world.activate_spline(a[0], flag(1)) {
                    self.inner.error(format!("spline-activate: Cannot find spline {:#010x}", a[0]));
                    return ok(0);
                }
                ok(1)
            }
            // Nav blockers (`sub_F44D0`) and the polygon under an object
            // (`sub_F4348`).
            "enable-nav-blocker-object" | "disable-nav-blocker-object" | "block-nav-poly-under-object" | "unblock-nav-poly-under-object" => {
                if n == 0 {
                    return ok(0);
                }
                let who = self.object(a[0]);
                let block = matches!(name, "enable-nav-blocker-object" | "block-nav-poly-under-object");
                if !self.world.nav_block(who, block) {
                    self.inner.error(format!("{name}: Object {who:#010x} was not found"));
                }
                ok(0)
            }
            // Cover action packs on an object's collision (`sub_F8B80`
            // removes, `sub_F94A0` regenerates them).
            "remove-cover-action-packs-from-object" | "regenerate-cover-action-packs-from-object" => {
                if n == 0 {
                    return ok(0);
                }
                let who = self.object(a[0]);
                if !self.world.object_cover(who, name.starts_with("regenerate")) {
                    self.inner.error(format!("{name}: Object {who:#010x} was not found"));
                }
                ok(0)
            }
            // (enable-action-pack ap on?): a traversal action pack spawner.
            "enable-action-pack" if n > 0 => {
                self.world.enable_action_pack(a[0], flag(1));
                ok(0)
            }
            // (debug-draw-locator locator [scale] [seconds] [depth?]
            // [color]): the debug renderer, which the player does not draw.
            "debug-draw-locator" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("debug-draw-locator: NULL locator".into());
                }
                ok(0)
            }
            _ => Ok(None),
        }
    }

    fn self_name_object(&self) -> u32 {
        self.proc.map_or(0, |p| self.inner.procs[p].name)
    }
}
