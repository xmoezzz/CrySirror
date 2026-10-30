// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Queries of original script feature flags, not host capability claims.
//! An actual unmapped skip/menu/save action still fails at its native command.
use crate::runtime::{CommandContext, Value, forms::codes::syscom_op as op};
use anyhow::{Context, Result, ensure};
pub(super) fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let call = ctx
        .vm_call
        .as_ref()
        .context("native SYSCOM flags metadata")?;
    let command = *call
        .element
        .get(1)
        .context("native SYSCOM flag operation")?;
    let activation = matches!(
        command,
        op::SET_READ_SKIP_ONOFF_FLAG
            | op::SET_AUTO_SKIP_ONOFF_FLAG
            | op::SET_AUTO_MODE_ONOFF_FLAG
            | op::SET_HIDE_MWND_ONOFF_FLAG
    );
    let setter = matches!(
        command,
        op::SET_READ_SKIP_ONOFF_FLAG
            | op::SET_READ_SKIP_ENABLE_FLAG
            | op::SET_READ_SKIP_EXIST_FLAG
            | op::SET_AUTO_SKIP_ONOFF_FLAG
            | op::SET_AUTO_SKIP_ENABLE_FLAG
            | op::SET_AUTO_SKIP_EXIST_FLAG
            | op::SET_AUTO_MODE_ONOFF_FLAG
            | op::SET_AUTO_MODE_ENABLE_FLAG
            | op::SET_AUTO_MODE_EXIST_FLAG
            | op::SET_HIDE_MWND_ONOFF_FLAG
            | op::SET_HIDE_MWND_ENABLE_FLAG
            | op::SET_HIDE_MWND_EXIST_FLAG
            | op::SET_MSG_BACK_ENABLE_FLAG
            | op::SET_MSG_BACK_EXIST_FLAG
            | op::SET_RETURN_TO_SEL_ENABLE_FLAG
            | op::SET_RETURN_TO_SEL_EXIST_FLAG
            | op::SET_RETURN_TO_MENU_ENABLE_FLAG
            | op::SET_RETURN_TO_MENU_EXIST_FLAG
            | op::SET_END_GAME_ENABLE_FLAG
            | op::SET_END_GAME_EXIST_FLAG
            | op::SET_SAVE_ENABLE_FLAG
            | op::SET_SAVE_EXIST_FLAG
            | op::SET_LOAD_ENABLE_FLAG
            | op::SET_LOAD_EXIST_FLAG
    );
    if setter {
        ensure!(
            call.element.len() == 2 && call.al_id == 0 && args.len() == 1,
            "native feature setter chain/arguments"
        );
        let value = args[0].as_i64().context("native feature flag value")? != 0;
        ensure!(
            !activation || !value,
            "native feature activation {command} needs its skip/auto/hide execution profile"
        );
        let st = &mut ctx.globals.syscom;
        let flag = match command {
            op::SET_READ_SKIP_ONOFF_FLAG => &mut st.read_skip.onoff,
            op::SET_READ_SKIP_ENABLE_FLAG => &mut st.read_skip.enable,
            op::SET_READ_SKIP_EXIST_FLAG => &mut st.read_skip.exist,
            op::SET_AUTO_SKIP_ONOFF_FLAG => &mut st.auto_skip.onoff,
            op::SET_AUTO_SKIP_ENABLE_FLAG => &mut st.auto_skip.enable,
            op::SET_AUTO_SKIP_EXIST_FLAG => &mut st.auto_skip.exist,
            op::SET_AUTO_MODE_ONOFF_FLAG => &mut st.auto_mode.onoff,
            op::SET_AUTO_MODE_ENABLE_FLAG => &mut st.auto_mode.enable,
            op::SET_AUTO_MODE_EXIST_FLAG => &mut st.auto_mode.exist,
            op::SET_HIDE_MWND_ONOFF_FLAG => &mut st.hide_mwnd.onoff,
            op::SET_HIDE_MWND_ENABLE_FLAG => &mut st.hide_mwnd.enable,
            op::SET_HIDE_MWND_EXIST_FLAG => &mut st.hide_mwnd.exist,
            op::SET_MSG_BACK_ENABLE_FLAG => &mut st.msg_back.enable,
            op::SET_MSG_BACK_EXIST_FLAG => &mut st.msg_back.exist,
            op::SET_RETURN_TO_SEL_ENABLE_FLAG => &mut st.return_to_sel.enable,
            op::SET_RETURN_TO_SEL_EXIST_FLAG => &mut st.return_to_sel.exist,
            op::SET_RETURN_TO_MENU_ENABLE_FLAG => &mut st.return_to_menu.enable,
            op::SET_RETURN_TO_MENU_EXIST_FLAG => &mut st.return_to_menu.exist,
            op::SET_END_GAME_ENABLE_FLAG => &mut st.end_game.enable,
            op::SET_END_GAME_EXIST_FLAG => &mut st.end_game.exist,
            op::SET_SAVE_ENABLE_FLAG => &mut st.save_feature.enable,
            op::SET_SAVE_EXIST_FLAG => &mut st.save_feature.exist,
            op::SET_LOAD_ENABLE_FLAG => &mut st.load_feature.enable,
            op::SET_LOAD_EXIST_FLAG => &mut st.load_feature.exist,
            _ => unreachable!(),
        };
        *flag = value;
        return Ok(true);
    }
    let st = &ctx.globals.syscom;
    let value = match command {
        op::GET_READ_SKIP_ONOFF_FLAG => st.read_skip.onoff,
        op::GET_READ_SKIP_ENABLE_FLAG => st.read_skip.enable,
        op::GET_READ_SKIP_EXIST_FLAG => st.read_skip.exist,
        op::CHECK_READ_SKIP_ENABLE => st.read_skip.enable && st.read_skip.exist,
        op::GET_AUTO_SKIP_ONOFF_FLAG => st.auto_skip.onoff,
        op::GET_AUTO_SKIP_ENABLE_FLAG => st.auto_skip.enable,
        op::GET_AUTO_SKIP_EXIST_FLAG => st.auto_skip.exist,
        op::CHECK_AUTO_SKIP_ENABLE => st.auto_skip.enable && st.auto_skip.exist,
        op::GET_AUTO_MODE_ONOFF_FLAG => st.auto_mode.onoff,
        op::GET_AUTO_MODE_ENABLE_FLAG => st.auto_mode.enable,
        op::GET_AUTO_MODE_EXIST_FLAG => st.auto_mode.exist,
        op::CHECK_AUTO_MODE_ENABLE => st.auto_mode.enable && st.auto_mode.exist,
        op::GET_HIDE_MWND_ONOFF_FLAG => st.hide_mwnd.onoff,
        op::GET_HIDE_MWND_ENABLE_FLAG => st.hide_mwnd.enable,
        op::GET_HIDE_MWND_EXIST_FLAG => st.hide_mwnd.exist,
        op::CHECK_HIDE_MWND_ENABLE => st.hide_mwnd.enable && st.hide_mwnd.exist,
        op::GET_MSG_BACK_ENABLE_FLAG => st.msg_back.enable,
        op::GET_MSG_BACK_EXIST_FLAG => st.msg_back.exist,
        op::CHECK_MSG_BACK_ENABLE => st.msg_back.enable && st.msg_back.exist,
        op::CHECK_MSG_BACK_OPEN => st.msg_back_open,
        op::GET_RETURN_TO_SEL_ENABLE_FLAG => st.return_to_sel.enable,
        op::GET_RETURN_TO_SEL_EXIST_FLAG => st.return_to_sel.exist,
        op::CHECK_RETURN_TO_SEL_ENABLE => st.return_to_sel.enable && st.return_to_sel.exist,
        op::GET_RETURN_TO_MENU_ENABLE_FLAG => st.return_to_menu.enable,
        op::GET_RETURN_TO_MENU_EXIST_FLAG => st.return_to_menu.exist,
        op::CHECK_RETURN_TO_MENU_ENABLE => st.return_to_menu.enable && st.return_to_menu.exist,
        op::GET_END_GAME_ENABLE_FLAG => st.end_game.enable,
        op::GET_END_GAME_EXIST_FLAG => st.end_game.exist,
        op::CHECK_END_GAME_ENABLE => st.end_game.enable && st.end_game.exist,
        op::GET_SAVE_ENABLE_FLAG => st.save_feature.enable,
        op::GET_SAVE_EXIST_FLAG => st.save_feature.exist,
        op::CHECK_SAVE_ENABLE => st.save_feature.enable && st.save_feature.exist,
        op::GET_LOAD_ENABLE_FLAG => st.load_feature.enable,
        op::GET_LOAD_EXIST_FLAG => st.load_feature.exist,
        op::CHECK_LOAD_ENABLE => st.load_feature.enable && st.load_feature.exist,
        _ => return Ok(false),
    };
    ensure!(
        call.element.len() == 2 && call.al_id == 0 && args.is_empty(),
        "native feature query chain/arguments"
    );
    ctx.push(Value::Int(i64::from(value)));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queries_observe_native_flags_without_acknowledging_unmapped_actions() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(crate::runtime::VmCallMeta {
            element: vec![63, op::GET_AUTO_SKIP_ONOFF_FLAG],
            ..Default::default()
        });
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(0));
        ctx.globals.syscom.auto_skip.onoff = true;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(1));
        ctx.vm_call.as_mut().unwrap().element[1] = op::CALL_SAVE_MENU;
        assert!(!dispatch(&mut ctx, &[]).unwrap());
        ctx.vm_call.as_mut().unwrap().element = vec![63, op::GET_AUTO_SKIP_ONOFF_FLAG, 999];
        assert!(dispatch(&mut ctx, &[]).is_err());
        assert!(ctx.stack.is_empty());
    }
}
