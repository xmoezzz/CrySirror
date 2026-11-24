// SPDX-License-Identifier: MPL-2.0
//! Texture-coordinate ranges per material group with the UV grid bits and
//! descriptor kinds, to check the UV unit. `uv_stats PAK…`
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for m in scene::package_meshes(&source, &directory, 1 << 30)? {
            for g in &m.groups {
                let uvs: Vec<[f32; 2]> = m.fragments[g.fragments.clone()].iter().flat_map(|f| f.uvs.iter().copied()).collect();
                if uvs.is_empty() {
                    continue;
                }
                let lo = [0, 1].map(|k| uvs.iter().map(|v| v[k]).fold(f32::MAX, f32::min));
                let hi = [0, 1].map(|k| uvs.iter().map(|v| v[k]).fold(f32::MIN, f32::max));
                println!("uv lo {:7.2?} hi {:7.2?} {}", lo, hi, g.material.name);
            }
        }
    }
    Ok(())
}
