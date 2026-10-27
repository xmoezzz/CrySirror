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
use serde::Deserialize;
use serde_json::json;

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
            #[derive(Default, Deserialize)]
            #[serde(default, deny_unknown_fields)]
            struct Options {
                measure_frames: bool,
                frame_limit: Option<u64>,
            }
            let options: Options = args
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
            let ticket = state.runtime.host.channel_audio.resources.begin(
                owner,
                AudioAsset {
                    path,
                    frame_limit: options.frame_limit,
                },
                options.measure_frames,
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
