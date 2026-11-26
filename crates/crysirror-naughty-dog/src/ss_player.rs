// SPDX-License-Identifier: MPL-2.0
//! Player script natives (`DcPlayer*`, PPU 0x670000–0x679000).
use super::{Ctx, Setting};
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

/// Script setting keys. `sub_15F480(owner, variable, value, priority)`
/// overrides a setting variable for 9 frames (`sub_15E628`, expiry frame
/// + 9). Natives whose owner sid is their own name are keyed by it; those
/// sharing the `script` owner are keyed by the variable's address.
pub const SETTING_BRUTAL_MELEE_DISABLED: u32 = 0x8e83_955a;
pub const SETTING_INVINCIBLE: u32 = 0xa707_046c;
pub const SETTING_LIGHT_COMBO_TUTORIAL: u32 = 0x0892_ffa1;
/// `set-player-infinite-ammo` writes its flag through `sub_15F8AC` into
/// the setting variable at 0xF5041A; the address keys it.
pub const SETTING_INFINITE_AMMO: u32 = 0x00f5_041a;
/// `set-ragdoll-flow` (`sub_15F934`, value float) and
/// `allow-camera-cut-compensate` settings.
pub const SETTING_RAGDOLL_FLOW: u32 = 0x00e1_4cb0;
/// `player-force-hard-land`: owner `script`, variable 0xF504F1, value true
/// at priority 100; the next landing is a hard one.
pub const SETTING_FORCE_HARD_LAND: u32 = 0x00f5_04f1;
pub const SETTING_CAMERA_CUT_COMPENSATE: u32 = 0xfaa7_265b;
const SETTING_FRAMES: u64 = 9;

/// Flag overrides set to true by their natives, keyed by the setting
/// variable each writes (`sub_15F480(owner, variable, &true, 100)`,
/// variables resolved from the TOC; owner sids in parentheses where they
/// differ from the name).
pub const FLAG_SETTINGS: &[(&str, u32)] = &[
    ("set-player-no-igc-kill-grenade", 0x00f5_0530),
    ("disable-player-cover-exit", 0x00f5_0503),
    ("player-disable-blindfire-targeting", 0x00f5_0529),
    ("player-no-reload", 0x00f5_041c),
    ("player-disable-melee", 0x00f5_0534),
    ("player-disable-flat-jump", 0x00f5_04e7),
    ("journal-disable", 0x00f5_0538),
    ("disable-follow-camera-collision", 0x00f5_047b),
    ("player-disable-cover", 0x00f5_04ea),
    ("player-disable-cover-exit-over", 0x00f5_047e),
    ("force-stealth-off", 0x00e1_63f4),
    ("player-disable-jump", 0x00f5_04e4),
    ("player-force-dyno-jump-up", 0x00f5_04e0),
    ("player-force-medium-combo-tutorial", 0x00f4_0e1b),
    ("player-force-stealth", 0x00f5_047c),
    ("player-hide-weapons", 0x00f5_0536),
    ("player-set-teaching-combo-mode", 0x00f4_0e1d),
    ("set-player-on-collapsing-building", 0x00f5_052e),
    ("dont-collide-against-allies", 0x00f5_0434),
    // owner `blindfire-only-right`
    ("player-blindfire-only-right", 0x00f6_3538),
    ("player-disable-cover-switch", 0x00f5_0550),
    ("player-disable-roll", 0x00f5_04e9),
    ("rail-enable-truck-jump", 0x00f6_e74c),
    ("set-player-carry-npc-slow-walk", 0x00f5_1298),
    ("sla-force-tackle", 0x00ec_6294),
    ("camera-shake-in-zoom-camera", 0x00f5_0532),
];
pub const SETTING_DISABLE_JUMP: u32 = 0x00f5_04e4;
pub const SETTING_DISABLE_COVER: u32 = 0x00f5_04ea;
pub const SETTING_NO_RELOAD: u32 = 0x00f5_041c;
pub const SETTING_HIDE_WEAPONS: u32 = 0x00f5_0536;
/// `set-player-max-fall-height (metres)`: `sub_15F934` float override of
/// 0xF5050C (owner 0x2516CF0D), default 8 m; falls beyond it kill.
pub const SETTING_MAX_FALL_HEIGHT: u32 = 0x00f5_050c;
pub const DEFAULT_MAX_FALL_HEIGHT: f32 = 8.;
/// `set-reverse-jump-enable (on)`: owner 0xF7374238, variable 0xF50495.
pub const SETTING_REVERSE_JUMP: u32 = 0x00f5_0495;
/// `player-force-aiming-time (seconds)`: owner `player-force-aiming-time`,
/// variable 0xF503FE, held for `seconds × 300` extra ticks.
pub const SETTING_FORCE_AIMING: u32 = 0x00f5_03fe;


type Reply = Option<(u32, Status)>;

impl Ctx<'_> {
    fn set_setting(&mut self, id: u32, value: u32) {
        let until = self.inner.frame + SETTING_FRAMES;
        self.inner.settings.insert(id, Setting { value, until });
    }
    /// An override held `seconds` longer (the last `sub_15F480` argument
    /// adds ticks to the expiry).
    fn set_setting_for(&mut self, id: u32, value: u32, seconds: f32) {
        let frames = (seconds.max(0.) / self.world.frame_seconds().max(1e-3)).ceil() as u64;
        let until = self.inner.frame + SETTING_FRAMES + frames;
        self.inner.settings.insert(id, Setting { value, until });
    }

    pub(super) fn player_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let player = self.world.locator(sid("player")).is_some();
        if let Some(&(_, var)) = FLAG_SETTINGS.iter().find(|(k, _)| *k == name) {
            self.set_setting(var, 1);
            return ok(0);
        }
        match name {
            "set-player-max-fall-height" => self.set_setting(SETTING_MAX_FALL_HEIGHT, if n > 0 { a[0] } else { DEFAULT_MAX_FALL_HEIGHT.to_bits() }),
            "set-reverse-jump-enable" if n == 1 => self.set_setting(SETTING_REVERSE_JUMP, u32::from(a[0] != 0)),
            "player-force-aiming-time" if n == 1 => {
                let t = f32::from_bits(a[0]);
                if t < 0. {
                    self.inner.error(format!("player-force-aiming-time : negative time {t} specified, setting time to 0.0"));
                }
                self.set_setting_for(SETTING_FORCE_AIMING, 1, t.max(0.));
            }
            // (set-reload-pistol-if-empty on): a persistent variable write
            // (`sub_15F8AC`, 0xF5049A), like infinite ammo.
            "set-reload-pistol-if-empty" if n == 1 => {
                self.inner.settings.insert(0x00f5_049a, Setting { value: u32::from(a[0] != 0), until: u64::MAX });
            }
            "player-disable-brutal-melee" => self.set_setting(SETTING_BRUTAL_MELEE_DISABLED, 1),
            "set-ragdoll-flow" if n > 0 => self.set_setting(SETTING_RAGDOLL_FLOW, a[0]),
            "player-force-hard-land" => self.set_setting(SETTING_FORCE_HARD_LAND, 1),
            "allow-camera-cut-compensate" => self.set_setting(SETTING_CAMERA_CUT_COMPENSATE, 1),
            // (player-disable-controls-explicit mask): as
            // `player-disable-controls`; mask 0 disables nothing.
            "player-disable-controls-explicit" => {
                self.world.player_controls(n > 0 && a[0] == 0);
            }
            "set-player-invincible" => self.set_setting(SETTING_INVINCIBLE, 1),
            // Also clears the tutorial's pending state (`sub_15FDCC`).
            "player-force-light-combo-tutorial" => self.set_setting(SETTING_LIGHT_COMBO_TUTORIAL, 1),
            "set-player-infinite-ammo" => {
                let on = n > 0 && a[0] != 0;
                self.set_setting(SETTING_INFINITE_AMMO, u32::from(on));
            }
            // The melee combo window belongs to the melee skill's states
            // (`sub_708EF0` 0xE1938143 / 0xFA706B68): the player
            // controller has none, so no combo is open.
            "player-can-melee-combo?" => return ok(0),
            // (player-in-combat?): the global combat flag (`sub_7E8748`):
            // an NPC in combat.
            "player-in-combat?" => {
                let fighting = self.world.agents_ref().is_some_and(|x| x.agents.values().any(|a| a.in_combat()));
                return ok(u32::from(fighting));
            }
            // The carried object's id (+0x4C) while carrying; nothing is
            // ever picked up by the controller.
            "get-player-carried-object" => {
                if !player {
                    self.inner.error("DcGetPlayerCarriedObject: cannot find player".into());
                }
                return ok(self.world.player_carried().unwrap_or(0));
            }
            // Inventory at player +0x388 (`sub_2CE6D0`).
            "player-has-weapon?" => {
                let weapon = if n > 0 { a[0] } else { 0 };
                return ok(u32::from(player && self.inner.weapons.contains_key(&weapon)));
            }
            // (give-player-weapon weapon [ammo]): set the ammo of a held
            // weapon, or spawn one (`sub_873F68`, full ammo for -1) and
            // send the player its pick-up event.
            "give-player-weapon" => {
                if player && n > 0 {
                    let ammo = if n > 1 { a[1] as i32 } else { -1 };
                    let held = self.inner.weapons.contains_key(&a[0]);
                    if !held || ammo > -1 {
                        self.inner.weapons.insert(a[0], ammo);
                    }
                    self.inner.current_weapon = Some(a[0]);
                }
                return ok(0);
            }
            // (weapon-set-reset): copy the `*player-starting-weapons*`
            // weapon-set (80 bytes: four 16-byte slots {name, weapon, ammo,
            // 0}) over the saved set (`sub_7E84CC`) the player's inventory
            // is restored from; here it replaces the inventory.
            "weapon-set-reset" => {
                if let Some(&set) = dc.symbols.get(&sid("*player-starting-weapons*")) {
                    self.inner.weapons.clear();
                    self.inner.current_weapon = None;
                    for slot in 0..4 {
                        let weapon = dc.mem.u32(set + 16 * slot + 4)?;
                        if weapon != 0 && weapon != sid("none") {
                            self.inner.weapons.insert(weapon, dc.mem.u32(set + 16 * slot + 8)? as i32);
                            self.inner.current_weapon.get_or_insert(weapon);
                        }
                    }
                }
                return ok(0);
            }
            // (player-set-prone-mode settings): the prone settings record
            // (player +0x2190); the controller has no prone movement.
            "player-set-prone-mode" => {
                if player && n > 0 {
                    self.inner.prone = Some(a[0]);
                }
                return ok(0);
            }
            // (mirror-aiming-at? object frame): the object's process is asked
            // (event 0x318A8879) whether the player aims at the frame; the
            // controller has no aiming, so it never does.
            "mirror-aiming-at?" => {
                if n <= 1 {
                    self.inner.error("mirror-aiming-at? : not enough parameters".into());
                } else if a[1] == 0 {
                    self.inner.error("mirror-aiming-at? : passed null boundFrame".into());
                }
                return ok(0);
            }
            // (get-joint-angle-diff-from-bindpose object joint): degrees
            // between the joint's current local rotation and its bind
            // rotation (`sub_151E60`, × 57.29578).
            "get-joint-angle-diff-from-bindpose" => {
                if n <= 1 {
                    self.inner.error(format!("{name}: Expected 2 arguments: (object-name string) (joint-name string)"));
                    return ok(0);
                }
                let object = self.inner.process(a[0]).and_then(|p| self.inner.procs[p].spawner).unwrap_or(if a[0] == sid("self") { self.proc.and_then(|p| self.inner.procs[p].spawner).unwrap_or(0) } else { a[0] });
                return match self.world.joint_bind_angle(object, a[1]) {
                    Some(radians) => ok((radians * 57.295_78).to_bits()),
                    None => {
                        self.inner.error(format!("{name}: Object {object:#010x} has no joint {:#010x}", a[1]));
                        ok(0)
                    }
                };
            }
            "player-in-state?" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                return ok(u32::from(self.world.player_state() == Some(a[0])));
            }
            // (kill-player-no-anim [ragdoll] [death-cam] [delay]): defaults
            // false, true, 1/15 s; 1 when there is a player.
            "kill-player-no-anim" => {
                if !player {
                    return ok(0);
                }
                let ragdoll = n > 0 && a[0] != 0;
                let camera = n < 2 || a[1] != 0;
                let delay = if n > 2 { f32::from_bits(a[2]) } else { 1. / 15. };
                self.world.kill_player(ragdoll, camera, delay);
                return ok(1);
            }
            _ => return self.player_native2(dc, name, a, n),
        }
        ok(0)
    }

    fn state_is(&self, class: &str) -> bool {
        self.world.player_state() == Some(sid(class))
    }

    /// Player queries and requests served by the host controller.
    fn player_native2(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let okf = |v: f32| Ok(Some((v.to_bits(), Status::Continue)));
        let f = |i: usize, default: f32| if n > i { f32::from_bits(a[i]) } else { default };
        let player = self.world.locator(sid("player"));
        let point = |dc: &Dc, at: u32| -> Result<[f32; 3]> { Ok([dc.mem.f32(at)?, dc.mem.f32(at + 4)?, dc.mem.f32(at + 8)?]) };
        match name {
            // (kill-player): event 0xC888CE24 to the player (`sub_673B28`).
            "kill-player" => {
                if player.is_none() {
                    return ok(0);
                }
                self.world.player_request(PlayerRequest::Kill { fall: false });
                ok(1)
            }
            // (fall-to-death flag value): event 0x9C13A939 to the player.
            "fall-to-death" => {
                self.world.player_request(PlayerRequest::Kill { fall: true });
                ok(0)
            }
            // Health system +348: set (+0x10) / get (+0x24) whole hit points.
            "set-player-health" => {
                if player.is_none() {
                    self.inner.error("pget-player-health: cannot find player".into());
                } else if n > 0 {
                    self.world.player_request(PlayerRequest::SetHealth(a[0] as i32 as f32));
                }
                ok(0)
            }
            "get-player-health" => {
                if player.is_none() {
                    self.inner.error("pget-player-health: cannot find player".into());
                    return ok(0);
                }
                ok(self.world.player_health().round() as i32 as u32)
            }
            // The length of the player's horizontal velocity.
            "get-player-speed" => {
                if player.is_none() {
                    self.inner.error("get-player-speed: cannot find player".into());
                }
                okf(self.world.player_speed().unwrap_or(0.))
            }
            // (player-turn-to point) (`sub_5794C0`).
            "player-turn-to" => {
                if n > 0 && a[0] != 0 {
                    let p = point(dc, a[0])?;
                    self.world.player_request(PlayerRequest::TurnTo(p));
                }
                ok(0)
            }
            "player-climb-up" => {
                self.world.player_request(PlayerRequest::ClimbUp);
                ok(0)
            }
            "player-force-edge-fall" => {
                self.world.player_request(PlayerRequest::EdgeFall);
                ok(0)
            }
            // (player-force-jump-at locator).
            "player-force-jump-at" => {
                if n > 0 && a[0] != 0 {
                    let p = point(dc, a[0])?;
                    self.world.player_request(PlayerRequest::JumpAt(p));
                }
                ok(0)
            }
            "player-unparent" => {
                self.world.player_request(PlayerRequest::Unparent);
                ok(0)
            }
            "player-hide-this-frame" => {
                self.world.player_request(PlayerRequest::HideThisFrame);
                ok(0)
            }
            "player-go-ragdoll" => {
                self.world.player_request(PlayerRequest::Ragdoll);
                ok(0)
            }
            "player-fire-weapon-at-reticle" => {
                self.world.player_request(PlayerRequest::Fire);
                ok(0)
            }
            // (player-take-melee-damage attacker).
            "player-take-melee-damage" => {
                if n == 0 {
                    self.inner.error("player-take-melee-damage: not enough parameters".into());
                    return ok(0);
                }
                if self.world.locator(a[0]).is_none() {
                    self.inner.error(format!("player-take-melee-damage: cannot find attacker {:#010x}", a[0]));
                    return ok(0);
                }
                self.world.player_request(PlayerRequest::MeleeDamage(a[0]));
                ok(0)
            }
            "player-climbing-up?" => ok(u32::from(self.state_is("EdgeClimbUp"))),
            "player-in-cover?" => ok(u32::from(self.state_is("Cover"))),
            // No swing bars, pushing, aiming or melee in the controller.
            "player-hanging-on-bar?" | "player-is-aiming?" | "player-can-melee?" | "player-teach-combo-square?" | "player-teach-combo-triangle?" => {
                if name == "player-hanging-on-bar?" && player.is_none() {
                    self.inner.error("player-hanging-on-bar? - player not found".into());
                }
                ok(0)
            }
            "player-aim-time?" => okf(0.),
            "player-pushing?" => {
                if n == 0 {
                    self.inner.error("player-pushing? - expects one argument 'name' of string type".into());
                }
                ok(0)
            }
            "player-pushing-speed" | "player-push-forward-speed" => okf(0.),
            "player-initiate-melee" => ok(0),
            // Hurt within the last half second.
            "player-check-for-got-hit" => ok(u32::from(self.world.player_hit_age() < 0.5)),
            "get-player-platform" => ok(self.world.player_support().unwrap_or(0)),
            // (player-has-weapon-and-ammo? weapon) / (…-and-no-ammo? weapon):
            // -1 stands for the weapon's full load.
            "player-has-weapon-and-ammo?" | "player-has-weapon-and-no-ammo?" => {
                let ammo = if n > 0 { self.inner.weapons.get(&a[0]).copied() } else { None };
                let want_ammo = name == "player-has-weapon-and-ammo?";
                ok(u32::from(ammo.is_some_and(|v| (v != 0) == want_ammo)))
            }
            // (remove-player-weapon weapon) / (player-remove-weapons).
            "remove-player-weapon" | "player-remove-weapons" => {
                if name == "player-remove-weapons" {
                    self.inner.weapons.clear();
                } else if n > 0 {
                    self.inner.weapons.remove(&a[0]);
                }
                if self.inner.current_weapon.is_some_and(|w| !self.inner.weapons.contains_key(&w)) {
                    self.inner.current_weapon = self.inner.weapons.keys().next().copied();
                }
                ok(0)
            }
            // (holster-player-weapon mode): `immediate`, `fast`, `slow` or
            // `false` (draw again).
            "holster-player-weapon" => {
                let mode = if n > 0 { a[0] } else { sid("slow") };
                self.inner.holstered = mode != sid("false") && mode != 0;
                ok(0)
            }
            // (player-instant-draw-weapon weapon).
            "player-instant-draw-weapon" => {
                if n > 0 && self.inner.weapons.contains_key(&a[0]) {
                    self.inner.current_weapon = Some(a[0]);
                }
                self.inner.holstered = false;
                ok(0)
            }
            // (player-facing-position? point max-degrees): the angle between
            // the player's forward and the point.
            "player-facing-position?" => {
                let (Some((p, q)), true) = (player, n > 1 && a[0] != 0) else { return ok(0) };
                let target = point(dc, a[0])?;
                let fwd = glam::Quat::from_array(q).normalize() * glam::Vec3::Z;
                let d = glam::Vec3::from_array(target) - glam::Vec3::from_array(p);
                let (fh, dh) = (glam::Vec3::new(fwd.x, 0., fwd.z).normalize_or_zero(), glam::Vec3::new(d.x, 0., d.z).normalize_or_zero());
                let angle = fh.dot(dh).clamp(-1., 1.).acos().to_degrees();
                ok(u32::from(angle <= f(1, 0.)))
            }
            // (player-in-npc-way? npc): within 1.5 m ahead of the NPC.
            "player-in-npc-way?" => {
                let (Some((p, _)), Some((o, q))) = (player, if n > 0 { self.world.locator(a[0]) } else { None }) else { return ok(0) };
                let fwd = glam::Quat::from_array(q).normalize() * glam::Vec3::Z;
                let d = glam::Vec3::from_array(p) - glam::Vec3::from_array(o);
                ok(u32::from(d.length() < 1.5 && d.dot(fwd) > 0.))
            }
            // The grabbable edge nearest the player (within 3 m).
            "player-get-closest-edge-locator" => {
                let Some((p, _)) = player else {
                    self.inner.error("player-get-closest-edge-locator: Can't find player.".into());
                    return ok(0);
                };
                match self.world.closest_edge(p, 3.) {
                    Some((at, q)) => ok(crate::ss::rail_natives::alloc_frame(dc, at, q)?),
                    None => {
                        self.inner.error("player-get-closest-edge-locator: Can't find edge locator.".into());
                        ok(0)
                    }
                }
            }
            // (is-stick-pressed-towards point): the left stick, taken in the
            // camera's frame, points within 45° of the direction to the
            // point.
            "is-stick-pressed-towards" => {
                let (Some((p, _)), Some(cam), true) = (player, self.world.camera_forward(), n > 0 && a[0] != 0) else { return ok(0) };
                let s = self.world.sticks(0);
                let stick = glam::Vec2::new(s[0], s[1]);
                if stick.length() < 0.3 {
                    return ok(0);
                }
                let fwd = glam::Vec3::new(cam[0], 0., cam[2]).normalize_or(glam::Vec3::Z);
                let right = fwd.cross(glam::Vec3::Y);
                let wish = (right * stick.x + fwd * -stick.y).normalize_or_zero();
                let d = glam::Vec3::from_array(point(dc, a[0])?) - glam::Vec3::from_array(p);
                ok(u32::from(wish.dot(glam::Vec3::new(d.x, 0., d.z).normalize_or_zero()) > std::f32::consts::FRAC_1_SQRT_2))
            }
            // (pad-get-button-press-interval pad button).
            "pad-get-button-press-interval" => okf(self.world.button_press_interval(if n > 0 { a[0] } else { 0 }, if n > 1 { a[1] } else { 0 })),
            // The start-phase family matches the player's current foot
            // positions against the clip's foot channels (`sub_4F5468`,
            // `sub_4F4DD0`); not decoded: phase 0.
            "calc-start-phase" | "calc-start-phase-range" | "calc-start-phase-max-frames" => okf(0.),
            // (calc-new-phase-from-speed phase …): the phase is kept.
            "calc-new-phase-from-speed" => okf(f(0, 0.)),
            // (choose-stumble-anim …anims): the first candidate (the
            // direction choice, `player-choose-stumble-dir`, is not decoded).
            "choose-stumble-anim" => ok(if n > 1 { a[1] } else { 0 }),
            // (get-grapple-ap-ref attacker victim): the victim's frame.
            "get-grapple-ap-ref" => {
                if n < 2 {
                    self.inner.error("get-grapple-ap-ref: not enough parameters".into());
                    return ok(0);
                }
                match self.world.locator(a[1]) {
                    Some((p, q)) => ok(crate::ss::rail_natives::alloc_frame(dc, p, q)?),
                    None => {
                        self.inner.error("get-grapple-ap-ref: could not find attacker and victim".into());
                        ok(0)
                    }
                }
            }
            // (set-move-mode mode): a `*player-move-modes*` entry
            // (`sub_644BC0`), recorded.
            "set-move-mode" => {
                self.inner.move_mode = if n > 0 { Some(a[0]) } else { None };
                ok(0)
            }
            "set-walk-mode" => {
                self.inner.error("set-walk-mode: This is no longer supported, talk to a programmer!".into());
                ok(0)
            }
            // Recorded without a consumer: animation update rate, move
            // tracker, jump spawner, binoculars, health regeneration.
            "player-lerp-anim-update-rate" | "player-enable-move-tracker" | "set-player-jump-spawner" | "show-binoculars" | "set-health-parameters" => ok(0),
            _ => Ok(None),
        }
    }
}

/// What a script asks of the player controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerRequest {
    Kill { fall: bool },
    SetHealth(f32),
    TurnTo([f32; 3]),
    ClimbUp,
    EdgeFall,
    JumpAt([f32; 3]),
    Unparent,
    HideThisFrame,
    Ragdoll,
    Fire,
    MeleeDamage(u32),
}
