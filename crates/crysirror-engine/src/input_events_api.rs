// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Optional per-client input journals over the same Core buffer used by adapters.
use crate::{EngineContext, EngineState, Value};
use anyhow::{Context, Result, ensure};
use crysirror_core::{input_buffer::InputBuffer, service::InputFrame};
use crysirror_scene::input_buffer::{InputInbox, Receipt, Sample, Snapshot};
use serde_json::json;
use std::collections::BTreeMap;
#[derive(Default)]
pub struct InputEvents {
    next_epoch: u64,
    restored: Option<u64>,
    buffers: BTreeMap<u32, InputInbox>,
}
fn sync(s: &mut EngineState) {
    let events = &mut s.input_events;
    if events.restored.is_some_and(|old| old != s.runtime.restored) {
        events.buffers.clear();
    }
    events.restored = Some(s.runtime.restored);
}
pub(crate) fn observe(s: &mut EngineState, input: InputFrame) -> Result<()> {
    sync(s);
    let time = s.runtime.now_ms();
    // Admission across enabled clients is atomic, including capacity failures.
    for (client, buffer) in &s.input_events.buffers {
        buffer.0.preflight_push(*client, time, input)?;
    }
    for (client, buffer) in &mut s.input_events.buffers {
        buffer.0.push(*client, time, input)?;
    }
    Ok(())
}
fn number(args: &[Value], i: usize) -> Result<u64> {
    Ok(u64::try_from(
        args.get(i)
            .context("input event argument missing")?
            .as_i64()?,
    )?)
}
pub(crate) fn call(cx: &mut EngineContext, name: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        name,
        "enableEvents"
            | "disableEvents"
            | "events"
            | "ackEvents"
            | "eventStats"
            | "captureEvents"
            | "restoreEvents"
    ) {
        return Ok(None);
    }
    let s = cx.state()?;
    sync(s);
    let owner = s.client.0;
    ensure!(
        owner > 0 && owner as usize <= s.runtime.project.clients.len(),
        "input event client missing"
    );
    let e = &mut s.input_events;
    let value = match name {
        "enableEvents" | "restoreEvents" => {
            let epoch = e
                .next_epoch
                .checked_add(1)
                .context("input event epoch exhausted")?;
            let buffer = if name == "enableEvents" {
                InputBuffer::new(owner, epoch, usize::try_from(number(args, 0)?)?)?
            } else {
                let wire: Snapshot = serde_json::from_value(
                    args.first().context("input snapshot required")?.to_json(),
                )?;
                let snapshot: crysirror_core::input_buffer::InputBufferSnapshot = wire.into();
                ensure!(
                    snapshot.observed_time <= s.runtime.now_ms(),
                    "input snapshot is ahead of the current owner clock"
                );
                InputBuffer::restore(owner, epoch, snapshot)?
            };
            e.next_epoch = epoch;
            e.buffers.insert(owner, InputInbox(buffer));
            json!(null)
        }
        "disableEvents" => {
            e.buffers.remove(&owner);
            json!(null)
        }
        "eventStats" => match e.buffers.get(&owner) {
            Some(b) => {
                json!({"enabled":true,"client":owner,"epoch":b.0.epoch().to_string(),"pending":b.0.len(),"capacity":b.0.capacity()})
            }
            None => json!({"enabled":false,"pending":0}),
        },
        _ => {
            let buffer = &mut e
                .buffers
                .get_mut(&owner)
                .context("enable input events before consuming them")?
                .0;
            match name {
                "events" => {
                    let batch = buffer.read(owner, usize::try_from(number(args, 0)?)?)?;
                    json!({"samples":batch.samples.into_iter().map(Sample).collect::<Vec<_>>(),"receipt":batch.receipt.map(Receipt)})
                }
                "ackEvents" => {
                    let Receipt(receipt) = serde_json::from_value(
                        args.first().context("input receipt required")?.to_json(),
                    )?;
                    buffer.acknowledge(owner, receipt)?;
                    json!(null)
                }
                "captureEvents" => serde_json::to_value(Snapshot::from(buffer.snapshot()))?,
                "restoreEvents" | "enableEvents" | "disableEvents" | "eventStats" => unreachable!(),
                _ => unreachable!(),
            }
        }
    };
    Ok(Some(Value::from_json(value)))
}
