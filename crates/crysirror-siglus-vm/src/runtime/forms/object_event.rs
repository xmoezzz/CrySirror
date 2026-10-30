// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native object target/subop policy; shared Scene IntEvent owns all motion.
use super::codes::int_event_op as op;
use crate::runtime::{Value, constants::RuntimeConstants, globals::ObjectState};
use anyhow::{Context, Result, bail, ensure};
pub(super) fn dispatch(
    object: &mut ObjectState,
    ids: &RuntimeConstants,
    tail: &[i32],
    args: &[Value],
    overload: i64,
) -> Result<Option<Value>> {
    ensure!(
        tail.len() == 2 && overload == 0,
        "native object event chain/overload"
    );
    let event = object
        .int_event_by_op_mut(ids, tail[0])
        .context("native object event target")?;
    run(event, tail[1], args)
}
pub(super) fn run(
    event: &mut crysirror_scene::animation::IntEvent,
    command: i32,
    args: &[Value],
) -> Result<Option<Value>> {
    let values = args
        .iter()
        .filter(|v| !matches!(v, Value::NamedArg { .. }))
        .map(|v| {
            v.as_i64()
                .map(|n| n as i32)
                .context("native object event integer")
        })
        .collect::<Result<Vec<_>>>()?;
    let real = i32::from(matches!(
        command,
        op::SET_REAL | op::LOOP_REAL | op::TURN_REAL
    ));
    for arg in args {
        if let Value::NamedArg { id, value } = arg {
            ensure!(
                *id == 0 && matches!(command, op::SET | op::SET_REAL),
                "unsupported native object event named argument"
            );
            let value = value
                .as_i64()
                .context("native object event initial value")? as i32;
            event.set_value(value);
            event.cur_value = value;
        }
    }
    match command {
        op::SET | op::SET_REAL => {
            ensure!(values.len() == 4, "native object event SET arguments");
            event.set_event(values[0], values[1], values[2], values[3], real);
        }
        op::LOOP | op::LOOP_REAL | op::TURN | op::TURN_REAL => {
            ensure!(values.len() == 5, "native object event LOOP/TURN arguments");
            if matches!(command, op::LOOP | op::LOOP_REAL) {
                event.loop_event(values[0], values[1], values[2], values[3], values[4], real);
            } else {
                event.turn_event(values[0], values[1], values[2], values[3], values[4], real);
            }
        }
        op::END => {
            ensure!(values.is_empty(), "native object event END arguments");
            event.end_event();
        }
        op::CHECK => {
            ensure!(values.is_empty(), "native object event CHECK arguments");
            return Ok(Some(Value::Int(i64::from(event.check_event()))));
        }
        _ => bail!("unsupported native object event suboperation {command}"),
    }
    Ok(None)
}
