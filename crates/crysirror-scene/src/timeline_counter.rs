// SPDX-License-Identifier: MPL-2.0
// Counter arithmetic ported from siglus_rs 5f358ef1360b0439e92aed2b749e8f25d8721954
// crates/siglus_scene_vm/src/runtime/globals.rs (see shared-native-capability-sources.json).
//! Engine-owned integer timelines over caller-supplied game/real clock deltas.
//! Contains no VM, host clock, resource handles, windows or devices.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Counter {
    cur_time: i64,
    is_running: bool,
    real_flag: bool,
    frame_mode: bool,
    frame_loop_flag: bool,
    frame_start_value: i64,
    frame_end_value: i64,
    frame_time: i64,
}

impl Counter {
    fn limit(min: i64, value: i64, max: i64) -> i64 {
        if value < min {
            min
        } else if value > max {
            max
        } else {
            value
        }
    }

    pub fn reinit(&mut self) {
        *self = Self::default();
    }

    pub fn reset(&mut self) {
        self.is_running = false;
        self.real_flag = false;
        self.frame_mode = false;
        self.cur_time = 0;
    }

    pub fn set_count(&mut self, value: i64) {
        if self.frame_mode {
            if self.frame_end_value == self.frame_start_value || self.frame_time <= 0 {
                self.cur_time = 0;
                return;
            }

            let denom = i128::from(self.frame_end_value) - i128::from(self.frame_start_value);
            let frame_time = self.frame_time;
            let time = (i128::from(value) - i128::from(self.frame_start_value))
                * i128::from(frame_time)
                / denom;
            let mut cur_time = time.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
            if self.frame_loop_flag {
                cur_time = Self::limit(0, cur_time, frame_time - 1);
            } else {
                cur_time = Self::limit(0, cur_time, frame_time);
            }
            self.cur_time = cur_time;
        } else {
            self.cur_time = value;
        }
    }

    pub fn start(&mut self) {
        self.is_running = true;
        self.real_flag = false;
        self.frame_mode = false;
        self.cur_time = 0;
    }

    pub fn start_real(&mut self) {
        self.is_running = true;
        self.real_flag = true;
        self.frame_mode = false;
        self.cur_time = 0;
    }

    pub fn start_frame(&mut self, from: i64, to: i64, frame_time: i64) {
        self.is_running = true;
        self.real_flag = false;
        self.frame_mode = true;
        self.frame_loop_flag = false;
        self.frame_start_value = from;
        self.frame_end_value = to;
        self.frame_time = frame_time;
        self.cur_time = 0;
    }

    pub fn start_frame_real(&mut self, from: i64, to: i64, frame_time: i64) {
        self.is_running = true;
        self.real_flag = true;
        self.frame_mode = true;
        self.frame_loop_flag = false;
        self.frame_start_value = from;
        self.frame_end_value = to;
        self.frame_time = frame_time;
        self.cur_time = 0;
    }

    pub fn start_frame_loop(&mut self, from: i64, to: i64, frame_time: i64) {
        self.is_running = true;
        self.real_flag = false;
        self.frame_mode = true;
        self.frame_loop_flag = true;
        self.frame_start_value = from;
        self.frame_end_value = to;
        self.frame_time = frame_time;
        self.cur_time = 0;
    }

    pub fn start_frame_loop_real(&mut self, from: i64, to: i64, frame_time: i64) {
        self.is_running = true;
        self.real_flag = true;
        self.frame_mode = true;
        self.frame_loop_flag = true;
        self.frame_start_value = from;
        self.frame_end_value = to;
        self.frame_time = frame_time;
        self.cur_time = 0;
    }

    pub fn stop(&mut self) {
        self.is_running = false;
    }

    pub fn resume(&mut self) {
        self.is_running = true;
    }

    pub fn update_time(&mut self, past_game_time: i32, past_real_time: i32) {
        if self.is_running {
            let add = if self.real_flag {
                past_real_time
            } else {
                past_game_time
            };
            self.cur_time = self.cur_time.saturating_add(add as i64);
        }

        if self.frame_mode && !self.frame_loop_flag && self.cur_time >= self.frame_time {
            self.is_running = false;
        }
    }

    pub fn get_count(&self) -> i64 {
        if self.frame_mode {
            if self.frame_time <= 0 {
                return self.frame_end_value;
            }
            if self.frame_start_value == self.frame_end_value {
                return self.frame_end_value;
            }

            let span = i128::from(self.frame_end_value) - i128::from(self.frame_start_value);
            let mut value = span * i128::from(self.cur_time) / i128::from(self.frame_time);
            if self.frame_loop_flag {
                value %= span;
                value += i128::from(self.frame_start_value);
            } else {
                value += i128::from(self.frame_start_value);
                let mut value = value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
                if self.frame_start_value > self.frame_end_value {
                    value = Self::limit(self.frame_end_value, value, self.frame_start_value);
                } else {
                    value = Self::limit(self.frame_start_value, value, self.frame_end_value);
                }
                return value;
            }
            value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
        } else {
            self.cur_time
        }
    }

    pub fn get_count_with_frame(&self, _current_frame: i64) -> i64 {
        self.get_count()
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn save_parts(&self) -> (bool, bool, bool, bool, i64, i64, i64, i64) {
        (
            self.is_running,
            self.real_flag,
            self.frame_mode,
            self.frame_loop_flag,
            self.frame_start_value,
            self.frame_end_value,
            self.frame_time,
            self.cur_time,
        )
    }

    pub fn from_save_parts(
        is_running: bool,
        real_flag: bool,
        frame_mode: bool,
        frame_loop_flag: bool,
        frame_start_value: i64,
        frame_end_value: i64,
        frame_time: i64,
        cur_time: i64,
    ) -> Self {
        Self {
            cur_time,
            is_running,
            real_flag,
            frame_mode,
            frame_loop_flag,
            frame_start_value,
            frame_end_value,
            frame_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_clocks_range_loop_pause_and_saved_parts() {
        let mut game = Counter::default();
        game.start_frame(10, 20, 100);
        let mut real = Counter::default();
        real.start_real();
        game.update_time(40, 70);
        real.update_time(40, 70);
        assert_eq!(game.get_count(), 14);
        assert_eq!(real.get_count(), 70);
        let (a, b, c, d, e, f, g, h) = game.save_parts();
        let mut restored = Counter::from_save_parts(a, b, c, d, e, f, g, h);
        restored.stop();
        restored.update_time(50, 50);
        assert_eq!(restored.get_count(), 14);
        restored.resume();
        restored.update_time(60, 60);
        assert_eq!(restored.get_count(), 20);
        assert!(!restored.is_running());
        game.start_frame_loop(20, 10, 100);
        game.update_time(140, 0);
        assert_eq!(game.get_count(), 16);
        game.set_count(12);
        assert_eq!(game.get_count(), 12);
        game.start_frame(i64::MIN, i64::MAX, 100);
        game.update_time(50, 50);
        assert_eq!(game.get_count(), -1);
        game.start_frame(1, 2, 0);
        assert_eq!(game.get_count(), 2);
    }
}
