// SPDX-License-Identifier: MPL-2.0
use crate::runtime::CommandContext;
fn extra_default(ctx: &CommandContext, index: usize, mode: bool) -> i64 {
    ctx.tables
        .gameexe
        .as_ref()
        .and_then(|cfg| {
            cfg.get_indexed_field_unquoted(
                if mode {
                    "CONFIG.GLOBAL_EXTRA_MODE"
                } else {
                    "CONFIG.GLOBAL_EXTRA_SWITCH"
                },
                index,
                if mode { "MODE" } else { "ONOFF" },
            )
        })
        .and_then(|s| s.parse().ok())
        .unwrap_or(i64::from(!mode))
}
pub(crate) fn initialize_config(ctx: &mut CommandContext) {
    super::syscom_preferences::initialize(ctx);
    let mode = ctx
        .tables
        .gameexe
        .as_ref()
        .and_then(|cfg| cfg.get_i64("CONFIG.WINDOW_MODE"))
        .unwrap_or(0)
        .clamp(0, 1);
    ctx.globals
        .syscom
        .config_int
        .insert(super::codes::syscom_op::GET_WINDOW_MODE, mode);
    ctx.globals.syscom.original_config.screen_size_mode = mode;
    ctx.globals.syscom.original_config.global_extra_switch_flag =
        (0..4).map(|i| extra_default(ctx, i, false) != 0).collect();
    ctx.globals.syscom.original_config.global_extra_mode_flag =
        (0..4).map(|i| extra_default(ctx, i, true)).collect();
}
/// Game-defined extra switches/modes are native saved script policy, without
/// a second engine service or hard-coded title behavior.
pub(crate) fn dispatch_config(
    ctx: &mut CommandContext,
    args: &[crate::runtime::Value],
) -> anyhow::Result<bool> {
    if super::syscom_preferences::dispatch(ctx, args)? {
        return Ok(true);
    }
    if dispatch_controls(ctx, args)? {
        return Ok(true);
    }
    if super::syscom_flags::dispatch(ctx, args)? {
        return Ok(true);
    }
    use crate::runtime::{Value, forms::codes::syscom_op as op};
    use anyhow::{Context, ensure};
    let call = ctx.vm_call.as_ref().context("native SYSCOM metadata")?;
    ensure!(
        ctx.globals
            .syscom
            .original_config
            .global_extra_switch_flag
            .len()
            == 4
            && ctx
                .globals
                .syscom
                .original_config
                .global_extra_mode_flag
                .len()
                == 4,
        "native extra configuration array shape"
    );
    ensure!(
        call.element.len() == 2,
        "unsupported nested SYSCOM configuration"
    );
    let (mode, action) = match call.element[1] {
        op::SET_GLOBAL_EXTRA_SWITCH_ONOFF => (false, 0),
        op::SET_GLOBAL_EXTRA_SWITCH_ONOFF_DEFAULT => (false, 1),
        op::GET_GLOBAL_EXTRA_SWITCH_ONOFF => (false, 2),
        op::SET_GLOBAL_EXTRA_MODE_VALUE => (true, 0),
        op::SET_GLOBAL_EXTRA_MODE_VALUE_DEFAULT => (true, 1),
        op::GET_GLOBAL_EXTRA_MODE_VALUE => (true, 2),
        _ => return Ok(false),
    };
    ensure!(
        args.len() == if action == 0 { 2 } else { 1 },
        "native extra switch/mode arguments"
    );
    let index = args[0].as_i64().context("native extra switch/mode index")?;
    let valid = (0..4).contains(&index);
    if action == 2 {
        let cfg = &ctx.globals.syscom.original_config;
        ctx.push(Value::Int(if valid {
            if mode {
                cfg.global_extra_mode_flag[index as usize]
            } else {
                i64::from(cfg.global_extra_switch_flag[index as usize])
            }
        } else {
            0
        }));
    } else {
        let value = if action == 0 {
            args[1].as_i64().context("native extra switch/mode value")?
        } else {
            extra_default(ctx, index as usize, mode)
        };
        if valid {
            let cfg = &mut ctx.globals.syscom.original_config;
            if mode {
                cfg.global_extra_mode_flag[index as usize] = value;
            } else {
                cfg.global_extra_switch_flag[index as usize] = value != 0;
            }
        }
    }
    Ok(true)
}
fn dispatch_controls(
    ctx: &mut CommandContext,
    args: &[crate::runtime::Value],
) -> anyhow::Result<bool> {
    use crate::runtime::forms::codes::syscom_op as op;
    use anyhow::{Context, ensure};
    let call = ctx
        .vm_call
        .as_ref()
        .context("native SYSCOM controls metadata")?;
    let command = *call
        .element
        .get(1)
        .context("native SYSCOM controls operation")?;
    if matches!(
        command,
        op::SET_WINDOW_MODE | op::SET_WINDOW_MODE_DEFAULT | op::GET_WINDOW_MODE
    ) {
        ensure!(
            call.element.len() == 2 && call.al_id == 0,
            "native window-mode chain/overload"
        );
        ensure!(
            args.len() == usize::from(command == op::SET_WINDOW_MODE),
            "native window-mode arguments"
        );
        if command == op::GET_WINDOW_MODE {
            ctx.push(crate::runtime::Value::Int(
                ctx.globals
                    .syscom
                    .config_int
                    .get(&op::GET_WINDOW_MODE)
                    .copied()
                    .unwrap_or(0),
            ));
        } else {
            let value = if command == op::SET_WINDOW_MODE {
                args[0].as_i64().context("native window mode")?
            } else {
                ctx.tables
                    .gameexe
                    .as_ref()
                    .and_then(|c| c.get_i64("CONFIG.WINDOW_MODE"))
                    .unwrap_or(0)
                    .clamp(0, 1)
            };
            ctx.globals
                .syscom
                .config_int
                .insert(op::GET_WINDOW_MODE, value);
            ctx.globals.syscom.original_config.screen_size_mode = value.clamp(0, 1);
        }
        return Ok(true);
    }
    let indexed = matches!(command, op::SET_MWND_BTN_ENABLE | op::SET_MWND_BTN_DISABLE);
    if !indexed
        && !matches!(
            command,
            op::SET_SYSCOM_MENU_ENABLE
                | op::SET_SYSCOM_MENU_DISABLE
                | op::SET_MWND_BTN_TOUCH_ENABLE
                | op::SET_MWND_BTN_TOUCH_DISABLE
        )
    {
        return Ok(false);
    }
    ensure!(
        call.element.len() == 2,
        "unsupported native SYSCOM controls chain"
    );
    if indexed {
        ensure!(
            args.len() <= 1 && call.al_id == args.len() as i64,
            "native button-control overload/arguments"
        );
        let disabled = command == op::SET_MWND_BTN_DISABLE;
        if args.is_empty() {
            ctx.globals.syscom.mwnd_btn_disable_all = disabled;
        } else {
            let index = args[0].as_i64().context("native message button index")?;
            ensure!(
                i32::try_from(index).is_ok()
                    && (ctx.globals.syscom.mwnd_btn_disable.contains_key(&index)
                        || ctx.globals.syscom.mwnd_btn_disable.len() < 1024),
                "native button-control index/map budget"
            );
            ctx.globals.syscom.mwnd_btn_disable.insert(index, disabled);
        }
    } else {
        ensure!(
            args.is_empty() && call.al_id == 0,
            "native SYSCOM control arguments"
        );
        match command {
            op::SET_SYSCOM_MENU_ENABLE => ctx.globals.syscom.syscom_menu_disable = false,
            op::SET_SYSCOM_MENU_DISABLE => ctx.globals.syscom.syscom_menu_disable = true,
            op::SET_MWND_BTN_TOUCH_ENABLE => ctx.globals.syscom.mwnd_btn_touch_disable = false,
            op::SET_MWND_BTN_TOUCH_DISABLE => ctx.globals.syscom.mwnd_btn_touch_disable = true,
            _ => unreachable!(),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod config_tests {
    use super::*;
    use crate::runtime::{Value, VmCallMeta, forms::codes::syscom_op as op};
    #[test]
    fn game_extra_defaults_and_overrides_are_native_saved_state() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.tables.gameexe = Some(std::sync::Arc::new(
            crate::gameexe::GameexeConfig::from_text(
                "#CONFIG.GLOBAL_EXTRA_SWITCH.000.ONOFF=0\n#CONFIG.GLOBAL_EXTRA_MODE.001.MODE=9\n",
            ),
        ));
        initialize_config(&mut ctx);
        assert!(!ctx.globals.syscom.original_config.global_extra_switch_flag[0]);
        assert_eq!(
            ctx.globals.syscom.original_config.global_extra_mode_flag[1],
            9
        );
        ctx.vm_call = Some(VmCallMeta {
            element: vec![63, op::SET_GLOBAL_EXTRA_SWITCH_ONOFF],
            ..Default::default()
        });
        dispatch_config(&mut ctx, &[Value::Int(0), Value::Int(1)]).unwrap();
        ctx.vm_call.as_mut().unwrap().element[1] = op::GET_GLOBAL_EXTRA_SWITCH_ONOFF;
        dispatch_config(&mut ctx, &[Value::Int(0)]).unwrap();
        assert_eq!(ctx.pop().and_then(|v| v.as_i64()), Some(1));
        ctx.vm_call.as_mut().unwrap().element[1] = op::SET_GLOBAL_EXTRA_SWITCH_ONOFF_DEFAULT;
        dispatch_config(&mut ctx, &[Value::Int(0)]).unwrap();
        assert!(!ctx.globals.syscom.original_config.global_extra_switch_flag[0]);
        let checkpoint = ctx.clone();
        assert!(
            std::sync::Arc::ptr_eq(
                ctx.tables.gameexe.as_ref().unwrap(),
                checkpoint.tables.gameexe.as_ref().unwrap()
            ),
            "checkpoints share immutable config"
        );
        ctx.globals.syscom.original_config.global_extra_mode_flag[1] = 3;
        assert_eq!(
            checkpoint
                .globals
                .syscom
                .original_config
                .global_extra_mode_flag[1],
            9,
            "mutable native config remains independent"
        );
    }
}
pub const CAPTURE_PRIOR_SAVE: i32 = 0;
pub fn prepare_runtime_save_thumb_capture(ctx: &mut CommandContext) {
    ctx.unknown
        .record_note("native save thumbnail capture unsupported");
}
pub fn free_runtime_save_thumb_capture(_: &mut CommandContext, _: i32) {}
pub fn write_runtime_slot_thumb(ctx: &mut CommandContext, _: usize) {
    ctx.unknown
        .record_note("native slot thumbnail write unsupported");
}
pub fn write_global_save(ctx: &CommandContext) {
    ctx.backend_faults
        .borrow_mut()
        .push("native global/config save backend unavailable".into());
}
pub fn read_save_flag_values(
    ctx: &mut CommandContext,
    quick: bool,
    idx: i64,
    count: usize,
) -> Option<Vec<i64>> {
    if idx < 0 {
        return None;
    }
    let slots = if quick {
        &ctx.globals.syscom.quick_save_slots
    } else {
        &ctx.globals.syscom.save_slots
    };
    let slot = slots.get(idx as usize).filter(|s| s.exist)?;
    Some(
        (0..count.min(256))
            .map(|i| slot.values.get(&(i as i32)).copied().unwrap_or(0))
            .collect(),
    )
}
pub fn write_save_flag_values(ctx: &mut CommandContext, quick: bool, idx: i64, v: &[i64]) -> bool {
    if idx < 0 {
        return false;
    }
    let slots = if quick {
        &mut ctx.globals.syscom.quick_save_slots
    } else {
        &mut ctx.globals.syscom.save_slots
    };
    let Some(slot) = slots.get_mut(idx as usize).filter(|s| s.exist) else {
        return false;
    };
    for (i, v) in v.iter().take(256).enumerate() {
        slot.values.insert(i as i32, *v);
    }
    true
}
pub fn append_current_save_message(ctx: &mut CommandContext, msg: &str) {
    ctx.globals.syscom.current_save_message.push_str(msg);
    ctx.globals.syscom.current_save_message = ctx
        .globals
        .syscom
        .current_save_message
        .chars()
        .take(256)
        .collect();
    ctx.globals.syscom.current_save_full_message.push_str(msg);
    ctx.globals.syscom.current_save_full_message = ctx
        .globals
        .syscom
        .current_save_full_message
        .chars()
        .take(256)
        .collect();
    if let Some(s) = &mut ctx.local_save_snapshot {
        s.save_msg = ctx.globals.syscom.current_save_message.clone();
        s.save_full_msg = ctx.globals.syscom.current_save_full_message.clone();
    }
}
