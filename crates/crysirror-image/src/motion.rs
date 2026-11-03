//! Device-free sampled integer motion. The caller owns clock admission and
//! interrupt policy; this engine resource owns schedules, splines and samples.
use crate::progress_curve::{FixedProgressShape, sample_fixed_progress};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionCurve {
    Linear,
    HalfCosine,
    SineOut,
    SineIn,
    QuadraticIn,
    QuadraticOut,
    PowerTwoHalfIn,
    PowerTwoHalfOut,
    CubicIn,
    CubicOut,
    QuarticIn,
    QuarticOut,
    QuinticIn,
    QuinticOut,
    SexticIn,
    SexticOut,
}
fn sample_curve(curve: MotionCurve, progress: i32) -> i32 {
    use MotionCurve::*;
    let shape = match curve {
        Linear => FixedProgressShape::Linear,
        HalfCosine => FixedProgressShape::HalfCosine,
        SineOut => FixedProgressShape::SineOut,
        SineIn => FixedProgressShape::SineIn,
        QuadraticIn => FixedProgressShape::PowerIn(2.),
        QuadraticOut => FixedProgressShape::PowerOut(2.),
        PowerTwoHalfIn => FixedProgressShape::PowerIn(2.5),
        PowerTwoHalfOut => FixedProgressShape::PowerOut(2.5),
        CubicIn => FixedProgressShape::PowerIn(3.),
        CubicOut => FixedProgressShape::PowerOut(3.),
        QuarticIn => FixedProgressShape::PowerIn(4.),
        QuarticOut => FixedProgressShape::PowerOut(4.),
        QuinticIn => FixedProgressShape::PowerIn(5.),
        QuinticOut => FixedProgressShape::PowerOut(5.),
        SexticIn => FixedProgressShape::PowerIn(6.),
        SexticOut => FixedProgressShape::PowerOut(6.),
    };
    sample_fixed_progress(shape, progress)
        .expect("closed motion curve variants have valid exponents")
}
#[derive(Debug, Default)]
pub struct MotionControls {
    native_object_controls: Vec<ObjectControl>,
    native_special_path_controls: Vec<PathControl>,
    native_spline_controls: Vec<SplineControl>,
    native_shake_object_controls: Vec<ShakeControl>,
    native_control_completions: BTreeMap<u64, ControlCompletion>,
    next_native_control_id: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlCompletion {
    /// First deferred value written by CProcCtrlDspObj::Tick (0x431F00).
    /// The target stores it as procedure_update_count * 1000 / duration_or_steps.
    pub progress_per_mille: i32,
    /// Second deferred value: 0 natural, 1 input/procedure-local forced end,
    /// -1 base CProcedure cancellation/abnormal termination.
    pub status: i32,
}

impl ControlCompletion {
    const NATURAL: Self = Self {
        progress_per_mille: 1000,
        status: 0,
    };
}

impl MotionControls {
    pub fn schedule_object(
        &mut self,
        object_id: i32,
        from_position: (i32, i32),
        to_position: (i32, i32),
        position_curve: MotionCurve,
        from_alpha: i32,
        to_alpha: i32,
        alpha_curve: MotionCurve,
        from_fixed_parameter_16_16: i32,
        to_fixed_parameter_16_16: i32,
        duration_ms: i32,
        update_denominator: i32,
        update_numerator: i32,
    ) -> u64 {
        let control_id = self.allocate_id();
        let duration_ms = if duration_ms == 0 { 1 } else { duration_ms };
        if duration_ms < 0 {
            return control_id;
        }
        let update_interval_ms = if update_numerator != 0 && update_denominator != 0 {
            1000i64
                .saturating_mul(i64::from(update_numerator))
                .checked_div(i64::from(update_denominator))
                .unwrap_or(0)
                .clamp(0, i64::from(u32::MAX)) as u64
        } else {
            0
        };
        self.native_object_controls.push(ObjectControl {
            control_id,
            object_id,
            from_x: from_position.0,
            from_y: from_position.1,
            to_x: to_position.0,
            to_y: to_position.1,
            position_curve,
            from_alpha: from_alpha.clamp(0, 256),
            to_alpha: to_alpha.clamp(0, 256),
            alpha_curve,
            from_fixed_parameter_16_16,
            to_fixed_parameter_16_16,
            duration_ms: duration_ms as u64,
            wall_elapsed_ms: 0,
            update_numerator,
            update_interval_ms,
            next_update_ms: update_interval_ms,

            force_complete: false,
            procedure_update_count: 0,
            last_x: from_position.0,
            last_y: from_position.1,
            last_alpha: from_alpha.clamp(0, 256),
            last_fixed_parameter_16_16: from_fixed_parameter_16_16,
        });
        control_id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn schedule_spline(
        &mut self,
        object_id: i32,
        from_vector: (i32, i32, i32),
        points: &[[i32; 4]],
        position_curve: MotionCurve,
        from_alpha: i32,
        to_alpha: i32,
        alpha_curve: MotionCurve,
        from_fixed_parameter_16_16: i32,
        fixed_parameter_target: i32,
        duration_ms: i32,
        update_denominator: i32,
        update_numerator: i32,
        spline_time_scale: u16,
    ) -> u64 {
        let control_id = self.allocate_id();
        let duration_ms = if duration_ms == 0 { 1 } else { duration_ms };
        if points.is_empty() || duration_ms < 0 {
            return control_id;
        }
        let mut x = Vec::with_capacity(points.len() + 1);
        let mut y = Vec::with_capacity(points.len() + 1);
        let mut z = Vec::with_capacity(points.len() + 1);
        x.push(from_vector.0);
        y.push(from_vector.1);
        z.push(from_vector.2);
        for point in points {
            x.push(point[0]);
            y.push(point[1]);
            z.push(point[2]);
        }
        let update_interval_ms = if update_numerator != 0 && update_denominator != 0 {
            1000i64
                .saturating_mul(i64::from(update_numerator))
                .checked_div(i64::from(update_denominator))
                .unwrap_or(0)
                .clamp(0, i64::from(u32::MAX)) as u64
        } else {
            0
        };
        let to_fixed_parameter_16_16 = if fixed_parameter_target < 0 {
            from_fixed_parameter_16_16
        } else {
            fixed_parameter_target.saturating_mul(0x1_0000)
        };
        self.native_spline_controls.push(SplineControl {
            control_id,
            object_id,
            x: FixedCubicSpline::new(x),
            y: FixedCubicSpline::new(y),
            z: FixedCubicSpline::new(z),
            final_vector: (
                points.last().unwrap()[0],
                points.last().unwrap()[1],
                points.last().unwrap()[2],
            ),
            position_curve,
            from_alpha: from_alpha.clamp(0, 256),
            to_alpha: to_alpha.clamp(0, 256),
            alpha_curve,
            from_fixed_parameter_16_16,
            to_fixed_parameter_16_16,
            duration_ms: duration_ms.max(1) as u64,
            wall_elapsed_ms: 0,
            update_numerator,
            update_interval_ms,
            next_update_ms: update_interval_ms,
            spline_time_scale: if spline_time_scale == 0 {
                0x1_0000u32
            } else {
                u32::from(spline_time_scale)
            },

            force_complete: false,
            procedure_update_count: 0,
            last_vector: (i32::MIN, i32::MIN, i32::MIN),
            last_alpha: i32::MIN,
            last_fixed_parameter_16_16: i32::MIN,
        });
        control_id
    }

    #[allow(clippy::too_many_arguments)]
    pub fn schedule_path(
        &mut self,
        object_id: i32,
        from_position: (i32, i32),
        middle_position: (i32, i32),
        to_position: (i32, i32),
        position_curve: MotionCurve,
        from_alpha: i32,
        to_alpha: i32,
        duration_ms: i32,
        sample_rate_hz: i32,
        max_catchup_steps: i32,
    ) -> Option<u64> {
        if sample_rate_hz <= 0
            || duration_ms < 0
            || i64::from(duration_ms) * i64::from(sample_rate_hz) / 1000 > 1_000_000
        {
            return None;
        }
        let sample_count = ((i64::from(duration_ms) * i64::from(sample_rate_hz)) / 1000)
            .max(1)
            .min(i64::from(u32::MAX)) as u32;
        let path = build_three_point_path(
            from_position,
            middle_position,
            to_position,
            sample_count.saturating_add(1),
        )?;
        let interval_ms = if sample_count <= 1 {
            duration_ms.max(0) as u64
        } else {
            (1000 / sample_rate_hz).max(0) as u64
        };
        let control_id = self.allocate_id();
        self.native_special_path_controls.push(PathControl {
            control_id,
            object_id,
            path,
            position_curve,
            from_alpha: from_alpha.clamp(0, 256),
            to_alpha: to_alpha.clamp(0, 256),
            total_steps: sample_count,
            current_step: 0,
            interval_ms,
            accumulated_ms: 0,
            max_catchup_steps: max_catchup_steps.max(0) as u32,

            force_complete: false,
            procedure_update_count: 0,
            last_x: i32::MIN,
            last_y: i32::MIN,
            last_alpha: i32::MIN,
        });
        Some(control_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn schedule_shake(
        &mut self,
        object_id: i32,
        mode: i32,
        from_position: (i32, i32),
        amplitude: i32,
        frequency_hz: i32,
        cycles: i32,
        decay_percent: i32,
        sample_rate_hz: i32,
    ) -> Option<u64> {
        if !(0..6).contains(&mode)
            || frequency_hz <= 0
            || cycles <= 0
            || cycles > 1_000_000
            || sample_rate_hz <= 0
            || sample_rate_hz < frequency_hz
        {
            return None;
        }
        let interval_ms = 1000 / sample_rate_hz;
        if interval_ms <= 0 {
            // The target's following division would fault for a zero interval;
            // reject it here rather than silently inventing a waveform.
            return None;
        }
        let waveform_len = 1000 / (frequency_hz.saturating_mul(interval_ms));
        if waveform_len <= 0 {
            return None;
        }

        // sub_43C8A0 builds one amplitude value per cycle in 16.16, applying
        // (100-decay)/100 after each cycle with integer truncation.
        let mut envelope = Vec::with_capacity(cycles as usize);
        let mut amplitude_fixed = i64::from(amplitude) << 16;
        for _ in 0..cycles {
            envelope.push((amplitude_fixed >> 16) as i32);
            amplitude_fixed = i64::from(100 - decay_percent).saturating_mul(amplitude_fixed) / 100;
        }

        // The second table is the target's deterministic triangular phase
        // wave. Modes 4/5 start half a phase later; 0/2 invert its sign.
        let mut waveform = Vec::with_capacity(waveform_len as usize);
        let center = if mode < 4 { 0_i64 } else { 0x8000_i64 };
        let mut phase = center;
        let mut delta = i64::from(0x20000 / waveform_len);
        for _ in 0..waveform_len {
            phase += delta;
            if phase >= 0x10000 {
                delta = -delta;
                phase = 0x20000 - phase;
            }
            if phase <= 0 {
                delta = -delta;
                phase = -phase;
            }
            let value = if mode == 0 || mode == 2 {
                center - phase
            } else {
                phase - center
            };
            waveform.push(value as i32);
        }

        let control_id = self.allocate_id();
        self.native_shake_object_controls.push(ShakeControl {
            control_id,
            object_id,
            mode,
            base_x: from_position.0,
            base_y: from_position.1,
            envelope,
            waveform,
            total_steps: (cycles as u32).saturating_mul(waveform_len as u32),
            current_step: 0,
            interval_ms: interval_ms as u64,
            accumulated_ms: 0,

            force_complete: false,
            procedure_update_count: 0,
            last_x: i32::MIN,
            last_y: i32::MIN,
        });
        Some(control_id)
    }

    fn allocate_id(&mut self) -> u64 {
        self.next_native_control_id = self.next_native_control_id.wrapping_add(1).max(1);
        self.next_native_control_id
    }

    pub fn is_active(&self, control_id: u64) -> bool {
        self.native_object_controls
            .iter()
            .any(|track| track.control_id == control_id)
            || self
                .native_special_path_controls
                .iter()
                .any(|track| track.control_id == control_id)
            || self
                .native_spline_controls
                .iter()
                .any(|track| track.control_id == control_id)
            || self
                .native_shake_object_controls
                .iter()
                .any(|track| track.control_id == control_id)
    }

    /// Model the target's input-interrupt path. CProcCtrlDspObj samples input
    /// before its subclass updater; the updater then commits the exact terminal
    /// state and 0x431F00 reports terminal status 1. The first deferred value is
    /// the pre-terminal procedure progress, not an unconditional 1000.
    pub fn finish(&mut self, control_id: u64) -> bool {
        // CProcCtrlDspObj::Tick (0x431F00) does not publish the deferred
        // outputs until the subclass updater has actually run.  Input/forced
        // completion therefore only latches the request here; the next native
        // procedure poll performs one terminal updater invocation, increments
        // CProcCtrlDspObj+0x40, commits the exact endpoint, and records status 1.
        if let Some(track) = self
            .native_object_controls
            .iter_mut()
            .find(|track| track.control_id == control_id)
        {
            track.force_complete = true;
            return true;
        }
        if let Some(track) = self
            .native_special_path_controls
            .iter_mut()
            .find(|track| track.control_id == control_id)
        {
            track.force_complete = true;
            return true;
        }
        if let Some(track) = self
            .native_spline_controls
            .iter_mut()
            .find(|track| track.control_id == control_id)
        {
            track.force_complete = true;
            return true;
        }
        if let Some(track) = self
            .native_shake_object_controls
            .iter_mut()
            .find(|track| track.control_id == control_id)
        {
            track.force_complete = true;
            return true;
        }
        false
    }

    /// Cancellation reports the stored pre-updater progress, without sampling.
    pub fn abort_progress(&self, id: u64) -> i32 {
        for t in &self.native_object_controls {
            if t.control_id == id {
                return progress_per_mille_from_updates(t.procedure_update_count, t.duration_ms);
            }
        }
        for t in &self.native_spline_controls {
            if t.control_id == id {
                return progress_per_mille_from_updates(t.procedure_update_count, t.duration_ms);
            }
        }
        for t in &self.native_special_path_controls {
            if t.control_id == id {
                return progress_per_mille_from_updates(
                    t.procedure_update_count,
                    u64::from(t.total_steps),
                );
            }
        }
        for t in &self.native_shake_object_controls {
            if t.control_id == id {
                return progress_per_mille_from_updates(
                    t.procedure_update_count,
                    u64::from(t.total_steps),
                );
            }
        }
        0
    }
    pub fn abort(&mut self, control_id: u64) -> bool {
        // Base CProcedure cancellation (0x431AF0 -> 0x431F00) exits before the
        // subclass updater.  It therefore neither increments the procedure
        // update counter nor snaps the controlled object to its endpoint.
        if let Some(index) = self
            .native_object_controls
            .iter()
            .position(|track| track.control_id == control_id)
        {
            let track = self.native_object_controls.swap_remove(index);
            self.native_control_completions.insert(
                control_id,
                ControlCompletion {
                    progress_per_mille: progress_per_mille_from_updates(
                        track.procedure_update_count,
                        track.duration_ms,
                    ),
                    status: -1,
                },
            );
            return true;
        }
        if let Some(index) = self
            .native_special_path_controls
            .iter()
            .position(|track| track.control_id == control_id)
        {
            let track = self.native_special_path_controls.swap_remove(index);
            self.native_control_completions.insert(
                control_id,
                ControlCompletion {
                    progress_per_mille: progress_per_mille_from_updates(
                        track.procedure_update_count,
                        u64::from(track.total_steps),
                    ),
                    status: -1,
                },
            );
            return true;
        }
        if let Some(index) = self
            .native_spline_controls
            .iter()
            .position(|track| track.control_id == control_id)
        {
            let track = self.native_spline_controls.swap_remove(index);
            self.native_control_completions.insert(
                control_id,
                ControlCompletion {
                    progress_per_mille: progress_per_mille_from_updates(
                        track.procedure_update_count,
                        track.duration_ms,
                    ),
                    status: -1,
                },
            );
            return true;
        }
        if let Some(index) = self
            .native_shake_object_controls
            .iter()
            .position(|track| track.control_id == control_id)
        {
            let track = self.native_shake_object_controls.swap_remove(index);
            self.native_control_completions.insert(
                control_id,
                ControlCompletion {
                    progress_per_mille: progress_per_mille_from_updates(
                        track.procedure_update_count,
                        u64::from(track.total_steps),
                    ),
                    status: -1,
                },
            );
            return true;
        }
        false
    }

    pub fn take_completion(&mut self, control_id: u64) -> ControlCompletion {
        self.native_control_completions
            .remove(&control_id)
            .unwrap_or(ControlCompletion::NATURAL)
    }

    pub fn object_active(&self, object_id: i32) -> bool {
        self.native_object_controls
            .iter()
            .any(|track| track.object_id == object_id)
            || self
                .native_special_path_controls
                .iter()
                .any(|track| track.object_id == object_id)
            || self
                .native_spline_controls
                .iter()
                .any(|track| track.object_id == object_id)
            || self
                .native_shake_object_controls
                .iter()
                .any(|track| track.object_id == object_id)
    }

    pub fn tick_objects(&mut self, elapsed_ms: u64) -> Vec<MotionEvent> {
        self.tick_objects_filtered(elapsed_ms, None)
    }

    fn tick_objects_filtered(
        &mut self,
        elapsed_ms: u64,
        only_control: Option<u64>,
    ) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.native_object_controls.len() {
            if only_control.is_some_and(|control_id| {
                self.native_object_controls[index].control_id != control_id
            }) {
                index += 1;
                continue;
            }
            let mut completion = None;
            {
                let track = &mut self.native_object_controls[index];
                track.wall_elapsed_ms = track.wall_elapsed_ms.saturating_add(elapsed_ms);

                // The base CProcedure is first scheduled for +1 ms.  A zero
                // elapsed host pass is not a procedure updater invocation,
                // except when input/local forced completion makes 0x431F00
                // invoke the updater immediately.
                if !track.force_complete && elapsed_ms == 0 {
                    index += 1;
                    continue;
                }

                let sampled_elapsed = if track.force_complete {
                    track.duration_ms
                } else if track.update_numerator != 0 {
                    // Exact 0x432160 catch-up limiter: one subclass updater
                    // invocation samples no later than its current deadline,
                    // then advances that deadline by 1000*numerator/denominator.
                    // Do not collapse multiple native procedure polls into one
                    // rendered host frame; scheduler cadence is a separate layer.
                    let sampled = track.wall_elapsed_ms.min(track.next_update_ms);
                    track.next_update_ms = sampled.saturating_add(track.update_interval_ms);
                    sampled
                } else {
                    track.wall_elapsed_ms
                };
                let elapsed = sampled_elapsed.min(track.duration_ms);
                track.procedure_update_count = track.procedure_update_count.wrapping_add(1);

                let progress_8_24 = (((u128::from(elapsed)) << 24)
                    / u128::from(track.duration_ms.max(1)))
                .min(0x0100_0000) as i32;
                let position_weight = sample_curve(track.position_curve, progress_8_24);
                let alpha_weight = sample_curve(track.alpha_curve, progress_8_24);
                let x = interpolate_target_fixed(track.from_x, track.to_x, position_weight);
                let y = interpolate_target_fixed(track.from_y, track.to_y, position_weight);
                let alpha_parameter =
                    interpolate_target_fixed(track.from_alpha, track.to_alpha, alpha_weight)
                        .clamp(0, 256);
                let fixed_parameter_16_16 = if elapsed >= track.duration_ms {
                    track.to_fixed_parameter_16_16
                } else {
                    let delta = i64::from(track.to_fixed_parameter_16_16)
                        - i64::from(track.from_fixed_parameter_16_16);
                    let value = i64::from(track.from_fixed_parameter_16_16)
                        + (delta * elapsed as i64) / track.duration_ms.max(1) as i64;
                    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                };
                let changed = x != track.last_x
                    || y != track.last_y
                    || alpha_parameter != track.last_alpha
                    || fixed_parameter_16_16 != track.last_fixed_parameter_16_16;
                if changed {
                    track.last_x = x;
                    track.last_y = y;
                    track.last_alpha = alpha_parameter;
                    track.last_fixed_parameter_16_16 = fixed_parameter_16_16;
                    events.push(MotionEvent::ObjectUpdated {
                        object_id: track.object_id,
                        x,
                        y,
                        alpha_parameter,
                        fixed_parameter_16_16,
                    });
                }
                if elapsed >= track.duration_ms {
                    if track.last_x != track.to_x
                        || track.last_y != track.to_y
                        || track.last_alpha != track.to_alpha
                        || track.last_fixed_parameter_16_16 != track.to_fixed_parameter_16_16
                    {
                        events.push(MotionEvent::ObjectUpdated {
                            object_id: track.object_id,
                            x: track.to_x,
                            y: track.to_y,
                            alpha_parameter: track.to_alpha,
                            fixed_parameter_16_16: track.to_fixed_parameter_16_16,
                        });
                    }
                    events.push(MotionEvent::Finished {
                        object_id: track.object_id,
                    });
                    completion = Some((
                        track.control_id,
                        ControlCompletion {
                            progress_per_mille: progress_per_mille_from_updates(
                                track.procedure_update_count,
                                track.duration_ms,
                            ),
                            status: if track.force_complete { 1 } else { 0 },
                        },
                    ));
                }
            }
            if let Some((control_id, result)) = completion {
                self.native_control_completions.insert(control_id, result);
                self.native_object_controls.swap_remove(index);
            } else {
                index += 1;
            }
        }
        events
    }

    pub fn tick_paths(&mut self, elapsed_ms: u64) -> Vec<MotionEvent> {
        self.tick_paths_filtered(elapsed_ms, None)
    }

    fn tick_paths_filtered(
        &mut self,
        elapsed_ms: u64,
        only_control: Option<u64>,
    ) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.native_special_path_controls.len() {
            if only_control.is_some_and(|control_id| {
                self.native_special_path_controls[index].control_id != control_id
            }) {
                index += 1;
                continue;
            }
            let mut completion = None;
            {
                let track = &mut self.native_special_path_controls[index];
                let updater_invoked = if track.force_complete {
                    track.current_step = track.total_steps;
                    true
                } else if track.current_step < track.total_steps {
                    track.accumulated_ms = track.accumulated_ms.saturating_add(elapsed_ms);
                    let remaining = track.total_steps - track.current_step;
                    let mut due = if track.interval_ms == 0 {
                        remaining
                    } else {
                        (track.accumulated_ms / track.interval_ms).min(u64::from(remaining)) as u32
                    };
                    if due == 0 {
                        false
                    } else {
                        if track.max_catchup_steps != 0 && due > track.max_catchup_steps {
                            due = track.max_catchup_steps;
                            // 0x4320E0 resets the deadline to now+interval once
                            // its per-updater catch-up cap is reached.
                            track.accumulated_ms = 0;
                        } else if track.interval_ms != 0 {
                            track.accumulated_ms %= track.interval_ms;
                        } else {
                            track.accumulated_ms = 0;
                        }
                        track.current_step = track
                            .current_step
                            .saturating_add(due)
                            .min(track.total_steps);
                        true
                    }
                } else {
                    false
                };
                if !updater_invoked {
                    index += 1;
                    continue;
                }
                // 0x431F00 increments this once per virtual updater call, not
                // once per path sample advanced inside 0x4320E0.
                track.procedure_update_count = track.procedure_update_count.wrapping_add(1);

                let done = track.current_step >= track.total_steps;
                let (x, y, alpha_parameter) = if done {
                    let &(x, y) = track.path.last().unwrap_or(&(0, 0));
                    (x, y, track.to_alpha)
                } else {
                    let progress_8_24 = (((u64::from(track.current_step)) << 24)
                        / u64::from(track.total_steps.max(1)))
                    .min(0x0100_0000) as i32;
                    let weight = sample_curve(track.position_curve, progress_8_24);
                    let point_count = track.total_steps.saturating_add(1);
                    let path_index = ((u64::from(point_count) * weight.max(0) as u64) >> 16)
                        .min(u64::from(track.total_steps))
                        as usize;
                    let (x, y) = track.path[path_index];
                    let delta = i64::from(track.to_alpha - track.from_alpha);
                    let alpha = (i64::from(track.from_alpha)
                        + (delta * i64::from(track.current_step))
                            / i64::from(track.total_steps.max(1)))
                    .clamp(0, 256) as i32;
                    (x, y, alpha)
                };
                if x != track.last_x || y != track.last_y || alpha_parameter != track.last_alpha {
                    track.last_x = x;
                    track.last_y = y;
                    track.last_alpha = alpha_parameter;
                    events.push(MotionEvent::PathUpdated {
                        object_id: track.object_id,
                        x,
                        y,
                        alpha_parameter,
                    });
                }
                if done {
                    events.push(MotionEvent::Finished {
                        object_id: track.object_id,
                    });
                    completion = Some((
                        track.control_id,
                        ControlCompletion {
                            progress_per_mille: progress_per_mille_from_updates(
                                track.procedure_update_count,
                                u64::from(track.total_steps),
                            ),
                            status: if track.force_complete { 1 } else { 0 },
                        },
                    ));
                }
            }
            if let Some((control_id, result)) = completion {
                self.native_control_completions.insert(control_id, result);
                self.native_special_path_controls.swap_remove(index);
            } else {
                index += 1;
            }
        }
        events
    }

    pub fn tick_shakes(&mut self, elapsed_ms: u64) -> Vec<MotionEvent> {
        self.tick_shakes_filtered(elapsed_ms, None)
    }

    fn tick_shakes_filtered(
        &mut self,
        elapsed_ms: u64,
        only_control: Option<u64>,
    ) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.native_shake_object_controls.len() {
            if only_control.is_some_and(|control_id| {
                self.native_shake_object_controls[index].control_id != control_id
            }) {
                index += 1;
                continue;
            }
            let mut completion = None;
            {
                let track = &mut self.native_shake_object_controls[index];
                let updater_invoked = if track.force_complete {
                    track.current_step = track.total_steps;
                    true
                } else if track.current_step < track.total_steps {
                    track.accumulated_ms = track.accumulated_ms.saturating_add(elapsed_ms);
                    let remaining = track.total_steps - track.current_step;
                    let due = if track.interval_ms == 0 {
                        remaining
                    } else {
                        (track.accumulated_ms / track.interval_ms).min(u64::from(remaining)) as u32
                    };
                    if due == 0 {
                        false
                    } else {
                        if track.interval_ms != 0 {
                            track.accumulated_ms %= track.interval_ms;
                        } else {
                            track.accumulated_ms = 0;
                        }
                        // CProcShakeDspObj leaves its catch-up cap at zero, so
                        // 0x4320E0 may advance multiple samples in this one
                        // virtual updater invocation.
                        track.current_step = track
                            .current_step
                            .saturating_add(due)
                            .min(track.total_steps);
                        true
                    }
                } else {
                    false
                };
                if !updater_invoked {
                    index += 1;
                    continue;
                }
                track.procedure_update_count = track.procedure_update_count.wrapping_add(1);

                let done = track.current_step >= track.total_steps;
                let (x, y) = if done || track.waveform.is_empty() || track.envelope.is_empty() {
                    (track.base_x, track.base_y)
                } else {
                    let wave_len = track.waveform.len() as u32;
                    let step_index = track.current_step;
                    let envelope_index = (step_index / wave_len)
                        .min(track.envelope.len().saturating_sub(1) as u32)
                        as usize;
                    let wave_index = (step_index % wave_len) as usize;
                    let offset = ((i64::from(track.envelope[envelope_index])
                        * i64::from(track.waveform[wave_index]))
                        >> 16)
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX))
                        as i32;
                    match track.mode {
                        0 | 1 | 4 => (track.base_x, track.base_y.wrapping_add(offset)),
                        2 | 3 | 5 => (track.base_x.wrapping_add(offset), track.base_y),
                        _ => (track.base_x, track.base_y),
                    }
                };
                if x != track.last_x || y != track.last_y {
                    track.last_x = x;
                    track.last_y = y;
                    events.push(MotionEvent::ShakeUpdated {
                        object_id: track.object_id,
                        x,
                        y,
                    });
                }
                if done {
                    events.push(MotionEvent::Finished {
                        object_id: track.object_id,
                    });
                    completion = Some((
                        track.control_id,
                        ControlCompletion {
                            progress_per_mille: progress_per_mille_from_updates(
                                track.procedure_update_count,
                                u64::from(track.total_steps),
                            ),
                            status: if track.force_complete { 1 } else { 0 },
                        },
                    ));
                }
            }
            if let Some((control_id, result)) = completion {
                self.native_control_completions.insert(control_id, result);
                self.native_shake_object_controls.swap_remove(index);
            } else {
                index += 1;
            }
        }
        events
    }

    pub fn tick_splines(&mut self, elapsed_ms: u64) -> Vec<MotionEvent> {
        self.tick_splines_filtered(elapsed_ms, None)
    }

    fn tick_splines_filtered(
        &mut self,
        elapsed_ms: u64,
        only_control: Option<u64>,
    ) -> Vec<MotionEvent> {
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.native_spline_controls.len() {
            if only_control.is_some_and(|control_id| {
                self.native_spline_controls[index].control_id != control_id
            }) {
                index += 1;
                continue;
            }
            let mut completion = None;
            {
                let track = &mut self.native_spline_controls[index];
                track.wall_elapsed_ms = track.wall_elapsed_ms.saturating_add(elapsed_ms);
                if !track.force_complete && elapsed_ms == 0 {
                    index += 1;
                    continue;
                }
                let sampled_elapsed = if track.force_complete {
                    track.duration_ms
                } else if track.update_numerator != 0 {
                    let sampled = track.wall_elapsed_ms.min(track.next_update_ms);
                    track.next_update_ms = sampled.saturating_add(track.update_interval_ms);
                    sampled
                } else {
                    track.wall_elapsed_ms
                };
                let elapsed = sampled_elapsed.min(track.duration_ms);
                track.procedure_update_count = track.procedure_update_count.wrapping_add(1);
                let done = elapsed >= track.duration_ms;
                let progress_8_24 = (((u128::from(elapsed)) << 24)
                    / u128::from(track.duration_ms.max(1)))
                .min(0x0100_0000) as i32;
                let vector = if done {
                    track.final_vector
                } else {
                    let weight = sample_curve(track.position_curve, progress_8_24);
                    let t = f64::from(weight) / 65_536.0;
                    (track.x.sample(t), track.y.sample(t), track.z.sample(t))
                };
                let alpha_parameter = if done {
                    track.to_alpha
                } else {
                    let scaled_progress = ((i128::from(progress_8_24) * 65_536)
                        / i128::from(track.spline_time_scale.max(1)))
                    .clamp(0, 0x0100_0000) as i32;
                    let weight = sample_curve(track.alpha_curve, scaled_progress);
                    interpolate_target_fixed(track.from_alpha, track.to_alpha, weight).clamp(0, 256)
                };
                let fixed_parameter_16_16 = if done {
                    track.to_fixed_parameter_16_16
                } else {
                    let delta = i64::from(track.to_fixed_parameter_16_16)
                        - i64::from(track.from_fixed_parameter_16_16);
                    (i64::from(track.from_fixed_parameter_16_16)
                        + delta * elapsed as i64 / track.duration_ms.max(1) as i64)
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                };
                if vector != track.last_vector
                    || alpha_parameter != track.last_alpha
                    || fixed_parameter_16_16 != track.last_fixed_parameter_16_16
                {
                    track.last_vector = vector;
                    track.last_alpha = alpha_parameter;
                    track.last_fixed_parameter_16_16 = fixed_parameter_16_16;
                    events.push(MotionEvent::SplineUpdated {
                        object_id: track.object_id,
                        fixed_x_16_16: vector.0,
                        fixed_y_16_16: vector.1,
                        fixed_z_16_16: vector.2,
                        alpha_parameter,
                        fixed_parameter_16_16,
                    });
                }
                if done {
                    events.push(MotionEvent::Finished {
                        object_id: track.object_id,
                    });
                    completion = Some((
                        track.control_id,
                        ControlCompletion {
                            progress_per_mille: progress_per_mille_from_updates(
                                track.procedure_update_count,
                                track.duration_ms,
                            ),
                            status: if track.force_complete { 1 } else { 0 },
                        },
                    ));
                }
            }
            if let Some((control_id, result)) = completion {
                self.native_control_completions.insert(control_id, result);
                self.native_spline_controls.swap_remove(index);
            } else {
                index += 1;
            }
        }
        events
    }

    /// Advance only the CProcCtrlDspObj-family instance currently being
    /// polled by the VM cooperative scheduler.  The target does not advance
    /// all graph controls from a renderer-wide frame callback.
    pub fn tick_control(&mut self, control_id: u64, elapsed_ms: u64) -> Vec<MotionEvent> {
        let mut events = self.tick_objects_filtered(elapsed_ms, Some(control_id));
        events.extend(self.tick_paths_filtered(elapsed_ms, Some(control_id)));
        events.extend(self.tick_splines_filtered(elapsed_ms, Some(control_id)));
        events.extend(self.tick_shakes_filtered(elapsed_ms, Some(control_id)));
        events
    }

    pub fn clear_object(&mut self, object_id: i32) {
        self.native_object_controls
            .retain(|track| track.object_id != object_id);
        self.native_special_path_controls
            .retain(|track| track.object_id != object_id);
        self.native_spline_controls
            .retain(|track| track.object_id != object_id);
        self.native_shake_object_controls
            .retain(|track| track.object_id != object_id);
    }
}
#[derive(Debug, Clone, Copy)]
pub enum MotionEvent {
    ObjectUpdated {
        object_id: i32,
        x: i32,
        y: i32,
        alpha_parameter: i32,
        fixed_parameter_16_16: i32,
    },
    Finished {
        object_id: i32,
    },
    PathUpdated {
        object_id: i32,
        x: i32,
        y: i32,
        alpha_parameter: i32,
    },
    ShakeUpdated {
        object_id: i32,
        x: i32,
        y: i32,
    },
    SplineUpdated {
        object_id: i32,
        fixed_x_16_16: i32,
        fixed_y_16_16: i32,
        fixed_z_16_16: i32,
        alpha_parameter: i32,
        fixed_parameter_16_16: i32,
    },
}

#[derive(Debug, Clone)]
struct ObjectControl {
    control_id: u64,
    object_id: i32,
    from_x: i32,
    from_y: i32,
    to_x: i32,
    to_y: i32,
    position_curve: MotionCurve,
    from_alpha: i32,
    to_alpha: i32,
    alpha_curve: MotionCurve,
    from_fixed_parameter_16_16: i32,
    to_fixed_parameter_16_16: i32,
    duration_ms: u64,
    wall_elapsed_ms: u64,
    update_numerator: i32,
    update_interval_ms: u64,
    next_update_ms: u64,

    force_complete: bool,
    procedure_update_count: u32,
    last_x: i32,
    last_y: i32,
    last_alpha: i32,
    last_fixed_parameter_16_16: i32,
}

#[derive(Debug, Clone)]
struct PathControl {
    control_id: u64,
    object_id: i32,
    path: Vec<(i32, i32)>,
    position_curve: MotionCurve,
    from_alpha: i32,
    to_alpha: i32,
    total_steps: u32,
    current_step: u32,
    interval_ms: u64,
    accumulated_ms: u64,
    max_catchup_steps: u32,

    force_complete: bool,
    procedure_update_count: u32,
    last_x: i32,
    last_y: i32,
    last_alpha: i32,
}

#[derive(Debug, Clone)]
struct ShakeControl {
    control_id: u64,
    object_id: i32,
    mode: i32,
    base_x: i32,
    base_y: i32,
    envelope: Vec<i32>,
    waveform: Vec<i32>,
    total_steps: u32,
    current_step: u32,
    interval_ms: u64,
    accumulated_ms: u64,

    force_complete: bool,
    procedure_update_count: u32,
    last_x: i32,
    last_y: i32,
}

#[derive(Debug, Clone)]
struct SplineControl {
    control_id: u64,
    object_id: i32,
    x: FixedCubicSpline,
    y: FixedCubicSpline,
    z: FixedCubicSpline,
    final_vector: (i32, i32, i32),
    position_curve: MotionCurve,
    from_alpha: i32,
    to_alpha: i32,
    alpha_curve: MotionCurve,
    from_fixed_parameter_16_16: i32,
    to_fixed_parameter_16_16: i32,
    duration_ms: u64,
    wall_elapsed_ms: u64,
    update_numerator: i32,
    update_interval_ms: u64,
    next_update_ms: u64,
    spline_time_scale: u32,

    force_complete: bool,
    procedure_update_count: u32,
    last_vector: (i32, i32, i32),
    last_alpha: i32,
    last_fixed_parameter_16_16: i32,
}

#[derive(Debug, Clone)]
struct FixedCubicSpline {
    values: Vec<i32>,
    second_derivatives: Vec<f64>,
}

impl FixedCubicSpline {
    fn new(values: Vec<i32>) -> Self {
        let count = values.len();
        let mut second_derivatives = vec![0.0; count];
        if count > 2 {
            let mut upper = vec![0.0; count];
            for index in 1..count - 1 {
                let denominator = 4.0 - upper[index - 1];
                upper[index] = 1.0 / denominator;
                second_derivatives[index] = (3.0
                    * (f64::from(values[index - 1]) - 2.0 * f64::from(values[index])
                        + f64::from(values[index + 1]))
                    - second_derivatives[index - 1])
                    / denominator;
            }
            for index in (1..count - 1).rev() {
                second_derivatives[index] -= upper[index] * second_derivatives[index + 1];
            }
        }
        Self {
            values,
            second_derivatives,
        }
    }

    fn sample(&self, t: f64) -> i32 {
        if self.values.len() <= 1 {
            return self.values.first().copied().unwrap_or_default();
        }
        let scaled = t.clamp(0.0, 1.0) * (self.values.len() - 1) as f64;
        let index = (scaled.floor() as usize).min(self.values.len() - 2);
        let local = scaled - index as f64;
        let a = 1.0 - local;
        let b = local;
        let value = a * f64::from(self.values[index])
            + b * f64::from(self.values[index + 1])
            + ((a * a * a - a) * self.second_derivatives[index]
                + (b * b * b - b) * self.second_derivatives[index + 1])
                / 3.0;
        value as i32
    }
}

fn progress_per_mille_from_updates(update_count: u32, denominator: u64) -> i32 {
    if denominator == 0 {
        return 0;
    }
    let value = (u128::from(update_count) * 1000) / u128::from(denominator);
    value.min(i32::MAX as u128) as i32
}

fn round_half_away_from_zero(value: f64) -> i32 {
    if value < 0.0 {
        (value - 0.5) as i32
    } else {
        (value + 0.5) as i32
    }
}

/// Target sub_494730 samples the 2D path used by CProcCtrlDspObjSp. X is
/// treated as the monotonic independent axis and Y is a natural cubic through
/// current/middle/final points. The sampled table is what sub_432AC0 indexes.
fn build_three_point_path(
    start: (i32, i32),
    middle: (i32, i32),
    end: (i32, i32),
    sample_points: u32,
) -> Option<Vec<(i32, i32)>> {
    if sample_points < 2 {
        return None;
    }
    let ascending = start.0 < end.0;
    if ascending {
        if middle.0 <= start.0 || middle.0 >= end.0 {
            return None;
        }
    } else if start.0 > end.0 {
        if middle.0 >= start.0 || middle.0 <= end.0 {
            return None;
        }
    } else {
        return None;
    }

    let x1 = if ascending {
        f64::from(middle.0 - start.0)
    } else {
        f64::from(start.0 - middle.0)
    };
    let x2 = if ascending {
        f64::from(end.0 - start.0)
    } else {
        f64::from(start.0 - end.0)
    };
    let y0 = 0.0_f64;
    let y1 = f64::from(middle.1 - start.1);
    let y2 = f64::from(end.1 - start.1);
    let h0 = x1;
    let h1 = x2 - x1;
    if h0 <= 0.0 || h1 <= 0.0 {
        return None;
    }
    // sub_494560 stores c=M/2. Natural endpoints are zero and only the
    // middle coefficient is nonzero for this fixed three-point path.
    let c1 = 3.0 * ((y2 - y1) / h1 - (y1 - y0) / h0) / (2.0 * (h0 + h1));
    let step = x2 / f64::from(sample_points - 1);
    let mut path = Vec::with_capacity(sample_points as usize);
    for index in 0..sample_points {
        let x = step * f64::from(index);
        let (base_x, base_y, h, c0, c_next) = if x < x1 {
            (0.0, y0, h0, 0.0, c1)
        } else {
            (x1, y1, h1, c1, 0.0)
        };
        let dx = x - base_x;
        let slope = if base_x == 0.0 {
            (y1 - y0) / h0
        } else {
            (y2 - y1) / h1
        };
        let y = base_y
            + dx * (slope - (2.0 * c0 + c_next) * h / 3.0
                + ((c_next - c0) / (3.0 * h) * dx + c0) * dx);
        let x_delta = round_half_away_from_zero(x);
        let sampled_x = if ascending {
            start.0.wrapping_add(x_delta)
        } else {
            start.0.wrapping_sub(x_delta)
        };
        let sampled_y = start.1.wrapping_add(round_half_away_from_zero(y));
        path.push((sampled_x, sampled_y));
    }
    Some(path)
}

fn interpolate_target_fixed(from: i32, to: i32, weight_16_16: i32) -> i32 {
    let delta = i64::from(to) - i64::from(from);
    let value = i64::from(from) + ((delta * i64::from(weight_16_16)) >> 16);
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
