// SPDX-License-Identifier: MPL-2.0
//! Per-release module surface of Ren'Py's OpenGL driver modules
//! (`renpy.gl*`, `renpy.gl2.*`, `renpy.uguu*`) and the shared machinery
//! that builds them.
//!
//! `gl_api.json` is extracted from the upstream sources of every reference
//! release (Ren'Py 6.18 - 8.6, MIT): which modules and top-level names
//! each release has, and the literal module data (GL enum values resolved
//! from the Khronos registry, shader source strings, tables such as
//! `required_functions`, `BLACKLIST` and `TEXTURE_SCALING`). A module is
//! importable exactly in the releases that ship it; each name is bound
//! exactly in the releases that define it, from that release's literal or
//! from the native provider of its module.
//!
//! CrySirror owns rendering (retained wgpu layers, see `rt::render` and
//! `rt::backend`); there is no OpenGL context, Live2D Cubism Core or assimp
//! here. Names that need one are still bound, as classes and functions
//! that raise `RuntimeError("<module>.<name>: <reason>")` when called -
//! they never report success. Each module lists them explicitly; any other
//! name without a provider is an import error, so a pure name cannot fall
//! back to an unavailable stub by accident.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, object::BaseKind, ops};
use serde_json::Value as J;
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
};

pub const NO_GL: &str = "OpenGL is unavailable: CrySirror renders through its own wgpu pipeline";
pub const NO_CUBISM: &str = "the Live2D Cubism SDK is unavailable: CrySirror does not load Live2D Cubism Core";
pub const NO_ASSIMP: &str = "assimp is unavailable: CrySirror does not load 3D model files through assimp";

#[derive(Debug)]
pub enum Kind {
    Class,
    Function,
    Import(String, String),
    Module(String),
    Lit(J),
    Expr,
}

struct Variant {
    kind: Kind,
    tags: Vec<usize>,
}

struct ModuleSpec {
    present: Vec<usize>,
    names: Vec<(String, Vec<Variant>)>,
}

struct Api {
    tags: Vec<[u32; 4]>,
    modules: HashMap<String, ModuleSpec>,
}

fn api() -> &'static Api {
    static API: OnceLock<Api> = OnceLock::new();
    API.get_or_init(|| {
        let j: J = serde_json::from_str(include_str!("gl_api.json")).expect("gl_api.json");
        let tags = j["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let mut v = [0u32; 4];
                for (i, p) in t.as_str().unwrap().split('.').take(4).enumerate() {
                    v[i] = p.parse().unwrap_or(0);
                }
                v
            })
            .collect();
        let mut modules = HashMap::new();
        for (m, e) in j["modules"].as_object().unwrap() {
            let idx = |v: &J| -> Vec<usize> {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap() as usize)
                    .collect()
            };
            let mut names = Vec::new();
            for (n, vs) in e["names"].as_object().unwrap() {
                let mut variants = Vec::new();
                for v in vs.as_array().unwrap() {
                    let k = &v["k"];
                    let kind = match k[0].as_str().unwrap() {
                        "class" => Kind::Class,
                        "function" => Kind::Function,
                        "import" => Kind::Import(
                            k[1][0].as_str().unwrap().to_owned(),
                            k[1][1].as_str().unwrap().to_owned(),
                        ),
                        "module" => Kind::Module(k[1].as_str().unwrap().to_owned()),
                        "lit" => Kind::Lit(k[1].clone()),
                        _ => Kind::Expr,
                    };
                    variants.push(Variant {
                        kind,
                        tags: idx(&v["t"]),
                    });
                }
                names.push((n.clone(), variants));
            }
            modules.insert(
                m.clone(),
                ModuleSpec {
                    present: idx(&e["present"]),
                    names,
                },
            );
        }
        Api { tags, modules }
    })
}

/// The running release.
pub fn version(vm: &Vm) -> [u32; 4] {
    super::state::try_rt(vm)
        .map(|r| r.profile.version)
        .unwrap_or([8, 6, 1, 0])
}

/// Index of the newest reference release not newer than the running one.
pub fn tag_index(vm: &Vm) -> usize {
    let v = version(vm);
    let key = |t: &[u32; 4]| (t[0], t[1], t[2]);
    let tags = &api().tags;
    let mut best = 0;
    for (i, t) in tags.iter().enumerate() {
        if key(t) <= key(&v) {
            best = i;
        }
    }
    best
}

/// Whether the running release has `module`.
pub fn module_present(vm: &Vm, module: &str) -> bool {
    let t = tag_index(vm);
    api()
        .modules
        .get(module)
        .is_some_and(|m| m.present.contains(&t))
}

/// Whether the running release defines `module.name`.
pub fn has_name(vm: &Vm, module: &str, name: &str) -> bool {
    let t = tag_index(vm);
    api().modules.get(module).is_some_and(|m| {
        m.names
            .iter()
            .any(|(n, vs)| n == name && vs.iter().any(|v| v.tags.contains(&t)))
    })
}

/// The module being built, as seen by a provider.
pub struct Ctx {
    pub module: ObjRef,
    pub name: &'static str,
    pub version: [u32; 4],
    provider: Provider,
    unavailable: &'static [&'static str],
    reason: &'static str,
    pending: Vec<(String, Kind)>,
}

impl Ctx {
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.version[0], self.version[1]) >= (major, minor)
    }
    /// A change released in 8.`py3` and its Python 2 twin 7.`py2`.
    pub fn since_pair(&self, py2: u32, py3: u32) -> bool {
        self.at_least(8, py3) || (self.at_least(7, py2) && !self.at_least(8, 0))
    }
}

pub type Provider = fn(&mut Vm, &mut Ctx, &str) -> PyResult<Option<Value>>;

/// Interned `'static` names for natives created from table entries.
pub fn static_name(s: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut set = NAMES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap();
    if let Some(n) = set.get(s) {
        return n;
    }
    let n: &'static str = Box::leak(s.to_owned().into_boxed_str());
    set.insert(n);
    n
}

/// Raises the unavailable-host error. `data` is the message.
fn raise_unavailable(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let msg = args
        .first()
        .and_then(|m| m.as_str())
        .unwrap_or("unavailable")
        .to_owned();
    Err(vm.exc_msg("RuntimeError", msg))
}

/// The error a host-dependent operation raises.
pub fn unavailable(vm: &Vm, what: &str, reason: &str) -> crysirror_python::PyErr {
    vm.exc_msg("RuntimeError", format!("{what}: {reason}"))
}

/// A function that raises the unavailable-host error naming itself.
pub fn unavailable_fn(vm: &Vm, module: &str, name: &str, reason: &str) -> Value {
    let msg = vm.types.new_string(format!("{module}.{name}: {reason}"));
    py::closure(vm, static_name(name), raise_unavailable, msg)
}

/// A class whose construction raises the unavailable-host error.
pub fn unavailable_class(
    vm: &mut Vm,
    module: &str,
    name: &str,
    reason: &str,
    bases: &[Value],
) -> PyResult<Value> {
    let init = unavailable_fn(vm, module, name, reason);
    let c = py::class(vm, module, name, bases, &[], &[("__init__", init)])?;
    Ok(Value::Obj(c))
}

/// Methods that raise the unavailable-host error, bound onto `class`.
pub fn unavailable_methods(
    vm: &mut Vm,
    class: &Value,
    module: &str,
    class_name: &str,
    methods: &[&str],
    reason: &str,
) -> PyResult<()> {
    for m in methods {
        let f = unavailable_fn(vm, module, &format!("{class_name}.{m}"), reason);
        let b = vm.types.new_str(m);
        let cls = class.clone();
        // A closure is not a descriptor: wrap it so `self` is dropped.
        let sm = py::staticmethod(vm, f);
        let _ = b;
        py::set(vm, &cls, m, sm)?;
    }
    Ok(())
}

/// `class name(Exception)`.
pub fn exception_class(vm: &mut Vm, module: &str, name: &str) -> PyResult<Value> {
    let exc = py::import_attr(vm, "builtins", "Exception")?;
    Ok(Value::Obj(py::class(vm, module, name, &[exc], &[], &[])?))
}

/// A native-payload type (no instance dict) with `__new__` and methods.
pub fn native_type(
    vm: &mut Vm,
    name: &str,
    module: &str,
    base: Option<&ObjRef>,
    new: crysirror_python::object::NativeFn,
    methods: py::Methods,
) -> ObjRef {
    let base = base.cloned().unwrap_or_else(|| vm.types.object.clone());
    let c = vm
        .types
        .new_builtin_type(name, &base, BaseKind::Native, true);
    let n = vm.types.new_builtin("__new__", new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&c, "__new__", n);
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str(module);
    vm.types.set_attr(&c, "__module__", mn);
    c
}

/// Builds the Python value of a tagged literal from `gl_api.json`.
pub fn lit(vm: &mut Vm, ctx: &mut Ctx, j: &J) -> PyResult<Value> {
    Ok(match j {
        J::Null => Value::None,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => vm.types.new_str(s),
        J::Array(_) => return Err(vm.type_error("untagged literal")),
        J::Object(o) => {
            let (k, v) = o.iter().next().ok_or_else(|| vm.type_error("empty literal"))?;
            let items = |vm: &mut Vm, ctx: &mut Ctx, v: &J| -> PyResult<Vec<Value>> {
                let mut out = Vec::new();
                for x in v.as_array().into_iter().flatten() {
                    out.push(lit(vm, ctx, x)?);
                }
                Ok(out)
            };
            match k.as_str() {
                "f" => Value::Float(v.as_f64().unwrap_or(0.0)),
                "tuple" => {
                    let i = items(vm, ctx, v)?;
                    vm.types.new_tuple(i)
                }
                "list" => {
                    let i = items(vm, ctx, v)?;
                    vm.types.new_list(i)
                }
                "set" => {
                    let i = items(vm, ctx, v)?;
                    let l = vm.types.new_list(i);
                    let set = py::import_attr(vm, "builtins", "set")?;
                    vm.call(&set, vec![l], Vec::new())?
                }
                "dict" => {
                    let d = vm.types.new_dict();
                    for pair in v.as_array().into_iter().flatten() {
                        let key = lit(vm, ctx, &pair[0])?;
                        let val = lit(vm, ctx, &pair[1])?;
                        ops::setitem(vm, &d, key, val)?;
                    }
                    d
                }
                "bytes" => {
                    let b = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|x| x.as_u64().unwrap_or(0) as u8)
                        .collect();
                    vm.types.new_bytes(b)
                }
                "ref" => {
                    let n = v.as_str().unwrap_or("").to_owned();
                    get(vm, ctx, &n)?
                }
                _ => return Err(vm.type_error(format!("unknown literal tag {k}"))),
            }
        }
    })
}

/// `module.name`, building it first when the module has not bound it yet
/// (providers use this for names defined later in upstream order).
pub fn get(vm: &mut Vm, ctx: &mut Ctx, name: &str) -> PyResult<Value> {
    if let Some(v) = crysirror_python::ops::get_raw_attr(&ctx.module, name) {
        return Ok(v);
    }
    let Some(pos) = ctx.pending.iter().position(|(n, _)| n == name) else {
        return Err(vm.exc_msg(
            "ImportError",
            format!("{}: {name} is not defined in this release", ctx.name),
        ));
    };
    let (n, kind) = ctx.pending.remove(pos);
    let v = build(vm, ctx, &n, kind)?;
    crysirror_python::modules::set(vm, &ctx.module, &n, v.clone());
    Ok(v)
}

fn build(vm: &mut Vm, ctx: &mut Ctx, name: &str, kind: Kind) -> PyResult<Value> {
    let provider = ctx.provider;
    if let Some(v) = provider(vm, ctx, name)? {
        return Ok(v);
    }
    // `["*"]`: every class and function of the module needs the host.
    let fallback = ctx.unavailable.contains(&name) || ctx.unavailable == ["*"];
    match kind {
        Kind::Lit(j) => lit(vm, ctx, &j),
        Kind::Import(m, n) => py::import_attr(vm, &m, &n),
        Kind::Module(m) => py::import(vm, &m),
        Kind::Class if fallback => unavailable_class(vm, ctx.name, name, ctx.reason, &[]),
        Kind::Function if fallback => Ok(unavailable_fn(vm, ctx.name, name, ctx.reason)),
        other => Err(vm.exc_msg(
            "ImportError",
            format!("{}: no native for {name} ({other:?})", ctx.name),
        )),
    }
}

/// Builds `module` for the running release, or raises
/// `ModuleNotFoundError` when the release does not ship it.
pub fn init(
    vm: &mut Vm,
    module: &'static str,
    provider: Provider,
    unavailable: &'static [&'static str],
    reason: &'static str,
) -> PyResult<ObjRef> {
    let t = tag_index(vm);
    let spec = api().modules.get(module);
    let Some(spec) = spec.filter(|s| s.present.contains(&t)) else {
        return Err(vm.exc_msg(
            "ModuleNotFoundError",
            format!("No module named '{module}'"),
        ));
    };
    let m = vm.new_module(module);
    let mut pending = Vec::new();
    for (n, vs) in &spec.names {
        if let Some(v) = vs.iter().find(|v| v.tags.contains(&t)) {
            let kind = match &v.kind {
                Kind::Class => Kind::Class,
                Kind::Function => Kind::Function,
                Kind::Import(a, b) => Kind::Import(a.clone(), b.clone()),
                Kind::Module(a) => Kind::Module(a.clone()),
                Kind::Lit(j) => Kind::Lit(j.clone()),
                Kind::Expr => Kind::Expr,
            };
            pending.push((n.clone(), kind));
        }
    }
    // Classes and functions first, then data that may refer to them.
    let order = |k: &Kind| match k {
        Kind::Class => 0,
        Kind::Function => 1,
        Kind::Import(..) | Kind::Module(_) => 2,
        Kind::Lit(_) => 3,
        Kind::Expr => 4,
    };
    pending.sort_by(|a, b| order(&a.1).cmp(&order(&b.1)).then(a.0.cmp(&b.0)));
    let mut ctx = Ctx {
        module: m.clone(),
        name: module,
        version: version(vm),
        provider,
        unavailable,
        reason,
        pending,
    };
    while let Some((n, _)) = ctx.pending.first().map(|(n, k)| (n.clone(), order(k))) {
        get(vm, &mut ctx, &n)?;
    }
    Ok(m)
}

/// Registers an (empty) package module that exists only in its releases.
pub fn package(vm: &mut Vm, module: &'static str) -> PyResult<ObjRef> {
    fn none(_: &mut Vm, _: &mut Ctx, _: &str) -> PyResult<Option<Value>> {
        Ok(None)
    }
    let m = init(vm, module, none, &[], NO_GL)?;
    let p = vm.types.new_list(Vec::new());
    crysirror_python::modules::set(vm, &m, "__path__", p);
    Ok(m)
}

/// Positional argument `i` after `self`.
pub fn a<'a>(args: &'a [Value], kw: Kw<'a>, i: usize, name: &str) -> Option<&'a Value> {
    py::arg(args, kw, i, name)
}

/// The values of a Python sequence as f64.
pub fn floats(vm: &mut Vm, v: &Value) -> PyResult<Vec<f64>> {
    let items = py::items(vm, v)?;
    let mut out = Vec::with_capacity(items.len());
    for x in &items {
        out.push(py::f64_of(vm, x)?);
    }
    Ok(out)
}

/// Python-style `{:.Nf}` with the requested width and sign handling used
/// by upstream `__repr__`s: `{: >W.Pf}` (space for positive sign) and
/// `{:> W.Pf}` (right aligned, space sign).
pub fn fmt_space(x: f64, width: usize, prec: usize) -> String {
    let s = if x.is_sign_negative() && !(x == 0.0 && x.is_sign_positive()) && x != 0.0 {
        format!("{x:.prec$}")
    } else if x == 0.0 && x.is_sign_negative() {
        format!("-{:.prec$}", 0.0)
    } else {
        format!(" {x:.prec$}")
    };
    format!("{s:>width$}")
}

/// Python `{:.3f}`.
pub fn fmt_fixed(x: f64, prec: usize) -> String {
    format!("{x:.prec$}")
}
