// SPDX-License-Identifier: MPL-2.0
//! `pygame_sdl2` surface used by Ren'Py and by game code: event types, key
//! and modifier constants (SDL2 values, as pygame_sdl2 exposes them), the
//! `Event` class, key/mouse state queries and the conversion of CrySirror
//! `InputFrame`s into pygame events.
use super::py::{self, *};
use crysirror_core::service::{InputFrame, buttons};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::cell::RefCell;

pub const NOEVENT: i64 = 0;
pub const QUIT: i64 = 0x100;
pub const APP_TERMINATING: i64 = 0x101;
pub const APP_LOWMEMORY: i64 = 0x102;
pub const APP_WILLENTERBACKGROUND: i64 = 0x103;
pub const APP_DIDENTERBACKGROUND: i64 = 0x104;
pub const APP_WILLENTERFOREGROUND: i64 = 0x105;
pub const APP_DIDENTERFOREGROUND: i64 = 0x106;
pub const WINDOWEVENT: i64 = 0x200;
pub const SYSWMEVENT: i64 = 0x201;
pub const KEYDOWN: i64 = 0x300;
pub const KEYUP: i64 = 0x301;
pub const TEXTEDITING: i64 = 0x302;
pub const TEXTINPUT: i64 = 0x303;
pub const KEYMAPCHANGED: i64 = 0x304;
pub const MOUSEMOTION: i64 = 0x400;
pub const MOUSEBUTTONDOWN: i64 = 0x401;
pub const MOUSEBUTTONUP: i64 = 0x402;
pub const MOUSEWHEEL: i64 = 0x403;
pub const JOYAXISMOTION: i64 = 0x600;
pub const JOYBALLMOTION: i64 = 0x601;
pub const JOYHATMOTION: i64 = 0x602;
pub const JOYBUTTONDOWN: i64 = 0x603;
pub const JOYBUTTONUP: i64 = 0x604;
pub const JOYDEVICEADDED: i64 = 0x605;
pub const JOYDEVICEREMOVED: i64 = 0x606;
pub const CONTROLLERAXISMOTION: i64 = 0x650;
pub const CONTROLLERBUTTONDOWN: i64 = 0x651;
pub const CONTROLLERBUTTONUP: i64 = 0x652;
pub const CONTROLLERDEVICEADDED: i64 = 0x653;
pub const CONTROLLERDEVICEREMOVED: i64 = 0x654;
pub const CONTROLLERDEVICEREMAPPED: i64 = 0x655;
pub const FINGERDOWN: i64 = 0x700;
pub const FINGERUP: i64 = 0x701;
pub const FINGERMOTION: i64 = 0x702;
pub const MULTIGESTURE: i64 = 0x802;
pub const CLIPBOARDUPDATE: i64 = 0x900;
pub const DROPFILE: i64 = 0x1000;
pub const AUDIODEVICEADDED: i64 = 0x1100;
pub const RENDER_TARGETS_RESET: i64 = 0x2000;
pub const USEREVENT: i64 = 0x8000;
/// Ren'Py's registered events (`renpy.display.core`).
pub const TIMEEVENT: i64 = USEREVENT + 1;
pub const PERIODIC: i64 = USEREVENT + 2;
pub const REDRAW: i64 = USEREVENT + 3;
pub const EVENTNAME: i64 = USEREVENT + 4;
pub const ACTIVEEVENT: i64 = USEREVENT + 5;
pub const VIDEORESIZE: i64 = USEREVENT + 6;
pub const VIDEOEXPOSE: i64 = USEREVENT + 7;
pub const WINDOWMOVED: i64 = USEREVENT + 8;

/// `pygame.event.get_standard_events()`.
pub fn standard_events() -> Vec<i64> {
    EVENT_NAMES.iter().map(|(_, v)| *v).filter(|v| *v != NOEVENT && *v < USEREVENT).collect()
}

/// `renpy.display.core.enabled_events`.
pub fn enabled_events() -> Vec<i64> {
    vec![
        QUIT, APP_TERMINATING, APP_LOWMEMORY, APP_WILLENTERBACKGROUND, APP_DIDENTERBACKGROUND, APP_WILLENTERFOREGROUND, APP_DIDENTERFOREGROUND, WINDOWEVENT, SYSWMEVENT, KEYDOWN, KEYUP, TEXTEDITING, TEXTINPUT, KEYMAPCHANGED, MOUSEMOTION, MOUSEBUTTONDOWN, MOUSEBUTTONUP, MOUSEWHEEL, JOYAXISMOTION, JOYHATMOTION, JOYBALLMOTION, JOYBUTTONDOWN, JOYBUTTONUP, JOYDEVICEADDED, JOYDEVICEREMOVED, CONTROLLERAXISMOTION, CONTROLLERBUTTONDOWN, CONTROLLERBUTTONUP, CONTROLLERDEVICEADDED, CONTROLLERDEVICEREMOVED, RENDER_TARGETS_RESET, TIMEEVENT, PERIODIC, REDRAW, EVENTNAME,
    ]
}

/// `renpy.display.core.input_events`.
pub fn input_events() -> Vec<i64> {
    vec![KEYDOWN, KEYUP, TEXTEDITING, TEXTINPUT, MOUSEMOTION, MOUSEBUTTONDOWN, MOUSEBUTTONUP, MOUSEWHEEL, JOYAXISMOTION, JOYHATMOTION, JOYBALLMOTION, JOYBUTTONDOWN, JOYBUTTONUP, CONTROLLERAXISMOTION, CONTROLLERBUTTONDOWN, CONTROLLERBUTTONUP]
}

const SC: i64 = 1 << 30;

/// (pygame name, SDL keycode) for keys Ren'Py's keymap and games use.
pub const KEYS: &[(&str, i64)] = &[
    ("K_UNKNOWN", 0), ("K_BACKSPACE", 8), ("K_TAB", 9), ("K_RETURN", 13), ("K_ESCAPE", 27), ("K_SPACE", 32),
    ("K_EXCLAIM", 33), ("K_QUOTEDBL", 34), ("K_HASH", 35), ("K_DOLLAR", 36), ("K_PERCENT", 37), ("K_AMPERSAND", 38),
    ("K_QUOTE", 39), ("K_LEFTPAREN", 40), ("K_RIGHTPAREN", 41), ("K_ASTERISK", 42), ("K_PLUS", 43), ("K_COMMA", 44),
    ("K_MINUS", 45), ("K_PERIOD", 46), ("K_SLASH", 47), ("K_0", 48), ("K_1", 49), ("K_2", 50), ("K_3", 51), ("K_4", 52),
    ("K_5", 53), ("K_6", 54), ("K_7", 55), ("K_8", 56), ("K_9", 57), ("K_COLON", 58), ("K_SEMICOLON", 59), ("K_LESS", 60),
    ("K_EQUALS", 61), ("K_GREATER", 62), ("K_QUESTION", 63), ("K_AT", 64), ("K_LEFTBRACKET", 91), ("K_BACKSLASH", 92),
    ("K_RIGHTBRACKET", 93), ("K_CARET", 94), ("K_UNDERSCORE", 95), ("K_BACKQUOTE", 96),
    ("K_a", 97), ("K_b", 98), ("K_c", 99), ("K_d", 100), ("K_e", 101), ("K_f", 102), ("K_g", 103), ("K_h", 104), ("K_i", 105),
    ("K_j", 106), ("K_k", 107), ("K_l", 108), ("K_m", 109), ("K_n", 110), ("K_o", 111), ("K_p", 112), ("K_q", 113), ("K_r", 114),
    ("K_s", 115), ("K_t", 116), ("K_u", 117), ("K_v", 118), ("K_w", 119), ("K_x", 120), ("K_y", 121), ("K_z", 122),
    ("K_DELETE", 127), ("K_CAPSLOCK", SC | 57),
    ("K_F1", SC | 58), ("K_F2", SC | 59), ("K_F3", SC | 60), ("K_F4", SC | 61), ("K_F5", SC | 62), ("K_F6", SC | 63),
    ("K_F7", SC | 64), ("K_F8", SC | 65), ("K_F9", SC | 66), ("K_F10", SC | 67), ("K_F11", SC | 68), ("K_F12", SC | 69),
    ("K_PRINTSCREEN", SC | 70), ("K_PRINT", SC | 70), ("K_SCROLLLOCK", SC | 71), ("K_PAUSE", SC | 72), ("K_INSERT", SC | 73),
    ("K_HOME", SC | 74), ("K_PAGEUP", SC | 75), ("K_END", SC | 77), ("K_PAGEDOWN", SC | 78), ("K_RIGHT", SC | 79),
    ("K_LEFT", SC | 80), ("K_DOWN", SC | 81), ("K_UP", SC | 82), ("K_NUMLOCKCLEAR", SC | 83), ("K_NUMLOCK", SC | 83),
    ("K_KP_DIVIDE", SC | 84), ("K_KP_MULTIPLY", SC | 85), ("K_KP_MINUS", SC | 86), ("K_KP_PLUS", SC | 87),
    ("K_KP_ENTER", SC | 88), ("K_KP1", SC | 89), ("K_KP2", SC | 90), ("K_KP3", SC | 91), ("K_KP4", SC | 92),
    ("K_KP5", SC | 93), ("K_KP6", SC | 94), ("K_KP7", SC | 95), ("K_KP8", SC | 96), ("K_KP9", SC | 97), ("K_KP0", SC | 98),
    ("K_KP_1", SC | 89), ("K_KP_2", SC | 90), ("K_KP_3", SC | 91), ("K_KP_4", SC | 92), ("K_KP_5", SC | 93),
    ("K_KP_6", SC | 94), ("K_KP_7", SC | 95), ("K_KP_8", SC | 96), ("K_KP_9", SC | 97), ("K_KP_0", SC | 98),
    ("K_KP_PERIOD", SC | 99), ("K_APPLICATION", SC | 101), ("K_POWER", SC | 102), ("K_KP_EQUALS", SC | 103),
    ("K_F13", SC | 104), ("K_F14", SC | 105), ("K_F15", SC | 106), ("K_HELP", SC | 117), ("K_MENU", SC | 118),
    ("K_SELECT", SC | 119), ("K_STOP", SC | 120), ("K_AGAIN", SC | 121), ("K_UNDO", SC | 122), ("K_MUTE", SC | 127),
    ("K_VOLUMEUP", SC | 128), ("K_VOLUMEDOWN", SC | 129), ("K_SYSREQ", SC | 154), ("K_CLEAR", SC | 156),
    ("K_LCTRL", SC | 224), ("K_LSHIFT", SC | 225), ("K_LALT", SC | 226), ("K_LGUI", SC | 227), ("K_LMETA", SC | 227),
    ("K_LSUPER", SC | 227), ("K_RCTRL", SC | 228), ("K_RSHIFT", SC | 229), ("K_RALT", SC | 230), ("K_RGUI", SC | 231),
    ("K_RMETA", SC | 231), ("K_RSUPER", SC | 231), ("K_MODE", SC | 257), ("K_AUDIONEXT", SC | 258), ("K_AUDIOPREV", SC | 259),
    ("K_AUDIOSTOP", SC | 260), ("K_AUDIOPLAY", SC | 261), ("K_AUDIOMUTE", SC | 262), ("K_AC_SEARCH", SC | 268),
    ("K_AC_HOME", SC | 269), ("K_AC_BACK", SC | 270), ("K_AC_FORWARD", SC | 271), ("K_AC_STOP", SC | 272),
    ("K_AC_REFRESH", SC | 273), ("K_AC_BOOKMARKS", SC | 274),
];

pub const MODS: &[(&str, i64)] = &[
    ("KMOD_NONE", 0), ("KMOD_LSHIFT", 1), ("KMOD_RSHIFT", 2), ("KMOD_LCTRL", 0x40), ("KMOD_RCTRL", 0x80),
    ("KMOD_LALT", 0x100), ("KMOD_RALT", 0x200), ("KMOD_LGUI", 0x400), ("KMOD_RGUI", 0x800), ("KMOD_LMETA", 0x400),
    ("KMOD_RMETA", 0x800), ("KMOD_NUM", 0x1000), ("KMOD_CAPS", 0x2000), ("KMOD_MODE", 0x4000), ("KMOD_SHIFT", 3),
    ("KMOD_CTRL", 0xC0), ("KMOD_ALT", 0x300), ("KMOD_GUI", 0xC00), ("KMOD_META", 0xC00),
];

const EVENT_NAMES: &[(&str, i64)] = &[
    ("NOEVENT", 0), ("QUIT", QUIT), ("APP_TERMINATING", APP_TERMINATING), ("APP_LOWMEMORY", APP_LOWMEMORY),
    ("APP_WILLENTERBACKGROUND", APP_WILLENTERBACKGROUND), ("APP_DIDENTERBACKGROUND", APP_DIDENTERBACKGROUND),
    ("APP_WILLENTERFOREGROUND", APP_WILLENTERFOREGROUND), ("APP_DIDENTERFOREGROUND", APP_DIDENTERFOREGROUND),
    ("WINDOWEVENT", WINDOWEVENT), ("SYSWMEVENT", SYSWMEVENT), ("KEYDOWN", KEYDOWN), ("KEYUP", KEYUP),
    ("TEXTEDITING", TEXTEDITING), ("TEXTINPUT", TEXTINPUT), ("KEYMAPCHANGED", KEYMAPCHANGED),
    ("MOUSEMOTION", MOUSEMOTION), ("MOUSEBUTTONDOWN", MOUSEBUTTONDOWN), ("MOUSEBUTTONUP", MOUSEBUTTONUP),
    ("MOUSEWHEEL", MOUSEWHEEL), ("JOYAXISMOTION", JOYAXISMOTION), ("JOYBALLMOTION", JOYBALLMOTION),
    ("JOYHATMOTION", JOYHATMOTION), ("JOYBUTTONDOWN", JOYBUTTONDOWN), ("JOYBUTTONUP", JOYBUTTONUP),
    ("JOYDEVICEADDED", JOYDEVICEADDED), ("JOYDEVICEREMOVED", JOYDEVICEREMOVED),
    ("CONTROLLERAXISMOTION", CONTROLLERAXISMOTION), ("CONTROLLERBUTTONDOWN", CONTROLLERBUTTONDOWN),
    ("CONTROLLERBUTTONUP", CONTROLLERBUTTONUP), ("CONTROLLERDEVICEADDED", CONTROLLERDEVICEADDED),
    ("CONTROLLERDEVICEREMOVED", CONTROLLERDEVICEREMOVED), ("CONTROLLERDEVICEREMAPPED", CONTROLLERDEVICEREMAPPED),
    ("FINGERDOWN", FINGERDOWN), ("FINGERUP", FINGERUP), ("FINGERMOTION", FINGERMOTION), ("MULTIGESTURE", MULTIGESTURE),
    ("CLIPBOARDUPDATE", CLIPBOARDUPDATE), ("DROPFILE", DROPFILE), ("AUDIODEVICEADDED", AUDIODEVICEADDED),
    ("RENDER_TARGETS_RESET", RENDER_TARGETS_RESET), ("USEREVENT", USEREVENT), ("ACTIVEEVENT", ACTIVEEVENT),
    ("VIDEORESIZE", VIDEORESIZE), ("VIDEOEXPOSE", VIDEOEXPOSE), ("WINDOWMOVED", WINDOWMOVED),
];

/// (InputFrame bit, SDL keycode) of keyboard buttons.
const BUTTON_KEYS: &[(u64, i64)] = &[
    (buttons::ENTER, 13), (buttons::SPACE, 32), (buttons::LEFT, SC | 80), (buttons::RIGHT, SC | 79),
    (buttons::UP, SC | 82), (buttons::DOWN, SC | 81), (buttons::TAB, 9), (buttons::KEY_W, 119), (buttons::KEY_A, 97),
    (buttons::KEY_S, 115), (buttons::KEY_D, 100), (buttons::SHIFT, SC | 225), (buttons::CTRL, SC | 224),
    (buttons::KEY_E, 101), (buttons::KEY_Q, 113), (buttons::KEY_F, 102), (buttons::KEY_R, 114), (buttons::ESCAPE, 27),
    (buttons::BACKSPACE, 8), (buttons::HOME, SC | 74), (buttons::END, SC | 77), (buttons::PAGE_UP, SC | 75),
    (buttons::PAGE_DOWN, SC | 78), (buttons::KEY_0, 48), (buttons::KEY_1, 49), (buttons::KEY_2, 50), (buttons::KEY_3, 51),
    (buttons::KEY_4, 52), (buttons::KEY_5, 53), (buttons::KEY_6, 54), (buttons::KEY_7, 55), (buttons::KEY_8, 56),
    (buttons::KEY_9, 57), (buttons::F1, SC | 58), (buttons::F2, SC | 59), (buttons::F3, SC | 60), (buttons::F4, SC | 61),
    (buttons::F5, SC | 62), (buttons::F6, SC | 63), (buttons::F7, SC | 64), (buttons::F8, SC | 65), (buttons::F9, SC | 66),
    (buttons::F10, SC | 67), (buttons::F11, SC | 68), (buttons::F12, SC | 69), (buttons::KEY_C, 99), (buttons::KEY_L, 108),
    (buttons::KEY_Z, 122), (buttons::INSERT, SC | 73), (buttons::ALT, SC | 226), (buttons::DELETE, 127),
    (buttons::KEY_H, 104), (buttons::KEY_I, 105), (buttons::KEY_J, 106), (buttons::KEY_M, 109), (buttons::KEY_V, 118),
    (buttons::KEY_T, 116), (buttons::KEY_B, 98), (buttons::KEY_N, 110),
];

const MOUSE_BUTTONS: &[(u64, i64)] = &[(buttons::MOUSE_LEFT, 1), (buttons::MOUSE_MIDDLE, 2), (buttons::MOUSE_RIGHT, 3)];

#[derive(Default)]
pub struct InputState {
    pub down: u64,
    pub pos: (i32, i32),
    pub mods: i64,
    pub pressed_keys: std::collections::BTreeSet<i64>,
    pub mouse_buttons: [bool; 6],
    pub queue: Vec<Value>,
    pub mouse_focused: bool,
    pub key_repeat: (i64, i64),
    pub text_input: bool,
    /// `pygame.time.set_timer` timers: (event type, interval ms, next due ms).
    pub timers: Vec<(i64, u64, u64)>,
    pub next_user_event: i64,
}

pub fn input(vm: &Vm) -> std::rc::Rc<RefCell<InputState>> {
    vm.ext_get::<std::rc::Rc<RefCell<InputState>>>().expect("input state").clone()
}

fn mods_of(down: u64) -> i64 {
    let mut m = 0;
    if down & buttons::SHIFT != 0 {
        m |= 1;
    }
    if down & buttons::CTRL != 0 {
        m |= 0x40;
    }
    if down & buttons::ALT != 0 {
        m |= 0x100;
    }
    m
}

fn event_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "pygame_sdl2.event", "Event")
}

pub fn new_event(vm: &mut Vm, ty: i64, fields: Vec<(&str, Value)>) -> PyResult<Value> {
    let c = event_class(vm)?;
    let kw = fields.into_iter().map(|(k, v)| (std::rc::Rc::from(k), v)).collect();
    vm.call(&c, vec![Value::Int(ty)], kw)
}

/// Converts one input frame into pygame events (key/mouse transitions and
/// pointer motion), updating the retained pressed state.
pub fn from_input(vm: &mut Vm, frame: &InputFrame, fit: Fit) -> PyResult<Vec<Value>> {
    let (old, old_pos) = {
        let s_rc = input(vm);
    let s = s_rc.borrow_mut();
        (s.down, s.pos)
    };
    let mut out = Vec::new();
    let pos = (frame.pointer_x, frame.pointer_y);
    let lp = fit.to_virtual(pos.0 as f64, pos.1 as f64);
    let mods = mods_of(frame.buttons_down);
    let tuple2 = |vm: &Vm, a: f64, b: f64| vm.types.new_tuple(vec![Value::Int(a as i64), Value::Int(b as i64)]);
    if pos != old_pos {
        let rel = tuple2(vm, (pos.0 - old_pos.0) as f64 / fit.scale, (pos.1 - old_pos.1) as f64 / fit.scale);
        let held = |b: u64| Value::Int((frame.buttons_down & b != 0) as i64);
        let btns = vm.types.new_tuple(vec![held(buttons::MOUSE_LEFT), held(buttons::MOUSE_MIDDLE), held(buttons::MOUSE_RIGHT)]);
        let p = tuple2(vm, lp.0, lp.1);
        out.push(new_event(vm, MOUSEMOTION, vec![("pos", p), ("rel", rel), ("buttons", btns), ("touch", Value::Bool(false))])?);
    }
    let pressed = frame.buttons_pressed | (frame.buttons_down & !old);
    let released = old & !frame.buttons_down & !frame.buttons_pressed;
    for &(bit, key) in BUTTON_KEYS {
        if pressed & bit != 0 {
            let uni = if (32..127).contains(&key) {
                let c = key as u8 as char;
                let c = if mods & 3 != 0 { c.to_ascii_uppercase() } else { c };
                vm.types.new_string(c.to_string())
            } else if key == 13 {
                vm.types.new_str("\r")
            } else if key == 8 {
                vm.types.new_str("\u{8}")
            } else if key == 9 {
                vm.types.new_str("\t")
            } else if key == 27 {
                vm.types.new_str("\u{1b}")
            } else {
                vm.types.new_str("")
            };
            out.push(new_event(vm, KEYDOWN, vec![("key", Value::Int(key)), ("mod", Value::Int(mods)), ("unicode", uni), ("scancode", Value::Int(key & !SC)), ("repeat", Value::Bool(false))])?);
            input(vm).borrow_mut().pressed_keys.insert(key);
            if frame.buttons_pressed & bit != 0 && frame.buttons_down & bit == 0 {
                out.push(new_event(vm, KEYUP, vec![("key", Value::Int(key)), ("mod", Value::Int(mods)), ("scancode", Value::Int(key & !SC)), ("repeat", Value::Bool(false))])?);
                input(vm).borrow_mut().pressed_keys.remove(&key);
            }
        } else if released & bit != 0 {
            out.push(new_event(vm, KEYUP, vec![("key", Value::Int(key)), ("mod", Value::Int(mods)), ("scancode", Value::Int(key & !SC)), ("repeat", Value::Bool(false))])?);
            input(vm).borrow_mut().pressed_keys.remove(&key);
        }
    }
    for &(bit, button) in MOUSE_BUTTONS {
        let p = tuple2(vm, lp.0, lp.1);
        if pressed & bit != 0 {
            out.push(new_event(vm, MOUSEBUTTONDOWN, vec![("pos", p.clone()), ("button", Value::Int(button)), ("touch", Value::Bool(false))])?);
            input(vm).borrow_mut().mouse_buttons[button as usize] = true;
            if frame.buttons_pressed & bit != 0 && frame.buttons_down & bit == 0 {
                out.push(new_event(vm, MOUSEBUTTONUP, vec![("pos", p), ("button", Value::Int(button)), ("touch", Value::Bool(false))])?);
                input(vm).borrow_mut().mouse_buttons[button as usize] = false;
            }
        } else if released & bit != 0 {
            out.push(new_event(vm, MOUSEBUTTONUP, vec![("pos", p), ("button", Value::Int(button)), ("touch", Value::Bool(false))])?);
            input(vm).borrow_mut().mouse_buttons[button as usize] = false;
        }
    }
    for (bit, button, dy) in [(buttons::WHEEL_UP, 4, 1), (buttons::WHEEL_DOWN, 5, -1)] {
        if frame.buttons_pressed & bit != 0 || (frame.buttons_down & bit != 0 && old & bit == 0) {
            out.push(new_event(vm, MOUSEWHEEL, vec![("x", Value::Int(0)), ("y", Value::Int(dy)), ("flipped", Value::Bool(false)), ("touch", Value::Bool(false))])?);
            let p = tuple2(vm, lp.0, lp.1);
            out.push(new_event(vm, MOUSEBUTTONDOWN, vec![("pos", p.clone()), ("button", Value::Int(button)), ("touch", Value::Bool(false))])?);
            out.push(new_event(vm, MOUSEBUTTONUP, vec![("pos", p), ("button", Value::Int(button)), ("touch", Value::Bool(false))])?);
        }
    }
    let queued = std::mem::take(&mut input(vm).borrow_mut().queue);
    out.extend(queued);
    let s_rc = input(vm);
    let mut s = s_rc.borrow_mut();
    s.down = frame.buttons_down;
    s.pos = pos;
    s.mods = mods;
    s.mouse_focused = true;
    Ok(out)
}

pub fn is_type(vm: &mut Vm, ev: &Value, ty: i64) -> PyResult<bool> {
    let t = py::get_or_none(vm, ev, "type")?;
    Ok(matches!(t, Value::Int(v) if v == ty))
}

pub fn is_mouse(vm: &mut Vm, ev: &Value) -> PyResult<bool> {
    let t = py::get_or_none(vm, ev, "type")?;
    Ok(matches!(t, Value::Int(v) if v == MOUSEMOTION || v == MOUSEBUTTONDOWN || v == MOUSEBUTTONUP))
}

// Event class ------------------------------------------------------------------

fn ev_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ty = args.get(1).cloned().unwrap_or(Value::Int(0));
    py::set(vm, this, "type", ty)?;
    if let Some(d) = args.get(2) {
        if !py::is_none(d) {
            for (k, v) in py::str_items(vm, d)? {
                py::set(vm, this, &k, v)?;
            }
        }
    }
    for (k, v) in kw {
        py::set(vm, this, k, v.clone())?;
    }
    Ok(Value::None)
}

fn ev_dict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "__dict__")?;
    let copy = vm.call_method(&d, "copy", Vec::new())?;
    vm.call_method(&copy, "pop", vec![vm.types.new_str("type"), Value::None])?;
    Ok(copy)
}

fn ev_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::get_or_none(vm, &args[0], "type")?;
    let t = py::int(vm, &t).unwrap_or(0);
    let name = EVENT_NAMES.iter().find(|(_, v)| *v == t).map(|(n, _)| *n).unwrap_or("UserEvent");
    let d = ev_dict(vm, args, &[])?;
    let d = ops::repr(vm, &d)?;
    Ok(vm.types.new_string(format!("<Event({t}-{name} {d})>")))
}

fn ev_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "__dict__")?;
    let b = match args.get(1) { Some(o) => py::get_opt(vm, o, "__dict__")?.unwrap_or(Value::None), None => Value::None };
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn f_event_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::int(vm, &args.first().cloned().unwrap_or(Value::Int(0)))?;
    let name = EVENT_NAMES.iter().find(|(_, v)| *v == t).map(|(n, _)| (*n).to_owned()).unwrap_or_else(|| "UserEvent".into());
    Ok(vm.types.new_string(name))
}

fn f_post(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if let Some(e) = args.first() {
        input(vm).borrow_mut().queue.push(e.clone());
    }
    Ok(Value::Bool(true))
}

/// The event types named by an optional `types` argument.
fn type_filter(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Option<Vec<i64>>> {
    let v = match py::arg(args, kw, 0, "eventtype").or_else(|| py::kw(kw, "type")) {
        None | Some(Value::None) => return Ok(None),
        Some(v) => v.clone(),
    };
    Ok(Some(match v {
        Value::Int(i) => vec![i],
        v => py::items(vm, &v)?.iter().filter_map(|x| if let Value::Int(i) = x { Some(*i) } else { None }).collect(),
    }))
}

fn event_type(vm: &mut Vm, e: &Value) -> PyResult<i64> {
    match py::get_or_none(vm, e, "type")? {
        Value::Int(t) => Ok(t),
        _ => Ok(-1),
    }
}

/// Removes and returns the queued events matching `types` (all when None).
fn take_events(vm: &mut Vm, types: &Option<Vec<i64>>) -> PyResult<Vec<Value>> {
    due_timers(vm);
    let q = std::mem::take(&mut input(vm).borrow_mut().queue);
    let mut taken = Vec::new();
    let mut kept = Vec::new();
    for e in q {
        let t = event_type(vm, &e)?;
        if types.as_ref().is_none_or(|ts| ts.contains(&t)) {
            taken.push(e);
        } else {
            kept.push(e);
        }
    }
    let s = input(vm);
    let mut s = s.borrow_mut();
    kept.append(&mut s.queue);
    s.queue = kept;
    Ok(taken)
}

fn f_event_get(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let types = type_filter(vm, args, kw)?;
    let q = take_events(vm, &types)?;
    Ok(vm.types.new_list(q))
}

fn f_event_peek(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    due_timers(vm);
    let q = input(vm).borrow_mut().queue.clone();
    let Some(types) = type_filter(vm, args, kw)? else {
        return Ok(Value::Bool(!q.is_empty()));
    };
    for e in &q {
        if types.contains(&event_type(vm, e)?) {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn f_event_clear(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let types = type_filter(vm, args, kw)?;
    take_events(vm, &types)?;
    Ok(Value::None)
}

/// Queues the events of timers that are due at the current host time.
pub fn due_timers(vm: &mut Vm) {
    let now = super::link::now_ms(vm);
    let s = input(vm);
    let mut s = s.borrow_mut();
    let mut fired = Vec::new();
    for t in s.timers.iter_mut() {
        if t.2 <= now {
            fired.push(t.0);
            t.2 = now + t.1.max(1);
        }
    }
    drop(s);
    for ty in fired {
        if let Ok(e) = new_event(vm, ty, Vec::new()) {
            input(vm).borrow_mut().queue.push(e);
        }
    }
}

fn next_timer_due(vm: &Vm) -> Option<u64> {
    input(vm).borrow().timers.iter().map(|t| t.2).min()
}

fn pop_event(vm: &mut Vm) -> Option<Value> {
    let s = input(vm);
    let mut s = s.borrow_mut();
    if s.queue.is_empty() { None } else { Some(s.queue.remove(0)) }
}

/// `pygame.event.poll()`: a queued event or NOEVENT. Once this tick has
/// drawn a frame and nothing is queued, the tick ends and the next tick's
/// events are returned.
fn f_event_poll(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    due_timers(vm);
    if let Some(e) = pop_event(vm) {
        return Ok(e);
    }
    if super::link::link(vm).is_some() && super::link::drew_this_tick(vm) {
        super::link::end_tick(vm, super::link::Wait::NextFrame)?;
        due_timers(vm);
        if let Some(e) = pop_event(vm) {
            return Ok(e);
        }
    }
    new_event(vm, NOEVENT, Vec::new())
}

/// `pygame.event.wait()`: blocks the script thread until an event exists.
fn f_event_wait(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    loop {
        due_timers(vm);
        if let Some(e) = pop_event(vm) {
            return Ok(e);
        }
        if super::link::link(vm).is_none() {
            return new_event(vm, NOEVENT, Vec::new());
        }
        let wait = match next_timer_due(vm) {
            Some(t) => super::link::Wait::Until(t),
            None => super::link::Wait::Input,
        };
        super::link::end_tick(vm, wait)?;
    }
}

fn f_set_timer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ty = py::arg(args, kw, 0, "eventid").cloned().unwrap_or(Value::Int(0));
    let ty = match ty {
        Value::Int(i) => i,
        e => event_type(vm, &e)?,
    };
    let ms = py::arg(args, kw, 1, "millis").cloned().unwrap_or(Value::Int(0));
    let ms = py::int(vm, &ms)?;
    let now = super::link::now_ms(vm);
    let s = input(vm);
    let mut s = s.borrow_mut();
    s.timers.retain(|t| t.0 != ty);
    if ms > 0 {
        s.timers.push((ty, ms as u64, now + ms as u64));
    }
    Ok(Value::None)
}

fn f_event_register(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = args.first().and_then(|n| n.as_str()).unwrap_or("").to_owned();
    Ok(Value::Int(match name.as_str() {
        "TIMEEVENT" => TIMEEVENT,
        "PERIODIC" => PERIODIC,
        "REDRAW" => REDRAW,
        "EVENTNAME" => EVENTNAME,
        _ => {
            let s_rc = input(vm);
            let mut s = s_rc.borrow_mut();
            s.next_user_event += 1;
            USEREVENT + 64 + s.next_user_event
        }
    }))
}

fn f_true(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn f_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn f_get_mods(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(input(vm).borrow_mut().mods))
}

fn f_get_pressed(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    // ScancodeWrapper: indexable by keycode.
    let keys: Vec<i64> = input(vm).borrow_mut().pressed_keys.iter().copied().collect();
    let d = vm.types.new_dict();
    for k in keys {
        ops::setitem(vm, &d, Value::Int(k), Value::Bool(true))?;
        ops::setitem(vm, &d, Value::Int(k & !SC), Value::Bool(true))?;
    }
    let cls = py::import_attr(vm, "pygame_sdl2.key", "_Pressed")?;
    vm.call(&cls, vec![d], Vec::new())
}

fn f_key_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let k = py::int(vm, &args.first().cloned().unwrap_or(Value::Int(0)))?;
    let name = match k {
        8 => "backspace".to_owned(),
        9 => "tab".into(),
        13 => "return".into(),
        27 => "escape".into(),
        32 => "space".into(),
        127 => "delete".into(),
        k if (33..127).contains(&k) => (k as u8 as char).to_string(),
        k => KEYS
            .iter()
            .find(|(_, v)| *v == k)
            .map(|(n, _)| n.trim_start_matches("K_").to_ascii_lowercase().replace('_', " "))
            .unwrap_or_else(|| "unknown key".into()),
    };
    Ok(vm.types.new_string(name))
}

fn f_mouse_get_pos(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let p = input(vm).borrow_mut().pos;
    Ok(vm.types.new_tuple(vec![Value::Int(p.0 as i64), Value::Int(p.1 as i64)]))
}

fn f_mouse_get_pressed(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let b = input(vm).borrow_mut().mouse_buttons;
    Ok(vm.types.new_tuple(vec![Value::Bool(b[1]), Value::Bool(b[2]), Value::Bool(b[3])]))
}

fn f_mouse_get_focused(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(input(vm).borrow_mut().mouse_focused))
}

fn f_get_ticks(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(super::link::now_ms(vm) as i64))
}

fn f_text_input(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let _ = args;
    input(vm).borrow_mut().text_input = true;
    Ok(Value::None)
}

fn f_stop_text_input(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    input(vm).borrow_mut().text_input = false;
    Ok(Value::None)
}

fn pressed_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "_keys")?;
    let k = args.get(1).cloned().unwrap_or(Value::Int(0));
    Ok(Value::Bool(ops::contains(vm, &d, &k)?))
}

fn pressed_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "_keys", args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

fn init_root(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "pygame_sdl2", &[
        ("init", f_true),
        ("quit", f_none),
        ("get_sdl_version", |vm, _, _| Ok(vm.types.new_tuple(vec![Value::Int(2), Value::Int(30), Value::Int(0)]))),
        ("get_platform", |vm, _, _| Ok(vm.types.new_str("Linux"))),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for (n, v) in EVENT_NAMES.iter().chain(KEYS).chain(MODS) {
        set(vm, n, Value::Int(*v));
    }
    for (n, v) in [("BUTTON_LEFT", 1), ("BUTTON_MIDDLE", 2), ("BUTTON_RIGHT", 3), ("BUTTON_X1", 4), ("BUTTON_X2", 5), ("SRCALPHA", 0x10000), ("RLEACCEL", 0x4000), ("BLEND_RGBA_MULT", 8), ("BLEND_RGBA_ADD", 6), ("BLEND_RGB_MULT", 3), ("BLEND_RGB_ADD", 1)] {
        set(vm, n, Value::Int(v));
    }
    for sub in ["event", "key", "mouse", "time", "display", "scrap", "image", "joystick", "controller", "transform", "mixer", "font", "draw", "rect", "surface", "locals"] {
        let full = format!("pygame_sdl2.{sub}");
        if let Ok(sm) = py::import(vm, &full) {
            set(vm, sub, sm);
        }
    }
    Ok(m)
}

fn init_event(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "pygame_sdl2.event", &[
        ("event_name", f_event_name),
        ("post", f_post),
        ("get", f_event_get),
        ("poll", f_event_poll),
        ("wait", f_event_wait),
        ("peek", f_event_peek),
        ("clear", f_event_clear),
        ("pump", f_none),
        ("register", f_event_register),
        ("set_blocked", f_none),
        ("set_allowed", f_none),
        ("get_blocked", |_, _, _| Ok(Value::Bool(false))),
        ("set_grab", f_none),
        ("get_grab", |_, _, _| Ok(Value::Bool(false))),
        ("copy_event_queue", f_event_get),
    ]);
    let ec = py::class(vm, "pygame_sdl2.event", "EventType", &[], &[
        ("__init__", ev_init),
        ("__repr__", ev_repr),
        ("__eq__", ev_eq),
    ], &[])?;
    let dict_prop = vm.types.new_builtin("dict", ev_dict);
    let prop = py::property(vm, dict_prop, None)?;
    vm.types.set_attr(&ec, "dict", prop);
    crysirror_python::modules::set(vm, &m, "EventType", Value::Obj(ec.clone()));
    crysirror_python::modules::set(vm, &m, "Event", Value::Obj(ec));
    Ok(m)
}

fn init_key(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "pygame_sdl2.key", &[
        ("get_mods", f_get_mods),
        ("set_mods", |vm, a, _| {
            let v = py::int(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
            input(vm).borrow_mut().mods = v;
            Ok(Value::None)
        }),
        ("get_pressed", f_get_pressed),
        ("name", f_key_name),
        ("set_repeat", |vm, a, _| {
            let d = py::int(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
            let i = py::int(vm, &a.get(1).cloned().unwrap_or(Value::Int(0)))?;
            input(vm).borrow_mut().key_repeat = (d, i);
            Ok(Value::None)
        }),
        ("get_repeat", |vm, _, _| {
            let r = input(vm).borrow_mut().key_repeat;
            Ok(vm.types.new_tuple(vec![Value::Int(r.0), Value::Int(r.1)]))
        }),
        ("start_text_input", f_text_input),
        ("stop_text_input", f_stop_text_input),
        ("set_text_input_rect", f_none),
        ("has_screen_keyboard_support", |_, _, _| Ok(Value::Bool(false))),
        ("get_focused", |_, _, _| Ok(Value::Bool(true))),
    ]);
    let pc = py::class(vm, "pygame_sdl2.key", "_Pressed", &[], &[("__init__", pressed_init), ("__getitem__", pressed_getitem)], &[])?;
    crysirror_python::modules::set(vm, &m, "_Pressed", Value::Obj(pc));
    Ok(m)
}

fn init_mouse(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(crysirror_python::modules::module_with(vm, "pygame_sdl2.mouse", &[
        ("get_pos", f_mouse_get_pos),
        ("get_pressed", f_mouse_get_pressed),
        ("get_focused", f_mouse_get_focused),
        ("set_pos", |vm, a, _| {
            let p = py::items(vm, &a.first().cloned().unwrap_or(Value::None))?;
            if p.len() == 2 {
                let x = py::int(vm, &p[0])? as i32;
                let y = py::int(vm, &p[1])? as i32;
                input(vm).borrow_mut().pos = (x, y);
            }
            Ok(Value::None)
        }),
        ("set_visible", |_, _, _| Ok(Value::Bool(true))),
        ("get_visible", |_, _, _| Ok(Value::Bool(true))),
        ("set_cursor", f_none),
        ("get_rel", |vm, _, _| Ok(vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]))),
    ]))
}

fn init_time(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(crysirror_python::modules::module_with(vm, "pygame_sdl2.time", &[
        ("get_ticks", f_get_ticks),
        ("wait", |_, _, _| Ok(Value::Int(0))),
        ("delay", |_, _, _| Ok(Value::Int(0))),
        ("set_timer", f_set_timer),
    ]))
}

fn init_scrap(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(crysirror_python::modules::module_with(vm, "pygame_sdl2.scrap", &[
        ("init", f_none),
        ("get", |vm, _, _| Ok(vm.types.new_bytes(super::clipboard_text(vm).into_bytes()))),
        ("put", |vm, a, _| {
            let v = a.get(1).cloned().unwrap_or(Value::None);
            let text = match ops::bytes_of(&v) {
                Some(b) => String::from_utf8_lossy(&b).into_owned(),
                None => py::string(vm, &v)?,
            };
            super::set_clipboard_text(vm, text);
            Ok(Value::None)
        }),
        ("get_types", |vm, _, _| Ok(vm.types.new_list(Vec::new()))),
        ("contains", |_, _, _| Ok(Value::Bool(false))),
        ("lost", |_, _, _| Ok(Value::Bool(false))),
        ("set_mode", f_none),
    ]))
}

/// `pygame_sdl2.display.Info()`: the host canvas stands for the desktop.
fn info_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let [w, h] = super::host(vm).borrow().canvas;
    for (n, v) in [("current_w", w), ("current_h", h), ("refresh_rate", 60), ("bitsize", 32), ("bytesize", 4)] {
        py::set(vm, &a[0], n, Value::Int(v as i64))?;
    }
    Ok(Value::None)
}

fn init_display(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = init_display_funcs(vm);
    let info = py::class(vm, "pygame_sdl2.display", "Info", &[], &[("__init__", info_init)], &[])?;
    crysirror_python::modules::set(vm, &m, "Info", Value::Obj(info));
    Ok(m)
}

fn init_display_funcs(vm: &mut Vm) -> ObjRef {
    crysirror_python::modules::module_with(vm, "pygame_sdl2.display", &[
        ("init", f_none),
        ("get_init", |_, _, _| Ok(Value::Bool(true))),
        ("get_driver", |vm, _, _| Ok(vm.types.new_str("crysirror"))),
        ("get_num_video_displays", |_, _, _| Ok(Value::Int(1))),
        ("get_active", |_, _, _| Ok(Value::Bool(true))),
        ("set_caption", |vm, a, _| {
            if let Some(t) = a.first() {
                let t = py::string(vm, t)?;
                super::set_window_title(vm, t);
            }
            Ok(Value::None)
        }),
        ("set_icon", f_none),
        ("hint", f_none),
        ("get_surface", f_none),
        ("toggle_fullscreen", f_none),
        ("iconify", |_, _, _| Ok(Value::Bool(false))),
    ])
}

fn init_joystick(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(crysirror_python::modules::module_with(vm, "pygame_sdl2.joystick", &[
        ("init", f_none),
        ("quit", f_none),
        ("get_init", |_, _, _| Ok(Value::Bool(true))),
        ("get_count", |_, _, _| Ok(Value::Int(0))),
    ]))
}

fn init_controller(vm: &mut Vm) -> PyResult<ObjRef> {
    Ok(crysirror_python::modules::module_with(vm, "pygame_sdl2.controller", &[
        ("init", f_none),
        ("quit", f_none),
        ("get_count", |_, _, _| Ok(Value::Int(0))),
        ("add_mapping", f_none),
        ("add_mappings", f_none),
        ("get_string_for_axis", |vm, a, _| {
            let i = a.first().map(|v| py::int(vm, v)).transpose()?.unwrap_or(-1);
            Ok(super::input::AXES.get(i as usize).map_or(Value::None, |s| vm.types.new_str(s)))
        }),
        ("get_string_for_button", |vm, a, _| {
            let i = a.first().map(|v| py::int(vm, v)).transpose()?.unwrap_or(-1);
            Ok(super::input::BUTTONS.get(i as usize).map_or(Value::None, |s| vm.types.new_str(s)))
        }),
    ]))
}

pub fn install(vm: &mut Vm) {
    vm.ext_set(std::rc::Rc::new(RefCell::new(InputState::default())));
    vm.register_native_module("pygame_sdl2", init_root);
    vm.register_native_module("pygame_sdl2.event", init_event);
    vm.register_native_module("pygame_sdl2.key", init_key);
    vm.register_native_module("pygame_sdl2.mouse", init_mouse);
    vm.register_native_module("pygame_sdl2.time", init_time);
    vm.register_native_module("pygame_sdl2.scrap", init_scrap);
    vm.register_native_module("pygame_sdl2.display", init_display);
    vm.register_native_module("pygame_sdl2.joystick", init_joystick);
    vm.register_native_module("pygame_sdl2.controller", init_controller);
}

/// Host pointer coordinates are in the client canvas; Ren'Py events use
/// the virtual screen (`config.screen_width/height`).
/// How the virtual screen (`config.screen_width/height`) sits in the
/// host canvas: uniformly scaled and centred (letterboxed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub x: f64,
    pub y: f64,
}

impl Fit {
    pub const IDENTITY: Fit = Fit { scale: 1., x: 0., y: 0. };

    /// Canvas point -> virtual point.
    pub fn to_virtual(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x) / self.scale, (y - self.y) / self.scale)
    }

    /// Virtual point -> canvas point.
    pub fn to_canvas(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.scale + self.x, y * self.scale + self.y)
    }
}

pub fn input_scale(vm: &mut Vm) -> Fit {
    let canvas = super::host(vm).borrow().canvas;
    let sw = py::config_f64(vm, "screen_width").unwrap_or(canvas[0] as f64);
    let sh = py::config_f64(vm, "screen_height").unwrap_or(canvas[1] as f64);
    if canvas[0] <= 0 || canvas[1] <= 0 || sw <= 0. || sh <= 0. {
        return Fit::IDENTITY;
    }
    let (cw, ch) = (canvas[0] as f64, canvas[1] as f64);
    let scale = (cw / sw).min(ch / sh);
    Fit { scale, x: (cw - sw * scale) / 2., y: (ch - sh * scale) / 2. }
}
