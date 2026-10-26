// SPDX-License-Identifier: MPL-2.0
//! VFX authoring and instance controls through the ordinary engine object API.
use crate::{
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, TypeHint as T},
    EngineContext, Value,
};
use anyhow::{bail, Context, Result};
use std::sync::{Arc, OnceLock};
pub fn register(r: &mut ClassRegistry) -> Result<()> {
    r.register(Arc::new(VfxClass))
}
struct VfxClass;
impl Class for VfxClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"Vfx",doc:"Engine-owned modular visual effects. Systems are data, particles execute on bounded CPU workers or native GPU compute; JS/TS controls instances and parameters.",properties:vec![],methods:vec![
            MethodInfo{name:"compile",params:vec![("system",T::Map)],returns:T::Map,doc:"Validate a modular system and typed parameter bindings without opening a device. Returns its emitter/module/target summary; unsupported modules fail."},
            MethodInfo{name:"play",params:vec![("system",T::Map),("options",T::Optional(Box::new(T::Map)))],returns:T::Object("Entity"),doc:"Spawn an entity with this VFX system. Options are ordinary World.spawn fields such as name, position, rotation, scale and parent. Simulation is published asynchronously."},
            MethodInfo{name:"setParameter",params:vec![("entity",T::Object("Entity")),("name",T::String),("value",T::Any)],returns:T::Void,doc:"Change an existing typed parameter on this client's VFX entity without restarting its particles."},
            MethodInfo{name:"pause",params:vec![("entity",T::Object("Entity")),("paused",T::Bool)],returns:T::Void,doc:"Freeze/resume the effect's simulation clock."},
            MethodInfo{name:"stop",params:vec![("entity",T::Object("Entity"))],returns:T::Void,doc:"Stop emission while existing particles finish."},
            MethodInfo{name:"restart",params:vec![("entity",T::Object("Entity"))],returns:T::Void,doc:"Restart an effect from its seed using a new logical revision."},
            MethodInfo{name:"stats",params:vec![],returns:T::Map,doc:"Read this client's published CPU counts, GPU timeline pressure and pending worker status. GPU capacity is not reported as a measured live count."},
            MethodInfo{name:"drainEvents",params:vec![],returns:T::List(Box::new(T::Map)),doc:"Drain this client's bounded CPU birth, death and collision event queue. GPU event records remain on device; they are not fabricated as CPU callbacks."},
        ]})
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, name: &str) -> Result<Value> {
        bail!("Vfx has no property {name}")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        match name {
            "compile" | "play" => {
                let system: crysirror_vfx::SystemSpec =
                    serde_json::from_value(args.first().context("VFX system")?.to_json())?;
                let plan = system.compile()?;
                if name == "compile" {
                    return Ok(Value::from_json(
                        serde_json::json!({"emitters":plan.spec.emitters.iter().map(|e|serde_json::json!({"id":e.id,"target":e.target,"capacity":e.capacity,"modules":e.update.len(),"render":e.render})).collect::<Vec<_>>(),"fixed_step_hz":plan.spec.fixed_step_hz,"parameters":plan.spec.parameters}),
                    ));
                }
                let mut options = args
                    .get(1)
                    .filter(|v| !matches!(v, Value::Null))
                    .map(Value::to_json)
                    .unwrap_or_else(|| serde_json::json!({}));
                let options = options
                    .as_object_mut()
                    .context("VFX play options must be a map")?;
                options.insert("vfx".into(), serde_json::to_value(system)?);
                options.entry("name").or_insert_with(|| "VFX".into());
                let value = Value::from_json(serde_json::Value::Object(options.clone()));
                let world = cx.singleton("World")?;
                cx.call(world, "spawn", &[value])
            }
            "setParameter" | "pause" | "stop" | "restart" => {
                let entity = args.first().context("VFX entity")?.as_object()?;
                let method = match name {
                    "setParameter" => "setVfxParameter",
                    "pause" => "pauseVfx",
                    "stop" => "stopVfx",
                    _ => "restartVfx",
                };
                cx.call(entity, method, &args[1..])
            }
            "stats" => {
                let s = cx.state()?;
                Ok(Value::from_json(s.runtime.vfx.inspect(Some(s.client.0))))
            }
            "drainEvents" => {
                let s = cx.state()?;
                let events = s.runtime.vfx.drain_events(s.client.0);
                Ok(Value::from_json(serde_json::to_value(events)?))
            }
            _ => bail!("unknown Vfx method {name}"),
        }
    }
}
