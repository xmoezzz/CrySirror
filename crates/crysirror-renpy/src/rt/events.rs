// SPDX-License-Identifier: MPL-2.0
//! `pygame_sdl2` surface used by Ren'Py and by game code: event types, key
//! and modifier constants (SDL2 values, as pygame_sdl2 exposes them), the
//! `Event` class, key/mouse state queries and the conversion of CrySirror
//! `InputFrame`s into pygame events.
use super::py;
use crysirror_core::service::{InputFrame, buttons};
use crysirror_python::{PyResult, Value, Vm};
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
/// pygame_sdl2's SDL1-style events (`SDL_LASTEVENT - 1` ... `- 4`).
pub const ACTIVEEVENT: i64 = 0xFFFE;
pub const VIDEORESIZE: i64 = 0xFFFD;
pub const VIDEOEXPOSE: i64 = 0xFFFC;
pub const WINDOWMOVED: i64 = 0xFFFB;
/// The highest user event number (SDL2 pygame_sdl2: `SDL_LASTEVENT - 6`).
pub const USEREVENT_MAX: i64 = 0xFFF9;

/// `pygame.event.get_standard_events()`.
pub fn standard_events() -> Vec<i64> {
    EVENT_NAMES
        .iter()
        .map(|(_, v)| *v)
        .filter(|v| *v != NOEVENT && (*v < USEREVENT || *v > USEREVENT_MAX))
        .collect()
}

/// `renpy.display.core.enabled_events`.
pub fn enabled_events() -> Vec<i64> {
    vec![
        QUIT,
        APP_TERMINATING,
        APP_LOWMEMORY,
        APP_WILLENTERBACKGROUND,
        APP_DIDENTERBACKGROUND,
        APP_WILLENTERFOREGROUND,
        APP_DIDENTERFOREGROUND,
        WINDOWEVENT,
        SYSWMEVENT,
        KEYDOWN,
        KEYUP,
        TEXTEDITING,
        TEXTINPUT,
        KEYMAPCHANGED,
        MOUSEMOTION,
        MOUSEBUTTONDOWN,
        MOUSEBUTTONUP,
        MOUSEWHEEL,
        JOYAXISMOTION,
        JOYHATMOTION,
        JOYBALLMOTION,
        JOYBUTTONDOWN,
        JOYBUTTONUP,
        JOYDEVICEADDED,
        JOYDEVICEREMOVED,
        CONTROLLERAXISMOTION,
        CONTROLLERBUTTONDOWN,
        CONTROLLERBUTTONUP,
        CONTROLLERDEVICEADDED,
        CONTROLLERDEVICEREMOVED,
        RENDER_TARGETS_RESET,
        TIMEEVENT,
        PERIODIC,
        REDRAW,
        EVENTNAME,
    ]
}

/// `renpy.display.core.input_events`.
pub fn input_events() -> Vec<i64> {
    vec![
        KEYDOWN,
        KEYUP,
        TEXTEDITING,
        TEXTINPUT,
        MOUSEMOTION,
        MOUSEBUTTONDOWN,
        MOUSEBUTTONUP,
        MOUSEWHEEL,
        JOYAXISMOTION,
        JOYHATMOTION,
        JOYBALLMOTION,
        JOYBUTTONDOWN,
        JOYBUTTONUP,
        CONTROLLERAXISMOTION,
        CONTROLLERBUTTONDOWN,
        CONTROLLERBUTTONUP,
    ]
}

pub const SC: i64 = 1 << 30;

/// (pygame name, SDL keycode) for keys Ren'Py's keymap and games use.
pub const KEYS: &[(&str, i64)] = &[
    ("K_UNKNOWN", 0),
    ("K_BACKSPACE", 8),
    ("K_TAB", 9),
    ("K_RETURN", 13),
    ("K_ESCAPE", 27),
    ("K_SPACE", 32),
    ("K_EXCLAIM", 33),
    ("K_QUOTEDBL", 34),
    ("K_HASH", 35),
    ("K_DOLLAR", 36),
    ("K_PERCENT", 37),
    ("K_AMPERSAND", 38),
    ("K_QUOTE", 39),
    ("K_LEFTPAREN", 40),
    ("K_RIGHTPAREN", 41),
    ("K_ASTERISK", 42),
    ("K_PLUS", 43),
    ("K_COMMA", 44),
    ("K_MINUS", 45),
    ("K_PERIOD", 46),
    ("K_SLASH", 47),
    ("K_0", 48),
    ("K_1", 49),
    ("K_2", 50),
    ("K_3", 51),
    ("K_4", 52),
    ("K_5", 53),
    ("K_6", 54),
    ("K_7", 55),
    ("K_8", 56),
    ("K_9", 57),
    ("K_COLON", 58),
    ("K_SEMICOLON", 59),
    ("K_LESS", 60),
    ("K_EQUALS", 61),
    ("K_GREATER", 62),
    ("K_QUESTION", 63),
    ("K_AT", 64),
    ("K_LEFTBRACKET", 91),
    ("K_BACKSLASH", 92),
    ("K_RIGHTBRACKET", 93),
    ("K_CARET", 94),
    ("K_UNDERSCORE", 95),
    ("K_BACKQUOTE", 96),
    ("K_a", 97),
    ("K_b", 98),
    ("K_c", 99),
    ("K_d", 100),
    ("K_e", 101),
    ("K_f", 102),
    ("K_g", 103),
    ("K_h", 104),
    ("K_i", 105),
    ("K_j", 106),
    ("K_k", 107),
    ("K_l", 108),
    ("K_m", 109),
    ("K_n", 110),
    ("K_o", 111),
    ("K_p", 112),
    ("K_q", 113),
    ("K_r", 114),
    ("K_s", 115),
    ("K_t", 116),
    ("K_u", 117),
    ("K_v", 118),
    ("K_w", 119),
    ("K_x", 120),
    ("K_y", 121),
    ("K_z", 122),
    ("K_DELETE", 127),
    ("K_CAPSLOCK", SC | 57),
    ("K_F1", SC | 58),
    ("K_F2", SC | 59),
    ("K_F3", SC | 60),
    ("K_F4", SC | 61),
    ("K_F5", SC | 62),
    ("K_F6", SC | 63),
    ("K_F7", SC | 64),
    ("K_F8", SC | 65),
    ("K_F9", SC | 66),
    ("K_F10", SC | 67),
    ("K_F11", SC | 68),
    ("K_F12", SC | 69),
    ("K_PRINTSCREEN", SC | 70),
    ("K_PRINT", SC | 70),
    ("K_SCROLLLOCK", SC | 71),
    ("K_PAUSE", SC | 72),
    ("K_INSERT", SC | 73),
    ("K_HOME", SC | 74),
    ("K_PAGEUP", SC | 75),
    ("K_END", SC | 77),
    ("K_PAGEDOWN", SC | 78),
    ("K_RIGHT", SC | 79),
    ("K_LEFT", SC | 80),
    ("K_DOWN", SC | 81),
    ("K_UP", SC | 82),
    ("K_NUMLOCKCLEAR", SC | 83),
    ("K_NUMLOCK", SC | 83),
    ("K_KP_DIVIDE", SC | 84),
    ("K_KP_MULTIPLY", SC | 85),
    ("K_KP_MINUS", SC | 86),
    ("K_KP_PLUS", SC | 87),
    ("K_KP_ENTER", SC | 88),
    ("K_KP1", SC | 89),
    ("K_KP2", SC | 90),
    ("K_KP3", SC | 91),
    ("K_KP4", SC | 92),
    ("K_KP5", SC | 93),
    ("K_KP6", SC | 94),
    ("K_KP7", SC | 95),
    ("K_KP8", SC | 96),
    ("K_KP9", SC | 97),
    ("K_KP0", SC | 98),
    ("K_KP_1", SC | 89),
    ("K_KP_2", SC | 90),
    ("K_KP_3", SC | 91),
    ("K_KP_4", SC | 92),
    ("K_KP_5", SC | 93),
    ("K_KP_6", SC | 94),
    ("K_KP_7", SC | 95),
    ("K_KP_8", SC | 96),
    ("K_KP_9", SC | 97),
    ("K_KP_0", SC | 98),
    ("K_KP_PERIOD", SC | 99),
    ("K_APPLICATION", SC | 101),
    ("K_POWER", SC | 102),
    ("K_KP_EQUALS", SC | 103),
    ("K_F13", SC | 104),
    ("K_F14", SC | 105),
    ("K_F15", SC | 106),
    ("K_HELP", SC | 117),
    ("K_MENU", SC | 118),
    ("K_SELECT", SC | 119),
    ("K_STOP", SC | 120),
    ("K_AGAIN", SC | 121),
    ("K_UNDO", SC | 122),
    ("K_MUTE", SC | 127),
    ("K_VOLUMEUP", SC | 128),
    ("K_VOLUMEDOWN", SC | 129),
    ("K_SYSREQ", SC | 154),
    ("K_CLEAR", SC | 156),
    ("K_LCTRL", SC | 224),
    ("K_LSHIFT", SC | 225),
    ("K_LALT", SC | 226),
    ("K_LGUI", SC | 227),
    ("K_LMETA", SC | 227),
    ("K_LSUPER", SC | 227),
    ("K_RCTRL", SC | 228),
    ("K_RSHIFT", SC | 229),
    ("K_RALT", SC | 230),
    ("K_RGUI", SC | 231),
    ("K_RMETA", SC | 231),
    ("K_RSUPER", SC | 231),
    ("K_MODE", SC | 257),
    ("K_AUDIONEXT", SC | 258),
    ("K_AUDIOPREV", SC | 259),
    ("K_AUDIOSTOP", SC | 260),
    ("K_AUDIOPLAY", SC | 261),
    ("K_AUDIOMUTE", SC | 262),
    ("K_AC_SEARCH", SC | 268),
    ("K_AC_HOME", SC | 269),
    ("K_AC_BACK", SC | 270),
    ("K_AC_FORWARD", SC | 271),
    ("K_AC_STOP", SC | 272),
    ("K_AC_REFRESH", SC | 273),
    ("K_AC_BOOKMARKS", SC | 274),
];

pub const MODS: &[(&str, i64)] = &[
    ("KMOD_NONE", 0),
    ("KMOD_LSHIFT", 1),
    ("KMOD_RSHIFT", 2),
    ("KMOD_LCTRL", 0x40),
    ("KMOD_RCTRL", 0x80),
    ("KMOD_LALT", 0x100),
    ("KMOD_RALT", 0x200),
    ("KMOD_LGUI", 0x400),
    ("KMOD_RGUI", 0x800),
    ("KMOD_LMETA", 0x400),
    ("KMOD_RMETA", 0x800),
    ("KMOD_NUM", 0x1000),
    ("KMOD_CAPS", 0x2000),
    ("KMOD_MODE", 0x4000),
    ("KMOD_SHIFT", 3),
    ("KMOD_CTRL", 0xC0),
    ("KMOD_ALT", 0x300),
    ("KMOD_GUI", 0xC00),
    ("KMOD_META", 0xC00),
];

pub const EVENT_NAMES: &[(&str, i64)] = &[
    ("NOEVENT", 0),
    ("QUIT", QUIT),
    ("APP_TERMINATING", APP_TERMINATING),
    ("APP_LOWMEMORY", APP_LOWMEMORY),
    ("APP_WILLENTERBACKGROUND", APP_WILLENTERBACKGROUND),
    ("APP_DIDENTERBACKGROUND", APP_DIDENTERBACKGROUND),
    ("APP_WILLENTERFOREGROUND", APP_WILLENTERFOREGROUND),
    ("APP_DIDENTERFOREGROUND", APP_DIDENTERFOREGROUND),
    ("WINDOWEVENT", WINDOWEVENT),
    ("SYSWMEVENT", SYSWMEVENT),
    ("KEYDOWN", KEYDOWN),
    ("KEYUP", KEYUP),
    ("TEXTEDITING", TEXTEDITING),
    ("TEXTINPUT", TEXTINPUT),
    ("KEYMAPCHANGED", KEYMAPCHANGED),
    ("MOUSEMOTION", MOUSEMOTION),
    ("MOUSEBUTTONDOWN", MOUSEBUTTONDOWN),
    ("MOUSEBUTTONUP", MOUSEBUTTONUP),
    ("MOUSEWHEEL", MOUSEWHEEL),
    ("JOYAXISMOTION", JOYAXISMOTION),
    ("JOYBALLMOTION", JOYBALLMOTION),
    ("JOYHATMOTION", JOYHATMOTION),
    ("JOYBUTTONDOWN", JOYBUTTONDOWN),
    ("JOYBUTTONUP", JOYBUTTONUP),
    ("JOYDEVICEADDED", JOYDEVICEADDED),
    ("JOYDEVICEREMOVED", JOYDEVICEREMOVED),
    ("CONTROLLERAXISMOTION", CONTROLLERAXISMOTION),
    ("CONTROLLERBUTTONDOWN", CONTROLLERBUTTONDOWN),
    ("CONTROLLERBUTTONUP", CONTROLLERBUTTONUP),
    ("CONTROLLERDEVICEADDED", CONTROLLERDEVICEADDED),
    ("CONTROLLERDEVICEREMOVED", CONTROLLERDEVICEREMOVED),
    ("CONTROLLERDEVICEREMAPPED", CONTROLLERDEVICEREMAPPED),
    ("FINGERDOWN", FINGERDOWN),
    ("FINGERUP", FINGERUP),
    ("FINGERMOTION", FINGERMOTION),
    ("MULTIGESTURE", MULTIGESTURE),
    ("CLIPBOARDUPDATE", CLIPBOARDUPDATE),
    ("DROPFILE", DROPFILE),
    ("AUDIODEVICEADDED", AUDIODEVICEADDED),
    ("RENDER_TARGETS_RESET", RENDER_TARGETS_RESET),
    ("USEREVENT", USEREVENT),
    ("ACTIVEEVENT", ACTIVEEVENT),
    ("VIDEORESIZE", VIDEORESIZE),
    ("VIDEOEXPOSE", VIDEOEXPOSE),
    ("WINDOWMOVED", WINDOWMOVED),
];

/// (InputFrame bit, SDL keycode) of keyboard buttons.
const BUTTON_KEYS: &[(u64, i64)] = &[
    (buttons::ENTER, 13),
    (buttons::SPACE, 32),
    (buttons::LEFT, SC | 80),
    (buttons::RIGHT, SC | 79),
    (buttons::UP, SC | 82),
    (buttons::DOWN, SC | 81),
    (buttons::TAB, 9),
    (buttons::KEY_W, 119),
    (buttons::KEY_A, 97),
    (buttons::KEY_S, 115),
    (buttons::KEY_D, 100),
    (buttons::SHIFT, SC | 225),
    (buttons::CTRL, SC | 224),
    (buttons::KEY_E, 101),
    (buttons::KEY_Q, 113),
    (buttons::KEY_F, 102),
    (buttons::KEY_R, 114),
    (buttons::ESCAPE, 27),
    (buttons::BACKSPACE, 8),
    (buttons::HOME, SC | 74),
    (buttons::END, SC | 77),
    (buttons::PAGE_UP, SC | 75),
    (buttons::PAGE_DOWN, SC | 78),
    (buttons::KEY_0, 48),
    (buttons::KEY_1, 49),
    (buttons::KEY_2, 50),
    (buttons::KEY_3, 51),
    (buttons::KEY_4, 52),
    (buttons::KEY_5, 53),
    (buttons::KEY_6, 54),
    (buttons::KEY_7, 55),
    (buttons::KEY_8, 56),
    (buttons::KEY_9, 57),
    (buttons::F1, SC | 58),
    (buttons::F2, SC | 59),
    (buttons::F3, SC | 60),
    (buttons::F4, SC | 61),
    (buttons::F5, SC | 62),
    (buttons::F6, SC | 63),
    (buttons::F7, SC | 64),
    (buttons::F8, SC | 65),
    (buttons::F9, SC | 66),
    (buttons::F10, SC | 67),
    (buttons::F11, SC | 68),
    (buttons::F12, SC | 69),
    (buttons::KEY_C, 99),
    (buttons::KEY_L, 108),
    (buttons::KEY_Z, 122),
    (buttons::INSERT, SC | 73),
    (buttons::ALT, SC | 226),
    (buttons::DELETE, 127),
    (buttons::KEY_H, 104),
    (buttons::KEY_I, 105),
    (buttons::KEY_J, 106),
    (buttons::KEY_M, 109),
    (buttons::KEY_V, 118),
    (buttons::KEY_T, 116),
    (buttons::KEY_B, 98),
    (buttons::KEY_N, 110),
];

const MOUSE_BUTTONS: &[(u64, i64)] = &[
    (buttons::MOUSE_LEFT, 1),
    (buttons::MOUSE_MIDDLE, 2),
    (buttons::MOUSE_RIGHT, 3),
];

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
    /// Timers set with `once=True`.
    pub one_shot: std::collections::HashSet<i64>,
    /// `time._suspend_timers`: due timers post nothing.
    pub timers_suspended: bool,
    pub next_user_event: i64,
    /// `event.set_blocked` types.
    pub blocked: std::collections::BTreeSet<i64>,
    pub grab: bool,
    /// `event.set_mousewheel_buttons(False)` (upstream default: buttons).
    pub mousewheel_axis: bool,
    /// `event.event_names` (type -> name), registered names included.
    pub event_names: Option<Value>,
    pub text_input_rect: Option<Value>,
    /// Pointer position at the last `mouse.get_rel`.
    pub rel_origin: Option<(i32, i32)>,
    pub cursor_visible: bool,
}

pub fn input(vm: &Vm) -> std::rc::Rc<RefCell<InputState>> {
    vm.ext_get::<std::rc::Rc<RefCell<InputState>>>()
        .expect("input state")
        .clone()
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

pub fn event_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "pygame_sdl2.event", "Event")
}

pub fn new_event(vm: &mut Vm, ty: i64, fields: Vec<(&str, Value)>) -> PyResult<Value> {
    let c = event_class(vm)?;
    let kw = fields
        .into_iter()
        .map(|(k, v)| (std::rc::Rc::from(k), v))
        .collect();
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
    let tuple2 = |vm: &Vm, a: f64, b: f64| {
        vm.types
            .new_tuple(vec![Value::Int(a as i64), Value::Int(b as i64)])
    };
    if pos != old_pos {
        let rel = tuple2(
            vm,
            (pos.0 - old_pos.0) as f64 / fit.scale,
            (pos.1 - old_pos.1) as f64 / fit.scale,
        );
        let held = |b: u64| Value::Int((frame.buttons_down & b != 0) as i64);
        let btns = vm.types.new_tuple(vec![
            held(buttons::MOUSE_LEFT),
            held(buttons::MOUSE_MIDDLE),
            held(buttons::MOUSE_RIGHT),
        ]);
        let p = tuple2(vm, lp.0, lp.1);
        out.push(new_event(
            vm,
            MOUSEMOTION,
            vec![
                ("pos", p),
                ("rel", rel),
                ("buttons", btns),
                ("touch", Value::Bool(false)),
            ],
        )?);
    }
    let pressed = frame.buttons_pressed | (frame.buttons_down & !old);
    let released = old & !frame.buttons_down & !frame.buttons_pressed;
    for &(bit, key) in BUTTON_KEYS {
        if pressed & bit != 0 {
            let uni = if (32..127).contains(&key) {
                let c = key as u8 as char;
                let c = if mods & 3 != 0 {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
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
            out.push(new_event(
                vm,
                KEYDOWN,
                vec![
                    ("key", Value::Int(key)),
                    ("mod", Value::Int(mods)),
                    ("unicode", uni),
                    ("scancode", Value::Int(key & !SC)),
                    ("repeat", Value::Bool(false)),
                ],
            )?);
            input(vm).borrow_mut().pressed_keys.insert(key);
            if frame.buttons_pressed & bit != 0 && frame.buttons_down & bit == 0 {
                out.push(new_event(
                    vm,
                    KEYUP,
                    vec![
                        ("key", Value::Int(key)),
                        ("mod", Value::Int(mods)),
                        ("scancode", Value::Int(key & !SC)),
                        ("repeat", Value::Bool(false)),
                    ],
                )?);
                input(vm).borrow_mut().pressed_keys.remove(&key);
            }
        } else if released & bit != 0 {
            out.push(new_event(
                vm,
                KEYUP,
                vec![
                    ("key", Value::Int(key)),
                    ("mod", Value::Int(mods)),
                    ("scancode", Value::Int(key & !SC)),
                    ("repeat", Value::Bool(false)),
                ],
            )?);
            input(vm).borrow_mut().pressed_keys.remove(&key);
        }
    }
    for &(bit, button) in MOUSE_BUTTONS {
        let p = tuple2(vm, lp.0, lp.1);
        if pressed & bit != 0 {
            out.push(new_event(
                vm,
                MOUSEBUTTONDOWN,
                vec![
                    ("pos", p.clone()),
                    ("button", Value::Int(button)),
                    ("touch", Value::Bool(false)),
                ],
            )?);
            input(vm).borrow_mut().mouse_buttons[button as usize] = true;
            if frame.buttons_pressed & bit != 0 && frame.buttons_down & bit == 0 {
                out.push(new_event(
                    vm,
                    MOUSEBUTTONUP,
                    vec![
                        ("pos", p),
                        ("button", Value::Int(button)),
                        ("touch", Value::Bool(false)),
                    ],
                )?);
                input(vm).borrow_mut().mouse_buttons[button as usize] = false;
            }
        } else if released & bit != 0 {
            out.push(new_event(
                vm,
                MOUSEBUTTONUP,
                vec![
                    ("pos", p),
                    ("button", Value::Int(button)),
                    ("touch", Value::Bool(false)),
                ],
            )?);
            input(vm).borrow_mut().mouse_buttons[button as usize] = false;
        }
    }
    for (bit, button, dy) in [(buttons::WHEEL_UP, 4, 1), (buttons::WHEEL_DOWN, 5, -1)] {
        if frame.buttons_pressed & bit != 0 || (frame.buttons_down & bit != 0 && old & bit == 0) {
            out.push(new_event(
                vm,
                MOUSEWHEEL,
                vec![
                    ("x", Value::Int(0)),
                    ("y", Value::Int(dy)),
                    ("flipped", Value::Bool(false)),
                    ("touch", Value::Bool(false)),
                ],
            )?);
            let p = tuple2(vm, lp.0, lp.1);
            out.push(new_event(
                vm,
                MOUSEBUTTONDOWN,
                vec![
                    ("pos", p.clone()),
                    ("button", Value::Int(button)),
                    ("touch", Value::Bool(false)),
                ],
            )?);
            out.push(new_event(
                vm,
                MOUSEBUTTONUP,
                vec![
                    ("pos", p),
                    ("button", Value::Int(button)),
                    ("touch", Value::Bool(false)),
                ],
            )?);
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

/// Queues the events of timers that are due at the current host time. A
/// one-shot timer ends once due; a suspended timer keeps its period but
/// posts nothing (a suspended one-shot is lost, as upstream).
pub fn due_timers(vm: &mut Vm) {
    let now = super::link::now_ms(vm);
    let s = input(vm);
    let mut s = s.borrow_mut();
    let mut fired = Vec::new();
    let mut ended = Vec::new();
    let suspended = s.timers_suspended;
    for t in s.timers.iter_mut() {
        if t.2 <= now {
            if !suspended {
                fired.push(t.0);
            }
            ended.push(t.0);
            t.2 = now + t.1.max(1);
        }
    }
    let once: Vec<i64> = ended.into_iter().filter(|t| s.one_shot.contains(t)).collect();
    for t in &once {
        s.one_shot.remove(t);
    }
    s.timers.retain(|t| !once.contains(&t.0));
    drop(s);
    for ty in fired {
        if let Ok(e) = new_event(vm, ty, Vec::new()) {
            input(vm).borrow_mut().queue.push(e);
        }
    }
}

pub fn install(vm: &mut Vm) {
    vm.ext_set(std::rc::Rc::new(RefCell::new(InputState::default())));
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
    pub const IDENTITY: Fit = Fit {
        scale: 1.,
        x: 0.,
        y: 0.,
    };

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
    Fit {
        scale,
        x: (cw - sw * scale) / 2.,
        y: (ch - sh * scale) / 2.,
    }
}
