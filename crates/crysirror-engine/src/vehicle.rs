// SPDX-License-Identifier: MPL-2.0
//! Arcade vehicle controller (ADR 0043), the NFS-style model: no tyre
//! physics simulation, but predictable grip, drift and suspension.
//!
//! * Four wheel rays find the ground; the body rides at `ride_height` above
//!   their mean and pitches/rolls with the contact plane.
//! * A bicycle model turns the body (`yaw rate = v / wheelbase · tan δ`).
//! * The world velocity is kept across the turn and only its lateral part
//!   is damped by grip, so a car that turns faster than its tyres grip
//!   slides: drift falls out of the model. The handbrake lowers grip.
//! * The body moves through the character-controller sweep, so walls stop
//!   and deflect it; the velocity keeps only what actually moved.
use crate::physics::Physics;
use anyhow::{ensure, Result};
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VehicleSpec {
    /// Axle distance and wheel track (world units).
    pub wheelbase: f32,
    pub track: f32,
    /// Body origin height above the ground contact.
    pub ride_height: f32,
    /// How far below the body the wheels reach for ground.
    pub suspension: f32,
    /// Top speed (units/s) and acceleration/braking (units/s²).
    pub max_speed: f32,
    pub acceleration: f32,
    pub braking: f32,
    /// Speed lost per second per unit speed (rolling/air resistance).
    pub drag: f32,
    /// Maximum front-wheel angle (degrees) at standstill; it shrinks with
    /// speed (`steer_falloff`) like arcade racers do.
    pub steer_degrees: f32,
    pub steer_falloff: f32,
    /// Lateral damping (1/s) with and without the handbrake.
    pub grip: f32,
    pub handbrake_grip: f32,
    pub gravity: f32,
    /// Character-sweep step and slope limits for the body.
    pub step: f32,
    pub max_slope: f32,
}
impl Default for VehicleSpec {
    fn default() -> Self {
        Self {
            wheelbase: 2.6,
            track: 1.6,
            ride_height: 0.6,
            suspension: 1.2,
            max_speed: 60.,
            acceleration: 12.,
            braking: 25.,
            drag: 0.08,
            steer_degrees: 32.,
            steer_falloff: 0.02,
            grip: 8.,
            handbrake_grip: 0.8,
            gravity: 9.81,
            step: 0.3,
            max_slope: 50.,
        }
    }
}
impl VehicleSpec {
    pub fn validate(&self) -> Result<()> {
        let v = [
            self.wheelbase, self.track, self.ride_height, self.suspension, self.max_speed, self.acceleration,
            self.braking, self.drag, self.steer_degrees, self.steer_falloff, self.grip, self.handbrake_grip,
            self.gravity, self.step, self.max_slope,
        ];
        ensure!(v.iter().all(|x| x.is_finite() && *x >= 0.), "vehicle values must be finite and non-negative");
        ensure!(self.wheelbase > 0.05 && self.track > 0.05 && self.max_speed > 0. && self.steer_degrees < 80., "vehicle dimensions");
        Ok(())
    }
}

/// Running state; `yaw` is degrees about +Y (0 faces +Z, positive turns
/// toward +X).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VehicleState {
    pub velocity: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub grounded: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DriveInput {
    /// -1 (reverse/brake) .. 1 (full throttle).
    pub throttle: f32,
    /// -1 .. 1; positive turns toward +X of the car's heading.
    pub steer: f32,
    /// 0 .. 1 service brake.
    pub brake: f32,
    pub handbrake: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveResult {
    pub position: Vec3,
    pub state: VehicleState,
    /// Speed along the heading and sideways (positive toward +X of it).
    pub forward_speed: f32,
    pub lateral_speed: f32,
    pub wheels_on_ground: u32,
    /// The sweep stopped part of the motion (wall contact).
    pub blocked: bool,
}

fn heading(yaw_deg: f32) -> (Vec3, Vec3) {
    let y = yaw_deg.to_radians();
    let fwd = Vec3::new(y.sin(), 0., y.cos());
    (fwd, Vec3::new(fwd.z, 0., -fwd.x))
}

/// Advances one vehicle by `dt` seconds. `key` is its character collider.
pub fn step(
    physics: &Physics,
    key: (u32, u32),
    spec: &VehicleSpec,
    mut s: VehicleState,
    position: Vec3,
    input: DriveInput,
    dt: f32,
) -> Result<DriveResult> {
    ensure!(dt.is_finite() && dt > 0. && dt <= 0.25, "vehicle step must be in (0, 0.25] s");
    let clamp = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0. };
    let (throttle, steer, brake) = (clamp(input.throttle, -1., 1.), clamp(input.steer, -1., 1.), clamp(input.brake, 0., 1.));
    let mut v = Vec3::from_array(s.velocity);
    // Ground under the four wheels, from just above the body.
    let (fwd0, right0) = heading(s.yaw);
    let half = (spec.wheelbase * 0.5, spec.track * 0.5);
    let reach = spec.ride_height + spec.suspension;
    let lift = spec.suspension * 0.5;
    let wheels = [(1., 1.), (1., -1.), (-1., 1.), (-1., -1.)].map(|(f, r): (f32, f32)| {
        let at = position + fwd0 * (f * half.0) + right0 * (r * half.1);
        physics.raycast(at + Vec3::Y * lift, Vec3::NEG_Y, reach + lift, Some(key)).map(|h| h.point.y)
    });
    let contacts: Vec<f32> = wheels.iter().flatten().copied().collect();
    let grounded = contacts.len() >= 2;
    if grounded {
        let vf = v.dot(fwd0);
        let mut a = throttle * spec.acceleration * (1. - (vf / spec.max_speed).clamp(-1., 1.).abs());
        if throttle < 0. && vf > 0.5 {
            a = throttle * spec.braking; // reverse input brakes first
        }
        a -= vf.signum() * brake * spec.braking + spec.drag * vf;
        let vf_new = if (vf + a * dt).signum() != vf.signum() && brake > 0. && throttle >= 0. { 0. } else { vf + a * dt };
        v += fwd0 * (vf_new - vf);
        let speed = v.dot(fwd0);
        let delta = (steer * spec.steer_degrees / (1. + spec.steer_falloff * speed.abs())).to_radians();
        s.yaw += (speed / spec.wheelbase * delta.tan()).to_degrees() * dt;
        // Keep the world velocity; tyres only damp its sideways part.
        let (fwd, right) = heading(s.yaw);
        let grip = if input.handbrake { spec.handbrake_grip } else { spec.grip };
        let (vf, vl) = (v.dot(fwd), v.dot(right) * (-grip * dt).exp());
        v = fwd * vf + right * vl + Vec3::Y * v.y.min(0.);
        // Suspension: follow the contact plane.
        let h = |i: usize| wheels[i].unwrap_or_else(|| contacts.iter().sum::<f32>() / contacts.len() as f32);
        let (front, rear) = ((h(0) + h(1)) * 0.5, (h(2) + h(3)) * 0.5);
        let (left, rgt) = ((h(1) + h(3)) * 0.5, (h(0) + h(2)) * 0.5);
        s.pitch = ((front - rear) / spec.wheelbase).atan().to_degrees();
        s.roll = ((rgt - left) / spec.track).atan().to_degrees();
        let ground = (front + rear) * 0.5;
        let target = ground + spec.ride_height;
        v.y = ((target - position.y) / dt).clamp(-spec.max_speed, spec.max_speed);
    } else {
        v.y -= spec.gravity * dt;
        s.pitch *= 0.98;
        s.roll *= 0.98;
    }
    let desired = v * dt;
    let mv = physics.move_character(key, desired, dt, spec.step, spec.max_slope)?;
    let horizontal = |x: Vec3| Vec3::new(x.x, 0., x.z);
    let blocked = horizontal(desired).length() > 1e-4 && horizontal(mv.translation).length() < horizontal(desired).length() * 0.9;
    if blocked {
        // Keep only the velocity that actually happened (walls absorb the rest).
        let actual = horizontal(mv.translation) / dt;
        v = Vec3::new(actual.x, v.y, actual.z);
    }
    if !grounded && mv.grounded {
        v.y = 0.;
    }
    s.velocity = v.to_array();
    s.grounded = grounded;
    let (fwd, right) = heading(s.yaw);
    Ok(DriveResult {
        position: position + mv.translation,
        state: s,
        forward_speed: v.dot(fwd),
        lateral_speed: v.dot(right),
        wheels_on_ground: contacts.len() as u32,
        blocked,
    })
}

/// Entity Euler rotation (degrees, XYZ) for a vehicle state.
pub fn rotation(s: &VehicleState) -> [f32; 3] {
    let q = Quat::from_rotation_y(s.yaw.to_radians())
        * Quat::from_rotation_x((-s.pitch).to_radians())
        * Quat::from_rotation_z(s.roll.to_radians());
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    [x.to_degrees(), y.to_degrees(), z.to_degrees()]
}
