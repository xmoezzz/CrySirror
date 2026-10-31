// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Persistent pattern jobs share the image worker pool and memory reservations.
use crate::image_processing::{self, Charge};
use crysirror_core::{service::*, state::ClientId, Error, Result};
use crysirror_image::{
    temporal_pattern::{PatternRng, TemporalPattern, TemporalPatternSnapshot, TemporalPatternSpec},
    DecoderRegistry,
};
use crysirror_tasks::{Task, TaskPool};
use std::sync::{atomic::AtomicUsize, Arc};
const MAX_PIXELS: u64 = 4 * 1024 * 1024;
const MAX_SNAPSHOT: usize = 16 * 1024 * 1024;
struct Resource {
    client: ClientId,
    host: Arc<()>,
    pattern: Arc<TemporalPattern>,
    snapshot: Arc<[u8]>,
    _charge: Charge,
}
impl std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatternResource")
            .field("info", &self.info())
            .finish()
    }
}
impl PatternResource for Resource {
    fn client(&self) -> ClientId {
        self.client
    }
    fn info(&self) -> PatternResourceInfo {
        PatternResourceInfo {
            client: self.client,
            size: self.pattern.spec().size,
            completed_steps: self.pattern.completed_steps(),
            resident_bytes: self.pattern.resident_bytes() + self.snapshot.len(),
        }
    }
    fn snapshot(&self) -> Arc<[u8]> {
        self.snapshot.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
struct Job {
    client: ClientId,
    task: Option<Task<PatternOutput>>,
    state: PatternProcessState,
}
impl PatternProcess for Job {
    fn client(&self) -> ClientId {
        self.client
    }
    fn poll(&mut self) -> Result<PatternProcessState> {
        if let Some(task) = &mut self.task {
            match task.try_take() {
                Ok(None) => {}
                Ok(Some(output)) => {
                    self.state = PatternProcessState::Ready(output);
                    self.task = None;
                }
                Err(error) => {
                    self.state = PatternProcessState::Failed(error.to_string());
                    self.task = None;
                }
            }
        }
        Ok(self.state.clone())
    }
}
fn io(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}
fn validate_spec(spec: &TemporalPatternSpec) -> Result<usize> {
    spec.validate().map_err(io)?;
    if u64::from(spec.size[0]) * u64::from(spec.size[1]) > MAX_PIXELS {
        return Err(Error::Limit("persistent pattern pixels"));
    }
    spec.state_bytes().map_err(io)
}
fn rng(bytes: &[u8]) -> Result<PatternRng> {
    if bytes.len() > 1024 {
        return Err(Error::Limit("pattern RNG specification"));
    }
    let value: PatternRng = serde_json::from_slice(bytes).map_err(io)?;
    value.validate().map_err(io)?;
    Ok(value)
}
struct Held {
    pattern: Arc<TemporalPattern>,
    _lease: Arc<dyn PatternResource>,
}
impl std::ops::Deref for Held {
    type Target = TemporalPattern;
    fn deref(&self) -> &Self::Target {
        &self.pattern
    }
}
fn source(resource: &Arc<dyn PatternResource>, owner: ClientId, host: &Arc<()>) -> Result<Held> {
    let typed = resource
        .as_any()
        .downcast_ref::<Resource>()
        .ok_or(Error::Invalid("unknown pattern resource"))?;
    if typed.client != owner || resource.client() != typed.client || !Arc::ptr_eq(host, &typed.host)
    {
        return Err(Error::Invalid("foreign pattern resource"));
    }
    Ok(Held {
        pattern: typed.pattern.clone(),
        _lease: resource.clone(),
    })
}
enum Work {
    Create(TemporalPatternSpec, PatternRng),
    Advance(Held, PatternRng,Option<usize>),
    Restore(Arc<[u8]>, PatternRng),
    Render(
        Held,
        crysirror_core::image_processing::ImageInput,
        [u32; 4],
        u16,
    ),
}
pub(crate) struct Context {
    pub owner: ClientId,
    pub host: Arc<()>,
    pub tasks: TaskPool,
    pub memory: Arc<AtomicUsize>,
    pub storage: Arc<dyn ClientStorage>,
    pub decoders: DecoderRegistry,
}
impl Context {
    pub fn begin(self, request: PatternRequest) -> Result<PatternJob> {
        let (work, output_bytes, scratch_bytes) = match request {
            PatternRequest::Create { spec, rng: state } => {
                if spec.len() > 4096 {
                    return Err(Error::Limit("pattern specification"));
                }
                let spec: TemporalPatternSpec = serde_json::from_slice(&spec).map_err(io)?;
                let bytes = validate_spec(&spec)?;
                (
                    Work::Create(spec, rng(&state)?),
                    bytes * 4 + 4096,
                    bytes * 4 + 4096,
                )
            }
            PatternRequest::Advance {
                source: input,
                rng: state,
            } => {
                let pattern = source(&input, self.owner, &self.host)?;
                let bytes = validate_spec(pattern.spec())?;
                (
                    Work::Advance(pattern, rng(&state)?,None),
                    bytes * 4 + 4096,
                    bytes * 4 + 4096,
                )
            }
            PatternRequest::AdvancePrefix{source:input,rng:state,words}=>{
                let pattern=source(&input,self.owner,&self.host)?;
                let bytes=validate_spec(pattern.spec())?;
                if words>bytes/4{return Err(Error::Invalid("pattern prefix bounds"));}
                (Work::Advance(pattern,rng(&state)?,Some(words)),bytes*4+4096,bytes*4+4096)
            }
            PatternRequest::Restore {
                snapshot,
                rng: state,
            } => {
                if snapshot.len() > MAX_SNAPSHOT {
                    return Err(Error::Limit("pattern snapshot"));
                }
                (
                    Work::Restore(snapshot, rng(&state)?),
                    MAX_SNAPSHOT + 4096,
                    64 * 1024 * 1024,
                )
            }
            PatternRequest::Render {
                source: input,
                destination,
                rect,
                opacity256,
            } => {
                let pattern = source(&input, self.owner, &self.host)?;
                validate_spec(pattern.spec())?;
                let mut nodes = 0;
                let mut bytes = 0;
                let size = image_processing::input_size(
                    self.owner,
                    &destination,
                    0,
                    &mut nodes,
                    &mut bytes,
                )?;
                if size != pattern.spec().size
                    || opacity256 > 256
                    || rect[2] == 0
                    || rect[3] == 0
                    || rect[0].checked_add(rect[2]).is_none_or(|v| v > size[0])
                    || rect[1].checked_add(rect[3]).is_none_or(|v| v > size[1])
                    || rect[2] % (4 * pattern.spec().cell[0]) != 0
                    || rect[3] % pattern.spec().cell[1] != 0
                {
                    return Err(Error::Invalid("pattern render context"));
                }
                let output = crysirror_image::rgba_len(size[0], size[1]).map_err(io)?;
                (
                    Work::Render(pattern, destination, rect, opacity256),
                    output,
                    output * 3 + bytes + 128 * 1024 * 1024,
                )
            }
        };
        let output = Charge::new(self.memory.clone(), output_bytes)?;
        let scratch = Charge::new(self.memory, scratch_bytes)?;
        let permit = self.tasks.reserve().map_err(io)?;
        let owner = self.owner;
        let task = permit
            .spawn(move |cancel| {
                let _scratch = scratch;
                cancel.check()?;
                let (pattern, state) = match work {
                    Work::Create(spec, mut state) => {
                        let pattern = TemporalPattern::new(spec, &mut state, &|| cancel.check())?;
                        (pattern, state)
                    }
                    Work::Advance(old, mut state,prefix) => {
                        let mut pattern = (*old).clone();
                        if let Some(prefix)=prefix{pattern.advance_prefix(prefix,&mut state,&||cancel.check())?;}else{pattern.advance(&mut state, &|| cancel.check())?;}
                        (pattern, state)
                    }
                    Work::Restore(data, state) => {
                        let snapshot: TemporalPatternSnapshot = serde_json::from_slice(&data)?;
                        validate_spec(&snapshot.spec).map_err(|e| anyhow::anyhow!("{e}"))?;
                        (
                            TemporalPattern::from_snapshot(snapshot, &|| cancel.check())?,
                            state,
                        )
                    }
                    Work::Render(pattern, input, rect, opacity) => {
                        let base = image_processing::load(
                            owner,
                            &input,
                            &self.storage,
                            &self.decoders,
                            &cancel,
                        )?;
                        let image = pattern.composite(&base, rect, opacity, &|| cancel.check())?;
                        return Ok(PatternOutput::Image(
                            image_processing::resource_from_surface(owner, image, output, &|| {
                                cancel.check()
                            })?,
                        ));
                    }
                };
                cancel.check()?;
                let data = serde_json::to_vec(&pattern.snapshot())?;
                anyhow::ensure!(
                    data.len() <= MAX_SNAPSHOT,
                    "pattern encoded snapshot budget"
                );
                let rng_after: Arc<[u8]> = serde_json::to_vec(&state)?.into();
                let resource: Arc<dyn PatternResource> = Arc::new(Resource {
                    client: owner,
                    host: self.host,
                    pattern: Arc::new(pattern),
                    snapshot: data.into(),
                    _charge: output,
                });
                cancel.check()?;
                Ok(PatternOutput::Resource {
                    resource,
                    rng_after,
                })
            })
            .map_err(io)?;
        Ok(Box::new(Job {
            client: owner,
            task: Some(task),
            state: PatternProcessState::Pending,
        }))
    }
}
