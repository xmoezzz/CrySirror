// SPDX-License-Identifier: MPL-2.0
//! `renpy.exports`, after Ren'Py 8.3 `exports/__init__.py` (MIT): the
//! public `renpy.*` API. Functions are native, grouped like upstream's
//! `*exports.py` modules; the remaining names are re-exports of other
//! engine modules.
//!
//! A re-export whose source is not part of this runtime is a function
//! that raises when called and names the missing source, so an import of
//! `renpy.exports` never hides an unported capability behind a working
//! name.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};

pub mod common;
pub mod context;
pub mod display;
pub mod io;
pub mod menu;
pub mod misc;
pub mod rollback;
pub mod say;
pub mod statement;
#[rustfmt::skip]
pub mod submodules;
pub mod util;

const MODULE: &str = "renpy.exports";

/// (source module, source name, exported name).
const REEXPORTS: &[(&str, &str, &str)] = &[
    ("renpy.ast", "eval_who", "eval_who"),
    ("renpy.atl", "atl_warper", "atl_warper"),
    (
        "renpy.bootstrap",
        "get_alternate_base",
        "get_alternate_base",
    ),
    ("renpy.character", "display_say", "display_say"),
    (
        "renpy.character",
        "predict_show_display_say",
        "predict_show_display_say",
    ),
    ("renpy.character", "show_display_say", "show_display_say"),
    ("renpy.curry", "curry", "curry"),
    ("renpy.curry", "partial", "partial"),
    ("renpy.display.behavior", "Keymap", "Keymap"),
    (
        "renpy.display.behavior",
        "clear_keymap_cache",
        "clear_keymap_cache",
    ),
    ("renpy.display.behavior", "is_selected", "is_selected"),
    ("renpy.display.behavior", "is_sensitive", "is_sensitive"),
    ("renpy.display.behavior", "map_event", "map_event"),
    ("renpy.display.behavior", "queue_event", "queue_event"),
    ("renpy.display.behavior", "run", "run"),
    ("renpy.display.behavior", "run", "run_action"),
    ("renpy.display.behavior", "run_periodic", "run_periodic"),
    ("renpy.display.behavior", "run_unhovered", "run_unhovered"),
    ("renpy.display.focus", "capture_focus", "capture_focus"),
    (
        "renpy.display.focus",
        "clear_capture_focus",
        "clear_capture_focus",
    ),
    (
        "renpy.display.focus",
        "focus_coordinates",
        "focus_coordinates",
    ),
    ("renpy.display.focus", "get_focus_rect", "get_focus_rect"),
    ("renpy.display.im", "load_image", "load_image"),
    ("renpy.display.im", "load_rgba", "load_rgba"),
    ("renpy.display.im", "load_surface", "load_surface"),
    (
        "renpy.display.image",
        "check_image_attributes",
        "check_image_attributes",
    ),
    (
        "renpy.display.image",
        "get_available_image_attributes",
        "get_available_image_attributes",
    ),
    (
        "renpy.display.image",
        "get_available_image_tags",
        "get_available_image_tags",
    ),
    (
        "renpy.display.image",
        "get_ordered_image_attributes",
        "get_ordered_image_attributes",
    ),
    (
        "renpy.display.image",
        "get_registered_image",
        "get_registered_image",
    ),
    ("renpy.display.image", "image_exists", "image_exists"),
    ("renpy.display.image", "image_exists", "has_image"),
    ("renpy.display.image", "list_images", "list_images"),
    ("renpy.display.minigame", "Minigame", "Minigame"),
    ("renpy.display.predict", "screen", "predict_screen"),
    ("renpy.display.screen", "ScreenProfile", "profile_screen"),
    ("renpy.display.screen", "current_screen", "current_screen"),
    ("renpy.display.screen", "define_screen", "define_screen"),
    ("renpy.display.screen", "get_displayable", "get_displayable"),
    (
        "renpy.display.screen",
        "get_displayable_properties",
        "get_displayable_properties",
    ),
    ("renpy.display.screen", "get_screen", "get_screen"),
    (
        "renpy.display.screen",
        "get_screen_variable",
        "get_screen_variable",
    ),
    ("renpy.display.screen", "get_widget", "get_widget"),
    (
        "renpy.display.screen",
        "get_widget_properties",
        "get_widget_properties",
    ),
    ("renpy.display.screen", "has_screen", "has_screen"),
    ("renpy.display.screen", "hide_screen", "hide_screen"),
    (
        "renpy.display.screen",
        "set_screen_variable",
        "set_screen_variable",
    ),
    ("renpy.display.screen", "show_screen", "show_screen"),
    ("renpy.display.screen", "use_screen", "use_screen"),
    ("renpy.display.tts", "speak", "alt"),
    ("renpy.display.tts", "speak_extra_alt", "speak_extra_alt"),
    (
        "renpy.display.video",
        "movie_start_displayable",
        "movie_start_displayable",
    ),
    (
        "renpy.display.video",
        "movie_start_fullscreen",
        "movie_start_fullscreen",
    ),
    ("renpy.display.video", "movie_stop", "movie_stop"),
    ("renpy.easy", "displayable", "displayable"),
    ("renpy.easy", "predict", "predict"),
    ("renpy.easy", "split_properties", "split_properties"),
    ("renpy.editor", "launch_editor", "launch_editor"),
    ("renpy.execution", "not_infinite_loop", "not_infinite_loop"),
    (
        "renpy.execution",
        "reset_all_contexts",
        "reset_all_contexts",
    ),
    (
        "renpy.gl2.gl2shadercache",
        "register_shader",
        "register_shader",
    ),
    ("renpy.gl2.live2d", "has_live2d", "has_live2d"),
    ("renpy.lexer", "unelide_filename", "unelide_filename"),
    ("renpy.lint", "try_compile", "try_compile"),
    ("renpy.lint", "try_eval", "try_eval"),
    (
        "renpy.loader",
        "add_python_directory",
        "add_python_directory",
    ),
    ("renpy.loadsave", "can_load", "can_load"),
    ("renpy.loadsave", "copy_save", "copy_save"),
    ("renpy.loadsave", "force_autosave", "force_autosave"),
    ("renpy.loadsave", "list_saved_games", "list_saved_games"),
    ("renpy.loadsave", "list_slots", "list_slots"),
    ("renpy.loadsave", "load", "load"),
    ("renpy.loadsave", "newest_slot", "newest_slot"),
    ("renpy.loadsave", "rename_save", "rename_save"),
    ("renpy.loadsave", "save", "save"),
    ("renpy.loadsave", "scan_saved_game", "scan_saved_game"),
    ("renpy.loadsave", "slot_json", "slot_json"),
    ("renpy.loadsave", "slot_mtime", "slot_mtime"),
    ("renpy.loadsave", "slot_screenshot", "slot_screenshot"),
    ("renpy.loadsave", "unlink_save", "unlink_save"),
    ("renpy.memory", "diff_memory", "diff_memory"),
    ("renpy.memory", "profile_memory", "profile_memory"),
    ("renpy.memory", "profile_rollback", "profile_rollback"),
    ("renpy.parser", "get_parse_errors", "get_parse_errors"),
    (
        "renpy.persistent",
        "register_persistent",
        "register_persistent",
    ),
    ("renpy.pyanalysis", "const", "const"),
    ("renpy.pyanalysis", "not_const", "not_const"),
    ("renpy.pyanalysis", "pure", "pure"),
    ("renpy.python", "py_eval", "eval"),
    ("renpy.rollback", "rng", "random"),
    (
        "renpy.savetoken",
        "get_save_token_keys",
        "get_save_token_keys",
    ),
    (
        "renpy.sl2.slparser",
        "CustomParser",
        "register_sl_statement",
    ),
    (
        "renpy.sl2.slparser",
        "register_sl_displayable",
        "register_sl_displayable",
    ),
    ("renpy.statements", "register", "register_statement"),
    (
        "renpy.text.extras",
        "ParameterizedText",
        "ParameterizedText",
    ),
    ("renpy.text.extras", "check_text_tags", "check_text_tags"),
    ("renpy.text.extras", "filter_text_tags", "filter_text_tags"),
    ("renpy.text.font", "register_bmfont", "register_bmfont"),
    (
        "renpy.text.font",
        "register_mudgefont",
        "register_mudgefont",
    ),
    ("renpy.text.font", "register_sfont", "register_sfont"),
    (
        "renpy.text.font",
        "variable_font_info",
        "variable_font_info",
    ),
    ("renpy.text.shader", "TextShader", "TextShader"),
    (
        "renpy.text.shader",
        "register_textshader",
        "register_textshader",
    ),
    ("renpy.text.text", "BASELINE", "BASELINE"),
    ("renpy.text.text", "language_tailor", "language_tailor"),
    ("renpy.text.textsupport", "DISPLAYABLE", "TEXT_DISPLAYABLE"),
    ("renpy.text.textsupport", "PARAGRAPH", "TEXT_PARAGRAPH"),
    ("renpy.text.textsupport", "TAG", "TEXT_TAG"),
    ("renpy.text.textsupport", "TEXT", "TEXT_TEXT"),
    ("renpy.translation", "change_language", "change_language"),
    (
        "renpy.translation",
        "get_translation_identifier",
        "get_translation_identifier",
    ),
    (
        "renpy.translation",
        "get_translation_info",
        "get_translation_info",
    ),
    ("renpy.translation", "known_languages", "known_languages"),
    ("renpy.translation", "translate_string", "translate_string"),
    (
        "renpy.translation.generation",
        "generic_filter",
        "transform_text",
    ),
    ("renpy.ui", "Choice", "Choice"),
    ("renpy.audio.sound", "", "sound"),
    ("renpy.audio.music", "", "music"),
];

/// The file of the statement being run (stands for the caller's file,
/// which natives cannot see).
pub(crate) fn current_filename(vm: &mut Vm) -> PyResult<String> {
    let f = py::import_attr(vm, MODULE, "get_filename_line")?;
    let r = vm.call(&f, Vec::new(), Vec::new())?;
    let n = crysirror_python::ops::getitem(vm, &r, Value::Int(0))?;
    py::string(vm, &n)
}

fn is_module_not_found(vm: &mut Vm, e: &crysirror_python::PyErr) -> bool {
    let crysirror_python::PyErr::Exc(o) = e else {
        return false;
    };
    let c = vm.types.exc("ModuleNotFoundError");
    crysirror_python::ops::isinstance(vm, &Value::Obj(o.clone()), &c)
}

/// A name whose source is not available: raises when used.
fn unavailable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let what = py::string(vm, &args[0])?;
    Err(py::error(
        vm,
        format!("renpy.{what} is not available in this runtime yet."),
    ))
}

fn renpy_pure(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if let Ok(p) = py::import_attr(vm, "renpy.pyanalysis", "pure") {
        let _ = vm.call(&p, vec![args[0].clone()], Vec::new());
    }
    Ok(args[0].clone())
}

fn version(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = arg_or(args, kw, 0, "tuple", Value::Bool(false));
    if py::truthy(vm, &t)? {
        return py::import_attr(vm, "renpy", "version_tuple");
    }
    py::import_attr(vm, "renpy", "version")
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[("renpy_pure", renpy_pure), ("version", version)],
    );
    let mut set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for (n, f) in context::functions()
        .into_iter()
        .chain(rollback::functions())
        .chain(statement::functions())
        .chain(say::functions())
        .chain(menu::functions())
        .chain(display::functions())
        .chain(misc::functions())
        .chain(io::functions())
    {
        let v = vm.types.new_builtin(n, f);
        set(vm, n, v);
    }
    let (v84, v85, v86) = {
        let p = &super::state::rt(vm).profile;
        (p.at_least(8, 4), p.at_least(8, 5), p.at_least(8, 6))
    };
    let mut later = Vec::new();
    if v84 {
        later.extend(common::functions_84());
    }
    if v85 {
        later.extend(common::functions_85());
    }
    if v86 {
        later.extend(common::functions_86());
        later.extend(common::functions_86_more());
    }
    for (n, f) in later {
        let v = vm.types.new_builtin(n, f);
        set(vm, n, v);
    }
    common::values(vm, &m)?;
    context::values(vm, &mut set)?;
    say::values(vm, &m)?;
    menu::values(vm, &m)?;
    display::values(vm, &m)?;
    misc::values(vm, &m)?;
    io::values(vm, &m)?;
    let mut missing = Vec::new();
    for (module, name, alias) in REEXPORTS {
        let v = match py::import(vm, module) {
            Ok(mv) if name.is_empty() => Some(mv),
            Ok(mv) => py::get_opt(vm, &mv, name)?,
            Err(e) if is_module_not_found(vm, &e) => None,
            Err(e) => return Err(e),
        };
        let v = match v {
            Some(v) => v,
            None => {
                missing.push(vm.types.new_str(&format!("{alias} ({module})")));
                let label = vm.types.new_str(alias);
                py::closure(vm, "unavailable", unavailable, label)
            }
        };
        set(vm, alias, v);
    }
    // Modules upstream's exports import at top level; scripts reach them
    // as `renpy.renpy`, `renpy.os`, ...
    for n in [
        "gc",
        "io",
        "re",
        "time",
        "sys",
        "threading",
        "fnmatch",
        "os",
        "renpy",
        "pygame_sdl2",
    ] {
        match py::import(vm, n) {
            Ok(mv) => set(vm, n, mv),
            Err(e) if is_module_not_found(vm, &e) => {
                missing.push(vm.types.new_str(&format!("{n} (module)")))
            }
            Err(e) => return Err(e),
        }
    }
    let missing = vm.types.new_list(missing);
    set(vm, "_crysirror_unavailable", missing);
    for n in ["version_only", "version_name", "version_tuple"] {
        let v = py::import_attr(vm, "renpy", n)?;
        set(vm, n, v);
    }
    let v = py::import_attr(vm, "renpy", "version")?;
    set(vm, "version_string", v);
    set(vm, "license", py::s(vm, ""));
    set(vm, "platform", py::s(vm, "CrySirror"));
    set(vm, "bits", Value::Int(64));
    set(vm, "menu_args", Value::None);
    set(vm, "menu_kwargs", Value::None);
    Ok(m)
}

/// `renpy.exports.<group>exports` (7.8 / 8.3+): the names upstream defines
/// in that submodule, which are the objects `renpy.exports` exports.
fn init_submodule(vm: &mut Vm, name: &'static str) -> PyResult<ObjRef> {
    let bit = super::aliases::tag_bit(vm);
    if !submodules::MODULES.iter().any(|(m, mask)| *m == name && mask & bit != 0) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{name}'")));
    }
    let exports = py::import(vm, MODULE)?;
    let m = vm.new_module(name);
    for (_, n, _) in submodules::NAMES.iter().filter(|(m, _, mask)| *m == name && mask & bit != 0) {
        if let Some(v) = py::get_opt(vm, &exports, n)? {
            crysirror_python::modules::set(vm, &m, n, v);
        }
    }
    Ok(m)
}

macro_rules! submodules {
    ($($name:literal,)*) => {
        fn register_submodules(vm: &mut Vm) {
            $(vm.register_native_module($name, |vm| init_submodule(vm, $name));)*
        }
    };
}

submodules! {
    "renpy.exports.actionexports",
    "renpy.exports.clipboardexports",
    "renpy.exports.commonexports",
    "renpy.exports.contextexports",
    "renpy.exports.debugexports",
    "renpy.exports.displayexports",
    "renpy.exports.fetchexports",
    "renpy.exports.inputexports",
    "renpy.exports.loaderexports",
    "renpy.exports.mediaexports",
    "renpy.exports.menuexports",
    "renpy.exports.persistentexports",
    "renpy.exports.platformexports",
    "renpy.exports.predictexports",
    "renpy.exports.restartexports",
    "renpy.exports.rollbackexports",
    "renpy.exports.sayexports",
    "renpy.exports.scriptexports",
    "renpy.exports.statementexports",
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
    register_submodules(vm);
}
