// SPDX-License-Identifier: MPL-2.0
//! Per material of the given packages: pass-0 sampler texture IDs with the
//! mean RGBA of each decoded texture (level 0), to identify texture roles.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene, texture::VramDesc};
use std::collections::{BTreeMap, HashMap};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        let mut textures = HashMap::new();
        for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
            let d = VramDesc::read(&source, item.payload_offset)?;
            textures.insert(d.id, d);
        }
        let mut means: HashMap<u32, String> = HashMap::new();
        let mut seen = BTreeMap::new();
        for m in scene::package_meshes(&source, &directory, 256 << 20)? {
            for g in &m.groups {
                let row: Vec<String> = g
                    .material
                    .samplers
                    .iter()
                    .map(|id| {
                        means
                            .entry(*id)
                            .or_insert_with(|| match textures.get(id).map(|d| d.decode(&source, directory.texture_base)) {
                                Some(Ok(img)) => {
                                    let n = (img.rgba.len() / 4).max(1) as f64;
                                    let mean: Vec<u32> = (0..4).map(|c| (img.rgba.iter().skip(c).step_by(4).map(|v| f64::from(*v)).sum::<f64>() / n) as u32).collect();
                                    format!("{id:08x}{mean:?}")
                                }
                                _ => format!("{id:08x}[?]"),
                            })
                            .clone()
                    })
                    .collect();
                *seen.entry((g.material.name.clone(), row.join(" "))).or_insert(0) += 1;
            }
        }
        for ((name, row), n) in seen {
            println!("{path}: {name} ×{n}: {row}");
        }
    }
    Ok(())
}
