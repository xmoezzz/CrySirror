// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared script binding specifications and host-owned await subscriptions.
//! The subscriptions are runtime capabilities, never project/save JSON.
use crate::{
    EngineContext, Value,
    object::{ClassRegistry, ObjectId},
};
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, fmt::Write};
#[derive(Clone, Copy)]
pub struct AsyncBinding {
    pub class: &'static str,
    pub name: &'static str,
    pub start: &'static str,
    pub status: &'static str,
    pub cancel: &'static str,
    /// ticket, read, commit, or a dot-separated status result field.
    pub completion: &'static str,
    pub returns: &'static str,
    pub release: bool,
}
pub const BINDINGS: &[AsyncBinding] = &[
    AsyncBinding {
        class: "Audio",
        name: "playSfxAsync",
        start: "beginPlaySfx",
        status: "playbackStatus",
        cancel: "cancelPlaybackJob",
        completion: "result.handle",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "preloadSfxAsync",
        start: "beginPreloadSfx",
        status: "playbackStatus",
        cancel: "cancelPlaybackJob",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "playMusicAsync",
        start: "beginPlayMusic",
        status: "playbackStatus",
        cancel: "cancelPlaybackJob",
        completion: "result.handle",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "seekVoiceAsync",
        start: "beginSeekVoice",
        status: "playbackStatus",
        cancel: "cancelPlaybackJob",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "prepareHrtfAsync",
        start: "beginHrtf",
        status: "hrtfStatus",
        cancel: "cancelHrtfJob",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "prepareAcousticsAsync",
        start: "beginAcousticScene",
        status: "acousticStatus",
        cancel: "cancelAcousticJob",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Audio",
        name: "traceAcousticsAsync",
        start: "beginAcousticProbe",
        status: "acousticStatus",
        cancel: "cancelAcousticJob",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Assets",
        name: "loadAsync",
        start: "load",
        status: "status",
        cancel: "release",
        completion: "ticket",
        returns: "number",
        release: false,
    },
    AsyncBinding {
        class: "Resources",
        name: "decodeAsync",
        start: "beginDecode",
        status: "decodeStatus",
        cancel: "cancelDecode",
        completion: "ticket",
        returns: "number",
        release: false,
    },
    AsyncBinding {
        class: "Resources",
        name: "imageAsync",
        start: "beginImage",
        status: "decodeStatus",
        cancel: "cancelDecode",
        completion: "ticket",
        returns: "number",
        release: false,
    },
    AsyncBinding {
        class: "Resources",
        name: "probeAsync",
        start: "beginProbe",
        status: "decodeStatus",
        cancel: "cancelDecode",
        completion: "read",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Text",
        name: "drawAsync",
        start: "beginDraw",
        status: "status",
        cancel: "cancel",
        completion: "result.surface",
        returns: "string",
        release: true,
    },
    AsyncBinding {
        class: "Text",
        name: "restoreAsync",
        start: "beginRestore",
        status: "status",
        cancel: "cancel",
        completion: "result.surface",
        returns: "string",
        release: true,
    },
    AsyncBinding {
        class: "Text",
        name: "measureAsync",
        start: "beginMeasure",
        status: "status",
        cancel: "cancel",
        completion: "result",
        returns: "Record<string, any>",
        release: true,
    },
    AsyncBinding {
        class: "Save",
        name: "saveAsync",
        start: "beginSave",
        status: "status",
        cancel: "cancel",
        completion: "bytes",
        returns: "number",
        release: true,
    },
    AsyncBinding {
        class: "Save",
        name: "loadAsync",
        start: "beginLoad",
        status: "status",
        cancel: "cancel",
        completion: "commit",
        returns: "void",
        release: false,
    },
    AsyncBinding {
        class: "Save",
        name: "existsAsync",
        start: "beginExists",
        status: "status",
        cancel: "cancel",
        completion: "exists",
        returns: "boolean",
        release: true,
    },
];
pub fn reference(classes: &ClassRegistry) -> Vec<Json> {
    BINDINGS.iter().filter_map(|spec|{
        let class=classes.get(spec.class).ok()?;let method=class.info().method(spec.start).expect("async start signature");
        class.info().method(spec.status).expect("async status method");class.info().method(spec.cancel).expect("async cancellation method");
        Some(json!({"class":spec.class,"name":spec.name,"start":spec.start,"status":spec.status,"cancel":spec.cancel,"completion":spec.completion,"release_on_success":spec.release,
            "params":method.params.iter().map(|(name,ty)|json!({"name":name,"type":ty.typescript(),"optional":matches!(ty,crate::TypeHint::Optional(_))})).collect::<Vec<_>>(),"returns":format!("CancellablePromise<{}>",spec.returns)}))
    }).collect()
}
pub fn declarations(classes: &ClassRegistry) -> String {
    let mut out = String::from(
        "\n/** JS/TS service awaits; callbacks run on owner lifecycle entries. */\ninterface CancellablePromise<T> extends Promise<T> { readonly ticket: number | null; cancel(): void; }\n",
    );
    for method in reference(classes) {
        let params = method["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}{}: {}",
                    p["name"].as_str().unwrap(),
                    if p["optional"] == true { "?" } else { "" },
                    p["type"].as_str().unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "interface {} {{ {}({params}): {}; }}",
            method["class"].as_str().unwrap(),
            method["name"].as_str().unwrap(),
            method["returns"].as_str().unwrap()
        );
    }
    out
}
struct Subscription {
    object: ObjectId,
    owner: ClientId,
    generation: u64,
    ticket: u64,
    spec: AsyncBinding,
}
/// One script host owns one table: another script context cannot poll its watches.
#[derive(Default)]
pub struct AsyncCalls {
    entries: BTreeMap<u64, Subscription>,
}
fn scoped<T>(
    cx: &mut EngineContext,
    client: ClientId,
    work: impl FnOnce(&mut EngineContext) -> Result<T>,
) -> Result<T> {
    struct Restore<'a> {
        cx: &'a mut EngineContext,
        previous: ClientId,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            if let Ok(state) = self.cx.state() {
                state.client = self.previous;
            }
        }
    }
    let previous = cx.state()?.client;
    cx.state()?.client = client;
    let guard = Restore { cx, previous };
    work(guard.cx)
}

impl AsyncCalls {
    pub fn begin(
        &mut self,
        cx: &mut EngineContext,
        object: ObjectId,
        binding: usize,
        args: &[Value],
    ) -> Result<Json> {
        ensure!(self.entries.len() < 128, "script await capacity exceeded");
        let spec = *BINDINGS
            .get(binding)
            .context("unknown asynchronous binding")?;
        let state = cx.state_ref()?;
        ensure!(
            state.objects.get(object)?.class == spec.class,
            "asynchronous binding class mismatch"
        );
        let owner = state.client;
        let generation = state.runtime.restored;
        let watch = crysirror_tasks::table::next_operation_id()?;
        let ticket = u64::try_from(cx.call(object, spec.start, args)?.as_i64()?)?;
        self.entries.insert(
            watch,
            Subscription {
                object,
                owner,
                generation,
                ticket,
                spec,
            },
        );
        Ok(json!({"watch":watch,"ticket":ticket}))
    }
    fn cancel_entry(cx: &mut EngineContext, entry: &Subscription) {
        let _ = scoped(cx, entry.owner, |cx| {
            cx.call(
                entry.object,
                entry.spec.cancel,
                &[Value::Int(entry.ticket as i64)],
            )
        });
    }
    pub fn cancel(&mut self, cx: &mut EngineContext, watch: u64) -> Result<()> {
        let entry = self
            .entries
            .remove(&watch)
            .context("script await already finished")?;
        Self::cancel_entry(cx, &entry);
        Ok(())
    }
    pub fn cancel_all(&mut self, cx: &mut EngineContext) {
        for (_, entry) in std::mem::take(&mut self.entries) {
            Self::cancel_entry(cx, &entry);
        }
    }
    pub fn poll(&mut self, cx: &mut EngineContext, watch: u64) -> Result<Json> {
        let entry = self
            .entries
            .remove(&watch)
            .context("script await already finished")?;
        let result = (|| {
            ensure!(
                cx.state_ref()?.runtime.restored == entry.generation,
                "script await belongs to an obsolete restore generation"
            );
            scoped(cx, entry.owner, |cx| {
                let status = cx
                    .call(
                        entry.object,
                        entry.spec.status,
                        &[Value::Int(entry.ticket as i64)],
                    )?
                    .to_json();
                match status["state"]
                    .as_str()
                    .context("invalid service operation status")?
                {
                    "pending" | "loading" => return Ok(None),
                    "ready" => {}
                    _ => anyhow::bail!("{}: {}", entry.spec.class, status["error"]),
                }
                let value = match entry.spec.completion {
                    "ticket" => json!(entry.ticket),
                    "read" => cx
                        .call(
                            entry.object,
                            "readDecode",
                            &[Value::Int(entry.ticket as i64)],
                        )?
                        .to_json(),
                    "commit" => {
                        cx.call(
                            entry.object,
                            "commitLoad",
                            &[Value::Int(entry.ticket as i64)],
                        )?;
                        Json::Null
                    }
                    path => {
                        let mut field = &status;
                        for key in path.split('.') {
                            field = field
                                .get(key)
                                .context("missing ready service result field")?;
                        }
                        field.clone()
                    }
                };
                Ok(Some(value))
            })
        })();
        match result {
            Ok(None) => {
                self.entries.insert(watch, entry);
                Ok(json!({"state":"pending"}))
            }
            Ok(Some(value)) => {
                if entry.spec.release {
                    Self::cancel_entry(cx, &entry);
                }
                Ok(json!({"state":"ready","value":value}))
            }
            Err(error) => {
                Self::cancel_entry(cx, &entry);
                Err(error)
            }
        }
    }
}
