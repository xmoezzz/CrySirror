// SPDX-License-Identifier: MPL-2.0
//! Cycle collector, finalizers and weak-reference bookkeeping.
//!
//! Objects are `Rc<PyObject>`, so acyclic garbage is freed by reference
//! counting. This module adds what reference counting cannot do:
//!
//! * **Cycle collection** (CPython-style trial deletion). Every object that
//!   can hold references ("container": a `__dict__`, a mutable/compound
//!   payload, a non-atomic tuple, or any instance of a class statement) is
//!   registered at allocation in a per-thread generation list of
//!   `Weak<PyObject>`. A collection of generation `g` upgrades the entries of
//!   generations `0..=g`, sets `gc_refs = strong_count - 1` (minus the
//!   collector's own handle) and subtracts every reference found by
//!   traversing the candidates. Objects left with `gc_refs > 0` are referenced
//!   from outside the candidate set (older generations, Rust locals, VM
//!   frames, suspended tasks, host data); they and everything reachable from
//!   them survive and move to the next generation. The rest is cyclic
//!   garbage. Dead `Weak` entries keep their allocation alive, so lists are
//!   pruned on every collection and whenever freed tracked objects outnumber
//!   live ones.
//! * **Conservative rules.** Traversal must never report a reference the
//!   object does not own (that could free reachable data); reporting too few
//!   only leaks. Hence: a `RefCell` that is currently borrowed makes its
//!   object a root and the object is not cleared; `Rc<Code>` and other
//!   shared Rust data are not traversed; `Payload::Native` (and
//!   `IterState::Native`) contents are opaque unless the payload type
//!   registered hooks with [`register_native_traverse`], so their references
//!   count as external.
//! * **Breaking cycles.** Unreachable objects are cleared like `tp_clear`:
//!   instance dicts, list/dict/set/cell contents, generator frames, iterator
//!   state, function `__module__`/`__doc__`, type MRO/bases, weakref callbacks
//!   and registered native payloads are moved out and dropped. Immutable
//!   members (tuples, bound methods) need no clearing; some other member of
//!   any cycle is clearable.
//! * **Finalizers** (PEP 442). `__del__` (and `close()` of a generator
//!   suspended inside `try`/`with`) runs at most once per object
//!   (`FINALIZED`). For cyclic garbage, weakrefs to the garbage are cleared and
//!   callbacks of reachable weakrefs invoked first, then finalizers run, then
//!   reachability is recomputed over the garbage and anything resurrected
//!   survives. A refcount death cannot call Python from `Drop` (no `Vm`):
//!   the `Drop` impl moves the contents into a new, finalized object (weak
//!   references are retargeted to it) and queues it; the VM calls `__del__`
//!   on it at the next safe point and it is freed normally afterwards. The
//!   only visible difference from CPython is that `id(self)` inside such a
//!   `__del__` differs from the object's earlier id.
//! * **Weak references.** A per-thread registry maps referent addresses to
//!   their weakref objects (`getweakrefcount`, `getweakrefs`, clearing).
//!   Callbacks due to refcount deaths are queued like finalizers.
//! * **Deep structures.** Traversal is iterative and `Drop` uses a
//!   CPython-style trashcan (contents of objects freed deeper than
//!   `TRASHCAN_DEPTH` nested drops are deferred and released by the outermost
//!   drop), so million-element linked structures neither overflow the stack
//!   when collected nor when freed.
//!
//! **Safe point.** Collections and queued finalizers/callbacks only run from
//! [`safe_point`], which the interpreter calls between bytecode instructions
//! (top of `Vm::step_loop`) and when an outermost `Vm::execute` returns.
//! Allocation (`new_obj`) and `Drop` only update counters and queues and set
//! a pending flag; they never run Python or clear objects. At an instruction
//! boundary the running loop holds no borrows; native code further down the
//! Rust stack that called back into Python must already tolerate arbitrary
//! Python running there, and every Rust-held `ObjRef` counts as an external
//! reference. Hosts may also call [`safe_point`] or [`collect`] directly
//! between script calls.
//!
//! The collector state is per thread; one `Vm` per thread is the supported
//! configuration (objects of several `Vm`s on one thread share the
//! generations, which is safe but runs their finalizers with whichever `Vm`
//! reaches a safe point).

use crate::dict::Dict;
use crate::object::*;
use crate::vm::{Frame, PyErr, Vm};
use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

pub const DEBUG_STATS: u32 = 1;
pub const DEBUG_COLLECTABLE: u32 = 2;
pub const DEBUG_UNCOLLECTABLE: u32 = 4;
pub const DEBUG_SAVEALL: u32 = 32;
pub const DEBUG_LEAK: u32 = DEBUG_COLLECTABLE | DEBUG_UNCOLLECTABLE | DEBUG_SAVEALL;

/// `PyObject::gc_flags` bits.
pub(crate) const TRACKED: u8 = 1;
pub(crate) const FINALIZED: u8 = 2;
pub(crate) const WEAKREFED: u8 = 4;
/// Member of the selection being analysed; `gc_index` holds its position.
const IN_SET: u8 = 8;

const NUM_GENERATIONS: usize = 3;
const TRASHCAN_DEPTH: u32 = 50;
const PRUNE_MIN: usize = 4096;

#[derive(Clone, Copy, Default)]
pub struct GenStats {
    pub collections: usize,
    pub collected: usize,
    pub uncollectable: usize,
}

/// Number of threads with queued finalizers/callbacks or a due collection.
/// A relaxed load of this is the per-instruction check; the thread-local
/// state is only consulted when it is non-zero.
static PENDING_THREADS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct PtrHasher(u64);
impl Hasher for PtrHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 << 8 | *b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    fn write_usize(&mut self, i: usize) {
        self.0 = ((i >> 4) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}
type PtrMap<V> = HashMap<usize, V, BuildHasherDefault<PtrHasher>>;

type Deferred = (Option<RefCell<Dict>>, Payload);

struct Heap {
    gens: [RefCell<Vec<Weak<PyObject>>>; NUM_GENERATIONS],
    permanent: RefCell<Vec<Weak<PyObject>>>,
    /// count[0]: tracked allocations minus deallocations since the last
    /// collection; count[g]: collections of generation g-1 since the last
    /// collection of generation g.
    count: [Cell<usize>; NUM_GENERATIONS],
    threshold: [Cell<usize>; NUM_GENERATIONS],
    enabled: Cell<bool>,
    debug: Cell<u32>,
    collecting: Cell<bool>,
    draining: Cell<bool>,
    signalled: Cell<bool>,
    /// Tracked objects freed since their list entries were last pruned.
    dead: Cell<usize>,
    long_lived_total: Cell<usize>,
    long_lived_pending: Cell<usize>,
    stats: [Cell<GenStats>; NUM_GENERATIONS],
    finalize: RefCell<VecDeque<ObjRef>>,
    callbacks: RefCell<VecDeque<(ObjRef, Value)>>,
    weakrefs: RefCell<PtrMap<Vec<Weak<PyObject>>>>,
    drop_depth: Cell<u32>,
    deferred: RefCell<Vec<Deferred>>,
    /// Set once `__del__` is assigned to an existing class, after which
    /// finalizer checks fall back to an MRO lookup.
    del_assigned: Cell<bool>,
}

impl Heap {
    fn new() -> Heap {
        Heap {
            gens: Default::default(),
            permanent: RefCell::new(Vec::new()),
            count: Default::default(),
            threshold: [Cell::new(700), Cell::new(10), Cell::new(10)],
            enabled: Cell::new(true),
            debug: Cell::new(0),
            collecting: Cell::new(false),
            draining: Cell::new(false),
            signalled: Cell::new(false),
            dead: Cell::new(0),
            long_lived_total: Cell::new(0),
            long_lived_pending: Cell::new(0),
            stats: Default::default(),
            finalize: RefCell::new(VecDeque::new()),
            callbacks: RefCell::new(VecDeque::new()),
            weakrefs: RefCell::new(PtrMap::default()),
            drop_depth: Cell::new(0),
            deferred: RefCell::new(Vec::new()),
            del_assigned: Cell::new(false),
        }
    }

    fn signal(&self) {
        if !self.signalled.replace(true) {
            PENDING_THREADS.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn unsignal(&self) {
        if self.signalled.replace(false) {
            PENDING_THREADS.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn collection_due(&self) -> bool {
        let t = self.threshold[0].get();
        self.enabled.get() && t != 0 && self.count[0].get() > t
    }

    /// The generation an automatic collection should process
    /// (CPython `gc_collect_generations`).
    fn auto_generation(&self) -> Option<usize> {
        if !self.collection_due() || self.collecting.get() {
            return None;
        }
        for g in (0..NUM_GENERATIONS).rev() {
            if self.count[g].get() > self.threshold[g].get() {
                // Full collections only once enough objects were promoted.
                if g == 2 && self.long_lived_pending.get() < self.long_lived_total.get() / 4 {
                    continue;
                }
                return Some(g);
            }
        }
        Some(0)
    }

    fn entries(&self) -> usize {
        self.gens.iter().map(|g| g.borrow().len()).sum::<usize>() + self.permanent.borrow().len()
    }

    fn prune_all(&self) {
        for g in self.gens.iter().chain(std::iter::once(&self.permanent)) {
            g.borrow_mut().retain(|w| w.strong_count() > 0);
        }
        self.dead.set(0);
    }
}

impl Drop for Heap {
    fn drop(&mut self) {
        self.unsignal();
    }
}

thread_local! {
    static HEAP: Heap = Heap::new();
}

fn addr(o: &PyObject) -> usize {
    o as *const PyObject as usize
}

fn addr_rc(o: &ObjRef) -> usize {
    Rc::as_ptr(o) as usize
}

// ---- native payload hooks ----

/// Visits the object references owned by a payload.
pub struct Visitor<'a> {
    f: &'a mut dyn FnMut(&ObjRef),
}

impl Visitor<'_> {
    pub fn obj(&mut self, o: &ObjRef) {
        (self.f)(o)
    }
    pub fn value(&mut self, v: &Value) {
        if let Value::Obj(o) = v {
            (self.f)(o)
        }
    }
    pub fn values<'v>(&mut self, vs: impl IntoIterator<Item = &'v Value>) {
        for v in vs {
            self.value(v);
        }
    }
}

type TraverseFn = Arc<dyn Fn(&dyn Any, &mut Visitor) + Send + Sync>;
type ClearFn = Arc<dyn Fn(&mut dyn Any) + Send + Sync>;

#[derive(Clone)]
struct NativeHook {
    traverse: TraverseFn,
    clear: Option<ClearFn>,
}

static NATIVE_HOOKS: OnceLock<RwLock<HashMap<TypeId, NativeHook>>> = OnceLock::new();

/// Lets the collector see the references held by a `Payload::Native` (or
/// `IterState::Native`) payload of type `T`, and optionally clear them when
/// the object is cyclic garbage.
///
/// `traverse` must report each reference that the payload owns exclusively,
/// once per strong `ObjRef`/`Value` it holds. It must not report references
/// held through shared Rust data (for example an `Rc<RefCell<..>>` that other
/// payloads or Rust code also hold): such references are external and
/// reporting them could free reachable objects. Reporting fewer references
/// is always safe. `clear` should move the references out (it runs while the
/// payload is mutably borrowed). Payload types without hooks are opaque and
/// their references count as external.
pub fn register_native_traverse<T: 'static>(traverse: fn(&T, &mut Visitor), clear: Option<fn(&mut T)>) {
    let hook = NativeHook {
        traverse: Arc::new(move |a: &dyn Any, v: &mut Visitor| {
            if let Some(t) = a.downcast_ref::<T>() {
                traverse(t, v)
            }
        }),
        clear: clear.map(|c| -> ClearFn {
            Arc::new(move |a: &mut dyn Any| {
                if let Some(t) = a.downcast_mut::<T>() {
                    c(t)
                }
            })
        }),
    };
    let lock = NATIVE_HOOKS.get_or_init(|| RwLock::new(HashMap::new()));
    lock.write().unwrap_or_else(|e| e.into_inner()).insert(TypeId::of::<T>(), hook);
}

struct Hooks(HashMap<TypeId, NativeHook>);

impl Hooks {
    fn snapshot() -> Hooks {
        Hooks(NATIVE_HOOKS.get().map(|l| l.read().unwrap_or_else(|e| e.into_inner()).clone()).unwrap_or_default())
    }
    fn traverse(&self, a: &dyn Any, f: &mut dyn FnMut(&ObjRef)) {
        if let Some(h) = self.0.get(&a.type_id()) {
            (h.traverse)(a, &mut Visitor { f });
        }
    }
    fn clear(&self, a: &mut dyn Any) {
        let id = (*a).type_id();
        if let Some(c) = self.0.get(&id).and_then(|h| h.clear.as_ref()) {
            c(a);
        }
    }
}

// ---- allocation / tracking ----

/// Whether a new object can take part in reference cycles. Atomic payloads
/// of builtin types and tuples of untracked items are left untracked.
pub(crate) fn needs_tracking(class: &ObjRef, has_dict: bool, payload: &Payload) -> bool {
    if has_dict || class.type_data().is_some_and(|t| !t.builtin) {
        return true;
    }
    let tracked = |v: &Value| matches!(v, Value::Obj(o) if o.gc_flags.get() & TRACKED != 0);
    match payload {
        Payload::None
        | Payload::Str(_)
        | Payload::Bytes(_)
        | Payload::ByteArray(_)
        | Payload::Int(_)
        | Payload::Float(_)
        | Payload::Complex(..)
        | Payload::Range(..)
        | Payload::BigRange(_)
        | Payload::Code(_)
        | Payload::NotImplemented
        | Payload::Ellipsis => false,
        Payload::Builtin(b) => b.bound.is_some(),
        Payload::Tuple(items) => items.iter().any(tracked),
        Payload::Slice(a, b, c) => tracked(a) || tracked(b) || tracked(c),
        _ => true,
    }
}

/// Registers a new container in generation 0.
pub(crate) fn track(o: &ObjRef) {
    let _ = HEAP.try_with(|h| {
        h.gens[0].borrow_mut().push(Rc::downgrade(o));
        h.count[0].set(h.count[0].get() + 1);
        if h.collection_due() {
            h.signal();
        }
        let dead = h.dead.get();
        if dead > PRUNE_MIN && dead > h.entries() / 2 {
            h.prune_all();
        }
    });
}

/// Records that `__del__` was set on (or removed from) class `t`.
pub(crate) fn note_del_assigned(t: &ObjRef) {
    if let Some(td) = t.type_data() {
        td.finalizer.set(true);
    }
    let _ = HEAP.try_with(|h| h.del_assigned.set(true));
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Finalizer {
    None,
    Del,
    GenClose,
}

fn finalizer_of(o: &PyObject, del_assigned: bool) -> Finalizer {
    if let Payload::Generator(g) = &o.payload {
        return match g.try_borrow() {
            Ok(g) if matches!(g.status, GenStatus::Suspended) && g.frame.as_ref().is_some_and(|f| !f.blocks.is_empty()) => Finalizer::GenClose,
            _ => Finalizer::None,
        };
    }
    let Ok(class) = o.class.try_borrow() else { return Finalizer::None };
    let Some(td) = class.as_ref().and_then(|c| c.type_data()) else { return Finalizer::None };
    if td.finalizer.get() {
        return Finalizer::Del;
    }
    if del_assigned && !td.builtin {
        if let Ok(mro) = td.mro.try_borrow() {
            for c in mro.iter() {
                if let Some(d) = &c.dict {
                    if d.try_borrow().is_ok_and(|d| d.get_str("__del__").is_some()) {
                        return Finalizer::Del;
                    }
                }
            }
        }
    }
    Finalizer::None
}

fn holds_refs(o: &PyObject) -> bool {
    o.dict.is_some()
        || !matches!(
            o.payload,
            Payload::None
                | Payload::Str(_)
                | Payload::Bytes(_)
                | Payload::ByteArray(_)
                | Payload::Int(_)
                | Payload::Float(_)
                | Payload::Complex(..)
                | Payload::Range(..)
                | Payload::BigRange(_)
                | Payload::Code(_)
                | Payload::NotImplemented
                | Payload::Ellipsis
        )
}

impl Drop for PyObject {
    fn drop(&mut self) {
        if self.gc_flags.get() & (TRACKED | WEAKREFED) == 0 && !holds_refs(self) {
            return;
        }
        // During thread teardown the fields are dropped the ordinary way.
        let _ = HEAP.try_with(|h| on_drop(self, h));
    }
}

fn on_drop(o: &mut PyObject, h: &Heap) {
    let flags = o.gc_flags.get();
    if flags & FINALIZED == 0 {
        let fin = finalizer_of(o, h.del_assigned.get());
        if fin != Finalizer::None && o.class.get_mut().is_some() {
            let r = resurrect(o, h);
            h.finalize.borrow_mut().push_back(r);
            h.signal();
            return;
        }
    }
    if flags & WEAKREFED != 0 {
        clear_weakrefs_on_death(addr(o), h);
    }
    if flags & TRACKED != 0 {
        h.count[0].set(h.count[0].get().saturating_sub(1));
        h.dead.set(h.dead.get() + 1);
    }
    if !holds_refs(o) {
        return;
    }
    // Trashcan: bound the Rust recursion of nested drops.
    let depth = h.drop_depth.get();
    let contents = (o.dict.take(), std::mem::replace(&mut o.payload, Payload::None));
    if depth >= TRASHCAN_DEPTH {
        h.deferred.borrow_mut().push(contents);
        return;
    }
    h.drop_depth.set(depth + 1);
    drop(contents);
    h.drop_depth.set(depth);
    if depth == 0 {
        loop {
            let next = h.deferred.borrow_mut().pop();
            let Some(c) = next else { break };
            h.drop_depth.set(1);
            drop(c);
            h.drop_depth.set(0);
        }
    }
}

/// Moves a dying object's contents into a new finalized object so its
/// finalizer can run at the next safe point. Weak references follow.
fn resurrect(o: &mut PyObject, h: &Heap) -> ObjRef {
    let flags = o.gc_flags.get();
    let new = Rc::new(PyObject {
        class: RefCell::new(o.class.get_mut().take()),
        dict: o.dict.take(),
        payload: std::mem::replace(&mut o.payload, Payload::None),
        gc_flags: Cell::new((flags | FINALIZED) & !WEAKREFED),
        gc_index: Cell::new(0),
    });
    if flags & TRACKED != 0 {
        h.gens[0].borrow_mut().push(Rc::downgrade(&new));
        h.dead.set(h.dead.get() + 1);
    }
    if flags & WEAKREFED != 0 {
        let refs = h.weakrefs.borrow_mut().remove(&addr(o));
        if let Some(refs) = refs {
            for w in refs.iter().filter_map(Weak::upgrade) {
                if let Payload::WeakRef(d) = &w.payload {
                    if let Ok(mut t) = d.target.try_borrow_mut() {
                        *t = Rc::downgrade(&new);
                    }
                }
            }
            new.gc_flags.set(new.gc_flags.get() | WEAKREFED);
            h.weakrefs.borrow_mut().insert(addr_rc(&new), refs);
        }
    }
    new
}

/// The referent at `key` died by reference counting: clear its weakrefs
/// and queue their callbacks, newest first.
fn clear_weakrefs_on_death(key: usize, h: &Heap) {
    let refs = h.weakrefs.borrow_mut().remove(&key);
    let Some(refs) = refs else { return };
    let mut queued = false;
    for w in refs.iter().rev().filter_map(Weak::upgrade) {
        let cb = clear_weakref(&w);
        if !cb.is_none() {
            h.callbacks.borrow_mut().push_back((w, cb));
            queued = true;
        }
    }
    if queued {
        h.signal();
    }
}

/// Detaches a weakref from its referent; returns its callback.
fn clear_weakref(w: &ObjRef) -> Value {
    let Payload::WeakRef(d) = &w.payload else { return Value::None };
    if let Ok(mut t) = d.target.try_borrow_mut() {
        *t = Weak::new();
    }
    d.callback.try_borrow_mut().map(|mut c| std::mem::replace(&mut *c, Value::None)).unwrap_or(Value::None)
}

// ---- weak references ----

/// Registers weakref object `w` (whose payload targets `target`).
pub(crate) fn add_weakref(target: &ObjRef, w: &ObjRef) {
    let _ = HEAP.try_with(|h| {
        let mut map = h.weakrefs.borrow_mut();
        let list = map.entry(addr_rc(target)).or_default();
        list.retain(|x| x.strong_count() > 0);
        list.push(Rc::downgrade(w));
    });
    target.gc_flags.set(target.gc_flags.get() | WEAKREFED);
}

/// Live weakref objects (including proxies) referring to `target`, oldest first.
pub fn weakrefs_of(target: &ObjRef) -> Vec<ObjRef> {
    if target.gc_flags.get() & WEAKREFED == 0 {
        return Vec::new();
    }
    HEAP.try_with(|h| h.weakrefs.borrow().get(&addr_rc(target)).map(|l| l.iter().filter_map(Weak::upgrade).collect()).unwrap_or_default()).unwrap_or_default()
}

// ---- traversal ----

/// Reports every strong reference `o` owns. Returns false when part of the
/// object could not be inspected (a `RefCell` was borrowed), in which case
/// the object must be treated as externally reachable and not cleared.
fn traverse(o: &PyObject, hooks: &Hooks, f: &mut dyn FnMut(&ObjRef)) -> bool {
    let mut ok = true;
    let val = |v: &Value, f: &mut dyn FnMut(&ObjRef)| {
        if let Value::Obj(x) = v {
            f(x)
        }
    };
    match o.class.try_borrow() {
        Ok(c) => {
            if let Some(c) = c.as_ref() {
                f(c)
            }
        }
        Err(_) => ok = false,
    }
    if let Some(d) = &o.dict {
        match d.try_borrow() {
            Ok(d) => {
                for e in d.iter() {
                    val(&e.key, f);
                    val(&e.value, f);
                }
            }
            Err(_) => ok = false,
        }
    }
    match &o.payload {
        Payload::Tuple(items) => items.iter().for_each(|v| val(v, f)),
        Payload::List(l) => match l.try_borrow() {
            Ok(l) => l.iter().for_each(|v| val(v, f)),
            Err(_) => ok = false,
        },
        Payload::Dict(d) | Payload::Set(d) => match d.try_borrow() {
            Ok(d) => {
                for e in d.iter() {
                    val(&e.key, f);
                    val(&e.value, f);
                }
            }
            Err(_) => ok = false,
        },
        Payload::FrozenSet(d) => {
            for e in d.iter() {
                val(&e.key, f);
                val(&e.value, f);
            }
        }
        Payload::Type(td) => {
            match td.bases.try_borrow() {
                Ok(b) => b.iter().for_each(|x| f(x)),
                Err(_) => ok = false,
            }
            match td.mro.try_borrow() {
                Ok(m) => m.iter().for_each(|x| f(x)),
                Err(_) => ok = false,
            }
        }
        Payload::Function(func) => {
            f(&func.globals);
            func.defaults.iter().for_each(|v| val(v, f));
            if let Some(k) = &func.kwdefaults {
                f(k);
            }
            func.closure.iter().for_each(|c| f(c));
            for cell in [&func.module, &func.doc] {
                match cell.try_borrow() {
                    Ok(v) => val(&v, f),
                    Err(_) => ok = false,
                }
            }
        }
        Payload::Builtin(b) => {
            if let Some(v) = &b.bound {
                val(v, f)
            }
        }
        Payload::Method { func, slf } => {
            val(func, f);
            val(slf, f);
        }
        Payload::Cell(c) => match c.try_borrow() {
            Ok(c) => {
                if let Some(v) = c.as_ref() {
                    val(v, f)
                }
            }
            Err(_) => ok = false,
        },
        Payload::Generator(g) => match g.try_borrow() {
            Ok(g) => {
                if let Some(fr) = &g.frame {
                    traverse_frame(fr, f);
                }
                if let Some(v) = &g.yield_from {
                    val(v, f)
                }
            }
            Err(_) => ok = false,
        },
        Payload::Property { fget, fset, fdel, doc } => {
            for v in [fget, fset, fdel, doc] {
                val(v, f);
            }
        }
        Payload::StaticMethod(v) | Payload::ClassMethod(v) => val(v, f),
        Payload::Slice(a, b, c) => {
            for v in [a, b, c] {
                val(v, f);
            }
        }
        Payload::Iter(st) => match st.try_borrow() {
            Ok(st) => traverse_iter(&st, hooks, f),
            Err(_) => ok = false,
        },
        Payload::DictView { dict, .. } => f(dict),
        Payload::Super { typ, obj } => {
            f(typ);
            val(obj, f);
        }
        Payload::WeakRef(d) => match d.callback.try_borrow() {
            Ok(c) => val(&c, f),
            Err(_) => ok = false,
        },
        Payload::Native(n) => match n.try_borrow() {
            Ok(n) => hooks.traverse(&**n, f),
            Err(_) => ok = false,
        },
        Payload::Module(_, ns) => f(ns),
        Payload::None
        | Payload::Str(_)
        | Payload::Bytes(_)
        | Payload::ByteArray(_)
        | Payload::Int(_)
        | Payload::Float(_)
        | Payload::Complex(..)
        | Payload::Range(..)
        | Payload::BigRange(_)
        | Payload::Code(_)
        | Payload::NotImplemented
        | Payload::Ellipsis => {}
    }
    ok
}

fn traverse_frame(fr: &Frame, f: &mut dyn FnMut(&ObjRef)) {
    let mut val = |v: &Value| {
        if let Value::Obj(x) = v {
            f(x)
        }
    };
    fr.stack.iter().for_each(&mut val);
    fr.fast.iter().flatten().for_each(&mut val);
    fr.blocks.iter().filter_map(|b| b.prev_exc.as_ref()).for_each(&mut val);
    fr.locals.iter().for_each(&mut val);
    fr.saved_exc.iter().for_each(&mut val);
    fr.cells.iter().for_each(|c| f(c));
    f(&fr.globals);
    f(&fr.builtins);
}

fn traverse_iter(st: &IterState, hooks: &Hooks, f: &mut dyn FnMut(&ObjRef)) {
    let val = |v: &Value, f: &mut dyn FnMut(&ObjRef)| {
        if let Value::Obj(x) = v {
            f(x)
        }
    };
    match st {
        IterState::Seq { seq, .. } | IterState::Reversed { seq, .. } => val(seq, f),
        IterState::StrChars { s, .. } => f(s),
        IterState::DictKeys { dict, .. } | IterState::DictValues { dict, .. } | IterState::DictItems { dict, .. } => f(dict),
        IterState::Enumerate { it, n } => {
            val(it, f);
            val(n, f);
        }
        IterState::Zip { its, .. } => its.iter().for_each(|v| val(v, f)),
        IterState::Map { func, its } => {
            val(func, f);
            its.iter().for_each(|v| val(v, f));
        }
        IterState::Filter { func, it, .. } => {
            val(func, f);
            val(it, f);
        }
        IterState::Callable { func, sentinel } => {
            val(func, f);
            val(sentinel, f);
        }
        IterState::Values { items, .. } => items.iter().for_each(|v| val(v, f)),
        IterState::Native(b) => hooks.traverse(&**b, f),
        IterState::Range { .. } | IterState::BigRange { .. } | IterState::Exhausted => {}
    }
}

/// Moves the references out of an unreachable object (`tp_clear`). The
/// contents are dropped by the caller after every object was cleared.
fn clear(o: &PyObject, hooks: &Hooks, grave: &mut Vec<Box<dyn Any>>) {
    if let Some(d) = &o.dict {
        if let Ok(mut d) = d.try_borrow_mut() {
            grave.push(Box::new(std::mem::take(&mut *d)));
        }
    }
    match &o.payload {
        Payload::List(l) => {
            if let Ok(mut l) = l.try_borrow_mut() {
                grave.push(Box::new(std::mem::take(&mut *l)));
            }
        }
        Payload::Dict(d) | Payload::Set(d) => {
            if let Ok(mut d) = d.try_borrow_mut() {
                grave.push(Box::new(std::mem::take(&mut *d)));
            }
        }
        Payload::Cell(c) => {
            if let Ok(mut c) = c.try_borrow_mut() {
                grave.push(Box::new(c.take()));
            }
        }
        Payload::Generator(g) => {
            if let Ok(mut g) = g.try_borrow_mut() {
                grave.push(Box::new(g.frame.take()));
                grave.push(Box::new(g.yield_from.take()));
                g.status = GenStatus::Finished;
            }
        }
        Payload::Iter(st) => {
            if let Ok(mut st) = st.try_borrow_mut() {
                grave.push(Box::new(std::mem::replace(&mut *st, IterState::Exhausted)));
            }
        }
        Payload::Function(func) => {
            for cell in [&func.module, &func.doc] {
                if let Ok(mut v) = cell.try_borrow_mut() {
                    grave.push(Box::new(std::mem::replace(&mut *v, Value::None)));
                }
            }
        }
        Payload::Type(td) => {
            if let Ok(mut b) = td.bases.try_borrow_mut() {
                grave.push(Box::new(std::mem::take(&mut *b)));
            }
            if let Ok(mut m) = td.mro.try_borrow_mut() {
                grave.push(Box::new(std::mem::take(&mut *m)));
            }
        }
        Payload::WeakRef(d) => {
            if let Ok(mut c) = d.callback.try_borrow_mut() {
                grave.push(Box::new(std::mem::replace(&mut *c, Value::None)));
            }
        }
        Payload::Native(n) => {
            if let Ok(mut n) = n.try_borrow_mut() {
                hooks.clear(&mut **n);
            }
        }
        _ => {}
    }
}

/// Indices of `objs[sel]` that are reachable from outside the selection:
/// `strong_count - 1` (the collector's handle) minus the references the
/// selection holds among itself, then propagation along references.
fn reachable_in(objs: &[ObjRef], sel: &[usize], hooks: &Hooks) -> Vec<bool> {
    let n = sel.len();
    let mut refs: Vec<isize> = Vec::with_capacity(n);
    for (k, &i) in sel.iter().enumerate() {
        let o = &objs[i];
        o.gc_index.set(k as u32);
        o.gc_flags.set(o.gc_flags.get() | IN_SET);
        refs.push(Rc::strong_count(o) as isize - 1);
    }
    let member = |c: &ObjRef| (c.gc_flags.get() & IN_SET != 0).then(|| c.gc_index.get() as usize);
    let mut root = vec![false; n];
    for (k, &i) in sel.iter().enumerate() {
        let complete = traverse(&objs[i], hooks, &mut |c| {
            if let Some(j) = member(c) {
                refs[j] -= 1;
            }
        });
        if !complete {
            root[k] = true;
        }
    }
    let mut reach = vec![false; n];
    let mut stack = Vec::new();
    for k in 0..n {
        debug_assert!(refs[k] >= 0, "gc: traversal reported more references than strong_count");
        if refs[k] != 0 || root[k] {
            reach[k] = true;
            stack.push(k);
        }
    }
    while let Some(k) = stack.pop() {
        traverse(&objs[sel[k]], hooks, &mut |c| {
            if let Some(j) = member(c) {
                if !reach[j] {
                    reach[j] = true;
                    stack.push(j);
                }
            }
        });
    }
    for &i in sel {
        let o = &objs[i];
        o.gc_flags.set(o.gc_flags.get() & !IN_SET);
    }
    reach
}

// ---- collection ----

/// Collects generations `0..=generation` and returns the number of
/// unreachable objects found (CPython `gc.collect`). Returns 0 when a
/// collection is already running.
pub fn collect(vm: &mut Vm, generation: usize) -> usize {
    let generation = generation.min(NUM_GENERATIONS - 1);
    let busy = HEAP.with(|h| h.collecting.replace(true));
    if busy {
        return 0;
    }
    invoke_callbacks(vm, "start", generation, 0, 0);
    let (found, uncollectable) = collect_inner(vm, generation);
    HEAP.with(|h| {
        let mut s = h.stats[generation].get();
        s.collections += 1;
        s.collected += found - uncollectable;
        s.uncollectable += uncollectable;
        h.stats[generation].set(s);
        h.collecting.set(false);
    });
    invoke_callbacks(vm, "stop", generation, found - uncollectable, uncollectable);
    found
}

fn collect_inner(vm: &mut Vm, generation: usize) -> (usize, usize) {
    let hooks = Hooks::snapshot();
    let debug = HEAP.with(|h| h.debug.get());
    let started = std::time::Instant::now();
    // Take the candidate generations and reset the counters.
    let objs = HEAP.with(|h| {
        // An object is in exactly one list; a duplicate would only make it
        // look externally referenced (its other handle), never freeable.
        let mut objs = Vec::new();
        let mut pruned = 0usize;
        for g in 0..=generation {
            let list = std::mem::take(&mut *h.gens[g].borrow_mut());
            objs.reserve(list.len());
            for w in list {
                match w.upgrade() {
                    Some(o) => objs.push(o),
                    None => pruned += 1,
                }
            }
            h.count[g].set(0);
        }
        if generation + 1 < NUM_GENERATIONS {
            let c = &h.count[generation + 1];
            c.set(c.get() + 1);
        }
        h.dead.set(h.dead.get().saturating_sub(pruned));
        objs
    });
    if debug & DEBUG_STATS != 0 {
        let sizes = HEAP.with(|h| h.gens.iter().map(|g| g.borrow().len()).collect::<Vec<_>>());
        report(vm, &format!("gc: collecting generation {generation}...\ngc: objects in each generation: {} {} {}\n", objs.len().max(sizes[0]), sizes[1], sizes[2]));
    }
    let all: Vec<usize> = (0..objs.len()).collect();
    let reach = reachable_in(&objs, &all, &hooks);
    let older = (generation + 1).min(NUM_GENERATIONS - 1);
    let mut survivors: Vec<usize> = Vec::new();
    let mut garbage: Vec<usize> = Vec::new();
    for (i, r) in reach.iter().enumerate() {
        if *r {
            survivors.push(i)
        } else {
            garbage.push(i)
        }
    }
    let found = garbage.len();
    let mut uncollectable = 0;
    let mut freed = Vec::new();
    if !garbage.is_empty() {
        if debug & DEBUG_COLLECTABLE != 0 {
            for &i in &garbage {
                let s = describe(&objs[i]);
                report(vm, &format!("gc: collectable {s}\n"));
            }
        }
        if debug & DEBUG_SAVEALL != 0 {
            if let Payload::List(l) = &vm.gc_garbage.payload {
                l.borrow_mut().extend(garbage.iter().map(|&i| Value::Obj(objs[i].clone())));
            }
            survivors.append(&mut garbage);
        } else {
            freed = handle_garbage(vm, &objs, garbage, &mut survivors, &hooks);
            uncollectable = 0;
        }
    }
    // Promote survivors.
    HEAP.with(|h| {
        let mut dst = h.gens[older].borrow_mut();
        dst.extend(survivors.iter().map(|&i| Rc::downgrade(&objs[i])));
        if generation == NUM_GENERATIONS - 1 {
            h.long_lived_pending.set(0);
            h.long_lived_total.set(dst.len());
        } else if older == NUM_GENERATIONS - 1 {
            h.long_lived_pending.set(h.long_lived_pending.get() + survivors.len());
        }
    });
    if debug & DEBUG_STATS != 0 {
        report(vm, &format!("gc: done, {} unreachable, {} uncollectable, {:.4}s elapsed\n", found, uncollectable, started.elapsed().as_secs_f64()));
    }
    drop(freed);
    drop(objs);
    (found, uncollectable)
}

/// Weakref clearing, finalizers, resurrection check and clearing for the
/// unreachable objects `garbage`. Returns the cleared contents.
fn handle_garbage(vm: &mut Vm, objs: &[ObjRef], garbage: Vec<usize>, survivors: &mut Vec<usize>, hooks: &Hooks) -> Vec<Box<dyn Any>> {
    let mut grave: Vec<Box<dyn Any>> = Vec::new();
    // Weak references to garbage are cleared first so no callback or
    // finalizer can reach it through them; only callbacks of weakrefs that
    // are themselves reachable run (CPython handle_weakrefs).
    let in_garbage: PtrMap<()> = garbage.iter().map(|&i| (addr_rc(&objs[i]), ())).collect();
    let mut calls = Vec::new();
    for &i in &garbage {
        let o = &objs[i];
        if o.gc_flags.get() & WEAKREFED == 0 {
            continue;
        }
        o.gc_flags.set(o.gc_flags.get() & !WEAKREFED);
        let refs = HEAP.with(|h| h.weakrefs.borrow_mut().remove(&addr_rc(o)));
        for w in refs.unwrap_or_default().iter().rev().filter_map(Weak::upgrade) {
            let cb = clear_weakref(&w);
            if cb.is_none() {
                continue;
            }
            if in_garbage.contains_key(&addr_rc(&w)) {
                grave.push(Box::new(cb));
            } else {
                calls.push((w, cb));
            }
        }
    }
    for (w, cb) in calls {
        call_weakref_callback(vm, &cb, w);
    }
    // Finalizers (PEP 442), once per object.
    let del_assigned = HEAP.with(|h| h.del_assigned.get());
    let mut finalized_any = false;
    for &i in &garbage {
        let o = &objs[i];
        if o.gc_flags.get() & FINALIZED != 0 {
            continue;
        }
        let fin = finalizer_of(o, del_assigned);
        if fin == Finalizer::None {
            continue;
        }
        o.gc_flags.set(o.gc_flags.get() | FINALIZED);
        run_finalizer(vm, o, fin);
        finalized_any = true;
    }
    // Anything a finalizer made reachable again survives.
    let mut dead = garbage;
    if finalized_any {
        let reach = reachable_in(objs, &dead, hooks);
        let mut still = Vec::with_capacity(dead.len());
        for (k, i) in dead.into_iter().enumerate() {
            if reach[k] {
                survivors.push(i)
            } else {
                still.push(i)
            }
        }
        dead = still;
    }
    for &i in &dead {
        clear(&objs[i], hooks, &mut grave);
    }
    grave
}

fn describe(o: &ObjRef) -> String {
    format!("<{} {:#x}>", o.class_name(), addr_rc(o))
}

fn invoke_callbacks(vm: &mut Vm, phase: &str, generation: usize, collected: usize, uncollectable: usize) {
    let cbs = match &vm.gc_callbacks.payload {
        Payload::List(l) => l.borrow().clone(),
        _ => return,
    };
    if cbs.is_empty() {
        return;
    }
    let phase = vm.types.new_str(phase);
    let info = vm.types.new_dict_obj(Dict::new());
    for (k, v) in [("generation", generation), ("collected", collected), ("uncollectable", uncollectable)] {
        let key = vm.types.intern(k);
        info.dict_set_raw(&vm.types, key, Value::Int(v as i64));
    }
    for cb in cbs {
        if let Err(e) = vm.call(&cb, vec![phase.clone(), Value::Obj(info.clone())], Vec::new()) {
            report_unraisable(vm, e, &cb);
        }
    }
}

// ---- safe points ----

/// Cheap check for the interpreter loop: some thread has queued work.
#[inline]
pub fn pending_hint() -> bool {
    PENDING_THREADS.load(Ordering::Relaxed) != 0
}

/// Runs a due automatic collection, then queued finalizers and weakref
/// callbacks. Must only be called where no Rust code holds borrows into
/// Python objects that Python code could need (see the module docs).
pub fn safe_point(vm: &mut Vm) {
    let go = HEAP.try_with(|h| {
        if h.draining.get() || h.collecting.get() || !h.signalled.get() {
            return false;
        }
        h.draining.set(true);
        h.unsignal();
        true
    });
    if go != Ok(true) {
        return;
    }
    loop {
        if let Some(g) = HEAP.with(|h| h.auto_generation()) {
            collect(vm, g);
            continue;
        }
        let fin = HEAP.with(|h| h.finalize.borrow_mut().pop_front());
        if let Some(o) = fin {
            let kind = finalizer_of(&o, true);
            run_finalizer(vm, &o, if kind == Finalizer::None { Finalizer::Del } else { kind });
            continue;
        }
        let cb = HEAP.with(|h| h.callbacks.borrow_mut().pop_front());
        if let Some((w, f)) = cb {
            call_weakref_callback(vm, &f, w);
            continue;
        }
        break;
    }
    HEAP.with(|h| h.draining.set(false));
}

fn run_finalizer(vm: &mut Vm, o: &ObjRef, fin: Finalizer) {
    let saved = vm.exc_state.take();
    let target = Value::Obj(o.clone());
    let r = match fin {
        Finalizer::GenClose => vm.call_method(&target, "close", Vec::new()).map(|_| ()),
        _ => match crate::ops::lookup_type(&o.class(), "__del__") {
            Some(del) => match crate::ops::call_special(vm, &target, "__del__", Vec::new()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    vm.exc_state = saved;
                    report_unraisable(vm, e, &del);
                    return;
                }
            },
            None => Ok(()),
        },
    };
    vm.exc_state = saved;
    if let Err(e) = r {
        report_unraisable(vm, e, &target);
    }
}

fn call_weakref_callback(vm: &mut Vm, cb: &Value, w: ObjRef) {
    let saved = vm.exc_state.take();
    let r = vm.call(cb, vec![Value::Obj(w)], Vec::new());
    vm.exc_state = saved;
    if let Err(e) = r {
        report_unraisable(vm, e, cb);
    }
}

/// Reports an exception that cannot propagate ("Exception ignored in").
pub fn report_unraisable(vm: &mut Vm, err: PyErr, obj: &Value) {
    let PyErr::Exc(exc) = err else { return };
    let what = crate::ops::repr(vm, obj).unwrap_or_else(|_| "<object>".into());
    let text = format!("Exception ignored in: {what}\n{}", crate::format_exception(vm, &exc));
    report(vm, &text);
}

/// Writes collector diagnostics to `sys.stderr` (falling back to the
/// process stderr before `sys` exists).
fn report(vm: &mut Vm, text: &str) {
    let saved = vm.exc_state.take();
    let err = vm.modules.dict_get_str_raw("sys").and_then(|s| crate::ops::get_raw_attr(s.obj()?, "stderr"));
    let ok = match err {
        Some(e) => {
            let t = vm.types.new_str(text);
            vm.call_method(&e, "write", vec![t]).is_ok()
        }
        None => false,
    };
    if !ok {
        eprint!("{text}");
    }
    vm.exc_state = saved;
}

// ---- introspection (gc module) ----

pub fn is_enabled() -> bool {
    HEAP.with(|h| h.enabled.get())
}

pub fn set_enabled(on: bool) {
    HEAP.with(|h| {
        h.enabled.set(on);
        if h.collection_due() {
            h.signal();
        }
    });
}

pub fn count() -> [usize; NUM_GENERATIONS] {
    HEAP.with(|h| [h.count[0].get(), h.count[1].get(), h.count[2].get()])
}

pub fn threshold() -> [usize; NUM_GENERATIONS] {
    HEAP.with(|h| [h.threshold[0].get(), h.threshold[1].get(), h.threshold[2].get()])
}

pub fn set_threshold(t: &[usize]) {
    HEAP.with(|h| {
        for (g, v) in t.iter().take(NUM_GENERATIONS).enumerate() {
            h.threshold[g].set(*v);
        }
    });
}

pub fn debug_flags() -> u32 {
    HEAP.with(|h| h.debug.get())
}

pub fn set_debug_flags(f: u32) {
    HEAP.with(|h| h.debug.set(f));
}

pub fn stats() -> [GenStats; NUM_GENERATIONS] {
    HEAP.with(|h| [h.stats[0].get(), h.stats[1].get(), h.stats[2].get()])
}

/// Live tracked objects of one generation, or of all generations.
pub fn objects(generation: Option<usize>) -> Vec<ObjRef> {
    HEAP.with(|h| {
        let range = match generation {
            Some(g) => g..g + 1,
            None => 0..NUM_GENERATIONS,
        };
        h.gens[range].iter().flat_map(|g| g.borrow().iter().filter_map(Weak::upgrade).collect::<Vec<_>>()).collect()
    })
}

fn all_tracked() -> Vec<ObjRef> {
    let mut v = objects(None);
    HEAP.with(|h| v.extend(h.permanent.borrow().iter().filter_map(Weak::upgrade)));
    v
}

/// Objects directly referred to by `o` (CPython `gc.get_referents`).
pub fn referents(o: &ObjRef) -> Vec<ObjRef> {
    let hooks = Hooks::snapshot();
    let mut out = Vec::new();
    traverse(o, &hooks, &mut |c| out.push(c.clone()));
    out
}

/// Tracked objects that refer to any of `targets`.
pub fn referrers(targets: &[Value]) -> Vec<ObjRef> {
    let hooks = Hooks::snapshot();
    let keys: PtrMap<()> = targets.iter().filter_map(|v| v.obj()).map(|o| (addr_rc(o), ())).collect();
    let mut out = Vec::new();
    for o in all_tracked() {
        let mut hit = false;
        traverse(&o, &hooks, &mut |c| hit |= keys.contains_key(&addr_rc(c)));
        if hit {
            out.push(o);
        }
    }
    out
}

pub fn is_tracked(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if o.gc_flags.get() & TRACKED != 0)
}

pub fn is_finalized(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if o.gc_flags.get() & FINALIZED != 0)
}

/// Moves every tracked object to the permanent generation.
pub fn freeze() {
    HEAP.with(|h| {
        let mut p = h.permanent.borrow_mut();
        for g in &h.gens {
            p.extend(std::mem::take(&mut *g.borrow_mut()));
        }
        p.retain(|w| w.strong_count() > 0);
        h.count[0].set(0);
        h.long_lived_total.set(0);
        h.long_lived_pending.set(0);
    });
}

/// Returns the permanent generation to the oldest generation.
pub fn unfreeze() {
    HEAP.with(|h| {
        let p = std::mem::take(&mut *h.permanent.borrow_mut());
        h.gens[NUM_GENERATIONS - 1].borrow_mut().extend(p);
    });
}

pub fn freeze_count() -> usize {
    HEAP.with(|h| h.permanent.borrow().iter().filter(|w| w.strong_count() > 0).count())
}
