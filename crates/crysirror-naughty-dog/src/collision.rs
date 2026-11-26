// SPDX-License-Identifier: MPL-2.0
//! Collision geometry, surface attributes (PAT) and climb-edge feature
//! databases of packages.
//!
//! - `COLLISION_DATA_FOREGROUND` (art groups, `mesh.collision.NAME`): `{0x1001,
//!   n, entry*[n]}`; an entry (0x30 bytes) holds a locator, the joint index
//!   at +0x20 (−1 none), the collision wrapper at +0x28 and an optional PAT
//!   override at +0x2c (unused in the data). Item login `sub_47928`; bodies
//!   are placed at their joint (`sub_3E76F4`), so shapes and feature
//!   databases are joint-local and the bind pose brings them to the
//!   art-group frame of the render meshes.
//! - Wrapper (`sub_3B7C10`): `{size, hasHeader, (rt) body, (rt) data,
//!   featureDb*, flags}`; the Havok 6.1 packfile follows at +0x20 with a
//!   header (+0x10 without). Its contents are an `hkpRigidBody`.
//! - `COLLISION_DATA_HAVOK_BG` (levels, login `sub_3B6EE0`): version at +0
//!   (≤ 1: an empty stub), `{matIndexBase* (u8 per triangle), PAT u64*,
//!   count}` records at +4, `TrianglesSubpart*` at +8 and their count at
//!   +0xc; vertices are float3 metres in the render meshes' space.
//! - Feature databases (`FEATURE_DB_2` of levels, wrapper +0x10 of props):
//!   `{edges*, count, corners*, count, info*, edge tree*, corner tree*}`;
//!   edges are 64 bytes `{p0, p1, n0 top-face normal, n1 wall normal, 0, 0,
//!   u16 prev, u16 next, u16 flags, u16 index}` (`sub_4F8A18`,
//!   `sub_4F8318`), corners 52 bytes `{p0, p1, n0, n1, u16 flags, u16
//!   index}`. Edges are built offline; the player query keeps an edge whose
//!   top normal is within 45° of up (`dot ≥ 0.707`).
//!
//! PAT low word: bits 0–5 surface type, flag bits 6 PlayerKill, 7 NoStand,
//! 8 SeeThru, 9 ShootThru, 10 PassThru, 11 Damageable, 12 Stairs, 13
//! CamPassThru, 14 Water, 15 NoWallJump, 16 HearThru, 17 Snow, 18
//! CameraBlockNear, 23 NoOcclude, 26 EdgeVault, 27 NoEdgeGrab, 28 NoCover,
//! 29 NoPlyrCover (flag name table EBOOT 0xd6c9e0, `sub_37907C`; debug
//! categories `sub_3EE7C4`).
use crate::mesh::Loaded;
use anyhow::{Context, Result, ensure};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

pub const PAT_NO_STAND: u32 = 1 << 7;
pub const PAT_PASS_THRU: u32 = 1 << 10;
pub const PAT_CAM_PASS_THRU: u32 = 1 << 13;
pub const PAT_WATER: u32 = 1 << 14;
pub const PAT_NO_EDGE_GRAB: u32 = 1 << 27;

/// A collision triangle with its PAT low word.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    pub vertices: [Vec3; 3],
    pub pat: u32,
}

/// A climb edge (or a wall corner) in the frame of its collision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub p0: Vec3,
    pub p1: Vec3,
    /// Edge: top-face normal; corner: wall normal A.
    pub n0: Vec3,
    /// Edge: wall (front) normal; corner: wall normal B.
    pub n1: Vec3,
    pub prev: Option<u16>,
    pub next: Option<u16>,
    pub flags: u16,
}

/// Edge flags (`sub_62EC50` prints them): `EdgeNoSwing` 0x1,
/// `EdgeCanStand` 0x2, `EdgeCanHang` 0x4, `EdgeResistWalk` 0x8,
/// `EdgeCanUseLowCover` 0x10, `EdgeLadder` 0x20, `EdgeNoJumpAim` 0x40,
/// `EdgeForeground` 0x2000, `EdgeLoop` 0x4000, `EdgeDummy` 0x8000.
/// `EdgeNoSwing`: a hang without swinging (ledges against a wall); hang
/// edges without it are bars.
pub const EDGE_NO_SWING: u16 = 0x1;
pub const EDGE_CAN_STAND: u16 = 0x2;
pub const EDGE_CAN_HANG: u16 = 0x4;
pub const EDGE_LADDER: u16 = 0x20;

#[derive(Clone, Debug, Default)]
pub struct FeatureDb {
    pub edges: Vec<Edge>,
    pub corners: Vec<Edge>,
}

impl FeatureDb {
    pub fn transformed(&self, m: Mat4) -> Self {
        let tf = |e: &Edge| Edge { p0: m.transform_point3(e.p0), p1: m.transform_point3(e.p1), n0: m.transform_vector3(e.n0).normalize_or_zero(), n1: m.transform_vector3(e.n1).normalize_or_zero(), ..*e };
        Self { edges: self.edges.iter().map(tf).collect(), corners: self.corners.iter().map(tf).collect() }
    }
}

/// One collision object: its triangles and feature database in the
/// art-group (prop) or world (level) frame.
#[derive(Clone, Debug, Default)]
pub struct Object {
    pub joint: Option<usize>,
    pub triangles: Vec<Triangle>,
    pub features: FeatureDb,
}

const MAX_TRIANGLES: usize = 1 << 22;
const MAX_FEATURES: u32 = 1 << 18;

pub fn feature_db(p: &Loaded, at: usize) -> Result<FeatureDb> {
    let (edges, corners) = (p.u32(at + 4)?, p.u32(at + 12)?);
    ensure!(edges <= MAX_FEATURES && corners <= MAX_FEATURES, "ND feature count");
    let v = |o: usize| -> Result<Vec3> { Ok(Vec3::new(p.f32(o)?, p.f32(o + 4)?, p.f32(o + 8)?)) };
    let link = |x: u16| (x != 0xffff).then_some(x);
    let edges = if edges == 0 {
        Vec::new()
    } else {
        let base = p.ptr(at)?;
        (0..edges as usize)
            .map(|i| {
                let e = base + 64 * i;
                Ok(Edge { p0: v(e)?, p1: v(e + 0xc)?, n0: v(e + 0x18)?, n1: v(e + 0x24)?, prev: link(p.u16(e + 0x38)?), next: link(p.u16(e + 0x3a)?), flags: p.u16(e + 0x3c)? })
            })
            .collect::<Result<_>>()?
    };
    let corners = if corners == 0 {
        Vec::new()
    } else {
        let base = p.ptr(at + 8)?;
        (0..corners as usize)
            .map(|i| {
                let c = base + 52 * i;
                Ok(Edge { p0: v(c)?, p1: v(c + 0xc)?, n0: v(c + 0x18)?, n1: v(c + 0x24)?, prev: None, next: None, flags: p.u16(c + 0x30)? })
            })
            .collect::<Result<_>>()?
    };
    Ok(FeatureDb { edges, corners })
}

/// A Havok 6.1.0 binary packfile (big-endian, 4-byte pointers) inside
/// package data: pointer fix-ups and object classes.
struct Packfile<'a> {
    b: &'a [u8],
    pointers: HashMap<usize, usize>,
    classes: HashMap<usize, String>,
    root: usize,
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(at..at + 4).context("ND Havok read outside package")?.try_into()?))
}

impl<'a> Packfile<'a> {
    fn read(b: &'a [u8], base: usize) -> Result<Self> {
        ensure!(b.get(base..base + 8) == Some(&[0x57, 0xe0, 0xe0, 0x57, 0x10, 0xc0, 0xc0, 0x10][..]), "ND Havok packfile magic");
        let i32_at = |o: usize| be32(b, o).map(|v| v as i32);
        let sections = i32_at(base + 20)?;
        ensure!((1..=16).contains(&sections), "ND Havok section count");
        let (contents_section, contents_offset) = (i32_at(base + 24)? as usize, i32_at(base + 28)? as usize);
        let mut starts = Vec::new();
        let mut headers = Vec::new();
        for i in 0..sections as usize {
            let o = base + 64 + 48 * i;
            let w: Vec<usize> = (0..7).map(|k| be32(b, o + 20 + 4 * k).map(|v| v as usize)).collect::<Result<_>>()?;
            starts.push(base + w[0]);
            headers.push(w);
        }
        let (mut pointers, mut classes) = (HashMap::new(), HashMap::new());
        for (i, w) in headers.iter().enumerate() {
            let d = starts[i];
            let (local, global, virt, exports) = (w[1], w[2], w[3], w[4]);
            ensure!(local <= global && global <= virt && virt <= exports && d + exports <= b.len(), "ND Havok fix-up tables");
            for o in (d + local..d + global).step_by(8) {
                let (src, dst) = (i32_at(o)?, i32_at(o + 4)?);
                if src >= 0 {
                    pointers.insert(d + src as usize, d + dst as usize);
                }
            }
            for o in (d + global..d + virt).step_by(12) {
                let (src, section, dst) = (i32_at(o)?, i32_at(o + 4)?, i32_at(o + 8)?);
                if src >= 0 {
                    pointers.insert(d + src as usize, starts.get(section as usize).context("ND Havok section")? + dst as usize);
                }
            }
            for o in (d + virt..d + exports).step_by(12) {
                let (object, section, name) = (i32_at(o)?, i32_at(o + 4)?, i32_at(o + 8)?);
                if object >= 0 {
                    let at = starts.get(section as usize).context("ND Havok section")? + name as usize;
                    let end = b.get(at..).and_then(|s| s.iter().take(64).position(|c| *c == 0)).context("ND Havok class name")?;
                    classes.insert(d + object as usize, String::from_utf8_lossy(&b[at..at + end]).into_owned());
                }
            }
        }
        let root = starts.get(contents_section).context("ND Havok contents section")? + contents_offset;
        Ok(Self { b, pointers, classes, root })
    }
    fn ptr(&self, at: usize) -> Result<usize> {
        self.pointers.get(&at).copied().context("ND Havok pointer")
    }
    fn u32(&self, at: usize) -> Result<u32> {
        be32(self.b, at)
    }
    fn f32(&self, at: usize) -> Result<f32> {
        self.u32(at).map(f32::from_bits)
    }
    fn vec3(&self, at: usize) -> Result<Vec3> {
        Ok(Vec3::new(self.f32(at)?, self.f32(at + 4)?, self.f32(at + 8)?))
    }
    /// `hkArray {data*, size, capacity}`: data and size (no data when empty).
    fn array(&self, at: usize) -> Result<(usize, usize)> {
        let n = self.u32(at + 4)? as usize;
        ensure!(n <= MAX_TRIANGLES, "ND Havok array size");
        Ok((if n == 0 { 0 } else { self.ptr(at)? }, n))
    }

    /// Triangles of shape `at` under `m`.
    fn shape(&self, at: usize, m: Mat4, out: &mut Vec<Triangle>, depth: usize) -> Result<()> {
        ensure!(depth < 16, "ND Havok shape depth");
        match self.classes.get(&at).map(String::as_str) {
            Some("hkpListShape") => {
                let (children, n) = self.array(at + 0x18)?;
                for i in 0..n {
                    self.shape(self.ptr(children + 16 * i)?, m, out, depth + 1)?;
                }
            }
            Some("hkpMoppBvTreeShape") => self.shape(self.ptr(at + 0x34)?, m, out, depth + 1)?,
            Some("hkpConvexVerticesShape") => {
                let (blocks, n) = self.array(at + 0x40)?;
                let count = self.u32(at + 0x4c)? as usize;
                let mut vertices = Vec::with_capacity(4 * n);
                for k in 0..n {
                    for j in 0..4 {
                        let b = blocks + 48 * k + 4 * j;
                        vertices.push(Vec3::new(self.f32(b)?, self.f32(b + 16)?, self.f32(b + 32)?));
                    }
                }
                vertices.truncate(count);
                let (planes, np) = self.array(at + 0x50)?;
                let planes = (0..np).map(|k| Ok((self.vec3(planes + 16 * k)?, self.f32(planes + 16 * k + 12)?))).collect::<Result<Vec<_>>>()?;
                let pat = self.u32(at + 8)?;
                for t in hull(&vertices, &planes, self.f32(at + 0x10)?) {
                    out.push(Triangle { vertices: t.map(|v| m.transform_point3(v)), pat });
                }
            }
            Some("hkpStorageExtendedMeshShape") => {
                let (subparts, n) = self.array(at + 0x94)?;
                let (storage, ns) = self.array(at + 0xc0)?;
                ensure!(ns >= n, "ND Havok mesh storage");
                for i in 0..n {
                    self.storage_subpart(subparts + 0x40 * i, self.ptr(storage + 4 * i)?, m, out)?;
                }
            }
            // `hkpBoxShape` half extents at +0x20 (no instance in the data).
            Some("hkpBoxShape") => {
                let h = self.vec3(at + 0x20)?;
                let vertices: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -h.x } else { h.x }, if i & 2 == 0 { -h.y } else { h.y }, if i & 4 == 0 { -h.z } else { h.z })).collect();
                let planes = [(Vec3::X, -h.x), (-Vec3::X, -h.x), (Vec3::Y, -h.y), (-Vec3::Y, -h.y), (Vec3::Z, -h.z), (-Vec3::Z, -h.z)];
                let pat = self.u32(at + 8)?;
                for t in hull(&vertices, &planes, 0.) {
                    out.push(Triangle { vertices: t.map(|v| m.transform_point3(v)), pat });
                }
            }
            other => anyhow::bail!("ND Havok shape class {other:?}"),
        }
        ensure!(out.len() <= MAX_TRIANGLES, "ND collision triangle count");
        Ok(())
    }

    fn storage_subpart(&self, sub: usize, storage: usize, m: Mat4, out: &mut Vec<Triangle>) -> Result<()> {
        let triangles = self.u32(sub + 0x10)? as usize;
        let index_stride = self.u32(sub + 0x34)? as usize;
        let int16 = *self.b.get(sub + 0x38).context("ND Havok subpart")? == 1;
        let (vertices, nv) = self.array(storage + 0x08)?;
        let (i16s, _) = self.array(storage + 0x14)?;
        let (i32s, _) = self.array(storage + 0x20)?;
        let (indices8, n8) = self.array(storage + 0x2c)?;
        let (materials, nm) = self.array(storage + 0x38)?;
        let (indices16, n16) = self.array(storage + 0x44)?;
        let pats = (0..nm / 2).map(|k| self.u32(materials + 8 * k + 4)).collect::<Result<Vec<_>>>()?;
        let vertex = |k: usize| -> Result<Vec3> {
            ensure!(k < nv, "ND Havok vertex index");
            Ok(m.transform_point3(self.vec3(vertices + 16 * k)?))
        };
        for t in 0..triangles {
            let [a, b, c] = if int16 {
                let at = i16s + index_stride * t;
                [0, 2, 4].map(|o| u16::from_be_bytes([self.b.get(at + o).copied().unwrap_or(0), self.b.get(at + o + 1).copied().unwrap_or(0)]) as usize)
            } else {
                let at = i32s + index_stride * t;
                [0, 4, 8].map(|o| self.u32(at + o).unwrap_or(u32::MAX) as usize)
            };
            let k = if n8 > 0 {
                usize::from(*self.b.get(indices8 + t).context("ND Havok material index")?)
            } else if n16 > 0 {
                usize::from(u16::from_be_bytes([self.b[indices16 + 2 * t], self.b[indices16 + 2 * t + 1]]))
            } else {
                0
            };
            out.push(Triangle { vertices: [vertex(a)?, vertex(b)?, vertex(c)?], pat: pats.get(k).copied().unwrap_or(0) });
        }
        Ok(())
    }
}

/// Triangulate a convex hull. Havok stores the vertices shrunk by the
/// convex radius while the planes `n·v + d = 0` keep the surface: a vertex
/// lies on a face when `n·v + d = −radius`, and the face is pushed out by
/// the radius (rounded hull edges are not modelled).
fn hull(vertices: &[Vec3], planes: &[(Vec3, f32)], radius: f32) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    for &(n, d) in planes {
        let mut on: Vec<Vec3> = Vec::new();
        for v in vertices.iter().filter(|v| (n.dot(**v) + d + radius).abs() < 2e-3).map(|v| *v + n * radius) {
            if on.iter().all(|u| u.distance_squared(v) > 1e-10) {
                on.push(v);
            }
        }
        if on.len() < 3 {
            continue;
        }
        let c = on.iter().copied().sum::<Vec3>() / on.len() as f32;
        let u = n.cross(if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y }).normalize();
        let w = n.cross(u);
        on.sort_by(|a, b| (*a - c).dot(w).atan2((*a - c).dot(u)).total_cmp(&(*b - c).dot(w).atan2((*b - c).dot(u))));
        for k in 1..on.len() - 1 {
            out.push([on[0], on[k], on[k + 1]]);
        }
    }
    out
}

/// Prop collision objects of an art-group package in the art-group frame;
/// `bind` gives each joint's bind-pose transform.
pub fn prop_objects(p: &Loaded, bind: &dyn Fn(usize) -> Option<Mat4>) -> Result<Vec<Object>> {
    let mut out = Vec::new();
    for item in p.directory.items.iter().filter(|i| i.kind == "COLLISION_DATA_FOREGROUND") {
        let at = item.payload_offset as usize;
        ensure!(p.u32(at)? == 0x1001, "ND foreground collision marker");
        let n = p.u32(at + 4)?;
        ensure!(n <= 4096, "ND collision entry count");
        let entries = if n == 0 { 0 } else { p.ptr(at + 8)? };
        for i in 0..n as usize {
            let e = p.ptr(entries + 4 * i)?;
            let joint = usize::try_from(p.u32(e + 0x20)? as i32).ok();
            let m = joint.and_then(bind).unwrap_or(Mat4::IDENTITY);
            let wrapper = p.ptr(e + 0x28)?;
            let header = p.u32(wrapper + 4)? != 0;
            let havok = Packfile::read(&p.data, wrapper + if header { 0x20 } else { 0x10 })?;
            ensure!(havok.classes.get(&havok.root).map(String::as_str) == Some("hkpRigidBody"), "ND collision root is not a rigid body");
            // Rigid-body transform: rotation columns at +0xf0, translation +0x120.
            let r = havok.root;
            let col = |k: usize| havok.vec3(r + 0xf0 + 16 * k);
            let body = Mat4::from_cols(col(0)?.extend(0.), col(1)?.extend(0.), col(2)?.extend(0.), havok.vec3(r + 0x120)?.extend(1.));
            let mut triangles = Vec::new();
            havok.shape(havok.ptr(r + 0x10)?, m * body, &mut triangles, 0)?;
            let features = match header.then(|| p.ptr(wrapper + 0x10).ok()).flatten() {
                Some(db) => feature_db(p, db)?.transformed(m),
                None => FeatureDb::default(),
            };
            out.push(Object { joint, triangles, features });
        }
    }
    Ok(out)
}

/// Level collision triangles (`COLLISION_DATA_HAVOK_BG`) and the level's
/// feature database (`FEATURE_DB_2`), in level space.
pub fn level_object(p: &Loaded) -> Result<Object> {
    let mut triangles = Vec::new();
    for item in p.directory.items.iter().filter(|i| i.kind == "COLLISION_DATA_HAVOK_BG") {
        let at = item.payload_offset as usize;
        if p.u32(at)? <= 1 {
            continue;
        }
        let (records, subparts, n) = (p.ptr(at + 4)?, p.ptr(at + 8)?, p.u32(at + 0xc)? as usize);
        ensure!(n <= 4096, "ND level collision subparts");
        for i in 0..n {
            let sub = p.ptr(subparts + 4 * i)?;
            let record = records + 12 * i;
            let count = p.u32(record + 8)? as usize;
            let pats = if count == 0 { Vec::new() } else { (0..count).map(|k| p.u32(p.ptr(record + 4)? + 8 * k + 4)).collect::<Result<Vec<_>>>()? };
            let index_of = p.ptr(record).ok();
            let triangles_n = p.u32(sub + 0x10)? as usize;
            let (vertices, vstride) = (p.ptr(sub + 0x14)?, p.u32(sub + 0x18)? as usize);
            let (indices, istride) = (p.ptr(sub + 0x30)?, p.u32(sub + 0x34)? as usize);
            let int16 = *p.data.get(sub + 0x38).context("ND level subpart")? == 1;
            ensure!(triangles.len() + triangles_n <= MAX_TRIANGLES, "ND level collision triangle count");
            let vertex = |k: usize| -> Result<Vec3> { Ok(Vec3::new(p.f32(vertices + vstride * k)?, p.f32(vertices + vstride * k + 4)?, p.f32(vertices + vstride * k + 8)?)) };
            for t in 0..triangles_n {
                let at = indices + istride * t;
                let index = |k: usize| -> Result<usize> { if int16 { p.u16(at + 2 * k).map(usize::from) } else { p.u32(at + 4 * k).map(|v| v as usize) } };
                let (a, b, c) = (index(0)?, index(1)?, index(2)?);
                let k = match index_of {
                    Some(m) => usize::from(*p.data.get(m + t).context("ND level material index")?),
                    None => 0,
                };
                triangles.push(Triangle { vertices: [vertex(a)?, vertex(b)?, vertex(c)?], pat: pats.get(k).copied().unwrap_or(0) });
            }
        }
    }
    let mut features = FeatureDb::default();
    for item in p.directory.items.iter().filter(|i| i.kind == "FEATURE_DB_2") {
        let db = feature_db(p, item.payload_offset as usize)?;
        features.edges.extend(db.edges);
        features.corners.extend(db.corners);
    }
    Ok(Object { joint: None, triangles, features })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_of_a_cube_has_twelve_outward_triangles() {
        let r = 0.1;
        // Vertices shrunk by the radius; planes at the unit cube's faces.
        let s = 0.5 - r;
        let vertices: Vec<Vec3> = (0..8).map(|i| Vec3::new(if i & 1 == 0 { -s } else { s }, if i & 2 == 0 { -s } else { s }, if i & 4 == 0 { -s } else { s })).collect();
        let planes = [(Vec3::X, -0.5), (-Vec3::X, -0.5), (Vec3::Y, -0.5), (-Vec3::Y, -0.5), (Vec3::Z, -0.5), (-Vec3::Z, -0.5)];
        let tris = hull(&vertices, &planes, r);
        assert_eq!(tris.len(), 12);
        for t in &tris {
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            let c = (t[0] + t[1] + t[2]) / 3.;
            assert!(n.dot(c) > 0., "outward winding");
            assert!(t.iter().all(|v| v.abs().max_element() > 0.499), "faces on the planes");
        }
    }
}
