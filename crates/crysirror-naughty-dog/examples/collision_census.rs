// SPDX-License-Identifier: MPL-2.0
//! Collision objects of packages: triangles, extents, PAT surface flags and
//! feature databases (climb edges and corners). Prop objects are placed in
//! the art-group frame by the package's bind pose.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{collision, mesh::Loaded, skeleton};
use glam::{Mat4, Vec3};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let p = Loaded::read(&MemoryAssetReader(std::fs::read(&path)?.into()), 1 << 30)?;
        let skel = skeleton::of_package(&p)?;
        let bind = |j: usize| skel.as_ref().and_then(|s| s.inverse_bind.get(j)).map(|m| Mat4::from_cols_array(m).inverse());
        let mut objects = collision::prop_objects(&p, &bind)?;
        objects.push(collision::level_object(&p)?);
        println!("{path}");
        for (i, o) in objects.iter().enumerate() {
            if o.triangles.is_empty() && o.features.edges.is_empty() && o.features.corners.is_empty() {
                continue;
            }
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for v in o.triangles.iter().flat_map(|t| t.vertices) {
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let no_grab = o.triangles.iter().filter(|t| t.pat & collision::PAT_NO_EDGE_GRAB != 0).count();
            let mut flags = std::collections::BTreeMap::new();
            for t in &o.triangles {
                for bit in 6..30 {
                    if t.pat >> bit & 1 != 0 {
                        *flags.entry(bit).or_insert(0usize) += 1;
                    }
                }
            }
            println!("  object {i} flag bits {flags:?}");
            // Surface types (bits 0–5) with how many of them are water (14)
            // and snow (17).
            let mut surfaces = std::collections::BTreeMap::new();
            for t in &o.triangles {
                let e = surfaces.entry(collision::surface_name(t.pat)).or_insert([0usize; 3]);
                e[0] += 1;
                e[1] += usize::from(t.pat & collision::PAT_WATER != 0);
                e[2] += usize::from(t.pat >> 17 & 1 != 0);
            }
            println!("  object {i} surfaces [all, water, snow] {surfaces:?}");
            let mut edge_bits = std::collections::BTreeMap::new();
            for e in &o.features.edges {
                for bit in 0..16 {
                    if e.flags >> bit & 1 != 0 {
                        *edge_bits.entry(bit).or_insert(0usize) += 1;
                    }
                }
            }
            println!("  object {i} edge flag bits {edge_bits:?}");
            // The wall face under each edge: the triangle holding both
            // endpoints whose normal is closest to horizontal; its normal
            // against the edge's n1 (+ the same way, − opposite).
            let (mut same, mut opposite) = (0usize, 0usize);
            for e in &o.features.edges {
                // The edge midpoint on the triangle within 3 cm.
                let mid = (e.p0 + e.p1) * 0.5;
                let on = |t: &collision::Triangle| {
                    let [a, b, c] = t.vertices;
                    let n = (b - a).cross(c - a);
                    if n.length_squared() < 1e-10 {
                        return false;
                    }
                    let n = n.normalize();
                    let q = mid - n * n.dot(mid - a);
                    let inside = [(a, b), (b, c), (c, a)].iter().all(|(u, v)| (*v - *u).cross(q - *u).dot(n) >= -0.03);
                    n.dot(mid - a).abs() < 0.03 && inside
                };
                let wall = o.triangles.iter().filter(|t| on(t)).map(|t| (t.vertices[1] - t.vertices[0]).cross(t.vertices[2] - t.vertices[0]).normalize_or_zero()).min_by(|a, b| a.y.abs().total_cmp(&b.y.abs()));
                match wall.map(|n| n.dot(e.n1)) {
                    Some(d) if d > 0.3 => same += 1,
                    Some(d) if d < -0.3 => opposite += 1,
                    _ => {}
                }
            }
            println!("  object {i} edge n1 vs wall face normal: same {same}, opposite {opposite}");
            println!("  object {i} joint {:?}: {} triangles ({no_grab} NoEdgeGrab), extents {lo:.2} .. {hi:.2}; {} edges, {} corners", o.joint, o.triangles.len(), o.features.edges.len(), o.features.corners.len());
        }
    }
    Ok(())
}
