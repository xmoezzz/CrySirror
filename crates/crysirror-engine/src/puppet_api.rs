// SPDX-License-Identifier: MPL-2.0
//! `Puppets`: drives 2D puppet layers (ADR 0029). Names are checked against
//! the layer's rig before the command is submitted, so a typo fails at the
//! call instead of at the next tick.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::state::ClientId;
use crysirror_scene::{
    layer::{LayerCommand, PuppetCommand},
    ServiceBatch,
};
use std::sync::{Arc, OnceLock};

pub struct PuppetsClass;

fn m(name: &'static str, params: Vec<(&'static str, T)>, returns: T, doc: &'static str) -> MethodInfo {
    MethodInfo { name, params, returns, doc }
}
fn info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Puppets",
        doc: "2D deformable puppets on layers whose paint is {kind:'puppet', asset} (engine rig JSON; import E-mote with `crysirror puppet-import`).",
        properties: vec![PropertyInfo { name: "count", ty: T::Int, writable: false, doc: "Puppet layers owned by this client." }],
        methods: vec![
            m("setParameter", vec![("layer", T::Int), ("parameter", T::String), ("value", T::Number), ("durationMs", T::Optional(Box::new(T::Number)))], T::Void,
                "Ease a parameter to value (clamped to its range) and hold it, overriding motions and physics."),
            m("releaseParameter", vec![("layer", T::Int), ("parameter", T::String)], T::Void, "Return a parameter to motion/physics control."),
            m("play", vec![("layer", T::Int), ("motion", T::String), ("options", T::Optional(Box::new(T::Map)))], T::Void,
                "Start (or restart) a motion; options {looping?: boolean, weight?: 0..1}."),
            m("stop", vec![("layer", T::Int), ("motion", T::Optional(Box::new(T::String)))], T::Void, "Fade a motion out; no name stops all motions."),
            m("parameters", vec![("layer", T::Int)], T::List(Box::new(T::Map)), "Rig parameters [{id, min, max, default, value}]."),
            m("motions", vec![("layer", T::Int)], T::List(Box::new(T::Map)), "Rig motions [{id, duration_ms, looping, playing}]."),
            m("value", vec![("layer", T::Int), ("parameter", T::String)], T::Number, "Current evaluated value of a parameter."),
        ],
    })
}

fn layer_arg(args: &[Value]) -> Result<u32> {
    Ok(u32::try_from(args.first().context("missing argument 'layer'")?.as_i64()?)?)
}
fn str_arg<'a>(args: &'a [Value], i: usize, what: &str) -> Result<&'a str> {
    args.get(i).with_context(|| format!("missing argument '{what}'"))?.as_str()
}

/// The rig and saved state of one of this client's puppet layers.
fn rig_of(cx: &mut EngineContext, layer: u32) -> Result<(Arc<crysirror_puppet::PuppetRig>, Option<crysirror_puppet::PuppetState>)> {
    let s = cx.state()?;
    let client = s.client;
    let slot = s.runtime.host.engine.layers.puppet_slot(client.0, layer).with_context(|| format!("layer {layer} is not a puppet layer"))?.clone();
    let rig = s.runtime.puppet_rig(ClientId(client.0), &slot.asset)?;
    Ok((rig, slot.state))
}

impl Class for PuppetsClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        ensure!(p == "count", "Puppets.{p}: no such property");
        let s = cx.state()?;
        let c = s.client.0;
        Ok((s.runtime.host.engine.layers.puppet_layers().iter().filter(|(owner, ..)| *owner == c).count() as u64).into())
    }
    fn call(&self, cx: &mut EngineContext, _: ObjectId, method: &str, args: &[Value]) -> Result<Value> {
        let layer = layer_arg(args)?;
        let (rig, state) = rig_of(cx, layer)?;
        let value_of = |id: &str| -> f32 {
            state.as_ref().and_then(|s| s.values.get(id).copied()).unwrap_or_else(|| rig.parameter(id).map_or(0., |p| p.default))
        };
        let command = match method {
            "parameters" => {
                return Ok(Value::from_json(serde_json::Value::Array(
                    rig.parameters
                        .iter()
                        .map(|p| serde_json::json!({"id": p.id, "min": p.min, "max": p.max, "default": p.default, "value": value_of(&p.id)}))
                        .collect(),
                )))
            }
            "motions" => {
                let playing: Vec<&str> = state.as_ref().map_or(Vec::new(), |s| s.motions.iter().map(|m| m.id.as_str()).collect());
                return Ok(Value::from_json(serde_json::Value::Array(
                    rig.motions
                        .iter()
                        .map(|m| serde_json::json!({"id": m.id, "duration_ms": m.duration_ms, "looping": m.looping, "playing": playing.contains(&m.id.as_str())}))
                        .collect(),
                )))
            }
            "value" => {
                let id = str_arg(args, 1, "parameter")?;
                ensure!(rig.parameter(id).is_some(), "unknown puppet parameter '{id}'");
                return Ok(Value::Float(value_of(id) as f64));
            }
            "setParameter" => {
                let parameter = str_arg(args, 1, "parameter")?.to_owned();
                ensure!(rig.parameter(&parameter).is_some(), "unknown puppet parameter '{parameter}'");
                let value = args.get(2).context("missing argument 'value'")?.as_f64()? as f32;
                let duration_ms = match args.get(3) {
                    None | Some(Value::Null) => 0.,
                    Some(v) => v.as_f64()? as f32,
                };
                PuppetCommand::SetParameter { parameter, value, duration_ms }
            }
            "releaseParameter" => {
                let parameter = str_arg(args, 1, "parameter")?.to_owned();
                ensure!(rig.parameter(&parameter).is_some(), "unknown puppet parameter '{parameter}'");
                PuppetCommand::ReleaseParameter { parameter }
            }
            "play" => {
                let motion = str_arg(args, 1, "motion")?.to_owned();
                ensure!(rig.motions.iter().any(|m| m.id == motion), "unknown puppet motion '{motion}'");
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Options {
                    looping: Option<bool>,
                    weight: Option<f32>,
                }
                let o: Options = match args.get(2) {
                    None | Some(Value::Null) => Options { looping: None, weight: None },
                    Some(v) => serde_json::from_value(v.to_json()).context("invalid play options")?,
                };
                PuppetCommand::Play { motion, looping: o.looping, weight: o.weight.unwrap_or(1.) }
            }
            "stop" => {
                let motion = match args.get(1) {
                    None | Some(Value::Null) => String::new(),
                    Some(v) => v.as_str()?.to_owned(),
                };
                ensure!(motion.is_empty() || rig.motions.iter().any(|m| m.id == motion), "unknown puppet motion '{motion}'");
                PuppetCommand::Stop { motion }
            }
            other => bail!("Puppets.{other}(): no such method"),
        };
        let s = cx.state()?;
        let client = s.client;
        s.runtime.host.engine.submit(client, &ServiceBatch { layers: vec![LayerCommand::Puppet { id: layer, command }], ..Default::default() })?;
        Ok(Value::Null)
    }
}
