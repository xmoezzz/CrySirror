// SPDX-License-Identifier: MPL-2.0
//! `itertools`. Unbounded iterators are lazy native states; the
//! combinatoric ones materialise their (finite) inputs as CPython does.

use super::{module_with, set};
use crate::builtins::{parse, Kw};
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

pub enum It {
    Count { cur: Value, step: Value },
    Cycle { saved: Vec<Value>, src: Option<Value>, idx: usize },
    Repeat { v: Value, left: Option<i64> },
    Chain { its: Vec<Value>, cur: Option<Value> },
    ChainFrom { outer: Value, cur: Option<Value> },
    ISlice { it: Value, next: i64, stop: Option<i64>, step: i64, pos: i64 },
    StarMap { func: Value, it: Value },
    TakeWhile { pred: Value, it: Value, done: bool },
    DropWhile { pred: Value, it: Value, dropping: bool },
    ZipLongest { its: Vec<Option<Value>>, fill: Value },
    Accumulate { it: Value, func: Value, acc: Option<Value>, initial: Option<Value> },
    Compress { data: Value, sel: Value },
    Pairwise { it: Value, prev: Option<Value> },
    Batched { it: Value, n: usize },
}

fn native_iter(vm: &Vm, st: It) -> Value {
    vm.types.new_iter(IterState::Native(Box::new(st)))
}

pub fn next_native(vm: &mut Vm, o: &ObjRef) -> PyResult<Option<Value>> {
    let Payload::Iter(cell) = &o.payload else { return Ok(None) };
    // Take the state out so callbacks can run without holding the borrow.
    let mut st = {
        let mut b = cell.borrow_mut();
        let IterState::Native(n) = &mut *b else { return Ok(None) };
        match n.downcast_mut::<It>() {
            Some(_) => {}
            None => return Ok(None),
        }
        std::mem::replace(&mut *b, IterState::Exhausted)
    };
    let r = {
        let IterState::Native(n) = &mut st else { unreachable!() };
        let it = n.downcast_mut::<It>().unwrap();
        step(vm, it)
    };
    let done = matches!(r, Ok(None) | Err(_));
    if !done {
        *cell.borrow_mut() = st;
    }
    r
}

fn step(vm: &mut Vm, it: &mut It) -> PyResult<Option<Value>> {
    match it {
        It::Count { cur, step } => {
            let v = cur.clone();
            *cur = ops::binary_op(vm, cur, step, crate::ast::BinOp::Add, false, false)?;
            Ok(Some(v))
        }
        It::Cycle { saved, src, idx } => {
            if let Some(s) = src.clone() {
                match ops::next(vm, &s)? {
                    Some(v) => {
                        saved.push(v.clone());
                        return Ok(Some(v));
                    }
                    None => *src = None,
                }
            }
            if saved.is_empty() {
                return Ok(None);
            }
            let v = saved[*idx % saved.len()].clone();
            *idx += 1;
            Ok(Some(v))
        }
        It::Repeat { v, left } => match left {
            Some(0) => Ok(None),
            Some(n) => {
                *n -= 1;
                Ok(Some(v.clone()))
            }
            None => Ok(Some(v.clone())),
        },
        It::Chain { its, cur } => loop {
            if let Some(c) = cur.clone() {
                if let Some(v) = ops::next(vm, &c)? {
                    return Ok(Some(v));
                }
                *cur = None;
            }
            if its.is_empty() {
                return Ok(None);
            }
            let n = its.remove(0);
            *cur = Some(ops::iter(vm, &n)?);
        },
        It::ChainFrom { outer, cur } => loop {
            if let Some(c) = cur.clone() {
                if let Some(v) = ops::next(vm, &c)? {
                    return Ok(Some(v));
                }
                *cur = None;
            }
            match ops::next(vm, outer)? {
                Some(n) => *cur = Some(ops::iter(vm, &n)?),
                None => return Ok(None),
            }
        },
        It::ISlice { it, next, stop, step, pos } => {
            if let Some(s) = stop {
                if *next >= *s {
                    // Consume up to stop as CPython does.
                    while *pos < *s {
                        if ops::next(vm, it)?.is_none() {
                            break;
                        }
                        *pos += 1;
                    }
                    return Ok(None);
                }
            }
            while *pos < *next {
                if ops::next(vm, it)?.is_none() {
                    return Ok(None);
                }
                *pos += 1;
            }
            match ops::next(vm, it)? {
                Some(v) => {
                    *pos += 1;
                    *next += *step;
                    Ok(Some(v))
                }
                None => Ok(None),
            }
        }
        It::StarMap { func, it } => match ops::next(vm, it)? {
            Some(args) => {
                let a = ops::collect(vm, &args)?;
                vm.call(func, a, Vec::new()).map(Some)
            }
            None => Ok(None),
        },
        It::TakeWhile { pred, it, done } => {
            if *done {
                return Ok(None);
            }
            match ops::next(vm, it)? {
                Some(v) => {
                    let r = vm.call(pred, vec![v.clone()], Vec::new())?;
                    if ops::truthy(vm, &r)? {
                        Ok(Some(v))
                    } else {
                        *done = true;
                        Ok(None)
                    }
                }
                None => Ok(None),
            }
        }
        It::DropWhile { pred, it, dropping } => loop {
            let Some(v) = ops::next(vm, it)? else { return Ok(None) };
            if *dropping {
                let r = vm.call(pred, vec![v.clone()], Vec::new())?;
                if ops::truthy(vm, &r)? {
                    continue;
                }
                *dropping = false;
            }
            return Ok(Some(v));
        },
        It::ZipLongest { its, fill } => {
            let mut out = Vec::with_capacity(its.len());
            let mut any = false;
            for slot in its.iter_mut() {
                match slot.clone() {
                    Some(i) => match ops::next(vm, &i)? {
                        Some(v) => {
                            any = true;
                            out.push(v);
                        }
                        None => {
                            *slot = None;
                            out.push(fill.clone());
                        }
                    },
                    None => out.push(fill.clone()),
                }
            }
            if !any {
                return Ok(None);
            }
            Ok(Some(vm.types.new_tuple(out)))
        }
        It::Accumulate { it, func, acc, initial } => {
            if let Some(i) = initial.take() {
                *acc = Some(i.clone());
                return Ok(Some(i));
            }
            let Some(v) = ops::next(vm, it)? else { return Ok(None) };
            let nv = match acc.take() {
                None => v,
                Some(a) => {
                    if func.is_none() {
                        ops::binary_op(vm, &a, &v, crate::ast::BinOp::Add, false, false)?
                    } else {
                        vm.call(func, vec![a, v], Vec::new())?
                    }
                }
            };
            *acc = Some(nv.clone());
            Ok(Some(nv))
        }
        It::Compress { data, sel } => loop {
            let (Some(d), Some(s)) = (ops::next(vm, data)?, ops::next(vm, sel)?) else { return Ok(None) };
            if ops::truthy(vm, &s)? {
                return Ok(Some(d));
            }
        },
        It::Pairwise { it, prev } => {
            if prev.is_none() {
                match ops::next(vm, it)? {
                    Some(v) => *prev = Some(v),
                    None => return Ok(None),
                }
            }
            match ops::next(vm, it)? {
                Some(v) => {
                    let p = prev.replace(v.clone()).unwrap();
                    Ok(Some(vm.types.new_tuple(vec![p, v])))
                }
                None => Ok(None),
            }
        }
        It::Batched { it, n } => {
            let mut out = Vec::new();
            while out.len() < *n {
                match ops::next(vm, it)? {
                    Some(v) => out.push(v),
                    None => break,
                }
            }
            if out.is_empty() {
                return Ok(None);
            }
            Ok(Some(vm.types.new_tuple(out)))
        }
    }
}

fn count(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [start, step] = parse(vm, "count", a, k, ["start", "step"], 0)?;
    Ok(native_iter(vm, It::Count { cur: start.unwrap_or(Value::Int(0)), step: step.unwrap_or(Value::Int(1)) }))
}

fn cycle(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let src = ops::iter(vm, &a[0])?;
    Ok(native_iter(vm, It::Cycle { saved: Vec::new(), src: Some(src), idx: 0 }))
}

fn repeat(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [obj, times] = parse(vm, "repeat", a, k, ["object", "times"], 1)?;
    let left = match times {
        Some(t) => Some(ops::index_of(vm, &t)?.max(0)),
        None => None,
    };
    Ok(native_iter(vm, It::Repeat { v: obj.unwrap(), left }))
}

fn chain(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(native_iter(vm, It::Chain { its: a.to_vec(), cur: None }))
}

fn chain_new(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    chain(vm, &a[1..], k)
}

fn chain_from_iterable(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let outer = ops::iter(vm, &a[0])?;
    Ok(native_iter(vm, It::ChainFrom { outer, cur: None }))
}

fn islice(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "islice", a, 2, 4)?;
    let it = ops::iter(vm, &a[0])?;
    let opt = |vm: &mut Vm, v: Option<&Value>| -> PyResult<Option<i64>> {
        match v {
            Some(x) if !x.is_none() => Ok(Some(ops::index_of(vm, x)?)),
            _ => Ok(None),
        }
    };
    let (start, stop, step) = if a.len() == 2 { (0, opt(vm, a.get(1))?, 1) } else { (opt(vm, a.get(1))?.unwrap_or(0), opt(vm, a.get(2))?, opt(vm, a.get(3))?.unwrap_or(1)) };
    if start < 0 || stop.is_some_and(|s| s < 0) || step < 1 {
        return Err(vm.value_error("Indices for islice() must be None or an integer: 0 <= x <= sys.maxsize."));
    }
    Ok(native_iter(vm, It::ISlice { it, next: start, stop, step, pos: 0 }))
}

fn starmap(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[1])?;
    Ok(native_iter(vm, It::StarMap { func: a[0].clone(), it }))
}

fn takewhile(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[1])?;
    Ok(native_iter(vm, It::TakeWhile { pred: a[0].clone(), it, done: false }))
}

fn dropwhile(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[1])?;
    Ok(native_iter(vm, It::DropWhile { pred: a[0].clone(), it, dropping: true }))
}

fn zip_longest(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut fill = Value::None;
    for (n, v) in k {
        if &**n == "fillvalue" {
            fill = v.clone();
        }
    }
    let mut its = Vec::new();
    for x in a {
        its.push(Some(ops::iter(vm, x)?));
    }
    Ok(native_iter(vm, It::ZipLongest { its, fill }))
}

fn accumulate(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [itv, func, initial] = parse(vm, "accumulate", a, k, ["iterable", "func", "initial"], 1)?;
    let it = ops::iter(vm, &itv.unwrap())?;
    Ok(native_iter(vm, It::Accumulate { it, func: func.unwrap_or(Value::None), acc: None, initial: initial.filter(|v| !v.is_none()) }))
}

fn compress(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let data = ops::iter(vm, &a[0])?;
    let sel = ops::iter(vm, &a[1])?;
    Ok(native_iter(vm, It::Compress { data, sel }))
}

fn filterfalse(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[1])?;
    Ok(vm.types.new_iter(IterState::Filter { func: a[0].clone(), it, invert: true }))
}

fn imap(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut its = Vec::new();
    for x in &a[1..] {
        its.push(ops::iter(vm, x)?);
    }
    Ok(vm.types.new_iter(IterState::Map { func: a[0].clone(), its }))
}

fn izip(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut its = Vec::new();
    for x in a {
        its.push(ops::iter(vm, x)?);
    }
    Ok(vm.types.new_iter(IterState::Zip { its, strict: false }))
}

fn ifilter(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[1])?;
    Ok(vm.types.new_iter(IterState::Filter { func: a[0].clone(), it, invert: false }))
}

fn pairwise(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[0])?;
    Ok(native_iter(vm, It::Pairwise { it, prev: None }))
}

fn batched(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let it = ops::iter(vm, &a[0])?;
    let n = ops::index_of(vm, &a[1])?;
    if n < 1 {
        return Err(vm.value_error("n must be at least one"));
    }
    Ok(native_iter(vm, It::Batched { it, n: n as usize }))
}

fn list_iter(vm: &Vm, items: Vec<Value>) -> Value {
    vm.types.new_iter(IterState::Values { items, idx: 0 })
}

fn product(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let mut rep = 1;
    for (n, v) in k {
        if &**n == "repeat" {
            rep = ops::index_of(vm, v)?.max(0) as usize;
        }
    }
    let mut pools = Vec::new();
    for x in a {
        pools.push(ops::collect(vm, x)?);
    }
    let pools: Vec<Vec<Value>> = (0..rep).flat_map(|_| pools.clone()).collect();
    let mut result: Vec<Vec<Value>> = vec![Vec::new()];
    for p in &pools {
        let mut next = Vec::new();
        for r in &result {
            for x in p {
                let mut n = r.clone();
                n.push(x.clone());
                next.push(n);
            }
        }
        result = next;
    }
    let items = result.into_iter().map(|r| vm.types.new_tuple(r)).collect();
    Ok(list_iter(vm, items))
}

fn permutations(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [itv, r] = parse(vm, "permutations", a, k, ["iterable", "r"], 1)?;
    let pool = ops::collect(vm, &itv.unwrap())?;
    let n = pool.len();
    let r = match r {
        Some(v) if !v.is_none() => ops::index_of(vm, &v)? as usize,
        _ => n,
    };
    let mut out = Vec::new();
    if r <= n {
        let mut idx: Vec<usize> = (0..n).collect();
        let mut cycles: Vec<usize> = (n - r + 1..=n).rev().collect();
        out.push(idx[..r].iter().map(|i| pool[*i].clone()).collect::<Vec<_>>());
        'outer: while n > 0 {
            for i in (0..r).rev() {
                cycles[i] -= 1;
                if cycles[i] == 0 {
                    let x = idx.remove(i);
                    idx.push(x);
                    cycles[i] = n - i;
                } else {
                    let j = cycles[i];
                    idx.swap(i, n - j);
                    out.push(idx[..r].iter().map(|i| pool[*i].clone()).collect());
                    continue 'outer;
                }
            }
            break;
        }
    }
    let items = out.into_iter().map(|r| vm.types.new_tuple(r)).collect();
    Ok(list_iter(vm, items))
}

fn combos(pool: &[Value], r: usize, repl: bool) -> Vec<Vec<Value>> {
    let n = pool.len();
    let mut out = Vec::new();
    if (!repl && r > n) || (repl && n == 0 && r > 0) {
        return out;
    }
    let mut idx: Vec<usize> = if repl { vec![0; r] } else { (0..r).collect() };
    out.push(idx.iter().map(|i| pool[*i].clone()).collect());
    loop {
        let mut i = r;
        let mut found = false;
        while i > 0 {
            i -= 1;
            let limit = if repl { n - 1 } else { i + n - r };
            if idx[i] != limit {
                found = true;
                break;
            }
        }
        if !found {
            return out;
        }
        idx[i] += 1;
        for j in i + 1..r {
            idx[j] = if repl { idx[i] } else { idx[j - 1] + 1 };
        }
        out.push(idx.iter().map(|i| pool[*i].clone()).collect());
    }
}

fn combinations(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let pool = ops::collect(vm, &a[0])?;
    let r = ops::index_of(vm, &a[1])?.max(0) as usize;
    let items = combos(&pool, r, false).into_iter().map(|c| vm.types.new_tuple(c)).collect();
    Ok(list_iter(vm, items))
}

fn combinations_wr(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let pool = ops::collect(vm, &a[0])?;
    let r = ops::index_of(vm, &a[1])?.max(0) as usize;
    let items = combos(&pool, r, true).into_iter().map(|c| vm.types.new_tuple(c)).collect();
    Ok(list_iter(vm, items))
}

fn groupby(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [itv, key] = parse(vm, "groupby", a, k, ["iterable", "key"], 1)?;
    let items = ops::collect(vm, &itv.unwrap())?;
    let key = key.filter(|v| !v.is_none());
    let mut groups: Vec<(Value, Vec<Value>)> = Vec::new();
    for it in items {
        let kv = match &key {
            Some(f) => vm.call(f, vec![it.clone()], Vec::new())?,
            None => it.clone(),
        };
        let same = match groups.last() {
            Some((lk, _)) => ops::eq(vm, lk, &kv)?,
            None => false,
        };
        if same {
            groups.last_mut().unwrap().1.push(it);
        } else {
            groups.push((kv, vec![it]));
        }
    }
    let items = groups.into_iter().map(|(k, g)| {
        let gi = list_iter(vm, g);
        vm.types.new_tuple(vec![k, gi])
    }).collect();
    Ok(list_iter(vm, items))
}

fn tee(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = ops::collect(vm, &a[0])?;
    let n = match a.get(1) {
        Some(v) => ops::index_of(vm, v)?.max(0) as usize,
        None => 2,
    };
    let its = (0..n).map(|_| list_iter(vm, items.clone())).collect();
    Ok(vm.types.new_tuple(its))
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "itertools", &[
        ("count", count), ("cycle", cycle), ("repeat", repeat), ("chain", chain), ("islice", islice), ("starmap", starmap),
        ("takewhile", takewhile), ("dropwhile", dropwhile), ("zip_longest", zip_longest), ("izip_longest", zip_longest),
        ("accumulate", accumulate), ("compress", compress), ("filterfalse", filterfalse), ("ifilterfalse", filterfalse),
        ("imap", imap), ("izip", izip), ("ifilter", ifilter), ("pairwise", pairwise), ("batched", batched), ("product", product),
        ("permutations", permutations), ("combinations", combinations), ("combinations_with_replacement", combinations_wr),
        ("groupby", groupby), ("tee", tee),
    ]);
    // `chain` is a type so that `chain.from_iterable` exists.
    let ct = vm.types.new_builtin_type("chain", &vm.types.object.clone(), BaseKind::Object, true);
    let n = vm.types.new_builtin("__new__", chain_new);
    let n = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(crate::dict::Dict::new()), Payload::StaticMethod(n)));
    vm.types.set_attr(&ct, "__new__", n);
    let cf = vm.types.new_builtin("from_iterable", chain_from_iterable);
    let cf = Value::Obj(new_obj(vm.types.staticmethod.clone(), Some(crate::dict::Dict::new()), Payload::StaticMethod(cf)));
    vm.types.set_attr(&ct, "from_iterable", cf);
    set(vm, &m, "chain", Value::Obj(ct));
    Ok(m)
}
