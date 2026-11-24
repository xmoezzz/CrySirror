// SPDX-License-Identifier: MPL-2.0
//! Attribute 0x15 statistics per material group: per-channel mean, the
//! share of 0/255 values and distinct values, to tell blend masks from
//! baked vertex lighting.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene};
use std::collections::HashSet;

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for m in scene::package_meshes(&source, &directory, 1 << 30)? {
            for g in &m.groups {
                let masks: Vec<[f32; 3]> = m.fragments[g.fragments.clone()].iter().flat_map(|f| f.masks.iter().copied()).collect();
                if masks.is_empty() {
                    continue;
                }
                let n = masks.len() as f32;
                let mean: [f32; 3] = std::array::from_fn(|c| masks.iter().map(|v| v[c]).sum::<f32>() / n);
                let ends: [f32; 3] = std::array::from_fn(|c| masks.iter().filter(|v| v[c] == 0. || v[c] == 1.).count() as f32 / n);
                let distinct = masks.iter().map(|v| v.map(|x| (x * 255.) as u8)).collect::<HashSet<_>>().len();
                println!("{:60} n={:6} mean {:.2?} ends {:.2?} distinct {distinct}", g.material.name, masks.len(), mean, ends);
            }
        }
    }
    Ok(())
}
