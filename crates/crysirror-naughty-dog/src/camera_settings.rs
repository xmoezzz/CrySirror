// SPDX-License-Identifier: MPL-2.0
//! Follow and aim camera settings from `camera.bin`.
//!
//! - `*camera-follow-settings*` (`camera-follow-settings`): +0x18 the
//!   follow distance (4 m) and +0x20 the horizontal field of view (90°).
//! - `*camera-zoom-right-pistol-offsets*` and `-longgun-` (type
//!   `0xd0759c7b`): +0x00 the camera offset from the player's aim pivot as
//!   (side, forward, up) in the view frame (pistol −0.6, −1.2, 0.3: right
//!   of and behind the shoulder; long guns −0.8, −0.6, 0.25), +0x4c the
//!   horizontal field of view while aiming (85°).
//!
//! The field meanings are inferred from their values (the follow distance
//! matches the player's 4 m camera); the camera code reading them is not
//! decoded (SATD-nd-retail-runtime).
use crate::dc::{Dc, sid};
use anyhow::{Context, Result, ensure};
use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimOffsets {
    /// (side, forward, up); +side is the player's left.
    pub offset: Vec3,
    pub fov_h: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraSettings {
    pub distance: f32,
    pub fov_h: f32,
    pub pistol: AimOffsets,
    pub long_gun: AimOffsets,
}

impl Default for CameraSettings {
    /// The retail values, for a missing module.
    fn default() -> Self {
        Self { distance: 4., fov_h: 90., pistol: AimOffsets { offset: Vec3::new(-0.6, -1.2, 0.3), fov_h: 85. }, long_gun: AimOffsets { offset: Vec3::new(-0.8, -0.6, 0.25), fov_h: 85. } }
    }
}

impl CameraSettings {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut dc = Dc::default();
        dc.load(sid("camera"), bytes)?;
        let m = dc.module(sid("camera")).context("camera module")?;
        let entry = |name: &str| m.entries.iter().find(|(n, _)| *n == sid(name)).map(|(_, v)| *v).with_context(|| format!("camera.bin {name}"));
        let f = |at: u32| dc.mem.f32(at);
        let follow = entry("*camera-follow-settings*")?;
        let aim = |name: &str| -> Result<AimOffsets> {
            let v = entry(name)?;
            Ok(AimOffsets { offset: Vec3::new(f(v)?, f(v + 4)?, f(v + 8)?), fov_h: f(v + 0x4c)? })
        };
        let s = Self { distance: f(follow + 0x18)?, fov_h: f(follow + 0x20)?, pistol: aim("*camera-zoom-right-pistol-offsets*")?, long_gun: aim("*camera-zoom-right-longgun-offsets*")? };
        let fov_ok = |h: f32| (20. ..=150.).contains(&h);
        ensure!(s.distance > 0.5 && s.distance < 20. && fov_ok(s.fov_h) && fov_ok(s.pistol.fov_h) && fov_ok(s.long_gun.fov_h), "camera settings out of range: {s:?}");
        Ok(s)
    }
}

/// The vertical field of view (degrees) of a horizontal one at `aspect`
/// (width / height).
pub fn vertical_fov(horizontal: f32, aspect: f32) -> f32 {
    (2. * ((horizontal.to_radians() * 0.5).tan() / aspect).atan()).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fov_conversion() {
        assert!((vertical_fov(90., 16. / 9.) - 58.715).abs() < 0.01);
    }
}
