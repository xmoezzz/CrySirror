// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared owner-published gain groups across pooled and native/media mixers.
use crate::{EngineContext, Value};
use anyhow::{Context, Result, ensure};
use crysirror_audio::mixer::VoiceHandle;
use crysirror_core::service::AudioMixGroupInfo;
use serde::Deserialize;
use serde_json::json;
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("audio mix group argument required")
}
fn info(i: AudioMixGroupInfo) -> Value {
    Value::from_json(
        json!({"settings":{"parent":i.settings.parent,"gain":i.settings.gain,"muted":i.settings.muted},"gain":i.gain,"effective_gain":i.effective_gain,"fade_frames":i.fade_frames,"output_rate":i.output_rate}),
    )
}
pub(crate) fn call(cx: &mut EngineContext, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "setMixGroup" | "mixGroup" | "nativeMixGroup" | "removeMixGroup" | "setVoiceMixGroup"
    ) {
        return Ok(None);
    }
    let state = cx.state()?;
    let owner = state.client.0;
    if method == "setVoiceMixGroup" {
        let h: VoiceHandle = serde_json::from_value(arg(args, 0)?.to_json())?;
        let name = match args.get(1) {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str()?.to_owned()),
        };
        state
            .mixer_audio
            .mixer
            .set_voice_mix_group(owner, h, name)?;
        return Ok(Some(Value::Null));
    }
    let name = arg(args, 0)?.as_str()?;
    let pooled = state.mixer_audio.mixer.mix_group(owner, name)?;
    let native = state
        .runtime
        .host
        .channel_audio
        .mixer
        .mix_group(owner, name)?;
    match method {
        "mixGroup" => return Ok(Some(pooled.map(info).unwrap_or(Value::Null))),
        "nativeMixGroup" => return Ok(Some(native.map(info).unwrap_or(Value::Null))),
        "setMixGroup" => {
            #[derive(Default, Deserialize)]
            #[serde(default, deny_unknown_fields)]
            struct Change {
                parent: Option<String>,
                gain: Option<f32>,
                muted: Option<bool>,
            }
            let input = arg(args, 1)?;
            let Value::Map(provided) = input else {
                anyhow::bail!("mix group settings must be an object")
            };
            let change: Change = serde_json::from_value(input.to_json())?;
            let mut settings = pooled.or(native).map(|i| i.settings).unwrap_or_default();
            if provided.contains_key("parent") {
                settings.parent = change.parent;
            }
            if provided.contains_key("gain") {
                settings.gain = change
                    .gain
                    .context("mix group gain cannot be null/nonfinite")?;
            }
            if provided.contains_key("muted") {
                settings.muted = change.muted.context("mix group mute cannot be null")?;
            }
            let fade = args
                .get(2)
                .filter(|v| !matches!(v, Value::Null))
                .map(Value::as_f32)
                .transpose()?
                .unwrap_or(0.);
            // Both dependency/budget validations precede either publication.
            state
                .mixer_audio
                .mixer
                .validate_mix_group(owner, name, &settings, fade)?;
            state
                .runtime
                .host
                .channel_audio
                .mixer
                .validate_mix_group(owner, name, &settings, fade)?;
            state
                .mixer_audio
                .mixer
                .set_mix_group(owner, name, settings.clone(), fade)?;
            state
                .runtime
                .host
                .channel_audio
                .mixer
                .set_mix_group(owner, name, settings, fade)?;
        }
        "removeMixGroup" => {
            ensure!(
                pooled.is_some() || native.is_some(),
                "audio mix group missing in this client"
            );
            if pooled.is_some() {
                state
                    .mixer_audio
                    .mixer
                    .validate_remove_mix_group(owner, name)?;
            }
            if native.is_some() {
                state
                    .runtime
                    .host
                    .channel_audio
                    .mixer
                    .validate_remove_mix_group(owner, name)?;
            }
            if pooled.is_some() {
                state.mixer_audio.mixer.remove_mix_group(owner, name)?;
            }
            if native.is_some() {
                state
                    .runtime
                    .host
                    .channel_audio
                    .mixer
                    .remove_mix_group(owner, name)?;
            }
        }
        _ => unreachable!(),
    }
    Ok(Some(Value::Null))
}
