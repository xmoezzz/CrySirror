// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-scoped immutable audio resources shared by scripts and native adapters.
use crate::{EngineContext, Value};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioAsset, AudioPreparePhase, AudioResourceHandle, AudioResourceInfo},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Immutable source preparation; transform runs before independent playback.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioSourcePrepareSpec {
    /// Decode/discard to measure sources with unknown frame count.
    pub measure_frames: bool,
    /// Optional decoded-input frame boundary, before any transform.
    pub frame_limit: Option<u64>,
    /// Optional named PCM numeric profile; ordinary playback rate is separate.
    pub transform: Option<AudioTransformSpec>,
}
/// CPU-baked voice processing, usable without a VM or device.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioTransformSpec {
    /// Native-compatible PCM16 splice compression, preserving sample rate.
    SiglusJitan {
        /// Integer compression percent, 100..400; 100 is identity.
        percent: u16,
    },
}
impl From<AudioTransformSpec> for crysirror_core::service::AudioResourceTransform {
    fn from(t: AudioTransformSpec) -> Self {
        match t {
            AudioTransformSpec::SiglusJitan { percent } => Self::SiglusJitan { percent },
        }
    }
}
pub(crate) fn handle(owner: u32, value: &Value) -> Result<AudioResourceHandle> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Handle {
        client: u32,
        id: u64,
    }
    let h: Handle = serde_json::from_value(value.to_json())?;
    ensure!(h.client == owner, "foreign prepared audio resource");
    Ok(AudioResourceHandle {
        client: h.client,
        id: h.id,
    })
}
fn info(i: AudioResourceInfo) -> serde_json::Value {
    json!({"source_rate":i.source_rate,"source_frames":i.source_frames,"encoded_bytes":i.encoded_bytes})
}
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("audio resource argument required")
}
pub(crate) fn call(cx: &mut EngineContext, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginPrepareSource"
            | "sourcePreparation"
            | "cancelSourcePreparation"
            | "sourceInfo"
            | "releaseSource"
    ) {
        return Ok(None);
    }
    let state = cx.state()?;
    let owner = state.client.0;
    let result = match method {
        "beginPrepareSource" => {
            let options: AudioSourcePrepareSpec = args
                .get(1)
                .filter(|v| !matches!(v, Value::Null))
                .map(|v| serde_json::from_value(v.to_json()))
                .transpose()?
                .unwrap_or_default();
            let asset = arg(args, 0)?.as_str()?;
            let path = VfsPath::new(Mount::Asset, asset).map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(path.relative() == asset, "canonical audio asset required");
            let storage = state
                .runtime
                .host
                .fs
                .storage(state.client)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let pool = state.runtime.assets.io_tasks();
            let ticket = state
                .runtime
                .host
                .channel_audio
                .resources
                .begin_transformed(
                    owner,
                    AudioAsset {
                        path,
                        frame_limit: options.frame_limit,
                    },
                    options.measure_frames,
                    options.transform.map(Into::into),
                    storage,
                    Some(pool),
                    true,
                )?;
            json!(ticket)
        }
        "sourcePreparation" => {
            let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
            let status = state
                .runtime
                .host
                .channel_audio
                .resources
                .preparation(owner, ticket)?;
            let result = status
                .resource
                .zip(status.info)
                .map(|(h, i)| json!({"resource":{"client":h.client,"id":h.id},"info":info(i)}));
            json!({"ticket":ticket,"kind":"audio_resource","state":match status.phase {AudioPreparePhase::Pending=>"pending",AudioPreparePhase::Ready=>"ready",AudioPreparePhase::Failed=>"failed"},"result":result,"error":status.error})
        }
        "cancelSourcePreparation" => {
            state
                .runtime
                .host
                .channel_audio
                .resources
                .release_preparation(owner, u64::try_from(arg(args, 0)?.as_i64()?)?)?;
            serde_json::Value::Null
        }
        "sourceInfo" => info(
            state
                .runtime
                .host
                .channel_audio
                .resources
                .capture(owner, handle(owner, arg(args, 0)?)?)?
                .info(),
        ),
        "releaseSource" => {
            state
                .runtime
                .host
                .channel_audio
                .resources
                .release(owner, handle(owner, arg(args, 0)?)?)?;
            serde_json::Value::Null
        }
        _ => unreachable!(),
    };
    Ok(Some(Value::from_json(result)))
}
