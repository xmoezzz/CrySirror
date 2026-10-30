// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native message proc policy. Layout, glyphs and reveal remain engine services.
use super::*;
use crate::runtime::{
    Value,
    forms::{codes, stage},
    globals::MwndState,
};

pub(super) fn state(ctx: &CommandContext) -> Option<&MwndState> {
    ctx.globals
        .stage_forms
        .get(&stage::current_stage_form_id(ctx))?
        .mwnd_lists
        .get(&1)?
        .first()
}
pub(super) fn state_mut(ctx: &mut CommandContext) -> &mut MwndState {
    let family = stage::current_stage_form_id(ctx);
    let list = ctx
        .globals
        .stage_forms
        .entry(family)
        .or_default()
        .mwnd_lists
        .entry(1)
        .or_default();
    if list.is_empty() {
        list.push(Default::default());
    }
    &mut list[0]
}
fn commit(ctx: &mut CommandContext) {
    let flags = std::mem::take(&mut state_mut(ctx).read_flag_stock);
    for (scene, flag) in flags {
        ctx.globals.set_read_flag(scene, flag);
    }
}
fn clear_ready(ctx: &mut CommandContext) {
    commit(ctx);
    let m = state_mut(ctx);
    m.clear_ready = true;
    m.msg_block_started = false;
    m.multi_msg = false;
    let script = &mut ctx.globals.script;
    script.multi_msg_mode = false;
    script.cur_koe_no = -1;
    script.cur_chr_no = -1;
    script.auto_mode_moji_cnt = 0;
    if script.async_msg_mode_once {
        script.async_msg_mode = false;
        script.async_msg_mode_once = false;
    }
}
pub(super) fn set_open(ctx: &mut CommandContext, open: bool, wait: bool) -> Result<()> {
    let now = crate::platform_time::Instant::now().0;
    let no_anime = ctx
        .globals
        .syscom
        .config_int
        .get(&crate::runtime::forms::codes::syscom_op::GET_NO_MWND_ANIME_ONOFF)
        .copied()
        .unwrap_or(0)
        != 0;
    let m = state_mut(ctx);
    let (kind, duration) = if open {
        (m.open_anime_type, m.open_anime_time)
    } else {
        (m.close_anime_type, m.close_anime_time)
    };
    ensure!(
        (0..=5).contains(&kind) && duration >= 0,
        "unsupported native MWND animation kind/duration"
    );
    let previous = f32::from(u8::from(m.open));
    let duration = if no_anime || kind == 0 { 0 } else { duration };
    m.open = open;
    m.window_appear = open;
    let start = ctx
        .ui
        .window_tween
        .as_ref()
        .map_or(previous, |t| t.sample(now));
    ctx.ui.window_tween = Some(crysirror_scene::animation::ScalarTween::new(
        start,
        if open { 1. } else { 0. },
        now,
        if kind == 0 { 0 } else { duration as u64 },
        crysirror_scene::layer::Easing::Linear,
    )?);
    ctx.ui.window_animation_type = if no_anime { 0 } else { kind };
    if wait && duration > 0 {
        ctx.wait.wait_mwnd_animation(duration as u64);
    }
    Ok(())
}
pub(super) fn visual(ctx: &CommandContext) -> ([f32; 2], u8) {
    let now = crate::platform_time::Instant::now().0;
    let p = ctx.ui.window_tween.as_ref().map_or_else(
        || f32::from(u8::from(state(ctx).is_some_and(|m| m.open))),
        |t| t.sample(now),
    );
    let eased = crysirror_scene::layer::Easing::Smooth.apply(p);
    let profile = ctx.ui.window.as_ref().unwrap();
    let rect = state(ctx)
        .and_then(|m| m.window_pos.zip(m.window_size))
        .map_or(profile.rect.map(|n| n as f32), |(p, s)| {
            [p.0 as f32, p.1 as f32, s.0 as f32, s.1 as f32]
        });
    let offset = match ctx.ui.window_animation_type {
        2 => [0., -(rect[1] + rect[3]) * (1. - eased)],
        3 => [0., (ctx.screen_h as f32 - rect[1]) * (1. - eased)],
        4 => [-(rect[0] + rect[2]) * (1. - eased), 0.],
        5 => [(ctx.screen_w as f32 - rect[0]) * (1. - eased), 0.],
        _ => [0., 0.],
    };
    (
        offset,
        if ctx.ui.window_animation_type == 0 {
            if p > 0. { 255 } else { 0 }
        } else {
            (255. * eased).clamp(0., 255.) as u8
        },
    )
}
pub(crate) fn after_input(ctx: &mut CommandContext) {
    if ctx.ui.clear_after_key
        && !ctx.wait.message_key_waiting()
        && !ctx.wait.message_reveal_waiting()
    {
        ctx.ui.clear_after_key = false;
        clear_ready(ctx);
    }
}
pub(crate) fn stock_flag(ctx: &mut CommandContext, flag: i32) {
    if let Some(scene) = ctx.current_scene_no
        && flag >= 0
    {
        state_mut(ctx)
            .read_flag_stock
            .push((scene, i64::from(flag)));
    }
}
pub(crate) fn dispatch(ctx: &mut CommandContext, op: i32, args: &[Value]) -> Result<bool> {
    super::config::initialize(ctx)?;
    ensure!(
        ctx.ui.window.is_some(),
        "native MWND requires an engine message-window profile"
    );
    let ret = ctx.vm_call.as_ref().map_or(0, |call| call.ret_form);
    match op {
        codes::ELM_MWND_PRINT | codes::ELM_MWND____OVER_FLOW_PRINT => {
            ensure!(args.len() == 1, "MWND.PRINT requires one string");
            let text = args[0].as_str().context("MWND.PRINT string")?;
            append(ctx, text, -1)?;
            ctx.request_read_flag_no();
        }
        codes::ELM_MWND_SET_NAMAE | codes::ELM_MWND_NAMAE | codes::ELM_MWND____OVER_FLOW_NAMAE => {
            ensure!(args.len() == 1, "MWND.NAMAE requires one string");
            name(ctx, args[0].as_str().context("MWND.NAMAE string")?)?;
        }
        codes::ELM_MWND_CLEAR => {
            ensure!(args.is_empty(), "MWND.CLEAR arguments");
            clear_ready(ctx);
        }
        codes::ELM_MWND_MSG_BLOCK | codes::ELM_MWND_MSG_PP_BLOCK => {
            ensure!(args.is_empty(), "MWND.MSG_BLOCK arguments");
            begin_block(ctx);
        }
        codes::ELM_MWND_OPEN | codes::ELM_MWND_OPEN_WAIT | codes::ELM_MWND_OPEN_NOWAIT => {
            ensure!(args.is_empty(), "MWND.OPEN arguments");
            set_open(ctx, true, op != codes::ELM_MWND_OPEN_NOWAIT)?;
        }
        codes::ELM_MWND_CLOSE | codes::ELM_MWND_CLOSE_WAIT | codes::ELM_MWND_CLOSE_NOWAIT => {
            ensure!(args.is_empty(), "MWND.CLOSE arguments");
            if state(ctx).is_some_and(|m| m.open) {
                commit(ctx);
                set_open(ctx, false, op != codes::ELM_MWND_CLOSE_NOWAIT)?;
            }
        }
        codes::ELM_MWND_CHECK_OPEN => {
            ensure!(args.is_empty(), "MWND.CHECK_OPEN arguments");
            ctx.push(Value::Int(i64::from(state(ctx).is_some_and(|m| m.open))));
            return Ok(true);
        }
        codes::ELM_MWND_SET_WAKU_FILE | codes::ELM_MWND_SET_FILTER_FILE => {
            ensure!(args.len() == 1, "native MWND frame requires one filename");
            let file = args[0].as_str().context("native MWND frame filename")?;
            ensure!(file.len() <= 4096, "native MWND frame filename limit");
            if op == codes::ELM_MWND_SET_WAKU_FILE {
                state_mut(ctx).waku_file = file.into();
            } else {
                state_mut(ctx).filter_file = file.into();
            }
        }
        codes::ELM_MWND_GET_WAKU_FILE | codes::ELM_MWND_GET_FILTER_FILE => {
            ensure!(args.is_empty(), "native MWND frame getter arguments");
            let m = state_mut(ctx);
            let file = if op == codes::ELM_MWND_GET_WAKU_FILE {
                m.waku_file.clone()
            } else {
                m.filter_file.clone()
            };
            ctx.push(Value::Str(file));
            return Ok(true);
        }
        codes::ELM_MWND_NL | codes::ELM_MWND_NLI => {
            ensure!(args.is_empty(), "MWND.NL arguments");
            let text = if op == codes::ELM_MWND_NL {
                "\u{7}"
            } else {
                "\n"
            };
            let mut cursor = ctx.ui.cursor.clone();
            let laid = layout_cells(
                text,
                &ctx.ui.window.as_ref().unwrap().layout(false),
                &mut cursor,
            )?;
            ensure!(
                laid.consumed == text.len(),
                "native MWND newline page overflow"
            );
            ctx.ui.cursor = cursor;
            ctx.ui.message.get_or_insert_default().push_str(text);
            sync_native(ctx);
        }
        codes::ELM_MWND_WAIT_MSG
        | codes::ELM_MWND_PP
        | codes::ELM_MWND_R
        | codes::ELM_MWND_PAGE => {
            ensure!(args.is_empty(), "native MWND message wait arguments");
            ensure!(
                state(ctx).is_none_or(|m| m.novel_mode == 0),
                "native novel-mode clear is not mapped"
            );
            if op == codes::ELM_MWND_WAIT_MSG {
                ctx.wait.wait_message_reveal();
            } else {
                ctx.wait.wait_message_reveal_then_key();
            }
            ctx.ui.clear_after_key = matches!(op, codes::ELM_MWND_R | codes::ELM_MWND_PAGE);
            ctx.request_proc_boundary(ProcKind::MessageWait);
        }
        _ => anyhow::bail!("unsupported Siglus MWND operation {op}"),
    }
    if ret != 0 {
        ctx.push(Value::Int(0));
    }
    Ok(true)
}
