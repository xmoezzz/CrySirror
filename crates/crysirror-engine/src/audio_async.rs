// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Worker-owned decoder I/O and owner-published playback; no device or VM code.
use crate::{
    EngineContext, EngineState, Value,
    audio_api::{Attachment, attach_state, voice_options},
};
use anyhow::{Context, Result, ensure};
use crysirror_audio::{
    buffered::{BufferedSource, StreamBufferSpec},
    decoder::{self, AudioSource, Clip},
    mixer::{Bus, VoiceHandle, VoiceSpec},
};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_tasks::table::{OperationState, OperationTable, next_operation_id};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
const HOST_BYTES: usize = 512 * 1024 * 1024;
const CLIP_BYTES: usize = decoder::MAX_CLIP_FRAMES * 8;
const STREAM_BYTES: usize = 4 * 1024 * 1024;
struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(budget: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n
                    .checked_add(bytes)
                    .filter(|n| *n <= HOST_BYTES))
                .is_ok(),
            "audio residency reservation exceeded"
        );
        Ok(Self { budget, bytes })
    }
    fn shrink(&mut self, bytes: usize) {
        let bytes = bytes.min(self.bytes);
        self.budget.fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        self.bytes = bytes;
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
enum Product {
    Prepared {
        asset: String,
        clip: Option<Clip>,
        stream: Option<Box<dyn AudioSource>>,
        spec: Option<VoiceSpec>,
        attachment: Option<Attachment>,
        music: Option<(String, f32)>,
    },
    Published(Json),
}
struct SeekJob {
    owner: u32,
    epoch: u64,
    handle: VoiceHandle,
    sequence: u64,
    result: Option<Result<Json, String>>,
}
pub(crate) struct AudioJobs {
    jobs: OperationTable<Product>,
    seeks: BTreeMap<u64, SeekJob>,
    music: BTreeMap<(u32, String), u64>,
    budget: Arc<AtomicUsize>,
}
impl Default for AudioJobs {
    fn default() -> Self {
        Self {
            jobs: OperationTable::new(16, 64).unwrap(),
            seeks: BTreeMap::new(),
            music: BTreeMap::new(),
            budget: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl AudioJobs {
    pub(crate) fn reset(&mut self) {
        self.jobs.clear();
        self.seeks.clear();
        self.music.clear();
    }
    fn admit(&self, owner: u32) -> Result<()> {
        ensure!(
            self.jobs.tickets().len() + self.seeks.len() < 64
                && self.jobs.tickets().iter().filter(|v| v.0 == owner).count()
                    + self.seeks.values().filter(|v| v.owner == owner).count()
                    < 16,
            "audio operation capacity exceeded; release results"
        );
        self.jobs.admit(owner)
    }
}
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("audio argument required")
}
fn handle(owner: u32, value: &Value) -> Result<VoiceHandle> {
    let h: VoiceHandle = serde_json::from_value(value.to_json())?;
    ensure!(h.client == owner, "foreign audio handle");
    Ok(h)
}
/// Owner-only publication; all VFS reads and decoding have already completed.
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    crate::audio_api::sync_generation(state);
    let epoch = state.runtime.restored;
    let mut runtime = std::mem::take(&mut state.mixer_audio.jobs);
    let outcome = (|| {
        runtime.jobs.discard_obsolete(epoch);
        runtime.seeks.retain(|_, s| s.epoch == epoch);
        runtime
            .music
            .retain(|_, ticket| runtime.jobs.tickets().iter().any(|v| v.2 == *ticket));
        let audio = &mut state.mixer_audio;
        audio
            .music
            .retain(|_, h| audio.mixer.is_playing(h.client, *h));
        for (owner, _, ticket) in runtime.jobs.tickets() {
            let status = runtime.jobs.status(owner, epoch, ticket)?;
            if status.state == OperationState::Failed {
                runtime.music.retain(|_, v| *v != ticket);
            }
            if status.state != OperationState::Ready {
                continue;
            }
            let _ = runtime.jobs.publish(owner, epoch, ticket, |product| {
                let Product::Prepared {
                    asset,
                    clip,
                    stream,
                    spec,
                    attachment,
                    music,
                } = product
                else {
                    return Ok(product);
                };
                // Validate all dependent capabilities again: resources/entities may
                // have disappeared while preparation was queued.
                if let Some(a) = &attachment {
                    if let Some(entity) = a.follow {
                        state.runtime.host.engine.world.entity(entity)?;
                    }
                    ensure!(
                        (a.acoustics.is_none() && a.reflections.is_none())
                            || state.mixer_audio.acoustics.has_scene(owner),
                        "acoustic scene removed while audio prepared"
                    );
                    if a.binaural {
                        state.mixer_audio.binaural.admit_voice(owner)?;
                    }
                }
                if let Some((name, _)) = &music {
                    ensure!(
                        runtime.music.get(&(owner, name.clone())) == Some(&ticket),
                        "music preparation superseded"
                    );
                }
                let a = &mut state.mixer_audio;
                if let Some(clip) = &clip {
                    let key = (owner, asset.clone());
                    if !a.clips.contains_key(&key) {
                        let bytes = clip.frames() * 8;
                        ensure!(
                            a.clip_bytes + bytes <= 128 * 1024 * 1024,
                            "SFX cache exceeds 128 MiB"
                        );
                        a.clip_bytes += bytes;
                        a.clips.insert(key, clip.clone());
                    }
                }
                let mut result = json!({"asset":asset});
                if let Some(spec) = spec {
                    let source = match stream {
                        Some(s) => s,
                        None => clip.context("prepared PCM missing")?.source(),
                    };
                    let h = a.mixer.play(owner, source, spec)?;
                    if let Err(e) = attach_state(state, h, attachment) {
                        let _ = state.mixer_audio.mixer.stop(owner, h, 0.);
                        return Err(e);
                    }
                    if let Some((name, fade)) = music {
                        let a = &mut state.mixer_audio;
                        if let Some(old) = a.music.insert((owner, name), h) {
                            if a.mixer.is_playing(owner, old) {
                                a.mixer.stop(owner, old, fade)?;
                            }
                        }
                    }
                    result["handle"] = serde_json::to_value(h)?;
                } else {
                    result["frames"] = json!(clip.context("prepared clip missing")?.frames());
                }
                Ok(Product::Published(result))
            });
        }
        state.mixer_audio.mixer.service_sources()?;
        for s in runtime.seeks.values_mut().filter(|s| s.result.is_none()) {
            s.result = match state
                .mixer_audio
                .mixer
                .seek_progress(s.owner, s.handle, s.sequence)
            {
                Err(e) => Some(Err(format!("{e:#}"))),
                Ok(p) if p.error.is_some() => Some(Err(p.error.unwrap())),
                Ok(p) if !p.pending => Some(Ok(json!({"handle":s.handle,"source_frame":p.frame}))),
                Ok(_) => None,
            };
        }
        Ok(())
    })();
    state.mixer_audio.jobs = runtime;
    outcome
}
pub(crate) fn call(cx: &mut EngineContext, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginPlaySfx"
            | "beginPreloadSfx"
            | "beginPlayMusic"
            | "playbackStatus"
            | "cancelPlaybackJob"
            | "beginSeekVoice"
            | "sourceStatus"
            | "clearSfxCache"
    ) {
        return Ok(None);
    }
    let owner = cx.state()?.client.0;
    let epoch = cx.state()?.runtime.restored;
    // Cancel before collecting ready jobs, so an unobserved result cannot start
    // playback merely because its cancellation request polls the service.
    if method == "cancelPlaybackJob" {
        let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
        let a = &mut cx.state()?.mixer_audio;
        if let Some(s) = a.jobs.seeks.get(&ticket) {
            ensure!(s.owner == owner, "foreign audio ticket");
            let s = a.jobs.seeks.remove(&ticket).unwrap();
            if s.result.is_none() && a.mixer.is_playing(owner, s.handle) {
                a.mixer.cancel_seek(owner, s.handle, s.sequence)?;
            }
        } else {
            a.jobs.jobs.cancel(owner, ticket)?;
            a.jobs.music.retain(|_, v| *v != ticket);
        }
        return Ok(Some(Value::Null));
    }
    poll(cx.state()?)?;
    if method == "clearSfxCache" {
        let a = &mut cx.state()?.mixer_audio;
        a.clips.retain(|(client, _), _| *client != owner);
        a.clip_bytes = a.clips.values().map(|c| c.frames() * 8).sum();
        return Ok(Some(Value::Null));
    }
    if method == "sourceStatus" {
        let h = handle(owner, arg(args, 0)?)?;
        return Ok(Some(Value::from_json(serde_json::to_value(
            cx.state()?.mixer_audio.mixer.source_status(owner, h)?,
        )?)));
    }
    if method == "beginSeekVoice" {
        let h = handle(owner, arg(args, 0)?)?;
        let frame = u64::try_from(arg(args, 1)?.as_i64()?)?;
        let a = &mut cx.state()?.mixer_audio;
        a.jobs.admit(owner)?;
        let ticket = next_operation_id()?;
        let sequence = a.mixer.begin_seek(owner, h, frame)?;
        a.jobs.seeks.insert(
            ticket,
            SeekJob {
                owner,
                epoch,
                handle: h,
                sequence,
                result: None,
            },
        );
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if method == "playbackStatus" {
        let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
        let a = &mut cx.state()?.mixer_audio;
        if let Some(s) = a.jobs.seeks.get(&ticket) {
            ensure!(
                s.owner == owner && s.epoch == epoch,
                "foreign or obsolete audio ticket"
            );
            let (state, result, error) = match &s.result {
                None => ("pending", Json::Null, Json::Null),
                Some(Ok(j)) => ("ready", j.clone(), Json::Null),
                Some(Err(e)) => ("failed", Json::Null, json!(e)),
            };
            return Ok(Some(Value::from_json(
                json!({"ticket":ticket,"kind":"audio_seek","state":state,"result":result,"error":error}),
            )));
        }
        let status = a.jobs.jobs.status(owner, epoch, ticket)?;
        let mut value = serde_json::to_value(&status)?;
        if status.state == OperationState::Ready {
            if let Some(Product::Published(result)) = a.jobs.jobs.poll(owner, epoch, ticket)? {
                value["result"] = result.clone();
            }
        }
        return Ok(Some(Value::from_json(value)));
    }
    cx.state()?.mixer_audio.jobs.admit(owner)?;
    let preload = method == "beginPreloadSfx";
    let music = method == "beginPlayMusic";
    let (asset, spec, attachment, music_options) = if preload {
        (arg(args, 0)?.as_str()?.to_owned(), None, None, None)
    } else {
        let mut value = arg(args, usize::from(music))?.clone();
        if music {
            if let Value::Map(map) = &mut value {
                map.entry("looping".into()).or_insert(Value::Bool(true));
            }
        }
        let (asset, mut spec, attachment) = voice_options(cx, &value, true)?;
        let music_options = if music {
            ensure!(attachment.is_none(), "music is non-spatial");
            let name = arg(args, 0)?.as_str()?.to_owned();
            ensure!(!name.is_empty() && name.len() <= 256, "music name");
            let fade = args.get(2).map(Value::as_f32).transpose()?.unwrap_or(0.);
            ensure!(
                fade.is_finite() && (0.0..=600000.).contains(&fade),
                "music crossfade duration"
            );
            spec.bus = Bus::Music;
            spec.fade_in_ms = fade;
            spec.validate()?;
            Some((name, fade))
        } else {
            None
        };
        (asset.unwrap(), Some(spec), attachment, music_options)
    };
    let logical = VfsPath::new(Mount::Asset, &asset).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        logical.relative() == asset,
        "canonical audio asset required"
    );
    let state = cx.state()?;
    let storage = state
        .runtime
        .host
        .fs
        .storage(state.client)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let pool = state.runtime.assets.io_tasks();
    let worker_pool = pool.clone();
    let cached = (!music)
        .then(|| {
            state
                .mixer_audio
                .clips
                .get(&(owner, asset.clone()))
                .cloned()
        })
        .flatten();
    let mut charge = Charge::new(
        state.mixer_audio.jobs.budget.clone(),
        if cached.is_some() {
            0
        } else if music {
            STREAM_BYTES
        } else {
            CLIP_BYTES
        },
    )?;
    let pending_name = music_options.as_ref().map(|v| v.0.clone());
    let ticket = state.mixer_audio.jobs.jobs.submit(
        owner,
        epoch,
        "audio_prepare",
        &pool,
        move |cancel| {
            cancel.check()?;
            let mut clip = cached;
            let mut stream = None;
            if clip.is_none() {
                let reader = storage
                    .open_asset(&logical)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let len = reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    len <= 512 * 1024 * 1024,
                    "encoded audio stream exceeds 512 MiB"
                );
                let source = decoder::open_asset(reader, asset.rsplit_once('.').map(|v| v.1))?;
                cancel.check()?;
                if music {
                    let loop_start = spec.as_ref().and_then(|s| s.loop_points.map(|p| p.start));
                    let mut source = BufferedSource::prepare_cancellable(
                        source,
                        worker_pool,
                        StreamBufferSpec::default(),
                        loop_start,
                        &cancel,
                    )?;
                    source.retain_storage(Arc::new(charge))?;
                    stream = Some(Box::new(source) as Box<dyn AudioSource>);
                } else {
                    // Decoder reads stay cancellable between bounded packets.
                    let mut source = source;
                    let mut samples = Vec::new();
                    let mut block = vec![[0.; 2]; 4096];
                    loop {
                        cancel.check()?;
                        let n = source.read(&mut block)?;
                        ensure!(n <= block.len(), "invalid audio read size");
                        if n == 0 {
                            break;
                        }
                        ensure!(
                            samples.len() + n <= decoder::MAX_CLIP_FRAMES,
                            "audio clip too long; use streaming music"
                        );
                        samples.extend_from_slice(&block[..n]);
                    }
                    let mut decoded = Clip::new(source.sample_rate(), samples)?;
                    charge.shrink(decoded.frames() * 8);
                    decoded.retain_storage(Arc::new(charge));
                    clip = Some(decoded);
                }
            }
            cancel.check()?;
            Ok(Product::Prepared {
                asset,
                clip,
                stream,
                spec,
                attachment,
                music: music_options,
            })
        },
    )?;
    if let Some(name) = pending_name {
        state.mixer_audio.jobs.music.insert((owner, name), ticket);
    }
    Ok(Some(Value::Int(ticket.try_into()?)))
}
