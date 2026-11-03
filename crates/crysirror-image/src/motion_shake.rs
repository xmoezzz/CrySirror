//! Sampled planar shake with caller-owned randomness and clock.
#[derive(Debug, Clone, Copy)]
pub struct Oscillation {
    active: bool,
    direction_mode: i32,
    current_amplitude_fixed: i64,
    cycle_count: u32,
    damping_percent: i32,
    updates_per_second: u32,
    updates_per_cycle: u32,
    cycle_index: u32,
    update_index: u32,
    scalar_fixed: i64,
    step_fixed: i64,
    previous_random: (i64, i64),
    current_random: (i64, i64),
    random_delta: (i64, i64),
    elapsed_ms: u64,
    update_interval_ms: u64,
    immediate_update_pending: bool,
    offset: (f32, f32),
}

impl Default for Oscillation {
    fn default() -> Self {
        Self {
            active: false,
            direction_mode: 0,
            current_amplitude_fixed: 0,
            cycle_count: 0,
            damping_percent: 0,
            updates_per_second: 60,
            updates_per_cycle: 1,
            cycle_index: 0,
            update_index: 0,
            scalar_fixed: 0,
            step_fixed: 0,
            previous_random: (0, 0),
            current_random: (0, 0),
            random_delta: (0, 0),
            elapsed_ms: 0,
            update_interval_ms: 16,
            immediate_update_pending: false,
            offset: (0.0, 0.0),
        }
    }
}

fn target_shake_random_average(rng: &mut dyn FnMut() -> i32, maximum: i64) -> i64 {
    // sub_43CFF0 is used only when CProcShakeScreen's current amplitude is
    // positive. It averages eight independent 30-bit CRT-rand reductions.
    if maximum <= 0 {
        return 0;
    }
    let modulus = maximum.saturating_add(1).min(i64::from(i32::MAX));
    let mut total = 0_i64;
    for _ in 0..8 {
        let high = i64::from(rng()) << 15;
        let low = i64::from(rng());
        total = total.saturating_add((high | low) % modulus);
    }
    total >> 3
}

impl Oscillation {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn configure(
        &mut self,
        direction_mode: i32,
        amplitude: i32,
        oscillation_ticks: i32,
        cycle_count: i32,
        damping_percent: i32,
        updates_per_second: i32,
    ) -> bool {
        // sub_43CCE0 validates exactly these four conditions. In particular,
        // damping is not clamped by the target and update rates above 1000
        // legitimately produce a zero-millisecond procedure deadline step.
        if !(0..=3).contains(&direction_mode)
            || oscillation_ticks < 1
            || cycle_count < 1
            || updates_per_second < 1
            || updates_per_second < oscillation_ticks
        {
            return false;
        }
        self.active = true;
        self.direction_mode = direction_mode;
        self.current_amplitude_fixed = i64::from(amplitude) << 11;
        self.cycle_count = cycle_count as u32;
        self.damping_percent = damping_percent;
        self.updates_per_second = updates_per_second as u32;
        self.updates_per_cycle = self.updates_per_second / oscillation_ticks as u32;
        self.cycle_index = 0;
        self.update_index = 0;
        self.scalar_fixed = 0;
        self.step_fixed = 0;
        self.previous_random = (0, 0);
        self.current_random = (0, 0);
        self.random_delta = (0, 0);
        self.elapsed_ms = 0;
        self.update_interval_ms = 1000 / u64::from(self.updates_per_second);
        // sub_43CCE0 schedules CProcShakeScreen at deadline +0, so the first
        // procedure poll is immediately eligible even if no clock time passed.
        self.immediate_update_pending = true;
        self.offset = (0.0, 0.0);
        true
    }

    fn begin_cycle(&mut self, rng: &mut dyn FnMut() -> i32) {
        // 0x43CDD0 resets the triangular scalar at every cycle and derives its
        // step from the *current* damped amplitude.
        self.scalar_fixed = 0;
        self.step_fixed = self.current_amplitude_fixed.saturating_mul(4)
            / i64::from(self.updates_per_cycle.max(1));

        self.previous_random = self.current_random;
        if self.direction_mode == 3 && self.current_amplitude_fixed > 0 {
            let x = target_shake_random_average(rng, self.current_amplitude_fixed);
            let y = target_shake_random_average(rng, self.current_amplitude_fixed);
            self.current_random = match self.cycle_index & 3 {
                0 => (-x, -y),
                1 => (x, y),
                2 => (-x, y),
                _ => (x, -y),
            };
        } else if self.direction_mode == 3 {
            self.current_random = (0, 0);
        }
        self.random_delta = (
            self.current_random.0 - self.previous_random.0,
            self.current_random.1 - self.previous_random.1,
        );
    }

    fn advance_one_update(&mut self, rng: &mut dyn FnMut() -> i32) {
        if !self.active {
            return;
        }
        if self.update_index == 0 {
            self.begin_cycle(rng);
        }

        // Exact scalar portion of 0x43CDD0: add 4*A/N, then reflect around
        // +/-A without discarding overshoot.
        self.scalar_fixed = self.scalar_fixed.saturating_add(self.step_fixed);
        let amplitude = self.current_amplitude_fixed;
        if self.scalar_fixed > amplitude || self.scalar_fixed < -amplitude {
            self.step_fixed = self.step_fixed.saturating_neg();
        }
        if self.scalar_fixed > amplitude {
            self.scalar_fixed = amplitude
                .saturating_mul(2)
                .saturating_sub(self.scalar_fixed);
        }
        if self.scalar_fixed < -amplitude {
            self.scalar_fixed = self
                .scalar_fixed
                .saturating_neg()
                .saturating_sub(amplitude.saturating_mul(2));
        }

        // The local masks in 0x43CDD0 are:
        //   mode 0: (scalar, 0)
        //   mode 1: (0, scalar)
        //   mode 2: (scalar, scalar)
        //   mode 3: interpolate two CRT-random endpoints.
        let (x_fixed, y_fixed) = match self.direction_mode {
            0 => (self.scalar_fixed, 0),
            1 => (0, self.scalar_fixed),
            2 => (self.scalar_fixed, self.scalar_fixed),
            3 => {
                let n = i64::from(self.updates_per_cycle.max(1));
                let i = i64::from(self.update_index);
                (
                    self.previous_random.0 + self.random_delta.0.saturating_mul(i) / n,
                    self.previous_random.1 + self.random_delta.1.saturating_mul(i) / n,
                )
            }
            _ => (0, 0),
        };
        self.offset = ((x_fixed >> 12) as f32, (y_fixed >> 12) as f32);

        self.update_index = self.update_index.saturating_add(1);
        if self.update_index >= self.updates_per_cycle {
            self.cycle_index = self.cycle_index.saturating_add(1);
            self.current_amplitude_fixed = self
                .current_amplitude_fixed
                .saturating_mul(i64::from(100 - self.damping_percent))
                / 100;
            self.update_index = 0;
            if self.cycle_index >= self.cycle_count {
                self.cancel();
            }
        }
    }

    pub fn cancel(&mut self) {
        self.active = false;
        self.scalar_fixed = 0;
        self.step_fixed = 0;
        self.immediate_update_pending = false;
        self.offset = (0.0, 0.0);
    }

    pub fn tick(&mut self, elapsed_ms: u64, rng: &mut dyn FnMut() -> i32) -> (f32, f32) {
        if !self.active {
            self.offset = (0.0, 0.0);
            return self.offset;
        }

        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
        if self.immediate_update_pending {
            self.immediate_update_pending = false;
            self.advance_one_update(rng);
        }

        if self.update_interval_ms == 0 {
            // A zero deadline increment remains continuously due in the target,
            // but one host pass must stay finite. One procedure update per pass
            // preserves forward progress without inventing a wall-clock delay.
            if self.active {
                self.advance_one_update(rng);
            }
            return self.offset;
        }

        while self.active && self.elapsed_ms >= self.update_interval_ms {
            self.elapsed_ms -= self.update_interval_ms;
            self.advance_one_update(rng);
        }
        self.offset
    }
}
