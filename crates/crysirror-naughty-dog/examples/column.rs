// SPDX-License-Identifier: MPL-2.0
//! Surfaces crossing the vertical line through `X,Z` (metres) in the given
//! packages: height, mesh and material, top first. `column X,Z PAK…`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let xz: Vec<f32> = args.next().context("X,Z")?.split(',').map(|v| v.parse()).collect::<Result<_, _>>()?;
    let mut hits = Vec::new();
    for path in args {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for (mi, m) in scene::package_meshes(&source, &directory, 1 << 30)?.iter().enumerate() {
            for i in 0..m.instances.len() {
                // `IDENTITY=1` keeps identity matrices at the origin.
                let t = if std::env::var_os("IDENTITY").is_some() { m.instances[i] } else { scene::placement(m, i) };
                let w = |p: [f32; 3]| -> [f32; 3] {
                    let p = p.map(|c| c);
                    std::array::from_fn(|c| p[0] * t[c] + p[1] * t[4 + c] + p[2] * t[8 + c] + t[12 + c])
                };
                for g in &m.groups {
                    for f in &m.fragments[g.fragments.clone()] {
                        for tri in &f.triangles {
                            let [a, b, c] = tri.map(|k| w(f.positions[k as usize]));
                            // Barycentric test in XZ.
                            let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
                            if d.abs() < 1e-9 {
                                continue;
                            }
                            let l1 = ((b[2] - c[2]) * (xz[0] - c[0]) + (c[0] - b[0]) * (xz[1] - c[2])) / d;
                            let l2 = ((c[2] - a[2]) * (xz[0] - c[0]) + (a[0] - c[0]) * (xz[1] - c[2])) / d;
                            let l3 = 1. - l1 - l2;
                            if l1 >= 0. && l2 >= 0. && l3 >= 0. {
                                hits.push((l1 * a[1] + l2 * b[1] + l3 * c[1], format!("{} mesh {mi} inst {i} {}", path.rsplit('/').next().unwrap_or(""), g.material.name)));
                            }
                        }
                    }
                }
            }
        }
    }
    hits.sort_by(|a, b| b.0.total_cmp(&a.0));
    hits.dedup_by(|a, b| a.1 == b.1 && (a.0 - b.0).abs() < 0.5);
    for (y, what) in hits {
        println!("{y:8.2} {what}");
    }
    Ok(())
}
