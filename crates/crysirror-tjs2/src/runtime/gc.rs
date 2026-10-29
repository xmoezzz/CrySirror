// CrySirror addition (not upstream): mark-and-sweep reclamation for the
// append-only object heap. krkrz frees objects by reference count; without
// reclamation a session's temporaries (arrays, dictionaries) grow unbounded.
//! Roots: the global object, shared natives, array extensions, script code
//! objects, the native caller's `this`, every suspended frame, everything the
//! host reports through [`TjsHost::gc_roots`], and extra values the caller
//! passes. Function, native and code objects are always roots (conservative
//! about handles captured inside native closures). Unreachable objects become
//! invalid tombstones whose slots are reused.
use super::*;
use crate::runtime::object::{Object, ObjectKind};

impl<H: TjsHost + 'static> Runtime<H> {
    /// Collect garbage. Only valid outside script execution (no live VM
    /// frames on the Rust stack); returns the number of freed objects, or 0
    /// when called during execution.
    pub fn collect_garbage(&mut self, extra: &[Variant]) -> usize {
        if self.call_depth != 0 {
            return 0;
        }
        let t0 = std::time::Instant::now();
        let n = self.heap.len();
        self.freed.resize(n, false);
        let mut marked = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut push_handle = |h: ObjectHandle, marked: &mut Vec<bool>, stack: &mut Vec<usize>| {
            if h.0 < marked.len() && !marked[h.0] {
                marked[h.0] = true;
                stack.push(h.0);
            }
        };
        fn value_handles(v: &Variant, out: &mut Vec<ObjectHandle>) {
            match v {
                Variant::Object(h) => out.push(*h),
                Variant::Closure(c) => {
                    out.push(c.object);
                    if let Some(t) = c.this_obj {
                        out.push(t);
                    }
                }
                _ => {}
            }
        }
        let mut roots: Vec<ObjectHandle> = vec![ObjectHandle(0), self.global];
        roots.extend(self.shared_natives.values().copied());
        for v in self.array_extensions.values().chain(extra).chain(self.resume_value.as_ref()) {
            value_handles(v, &mut roots);
        }
        for file in &self.script_files {
            roots.extend(file.code_handles.iter().copied());
        }
        roots.extend(self.native_caller_this);
        if let Some(suspended) = &self.suspended_call {
            let (mut from_values, mut handles) = (Vec::new(), Vec::new());
            suspended.for_each_root(&mut |v| value_handles(v, &mut from_values), &mut |h| handles.push(h));
            roots.extend(from_values);
            roots.extend(handles);
        }
        self.host.gc_roots(&mut roots);
        let t1 = t0.elapsed();
        for (i, object) in self.heap.iter().enumerate() {
            if !self.freed[i]
                && matches!(
                    object.kind,
                    ObjectKind::NativeFunction { .. }
                        | ObjectKind::VmNativeFunction { .. }
                        | ObjectKind::NativeProperty { .. }
                        | ObjectKind::InterCode { .. }
                )
            {
                roots.push(ObjectHandle(i));
            }
        }
        for h in roots {
            push_handle(h, &mut marked, &mut stack);
        }
        let mut children = Vec::new();
        while let Some(i) = stack.pop() {
            let object = &self.heap[i];
            children.clear();
            match &object.kind {
                ObjectKind::Proxy { primary, fallback, bind_this } => {
                    children.extend(*primary);
                    children.push(*fallback);
                    children.extend(*bind_this);
                }
                ObjectKind::Array { elements } => {
                    for v in elements {
                        value_handles(v, &mut children);
                    }
                }
                _ => {}
            }
            object.members.for_each_value(|v| value_handles(v, &mut children));
            children.extend(object.super_class);
            for h in children.drain(..) {
                push_handle(h, &mut marked, &mut stack);
            }
        }
        let t2 = t0.elapsed();
        // The this-proxy cache is weak in its keys.
        self.this_proxies.retain(|key, proxy| marked[*key] && marked[proxy.0]);
        // Diagnostic: TJS_GC_NO_REUSE keeps freed slots as invalid tombstones
        // forever, so a missing root shows as an invalid-object error instead
        // of silently aliasing a newer object.
        let reuse = std::env::var_os("TJS_GC_NO_REUSE").is_none();
        let mut count = 0;
        let mut dead = Vec::new();
        for i in 1..n {
            if !marked[i] && !self.freed[i] {
                dead.push(std::mem::replace(&mut self.heap[i], Object::tombstone()));
                self.freed[i] = true;
                if reuse {
                    self.free_slots.push(i);
                }
                count += 1;
                // Like a reference-count release reaching zero: weakly held
                // native state keyed by the object goes with it.
                self.host.object_collected(ObjectHandle(i));
            }
        }
        // Releasing the dead objects' storage is most of the sweep cost and
        // touches nothing the runtime still uses: do it off this thread.
        std::thread::spawn(move || drop(dead));
        if std::env::var_os("TJS_GC_TIMING").is_some() {
            eprintln!("[gc-timing] roots {t1:?} mark {:?} sweep {:?}", t2 - t1, t0.elapsed() - t2);
        }
        count
    }

    /// Heap slots currently holding live (not reclaimed) objects.
    pub fn live_object_count(&self) -> usize {
        self.heap.len() - self.free_slots.len()
    }
}
