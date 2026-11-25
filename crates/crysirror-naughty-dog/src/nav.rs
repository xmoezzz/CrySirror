// SPDX-License-Identifier: MPL-2.0
//! Navigation data: `NAV_MESH_1` polygon meshes and their A* path search
//! (`NavControl::PathFindPs`, `sub_2AEF88`), `SPLINE_TABLE` splines
//! (`sub_39F60`, `sub_36960`) and the cover points the engine derives from
//! `FEATURE_DB_2` corners at level login (`sub_B6A98`, `sub_197F10`).
//!
//! Frames: `LEVEL_OFFSET_1` (level +0x20) is added to nav-mesh origins,
//! spline points, feature-database points and the level collision; the
//! callers here keep every level in its stored frame (zero offset on the
//! train-wreck and menu levels; SATD-nd-retail-runtime).
//!
//! Nav mesh header (0x50 bytes): origin (+0), samples (+0x10, count
//! +0x14), name (+0x1c), half extent (+0x20), `u16` polygon count (+0x30),
//! `u8` link count (+0x32), polygons (+0x34, 80 bytes), lookup grid
//! (+0x3c), links (+0x40, 16 bytes), name `StringId` (+0x48) and parent
//! spawner (+0x4c). A polygon holds up to four mesh-local vertices (+0x10,
//! +0x20, +0x30, +0x40; triangles repeat v0), its index (+0x1c), vertex
//! count (+0x1d), flags (+0x1e: 0x02 blocked, 0x20 link polygon), link index
//! (+0x1f), per-edge neighbours (+0x2c, 0xff none; edge k runs v_k →
//! v_k+1) and a cost multiplier (+4). Polygons wind clockwise in (x, z).
//! Links `{StringId, …, u8 link polygon (+0xc), u8 pre-link polygon (+0xd)}`
//! pair meshes by `StringId`; a link polygon is a copy of the partner's
//! pre-link polygon, so the search steps onto that polygon.
//!
//! Spline record (48 bytes): name `StringId` (+0), parent spawner (+4),
//! flags (+0xc: 1 closed, 2 linear), properties (+0x10 count, +0x14
//! `{key, type, value}`), points (+0x18 count, +0x1c vec4), arc tables
//! (+0x24, 31 parameters per segment at `(k+1)/32` of its length) and
//! cumulative segment lengths (+0x28). Curved splines are uniform
//! Catmull-Rom with clamped (open) or wrapped (closed) end points.
use crate::collision::Edge;
use crate::mesh::Loaded;
use anyhow::{Result, ensure};
use glam::Vec3;
use std::collections::{BinaryHeap, HashMap};

const POLY_BLOCKED: u8 = 0x02;
const POLY_LINK: u8 = 0x20;
/// `FindPoly`'s vertical tolerance (all.c:464017).
const FIND_TOLERANCE: f32 = 2.;
/// The native open list's capacity.
const OPEN_MAX: usize = 300;
const NONE: u8 = 0xff;

#[derive(Clone, Debug, PartialEq)]
pub struct Poly {
    /// Mesh-local vertices (`count` used).
    pub vertices: [Vec3; 4],
    pub count: usize,
    pub flags: u8,
    pub link: u8,
    pub neighbours: [u8; 4],
    pub cost: f32,
}

impl Poly {
    pub fn centroid(&self) -> Vec3 {
        self.vertices[..self.count].iter().copied().sum::<Vec3>() / self.count as f32
    }

    /// The 4-edge XZ containment test (`sub_2B8368`).
    fn contains_xz(&self, p: Vec3) -> bool {
        (0..4).all(|k| {
            let (a, b) = (self.vertices[k], self.vertices[(k + 1) % 4]);
            a == b || (b.x - a.x) * (p.z - a.z) - (b.z - a.z) * (p.x - a.x) <= 1e-6
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: u32,
    pub poly: u8,
    pub pre_link: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavMesh {
    pub name: String,
    pub id: u32,
    /// Parent spawner `StringId` (moving platforms; its frame is not
    /// composed here).
    pub parent: Option<u32>,
    pub origin: Vec3,
    pub polys: Vec<Poly>,
    pub links: Vec<Link>,
}

impl NavMesh {
    pub fn read(p: &Loaded, at: usize) -> Result<Self> {
        let v = |o: usize| -> Result<Vec3> { Ok(Vec3::new(p.f32(o)?, p.f32(o + 4)?, p.f32(o + 8)?)) };
        let byte = |o: usize| -> Result<u8> { p.data.get(o).copied().ok_or_else(|| anyhow::anyhow!("ND nav mesh outside package")) };
        let count = usize::from(p.u16(at + 0x30)?);
        let links = usize::from(byte(at + 0x32)?);
        ensure!(count <= 255, "ND nav polygon count");
        let base = p.ptr(at + 0x34)?;
        let polys = (0..count)
            .map(|i| {
                let a = base + 80 * i;
                ensure!(usize::from(byte(a + 0x1c)?) == i, "ND nav polygon index");
                let count = usize::from(byte(a + 0x1d)?);
                ensure!((3..=4).contains(&count), "ND nav polygon vertex count");
                Ok(Poly {
                    vertices: [v(a + 0x10)?, v(a + 0x20)?, v(a + 0x30)?, v(a + 0x40)?],
                    count,
                    flags: byte(a + 0x1e)?,
                    link: byte(a + 0x1f)?,
                    neighbours: [byte(a + 0x2c)?, byte(a + 0x2d)?, byte(a + 0x2e)?, byte(a + 0x2f)?],
                    cost: p.f32(a + 4)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let links = match p.ptr(at + 0x40) {
            Ok(base) => (0..links).map(|k| Ok(Link { id: p.u32(base + 16 * k)?, poly: byte(base + 16 * k + 12)?, pre_link: byte(base + 16 * k + 13)? })).collect::<Result<Vec<_>>>()?,
            Err(_) => Vec::new(),
        };
        let name = p.ptr(at + 0x1c).ok().and_then(|n| {
            let b = p.data.get(n..)?;
            let e = b.iter().take(256).position(|c| *c == 0)?;
            Some(String::from_utf8_lossy(&b[..e]).into_owned())
        });
        let parent = p.u32(at + 0x4c)?;
        Ok(Self { name: name.unwrap_or_default(), id: p.u32(at + 0x48)?, parent: (parent != 0).then_some(parent), origin: v(at)?, polys, links })
    }

    /// The polygon holding `point` (level frame): not blocked, containing
    /// it in XZ, the least vertical distance outside its height range and
    /// within the tolerance (`sub_2B8748`; the lookup grid only narrows the
    /// candidates).
    pub fn find(&self, point: Vec3) -> Option<usize> {
        self.find_any(point, false)
    }

    fn find_any(&self, point: Vec3, blocked: bool) -> Option<usize> {
        let local = point - self.origin;
        let mut best = None;
        let mut distance = FIND_TOLERANCE;
        for (i, q) in self.polys.iter().enumerate() {
            if (!blocked && q.flags & POLY_BLOCKED != 0) || !q.contains_xz(local) {
                continue;
            }
            let (lo, hi) = q.vertices.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.y), hi.max(v.y)));
            let d = (lo - local.y).max(local.y - hi).max(0.);
            if d < distance {
                best = Some(i);
                distance = d;
            }
        }
        best
    }
}

/// A polygon of a [`Navigation`]: mesh and polygon index.
pub type PolyRef = (usize, usize);

/// The registered nav meshes of the loaded levels with their links.
#[derive(Clone, Debug, Default)]
pub struct Navigation {
    pub meshes: Vec<NavMesh>,
    /// `(mesh, link)` → the partner's pre-link polygon.
    partners: HashMap<(usize, usize), PolyRef>,
}

#[derive(PartialEq)]
struct Open(f32, usize, PolyRef);
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let d = b - a;
    let t = if d.length_squared() > 0. { ((p - a).dot(d) / d.length_squared()).clamp(0., 1.) } else { 0. };
    a + d * t
}

impl Navigation {
    pub fn add(&mut self, mesh: NavMesh) {
        self.meshes.push(mesh);
        self.link();
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Pair links of the same `StringId` (`NavMesh::Login`); the partner
    /// polygon must be a quad with a vertex within 5 cm of the link
    /// polygon's vertex 1 (`LoginLink`, `sub_2BBC30`).
    fn link(&mut self) {
        let mut by_id: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
        for (m, mesh) in self.meshes.iter().enumerate() {
            for (k, l) in mesh.links.iter().enumerate() {
                by_id.entry(l.id).or_default().push((m, k));
            }
        }
        self.partners.clear();
        for pair in by_id.values().filter(|v| v.len() == 2) {
            for (&(m, k), &(n, l)) in [(&pair[0], &pair[1]), (&pair[1], &pair[0])] {
                let (a, b) = (&self.meshes[m], &self.meshes[n]);
                let Some(lp) = a.polys.get(usize::from(a.links[k].poly)) else { continue };
                let target = usize::from(b.links[l].pre_link);
                let Some(dp) = b.polys.get(target) else { continue };
                let anchor = a.origin + lp.vertices[1];
                if dp.count == 4 && dp.vertices.iter().any(|v| (b.origin + *v).distance_squared(anchor) < 0.0025) {
                    self.partners.insert((m, k), (n, target));
                }
            }
        }
    }

    pub fn find(&self, point: Vec3) -> Option<PolyRef> {
        self.meshes.iter().enumerate().find_map(|(m, mesh)| mesh.find(point).map(|q| (m, q)))
    }

    /// Block or unblock the polygon under `point` (`sub_F4348`, the nav
    /// blockers of `sub_F44D0`); false when no polygon holds it.
    pub fn set_blocked(&mut self, point: Vec3, blocked: bool) -> bool {
        let Some((m, q)) = self.meshes.iter().enumerate().find_map(|(m, mesh)| mesh.find_any(point, true).map(|q| (m, q))) else { return false };
        let flags = &mut self.meshes[m].polys[q].flags;
        if blocked { *flags |= POLY_BLOCKED } else { *flags &= !POLY_BLOCKED }
        true
    }

    /// A link polygon stands for the partner's pre-link polygon.
    fn resolve(&self, (m, q): PolyRef) -> Option<PolyRef> {
        let poly = &self.meshes[m].polys[q];
        if poly.flags & POLY_LINK != 0 && poly.link != NONE {
            return self.partners.get(&(m, usize::from(poly.link))).copied();
        }
        Some((m, q))
    }

    /// The corridor from `start` to `goal` (level frame) as world points:
    /// A* over polygons, a successor's node at the closest point of the
    /// shared edge to the current node, `g + |step|`, `h = |goal − p|`, at
    /// most 300 open nodes. `None` when either end is off the meshes or no
    /// path exists. Path smoothing is not reproduced.
    pub fn path(&self, start: Vec3, goal: Vec3) -> Option<Vec<Vec3>> {
        let s = self.resolve(self.find(start)?)?;
        let g = self.resolve(self.find(goal)?)?;
        let mut node: HashMap<PolyRef, (Vec3, f32, Option<PolyRef>)> = HashMap::from([(s, (start, 0., None))]);
        let mut closed = std::collections::HashSet::new();
        let mut open = BinaryHeap::from([Open(start.distance(goal), 0, s)]);
        let mut sequence = 1;
        while let Some(Open(_, _, q)) = open.pop() {
            if !closed.insert(q) {
                continue;
            }
            if q == g {
                break;
            }
            let (pos, cost, _) = node[&q];
            let poly = &self.meshes[q.0].polys[q.1];
            let origin = self.meshes[q.0].origin;
            for k in 0..poly.count {
                let j = poly.neighbours[k];
                if j == NONE || usize::from(j) >= self.meshes[q.0].polys.len() {
                    continue;
                }
                let Some(r) = self.resolve((q.0, usize::from(j))) else { continue };
                if closed.contains(&r) || self.meshes[r.0].polys[r.1].flags & POLY_BLOCKED != 0 {
                    continue;
                }
                let p = closest_on_segment(origin + poly.vertices[k], origin + poly.vertices[(k + 1) % poly.count], pos);
                let ng = cost + pos.distance(p);
                if node.get(&r).is_none_or(|n| ng < n.1) {
                    node.insert(r, (p, ng, Some(q)));
                    if open.len() < OPEN_MAX {
                        open.push(Open(ng + p.distance(goal), sequence, r));
                        sequence += 1;
                    }
                }
            }
        }
        node.get(&g)?;
        let mut out = vec![goal];
        let mut at = Some(g);
        while let Some(q) = at {
            let (p, _, parent) = node[&q];
            out.push(p);
            at = parent;
        }
        out.reverse();
        Some(out)
    }
}

/// Every nav mesh of a package.
pub fn nav_meshes(p: &Loaded, directory: &crate::package::Directory) -> Result<Vec<NavMesh>> {
    directory.items.iter().filter(|i| i.kind == "NAV_MESH_1").map(|i| NavMesh::read(p, i.payload_offset as usize)).collect()
}

/// Catmull-Rom (tension ½) and its derivative.
fn catmull_rom(c: [Vec3; 4], u: f32) -> (Vec3, Vec3) {
    let [a, b, cc, d] = c;
    let (u2, u3) = (u * u, u * u * u);
    let p = 0.5 * (2. * b + (cc - a) * u + (2. * a - 5. * b + 4. * cc - d) * u2 + (3. * b - a - 3. * cc + d) * u3);
    let t = 0.5 * ((cc - a) + 2. * (2. * a - 5. * b + 4. * cc - d) * u + 3. * (3. * b - a - 3. * cc + d) * u2);
    (p, t)
}

/// Five-point Gauss–Legendre nodes and weights.
const GAUSS: [(f32, f32); 5] = [(-0.906_179_8, 0.236_926_88), (-0.538_469_3, 0.478_628_67), (0., 0.568_888_9), (0.538_469_3, 0.478_628_67), (0.906_179_8, 0.236_926_88)];
const ARC_SAMPLES: usize = 31;

#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    pub id: u32,
    /// Parent spawner name (`StringId`).
    pub parent: Option<u32>,
    pub flags: u32,
    /// `{key, type, value}` properties.
    pub properties: Vec<(u32, u32, u32)>,
    pub points: Vec<Vec3>,
    /// Cumulative segment end lengths.
    pub cumulative: Vec<f32>,
    /// Per curved segment, the parameters at `(k+1)/32` of its length.
    pub arc: Vec<[f32; ARC_SAMPLES]>,
}

impl Spline {
    pub fn closed(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Linear: flag 2, or two points (`sub_36960`).
    pub fn linear(&self) -> bool {
        self.flags & 2 != 0 || self.points.len() + usize::from(self.closed()) == 2
    }

    pub fn segments(&self) -> usize {
        (self.points.len() + usize::from(self.closed())).saturating_sub(1)
    }

    pub fn length(&self) -> f32 {
        self.cumulative.last().copied().unwrap_or(0.)
    }

    /// Segment `i`'s control points (`sub_34528`).
    fn controls(&self, i: usize) -> [Vec3; 4] {
        let n = self.points.len();
        let idx = if self.closed() { [(i + n - 1) % n, i % n, (i + 1) % n, (i + 2) % n] } else { [i.saturating_sub(1), i, (i + 1).min(n - 1), (i + 2).min(n - 1)] };
        idx.map(|j| self.points[j])
    }

    fn eval(&self, i: usize, u: f32) -> (Vec3, Vec3) {
        let c = self.controls(i);
        if self.linear() { (c[1] + (c[2] - c[1]) * u, c[2] - c[1]) } else { catmull_rom(c, u) }
    }

    /// Arc length of segment `i` over `[0, u]` (`sub_90A3D0`).
    fn arc_length(&self, i: usize, u: f32) -> f32 {
        if self.linear() {
            let c = self.controls(i);
            return c[1].distance(c[2]) * u;
        }
        let steps = 8;
        (0..steps)
            .map(|k| {
                let (a, b) = (u * k as f32 / steps as f32, u * (k + 1) as f32 / steps as f32);
                GAUSS.iter().map(|(x, w)| w * 0.5 * (b - a) * self.eval(i, 0.5 * (a + b) + 0.5 * (b - a) * x).1.length()).sum::<f32>()
            })
            .sum()
    }

    /// The arc table of segment `i` (`sub_36480`, Newton steps).
    fn arc_table(&self, i: usize) -> [f32; ARC_SAMPLES] {
        let length = self.arc_length(i, 1.);
        std::array::from_fn(|k| {
            let target = (k + 1) as f32 / 32. * length;
            let mut u = target / length.max(1e-6);
            for _ in 0..7 {
                let speed = self.eval(i, u).1.length();
                let error = self.arc_length(i, u) - target;
                if speed <= 0. || error * error < 1e-14 {
                    break;
                }
                u = (u - error / speed).clamp(0., 1.);
            }
            u
        })
    }

    /// Segment and parameter at arc distance `s` (`sub_35768`, `sub_33268`):
    /// closed splines wrap, open ones clamp.
    fn locate(&self, s: f32) -> (usize, f32) {
        let length = self.length();
        let n = self.segments();
        if n == 0 || length <= 0. {
            return (0, 0.);
        }
        let s = if self.closed() { s.rem_euclid(length) } else { s };
        if s >= length {
            return (n - 1, 1.);
        }
        if s <= 0. {
            return (0, 0.);
        }
        let i = self.cumulative.iter().position(|c| s < *c).unwrap_or(n - 1);
        let base = if i > 0 { self.cumulative[i - 1] } else { 0. };
        let span = self.cumulative[i] - base;
        let f = if span > 0. { ((s - base) / span).clamp(0., 1.) } else { 0. };
        if self.linear() || f >= 1. {
            return (i, f);
        }
        let table = &self.arc[i];
        let j = (f * 32.).floor() as usize;
        let lo = if j == 0 { 0. } else { table[j - 1] };
        let hi = if j >= ARC_SAMPLES { 1. } else { table[j] };
        (i, if hi > lo { lo + (f - j as f32 / 32.) * 32. * (hi - lo) } else { lo })
    }

    /// Position and unit tangent at arc distance `s` (`sub_35A20`).
    pub fn at(&self, s: f32) -> (Vec3, Vec3) {
        if self.points.is_empty() {
            return (Vec3::ZERO, Vec3::Z);
        }
        let (i, u) = self.locate(s);
        let (p, t) = self.eval(i, u);
        (p, t.normalize_or(Vec3::Z))
    }

    /// Arc distance of the closest sampled point to `point` (the native
    /// closest-point search, `sub_35DB8`, is not decoded).
    pub fn closest_distance(&self, point: Vec3) -> f32 {
        let steps = 64;
        let mut best = (f32::MAX, 0.);
        for i in 0..self.segments() {
            let base = if i > 0 { self.cumulative[i - 1] } else { 0. };
            let span = self.cumulative[i] - base;
            for k in 0..=steps {
                let s = base + span * k as f32 / steps as f32;
                let d = self.at(s).0.distance(point);
                if d < best.0 {
                    best = (d, s);
                }
            }
        }
        best.1
    }
}

/// Every spline of a package's `SPLINE_TABLE` items (`u32` count, 48-byte
/// records); lengths and arc tables are computed as at login.
pub fn splines(p: &Loaded, directory: &crate::package::Directory) -> Result<Vec<Spline>> {
    let mut out = Vec::new();
    for item in directory.items.iter().filter(|i| i.kind == "SPLINE_TABLE") {
        let at = item.payload_offset as usize;
        let n = p.u32(at)? as usize;
        ensure!(n <= 4096, "ND spline count");
        for i in 0..n {
            let a = at + 4 + 48 * i;
            let parent = p.ptr(a + 4).ok().and_then(|s| p.ptr(s + 0x20).ok()).and_then(|t| {
                let b = p.data.get(t..)?;
                let e = b.iter().take(256).position(|c| *c == 0)?;
                Some(crate::dc::sid(&String::from_utf8_lossy(&b[..e])))
            });
            let count = p.u32(a + 0x18)? as usize;
            ensure!(count <= 4096, "ND spline point count");
            let points = match p.ptr(a + 0x1c) {
                Ok(base) => (0..count).map(|k| Ok(Vec3::new(p.f32(base + 16 * k)?, p.f32(base + 16 * k + 4)?, p.f32(base + 16 * k + 8)?))).collect::<Result<Vec<_>>>()?,
                Err(_) => Vec::new(),
            };
            let props = p.u32(a + 0x10)? as usize;
            ensure!(props <= 256, "ND spline property count");
            let properties = match p.ptr(a + 0x14) {
                Ok(base) => (0..props).map(|k| Ok((p.u32(base + 12 * k)?, p.u32(base + 12 * k + 4)?, p.u32(base + 12 * k + 8)?))).collect::<Result<Vec<_>>>()?,
                Err(_) => Vec::new(),
            };
            let mut spline = Spline { id: p.u32(a)?, parent, flags: p.u32(a + 0xc)?, properties, points, cumulative: Vec::new(), arc: Vec::new() };
            let mut total = 0.;
            for s in 0..spline.segments() {
                total += spline.arc_length(s, 1.);
                spline.cumulative.push(total);
            }
            if !spline.linear() {
                spline.arc = (0..spline.segments()).map(|s| spline.arc_table(s)).collect();
            }
            out.push(spline);
        }
    }
    Ok(out)
}

/// A cover point (`sub_197F10`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cover {
    pub position: Vec3,
    /// Into the wall, horizontal.
    pub direction: Vec3,
    pub height: f32,
    /// 0 high-left, 1 high-right, 2 low-left, 3 low-right.
    pub kind: u8,
    /// Usable by the player (`CornerNoPlayer` clear).
    pub player: bool,
}

/// Corner flags (`sub_62EC50` prints them): `CornerHandPlaced` 0x1,
/// `CornerLowCover` 0x2, `CornerCanStepOut` 0x4, `CornerPeekLeft` 0x8,
/// `CornerPeekRight` 0x10, `CornerPeekOver` 0x20, `CornerNoPlayer` 0x40,
/// `CornerForeground` 0x2000, `CornerIsActive` 0x8000.
pub const CORNER_LOW_COVER: u16 = 0x2;
pub const CORNER_PEEK_LEFT: u16 = 0x8;
pub const CORNER_PEEK_RIGHT: u16 = 0x10;
pub const CORNER_PEEK_OVER: u16 = 0x20;
pub const CORNER_NO_PLAYER: u16 = 0x40;

/// The cover point of a `FEATURE_DB_2` corner: base `p0`, direction `−n0`
/// in XZ, height `p1.y − p0.y`; low when flagged low cover or peeked over,
/// the side by the peek flags. Corners without a peek are not cover.
pub fn cover(corner: &Edge) -> Option<Cover> {
    let height = corner.p1.y - corner.p0.y;
    let f = corner.flags;
    let low = f & (CORNER_LOW_COVER | CORNER_PEEK_OVER) != 0;
    let side = if f & CORNER_PEEK_LEFT != 0 {
        0
    } else if f & CORNER_PEEK_RIGHT != 0 || f & CORNER_PEEK_OVER != 0 {
        1
    } else {
        return None;
    };
    let kind = if low { 2 + side } else { side };
    let d = Vec3::new(-corner.n0.x, 0., -corner.n0.z);
    Some(Cover { position: corner.p0, direction: if d.length() > 1e-4 { d.normalize() } else { d }, height, kind, player: f & CORNER_NO_PLAYER == 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(x0: f32, x1: f32) -> Poly {
        // Clockwise in (x, z).
        Poly { vertices: [Vec3::new(x0, 0., 0.), Vec3::new(x0, 0., 1.), Vec3::new(x1, 0., 1.), Vec3::new(x1, 0., 0.)], count: 4, flags: 0, link: NONE, neighbours: [NONE; 4], cost: 1. }
    }

    #[test]
    fn corner_flags_select_cover_kinds() {
        let corner = |flags| Edge { p0: Vec3::ZERO, p1: Vec3::Y * 2., n0: Vec3::X, n1: Vec3::Z, prev: None, next: None, flags };
        assert_eq!(cover(&corner(CORNER_PEEK_LEFT)).unwrap().kind, 0);
        assert_eq!(cover(&corner(CORNER_PEEK_RIGHT)).unwrap().kind, 1);
        assert_eq!(cover(&corner(CORNER_PEEK_LEFT | CORNER_LOW_COVER)).unwrap().kind, 2);
        assert_eq!(cover(&corner(CORNER_PEEK_OVER)).unwrap().kind, 3);
        assert!(!cover(&corner(CORNER_PEEK_LEFT | CORNER_NO_PLAYER)).unwrap().player);
        assert!(cover(&corner(0x8000)).is_none());
    }

    #[test]
    fn path_crosses_shared_edges() {
        let mut a = quad(0., 1.);
        let mut b = quad(1., 2.);
        a.neighbours[2] = 1;
        b.neighbours[0] = 0;
        let mut nav = Navigation::default();
        nav.add(NavMesh { name: String::new(), id: 0, parent: None, origin: Vec3::ZERO, polys: vec![a, b], links: vec![] });
        assert_eq!(nav.find(Vec3::new(0.5, 0.5, 0.5)), Some((0, 0)));
        let path = nav.path(Vec3::new(0.5, 0., 0.5), Vec3::new(1.5, 0., 0.5)).unwrap();
        assert_eq!(path.len(), 3);
        assert!((path[1].x - 1.).abs() < 1e-6);
    }

    #[test]
    fn catmull_rom_spline_ends_and_length() {
        let mut s = Spline { id: 0, parent: None, flags: 0, properties: vec![], points: vec![Vec3::ZERO, Vec3::X, Vec3::new(2., 0., 0.)], cumulative: vec![], arc: vec![] };
        let mut total = 0.;
        for i in 0..s.segments() {
            total += s.arc_length(i, 1.);
            s.cumulative.push(total);
        }
        s.arc = (0..s.segments()).map(|i| s.arc_table(i)).collect();
        assert!((s.length() - 2.).abs() < 1e-3);
        assert!((s.at(1.).0 - Vec3::X).length() < 1e-3);
        assert!((s.at(5.).0 - Vec3::new(2., 0., 0.)).length() < 1e-4);
        assert!((s.closest_distance(Vec3::new(1.5, 1., 0.)) - 1.5).abs() < 0.05);
    }
}
