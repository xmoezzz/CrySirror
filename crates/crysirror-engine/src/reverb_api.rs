// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded worker preparation and owner publication of client reverb buses.
use crate::{EngineState, Value};
use anyhow::{Context, Result, ensure};
use crysirror_audio::reverb::{PreparedReverb, ReverbSettings};
use crysirror_tasks::table::{OperationState, OperationTable};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
const PAIR_BYTES: usize = 64 * 1024 * 1024;
const HOST_BYTES: usize = 512 * 1024 * 1024;
struct Charge(Arc<AtomicUsize>);
impl Charge {
    fn new(budget: Arc<AtomicUsize>) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n
                    .checked_add(PAIR_BYTES)
                    .filter(|n| *n <= HOST_BYTES))
                .is_ok(),
            "reverb residency reservation exceeded"
        );
        Ok(Self(budget))
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.0.fetch_sub(PAIR_BYTES, Ordering::AcqRel);
    }
}
enum Product {
    Prepared {
        main: PreparedReverb,
        native: PreparedReverb,
    },
    Published(Json),
}
pub struct ReverbRuntime {
    jobs: OperationTable<Product>,
    pending: BTreeMap<u32, u64>,
    budget: Arc<AtomicUsize>,
}
impl Default for ReverbRuntime {
    fn default() -> Self {
        Self {
            jobs: OperationTable::new(8, 32).unwrap(),
            pending: BTreeMap::new(),
            budget: Arc::new(AtomicUsize::new(0)),
        }
    }
}
impl ReverbRuntime {
    pub fn reset(&mut self) {
        self.jobs.clear();
        self.pending.clear();
    }
}
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    crate::audio_api::sync_generation(state);
    let epoch = state.runtime.restored;
    let mut runtime = std::mem::take(&mut state.mixer_audio.reverb);
    let outcome = (|| {
        runtime.jobs.discard_obsolete(epoch);
        for (owner, _, ticket) in runtime.jobs.tickets() {
            let status = runtime.jobs.status(owner, epoch, ticket)?;
            if status.state == OperationState::Failed {
                if runtime.pending.get(&owner) == Some(&ticket) {
                    runtime.pending.remove(&owner);
                }
            }
            if status.state != OperationState::Ready {
                continue;
            }
            let pending = &mut runtime.pending;
            let _=runtime.jobs.publish(owner,epoch,ticket,|product| {
                let Product::Prepared {main,native}=product else {return Ok(product)};
                ensure!(pending.get(&owner)==Some(&ticket),"superseded reverb preparation");
                let info=json!({"revision":ticket,"kind":"parametric_late_reverb","settings":main.settings(),"sample_rate":main.sample_rate(),"native_sample_rate":native.sample_rate()});
                let old_main=state.mixer_audio.mixer.set_reverb(owner,Some(main))?;
                if let Err(error)=state.runtime.host.channel_audio.mixer.set_reverb(owner,Some(native)) {
                    let _=state.mixer_audio.mixer.set_reverb(owner,old_main);
                    return Err(error);
                }
                pending.remove(&owner);
                Ok(Product::Published(info))
            });
        }
        Ok(())
    })();
    state.mixer_audio.reverb = runtime;
    outcome
}
fn arg(args: &[Value], at: usize) -> Result<&Value> {
    args.get(at).context("reverb argument required")
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginReverb" | "reverbStatus" | "cancelReverbJob" | "clearReverb" | "reverbSettings"
    ) {
        return Ok(None);
    }
    poll(state)?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    if method == "beginReverb" {
        let settings: ReverbSettings = serde_json::from_value(arg(args, 0)?.to_json())?;
        settings.validate()?;
        let main_rate = state.mixer_audio.mixer.sample_rate();
        let native_rate = state.runtime.host.channel_audio.mixer.sample_rate();
        let pool = state.runtime.assets.cpu_tasks();
        let runtime = &mut state.mixer_audio.reverb;
        runtime.jobs.admit(owner)?;
        let charge = Arc::new(Charge::new(runtime.budget.clone())?);
        let ticket =
            runtime
                .jobs
                .submit_deferred(owner, epoch, "reverb", &pool, move |cancel| {
                    cancel.check()?;
                    let mut main = PreparedReverb::prepare(main_rate, settings.clone())?;
                    main.retain_storage(charge.clone());
                    cancel.check()?;
                    let mut native = PreparedReverb::prepare(native_rate, settings)?;
                    native.retain_storage(charge);
                    cancel.check()?;
                    Ok(Product::Prepared { main, native })
                })?;
        if let Some(old) = runtime.pending.insert(owner, ticket) {
            let _ = runtime.jobs.cancel(owner, old);
        }
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if method == "reverbSettings" {
        return Ok(Some(Value::from_json(json!(
            state.mixer_audio.mixer.reverb(owner)
        ))));
    }
    if method == "clearReverb" {
        let runtime = &mut state.mixer_audio.reverb;
        if let Some(ticket) = runtime.pending.remove(&owner) {
            let _ = runtime.jobs.cancel(owner, ticket);
        }
        state.mixer_audio.mixer.set_reverb(owner, None)?;
        state
            .runtime
            .host
            .channel_audio
            .mixer
            .set_reverb(owner, None)?;
        return Ok(Some(Value::Null));
    }
    let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
    if method == "cancelReverbJob" {
        let runtime = &mut state.mixer_audio.reverb;
        runtime.jobs.cancel(owner, ticket)?;
        runtime.pending.retain(|_, id| *id != ticket);
        return Ok(Some(Value::Null));
    }
    // Poll again after readiness observation so Ready always means published.
    if state
        .mixer_audio
        .reverb
        .jobs
        .status(owner, epoch, ticket)?
        .state
        == OperationState::Ready
    {
        poll(state)?;
    }
    let runtime = &mut state.mixer_audio.reverb;
    let mut status = serde_json::to_value(runtime.jobs.status(owner, epoch, ticket)?)?;
    if status["state"] == "ready" {
        if let Some(Product::Published(info)) = runtime.jobs.poll(owner, epoch, ticket)? {
            status["result"] = info.clone();
        }
    }
    Ok(Some(Value::from_json(status)))
}
