// SPDX-License-Identifier: MPL-2.0
//! Placed bounds (metres) of each given package's prototype meshes, and the
//! nearest placed vertex to an optional point `--near X,Y,Z`.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let near: Option<[f32; 3]> = args.iter().position(|a| a == "--near").map(|i| {
        let v = args.remove(i + 1);
        args.remove(i);
        let p: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
        [p[0], p[1], p[2]]
    });
    for path in &args {
        let source = MemoryAssetReader(std::fs::read(path)?.into());
        let directory = Directory::read(&source)?;
        let meshes = scene::package_meshes(&source, &directory, 256 << 20)?;
        let mut best = (f32::MAX, [0f32; 3]);
        if let Some(q) = near {
            for m in &meshes {
                for i in 0..m.instances.len() {
                    let t = scene::placement(m, i);
                    for f in &m.fragments {
                        for p in &f.positions {
                            let p = p.map(|c| c);
                            let w: [f32; 3] = std::array::from_fn(|c| p[0] * t[c] + p[1] * t[4 + c] + p[2] * t[8 + c] + t[12 + c]);
                            let d = (0..3).map(|k| (w[k] - q[k]).powi(2)).sum::<f32>().sqrt();
                            if d < best.0 {
                                best = (d, w);
                            }
                        }
                    }
                }
            }
        }
        println!("{path}: {} meshes bounds {:?} nearest {:?}", meshes.len(), scene::placed_bounds(&meshes), near.map(|_| best));
    }
    Ok(())
}
