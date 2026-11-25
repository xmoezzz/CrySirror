// SPDX-License-Identifier: MPL-2.0
//! Dialogue script natives (`say`, `wait-say`, `stop-speaking`,
//! `is-speaking`, `say-pre-roll`); see `crate::vox`.
use super::{Ctx, Wait};
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

type Reply = Option<(u32, Status)>;

impl Ctx<'_> {
    fn speaker(&self, name: u32) -> u32 {
        let name = if name == sid("self") { self.proc.map_or(name, |p| self.inner.procs[p].name) } else { name };
        self.inner.process(name).and_then(|p| self.inner.procs[p].spawner).unwrap_or(name)
    }

    pub(super) fn sound_native(&mut self, _dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        match name {
            // (say object vox [distance] [distance] …): the object's vox
            // controller plays the line (`sub_14E790`, event 0x1300F59C);
            // 1 when it plays. (wait-say …) also waits for it to end, or one
            // second when nothing played (`sub_14EBD8`).
            "say" | "wait-say" => {
                let wait = name == "wait-say";
                if wait && self.proc.is_none() {
                    return ok(0);
                }
                let played = if n <= 1 {
                    self.inner.error("say: Expected at least two arguments (object-name vox-name).".into());
                    false
                } else if a[0] == 0 {
                    self.inner.error("say: NULL or empty object name.".into());
                    false
                } else if a[1] == 0 {
                    self.inner.error("say: NULL or empty vox name.".into());
                    false
                } else {
                    let who = self.speaker(a[0]);
                    if self.world.locator(who).is_none() {
                        self.inner.error(format!("say: Could not find object {who:#010x}."));
                        false
                    } else {
                        self.world.say(who, a[1])
                    }
                };
                if !wait {
                    return ok(u32::from(played));
                }
                if !played {
                    self.inner.error(format!("WARNING: Vox {:#010x} did not play -- waiting one second to simulate time passing...", if n > 1 { a[1] } else { 0 }));
                    let until = self.world.now() + 1.;
                    return Ok(Some(self.suspend(Wait::Until(until))));
                }
                let who = self.speaker(a[0]);
                let (_, status) = self.suspend(Wait::Speech(who));
                Ok(Some((1, status)))
            }
            "stop-speaking" | "is-speaking" => {
                if n == 0 {
                    self.inner.error(format!("{name}: Expected one argument (object-name symbol)"));
                    return ok(0);
                }
                if a[0] == 0 {
                    self.inner.error(format!("{name}: NULL or empty object name"));
                    return ok(0);
                }
                let who = self.speaker(a[0]);
                if self.world.locator(who).is_none() {
                    self.inner.error(format!("{name}: Unable to find object {who:#010x}"));
                    return ok(0);
                }
                if name == "is-speaking" {
                    return ok(u32::from(self.world.speaking(who)));
                }
                self.world.stop_speaking(who);
                ok(1)
            }
            // (say-pre-roll spawner vox): start streaming the line ahead
            // (`sub_32B5A8`); lines are read when played here.
            "say-pre-roll" => {
                if n != 2 {
                    self.inner.error("say-pre-roll: Expected two arguments (spawner vox-id)".into());
                    return ok(0);
                }
                ok(1)
            }
            _ => Ok(None),
        }
    }
}
