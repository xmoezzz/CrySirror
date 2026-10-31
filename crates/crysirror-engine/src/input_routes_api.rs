// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Public receiver/receipt operations over the canonical shared Scene router.
use crate::{
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, TypeHint as T},
    EngineContext, Value,
};
use anyhow::{Context, Result};
use crysirror_core::service::{EngineQuery, HostServices};
use crysirror_scene::input_routes::InputRouteCommand;
use std::sync::{Arc, OnceLock};
pub(crate) fn register(registry: &mut ClassRegistry) -> Result<()> {
    registry.register(Arc::new(Routes))
}
fn m(
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
fn arg<'a>(args: &'a [Value], index: usize) -> Result<&'a Value> {
    args.get(index).context("input route argument")
}
fn id(args: &[Value]) -> Result<u32> {
    Ok(u32::try_from(arg(args, 0)?.as_i64()?)?)
}
struct Routes;
impl Class for Routes {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"InputRoutes",doc:"Client-scoped device-event receivers with stable priority, explicit delivery receipts, capture and cancellation. Interactive Widget mixing and simultaneous active receiver clients are explicitly unsupported in the current routing profile.",properties:vec![],methods:vec![
            m("put",vec![("receiver",T::Map)],T::Void,"Register/update a bounded receiver with signed priority, source mask, optional half-open pointer rect/coverage and enabled/hover flags."),
            m("remove",vec![("id",T::Int)],T::Void,"Remove an owned receiver; stale capture/delivery cannot activate a different receiver."),
            m("clear",vec![],T::Void,"Clear this client's receivers and pending input state."),
            m("setEnabled",vec![("id",T::Int),("enabled",T::Bool)],T::Void,"Enable or disable a registered receiver."),
            m("setPriority",vec![("id",T::Int),("priority",T::Int)],T::Void,"Set signed priority, high first, preserving equal-priority stable history."),
            m("delivery",vec![],T::Optional(Box::new(T::Map)),"Lease the next routed event. Repeated polls retain the same receipt until acknowledgement; u64 receipt fields are exact decimal strings."),
            m("acknowledge",vec![("receipt",T::Map),("handled",T::Bool)],T::Void,"Acknowledge exactly the current scoped receipt. Unhandled events continue to the next eligible receiver; stale or foreign receipts fail."),
            m("cancel",vec![],T::Void,"Cancel transient delivery/capture state without synthesizing input."),
            m("receiver",vec![("id",T::Int)],T::Optional(Box::new(T::Map)),"Inspect one owned receiver, or null when absent."),
            m("stats",vec![],T::Map,"Inspect bounded receiver/journal/delivery state; this does not inject a device event."),
        ]})
    }
    fn get(&self, _cx: &mut EngineContext, _this: ObjectId, _property: &str) -> Result<Value> {
        unreachable!()
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _this: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        let query = match name {
            "delivery" => Some(EngineQuery::InputRouteDelivery),
            "receiver" => Some(EngineQuery::InputRouteReceiver(id(args)?)),
            "stats" => Some(EngineQuery::InputRouteStats),
            _ => None,
        };
        if let Some(query) = query {
            let state = cx.state()?;
            let bytes = state.runtime.host.query_engine(state.client, query)?;
            return Ok(Value::from_json(serde_json::from_slice(&bytes)?));
        }
        let command = match name {
            "put" => InputRouteCommand::Put {
                receiver: serde_json::from_value(arg(args, 0)?.to_json())?,
            },
            "remove" => InputRouteCommand::Remove { id: id(args)? },
            "clear" => InputRouteCommand::Clear,
            "setEnabled" => InputRouteCommand::SetEnabled {
                id: id(args)?,
                enabled: arg(args, 1)?.as_bool()?,
            },
            "setPriority" => InputRouteCommand::SetPriority {
                id: id(args)?,
                priority: i32::try_from(arg(args, 1)?.as_i64()?)?,
            },
            "acknowledge" => InputRouteCommand::Acknowledge {
                receipt: serde_json::from_value(arg(args, 0)?.to_json())?,
                handled: arg(args, 1)?.as_bool()?,
            },
            "cancel" => InputRouteCommand::Cancel,
            _ => unreachable!(),
        };
        let state = cx.state()?;
        state.runtime.host.submit_engine(
            state.client,
            1,
            &serde_json::to_vec(&serde_json::json!({"input_routes":[command]}))?,
        )?;
        Ok(Value::Null)
    }
}
