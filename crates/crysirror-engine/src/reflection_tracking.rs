// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Coalesced pose snapshots and worker plans; delay history stays in the mixer.
use crate::{
    EngineState,
    reflections_api::{Charge, VOICE_BYTES},
};
use anyhow::{Context, Result, ensure};
use crysirror_acoustics::{
    reflection_dsp::{EarlyReflections, ReflectionPlan},
    reflections::{ReflectionQuery, ReflectionTrackingSpec},
};
use crysirror_audio::mixer::VoiceHandle;
use crysirror_tasks::Task;
use glam::Vec3;
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
type Key = (u32, u32, u64);
fn key(h: VoiceHandle) -> Key {
    (h.client, h.slot, h.generation)
}
#[derive(Clone, Serialize)]
struct Snapshot {
    captured_at_ms: u64,
    revision: u64,
    scene_generation: u64,
    query: ReflectionQuery,
    latency: usize,
}
enum Prepared {
    Initial(EarlyReflections),
    Plan(ReflectionPlan),
}
struct Flight {
    snapshot: Snapshot,
    task: Task<Prepared>,
}
struct Tracked {
    handle: VoiceHandle,
    spec: ReflectionTrackingSpec,
    desired: Option<Snapshot>,
    submitted: Option<Snapshot>,
    published: Option<Snapshot>,
    flight: Option<Flight>,
    ready: Option<(Snapshot, Prepared)>,
    next_due: u64,
    error: Option<String>,
    waiting: Option<String>,
    completed: u64,
    draining: bool,
    muted: bool,
    current_revision: Option<u64>,
}
#[derive(Default)]
pub(crate) struct TrackingRuntime {
    voices: BTreeMap<Key, Tracked>,
    order: VecDeque<Key>,
    clock: u64,
}
impl TrackingRuntime {
    pub(crate) fn reset(&mut self) {
        self.voices.clear();
        self.order.clear();
        self.clock = 0;
    }
    pub(crate) fn remove(&mut self, h: VoiceHandle) -> bool {
        self.order.retain(|k| *k != key(h));
        self.voices.remove(&key(h)).is_some()
    }
    pub(crate) fn status(&self, h: VoiceHandle) -> Option<Json> {
        let t = self.voices.get(&key(h))?;
        Some(
            json!({"settings":t.spec,"desired":t.desired,"submitted":t.submitted,"published":t.published,
            "pending":!t.draining&&(t.flight.is_some()||t.ready.is_some()||t.waiting.is_some()||(t.published.is_none()&&t.error.is_none())||t.desired.as_ref().is_some_and(|d|t.submitted.as_ref().is_none_or(|s|changed(s,d,&t.spec)))),"published_age_ms":t.published.as_ref().map(|p|self.clock.saturating_sub(p.captured_at_ms)),"waiting_for":t.waiting,"error":t.error,"completed_updates":t.completed,"draining":t.draining,"muted":t.muted,"clock_ms":self.clock,"next_due_ms":t.next_due,"current_revision":t.current_revision,"geometry_stale":t.published.as_ref().is_some_and(|p|Some(p.revision)!=t.current_revision)}),
        )
    }
    pub(crate) fn revision(&self, h: VoiceHandle) -> Option<u64> {
        self.voices
            .get(&key(h))?
            .published
            .as_ref()
            .map(|p| p.revision)
    }
}
fn snapshot(
    state: &EngineState,
    h: VoiceHandle,
    spec: &ReflectionTrackingSpec,
    clock: u64,
) -> Result<Snapshot> {
    let mixer = &state.mixer_audio.mixer;
    let source = mixer
        .spec(h.client, h)?
        .spatial
        .context("tracked reflections require spatial voice")?
        .position;
    let l = mixer.listener(h.client);
    Ok(Snapshot {
        captured_at_ms: clock,
        revision: state
            .mixer_audio
            .acoustics
            .revision(h.client)
            .context("acoustic scene not prepared")?,
        scene_generation: state
            .mixer_audio
            .acoustics
            .generation(h.client)
            .context("acoustic scene generation missing")?,
        latency: mixer.direct_latency_frames(h.client, h)?,
        query: ReflectionQuery {
            source,
            listener: l.position,
            listener_forward: l.forward,
            listener_up: l.up,
            options: spec.options.clone(),
        },
    })
}
fn changed(a: &Snapshot, b: &Snapshot, spec: &ReflectionTrackingSpec) -> bool {
    if a.revision != b.revision || a.latency != b.latency {
        return true;
    }
    let distance = |a: [f32; 3], b: [f32; 3]| {
        Vec3::from(a).distance(Vec3::from(b)) * spec.options.propagation.metres_per_unit
    };
    let angle = |a: [f32; 3], b: [f32; 3]| {
        if a == b {
            0.
        } else {
            Vec3::from(a)
                .normalize()
                .dot(Vec3::from(b).normalize())
                .clamp(-1., 1.)
                .acos()
                .to_degrees()
        }
    };
    distance(a.query.source, b.query.source) > spec.position_threshold_metres
        || distance(a.query.listener, b.query.listener) > spec.position_threshold_metres
        || angle(a.query.listener_forward, b.query.listener_forward) > spec.angle_threshold_degrees
        || angle(a.query.listener_up, b.query.listener_up) > spec.angle_threshold_degrees
}
pub(crate) fn track(
    state: &mut EngineState,
    h: VoiceHandle,
    spec: ReflectionTrackingSpec,
) -> Result<()> {
    spec.validate()?;
    let mixer = &state.mixer_audio.mixer;
    ensure!(
        mixer.source_is_playing(h.client, h),
        "reflection source has ended"
    );
    ensure!(
        mixer.spec(h.client, h)?.spatial.is_some(),
        "tracked reflections require spatial voice"
    );
    ensure!(
        state.mixer_audio.acoustics.has_scene(h.client),
        "prepare an acoustic scene before reflection tracking"
    );
    let runtime = &state.mixer_audio.reflections.tracking;
    ensure!(
        runtime.voices.contains_key(&key(h)) || runtime.voices.len() < 64,
        "reflection tracking voice capacity exceeded"
    );
    crate::reflections_api::clear_static(state, h)?;
    state
        .mixer_audio
        .mixer
        .pending_tracked_reflections(h.client, h)?;
    let runtime = &mut state.mixer_audio.reflections.tracking;
    runtime.remove(h);
    runtime.voices.insert(
        key(h),
        Tracked {
            handle: h,
            spec,
            desired: None,
            submitted: None,
            published: None,
            flight: None,
            ready: None,
            next_due: runtime.clock,
            error: None,
            waiting: None,
            completed: 0,
            draining: false,
            muted: true,
            current_revision: None,
        },
    );
    runtime.order.push_back(key(h));
    Ok(())
}
pub(crate) fn collect(state: &mut EngineState) -> Result<()> {
    let mut runtime = std::mem::take(&mut state.mixer_audio.reflections.tracking);
    let result = (|| {
        let mut remove = vec![];
        for (k, t) in &mut runtime.voices {
            let h = t.handle;
            if !state.mixer_audio.mixer.is_playing(h.client, h) {
                remove.push(*k);
                continue;
            }
            let scene = state.mixer_audio.acoustics.revision(h.client);
            let generation = state.mixer_audio.acoustics.generation(h.client);
            t.current_revision = scene;
            if scene.is_none() || state.mixer_audio.mixer.spec(h.client, h)?.spatial.is_none() {
                state.mixer_audio.mixer.set_reflections(h.client, h, None)?;
                remove.push(*k);
                continue;
            }
            state
                .mixer_audio
                .mixer
                .retire_reflection_plan(h.client, h)?;
            if !state.mixer_audio.mixer.source_is_playing(h.client, h) {
                t.draining = true;
                t.flight = None;
                t.ready = None;
                continue;
            }
            t.draining = false;
            if t.published.as_ref().is_some_and(|p| {
                Some(p.scene_generation) != generation
                    || state
                        .mixer_audio
                        .mixer
                        .direct_latency_frames(h.client, h)
                        .is_ok_and(|latency| latency != p.latency)
            }) {
                state
                    .mixer_audio
                    .mixer
                    .suppress_tracked_reflections(h.client, h)?;
                t.muted = true;
            }
            if t.flight
                .as_ref()
                .is_some_and(|f| Some(f.snapshot.scene_generation) != generation)
            {
                t.flight = None;
                t.submitted = None;
                t.next_due = runtime.clock;
            }
            if t.ready
                .as_ref()
                .is_some_and(|(s, _)| Some(s.scene_generation) != generation)
            {
                t.ready = None;
                t.submitted = None;
                t.next_due = runtime.clock;
            }
            if let Some(f) = &mut t.flight {
                match f.task.try_take() {
                    Ok(None) => {}
                    Ok(Some(prepared)) => {
                        t.ready = Some((f.snapshot.clone(), prepared));
                        t.flight = None;
                    }
                    Err(error) => {
                        t.error = Some(format!("{error:#}"));
                        t.flight = None;
                        state
                            .mixer_audio
                            .mixer
                            .cancel_reflection_preparation(h.client, h)?;
                    }
                }
            }
            if let Some((s, _)) = &t.ready {
                // Poses may advance while a worker runs. Publish the latest completed
                // snapshot, then coalesce another update. Topology generation/latency
                // stay strict; instance-only geometry may lag and is reported stale.
                match state.mixer_audio.mixer.direct_latency_frames(h.client, h) {
                    Ok(latency) if latency != s.latency => {
                        t.ready = None;
                        t.submitted = None;
                        t.next_due = runtime.clock;
                        continue;
                    }
                    Err(_) => continue,
                    _ => {}
                }
                let initial = matches!(t.ready.as_ref().unwrap().1, Prepared::Initial(_));
                if !initial && !state.mixer_audio.mixer.reflection_plan_ready(h.client, h)? {
                    continue;
                }
                let (s, prepared) = t.ready.take().unwrap();
                let result = match prepared {
                    Prepared::Initial(effect) => state
                        .mixer_audio
                        .mixer
                        .set_reflections(h.client, h, Some(effect))
                        .map(|_| ()),
                    Prepared::Plan(plan) => state
                        .mixer_audio
                        .mixer
                        .publish_reflection_plan(h.client, h, plan, t.spec.crossfade_ms)
                        .map(|_| ()),
                };
                match result {
                    Ok(()) => {
                        t.published = Some(s);
                        t.error = None;
                        t.muted = false;
                        t.completed = t.completed.saturating_add(1);
                    }
                    Err(error) => {
                        t.error = Some(format!("{error:#}"));
                        state
                            .mixer_audio
                            .mixer
                            .cancel_reflection_preparation(h.client, h)?;
                    }
                }
            }
        }
        for k in remove {
            runtime.voices.remove(&k);
            runtime.order.retain(|v| *v != k);
        }
        Ok(())
    })();
    state.mixer_audio.reflections.tracking = runtime;
    result
}
pub(crate) fn update(state: &mut EngineState, dt_ms: u64) -> Result<()> {
    collect(state)?;
    let budget = state.mixer_audio.reflections.budget.clone();
    let pool = state.runtime.assets.cpu_tasks();
    let rate = state.mixer_audio.mixer.sample_rate();
    let mut runtime = std::mem::take(&mut state.mixer_audio.reflections.tracking);
    runtime.clock = runtime.clock.saturating_add(dt_ms);
    let result = (|| {
        let mut starts = 0;
        let count = runtime.order.len();
        for _ in 0..count {
            if starts >= 4 {
                break;
            }
            let k = runtime.order.pop_front().unwrap();
            runtime.order.push_back(k);
            let Some(t) = runtime.voices.get_mut(&k) else {
                continue;
            };
            if t.draining {
                continue;
            }
            let desired = match snapshot(state, t.handle, &t.spec, runtime.clock) {
                Ok(s) => s,
                Err(error) => {
                    t.waiting = Some(format!("{error:#}"));
                    continue;
                }
            };
            t.waiting = None;
            t.desired = Some(desired.clone());
            if t.flight.is_some()
                || t.ready.is_some()
                || runtime.clock < t.next_due
                || t.submitted
                    .as_ref()
                    .is_some_and(|s| !changed(s, &desired, &t.spec))
            {
                continue;
            }
            let global = runtime
                .voices
                .values()
                .filter(|v| v.flight.is_some())
                .count();
            let owner = runtime
                .voices
                .values()
                .filter(|v| v.handle.client == k.0 && v.flight.is_some())
                .count();
            if global >= 32 {
                break;
            }
            if owner >= 8 {
                continue;
            }
            let permit = match pool.reserve() {
                Ok(p) => p,
                Err(error) if error.is::<crysirror_tasks::CapacityExceeded>() => break,
                Err(error) => return Err(error),
            };
            let t = runtime.voices.get_mut(&k).unwrap();
            let initial = state
                .mixer_audio
                .mixer
                .reflection_paths(k.0, t.handle)?
                .is_none();
            let (revision, scene) = state.mixer_audio.acoustics.scene(k.0)?;
            let mut charge =
                match Charge::reserve(budget.clone(), if initial { VOICE_BYTES } else { 65536 }) {
                    Ok(charge) => charge,
                    Err(error) => {
                        t.waiting = Some(format!("{error:#}"));
                        t.next_due = runtime.clock.saturating_add(t.spec.refresh_interval_ms);
                        continue;
                    }
                };
            let snapshot = desired.clone();
            let query = desired.query.clone();
            let latency = desired.latency;
            let task = permit.spawn(move |cancel| {
                let paths = crysirror_acoustics::reflections::simulate_cancellable(
                    &scene.scene,
                    &query,
                    || cancel.check(),
                )?;
                cancel.check()?;
                let value = if initial {
                    let mut effect = EarlyReflections::prepare_tracked(rate, paths, latency)?;
                    charge.shrink(effect.resident_bytes() + 8192)?;
                    effect.retain_storage(Arc::new(charge));
                    Prepared::Initial(effect)
                } else {
                    let mut plan = ReflectionPlan::prepare(rate, paths, latency)?;
                    charge.shrink(plan.resident_bytes() + 8192)?;
                    plan.retain_storage(Arc::new(charge));
                    Prepared::Plan(plan)
                };
                cancel.check()?;
                Ok(value)
            })?;
            debug_assert_eq!(revision, snapshot.revision);
            t.flight = Some(Flight { snapshot, task });
            t.submitted = Some(desired);
            t.next_due = runtime.clock.saturating_add(t.spec.refresh_interval_ms);
            starts += 1;
        }
        Ok(())
    })();
    state.mixer_audio.reflections.tracking = runtime;
    result
}
