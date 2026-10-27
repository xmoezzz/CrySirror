// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Script/data declarations over the host-owned, device-free cue scheduler.
use crate::{EngineContext, Value};
use anyhow::{Context, Result, bail};
use crysirror_core::service::{self as core, HostServices};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueSequenceMode {
    #[default]
    Once,
    Loop,
    Weighted,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueSequenceClock {
    #[default]
    Game,
    Real,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueSequenceTiming {
    #[default]
    Reset,
    Carry,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CueSequenceRandom {
    #[default]
    Uniform,
    Modulo32,
}
fn weight() -> u32 {
    1
}
/// A symbolic cue. The caller maps the emitted key to an engine resource.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueSequenceLine {
    /// Nonempty logical key, up to 2048 UTF-8 bytes; no filesystem authority.
    pub key: String,
    /// Nonnegative relative weight. All-zero weighted lists select line zero.
    #[serde(default = "weight")]
    pub weight: u32,
    /// Signed millisecond interval bounds; reversed bounds are accepted.
    #[serde(default)]
    pub min_ms: i64,
    /// Upper bound is exclusive unless equal to min_ms. Each bound <= 7 days.
    #[serde(default)]
    pub max_ms: i64,
}
/// Owner-clock cue program; it does not open or play resources by itself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CueSequenceSpec {
    /// once, loop, or weighted. A once program completes after its last interval.
    pub mode: CueSequenceMode,
    /// Clock selected from each explicit advanceCueSequence(game_ms, real_ms).
    pub clock: CueSequenceClock,
    /// reset discards late time at emission; carry retains positive overdue time.
    pub timing: CueSequenceTiming,
    /// uniform rejection sampling; modulo32 preserves legacy native RNG draws.
    pub random: CueSequenceRandom,
    /// Weighted mode halves the last selected line's relative weight.
    pub soften_repeat: bool,
    /// Add acknowledged decoded-source duration to each sampled interval.
    pub include_duration: bool,
    /// At most 1024 lines and 64 KiB of key bytes. Empty completes on first poll.
    pub lines: Vec<CueSequenceLine>,
}
impl From<CueSequenceSpec> for core::AudioSequenceSpec {
    fn from(s: CueSequenceSpec) -> Self {
        Self {
            mode: match s.mode {
                CueSequenceMode::Once => core::AudioSequenceMode::Once,
                CueSequenceMode::Loop => core::AudioSequenceMode::Loop,
                CueSequenceMode::Weighted => core::AudioSequenceMode::Weighted,
            },
            clock: match s.clock {
                CueSequenceClock::Game => core::AudioSequenceClock::Game,
                CueSequenceClock::Real => core::AudioSequenceClock::Real,
            },
            timing: match s.timing {
                CueSequenceTiming::Reset => core::AudioSequenceTiming::Reset,
                CueSequenceTiming::Carry => core::AudioSequenceTiming::Carry,
            },
            random: match s.random {
                CueSequenceRandom::Uniform => core::AudioSequenceRandom::Uniform,
                CueSequenceRandom::Modulo32 => core::AudioSequenceRandom::Modulo32,
            },
            soften_repeat: s.soften_repeat,
            include_duration: s.include_duration,
            lines: s
                .lines
                .into_iter()
                .map(|l| core::AudioSequenceLine {
                    key: l.key,
                    weight: l.weight,
                    min_ms: l.min_ms,
                    max_ms: l.max_ms,
                })
                .collect(),
        }
    }
}
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("cue sequence argument required")
}
fn integer(args: &[Value], i: usize) -> Result<u64> {
    Ok(u64::try_from(arg(args, i)?.as_i64()?)?)
}
fn handle(value: &Value) -> Result<core::AudioSequenceHandle> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Handle {
        client: u32,
        generation: u64,
    }
    let h: Handle = serde_json::from_value(value.to_json())?;
    Ok(core::AudioSequenceHandle {
        client: h.client,
        generation: h.generation,
    })
}
fn emission(e: core::AudioSequenceEmission) -> serde_json::Value {
    json!({"serial":e.serial,"index":e.index,"key":e.key})
}
pub(crate) fn call(cx: &mut EngineContext, name: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        name,
        "createCueSequence"
            | "advanceCueSequence"
            | "pollCueSequence"
            | "completeCueSequence"
            | "cueSequence"
            | "releaseCueSequence"
    ) {
        return Ok(None);
    }
    let state = cx.state()?;
    let client = state.client;
    let host = &mut state.runtime.host;
    let result = if name == "createCueSequence" {
        let spec: CueSequenceSpec = serde_json::from_value(arg(args, 0)?.to_json())?;
        let seed = u32::try_from(integer(args, 1)?)?;
        let h = host
            .create_audio_sequence(client, spec.into(), seed)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        json!({"client":h.client,"generation":h.generation})
    } else {
        let h = handle(arg(args, 0)?)?;
        match name {
            "advanceCueSequence" => {
                host.advance_audio_sequence(client, h, integer(args, 1)?, integer(args, 2)?)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!(null)
            }
            "pollCueSequence" => {
                #[derive(Default, Deserialize)]
                #[serde(default, deny_unknown_fields)]
                struct Options {
                    seed: Option<u32>,
                }
                let options: Options = serde_json::from_value(arg(args, 1)?.to_json())?;
                host.poll_audio_sequence(client, h, options.seed)
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                    .map(emission)
                    .unwrap_or(json!(null))
            }
            "completeCueSequence" => {
                host.complete_audio_sequence(client, h, integer(args, 1)?, integer(args, 2)?)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!(null)
            }
            "cueSequence" => {
                let i = host
                    .audio_sequence_info(client, h)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!({"active":i.active,"elapsed_ms":i.elapsed_ms,"next_ms":i.next_ms,"last_index":i.last_index,"rng_state":i.rng_state,"emissions":i.emissions,"pending":i.pending.map(emission),"budget_exhausted":i.budget_exhausted})
            }
            "releaseCueSequence" => {
                host.release_audio_sequence(client, h)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!(null)
            }
            _ => bail!("unknown cue sequence operation"),
        }
    };
    Ok(Some(Value::from_json(result)))
}
