// SPDX-License-Identifier: MPL-2.0
//! Build a package's model document as the player does and print each
//! material's render state, alpha mode, sidedness and base-colour image
//! (with its mean RGBA), to check base colours and blended and
//! double-sided materials.
//!
//! `model_materials PAK`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, package::Directory, scene};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("PAK")?;
    let source = MemoryAssetReader(std::fs::read(&path)?.into());
    let directory = Directory::read(&source)?;
    let loaded = mesh::Loaded::read(&source, 1 << 30)?;
    let meshes = scene::meshes_of(&loaded, &directory)?;
    let states: std::collections::HashMap<String, u16> = meshes.iter().flat_map(|m| m.groups.iter().map(|g| (g.material.name.clone(), g.material.state))).collect();
    let doc = scene::document_lit("model", &meshes, &source, &directory, 512, None, None)?;
    for m in &doc.materials {
        let mean = m.base_color_texture.and_then(|i| doc.images.get(i)).and_then(|(png, _)| crysirror_image::decode_standard(png).ok()).map(|s| {
            let n = (s.rgba.len() as u64 / 4).max(1);
            std::array::from_fn::<u64, 4, _>(|c| s.rgba.chunks_exact(4).map(|p| u64::from(p[c])).sum::<u64>() / n)
        });
        println!("{:70} state {:#06x} {} double-sided {} base image {:?} mean RGBA {:?}", m.name, states.get(&m.name).copied().unwrap_or(0), m.alpha_mode, m.double_sided, m.base_color_texture, mean);
    }
    Ok(())
}
