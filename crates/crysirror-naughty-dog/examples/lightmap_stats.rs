// SPDX-License-Identifier: MPL-2.0
//! Read-only: channel statistics of a package's lightmap textures
//! (`indirect.*` VRAM descriptors).
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, texture::VramDesc};

fn main() -> Result<()> {
    let path = std::env::args_os().nth(1).context("usage: lightmap_stats PACKAGE.pak")?;
    let source = MemoryAssetReader(std::fs::read(path)?.into());
    let directory = Directory::read(&source)?;
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let d = VramDesc::read(&source, item.payload_offset)?;
        if !d.name.contains("indirect") {
            continue;
        }
        let img = d.decode(&source, directory.texture_base)?;
        let mut hist = [[0u64; 4]; 4];
        let mut sum = [0f64; 4];
        let n = (img.rgba.len() / 4) as f64;
        for px in img.rgba.chunks_exact(4) {
            for c in 0..4 {
                hist[c][px[c] as usize / 64] += 1;
                sum[c] += px[c] as f64;
            }
        }
        println!("{} {:?} {}x{} id {:08x}", d.name.rsplit('/').nth(1).unwrap_or(""), d.format, d.width, d.height, d.id);
        for c in 0..4 {
            println!("  ch{c} mean {:.1} quartile histogram {:?}", sum[c] / n, hist[c]);
        }
    }
    Ok(())
}
