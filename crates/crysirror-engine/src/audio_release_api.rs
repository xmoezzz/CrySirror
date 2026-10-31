// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Public data/script facade for the shared, owner-clock channel release actor.
use crate::{EngineContext, Value};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    service::{self as core, HostServices},
    state::ClientId,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
/// A live decoder transfer followed by a stepped logarithmic release. Updates
/// are explicit counters; this declaration does not interpret them as time.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceReleaseSpec {
    pub source_channel: u32,
    pub tail_channel: u32,
    pub threshold_db100: i32,
    pub target_above_db100: i32,
    pub target_below_db100: i32,
    pub steps: u32,
    #[serde(default)]
    pub pan: Option<f32>,
    #[serde(default)]
    pub playback_rate: Option<f32>,
    pub freeze_group_gain: bool,
    pub release_tail_when_source_missing: bool,
}
impl From<VoiceReleaseSpec> for core::AudioReleaseSpec {
    fn from(s: VoiceReleaseSpec) -> Self {
        Self {
            source_channel: s.source_channel,
            tail_channel: s.tail_channel,
            threshold_db100: s.threshold_db100,
            target_above_db100: s.target_above_db100,
            target_below_db100: s.target_below_db100,
            steps: s.steps,
            pan: s.pan,
            playback_rate: s.playback_rate,
            freeze_group_gain: s.freeze_group_gain,
            release_tail_when_source_missing: s.release_tail_when_source_missing,
        }
    }
}
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("voice release argument required")
}
/// JS Number cannot preserve arbitrary u64 operation identities. Accept exact
/// decimal strings, and only the safe-integer subset of numeric input.
fn counter(value: &serde_json::Value) -> Result<u64> {
    if let Some(s) = value.as_str() {
        ensure!(
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()),
            "voice release counter needs an unsigned decimal string"
        );
        return Ok(s.parse()?);
    }
    let n = value
        .as_u64()
        .context("voice release counter needs an unsigned integer")?;
    ensure!(
        n <= 9_007_199_254_740_991,
        "voice release counter above JS safe integer requires a decimal string"
    );
    Ok(n)
}
fn exact(n: u64) -> serde_json::Value {
    if n <= 9_007_199_254_740_991 {
        json!(n)
    } else {
        json!(n.to_string())
    }
}
fn handle(value: &Value) -> Result<core::AudioReleaseHandle> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Handle {
        client: u32,
        generation: serde_json::Value,
    }
    let h: Handle = serde_json::from_value(value.to_json())?;
    Ok(core::AudioReleaseHandle {
        client: ClientId(h.client),
        generation: counter(&h.generation)?,
    })
}
pub(crate) fn call(cx: &mut EngineContext, name: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        name,
        "beginVoiceRelease" | "advanceVoiceRelease" | "voiceRelease" | "releaseVoiceRelease"
    ) {
        return Ok(None);
    }
    let state = cx.state()?;
    let client = state.client;
    let host = &mut state.runtime.host;
    let result = if name == "beginVoiceRelease" {
        let spec: VoiceReleaseSpec = serde_json::from_value(arg(args, 0)?.to_json())?;
        let h = host
            .begin_audio_release(client, spec.into())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        json!({"client":h.client.0,"generation":h.generation.to_string()})
    } else {
        let h = handle(arg(args, 0)?)?;
        match name {
            "advanceVoiceRelease" => {
                let elapsed = counter(&arg(args, 1)?.to_json())?;
                host.advance_audio_release(client, h, elapsed)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!(null)
            }
            "releaseVoiceRelease" => {
                host.release_audio_release(client, h)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                json!(null)
            }
            _ => {
                let i = host
                    .audio_release_info(client, h)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let phase = match i.phase {
                    core::AudioReleasePhase::Pending => "pending",
                    core::AudioReleasePhase::Active => "active",
                    core::AudioReleasePhase::Complete => "complete",
                    core::AudioReleasePhase::Cancelled => "cancelled",
                };
                json!({"phase":phase,"submitted_updates":exact(i.submitted_updates),"completed_updates":exact(i.completed_updates),"volume_db100":i.volume_db100,"source_position_frames":i.source_position_frames.map(exact),"stopped_at_update":i.stopped_at_update.map(exact)})
            }
        }
    };
    Ok(Some(Value::from_json(result)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_identity_and_counters_use_exact_decimal_strings() {
        assert_eq!(counter(&json!("18446744073709551615")).unwrap(), u64::MAX);
        assert!(counter(&json!(9_007_199_254_740_992u64)).is_err());
        assert!(counter(&json!("18446744073709551616")).is_err());
        assert!(counter(&json!("-1")).is_err());
        assert_eq!(counter(&json!(42)).unwrap(), 42);
        assert_eq!(exact(u64::MAX), json!(u64::MAX.to_string()));
    }
}
