// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.quaternion` (7.6/8.1+) and `renpy.pydict` (7.0+), after
//! Ren'Py's `quaternion.pyx` and `pydict.pyx` (MIT).
//!
//! `euler_slerp(complete, old, new)` interpolates two XYZ Euler rotations
//! (degrees) through quaternion slerp, as ATL's `orientation` does.
//! `pydict.DictItems(d)` snapshots a dict's items; `find_changes(old, new,
//! deleted)` returns, for every key whose value object differs between the
//! snapshots, the old value (or `deleted` for keys only in `new`), or None
//! when nothing changed. Values compare by identity, as upstream compares
//! object pointers.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::f64::consts::PI;

fn quat(x: f64, y: f64, z: f64) -> [f64; 4] {
    let (hx, hy, hz) = (x.to_radians() * 0.5, y.to_radians() * 0.5, z.to_radians() * 0.5);
    let (cx, sx, cy, sy, cz, sz) = (hx.cos(), hx.sin(), hy.cos(), hy.sin(), hz.cos(), hz.sin());
    [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ]
}

/// Python's float `%` (result has the divisor's sign).
fn pymod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && (r < 0.0) != (b < 0.0) { r + b } else { r }
}

/// None when the rotations coincide (upstream returns `new` itself).
pub fn euler_slerp(complete: f64, old: [f64; 3], new: [f64; 3]) -> Option<[f64; 3]> {
    let o = old.map(|v| pymod(v, 360.0));
    let mut n = new.map(|v| pymod(v, 360.0));
    for i in 0..3 {
        if n[i] - o[i] > 180.0 {
            n[i] -= 360.0;
        }
    }
    let oq = quat(o[0], o[1], o[2]);
    let nq = quat(n[0], n[1], n[2]);
    let dot = (oq[0] * nq[0] + oq[1] * nq[1] + oq[2] * nq[2] + oq[3] * nq[3]).clamp(-1.0, 1.0);
    let theta = dot.acos().abs();
    let st = theta.sin();
    if st == 0.0 {
        return None;
    }
    let c1 = (theta * (1.0 - complete)).sin() / st;
    let c2 = (theta * complete).sin() / st;
    let [qx, qy, qz, qw] = [0, 1, 2, 3].map(|i| c1 * oq[i] + c2 * nq[i]);
    let sinx_cosp = 2.0 * (qw * qx + qy * qz);
    let cosx_cosp = 1.0 - 2.0 * (qx * qx + qy * qy);
    let siny = 2.0 * (qw * qy - qz * qx);
    let sinz_cosp1 = 2.0 * (qx * qy - qw * qz);
    let cosz_cosp1 = 1.0 - 2.0 * (qx * qx + qz * qz);
    let sinz_cosp2 = 2.0 * (qw * qz + qx * qy);
    let cosz_cosp2 = 1.0 - 2.0 * (qy * qy + qz * qz);
    let (x, y, z) = if siny >= 1.0 {
        (0.0, PI / 2.0, sinz_cosp1.atan2(cosz_cosp1))
    } else if siny <= -1.0 {
        (0.0, -PI / 2.0, sinz_cosp1.atan2(cosz_cosp1))
    } else {
        (sinx_cosp.atan2(cosx_cosp), siny.asin(), sinz_cosp2.atan2(cosz_cosp2))
    };
    Some([x, y, z].map(|r| pymod(r.to_degrees(), 360.0)))
}

fn f_euler_slerp(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let complete = py::arg_or(args, kw, 0, "complete", Value::Float(0.0));
    let complete = py::f64_of(vm, &complete)?;
    let old = py::arg_or(args, kw, 1, "old", Value::None);
    let new = py::arg_or(args, kw, 2, "new", Value::None);
    if ops::eq(vm, &old, &new)? {
        return Ok(new);
    }
    let three = |vm: &mut Vm, v: &Value| -> PyResult<[f64; 3]> {
        let items = py::items(vm, v)?;
        if items.len() != 3 {
            return Err(vm.value_error(format!("expected 3 values to unpack, got {}", items.len())));
        }
        Ok([py::f64_of(vm, &items[0])?, py::f64_of(vm, &items[1])?, py::f64_of(vm, &items[2])?])
    };
    let (o, n) = (three(vm, &old)?, three(vm, &new)?);
    match euler_slerp(complete, o, n) {
        Some(r) => Ok(vm.types.new_tuple(r.map(Value::Float).to_vec())),
        None => Ok(new),
    }
}

fn init_quaternion(vm: &mut Vm) -> PyResult<ObjRef> {
    const M: &str = "renpy.display.quaternion";
    if !super::state::rt(vm).profile.since_pair(6, 1) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{M}'")));
    }
    Ok(crysirror_python::modules::module_with(vm, M, &[("euler_slerp", f_euler_slerp)]))
}

// renpy.pydict -------------------------------------------------------------------------------

const PYDICT: &str = "renpy.pydict";

fn di_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = py::arg_or(&args[1..], kw, 0, "d", Value::None);
    let items = vm.call_method(&d, "items", Vec::new())?;
    let l = py::items(vm, &items)?;
    let l = vm.types.new_list(l);
    py::set(vm, &args[0], "_items", l)?;
    Ok(Value::None)
}

fn di_as_dict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = py::get(vm, &args[0], "_items")?;
    let dict = Value::Obj(vm.types.dict.clone());
    vm.call(&dict, vec![l], Vec::new())
}

fn pairs(vm: &mut Vm, di: &Value) -> PyResult<Vec<(Value, Value)>> {
    let l = py::get(vm, di, "_items")?;
    let mut out = Vec::new();
    for p in py::items(vm, &l)? {
        let kv = py::items(vm, &p)?;
        out.push((kv[0].clone(), kv[1].clone()));
    }
    Ok(out)
}

fn f_find_changes(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let old = py::arg_or(args, kw, 0, "old", Value::None);
    let new = py::arg_or(args, kw, 1, "new", Value::None);
    let deleted = py::arg_or(args, kw, 2, "deleted", Value::None);
    let (old, new) = (pairs(vm, &old)?, pairs(vm, &new)?);
    let new_d = vm.types.new_dict();
    for (k, v) in &new {
        ops::setitem(vm, &new_d, k.clone(), v.clone())?;
    }
    let mut rv: Option<Value> = None;
    let mut put = |vm: &mut Vm, k: Value, v: Value| -> PyResult<()> {
        let d = rv.get_or_insert_with(|| vm.types.new_dict()).clone();
        ops::setitem(vm, &d, k, v)
    };
    let old_d = vm.types.new_dict();
    for (k, v) in &old {
        ops::setitem(vm, &old_d, k.clone(), v.clone())?;
        let nv = match new_d.obj() {
            Some(o) => ops::dict_get(vm, o, k)?,
            None => None,
        };
        match nv {
            Some(nv) if py::same(&nv, v) => {}
            _ => put(vm, k.clone(), v.clone())?,
        }
    }
    for (k, _) in &new {
        if !ops::contains(vm, &old_d, k)? {
            put(vm, k.clone(), deleted.clone())?;
        }
    }
    Ok(rv.unwrap_or(Value::None))
}

fn init_pydict(vm: &mut Vm) -> PyResult<ObjRef> {
    if !super::state::rt(vm).profile.at_least(7, 0) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{PYDICT}'")));
    }
    let m = crysirror_python::modules::module_with(vm, PYDICT, &[("find_changes", f_find_changes)]);
    let c = py::class(vm, PYDICT, "DictItems", &[], &[("__init__", di_init), ("as_dict", di_as_dict)], &[])?;
    crysirror_python::modules::set(vm, &m, "DictItems", Value::Obj(c));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module("renpy.display.quaternion", init_quaternion);
    vm.register_native_module(PYDICT, init_pydict);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slerp_endpoints_and_midpoint() {
        let a = euler_slerp(0.0, [0.0, 0.0, 0.0], [0.0, 0.0, 90.0]).unwrap();
        assert!(a.iter().all(|v| v.abs() < 1e-9 || (v - 360.0).abs() < 1e-9));
        let b = euler_slerp(1.0, [0.0, 0.0, 0.0], [0.0, 0.0, 90.0]).unwrap();
        assert!((b[2] - 90.0).abs() < 1e-9);
        let m = euler_slerp(0.5, [0.0, 0.0, 0.0], [0.0, 0.0, 90.0]).unwrap();
        assert!(euler_slerp(0.5, [10.0, 0.0, 0.0], [370.0, 0.0, 0.0]).is_none());
        assert!((m[2] - 45.0).abs() < 1e-9);
    }
}
