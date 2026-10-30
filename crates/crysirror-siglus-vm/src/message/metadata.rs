// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native MWND logical queries do not instantiate text, textures or UI backends.
use super::*;
use crate::runtime::{
    Value,
    forms::{codes, stage},
};
pub(crate) fn query(
    ctx: &mut CommandContext,
    window: usize,
    op: i32,
    args: &[Value],
) -> Result<bool> {
    let info = match op {
        codes::ELM_MWND_GET_OPEN_ANIME_TYPE => ("OPEN_ANIME_TYPE", false, 0),
        codes::ELM_MWND_GET_OPEN_ANIME_TIME => ("OPEN_ANIME_TIME", false, 1),
        codes::ELM_MWND_GET_CLOSE_ANIME_TYPE => ("CLOSE_ANIME_TYPE", false, 2),
        codes::ELM_MWND_GET_CLOSE_ANIME_TIME => ("CLOSE_ANIME_TIME", false, 3),
        codes::ELM_MWND_GET_DEFAULT_OPEN_ANIME_TYPE => ("OPEN_ANIME_TYPE", true, 0),
        codes::ELM_MWND_GET_DEFAULT_OPEN_ANIME_TIME => ("OPEN_ANIME_TIME", true, 1),
        codes::ELM_MWND_GET_DEFAULT_CLOSE_ANIME_TYPE => ("CLOSE_ANIME_TYPE", true, 2),
        codes::ELM_MWND_GET_DEFAULT_CLOSE_ANIME_TIME => ("CLOSE_ANIME_TIME", true, 3),
        codes::ELM_MWND_CHECK_OPEN => {
            ensure!(args.is_empty(), "native MWND check-open arguments");
            let open = ctx
                .globals
                .stage_forms
                .get(&stage::current_stage_form_id(ctx))
                .and_then(|s| s.mwnd_lists.get(&1))
                .and_then(|list| list.get(window))
                .is_some_and(|m| m.open);
            ctx.push(Value::Int(i64::from(open)));
            return Ok(true);
        }
        _ => return Ok(false),
    };
    ensure!(
        window < 16 && args.is_empty(),
        "native MWND metadata query bounds/arguments"
    );
    let native = ctx
        .globals
        .stage_forms
        .get(&stage::current_stage_form_id(ctx))
        .and_then(|s| s.mwnd_lists.get(&1))
        .and_then(|list| list.get(window))
        .filter(|m| m.initialized_from_gameexe);
    let value = if !info.1 && native.is_some() {
        let m = native.unwrap();
        [
            m.open_anime_type,
            m.open_anime_time,
            m.close_anime_type,
            m.close_anime_time,
        ][info.2]
    } else {
        i64::from(super::config::scalar(
            ctx.tables
                .gameexe
                .as_ref()
                .and_then(|cfg| cfg.get_indexed_field("MWND", window, info.0)),
            0,
        )?)
    };
    ctx.push(Value::Int(value));
    Ok(true)
}
