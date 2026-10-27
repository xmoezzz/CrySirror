// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::region_map::RegionMap;
#[test]
fn clipped_integer_regions_overlap_clear_and_keep_client_authority() {
    let mut m = RegionMap::new(2, [20, 10]).unwrap();
    m.fill(2, [-2, -2, 8, 8], 7).unwrap();
    m.fill(2, [3, 3, 2, 2], 9).unwrap();
    assert_eq!(m.get(2, [0, 0]).unwrap(), 7);
    assert_eq!(m.get(2, [3, 3]).unwrap(), 9);
    assert_eq!(m.get(2, [6, 6]).unwrap(), 0);
    assert_eq!(m.get(2, [-1, 0]).unwrap(), 0);
    assert!(m.fill(1, [0, 0, 20, 10], 0).is_err());
    assert!(m.get(1, [0, 0]).is_err());
    m.fill(2, [0, 0, 20, 10], 0).unwrap();
    assert_eq!(m.get(2, [3, 3]).unwrap(), 0);
}
#[test]
fn region_budget_failure_is_atomic_and_full_clear_recovers() {
    let mut m = RegionMap::new(1, [20, 20]).unwrap();
    for n in 0..4096 {
        m.fill(1, [1, 1, 1, 1], n).unwrap();
    }
    assert!(m.fill(1, [1, 1, 1, 1], 99).is_err());
    assert_eq!(m.get(1, [1, 1]).unwrap(), 4095);
    m.fill(1, [0, 0, 20, 20], 0).unwrap();
    m.fill(1, [1, 1, 1, 1], 8).unwrap();
    assert_eq!(m.get(1, [1, 1]).unwrap(), 8);
}

#[test]
fn reflected_regions_preserve_overlap_and_owner() {
    let mut r = RegionMap::new(7, [8, 6]).unwrap();
    r.fill(7, [0, 0, 4, 3], 1).unwrap();
    r.fill(7, [2, 1, 1, 1], 2).unwrap();
    assert!(r.flip(8, true, true).is_err());
    r.flip(7, true, true).unwrap();
    assert_eq!(r.get(7, [5, 4]).unwrap(), 2);
    assert_eq!(r.get(7, [7, 5]).unwrap(), 1);
    assert_eq!(r.get(7, [0, 0]).unwrap(), 0);
    r.flip(7, true, true).unwrap();
    assert_eq!(r.get(7, [2, 1]).unwrap(), 2);
}
