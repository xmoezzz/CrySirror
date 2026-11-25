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
            // (grab-rope grabber rope): the grabber is sent event 0x37DDF093
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
            // Single player (`sub_4466F4` false): not the host, no coop
            // ids or tasks, every player is `player`; `net-send-event`
            // delivers to the local process (the net manager's loopback
            // is assumed).
            "net-is-host" | "net-coop-task" | "net-coop-host-script-id" | "net-player-coop-id" | "net-coop-script-id"
            | "net-current-net-task" | "net-get-net-id-from-player" | "net-all-players-dead?" | "net-get-grapple-time" => ok(0),
            "net-get-player-by-net-id" => ok(sid("player")),
            "net-all-players-in-region?" if n > 0 => ok(u32::from(self.world.in_region(sid("player"), a[0]))),
            "net-send-event" => {
                if n <= 1 {
                    self.inner.error("send-event: Expected at least two arguments (event-id receiver-name).".into());
                    return ok(0);
                }
                let receiver = if a[1] == sid("self") { self.proc.map_or(0, |p| self.inner.procs[p].name) } else { a[1] };
                self.send(a[0], receiver, a[2..n.min(MAX_ARGS)].to_vec());
                ok(0)
            }
            "net-kill-entity" if n > 0 => {
                self.world.kill(a[0]);
                ok(0)
            }
            // Coop presentation and scoring have no single-player effect.
            "net-show-wave-screen" | "net-hide-wave-screen" | "net-draw-objective-icon" | "net-pause-coop-combo" | "net-set-force-tension"
            | "net-npc-confirm-coop-players-contact" | "net-capture-flag" | "net-respawn-flag" | "net-apply-npc-fight-damage" | "net-get-npc-fight-locator" => ok(0),
            // (event-sender): the sending process of the event being
            // handled; (event-sender? name) compares it.
            "event-sender" | "event-sender?" => {
                let sender = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == super::KIND_EVENT)).map(|g| g.sender);
                match sender {
                    None => {
                        self.inner.error(format!("{name}: Called from outside event context, ignored"));
                        ok(0)
                    }
                    Some(s) if name == "event-sender" => ok(s.unwrap_or(0)),
                    Some(s) => {
                        let want = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
                        ok(u32::from(s == Some(want)))
                    }
                }
            }
            // (event-return-boolean v) etc.: the reply to the sender
            // (`sub_7DB488`). Events are queued here, so senders read no
            // reply; the value is kept on the process.
            "event-return-boolean" | "event-return-int32" | "event-return-symbol" | "event-return-float" => {
                if let Some(p) = self.proc {
                    let v = if n > 0 { a[0] } else { 0 };
                    let v = if name == "event-return-boolean" { u32::from(v != 0) } else { v };
                    self.inner.procs[p].vars.insert(sid("event-return"), v);
                }
                ok(0)
            }
            // (get-state-time [process]): seconds in the current state
            // (frames × 1/300 s ticks, `sub_7D7318`).
            "get-state-time" => {
                let who = if n > 0 { self.inner.process(a[0]) } else { self.proc };
                let t = who.map_or(0., |p| (self.world.now() - self.inner.procs[p].state_start) as f32);
                ok(t.to_bits())
            }
            // (wait-until seconds): until `seconds` into the state.
            "wait-until" => {
                if n == 0 {
                    self.inner.error("wait-until: Expected at least one argument: (abs-seconds float)".into());
                    return ok(0);
                }
                let Some(p) = self.proc else { return ok(1) };
                let at = self.inner.procs[p].state_start + f64::from(f32::from_bits(a[0]));
                if self.world.now() >= at {
                    return ok(1);
                }
                let (_, status) = self.suspend(super::Wait::Until(at));
                Ok(Some((1, status)))
            }
            // (get-self-id): the `self` object's id word (+0x4C): its name.
            "get-self-id" => ok(self.proc.map_or(0, |p| self.inner.procs[p].spawner.unwrap_or(self.inner.procs[p].name))),
            "current-time" => ok((self.world.now() as f32).to_bits()),
            "print-integer" => {
                let text = format!("print-integer: {}", if n > 0 { a[0] as i32 } else { 0 });
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            // (auto-save-game): write a checkpoint.
            "auto-save-game" => {
                self.inner.save_requested = true;
                ok(0)
            }
            // (is-difficulty-very-easy?): the profile's very-easy level
            // (index 4 here).
            "is-difficulty-very-easy?" => ok(u32::from(self.inner.difficulty == 4)),
            // (set-render-settings name [name blend]): apply a record at
            // priority 140 (`sub_12A7D0`) or blend two (`sub_127BA4`); 1
            // when the records exist. (set-render-settings-defaults) drops
            // the script's choice.
            "set-render-settings" => {
                if n == 0 {
                    return ok(0);
                }
                let second = (n > 1 && a[1] != 0).then(|| (a[1], if n > 2 { f32::from_bits(a[2]) } else { 0. }));
                let found = crate::render_settings::read(dc, a[0]).ok().flatten().is_some()
                    && second.is_none_or(|(b, _)| crate::render_settings::read(dc, b).ok().flatten().is_some());
                if found {
                    self.world.set_render_settings(Some((a[0], second)));
                }
                ok(u32::from(found))
            }
            "set-render-settings-defaults" => {
                self.world.set_render_settings(None);
                ok(0)
            }
            // (from-ups value dt) = value·dt; (from-ups-no-dt value) =
            // value · the frame time (`sub_46938`).
            "from-ups" if n > 1 => ok((f32::from_bits(a[0]) * f32::from_bits(a[1])).to_bits()),
            "from-ups-no-dt" => {
                if n != 1 {
                    self.inner.error("ERROR: from-ups-no-dt -- wrong number of params".into());
                    return ok(0);
                }
                ok((f32::from_bits(a[0]) * self.world.frame_seconds()).to_bits())
            }
            // (fma-request-part-module module): load a particle module
            // (`sub_76CFC0`).
            "fma-request-part-module" => {
                let module = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
                if module != 0 && !self.inner.module_requests.contains(&module) {
                    self.inner.module_requests.push(module);
                }
                ok(0)
            }
            // (kill-part-spawner spawner [immediately]): the `PartSpawner`
            // (type 0xA51C3C09) is sent event 0x188467CC (`sub_1460B8`).
            "kill-part-spawner" => {
                let who = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
                if self.world.object_type(who) != Some(0xa51c_3c09) {
                    self.inner.error(format!("DcKillPartSpawner: could not find part spawner {who:#010x}"));
                    return ok(0);
                }
                self.world.kill(who);
                ok(1)
            }
            _ => Ok(None),
        }
    }
}
