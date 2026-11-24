// SPDX-License-Identifier: MPL-2.0
//! Walkable surfaces of the placed level meshes for keeping the player on
//! the ground: upward-facing triangles bucketed in a horizontal grid.
//! Native collision (Havok data in the level packages) is not used; this
//! is the viewer's ground model (SATD-nd-retail-runtime).
use crate::{mesh::Mesh, scene};
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
}

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
                        self.add_solid(p);
                        let (u, w) = (sub(p[1], p[0]), sub(p[2], p[0]));
                        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                        if !(len > 1e-8) || (n[1] / len).abs() < MIN_UP {
                            continue;
                        }
                        let id = self.tris.len() as u32;
                        self.tris.push(p);
                        let (x0, x1) = (cell(p.iter().map(|v| v[0]).fold(f32::MAX, f32::min)), cell(p.iter().map(|v| v[0]).fold(f32::MIN, f32::max)));
                        let (z0, z1) = (cell(p.iter().map(|v| v[2]).fold(f32::MAX, f32::min)), cell(p.iter().map(|v| v[2]).fold(f32::MIN, f32::max)));
                        if (x1 - x0) > 64 || (z1 - z0) > 64 {
                            continue;
                        }
                        for x in x0..=x1 {
                            for z in z0..=z1 {
                                self.cells.entry((x, z)).or_default().push(id);
                            }
                        }
                    }
                }
            }
        }
    }

    fn add_solid(&mut self, p: [[f32; 3]; 3]) {
        if self.solid.len() < MAX_SOLID {
            self.solid.push(p);
            self.nodes.clear();
        }
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
        let mut best: Option<f32> = None;
        for id in self.cells.get(&(cell(x), cell(z))).into_iter().flatten() {
            let [a, b, c] = self.tris[*id as usize];
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
            if y <= top && best.is_none_or(|h| y > h) {
                best = Some(y);
            }
        }
        best
    }
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
