// SPDX-License-Identifier: MPL-2.0
//! Names upstream Ren'Py modules bind through top-level imports
//! (`from renpy.display.render import render, Render`, `import
//! pygame_sdl2 as pygame`, ...), per release, from the generated
//! `import_aliases.rs`.
//!
//! The native modules here bind what their own code needs. Every other
//! imported name resolves on first use through a module-level `__getattr__`
//! (PEP 562) installed when the module loads, to the same object upstream
//! binds, and is then stored in the module. Lazy resolution keeps upstream's
//! import cycles out of module initialization.
use super::import_aliases::{ALIASES, TAGS};
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

/// Bit of the newest reference release not newer than the running one.
pub fn tag_bit(vm: &Vm) -> u16 {
    let v = super::state::try_rt(vm).map(|r| r.profile.version).unwrap_or([8, 6, 1, 0]);
    let cur = [v[0], v[1], v[2]];
    let mut best = 0;
    for (i, t) in TAGS.iter().enumerate() {
        if *t <= cur {
            best = i;
        }
    }
    1 << best
}

fn has_aliases(module: &str, bit: u16) -> bool {
    ALIASES.iter().any(|(m, _, _, _, mask)| *m == module && mask & bit != 0)
}

/// `module.__getattr__(name)`; the module name is the bound data.
fn module_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let module = py::string(vm, &args[0])?;
    let name = py::string(vm, &args[1])?;
    let bit = tag_bit(vm);
    let Some(&(_, _, source, attr, _)) =
        ALIASES.iter().find(|(m, n, _, _, mask)| *m == module && *n == name && mask & bit != 0)
    else {
        return Err(vm.attr_error(format!("module '{module}' has no attribute '{name}'")));
    };
    // A source this runtime does not provide leaves the name missing.
    let mut v = match py::import(vm, source) {
        Ok(v) => v,
        Err(e) if vm.is_exc(&e, "ImportError") => {
            return Err(vm.attr_error(format!(
                "module '{module}' has no attribute '{name}' (its source {source} is not available)"
            )));
        }
        Err(e) => return Err(e),
    };
    if !attr.is_empty() {
        v = ops::getattr(vm, &v, attr)?;
    }
    let m = py::import(vm, &module)?;
    py::set(vm, &m, &name, v.clone())?;
    Ok(v)
}

/// The VM's module-loaded hook.
pub fn on_module_loaded(vm: &mut Vm, name: &str, m: &ObjRef) -> PyResult<()> {
    if !name.starts_with("renpy") || !has_aliases(name, tag_bit(vm)) {
        return Ok(());
    }
    if crysirror_python::ops::get_raw_attr(m, "__getattr__").is_some() {
        return Ok(());
    }
    let data = vm.types.new_str(name);
    let f = py::closure(vm, "__getattr__", module_getattr, data);
    crysirror_python::modules::set(vm, m, "__getattr__", f);
    Ok(())
}

pub fn install(vm: &mut Vm) {
    vm.module_loaded = Some(on_module_loaded);
}
