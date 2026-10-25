// SPDX-License-Identifier: MPL-2.0
//! Nonblocking asset requests shared by Rust, scripts and the renderer.
use crate::{
    EngineContext, Value,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, TypeHint as T},
};
use anyhow::{Context, Result};
use crysirror_runtime::assets::AssetKind;
use std::sync::{Arc, OnceLock};
pub fn register(classes: &mut ClassRegistry) -> Result<()> {
    classes.register(Arc::new(AssetsClass))
}
pub struct AssetsClass;
impl Class for AssetsClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"Assets",doc:"Client-scoped asynchronous resource loading on bounded engine workers. Ready models/images feed the normal renderer.",properties:vec![],methods:vec![
  MethodInfo{name:"load",params:vec![("path",T::String),("kind",T::String)],returns:T::Int,doc:"Queue bytes, image, model or baked scene or PBR level import without blocking. Returns a client-owned request id; duplicate requests share a job."},
  MethodInfo{name:"status",params:vec![("id",T::Int)],returns:T::Map,doc:"Poll loading/ready/failed/cancelled status and error. This never waits for disk or decoding."},
  MethodInfo{name:"cancel",params:vec![("id",T::Int)],returns:T::Void,doc:"Request cooperative cancellation of a loading asset. Late worker results cannot publish."},
  MethodInfo{name:"release",params:vec![("id",T::Int)],returns:T::Void,doc:"Release a request and its CPU cache entry; cancel work if pending. Does not remove existing scene entities or GPU copies."},
  MethodInfo{name:"stats",params:vec![],returns:T::Map,doc:"Inspect shared worker activity, bounded queue counts, CPU cache bytes and only this client's requests."},
 ]})
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
        let state = cx.state()?;
        let client = state.client;
        let assets = &mut state.runtime.assets;
        match name {
            "load" => {
                let path = args.first().context("path required")?.as_str()?;
                let kind = match args.get(1).context("kind required")?.as_str()? {
                    "bytes" => AssetKind::Bytes,
                    "image" => AssetKind::Image,
                    "model" => AssetKind::Model,
                    "scene" => AssetKind::Scene,
                    "level" => AssetKind::Level,
                    _ => anyhow::bail!("asset kind must be bytes, image, model, scene or level"),
                };
                Ok(Value::Int(i64::try_from(
                    assets.request(client, path, kind)?.id,
                )?))
            }
            "status" | "cancel" | "release" => {
                let id = u64::try_from(args.first().context("request id required")?.as_i64()?)?;
                let ticket = assets.ticket(client, id)?;
                match name {
                    "status" => Ok(Value::from_json(serde_json::to_value(
                        assets.task_status(client, ticket)?,
                    )?)),
                    "cancel" => {
                        assets.cancel_task(client, ticket)?;
                        Ok(Value::Null)
                    }
                    _ => {
                        assets.release_task(client, ticket)?;
                        Ok(Value::Null)
                    }
                }
            }
            "stats" => {
                let mut stats = assets.tasks_inspect();
                if let Some(entries) = stats["assets"].as_array_mut() {
                    entries.retain(|e| e["ticket"]["client"] == client.0);
                }
                Ok(Value::from_json(stats))
            }
            _ => unreachable!(),
        }
    }
}
