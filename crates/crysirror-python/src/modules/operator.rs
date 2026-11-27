// SPDX-License-Identifier: MPL-2.0
//! `operator`.

use super::{module_with, set};
use crate::ast::{BinOp, CmpOp, UnaryOp};
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

macro_rules! bin {
    ($($name:ident => $op:expr, $inplace:expr),* $(,)?) => {
        $(
            fn $name(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                crate::builtins::arg_count(vm, stringify!($name), a, 2, 2)?;
                ops::binary_op(vm, &a[0], &a[1], $op, $inplace, false)
            }
        )*
    };
}

bin! {
    add => BinOp::Add, false, sub => BinOp::Sub, false, mul => BinOp::Mult, false, truediv => BinOp::Div, false,
    floordiv => BinOp::FloorDiv, false, mod_ => BinOp::Mod, false, pow => BinOp::Pow, false, lshift => BinOp::LShift, false,
    rshift => BinOp::RShift, false, and_ => BinOp::BitAnd, false, or_ => BinOp::BitOr, false, xor => BinOp::BitXor, false,
    matmul => BinOp::MatMult, false,
    iadd => BinOp::Add, true, isub => BinOp::Sub, true, imul => BinOp::Mult, true, itruediv => BinOp::Div, true,
    ifloordiv => BinOp::FloorDiv, true, imod => BinOp::Mod, true, ior => BinOp::BitOr, true, iand => BinOp::BitAnd, true,
    ixor => BinOp::BitXor, true, ipow => BinOp::Pow, true, ilshift => BinOp::LShift, true, irshift => BinOp::RShift, true,
}

macro_rules! cmp {
    ($($name:ident => $op:expr),* $(,)?) => {
        $(
            fn $name(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                crate::builtins::arg_count(vm, stringify!($name), a, 2, 2)?;
                ops::compare(vm, &a[0], &a[1], $op)
            }
        )*
    };
}

cmp! { eq => CmpOp::Eq, ne => CmpOp::NotEq, lt => CmpOp::Lt, le => CmpOp::LtE, gt => CmpOp::Gt, ge => CmpOp::GtE, is_ => CmpOp::Is, is_not => CmpOp::IsNot }

fn neg(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::unary_op(vm, &a[0], UnaryOp::USub)
}
fn pos(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::unary_op(vm, &a[0], UnaryOp::UAdd)
}
fn invert(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::unary_op(vm, &a[0], UnaryOp::Invert)
}
fn not_(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(!ops::truthy(vm, &a[0])?))
}
fn truth(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(ops::truthy(vm, &a[0])?))
}
fn abs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = vm.builtins.dict_get_str_raw("abs").unwrap();
    vm.call(&f, a.to_vec(), Vec::new())
}
fn index(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(ops::index_of(vm, &a[0])?))
}
fn getitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::getitem(vm, &a[0], a[1].clone())
}
fn setitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::setitem(vm, &a[0], a[1].clone(), a[2].clone())?;
    Ok(Value::None)
}
fn delitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    ops::delitem(vm, &a[0], a[1].clone())?;
    Ok(Value::None)
}
fn contains(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(ops::contains(vm, &a[0], &a[1])?))
}
fn count_of(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut n = 0;
    for x in ops::collect(vm, &a[0])? {
        if ops::same_or_eq(vm, &x, &a[1])? {
            n += 1;
        }
    }
    Ok(Value::Int(n))
}
fn index_of(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    for (i, x) in ops::collect(vm, &a[0])?.iter().enumerate() {
        if ops::same_or_eq(vm, x, &a[1])? {
            return Ok(Value::Int(i as i64));
        }
    }
    Err(vm.value_error("sequence.index(x): x not in sequence"))
}
fn length_hint(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match ops::len(vm, &a[0]) {
        Ok(n) => Ok(Value::Int(n as i64)),
        Err(_) => Ok(a.get(1).cloned().unwrap_or(Value::Int(0))),
    }
}

fn store_new(vm: &mut Vm, cls: &ObjRef, data: Vec<Value>) -> Value {
    let o = new_obj(cls.clone(), Some(Dict::new()), Payload::None);
    let t = vm.types.new_tuple(data);
    ops::set_raw_attr(vm, &o, "_items", t);
    Value::Obj(o)
}

fn items_of(vm: &mut Vm, o: &Value) -> PyResult<Vec<Value>> {
    let t = ops::get_raw_attr(o.obj().unwrap(), "_items").unwrap_or(Value::None);
    ops::collect(vm, &t)
}

fn itemgetter_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(store_new(vm, &cls, a[1..].to_vec()))
}

fn itemgetter_call(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = items_of(vm, &a[0])?;
    if items.len() == 1 {
        return ops::getitem(vm, &a[1], items[0].clone());
    }
    let mut out = Vec::new();
    for k in items {
        out.push(ops::getitem(vm, &a[1], k)?);
    }
    Ok(vm.types.new_tuple(out))
}

fn attrgetter_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(store_new(vm, &cls, a[1..].to_vec()))
}

fn dotted(vm: &mut Vm, obj: &Value, name: &str) -> PyResult<Value> {
    let mut v = obj.clone();
    for part in name.split('.') {
        v = ops::getattr(vm, &v, part)?;
    }
    Ok(v)
}

fn attrgetter_call(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = items_of(vm, &a[0])?;
    let mut out = Vec::new();
    for n in &items {
        let s = crate::builtins::str_arg(vm, n, "attribute name")?;
        out.push(dotted(vm, &a[1], &s)?);
    }
    if out.len() == 1 {
        return Ok(out.pop().unwrap());
    }
    Ok(vm.types.new_tuple(out))
}

fn methodcaller_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let o = store_new(vm, &cls, a[1..].to_vec());
    let d = vm.types.new_dict_obj(Dict::new());
    for (n, v) in k {
        let key = vm.types.new_str(n);
        ops::dict_set(vm, &d, key, v.clone())?;
    }
    ops::set_raw_attr(vm, o.obj().unwrap(), "_kw", Value::Obj(d));
    Ok(o)
}

fn methodcaller_call(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = items_of(vm, &a[0])?;
    let name = crate::builtins::str_arg(vm, &items[0], "method name")?;
    let f = ops::getattr(vm, &a[1], &name)?;
    let kw = ops::get_raw_attr(a[0].obj().unwrap(), "_kw").unwrap_or(Value::None);
    let mut kwargs = Vec::new();
    for (key, v) in ops::mapping_items(vm, &kw)? {
        kwargs.push((std::rc::Rc::from(key.as_str().unwrap_or("")), v));
    }
    vm.call(&f, items[1..].to_vec(), kwargs)
}

fn callable_class(vm: &mut Vm, name: &str, new: NativeFn, call: NativeFn) -> Value {
    let c = vm.types.new_builtin_type(name, &vm.types.object.clone(), BaseKind::Object, false);
    let n = vm.types.new_builtin("__new__", new);
    let n = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(n)));
    vm.types.set_attr(&c, "__new__", n);
    vm.types.add_method(&c, "__call__", call);
    let mn = vm.types.new_str("operator");
    vm.types.set_attr(&c, "__module__", mn);
    Value::Obj(c)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let list: &[(&'static str, NativeFn)] = &[
        ("add", add), ("sub", sub), ("mul", mul), ("truediv", truediv), ("div", truediv), ("floordiv", floordiv), ("mod", mod_),
        ("pow", pow), ("lshift", lshift), ("rshift", rshift), ("and_", and_), ("or_", or_), ("xor", xor), ("matmul", matmul),
        ("iadd", iadd), ("isub", isub), ("imul", imul), ("itruediv", itruediv), ("ifloordiv", ifloordiv), ("imod", imod),
        ("ior", ior), ("iand", iand), ("ixor", ixor), ("ipow", ipow), ("ilshift", ilshift), ("irshift", irshift), ("iconcat", iadd),
        ("eq", eq), ("ne", ne), ("lt", lt), ("le", le), ("gt", gt), ("ge", ge), ("is_", is_), ("is_not", is_not),
        ("neg", neg), ("pos", pos), ("invert", invert), ("inv", invert), ("not_", not_), ("truth", truth), ("abs", abs),
        ("index", index), ("getitem", getitem), ("setitem", setitem), ("delitem", delitem), ("contains", contains),
        ("concat", add), ("countOf", count_of), ("indexOf", index_of), ("length_hint", length_hint),
        ("__add__", add), ("__sub__", sub), ("__mul__", mul), ("__eq__", eq), ("__getitem__", getitem), ("__contains__", contains),
        ("isCallable", is_callable), ("sequenceIncludes", contains),
    ];
    let m = module_with(vm, "operator", list);
    let ig = callable_class(vm, "itemgetter", itemgetter_new, itemgetter_call);
    set(vm, &m, "itemgetter", ig);
    let ag = callable_class(vm, "attrgetter", attrgetter_new, attrgetter_call);
    set(vm, &m, "attrgetter", ag);
    let mc = callable_class(vm, "methodcaller", methodcaller_new, methodcaller_call);
    set(vm, &m, "methodcaller", mc);
    Ok(m)
}

fn is_callable(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(crate::builtins::is_callable(vm, &a[0])))
}
