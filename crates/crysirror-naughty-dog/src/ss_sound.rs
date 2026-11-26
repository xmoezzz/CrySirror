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

    pub(super) fn sound_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        match name {
            // (say object vox [distance] [distance] …): the object's vox
            // controller plays the line (`sub_14E790`, event 0x13011B9C);
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
                    } else if self.inner.game.muted.iter().any(|(o, until)| *o == who && *until > self.world.now()) {
                        // Muted by `shut-up`.
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
            // (play-sound name [volume 1] [pitch 0]),
            // (play-sound-at-position name point [volume] [pitch]),
            // (play-sound-at-joint name object joint [volume] [pitch]): a
            // sound process (`sub_138650`); its id, 0 when none started.
            "play-sound" | "play-sound-at-position" | "play-sound-at-joint" => {
                let (at, first) = match name {
                    "play-sound" => (Some(SoundAt::Listener), 1),
                    "play-sound-at-position" => {
                        let at = (n > 1 && a[1] != 0).then(|| -> Result<_> { Ok(SoundAt::Point([dc.mem.f32(a[1])?, dc.mem.f32(a[1] + 4)?, dc.mem.f32(a[1] + 8)?])) }).transpose()?;
                        (at, 2)
                    }
                    _ => {
                        let object = self.speaker(if n > 1 { a[1] } else { sid("self") });
                        let joint = if n > 2 { a[2] } else { 0 };
                        if n > 0 && self.world.locator(object).is_none() {
                            self.inner.error(format!("play-sound-at-joint: Object {object:#010x} was not found, or is not a ProcessGameObject."));
                            return ok(0);
                        }
                        (Some(SoundAt::Joint { object, joint }), 3)
                    }
                };
                let id = if n > 0 { a[0] } else { 0 };
                let (Some(at), true) = (at, id != 0) else {
                    self.inner.error(format!("{name}: Expected (sound-name string) …"));
                    return ok(0);
                };
                let volume = if n > first { f32::from_bits(a[first]) } else { 1. };
                let pitch = if n > first + 1 { f32::from_bits(a[first + 1]) } else { 0. };
                ok(self.world.play_sound(id, at, volume, pitch))
            }
            // (stop-sound id): the sound process ends (`sub_7092F0`,
            // `sub_7095F0`).
            "stop-sound" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("stop-sound: Invalid sound-id".into());
                    return ok(0);
                }
                self.world.stop_sound(a[0]);
                ok(0)
            }
            // (cache-sound-stream name slot): prefetch a stream
            // (`sub_2FA2E0`); streams are read when played here.
            "cache-sound-stream" => ok(0),
            // (sync-audio-meta-data) / (sync-vox-meta-data): reload the
            // audio and vox tables after a module load (`sub_3037B8`,
            // `sub_32FD08`); the tables load with the game here.
            "sync-audio-meta-data" | "sync-vox-meta-data" => ok(0),
            // (set-mixer-override mixer): the named mixer snapshot
            // (`sub_2F04B8`).
            "set-mixer-override" => {
                if n > 0 {
                    self.world.set_mixer(a[0]);
                }
                ok(0)
            }
            _ => Ok(None),
        }
    }
}

/// Where a sound plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundAt {
    /// Unpositioned (stereo).
    Listener,
    Point([f32; 3]),
    Joint { object: u32, joint: u32 },
}
