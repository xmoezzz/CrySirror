//! BP control parameters and CProcedure lifetimes over shared sampled motion.
use super::*;
use crysirror_bgi_vm::{GraphControlSignal, native_call::NativeProcedureStart};
use crysirror_image::motion::{MotionControls, MotionCurve, MotionEvent};
#[derive(Default)]
pub(crate) struct ControlState {
    motion: MotionControls,
    shake: crysirror_image::motion_shake::Oscillation,
    shake_clock: Option<u64>,
    shake_tick_clock: Option<u64>,
    shake_lock: bool,
    records: BTreeMap<u64, Control>,
}
struct Control {
    object: i32,
    last_poll: u64,
    input: Option<InputLatch>,
    result: Option<[i32; 2]>,
}
struct InputLatch {
    scope: i32,
    pointer: u64,
    auxiliary: u64,
    pointer_eligible: bool,
    keyboard_eligible: bool,
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    (group == 0xb0 && id == 0x08)
        || (group == 0x90 && matches!(id, 0x20..=0x24 | 0x28 | 0x29 | 0x2c))
}
fn curve(value: i32) -> MotionCurve {
    use MotionCurve::*;
    match value {
        1 => HalfCosine,
        2 => SineOut,
        3 => SineIn,
        4 => QuadraticIn,
        5 => QuadraticOut,
        6 => PowerTwoHalfIn,
        7 => PowerTwoHalfOut,
        8 => CubicIn,
        9 => CubicOut,
        10 => QuarticIn,
        11 => QuarticOut,
        12 => QuinticIn,
        13 => QuinticOut,
        14 => SexticIn,
        15 => SexticOut,
        _ => Linear, // Original sub_41A690's defined default branch.
    }
}
impl BgiVm {
    pub(crate) fn control_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<()> {
        let a = &call.args;
        if call.group == 0xb0 {
            ensure!(
                self.controls
                    .shake
                    .configure(a[0], a[1], a[2], a[3], a[4], a[5]),
                "BGI screen shake direction/frequency/cycles/rate"
            );
            self.controls.shake_clock = Some(host.now_ms());
            self.controls.shake_tick_clock = None;
            self.controls.shake_lock = a[6] != 0;
            vm.update_screen_shake(true);
            vm.commit_call_with_start(&[], NativeProcedureStart::GraphShake)?;
            self.calls += 1;
            return Ok(());
        }
        let object = a[0];
        if call.id != 0x2c {
            self.graph.require(object)?;
        }
        let props = self.graph.objects.get(&object).cloned().unwrap_or_default();
        let n = props.native;
        let from = (n.position_x, n.position_y);
        let from_fixed = (((n.fixed_parameter_16_16 as u32 >> 16) & 0xffff) as i32) << 16;
        let from_alpha = props.alpha_virtual();
        let (input_enabled, scope) = match call.id {
            0x20 => (a[4] != 0, a[5]),
            0x21 => (a[7] != 0, a[8]),
            0x22 => (a[5] != 0, a[6]),
            0x23 => (a[8] != 0, a[9]),
            0x24 | 0x28 | 0x29 => (a[10] != 0, a[11]),
            0x2c => (a[7] != 0, a[8]),
            _ => bail!("BGI control dispatch"),
        };
        if call.id != 0x2c {
            target_checks::priority(scope)?;
        }
        let id = match call.id {
            0x20..=0x23 | 0x28 => {
                let (
                    to,
                    position_curve,
                    alpha,
                    alpha_curve,
                    fixed,
                    duration,
                    denominator,
                    numerator,
                ) = match call.id {
                    0x20 => (from, 0, a[1], 0, -1, a[2], a[3], 0),
                    0x21 => ((a[1], a[2]), a[3], a[4], 0, -1, a[5], a[6], 0),
                    0x22 => (from, 0, a[1], 0, -1, a[2], a[3], a[4]),
                    0x23 => ((a[1], a[2]), a[3], a[4], 0, -1, a[5], a[6], a[7]),
                    _ => ((a[1], a[2]), a[3], a[4], a[5], a[6], a[7], a[8], a[9]),
                };
                target_checks::effect_level(alpha)?;
                if call.id == 0x28 {
                    target_checks::addition_level(fixed)?;
                }
                ensure!(denominator != 0, "BGI control update denominator is zero");
                let to_fixed = if fixed < 0 {
                    from_fixed
                } else {
                    fixed.saturating_mul(65536)
                };
                self.controls.motion.schedule_object(
                    object,
                    from,
                    to,
                    curve(position_curve),
                    from_alpha,
                    alpha,
                    curve(alpha_curve),
                    from_fixed,
                    to_fixed,
                    duration.max(1),
                    denominator,
                    numerator,
                )
            }
            0x24 => {
                target_checks::effect_level(a[6])?;
                self.controls
                    .motion
                    .schedule_path(
                        object,
                        from,
                        (a[1], a[2]),
                        (a[3], a[4]),
                        curve(a[5]),
                        from_alpha,
                        a[6],
                        a[7].max(0),
                        a[8],
                        a[9],
                    )
                    .context("BGI control path direction/rate/sample budget")?
            }
            0x29 => {
                target_checks::effect_level(a[4])?;
                target_checks::addition_level(a[6])?;
                ensure!(a[8] != 0, "BGI spline update denominator is zero");
                let count = a[1].clamp(0, 256) as usize;
                ensure!(count > 0, "BGI spline has no points");
                let bytes = vm.memory(a[2] as u32, count * 16)?;
                let points = bytes
                    .chunks_exact(16)
                    .map(|p| {
                        std::array::from_fn(|i| {
                            i32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
                        })
                    })
                    .collect::<Vec<[i32; 4]>>();
                let packed = a[5] as u32;
                self.controls.motion.schedule_spline(
                    object,
                    (
                        n.fixed_position_x_16_16,
                        n.fixed_position_y_16_16,
                        n.fixed_position_z_16_16,
                    ),
                    &points,
                    curve(a[3]),
                    from_alpha,
                    a[4],
                    curve((packed & 0xffff) as i32),
                    from_fixed,
                    a[6],
                    a[7].max(1),
                    a[8],
                    a[9],
                    (packed >> 16) as u16,
                )
            }
            0x2c if !self.graph.tree.contains(object) => 0,
            0x2c => self
                .controls
                .motion
                .schedule_shake(object, a[1], from, a[2], a[3], a[4], a[5], a[6])
                .context("BGI object shake pattern/frequency/cycles/rate/sample budget")?,
            _ => unreachable!(),
        };
        ensure!(
            self.controls.records.len() < 4096,
            "BGI concurrent control limit"
        );
        let input = if input_enabled && id != 0 {
            let packed = scope.wrapping_shl(16) | 0xffff;
            vm.register_selection_input(id, packed, true, true);
            let (pointer, auxiliary, _, _, _) = vm.native_control_input_snapshot(scope);
            Some(InputLatch {
                scope,
                pointer,
                auxiliary,
                pointer_eligible: false,
                keyboard_eligible: false,
            })
        } else {
            None
        };
        self.controls.records.insert(
            id,
            Control {
                object,
                last_poll: host.now_ms(),
                input,
                result: (id == 0).then_some([1000, 0]),
            },
        );
        self.sync_control_states(vm);
        vm.commit_call_with_start(
            &[],
            NativeProcedureStart::GraphControl {
                object_id: object,
                control_id: id,
            },
        )?;
        self.calls += 1;
        Ok(())
    }
    fn sync_control_states(&self, vm: &mut Machine) {
        vm.update_graph_controls(
            self.controls
                .records
                .iter()
                .map(|(&id, r)| {
                    (
                        id,
                        r.object,
                        self.controls.motion.is_active(id),
                        self.controls.motion.abort_progress(id),
                        r.result.unwrap_or([0, -1]),
                    )
                })
                .collect(),
        );
    }
    pub(crate) fn tick_controls(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
    ) -> Result<()> {
        if vm.take_screen_shake_cancel() {
            self.controls.shake.cancel();
        }
        if self.controls.shake_tick_clock != Some(host.now_ms()) {
            let elapsed = self
                .controls
                .shake_clock
                .map(|last| host.now_ms().saturating_sub(last))
                .unwrap_or(0);
            self.controls.shake_clock = Some(host.now_ms());
            self.controls.shake_tick_clock = Some(host.now_ms());
            let offset = self
                .controls
                .shake
                .tick(elapsed, &mut || vm.next_shared_native_rand());
            self.set_screen_shake_offset([offset.0 as i32, offset.1 as i32]);
        } else if !self.controls.shake.active() {
            self.set_screen_shake_offset([0; 2]);
        }
        vm.update_screen_shake(self.controls.shake.active());
        for signal in vm.take_graph_control_signals() {
            let id = match signal {
                GraphControlSignal::Poll(id)
                | GraphControlSignal::Cancel(id)
                | GraphControlSignal::Stop(id, _)
                | GraphControlSignal::Release(id) => id,
            };
            match signal {
                GraphControlSignal::Release(_) => {
                    if let Some(record) = self.controls.records.remove(&id) {
                        if let Some(latch) = record.input {
                            vm.unregister_selection_input(
                                id,
                                latch.scope.wrapping_shl(16) | 0xffff,
                                true,
                                true,
                            );
                        }
                    }
                }
                GraphControlSignal::Cancel(_) => {
                    self.controls.motion.abort(id);
                    if let Some(record) = self.controls.records.get_mut(&id) {
                        let result = self.controls.motion.take_completion(id);
                        record.result = Some([result.progress_per_mille, result.status]);
                    }
                }
                GraphControlSignal::Stop(_, payload) => {
                    if payload != 0
                        || self
                            .controls
                            .records
                            .get(&id)
                            .is_some_and(|r| r.input.is_some())
                    {
                        self.controls.motion.finish(id);
                    }
                }
                GraphControlSignal::Poll(_) => {
                    let Some(record) = self.controls.records.get_mut(&id) else {
                        continue;
                    };
                    if !self.graph.tree.contains(record.object) {
                        self.controls.motion.abort(id);
                    } else {
                        if let Some(latch) = record.input.as_mut() {
                            let (pointer, auxiliary, pe, ke, gate) =
                                vm.native_control_input_snapshot(latch.scope);
                            let interrupt = gate
                                || pe && latch.pointer_eligible && pointer > latch.pointer
                                || ke && latch.keyboard_eligible && auxiliary > latch.auxiliary;
                            latch.pointer = pointer;
                            latch.auxiliary = auxiliary;
                            latch.pointer_eligible = pe;
                            latch.keyboard_eligible = ke;
                            if interrupt {
                                vm.consume_native_control_input(latch.scope);
                                self.controls.motion.finish(id);
                            }
                        }
                        let elapsed = host.now_ms().saturating_sub(record.last_poll);
                        record.last_poll = host.now_ms();
                        let events = self.controls.motion.tick_control(id, elapsed);
                        for event in events {
                            self.apply_control_event(host, event)?;
                        }
                    }
                    if !self.controls.motion.is_active(id) {
                        let result = self.controls.motion.take_completion(id);
                        if let Some(record) = self.controls.records.get_mut(&id) {
                            record.result = Some([result.progress_per_mille, result.status]);
                        }
                    }
                }
            }
        }
        self.sync_control_states(vm);
        Ok(())
    }
    fn apply_control_event(
        &mut self,
        host: &mut dyn HostServices,
        event: MotionEvent,
    ) -> Result<()> {
        match event {
            MotionEvent::ObjectUpdated {
                object_id,
                x,
                y,
                alpha_parameter,
                fixed_parameter_16_16,
            } => {
                self.graph.position(object_id, x, y);
                self.set_control_parameters(
                    host,
                    object_id,
                    Some(alpha_parameter),
                    Some(fixed_parameter_16_16),
                )?;
            }
            MotionEvent::PathUpdated {
                object_id,
                x,
                y,
                alpha_parameter,
            } => {
                self.graph.position(object_id, x, y);
                self.set_control_parameters(host, object_id, Some(alpha_parameter), None)?;
            }
            MotionEvent::ShakeUpdated { object_id, x, y } => self.graph.position(object_id, x, y),
            MotionEvent::SplineUpdated {
                object_id,
                fixed_x_16_16,
                fixed_y_16_16,
                fixed_z_16_16,
                alpha_parameter,
                fixed_parameter_16_16,
            } => {
                let n = &mut self.graph.objects.entry(object_id).or_default().native;
                let round = |v: i32| {
                    if n.fixed_position_rounding_enabled != 0
                        && (fixed_z_16_16 == 0 || n.fixed_position_rounding_mode == 1)
                    {
                        v.wrapping_add(0x8000) & !0xffff
                    } else {
                        v
                    }
                };
                let x = round(fixed_x_16_16);
                let y = round(fixed_y_16_16);
                n.fixed_position_x_16_16 = x;
                n.fixed_position_y_16_16 = y;
                n.fixed_position_z_16_16 = fixed_z_16_16;
                if n.fixed_position_updates_integer_position != 0 {
                    self.graph.position(object_id, x >> 16, y >> 16);
                }
                self.set_control_parameters(
                    host,
                    object_id,
                    Some(alpha_parameter),
                    Some(fixed_parameter_16_16),
                )?;
            }
            MotionEvent::Finished { .. } => {}
        }
        Ok(())
    }
}
