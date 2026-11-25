// SPDX-License-Identifier: MPL-2.0
//! The remaining game-state natives: explosions, NPC spawning and
//! culling, the grenade director, global material tables, character
//! weather state, the journal, ropes, the HUD and messages.
use super::Ctx;
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use anyhow::{Result, ensure};

/// `npc-collect-one` culls only above this many live NPCs (`sub_2231A8`).
const NPC_COLLECT_ABOVE: usize = 10;

type Reply = Option<(u32, Status)>;

/// Game state the natives write (the game object fields in comments).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameState {
    /// The journal page (+0x1F1C; −1 none) and whether it is open.
    pub journal_page: i32,
    /// HUD +0x64D: hidden.
    pub hud_hidden: bool,
    /// The player's grenade director (player +0x20B0) enabled.
    pub grenade_director: bool,
    /// Character weather state (+0xEE11C0 object): the wet-mask value of
    /// `set-character-wet-mask-value` and snow accumulation (+0x4E1).
    pub wet_mask: u32,
    pub accumulates_snow: bool,
    /// Ropes held: grabber → rope.
    pub ropes: Vec<(u32, u32)>,
    /// Global material parameters by `(material, parameter)`.
    pub material_params: Vec<((u32, u32), Vec<f32>)>,
    /// Messages posted (`spawn-message` text id and arguments).
    pub messages: Vec<(u32, u32, u32)>,
}

impl Ctx<'_> {
    pub(super) fn misc_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        match name {
            // (spawn-explosion locator type [owner] [source] [radius] …): an
            // explosion of `type` (`sub_EE608`).
            "spawn-explosion" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("spawn-explosion failed! -- no valid position".into());
                    return ok(0);
                }
                let (p, q) = Self::frame(dc, a[0])?;
                let kind = if n > 1 { a[1] } else { 0 };
                self.world.spawn_explosion(kind, p, q);
                ok(0)
            }
            // (c-spawn-npc spawner [at-spawner]): spawn the spawner's NPC,
            // at another spawner's locator when given (`sub_80E00`).
            "c-spawn-npc" => {
                if n == 0 || self.world.locator(a[0]).is_none() {
                    self.inner.error(format!("spawn-npc: Failed to spawn {:#010x}", if n > 0 { a[0] } else { 0 }));
                    return ok(0);
                }
                let spawned = self.world.spawn(a[0]);
                if let (true, Some((p, q))) = (spawned, (n > 1 && a[1] != 0).then(|| self.world.locator(a[1])).flatten()) {
                    self.world.place(a[0], p, q);
                }
                ok(u32::from(spawned))
            }
            // (npc-collect-one): above ten live NPCs, despawn one the
            // player cannot see, preferring … (`sub_2231A8`: an off-screen
            // NPC, else any flagged one); 1 when one went.
            "npc-collect-one" => {
                let mut names: Vec<(u32, [f32; 3])> = self.world.agents_ref().map(|x| x.agents.iter().map(|(n, a)| (*n, a.position)).collect()).unwrap_or_default();
                if names.len() <= NPC_COLLECT_ABOVE {
                    return ok(0);
                }
                names.sort_unstable_by_key(|n| n.0);
                let pick = names.iter().find(|(_, p)| !self.world.on_screen(*p)).or(names.first()).map(|n| n.0);
                match pick {
                    Some(who) => {
                        self.world.kill(who);
                        ok(1)
                    }
                    None => ok(0),
                }
            }
            "grenade-director-enable" | "grenade-director-disable" => {
                self.inner.game.grenade_director = name.ends_with("enable");
                ok(0)
            }
            // (set-table-shader-material-values-param table t): a table of
            // 20-byte `{material, parameter, components, from, to}` entries;
            // each parameter becomes `(1 − t)·from + t·to` (`sub_145720`,
            // components 1 or 3).
            "set-table-shader-material-values-param" => {
                if n <= 1 {
                    return ok(0);
                }
                let Some(&table) = dc.symbols.get(&a[0]) else { return ok(1) };
                let t = f32::from_bits(a[1]);
                let (count, entries) = (dc.mem.u32(table)?, dc.mem.u32(table + 4)?);
                ensure!(count <= 4096, "material table size");
                for i in 0..count {
                    let e = entries + 20 * i;
                    let (material, param, components) = (dc.mem.u32(e)?, dc.mem.u32(e + 4)?, dc.mem.u32(e + 8)?);
                    let (from, to) = (dc.mem.u32(e + 12)?, dc.mem.u32(e + 16)?);
                    if components != 1 && components != 3 {
                        continue;
                    }
                    let values = (0..components).map(|k| Ok((1. - t) * dc.mem.f32(from + 4 * k)? + t * dc.mem.f32(to + 4 * k)?)).collect::<Result<Vec<f32>>>()?;
                    let params = &mut self.inner.game.material_params;
                    match params.iter_mut().find(|(k, _)| *k == (material, param)) {
                        Some(slot) => slot.1 = values,
                        None => params.push(((material, param), values)),
                    }
                }
                ok(1)
            }
            "set-character-wet-mask-value" => {
                if n == 0 {
                    self.inner.error("set-character-wet-mask-value: Expected two arguments (int32 float)".into());
                    return ok(0);
                }
                self.inner.game.wet_mask = a[0];
                ok(1)
            }
            "set-state-character-accumulates-snow-from-sky" => {
                if n <= 1 {
                    self.inner.error("set-state-character-accumulates-snow-from-sky: Expected 2 argument2 (:state int) (:time float)".into());
                    return ok(0);
                }
                self.inner.game.accumulates_snow = a[0] != 0;
                ok(1)
            }
            // Journal page +0x1F1C: `journal-set-page` stores page − 1 (pages
            // from 2) and opens the journal; `journal-clear-page` stores −1.
            "journal-set-page" => {
                if n > 0 {
                    if a[0] as i32 <= 1 {
                        self.inner.error("journal-set-page: pageNumber > 1".into());
                    }
                    self.inner.game.journal_page = a[0] as i32 - 1;
                }
                ok(0)
            }
            "journal-clear-page" => {
                self.inner.game.journal_page = -1;
                ok(0)
            }
            // (grab-rope grabber rope): the grabber is sent event 0x37DE2D93
            // with the rope; (drop-rope grabber) lets go.
            "grab-rope" => {
                if n <= 1 {
                    self.inner.error("grab-rope: Expected at least two arguments (grabber-name rope-name).".into());
                    return ok(0);
                }
                if a[0] == 0 || a[1] == 0 {
                    self.inner.error("grab-rope: Invalid grabber-name or rope-name.".into());
                    return ok(0);
                }
                if self.world.locator(a[0]).is_none() {
                    self.inner.error(format!("grab-rope: Cannot find grabber {:#010x}.", a[0]));
                    return ok(0);
                }
                if self.world.locator(a[1]).is_none() {
                    self.inner.error(format!("grab-rope: Unable to find ProcessPhysRope {:#010x}.", a[1]));
                    return ok(0);
                }
                self.inner.game.ropes.retain(|(g, _)| *g != a[0]);
                self.inner.game.ropes.push((a[0], a[1]));
                ok(1)
            }
            "drop-rope" => {
                let who = if n > 0 { a[0] } else { sid("player") };
                self.inner.game.ropes.retain(|(g, _)| *g != who);
                ok(0)
            }
            "hide-hud" => {
                self.inner.game.hud_hidden = true;
                ok(0)
            }
            // (spawn-message text [a] [b]), `sub_D45C4`.
            "spawn-message" => {
                if n > 0 {
                    let message = (a[0], if n > 1 { a[1] } else { 0 }, if n > 2 { a[2] } else { 0 });
                    self.inner.game.messages.push(message);
                    self.world.message(message.0);
                }
                ok(0)
            }
            // (kill-all-breakables): every breakable process (`sub_70F788`).
            "kill-all-breakables" => {
                self.world.kill_all_breakables();
                ok(0)
            }
            _ => Ok(None),
        }
    }
}
