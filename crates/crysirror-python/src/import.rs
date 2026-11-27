// SPDX-License-Identifier: MPL-2.0
//! `import` machinery: sys.modules, native modules registered by Rust code,
//! embedded Python-source modules and host-supplied sources.

use crate::builtins::{parse, Kw};
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use crate::Dialect;

fn resolve_name(vm: &mut Vm, name: &str, level: i64) -> PyResult<String> {
    if level <= 0 {
        return Ok(name.to_string());
    }
    let g = vm.frames.last().map(|f| f.globals.clone());
    let package = g.as_ref().and_then(|g| {
        g.dict_get_str_raw("__package__").and_then(|p| p.as_str().map(|s| s.to_string())).or_else(|| {
            let n = g.dict_get_str_raw("__name__")?.as_str()?.to_string();
            if g.dict_get_str_raw("__path__").is_some() {
                Some(n)
            } else {
                Some(n.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_default())
            }
        })
    });
    let mut base = package.unwrap_or_default();
    for _ in 1..level {
        base = base.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_default();
    }
    if base.is_empty() {
        return Err(vm.exc_msg("ImportError", "attempted relative import with no known parent package"));
    }
    Ok(if name.is_empty() { base } else { format!("{base}.{name}") })
}

pub fn get_module(vm: &Vm, name: &str) -> Option<Value> {
    vm.modules.dict_get_str_raw(name)
}

/// Imports `fullname` (and its parents), returning the leaf module.
pub fn import_module(vm: &mut Vm, fullname: &str) -> PyResult<Value> {
    if let Some(m) = get_module(vm, fullname) {
        return Ok(m);
    }
    let mut parent: Option<Value> = None;
    let mut cur = String::new();
    for part in fullname.split('.') {
        if !cur.is_empty() {
            cur.push('.');
        }
        cur.push_str(part);
        let m = match get_module(vm, &cur) {
            Some(m) => m,
            None => load_module(vm, &cur)?,
        };
        if let Some(Value::Obj(p)) = &parent {
            if p.attr_table().map(|d| d.borrow().get_str(part).is_none()).unwrap_or(false) {
                ops::set_raw_attr(vm, p, part, m.clone());
            }
        }
        parent = Some(m);
    }
    Ok(parent.unwrap())
}

fn load_module(vm: &mut Vm, name: &str) -> PyResult<Value> {
    if let Some(init) = vm.native_modules.iter().find(|(n, _)| *n == name).map(|(_, f)| *f) {
        let m = init(vm)?;
        let k = vm.types.new_str(name);
        vm.modules.dict_set_raw(&vm.types, k, Value::Obj(m.clone()));
        return Ok(Value::Obj(m));
    }
    let frozen = crate::modules::source_module(name).map(|s| (s.to_string(), format!("<frozen {name}>"), Dialect::Py3, crate::modules::is_package(name)));
    let src = match frozen {
        Some(f) => Some(f),
        None => match vm.host.find_module_source(name) {
            Some((s, f)) => {
                let d = vm.host.module_dialect(name).unwrap_or(vm.dialect_default);
                let p = vm.host.module_is_package(name);
                Some((s, f, d, p))
            }
            None => None,
        },
    };
    if let Some((src, filename, dialect, is_pkg)) = src {
        let m = vm.new_module(name);
        if is_pkg {
            let p = vm.types.new_list(Vec::new());
            ops::set_raw_attr(vm, &m, "__path__", p);
        }
        ops::set_raw_attr(vm, &m, "__builtins__", Value::Obj(vm.builtins_module.clone()));
        let pkg = if is_pkg { name.to_string() } else { name.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_default() };
        let pv = vm.types.new_string(pkg);
        ops::set_raw_attr(vm, &m, "__package__", pv);
        let fv = vm.types.new_string(filename.clone());
        ops::set_raw_attr(vm, &m, "__file__", fv);
        let code = vm.compile_exec(&src, &filename, dialect).map_err(|e| vm.syntax_error(e))?;
        let Payload::Module(_, g) = &m.payload else { unreachable!() };
        let g = g.clone();
        vm.native_depth += 1;
        let r = vm.run_code(code, g, None);
        vm.native_depth -= 1;
        if let Err(e) = r {
            let k = vm.types.new_str(name);
            let _ = ops::dict_del(vm, &vm.modules.clone(), k);
            return Err(e);
        }
        // The module may have replaced itself in sys.modules.
        return Ok(get_module(vm, name).unwrap_or(Value::Obj(m)));
    }
    Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{name}'")))
}

pub fn import_name(vm: &mut Vm, name: &Value, fromlist: &Value, level: &Value) -> PyResult<Value> {
    let name = name.as_str().unwrap_or("").to_string();
    let level = match level {
        Value::Int(i) => *i,
        _ => 0,
    };
    // A user-overridden __import__ in builtins takes precedence.
    if let Some(imp) = vm.builtins.dict_get_str_raw("__import__") {
        if !matches!(imp.payload(), Some(Payload::Builtin(b)) if b.name == "__import__") {
            let g = vm.frames.last().map(|f| Value::Obj(f.globals.clone())).unwrap_or(Value::None);
            let n = vm.types.new_string(name);
            return vm.call(&imp, vec![n, g.clone(), g, fromlist.clone(), Value::Int(level.max(0))], Vec::new());
        }
    }
    do_import(vm, &name, fromlist, level)
}

fn do_import(vm: &mut Vm, name: &str, fromlist: &Value, level: i64) -> PyResult<Value> {
    // Python 2 implicit relative imports (level -1) try the package first.
    let full = if level < 0 {
        match resolve_name(vm, name, 1) {
            Ok(rel) if !rel.is_empty() && import_module(vm, &rel).is_ok() => rel,
            _ => name.to_string(),
        }
    } else {
        resolve_name(vm, name, level)?
    };
    let leaf = import_module(vm, &full)?;
    let has_from = !fromlist.is_none() && ops::truthy(vm, fromlist)?;
    if has_from {
        return Ok(leaf);
    }
    if level > 0 || full != name {
        // `import .a.b` binds the package-relative top.
        let top = name.split('.').next().unwrap_or(name);
        let base = full.strip_suffix(&name[top.len()..]).unwrap_or(&full).to_string();
        return import_module(vm, &base);
    }
    let top = full.split('.').next().unwrap();
    import_module(vm, top)
}

pub fn import_from(vm: &mut Vm, m: &Value, name: &Value) -> PyResult<Value> {
    match ops::getattr_v(vm, m, name) {
        Ok(v) => Ok(v),
        Err(e) if vm.is_exc(&e, "AttributeError") => {
            let mname = ops::getattr_opt(vm, m, "__name__")?.and_then(|v| v.as_str().map(|s| s.to_string()));
            if let Some(mn) = mname {
                let sub = format!("{}.{}", mn, name.as_str().unwrap_or(""));
                if let Ok(v) = import_module(vm, &sub) {
                    return Ok(v);
                }
                return Err(vm.exc_msg("ImportError", format!("cannot import name '{}' from '{}'", name.as_str().unwrap_or(""), mn)));
            }
            Err(e)
        }
        Err(e) => Err(e),
    }
}

pub fn import_star(vm: &mut Vm, m: &Value) -> PyResult<()> {
    let names: Vec<Value> = match ops::getattr_opt(vm, m, "__all__")? {
        Some(all) => ops::collect(vm, &all)?,
        None => {
            let d = ops::getattr(vm, m, "__dict__")?;
            let keys = ops::mapping_items(vm, &d)?;
            keys.into_iter().map(|(k, _)| k).filter(|k| !k.as_str().unwrap_or("_").starts_with('_')).collect()
        }
    };
    let locals = {
        let f = vm.frames.last().unwrap();
        f.locals.clone().unwrap_or(Value::Obj(f.globals.clone()))
    };
    for n in names {
        let v = ops::getattr_v(vm, m, &n)?;
        ops::setitem(vm, &locals, n, v)?;
    }
    Ok(())
}

pub fn b_import(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let [name, _g, _l, fromlist, level] = parse(vm, "__import__", args, kw, ["name", "globals", "locals", "fromlist", "level"], 1)?;
    let name = crate::builtins::str_arg(vm, name.as_ref().unwrap(), "__import__() argument 1")?;
    let level = crate::builtins::opt_i64(vm, &level, 0)?;
    do_import(vm, &name, &fromlist.unwrap_or(Value::None), level)
}
