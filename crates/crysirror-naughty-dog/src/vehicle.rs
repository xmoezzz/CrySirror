// SPDX-License-Identifier: MPL-2.0
//! Tanks and helicopters (`DcTank*`, PPU 0x83B000; `DcHeli*`, 0x808000).
//! Tank natives post command events to the tank process (`sub_83AED4`);
//! the helicopter natives write the heli process fields directly
//! (`sub_808BC0` finds it). Vehicles are created on first use for a live
//! object and moved by the host each frame at fixed speeds (not native:
//! the rail vehicles follow their own splines and dynamics).
use glam::Vec3;
use std::collections::HashMap;

/// What a gun aims at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Point([f32; 3]),
    Object(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Order {
    /// Move to a point (`wait-tank-move-to` position speed accel,
    /// `heli-move-to`).
    MoveTo { target: [f32; 3], speed: f32 },
    /// Follow a spline from its start at `speed` (`heli-move-to-spline`).
    Spline { spline: u32, distance: f32, speed: f32 },
    /// Keep between `min` and `max` metres of an object
    /// (`tank-move-tracking-object`).
    Track { object: u32, min: f32, max: f32, speed: f32 },
    /// Fire at an object for `seconds` (`wait-tank-fire-at-object`).
    Fire { target: Target, seconds: f32 },
    /// Turn the turret to a target, or forward (`tank-aim-…`, events
    /// 0x9F55DAFF position, 0x1583C2B2 object, 0x1348E62B forward; the wait
    /// forms end on event 0x6E9B6EF2).
    Aim(Option<Target>),
    /// Aim the cannon, fire once and pause (`tank-fire-at-position`,
    /// event 0x62B2F34F).
    Cannon { target: Target, pause: f32, fired: bool },
    /// Fire a missile after `delay` and wait for it (`wait-heli-fire-
    /// missile`, `sub_803D10`).
    Missile { target: Target, delay: f32, fired: bool },
}

/// Turret turn rate (rad/s) and the alignment that counts as aimed; shell
/// and missile speeds (m/s). Not native values.
pub const TURRET_RATE: f32 = 1.05;
pub const AIMED: f32 = 0.035;
pub const SHELL_SPEED: f32 = 250.;
pub const MISSILE_SPEED: f32 = 60.;

/// A shell or missile in flight toward its impact point.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// Seconds until impact.
    pub eta: f32,
    pub missile: bool,
}

/// Default speeds (m/s) when a native leaves them out.
pub const TANK_SPEED: f32 = 5.;
pub const HELI_SPEED: f32 = 15.;

#[derive(Clone, Debug, PartialEq)]
pub struct Vehicle {
    pub position: [f32; 3],
    pub yaw: f32,
    /// Tank hit points (+0x1040) and their maximum (+0x1044);
    /// `tank-set-health` clamps to the maximum, 0 destroys the tank.
    pub health: u32,
    pub max_health: u32,
    /// The 30-calibre gun's target (`tank-30cal-fire-at-…`, cleared by
    /// `tank-30cal-cease-fire`).
    pub gun: Option<Target>,
    /// Heli aim point and its radius (+0x240; +0x244/+0x248 the object
    /// handle, cleared), or the object (`heli-set-target-object`).
    pub aim: Option<(Target, f32)>,
    /// Heli firing enabled (+0x2CC) and its state (+0x2EC).
    pub firing: bool,
    pub firing_state: bool,
    /// Heli machine-gun stats: burst (+0x2E4, default 20), spread
    /// (+0x2D8, 0), +0x2D4 (default 1.0), +0x2DC (default 5.0).
    pub mg_stats: (u32, f32, f32, f32),
    /// Heli base pitch, radians (+0x31C).
    pub base_pitch: f32,
    pub order: Option<Order>,
    pub destroyed: bool,
    /// The turret's target (None: forward) and its world yaw.
    pub turret: Option<Target>,
    pub turret_yaw: f32,
    /// Heli gun animation (+1133) and hover (+1132) flags.
    pub gun_animated: bool,
    pub hover: bool,
    pub projectiles: Vec<Projectile>,
    /// Shots fired this step (muzzle points) and impacts (the host spawns
    /// their effects and damage).
    pub muzzles: Vec<[f32; 3]>,
    pub impacts: Vec<([f32; 3], bool)>,
}

impl Vehicle {
    pub fn new(position: [f32; 3], rotation: [f32; 4]) -> Self {
        let f = glam::Quat::from_array(rotation).normalize() * Vec3::Z;
        Self {
            position,
            yaw: f.x.atan2(f.z),
            health: 100,
            max_health: 100,
            gun: None,
            aim: None,
            firing: true,
            firing_state: true,
            mg_stats: (20, 0., 1., 5.),
            base_pitch: 0.,
            order: None,
            destroyed: false,
            turret: None,
            turret_yaw: f.x.atan2(f.z),
            gun_animated: true,
            hover: true,
            projectiles: Vec::new(),
            muzzles: Vec::new(),
            impacts: Vec::new(),
        }
    }

    fn target_point(&self, target: Target, locate: &dyn Fn(u32) -> Option<[f32; 3]>) -> Option<Vec3> {
        match target {
            Target::Point(p) => Some(Vec3::from_array(p)),
            Target::Object(o) => locate(o).map(Vec3::from_array),
        }
    }

    /// Turn the turret toward its target by `dt`; true when aligned.
    fn turn_turret(&mut self, dt: f32, locate: &dyn Fn(u32) -> Option<[f32; 3]>) -> bool {
        let want = match self.turret.and_then(|t| self.target_point(t, locate)) {
            Some(p) => {
                let d = p - Vec3::from_array(self.position);
                if d.x * d.x + d.z * d.z < 1e-6 { self.turret_yaw } else { d.x.atan2(d.z) }
            }
            None => self.yaw,
        };
        let diff = (want - self.turret_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let step = TURRET_RATE * dt;
        self.turret_yaw += diff.clamp(-step, step);
        diff.abs() <= AIMED + step
    }

    /// Launch a projectile from 2 m up toward `to`.
    fn launch(&mut self, to: Vec3, missile: bool) {
        let from = Vec3::from_array(self.position) + Vec3::Y * 2.;
        let speed = if missile { MISSILE_SPEED } else { SHELL_SPEED };
        self.muzzles.push(from.to_array());
        self.projectiles.push(Projectile { from: from.to_array(), to: to.to_array(), eta: from.distance(to) / speed, missile });
    }

    /// Fire the cannon now at the turret's target, else 60 m ahead
    /// (`tank-fire`, event 0xD310256D).
    pub fn fire_cannon(&mut self, locate: &dyn Fn(u32) -> Option<[f32; 3]>) {
        let ahead = Vec3::from_array(self.position) + Vec3::new(self.turret_yaw.sin(), 0., self.turret_yaw.cos()) * 60.;
        let to = self.turret.and_then(|t| self.target_point(t, locate)).unwrap_or(ahead);
        self.launch(to, false);
    }

    /// Fire a heli missile at once (`heli-fire-missile`, no wait).
    pub fn fire_missile(&mut self, target: Target, locate: &dyn Fn(u32) -> Option<[f32; 3]>) {
        if let Some(p) = self.target_point(target, locate) {
            self.launch(p, true);
        }
    }

    /// `sub_832808`: clamp to the maximum; true when the tank is
    /// destroyed.
    pub fn set_health(&mut self, value: u32) -> bool {
        self.health = value.min(self.max_health);
        self.destroyed = self.health == 0;
        self.destroyed
    }

    pub fn rotation(&self) -> [f32; 4] {
        glam::Quat::from_rotation_y(self.yaw).to_array()
    }

    fn approach(&mut self, target: Vec3, step: f32) -> bool {
        let p = Vec3::from_array(self.position);
        let d = target - p;
        let flat = Vec3::new(d.x, 0., d.z);
        if flat.length_squared() > 1e-6 {
            self.yaw = flat.x.atan2(flat.z);
        }
        if d.length() <= step.max(1e-3) {
            self.position = target.to_array();
            return true;
        }
        self.position = (p + d.normalize() * step).to_array();
        false
    }

    /// Step the order by `dt` seconds with the world's object and spline
    /// lookups; true when it completed.
    pub fn step(&mut self, dt: f32, locate: &dyn Fn(u32) -> Option<[f32; 3]>, spline: &dyn Fn(u32, f32) -> Option<([f32; 3], f32)>) -> bool {
        let dt = dt.clamp(0., 0.25);
        for p in &mut self.projectiles {
            p.eta -= dt;
        }
        let (hit, flying): (Vec<_>, Vec<_>) = std::mem::take(&mut self.projectiles).into_iter().partition(|p| p.eta <= 0.);
        self.projectiles = flying;
        self.impacts.extend(hit.into_iter().map(|p| (p.to, p.missile)));
        match self.order {
            Some(Order::Aim(t)) => self.turret = t,
            Some(Order::Cannon { target, .. }) => self.turret = Some(target),
            _ => {}
        }
        let aimed = self.turn_turret(dt, locate);
        let Some(order) = self.order.clone() else { return false };
        let done = match order {
            Order::MoveTo { target, speed } => self.approach(Vec3::from_array(target), speed * dt),
            Order::Spline { spline: s, distance, speed } => {
                let distance = distance + speed * dt;
                match spline(s, distance) {
                    Some((p, length)) => {
                        self.approach(Vec3::from_array(p), f32::INFINITY);
                        if distance >= length {
                            true
                        } else {
                            self.order = Some(Order::Spline { spline: s, distance, speed });
                            false
                        }
                    }
                    None => true,
                }
            }
            Order::Track { object, min, max, speed } => {
                if let Some(t) = locate(object) {
                    let (p, t) = (Vec3::from_array(self.position), Vec3::from_array(t));
                    let distance = p.distance(t);
                    if distance > max {
                        self.approach(t, speed * dt);
                    } else if distance < min && distance > 1e-3 {
                        let away = p + (p - t).normalize() * speed * dt;
                        self.position = away.to_array();
                    }
                }
                false
            }
            Order::Aim(_) => aimed,
            Order::Cannon { target, pause, fired } => {
                if !fired {
                    if aimed {
                        if let Some(p) = self.target_point(target, locate) {
                            self.launch(p, false);
                        }
                        self.order = Some(Order::Cannon { target, pause, fired: true });
                    }
                    false
                } else if pause - dt <= 0. {
                    true
                } else {
                    self.order = Some(Order::Cannon { target, pause: pause - dt, fired: true });
                    false
                }
            }
            Order::Missile { target, delay, fired } => {
                if !fired {
                    if delay - dt <= 0. {
                        self.fire_missile(target, locate);
                        self.order = Some(Order::Missile { target, delay: 0., fired: true });
                    } else {
                        self.order = Some(Order::Missile { target, delay: delay - dt, fired: false });
                    }
                    false
                } else {
                    !self.projectiles.iter().any(|p| p.missile)
                }
            }
            Order::Fire { target, seconds } => {
                self.gun = Some(target);
                let left = seconds - dt;
                if left <= 0. {
                    true
                } else {
                    self.order = Some(Order::Fire { target, seconds: left });
                    false
                }
            }
        };
        if done {
            self.order = None;
        }
        done
    }
}

/// Vehicles by object name.
#[derive(Clone, Debug, Default)]
pub struct Vehicles {
    pub vehicles: HashMap<u32, Vehicle>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_clamps_and_destroys() {
        let mut v = Vehicle::new([0.; 3], [0., 0., 0., 1.]);
        v.max_health = 50;
        assert!(!v.set_health(80));
        assert_eq!(v.health, 50);
        assert!(v.set_health(0));
    }

    #[test]
    fn move_and_fire_orders_complete() {
        let mut v = Vehicle::new([0.; 3], [0., 0., 0., 1.]);
        v.order = Some(Order::MoveTo { target: [0., 0., 10.], speed: 5. });
        let none = |_: u32| None;
        let no_spline = |_: u32, _: f32| None;
        let steps = (0..100).take_while(|_| !v.step(0.1, &none, &no_spline)).count();
        assert_eq!(steps, 19);
        v.order = Some(Order::Fire { target: Target::Object(7), seconds: 0.25 });
        assert!(!v.step(0.1, &none, &no_spline));
        assert!(!v.step(0.1, &none, &no_spline));
        assert!(v.step(0.1, &none, &no_spline));
        assert_eq!(v.gun, Some(Target::Object(7)));
    }

    #[test]
    fn cannon_aims_fires_and_lands() {
        let mut v = Vehicle::new([0.; 3], [0., 0., 0., 1.]);
        let none = |_: u32| None;
        let no_spline = |_: u32, _: f32| None;
        // A target 90° to the right: 1.5 s of turning at 1.05 rad/s.
        v.order = Some(Order::Cannon { target: Target::Point([100., 0., 0.]), pause: 0.5, fired: false });
        let mut steps = 0;
        while !v.step(0.1, &none, &no_spline) {
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(v.muzzles.len(), 1);
        assert!((14..=22).contains(&steps), "{steps}");
        assert!(v.impacts.len() == 1 || v.projectiles.len() == 1);
    }
}
