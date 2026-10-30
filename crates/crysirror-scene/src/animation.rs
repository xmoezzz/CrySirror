// SPDX-License-Identifier: MPL-2.0
// Ported from xmoezzz/siglus_rs 5f358ef1360b0439e92aed2b749e8f25d8721954.
// Source and changes recorded in ../UPSTREAM.md.
//! Integer event (linear interpolation / loop / turn) used by a few engine subsystems.
//!
//! This follows the original Siglus behavior:
//! - `value` is the "current" stored value (for oneshot, it is set to the target immediately).
//! - `cur_value` is the animated/interpolated value computed each frame.

/// Device-free scalar motion on a caller-supplied monotonic millisecond clock.
/// The engine owns the immutable curve data; adapters choose native targets.
/// No wall clock, thread, device or VM reference is retained.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarTween {
    pub start: f32,
    pub end: f32,
    pub started_ms: u64,
    pub duration_ms: u64,
    pub easing: crate::layer::Easing,
}
impl ScalarTween {
    pub fn new(
        start: f32,
        end: f32,
        started_ms: u64,
        duration_ms: u64,
        easing: crate::layer::Easing,
    ) -> anyhow::Result<Self> {
        let value = Self {
            start,
            end,
            started_ms,
            duration_ms,
            easing,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.start.is_finite() && self.end.is_finite(),
            "scalar tween endpoints must be finite"
        );
        Ok(())
    }
    /// Times before admission retain the start value. Zero duration completes
    /// at admission; elapsed time is subtracted, never added to a deadline.
    pub fn sample(&self, now_ms: u64) -> f32 {
        if now_ms < self.started_ms {
            return self.start;
        }
        let elapsed = now_ms - self.started_ms;
        if elapsed >= self.duration_ms {
            return self.end;
        }
        let ratio=elapsed as f64 / self.duration_ms as f64;
        let eased=if self.easing==crate::layer::Easing::Linear{ratio}else{f64::from(self.easing.apply(ratio as f32))};
        // f64 avoids overflowing an intermediate end-start for finite f32
        // endpoints of opposite signs.
        ((1. - eased) * f64::from(self.start) + eased * f64::from(self.end))
            as f32
    }
    pub fn complete(&self, now_ms: u64) -> bool {
        now_ms >= self.started_ms && now_ms - self.started_ms >= self.duration_ms
    }
    /// Continue from the exact sampled value when reversing/changing target.
    pub fn retarget(
        &mut self,
        now_ms: u64,
        end: f32,
        duration_ms: u64,
        easing: crate::layer::Easing,
    ) -> anyhow::Result<()> {
        *self = Self::new(self.sample(now_ms), end, now_ms, duration_ms, easing)?;
        Ok(())
    }
}

#[cfg(test)]
mod scalar_tween_tests {
    use super::*;
    use crate::layer::Easing;
    #[test]
    fn reversed_motion_and_serialized_continuation_have_no_jump() {
        let mut tween = ScalarTween::new(0., 1., 100, 400, Easing::Smooth).unwrap();
        assert_eq!(tween.sample(200), 0.15625);
        tween.retarget(200, 0., 200, Easing::Smooth).unwrap();
        assert_eq!(tween.sample(200), 0.15625);
        let restored: ScalarTween =
            serde_json::from_slice(&serde_json::to_vec(&tween).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.sample(300), 0.078125);
        assert_eq!(restored.sample(400), 0.);
        assert!(restored.complete(400));
    }
    #[test]
    fn future_zero_duration_extreme_times_and_finite_endpoints() {
        let t = ScalarTween::new(7., 9., u64::MAX, 0, Easing::Linear).unwrap();
        assert_eq!(t.sample(u64::MAX - 1), 7.);
        assert!(!t.complete(u64::MAX - 1));
        assert_eq!(t.sample(u64::MAX), 9.);
        assert!(t.complete(u64::MAX));
        let t = ScalarTween::new(-f32::MAX, f32::MAX, u64::MAX - 10, 100, Easing::Linear).unwrap();
        assert!(t.sample(u64::MAX).is_finite());
        assert!(!t.complete(u64::MAX));
        assert!(ScalarTween::new(f32::NAN, 0., 0, 1, Easing::Linear).is_err());
        assert!(ScalarTween::new(0., f32::INFINITY, 0, 1, Easing::Linear).is_err());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntEvent {
    pub def_value: i32,
    pub value: i32,
    pub cur_time: i32,
    pub end_time: i32,
    pub delay_time: i32,
    pub start_value: i32,
    pub cur_value: i32,
    pub end_value: i32,
    /// -1: none, 0: oneshot, 1: loop, 2: turn
    pub loop_type: i32,
    /// -1: none, 0: linear, 1: speed_up, 2: speed_down
    pub speed_type: i32,
    /// 0: game time, 1: real time
    pub real_flag: i32,
}

impl IntEvent {
    pub fn new(def_value: i32) -> Self {
        let mut s = Self {
            def_value,
            value: def_value,
            cur_time: 0,
            end_time: 0,
            delay_time: 0,
            start_value: def_value,
            cur_value: def_value,
            end_value: def_value,
            loop_type: -1,
            speed_type: -1,
            real_flag: 0,
        };
        s.reinit();
        s
    }

    pub fn reinit(&mut self) {
        self.value = self.def_value;
        self.cur_time = 0;
        self.end_time = 0;
        self.delay_time = 0;
        self.start_value = self.def_value;
        self.cur_value = self.def_value;
        self.end_value = self.def_value;
        self.loop_type = -1;
        self.speed_type = -1;
        self.real_flag = 0;
    }

    pub fn set_value(&mut self, v: i32) {
        self.value = v;
        // Note: the original does not touch the event fields here.
    }

    pub fn get_value(&self) -> i32 {
        self.value
    }

    pub fn get_total_value(&self) -> i32 {
        self.cur_value
    }

    pub fn set_event(
        &mut self,
        value: i32,
        total_time: i32,
        delay_time: i32,
        speed_type: i32,
        real_flag: i32,
    ) {
        self.cur_time = 0;
        self.end_time = total_time;
        self.delay_time = delay_time;
        self.start_value = self.value;
        self.cur_value = self.value;
        self.end_value = value;
        self.loop_type = 0; // oneshot
        self.speed_type = speed_type;
        self.real_flag = real_flag;

        // The original updates the stored value immediately.
        self.value = value;
    }

    pub fn loop_event(
        &mut self,
        start_value: i32,
        end_value: i32,
        loop_time: i32,
        delay_time: i32,
        speed_type: i32,
        real_flag: i32,
    ) {
        self.cur_time = 0;
        self.end_time = loop_time;
        self.delay_time = delay_time;
        self.start_value = start_value;
        self.cur_value = start_value;
        self.end_value = end_value;
        self.loop_type = 1; // loop
        self.speed_type = speed_type;
        self.real_flag = real_flag;
    }

    pub fn turn_event(
        &mut self,
        start_value: i32,
        end_value: i32,
        loop_time: i32,
        delay_time: i32,
        speed_type: i32,
        real_flag: i32,
    ) {
        self.cur_time = 0;
        self.end_time = loop_time;
        self.delay_time = delay_time;
        self.start_value = start_value;
        self.cur_value = start_value;
        self.end_value = end_value;
        self.loop_type = 2; // turn
        self.speed_type = speed_type;
        self.real_flag = real_flag;
    }

    pub fn end_event(&mut self) {
        self.loop_type = -1;
        self.cur_value = self.value;
    }

    pub fn check_event(&self) -> bool {
        self.loop_type != -1
    }

    pub fn update_time(&mut self, past_game_time: i32, past_real_time: i32) {
        if self.loop_type == -1 {
            return;
        }
        if self.real_flag == 0 {
            self.cur_time = self.cur_time.saturating_add(past_game_time);
        } else {
            self.cur_time = self.cur_time.saturating_add(past_real_time);
        }
    }

    pub fn frame(&mut self) {
        self.cur_value = self.value;
        if self.loop_type == -1 {
            return;
        }
        self.frame_sub();
    }

    fn frame_sub(&mut self) {
        let end_time = self.end_time;
        let start_value = self.start_value;
        let end_value = self.end_value;
        let mut cur_time = self.cur_time - self.delay_time;

        // oneshot: if time is over, stop the event.
        if self.loop_type == 0 && cur_time - end_time >= 0 {
            self.loop_type = -1;
            return;
        }

        // Not started yet.
        if cur_time <= 0 {
            self.cur_value = start_value;
            return;
        }

        if end_time <= 0 {
            // Avoid division by zero / modulo by zero.
            self.cur_value = end_value;
            return;
        }

        if self.loop_type == 1 {
            cur_time %= end_time;
        }

        if self.loop_type == 2 {
            cur_time %= end_time * 2;
            if cur_time - end_time > 0 {
                cur_time = end_time - (cur_time - end_time);
            }
        }

        match self.speed_type {
            0 => {
                self.cur_value = (((end_value - start_value) as f64) * (cur_time as f64)
                    / (end_time as f64)
                    + (start_value as f64)) as i32;
            }
            1 => {
                let ct = cur_time as f64;
                let et = end_time as f64;
                self.cur_value = (((end_value - start_value) as f64) * ct * ct / et / et
                    + (start_value as f64)) as i32;
            }
            2 => {
                let ct = (cur_time - end_time) as f64;
                let et = end_time as f64;
                self.cur_value = (-(end_value - start_value) as f64 * ct * ct / et / et
                    + (end_value as f64)) as i32;
            }
            _ => {
                // Unknown/none: keep the last computed value.
            }
        }
    }

    /// Convenience: advance by one "tick".
    pub fn tick(&mut self, delta: i32) {
        self.update_time(delta, delta);
        self.frame();
    }
}
