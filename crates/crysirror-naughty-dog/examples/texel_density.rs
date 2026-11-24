// SPDX-License-Identifier: MPL-2.0
//! Read-only check of the texture-coordinate unit: median texels per
//! native unit for every material group of one package.
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, package::Directory, texture::VramDesc};
use std::collections::HashMap;

fn main() -> Result<()> {
    let path = std::env::args_os().nth(1).context("usage: texel_density PACKAGE.pak")?;
    let source = MemoryAssetReader(std::fs::read(path)?.into());
    let directory = Directory::read(&source)?;
    let mut sizes = HashMap::new();
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let d = VramDesc::read(&source, item.payload_offset)?;
        sizes.insert(d.id, (d.width, d.height));
    }
    let item = directory.items.iter().find(|i| i.kind == "PM_COLLECTION_2").context("no prototype meshes")?;
    let loaded = mesh::Loaded::read(&source, 256 << 20)?;
    for m in mesh::prototype_meshes(&loaded, item.payload_offset as usize)? {
        for g in &m.groups {
            let Some(&(w, h)) = g.material.base_color().and_then(|id| sizes.get(&id)) else { continue };
            let mut d = Vec::new();
            for f in &m.fragments[g.fragments.clone()] {
                if f.uvs.len() != f.positions.len() {
                    continue;
                }
                for t in &f.triangles {
                    let (a, b) = (t[0] as usize, t[1] as usize);
                    let p = f.positions[a].iter().zip(f.positions[b]).map(|(x, y)| (x - y).powi(2)).sum::<f32>().sqrt();
                    let u = ((f.uvs[a][0] - f.uvs[b][0]) * w as f32).hypot((f.uvs[a][1] - f.uvs[b][1]) * h as f32);
                    if p > 1. {
                        d.push(u / p);
                    }
                }
            }
            if d.is_empty() {
                continue;
            }
            d.sort_by(f32::total_cmp);
            println!("{:<40} {w}x{h} median {:.3} texels/unit over {} edges", g.material.name.rsplit('/').next().unwrap_or(""), d[d.len() / 2], d.len());
        }
    }
    Ok(())
}
