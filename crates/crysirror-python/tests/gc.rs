// SPDX-License-Identifier: MPL-2.0
//! Cycle collection, finalizers and weak references: each script asserts
//! its own results.

use crysirror_python::{Dialect, Payload, Value, Vm, run_source};

fn run(src: &str) {
    let mut vm = Vm::new();
    if let Err(e) = run_source(&mut vm, src, Dialect::Py3) {
        panic!("{e}");
    }
}

fn run2(src: &str) {
    let mut vm = Vm::new();
    crysirror_python::modules::sys::set_dialect(&mut vm, Dialect::Py2);
    if let Err(e) = run_source(&mut vm, src, Dialect::Py2) {
        panic!("{e}");
    }
}

#[test]
fn cycles_are_collected() {
    run(r#"
import gc, weakref
gc.disable()
class C:
    pass
a = C(); b = C()
a.other = b; b.other = a
wa = weakref.ref(a); wb = weakref.ref(b)
del a, b
assert wa() is not None and wb() is not None
n = gc.collect()
assert n >= 2, n
assert wa() is None and wb() is None
# Self-referencing list and dict.
l = []; l.append(l); wl = weakref.ref(C()); d = {}; d['d'] = d
holder = C(); holder.l = l; holder.d = d; holder.me = holder
wh = weakref.ref(holder)
del l, d, holder
assert gc.collect() >= 3
assert wh() is None
# Nothing left to find.
assert gc.collect() == 0
"#);
}

#[test]
fn reachable_objects_survive() {
    run(r#"
import gc, weakref
gc.disable()
class C:
    pass
keep = C(); keep.me = keep; keep.data = [1, 2, 3]
w = weakref.ref(keep)
for g in (0, 1, 2, 2):
    gc.collect(g)
assert w() is keep and keep.data == [1, 2, 3] and keep.me is keep
# Reachable only through an older generation.
old = C(); old.items = []
gc.collect()
young = C(); young.me = young; old.items.append(young)
wy = weakref.ref(young)
del young
gc.collect(0)
assert wy() is not None and wy().me is wy()
"#);
}

#[test]
fn del_runs_once_for_refcount_and_cycle_deaths() {
    run(r#"
import gc
gc.disable()
log = []
class D:
    def __init__(self, name):
        self.name = name
    def __del__(self):
        log.append(self.name)
d = D('plain')
del d
assert log == ['plain'], log
x = D('cycle'); x.me = x
y = D('pair1'); z = D('pair2'); y.o = z; z.o = y
del x, y, z
assert log == ['plain']
gc.collect()
assert sorted(log) == ['cycle', 'pair1', 'pair2', 'plain'], log
gc.collect()
assert len(log) == 4
"#);
}

#[test]
fn resurrection_runs_del_once() {
    run(r#"
import gc, weakref
gc.disable()
saved = []
calls = []
class R:
    def __del__(self):
        calls.append(1)
        saved.append(self)
# Refcount death with resurrection.
r = R(); r.tag = 'refcount'
del r
assert len(calls) == 1 and saved[0].tag == 'refcount'
assert gc.is_finalized(saved[0])
saved.clear()
assert len(calls) == 1
# Cyclic garbage resurrected by its finalizer.
c = R(); c.me = c; c.tag = 'cycle'
del c
gc.collect()
assert len(calls) == 2 and saved[0].tag == 'cycle' and saved[0].me is saved[0]
w = weakref.ref(saved[0])
saved.clear()
gc.collect()
assert len(calls) == 2 and w() is None
"#);
}

#[test]
fn weakref_callbacks_on_refcount_death_and_gc() {
    run(r#"
import gc, weakref
gc.disable()
class C:
    pass
seen = []
o = C()
r = weakref.ref(o, seen.append)
assert r.__callback__ is not None and weakref.getweakrefcount(o) == 1
del o
assert seen == [r] and r() is None and r.__callback__ is None
c = C(); c.me = c
r2 = weakref.ref(c, seen.append)
del c
assert len(seen) == 1
gc.collect()
assert len(seen) == 2 and seen[1] is r2 and r2() is None
# A weakref that is itself part of the garbage does not call back.
g = C(); g.me = g; g.wr = weakref.ref(g, seen.append)
del g
gc.collect()
assert len(seen) == 2
# Callbacks run newest first.
order = []
o = C()
r_a = weakref.ref(o, lambda w: order.append('a'))
r_b = weakref.ref(o, lambda w: order.append('b'))
del o
assert order == ['b', 'a'], order
"#);
}

#[test]
fn weakref_identity_hash_and_errors() {
    run(r#"
import weakref
class C:
    pass
o = C()
r1 = weakref.ref(o); r2 = weakref.ref(o)
assert r1 is r2
assert weakref.getweakrefcount(o) == 1 and weakref.getweakrefs(o) == [r1]
r3 = weakref.ref(o, lambda w: None)
assert r3 is not r1 and r3 == r1 and weakref.getweakrefcount(o) == 2
assert hash(r1) == hash(o)
h = hash(r1)
o2 = C()
assert weakref.ref(o2) != r1
del o
assert hash(r1) == h
assert r1 != r3 and r1 == r1
dead = weakref.ref(C())
try:
    hash(dead)
    raise AssertionError
except TypeError:
    pass
for bad in (1, 'str', (1,), [1], {}, None):
    try:
        weakref.ref(bad)
        raise AssertionError(bad)
    except TypeError:
        pass
class L(list):
    pass
assert weakref.ref(L())() is None
assert 'dead' in repr(dead)
def f(): pass
assert weakref.ref(f)() is f
"#);
}

#[test]
fn proxies_forward_and_raise_reference_error() {
    run(r#"
import weakref
class C:
    def __init__(self):
        self.v = 5
        self.items = [1, 2, 3]
    def double(self, x):
        return 2 * x
    def __len__(self):
        return 7
    def __add__(self, other):
        return 'add%r' % (other,)
    def __call__(self, a):
        return a + 1
    def __getitem__(self, i):
        return i * 10
o = C()
p = weakref.proxy(o)
assert type(p) is weakref.CallableProxyType
assert p.v == 5 and p.double(4) == 8 and len(p) == 7 and p[3] == 30 and p(1) == 2
assert p + 1 == 'add1'
p.w = 9
assert o.w == 9
del p.w
assert not hasattr(o, 'w')
assert p == o and bool(p)
assert weakref.getweakrefcount(o) == 1
class N:
    pass
n = N()
q = weakref.proxy(n)
assert type(q) is weakref.ProxyType and isinstance(q, weakref.ProxyTypes)
try:
    hash(q)
    raise AssertionError
except TypeError:
    pass
fired = []
q2 = weakref.proxy(n, fired.append)
del n
assert fired == [q2]
for op in (lambda: q.x, lambda: q2.x, lambda: str(q)):
    try:
        op()
        raise AssertionError
    except ReferenceError:
        pass
assert 'dead' in repr(q)
"#);
}

#[test]
fn weak_containers() {
    run(r#"
import gc, weakref
gc.disable()
class C:
    pass
d = weakref.WeakValueDictionary()
a, b = C(), C()
d['a'] = a; d['b'] = b
assert len(d) == 2 and d['a'] is a and 'b' in d and sorted(d.keys()) == ['a', 'b']
del a
assert len(d) == 1 and 'a' not in d and d.get('a') is None and list(d) == ['b']
b.me = b
del b
assert len(d) == 1
gc.collect()
assert len(d) == 0
d['x'] = c = C()
assert d.pop('x') is c and len(d) == 0
assert d.setdefault('y', c) is c and dict(d.items()) == {'y': c}

k = weakref.WeakKeyDictionary()
k1, k2 = C(), C()
k[k1] = 1; k[k2] = 2
assert len(k) == 2 and k[k1] == 1 and k2 in k
del k1
assert len(k) == 1 and list(k.values()) == [2]
k2.me = k2
del k2
gc.collect()
assert len(k) == 0

s = weakref.WeakSet()
m1, m2 = C(), C()
s.add(m1); s.add(m2)
assert len(s) == 2 and m1 in s
del m1
assert len(s) == 1 and list(s) == [m2]
s.discard(m2)
assert len(s) == 0

class Obj:
    def method(self):
        return 'm'
o = Obj()
wm = weakref.WeakMethod(o.method)
assert wm()() == 'm'
del o
assert wm() is None
"#);
}

#[test]
fn finalize_objects() {
    run(r#"
import gc, weakref, atexit
gc.disable()
class C:
    pass
calls = []
o = C()
f = weakref.finalize(o, calls.append, 'gone')
assert f.alive and f.peek()[0] is o and f.atexit
del o
assert calls == ['gone'] and not f.alive and f() is None
o = C(); o.me = o
f = weakref.finalize(o, calls.append, 'cycle')
del o
gc.collect()
assert calls == ['gone', 'cycle']
o = C()
f = weakref.finalize(o, calls.append, 'detached')
obj, func, args, kw = f.detach()
assert obj is o and args == ('detached',) and not f.alive
del o, obj
assert calls == ['gone', 'cycle']
keep = C()
f = weakref.finalize(keep, calls.append, 'exit')
assert f() is None and calls[-1] == 'exit' and not f.alive
keep2 = C()
weakref.finalize(keep2, calls.append, 'atexit')
atexit._run_exitfuncs()
assert calls[-1] == 'atexit'
"#);
}

#[test]
fn generator_closure_and_method_cycles() {
    run(r#"
import gc, weakref
gc.disable()
class C:
    def method(self):
        return 1
# Bound method stored on its own instance.
o = C(); o.m = o.method
w1 = weakref.ref(o)
# Closure cell referring back to the function.
def outer():
    def inner():
        return inner
    return inner
f = outer()
w2 = weakref.ref(f)
# Generator whose frame references the generator.
def gen():
    me = yield
    yield me
g = gen(); next(g); g.send(g)
w3 = weakref.ref(g)
# Class <-> method <-> module-like namespace.
ns = {}
exec("class K:\n    def f(self):\n        return K\n", ns)
w4 = weakref.ref(ns['K'])
del o, f, g, ns
gc.collect()
assert (w1(), w2(), w3(), w4()) == (None, None, None, None), (w1(), w2(), w3(), w4())
"#);
}

#[test]
fn generator_finally_runs_when_collected() {
    run(r#"
import gc
gc.disable()
log = []
def g():
    try:
        yield 1
        yield 2
    finally:
        log.append('closed')
it = g(); next(it)
del it
assert log == ['closed'], log
it = g(); next(it)
holder = [it]; holder.append(holder)
del it, holder
assert log == ['closed']
gc.collect()
assert log == ['closed', 'closed'], log
"#);
}

#[test]
fn automatic_collection_follows_enable_state() {
    run(r#"
import gc, weakref
class C:
    pass
def make(n):
    refs = []
    for i in range(n):
        c = C(); c.me = c
        refs.append(weakref.ref(c))
    return refs
gc.disable()
before = gc.get_stats()[0]['collections']
refs = make(3000)
assert gc.get_stats()[0]['collections'] == before
assert all(r() is not None for r in refs)
assert gc.get_count()[0] > 700
gc.collect()
gc.enable()
assert gc.isenabled()
refs = make(3000)
assert gc.get_stats()[0]['collections'] > before
dead = sum(1 for r in refs if r() is None)
assert dead > 1000, dead
"#);
}

#[test]
fn deep_structures_do_not_overflow() {
    run(r#"
import gc, weakref
gc.disable()
class Node:
    pass
# A million-node linked list closed into a cycle: traversal and clearing.
head = Node(); node = head
for i in range(1000000):
    nxt = Node(); node.next = nxt; node = nxt
node.next = head
w = weakref.ref(head)
del head, node, nxt
assert gc.collect() >= 1000000
assert w() is None
# Freed by reference counting: nested lists and tuples.
deep = None
for i in range(1000000):
    deep = [deep]
del deep
t = ()
for i in range(1000000):
    t = (t, i)
del t
"#);
}

#[test]
fn objects_held_by_rust_survive() {
    let mut vm = Vm::new();
    let g = run_source(&mut vm, "class C:\n    pass\nc = C()\nc.me = c\nc.data = [1, 2, 3]\n", Dialect::Py3).unwrap();
    let held: Vec<Value> = vec![g.dict_get_str_raw("c").unwrap()];
    let code = vm.compile_exec("import gc\ndel c\nn = gc.collect()\n", "<t>", Dialect::Py3).unwrap();
    vm.run_code(code, g.clone(), None).unwrap();
    assert_eq!(g.dict_get_str_raw("n").map(|v| matches!(v, Value::Int(0))), Some(true));
    let Value::Obj(c) = &held[0] else { panic!() };
    let data = c.dict.as_ref().unwrap().borrow().get_str("data").cloned().unwrap();
    let Some(Payload::List(l)) = data.payload() else { panic!() };
    assert_eq!(l.borrow().len(), 3);
    let me = c.dict.as_ref().unwrap().borrow().get_str("me").cloned().unwrap();
    assert!(me.is(&held[0]));
    // Once Rust lets go the cycle is ordinary garbage.
    drop(data);
    drop(me);
    drop(held);
    let code = vm.compile_exec("n = gc.collect()\n", "<t>", Dialect::Py3).unwrap();
    vm.run_code(code, g.clone(), None).unwrap();
    assert!(matches!(g.dict_get_str_raw("n"), Some(Value::Int(n)) if n >= 1));
}

struct Holder(Value);
struct Opaque(#[allow(dead_code)] Value);

#[test]
fn native_payload_hooks() {
    crysirror_python::gc::register_native_traverse::<Holder>(|h, v| v.value(&h.0), Some(|h| h.0 = Value::None));
    let mut vm = Vm::new();
    let g = run_source(&mut vm, "import gc, weakref\ngc.disable()\nclass C:\n    pass\n", Dialect::Py3).unwrap();
    let cls = vm.types.native.clone();
    for (name, opaque) in [("hooked", false), ("opaque", true)] {
        // instance -> native payload -> instance
        let code = vm.compile_exec("o = C()\nw = weakref.ref(o)\n", "<t>", Dialect::Py3).unwrap();
        vm.run_code(code, g.clone(), None).unwrap();
        let o = g.dict_get_str_raw("o").unwrap();
        let n = if opaque { vm.types.new_native(&cls, Box::new(Opaque(o.clone()))) } else { vm.types.new_native(&cls, Box::new(Holder(o.clone()))) };
        drop(o);
        let key = vm.types.new_str("native");
        g.dict_set_raw(&vm.types, key.clone(), n.clone());
        let code = vm.compile_exec("o.native = native\ndel o, native\ngc.collect()\nalive = w() is not None\n", "<t>", Dialect::Py3).unwrap();
        drop(n);
        vm.run_code(code, g.clone(), None).unwrap();
        let alive = matches!(g.dict_get_str_raw("alive"), Some(Value::Bool(true)));
        assert_eq!(alive, opaque, "{name}");
    }
}

#[test]
fn gc_module_api() {
    run(r#"
import gc
gc.disable()
assert not gc.isenabled()
assert gc.get_threshold() == (700, 10, 10)
gc.set_threshold(500, 5)
assert gc.get_threshold() == (500, 5, 10)
gc.set_threshold(700, 10, 10)
assert gc.get_debug() == 0
assert gc.DEBUG_LEAK == gc.DEBUG_COLLECTABLE | gc.DEBUG_UNCOLLECTABLE | gc.DEBUG_SAVEALL
assert len(gc.get_count()) == 3 and len(gc.get_stats()) == 3
class C:
    pass
c = C(); c.items = [1]
assert gc.is_tracked(c) and gc.is_tracked([]) and not gc.is_tracked(1) and not gc.is_tracked('s')
assert not gc.is_tracked((1, 'a')) and gc.is_tracked(([],))
assert not gc.is_finalized(c)
assert c.items in gc.get_referents(c) or any(x is c.items for x in gc.get_referents(c))
assert any(r is c for r in gc.get_referrers(c.items))
assert any(o is c for o in gc.get_objects())
gc.collect()
assert any(o is c for o in gc.get_objects(2)) and not any(o is c for o in gc.get_objects(0))
try:
    gc.collect(3)
    raise AssertionError
except ValueError:
    pass
events = []
def cb(phase, info):
    events.append((phase, info['generation']))
gc.callbacks.append(cb)
gc.collect(1)
gc.callbacks.remove(cb)
assert events == [('start', 1), ('stop', 1)], events
gc.set_debug(gc.DEBUG_SAVEALL)
x = C(); x.me = x
del x
assert gc.collect() >= 1
assert any(isinstance(o, C) for o in gc.garbage)
gc.set_debug(0)
del gc.garbage[:]
gc.freeze()
assert gc.get_freeze_count() > 0 and len(gc.get_objects()) == 0
gc.unfreeze()
assert gc.get_freeze_count() == 0 and len(gc.get_objects(2)) > 0
s = gc.get_stats()[2]
assert s['collections'] >= 2 and s['collected'] >= 1
"#);
}

#[test]
fn del_exceptions_are_reported() {
    run(r#"
import sys, gc
class Capture:
    def __init__(self):
        self.text = ''
    def write(self, s):
        self.text += s
cap = Capture()
old = sys.stderr
sys.stderr = cap
class Bad:
    def __del__(self):
        raise ValueError('boom')
b = Bad()
del b
x = 1
sys.stderr = old
assert 'Exception ignored in' in cap.text and 'ValueError: boom' in cap.text, cap.text
"#);
}

#[test]
fn getrefcount_counts_the_argument() {
    run(r#"
import sys
class C:
    pass
c = C()
base = sys.getrefcount(c)
assert base >= 2
d = c
assert sys.getrefcount(c) == base + 1
"#);
}

#[test]
fn python2_dialect_weakrefs() {
    run2(r#"
import gc, weakref
class C:
    pass
d = weakref.WeakValueDictionary()
a = C()
d['a'] = a
assert d.items() == [('a', a)] and d.keys() == ['a'] and d.has_key('a')
del a
assert d.keys() == []
class D:
    def __del__(self):
        log.append(1)
log = []
x = D(); x.me = x
del x
gc.collect()
assert log == [1]
"#);
}

/// Allocation-heavy loops with the collector enabled stay within a small
/// factor of the same loops with it disabled.
#[test]
fn allocation_overhead_sanity() {
    let mut vm = Vm::new();
    let src = r#"
import gc, time
class C:
    def __init__(self, v):
        self.v = v
def churn():
    t = time.time()
    for i in range(1000000):
        o = C(i)
    return time.time() - t
def kept():
    t = time.time()
    keep = []
    for i in range(300000):
        keep.append([i])
    return time.time() - t
gc.disable()
off = (churn(), kept())
gc.collect()
gc.enable()
on = (churn(), kept())
"#;
    let g = run_source(&mut vm, src, Dialect::Py3).unwrap();
    let pair = |n: &str| -> (f64, f64) {
        let v = g.dict_get_str_raw(n).unwrap();
        let Some(Payload::Tuple(t)) = v.payload() else { panic!() };
        let f = |v: &Value| match v {
            Value::Float(x) => *x,
            _ => panic!(),
        };
        (f(&t[0]), f(&t[1]))
    };
    let (off, on) = (pair("off"), pair("on"));
    eprintln!("1e6 instance churn: gc off {:.3}s, on {:.3}s; 3e5 kept lists: gc off {:.3}s, on {:.3}s", off.0, on.0, off.1, on.1);
    assert!(on.0 < off.0 * 3.0 + 0.5, "churn slowed down: {off:?} -> {on:?}");
    assert!(on.1 < off.1 * 6.0 + 0.5, "kept allocations slowed down: {off:?} -> {on:?}");
}
