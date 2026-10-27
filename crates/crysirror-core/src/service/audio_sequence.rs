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
