// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::{input_buffer::*, service::InputFrame};
fn f(down: u64, pressed: u64, x: i32) -> InputFrame {
    InputFrame {
        buttons_down: down,
        buttons_pressed: pressed,
        pointer_x: x,
        pointer_y: 7,
    }
}
#[test]
fn held_edges_and_short_pulses_are_ordered_and_acknowledged_exactly_once() {
    let mut q = InputBuffer::new(1, 9, 8).unwrap();
    q.push(1, 0, f(1, 0, 10)).unwrap();
    q.push(1, 1, f(0, 0, 11)).unwrap();
    q.push(1, 1, f(0, 2, 12)).unwrap();
    let b = q.read(1, 8).unwrap();
    assert_eq!(b.samples.len(), 3);
    assert_eq!(
        (b.samples[0].frame.buttons_pressed, b.samples[0].released),
        (1, 0)
    );
    assert_eq!(
        (b.samples[1].frame.buttons_pressed, b.samples[1].released),
        (0, 1)
    );
    assert_eq!(
        (b.samples[2].frame.buttons_pressed, b.samples[2].released),
        (2, 2)
    );
    assert_eq!(q.read(1, 8).unwrap(), b);
    let r = b.receipt.unwrap();
    q.acknowledge(1, r).unwrap();
    assert!(q.read(1, 8).unwrap().samples.is_empty());
    assert!(q.acknowledge(1, r).is_err());
    for t in 2..1000 {
        q.push(1, t, f(0, 0, 12)).unwrap();
    }
    assert!(q.is_empty());
}
#[test]
fn unleased_motion_coalesces_but_published_samples_and_click_coordinates_stay_immutable() {
    let mut q = InputBuffer::new(1, 1, 8).unwrap();
    q.push(1, 0, f(0, 0, 0)).unwrap();
    let first = q.read(1, 1).unwrap();
    q.push(1, 1, f(0, 0, 10)).unwrap();
    q.push(1, 2, f(0, 0, 20)).unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q.read(1, 1).unwrap(), first);
    q.push(1, 3, f(1, 1, 30)).unwrap();
    q.push(1, 4, f(1, 0, 40)).unwrap();
    q.push(1, 5, f(0, 0, 50)).unwrap();
    let b = q.read(1, 8).unwrap();
    assert_eq!(
        b.samples
            .iter()
            .map(|s| s.frame.pointer_x)
            .collect::<Vec<_>>(),
        vec![0, 20, 30, 40, 50]
    );
    assert_eq!(b.samples[2].frame.buttons_pressed, 1);
    assert_eq!(b.samples[4].released, 1);
}
#[test]
fn overflow_wrong_owners_receipts_and_backwards_time_leave_the_queue_unchanged() {
    let mut q = InputBuffer::new(1, 7, 2).unwrap();
    q.push(1, 4, f(1, 1, 0)).unwrap();
    q.push(1, 5, f(0, 0, 0)).unwrap();
    let before = q.snapshot();
    assert!(q.push(1, 6, f(0, 1, 0)).is_err());
    assert!(q.push(2, 6, f(0, 1, 0)).is_err());
    assert!(q.push(1, 3, f(0, 0, 0)).is_err());
    assert!(q.read(2, 1).is_err());
    assert!(q.read(1, 0).is_err());
    assert_eq!(q.snapshot(), before);
    let r = q.read(1, 1).unwrap().receipt.unwrap();
    let leased = q.snapshot();
    assert!(q.acknowledge(1, InputReceipt { epoch: 8, ..r }).is_err());
    assert!(
        q.acknowledge(
            1,
            InputReceipt {
                sequence: r.sequence + 1,
                ..r
            }
        )
        .is_err()
    );
    assert_eq!(q.snapshot(), leased);
    q.acknowledge(1, r).unwrap();
    q.push(1, 6, f(0, 1, 0)).unwrap();
    assert_eq!(q.len(), 2);
}
#[test]
fn portable_snapshot_replays_the_unconsumed_prefix_and_invalidates_old_receipts_with_a_new_epoch() {
    let mut q = InputBuffer::new(1, 1, 4).unwrap();
    q.push(1, 1, f(1, 0, 10)).unwrap();
    q.push(1, 2, f(0, 0, 11)).unwrap();
    let b = q.read(1, 1).unwrap();
    q.acknowledge(1, b.receipt.unwrap()).unwrap();
    let snapshot = q.snapshot();
    snapshot.validate().unwrap();
    let mut restored = InputBuffer::restore(1, 2, snapshot.clone()).unwrap();
    assert_eq!(restored.snapshot(), snapshot);
    assert!(restored.acknowledge(1, b.receipt.unwrap()).is_err());
    let r = restored.read(1, 4).unwrap();
    assert_eq!(r.samples.len(), 1);
    assert_eq!(r.samples[0].released, 1);
    restored.acknowledge(1, r.receipt.unwrap()).unwrap();
    for variant in 0..5 {
        let mut bad = snapshot.clone();
        match variant {
            0 => bad.schema = 2,
            1 => bad.samples[0].released = 0,
            2 => bad.samples[0].sequence = 99,
            3 => bad.observed.as_mut().unwrap().buttons_pressed = 1,
            _ => bad.capacity = 0,
        };
        assert!(InputBuffer::restore(1, 3, bad).is_err());
    }
}
#[test]
fn discard_drops_old_clicks_without_synthesizing_a_press_for_an_already_held_key() {
    let mut q = InputBuffer::new(1, 1, 4).unwrap();
    q.push(1, 1, f(1, 1, 0)).unwrap();
    let old = q.read(1, 4).unwrap().receipt.unwrap();
    q.discard(1).unwrap();
    q.snapshot().validate().unwrap();
    assert!(q.acknowledge(1, old).is_err());
    q.push(1, 2, f(1, 0, 0)).unwrap();
    assert!(q.is_empty());
    q.push(1, 3, f(0, 0, 0)).unwrap();
    let b = q.read(1, 4).unwrap();
    assert_eq!(b.samples[0].frame.buttons_pressed, 0);
    assert_eq!(b.samples[0].released, 1);
}
