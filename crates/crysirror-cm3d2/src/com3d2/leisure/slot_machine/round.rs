// SPDX-License-Identifier: MPL-2.0
//! SlotManager and Reel rules from the standalone 1.0.4 assembly.
use super::{LINES, Machine, ResultLights, symbol};

const SPEED: f32 = 0.0012;

#[derive(Default)]
pub(super) struct Reach {
    symbols: [Option<usize>; 5],
}

impl Reach {
    pub fn active(&self) -> bool {
        self.symbols.iter().any(Option::is_some)
    }
}

#[derive(Default)]
pub(super) struct ReelMarks {
    pub ranges: [[[f32; 2]; 3]; 3],
    next: [usize; 3],
}

impl ReelMarks {
    fn clear_ranges(&mut self) {
        self.ranges = [[[0.; 2]; 3]; 3];
    }

    fn mark(&mut self, reel: usize, stop: i32, row: i32) {
        let number = (stop + 8 + row).rem_euclid(21) + 1;
        let phase = stop.rem_euclid(21) as f32 / 21.;
        let n = number as f32;
        let (lo, hi) = match number {
            9 if phase < 0.1 => (1. - 29. / 21., 1. - 30. / 21.),
            8 if phase <= 0.9 => (7. / 21., 8. / 21.),
            1..=9 => ((n + 20.) / 21., (n + 21.) / 21.),
            _ => ((n - 1.) / 21., n / 21.),
        };
        let slot = &mut self.next[reel];
        self.ranges[reel][*slot] = [lo.abs().min(hi.abs()), lo.abs().max(hi.abs())];
        *slot = (*slot + 1) % 3;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PayPhase {
    Show,
    Deposit,
    Count,
    Finish,
    Done,
}

pub(super) struct Payout {
    prizes: [Option<i32>; 2],
    pace: u64,
    due: u64,
    index: usize,
    phase: PayPhase,
    outstanding: i32,
    pub trophy: Option<i32>,
}

impl Payout {
    pub fn new(now: u64, cherry: Option<i32>, symbol: Option<i32>, pace: u64) -> Self {
        let index = usize::from(cherry.is_none());
        let prizes = [cherry, symbol];
        let phase = if prizes[index].is_some() {
            PayPhase::Show
        } else {
            PayPhase::Finish
        };
        Self {
            prizes,
            pace,
            due: now
                + if phase == PayPhase::Show && index == 1 {
                    500
                } else {
                    0
                },
            index,
            phase,
            outstanding: 0,
            trophy: None,
        }
    }

    pub fn done(&self) -> bool {
        self.phase == PayPhase::Done
    }

    fn finish_prize(&mut self, now: u64) {
        if self.index == 0 && self.prizes[1].is_some() {
            self.index = 1;
            self.phase = PayPhase::Show;
            self.due = now + 500;
        } else if self.index == 1 {
            self.trophy = Some(
                self.prizes
                    .iter()
                    .flatten()
                    .copied()
                    .fold(0i32, i32::wrapping_add),
            );
            self.phase = PayPhase::Finish;
            self.due = now + 500;
        } else {
            self.phase = PayPhase::Done;
        }
    }

    /// Payout is an original coroutine, so each prepared update transfers at
    /// most one 100-credit step on the mechanical display.
    pub fn tick(&mut self, now: u64, bank: &mut i64, credit: &mut i64, digit: &mut i64) -> bool {
        if now < self.due {
            return self.phase == PayPhase::Count;
        }
        match self.phase {
            PayPhase::Show => {
                self.outstanding = self.prizes[self.index].expect("displayed prize");
                *digit = i64::from(self.outstanding);
                self.phase = PayPhase::Deposit;
                self.due = now + 1000;
            }
            PayPhase::Deposit | PayPhase::Count => {
                if self.phase == PayPhase::Deposit {
                    *bank += i64::from(self.outstanding);
                }
                if self.outstanding > 0 {
                    self.outstanding -= 100;
                    *digit = i64::from(self.outstanding);
                    *credit += 100;
                    self.phase = PayPhase::Count;
                    self.due = now + self.pace;
                } else {
                    self.finish_prize(now);
                }
            }
            PayPhase::Finish => self.phase = PayPhase::Done,
            PayPhase::Done => {}
        }
        self.phase == PayPhase::Count
    }
}

impl Machine {
    fn spinning_phase(&self, reel: usize, now: u64) -> f32 {
        (self.carry[reel]
            + self
                .spin
                .map_or(0., |start| now.saturating_sub(start) as f32 * SPEED))
        .fract()
    }

    pub(super) fn phase(&self, reel: usize, now: u64) -> f32 {
        if self.spin.is_some_and(|start| now >= start)
            && (self.stops[reel].is_none() || self.sliding == Some(reel))
        {
            self.spinning_phase(reel, now)
        } else {
            self.offsets[reel]
        }
    }

    pub(super) fn stop(&mut self, reel: usize, now: u64) {
        let scroll = self.spinning_phase(reel, now);
        let initial = (scroll / (1. / 21.) + 1.) as i32;
        if self.stops.iter().all(Option::is_none) && std::mem::take(&mut self.bar_pending) {
            self.target = Some((1, 0));
        }
        let position = if let Some((target, row)) = self.target {
            (initial..=initial + 11)
                .find(|&p| symbol(reel, p, row) == target)
                .unwrap_or(initial + 11)
        } else {
            let mut position = initial;
            if reel == 0
                && !self.chance
                && self.normal_rounds <= 20
                && symbol(reel, initial, 0) == 6
            {
                position += 2;
            }
            for (line, reach) in LINES.iter().zip(self.reach.symbols) {
                if let Some(wanted) = reach.filter(|&symbol| symbol != 3) {
                    while symbol(reel, position, line[reel]) == wanted {
                        position += 1;
                    }
                }
            }
            position
        };
        self.stops[reel] = Some(position);
        self.sliding = Some(reel);
        self.last_stop = Some(reel);
        // Reel compares against the wrapped threshold, including a slide
        // landing before the current phase across the strip seam.
        let target_phase = position.rem_euclid(21) as f32 / 21.;
        self.next_stop = now + (((target_phase - scroll) / SPEED).ceil() as u64).max(1);
    }

    pub(super) fn finish_stop(&mut self, now: u64, mut random: impl FnMut(usize) -> usize) -> bool {
        let reel = self.sliding.take().expect("a reel is sliding");
        let stop = self.stops[reel].expect("a slide has a target");
        self.carry[reel] = self.spinning_phase(reel, now);
        self.offsets[reel] = stop.rem_euclid(21) as f32 / 21.;
        let count = self.stops.iter().flatten().count();
        if count == 1 {
            let mode = usize::from(self.chance);
            let weights = &self.weights[mode];
            // The original consumes one unused draw before the weighted draw.
            random(100);
            let mut roll = random(self.max_probability[mode].max(1));
            for row in [1, 0, -1] {
                let candidate = symbol(reel, stop, row);
                if roll < weights[candidate] {
                    self.target = Some((candidate, row));
                    break;
                }
                roll -= weights[candidate];
            }
            self.seven[reel] = self.chance
                && [1, 0, -1]
                    .into_iter()
                    .any(|row| symbol(reel, stop, row) == 0);
        }
        if reel == 0 {
            for row in [1, 0, -1] {
                if symbol(0, stop, row) == 6 {
                    self.marks.mark(0, stop, row);
                }
            }
        }
        if count == 2 {
            for (i, line) in LINES.iter().enumerate() {
                let pairs = self
                    .stops
                    .iter()
                    .enumerate()
                    .filter_map(|(r, stop)| stop.map(|p| (r, p, symbol(r, p, line[r]))))
                    .collect::<Vec<_>>();
                if pairs[0].2 == pairs[1].2 {
                    self.reach.symbols[i] = Some(pairs[0].2);
                    for (r, p, _) in pairs {
                        self.marks.mark(r, p, line[r]);
                    }
                }
            }
            self.seven[reel] = self.chance && self.reach.symbols.contains(&Some(0));
            return self.reach.symbols.iter().any(Option::is_some);
        }
        false
    }

    pub(super) fn settle(&mut self, now: u64) {
        let stops = self.stops.map(|p| p.expect("all reels stopped"));
        self.spin = None;
        self.marks.clear_ranges(); // WinCheck clears vectors without resetting their cursors.
        let at = |r, row| symbol(r, stops[r], row);
        let mut win = None;
        let mut paylines = [false; 5];
        for (i, line) in LINES.iter().enumerate() {
            let s = at(0, line[0]);
            if s == at(1, line[1]) && s == at(2, line[2]) {
                paylines[i] = true;
                win = Some(s);
                for r in 0..3 {
                    self.marks.mark(r, stops[r], line[r]);
                }
            }
        }
        let mut cherry = 0;
        for (row, lines) in [(1, &[1, 3][..]), (0, &[0][..]), (-1, &[2, 4][..])] {
            if at(0, row) == 6 {
                cherry = if row == 0 { 1 } else { 2 };
                for &index in lines {
                    paylines[index] = true;
                }
                self.marks.mark(0, stops[0], row);
            }
        }
        for (left, right, kind) in [(1, 1, 3), (0, 0, 4), (-1, -1, 3), (1, 0, 5), (-1, 0, 5)] {
            if at(0, left) == 6 && at(1, right) == 6 {
                cherry = kind;
                self.marks.mark(0, stops[0], left);
                self.marks.mark(1, stops[1], right);
            }
        }
        let multiplier = if self.chance {
            [0, 1, 2, 1, 2, 3]
        } else {
            [0, 1, 2, 2, 2, 3]
        };
        let cherry_pay =
            (cherry != 0).then(|| (self.payouts[6] as i32).wrapping_mul(multiplier[cherry]));
        self.bar_pending |= !self.chance && matches!(cherry, 1 | 4);
        let jackpot = win == Some(0);
        let chance_correction = match win {
            Some(0) => -1,
            Some(1) => 99,
            _ if !self.chance && cherry == 3 => 19,
            _ => 0,
        };
        let payout = Payout::new(
            now + if jackpot { 12_500 } else { 0 },
            cherry_pay,
            win.map(|s| self.payouts[s] as i32),
            self.pay_step_ms,
        );
        self.result = Some(ResultLights {
            start: now,
            jackpot,
            paylines,
            chance_correction,
            voice_played: false,
            payout,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn third_reel_avoids_an_unfixed_center_reach() {
        let mut machine = Machine {
            spin: Some(0),
            stops: [Some(0), Some(0), None],
            sliding: Some(1),
            ..Machine::default()
        };
        assert!(machine.finish_stop(0, |_| unreachable!("second stop does not draw")));
        assert_eq!(machine.reach.symbols[0], Some(2));
        assert!(machine.marks.ranges[0].iter().any(|[low, high]| low < high));
        machine.stop(2, 40);
        assert_eq!(machine.stops[2], Some(3));
        assert_ne!(symbol(2, 3, 0), 2);
    }

    #[test]
    fn payout_preserves_deposit_and_final_counter_step() {
        let mut payout = Payout::new(0, Some(150), Some(200), 40);
        let (mut bank, mut credit, mut digit) = (700, 700, 0);
        assert!(!payout.tick(0, &mut bank, &mut credit, &mut digit));
        assert_eq!((bank, credit, digit), (700, 700, 150));
        assert!(payout.tick(1_000, &mut bank, &mut credit, &mut digit));
        assert_eq!((bank, credit, digit), (850, 800, 50));
        assert!(payout.tick(5_000, &mut bank, &mut credit, &mut digit));
        assert_eq!((bank, credit, digit), (850, 900, -50));
        assert!(!payout.tick(5_040, &mut bank, &mut credit, &mut digit));
        assert!(!payout.tick(5_540, &mut bank, &mut credit, &mut digit));
        assert_eq!(digit, 200);
        assert!(payout.tick(6_540, &mut bank, &mut credit, &mut digit));
        assert!(payout.tick(6_580, &mut bank, &mut credit, &mut digit));
        assert!(!payout.tick(6_620, &mut bank, &mut credit, &mut digit));
        assert_eq!((bank, credit, digit), (1_050, 1_100, 0));
        assert_eq!(payout.trophy, Some(350));
        assert!(!payout.done());
        payout.tick(7_120, &mut bank, &mut credit, &mut digit);
        assert!(payout.done());
    }
}
