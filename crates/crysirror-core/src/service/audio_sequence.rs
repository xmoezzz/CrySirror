// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free cue scheduling contract. Cues name declarations, never host paths.
use alloc::{string::String, vec::Vec};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSequenceMode {
    #[default]
    Once,
    Loop,
    Weighted,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSequenceClock {
    #[default]
    Game,
    Real,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSequenceTiming {
    /// Each emitted cue starts a new interval; discard overdue frame time.
    #[default]
    Reset,
    /// Keep overdue time so positive intervals can catch up within the budget.
    Carry,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioSequenceRandom {
    /// Rejection sampling over the generator's nonzero 32-bit period.
    #[default]
    Uniform,
    /// Exact 32-bit modulo draws for native formats that require that sequence.
    Modulo32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSequenceLine {
    pub key: String,
    pub weight: u32,
    /// Bounds may be reversed; sampling uses [min,max), or the exact value.
    pub min_ms: i64,
    pub max_ms: i64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSequenceSpec {
    pub mode: AudioSequenceMode,
    pub clock: AudioSequenceClock,
    pub timing: AudioSequenceTiming,
    pub random: AudioSequenceRandom,
    /// Weighted mode halves the preceding line's relative weight, not a ban.
    pub soften_repeat: bool,
    /// Add the acknowledged source duration to each sampled interval.
    pub include_duration: bool,
    pub lines: Vec<AudioSequenceLine>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSequenceHandle {
    pub client: u32,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSequenceEmission {
    pub serial: u64,
    pub index: usize,
    pub key: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSequenceInfo {
    pub active: bool,
    pub elapsed_ms: u64,
    pub next_ms: i64,
    pub last_index: Option<usize>,
    pub rng_state: u32,
    pub emissions: u64,
    pub pending: Option<AudioSequenceEmission>,
    /// True after this owner-frame's 1024 emissions; advance opens a new budget.
    pub budget_exhausted: bool,
}
/// Portable scheduling state only; a dependency must be acknowledged first.
/// Restoring creates a fresh identity, never revives worker/device handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioSequenceCursor {
    pub active: bool,
    pub elapsed_ms: u64,
    pub next_ms: i64,
    pub last_index: Option<usize>,
    pub rng_state: u32,
    pub emissions: u64,
    pub frame_emissions: u32,
    pub budget_exhausted: bool,
}

/// Pure admission validation is shared with native snapshot staging, without
/// importing an audio implementation into a VM.
impl AudioSequenceSpec {
    pub fn validate(&self) -> crate::Result<()> {
        use crate::Error;
        if self.lines.len() > 1024 {
            return Err(Error::Invalid("cue sequence exceeds 1024 lines"));
        }
        let mut bytes = 0;
        for l in &self.lines {
            if l.key.is_empty() || l.key.len() > 2048 || l.key.chars().any(char::is_control) {
                return Err(Error::Invalid("cue key"));
            }
            if l.min_ms.abs_diff(0) > 604800000 || l.max_ms.abs_diff(0) > 604800000 {
                return Err(Error::Invalid("cue interval exceeds seven days"));
            }
            bytes += l.key.len();
        }
        if bytes > 65536 {
            return Err(Error::Invalid("cue keys exceed 64 KiB"));
        }
        let total: u64 = self.lines.iter().map(|l| u64::from(l.weight)).sum();
        if self.random == AudioSequenceRandom::Uniform
            && self.mode == AudioSequenceMode::Weighted
            && total * if self.soften_repeat { 2 } else { 1 } > u64::from(u32::MAX)
        {
            return Err(Error::Invalid(
                "uniform cue weight total exceeds 32-bit period",
            ));
        }
        Ok(())
    }
}
impl AudioSequenceCursor {
    pub fn validate_against(&self, spec: &AudioSequenceSpec) -> crate::Result<()> {
        use crate::Error;
        spec.validate()?;
        if self.elapsed_ms > i64::MAX as u64 || self.next_ms.abs_diff(0) > 1209600000 {
            return Err(Error::Invalid("cue cursor time"));
        }
        if self.last_index.is_some_and(|i| i >= spec.lines.len()) {
            return Err(Error::Invalid("cue cursor line"));
        }
        if self.last_index.is_some() != (self.emissions > 0) {
            return Err(Error::Invalid("cue cursor emissions"));
        }
        if self.frame_emissions > 1024
            || u64::from(self.frame_emissions) > self.emissions
            || (self.budget_exhausted && self.frame_emissions != 1024)
        {
            return Err(Error::Invalid("cue cursor budget"));
        }
        if self.active
            && spec.mode == AudioSequenceMode::Once
            && self.last_index.is_some_and(|i| i + 1 == spec.lines.len())
            && self.elapsed_ms as i64 >= self.next_ms
        {
            return Err(Error::Invalid("completed once cursor cannot be active"));
        }
        Ok(())
    }
}
