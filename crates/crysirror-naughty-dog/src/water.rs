// SPDX-License-Identifier: MPL-2.0
//! Water bodies of level packages (`WATER_DATA`): +0x4 count, +0x8 the
//! records (0x70 bytes each): +0x1c surface plane normal, +0x28 plane
//! constant (`n·p = d`), +0x2c bounding box minimum and +0x38 maximum.
//! `dig-02-ship-water` holds six bodies, the first level at y = −103.5.
use crate::mesh::Loaded;
use anyhow::{Result, ensure};
use glam::Vec3;

const RECORD: usize = 0x70;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    pub normal: Vec3,
    pub d: f32,
    pub min: Vec3,
    pub max: Vec3,
}

impl Water {
    /// The surface height above `(x, z)` when inside the box's footprint.
    pub fn surface(&self, x: f32, z: f32) -> Option<f32> {
        if x < self.min.x || x > self.max.x || z < self.min.z || z > self.max.z || self.normal.y.abs() < 1e-3 {
            return None;
        }
        Some((self.d - self.normal.x * x - self.normal.z * z) / self.normal.y)
    }
}

fn vec3(p: &Loaded, at: usize) -> Result<Vec3> {
    Ok(Vec3::new(p.f32(at)?, p.f32(at + 4)?, p.f32(at + 8)?))
}

pub fn read(p: &Loaded, payload: usize) -> Result<Vec<Water>> {
    let count = p.u32(payload + 4)? as usize;
    ensure!(count <= 1024, "water body count");
    let base = p.ptr(payload + 8)?;
    (0..count)
        .map(|i| {
            let r = base + RECORD * i;
            let (a, b) = (vec3(p, r + 0x2c)?, vec3(p, r + 0x38)?);
            Ok(Water { normal: vec3(p, r + 0x1c)?, d: p.f32(r + 0x28)?, min: a.min(b), max: a.max(b) })
        })
        .collect()
}

pub fn bodies(p: &Loaded, directory: &crate::package::Directory) -> Result<Vec<Water>> {
    let mut out = Vec::new();
    for item in directory.items.iter().filter(|i| i.kind == "WATER_DATA") {
        out.extend(read(p, item.payload_offset as usize)?);
    }
    Ok(out)
}

/// The water surface above a point, if any body holds it.
pub fn surface_at(bodies: &[Water], point: Vec3) -> Option<f32> {
    bodies.iter().filter_map(|w| w.surface(point.x, point.z)).filter(|h| *h > point.y - 0.5).min_by(|a, b| a.total_cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilted_surface_height() {
        let w = Water { normal: Vec3::Y, d: -103.5, min: Vec3::new(-3., -104., -226.), max: Vec3::new(78., -103., -170.) };
        assert_eq!(w.surface(0., -200.), Some(-103.5));
        assert_eq!(w.surface(100., -200.), None);
        assert_eq!(surface_at(&[w], Vec3::new(0., -106., -200.)), Some(-103.5));
        assert_eq!(surface_at(&[w], Vec3::new(0., -90., -200.)), None);
    }
}
