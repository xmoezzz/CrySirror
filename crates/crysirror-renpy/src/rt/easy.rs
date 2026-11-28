// SPDX-License-Identifier: MPL-2.0
//! `renpy.easy`: displayable coercion and small helpers, after Ren'Py
//! `easy.py` (MIT), 6.18 through 8.6.
//!
//! Release differences: before 6.99.11 the duck-typed displayable check is
//! `hasattr(d, "parameterize")` (renamed `_duplicate` in 6.99.11), and
//! `color` is a hex-string/tuple parsing function rather than
//! `renpy.color.Color`; dynamic `[...]` images arrive in 6.99.7, Color and
//! list displayables in 6.99.11, `config.displayable_prefix` in 7.0.
use super::{
    core::is_displayable,
    py::{self, *},
    release,
};
use crysirror_python::{ObjRef, Payload, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.easy";

fn store_attr(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "store", name)
}

/// `lookup_displayable_prefix(d)`.
pub fn lookup_displayable_prefix(vm: &mut Vm, d: &str) -> PyResult<Option<Value>> {
    let Some((prefix, arg)) = d.split_once(':') else {
        return Ok(None);
    };
    let table = py::config(vm, "displayable_prefix")?;
    if py::is_none(&table) {
        return Ok(None);
    }
    let key = vm.types.new_str(prefix);
    let f = vm.call_method(&table, "get", vec![key, Value::None])?;
    if py::is_none(&f) {
        return Ok(None);
    }
    let a = vm.types.new_str(arg);
    let r = vm.call(&f, vec![a], Vec::new())?;
    Ok(Some(displayable(vm, &r, &Value::None, true)?))
}

fn is_color(vm: &mut Vm, d: &Value) -> PyResult<bool> {
    let c = py::import_attr(vm, "renpy.color", "Color")?;
    Ok(c.obj().is_some_and(|c| ops::isinstance(vm, d, c)))
}

fn dynamic_image(vm: &mut Vm, d: &Value, scope: &Value) -> PyResult<Value> {
    let c = py::import_attr(vm, "renpy.display.image", "DynamicImage")?;
    vm.call(&c, vec![d.clone()], vec![("scope".into(), scope.clone())])
}

/// `displayable(d, scope=None)` / `displayable_or_none`.
pub fn displayable(vm: &mut Vm, d: &Value, scope: &Value, dynamic: bool) -> PyResult<Value> {
    if is_displayable(vm, d) {
        return Ok(d.clone());
    }
    if let Some(s) = d.as_str() {
        if s.is_empty() {
            return Err(py::error(
                vm,
                "An empty string cannot be used as a displayable.",
            ));
        }
        if release::at_least(vm, &[6, 99, 7])
            && s.contains('[')
            && dynamic
            && py::config_bool(vm, "dynamic_images").unwrap_or(true)
        {
            return dynamic_image(vm, d, scope);
        }
        if release::at_least(vm, &[7, 0]) {
            if let Some(rv) = lookup_displayable_prefix(vm, s)? {
                return Ok(rv);
            }
        }
        if s.starts_with('#') {
            let solid = store_attr(vm, "Solid")?;
            return vm.call(&solid, vec![d.clone()], Vec::new());
        }
        if s.contains('.') {
            let image = store_attr(vm, "Image")?;
            return vm.call(&image, vec![d.clone()], Vec::new());
        }
        let r = store_attr(vm, "ImageReference")?;
        let parts = s.split_whitespace().map(|p| vm.types.new_str(p)).collect();
        let t = vm.types.new_tuple(parts);
        return vm.call(&r, vec![t], Vec::new());
    }
    // "We assume the user knows what he's doing in this case."
    if release::before(vm, &[6, 99, 11]) {
        // The runtime's own parameterized displayables carry the 6.99.11
        // name (`_duplicate`) of the same protocol.
        if py::get_opt(vm, d, "parameterize")?.is_some()
            || py::get_opt(vm, d, "_duplicate")?.is_some()
        {
            return Ok(d.clone());
        }
    } else {
        if is_color(vm, d)? {
            let solid = store_attr(vm, "Solid")?;
            return vm.call(&solid, vec![d.clone()], Vec::new());
        }
        if matches!(d.payload(), Some(crysirror_python::Payload::List(_))) {
            return dynamic_image(vm, d, scope);
        }
        if py::get_opt(vm, d, "_duplicate")?.is_some() {
            return Ok(d.clone());
        }
    }
    if matches!(d, Value::Bool(_)) {
        return Ok(d.clone());
    }
    let r = ops::repr(vm, d)?;
    Err(py::error(vm, format!("Not a displayable: {r}")))
}

fn f_displayable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    displayable(vm, &d, &scope, true)
}

fn f_displayable_or_none(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    let dynamic = match arg(args, kw, 2, "dynamic") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    displayable(vm, &d, &scope, dynamic)
}

fn f_lookup_displayable_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    let s = py::string(vm, &d)?;
    Ok(lookup_displayable_prefix(vm, &s)?.unwrap_or(Value::None))
}

fn find_image(vm: &mut Vm, name: &Value, single: bool) -> PyResult<bool> {
    let exists = py::import_attr(vm, "renpy.exports", "image_exists")?;
    let r = vm.call(&exists, vec![name.clone()], Vec::new())?;
    if ops::truthy(vm, &r)? {
        return Ok(true);
    }
    let loadable = py::import_attr(vm, "renpy.loader", "loadable")?;
    let r = vm.call(
        &loadable,
        vec![name.clone()],
        vec![("directory".into(), vm.types.new_str("images"))],
    )?;
    if ops::truthy(vm, &r)? {
        return Ok(true);
    }
    let s = py::string(vm, name)?;
    if lookup_displayable_prefix(vm, &s)?.is_some() {
        return Ok(true);
    }
    if single {
        let cb = py::config(vm, "missing_image_callback")?;
        if !py::is_none(&cb) {
            let r = vm.call(&cb, vec![name.clone()], Vec::new())?;
            if ops::truthy(vm, &r)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn substitute(vm: &mut Vm, s: &Value, scope: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    let r = vm.call(
        &f,
        vec![s.clone()],
        vec![
            ("scope".into(), scope.clone()),
            ("force".into(), Value::Bool(true)),
            ("translate".into(), Value::Bool(false)),
        ],
    )?;
    ops::getitem(vm, &r, Value::Int(0))
}

/// `dynamic_image(d, scope=None, prefix=None, search=None)`.
fn f_dynamic_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let mut scope = arg_or(args, kw, 1, "scope", Value::None);
    let prefix = arg_or(args, kw, 2, "prefix", Value::None);
    let search = arg_or(args, kw, 3, "search", Value::None);
    let list = match d.payload() {
        Some(crysirror_python::Payload::List(l)) => l.borrow().clone(),
        _ => vec![d.clone()],
    };
    let single = list.len() == 1;
    for i in &list {
        let Some(text) = i.as_str() else { continue };
        if !py::is_none(&prefix) && text.contains("[prefix_") {
            let base = if ops::truthy(vm, &scope)? {
                vm.call(
                    &Value::Obj(vm.types.dict.clone()),
                    vec![scope.clone()],
                    Vec::new(),
                )?
            } else {
                vm.types.new_dict()
            };
            scope = base;
            let search_table = py::import_attr(vm, "renpy.styledata.stylesets", "prefix_search")?;
            let prefixes = ops::getitem(vm, &search_table, prefix.clone())?;
            for p in py::items(vm, &prefixes)? {
                py::dict_set(vm, &scope, "prefix_", p)?;
                let rv = substitute(vm, i, &scope)?;
                if find_image(vm, &rv, single)? {
                    return displayable(vm, &rv, &Value::None, true);
                }
                if !py::is_none(&search) {
                    ops::list_append(vm, &search, rv)?;
                }
            }
        } else {
            let rv = substitute(vm, i, &scope)?;
            if find_image(vm, &rv, single)? {
                return displayable(vm, &rv, &Value::None, true);
            }
            if !py::is_none(&search) {
                ops::list_append(vm, &search, rv)?;
            }
        }
    }
    let last = list.last().cloned().unwrap_or(Value::None);
    if !py::is_none(&last) && find_image(vm, &last, single)? {
        return displayable(vm, &last, &Value::None, false);
    }
    Ok(Value::None)
}

fn f_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&d) {
        return Ok(Value::None);
    }
    let d = displayable(vm, &d, &Value::None, true)?;
    if let Ok(p) = py::import_attr(vm, "renpy.display.predict", "displayable") {
        vm.call(&p, vec![d], Vec::new())?;
    }
    Ok(Value::None)
}

fn f_split_properties(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let props = args.first().cloned().unwrap_or(Value::None);
    let prefixes: Vec<String> = args[1..]
        .iter()
        .map(|p| p.as_str().unwrap_or("").to_owned())
        .collect();
    let out: Vec<Value> = prefixes.iter().map(|_| vm.types.new_dict()).collect();
    if !py::is_none(&props) && ops::truthy(vm, &props)? {
        for (k, v) in py::str_items(vm, &props)? {
            match prefixes.iter().position(|p| k.starts_with(p.as_str())) {
                Some(i) => py::dict_set(vm, &out[i], &k[prefixes[i].len()..], v)?,
                None => {
                    return Err(py::error(
                        vm,
                        format!("Property {k} begins with an unknown prefix."),
                    ));
                }
            }
        }
    }
    Ok(vm.types.new_list(out))
}

fn iterable(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    if v.as_str().is_some() {
        return Ok(false);
    }
    Ok(ops::iter(vm, v).is_ok())
}

fn f_to_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "value", Value::None);
    let copy = match arg(args, kw, 1, "copy") {
        Some(c) => ops::truthy(vm, c)?,
        None => false,
    };
    if let Some(crysirror_python::Payload::List(l)) = v.payload() {
        if !copy {
            return Ok(v.clone());
        }
        let items = l.borrow().clone();
        return Ok(vm.types.new_list(items));
    }
    if iterable(vm, &v)? {
        let items = ops::collect(vm, &v)?;
        return Ok(vm.types.new_list(items));
    }
    Ok(vm.types.new_list(vec![v]))
}

fn f_to_tuple(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "value", Value::None);
    if matches!(v.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return Ok(v);
    }
    if iterable(vm, &v)? {
        let items = ops::collect(vm, &v)?;
        return Ok(vm.types.new_tuple(items));
    }
    Ok(vm.types.new_tuple(vec![v]))
}

pub fn run_callbacks(
    vm: &mut Vm,
    cb: &Value,
    args: Vec<Value>,
    kw: Vec<(std::rc::Rc<str>, Value)>,
) -> PyResult<Value> {
    if py::is_none(cb) {
        return Ok(Value::None);
    }
    if let Some(items) = py::seq(cb) {
        let mut rv = Value::None;
        for i in items {
            let r = run_callbacks(vm, &i, args.clone(), kw.clone())?;
            if !py::is_none(&r) {
                rv = r;
            }
        }
        return Ok(rv);
    }
    vm.call(cb, args, kw)
}

fn f_run_callbacks(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cb = args.first().cloned().unwrap_or(Value::None);
    run_callbacks(
        vm,
        &cb,
        args.get(1..).map(|a| a.to_vec()).unwrap_or_default(),
        kw.to_vec(),
    )
}

/// 6.18 `color(c)`: a 4-tuple, None, or a 3/4/6/8-digit hex string
/// (optionally `#`-prefixed) parsed to an `(r, g, b, a)` tuple.
fn f_color_618(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = arg_or(args, kw, 0, "c", Value::None);
    if let Some(Payload::Tuple(t)) = c.payload() {
        if t.len() == 4 {
            return Ok(c.clone());
        }
    }
    if py::is_none(&c) {
        return Ok(c);
    }
    if let Some(s) = c.as_str() {
        let h = s.strip_prefix('#').unwrap_or(s);
        let digit = |vm: &mut Vm, t: &str| -> PyResult<i64> {
            i64::from_str_radix(t, 16).map_err(|_| {
                vm.value_error(format!("invalid literal for int() with base 16: '{t}'"))
            })
        };
        let cs: Vec<char> = h.chars().collect();
        let pair = |i: usize| cs[i..i + 2].iter().collect::<String>();
        let one = |i: usize| cs[i].to_string();
        let rgba: [i64; 4] = match cs.len() {
            6 | 8 => [
                digit(vm, &pair(0))?,
                digit(vm, &pair(2))?,
                digit(vm, &pair(4))?,
                if cs.len() == 8 { digit(vm, &pair(6))? } else { 255 },
            ],
            3 | 4 => [
                digit(vm, &one(0))? * 0x11,
                digit(vm, &one(1))? * 0x11,
                digit(vm, &one(2))? * 0x11,
                if cs.len() == 4 { digit(vm, &one(3))? * 0x11 } else { 255 },
            ],
            _ => {
                return Err(py::error(
                    vm,
                    "Color string must be 3, 4, 6, or 8 hex digits long.",
                ));
            }
        };
        return Ok(vm.types.new_tuple(rgba.iter().map(|v| Value::Int(*v)).collect()));
    }
    let r = ops::repr(vm, &c)?;
    Err(py::error(vm, format!("Not a color: {r}")))
}

/// `timed(name)`: a context manager printing the wall time of its body as
/// `"{name}: {ms:.2f} ms"` (upstream `contextlib.contextmanager`).
fn f_timed(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = arg_or(args, kw, 0, "name", Value::None);
    let cls = py::import_attr(vm, MODULE, "_Timed")?;
    vm.call(&cls, vec![name], Vec::new())
}

fn wall_time() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.)
}

fn timed_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "name", args.get(1).cloned().unwrap_or(Value::None))?;
    Ok(Value::None)
}

fn timed_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "start", py::f(wall_time()))?;
    Ok(Value::None)
}

fn timed_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    // An exception in the body propagates without the report, as with the
    // generator-based upstream manager.
    if args.get(1).is_some_and(|e| !py::is_none(e)) {
        return Ok(Value::Bool(false));
    }
    let start = py::get(vm, &args[0], "start")?;
    let start = py::f64_of(vm, &start)?;
    let name = py::get(vm, &args[0], "name")?;
    let name = py::string(vm, &name)?;
    let line = format!("{name}: {:.2} ms", (wall_time() - start) * 1000.0);
    let print = py::import_attr(vm, "builtins", "print")?;
    let s = vm.types.new_string(line);
    vm.call(&print, vec![s], Vec::new())?;
    Ok(Value::Bool(false))
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("displayable_or_none", f_displayable_or_none),
            ("displayable", f_displayable),
            ("predict", f_predict),
            ("timed", f_timed),
        ],
    );
    let set = |vm: &mut Vm, n: &'static str, f: crysirror_python::object::NativeFn| {
        let b = vm.types.new_builtin(n, f);
        crysirror_python::modules::set(vm, &m, n, b);
    };
    let timed = py::class(
        vm,
        MODULE,
        "_Timed",
        &[],
        &[
            ("__init__", timed_init),
            ("__enter__", timed_enter),
            ("__exit__", timed_exit),
        ],
        &[],
    )?;
    crysirror_python::modules::set(vm, &m, "_Timed", Value::Obj(timed));
    if release::before(vm, &[6, 99]) {
        set(vm, "color", f_color_618);
        return Ok(m);
    }
    set(vm, "dynamic_image", f_dynamic_image);
    set(vm, "split_properties", f_split_properties);
    let color = py::import_attr(vm, "renpy.color", "Color")?;
    crysirror_python::modules::set(vm, &m, "Color", color.clone());
    crysirror_python::modules::set(vm, &m, "color", color.clone());
    if release::at_least(vm, &[7, 0]) {
        set(vm, "lookup_displayable_prefix", f_lookup_displayable_prefix);
    }
    if release::state_since_pair(vm, 6, 1) {
        set(vm, "to_list", f_to_list);
        set(vm, "to_tuple", f_to_tuple);
        set(vm, "run_callbacks", f_run_callbacks);
    }
    if release::at_least(vm, &[8, 5]) {
        // `renpy.types` aliases: `type Displayable = Displayable` and
        // `type DisplayableLike = Displayable | str | list[str] | Color`.
        let disp = match py::import_attr(vm, "renpy.types", "Displayable") {
            Ok(v) => v,
            Err(_) => py::import_attr(vm, "renpy.display.displayable", "Displayable")?,
        };
        let like = match py::import_attr(vm, "renpy.types", "DisplayableLike") {
            Ok(v) => v,
            Err(_) => {
                let s = Value::Obj(vm.types.str_.clone());
                let l = Value::Obj(vm.types.list.clone());
                vm.types.new_tuple(vec![disp.clone(), s, l, color])
            }
        };
        crysirror_python::modules::set(vm, &m, "Displayable", disp);
        crysirror_python::modules::set(vm, &m, "DisplayableLike", like);
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
