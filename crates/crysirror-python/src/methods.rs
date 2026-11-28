// SPDX-License-Identifier: MPL-2.0
//! Method tables and constructors of the builtin types (except text and
//! bytes, which live in `methods_str`).

use crate::ast::{BinOp, CmpOp};
use crate::builtins::{arg_count, parse, str_arg, Kw};
use crate::dict::Dict;
use crate::num::{self, Num};
use crate::object::*;
use crate::ops;
use crate::vm::{GenStep, PyResult, Resume, Vm};
use crate::Dialect;
use num_bigint::BigInt;
use num_traits::{Num as _, Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::rc::Rc;

fn m(vm: &Vm, t: &ObjRef, name: &'static str, f: NativeFn) {
    vm.types.add_method(t, name, f);
}

fn static_m(vm: &Vm, t: &ObjRef, name: &'static str, f: NativeFn) {
    let b = vm.types.new_builtin(name, f);
    let s = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(Dict::new()), Payload::StaticMethod(b)));
    vm.types.set_attr(t, name, s);
}

fn class_m(vm: &Vm, t: &ObjRef, name: &'static str, f: NativeFn) {
    let b = vm.types.new_builtin(name, f);
    let s = Value::Obj(new_obj(vm.types.classmethod.clone(), Some(Dict::new()), Payload::ClassMethod(b)));
    vm.types.set_attr(t, name, s);
}

macro_rules! binop_dunders {
    ($($fname:ident, $rname:ident => $op:expr),* $(,)?) => {
        $(
            fn $fname(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                arg_count(vm, stringify!($fname), a, 2, 2)?;
                Ok(ops::native_binary(vm, &a[0], &a[1], $op, false)?.unwrap_or_else(|| vm.types.not_implemented.clone()))
            }
            fn $rname(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                arg_count(vm, stringify!($rname), a, 2, 2)?;
                Ok(ops::native_binary(vm, &a[1], &a[0], $op, false)?.unwrap_or_else(|| vm.types.not_implemented.clone()))
            }
        )*
    };
}

binop_dunders! {
    d_add, d_radd => BinOp::Add,
    d_sub, d_rsub => BinOp::Sub,
    d_mul, d_rmul => BinOp::Mult,
    d_truediv, d_rtruediv => BinOp::Div,
    d_floordiv, d_rfloordiv => BinOp::FloorDiv,
    d_mod, d_rmod => BinOp::Mod,
    d_pow, d_rpow => BinOp::Pow,
    d_lshift, d_rlshift => BinOp::LShift,
    d_rshift, d_rrshift => BinOp::RShift,
    d_and, d_rand => BinOp::BitAnd,
    d_or, d_ror => BinOp::BitOr,
    d_xor, d_rxor => BinOp::BitXor,
}

macro_rules! cmp_dunders {
    ($($fname:ident => $op:expr),* $(,)?) => {
        $(
            fn $fname(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
                arg_count(vm, stringify!($fname), a, 2, 2)?;
                native_cmp(vm, &a[0], &a[1], $op)
            }
        )*
    };
}

cmp_dunders! {
    d_eq => CmpOp::Eq,
    d_ne => CmpOp::NotEq,
    d_lt => CmpOp::Lt,
    d_le => CmpOp::LtE,
    d_gt => CmpOp::Gt,
    d_ge => CmpOp::GtE,
}

/// Builtin comparison ignoring user overrides on the receiver (super() calls).
fn native_cmp(vm: &mut Vm, a: &Value, b: &Value, op: CmpOp) -> PyResult<Value> {
    match op {
        CmpOp::Eq | CmpOp::NotEq => {
            let pa = num::num_of(a).is_some() || matches!(a.payload(), Some(Payload::Str(_) | Payload::Bytes(_) | Payload::Tuple(_) | Payload::List(_) | Payload::Dict(_) | Payload::Set(_) | Payload::FrozenSet(_)));
            if !pa {
                let same = a.is(b);
                return Ok(if same { Value::Bool(op == CmpOp::Eq) } else { vm.types.not_implemented.clone() });
            }
            let ka = ops::base_kind(vm, a);
            let kb = ops::base_kind(vm, b);
            let comparable = |k: BaseKind| matches!(k, BaseKind::Int | BaseKind::Bool | BaseKind::Float | BaseKind::Complex);
            if ka != kb && !(comparable(ka) && comparable(kb)) && !(matches!(ka, BaseKind::Set | BaseKind::FrozenSet) && matches!(kb, BaseKind::Set | BaseKind::FrozenSet)) {
                return Ok(vm.types.not_implemented.clone());
            }
            let (a2, b2) = (strip(vm, a), strip(vm, b));
            ops::compare(vm, &a2, &b2, op)
        }
        _ => {
            let (a2, b2) = (strip(vm, a), strip(vm, b));
            match ops::native_order(vm, &a2, &b2)? {
                Some(_) => ops::compare(vm, &a2, &b2, op),
                None => Ok(vm.types.not_implemented.clone()),
            }
        }
    }
}

/// A plain builtin value equal to a subclass instance (drops user overrides).
fn strip(vm: &Vm, v: &Value) -> Value {
    let Value::Obj(o) = v else { return v.clone() };
    if !ops::is_user_type(&o.class()) {
        return v.clone();
    }
    match &o.payload {
        Payload::Str(s) => vm.types.new_str(s.as_str()),
        Payload::Int(b) => vm.types.new_bigint(b.clone()),
        Payload::Float(f) => Value::Float(*f),
        Payload::Tuple(t) => vm.types.new_tuple(t.to_vec()),
        Payload::List(l) => vm.types.new_list(l.borrow().clone()),
        Payload::Dict(d) => Value::Obj(vm.types.new_dict_obj(d.borrow().clone())),
        Payload::Set(d) => vm.types.new_set(d.borrow().clone()),
        Payload::FrozenSet(d) => vm.types.new_frozenset(d.clone()),
        _ => v.clone(),
    }
}

fn d_hash(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    if matches!(v.payload(), Some(Payload::None)) || matches!(a[0].payload(), Some(Payload::Native(_))) {
        return Ok(Value::Int((a[0].id() >> 4) ^ (a[0].id() << 60)));
    }
    Ok(Value::Int(ops::hash(vm, &v)?))
}

fn d_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = crate::format::native_repr(vm, &a[0])?;
    Ok(vm.types.new_string(s))
}

fn d_str(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    let s = match &v {
        Value::Float(f) => crate::format::str_of_float(vm, *f),
        _ => {
            if let Some(Payload::Str(s)) = v.payload() {
                s.as_str().to_string()
            } else {
                crate::format::native_repr(vm, &v)?
            }
        }
    };
    Ok(vm.types.new_string(s))
}

fn d_format(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__format__", a, 2, 2)?;
    let spec = str_arg(vm, &a[1], "format spec")?;
    let v = strip(vm, &a[0]);
    if matches!(v.payload(), Some(Payload::None)) || matches!(v.payload(), Some(Payload::Native(_))) {
        if !spec.is_empty() {
            return Err(vm.type_error(format!("unsupported format string passed to {}.__format__", ops::type_name(vm, &a[0]))));
        }
        let s = ops::str(vm, &a[0])?;
        return Ok(vm.types.new_string(s));
    }
    let s = crate::format::format_value(vm, &v, &spec)?;
    Ok(vm.types.new_string(s))
}

fn d_len(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    Ok(Value::Int(ops::len(vm, &v)? as i64))
}

fn d_bool(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    Ok(Value::Bool(ops::truthy(vm, &v)?))
}

fn d_contains(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__contains__", a, 2, 2)?;
    let v = strip(vm, &a[0]);
    Ok(Value::Bool(ops::contains(vm, &v, &a[1])?))
}

fn d_iter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let st = match a[0].payload() {
        Some(Payload::List(_) | Payload::Tuple(_)) => IterState::Seq { seq: a[0].clone(), idx: 0 },
        Some(Payload::Dict(d)) => IterState::DictKeys { dict: a[0].obj().unwrap().clone(), idx: 0, len: d.borrow().len() },
        Some(Payload::Iter(_) | Payload::Generator(_)) => return Ok(a[0].clone()),
        _ => {
            let v = strip(vm, &a[0]);
            return ops::iter(vm, &v);
        }
    };
    Ok(vm.types.new_iter(st))
}

fn d_getitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__getitem__", a, 2, 2)?;
    native_item(vm, &a[0], |vm, v| ops::getitem(vm, v, a[1].clone()))
}

fn d_setitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__setitem__", a, 3, 3)?;
    native_item(vm, &a[0], |vm, v| ops::setitem(vm, v, a[1].clone(), a[2].clone()).map(|_| Value::None))
}

fn d_delitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__delitem__", a, 2, 2)?;
    native_item(vm, &a[0], |vm, v| ops::delitem(vm, v, a[1].clone()).map(|_| Value::None))
}

/// Runs a container operation on the builtin payload of a (subclass)
/// instance without re-dispatching to its own overrides.
fn native_item(vm: &mut Vm, obj: &Value, f: impl FnOnce(&mut Vm, &Value) -> PyResult<Value>) -> PyResult<Value> {
    let Value::Obj(o) = obj else { return f(vm, obj) };
    if !ops::is_user_type(&o.class()) {
        return f(vm, obj);
    }
    // Temporarily view the instance through its builtin base.
    let base = builtin_base(&o.class());
    let saved = o.class();
    o.set_class(base);
    let r = f(vm, obj);
    o.set_class(saved);
    r
}

fn builtin_base(t: &ObjRef) -> ObjRef {
    for c in ops::mro(t) {
        if c.type_data().map(|d| d.builtin).unwrap_or(false) {
            return c;
        }
    }
    t.clone()
}

fn d_not_implemented(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.not_implemented.clone())
}

fn noop(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn add_numeric_dunders(vm: &Vm, t: &ObjRef) {
    let list: &[(&'static str, NativeFn)] = &[
        ("__add__", d_add), ("__radd__", d_radd), ("__sub__", d_sub), ("__rsub__", d_rsub), ("__mul__", d_mul), ("__rmul__", d_rmul),
        ("__truediv__", d_truediv), ("__rtruediv__", d_rtruediv), ("__floordiv__", d_floordiv), ("__rfloordiv__", d_rfloordiv),
        ("__mod__", d_mod), ("__rmod__", d_rmod), ("__pow__", d_pow), ("__rpow__", d_rpow), ("__lshift__", d_lshift), ("__rlshift__", d_rlshift),
        ("__rshift__", d_rshift), ("__rrshift__", d_rrshift), ("__and__", d_and), ("__rand__", d_rand), ("__or__", d_or), ("__ror__", d_ror),
        ("__xor__", d_xor), ("__rxor__", d_rxor), ("__div__", d_truediv), ("__rdiv__", d_rtruediv),
    ];
    for (n, f) in list {
        m(vm, t, n, *f);
    }
}

fn add_cmp_dunders(vm: &Vm, t: &ObjRef) {
    for (n, f) in [("__eq__", d_eq as NativeFn), ("__ne__", d_ne), ("__lt__", d_lt), ("__le__", d_le), ("__gt__", d_gt), ("__ge__", d_ge)] {
        m(vm, t, n, f);
    }
    m(vm, t, "__hash__", d_hash);
    m(vm, t, "__repr__", d_repr);
    m(vm, t, "__str__", d_str);
    m(vm, t, "__format__", d_format);
}

pub fn install(vm: &mut Vm) {
    let t = vm.types.object.clone();
    static_m(vm, &t, "__new__", crate::builtins::object_new);
    m(vm, &t, "__init__", object_init);
    let r = vm.types.new_builtin("object.__repr__", object_repr);
    vm.types.set_attr(&t, "__repr__", r);
    let s = vm.types.new_builtin("object.__str__", object_str);
    vm.types.set_attr(&t, "__str__", s);
    m(vm, &t, "__eq__", object_eq);
    m(vm, &t, "__ne__", object_ne);
    for n in ["__lt__", "__le__", "__gt__", "__ge__"] {
        m(vm, &t, n, d_not_implemented);
    }
    let h = vm.types.new_builtin("object.__hash__", object_hash);
    vm.types.set_attr(&t, "__hash__", h);
    m(vm, &t, "__setattr__", object_setattr);
    m(vm, &t, "__getattribute__", object_getattribute);
    m(vm, &t, "__delattr__", object_delattr);
    m(vm, &t, "__format__", d_format);
    m(vm, &t, "__dir__", object_dir);
    m(vm, &t, "__reduce_ex__", crate::modules::copyreg::object_reduce_ex);
    m(vm, &t, "__reduce__", crate::modules::copyreg::object_reduce);
    m(vm, &t, "__sizeof__", object_sizeof);
    class_m(vm, &t, "__init_subclass__", noop);
    class_m(vm, &t, "__subclasshook__", d_not_implemented);

    let t = vm.types.type_.clone();
    static_m(vm, &t, "__new__", crate::builtins::type_new);
    m(vm, &t, "__init__", noop);
    m(vm, &t, "__call__", type_call);
    let p = vm.types.new_builtin("type.__prepare__", type_prepare);
    let p = Value::Obj(new_obj(vm.types.classmethod.clone(), Some(Dict::new()), Payload::ClassMethod(p)));
    vm.types.set_attr(&t, "__prepare__", p);
    m(vm, &t, "mro", type_mro);
    m(vm, &t, "__subclasses__", type_subclasses);
    m(vm, &t, "__instancecheck__", type_instancecheck);
    m(vm, &t, "__subclasscheck__", type_subclasscheck);
    m(vm, &t, "__repr__", d_repr);

    let t = vm.types.int.clone();
    static_m(vm, &t, "__new__", int_new);
    add_numeric_dunders(vm, &t);
    add_cmp_dunders(vm, &t);
    m(vm, &t, "bit_length", int_bit_length);
    m(vm, &t, "bit_count", int_bit_count);
    m(vm, &t, "to_bytes", int_to_bytes);
    class_m(vm, &t, "from_bytes", int_from_bytes);
    m(vm, &t, "conjugate", num_identity);
    m(vm, &t, "__index__", int_int);
    m(vm, &t, "__int__", int_int);
    m(vm, &t, "__trunc__", int_int);
    m(vm, &t, "__floor__", int_int);
    m(vm, &t, "__ceil__", int_int);
    m(vm, &t, "__float__", num_float);
    m(vm, &t, "__bool__", d_bool);
    m(vm, &t, "__neg__", num_neg);
    m(vm, &t, "__pos__", num_identity);
    m(vm, &t, "__abs__", num_abs);
    m(vm, &t, "__invert__", int_invert);
    m(vm, &t, "__round__", num_round);
    m(vm, &t, "as_integer_ratio", int_ratio);
    m(vm, &t, "is_integer", int_is_integer);
    m(vm, &t, "__getnewargs__", num_getnewargs);

    let t = vm.types.bool_.clone();
    static_m(vm, &t, "__new__", bool_new);
    m(vm, &t, "__repr__", d_repr);
    m(vm, &t, "__str__", d_repr);
    for (n, f) in [("__and__", bool_and as NativeFn), ("__or__", bool_or), ("__xor__", bool_xor), ("__rand__", bool_and), ("__ror__", bool_or), ("__rxor__", bool_xor)] {
        m(vm, &t, n, f);
    }

    let t = vm.types.float.clone();
    static_m(vm, &t, "__new__", float_new);
    add_numeric_dunders(vm, &t);
    add_cmp_dunders(vm, &t);
    m(vm, &t, "is_integer", float_is_integer);
    m(vm, &t, "as_integer_ratio", float_ratio);
    m(vm, &t, "hex", float_hex);
    class_m(vm, &t, "fromhex", float_fromhex);
    m(vm, &t, "conjugate", num_identity);
    m(vm, &t, "__int__", float_int);
    m(vm, &t, "__trunc__", float_int);
    m(vm, &t, "__floor__", float_floor);
    m(vm, &t, "__ceil__", float_ceil);
    m(vm, &t, "__float__", num_float);
    m(vm, &t, "__bool__", d_bool);
    m(vm, &t, "__neg__", num_neg);
    m(vm, &t, "__pos__", num_identity);
    m(vm, &t, "__abs__", num_abs);
    m(vm, &t, "__round__", num_round);
    m(vm, &t, "__getnewargs__", num_getnewargs);

    let t = vm.types.complex.clone();
    static_m(vm, &t, "__new__", complex_new);
    add_numeric_dunders(vm, &t);
    add_cmp_dunders(vm, &t);
    m(vm, &t, "conjugate", complex_conjugate);
    m(vm, &t, "__abs__", num_abs);

    let t = vm.types.none_type.clone();
    static_m(vm, &t, "__new__", none_new);
    m(vm, &t, "__repr__", d_repr);
    m(vm, &t, "__bool__", d_bool);

    let t = vm.types.tuple.clone();
    static_m(vm, &t, "__new__", tuple_new);
    add_seq_dunders(vm, &t);
    m(vm, &t, "count", seq_count);
    m(vm, &t, "index", seq_index);
    m(vm, &t, "__hash__", d_hash);
    m(vm, &t, "__getnewargs__", tuple_getnewargs);

    let t = vm.types.list.clone();
    static_m(vm, &t, "__new__", list_new);
    m(vm, &t, "__init__", list_init);
    add_seq_dunders(vm, &t);
    m(vm, &t, "__setitem__", d_setitem);
    m(vm, &t, "__delitem__", d_delitem);
    m(vm, &t, "__iadd__", list_iadd);
    m(vm, &t, "__imul__", list_imul);
    m(vm, &t, "__reversed__", list_reversed);
    m(vm, &t, "append", list_append);
    m(vm, &t, "extend", list_extend);
    m(vm, &t, "insert", list_insert);
    m(vm, &t, "remove", list_remove);
    m(vm, &t, "pop", list_pop);
    m(vm, &t, "clear", list_clear);
    m(vm, &t, "index", seq_index);
    m(vm, &t, "count", seq_count);
    m(vm, &t, "sort", list_sort);
    m(vm, &t, "reverse", list_reverse);
    m(vm, &t, "copy", list_copy);
    vm.types.set_attr(&t, "__hash__", Value::None);

    let t = vm.types.dict.clone();
    static_m(vm, &t, "__new__", dict_new);
    m(vm, &t, "__init__", dict_init);
    add_cmp_dunders(vm, &t);
    vm.types.set_attr(&t, "__hash__", Value::None);
    m(vm, &t, "__getitem__", d_getitem);
    m(vm, &t, "__setitem__", d_setitem);
    m(vm, &t, "__delitem__", d_delitem);
    m(vm, &t, "__contains__", d_contains);
    m(vm, &t, "__len__", d_len);
    m(vm, &t, "__iter__", d_iter);
    m(vm, &t, "__or__", d_or);
    m(vm, &t, "__ior__", dict_ior);
    m(vm, &t, "__reversed__", dict_reversed);
    m(vm, &t, "keys", dict_keys);
    m(vm, &t, "values", dict_values);
    m(vm, &t, "items", dict_items);
    m(vm, &t, "iterkeys", dict_iterkeys);
    m(vm, &t, "itervalues", dict_itervalues);
    m(vm, &t, "iteritems", dict_iteritems);
    m(vm, &t, "viewkeys", dict_viewkeys);
    m(vm, &t, "viewvalues", dict_viewvalues);
    m(vm, &t, "viewitems", dict_viewitems);
    m(vm, &t, "has_key", dict_has_key);
    m(vm, &t, "get", dict_get);
    m(vm, &t, "setdefault", dict_setdefault);
    m(vm, &t, "pop", dict_pop);
    m(vm, &t, "popitem", dict_popitem);
    m(vm, &t, "update", dict_update);
    m(vm, &t, "clear", dict_clear);
    m(vm, &t, "copy", dict_copy);
    class_m(vm, &t, "fromkeys", dict_fromkeys);

    for t in [vm.types.set.clone(), vm.types.frozenset.clone()] {
        add_cmp_dunders(vm, &t);
        m(vm, &t, "__contains__", d_contains);
        m(vm, &t, "__len__", d_len);
        m(vm, &t, "__iter__", d_iter);
        m(vm, &t, "__or__", d_or);
        m(vm, &t, "__and__", d_and);
        m(vm, &t, "__sub__", d_sub);
        m(vm, &t, "__xor__", d_xor);
        m(vm, &t, "copy", set_copy);
        m(vm, &t, "union", set_union);
        m(vm, &t, "intersection", set_intersection);
        m(vm, &t, "difference", set_difference);
        m(vm, &t, "symmetric_difference", set_symdiff);
        m(vm, &t, "issubset", set_issubset);
        m(vm, &t, "issuperset", set_issuperset);
        m(vm, &t, "isdisjoint", set_isdisjoint);
    }
    let t = vm.types.set.clone();
    static_m(vm, &t, "__new__", set_new);
    m(vm, &t, "__init__", set_init);
    vm.types.set_attr(&t, "__hash__", Value::None);
    m(vm, &t, "add", set_add);
    m(vm, &t, "remove", set_remove);
    m(vm, &t, "discard", set_discard);
    m(vm, &t, "pop", set_pop);
    m(vm, &t, "clear", set_clear);
    m(vm, &t, "update", set_update);
    m(vm, &t, "intersection_update", set_intersection_update);
    m(vm, &t, "difference_update", set_difference_update);
    m(vm, &t, "symmetric_difference_update", set_symdiff_update);
    let t = vm.types.frozenset.clone();
    static_m(vm, &t, "__new__", frozenset_new);

    let t = vm.types.range.clone();
    static_m(vm, &t, "__new__", range_new);
    m(vm, &t, "__len__", d_len);
    m(vm, &t, "__iter__", d_iter);
    m(vm, &t, "__contains__", d_contains);
    m(vm, &t, "__getitem__", d_getitem);
    m(vm, &t, "__reversed__", range_reversed);
    m(vm, &t, "count", seq_count);
    m(vm, &t, "index", seq_index);
    add_cmp_dunders(vm, &t);

    let t = vm.types.slice.clone();
    static_m(vm, &t, "__new__", slice_new);
    m(vm, &t, "indices", slice_indices_m);

    let t = vm.types.property.clone();
    static_m(vm, &t, "__new__", property_new);
    m(vm, &t, "__init__", noop);
    m(vm, &t, "getter", property_getter);
    m(vm, &t, "setter", property_setter);
    m(vm, &t, "deleter", property_deleter);
    m(vm, &t, "__get__", property_get);
    m(vm, &t, "__set__", property_set);

    let t = vm.types.staticmethod.clone();
    static_m(vm, &t, "__new__", staticmethod_new);
    m(vm, &t, "__init__", noop);
    m(vm, &t, "__get__", staticmethod_get);
    let t = vm.types.classmethod.clone();
    static_m(vm, &t, "__new__", classmethod_new);
    m(vm, &t, "__init__", noop);
    m(vm, &t, "__get__", classmethod_get);

    let t = vm.types.super_.clone();
    static_m(vm, &t, "__new__", super_new);
    m(vm, &t, "__init__", noop);

    let t = vm.types.function.clone();
    m(vm, &t, "__get__", function_get);
    m(vm, &t, "__call__", callable_call);
    let t = vm.types.method.clone();
    m(vm, &t, "__call__", callable_call);
    let t = vm.types.builtin_function.clone();
    m(vm, &t, "__call__", callable_call);

    let t = vm.types.generator.clone();
    m(vm, &t, "__iter__", identity);
    m(vm, &t, "__next__", gen_next);
    m(vm, &t, "next", gen_next);
    m(vm, &t, "send", gen_send);
    m(vm, &t, "throw", gen_throw);
    m(vm, &t, "close", gen_close);

    let t = vm.types.iterator.clone();
    m(vm, &t, "__iter__", identity);
    m(vm, &t, "__next__", iter_next_m);
    m(vm, &t, "next", iter_next_m);
    m(vm, &t, "__length_hint__", iter_length_hint);

    for t in [vm.types.dict_keys.clone(), vm.types.dict_values.clone(), vm.types.dict_items.clone()] {
        m(vm, &t, "__iter__", d_iter);
        m(vm, &t, "__len__", d_len);
        m(vm, &t, "__contains__", d_contains);
        m(vm, &t, "isdisjoint", set_isdisjoint);
    }

    let t = vm.types.exc("BaseException");
    static_m(vm, &t, "__new__", crate::builtins::object_new);
    m(vm, &t, "__init__", exc_init);
    m(vm, &t, "__str__", exc_str);
    m(vm, &t, "__repr__", d_repr);
    m(vm, &t, "with_traceback", exc_with_traceback);
    m(vm, &t, "__reduce__", exc_reduce);
    m(vm, &t, "add_note", noop);
    let t = vm.types.exc("OSError");
    m(vm, &t, "__init__", oserror_init);
    let t = vm.types.exc("SyntaxError");
    m(vm, &t, "__init__", syntaxerror_init);
    let t = vm.types.exc("ImportError");
    m(vm, &t, "__init__", importerror_init);
    // Raised natively without `__init__`: the attributes default to None.
    for n in ["msg", "name", "path", "name_from"] {
        vm.types.set_attr(&t, n, Value::None);
    }

    let t = vm.types.module.clone();
    static_m(vm, &t, "__new__", module_new);
    m(vm, &t, "__init__", module_init);
    m(vm, &t, "__repr__", d_repr);

    crate::modules::weakref::install_type(vm);

    let t = vm.types.cell.clone();
    m(vm, &t, "__repr__", d_repr);

    // `obj.__dict__` proxy.
    let mp = vm.types.new_builtin_type("mappingproxy", &vm.types.object.clone(), BaseKind::Native, true);
    m(vm, &mp, "__getitem__", proxy_getitem);
    m(vm, &mp, "__setitem__", proxy_setitem);
    m(vm, &mp, "__delitem__", proxy_delitem);
    m(vm, &mp, "__contains__", proxy_contains);
    m(vm, &mp, "__iter__", proxy_iter);
    m(vm, &mp, "__len__", proxy_len);
    m(vm, &mp, "__repr__", proxy_repr);
    m(vm, &mp, "__eq__", proxy_eq);
    m(vm, &mp, "keys", proxy_keys);
    m(vm, &mp, "values", proxy_values);
    m(vm, &mp, "items", proxy_items);
    m(vm, &mp, "iteritems", proxy_items);
    m(vm, &mp, "iterkeys", proxy_keys);
    m(vm, &mp, "get", proxy_get);
    m(vm, &mp, "setdefault", proxy_setdefault);
    m(vm, &mp, "pop", proxy_pop);
    m(vm, &mp, "update", proxy_update);
    m(vm, &mp, "copy", proxy_copy);
    m(vm, &mp, "clear", proxy_clear);
    m(vm, &mp, "has_key", proxy_contains);
    vm.classes.mappingproxy = Some(mp);

    crate::methods_str::install(vm);
}

fn identity(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(a[0].clone())
}

// ---- object / type ----

fn is_builtin_fn(v: &Option<Value>, f: NativeFn) -> bool {
    let inner = match v.as_ref().and_then(|v| v.payload()) {
        Some(Payload::StaticMethod(x)) => x.clone(),
        Some(_) => v.clone().unwrap(),
        None => return false,
    };
    matches!(inner.payload(), Some(Payload::Builtin(b)) if b.func as usize == f as usize)
}

/// CPython `object_init`: excess arguments are an error when `__init__`
/// is overridden or `__new__` is `object.__new__`.
fn object_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    if a.len() > 1 || !k.is_empty() {
        let t = ops::type_of(vm, &a[0]);
        let init_overridden = !is_builtin_fn(&ops::lookup_type(&t, "__init__"), object_init);
        let new_is_object = is_builtin_fn(&ops::lookup_type(&t, "__new__"), crate::builtins::object_new);
        if init_overridden || new_is_object {
            if ops::vm_dialect(vm) == Dialect::Py3 || init_overridden {
                return Err(vm.type_error(if init_overridden {
                    "object.__init__() takes exactly one argument (the instance to initialize)".to_string()
                } else {
                    format!("{}() takes no arguments", ops::type_name(vm, &a[0]))
                }));
            }
        }
    }
    Ok(Value::None)
}

fn object_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = &a[0];
    let t = ops::type_of(vm, v);
    let module = ops::lookup_type(&t, "__module__").and_then(|m| m.as_str().map(|s| s.to_string()));
    let qual = t.type_data().map(|d| d.qualname.borrow().to_string()).unwrap_or_default();
    let s = match module {
        Some(m) if m != "builtins" => format!("<{}.{} object at 0x{:x}>", m, qual, v.id()),
        _ => format!("<{} object at 0x{:x}>", qual, v.id()),
    };
    Ok(vm.types.new_string(s))
}

fn object_str(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = ops::repr(vm, &a[0])?;
    Ok(vm.types.new_string(s))
}

fn object_eq(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__eq__", a, 2, 2)?;
    Ok(if a[0].is(&a[1]) { Value::Bool(true) } else { vm.types.not_implemented.clone() })
}

fn object_ne(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__ne__", a, 2, 2)?;
    let r = ops::compare(vm, &a[0], &a[1], CmpOp::Eq)?;
    if ops::is_not_implemented(&r) {
        return Ok(r);
    }
    Ok(Value::Bool(!ops::truthy(vm, &r)?))
}

fn object_hash(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int((a[0].id() >> 4) ^ (a[0].id() << 60)))
}

fn object_setattr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__setattr__", a, 3, 3)?;
    ops::generic_setattr(vm, &a[0], &a[1], a[2].clone())?;
    Ok(Value::None)
}

fn object_getattribute(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__getattribute__", a, 2, 2)?;
    match ops::generic_getattr(vm, &a[0], &a[1])? {
        Some(v) => Ok(v),
        None => Err(vm.attr_error(format!("'{}' object has no attribute '{}'", ops::type_name(vm, &a[0]), a[1].as_str().unwrap_or("")))),
    }
}

fn object_delattr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__delattr__", a, 2, 2)?;
    ops::generic_delattr(vm, &a[0], &a[1])?;
    Ok(Value::None)
}

fn object_dir(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = vm.builtins.dict_get_str_raw("dir").unwrap();
    vm.call(&d, vec![a[0].clone()], Vec::new())
}

fn object_sizeof(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(32))
}

fn type_call(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().ok_or_else(|| vm.type_error("descriptor '__call__' requires a type"))?;
    ops::type_call_default(vm, &cls, a[1..].to_vec(), k.to_vec())
}

fn type_prepare(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.new_dict())
}

fn type_mro(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let t = a[0].obj().cloned().unwrap();
    let items = ops::mro(&t).into_iter().map(Value::Obj).collect();
    Ok(vm.types.new_list(items))
}

fn type_subclasses(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.new_list(Vec::new()))
}

fn type_instancecheck(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__instancecheck__", a, 2, 2)?;
    Ok(Value::Bool(ops::isinstance(vm, &a[1], a[0].obj().unwrap())))
}

fn type_subclasscheck(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "__subclasscheck__", a, 2, 2)?;
    let Some(c) = a[1].obj().filter(|o| o.type_data().is_some()) else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(ops::is_subclass(c, a[0].obj().unwrap())))
}

// ---- numbers ----

/// Wraps a builtin value as an instance of a subclass `cls` when needed.
fn subclass_instance(vm: &Vm, cls: &ObjRef, base: &ObjRef, v: Value) -> Value {
    if Rc::ptr_eq(cls, base) {
        return v;
    }
    let payload = match &v {
        Value::Int(i) => Payload::Int(BigInt::from(*i)),
        Value::Bool(b) => Payload::Int(BigInt::from(*b as i64)),
        Value::Float(f) => Payload::Float(*f),
        Value::Obj(o) => match &o.payload {
            Payload::Int(b) => Payload::Int(b.clone()),
            Payload::Str(s) => Payload::Str(PyStr::new(s.as_str())),
            Payload::Bytes(b) => Payload::Bytes(b.clone()),
            Payload::Tuple(t) => Payload::Tuple(t.clone()),
            Payload::FrozenSet(d) => Payload::FrozenSet(d.clone()),
            Payload::Complex(r, i) => Payload::Complex(*r, *i),
            _ => return v,
        },
        Value::None => return v,
    };
    let _ = vm;
    Value::Obj(new_obj(cls.clone(), Some(Dict::new()), payload))
}

pub fn parse_int_str(vm: &Vm, s: &str, base: u32) -> PyResult<BigInt> {
    let orig = s;
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let mut base = base;
    let mut body = t;
    let lower = t.to_ascii_lowercase();
    if base == 0 || base == 16 || base == 8 || base == 2 {
        let pre = if lower.starts_with("0x") { 16 } else if lower.starts_with("0o") { 8 } else if lower.starts_with("0b") { 2 } else { 0 };
        if pre != 0 && (base == 0 || base == pre) {
            base = pre;
            body = &t[2..];
        } else if base == 0 {
            base = 10;
            if body.len() > 1 && body.starts_with('0') && body.chars().any(|c| c != '0' && c != '_') {
                if crate::ops::vm_dialect(vm) == Dialect::Py2 {
                    base = 8;
                } else {
                    return Err(vm.value_error(format!("invalid literal for int() with base 0: {}", crate::format::quote_str(orig))));
                }
            }
        }
    }
    let body = body.strip_suffix(['l', 'L']).filter(|_| crate::ops::vm_dialect(vm) == Dialect::Py2).unwrap_or(body);
    let clean: String = body.chars().filter(|c| *c != '_').map(|c| c.to_digit(36).map(|d| char::from_digit(d, 36).unwrap()).unwrap_or(c)).collect();
    if clean.is_empty() || body.starts_with('_') || body.ends_with('_') || body.contains("__") {
        return Err(vm.value_error(format!("invalid literal for int() with base {}: {}", base, crate::format::quote_str(orig))));
    }
    let v = BigInt::from_str_radix(&clean, base).map_err(|_| vm.value_error(format!("invalid literal for int() with base {}: {}", base, crate::format::quote_str(orig))))?;
    Ok(if neg { -v } else { v })
}

fn int_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let [x, base] = parse(vm, "int", &a[1..], k, ["x", "base"], 0)?;
    let int_t = vm.types.int.clone();
    let v = match (x, base) {
        (None, _) => Value::Int(0),
        (Some(x), Some(b)) => {
            let b = ops::index_of(vm, &b)?;
            if b != 0 && !(2..=36).contains(&b) {
                return Err(vm.value_error("int() base must be >= 2 and <= 36, or 0"));
            }
            let s = match x.payload() {
                Some(Payload::Str(s)) => s.as_str().to_string(),
                Some(Payload::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
                _ => return Err(vm.type_error("int() can't convert non-string with explicit base")),
            };
            crate::builtins::big_to_value(vm, parse_int_str(vm, &s, b as u32)?)
        }
        (Some(x), None) => to_int(vm, &x)?,
    };
    Ok(subclass_instance(vm, &cls, &int_t, v))
}

pub fn to_int(vm: &mut Vm, x: &Value) -> PyResult<Value> {
    match num::num_of(x) {
        Some(Num::I(i)) => return Ok(Value::Int(i)),
        Some(Num::B(b)) => return Ok(vm.types.new_bigint(b)),
        Some(Num::F(f)) => {
            if f.is_nan() {
                return Err(vm.value_error("cannot convert float NaN to integer"));
            }
            if f.is_infinite() {
                return Err(vm.exc_msg("OverflowError", "cannot convert float infinity to integer"));
            }
            let b: BigInt = num_traits::FromPrimitive::from_f64(f.trunc()).unwrap_or_default();
            return Ok(vm.types.new_bigint(b));
        }
        _ => {}
    }
    match x.payload() {
        Some(Payload::Str(s)) if !ops::is_user_type(&ops::type_of(vm, x)) || ops::lookup_user(&ops::type_of(vm, x), "__int__").is_none() => {
            let st = s.as_str().to_string();
            return Ok(crate::builtins::big_to_value(vm, parse_int_str(vm, &st, 10)?));
        }
        Some(Payload::Bytes(b)) => {
            let st = String::from_utf8_lossy(b).to_string();
            return Ok(crate::builtins::big_to_value(vm, parse_int_str(vm, &st, 10)?));
        }
        _ => {}
    }
    for name in ["__int__", "__index__", "__trunc__"] {
        if let Some(r) = ops::call_special(vm, x, name, Vec::new())? {
            return match num::num_of(&r) {
                Some(Num::I(i)) => Ok(Value::Int(i)),
                Some(Num::B(b)) => Ok(vm.types.new_bigint(b)),
                _ => to_int(vm, &r),
            };
        }
    }
    Err(vm.type_error(format!("int() argument must be a string, a bytes-like object or a real number, not '{}'", ops::type_name(vm, x))))
}

fn bool_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match a.get(1) {
        None => Ok(Value::Bool(false)),
        Some(v) => Ok(Value::Bool(ops::truthy(vm, v)?)),
    }
}

fn bool_and(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let (Value::Bool(x), Value::Bool(y)) = (&a[0], &a[1]) {
        return Ok(Value::Bool(*x && *y));
    }
    d_and(vm, a, &[])
}
fn bool_or(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let (Value::Bool(x), Value::Bool(y)) = (&a[0], &a[1]) {
        return Ok(Value::Bool(*x || *y));
    }
    d_or(vm, a, &[])
}
fn bool_xor(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let (Value::Bool(x), Value::Bool(y)) = (&a[0], &a[1]) {
        return Ok(Value::Bool(*x ^ *y));
    }
    d_xor(vm, a, &[])
}

fn int_bit_length(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = ops::to_bigint(&a[0]).unwrap_or_default();
    Ok(Value::Int(b.abs().bits() as i64))
}

fn int_bit_count(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let b = ops::to_bigint(&a[0]).unwrap_or_default();
    Ok(Value::Int(b.abs().to_u64_digits().1.iter().map(|d| d.count_ones() as i64).sum()))
}

fn int_to_bytes(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [length, order, signed] = parse(vm, "to_bytes", &a[1..], k, ["length", "byteorder", "signed"], 0)?;
    let n = crate::builtins::opt_i64(vm, &length, 1)? as usize;
    let order = match order {
        Some(o) => str_arg(vm, &o, "byteorder")?,
        None => "big".into(),
    };
    let signed = match signed {
        Some(s) => ops::truthy(vm, &s)?,
        None => false,
    };
    let b = ops::to_bigint(&a[0]).unwrap_or_default();
    if b.is_negative() && !signed {
        return Err(vm.exc_msg("OverflowError", "can't convert negative int to unsigned"));
    }
    let mut bytes = if signed { b.to_signed_bytes_le() } else { b.to_bytes_le().1 };
    if b.is_zero() {
        bytes = vec![0];
    }
    if bytes.len() > n {
        if !(bytes.len() == n + 1 && !signed && bytes[n] == 0) {
            return Err(vm.exc_msg("OverflowError", "int too big to convert"));
        }
        bytes.truncate(n);
    }
    let pad = if b.is_negative() { 0xff } else { 0 };
    bytes.resize(n, pad);
    if order == "big" {
        bytes.reverse();
    }
    Ok(vm.types.new_bytes(bytes))
}

fn int_from_bytes(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [bytes, order, signed] = parse(vm, "from_bytes", &a[1..], k, ["bytes", "byteorder", "signed"], 1)?;
    let mut b = match ops::bytes_of(bytes.as_ref().unwrap()) {
        Some(b) => b,
        None => ops::collect(vm, bytes.as_ref().unwrap())?.iter().map(|v| match v {
            Value::Int(i) => *i as u8,
            _ => 0,
        }).collect(),
    };
    let order = match order {
        Some(o) => str_arg(vm, &o, "byteorder")?,
        None => "big".into(),
    };
    if order == "big" {
        b.reverse();
    }
    let signed = match signed {
        Some(s) => ops::truthy(vm, &s)?,
        None => false,
    };
    let v = if signed { BigInt::from_signed_bytes_le(&b) } else { BigInt::from_bytes_le(num_bigint::Sign::Plus, &b) };
    Ok(crate::builtins::big_to_value(vm, v))
}

fn num_identity(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(strip(vm, &a[0]))
}

fn int_int(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match num::num_of(&a[0]) {
        Some(Num::I(i)) => Ok(Value::Int(i)),
        Some(Num::B(b)) => Ok(vm.types.new_bigint(b)),
        _ => Err(vm.type_error("int expected")),
    }
}

fn num_float(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Float(ops::to_f64(vm, &a[0])?))
}

fn num_neg(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = num::num_of(&a[0]).ok_or_else(|| vm.type_error("number expected"))?;
    num::num_unary(vm, n, crate::ast::UnaryOp::USub)
}

fn int_invert(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = num::num_of(&a[0]).ok_or_else(|| vm.type_error("number expected"))?;
    num::num_unary(vm, n, crate::ast::UnaryOp::Invert)
}

fn num_abs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = vm.builtins.dict_get_str_raw("abs").unwrap();
    let v = strip(vm, &a[0]);
    vm.call(&f, vec![v], Vec::new())
}

fn num_round(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = vm.builtins.dict_get_str_raw("round").unwrap();
    let mut args = vec![strip(vm, &a[0])];
    args.extend(a[1..].iter().cloned());
    vm.call(&f, args, Vec::new())
}

fn num_getnewargs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    Ok(vm.types.new_tuple(vec![v]))
}

fn int_ratio(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = strip(vm, &a[0]);
    Ok(vm.types.new_tuple(vec![v, Value::Int(1)]))
}

fn int_is_integer(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn float_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let v = match a.get(1) {
        None => Value::Float(0.0),
        Some(x) => Value::Float(to_float(vm, x)?),
    };
    let ft = vm.types.float.clone();
    Ok(subclass_instance(vm, &cls, &ft, v))
}

pub fn parse_float_str(s: &str) -> Option<f64> {
    let t = s.trim().replace('_', "");
    let l = t.to_ascii_lowercase();
    match l.trim_start_matches(['+', '-']) {
        "inf" | "infinity" => return Some(if l.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY }),
        "nan" => return Some(f64::NAN),
        _ => {}
    }
    if t.is_empty() || t.chars().any(|c| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))) {
        return None;
    }
    t.parse().ok()
}

pub fn to_float(vm: &mut Vm, x: &Value) -> PyResult<f64> {
    if let Some(Payload::Str(s)) = x.payload() {
        return parse_float_str(s.as_str()).ok_or_else(|| vm.value_error(format!("could not convert string to float: {}", crate::format::quote_str(s.as_str()))));
    }
    if let Some(Payload::Bytes(b)) = x.payload() {
        let s = String::from_utf8_lossy(b).to_string();
        return parse_float_str(&s).ok_or_else(|| vm.value_error(format!("could not convert string to float: {s:?}")));
    }
    ops::to_f64(vm, x)
}

fn float_is_integer(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = ops::to_f64(vm, &a[0])?;
    Ok(Value::Bool(f.is_finite() && f.fract() == 0.0))
}

fn float_ratio(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = ops::to_f64(vm, &a[0])?;
    if !f.is_finite() {
        return Err(vm.exc_msg("OverflowError", "cannot convert Infinity to integer ratio"));
    }
    let (mut m, mut e) = crate::hash::frexp(f);
    let mut i = 0;
    while m != m.floor() && i < 300 {
        m *= 2.0;
        e -= 1;
        i += 1;
    }
    let mut num: BigInt = num_traits::FromPrimitive::from_f64(m).unwrap_or_default();
    let mut den = BigInt::from(1);
    if e > 0 {
        num <<= e as usize;
    } else {
        den <<= (-e) as usize;
    }
    Ok(vm.types.new_tuple(vec![crate::builtins::big_to_value(vm, num), crate::builtins::big_to_value(vm, den)]))
}

fn float_hex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = ops::to_f64(vm, &a[0])?;
    let s = if f == 0.0 {
        if f.is_sign_negative() { "-0x0.0p+0".to_string() } else { "0x0.0p+0".to_string() }
    } else if !f.is_finite() {
        crate::format::float_repr(f)
    } else {
        let bits = f.to_bits();
        let sign = if f < 0.0 { "-" } else { "" };
        let exp = ((bits >> 52) & 0x7ff) as i64;
        let mant = bits & ((1u64 << 52) - 1);
        let (lead, e) = if exp == 0 { (0, -1022) } else { (1, exp - 1023) };
        let hex = format!("{mant:013x}");
        let hex = hex.trim_end_matches('0');
        let hex = if hex.is_empty() { "0" } else { hex };
        format!("{sign}0x{lead}.{hex}p{}{}", if e < 0 { '-' } else { '+' }, e.abs())
    };
    Ok(vm.types.new_string(s))
}

fn float_fromhex(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = str_arg(vm, &a[1], "fromhex")?;
    let t = s.trim().to_ascii_lowercase();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, t.trim_start_matches('+').to_string()),
    };
    let t = t.trim_start_matches("0x");
    let (mant, exp) = match t.split_once('p') {
        Some((m, e)) => (m, e.parse::<i32>().unwrap_or(0)),
        None => (t, 0),
    };
    let (ip, fp) = mant.split_once('.').unwrap_or((mant, ""));
    let mut v = 0f64;
    for c in ip.chars() {
        v = v * 16.0 + c.to_digit(16).ok_or_else(|| vm.value_error("invalid hexadecimal floating-point string"))? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in fp.chars() {
        v += c.to_digit(16).ok_or_else(|| vm.value_error("invalid hexadecimal floating-point string"))? as f64 * scale;
        scale /= 16.0;
    }
    let r = v * 2f64.powi(exp);
    Ok(Value::Float(if neg { -r } else { r }))
}

fn float_int(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    to_int(vm, &strip(vm, &a[0]))
}

fn float_floor(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = ops::to_f64(vm, &a[0])?;
    to_int(vm, &Value::Float(f.floor()))
}

fn float_ceil(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = ops::to_f64(vm, &a[0])?;
    to_int(vm, &Value::Float(f.ceil()))
}

fn complex_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [re, im] = parse(vm, "complex", &a[1..], k, ["real", "imag"], 0)?;
    let (mut r, mut i) = (0.0, 0.0);
    if let Some(re) = re {
        match num::num_of(&re) {
            Some(Num::C(a, b)) => {
                r = a;
                i = b;
            }
            Some(_) => r = ops::to_f64(vm, &re)?,
            None => {
                if let Some(s) = re.as_str() {
                    let t = s.trim().trim_start_matches('(').trim_end_matches(')').replace(' ', "");
                    if let Some(body) = t.strip_suffix(['j', 'J']) {
                        let split = body.char_indices().skip(1).filter(|(_, c)| *c == '+' || *c == '-').filter(|(p, _)| !body[..*p].ends_with(['e', 'E'])).last().map(|(p, _)| p);
                        match split {
                            Some(p) => {
                                r = parse_float_str(&body[..p]).unwrap_or(0.0);
                                let ip = &body[p..];
                                i = if ip == "+" { 1.0 } else if ip == "-" { -1.0 } else { parse_float_str(ip).unwrap_or(0.0) };
                            }
                            None => i = if body.is_empty() || body == "+" { 1.0 } else if body == "-" { -1.0 } else { parse_float_str(body).unwrap_or(0.0) },
                        }
                    } else {
                        r = parse_float_str(&t).ok_or_else(|| vm.value_error("complex() arg is a malformed string"))?;
                    }
                } else {
                    r = ops::to_f64(vm, &re)?;
                }
            }
        }
    }
    if let Some(im) = im {
        match num::num_of(&im) {
            Some(Num::C(a, b)) => {
                r -= b;
                i += a;
            }
            _ => i += ops::to_f64(vm, &im)?,
        }
    }
    Ok(vm.types.new_complex(r, i))
}

fn complex_conjugate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match num::num_of(&a[0]) {
        Some(Num::C(r, i)) => Ok(vm.types.new_complex(r, -i)),
        _ => Ok(a[0].clone()),
    }
}

fn none_new(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

// ---- sequences ----

fn add_seq_dunders(vm: &Vm, t: &ObjRef) {
    add_cmp_dunders(vm, t);
    m(vm, t, "__getitem__", d_getitem);
    m(vm, t, "__contains__", d_contains);
    m(vm, t, "__len__", d_len);
    m(vm, t, "__iter__", d_iter);
    m(vm, t, "__add__", d_add);
    m(vm, t, "__mul__", d_mul);
    m(vm, t, "__rmul__", d_rmul);
}

fn seq_items(vm: &mut Vm, v: &Value) -> PyResult<Vec<Value>> {
    match v.payload() {
        Some(Payload::List(l)) => Ok(l.borrow().clone()),
        Some(Payload::Tuple(t)) => Ok(t.to_vec()),
        _ => {
            let s = strip(vm, v);
            ops::collect(vm, &s)
        }
    }
}

fn seq_count(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "count", a, 2, 2)?;
    let items = seq_items(vm, &a[0])?;
    let mut n = 0;
    for it in &items {
        if ops::same_or_eq(vm, it, &a[1])? {
            n += 1;
        }
    }
    Ok(Value::Int(n))
}

fn seq_index(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "index", a, 2, 4)?;
    let items = seq_items(vm, &a[0])?;
    let n = items.len() as i64;
    let norm = |i: i64| if i < 0 { (i + n).max(0) } else { i.min(n) };
    let start = norm(crate::builtins::opt_i64(vm, &a.get(2).cloned(), 0)?);
    let stop = norm(crate::builtins::opt_i64(vm, &a.get(3).cloned(), n)?);
    for i in start..stop {
        if ops::same_or_eq(vm, &items[i as usize], &a[1])? {
            return Ok(Value::Int(i));
        }
    }
    let what = if matches!(a[0].payload(), Some(Payload::Tuple(_))) { "tuple.index(x): x not in tuple".to_string() } else {
        let r = ops::repr(vm, &a[1])?;
        format!("{r} is not in list")
    };
    Err(vm.value_error(what))
}

fn tuple_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let items = match a.get(1) {
        None => Vec::new(),
        Some(v) => ops::collect(vm, v)?,
    };
    let tt = vm.types.tuple.clone();
    if Rc::ptr_eq(&cls, &tt) {
        return Ok(vm.types.new_tuple(items));
    }
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::Tuple(items.into_boxed_slice()))))
}

fn tuple_getnewargs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = seq_items(vm, &a[0])?;
    let t = vm.types.new_tuple(items);
    Ok(vm.types.new_tuple(vec![t]))
}

fn list_of<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a RefCell<Vec<Value>>> {
    match v.payload() {
        Some(Payload::List(l)) => Ok(l),
        _ => Err(vm.type_error(format!("descriptor requires a 'list' object but received a '{}'", ops::type_name(vm, v)))),
    }
}

fn list_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(ops::alloc_instance(vm, &cls)))
}

fn list_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    crate::builtins::no_kw(vm, "list", k)?;
    arg_count(vm, "list", a, 1, 2)?;
    let items = match a.get(1) {
        Some(v) => ops::collect(vm, v)?,
        None => Vec::new(),
    };
    *list_of(vm, &a[0])?.borrow_mut() = items;
    Ok(Value::None)
}

fn list_iadd(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = ops::collect(vm, &a[1])?;
    list_of(vm, &a[0])?.borrow_mut().extend(items);
    Ok(a[0].clone())
}

fn list_imul(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[1])?.max(0) as usize;
    let l = list_of(vm, &a[0])?;
    let cur = l.borrow().clone();
    let mut out = Vec::with_capacity(cur.len() * n);
    for _ in 0..n {
        out.extend(cur.iter().cloned());
    }
    *l.borrow_mut() = out;
    Ok(a[0].clone())
}

fn list_reversed(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = list_of(vm, &a[0])?.borrow().len();
    Ok(vm.types.new_iter(IterState::Reversed { seq: a[0].clone(), idx: n as isize - 1 }))
}

fn list_append(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "append", a, 2, 2)?;
    list_of(vm, &a[0])?.borrow_mut().push(a[1].clone());
    Ok(Value::None)
}

fn list_extend(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "extend", a, 2, 2)?;
    let items = ops::collect(vm, &a[1])?;
    list_of(vm, &a[0])?.borrow_mut().extend(items);
    Ok(Value::None)
}

fn list_insert(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "insert", a, 3, 3)?;
    let i = ops::index_of(vm, &a[1])?;
    let l = list_of(vm, &a[0])?;
    let n = l.borrow().len() as i64;
    let j = if i < 0 { (i + n).max(0) } else { i.min(n) };
    l.borrow_mut().insert(j as usize, a[2].clone());
    Ok(Value::None)
}

fn list_remove(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "remove", a, 2, 2)?;
    let items = list_of(vm, &a[0])?.borrow().clone();
    for (i, it) in items.iter().enumerate() {
        if ops::same_or_eq(vm, it, &a[1])? {
            let l = list_of(vm, &a[0])?;
            if i < l.borrow().len() {
                l.borrow_mut().remove(i);
            }
            return Ok(Value::None);
        }
    }
    Err(vm.value_error("list.remove(x): x not in list"))
}

fn list_pop(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "pop", a, 1, 2)?;
    let i = crate::builtins::opt_i64(vm, &a.get(1).cloned(), -1)?;
    let l = list_of(vm, &a[0])?;
    let n = l.borrow().len();
    if n == 0 {
        return Err(vm.index_error("pop from empty list"));
    }
    let j = ops::normalize_index(vm, i, n, "pop")?;
    Ok(l.borrow_mut().remove(j))
}

fn list_clear(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    list_of(vm, &a[0])?.borrow_mut().clear();
    Ok(Value::None)
}

fn list_reverse(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    list_of(vm, &a[0])?.borrow_mut().reverse();
    Ok(Value::None)
}

fn list_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = list_of(vm, &a[0])?.borrow().clone();
    Ok(vm.types.new_list(items))
}

/// Stable merge sort with fallible comparisons (`list.sort`).
pub fn list_sort(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut key = None;
    let mut reverse = false;
    let mut cmp = None;
    for (n, v) in k {
        match &**n {
            "key" => key = Some(v.clone()).filter(|v| !v.is_none()),
            "reverse" => reverse = ops::truthy(vm, v)?,
            "cmp" => cmp = Some(v.clone()).filter(|v| !v.is_none()),
            _ => return Err(vm.type_error(format!("'{n}' is an invalid keyword argument for sort()"))),
        }
    }
    if a.len() > 1 {
        // Python 2 positional cmp.
        cmp = Some(a[1].clone()).filter(|v| !v.is_none());
    }
    let l = list_of(vm, &a[0])?;
    let items = std::mem::take(&mut *l.borrow_mut());
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
    for it in items {
        let kv = match &key {
            Some(f) => vm.call(f, vec![it.clone()], Vec::new())?,
            None => it.clone(),
        };
        keyed.push((kv, it));
    }
    if reverse {
        keyed.reverse();
    }
    let mut err = None;
    let sorted = merge_sort(vm, keyed, &mut |vm, x, y| {
        if err.is_some() {
            return false;
        }
        let r = match &cmp {
            Some(c) => vm.call(c, vec![y.clone(), x.clone()], Vec::new()).and_then(|r| ops::index_of(vm, &r)).map(|n| n > 0),
            None => ops::compare(vm, y, x, CmpOp::Lt).and_then(|r| ops::truthy(vm, &r)),
        };
        match r {
            Ok(b) => b,
            Err(e) => {
                err = Some(e);
                false
            }
        }
    });
    let mut out: Vec<Value> = sorted.into_iter().map(|(_, v)| v).collect();
    if reverse {
        out.reverse();
    }
    *list_of(vm, &a[0])?.borrow_mut() = out;
    match err {
        Some(e) => Err(e),
        None => Ok(Value::None),
    }
}

/// `less(vm, a, b)` is "b < a" inverted: returns true when `b` must come
/// before `a` (keeps stability when equal).
fn merge_sort(vm: &mut Vm, v: Vec<(Value, Value)>, before: &mut dyn FnMut(&mut Vm, &Value, &Value) -> bool) -> Vec<(Value, Value)> {
    if v.len() <= 1 {
        return v;
    }
    // Insertion sort for short runs.
    if v.len() <= 12 {
        let mut out: Vec<(Value, Value)> = Vec::with_capacity(v.len());
        for item in v {
            let mut pos = out.len();
            while pos > 0 && before(vm, &out[pos - 1].0, &item.0) {
                pos -= 1;
            }
            out.insert(pos, item);
        }
        return out;
    }
    let mut v = v;
    let right = v.split_off(v.len() / 2);
    let l = merge_sort(vm, v, before);
    let r = merge_sort(vm, right, before);
    let mut out = Vec::with_capacity(l.len() + r.len());
    let mut li = l.into_iter().peekable();
    let mut ri = r.into_iter().peekable();
    loop {
        match (li.peek(), ri.peek()) {
            (Some(a), Some(b)) => {
                if before(vm, &a.0, &b.0) {
                    out.push(ri.next().unwrap());
                } else {
                    out.push(li.next().unwrap());
                }
            }
            (Some(_), None) => out.push(li.next().unwrap()),
            (None, Some(_)) => out.push(ri.next().unwrap()),
            (None, None) => break,
        }
    }
    out
}

// ---- dict ----

fn dict_obj<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a ObjRef> {
    match v {
        Value::Obj(o) if matches!(o.payload, Payload::Dict(_)) => Ok(o),
        _ => Err(vm.type_error(format!("descriptor requires a 'dict' object but received a '{}'", ops::type_name(vm, v)))),
    }
}

fn dict_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(ops::alloc_instance(vm, &cls)))
}

pub fn dict_update_from(vm: &mut Vm, d: &ObjRef, src: &[Value], k: Kw) -> PyResult<()> {
    if let Some(s) = src.first() {
        for (key, v) in ops::mapping_items(vm, s)? {
            ops::dict_set(vm, d, key, v)?;
        }
    }
    for (key, v) in k {
        let kv = vm.types.new_str(key);
        ops::dict_set(vm, d, kv, v.clone())?;
    }
    Ok(())
}

fn dict_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    arg_count(vm, "dict", a, 1, 2)?;
    let d = dict_obj(vm, &a[0])?.clone();
    dict_update_from(vm, &d, &a[1..], k)?;
    Ok(Value::None)
}

fn dict_ior(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?.clone();
    dict_update_from(vm, &d, &a[1..], &[])?;
    Ok(a[0].clone())
}

fn dict_reversed(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?;
    let Payload::Dict(dd) = &d.payload else { unreachable!() };
    let mut keys = dd.borrow().keys();
    keys.reverse();
    Ok(vm.types.new_iter(IterState::Values { items: keys, idx: 0 }))
}

fn view(vm: &mut Vm, a: &[Value], kind: u8) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?.clone();
    let t = match kind {
        0 => vm.types.dict_keys.clone(),
        1 => vm.types.dict_values.clone(),
        _ => vm.types.dict_items.clone(),
    };
    Ok(Value::Obj(new_obj(t, None, Payload::DictView { dict: d, kind })))
}

fn view_or_list(vm: &mut Vm, a: &[Value], kind: u8) -> PyResult<Value> {
    let v = view(vm, a, kind)?;
    if ops::vm_dialect(vm) == Dialect::Py2 {
        let items = ops::collect(vm, &v)?;
        return Ok(vm.types.new_list(items));
    }
    Ok(v)
}

fn dict_keys(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view_or_list(vm, a, 0)
}
fn dict_values(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view_or_list(vm, a, 1)
}
fn dict_items(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view_or_list(vm, a, 2)
}
fn dict_viewkeys(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view(vm, a, 0)
}
fn dict_viewvalues(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view(vm, a, 1)
}
fn dict_viewitems(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    view(vm, a, 2)
}
fn dict_iterkeys(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = view(vm, a, 0)?;
    ops::iter(vm, &v)
}
fn dict_itervalues(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = view(vm, a, 1)?;
    ops::iter(vm, &v)
}
fn dict_iteritems(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let v = view(vm, a, 2)?;
    ops::iter(vm, &v)
}

fn dict_has_key(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "has_key", a, 2, 2)?;
    let d = dict_obj(vm, &a[0])?.clone();
    Ok(Value::Bool(ops::dict_get(vm, &d, &a[1])?.is_some()))
}

fn dict_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "get", a, 2, 3)?;
    let d = dict_obj(vm, &a[0])?.clone();
    Ok(ops::dict_get(vm, &d, &a[1])?.unwrap_or_else(|| a.get(2).cloned().unwrap_or(Value::None)))
}

fn dict_setdefault(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "setdefault", a, 2, 3)?;
    let d = dict_obj(vm, &a[0])?.clone();
    if let Some(v) = ops::dict_get(vm, &d, &a[1])? {
        return Ok(v);
    }
    let v = a.get(2).cloned().unwrap_or(Value::None);
    ops::dict_set(vm, &d, a[1].clone(), v.clone())?;
    Ok(v)
}

fn dict_pop(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "pop", a, 2, 3)?;
    let d = dict_obj(vm, &a[0])?.clone();
    match ops::dict_pop(vm, &d, &a[1])? {
        Some(v) => Ok(v),
        None => match a.get(2) {
            Some(v) => Ok(v.clone()),
            None => Err(vm.key_error(a[1].clone())),
        },
    }
}

fn dict_popitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?.clone();
    let Payload::Dict(dd) = &d.payload else { unreachable!() };
    let e = dd.borrow_mut().pop_last();
    match e {
        Some(e) => Ok(vm.types.new_tuple(vec![e.key, e.value])),
        None => Err(vm.exc_msg("KeyError", "popitem(): dictionary is empty")),
    }
}

fn dict_update(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    arg_count(vm, "update", a, 1, 2)?;
    let d = dict_obj(vm, &a[0])?.clone();
    dict_update_from(vm, &d, &a[1..], k)?;
    Ok(Value::None)
}

fn dict_clear(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?;
    if let Payload::Dict(dd) = &d.payload {
        dd.borrow_mut().clear();
    }
    Ok(Value::None)
}

fn dict_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = dict_obj(vm, &a[0])?;
    let Payload::Dict(dd) = &d.payload else { unreachable!() };
    let mut c = dd.borrow().clone();
    c.compact();
    Ok(Value::Obj(vm.types.new_dict_obj(c)))
}

fn dict_fromkeys(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "fromkeys", a, 2, 3)?;
    let d = vm.call(&a[0], Vec::new(), Vec::new())?;
    let v = a.get(2).cloned().unwrap_or(Value::None);
    for key in ops::collect(vm, &a[1])? {
        ops::setitem(vm, &d, key, v.clone())?;
    }
    Ok(d)
}

// ---- set ----

fn set_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(ops::alloc_instance(vm, &cls)))
}

fn set_init(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "set", a, 1, 2)?;
    if let Some(Payload::Set(d)) = a[0].payload() {
        d.borrow_mut().clear();
    }
    if let Some(src) = a.get(1) {
        for it in ops::collect(vm, src)? {
            ops::set_add(vm, &a[0], it)?;
        }
    }
    Ok(Value::None)
}

fn frozenset_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let items = match a.get(1) {
        Some(v) => ops::collect(vm, v)?,
        None => Vec::new(),
    };
    let t = ops::make_table(vm, items)?;
    let fs = vm.types.frozenset.clone();
    if Rc::ptr_eq(&cls, &fs) {
        return Ok(vm.types.new_frozenset(t));
    }
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::FrozenSet(t))))
}

fn as_set(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    match v.payload() {
        Some(Payload::Set(_) | Payload::FrozenSet(_)) => Ok(v.clone()),
        _ => {
            let items = ops::collect(vm, v)?;
            ops::make_set(vm, items)
        }
    }
}

fn same_kind(vm: &mut Vm, model: &Value, r: Value) -> Value {
    if let Some(Payload::FrozenSet(_)) = model.payload() {
        let keys = ops::set_keys(&r);
        let t = ops::make_table(vm, keys).unwrap_or_default();
        return vm.types.new_frozenset(t);
    }
    r
}

fn set_binop(vm: &mut Vm, a: &[Value], op: BinOp) -> PyResult<Value> {
    let mut acc = {
        let s = strip(vm, &a[0]);
        let keys = ops::set_keys(&s);
        ops::make_set(vm, keys)?
    };
    for other in &a[1..] {
        let o = as_set(vm, other)?;
        acc = ops::native_binary(vm, &acc, &o, op, false)?.unwrap();
    }
    Ok(same_kind(vm, &a[0], acc))
}

fn set_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    set_binop(vm, &a[..1], BinOp::BitOr)
}
fn set_union(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    set_binop(vm, a, BinOp::BitOr)
}
fn set_intersection(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    set_binop(vm, a, BinOp::BitAnd)
}
fn set_difference(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    set_binop(vm, a, BinOp::Sub)
}
fn set_symdiff(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    set_binop(vm, a, BinOp::BitXor)
}

fn set_replace(vm: &mut Vm, target: &Value, r: Value) -> PyResult<Value> {
    let (Some(Payload::Set(d)), Some(Payload::Set(rd))) = (target.payload(), r.payload()) else {
        return Err(vm.type_error("set expected"));
    };
    let nd = rd.borrow().clone();
    *d.borrow_mut() = nd;
    Ok(Value::None)
}

fn set_update(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    for other in &a[1..] {
        for it in ops::collect(vm, other)? {
            ops::set_add(vm, &a[0], it)?;
        }
    }
    Ok(Value::None)
}
fn set_intersection_update(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let r = set_binop(vm, a, BinOp::BitAnd)?;
    set_replace(vm, &a[0], r)
}
fn set_difference_update(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let r = set_binop(vm, a, BinOp::Sub)?;
    set_replace(vm, &a[0], r)
}
fn set_symdiff_update(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let r = set_binop(vm, a, BinOp::BitXor)?;
    set_replace(vm, &a[0], r)
}

fn set_issubset(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "issubset", a, 2, 2)?;
    let o = as_set(vm, &a[1])?;
    for k in ops::set_keys(&a[0]) {
        if !ops::set_contains(vm, &o, &k)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_issuperset(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "issuperset", a, 2, 2)?;
    for k in ops::collect(vm, &a[1])? {
        if !ops::set_contains(vm, &a[0], &k)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_isdisjoint(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "isdisjoint", a, 2, 2)?;
    let s = as_set(vm, &a[0])?;
    for k in ops::collect(vm, &a[1])? {
        if ops::set_contains(vm, &s, &k)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn set_add(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "add", a, 2, 2)?;
    ops::set_add(vm, &a[0], a[1].clone())?;
    Ok(Value::None)
}

fn set_remove(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "remove", a, 2, 2)?;
    if !ops::set_discard(vm, &a[0], &a[1])? {
        return Err(vm.key_error(a[1].clone()));
    }
    Ok(Value::None)
}

fn set_discard(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "discard", a, 2, 2)?;
    ops::set_discard(vm, &a[0], &a[1])?;
    Ok(Value::None)
}

fn set_pop(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Some(Payload::Set(d)) = a[0].payload() else { return Err(vm.type_error("set expected")) };
    let mut b = d.borrow_mut();
    let first = b.iter().next().map(|e| e.key.clone());
    match first {
        Some(k) => {
            let idx = (0..b.slots()).find(|i| b.entry(*i).is_some()).unwrap();
            b.remove_at(idx);
            Ok(k)
        }
        None => {
            drop(b);
            Err(vm.exc_msg("KeyError", "pop from an empty set"))
        }
    }
}

fn set_clear(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(Payload::Set(d)) = a[0].payload() {
        d.borrow_mut().clear();
        return Ok(Value::None);
    }
    Err(vm.type_error("set expected"))
}

// ---- range / slice ----

pub fn range_new_impl(vm: &mut Vm, a: &[Value], _k: Kw, _is_range: bool) -> PyResult<Value> {
    arg_count(vm, "range", a, 1, 3)?;
    let bigs: Vec<BigInt> = a.iter().map(|v| ops::to_bigint(v).ok_or(())).collect::<Result<_, _>>().map_err(|_| vm.type_error("range() integer argument expected"))?;
    let (start, stop, step) = match bigs.len() {
        1 => (BigInt::zero(), bigs[0].clone(), BigInt::from(1)),
        2 => (bigs[0].clone(), bigs[1].clone(), BigInt::from(1)),
        _ => (bigs[0].clone(), bigs[1].clone(), bigs[2].clone()),
    };
    if step.is_zero() {
        return Err(vm.value_error("range() arg 3 must not be zero"));
    }
    let r = match (start.to_i64(), stop.to_i64(), step.to_i64()) {
        (Some(a), Some(b), Some(s)) if a.checked_sub(b).is_some() && b.checked_sub(a).is_some() => Value::Obj(new_obj(vm.types.range.clone(), None, Payload::Range(a, b, s))),
        _ => Value::Obj(new_obj(vm.types.range.clone(), None, Payload::BigRange(Box::new((start, stop, step))))),
    };
    Ok(r)
}

fn range_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let r = range_new_impl(vm, &a[1..], k, true)?;
    if ops::vm_dialect(vm) == Dialect::Py2 {
        let items = ops::collect(vm, &r)?;
        return Ok(vm.types.new_list(items));
    }
    Ok(r)
}

fn range_reversed(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = vm.builtins.dict_get_str_raw("reversed").unwrap();
    vm.call(&f, vec![a[0].clone()], Vec::new())
}

fn slice_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let a = &a[1..];
    let (s, e, st) = match a.len() {
        1 => (Value::None, a[0].clone(), Value::None),
        2 => (a[0].clone(), a[1].clone(), Value::None),
        3 => (a[0].clone(), a[1].clone(), a[2].clone()),
        _ => return Err(vm.type_error("slice expected at least 1 argument")),
    };
    Ok(Value::Obj(new_obj(vm.types.slice.clone(), None, Payload::Slice(s, e, st))))
}

fn slice_indices_m(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "indices", a, 2, 2)?;
    let n = ops::index_of(vm, &a[1])?.max(0) as usize;
    let (s, e, st, _) = ops::slice_indices(vm, &a[0], n)?;
    Ok(vm.types.new_tuple(vec![Value::Int(s), Value::Int(e), Value::Int(st)]))
}

// ---- descriptors ----

fn property_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [fget, fset, fdel, doc] = parse(vm, "property", &a[1..], k, ["fget", "fset", "fdel", "doc"], 0)?;
    let fget = fget.unwrap_or(Value::None);
    let doc = match doc {
        Some(d) if !d.is_none() => d,
        _ => {
            if fget.is_none() { Value::None } else { ops::getattr_opt(vm, &fget, "__doc__")?.unwrap_or(Value::None) }
        }
    };
    let cls = a[0].obj().cloned().unwrap();
    let dict = if Rc::ptr_eq(&cls, &vm.types.property) { None } else { Some(Dict::new()) };
    Ok(Value::Obj(new_obj(cls, dict, Payload::Property { fget, fset: fset.unwrap_or(Value::None), fdel: fdel.unwrap_or(Value::None), doc })))
}

fn prop_parts(vm: &Vm, v: &Value) -> PyResult<(Value, Value, Value, Value)> {
    match v.payload() {
        Some(Payload::Property { fget, fset, fdel, doc }) => Ok((fget.clone(), fset.clone(), fdel.clone(), doc.clone())),
        _ => Err(vm.type_error("property expected")),
    }
}

fn prop_copy(vm: &mut Vm, model: &Value, parts: (Value, Value, Value, Value)) -> PyResult<Value> {
    let cls = ops::type_of(vm, model);
    let t = Value::Obj(cls);
    vm.call(&t, vec![parts.0, parts.1, parts.2, parts.3], Vec::new())
}

fn property_getter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut p = prop_parts(vm, &a[0])?;
    p.0 = a[1].clone();
    prop_copy(vm, &a[0], p)
}
fn property_setter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut p = prop_parts(vm, &a[0])?;
    p.1 = a[1].clone();
    prop_copy(vm, &a[0], p)
}
fn property_deleter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut p = prop_parts(vm, &a[0])?;
    p.2 = a[1].clone();
    prop_copy(vm, &a[0], p)
}
fn property_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = prop_parts(vm, &a[0])?;
    if a.len() < 2 || a[1].is_none() {
        return Ok(a[0].clone());
    }
    vm.call(&p.0, vec![a[1].clone()], Vec::new())
}
fn property_set(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = prop_parts(vm, &a[0])?;
    vm.call(&p.1, vec![a[1].clone(), a[2].clone()], Vec::new())
}

fn staticmethod_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "staticmethod", a, 2, 2)?;
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::StaticMethod(a[1].clone()))))
}
fn staticmethod_get(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match a[0].payload() {
        Some(Payload::StaticMethod(f)) => Ok(f.clone()),
        _ => Ok(a[0].clone()),
    }
}
fn classmethod_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "classmethod", a, 2, 2)?;
    let cls = a[0].obj().cloned().unwrap();
    Ok(Value::Obj(new_obj(cls, Some(Dict::new()), Payload::ClassMethod(a[1].clone()))))
}
fn classmethod_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let f = match a[0].payload() {
        Some(Payload::ClassMethod(f)) => f.clone(),
        _ => return Ok(a[0].clone()),
    };
    let owner = match (a.get(2), a.get(1)) {
        (Some(o), _) if !o.is_none() => o.clone(),
        (_, Some(i)) => Value::Obj(ops::type_of(vm, i)),
        _ => Value::None,
    };
    Ok(Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: f, slf: owner })))
}

fn super_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (typ, obj) = match a.len() {
        1 => {
            // Zero-argument form: __class__ cell and first argument of the caller.
            let f = vm.frames.last().ok_or_else(|| vm.runtime_error("super(): no current frame"))?;
            let code = f.code.clone();
            let idx = code.cellvars.iter().chain(code.freevars.iter()).position(|n| &**n == "__class__");
            let cls = idx.and_then(|i| match &f.cells[i].payload {
                Payload::Cell(c) => c.borrow().clone(),
                _ => None,
            });
            let Some(Value::Obj(cls)) = cls else {
                return Err(vm.runtime_error("super(): __class__ cell not found"));
            };
            if code.argcount == 0 && code.flags & crate::code::CO_VARARGS == 0 {
                return Err(vm.runtime_error("super(): no arguments"));
            }
            let first = if code.argcount > 0 {
                let ci = code.cell2arg.iter().position(|x| *x == Some(0));
                match ci {
                    Some(ci) => match &f.cells[ci].payload {
                        Payload::Cell(c) => c.borrow().clone(),
                        _ => None,
                    },
                    None => f.fast.first().cloned().flatten(),
                }
            } else {
                None
            };
            let Some(obj) = first else {
                return Err(vm.runtime_error("super(): arg[0] deleted"));
            };
            (cls, obj)
        }
        2 => (a[1].obj().cloned().ok_or_else(|| vm.type_error("super() argument 1 must be type"))?, Value::None),
        _ => (a[1].obj().cloned().ok_or_else(|| vm.type_error("super() argument 1 must be type"))?, a[2].clone()),
    };
    if typ.type_data().is_none() {
        return Err(vm.type_error("super() argument 1 must be type"));
    }
    if !obj.is_none() {
        let ok = ops::isinstance(vm, &obj, &typ) || (ops::is_type(&obj) && ops::is_subclass(obj.obj().unwrap(), &typ));
        if !ok {
            return Err(vm.type_error("super(type, obj): obj must be an instance or subtype of type"));
        }
    }
    Ok(Value::Obj(new_obj(vm.types.super_.clone(), None, Payload::Super { typ, obj })))
}

fn function_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if a.len() < 2 || a[1].is_none() {
        return Ok(a[0].clone());
    }
    Ok(Value::Obj(new_obj(vm.types.method.clone(), None, Payload::Method { func: a[0].clone(), slf: a[1].clone() })))
}

fn callable_call(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    vm.call(&a[0], a[1..].to_vec(), k.to_vec())
}

// ---- generators / iterators ----

fn gen_obj<'a>(vm: &Vm, v: &'a Value) -> PyResult<&'a ObjRef> {
    match v {
        Value::Obj(o) if matches!(o.payload, Payload::Generator(_)) => Ok(o),
        _ => Err(vm.type_error("generator expected")),
    }
}

fn gen_next(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let g = gen_obj(vm, &a[0])?.clone();
    match vm.gen_resume(&g, Resume::Send(Value::None))? {
        GenStep::Yielded(v) => Ok(v),
        GenStep::Returned(v) => Err(vm.stop_iteration(v)),
    }
}

fn gen_send(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "send", a, 2, 2)?;
    let g = gen_obj(vm, &a[0])?.clone();
    match vm.gen_resume(&g, Resume::Send(a[1].clone()))? {
        GenStep::Yielded(v) => Ok(v),
        GenStep::Returned(v) => Err(vm.stop_iteration(v)),
    }
}

fn gen_throw(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    arg_count(vm, "throw", a, 2, 4)?;
    let g = gen_obj(vm, &a[0])?.clone();
    let exc = if ops::is_type(&a[1]) && a.len() >= 3 && !a[2].is_none() {
        let v = vm.call(&a[1], vec![a[2].clone()], Vec::new())?;
        vm.instantiate_exc(v)?
    } else {
        vm.instantiate_exc(a[1].clone())?
    };
    if let Some(Payload::Generator(gs)) = g.payload.clone_status() {
        let _ = gs;
    }
    match vm.gen_resume(&g, Resume::Throw(exc))? {
        GenStep::Yielded(v) => Ok(v),
        GenStep::Returned(v) => Err(vm.stop_iteration(v)),
    }
}

trait CloneStatus {
    fn clone_status(&self) -> Option<Payload>;
}
impl CloneStatus for Payload {
    fn clone_status(&self) -> Option<Payload> {
        None
    }
}

fn gen_close(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let g = gen_obj(vm, &a[0])?.clone();
    let started = match &g.payload {
        Payload::Generator(gs) => matches!(gs.borrow().status, GenStatus::Suspended),
        _ => false,
    };
    if !started {
        if let Payload::Generator(gs) = &g.payload {
            gs.borrow_mut().status = GenStatus::Finished;
        }
        return Ok(Value::None);
    }
    let ge = vm.types.exc("GeneratorExit");
    let exc = vm.new_exc(&ge, Vec::new());
    match vm.gen_resume(&g, Resume::Throw(exc)) {
        Ok(GenStep::Yielded(_)) => Err(vm.runtime_error("generator ignored GeneratorExit")),
        Ok(GenStep::Returned(_)) => Ok(Value::None),
        Err(e) if vm.is_exc(&e, "GeneratorExit") || vm.is_exc(&e, "StopIteration") => Ok(Value::None),
        Err(e) => Err(e),
    }
}

fn iter_next_m(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    match ops::next(vm, &a[0])? {
        Some(v) => Ok(v),
        None => Err(vm.stop_iteration(Value::None)),
    }
}

fn iter_length_hint(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(Payload::Iter(st)) = a[0].payload() {
        if let IterState::Seq { seq, idx } = &*st.borrow() {
            if let Some(Payload::List(l)) = seq.payload() {
                return Ok(Value::Int(l.borrow().len().saturating_sub(*idx) as i64));
            }
            if let Some(Payload::Tuple(t)) = seq.payload() {
                return Ok(Value::Int(t.len().saturating_sub(*idx) as i64));
            }
        }
    }
    Ok(Value::Int(0))
}

// ---- exceptions ----

fn exc_init(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Value::Obj(o) = &a[0] {
        let t = vm.types.new_tuple(a[1..].to_vec());
        ops::set_raw_attr(vm, o, "args", t);
    }
    Ok(Value::None)
}

fn exc_str(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { return Ok(vm.types.new_str("")) };
    let s = crate::format::exception_str(vm, o)?;
    Ok(vm.types.new_string(s))
}

fn exc_with_traceback(_vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(a[0].clone())
}

fn exc_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { return Err(vm.type_error("exception expected")) };
    let cls = Value::Obj(o.class());
    let args = vm.types.new_tuple(crate::format::exc_args(o));
    Ok(vm.types.new_tuple(vec![cls, args]))
}

fn oserror_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    exc_init(vm, a, k)?;
    if let Value::Obj(o) = &a[0] {
        if a.len() >= 3 {
            ops::set_raw_attr(vm, o, "errno", a[1].clone());
            ops::set_raw_attr(vm, o, "strerror", a[2].clone());
            ops::set_raw_attr(vm, o, "filename", a.get(3).cloned().unwrap_or(Value::None));
        } else {
            ops::set_raw_attr(vm, o, "errno", Value::None);
            ops::set_raw_attr(vm, o, "strerror", Value::None);
            ops::set_raw_attr(vm, o, "filename", Value::None);
        }
    }
    Ok(Value::None)
}

/// `ImportError(*args, name=None, path=None)`; `msg` is the sole argument.
fn importerror_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut name = Value::None;
    let mut path = Value::None;
    for (key, v) in k {
        match &**key {
            "name" => name = v.clone(),
            "path" => path = v.clone(),
            other => return Err(vm.type_error(format!("'{other}' is an invalid keyword argument for ImportError()"))),
        }
    }
    exc_init(vm, a, &[])?;
    if let Value::Obj(o) = &a[0] {
        let msg = if a.len() == 2 { a[1].clone() } else { Value::None };
        ops::set_raw_attr(vm, o, "msg", msg);
        ops::set_raw_attr(vm, o, "name", name);
        ops::set_raw_attr(vm, o, "path", path);
    }
    Ok(Value::None)
}

fn syntaxerror_init(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    exc_init(vm, a, k)?;
    if let Value::Obj(o) = &a[0] {
        ops::set_raw_attr(vm, o, "msg", a.get(1).cloned().unwrap_or(Value::None));
        if let Some(Payload::Tuple(t)) = a.get(2).and_then(|v| v.payload()) {
            let names = ["filename", "lineno", "offset", "text"];
            for (i, n) in names.iter().enumerate() {
                ops::set_raw_attr(vm, o, n, t.get(i).cloned().unwrap_or(Value::None));
            }
        }
    }
    Ok(Value::None)
}

// ---- module / weakref ----

fn module_new(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = a[0].obj().cloned().unwrap();
    let name = a.get(1).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let ns = vm.types.new_dict_obj(Dict::new());
    let m = new_obj(cls, None, Payload::Module(name.as_str().into(), ns));
    Ok(Value::Obj(m))
}

fn module_init(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Value::Obj(o) = &a[0] {
        let name = a.get(1).cloned().unwrap_or(Value::None);
        ops::set_raw_attr(vm, o, "__name__", name);
        ops::set_raw_attr(vm, o, "__doc__", a.get(2).cloned().unwrap_or(Value::None));
    }
    Ok(Value::None)
}

// ---- namespace proxy ----

fn proxy_ns(vm: &Vm, v: &Value) -> PyResult<ObjRef> {
    crate::builtins::proxy_target(v).ok_or_else(|| vm.type_error("mappingproxy expected"))
}

fn ns_dict(o: &ObjRef) -> &RefCell<Dict> {
    o.dict.as_ref().unwrap()
}

fn key_str(vm: &mut Vm, k: &Value) -> PyResult<Value> {
    if k.as_str().is_some() {
        return Ok(k.clone());
    }
    // Non-string keys are rare in namespaces; store their string form.
    let s = ops::str(vm, k)?;
    Ok(vm.types.new_string(s))
}

fn proxy_getitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let key = key_str(vm, &a[1])?;
    let v = ns_dict(&o).borrow().get_str(key.as_str().unwrap()).cloned();
    v.ok_or_else(|| vm.key_error(a[1].clone()))
}

fn proxy_setitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let key = key_str(vm, &a[1])?;
    ns_dict(&o).borrow_mut().set_str(key, a[2].clone());
    Ok(Value::None)
}

fn proxy_delitem(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let key = key_str(vm, &a[1])?;
    if ns_dict(&o).borrow_mut().del_str(key.as_str().unwrap()).is_none() {
        return Err(vm.key_error(a[1].clone()));
    }
    Ok(Value::None)
}

fn proxy_contains(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let Some(s) = a[1].as_str() else { return Ok(Value::Bool(false)) };
    Ok(Value::Bool(ns_dict(&o).borrow().get_str(s).is_some()))
}

fn proxy_iter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let keys = ns_dict(&o).borrow().keys();
    Ok(vm.types.new_iter(IterState::Values { items: keys, idx: 0 }))
}

fn proxy_len(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    Ok(Value::Int(ns_dict(&o).borrow().len() as i64))
}

fn proxy_snapshot(vm: &mut Vm, a: &[Value]) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let mut d = ns_dict(&o).borrow().clone();
    d.compact();
    Ok(Value::Obj(vm.types.new_dict_obj(d)))
}

fn proxy_repr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = proxy_snapshot(vm, a)?;
    let s = ops::repr(vm, &d)?;
    Ok(vm.types.new_string(s))
}

fn proxy_eq(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let d = proxy_snapshot(vm, &a[..1])?;
    let other = match crate::builtins::proxy_target(&a[1]) {
        Some(_) => proxy_snapshot(vm, &a[1..2])?,
        None => a[1].clone(),
    };
    Ok(Value::Bool(ops::eq(vm, &d, &other)?))
}

fn proxy_keys(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let keys = ns_dict(&o).borrow().keys();
    Ok(vm.types.new_list(keys))
}

fn proxy_values(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let vals = ns_dict(&o).borrow().values();
    Ok(vm.types.new_list(vals))
}

fn proxy_items(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let items = ns_dict(&o).borrow().items();
    let v = items.into_iter().map(|(k, v)| vm.types.new_tuple(vec![k, v])).collect();
    Ok(vm.types.new_list(v))
}

fn proxy_get(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let r = a[1].as_str().and_then(|s| ns_dict(&o).borrow().get_str(s).cloned());
    Ok(r.unwrap_or_else(|| a.get(2).cloned().unwrap_or(Value::None)))
}

fn proxy_setdefault(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let key = key_str(vm, &a[1])?;
    if let Some(v) = ns_dict(&o).borrow().get_str(key.as_str().unwrap()).cloned() {
        return Ok(v);
    }
    let v = a.get(2).cloned().unwrap_or(Value::None);
    ns_dict(&o).borrow_mut().set_str(key, v.clone());
    Ok(v)
}

fn proxy_pop(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    let key = key_str(vm, &a[1])?;
    match ns_dict(&o).borrow_mut().del_str(key.as_str().unwrap()) {
        Some(v) => Ok(v),
        None => a.get(2).cloned().ok_or_else(|| vm.key_error(a[1].clone())),
    }
}

fn proxy_update(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    if let Some(src) = a.get(1) {
        for (key, v) in ops::mapping_items(vm, src)? {
            let key = key_str(vm, &key)?;
            ns_dict(&o).borrow_mut().set_str(key, v);
        }
    }
    for (key, v) in k {
        let kv = vm.types.new_str(key);
        ns_dict(&o).borrow_mut().set_str(kv, v.clone());
    }
    Ok(Value::None)
}

fn proxy_copy(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    proxy_snapshot(vm, a)
}

fn proxy_clear(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let o = proxy_ns(vm, &a[0])?;
    ns_dict(&o).borrow_mut().clear();
    Ok(Value::None)
}

pub fn unused(_: &str) -> Option<i64> {
    i64::from_str_radix("0", 10).ok()
}
