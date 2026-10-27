// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded owner-clock cue programs. Selection, timing and dependency receipts
//! are independent of decoders, channel transport, VMs and output devices.
use anyhow::{Context, Result, ensure};
pub use crysirror_core::service::{
    AudioSequenceClock as Clock, AudioSequenceEmission as Emission, AudioSequenceHandle as Handle,
    AudioSequenceInfo as Info, AudioSequenceLine as Line, AudioSequenceMode as Mode,
    AudioSequenceRandom as Random, AudioSequenceSpec as Spec, AudioSequenceTiming as Timing,
};
use std::collections::BTreeMap;

const INTERVAL_LIMIT: i64 = 7 * 24 * 60 * 60 * 1000;
const FRAME_BUDGET: u32 = 1024;
const MAX_SEQUENCES: usize = 1024;
const MAX_CLIENT_SEQUENCES: usize = 128;

fn random(state: &mut u32) -> u32 {
    let mut x = if *state == 0 { 0x1234_5678 } else { *state };
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}
fn sample(state: &mut u32, bound: u64, mode: Random) -> u64 {
    if mode == Random::Modulo32 {
        return u64::from(random(state)) % bound;
    }
    // Xorshift32's period excludes zero: map 1..=MAX to 0..MAX before
    // rejecting the incomplete last bucket. Bounds were validated at create.
    let limit = u64::from(u32::MAX) - u64::from(u32::MAX) % bound;
    loop {
        let value = u64::from(random(state)) - 1;
        if value < limit {
            return value % bound;
        }
    }
}
fn interval(state: &mut u32, a: i64, b: i64, mode: Random) -> i64 {
    let lo = a.min(b);
    let hi = a.max(b);
    if hi == lo {
        lo
    } else {
        lo + sample(state, (hi - lo) as u64, mode) as i64
    }
}
struct Sequence {
    spec: Spec,
    info: Info,
    frame_emissions: u32,
}
impl Sequence {
    fn finish_if_due(&mut self) {
        if self.info.active
            && self.info.pending.is_none()
            && self.spec.mode == Mode::Once
            && self
                .info
                .last_index
                .is_some_and(|i| i + 1 == self.spec.lines.len())
            && self.info.elapsed_ms as i64 >= self.info.next_ms
        {
            self.info.active = false;
        }
    }
    fn select(&mut self) -> usize {
        match self.spec.mode {
            Mode::Once => self.info.last_index.map_or(0, |i| i + 1),
            Mode::Loop => self
                .info
                .last_index
                .map_or(0, |i| (i + 1) % self.spec.lines.len()),
            Mode::Weighted => {
                let weight = |i: usize, line: &Line| {
                    u64::from(line.weight)
                        * if self.spec.soften_repeat && Some(i) != self.info.last_index {
                            2
                        } else {
                            1
                        }
                };
                let total: u64 = self
                    .spec
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, l)| weight(i, l))
                    .sum();
                if total == 0 {
                    return 0;
                }
                let mut at = sample(&mut self.info.rng_state, total, self.spec.random);
                for (i, line) in self.spec.lines.iter().enumerate() {
                    let w = weight(i, line);
                    if at < w {
                        return i;
                    }
                    at -= w;
                }
                unreachable!("validated nonempty weighted distribution")
            }
        }
    }
}

/// Host-owned identities survive clear as tombstones: a stale handle can never
/// alias a newly created program. Mutations run at the owner control boundary;
/// worker dependencies return an emission serial rather than borrowing state.
#[derive(Default)]
pub struct Sequences {
    next: u64,
    entries: BTreeMap<(u32, u64), Sequence>,
}
impl Sequences {
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn clear_client(&mut self, client: u32) {
        self.entries.retain(|k, _| k.0 != client);
    }
    pub fn create(&mut self, client: u32, spec: Spec, seed: u32) -> Result<Handle> {
        ensure!(client != 0, "cue sequence requires a client");
        ensure!(spec.lines.len() <= 1024, "cue sequence exceeds 1024 lines");
        let mut bytes = 0;
        for line in &spec.lines {
            ensure!(
                !line.key.is_empty()
                    && line.key.len() <= 2048
                    && !line.key.chars().any(char::is_control),
                "invalid cue key"
            );
            bytes += line.key.len();
            ensure!(
                line.min_ms.abs_diff(0) <= INTERVAL_LIMIT as u64
                    && line.max_ms.abs_diff(0) <= INTERVAL_LIMIT as u64,
                "cue interval exceeds seven days"
            );
        }
        ensure!(bytes <= 64 * 1024, "cue keys exceed 64 KiB");
        if spec.random == Random::Uniform && spec.mode == Mode::Weighted {
            let total: u64 = spec.lines.iter().map(|l| u64::from(l.weight)).sum();
            ensure!(
                total * if spec.soften_repeat { 2 } else { 1 } <= u64::from(u32::MAX),
                "uniform cue weight total exceeds 32-bit period"
            );
        }
        ensure!(
            self.entries.len() < MAX_SEQUENCES
                && self.entries.keys().filter(|k| k.0 == client).count() < MAX_CLIENT_SEQUENCES,
            "cue sequence capacity"
        );
        let generation = self
            .next
            .checked_add(1)
            .context("cue generation exhausted")?;
        self.next = generation;
        let info = Info {
            active: true,
            elapsed_ms: 0,
            next_ms: 0,
            last_index: None,
            rng_state: seed,
            emissions: 0,
            pending: None,
            budget_exhausted: false,
        };
        self.entries.insert(
            (client, generation),
            Sequence {
                spec,
                info,
                frame_emissions: 0,
            },
        );
        Ok(Handle { client, generation })
    }
    fn get(&self, client: u32, h: Handle) -> Result<&Sequence> {
        ensure!(client == h.client, "foreign cue sequence");
        self.entries
            .get(&(client, h.generation))
            .context("stale cue sequence")
    }
    fn get_mut(&mut self, client: u32, h: Handle) -> Result<&mut Sequence> {
        self.get(client, h)?;
        Ok(self.entries.get_mut(&(client, h.generation)).unwrap())
    }
    pub fn info(&self, client: u32, h: Handle) -> Result<Info> {
        Ok(self.get(client, h)?.info.clone())
    }
    pub fn release(&mut self, client: u32, h: Handle) -> Result<()> {
        self.get(client, h)?;
        self.entries.remove(&(client, h.generation));
        Ok(())
    }
    pub fn advance(&mut self, client: u32, h: Handle, game_ms: u64, real_ms: u64) -> Result<()> {
        ensure!(
            game_ms <= 60000 && real_ms <= 60000,
            "cue frame exceeds 60 seconds"
        );
        let s = self.get_mut(client, h)?;
        s.frame_emissions = 0;
        s.info.budget_exhausted = false;
        if s.info.active && s.info.pending.is_none() {
            let dt = if s.spec.clock == Clock::Game {
                game_ms
            } else {
                real_ms
            };
            s.info.elapsed_ms = s.info.elapsed_ms.saturating_add(dt).min(i64::MAX as u64);
            s.finish_if_due();
        }
        Ok(())
    }
    /// A pending token is idempotent. New selection commits RNG and last-index
    /// once; no repeated poll consumes random values or frame budget.
    pub fn poll(&mut self, client: u32, h: Handle, seed: Option<u32>) -> Result<Option<Emission>> {
        let s = self.get_mut(client, h)?;
        if s.info.pending.is_some() {
            return Ok(s.info.pending.clone());
        }
        if !s.info.active || (s.info.elapsed_ms as i64) < s.info.next_ms {
            return Ok(None);
        }
        if s.spec.lines.is_empty() {
            s.info.active = false;
            return Ok(None);
        }
        if s.frame_emissions == FRAME_BUDGET {
            s.info.budget_exhausted = true;
            return Ok(None);
        }
        if let Some(seed) = seed {
            s.info.rng_state = seed;
        }
        let index = s.select();
        let serial = s
            .info
            .emissions
            .checked_add(1)
            .context("cue emission serial exhausted")?;
        let emission = Emission {
            serial,
            index,
            key: s.spec.lines[index].key.clone(),
        };
        if s.spec.timing == Timing::Reset {
            s.info.elapsed_ms = 0;
        } else {
            s.info.elapsed_ms = s
                .info
                .elapsed_ms
                .saturating_sub(s.info.next_ms.max(0) as u64);
        }
        s.info.last_index = Some(index);
        s.info.emissions = serial;
        s.info.pending = Some(emission.clone());
        s.frame_emissions += 1;
        Ok(Some(emission))
    }
    /// Complete only after successful resource/voice publication (or an
    /// explicit failed-cue policy). Invalid/stale receipts do not change state.
    pub fn complete(
        &mut self,
        client: u32,
        h: Handle,
        serial: u64,
        duration_ms: u64,
    ) -> Result<()> {
        ensure!(
            duration_ms <= INTERVAL_LIMIT as u64,
            "cue duration exceeds seven days"
        );
        let s = self.get_mut(client, h)?;
        let pending = s
            .info
            .pending
            .as_ref()
            .context("cue has no pending emission")?;
        ensure!(pending.serial == serial, "stale cue emission");
        let line = &s.spec.lines[pending.index];
        let delay = interval(
            &mut s.info.rng_state,
            line.min_ms,
            line.max_ms,
            s.spec.random,
        );
        s.info.next_ms = delay
            + if s.spec.include_duration {
                duration_ms as i64
            } else {
                0
            };
        s.info.pending = None;
        s.finish_if_due();
        Ok(())
    }
}
