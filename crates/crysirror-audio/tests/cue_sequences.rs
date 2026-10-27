// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::sequence::*;
fn line(key: &str, delay: i64) -> Line {
    Line {
        key: key.into(),
        weight: 1,
        min_ms: delay,
        max_ms: delay,
    }
}
#[test]
fn dependency_receipt_freezes_selected_clock_and_once_waits_last_interval() {
    let mut bank = Sequences::default();
    let h = bank
        .create(
            1,
            Spec {
                include_duration: true,
                lines: vec![line("a", 10), line("b", 20)],
                ..Default::default()
            },
            42,
        )
        .unwrap();
    let first = bank.poll(1, h, None).unwrap().unwrap();
    let before = bank.info(1, h).unwrap();
    for _ in 0..30 {
        bank.advance(1, h, 20, 30).unwrap();
        assert_eq!(bank.poll(1, h, Some(99)).unwrap(), Some(first.clone()));
    }
    assert_eq!(bank.info(1, h).unwrap(), before);
    assert!(bank.complete(1, h, first.serial + 1, 3).is_err());
    assert!(bank.complete(1, h, first.serial, u64::MAX).is_err());
    assert_eq!(bank.info(1, h).unwrap(), before);
    bank.complete(1, h, first.serial, 3).unwrap();
    bank.advance(1, h, 0, 100).unwrap(); // Game clock ignores real time.
    assert!(bank.poll(1, h, None).unwrap().is_none());
    bank.advance(1, h, 12, 0).unwrap();
    assert!(bank.poll(1, h, None).unwrap().is_none());
    bank.advance(1, h, 1, 0).unwrap();
    let second = bank.poll(1, h, None).unwrap().unwrap();
    assert_eq!(second.key, "b");
    bank.complete(1, h, second.serial, 5).unwrap();
    bank.advance(1, h, 24, 24).unwrap();
    assert!(bank.info(1, h).unwrap().active);
    bank.advance(1, h, 1, 1).unwrap();
    assert!(!bank.info(1, h).unwrap().active);
    assert!(bank.poll(1, h, None).unwrap().is_none());
}
#[test]
fn carry_catches_up_but_reset_discards_overdue_time() {
    for (timing, want) in [(Timing::Reset, 1), (Timing::Carry, 6)] {
        let mut bank = Sequences::default();
        let h = bank
            .create(
                1,
                Spec {
                    mode: Mode::Loop,
                    timing,
                    clock: Clock::Real,
                    lines: vec![line("a", 10)],
                    ..Default::default()
                },
                1,
            )
            .unwrap();
        bank.advance(1, h, 0, 50).unwrap();
        let mut count = 0;
        while let Some(e) = bank.poll(1, h, None).unwrap() {
            count += 1;
            bank.complete(1, h, e.serial, 0).unwrap();
        }
        assert_eq!(count, want);
    }
}
#[test]
fn modulo_profile_matches_original_weight_bias_and_draw_order() {
    let mut a = line("a", 10);
    a.max_ms = 20;
    let mut b = line("b", 20);
    b.max_ms = 10;
    b.weight = 2;
    let mut bank = Sequences::default();
    let h = bank
        .create(
            1,
            Spec {
                mode: Mode::Weighted,
                random: Random::Modulo32,
                soften_repeat: true,
                lines: vec![a, b],
                ..Default::default()
            },
            1,
        )
        .unwrap();
    let e = bank.poll(1, h, None).unwrap().unwrap();
    assert_eq!(e.index, 1); // xorshift(1)=270369; modulo 6 => 3.
    assert_eq!(bank.info(1, h).unwrap().rng_state, 270369);
    bank.complete(1, h, e.serial, 0).unwrap();
    assert_eq!(bank.info(1, h).unwrap().next_ms, 19); // 67634689 modulo 10.
    bank.advance(1, h, 19, 0).unwrap();
    let e = bank.poll(1, h, None).unwrap().unwrap();
    assert_eq!(e.index, 0); // 2647435461 modulo 4 => 1; previous b has weight 2.
    bank.complete(1, h, e.serial, 0).unwrap();
    assert_eq!(bank.info(1, h).unwrap().next_ms, 15);
    assert_eq!(bank.info(1, h).unwrap().rng_state, 307599695);
}
#[test]
fn uniform_weighted_draws_are_reproducible_and_zero_weights_are_never_selected() {
    let mut a = line("a", 1);
    a.weight = 3;
    let b = line("b", 1);
    let mut zero = line("zero", 1);
    zero.weight = 0;
    let spec = Spec {
        mode: Mode::Weighted,
        lines: vec![a, b, zero],
        ..Default::default()
    };
    let mut bank = Sequences::default();
    let h = bank.create(1, spec.clone(), 0).unwrap();
    let h2 = bank.create(2, spec, 0).unwrap();
    let mut a_count = 0;
    for _ in 0..10000 {
        bank.advance(1, h, 1, 1).unwrap();
        bank.advance(2, h2, 1, 1).unwrap();
        let a = bank.poll(1, h, None).unwrap().unwrap();
        let b = bank.poll(2, h2, None).unwrap().unwrap();
        assert_eq!(a, b);
        assert_ne!(a.index, 2);
        a_count += usize::from(a.index == 0);
        bank.complete(1, h, a.serial, 0).unwrap();
        bank.complete(2, h2, b.serial, 0).unwrap();
    }
    assert!((7300..7700).contains(&a_count), "{a_count}");
}
#[test]
fn malformed_zero_intervals_are_bounded_and_poll_cannot_reset_the_budget() {
    let mut bank = Sequences::default();
    let h = bank
        .create(
            1,
            Spec {
                mode: Mode::Loop,
                lines: vec![line("a", -1)],
                ..Default::default()
            },
            1,
        )
        .unwrap();
    for serial in 1..=1024 {
        let e = bank.poll(1, h, None).unwrap().unwrap();
        assert_eq!(e.serial, serial);
        bank.complete(1, h, e.serial, 0).unwrap();
    }
    for _ in 0..100 {
        assert!(bank.poll(1, h, None).unwrap().is_none());
    }
    assert!(bank.info(1, h).unwrap().active && bank.info(1, h).unwrap().budget_exhausted);
    bank.advance(1, h, 0, 0).unwrap();
    assert_eq!(bank.poll(1, h, None).unwrap().unwrap().serial, 1025);
}
#[test]
fn scope_generation_capacity_and_invalid_inputs_do_not_alias_or_mutate() {
    let mut bank = Sequences::default();
    let mut invalid = Spec {
        lines: vec![line("ok", 0)],
        ..Default::default()
    };
    invalid.lines[0].min_ms = i64::MIN;
    assert!(bank.create(1, invalid, 0).is_err());
    assert!(bank.create(0, Spec::default(), 0).is_err());
    let h = bank.create(1, Spec::default(), 0).unwrap();
    assert_eq!(h.generation, 1);
    assert!(bank.info(2, h).is_err());
    assert!(bank.release(2, h).is_err());
    let before = bank.info(1, h).unwrap();
    assert!(bank.advance(1, h, 60001, 0).is_err());
    assert_eq!(bank.info(1, h).unwrap(), before);
    assert!(bank.poll(1, h, None).unwrap().is_none());
    assert!(!bank.info(1, h).unwrap().active);
    for _ in 1..128 {
        bank.create(1, Spec::default(), 0).unwrap();
    }
    assert!(bank.create(1, Spec::default(), 0).is_err());
    let other = bank.create(2, Spec::default(), 0).unwrap();
    bank.clear_client(1);
    assert!(bank.info(1, h).is_err());
    assert!(bank.info(2, other).is_ok());
    let new = bank.create(1, Spec::default(), 0).unwrap();
    assert_ne!(h, new);
    bank.clear();
    assert!(bank.info(1, new).is_err());
    assert_ne!(bank.create(1, Spec::default(), 0).unwrap(), new);
}
#[test]
fn zero_weight_fallback_exact_intervals_and_pending_release_do_not_draw_rng() {
    let mut bank = Sequences::default();
    let mut a = line("a", -2);
    a.weight = 0;
    let h = bank
        .create(
            1,
            Spec {
                mode: Mode::Weighted,
                lines: vec![a],
                ..Default::default()
            },
            123,
        )
        .unwrap();
    let e = bank.poll(1, h, None).unwrap().unwrap();
    bank.complete(1, h, e.serial, 0).unwrap();
    assert_eq!(bank.info(1, h).unwrap().rng_state, 123);
    let old = e.serial;
    let e = bank.poll(1, h, Some(7)).unwrap().unwrap();
    assert_eq!(bank.info(1, h).unwrap().rng_state, 7);
    assert!(bank.complete(1, h, old, 0).is_err());
    bank.release(1, h).unwrap();
    assert!(bank.complete(1, h, e.serial, 0).is_err());
}
