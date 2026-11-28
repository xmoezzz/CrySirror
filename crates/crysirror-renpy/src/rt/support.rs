// SPDX-License-Identifier: MPL-2.0
//! Small engine modules, after Ren'Py 8.3 (MIT): `renpy.pyanalysis`
//! (constant registries), `renpy.display.minigame`, `renpy.editor`,
//! `renpy.memory`, `renpy.lint` (the helpers statements call),
//! `renpy.bootstrap`, `renpy.gl2.live2d`, `renpy.gl2.gl2shadercache`
//! (shader-part registry) and `renpy.translation.generation`.
//!
//! Developer tooling that inspects the host (editor launch, memory
//! profiling, the lint report) has no host here: launching an editor
//! reports failure as upstream does without one, and the profilers write
//! a log line saying they are unavailable. Live2D models are not
//! supported (`has_live2d()` is False; `Live2D` raises). Registered
//! shader parts are kept for the renderer; a displayable that uses a
//! custom shader is rejected when drawn.
use super::py::{self, *};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

fn module(
    vm: &mut Vm,
    name: &str,
    funcs: &[(&'static str, crysirror_python::object::NativeFn)],
) -> ObjRef {
    crysirror_python::modules::module_with(vm, name, funcs)
}

fn set(vm: &Vm, m: &ObjRef, n: &str, v: Value) {
    crysirror_python::modules::set(vm, m, n, v);
}

fn str_set(vm: &mut Vm, items: &[&str]) -> PyResult<Value> {
    let s = vm.types.new_set(Default::default());
    for i in items {
        ops::set_add(vm, &s, vm.types.new_str(i))?;
    }
    Ok(s)
}

// renpy.pyanalysis ----------------------------------------------------------

const PURE_FUNCTIONS: &[&str] = &[
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "bytes",
    "callable",
    "chr",
    "complex",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "hasattr",
    "hash",
    "hex",
    "int",
    "isinstance",
    "issubclass",
    "len",
    "list",
    "map",
    "max",
    "min",
    "oct",
    "ord",
    "pow",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "slice",
    "sorted",
    "str",
    "sum",
    "tuple",
    "type",
    "zip",
    "_",
    "_p",
    "absolute",
    "position",
    "__renpy__list__",
    "__renpy__dict__",
    "__renpy__set__",
    "ImageReference",
    "Image",
    "Frame",
    "Solid",
    "LiveComposite",
    "LiveCrop",
    "LiveTile",
    "Flatten",
    "Null",
    "Window",
    "Viewport",
    "DynamicDisplayable",
    "ConditionSwitch",
    "ShowingSwitch",
    "Transform",
    "Animation",
    "Movie",
    "Particles",
    "SnowBlossom",
    "Text",
    "ParameterizedText",
    "FontGroup",
    "Drag",
    "Alpha",
    "AlphaMask",
    "Position",
    "Pan",
    "Move",
    "Motion",
    "Revolve",
    "Zoom",
    "RotoZoom",
    "FactorZoom",
    "SizeZoom",
    "Fade",
    "Dissolve",
    "ImageDissolve",
    "AlphaDissolve",
    "CropMove",
    "PushMove",
    "Pixellate",
    "OldMoveTransition",
    "MoveTransition",
    "MoveFactory",
    "MoveIn",
    "MoveOut",
    "ZoomInOut",
    "RevolveInOut",
    "MultipleTransition",
    "ComposeTransition",
    "Pause",
    "SubTransition",
    "ADVSpeaker",
    "ADVCharacter",
    "Speaker",
    "Character",
    "DynamicCharacter",
    "Fixed",
    "HBox",
    "VBox",
    "Grid",
    "AlphaBlend",
    "At",
    "color",
    "Color",
    "ui.returns",
    "ui.jumps",
    "ui.jumpsoutofcontext",
    "ui.callsinnewcontext",
    "ui.invokesinnewcontext",
    "ui.gamemenus",
    "renpy.version_string",
    "renpy.version_only",
    "renpy.version_tuple",
    "renpy.version_name",
    "renpy.license",
];

const PA: &str = "renpy.pyanalysis";

fn pa_set(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, PA, name)
}

fn pa_const(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let nc = pa_set(vm, "not_constants")?;
    if !ops::contains(vm, &nc, &args[0])? {
        let c = pa_set(vm, "constants")?;
        ops::set_add(vm, &c, args[0].clone())?;
    }
    Ok(Value::None)
}

fn pa_not_const(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for s in ["constants", "pure_functions"] {
        let c = pa_set(vm, s)?;
        vm.call_method(&c, "discard", vec![args[0].clone()])?;
    }
    let nc = pa_set(vm, "not_constants")?;
    ops::set_add(vm, &nc, args[0].clone())?;
    Ok(Value::None)
}

fn pa_pure(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = &args[0];
    let mut name = match f.as_str() {
        Some(s) => s.to_owned(),
        None => {
            let n = py::get(vm, f, "__name__")?;
            let m = py::get(vm, f, "__module__")?;
            format!("{}.{}", py::string(vm, &m)?, py::string(vm, &n)?)
        }
    };
    if let Some(rest) = name.strip_prefix("store.") {
        name = rest.to_owned();
    }
    let n = vm.types.new_str(&name);
    let nc = pa_set(vm, "not_constants")?;
    if !ops::contains(vm, &nc, &n)? {
        for s in ["pure_functions", "constants"] {
            let c = pa_set(vm, s)?;
            ops::set_add(vm, &c, n.clone())?;
        }
    }
    Ok(f.clone())
}

fn noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn pyanalysis(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module(
        vm,
        PA,
        &[
            ("const", pa_const),
            ("not_const", pa_not_const),
            ("pure", pa_pure),
            ("load_cache", noop),
            ("save_cache", noop),
        ],
    );
    let always = str_set(vm, &["True", "False", "None"])?;
    let pure = str_set(vm, PURE_FUNCTIONS)?;
    let constants = str_set(vm, &["config", "style", "True", "False", "None"])?;
    vm.call_method(&constants, "update", vec![pure.clone()])?;
    set(vm, &m, "always_constants", always);
    set(vm, &m, "pure_functions", pure);
    set(vm, &m, "constants", constants);
    set(
        vm,
        &m,
        "not_constants",
        vm.types.new_set(Default::default()),
    );
    set(
        vm,
        &m,
        "local_constants",
        vm.types.new_set(Default::default()),
    );
    Ok(m)
}

// renpy.display.minigame ------------------------------------------------------

fn minigame(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Minigame is no longer implemented."))
}

// renpy.editor ------------------------------------------------------------------

/// No editor can be launched from the player: upstream's result when no
/// editor is configured.
fn launch_editor(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

// renpy.memory -----------------------------------------------------------------

fn memory_unavailable(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    super::log::write(vm, "Memory profiling is not available in this runtime.");
    Ok(Value::None)
}

// renpy.lint ---------------------------------------------------------------------

fn lint_report(vm: &mut Vm, msg: &str) -> PyResult<()> {
    let m = py::import(vm, "renpy.lint")?;
    let node = py::get(vm, &m, "report_node")?;
    let mut out = String::new();
    if !py::is_none(&node) {
        let f = py::get(vm, &node, "filename")?;
        let l = py::get(vm, &node, "linenumber")?;
        out = format!("{}:{} ", py::string(vm, &f)?, py::int(vm, &l)?);
    }
    out.push_str(msg);
    let print = py::import_attr(vm, "builtins", "print")?;
    vm.call(&print, vec![py::s(vm, "")], Vec::new())?;
    let o = vm.types.new_str(&out);
    vm.call(&print, vec![o], Vec::new())?;
    py::set(vm, &m, "error_reported", Value::Bool(true))?;
    Ok(())
}

fn lint_add(vm: &mut Vm, msg: &Value) -> PyResult<()> {
    let added = py::import_attr(vm, "renpy.lint", "added")?;
    if ops::contains(vm, &added, msg)? {
        return Ok(());
    }
    ops::setitem(vm, &added, msg.clone(), Value::Bool(true))?;
    let print = py::import_attr(vm, "builtins", "print")?;
    vm.call(&print, vec![msg.clone()], Vec::new())?;
    Ok(())
}

fn compiles(vm: &mut Vm, expr: &str) -> bool {
    super::python::compile(vm, expr, "eval", "<lint>", 1, None).is_ok()
}

fn try_compile(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let where_ = py::string(vm, &args[0])?;
    let expr = py::string(vm, &args[1])?;
    let additional = arg_or(&args[2..], kw, 0, "additional", Value::None);
    if !compiles(vm, &expr) {
        lint_report(
            vm,
            &format!("'{expr}' could not be compiled as a python expression, {where_}."),
        )?;
        if !py::is_none(&additional) {
            lint_add(vm, &additional)?;
        }
    }
    Ok(Value::None)
}

fn try_eval(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    try_compile(vm, &args[..2], &[])?;
    let where_ = py::string(vm, &args[0])?;
    let expr = py::string(vm, &args[1])?;
    let additional = arg_or(&args[2..], kw, 0, "additional", Value::None);
    let t = expr.trim_start();
    let name: String = t
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Ok(Value::None);
    }
    let store = py::import(vm, "store")?;
    if py::get_opt(vm, &store, &name)?.is_some() {
        return Ok(Value::None);
    }
    let b = py::import(vm, "builtins")?;
    if py::get_opt(vm, &b, &name)?.is_some() {
        return Ok(Value::None);
    }
    lint_report(vm, &format!("Could not evaluate '{expr}', in {where_}."))?;
    if !py::is_none(&additional) {
        lint_add(vm, &additional)?;
    }
    Ok(Value::None)
}

fn lint_run(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "The lint report is not available in this runtime.",
    ))
}

fn lint(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module(
        vm,
        "renpy.lint",
        &[
            ("try_compile", try_compile),
            ("try_eval", try_eval),
            ("lint", lint_run),
        ],
    );
    set(vm, &m, "report_node", Value::None);
    set(vm, &m, "error_reported", Value::Bool(false));
    set(vm, &m, "added", py::dict(vm));
    Ok(m)
}

// renpy.bootstrap --------------------------------------------------------------

/// `get_alternate_base(basedir, always=False)`: updated engine bases do not
/// exist for this runtime, so the base directory is the game's own.
fn get_alternate_base(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let base = args[0].clone();
    let always = arg_or(&args[1..], kw, 0, "always", Value::Bool(false));
    if py::truthy(vm, &always)? {
        let s = py::string(vm, &base)?;
        return Ok(vm
            .types
            .new_str(&format!("{}/base", s.trim_end_matches('/'))));
    }
    Ok(base)
}

// renpy.gl2 ------------------------------------------------------------------------

fn has_live2d(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn live2d(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "Live2D is not supported by this runtime."))
}

// renpy.translation.generation ------------------------------------------------

/// `generic_filter(s, function)`: applies `function` to text outside
/// `[...]` substitutions and `{...}` tags.
pub fn generic_filter(vm: &mut Vm, s: &str, function: &Value) -> PyResult<String> {
    fn remove_special(
        vm: &mut Vm,
        s: &str,
        start: char,
        end: char,
        process: &mut dyn FnMut(&mut Vm, &str) -> PyResult<String>,
    ) -> PyResult<String> {
        let mut specials = 0;
        let mut rv = String::new();
        let mut buf = String::new();
        for i in s.chars() {
            if i == start {
                if buf.len() == i.len_utf8() && buf.starts_with(i) && specials > 0 {
                    rv.push_str(&buf);
                    rv.push(i);
                    specials = 0;
                    buf.clear();
                    continue;
                }
                if specials == 0 {
                    rv.push_str(&process(vm, &buf)?);
                    buf.clear();
                }
                buf.push(i);
                specials += 1;
            } else if i == end && specials > 0 {
                buf.push(i);
                specials -= 1;
                if specials == 0 {
                    rv.push_str(&buf);
                    buf.clear();
                }
            } else {
                buf.push(i);
            }
        }
        if !buf.is_empty() {
            if specials == 0 {
                rv.push_str(&process(vm, &buf)?);
            } else {
                rv.push_str(&buf);
            }
        }
        Ok(rv)
    }
    let mut call = |vm: &mut Vm, t: &str| -> PyResult<String> {
        let a = vm.types.new_str(t);
        let r = vm.call(function, vec![a], Vec::new())?;
        py::string(vm, &r)
    };
    let mut braces = |vm: &mut Vm, t: &str| remove_special(vm, t, '{', '}', &mut call);
    remove_special(vm, s, '[', ']', &mut braces)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(PA, pyanalysis);
    vm.register_native_module("renpy.display.minigame", |vm| {
        Ok(module(
            vm,
            "renpy.display.minigame",
            &[("Minigame", minigame)],
        ))
    });
    vm.register_native_module("renpy.editor", |vm| {
        let m = module(
            vm,
            "renpy.editor",
            &[("launch_editor", launch_editor), ("init", noop)],
        );
        set(vm, &m, "editor", Value::None);
        Ok(m)
    });
    vm.register_native_module("renpy.memory", |vm| {
        Ok(module(
            vm,
            "renpy.memory",
            &[
                ("profile_memory", memory_unavailable),
                ("diff_memory", memory_unavailable),
                ("profile_rollback", memory_unavailable),
                ("find_parents", memory_unavailable),
                ("print_garbage", memory_unavailable),
            ],
        ))
    });
    vm.register_native_module("renpy.lint", lint);
    vm.register_native_module("renpy.bootstrap", |vm| {
        Ok(module(
            vm,
            "renpy.bootstrap",
            &[("get_alternate_base", get_alternate_base)],
        ))
    });
    vm.register_native_module("renpy.gl2.live2d", |vm| {
        Ok(module(
            vm,
            "renpy.gl2.live2d",
            &[
                ("has_live2d", has_live2d),
                ("Live2D", live2d),
                ("reset", noop),
                ("reset_states", noop),
                ("update_states", noop),
                ("init", noop),
            ],
        ))
    });
}
