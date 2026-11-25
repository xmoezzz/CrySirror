// SPDX-License-Identifier: MPL-2.0
//! Player script natives (`DcPlayer*`, PPU 0x670000–0x679000).
use super::{Ctx, Setting};
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

/// Script setting ids (`sub_15F480`'s first argument): each call keeps
/// the override alive for 9 frames (`sub_15E628`, expiry frame + 9).
pub const SETTING_BRUTAL_MELEE_DISABLED: u32 = 0x8e83_ae5a;
pub const SETTING_INVINCIBLE: u32 = 0xa706_5a6c;
pub const SETTING_LIGHT_COMBO_TUTORIAL: u32 = 0x0893_05a1;
/// `set-player-infinite-ammo` writes its flag through `sub_15F8AC` into
/// the setting variable at 0xF5041A; the address keys it.
pub const SETTING_INFINITE_AMMO: u32 = 0x00f5_041a;
/// `set-ragdoll-flow` (`sub_15F934`, value float) and
/// `allow-camera-cut-compensate` settings.
pub const SETTING_RAGDOLL_FLOW: u32 = 0x4d95_a0b1;
pub const SETTING_CAMERA_CUT_COMPENSATE: u32 = 0xfaa6_c25b;
const SETTING_FRAMES: u64 = 9;


type Reply = Option<(u32, Status)>;

impl Ctx<'_> {
    fn set_setting(&mut self, id: u32, value: u32) {
        let until = self.inner.frame + SETTING_FRAMES;
        self.inner.settings.insert(id, Setting { value, until });
    }

    pub(super) fn player_native(&mut self, _dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let player = self.world.locator(sid("player")).is_some();
        match name {
            "player-disable-brutal-melee" => self.set_setting(SETTING_BRUTAL_MELEE_DISABLED, 1),
            "set-ragdoll-flow" if n > 0 => self.set_setting(SETTING_RAGDOLL_FLOW, a[0]),
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
            _ => return Ok(None),
        }
        ok(0)
    }
}
