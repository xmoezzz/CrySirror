// SPDX-License-Identifier: MPL-2.0
// Original siglus_rs RealLive VM source, commit 5f358ef1360b0439e92aed2b749e8f25d8721954
// (`modules/sys.rs`): the 1:4 functions that only read or write VM state —
// arithmetic, date/time, player settings and system-command values. Every
// function body below is copied unchanged; the adapter handles the rest.
use anyhow::Result;

use crate::vm::bytecode::Command;
use crate::vm::clock::{FrameCounter, FrameKind};
use crate::vm::expr::Expr;
use crate::vm::machine::{Frame, FrameKind as CallKind, IntTarget, Machine, Next};
use crate::vm::settings::{WindowAttr, syscom};

/// Some(next) when the opcode is one of the VM-state functions copied here.
pub fn dispatch(machine: &mut Machine, command: &Command) -> Result<Option<Next>> {
    let op = command.op;
    let now = machine.sys.now();
    match op.opcode {
        // InitFrame* / InitExFrame*
        500..=504 | 520..=524 => {
            let counter = machine.int_param(command, 0)?;
            let from = machine.int_param(command, 1)?;
            let to = machine.int_param(command, 2)?;
            let time = machine.int_param(command, 3)?;
            let kind = FrameKind::from_opcode(op.opcode);
            machine.sys.frames.set(
                frame_layer(op.opcode),
                counter,
                FrameCounter::new(kind, from, to, time, now),
            );
        }
        // ReadFrame / FrameActive / AnyFrameActive (and Ex)
        510 | 530 => {
            let counter = machine.int_param(command, 0)?;
            machine.store = machine
                .sys
                .frames
                .read(frame_layer(op.opcode), counter, now);
        }
        511 | 531 => {
            let counter = machine.int_param(command, 0)?;
            machine.store = i32::from(machine.sys.frames.active(
                frame_layer(op.opcode),
                counter,
                now,
            ));
        }
        512 | 532 => {
            machine.store = i32::from(machine.sys.frames.any_active(frame_layer(op.opcode), now));
        }
        // ClearFrame(counter[, value]) / ClearAllFrames([value])
        513 | 533 => {
            let counter = machine.int_param(command, 0)?;
            let value = machine.int_param_or(command, 1, 0)?;
            if let Some(frame) = machine.sys.frames.get_mut(frame_layer(op.opcode), counter) {
                frame.stop(value);
            }
        }
        514 | 534 => {
            let value = machine.int_param_or(command, 0, 0)?;
            machine.sys.frames.clear_all(frame_layer(op.opcode), value);
        }
        // InitFrames*({counter, from, to, time}...)
        600..=604 | 620..=624 => {
            let kind = FrameKind::from_opcode(op.opcode);
            for index in 0..command.params.len() {
                let pieces = machine.complex_param(command, index)?;
                let values = pieces
                    .iter()
                    .map(|piece| machine.eval_int(piece))
                    .collect::<Result<Vec<_>>>()?;
                if let [counter, from, to, time, ..] = values[..] {
                    machine.sys.frames.set(
                        frame_layer(op.opcode),
                        counter,
                        FrameCounter::new(kind, from, to, time, now),
                    );
                }
            }
        }
        // ReadFrames({counter, var}...)
        610 | 630 => {
            let layer = frame_layer(op.opcode);
            let mut any = false;
            for index in 0..command.params.len() {
                let pieces = machine.complex_param(command, index)?;
                let (Some(counter), Some(target)) = (pieces.first(), pieces.get(1)) else {
                    continue;
                };
                let counter = machine.eval_int(counter)?;
                let value = machine.sys.frames.read(layer, counter, now);
                any |= machine.sys.frames.active(layer, counter, now);
                let target = machine.int_target(target)?;
                machine.set_target(target, value)?;
            }
            machine.store = i32::from(any);
        }
        800 => machine.store = index_series(machine, command, false)?,
        // rnd([min], max)
        1000 => {
            let values = machine.int_params_from(command, 0)?;
            machine.store = match values[..] {
                [max] => machine.sys.random(0, max),
                [min, max, ..] => machine.sys.random(min, max),
                [] => 0,
            };
        }
        // pcnt(n, d)
        1001 => {
            let n = machine.int_param(command, 0)?;
            let d = machine.int_param(command, 1)?;
            machine.store = if d == 0 {
                0
            } else {
                (f64::from(n) / f64::from(d) * 100.0).round() as i32
            };
        }
        1002 => machine.store = machine.int_param(command, 0)?.wrapping_abs(),
        // power(base[, exponent])
        1003 => {
            let base = machine.int_param(command, 0)?;
            let exponent = machine.int_param_or(command, 1, 2)?;
            machine.store = if exponent < 0 {
                0
            } else {
                base.wrapping_pow(exponent as u32)
            };
        }
        // sin / cos (value[, divisor]) and the undocumented variants
        1004 | 1010 | 1012 | 1013 => {
            let value = machine.int_param(command, 0)?;
            let divisor = machine.int_param_or(command, 1, 0)?;
            let f = if matches!(op.opcode, 1004 | 1012) {
                f64::sin
            } else {
                f64::cos
            };
            machine.store = trig(value, divisor, f);
        }
        // modulus(x1, y1, x2, y2) / angle(...)
        1005 | 1006 => {
            let v = machine.int_params_from(command, 0)?;
            let [x1, y1, x2, y2] = [0, 1, 2, 3].map(|i| f64::from(v.get(i).copied().unwrap_or(0)));
            let (dx, dy) = (x2 - x1, y2 - y1);
            machine.store = if op.opcode == 1005 {
                (dx * dx + dy * dy).sqrt() as i32
            } else {
                // 0 degrees points up, increasing clockwise.
                let degrees = dx.atan2(-dy).to_degrees();
                ((degrees + 360.0) % 360.0) as i32
            };
        }
        1007 => {
            let a = machine.int_param(command, 0)?;
            let b = machine.int_param(command, 1)?;
            machine.store = a.min(b);
        }
        1008 => {
            let a = machine.int_param(command, 0)?;
            let b = machine.int_param(command, 1)?;
            machine.store = a.max(b);
        }
        // constrain(min, value, max)
        1009 => {
            let min = machine.int_param(command, 0)?;
            let value = machine.int_param(command, 1)?;
            let max = machine.int_param(command, 2)?;
            machine.store = value.max(min).min(max.max(min));
        }
        1011 => machine.store = machine.int_param(command, 0)?.signum(),
        1100..=1107 => machine.store = now_parts()[usize::from(op.opcode - 1100)],
        // GetDate / GetTime / GetDateTime
        1110 => write_targets(machine, command, 0, &now_parts()[..4])?,
        1111 => write_targets(machine, command, 0, &now_parts()[4..])?,
        1112 => write_targets(machine, command, 0, &now_parts())?,
        1120 => machine.store = machine.scene_number(),
        // CLEAR_MSGBK / SET_MSGBK_ON / SET_MSGBK_OFF
        300 => {}
        301 => machine.sys.backlog_enabled = true,
        302 => machine.sys.backlog_enabled = false,
        // SET_READJUMPCANCEL_ON / _OFF
        335 => machine.sys.read_jump_cancel = true,
        336 => machine.sys.read_jump_cancel = false,
        // SET_CTRLKEY_ENABLE / SET_CTRLKEY_DISENABLE
        358 => machine.sys.ctrl_key_skip = true,
        359 => machine.sys.ctrl_key_skip = false,
        // SET_P_CURSORNO(id)
        366 => machine.sys.key_cursor = machine.int_param(command, 0)?,
        // TIMETABLE2 / TIMETABLELEN2
        810 | 811 => machine.store = timetable2(machine, command, op.opcode == 811)?,
        // KeyMouseOn / KeyMouseOff
        200 => machine.sys.input.key_mouse = true,
        201 => machine.sys.input.key_mouse = false,
        206 => machine.store = machine.sys.mouse_cursor,
        207 => machine.sys.mouse_cursor = machine.int_param(command, 0)?,
        // CallStackNop([count])
        321 => {
            let count = machine.int_param_or(command, 0, 1)?.max(0) as usize;
            let current = machine.frame().clone();
            let frame = Frame {
                ip: current.ip + 1,
                kind: CallKind::Gosub,
                ..current
            };
            let top = machine.stack.len() - 1;
            for _ in 0..count {
                machine.stack.insert(top, frame.clone());
            }
        }
        // CallStackPop([count]): returns `count` levels.
        322 => {
            let count = machine.int_param_or(command, 0, 1)?.max(0) as usize;
            let keep = machine.stack.len().saturating_sub(count).max(1);
            machine.stack.truncate(keep);
            return Ok(Some(Next::Jumped));
        }
        323 => machine.store = machine.stack.len() as i32,
        // CallStackTrunc(length)
        324 => {
            let length = machine.int_param(command, 0)?.max(1) as usize;
            if length < machine.stack.len() {
                machine.stack.truncate(length);
                return Ok(Some(Next::Jumped));
            }
        }
        // EnableSkipMode / DisableSkipMode
        330 => machine.sys.syscom.skip_mode_allowed = true,
        331 => machine.sys.syscom.skip_mode_allowed = false,
        // LocalSkipMode / SetLocalSkipMode / ClearLocalSkipMode
        332 => machine.store = i32::from(machine.sys.syscom.skip_mode),
        333 => machine.sys.syscom.skip_mode = true,
        334 => machine.sys.syscom.skip_mode = false,
        // CtrlKeySkip / On / Off / CtrlPressed / ShiftPressed
        350 => machine.store = i32::from(machine.sys.ctrl_key_skip),
        351 => machine.sys.ctrl_key_skip = true,
        352 => machine.sys.ctrl_key_skip = false,
        353 => machine.store = i32::from(machine.sys.input.ctrl),
        354 => machine.store = i32::from(machine.sys.input.shift),
        // PauseCursor(index)
        364 => machine.sys.key_cursor = machine.int_param(command, 0)?,
        // GetWakuAll / SetWakuAll
        410 => machine.store = machine.sys.settings.waku_all,
        411 => machine.sys.settings.waku_all = machine.int_param(command, 0)?,
        // GetWindowAttr2 / SetWindowAttr2: the global attributes.
        420 => {
            let values = window_attr_values(machine.sys.settings.window_attr);
            write_targets(machine, command, 0, &values)?;
        }
        421 => {
            let values = machine.int_params_from(command, 0)?;
            if let Some(attr) = WindowAttr::from_slice(&values) {
                machine.sys.settings.window_attr = attr;
            }
        }
        // Undocumented get/enable/disable flag triples.
        430..=457 => {
            let base = 430 + (op.opcode - 430) / 5 * 5;
            let slot = usize::from((op.opcode - 430) / 5);
            let flags = &mut machine.sys.misc_flags;
            if flags.len() <= slot {
                flags.resize(slot + 1, 0);
            }
            match op.opcode - base {
                0 => machine.store = flags[slot],
                1 => flags[slot] = 1,
                2 => flags[slot] = 0,
                _ => {}
            }
        }
        // CALL_SYSTEMMENU_MOVIESUPPORT / CALL_SYSTEMMENU_MMXSUPPORT: the
        // original's hardware-support dialogs; nothing to configure here.
        1218 | 1260 => {}
        // SET_SELPOINTMOD_ON / _OFF, SELPOINT, SET_SELPOINT_CLEAR
        1221 => machine.selpoint_auto = true,
        1222 => machine.selpoint_auto = false,
        1231 => machine.previous_selection = None,
        // EnableSyscom([n]) / HideSyscom([n]) / DisableSyscom(n)
        1211 | 1212 => {
            let state = if op.opcode == 1211 { 1 } else { 0 };
            match command.params.first() {
                Some(_) => {
                    let index = machine.int_param(command, 0)?;
                    machine.sys.syscom.set_state(index, state);
                }
                None => machine.sys.syscom.menu_enabled = state == 1,
            }
        }
        1213 => {
            let index = machine.int_param(command, 0)?;
            machine.sys.syscom.set_state(index, 2);
        }
        1214 => {
            let index = machine.int_param(command, 0)?;
            machine.store = machine.sys.syscom.state(index);
        }
        1216 => {
            let index = machine.int_param(command, 0)?;
            machine.store = read_syscom(machine, index);
        }
        // ENABLE_SYSTEMMENU_ANIME / DISENABLE_SYSTEMMENU_ANIME
        2402 => machine.sys.system_menu_animation = true,
        2502 => machine.sys.system_menu_animation = false,
        // Miscellaneous flags: getters 2000..2009, setters 2050..2059
        2000..=2009 | 2050..=2059 => {
            let setter = op.opcode >= 2050;
            let which = op.opcode % 50;
            let value = if setter {
                machine.int_param(command, 0)?
            } else {
                0
            };
            let settings = &mut machine.sys.settings;
            let slot: Option<&mut bool> = match which {
                0 => Some(&mut settings.cursor_mono),
                1 => Some(&mut settings.skip_animations),
                2 => Some(&mut settings.low_priority),
                3 => Some(&mut settings.confirm_save_load),
                4 => Some(&mut settings.reduce_distortion),
                _ => None,
            };
            match (slot, setter) {
                (Some(flag), true) => *flag = value != 0,
                (Some(flag), false) => machine.store = i32::from(*flag),
                (None, true) if which == 9 => settings.sound_quality = value,
                (None, false) if which == 9 => machine.store = settings.sound_quality,
                // Undocumented flags (5..=8): remembered as set.
                (None, setter) => {
                    let flags = &mut machine.sys.misc_flags;
                    if flags.len() <= which as usize {
                        flags.resize(which as usize + 1, 0);
                    }
                    if setter {
                        flags[which as usize] = value;
                    } else {
                        machine.store = flags[which as usize];
                    }
                }
            }
        }
        // SAVEPOINT_CLEAR: forget the old savepoint; saving now keeps this
        // position.
        3503 => machine.mark_savepoint(),
        // Savepoint / EnableAutoSavepoints / DisableAutoSavepoints
        3500 => machine.mark_savepoint(),
        3501 => machine.mark_savepoints = true,
        3502 => machine.mark_savepoints = false,
        // end()
        1200 => {
            machine.halt();
            return Ok(Some(Next::Jumped));
        }
        1131 => machine.sys.default_grp = machine.str_param(command, 0)?,
        1133 => machine.sys.default_bgr = machine.str_param(command, 0)?,
        2221..=2276 | 2321..=2376 | 2600..=2621 => return settings_op(machine, command).map(Some),
        _ => return Ok(None),
    }
    Ok(Some(Next::Advance))
}

fn frame_layer(opcode: u16) -> usize {
    usize::from(matches!(opcode, 520..=534 | 620..=634))
}

fn trig(value: i32, divisor: i32, f: fn(f64) -> f64) -> i32 {
    let result = f(f64::from(value).to_radians()) * 32640.0;
    let result = if divisor != 0 {
        result / f64::from(divisor)
    } else {
        result
    };
    result as i32
}

/// `index_series` interpolation: linear, accelerating or decelerating.
fn interpolate(start: i32, current: i32, end: i32, amount: i32, mode: i32) -> i32 {
    if end == start {
        return amount;
    }
    let p = f64::from(current - start) / f64::from(end - start);
    let shaped = match mode {
        1 => 2f64.powf(p) - 1.0,
        2 => (p + 1.0).log2(),
        3 => p * p,
        4 => 1.0 - (1.0 - p) * (1.0 - p),
        _ => p,
    };
    (shaped * f64::from(amount)) as i32
}

/// `TIMETABLE2` / `TIMETABLELEN2`: the value at `nowTime + repTime` of a
/// piecewise animation starting at `startNum` at `startTime`. With

pub(crate) fn index_series(machine: &mut Machine, command: &Command, lengths: bool) -> Result<i32> {
    let index = machine.int_param(command, 0)?;
    let offset = machine.int_param(command, 1)?;
    let mut init = machine.int_param(command, 2)?;
    let mut value = init;
    let mut previous_finished = false;
    // TIMETABLELEN: each segment's start and end count from the previous
    // segment's end.
    let mut elapsed = 0;
    for param in command.params.iter().skip(3) {
        let Expr::Special { tag, pieces } = &param.value else {
            continue;
        };
        let values = pieces
            .iter()
            .map(|piece| machine.eval_int(piece))
            .collect::<Result<Vec<_>>>()?;
        match (tag, values.as_slice()) {
            (0, [val, ..]) => {
                if previous_finished {
                    value = *val;
                    init = *val;
                }
            }
            (1 | 2, [start, end, end_value, rest @ ..]) => {
                let mode = rest.first().copied().unwrap_or(0);
                let (start, end) = if lengths {
                    let start = elapsed + start;
                    elapsed = start + end;
                    (start, elapsed)
                } else {
                    (*start, *end)
                };
                let (start, end) = (start + offset, end + offset);
                if index > start && index < end {
                    value += interpolate(start, index, end, end_value - init, mode);
                    previous_finished = false;
                } else if index >= end {
                    value = *end_value;
                    init = *end_value;
                    previous_finished = true;
                }
            }
            _ => {}
        }
    }
    Ok(value)
}

fn now_parts() -> [i32; 8] {
    use chrono::{Datelike, Local, Timelike};
    let now = Local::now();
    [
        now.year(),
        now.month() as i32,
        now.day() as i32,
        now.weekday().num_days_from_sunday() as i32,
        now.hour() as i32,
        now.minute() as i32,
        now.second() as i32,
        (now.nanosecond() / 1_000_000).min(999) as i32,
    ]
}

fn write_targets(
    machine: &mut Machine,
    command: &Command,
    from: usize,
    values: &[i32],
) -> Result<()> {
    for (offset, value) in values.iter().enumerate() {
        if from + offset >= command.params.len() {
            break;
        }
        let target = machine.int_target_param(command, from + offset)?;
        machine.set_target(target, *value)?;
    }
    Ok(())
}

fn window_attr_values(attr: WindowAttr) -> [i32; 5] {
    [attr.r, attr.g, attr.b, attr.alpha, attr.filter]
}

fn set_attr_component(attr: &mut WindowAttr, component: usize, value: i32) {
    match component {
        0 => attr.r = value,
        1 => attr.g = value,
        2 => attr.b = value,
        3 => attr.alpha = value,
        _ => attr.filter = value,
    }
}


/// `ReadSyscom(n)`: the setting associated with a system command.
pub fn read_syscom(machine: &Machine, index: i32) -> i32 {
    let s = &machine.sys.settings;
    match index {
        syscom::MESSAGE_SPEED => s.message_speed,
        syscom::DISPLAY_MODE => s.screen_mode,
        syscom::VOICE_SETTINGS => s.koe_mode,
        syscom::BGM_FADE => i32::from(s.bgm_koe_fade),
        syscom::WINDOW_DECORATION_STYLE => s.waku_all,
        syscom::SHOW_WEATHER => i32::from(s.show_weather),
        syscom::SHOW_OBJECT_1 => i32::from(s.show_object[0]),
        syscom::SHOW_OBJECT_2 => i32::from(s.show_object[1]),
        syscom::CLASSIFY_TEXT => i32::from(s.classify_text),
        syscom::GENERIC_1 => s.generic[0],
        syscom::GENERIC_2 => s.generic[1],
        syscom::SET_SKIP_MODE => i32::from(machine.sys.syscom.skip_mode),
        syscom::AUTO_MODE => i32::from(s.auto_mode),
        _ => 0,
    }
}

/// Applies a value to the setting behind a system command.
pub fn write_syscom(machine: &mut Machine, index: i32, value: i32) {
    let s = &mut machine.sys.settings;
    match index {
        syscom::MESSAGE_SPEED => s.message_speed = value,
        syscom::DISPLAY_MODE => s.screen_mode = value,
        syscom::VOICE_SETTINGS => s.koe_mode = value,
        syscom::BGM_FADE => s.bgm_koe_fade = value != 0,
        syscom::WINDOW_DECORATION_STYLE => s.waku_all = value,
        syscom::SHOW_WEATHER => s.show_weather = value != 0,
        syscom::SHOW_OBJECT_1 => s.show_object[0] = value != 0,
        syscom::SHOW_OBJECT_2 => s.show_object[1] = value != 0,
        syscom::CLASSIFY_TEXT => s.classify_text = value != 0,
        syscom::GENERIC_1 => s.generic[0] = value,
        syscom::GENERIC_2 => s.generic[1] = value,
        syscom::SET_SKIP_MODE => machine.sys.syscom.skip_mode = value != 0,
        syscom::AUTO_MODE => s.auto_mode = value != 0,
        _ => {}
    }
}

/// The 22xx/23xx/26xx setting families: `Set*`, getter, `Def*`.
fn settings_op(machine: &mut Machine, command: &Command) -> Result<Next> {
    let opcode = command.op.opcode;
    let (kind, which) = match opcode {
        2221..=2299 => (0, opcode - 2200),
        2321..=2399 => (1, opcode - 2300),
        _ => (2, opcode),
    };
    // Defaults use their own numbering.
    let which = if kind == 2 {
        match opcode {
            2600 => 23,
            2601 => 24,
            2602 => 26,
            2603 => 27,
            2604 => 50,
            2605 => 51,
            2606 => 52,
            2610..=2614 => 60 + (opcode - 2610),
            2615..=2617 => 65 + (opcode - 2615),
            2620 => 21,
            2621 => 22,
            _ => return machine.unimplemented(command),
        }
    } else {
        which
    };
    let value = if kind == 0 {
        match which {
            // Window attribute tuples take several values.
            65..=67 => 0,
            74 => machine.int_param(command, 1)?,
            _ => machine.int_param(command, 0)?,
        }
    } else {
        0
    };
    let defaults = machine.sys.defaults.clone();
    let source = if kind == 2 {
        &defaults
    } else {
        &machine.sys.settings
    };
    let current: Option<i32> = match which {
        21 => Some(source.generic[0]),
        22 => Some(source.generic[1]),
        23 => Some(source.message_speed),
        24 => Some(i32::from(source.message_no_wait)),
        25 => Some(source.koe_mode),
        26 => Some(source.bgm_koe_fade_vol),
        27 => Some(i32::from(source.bgm_koe_fade)),
        30..=33 => Some(source.volume[usize::from(which - 30)]),
        40..=43 => Some(i32::from(source.enabled[usize::from(which - 40)])),
        50 => Some(i32::from(source.auto_mode)),
        51 => Some(source.auto_char_time),
        52 => Some(source.auto_base_time),
        55 => Some(source.font_quality),
        56 => Some(source.font_weight),
        57 => Some(source.font_shadow),
        60..=64 => Some(window_attr_values(source.window_attr)[usize::from(which - 60)]),
        70 => Some(i32::from(source.show_object[0])),
        71 => Some(i32::from(source.show_object[1])),
        72 => Some(i32::from(source.show_weather)),
        73 => Some(i32::from(source.classify_text)),
        75 => Some(source.screen_mode),
        _ => None,
    };
    match kind {
        // Getters and defaults.
        1 | 2 => match which {
            65..=67 => {
                let values = window_attr_values(source.window_attr);
                let count = [3, 4, 5][usize::from(which - 65)];
                write_targets(machine, command, 0, &values[..count])?;
            }
            74 => {
                let character = machine.int_param(command, 0)?;
                machine.store = i32::from(
                    *machine
                        .sys
                        .settings
                        .use_koe
                        .get(&character)
                        .unwrap_or(&true),
                );
            }
            _ => match current {
                Some(value) => machine.store = value,
                None => return machine.unimplemented(command),
            },
        },
        // Setters.
        _ => {
            let s = &mut machine.sys.settings;
            match which {
                21 => s.generic[0] = value,
                22 => s.generic[1] = value,
                23 => s.message_speed = value,
                24 => s.message_no_wait = value != 0,
                25 => s.koe_mode = value,
                26 => s.bgm_koe_fade_vol = value,
                27 => s.bgm_koe_fade = value != 0,
                30..=33 => s.volume[usize::from(which - 30)] = value.clamp(0, 255),
                40..=43 => s.enabled[usize::from(which - 40)] = value != 0,
                50 => s.auto_mode = value != 0,
                51 => s.auto_char_time = value,
                52 => s.auto_base_time = value,
                55 => s.font_quality = value,
                56 => s.font_weight = value,
                57 => s.font_shadow = value,
                60..=64 => set_attr_component(&mut s.window_attr, usize::from(which - 60), value),
                65..=67 => {
                    let count = [3, 4, 5][usize::from(which - 65)];
                    let values = machine.int_params_from(command, 0)?;
                    let attr = &mut machine.sys.settings.window_attr;
                    for (component, value) in values.iter().take(count).enumerate() {
                        set_attr_component(attr, component, *value);
                    }
                }
                70 => s.show_object[0] = value != 0,
                71 => s.show_object[1] = value != 0,
                72 => s.show_weather = value != 0,
                73 => s.classify_text = value != 0,
                74 => {
                    let character = machine.int_param(command, 0)?;
                    machine.sys.settings.use_koe.insert(character, value != 0);
                }
                75 => s.screen_mode = value,
                _ => return machine.unimplemented(command),
            }
            if (30..=33).contains(&which) || (40..=43).contains(&which) {
                machine.sys.volumes_changed = true;
            }
        }
    }
    Ok(Next::Advance)
}

fn write_consecutive(machine: &mut Machine, target: IntTarget, values: &[i32]) -> Result<()> {
    match target {
        IntTarget::Mem(reference) => {
            for (offset, &value) in values.iter().enumerate() {
                let next = crate::vm::memory::IntRef {
                    index: reference.index + offset as i32,
                    ..reference
                };
                machine.set_target(IntTarget::Mem(next), value)?;
            }
        }
        IntTarget::Store => {
            if let Some(&value) = values.first() {
                machine.store = value;
            }
        }
    }
    Ok(())
}


/// `TIMETABLE2` / `TIMETABLELEN2`: the value at `nowTime + repTime` of a
/// piecewise animation starting at `startNum` at `startTime`. With
/// `lengths`, the segment times are durations rather than end times.
fn timetable2(machine: &mut Machine, command: &Command, lengths: bool) -> Result<i32> {
    let now = machine.int_param(command, 0)? + machine.int_param(command, 1)?;
    let mut start = machine.int_param(command, 2)?;
    let mut value = machine.int_param(command, 3)?;
    let mut segments = Vec::new();
    let mut total = start;
    for param in command.params.iter().skip(4) {
        let Expr::Special { tag, pieces } = &param.value else {
            continue;
        };
        let mut v = pieces
            .iter()
            .map(|piece| machine.eval_int(piece))
            .collect::<Result<Vec<_>>>()?;
        let kind = *tag & 0xffff;
        // _SET has no time; the others start with one.
        if lengths && kind != 49 && !v.is_empty() {
            total += v[0];
            v[0] = total;
        }
        segments.push((kind, *tag >> 16, v));
    }
    if start > now {
        return Ok(value);
    }
    let arg = |v: &[i32], i: usize| v.get(i).copied().unwrap_or(0);
    for (kind, overload, v) in segments {
        let (end, target) = (arg(&v, 0), arg(&v, 1));
        let inside = now > start && now <= end;
        match kind {
            // _MOVE(endTime, endNum[, mod])
            48 => {
                let mode = if overload == 1 { arg(&v, 2) } else { 0 };
                if inside {
                    return Ok(value + interpolate(start, now, end, target - value, mode));
                }
                value = target;
                start = end;
            }
            // _SET(endNum)
            49 => value = arg(&v, 0),
            // _WAIT(endTime)
            50 => {
                if inside {
                    return Ok(value);
                }
                start = end;
            }
            // _TURN / _TURNUP / _LOOP / _JUMP(endTime, endNum, count)
            51..=54 => {
                let count = arg(&v, 2).max(1);
                if inside {
                    let cycle = ((end - start) / count).max(1);
                    let p = f64::from((now - start) % cycle) / f64::from(cycle);
                    let shape = match kind {
                        // there and back
                        51 => 1.0 - (p * 2.0 - 1.0).abs(),
                        // up, then straight back
                        52 | 53 => p,
                        // a parabola, as rlvm does
                        _ => 1.0 - (p * 2.0 - 1.0).powi(2),
                    };
                    return Ok(value + (f64::from(target - value) * shape) as i32);
                }
                if kind != 51 {
                    value = target;
                }
                start = end;
            }
            // _WAITSET(endTime, endNum)
            55 => {
                if inside {
                    return Ok(value);
                }
                value = target;
                start = end;
            }
            _ => {}
        }
    }
    Ok(value)
}

