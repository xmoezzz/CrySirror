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
    /// Speech muted until (world seconds) by object (`shut-up`), and idle
    /// chatter suppressed until (`suppress-idle-speech`).
    pub muted: Vec<(u32, f64)>,
    pub idle_speech_until: f64,
    /// Contextual look targets offered by state scripts.
    pub contextual: Vec<u32>,
    pub(crate) movie_sound: Option<(String, i32)>,
    pub(crate) stingers: Vec<u32>,
    /// Flags recorded without a consumer, by native name `StringId`.
    pub flags: std::collections::BTreeMap<u32, u32>,
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
            // (spawn-message id text [element] [flag]), `sub_D45C4`: with a
            // HUD element the text shows on it until `despawn-message`.
            "spawn-message" => {
                if n > 0 {
                    let message = (a[0], if n > 1 { a[1] } else { 0 }, if n > 2 { a[2] } else { 0 });
                    self.inner.game.messages.push(message);
                    if message.2 != 0 {
                        self.world.show_message(message.0, message.1, message.2);
                    } else {
                        self.world.message(message.0);
                    }
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
            // (display-button-hud element): show a `hud-settings` element
            // (`sub_8A26D0`); scripts repeat it while it should show.
            "display-button-hud" if n > 0 => {
                self.world.display_hud(Some(a[0]));
                ok(0)
            }
            // (hide-hint): clears the hint element and its setting
            // 0x018279E9 (`sub_14209C`).
            "hide-hint" => {
                self.world.display_hud(None);
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
            _ => self.misc_native2(dc, name, a, n),
        }
    }

    fn misc_native2(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let f = |i: usize, default: f32| if n > i { f32::from_bits(a[i]) } else { default };
        let object = |ctx: &Self, i: usize| -> u32 {
            let name = if n > i { a[i] } else { sid("self") };
            let name = if name == sid("self") { ctx.proc.map_or(name, |p| ctx.inner.procs[p].name) } else { name };
            ctx.inner.process(name).and_then(|p| ctx.inner.procs[p].spawner).unwrap_or(name)
        };
        match name {
            // (attach-flashlight object joint index r g b …): a spot light
            // on the joint (radius 15 m, 30° cone: not native values).
            "attach-flashlight" => {
                if n < 2 {
                    self.inner.error("attach-flashlight: Expected at least two args: (object-name string) (joint-name string) (flashlight-index int32) (r float) (g float) (b float)".into());
                    return ok(0);
                }
                let who = object(self, 0);
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("attach-flashlight: Unable to find game object '{who:#010x}'."));
                    return ok(0);
                }
                let light = flashlight_id(who, if n > 2 { a[2] } else { 0 });
                self.world.add_dynamic_light(light, super::LightLocator::Joint { object: who, joint: a[1] }, FLASHLIGHT_RADIUS, FLASHLIGHT_CONE, true);
                if n > 5 {
                    self.world.dynamic_light_color(light, [f(3, 1.), f(4, 1.), f(5, 1.)], 1.);
                }
                ok(1)
            }
            // (turn-on-flashlight object on) / (kill-flashlight object).
            "turn-on-flashlight" | "kill-flashlight" => {
                let who = object(self, 0);
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("turn-on-flashlight: Unable to find game object '{who:#010x}'."));
                    return ok(0);
                }
                let light = flashlight_id(who, 0);
                let on = name == "turn-on-flashlight" && (n < 2 || a[1] != 0);
                if on {
                    self.world.add_dynamic_light(light, super::LightLocator::Joint { object: who, joint: sid("headlight") }, FLASHLIGHT_RADIUS, FLASHLIGHT_CONE, true);
                } else {
                    self.world.remove_dynamic_light(light);
                }
                ok(1)
            }
            // (look-at character point [blend]): head look (presentation;
            // the point form keeps the target object's form off).
            "look-at" => {
                if n < 2 || a[0] == 0 || a[1] == 0 {
                    self.inner.error("look-at: Expected at least two arguments (character-name world-point)".into());
                    return ok(0);
                }
                let who = object(self, 0);
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("look-at: Unable to find character '{who:#010x}'"));
                    return ok(0);
                }
                self.world.look_at(who, None);
                ok(1)
            }
            // (cutscene-hide-actor name).
            "cutscene-hide-actor" => {
                let who = object(self, 0);
                self.world.set_visible(who, false);
                ok(0)
            }
            // (shut-up object [seconds]): stop speaking, muted that long.
            "shut-up" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("shut-up: Expected at least one argument (object-name symbol) (seconds float)".into());
                    return ok(0);
                }
                let who = object(self, 0);
                self.world.stop_speaking(who);
                let until = self.world.now() + f64::from(f(1, 0.));
                self.inner.game.muted.retain(|(o, _)| *o != who);
                self.inner.game.muted.push((who, until));
                ok(0)
            }
            "suppress-idle-speech" => {
                if n == 0 {
                    self.inner.error("suppress-idle-speech: Expected one argument (seconds float)".into());
                    return ok(0);
                }
                self.inner.game.idle_speech_until = self.world.now() + f64::from(f(0, 0.));
                ok(0)
            }
            // (group-spawned-count group).
            "group-spawned-count" => {
                let group = if n > 0 { a[0] } else { 0 };
                if !dc.symbols.contains_key(&group) {
                    self.inner.error(format!("group-spawned-count: Unknown group '{group:#010x}'"));
                    return ok(0);
                }
                let members = Self::group_members(dc, group)?;
                ok(members.iter().filter(|m| self.world.object_spawned(**m)).count() as u32)
            }
            // (battle-select-best-spawner-from-group group …): the first
            // member not yet spawned (the native scoring is not decoded).
            "battle-select-best-spawner-from-group" => {
                let members = Self::group_members(dc, if n > 0 { a[0] } else { 0 })?;
                ok(members.iter().copied().find(|m| !self.world.object_spawned(*m)).or(members.first().copied()).unwrap_or(0))
            }
            // (entity-alive? name): spawned and not a killed NPC.
            "entity-alive?" => {
                let who = object(self, 0);
                ok(u32::from(self.world.object_spawned(who) && !self.world.npc_killed(who)))
            }
            "entity-name" => ok(0),
            // (swap-entity process spawner): the process's object goes, the
            // spawner's comes.
            "swap-entity" => {
                if n < 2 {
                    self.inner.error("swap-entity: needs two arguments".into());
                    return ok(0);
                }
                let out = object(self, 0);
                self.world.kill(out);
                if !self.world.spawn(a[1]) {
                    self.inner.error(format!("swap-entity: FAILED to spawn {:#010x}", a[1]));
                    return ok(0);
                }
                ok(1)
            }
            // (translate-y-loop object speed min max): each call moves the
            // object up by speed × frame time, back to `min` past `max`.
            "translate-y-loop" => {
                let who = object(self, 0);
                let Some((mut p, q)) = self.world.locator(who) else {
                    self.inner.error(format!("get-object-locator: Object '{who:#010x}' was not found, is not a ProcessGameObject or has a NULL GameObject"));
                    return ok(0);
                };
                let (speed, min, max) = (f(1, 0.), f(2, 0.), f(3, 0.));
                p[1] += speed * self.world.frame_seconds();
                if p[1] > max {
                    p[1] = min;
                }
                self.world.place(who, p, q);
                ok(0)
            }
            // Contextual look offers from state scripts.
            "offer-contextual-entity" | "offer-contextual-process" | "remove-contextual-entity" => {
                if self.proc.is_none() {
                    self.inner.error(format!("{name}: Called from outside state script context, ignored"));
                    return ok(0);
                }
                let me = object(self, usize::MAX);
                self.inner.game.contextual.retain(|c| *c != me);
                if name != "remove-contextual-entity" {
                    self.inner.game.contextual.push(me);
                }
                ok(0)
            }
            // No dropped weapons or attachables are simulated.
            "kill-all-attachables" | "kill-all-dropped-weapons" | "kill-dropped-weapons-in-sphere" | "reset-actors" => ok(0),
            // Physics, IK and snow effects recorded without a consumer.
            "apply-explosion-impulse" | "predict-on-physics-driven" | "freeze-binding-when-non-physical" | "allow-leg-ik-by-others" | "disallow-leg-ik-by-others"
            | "set-off-balance-by-spawner" | "set-state-character-accumulates-snow-from-ground" | "spawn-snow-disturb" | "set-focusable-ignore" | "set-focusable-grappled" => {
                self.inner.game.flags.insert(sid(name), if n > 0 { a[0] } else { 1 });
                ok(0)
            }
            _ => self.misc_native3(dc, name, a, n),
        }
    }
}

impl Ctx<'_> {
    fn misc_native3(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let okf = |v: f32| Ok(Some((v.to_bits(), Status::Continue)));
        let f = |i: usize, default: f32| if n > i { f32::from_bits(a[i]) } else { default };
        let arg = |i: usize| if n > i { a[i] } else { 0 };
        let vec3 = |dc: &Dc, at: u32| -> Result<[f32; 3]> { Ok([dc.mem.f32(at)?, dc.mem.f32(at + 4)?, dc.mem.f32(at + 8)?]) };
        match name {
            // Attack events carry an attack record as their first argument
            // (`sub_81ADC`: +4 type 0–8, +8/+12 source handle, +24 damage).
            // The host's combat does not post attack events, so inside an
            // event these take the native's not-an-attack path.
            "event-attack-type?" | "event-attack-source?" | "event-object-damage" => {
                let group = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == super::KIND_EVENT)).map(|g| (g.event, g.args.first().copied()));
                let record = match group {
                    None => {
                        self.inner.error(format!("{name}: Called from outside event context, ignored"));
                        return ok(0);
                    }
                    Some((event, Some(record))) if event == super::EVENT_ATTACK && record != 0 => record,
                    Some(_) => {
                        self.inner.error(format!("{name}: Not an Attack event, ignored"));
                        return ok(0);
                    }
                };
                match name {
                    "event-attack-type?" => {
                        if n == 0 || a[0] > 8 {
                            self.inner.error(format!("event-attack-type?: Invalid attack type {}", if n > 0 { a[0] as i32 } else { -1 }));
                            return ok(0);
                        }
                        ok(u32::from(dc.mem.u32(record + 4)? == a[0]))
                    }
                    "event-attack-source?" => {
                        let source = dc.mem.u32(record + 8)?;
                        let want = if n > 0 { a[0] } else { 0 };
                        let want = self.inner.process(want).and_then(|p| self.inner.procs[p].spawner).unwrap_or(want);
                        ok(u32::from(source == want || self.inner.process(source).is_some_and(|p| self.inner.procs[p].name == want)))
                    }
                    _ => ok(dc.mem.u32(record + 24)?),
                }
            }
            // Weather, wind and water-reflection state (`sub_84F8E0`,
            // `sub_379244`): recorded; no rain or wetness is drawn.
            "set-rain-state" | "set-time-to-dry-period" | "use-rain-intensity-values" | "use-wind-intensity-values" | "use-audio-rain-level"
            | "use-water-unit-for-reflection" | "lerp-reflection-plane-of-water-units" | "rope-disable-character-collision" | "disable-platform-control-on-object"
            | "disable-gadgetry" | "enable-gadgetry" | "grenade-director-change-settings" | "journal-set-ambient-scale" | "update-ap-ref" | "set-e3-loading-screen"
            | "demo-reset-defaults" | "draw-reticle" | "sync-vox-mp-meta-data" | "disable-cover-camera" | "camera-set-default-follow"
            | "camera-aim-override-point" | "camera-cancel-aim-override" | "camera-store-locator" | "camera-restore-locator" => {
                self.inner.game.flags.insert(sid(name), arg(0));
                ok(0)
            }
            // The clock multiplier and slow motion (`sub_15F824` time-scale
            // settings) are recorded; the host clock is not scaled.
            "set-clock-multiplier" | "slow-mo-effect" => {
                self.inner.game.flags.insert(sid(name), f(0, 1.).to_bits());
                ok(0)
            }
            "set-character-wet-mask-state" => {
                if n < 2 {
                    self.inner.error("set-character-wet-mask-state: Expected two arguments (int32 int32)".into());
                    return ok(0);
                }
                self.inner.game.wet_mask = a[1];
                ok(0)
            }
            // sub_13C5E0: begin the companion immediately; the signed
            // second argument is retained by the movie synchronizer.
            "start-movie-audio" => {
                ensure!(n == 2, "start-movie-audio expects a filename and sync offset");
                self.inner.game.movie_sound = Some((dc.mem.cstr(a[0])?, a[1] as i32));
                ok(0)
            }
            // sub_13BA68 looks up a music-settings record; it returns zero.
            "play-music-stinger" => {
                if n > 0 { self.inner.game.stingers.push(a[0]); }
                ok(0)
            }
            // (play-sound-attached name object joint [volume] [pitch]).
            "play-sound-attached" => {
                if n < 2 {
                    self.inner.error("play-sound-attached: Expected (sound-name string) (attach-object …)".into());
                    return ok(0);
                }
                if self.world.locator(a[1]).is_none() {
                    self.inner.error(format!("play-sound-attached: Object '{:#010x}' was not found, or is not a ProcessGameObject.", a[1]));
                    return ok(0);
                }
                let at = super::sound_natives::SoundAt::Joint { object: a[1], joint: arg(2) };
                ok(self.world.play_sound(a[0], at, f(3, 1.), f(4, 0.)))
            }
            "play-movie" => {
                self.world.print(&format!("play-movie {:#010x}: not played from scripts", arg(0)));
                ok(0)
            }
            // (despawn-message id).
            "despawn-message" => {
                self.inner.game.messages.retain(|m| m.0 != arg(0));
                self.world.hide_message(arg(0));
                ok(0)
            }
            "display-message" => {
                self.world.message(arg(0));
                ok(0)
            }
            // (task-failed!): the task restarts from its checkpoint.
            "task-failed!" => {
                self.inner.task_change = self.inner.task;
                ok(0)
            }
            // (wait-player-move-to point …) / (wait-player-turn-to point):
            // the player turns there; moving is a turn and a jump-free
            // placement (not the native walk).
            "wait-player-move-to" | "wait-player-turn-to" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                let p = vec3(dc, a[0])?;
                self.world.player_request(super::player_natives::PlayerRequest::TurnTo(p));
                if name == "wait-player-move-to" {
                    if let Some((_, q)) = self.world.locator(sid("player")) {
                        self.world.place(sid("player"), p, q);
                    }
                }
                ok(0)
            }
            "wait-until-net-time" => {
                self.inner.error("wait-until-net-time: You must specify is-net-timer in the options block if you want to use this function".into());
                ok(0)
            }
            // Debug output.
            "print-event" => {
                let event = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == super::KIND_EVENT)).map(|g| (g.event, g.sender));
                let text = match event {
                    Some((e, Some(s))) => format!("Event  :: {e:#010x} / Sender :: {s:#010x}"),
                    Some((e, None)) => format!("Event  :: {e:#010x} / Sender :: NO SENDER?!?"),
                    None => "Event  :: none".into(),
                };
                self.world.print(&text);
                ok(0)
            }
            "printf" => {
                let text = if n > 0 && a[0] != 0 { dc.mem.cstr(a[0]).unwrap_or_default() } else { String::new() };
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            "print-point" => {
                if n > 0 && a[0] != 0 {
                    let p = vec3(dc, a[0])?;
                    self.world.print(&format!("print-point: ({:4.2}, {:4.2}, {:4.2})", p[0], p[1], p[2]));
                }
                ok(0)
            }
            "debug-break" => {
                self.world.print("*** debug-break ***");
                ok(0)
            }
            // Journal and tutorial state.
            "journal-get-page" => ok(self.inner.game.journal_page.max(0) as u32 + u32::from(self.inner.game.journal_page >= 0)),
            "journal-open?" => ok(0),
            "tutorial-hints-on?" => ok(1),
            // Vectors and locators.
            "make-vector" => {
                let at = dc.mem.alloc(16)?;
                for (k, v) in [f(0, 0.), f(1, 0.), f(2, 0.), f(3, 1.)].iter().enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // (pow-float x): `sub_ABB33C`, two to the power x (the exposure
            // code's exp2).
            "pow-float" => okf(f(0, 0.).exp2()),
            // (units-per-frame v): per-second units scaled to a frame.
            "units-per-frame" => okf(f(0, 0.) * self.world.frame_seconds()),
            "get-origin-locator" => ok(super::rail_natives::alloc_frame(dc, [0.; 3], [0., 0., 0., 1.])?),
            "reverse-locator" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("reverse-locator: Expected 1 argument: (locator)".into());
                    return ok(0);
                }
                let (p, q) = Self::frame(dc, a[0])?;
                let r = glam::Quat::from_array(q).normalize() * glam::Quat::from_rotation_y(std::f32::consts::PI);
                ok(super::rail_natives::alloc_frame(dc, p, r.to_array())?)
            }
            // (rotate-locator-lookat locator point [weight]): turned toward
            // the point about +Y, by the weight.
            "rotate-locator-lookat" => {
                if n < 2 || a[0] == 0 || a[1] == 0 {
                    self.inner.error("rotate-locator-lookat: not enough arguments".into());
                    return ok(0);
                }
                let (p, q) = Self::frame(dc, a[0])?;
                let t = vec3(dc, a[1])?;
                let d = glam::Vec3::from_array(t) - glam::Vec3::from_array(p);
                let look = glam::Quat::from_rotation_y(d.x.atan2(d.z));
                let q = glam::Quat::from_array(q).normalize().slerp(look, f(2, 1.).clamp(0., 1.));
                ok(super::rail_natives::alloc_frame(dc, p, q.to_array())?)
            }
            // (get-object-offset-locator object offset): the object frame
            // moved by the offset in its own axes.
            "get-object-offset-locator" => {
                if n < 2 || a[1] == 0 {
                    self.inner.error("get-object-offset-locator: Expected 2 arguments: (spawner-name string) (offset vector)".into());
                    return ok(0);
                }
                let Some((p, q)) = self.world.locator(a[0]) else { return ok(0) };
                let off = glam::Quat::from_array(q).normalize() * glam::Vec3::from_array(vec3(dc, a[1])?);
                ok(super::rail_natives::alloc_frame(dc, (glam::Vec3::from_array(p) + off).to_array(), q)?)
            }
            // (get-align-locator object anim …) / (get-spawner-locator-
            // projected spawner …): the object's frame (the clip alignment
            // and the ground projection are not applied).
            "get-align-locator" | "get-spawner-locator-projected" => match self.world.locator(arg(0)) {
                Some((p, q)) => ok(super::rail_natives::alloc_frame(dc, p, q)?),
                None => {
                    self.inner.error(format!("{name}: Unable to find spawner '{:#010x}'", arg(0)));
                    ok(0)
                }
            },
            // (facing-point? frame frame): the first frame's forward points
            // toward the second's position.
            "facing-point?" => {
                if n != 2 || a[0] == 0 || a[1] == 0 {
                    return ok(0);
                }
                let (p, q) = Self::frame(dc, a[0])?;
                let (t, _) = Self::frame(dc, a[1])?;
                let fwd = glam::Quat::from_array(q).normalize() * glam::Vec3::Z;
                ok(u32::from((glam::Vec3::from_array(t) - glam::Vec3::from_array(p)).dot(fwd) > 0.))
            }
            "camera-locator" => match self.world.camera_frame() {
                Some((p, q)) => ok(super::rail_natives::alloc_frame(dc, p, q)?),
                None => ok(0),
            },
            "is-camera-stick-pressed" => ok(u32::from(self.world.sticks(0)[2..].iter().any(|v| v.abs() > 0.3))),
            "get-water-offset" => {
                let at = dc.mem.alloc(16)?;
                ok(at)
            }
            // (rail-fire-turret-grenade vehicle npc target): a grenade from
            // the vehicle to the target.
            "rail-fire-turret-grenade" => {
                let (Some((from, _)), Some((to, _))) = (self.world.locator(arg(0)), self.world.locator(arg(2))) else {
                    self.inner.error("rail-fire-turret-grenade: Unable to find target.".into());
                    return ok(0);
                };
                self.world.throw_grenade([from[0], from[1] + 2., from[2]], to);
                ok(1)
            }
            // Multiplayer and co-op natives: no network game in single
            // player (`sub_4466F4` false).
            "net-alive-player-count" => ok(1),
            "net-character-killed?" => ok(u32::from(self.world.npc_killed(arg(0)))),
            "net-clear-npc-fight-owner" | "net-coop-team-failed?" | "net-end-wave" | "net-get-booster-amount" | "net-get-capture-zone" | "net-hud-set-grapple-indicator" | "net-is-cinema?" | "net-safety-npc-fight-damage" | "net-spawn-flag" | "net-update-fight-ref" => ok(0),
            _ => Ok(None),
        }
    }
}

/// Flashlight light handles (an object and its flashlight index).
fn flashlight_id(object: u32, index: u32) -> u32 {
    object.rotate_left(5) ^ 0x464c_0000 ^ index
}
const FLASHLIGHT_RADIUS: f32 = 15.;
const FLASHLIGHT_CONE: f32 = 30.;
