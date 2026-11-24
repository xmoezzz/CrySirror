// SPDX-License-Identifier: MPL-2.0
//! Report materials whose base colour (first sampler that is not a
//! single-colour placeholder or lightmap) is missing or strongly blue,
//! with every sampler's ID, format, size and mean colour.
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
        let mut stats: HashMap<u32, Option<([u32; 4], bool, String)>> = HashMap::new();
        let mut stat = |id: u32| -> Option<([u32; 4], bool, String)> {
            stats
                .entry(id)
                .or_insert_with(|| {
                    let d = textures.get(&id)?;
                    let s = d.decode(&source, directory.texture_base).ok()?;
                    let n = (s.rgba.len() / 4).max(1) as u64;
                    let mean = std::array::from_fn(|c| (s.rgba.iter().skip(c).step_by(4).map(|v| u64::from(*v)).sum::<u64>() / n) as u32);
                    let flat = s.rgba.chunks_exact(4).all(|t| t == &s.rgba[..4]);
                    Some((mean, flat, format!("{:?} {}x{} {}", d.format, d.width, d.height, d.name)))
                })
                .clone()
        };
        let mut seen = BTreeMap::new();
        for m in scene::package_meshes(&source, &directory, 256 << 20)? {
            for g in &m.groups {
                let chosen = g.material.samplers.iter().copied().find(|id| stat(*id).is_some_and(|(_, flat, name)| !flat && !name.contains("indirect")));
                let bad = match chosen.and_then(|id| stat(id)) {
                    None => "none".to_owned(),
                    Some((c, _, _)) if c[2] > c[0] + 80 && c[2] > c[1] + 40 => format!("blue {c:?}"),
                    _ => continue,
                };
                let row: Vec<String> = g.material.samplers.iter().map(|id| format!("{id:08x}{:?}", stat(*id).map(|(c, f, n)| (c, f, n)))).collect();
                *seen.entry((g.material.name.clone(), bad, row.join(" | "))).or_insert(0) += g.fragments.len();
            }
        }
        for ((name, bad, row), n) in seen {
            println!("{path}: {name} [{bad}] fragments {n}: {row}");
        }
    }
    Ok(())
}
