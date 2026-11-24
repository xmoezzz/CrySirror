// SPDX-License-Identifier: MPL-2.0
//! Baked probe statistics of one package: probe count and the per-vertex
//! irradiance of its meshes placed once (percentiles of luminance).
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh::Loaded, package::Directory, probes, scene};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        let loaded = Loaded::read(&source, 256 << 20)?;
        let mut field = probes::ProbeField::default();
        for item in directory.items.iter().filter(|i| i.kind == "LIGHT_TABLE_1") {
            field.add(probes::read(&loaded, item.payload_offset as usize)?);
        }
        let mut lum = Vec::new();
        for m in scene::meshes_of(&loaded, &directory)?.iter().filter(|m| m.instances.len() == 1) {
            for f in &m.fragments {
                for e in probes::bake(&field, &scene::placement(m, 0), &f.positions, &f.normals, &f.triangles) {
                    lum.push(0.2126 * e[0] + 0.7152 * e[1] + 0.0722 * e[2]);
                }
            }
        }
        lum.sort_by(f32::total_cmp);
        let q = |p: f64| lum.get(((lum.len() as f64 - 1.) * p) as usize).copied().unwrap_or(0.);
        println!("{path}: {} probes, {} vertices, luminance p5 {:.3} p50 {:.3} p95 {:.3} zero {:.1}%", field.len(), lum.len(), q(0.05), q(0.5), q(0.95), 100. * lum.iter().filter(|v| **v == 0.).count() as f64 / lum.len().max(1) as f64);
    }
    Ok(())
}
