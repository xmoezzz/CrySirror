// SPDX-License-Identifier: MPL-2.0
//! Walkable surfaces and solids for the player and camera, and the climb
//! edges: upward-facing triangles bucketed in a horizontal grid, every
//! triangle in a ray hierarchy. Native collision ([`crate::collision`]) is
//! used where a package has it (its PAT flags decide what stands and
//! blocks); packages without it fall back to their render triangles. The
//! queries are the viewer's, not Havok's (SATD-nd-retail-runtime). Spawned
//! art groups are movable bodies tested at their current transform; their
//! edges move with them.
use crate::collision::{self, Edge, Triangle};
use crate::{mesh::Mesh, scene};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

const CELL: f32 = 4.;
/// Triangles steeper than this (normal y below) are not walkable.
const MIN_UP: f32 = 0.5;
const MAX_TRIANGLES: usize = 6 << 20;

#[derive(Default)]
pub struct Ground {
    tris: Vec<[[f32; 3]; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// Every placed triangle, for camera rays, and its bounding-volume
    /// hierarchy (built by [`Ground::prepare`]).
    solid: Vec<[[f32; 3]; 3]>,
    nodes: Vec<Node>,
    /// Level climb edges (world space).
    edges: Vec<Edge>,
    /// Spawned art groups, which scripts can move (`animate` root motion):
    /// tested at their current transform.
    bodies: HashMap<u32, Body>,
}

/// A movable set of triangles (an entity's meshes).
struct Body {
    local: Vec<[[f32; 3]; 3]>,
    world: Vec<[[f32; 3]; 3]>,
    /// Per triangle: whether it can be stood on (not `NoStand`).
    stand: Vec<bool>,
    edges_local: Vec<Edge>,
    edges: Vec<Edge>,
    lo: [f32; 3],
    hi: [f32; 3],
    enabled: bool,
    /// Its edges can be grabbed (`disable-object-features` clears it).
    features: bool,
    /// Column-major world transform.
    transform: [f32; 16],
}

impl Body {
    fn place(&mut self, m: [f32; 16]) {
        self.transform = m;
        let tf = |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|c| p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c]) };
        self.world = self.local.iter().map(|t| t.map(tf)).collect();
        self.edges = collision::FeatureDb { edges: self.edges_local.clone(), corners: Vec::new() }.transformed(Mat4::from_cols_array(&m)).edges;
        self.lo = [f32::MAX; 3];
        self.hi = [f32::MIN; 3];
        for v in self.world.iter().flatten() {
            for k in 0..3 {
                self.lo[k] = self.lo[k].min(v[k]);
                self.hi[k] = self.hi[k].max(v[k]);
            }
        }
    }
}

const MAX_BODY_TRIANGLES: usize = 1 << 18;

const MAX_SOLID: usize = 8 << 20;
const LEAF: usize = 8;

/// A BVH node: bounds and either children (`count == 0`, left child at
/// `start`, right at `start + 1`) or `count` triangles from `start`.
struct Node {
    lo: [f32; 3],
    hi: [f32; 3],
    start: u32,
    count: u32,
}

fn cell(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl Ground {
    pub fn len(&self) -> usize {
        self.tris.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Add the walkable triangles of placed meshes (metres).
    pub fn add(&mut self, meshes: &[Mesh]) {
        self.add_placed(meshes, None);
    }

    /// Add placed meshes moved by a column-major world matrix (an actor's
    /// spawner transform).
    pub fn add_placed(&mut self, meshes: &[Mesh], world: Option<[f32; 16]>) {
        for m in meshes {
            for i in 0..m.instances.len() {
                let local = scene::placement(m, i);
                let t = match world {
                    None => local,
                    Some(w) => std::array::from_fn(|k| {
                        let (c, r) = (k / 4, k % 4);
                        (0..4).map(|j| w[j * 4 + r] * local[c * 4 + j]).sum()
                    }),
                };
                for f in &m.fragments {
                    for tri in &f.triangles {
                        if self.tris.len() >= MAX_TRIANGLES {
                            return;
                        }
                        let p = tri.map(|v| {
                            let p = f.positions[v as usize];
                            std::array::from_fn(|c| p[0] * t[c] + p[1] * t[4 + c] + p[2] * t[8 + c] + t[12 + c])
                        });
                        self.add_triangle(p, true);
                    }
                }
            }
        }
    }

    /// Add native collision triangles (level space). `PassThru` and water
    /// surfaces do not block; `NoStand` ones block but cannot be stood on.
    pub fn add_collision(&mut self, triangles: &[Triangle]) {
        for t in triangles {
            if self.tris.len() >= MAX_TRIANGLES {
                return;
            }
            if t.pat & (collision::PAT_PASS_THRU | collision::PAT_WATER) != 0 {
                continue;
            }
            self.add_triangle(t.vertices.map(|v| v.to_array()), t.pat & collision::PAT_NO_STAND == 0);
        }
    }

    /// Add climb edges in world space.
    pub fn add_edges(&mut self, edges: &[Edge]) {
        self.edges.extend_from_slice(edges);
    }

    fn add_triangle(&mut self, p: [[f32; 3]; 3], stand: bool) {
        self.add_solid(p);
        if !stand || !walkable(&p) {
            return;
        }
        let id = self.tris.len() as u32;
        self.tris.push(p);
        let (x0, x1) = (cell(p.iter().map(|v| v[0]).fold(f32::MAX, f32::min)), cell(p.iter().map(|v| v[0]).fold(f32::MIN, f32::max)));
        let (z0, z1) = (cell(p.iter().map(|v| v[2]).fold(f32::MAX, f32::min)), cell(p.iter().map(|v| v[2]).fold(f32::MIN, f32::max)));
        if (x1 - x0) > 64 || (z1 - z0) > 64 {
            return;
        }
        for x in x0..=x1 {
            for z in z0..=z1 {
                self.cells.entry((x, z)).or_default().push(id);
            }
        }
    }

    fn add_solid(&mut self, p: [[f32; 3]; 3]) {
        if self.solid.len() < MAX_SOLID {
            self.solid.push(p);
            self.nodes.clear();
        }
    }

    /// Add an entity's meshes as a movable body at column-major `world`.
    pub fn add_body(&mut self, id: u32, meshes: &[Mesh], world: [f32; 16], enabled: bool) {
        let mut local = Vec::new();
        for m in meshes {
            for i in 0..m.instances.len() {
                let t = scene::placement(m, i);
                for f in &m.fragments {
                    for tri in &f.triangles {
                        if local.len() >= MAX_BODY_TRIANGLES {
                            break;
                        }
                        local.push(tri.map(|v| {
                            let p = f.positions[v as usize];
                            std::array::from_fn(|c| p[0] * t[c] + p[1] * t[4 + c] + p[2] * t[8 + c] + t[12 + c])
                        }));
                    }
                }
            }
        }
        let stand = vec![true; local.len()];
        let mut body = Body { local, world: Vec::new(), stand, edges_local: Vec::new(), edges: Vec::new(), lo: [0.; 3], hi: [0.; 3], enabled, features: true, transform: [0.; 16] };
        body.place(world);
        self.bodies.insert(id, body);
    }

    /// Add an entity's native collision (art-group frame) and climb edges
    /// as a movable body at column-major `world`.
    pub fn add_body_collision(&mut self, id: u32, triangles: &[Triangle], edges: &[Edge], world: [f32; 16], enabled: bool) {
        let solid: Vec<&Triangle> = triangles.iter().filter(|t| t.pat & (collision::PAT_PASS_THRU | collision::PAT_WATER) == 0).take(MAX_BODY_TRIANGLES).collect();
        let local = solid.iter().map(|t| t.vertices.map(|v| v.to_array())).collect();
        let stand = solid.iter().map(|t| t.pat & collision::PAT_NO_STAND == 0).collect();
        let mut body = Body { local, world: Vec::new(), stand, edges_local: edges.to_vec(), edges: Vec::new(), lo: [0.; 3], hi: [0.; 3], enabled, features: true, transform: [0.; 16] };
        body.place(world);
        self.bodies.insert(id, body);
    }

    /// Climb edges within `radius` of `point` whose top face points up
    /// (the native query keeps `dot(up, n0) ≥ 0.707`, `sub_4F8318`), with
    /// the body carrying each (`None`: the level).
    pub fn edges_near(&self, point: Vec3, radius: f32) -> Vec<(usize, Edge, Option<u32>)> {
        let near = |e: &Edge| e.n0.y >= 0.707 && segment_distance(point, e.p0, e.p1) <= radius;
        let mut out: Vec<(usize, Edge, Option<u32>)> = self.edges.iter().enumerate().filter(|(_, e)| near(e)).map(|(i, e)| (i, *e, None)).collect();
        for (id, b) in self.bodies.iter().filter(|(_, b)| b.enabled && b.features) {
            out.extend(b.edges.iter().enumerate().filter(|(_, e)| near(e)).map(|(i, e)| (i, *e, Some(*id))));
        }
        out
    }

    /// Edge `index` of a body (`None`: the level), in world space.
    pub fn edge(&self, body: Option<u32>, index: usize) -> Option<Edge> {
        match body {
            Some(id) => self.bodies.get(&id).filter(|b| b.enabled).and_then(|b| b.edges.get(index)).copied(),
            None => self.edges.get(index).copied(),
        }
    }

    /// A body's current edges (world space).
    pub fn body_edges(&self, id: u32) -> &[Edge] {
        self.bodies.get(&id).map_or(&[], |b| b.edges.as_slice())
    }

    /// The level edges (world space).
    pub fn level_edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Move a body to column-major `world`.
    pub fn move_body(&mut self, id: u32, world: [f32; 16]) {
        if let Some(b) = self.bodies.get_mut(&id) {
            b.place(world);
        }
    }

    /// Whether a body's edges are offered (`disable-object-features`).
    pub fn enable_features(&mut self, id: u32, enabled: bool) {
        if let Some(b) = self.bodies.get_mut(&id) {
            b.features = enabled;
        }
    }

    pub fn enable_body(&mut self, id: u32, enabled: bool) {
        if let Some(b) = self.bodies.get_mut(&id) {
            b.enabled = enabled;
        }
    }

    fn live_bodies(&self) -> impl Iterator<Item = &Body> {
        self.bodies.values().filter(|b| b.enabled)
    }

    /// Build the ray hierarchy over the triangles added so far.
    pub fn prepare(&mut self) {
        if !self.nodes.is_empty() || self.solid.is_empty() {
            return;
        }
        let bounds = |t: &[[f32; 3]; 3]| -> ([f32; 3], [f32; 3]) {
            (std::array::from_fn(|k| t[0][k].min(t[1][k]).min(t[2][k])), std::array::from_fn(|k| t[0][k].max(t[1][k]).max(t[2][k])))
        };
        // Build over centroids, then reorder the triangles to match.
        let mut items: Vec<(u32, [f32; 3])> = self.solid.iter().enumerate().map(|(i, t)| (i as u32, std::array::from_fn(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.))).collect();
        let mut nodes = vec![Node { lo: [0.; 3], hi: [0.; 3], start: 0, count: 0 }];
        let mut stack = vec![(0usize, 0usize, items.len())];
        while let Some((node, start, end)) = stack.pop() {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for (i, _) in &items[start..end] {
                let (a, b) = bounds(&self.solid[*i as usize]);
                for k in 0..3 {
                    lo[k] = lo[k].min(a[k]);
                    hi[k] = hi[k].max(b[k]);
                }
            }
            nodes[node].lo = lo;
            nodes[node].hi = hi;
            if end - start <= LEAF {
                nodes[node].start = start as u32;
                nodes[node].count = (end - start) as u32;
                continue;
            }
            let axis = (0..3).max_by(|a, b| (hi[*a] - lo[*a]).total_cmp(&(hi[*b] - lo[*b]))).unwrap_or(0);
            let mid = (start + end) / 2;
            items[start..end].select_nth_unstable_by(mid - start, |a, b| a.1[axis].total_cmp(&b.1[axis]));
            let left = nodes.len();
            nodes.push(Node { lo: [0.; 3], hi: [0.; 3], start: 0, count: 0 });
            nodes.push(Node { lo: [0.; 3], hi: [0.; 3], start: 0, count: 0 });
            nodes[node].start = left as u32;
            stack.push((left, start, mid));
            stack.push((left + 1, mid, end));
        }
        self.solid = items.iter().map(|(i, _)| self.solid[*i as usize]).collect();
        self.nodes = nodes;
    }

    /// Fraction along `from → to` of the first triangle hit, if any
    /// (requires [`Ground::prepare`]).
    pub fn raycast(&self, from: [f32; 3], to: [f32; 3]) -> Option<f32> {
        let d = sub(to, from);
        let mut nearest: Option<f32> = None;
        for b in self.live_bodies() {
            let (mut t0, mut t1) = (0f32, 1f32);
            for k in 0..3 {
                let inv = 1. / d[k];
                let (a, c) = ((b.lo[k] - from[k]) * inv, (b.hi[k] - from[k]) * inv);
                t0 = t0.max(a.min(c));
                t1 = t1.min(a.max(c));
            }
            if t0 > t1 {
                continue;
            }
            for t in &b.world {
                if let Some(h) = ray_triangle(from, d, *t).filter(|h| *h <= 1.) {
                    nearest = Some(nearest.map_or(h, |n| n.min(h)));
                }
            }
        }
        let fixed = self.raycast_static(from, to);
        match (fixed, nearest) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn raycast_static(&self, from: [f32; 3], to: [f32; 3]) -> Option<f32> {
        if self.nodes.is_empty() {
            return None;
        }
        let d = sub(to, from);
        let inv: [f32; 3] = std::array::from_fn(|k| 1. / d[k]);
        let mut best = 1f32;
        let mut hit = false;
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n as usize];
            let (mut t0, mut t1) = (0f32, best);
            for k in 0..3 {
                let (a, b) = ((node.lo[k] - from[k]) * inv[k], (node.hi[k] - from[k]) * inv[k]);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
            if t0 > t1 {
                continue;
            }
            if node.count == 0 {
                stack.push(node.start);
                stack.push(node.start + 1);
                continue;
            }
            for t in &self.solid[node.start as usize..(node.start + node.count) as usize] {
                if let Some(h) = ray_triangle(from, d, *t) {
                    if h <= best {
                        best = h;
                        hit = true;
                    }
                }
            }
        }
        hit.then_some(best)
    }

    /// The highest walkable surface at `(x, z)` not above `top`.
    pub fn height(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        self.support(x, z, top).map(|(h, _)| h)
    }

    /// A body's column-major world transform.
    pub fn body_transform(&self, id: u32) -> Option<Mat4> {
        self.bodies.get(&id).map(|b| Mat4::from_cols_array(&b.transform))
    }

    /// [`Ground::height`] with the body that surface belongs to (`None`:
    /// the level).
    pub fn support(&self, x: f32, z: f32, top: f32) -> Option<(f32, Option<u32>)> {
        let mut best: Option<(f32, Option<u32>)> = None;
        let bodies = self.bodies.iter().filter(|(_, b)| b.enabled && (b.lo[0]..=b.hi[0]).contains(&x) && (b.lo[2]..=b.hi[2]).contains(&z)).flat_map(|(id, b)| b.world.iter().zip(&b.stand).filter(|(t, s)| **s && walkable(t)).map(move |(t, _)| (t, Some(*id))));
        let fixed = self.cells.get(&(cell(x), cell(z))).into_iter().flatten().map(|id| (&self.tris[*id as usize], None));
        for (&[a, b, c], owner) in fixed.chain(bodies) {
            let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
            if d.abs() < 1e-9 {
                continue;
            }
            let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
            let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
            let l3 = 1. - l1 - l2;
            if l1 < -1e-4 || l2 < -1e-4 || l3 < -1e-4 {
                continue;
            }
            let y = l1 * a[1] + l2 * b[1] + l3 * c[1];
            if y <= top && best.is_none_or(|(h, _)| y > h) {
                best = Some((y, owner));
            }
        }
        best
    }
}

fn walkable(p: &[[f32; 3]; 3]) -> bool {
    let (u, w) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    len > 1e-8 && (n[1] / len).abs() >= MIN_UP
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    let t = if d.length_squared() > 1e-12 { ((p - a).dot(d) / d.length_squared()).clamp(0., 1.) } else { 0. };
    p.distance(a + d * t)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Möller–Trumbore: the ray parameter of the hit, both faces.
fn ray_triangle(o: [f32; 3], d: [f32; 3], [a, b, c]: [[f32; 3]; 3]) -> Option<f32> {
    let cross = |u: [f32; 3], v: [f32; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let dot = |u: [f32; 3], v: [f32; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-9 {
        return None;
    }
    let s = sub(o, a);
    let u = dot(s, p) / det;
    if !(0. ..=1.).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    if v < 0. || u + v > 1. {
        return None;
    }
    let t = dot(e2, q) / det;
    (t > 0.).then_some(t)
}
