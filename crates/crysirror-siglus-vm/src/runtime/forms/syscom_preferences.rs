// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native saved preference keys. Active unsupported features still fail at
//! their execution boundary; supported MWND/voice policy consumes these keys.
use crate::runtime::{CommandContext, Value, forms::codes::syscom_op as op};
use anyhow::{Context, Result, ensure};
const BOOLS: &[(i32, i32, i32, &str, bool)] = &[
    (
        op::SET_SAVELOAD_ALERT_ONOFF,
        op::SET_SAVELOAD_ALERT_ONOFF_DEFAULT,
        op::GET_SAVELOAD_ALERT_ONOFF,
        "",
        true,
    ),
    (
        op::SET_NO_WIPE_ANIME_ONOFF,
        op::SET_NO_WIPE_ANIME_ONOFF_DEFAULT,
        op::GET_NO_WIPE_ANIME_ONOFF,
        "CONFIG.NO_WIPE_ANIME.ONOFF",
        false,
    ),
    (
        op::SET_SKIP_WIPE_ANIME_ONOFF,
        op::SET_SKIP_WIPE_ANIME_ONOFF_DEFAULT,
        op::GET_SKIP_WIPE_ANIME_ONOFF,
        "CONFIG.SKIP_WIPE_ANIME.ONOFF",
        true,
    ),
    (
        op::SET_NO_MWND_ANIME_ONOFF,
        op::SET_NO_MWND_ANIME_ONOFF_DEFAULT,
        op::GET_NO_MWND_ANIME_ONOFF,
        "CONFIG.NO_MWND_ANIME.ONOFF",
        false,
    ),
    (
        op::SET_SKIP_UNREAD_MESSAGE_ONOFF,
        op::SET_SKIP_UNREAD_MESSAGE_ONOFF_DEFAULT,
        op::GET_SKIP_UNREAD_MESSAGE_ONOFF,
        "CONFIG.SKIP_UNREAD_MESSAGE.ONOFF",
        false,
    ),
    (
        op::SET_KOE_DONT_STOP_ONOFF,
        op::SET_KOE_DONT_STOP_ONOFF_DEFAULT,
        op::GET_KOE_DONT_STOP_ONOFF,
        "CONFIG.KOE_DONT_STOP.ONOFF",
        false,
    ),
];
fn default(ctx: &CommandContext, key: &str, fallback: bool) -> bool {
    ctx.tables
        .gameexe
        .as_ref()
        .and_then(|cfg| cfg.get_i64(key))
        .map_or(fallback, |v| v != 0)
}
pub(super) fn initialize(ctx: &mut CommandContext) {
    for &(_, _, get, key, fallback) in BOOLS {
        let value = default(ctx, key, fallback);
        store(ctx, get, value);
    }
}
fn store(ctx: &mut CommandContext, get: i32, value: bool) {
    ctx.globals.syscom.config_int.insert(get, i64::from(value));
    let cfg = &mut ctx.globals.syscom.original_config;
    match get {
        op::GET_SAVELOAD_ALERT_ONOFF => cfg.saveload_alert_flag = value,
        op::GET_NO_WIPE_ANIME_ONOFF => cfg.no_wipe_anime_flag = value,
        op::GET_SKIP_WIPE_ANIME_ONOFF => cfg.skip_wipe_anime_flag = value,
        op::GET_NO_MWND_ANIME_ONOFF => cfg.no_mwnd_anime_flag = value,
        op::GET_SKIP_UNREAD_MESSAGE_ONOFF => cfg.skip_unread_message_flag = value,
        op::GET_KOE_DONT_STOP_ONOFF => cfg.koe_dont_stop_flag = value,
        _ => unreachable!(),
    }
}
pub(super) fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let call = ctx.vm_call.as_ref().context("native preference metadata")?;
    let command = *call.element.get(1).context("native preference operation")?;
    let Some(&(set, reset, get, key, fallback)) = BOOLS
        .iter()
        .find(|(s, r, g, _, _)| [s, r, g].into_iter().any(|v| *v == command))
    else {
        return Ok(false);
    };
    ensure!(
        call.element.len() == 2 && call.al_id == 0,
        "native preference chain/overload"
    );
    ensure!(
        args.len() == usize::from(command == set),
        "native preference arguments"
    );
    if command == get {
        let value = ctx
            .globals
            .syscom
            .config_int
            .get(&get)
            .copied()
            .unwrap_or_else(|| i64::from(default(ctx, key, fallback)));
        ctx.push(Value::Int(value));
    } else {
        let value = if command == reset {
            default(ctx, key, fallback)
        } else {
            args[0].as_i64().context("native preference value")? != 0
        };
        store(ctx, get, value);
    }
    Ok(true)
}
