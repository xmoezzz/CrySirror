// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Snapshot safepoints, worker serialization/I/O/validation, atomic publication.
use crate::{
    EngineContext, EngineState, Value,
    object::{ClassInfo, MethodInfo, TypeHint as T},
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use crysirror_runtime::runtime::{PreparedRestore, RestorePlan};
use crysirror_tasks::table::{OperationState, OperationTable};
use serde_json::json;
use std::sync::{Arc, OnceLock};

enum Product {
    Encoded {
        bytes: Arc<[u8]>,
        storage: Arc<dyn ClientStorage>,
        path: VfsPath,
    },
    Loaded {
        bytes: Arc<[u8]>,
        plan: Arc<RestorePlan>,
    },
    Saved(usize),
    Prepared(PreparedRestore),
    Exists(bool),
}
#[derive(Default)]
pub struct SaveRuntime {
    jobs: OperationTable<Product>,
}

fn advance(state: &mut EngineState, owner: u32, ticket: u64) -> Result<()> {
    let epoch = state.runtime.restored;
    if state.saves.jobs.status(owner, epoch, ticket)?.state == OperationState::Failed {
        return Ok(());
    }
    let next = match state.saves.jobs.poll(owner, epoch, ticket)? {
        Some(Product::Encoded { .. }) => Some(state.runtime.assets.io_tasks()),
        Some(Product::Loaded { .. }) => Some(state.runtime.assets.cpu_tasks()),
        _ => None,
    };
    let Some(pool) = next else {
        return Ok(());
    };
    let result = state
        .saves
        .jobs
        .advance(owner, epoch, ticket, |product| match product {
            Product::Encoded {
                bytes,
                storage,
                path,
            } => {
                let (bytes, storage, path) = (bytes.clone(), storage.clone(), path.clone());
                pool.spawn(move |cancel| {
                    cancel.check()?;
                    storage
                        .write_atomic(&path, &bytes)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    // Once write_atomic commits, its result must report that commit.
                    // Cancellation cannot undo filesystem side effects already made.
                    Ok(Product::Saved(bytes.len()))
                })
            }
            Product::Loaded { bytes, plan } => {
                let (bytes, plan) = (bytes.clone(), plan.clone());
                pool.spawn(move |cancel| {
                    cancel.check()?;
                    let prepared = (*plan).clone().decode(&bytes)?;
                    cancel.check()?;
                    Ok(Product::Prepared(prepared))
                })
            }
            _ => unreachable!(),
        });
    match result {
        Err(error) if error.is::<crysirror_tasks::CapacityExceeded>() => Ok(()),
        other => other,
    }
}
pub fn update(cx: &mut EngineContext) -> Result<()> {
    let state = cx.state()?;
    state.saves.jobs.discard_obsolete(state.runtime.restored);
    for (owner, _, ticket) in state.saves.jobs.tickets() {
        // A failed operation is queried through its own ticket, not an App fault.
        let _ = advance(state, owner, ticket);
    }
    Ok(())
}
fn arg<'a>(args: &'a [Value], n: usize) -> Result<&'a Value> {
    args.get(n).context("save argument required")
}
fn ticket(args: &[Value]) -> Result<u64> {
    Ok(u64::try_from(arg(args, 0)?.as_i64()?)?)
}
fn storage(state: &mut EngineState) -> Result<Arc<dyn ClientStorage>> {
    state
        .runtime
        .host
        .fs
        .storage(state.client)
        .map_err(|e| anyhow::anyhow!("{e}"))
}
pub(crate) fn info() -> &'static ClassInfo {
    static INFO: OnceLock<ClassInfo> = OnceLock::new();
    INFO.get_or_init(||{
        fn m(name:&'static str,params:Vec<(&'static str,T)>,returns:T,doc:&'static str)->MethodInfo{MethodInfo{name,params,returns,doc}}
        ClassInfo{name:"Save",doc:"Client-capability snapshot operations. Captures preserve ordered VM safepoints; worker jobs perform encoding, disk I/O and validation, and prepared loads commit atomically.",properties:vec![],methods:vec![
            m("save",vec![("path",T::String)],T::Int,"Offline snapshot/write; realtime hosts use beginSave. Returns encoded byte size."),
            m("load",vec![("path",T::String)],T::Void,"Offline read/restore; realtime hosts use beginLoad then commitLoad. A failed restore leaves the runtime unchanged."),
            m("exists",vec![("path",T::String)],T::Bool,"Offline existence query; realtime hosts use beginExists. Permission/read errors remain errors."),
            m("beginSave",vec![("path",T::String)],T::Int,"Capture logical state and native VM checkpoints at the current owner safepoint, then serialize and write on bounded CPU/I/O workers. Returns a client ticket. Native checkpoint capture itself remains ordered."),
            m("beginLoad",vec![("path",T::String)],T::Int,"Read, decode and validate a snapshot on workers. A ready result is prepared, not applied; commitLoad publishes it at an owner safepoint."),
            m("beginExists",vec![("path",T::String)],T::Int,"Query the save capability on the bounded I/O executor. Only NotFound becomes false."),
            m("status",vec![("ticket",T::Int)],T::Map,"Poll pending/ready/failed without joining workers. Ready save returns bytes; exists returns exists; load returns prepared=true. All tickets are client/generation checked."),
            m("commitLoad",vec![("ticket",T::Int)],T::Void,"Atomically apply a prepared load, preserving native restore semantics. Reject pending, wrong-kind and obsolete results. Successful restore invalidates earlier-generation jobs."),
            m("cancel",vec![("ticket",T::Int)],T::Void,"Cancel/release retained work. Running atomic writes may already commit; cancellation cannot undo an existing disk write."),
        ]}
    })
}
pub(crate) fn call(cx: &mut EngineContext, method: &str, args: &[Value]) -> Result<Value> {
    let state = cx.state()?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    state.saves.jobs.discard_obsolete(epoch);
    if matches!(method, "status" | "cancel" | "commitLoad") {
        let id = ticket(args)?;
        if method == "cancel" {
            state.saves.jobs.cancel(owner, id)?;
            return Ok(Value::Null);
        }
        advance(state, owner, id)?;
        if method == "status" {
            let status = state.saves.jobs.status(owner, epoch, id)?;
            let mut value = serde_json::to_value(&status)?;
            if status.state == OperationState::Ready {
                match state.saves.jobs.poll(owner, epoch, id)?.unwrap() {
                    Product::Saved(bytes) => value["bytes"] = json!(bytes),
                    Product::Exists(exists) => value["exists"] = json!(exists),
                    Product::Prepared(_) => value["prepared"] = json!(true),
                    _ => value["state"] = json!("pending"),
                }
            }
            return Ok(Value::from_json(value));
        }
        ensure!(
            matches!(
                state.saves.jobs.poll(owner, epoch, id)?,
                Some(Product::Prepared(_))
            ),
            "load is pending or ticket is not a prepared load"
        );
        let Product::Prepared(prepared) = state.saves.jobs.take(owner, epoch, id)?.unwrap() else {
            unreachable!()
        };
        state.runtime.commit_restore(prepared)?;
        state.saves.jobs.discard_obsolete(state.runtime.restored);
        return Ok(Value::Null);
    }
    let path =
        VfsPath::new(Mount::Save, arg(args, 0)?.as_str()?).map_err(|e| anyhow::anyhow!("{e}"))?;
    if matches!(method, "save" | "load" | "exists") {
        ensure!(
            !state.runtime.assets.is_nonblocking(),
            "synchronous save operations are offline-only; use beginSave/beginLoad/beginExists"
        );
        let storage = storage(state)?;
        return match method {
            "save" => {
                let bytes = state.runtime.snapshot()?;
                storage
                    .write_atomic(&path, &bytes)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                Ok(Value::Int(bytes.len() as i64))
            }
            "load" => {
                let bytes = storage.read(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
                state.runtime.restore(&bytes)?;
                Ok(Value::Null)
            }
            _ => match storage.read(&path) {
                Ok(_) => Ok(Value::Bool(true)),
                Err(crysirror_core::Error::NotFound) => Ok(Value::Bool(false)),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            },
        };
    }
    state.saves.jobs.admit(owner)?;
    let storage = storage(state)?;
    let id = match method {
        "beginSave" => {
            let captured = state.runtime.capture_snapshot()?;
            let pool = state.runtime.assets.cpu_tasks();
            state
                .saves
                .jobs
                .submit(owner, epoch, "save", &pool, move |cancel| {
                    let bytes = captured.encode()?;
                    cancel.check()?;
                    Ok(Product::Encoded {
                        bytes: Arc::from(bytes),
                        storage,
                        path,
                    })
                })?
        }
        "beginLoad" => {
            let plan = Arc::new(state.runtime.restore_plan());
            let pool = state.runtime.assets.io_tasks();
            state
                .saves
                .jobs
                .submit(owner, epoch, "load", &pool, move |cancel| {
                    let bytes = storage.read(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
                    ensure!(
                        bytes.len() <= crysirror_core::save::SaveLimits::default().total_bytes,
                        "snapshot byte limit"
                    );
                    cancel.check()?;
                    Ok(Product::Loaded {
                        bytes: Arc::from(bytes),
                        plan,
                    })
                })?
        }
        "beginExists" => {
            let pool = state.runtime.assets.io_tasks();
            state
                .saves
                .jobs
                .submit(owner, epoch, "exists", &pool, move |_| {
                    let exists = match storage.read(&path) {
                        Ok(_) => true,
                        Err(crysirror_core::Error::NotFound) => false,
                        Err(e) => return Err(anyhow::anyhow!("{e}")),
                    };
                    Ok(Product::Exists(exists))
                })?
        }
        _ => anyhow::bail!("unknown Save method {method}"),
    };
    Ok(Value::Int(i64::try_from(id)?))
}
