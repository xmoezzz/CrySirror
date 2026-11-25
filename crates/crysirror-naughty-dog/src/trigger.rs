// SPDX-License-Identifier: MPL-2.0
//! Trigger regions of `REGION_TREE` items and the point test of
//! `player-in-region?` / `object-in-region?` (`sub_F7D94` → `sub_115EE0` →
//! `sub_114D60`).
//!
//! The payload is `{grid, region pointers…}`. The grid (`sub_B5A00`,
//! `sub_B5CE8`) holds a min and extent vec4, `nx, ny, nz` and `nx·ny·nz +
//! 1` cells of `{pairs, count}` with pairs `{category StringId, {regions,
//! count}}`; here it only supplies each region's categories (`player`,
//! `camera`, `npc`, … — the region's `apply-to` entry). A region holds its
//! name (+0), `StringId` (+4), flags (+0xa; bit 0 makes it always false),
//! the parent spawner (+0x10) and a list of shape nodes (+0x18, `next` at
//! +0, `u16` type at +0xc, `u16` count at +0xe). A point is inside when
//! any node holds it, in the parent's live frame (a region whose parent is
//! not spawned holds nothing) or in level space without a parent:
//!
//! - type 1: `count` 112-byte faces from +0x10, a row-major matrix taking
//!   the shape frame to the face frame (rows +0…+0x20, translation +0x30),
//!   the face's +z its outward normal: inside when `max(n·p + d) < 0`;
//! - type 0: a sphere (+0x10 centre, radius in w): `|p − c| ≤ r`;
//! - type 3: a cylinder, base +0x10, quaternion +0x20, radius +0x30,
//!   height +0x34 along the rotated +y;
//! - type 2: quads for a crossing test, never a containing volume.
//!
//! The native search threshold is 0. The level origin (level +0x20) is
//! taken as zero (SATD-nd-retail-runtime).
//!
//! Tags (+0x20, count `u16` +0x1e) are 8-byte `{key, cell}` pairs sorted by
//! key, the cell holding a script lambda (`sub_110AD0`). The per-object
//! region update (`sub_115828`) runs key 0x1157c38c when an object enters
//! a region of its category and key 0x5b4bfb24 when it leaves (also when
//! it is removed, `sub_115500`), and posts events 0x2691c757 / 0x0f17c6a1
//! to the region's process (+0xc) when it has one.
use crate::dc::sid;
use crate::mesh::Loaded;
use anyhow::{Result, ensure};
use glam::{Quat, Vec3};

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Face planes `(n, d)`: inside when every `n·p + d < 0`.
    Planes(Vec<(Vec3, f32)>),
    Sphere(Vec3, f32),
    Cylinder { base: Vec3, rotation: Quat, radius: f32, height: f32 },
    Quads,
}

impl Shape {
    pub fn contains(&self, p: Vec3) -> bool {
        match self {
            Self::Planes(planes) => !planes.is_empty() && planes.iter().map(|(n, d)| n.dot(p) + d).fold(f32::MIN, f32::max) < 0.,
            Self::Sphere(c, r) => (p - *c).length() <= *r,
            Self::Cylinder { base, rotation, radius, height } => {
                let d = p - *base;
                let (x, y, z) = (*rotation * Vec3::X, *rotation * Vec3::Y, *rotation * Vec3::Z);
                let (a, b, c) = (d.dot(z), d.dot(y), d.dot(x));
                (a * a + c * c).sqrt() <= *radius && b >= 0. && b <= *height
            }
            Self::Quads => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub name: String,
    pub id: u32,
    pub flags: u16,
    /// Parent spawner name `StringId`.
    pub parent: Option<u32>,
    pub categories: Vec<u32>,
    pub shapes: Vec<Shape>,
    /// The `render-settings` entry: a `render-settings.bin` record name.
    pub render_settings: Option<u32>,
    /// The `part-module` entry: the particle DC module the area uses.
    pub part_module: Option<u32>,
    /// Package offsets of the enter and exit lambdas.
    pub enter: Option<usize>,
    pub exit: Option<usize>,
}

pub const TAG_ENTER: u32 = 0x1157_c38c;
pub const TAG_EXIT: u32 = 0x5b4b_fb24;

impl Region {
    /// Whether the point (in the region's frame) is inside.
    pub fn contains_local(&self, p: Vec3) -> bool {
        self.flags & 1 == 0 && self.shapes.iter().any(|s| s.contains(p))
    }
}

fn text(p: &Loaded, at: usize) -> String {
    let b = p.data.get(at..).unwrap_or_default();
    let e = b.iter().take(256).position(|c| *c == 0).unwrap_or(0);
    String::from_utf8_lossy(&b[..e]).into_owned()
}

fn vec3(p: &Loaded, at: usize) -> Result<Vec3> {
    Ok(Vec3::new(p.f32(at)?, p.f32(at + 4)?, p.f32(at + 8)?))
}

fn shape(p: &Loaded, at: usize) -> Result<Shape> {
    let (kind, count) = (p.u16(at + 0xc)?, usize::from(p.u16(at + 0xe)?));
    Ok(match kind {
        0 => Shape::Sphere(vec3(p, at + 0x10)?, p.f32(at + 0x1c)?),
        1 => {
            ensure!(count <= 64, "ND region face count");
            let planes = (0..count)
                .map(|k| {
                    let f = at + 0x10 + 0x70 * k;
                    let rows = [vec3(p, f)?, vec3(p, f + 0x10)?, vec3(p, f + 0x20)?];
                    let t = vec3(p, f + 0x30)?;
                    // Face-local → shape: s_i = row_i · (l − t).
                    let origin = Vec3::new(rows[0].dot(-t), rows[1].dot(-t), rows[2].dot(-t));
                    let normal = Vec3::new(rows[0].z, rows[1].z, rows[2].z);
                    Ok((normal, -normal.dot(origin)))
                })
                .collect::<Result<Vec<_>>>()?;
            Shape::Planes(planes)
        }
        2 => Shape::Quads,
        3 => {
            let q = [p.f32(at + 0x20)?, p.f32(at + 0x24)?, p.f32(at + 0x28)?, p.f32(at + 0x2c)?];
            Shape::Cylinder { base: vec3(p, at + 0x10)?, rotation: Quat::from_array(q).normalize(), radius: p.f32(at + 0x30)?, height: p.f32(at + 0x34)? }
        }
        other => anyhow::bail!("ND region shape type {other}"),
    })
}

/// Every trigger region of a `REGION_TREE` payload.
pub fn regions(p: &Loaded, payload: usize) -> Result<Vec<Region>> {
    let grid = p.ptr(payload)?;
    let mut out = Vec::new();
    let mut addresses = Vec::new();
    for i in 1..65536 {
        let Ok(a) = p.ptr(payload + 4 * i) else { break };
        if a == grid {
            break;
        }
        let parent = p.ptr(a + 0x10).ok().and_then(|s| p.ptr(s + 0x20).ok()).map(|n| sid(&text(p, n)));
        let mut shapes = Vec::new();
        let mut node = p.ptr(a + 0x18).ok();
        while let Some(n) = node {
            ensure!(shapes.len() < 256, "ND region shape list");
            shapes.push(shape(p, n)?);
            node = p.ptr(n).ok().filter(|next| *next != n);
        }
        let tags = usize::from(p.u16(a + 0x1e)?);
        ensure!(tags <= 256, "ND region tag count");
        let tag = |key: u32| -> Option<usize> {
            let at = p.ptr(a + 0x20).ok()?;
            (0..tags).find(|k| p.u32(at + 8 * k).ok() == Some(key)).and_then(|k| p.ptr(p.ptr(at + 8 * k + 4).ok()?).ok())
        };
        // Entries `{key, type, value}` (+0x24, count `u16` +0x1c); type 4
        // holds a `StringId`.
        let entries = usize::from(p.u16(a + 0x1c)?);
        ensure!(entries <= 256, "ND region entry count");
        let entry = |key: &str| p.ptr(a + 0x24).ok().and_then(|at| (0..entries).find(|k| p.u32(at + 12 * k).ok() == Some(sid(key)) && p.u32(at + 12 * k + 4).ok() == Some(4)).and_then(|k| p.u32(at + 12 * k + 8).ok()));
        out.push(Region {
            name: p.ptr(a).map(|n| text(p, n)).unwrap_or_default(),
            id: p.u32(a + 4)?,
            flags: p.u16(a + 0xa)?,
            parent,
            categories: Vec::new(),
            shapes,
            render_settings: entry("render-settings"),
            part_module: entry("part-module"),
            enter: tag(TAG_ENTER),
            exit: tag(TAG_EXIT),
        });
        addresses.push(a);
    }
    // Categories from the grid's buckets.
    let dims = [p.u32(grid + 0x20)?, p.u32(grid + 0x24)?, p.u32(grid + 0x28)?];
    let cells = dims.iter().map(|d| *d as usize).product::<usize>() + 1;
    ensure!(cells <= 1 << 20, "ND region grid size");
    for c in 0..cells {
        let Ok(cell) = p.ptr(grid + 0x2c + 4 * c) else { continue };
        let n = p.u32(cell + 4)? as usize;
        ensure!(n <= 4096, "ND region grid bucket count");
        let Ok(pairs) = p.ptr(cell) else { continue };
        for k in 0..n {
            let key = p.u32(pairs + 8 * k)?;
            let list = p.ptr(pairs + 8 * k + 4)?;
            let m = p.u32(list + 4)? as usize;
            let Ok(array) = p.ptr(list) else { continue };
            for j in 0..m.min(4096) {
                let Ok(r) = p.ptr(array + 4 * j) else { continue };
                if let Some(i) = addresses.iter().position(|a| *a == r) {
                    if !out[i].categories.contains(&key) {
                        out[i].categories.push(key);
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_box_planes_and_cylinder() {
        let planes = Shape::Planes(vec![(Vec3::X, -1.), (-Vec3::X, -1.), (Vec3::Y, -1.), (-Vec3::Y, -1.), (Vec3::Z, -1.), (-Vec3::Z, -1.)]);
        assert!(planes.contains(Vec3::new(0.5, -0.5, 0.9)));
        assert!(!planes.contains(Vec3::new(1.5, 0., 0.)));
        let c = Shape::Cylinder { base: Vec3::ZERO, rotation: Quat::IDENTITY, radius: 1., height: 2. };
        assert!(c.contains(Vec3::new(0.5, 1.5, 0.)));
        assert!(!c.contains(Vec3::new(0., -0.1, 0.)));
        assert!(!c.contains(Vec3::new(1.2, 1., 0.)));
    }
}
