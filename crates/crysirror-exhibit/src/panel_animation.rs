// SPDX-License-Identifier: MPL-2.0
//! RLD v1 fixed-list tmPrepareFade and tmPrepareMove forms.
use crate::{branch::number, select_items::Panels};
use anyhow::{Context, Result, ensure};
use crysirror_core::{service::HostServices, state::ClientId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    Fade,
    Move,
}
#[derive(Debug, PartialEq)]
struct Target {
    slot: i32,
    value: [i32; 2],
    duration: u32,
    mode: i32,
}
#[derive(Debug, PartialEq)]
pub(super) struct Spec {
    kind: Kind,
    channel: i32,
    asynchronous: bool,
    relative: bool,
    delay: u32,
    targets: Vec<Target>,
}
impl Spec {
    pub fn parse(kind: Kind, raw: &[u8]) -> Result<Self> {
        let n = raw
            .split(|b| *b == b',')
            .map(number)
            .collect::<Result<Vec<_>>>()?;
        let (start, asynchronous, relative) = match kind {
            Kind::Fade => {
                ensure!(n.len() == 34, "unsupported ExHibit fade definition");
                (2, n[1] != 0, false)
            }
            Kind::Move => {
                ensure!(
                    n.len() == 28 && n[1] >= 0,
                    "unsupported ExHibit move sequence"
                );
                (4, n[2] != 0, n[3] != 0)
            }
        };
        ensure!(
            n[0] >= 0 && n[0] < if kind == Kind::Fade { 10 } else { 7 },
            "unsupported ExHibit animation channel"
        );
        let mut delay = 0;
        let mut targets = Vec::new();
        for f in n[start..].chunks_exact(4) {
            if f[0] < 0 {
                continue;
            }
            let (value, ms, mode) = match kind {
                Kind::Fade => {
                    ensure!(
                        (0..=16).contains(&f[1]) && f[2] >= 0,
                        "unsupported ExHibit fade level or duration"
                    );
                    if f[3] > 0 {
                        delay = f[3] as u32;
                    }
                    ([f[1], 0], f[2] as u32, 0)
                }
                Kind::Move => {
                    ensure!(
                        (0..=3).contains(&f[3]),
                        "unsupported ExHibit move interpolation"
                    );
                    ensure!(
                        f[3] != 1 || n[1] == 0 || (n[1] as u32).wrapping_pow(2) != 0,
                        "invalid ExHibit quadratic move duration"
                    );
                    ([f[1], f[2]], n[1] as u32, f[3])
                }
            };
            targets.push(Target {
                slot: f[0],
                value,
                duration: ms,
                mode,
            });
        }
        Ok(Self {
            kind,
            channel: n[0],
            asynchronous,
            relative,
            delay,
            targets,
        })
    }
}

/// TmAsyncFade samples levels in unsigned 32-bit arithmetic, including descent.
pub(super) fn fade_level(from: i32, to: i32, duration: u32, age: u32) -> i32 {
    if age >= duration {
        return to;
    }
    if age == 0 {
        return from;
    }
    (duration.wrapping_mul(from as u32)
        .wrapping_add(age.wrapping_mul(to.wrapping_sub(from) as u32)) / duration) as i32
}

struct Lane {
    slot: u8,
    from: [i32; 2],
    to: [i32; 2],
    ms: u32,
    mode: i32,
}
impl Lane {
    fn sample(&self, kind: Kind, age: u32) -> [i32; 2] {
        if self.ms == 0 || age >= self.ms {
            return self.to;
        }
        if age == 0 {
            return self.from;
        }
        std::array::from_fn(|i| {
            let delta = self.to[i].wrapping_sub(self.from[i]);
            if kind == Kind::Fade {
                return fade_level(self.from[i], self.to[i], self.ms, age);
            }
            let step = match self.mode {
                0 => i64::from(delta) * i64::from(age) / i64::from(self.ms),
                1 => {
                    i64::from(delta) * i64::from(age.wrapping_pow(2))
                        / i64::from(self.ms.wrapping_pow(2))
                }
                // In the v1 x86 dispatcher the mode-2 square root falls
                // through to the mode-3 logarithm, overwriting both coordinates.
                _ => (f64::from(delta) * f64::from(age).ln() / f64::from(self.ms).ln()) as i64,
            };
            self.from[i].wrapping_add(step as i32)
        })
    }
}
fn covered_empty_fade(lanes: &[Lane], duration: u32) -> bool {
    // The arithmetic proof needs no overflow in level * duration.
    duration <= u32::MAX / 16
        && lanes
            .iter()
            .any(|l| l.from == [16, 0] && l.to == [0, 0] && l.ms == duration)
}

struct Sequence {
    start: u64,
    delay: u32,
    lanes: Vec<Lane>,
    done: bool,
}
#[derive(Default)]
pub(super) struct Animations {
    channels: BTreeMap<(Kind, i32), Sequence>,
    waiting: Option<(Kind, i32)>,
}
impl Animations {
    pub fn replace_billboard_fade(&mut self, panels: &Panels, now: u64, slot: u8, level: u8, duration: u32, skip: bool) -> Result<()> {
        self.leave(Kind::Fade, 8, false);
        self.begin(Spec { kind: Kind::Fade, channel: 8, asynchronous: false, relative: false, delay: 0,
            targets: vec![Target { slot: i32::from(slot), value: [i32::from(level),0], duration, mode: 0 }],
        }, panels, now, skip)
    }

    pub fn begin(&mut self, spec: Spec, panels: &Panels, now: u64, skip: bool) -> Result<()> {
        let key = (spec.kind, spec.channel);
        ensure!(
            !self.channels.contains_key(&key),
            "unsupported ExHibit append to retained animation channel"
        );
        let mut lanes = Vec::new();
        let mut empty_fades = Vec::new();
        for target in spec.targets {
            let Some(slot) = panels.resolve_slot(target.slot) else {
                continue;
            };
            if spec.kind == Kind::Fade && !panels.is_bound(slot) {
                ensure!(
                    !spec.asynchronous && target.value == [0, 0],
                    "unsupported ExHibit fade of unbound slot {slot}"
                );
                empty_fades.push((slot, target.duration));
                continue;
            }
            let from = panels.values(slot, spec.kind)?;
            let to = if spec.relative {
                std::array::from_fn(|i| from[i].wrapping_add(target.value[i]))
            } else {
                target.value
            };
            ensure!(
                !lanes.iter().any(|l: &Lane| l.slot == slot)
                    && !self.channels.iter().any(|((k, _), s)| *k == spec.kind
                        && !s.done
                        && s.lanes.iter().any(|l| l.slot == slot)),
                "unsupported ExHibit overlapping animation lanes"
            );
            lanes.push(Lane {
                slot,
                from,
                to,
                ms: if skip { 0 } else { target.duration },
                mode: target.mode,
            });
        }
        for (slot, duration) in empty_fades {
            // The native inactive slot retains its descriptor level and cannot
            // paint (0x81090). Any level 0..=16 reaches zero no later than a
            // full-opacity bound lane of the same duration and channel delay
            // under the native integer interpolation (0xd7a0). Only admit
            // that synchronized case rather than guessing a dormant level.
            ensure!(
                covered_empty_fade(&lanes, if skip { 0 } else { duration }),
                "unsupported ExHibit fade completion for unbound slot {slot}"
            );
        }
        if !lanes.is_empty() {
            self.channels.insert(
                key,
                Sequence {
                    start: now,
                    delay: spec.delay,
                    lanes,
                    done: false,
                },
            );
            if !spec.asynchronous {
                self.waiting = Some(key);
            }
        }
        Ok(())
    }

    pub fn leave(&mut self, kind: Kind, channel: i32, wait: bool) {
        let key = (kind, channel);
        if wait && self.channels.get(&key).is_some_and(|s| !s.done) {
            self.waiting = Some(key);
        } else {
            self.channels.remove(&key);
        }
    }
    pub fn live(&self) -> bool {
        self.channels.values().any(|s| !s.done)
    }
    pub fn waiting(&self) -> bool {
        self.waiting.is_some()
    }
    pub fn tick(
        &mut self,
        panels: &mut Panels,
        h: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        for (&(kind, _), s) in &mut self.channels {
            if s.done {
                continue;
            }
            let Some(age) = now.saturating_sub(s.start).checked_sub(u64::from(s.delay)) else {
                continue;
            };
            let age = u32::try_from(age).unwrap_or(u32::MAX);
            let mut done = true;
            for l in &s.lanes {
                let value = l.sample(kind, age);
                panels.set_values(h, client, l.slot, kind, value)?;
                done &= if kind == Kind::Fade {
                    value == l.to
                } else {
                    l.ms == 0 || age > l.ms
                };
            }
            s.done = done;
        }
        if let Some(key) = self.waiting {
            if self
                .channels
                .get(&key)
                .context("missing ExHibit animation wait")?
                .done
            {
                self.channels.remove(&key);
                self.waiting = None;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_fade_requires_a_matching_bound_completion() {
        let mut bound = Lane { slot: 42, from: [16, 0], to: [0, 0], ms: 500, mode: 0 };
        assert!(covered_empty_fade(&[bound], 500));
        assert!(!covered_empty_fade(&[], 500));
        bound = Lane { slot: 42, from: [15, 0], to: [0, 0], ms: 500, mode: 0 };
        assert!(!covered_empty_fade(&[bound], 500));
        bound = Lane { slot: 42, from: [16, 0], to: [0, 0], ms: 499, mode: 0 };
        assert!(!covered_empty_fade(&[bound], 500));
        bound = Lane { slot: 42, from: [16, 0], to: [1, 0], ms: 500, mode: 0 };
        assert!(!covered_empty_fade(&[bound], 500));
    }
    #[test]
    fn original_fade_and_move_have_distinct_rounding() {
        let mut lane = Lane {
            slot: 42,
            from: [16, 0],
            to: [0, 0],
            ms: 800,
            mode: 0,
        };
        assert_eq!(lane.sample(Kind::Fade, 1), [15, 0]);
        assert_eq!(lane.sample(Kind::Fade, 400), [8, 0]);
        lane.from = [800, 460];
        lane.to = [338, 460];
        lane.mode = 2;
        assert_eq!(lane.sample(Kind::Move, 1), [800, 460]);
        assert_eq!(lane.sample(Kind::Move, 400), [386, 460]);
        assert_eq!(lane.sample(Kind::Move, 800), [338, 460]);
    }
}
