// SPDX-License-Identifier: MPL-2.0
//! Cluster LOD hierarchy for virtualized geometry (ADR 0026). Own
//! implementation of the published idea behind UE5 Nanite's DAG:
//!
//! 1. Split the mesh into clusters of at most `MAX_TRIS` triangles.
//! 2. Repeatedly group adjacent clusters (about 4 per group), merge each
//!    group, simplify it to half with its open boundary locked, and split the
//!    result into new clusters. Every cluster produced from one group shares
//!    that group's LOD error and bounding sphere.
//! 3. At runtime a cluster is drawn iff its own projected error is within the
//!    threshold and its parent group's is not. Errors grow and spheres enclose
//!    monotonically up the DAG, and siblings switch together across locked
//!    borders, so every cut is crack-free.
//!
//! Simplification is a half-edge-collapse quadric error metric on positions.
//! Vertices on open edges of the group (group borders, mesh borders, attribute
//! seams) are locked. Collapses that flip or degenerate a triangle are
//! rejected. No new vertices are created, so indices stay valid in the source
//! vertex buffer.
use glam::{DMat4, DVec3, DVec4, Vec3};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

pub const MAX_TRIS: usize = 128;
const GROUP: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}
impl Sphere {
    fn of_points(points: impl Iterator<Item = Vec3> + Clone) -> Sphere {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in points.clone() {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let center = (lo + hi) * 0.5;
        let radius = points.map(|p| (p - center).length()).fold(0., f32::max);
        Sphere { center, radius }
    }
    fn enclosing(spheres: &[Sphere]) -> Sphere {
        let mut s = spheres[0];
        for t in &spheres[1..] {
            let d = t.center - s.center;
            let dist = d.length();
            if dist + t.radius <= s.radius {
                continue;
            }
            if dist + s.radius <= t.radius {
                s = *t;
                continue;
            }
            let radius = (dist + s.radius + t.radius) * 0.5;
            s = Sphere { center: s.center + d / dist.max(1e-12) * (radius - s.radius), radius };
        }
        // Float slack keeps containment strict.
        s.radius *= 1.0 + 1e-5;
        s
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    /// Triangle list into the source vertex buffer.
    pub indices: Vec<u32>,
    /// Bounds of the cluster's own triangles (culling).
    pub bounds: Sphere,
    /// Error and sphere of the group this cluster was simplified in (0 for leaves).
    pub lod_error: f32,
    pub lod_bounds: Sphere,
    /// Error and sphere of the group that replaces this cluster (∞ for roots).
    pub parent_error: f32,
    pub parent_bounds: Sphere,
    pub level: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClusterDag {
    pub clusters: Vec<Cluster>,
    pub levels: u32,
}

/// Screen-independent projected error: error over the distance to the sphere.
pub fn projected(error: f32, sphere: Sphere, eye: Vec3) -> f32 {
    if !error.is_finite() {
        return f32::INFINITY;
    }
    error / ((eye - sphere.center).length() - sphere.radius).max(1e-6)
}

impl ClusterDag {
    /// The crack-free cut for `eye` at `threshold` (projected error units).
    pub fn select(&self, eye: Vec3, threshold: f32) -> Vec<usize> {
        (0..self.clusters.len())
            .filter(|&i| {
                let c = &self.clusters[i];
                projected(c.lod_error, c.lod_bounds, eye) <= threshold
                    && projected(c.parent_error, c.parent_bounds, eye) > threshold
            })
            .collect()
    }
    pub fn triangles(&self, selection: &[usize]) -> usize {
        selection.iter().map(|&i| self.clusters[i].indices.len() / 3).sum()
    }
}

fn edge(a: u32, b: u32) -> (u32, u32) {
    if a < b { (a, b) } else { (b, a) }
}

/// Greedy adjacency-growing split into clusters of at most `max_tris`.
pub fn split(positions: &[Vec3], indices: &[u32], max_tris: usize) -> Vec<Vec<u32>> {
    split_ids(positions, indices, max_tris)
        .into_iter()
        .map(|c| c.iter().flat_map(|&t| indices[3 * t..3 * t + 3].iter().copied()).collect())
        .collect()
}
/// `split` returning triangle ids.
fn split_ids(positions: &[Vec3], indices: &[u32], max_tris: usize) -> Vec<Vec<usize>> {
    let tris = indices.len() / 3;
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for t in 0..tris {
        for k in 0..3 {
            by_edge.entry(edge(indices[3 * t + k], indices[3 * t + (k + 1) % 3])).or_default().push(t);
        }
    }
    let centroid = |t: usize| (0..3).map(|k| positions[indices[3 * t + k] as usize]).sum::<Vec3>() / 3.;
    let mut used = vec![false; tris];
    let mut out = Vec::new();
    let mut next_seed = 0;
    while next_seed < tris {
        if used[next_seed] {
            next_seed += 1;
            continue;
        }
        let mut cluster = Vec::new();
        let mut center = centroid(next_seed);
        let mut frontier: Vec<usize> = vec![next_seed];
        used[next_seed] = true;
        while let Some(pick) = (!frontier.is_empty()).then(|| {
            // Closest frontier triangle to the running centroid keeps clusters compact.
            let (i, _) = frontier
                .iter()
                .enumerate()
                .map(|(i, &t)| (i, (centroid(t) - center).length_squared()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            frontier.swap_remove(i)
        }) {
            cluster.push(pick);
            let n = cluster.len() as f32;
            center = center + (centroid(pick) - center) / n;
            if cluster.len() == max_tris {
                break;
            }
            for k in 0..3 {
                for &t in &by_edge[&edge(indices[3 * pick + k], indices[3 * pick + (k + 1) % 3])] {
                    if !used[t] {
                        used[t] = true;
                        frontier.push(t);
                    }
                }
            }
        }
        // Unplaced frontier triangles return to the pool.
        for t in frontier {
            used[t] = false;
        }
        out.push(cluster);
    }
    // Greedy growth leaves small islands; fold each into the adjacent cluster
    // with the most shared edges while it has room (islands lock neighbours).
    let mut of = vec![0usize; tris];
    for (c, ts) in out.iter().enumerate() {
        for &t in ts {
            of[t] = c;
        }
    }
    for c in 0..out.len() {
        if out[c].is_empty() || out[c].len() > max_tris / 8 {
            continue;
        }
        let mut shared: BTreeMap<usize, usize> = BTreeMap::new();
        for &t in &out[c] {
            for k in 0..3 {
                for &u in &by_edge[&edge(indices[3 * t + k], indices[3 * t + (k + 1) % 3])] {
                    if of[u] != c {
                        *shared.entry(of[u]).or_default() += 1;
                    }
                }
            }
        }
        let target = shared
            .into_iter()
            .filter(|(d, _)| out[*d].len() + out[c].len() <= max_tris)
            .max_by_key(|(d, n)| (*n, std::cmp::Reverse(*d)))
            .map(|(d, _)| d);
        if let Some(d) = target {
            let moved = std::mem::take(&mut out[c]);
            for &t in &moved {
                of[t] = d;
            }
            out[d].extend(moved);
        }
    }
    out.retain(|c| !c.is_empty());
    out
}

#[derive(PartialEq)]
struct Candidate {
    cost: f64,
    from: u32,
    to: u32,
    stamp: (u32, u32),
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Candidate {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}

/// Half-edge-collapse QEM simplification toward `target_tris` with vertices on
/// open edges locked. Returns the new triangle list and the geometric error
/// (square root of the largest accepted quadric cost, in position units).
pub fn simplify(positions: &[Vec3], indices: &[u32], target_tris: usize) -> (Vec<u32>, f32) {
    let mut tris: Vec<[u32; 3]> = indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let mut alive = vec![true; tris.len()];
    let mut edge_count: HashMap<(u32, u32), u32> = HashMap::new();
    for t in &tris {
        for k in 0..3 {
            *edge_count.entry(edge(t[k], t[(k + 1) % 3])).or_default() += 1;
        }
    }
    let locked: HashSet<u32> = edge_count.iter().filter(|(_, &n)| n != 2).flat_map(|(e, _)| [e.0, e.1]).collect();
    let p = |v: u32| positions[v as usize].as_dvec3();
    let mut quadric: HashMap<u32, DMat4> = HashMap::new();
    let mut around: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, t) in tris.iter().enumerate() {
        let n = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        let area = n.length();
        let n = if area > 0. { n / area } else { DVec3::ZERO };
        let plane = DVec4::new(n.x, n.y, n.z, -n.dot(p(t[0])));
        let q = DMat4::from_cols(plane * plane.x, plane * plane.y, plane * plane.z, plane * plane.w) * area.max(1e-12);
        for &v in t {
            *quadric.entry(v).or_insert(DMat4::ZERO) += q;
            around.entry(v).or_default().push(i);
        }
    }
    let cost = |q: &DMat4, v: DVec3| {
        let h = DVec4::new(v.x, v.y, v.z, 1.);
        h.dot(*q * h).max(0.)
    };
    let mut version: HashMap<u32, u32> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let push = |heap: &mut BinaryHeap<Candidate>, version: &HashMap<u32, u32>, quadric: &HashMap<u32, DMat4>, a: u32, b: u32| {
        for (from, to) in [(a, b), (b, a)] {
            if locked.contains(&from) {
                continue;
            }
            let q = quadric[&from] + quadric[&to];
            heap.push(Candidate {
                cost: cost(&q, p(to)),
                from,
                to,
                stamp: (version.get(&from).copied().unwrap_or(0), version.get(&to).copied().unwrap_or(0)),
            });
        }
    };
    for e in edge_count.keys() {
        push(&mut heap, &version, &quadric, e.0, e.1);
    }
    let mut live = tris.len();
    let mut error = 0f64;
    while live > target_tris {
        let Some(c) = heap.pop() else { break };
        if c.stamp != (version.get(&c.from).copied().unwrap_or(0), version.get(&c.to).copied().unwrap_or(0)) {
            continue;
        }
        // Reject collapses that flip or degenerate a surviving triangle, or
        // that would create a non-manifold edge (link condition).
        let faces: Vec<usize> = around[&c.from].iter().copied().filter(|&f| alive[f]).collect();
        let shared: Vec<usize> = faces.iter().copied().filter(|&f| tris[f].contains(&c.to)).collect();
        if shared.is_empty() {
            continue;
        }
        let ring = |v: u32| -> HashSet<u32> {
            around[&v].iter().filter(|&&f| alive[f]).flat_map(|&f| tris[f]).filter(|&w| w != v).collect()
        };
        let common = ring(c.from).intersection(&ring(c.to)).count();
        if common != shared.len() {
            continue;
        }
        let mut ok = true;
        for &f in &faces {
            if shared.contains(&f) {
                continue;
            }
            let t = tris[f];
            let moved = t.map(|v| if v == c.from { c.to } else { v });
            let n0 = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            let n1 = (p(moved[1]) - p(moved[0])).cross(p(moved[2]) - p(moved[0]));
            if n1.length_squared() <= 1e-24 || n0.dot(n1) <= 0. {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        for &f in &shared {
            alive[f] = false;
            live -= 1;
        }
        let moved_faces: Vec<usize> = faces.iter().copied().filter(|f| !shared.contains(f)).collect();
        for &f in &moved_faces {
            for v in &mut tris[f] {
                if *v == c.from {
                    *v = c.to;
                }
            }
            around.get_mut(&c.to).unwrap().push(f);
        }
        error = error.max(c.cost);
        let q = quadric[&c.from] + quadric[&c.to];
        quadric.insert(c.to, q);
        *version.entry(c.from).or_default() += 1;
        *version.entry(c.to).or_default() += 1;
        let neighbours: HashSet<u32> = around[&c.to].iter().filter(|&&f| alive[f]).flat_map(|&f| tris[f]).filter(|&w| w != c.to).collect();
        for w in neighbours {
            *version.entry(w).or_default() += 0;
            push(&mut heap, &version, &quadric, c.to, w);
        }
    }
    let out = tris.iter().zip(&alive).filter(|(_, a)| **a).flat_map(|(t, _)| *t).collect();
    (out, error.sqrt() as f32)
}

/// Exact position identity for welding (-0.0 and +0.0 are the same point).
pub fn position_key(p: Vec3) -> [u32; 3] {
    (p + Vec3::ZERO).to_array().map(f32::to_bits)
}

/// Build the cluster DAG of one triangle mesh.
///
/// Topology is computed on position-welded indices, so attribute seams
/// (duplicated vertices with equal positions) do not lock simplification.
/// Triangles degenerate after welding are dropped (zero area). Leaf clusters
/// keep the original indices (exact seams at full detail); coarser clusters
/// reference one representative vertex per position. Both sides of every
/// border have identical positions, so cuts are crack-free in position space.
pub fn build(positions: &[Vec3], indices: &[u32]) -> ClusterDag {
    assert!(indices.len() % 3 == 0, "triangle list");
    let mut first: HashMap<[u32; 3], u32> = HashMap::new();
    let weld: Vec<u32> = positions
        .iter()
        .enumerate()
        .map(|(i, p)| *first.entry(position_key(*p)).or_insert(i as u32))
        .collect();
    let mut topo = Vec::with_capacity(indices.len());
    let mut original = Vec::with_capacity(indices.len());
    for t in indices.chunks_exact(3) {
        let w = [weld[t[0] as usize], weld[t[1] as usize], weld[t[2] as usize]];
        if w[0] != w[1] && w[1] != w[2] && w[0] != w[2] {
            topo.extend(w);
            original.extend_from_slice(t);
        }
    }
    build_welded(positions, &topo, &original)
}

fn build_welded(positions: &[Vec3], topo_indices: &[u32], original: &[u32]) -> ClusterDag {
    let sphere_of = |ix: &[u32]| Sphere::of_points(ix.iter().map(|&i| positions[i as usize]));
    // Welded topology per cluster (draw indices live in `Cluster::indices`).
    let mut topo: Vec<Vec<u32>> = Vec::new();
    let mut clusters: Vec<Cluster> = split_ids(positions, topo_indices, MAX_TRIS)
        .into_iter()
        .map(|ids| {
            topo.push(ids.iter().flat_map(|&t| topo_indices[3 * t..3 * t + 3].iter().copied()).collect());
            let ix: Vec<u32> = ids.iter().flat_map(|&t| original[3 * t..3 * t + 3].iter().copied()).collect();
            let s = sphere_of(&ix);
            Cluster {
                indices: ix,
                bounds: s,
                lod_error: 0.,
                lod_bounds: s,
                parent_error: f32::INFINITY,
                parent_bounds: s,
                level: 0,
            }
        })
        .collect();
    let mut current: Vec<usize> = (0..clusters.len()).collect();
    let mut level = 0;
    while current.len() > 1 {
        // Adjacency between clusters of this level by shared edges.
        let mut owner: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (slot, &c) in current.iter().enumerate() {
            for t in topo[c].chunks_exact(3) {
                for k in 0..3 {
                    owner.entry(edge(t[k], t[(k + 1) % 3])).or_default().push(slot);
                }
            }
        }
        let mut links: Vec<BTreeMap<usize, u32>> = vec![BTreeMap::new(); current.len()];
        for slots in owner.values() {
            for &a in slots {
                for &b in slots {
                    if a != b {
                        *links[a].entry(b).or_default() += 1;
                    }
                }
            }
        }
        let mut grouped = vec![false; current.len()];
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for seed in 0..current.len() {
            if grouped[seed] {
                continue;
            }
            grouped[seed] = true;
            let mut g = vec![seed];
            while g.len() < GROUP {
                let best = g
                    .iter()
                    .flat_map(|&m| links[m].iter())
                    .filter(|(n, _)| !grouped[**n])
                    .fold(BTreeMap::<usize, u32>::new(), |mut acc, (n, w)| {
                        *acc.entry(*n).or_default() += w;
                        acc
                    })
                    .into_iter()
                    .max_by_key(|(n, w)| (*w, std::cmp::Reverse(*n)));
                let Some((n, _)) = best else { break };
                grouped[n] = true;
                g.push(n);
            }
            groups.push(g);
        }
        // A singleton whose neighbours were all taken joins its best neighbouring group.
        let mut group_of = vec![usize::MAX; current.len()];
        for (gi, g) in groups.iter().enumerate() {
            for &m in g {
                group_of[m] = gi;
            }
        }
        for gi in 0..groups.len() {
            if groups[gi].len() != 1 {
                continue;
            }
            let m = groups[gi][0];
            let best = links[m]
                .iter()
                .filter(|(n, _)| group_of[**n] != gi && groups[group_of[**n]].len() > 1)
                .max_by_key(|(n, w)| (**w, std::cmp::Reverse(**n)))
                .map(|(n, _)| group_of[*n]);
            if let Some(target) = best {
                groups[gi].clear();
                groups[target].push(m);
                group_of[m] = target;
            }
        }
        groups.retain(|g| !g.is_empty());
        let mut next = Vec::new();
        let mut progressed = false;
        for g in groups {
            let members: Vec<usize> = g.iter().map(|&s| current[s]).collect();
            let merged: Vec<u32> = members.iter().flat_map(|&c| topo[c].iter().copied()).collect();
            let tris = merged.len() / 3;
            if members.len() == 1 && current.len() > 1 && tris <= MAX_TRIS / 2 {
                // Isolated small cluster: carry it to the next level unchanged.
                next.push(members[0]);
                continue;
            }
            let (simplified, err) = simplify(positions, &merged, tris / 2);
            if simplified.len() / 3 * 100 > tris * 85 || simplified.is_empty() {
                // No useful reduction (too much locked border): regroup these
                // with already simplified neighbours on the next pass. They
                // become roots only if a whole pass makes no progress.
                next.extend(&members);
                continue;
            }
            progressed = true;
            let child_bounds: Vec<Sphere> = members.iter().map(|&c| clusters[c].lod_bounds).collect();
            let bounds = Sphere::enclosing(&child_bounds);
            let error = members.iter().map(|&c| clusters[c].lod_error).fold(err, f32::max);
            for &c in &members {
                clusters[c].parent_error = error;
                clusters[c].parent_bounds = bounds;
            }
            for ix in split(positions, &simplified, MAX_TRIS) {
                let s = sphere_of(&ix);
                next.push(clusters.len());
                topo.push(ix.clone());
                clusters.push(Cluster {
                    indices: ix,
                    bounds: s,
                    lod_error: error,
                    lod_bounds: bounds,
                    parent_error: f32::INFINITY,
                    parent_bounds: bounds,
                    level: level + 1,
                });
            }
        }
        level += 1;
        if !progressed {
            break;
        }
        current = next;
    }
    ClusterDag { levels: level + 1, clusters }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed UV sphere with welded poles and seam (watertight).
    fn sphere(rings: u32, segments: u32) -> (Vec<Vec3>, Vec<u32>) {
        let mut p = vec![Vec3::Y];
        for r in 1..rings {
            let th = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..segments {
                let ph = std::f32::consts::TAU * s as f32 / segments as f32;
                p.push(Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()));
            }
        }
        p.push(-Vec3::Y);
        let south = p.len() as u32 - 1;
        let at = |r: u32, s: u32| 1 + (r - 1) * segments + s % segments;
        let mut ix = Vec::new();
        for s in 0..segments {
            ix.extend([0, at(1, s + 1), at(1, s)]);
            ix.extend([south, at(rings - 1, s), at(rings - 1, s + 1)]);
        }
        for r in 1..rings - 1 {
            for s in 0..segments {
                ix.extend([at(r, s), at(r, s + 1), at(r + 1, s)]);
                ix.extend([at(r, s + 1), at(r + 1, s + 1), at(r + 1, s)]);
            }
        }
        (p, ix)
    }
    fn edges(dag: &ClusterDag, sel: &[usize]) -> HashMap<(u32, u32), u32> {
        let mut e = HashMap::new();
        for &c in sel {
            for t in dag.clusters[c].indices.chunks_exact(3) {
                for k in 0..3 {
                    *e.entry(edge(t[k], t[(k + 1) % 3])).or_default() += 1;
                }
            }
        }
        e
    }

    #[test]
    fn uv_seams_do_not_block_simplification_and_stay_closed_by_position() {
        // Unwelded UV sphere: duplicated seam column and per-segment pole vertices.
        let (rings, segments) = (48u32, 96u32);
        let mut p = Vec::new();
        for r in 0..=rings {
            let th = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..=segments {
                let ph = std::f32::consts::TAU * s as f32 / segments as f32;
                // Exact pole/seam coordinates so duplicates compare equal.
                let (st, ct) = if r == 0 { (0., 1.) } else if r == rings { (0., -1.) } else { (th.sin(), th.cos()) };
                let (sp, cp) = if s == segments { (0., 1.) } else { (ph.sin(), ph.cos()) };
                p.push(Vec3::new(st * cp, ct, st * sp));
            }
        }
        let at = |r: u32, s: u32| r * (segments + 1) + s;
        let mut ix = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                ix.extend([at(r, s), at(r, s + 1), at(r + 1, s), at(r, s + 1), at(r + 1, s + 1), at(r + 1, s)]);
            }
        }
        let dag = build(&p, &ix);
        let coarse = dag.select(Vec3::new(0., 0., 50.), 0.05);
        assert!(dag.triangles(&coarse) * 10 < ix.len() / 3, "seams must not lock: {} tris", dag.triangles(&coarse));
        // Full detail keeps the original (seam-split) indices.
        let full = dag.select(Vec3::new(0., 0., 3.), 0.);
        let used: HashSet<u32> = full.iter().flat_map(|&c| dag.clusters[c].indices.iter().copied()).collect();
        assert!(used.contains(&at(10, segments)), "seam duplicate kept at full detail");
        for threshold in [0., 1e-3, 1e-2, 0.05] {
            let sel = dag.select(Vec3::new(2., 1., 3.), threshold);
            let key = |v: u32| position_key(p[v as usize]);
            let mut e: HashMap<([u32; 3], [u32; 3]), u32> = HashMap::new();
            for &c in &sel {
                for t in dag.clusters[c].indices.chunks_exact(3) {
                    for k in 0..3 {
                        let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
                        if a != b {
                            *e.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
                        }
                    }
                }
            }
            assert!(e.values().all(|n| n % 2 == 0), "position-space crack at {threshold}");
        }
    }

    #[test]
    fn split_respects_the_cluster_size_and_keeps_every_triangle() {
        let (p, ix) = sphere(24, 48);
        let parts = split(&p, &ix, MAX_TRIS);
        assert!(parts.iter().all(|c| !c.is_empty() && c.len() / 3 <= MAX_TRIS));
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>(), ix.len());
    }

    #[test]
    fn simplify_halves_a_closed_mesh_without_breaking_it() {
        let (p, ix) = sphere(16, 32);
        let (out, err) = simplify(&p, &ix, ix.len() / 6);
        assert!(out.len() / 3 <= ix.len() / 6 + 2, "{} tris", out.len() / 3);
        assert!(err > 0. && err < 0.2, "error {err}");
        let mut e: HashMap<(u32, u32), u32> = HashMap::new();
        for t in out.chunks_exact(3) {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
            for k in 0..3 {
                *e.entry(edge(t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        assert!(e.values().all(|&n| n == 2), "still closed and manifold");
    }

    #[test]
    fn open_borders_are_locked() {
        // A grid: border vertices must survive simplification unchanged.
        let n = 12u32;
        let p: Vec<Vec3> = (0..=n).flat_map(|y| (0..=n).map(move |x| Vec3::new(x as f32, (x * y) as f32 * 0.001, y as f32))).collect();
        let mut ix = Vec::new();
        let at = |x: u32, y: u32| y * (n + 1) + x;
        for y in 0..n {
            for x in 0..n {
                ix.extend([at(x, y), at(x, y + 1), at(x + 1, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]);
            }
        }
        let (out, _) = simplify(&p, &ix, 8);
        let border: HashSet<u32> = (0..=n).flat_map(|i| [at(i, 0), at(i, n), at(0, i), at(n, i)]).collect();
        let used: HashSet<u32> = out.iter().copied().collect();
        assert!(border.is_subset(&used), "border vertices kept");
        assert!(out.len() / 3 < ix.len() / 3, "interior simplified");
    }

    #[test]
    fn dag_cuts_are_crack_free_and_monotonic() {
        let (p, ix) = sphere(48, 96);
        let dag = build(&p, &ix);
        assert!(dag.levels >= 4, "levels {}", dag.levels);
        for c in &dag.clusters {
            assert!(c.parent_error >= c.lod_error, "error grows upward");
            if c.parent_error.is_finite() {
                let (a, b) = (c.lod_bounds, c.parent_bounds);
                assert!((a.center - b.center).length() + a.radius <= b.radius * 1.0001 + 1e-5, "parent sphere encloses");
            }
        }
        let full = dag.select(Vec3::new(0., 0., 3.), 0.);
        assert_eq!(dag.triangles(&full), ix.len() / 3, "zero threshold = full detail");
        let mut previous = usize::MAX;
        for threshold in [1e-4, 1e-3, 1e-2, 0.05, 1e9] {
            for eye in [Vec3::new(0., 0., 3.), Vec3::new(5., 2., -1.), Vec3::new(0., 40., 0.)] {
                let sel = dag.select(eye, threshold);
                assert!(!sel.is_empty());
                let e = edges(&dag, &sel);
                // A crack is an open (odd-count) edge. Even counts > 2 are
                // chords that two neighbouring groups created independently
                // between the same locked border vertices; the surface stays closed.
                let bad: Vec<_> = e.iter().filter(|(_, &n)| n % 2 == 1).collect();
                if !bad.is_empty() {
                    let owners: Vec<_> = sel.iter().filter(|&&c| dag.clusters[c].indices.chunks_exact(3).any(|t| (0..3).any(|k| edge(t[k], t[(k + 1) % 3]) == *bad[0].0))).map(|&c| (c, dag.clusters[c].level, dag.clusters[c].lod_error, dag.clusters[c].parent_error, projected(dag.clusters[c].lod_error, dag.clusters[c].lod_bounds, eye), projected(dag.clusters[c].parent_error, dag.clusters[c].parent_bounds, eye))).collect();
                    panic!("crack at threshold {threshold} eye {eye}: {} bad edges, first {:?}, owners {owners:?}", bad.len(), bad[0]);
                }
            }
            let sel = dag.select(Vec3::new(0., 0., 3.), threshold);
            let tris = dag.triangles(&sel);
            assert!(tris <= previous, "coarser with larger threshold");
            previous = tris;
        }
        assert!(previous * 8 < ix.len() / 3, "coarsest cut {previous} tris");
    }
}
