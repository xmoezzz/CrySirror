// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Background probes and native-image preparation over the same registries.
use crate::{EngineState, Value};
use anyhow::{Context, Result, ensure};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_tasks::table::{OperationState, OperationTable};
use serde_json::json;
use std::sync::Arc;
enum Product {
    Probe(serde_json::Value),
    Image {
        width: u32,
        height: u32,
        rgba: Arc<[u8]>,
        _profile_charge: Arc<dyn crysirror_core::service::AssetReader>,
        _host_charge: Arc<dyn crysirror_core::service::AssetReader>,
    },
}
pub(crate) struct ResourceJobs {
    jobs: OperationTable<Product>,
    image_budget: crysirror_resource::DecodeContext,
}
impl Default for ResourceJobs {
    fn default() -> Self {
        Self {
            jobs: Default::default(),
            image_budget: crysirror_resource::DecodeContext::new(crysirror_core::state::ClientId(
                0,
            )),
        }
    }
}
fn arg(args: &[Value], n: usize) -> Result<&Value> {
    args.get(n).context("resource argument required")
}
pub(crate) fn update(state: &mut EngineState) {
    let epoch = state.runtime.restored;
    state.resources.operations.jobs.discard_obsolete(epoch);
    let obsolete = state
        .resources
        .decode_generations
        .iter()
        .filter(|(_, (_, generation))| *generation != epoch)
        .map(|(id, (owner, _))| (*id, *owner))
        .collect::<Vec<_>>();
    for (id, owner) in obsolete {
        let _ = state.resources.jobs.cancel(owner, id);
        state.resources.decode_generations.remove(&id);
    }
}
pub(crate) fn call(state: &mut EngineState, name: &str, args: &[Value]) -> Result<Option<Value>> {
    update(state);
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    if matches!(name, "beginProbe" | "beginImage") {
        state.resources.operations.jobs.admit(owner)?;
        let path = arg(args, 0)?.as_str()?.to_owned();
        let kind = if name == "beginImage" {
            "image".to_owned()
        } else {
            arg(args, 1)?.as_str()?.to_owned()
        };
        ensure!(
            ["archive", "file", "image"].contains(&kind.as_str()),
            "probe kind must be archive, file or image"
        );
        let options = crate::resource_api::options(args, if name == "beginImage" { 1 } else { 2 })?;
        let context =
            crate::resource_api::scoped_context(state, &path, options.profile.as_deref())?;
        let storage = state.resources.storage_authorities[&state.client].clone();
        let logical = VfsPath::new(Mount::Asset, &path).map_err(|e| anyhow::anyhow!("{e}"))?;
        let formats = state.resources.formats.clone();
        let files = state.resources.files.clone();
        let images = state.runtime.assets.decoders.clone();
        let image = name == "beginImage";
        let image_budget = state.resources.operations.image_budget.clone();
        let pool = if image {
            state.runtime.assets.cpu_tasks()
        } else {
            state.runtime.assets.io_tasks()
        };
        let ticket=state.resources.operations.jobs.submit(owner,epoch,if image{"image"}else{"probe"},&pool,move|cancel|{
            let input=storage.open_asset(&logical).map_err(|e|anyhow::anyhow!("{e}"))?;cancel.check()?;
            if !image && kind=="archive" {
                return Ok(Product::Probe(json!({"kind":kind,"matches":formats.probe(input,&context)?.into_iter().map(|(format,probe)|json!({"format":format,"confidence":probe.confidence})).collect::<Vec<_>>()})));
            }
            if !image && kind=="file" {
                return Ok(Product::Probe(json!({"kind":kind,"matches":files.probe(&path,input,&context)?.into_iter().map(|(format,probe)|json!({"format":format,"confidence":probe.confidence})).collect::<Vec<_>>()})));
            }
            let bytes=crysirror_resource::read_all(input.as_ref(),context.limits.max_entry_size.min(crysirror_image::MAX_DECODED_BYTES as u64))?;
            if !image{return Ok(Product::Probe(json!({"kind":kind,"matches":images.probe(&path,&bytes)?})));}
            let dependencies=|path:&str|context.open_dependency(path).and_then(|input|crysirror_resource::read_all(input.as_ref(),context.limits.max_entry_size)).map_err(|e|crysirror_core::Error::Io(format!("{e:#}")));
            let surface=images.decode_with(&path,&bytes,Some(&dependencies),Some(&context),options.format.as_deref())?;
            ensure!(surface.rgba.len() as u64<=context.limits.max_entry_size,"decoded image byte limit");
            let rgba:Arc<[u8]>=Arc::from(surface.rgba);
            let profile_charge=context.memory_shared(rgba.clone())?;
            let host_charge=image_budget.memory_shared(rgba.clone())?;
            cancel.check()?;Ok(Product::Image{width:surface.width,height:surface.height,rgba,_profile_charge:profile_charge,_host_charge:host_charge})
        })?;
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    if !matches!(
        name,
        "decodeStatus" | "readDecode" | "cancelDecode" | "publishImage"
    ) {
        return Ok(None);
    }
    let ticket = u64::try_from(arg(args, 0)?.as_i64()?)?;
    if !state.resources.operations.jobs.contains(owner, ticket) {
        ensure!(name != "publishImage", "unknown or foreign image ticket");
        return Ok(None);
    }
    if name == "cancelDecode" {
        state.resources.operations.jobs.cancel(owner, ticket)?;
        return Ok(Some(Value::Null));
    }
    let status = state
        .resources
        .operations
        .jobs
        .status(owner, epoch, ticket)?;
    if name == "decodeStatus" {
        return Ok(Some(Value::from_json(serde_json::to_value(status)?)));
    }
    ensure!(
        status.state == OperationState::Ready,
        "resource operation is pending or failed: {:?}",
        status.error
    );
    let product = state
        .resources
        .operations
        .jobs
        .poll(owner, epoch, ticket)?
        .context("resource operation pending")?;
    let result = match product {
        Product::Probe(value) => {
            ensure!(name == "readDecode", "probe result is not an image");
            value.clone()
        }
        Product::Image {
            width,
            height,
            rgba,
            ..
        } => {
            if name == "publishImage" {
                let slot = u32::try_from(arg(args, 1)?.as_i64()?)?;
                let path = state.runtime.assets.put_surface_arc(
                    state.client,
                    slot,
                    *width,
                    *height,
                    rgba.clone(),
                )?;
                json!({"surface":path,"width":width,"height":height})
            } else {
                json!({"kind":"image","width":width,"height":height,"bytes":rgba.len()})
            }
        }
    };
    Ok(Some(Value::from_json(result)))
}
