// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::deferred_dispatch::{DeferredDispatch, DispatchPhase::*};
#[test]
fn scoped_phases_and_turn_fences_keep_recursive_notifications_bounded() {
    let mut q = DeferredDispatch::new(8).unwrap();
    q.post(1, 1, Normal, false).unwrap();
    q.post(2, 1, Exclusive, false).unwrap();
    q.post(1, 2, Idle, false).unwrap();
    q.post(1, 3, Exclusive, false).unwrap();
    let fence = q.fence();
    assert_eq!(q.pop(1, fence, Exclusive), Some(3));
    assert_eq!(q.pop(1, fence, Normal), Some(1));
    q.post(1, 1, Normal, false).unwrap();
    assert_eq!(q.pop(1, fence, Normal), None);
    assert_eq!(q.pop(1, fence, Idle), Some(2));
    assert_eq!(q.pop(2, fence, Exclusive), Some(1));
    assert_eq!(q.pop(1, q.fence(), Normal), Some(1));
}
#[test]
fn cached_replacement_capacity_and_cancellation_preserve_other_owners() {
    let mut q = DeferredDispatch::new(2).unwrap();
    q.post(1, 1, Normal, false).unwrap();
    q.post(2, 1, Normal, false).unwrap();
    let f = q.fence();
    assert!(q.post(1, 2, Idle, false).is_err());
    assert_eq!(q.fence(), f);
    assert!(q.post(0, 1, Idle, true).is_err());
    q.post(1, 1, Idle, true).unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q.pop(1, f, Idle), None);
    assert_eq!(q.cancel(1, 1), 1);
    assert_eq!(q.pop(2, q.fence(), Normal), Some(1));
    assert!(q.is_empty());
}
