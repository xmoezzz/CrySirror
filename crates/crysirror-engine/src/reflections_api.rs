// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Captured-pose reflection jobs over the shared acoustic scene and mixer.
use crate::{EngineState, Value};
use anyhow::{Context, Result, ensure};
use crysirror_acoustics::{
    reflection_dsp::EarlyReflections,
    reflections::{ReflectionOptions, ReflectionPaths, ReflectionQuery},
};
use crysirror_audio::mixer::VoiceHandle;
use crysirror_tasks::table::{OperationState, OperationTable};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
type Key = (u32, u32, u64);
fn key(h: VoiceHandle) -> Key {
    (h.client, h.slot, h.generation)
}
const VOICE_BYTES: usize = 8 * 1024 * 1024;
const HOST_BYTES: usize = 512 * 1024 * 1024;
struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(budget: Arc<AtomicUsize>) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n
                    .checked_add(VOICE_BYTES)
                    .filter(|n| *n <= HOST_BYTES))
                .is_ok(),
            "reflection residency budget exceeded"
        );
        Ok(Self {
            budget,
            bytes: VOICE_BYTES,
        })
    }
}
impl Charge {
    fn shrink(&mut self, bytes: usize) -> Result<()> {
        ensure!(
            bytes <= self.bytes,
            "reflection prepared storage exceeds reservation"
        );
        self.budget.fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
enum Product {
    Probe {
        revision: u64,
        paths: ReflectionPaths,
    },
    Voice {
        revision: u64,
        handle: VoiceHandle,
        effect: EarlyReflections,
    },
    Published(Json),
}
#[derive(Clone)]
struct Request {
    owner: u32,
    revision: u64,
    voice: Option<VoiceHandle>,
    query: ReflectionQuery,
    latency: usize,
}
pub struct ReflectionsRuntime {
    jobs: OperationTable<Product>,
    requests: BTreeMap<u64, Request>,
    pending: BTreeMap<Key, u64>,
    installed: BTreeMap<Key, (VoiceHandle, u64)>,
    errors: BTreeMap<Key, String>,
    budget: Arc<AtomicUsize>,
}
impl Default for ReflectionsRuntime {
    fn default() -> Self {
        Self {
            jobs: OperationTable::new(16, 64).unwrap(),
            requests: BTreeMap::new(),
            pending: BTreeMap::new(),
            installed: BTreeMap::new(),
            errors: BTreeMap::new(),
            budget: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl ReflectionsRuntime {
    pub fn reset(&mut self) {
        self.jobs.clear();
        self.requests.clear();
        self.pending.clear();
        self.installed.clear();
        self.errors.clear();
    }
}
fn stale(state: &EngineState, request: &Request) -> Option<&'static str> {
    if state.mixer_audio.acoustics.revision(request.owner) != Some(request.revision) {
        return Some("acoustic scene changed during reflection preparation");
    }
    if let Some(h) = request.voice {
        let mixer = &state.mixer_audio.mixer;
        if !mixer.source_is_playing(request.owner, h) {
            return Some("reflection source ended during preparation");
        }
        let listener = mixer.listener(request.owner);
        if mixer
            .spec(request.owner, h)
            .ok()
            .and_then(|s| s.spatial)
            .is_none_or(|s| s.position != request.query.source)
            || listener.position != request.query.listener
            || listener.forward != request.query.listener_forward
            || listener.up != request.query.listener_up
            || mixer.direct_latency_frames(request.owner, h).ok() != Some(request.latency)
        {
            return Some("reflection captured pose or HRTF latency changed; prepare again");
        }
    }
    None
}
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    crate::audio_api::sync_generation(state);
    let epoch = state.runtime.restored;
    let mut runtime = std::mem::take(&mut state.mixer_audio.reflections);
    let outcome = (|| {
        runtime.jobs.discard_obsolete(epoch);
        for (k, (h, revision)) in runtime.installed.clone() {
            let mixer = &mut state.mixer_audio.mixer;
            if !mixer.is_playing(h.client, h) {
                runtime.installed.remove(&k);
                continue;
            }
            if state.mixer_audio.acoustics.revision(h.client) != Some(revision) {
                mixer.set_reflections(h.client, h, None)?;
                runtime.installed.remove(&k);
                runtime.errors.insert(
                    k,
                    "acoustic scene changed; prepare reflections again".into(),
                );
            } else if mixer.reflection_paths(h.client, h)?.is_none() {
                runtime.installed.remove(&k);
                runtime.errors.insert(
                    k,
                    "captured pose or HRTF latency changed; prepare again".into(),
                );
            }
        }
        for (ticket, request) in runtime.requests.clone() {
            if let Some(reason) = stale(state, &request) {
                runtime
                    .jobs
                    .fail(request.owner, epoch, ticket, reason.into())?;
            }
        }
        for (owner, _, ticket) in runtime.jobs.tickets() {
            let Some(request) = runtime.requests.get(&ticket).cloned() else {
                continue;
            };
            let status = runtime.jobs.status(owner, epoch, ticket)?;
            if status.state == OperationState::Pending {
                continue;
            }
            let result = if status.state == OperationState::Ready {
                runtime.jobs.publish(owner,epoch,ticket,|product| {
                    ensure!(state.mixer_audio.acoustics.revision(owner)==Some(request.revision),"reflection scene changed before publication");
                    match product {
                        Product::Probe {revision,paths}=>Ok(Product::Published(json!({"revision":revision,"paths":paths}))),
                        Product::Voice {revision,handle,effect}=> {
                            ensure!(runtime.pending.get(&key(handle))==Some(&ticket),"reflection preparation superseded");
                            let info=json!({"revision":revision,"handle":handle,"paths":effect.paths(),"direct_latency_frames":effect.direct_latency_frames(),"tail_frames":effect.tail_frames(),"resident_bytes":effect.resident_bytes()});
                            state.mixer_audio.mixer.set_reflections(owner,handle,Some(effect))?;
                            runtime.installed.insert(key(handle),(handle,revision));runtime.errors.remove(&key(handle));
                            Ok(Product::Published(info))
                        }
                        Product::Published(info)=>Ok(Product::Published(info)),
                    }
                })
            } else {
                Err(anyhow::anyhow!(
                    "{}",
                    status
                        .error
                        .unwrap_or_else(|| "reflection preparation failed".into())
                ))
            };
            if let Some(h) = request.voice {
                if runtime.pending.get(&key(h)) == Some(&ticket) {
                    runtime.pending.remove(&key(h));
                }
                let _ = state
                    .mixer_audio
                    .mixer
                    .cancel_reflection_preparation(owner, h);
                if let Err(error) = result {
                    runtime.errors.insert(key(h), format!("{error:#}"));
                }
            }
            runtime.requests.remove(&ticket);
        }
        runtime.errors.retain(|&(client, slot, generation), _| {
            state.mixer_audio.mixer.is_playing(
                client,
                VoiceHandle {
                    client,
                    slot,
                    generation,
                },
            )
        });
        Ok(())
    })();
    state.mixer_audio.reflections = runtime;
    outcome
}
fn arg(args: &[Value], i: usize) -> Result<&Value> {
    args.get(i).context("reflection argument required")
}
fn handle(owner: u32, value: &Value) -> Result<VoiceHandle> {
    let h: VoiceHandle = serde_json::from_value(value.to_json())?;
    ensure!(h.client == owner, "foreign reflection voice handle");
    Ok(h)
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginReflectionProbe"
            | "beginVoiceReflections"
            | "reflectionStatus"
            | "cancelReflectionJob"
            | "clearVoiceReflections"
            | "reflectionsVoice"
    ) {
        return Ok(None);
    }
    poll(state)?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    if matches!(method, "beginReflectionProbe" | "beginVoiceReflections") {
        let voice = if method == "beginVoiceReflections" {
            Some(handle(owner, arg(args, 0)?)?)
        } else {
            None
        };
        let (query, latency) = if let Some(h) = voice {
            let options: ReflectionOptions = serde_json::from_value(arg(args, 1)?.to_json())?;
            let mixer = &state.mixer_audio.mixer;
            ensure!(
                mixer.source_is_playing(owner, h),
                "reflection source has ended"
            );
            let source = mixer
                .spec(owner, h)?
                .spatial
                .context("engine reflection voices require spatial settings")?
                .position;
            let listener = mixer.listener(owner);
            (
                ReflectionQuery {
                    source,
                    listener: listener.position,
                    listener_forward: listener.forward,
                    listener_up: listener.up,
                    options,
                },
                mixer.direct_latency_frames(owner, h)?,
            )
        } else {
            (
                serde_json::from_value::<ReflectionQuery>(arg(args, 0)?.to_json())?,
                0,
            )
        };
        query.validate()?;
        let (revision, scene) = state.mixer_audio.acoustics.scene(owner)?;
        let pool = state.runtime.assets.cpu_tasks();
        let rate = state.mixer_audio.mixer.sample_rate();
        let runtime = &mut state.mixer_audio.reflections;
        runtime.jobs.admit(owner)?;
        let request = Request {
            owner,
            revision,
            voice,
            query: query.clone(),
            latency,
        };
        let charge = if voice.is_some() {
            Some(Charge::new(runtime.budget.clone())?)
        } else {
            None
        };
        let ticket =
            runtime
                .jobs
                .submit_deferred(owner, epoch, "reflections", &pool, move |cancel| {
                    let paths = crysirror_acoustics::reflections::simulate_cancellable(
                        &scene.scene,
                        &query,
                        || cancel.check(),
                    )?;
                    cancel.check()?;
                    if let Some(h) = voice {
                        let mut effect = EarlyReflections::prepare(rate, paths, latency)?;
                        let mut charge = charge.unwrap();
                        let estimate = effect.resident_bytes()
                            + effect.paths().paths.len()
                                * core::mem::size_of::<
                                    crysirror_acoustics::reflections::ReflectionPath,
                                >()
                            + 8192;
                        charge.shrink(estimate)?;
                        effect.retain_storage(Arc::new(charge));
                        cancel.check()?;
                        Ok(Product::Voice {
                            revision,
                            handle: h,
                            effect,
                        })
                    } else {
                        Ok(Product::Probe { revision, paths })
                    }
                })?;
        if let Some(h) = voice {
            if let Err(error) = state.mixer_audio.mixer.pending_reflections(owner, h) {
                runtime.jobs.cancel(owner, ticket)?;
                return Err(error);
            }
            if let Some(old) = runtime.pending.insert(key(h), ticket) {
                runtime.jobs.cancel(owner, old)?;
                runtime.requests.remove(&old);
            }
            runtime.errors.remove(&key(h));
        }
        runtime.requests.insert(ticket, request);
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if matches!(method, "clearVoiceReflections" | "reflectionsVoice") {
        let h = handle(owner, arg(args, 0)?)?;
        state.mixer_audio.mixer.spec(owner, h)?;
        let runtime = &mut state.mixer_audio.reflections;
        if method == "clearVoiceReflections" {
            if let Some(ticket) = runtime.pending.remove(&key(h)) {
                runtime.jobs.cancel(owner, ticket)?;
                runtime.requests.remove(&ticket);
            }
            runtime.installed.remove(&key(h));
            runtime.errors.remove(&key(h));
            state.mixer_audio.mixer.set_reflections(owner, h, None)?;
            return Ok(Some(Value::Null));
        }
        return Ok(Some(Value::from_json(
            json!({"pending":runtime.pending.contains_key(&key(h)),"scene_revision":runtime.installed.get(&key(h)).map(|p|p.1),"paths":state.mixer_audio.mixer.reflection_paths(owner,h)?,"error":runtime.errors.get(&key(h))}),
        )));
    }
    let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
    if method == "cancelReflectionJob" {
        let runtime = &mut state.mixer_audio.reflections;
        runtime.jobs.cancel(owner, ticket)?;
        if let Some(request) = runtime.requests.remove(&ticket) {
            if let Some(h) = request.voice {
                if runtime.pending.get(&key(h)) == Some(&ticket) {
                    runtime.pending.remove(&key(h));
                }
                let _ = state
                    .mixer_audio
                    .mixer
                    .cancel_reflection_preparation(owner, h);
            }
        }
        return Ok(Some(Value::Null));
    }
    if state
        .mixer_audio
        .reflections
        .jobs
        .status(owner, epoch, ticket)?
        .state
        == OperationState::Ready
    {
        poll(state)?;
    }
    let runtime = &mut state.mixer_audio.reflections;
    let mut status = serde_json::to_value(runtime.jobs.status(owner, epoch, ticket)?)?;
    if status["state"] == "ready" {
        if let Some(Product::Published(info)) = runtime.jobs.poll(owner, epoch, ticket)? {
            status["result"] = info.clone();
        }
    }
    Ok(Some(Value::from_json(status)))
}
