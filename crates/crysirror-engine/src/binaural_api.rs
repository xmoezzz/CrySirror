// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Client-owned HRTF datasets and bounded voice/filter preparation.
use crate::{EngineState, Value, audio_api::Attachment};
use anyhow::{Context, Result, ensure};
use crysirror_acoustics::binaural::{
    BinauralVoice, FilterPublisher, HrtfDataset, HrtfOptions, PreparedFilter,
};
use crysirror_audio::mixer::{Listener, Mixer, VoiceHandle};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_tasks::{
    Task,
    table::{OperationState, OperationTable},
};
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
type Key = (u32, u32, u64);
fn key(h: VoiceHandle) -> Key {
    (h.client, h.slot, h.generation)
}
const HOST_BYTES: usize = 1024 * 1024 * 1024;
const DATASET_BYTES: usize = 256 * 1024 * 1024;
const VOICE_BYTES: usize = 2 * 1024 * 1024;
struct Charge {
    counter: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(counter: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        ensure!(
            counter
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| old
                    .checked_add(bytes)
                    .filter(|v| *v <= HOST_BYTES))
                .is_ok(),
            "HRTF memory reservation exceeded"
        );
        Ok(Self { counter, bytes })
    }
    fn shrink(&mut self, bytes: usize) {
        let bytes = bytes.min(self.bytes);
        self.counter
            .fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        self.bytes = bytes;
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
struct Dataset {
    data: HrtfDataset,
    _charge: Charge,
}
enum Product {
    Dataset(Arc<Dataset>),
    Summary(Json),
}
enum VoiceProduct {
    Voice(FilterPublisher, BinauralVoice),
    Filter(PreparedFilter),
}
struct Voice {
    handle: VoiceHandle,
    revision: u64,
    dataset: Arc<Dataset>,
    charge: Arc<Charge>,
    latest: Option<[f32; 3]>,
    submitted: Option<[f32; 3]>,
    published: Option<[f32; 3]>,
    task: Option<Task<VoiceProduct>>,
    publisher: Option<FilterPublisher>,
    error: Option<String>,
}
/// Derived state. Dataset/filter pointers never enter project/save JSON.
pub struct BinauralRuntime {
    epoch: Option<u64>,
    jobs: OperationTable<Product>,
    budget: Arc<AtomicUsize>,
    datasets: BTreeMap<u32, (u64, Arc<Dataset>)>,
    preparing: BTreeMap<u32, u64>,
    voices: BTreeMap<Key, Voice>,
    order: VecDeque<Key>,
}
impl Default for BinauralRuntime {
    fn default() -> Self {
        Self {
            epoch: None,
            jobs: OperationTable::new(8, 32).unwrap(),
            budget: Arc::new(AtomicUsize::new(0)),
            datasets: BTreeMap::new(),
            preparing: BTreeMap::new(),
            voices: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}
impl BinauralRuntime {
    pub fn reset(&mut self) {
        self.jobs.clear();
        self.datasets.clear();
        self.preparing.clear();
        self.voices.clear();
        self.order.clear();
        self.epoch = None;
    }
    pub fn has_dataset(&self, owner: u32) -> bool {
        self.datasets.contains_key(&owner)
    }
    pub fn admit_voice(&self, owner: u32) -> Result<()> {
        ensure!(self.has_dataset(owner), "prepare an HRTF dataset first");
        ensure!(
            self.budget
                .load(Ordering::Relaxed)
                .checked_add(VOICE_BYTES)
                .is_some_and(|n| n <= HOST_BYTES),
            "HRTF voice reservation exceeded"
        );
        ensure!(
            self.voices.len() < 64 && self.voices.keys().filter(|k| k.0 == owner).count() < 32,
            "HRTF voice capacity exceeded"
        );
        Ok(())
    }
    fn register(&mut self, h: VoiceHandle) -> Result<()> {
        if self.voices.contains_key(&key(h)) {
            return Ok(());
        }
        self.admit_voice(h.client)?;
        let (revision, dataset) = self
            .datasets
            .get(&h.client)
            .cloned()
            .context("HRTF dataset missing")?;
        let charge = Arc::new(Charge::new(self.budget.clone(), VOICE_BYTES)?);
        let k = key(h);
        self.voices.insert(
            k,
            Voice {
                handle: h,
                revision,
                dataset,
                charge,
                latest: None,
                submitted: None,
                published: None,
                task: None,
                publisher: None,
                error: None,
            },
        );
        self.order.push_back(k);
        Ok(())
    }
    fn remove(&mut self, h: VoiceHandle) {
        let k = key(h);
        self.voices.remove(&k);
        self.order.retain(|v| *v != k);
    }
    fn poll(&mut self, mixer: &mut Mixer, epoch: u64) -> Result<()> {
        if self.epoch.is_some_and(|old| old != epoch) {
            self.reset();
        }
        self.epoch = Some(epoch);
        for (owner, _, ticket) in self.jobs.tickets() {
            let status = self.jobs.status(owner, epoch, ticket)?;
            if status.state == OperationState::Failed {
                if self.preparing.get(&owner) == Some(&ticket) {
                    self.preparing.remove(&owner);
                }
                continue;
            }
            if status.state != OperationState::Ready {
                continue;
            }
            let datasets = &mut self.datasets;
            let preparing = &mut self.preparing;
            let mut replaced = false;
            let _ = self.jobs.publish(owner, epoch, ticket, |product| {
                let Product::Dataset(dataset) = product else {
                    return Ok(product);
                };
                ensure!(
                    preparing.get(&owner) == Some(&ticket),
                    "superseded HRTF dataset"
                );
                let mut metadata = dataset.data.metadata();
                metadata["revision"] = json!(ticket);
                datasets.insert(owner, (ticket, dataset));
                preparing.remove(&owner);
                replaced = true;
                Ok(Product::Summary(metadata))
            });
            if replaced {
                let keys = self
                    .voices
                    .iter()
                    .filter(|(k, _)| k.0 == owner)
                    .map(|(_, v)| v.handle)
                    .collect::<Vec<_>>();
                for h in keys {
                    self.remove(h);
                    if mixer.is_playing(owner, h) {
                        mixer.pending_binaural(owner, h)?;
                    }
                }
            }
        }
        let keys = self.voices.keys().copied().collect::<Vec<_>>();
        for k in keys {
            let h = self.voices[&k].handle;
            if !mixer.is_playing(k.0, h) {
                self.remove(h);
                continue;
            }
            let v = self.voices.get_mut(&k).unwrap();
            if let Some(publisher) = &mut v.publisher {
                publisher.reclaim();
            }
            let Some(task) = &mut v.task else {
                continue;
            };
            match task.try_take() {
                Ok(None) => {}
                Ok(Some(product)) => {
                    v.task = None;
                    let result = match product {
                        VoiceProduct::Voice(publisher, voice) => {
                            let result = mixer.set_binaural(k.0, h, Some(voice));
                            if result.is_ok() {
                                v.publisher = Some(publisher);
                            }
                            result
                        }
                        VoiceProduct::Filter(filter) => v
                            .publisher
                            .as_mut()
                            .context("HRTF publisher missing")
                            .and_then(|p| p.publish(filter).map_err(|e| anyhow::anyhow!("{e}"))),
                    };
                    match result {
                        Ok(()) => {
                            v.published = v.submitted;
                            v.error = None;
                        }
                        Err(error) => v.error = Some(format!("{error:#}")),
                    }
                }
                Err(error) => {
                    v.task = None;
                    v.error = Some(format!("{error:#}"));
                }
            }
        }
        Ok(())
    }
}
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    let epoch = state.runtime.restored;
    let a = &mut state.mixer_audio;
    a.binaural.poll(&mut a.mixer, epoch)
}
pub(crate) fn enable(state: &mut EngineState, h: VoiceHandle) -> Result<()> {
    let a = &mut state.mixer_audio;
    if a.binaural.voices.contains_key(&key(h)) {
        return Ok(());
    }
    a.binaural.register(h)?;
    a.mixer.pending_binaural(h.client, h)
}
pub(crate) fn queue(
    state: &mut EngineState,
    h: VoiceHandle,
    source: [f32; 3],
    listener: Listener,
) -> Result<()> {
    let direction = crysirror_acoustics::binaural::listener_direction(
        source,
        listener.position,
        listener.forward,
        listener.up,
    )?;
    let a = &mut state.mixer_audio;
    if !a.binaural.voices.contains_key(&key(h)) {
        a.binaural.register(h)?;
        a.mixer.pending_binaural(h.client, h)?;
    }
    a.binaural.voices.get_mut(&key(h)).unwrap().latest = Some(direction);
    Ok(())
}
pub(crate) fn dispatch(state: &mut EngineState) -> Result<()> {
    let pool = state.runtime.assets.cpu_tasks();
    let runtime = &mut state.mixer_audio.binaural;
    let mut flight = runtime.voices.values().filter(|v| v.task.is_some()).count();
    for _ in 0..runtime.order.len() {
        if flight >= 16 {
            break;
        }
        let k = runtime.order.pop_front().unwrap();
        runtime.order.push_back(k);
        let Some(v) = runtime.voices.get_mut(&k) else {
            continue;
        };
        let Some(direction) = v.latest else {
            continue;
        };
        if v.task.is_some() || v.error.is_some() || v.submitted == Some(direction) {
            continue;
        }
        let dataset = v.dataset.clone();
        let charge = v.charge.clone();
        let initial = v.publisher.is_none();
        match pool.spawn(move |cancel| {
            cancel.check()?;
            let product = if initial {
                let (publisher, mut voice) = dataset.data.voice(direction)?;
                voice.retain_storage(charge);
                VoiceProduct::Voice(publisher, voice)
            } else {
                VoiceProduct::Filter(dataset.data.prepare_filter(direction)?)
            };
            cancel.check()?;
            Ok(product)
        }) {
            Ok(task) => {
                v.submitted = Some(direction);
                v.task = Some(task);
                flight += 1;
            }
            Err(error) if error.is::<crysirror_tasks::CapacityExceeded>() => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
fn arg(args: &[Value], index: usize) -> Result<&Value> {
    args.get(index).context("HRTF argument required")
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginHrtf"
            | "hrtfStatus"
            | "cancelHrtfJob"
            | "clearHrtf"
            | "hrtfStats"
            | "hrtfVoice"
            | "setVoiceHrtf"
    ) {
        return Ok(None);
    }
    poll(state)?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    if method == "beginHrtf" {
        let name = arg(args, 0)?.as_str()?;
        let path = VfsPath::new(Mount::Asset, name).map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(path.relative() == name, "canonical HRTF asset required");
        let options: HrtfOptions = args
            .get(1)
            .map(|v| serde_json::from_value(v.to_json()))
            .transpose()?
            .unwrap_or_default();
        options.validate()?;
        let pool = state.runtime.assets.cpu_tasks();
        let rate = state.mixer_audio.mixer.sample_rate();
        let storage = state
            .runtime
            .host
            .fs
            .storage(state.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let r = &mut state.mixer_audio.binaural;
        r.jobs.admit(owner)?;
        let mut charge = Charge::new(r.budget.clone(), DATASET_BYTES)?;
        let ticket = r
            .jobs
            .submit(owner, epoch, "hrtf_dataset", &pool, move |cancel| {
                cancel.check()?;
                let reader = storage
                    .open_asset(&path)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let bytes = crysirror_resource::read_all(reader.as_ref(), 32 * 1024 * 1024)?;
                cancel.check()?;
                let data = HrtfDataset::from_bytes(&bytes, rate, options)?;
                charge.shrink(data.estimated_bytes() + 1024 * 1024);
                cancel.check()?;
                Ok(Product::Dataset(Arc::new(Dataset {
                    data,
                    _charge: charge,
                })))
            })?;
        if let Some(old) = r.preparing.insert(owner, ticket) {
            let _ = r.jobs.cancel(owner, old);
        }
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if matches!(method, "hrtfStatus" | "cancelHrtfJob") {
        let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
        let r = &mut state.mixer_audio.binaural;
        if method == "cancelHrtfJob" {
            r.jobs.cancel(owner, ticket)?;
            r.preparing.retain(|_, v| *v != ticket);
            return Ok(Some(Value::Null));
        }
        if r.jobs.status(owner, epoch, ticket)?.state == OperationState::Ready {
            poll(state)?;
        }
        let r = &mut state.mixer_audio.binaural;
        let mut result = serde_json::to_value(r.jobs.status(owner, epoch, ticket)?)?;
        if result["state"] == "ready" {
            if let Some(product) = r.jobs.poll(owner, epoch, ticket)? {
                let Product::Summary(data) = product else {
                    anyhow::bail!("HRTF dataset publication pending");
                };
                result["result"] = data.clone();
            }
        }
        return Ok(Some(Value::from_json(result)));
    }
    if method == "clearHrtf" {
        let a = &mut state.mixer_audio;
        let r = &mut a.binaural;
        r.datasets.remove(&owner);
        if let Some(ticket) = r.preparing.remove(&owner) {
            let _ = r.jobs.cancel(owner, ticket);
        }
        r.voices.retain(|k, _| k.0 != owner);
        r.order.retain(|k| k.0 != owner);
        for v in a
            .attachments
            .values_mut()
            .filter(|a| a.handle.client == owner && a.binaural)
        {
            v.binaural = false;
            if a.mixer.is_playing(owner, v.handle) {
                a.mixer.set_binaural(owner, v.handle, None)?;
            }
        }
        return Ok(Some(Value::Null));
    }
    if matches!(method, "hrtfVoice" | "setVoiceHrtf") {
        let h: VoiceHandle = serde_json::from_value(arg(args, 0)?.to_json())?;
        ensure!(h.client == owner, "foreign HRTF voice handle");
        let spec = state.mixer_audio.mixer.spec(owner, h)?;
        if method == "setVoiceHrtf" {
            let enabled = arg(args, 1)?.as_bool()?;
            if enabled {
                let spatial = spec.spatial.context("HRTF requires spatial audio")?;
                enable(state, h)?;
                state
                    .mixer_audio
                    .attachments
                    .entry(key(h))
                    .or_insert(Attachment {
                        handle: h,
                        follow: None,
                        spatial,
                        raycast: false,
                        acoustics: None,
                        binaural: false,
                    })
                    .binaural = true;
            } else {
                let a = &mut state.mixer_audio;
                a.binaural.remove(h);
                a.mixer.set_binaural(owner, h, None)?;
                if let Some(v) = a.attachments.get_mut(&key(h)) {
                    v.binaural = false;
                }
            }
            return Ok(Some(Value::Null));
        }
        let r = &state.mixer_audio.binaural;
        let result=r.voices.get(&key(h)).map(|v|json!({"revision":v.revision,"ready":v.publisher.is_some(),"pending":v.error.is_none() && (v.publisher.is_none() || v.task.is_some() || v.latest!=v.published),"published_direction":v.published,"error":v.error}));
        return Ok(Some(Value::from_json(
            json!({"enabled":result.is_some(),"result":result}),
        )));
    }
    let r = &state.mixer_audio.binaural;
    let scene = r.datasets.get(&owner).map(|(revision, dataset)| {
        let mut data = dataset.data.metadata();
        data["revision"] = json!(revision);
        data
    });
    Ok(Some(Value::from_json(
        json!({"dataset":scene,"preparing":r.preparing.contains_key(&owner),"voices":r.voices.keys().filter(|k|k.0==owner).count(),"pending_sources":r.voices.iter().filter(|(k,v)|k.0==owner && v.task.is_some()).count(),"host_reserved_bytes":r.budget.load(Ordering::Relaxed),"host_budget_bytes":HOST_BYTES}),
    )))
}
