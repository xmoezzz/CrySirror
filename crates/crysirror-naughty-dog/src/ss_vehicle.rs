// SPDX-License-Identifier: MPL-2.0
//! Tank and helicopter script natives; see `crate::vehicle`.
use super::{Ctx, Wait};
use crate::dc::{Dc, sid};
use crate::script::{MAX_ARGS, Status};
use crate::vehicle::{HELI_SPEED, Order, TANK_SPEED, Target, Vehicle};
use anyhow::Result;

/// `heli-enable-wind`'s script setting (`sub_15F480`, 30 frames).
pub const SETTING_HELI_WIND: u32 = 0xc282_3eeb;

type Reply = Option<(u32, Status)>;

impl Ctx<'_> {
    /// The live object's vehicle, created on first use.
    fn vehicle(&mut self, name: u32) -> Option<&mut Vehicle> {
        let name = if name == sid("self") { self.proc.map_or(name, |p| self.inner.procs[p].name) } else { name };
        let name = self.inner.process(name).and_then(|p| self.inner.procs[p].spawner).unwrap_or(name);
        let at = self.world.locator(name);
        let vehicles = self.world.vehicles()?;
        if !vehicles.vehicles.contains_key(&name) {
            let (p, q) = at?;
            vehicles.vehicles.insert(name, Vehicle::new(p, q));
        }
        vehicles.vehicles.get_mut(&name)
    }
    fn vehicle_name(&self, a: &[u32; MAX_ARGS], n: usize) -> u32 {
        let name = if n > 0 { a[0] } else { self.proc.map_or(0, |p| self.inner.procs[p].name) };
        self.inner.process(name).and_then(|p| self.inner.procs[p].spawner).unwrap_or(name)
    }
    fn point(dc: &Dc, at: u32) -> Result<[f32; 3]> {
        Ok([dc.mem.f32(at)?, dc.mem.f32(at + 4)?, dc.mem.f32(at + 8)?])
    }
    /// `sub_83AED4`: the argument count must lie in `min..=max`.
    fn tank_args(&mut self, name: &str, usage: &str, n: usize, min: usize, max: usize) -> bool {
        if n < min || n > max {
            self.inner.error(format!("{name}: expected {usage}"));
            return false;
        }
        true
    }

    pub(super) fn vehicle_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let f = |i: usize, default: f32| if n > i { f32::from_bits(a[i]) } else { default };
        let who = self.vehicle_name(a, n);
        match name {
            "tank-get-health" => ok(self.vehicle(who).map_or(0, |v| v.health)),
            "tank-set-health" => {
                if n > 1 {
                    let destroyed = self.vehicle(who).is_some_and(|v| v.set_health(a[1]));
                    if destroyed {
                        self.world.kill(who);
                    }
                }
                ok(0)
            }
            "tank-teleport" | "wait-tank-teleport" => {
                let min = if name == "tank-teleport" { 2 } else { 3 };
                if !self.tank_args(name, "(tank-name locator)", n, min, 3) || a[1] == 0 {
                    return ok(1);
                }
                let (p, q) = Self::frame(dc, a[1])?;
                if let Some(v) = self.vehicle(who) {
                    v.position = p;
                    let fwd = glam::Quat::from_array(q).normalize() * glam::Vec3::Z;
                    v.yaw = fwd.x.atan2(fwd.z);
                    v.order = None;
                }
                self.world.place(who, p, q);
                if name == "wait-tank-teleport" {
                    if self.proc.is_none() {
                        self.inner.error("wait-tank-teleport: Called outside script context -- will not wait".into());
                        return ok(0);
                    }
                    let frame = self.inner.frame + 1;
                    return Ok(Some(self.suspend(Wait::Frame(frame))));
                }
                ok(0)
            }
            "tank-30cal-fire-at-location" | "tank-30cal-fire-at-object" => {
                if !self.tank_args(name, "(tank-name target)", n, 2, 2) || a[1] == 0 {
                    return ok(1);
                }
                let target = if name.ends_with("location") { Target::Point(Self::point(dc, a[1])?) } else { Target::Object(a[1]) };
                if let Some(v) = self.vehicle(who) {
                    v.gun = Some(target);
                }
                ok(0)
            }
            "tank-30cal-cease-fire" => {
                if self.tank_args(name, "(tank-name)", n, 1, 1) {
                    if let Some(v) = self.vehicle(who) {
                        v.gun = None;
                    }
                }
                ok(0)
            }
            // (wait-tank-fire-at-object tank object [pause]).
            "wait-tank-fire-at-object" => {
                if !self.tank_args(name, "(tank-name object-name pause-time)", n, 2, 3) {
                    return ok(1);
                }
                let seconds = f(2, 1.);
                let Some(v) = self.vehicle(who) else { return ok(0) };
                v.order = Some(Order::Fire { target: Target::Object(a[1]), seconds });
                Ok(Some(self.suspend(Wait::Vehicle(who))))
            }
            "tank-move-tracking-object" => {
                if !self.tank_args(name, "(tank-name object min-distance max-distance move-speed accel)", n, 3, 6) {
                    return ok(1);
                }
                let (min, max, speed) = (f(2, 0.), f(3, f(2, 0.)), f(4, TANK_SPEED));
                if let Some(v) = self.vehicle(who) {
                    v.order = Some(Order::Track { object: a[1], min, max, speed });
                }
                ok(0)
            }
            "wait-tank-move-to" | "wait-tank-reverse-to" | "heli-move-to" | "wait-heli-move-to" => {
                if n < 2 || a[1] == 0 {
                    self.inner.error(format!("{name}: expected (name position [speed])"));
                    return ok(0);
                }
                let target = Self::point(dc, a[1])?;
                let speed = f(2, if name.contains("heli") { HELI_SPEED } else { TANK_SPEED });
                let Some(v) = self.vehicle(who) else { return ok(0) };
                v.order = Some(Order::MoveTo { target, speed });
                if name.starts_with("wait-") {
                    return Ok(Some(self.suspend(Wait::Vehicle(who))));
                }
                ok(1)
            }
            "heli-move-to-spline" => {
                let speed = f(2, HELI_SPEED);
                if n > 1 {
                    if let Some(v) = self.vehicle(who) {
                        v.order = Some(Order::Spline { spline: a[1], distance: 0., speed });
                    }
                }
                ok(0)
            }
            // (tank-dist-to-point tank point), `sub_83CBE0`.
            "tank-dist-to-point" => {
                if n <= 1 {
                    self.inner.error("tank-dist-to-point: expected (tank-name point)".into());
                    return ok(0f32.to_bits());
                }
                if a[1] == 0 {
                    self.inner.error("tank-dist-to-point: invalid test-point argument.".into());
                    return ok(0f32.to_bits());
                }
                let p = Self::point(dc, a[1])?;
                match self.vehicle(who) {
                    Some(v) => ok(glam::Vec3::from_array(v.position).distance(glam::Vec3::from_array(p)).to_bits()),
                    None => {
                        self.inner.error("tank-dist-to-point: invalid tank arguement.".into());
                        ok(0f32.to_bits())
                    }
                }
            }
            "heli-enable-wind" => {
                let until = self.inner.frame + 30;
                self.inner.settings.insert(SETTING_HELI_WIND, super::Setting { value: 1, until });
                ok(0)
            }
            // (heli-set-target-locator heli locator [radius]).
            "heli-set-target-locator" => {
                if n > 1 && a[1] != 0 {
                    let p = Self::point(dc, a[1])?;
                    let radius = f(2, 0.);
                    if let Some(v) = self.vehicle(who) {
                        v.aim = Some((Target::Point(p), radius));
                    }
                }
                ok(0)
            }
            // (heli-set-target-object heli [object player]).
            "heli-set-target-object" => {
                let object = if n > 1 { a[1] } else { sid("player") };
                if self.world.locator(object).is_some() {
                    if let Some(v) = self.vehicle(who) {
                        v.aim = Some((Target::Object(object), 0.));
                    }
                }
                ok(0)
            }
            // (heli-set-firing-state heli [state] [enabled]), `sub_809628`:
            // both default to true.
            "heli-set-firing-state" => {
                let (state, enabled) = (n < 2 || a[1] != 0, n < 3 || a[2] != 0);
                if let Some(v) = self.vehicle(who) {
                    v.firing_state = state;
                    v.firing = enabled;
                }
                ok(0)
            }
            // (heli-set-base-pitch heli degrees).
            "heli-set-base-pitch" => {
                let degrees = f(1, 0.);
                if let Some(v) = self.vehicle(who) {
                    v.base_pitch = degrees.to_radians();
                }
                ok(0)
            }
            // (heli-set-mg-stats heli [burst 20] [spread 0] [a 1] [b 5]).
            "heli-set-mg-stats" => {
                let burst = if n > 1 { a[1] } else { 20 };
                let stats = (burst, f(2, 0.), f(3, 1.), f(4, 5.));
                if let Some(v) = self.vehicle(who) {
                    v.mg_stats = stats;
                }
                ok(0)
            }
            _ => Ok(None),
        }
    }
}
