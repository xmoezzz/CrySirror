// SPDX-License-Identifier: MPL-2.0
//! Public API over the same retained canvas and media services native VMs use.
//! No interpreter, private renderer or duplicated playback state lives here.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, TypeHint as T},
    value::Value,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{HostServices, MediaRequest};
use crysirror_scene::{ServiceBatch, canvas::CanvasCommand};
use serde::Deserialize;
use std::sync::{Arc, OnceLock};

pub fn register(classes: &mut ClassRegistry) -> Result<()> {
    classes.register(Arc::new(CanvasesClass))?;
    classes.register(Arc::new(MediaClass))
}
fn method(
    name: &'static str,
    params: Vec<(&'static str, T)>,
    returns: T,
    doc: &'static str,
) -> MethodInfo {
    MethodInfo {
        name,
        params,
        returns,
        doc,
    }
}
fn arg<'a>(args: &'a [Value], index: usize, name: &str) -> Result<&'a Value> {
    args.get(index)
        .with_context(|| format!("missing argument '{name}'"))
}
fn id(args: &[Value]) -> Result<u32> {
    u32::try_from(arg(args, 0, "id")?.as_i64()?).context("resource id must be a u32")
}
fn json(value: impl serde::Serialize) -> Result<Value> {
    Ok(Value::from_json(serde_json::to_value(value)?))
}

pub struct CanvasesClass;
impl Class for CanvasesClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo {
            name:"Canvases", doc:"Client-scoped retained drawing canvases backed by inspectable engine Layer nodes.",
            properties:vec![], methods:vec![
                method("apply",vec![("commands",T::List(Box::new(T::Map)))],T::Void,
                    "Atomically apply typed canvas commands: create, clear, draw, set, copy, present and hide. Draws contain ordinary Layer specs, including positioned glyph runs."),
                method("get",vec![("id",T::Int)],T::Optional(Box::new(T::Map)),
                    "Inspect this client's canvas dimensions and logical draw nodes, or null when absent. No flattened native frame is returned."),
            ],
        })
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, _: &str) -> Result<Value> {
        unreachable!()
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        match name {
            "apply" => {
                let Value::List(values) = arg(args, 0, "commands")? else {
                    anyhow::bail!("canvas commands must be a list");
                };
                ensure!(values.len() <= 4096, "canvas command budget");
                let data = serde_json::to_vec(&values)?;
                ensure!(
                    data.len() <= 4 * 1024 * 1024,
                    "canvas command payload exceeds 4 MiB"
                );
                let commands: Vec<CanvasCommand> =
                    serde_json::from_slice(&data).context("canvas command data")?;
                let state = cx.state()?;
                state.runtime.host.engine.submit(
                    state.client,
                    &ServiceBatch {
                        canvases: commands,
                        ..Default::default()
                    },
                )?;
                Ok(Value::Null)
            }
            "get" => {
                let id = id(args)?;
                let state = cx.state()?;
                match state.runtime.host.engine.canvases.surface(state.client, id) {
                    Some(surface) => json(surface),
                    None => Ok(Value::Null),
                }
            }
            _ => unreachable!(),
        }
    }
}

fn yes() -> bool {
    true
}
fn gain() -> f32 {
    1.
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayOptions {
    #[serde(default, rename = "loop")]
    looping: bool,
    #[serde(default = "yes")]
    audio: bool,
    #[serde(default = "gain")]
    gain: f32,
}
impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            looping: false,
            audio: true,
            gain: 1.,
        }
    }
}
pub struct MediaClass;
impl Class for MediaClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo {
            name:"Media",doc:"Shared asynchronous media resources; decoding, clocks and errors belong to the engine.",
            properties:vec![],methods:vec![
                method("play",vec![("id",T::Int),("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::String,
                    "Start a scoped media resource; options {loop?:false,audio?:true,gain?:1}. Returns its logical image resource path for Layer/Canvas/Material APIs; inspect status for loading, playback and errors."),
                method("stop",vec![("id",T::Int)],T::Void,"Stop only this client's media resource."),
                method("remove",vec![("id",T::Int)],T::Void,"Release this client's media resource and decoded surface after detaching its image references."),
                method("pause",vec![("id",T::Int)],T::Void,"Freeze the media clock; audio stops and the current picture stays."),
                method("resume",vec![("id",T::Int)],T::Void,"Continue a paused media clock; audio restarts at the matching offset."),
                method("setClock",vec![("id",T::Int),("nowMs",T::Any)],T::Void,"Select a monotonic external video clock before first playback, then supply clock samples. Exact nonnegative integers or decimal strings are accepted. The engine owns pause/seek/loop offsets; decoded audio tracks require audio-master mapping and currently fail explicitly."),
                method("seek",vec![("id",T::Int),("position_ms",T::Int)],T::Void,
                    "Restart decoding and present the picture due at position_ms (stream time); earlier pictures are decoded but not published. A paused resource stays paused there."),
                method("status",vec![("id",T::Int)],T::Optional(Box::new(T::Map)),
                    "Media state, position_ms, frame_pts_ms, published/decoded frames, audio_started, loops_completed, paused, validated size, declared duration_ms and error, or null when absent. Size/duration remain null while pending or unknown."),
            ],
        })
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, _: &str) -> Result<Value> {
        unreachable!()
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        let id = id(args)?;
        match name {
            "play" => {
                let asset = arg(args, 1, "asset")?.as_str()?.to_owned();
                let options = match args.get(2).filter(|v| !matches!(v, Value::Null)) {
                    Some(value) => serde_json::from_value::<PlayOptions>(value.to_json())
                        .context("media options")?,
                    None => PlayOptions::default(),
                };
                ensure!(
                    options.gain.is_finite() && (0.0..=4.0).contains(&options.gain),
                    "media gain must be finite and 0..4"
                );
                let state = cx.state()?;
                let path = state
                    .runtime
                    .host
                    .media_request(
                        state.client,
                        MediaRequest::Play {
                            id,
                            asset,
                            looping: options.looping,
                            audio: options.audio,
                            gain: options.gain,
                        },
                    )
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(path.relative().to_owned().into())
            }
            "stop" | "remove" => {
                let state = cx.state()?;
                state
                    .runtime
                    .host
                    .media_request(state.client, if name == "remove" { MediaRequest::Remove { id } } else { MediaRequest::Stop { id } })
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(Value::Null)
            }
            "pause" | "resume" | "seek" | "setClock" => {
                let request = match name {
                    "pause" => MediaRequest::Pause { id },
                    "resume" => MediaRequest::Resume { id },
                    "setClock" => {
                        let value = arg(args, 1, "nowMs")?;
                        let now_ms = if let Value::Str(text) = value {
                            text.parse::<u64>().context("decimal media clock")?
                        } else {
                            let n = value.as_i64()?;
                            ensure!((0..=9_007_199_254_740_991).contains(&n), "media clock requires an exact nonnegative integer or decimal string");
                            n as u64
                        };
                        MediaRequest::SetClock { id, now_ms }
                    }
                    _ => MediaRequest::Seek {
                        id,
                        position_ms: u64::try_from(arg(args, 1, "position_ms")?.as_i64()?)
                            .context("media seek position must be non-negative")?,
                    },
                };
                let state = cx.state()?;
                state
                    .runtime
                    .host
                    .media_request(state.client, request)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(Value::Null)
            }
            "status" => {
                let state = cx.state()?;
                match state.runtime.host.media.status(state.client, id) {
                    Some(status) => json(status),
                    None => Ok(Value::Null),
                }
            }
            _ => unreachable!(),
        }
    }
}
