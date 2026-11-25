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
            let mut edge_bits = std::collections::BTreeMap::new();
            for e in &o.features.edges {
                for bit in 0..16 {
                    if e.flags >> bit & 1 != 0 {
                        *edge_bits.entry(bit).or_insert(0usize) += 1;
                    }
                }
            }
            println!("  object {i} edge flag bits {edge_bits:?}");
            println!("  object {i} joint {:?}: {} triangles ({no_grab} NoEdgeGrab), extents {lo:.2} .. {hi:.2}; {} edges, {} corners", o.joint, o.triangles.len(), o.features.edges.len(), o.features.corners.len());
        }
    }
    Ok(())
}
