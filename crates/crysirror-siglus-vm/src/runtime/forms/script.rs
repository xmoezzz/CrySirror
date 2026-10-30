// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Script-local pointer policy. The adapter publishes engine UI cursor state.
use super::codes;
use crate::runtime::{CommandContext, Value};
use anyhow::{Context, Result, ensure};
/// Original eng_etc.cpp skip precedence, shared by the native query and
/// native frame's 32x game-time selection. No engine playback timer lives here.
pub(crate) fn is_skipping(ctx: &CommandContext) -> Result<bool> {
    let ctrl = ctx.globals.focused_editbox.is_none()
        && ctx.input.vk_is_down(0x11)
        && !ctx.input.vk_is_down(0x10);
    let value = if ctx.globals.syscom.msg_back_open || ctx.globals.script.ctrl_disable {
        false
    } else if ctrl {
        true
    } else if ctx.globals.script.skip_disable {
        false
    } else {
        ensure!(
            !ctx.globals.script.skip_trigger,
            "native transient trigger-skip execution profile unsupported"
        );
        ctx.globals.syscom.read_skip.onoff
    };
    Ok(value)
}
pub fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let call = ctx
        .vm_call
        .as_ref()
        .context("native SCRIPT call metadata")?;
    ensure!(
        call.element.len() == 2,
        "unsupported nested native SCRIPT operation"
    );
    let op = call.element[1];
    let ret = call.ret_form;
    match op {
        codes::ELM_SCRIPT_SET_CTRL_SKIP_DISABLE
        | codes::ELM_SCRIPT_SET_CTRL_SKIP_ENABLE
        | codes::ELM_SCRIPT_SET_SKIP_DISABLE
        | codes::ELM_SCRIPT_SET_SKIP_ENABLE
        | codes::ELM_SCRIPT_SET_MSG_BACK_DISABLE
        | codes::ELM_SCRIPT_SET_MSG_BACK_ENABLE
        | codes::ELM_SCRIPT_SET_HIDE_MWND_DISABLE
        | codes::ELM_SCRIPT_SET_HIDE_MWND_ENABLE => {
            ensure!(args.is_empty(), "native script policy toggle arguments");
            match op {
                codes::ELM_SCRIPT_SET_CTRL_SKIP_DISABLE => ctx.globals.script.ctrl_disable = true,
                codes::ELM_SCRIPT_SET_CTRL_SKIP_ENABLE => ctx.globals.script.ctrl_disable = false,
                codes::ELM_SCRIPT_SET_SKIP_DISABLE => ctx.globals.script.skip_disable = true,
                codes::ELM_SCRIPT_SET_SKIP_ENABLE => ctx.globals.script.skip_disable = false,
                codes::ELM_SCRIPT_SET_MSG_BACK_DISABLE => {
                    ctx.globals.script.msg_back_disable = true
                }
                codes::ELM_SCRIPT_SET_MSG_BACK_ENABLE => {
                    ctx.globals.script.msg_back_disable = false
                }
                codes::ELM_SCRIPT_SET_HIDE_MWND_DISABLE => {
                    ctx.globals.script.hide_mwnd_disable = true
                }
                codes::ELM_SCRIPT_SET_HIDE_MWND_ENABLE => {
                    ctx.globals.script.hide_mwnd_disable = false
                }
                _ => unreachable!(),
            }
        }
        codes::ELM_SCRIPT_CHECK_SKIP => {
            ensure!(args.is_empty(), "SCRIPT.CHECK_SKIP arguments");
            let value = is_skipping(ctx)?;
            ctx.push(Value::Int(i64::from(value)));
            return Ok(true);
        }
        codes::ELM_SCRIPT_SET_FONT_BOLD | codes::ELM_SCRIPT_SET_FONT_SHADOW => {
            ensure!(args.len() == 1, "native font override requires one integer");
            let value = args[0].as_i64().context("native font override")?;
            ensure!(
                i32::try_from(value).is_ok(),
                "native font override integer width"
            );
            if op == codes::ELM_SCRIPT_SET_FONT_BOLD {
                ctx.globals.script.font_bold = value;
            } else {
                ctx.globals.script.font_shadow = value;
            }
        }
        codes::ELM_SCRIPT_SET_FONT_BOLD_DEFAULT | codes::ELM_SCRIPT_SET_FONT_SHADOW_DEFAULT => {
            ensure!(args.is_empty(), "native default font override arguments");
            if op == codes::ELM_SCRIPT_SET_FONT_BOLD_DEFAULT {
                ctx.globals.script.font_bold = -1;
            } else {
                ctx.globals.script.font_shadow = -1;
            }
        }
        codes::ELM_SCRIPT_GET_FONT_BOLD | codes::ELM_SCRIPT_GET_FONT_SHADOW => {
            ensure!(args.is_empty(), "native font query arguments");
            ctx.push(Value::Int(if op == codes::ELM_SCRIPT_GET_FONT_BOLD {
                ctx.globals.script.font_bold
            } else {
                ctx.globals.script.font_shadow
            }));
            return Ok(true);
        }
        codes::ELM_SCRIPT_SET_CURSOR_NO => {
            ensure!(args.len() == 1, "SCRIPT.SET_CURSOR_NO requires one integer");
            let no = args[0].as_i64().context("native cursor index")?;
            ensure!((-1..256).contains(&no), "native cursor index limit");
            ctx.globals.script.cursor_no = no;
        }
        codes::ELM_SCRIPT_GET_CURSOR_NO => {
            ensure!(args.is_empty(), "SCRIPT.GET_CURSOR_NO arguments");
            ctx.push(Value::Int(ctx.globals.script.cursor_no));
            return Ok(true);
        }
        codes::ELM_SCRIPT_SET_MOUSE_DISP_OFF | codes::ELM_SCRIPT_SET_MOUSE_DISP_ON => {
            ensure!(args.is_empty(), "native cursor visibility arguments");
            ctx.globals.script.cursor_disp_off = op == codes::ELM_SCRIPT_SET_MOUSE_DISP_OFF;
        }
        _ => anyhow::bail!("unsupported Siglus SCRIPT operation {op}"),
    }
    if ret != 0 {
        ctx.push(Value::Int(0));
    }
    Ok(true)
}

#[cfg(test)]
mod font_tests {
    use super::*;
    #[test]
    fn font_overrides_keep_raw_default_values_and_validate_before_mutation() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(crate::runtime::VmCallMeta {
            element: vec![72, codes::ELM_SCRIPT_SET_FONT_SHADOW],
            ..Default::default()
        });
        dispatch(&mut ctx, &[Value::Int(3)]).unwrap();
        assert_eq!(ctx.globals.script.font_shadow, 3);
        assert!(dispatch(&mut ctx, &[Value::Int(i64::MAX)]).is_err());
        assert_eq!(ctx.globals.script.font_shadow, 3);
        ctx.vm_call.as_mut().unwrap().element[1] = codes::ELM_SCRIPT_GET_FONT_SHADOW;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(3));
        ctx.vm_call.as_mut().unwrap().element[1] = codes::ELM_SCRIPT_SET_FONT_SHADOW_DEFAULT;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.globals.script.font_shadow, -1);
        ctx.vm_call.as_mut().unwrap().element[1] = codes::ELM_SCRIPT_GET_FONT_SHADOW;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(-1));
    }
    #[test]
    fn skip_query_preserves_ctrl_disable_focus_and_shift_precedence() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(crate::runtime::VmCallMeta {
            element: vec![64, codes::ELM_SCRIPT_CHECK_SKIP],
            ..Default::default()
        });
        ctx.input.on_key_down(crate::runtime::input::VmKey::Control);
        ctx.globals.script.skip_disable = true;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(1));
        ctx.globals.script.ctrl_disable = true;
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(0));
        ctx.globals.script.ctrl_disable = false;
        ctx.input.on_key_down(crate::runtime::input::VmKey::Shift);
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(0));
        ctx.input.on_key_up(crate::runtime::input::VmKey::Shift);
        ctx.globals.focused_editbox = Some((1, 0));
        dispatch(&mut ctx, &[]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(0));
    }
}
