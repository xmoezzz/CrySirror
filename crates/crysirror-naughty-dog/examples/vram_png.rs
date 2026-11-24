// SPDX-License-Identifier: MPL-2.0
//! Write `VRAM_DESC` textures of a package as PNG files for inspection
//! (outside the repository): `vram_png PAK OUT_DIR [ID…]`.
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, texture::VramDesc};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, out) = (args.first().context("PAK")?, args.get(1).context("OUT_DIR")?);
    let ids: Vec<u32> = args[2..].iter().filter_map(|s| u32::from_str_radix(s, 16).ok()).collect();
    let source = MemoryAssetReader(std::fs::read(path)?.into());
    let directory = Directory::read(&source)?;
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let d = VramDesc::read(&source, item.payload_offset)?;
        if !ids.is_empty() && !ids.contains(&d.id) {
            continue;
        }
        let s = d.decode(&source, directory.texture_base)?;
        std::fs::write(format!("{out}/{:08x}.png", d.id), crysirror_image::encode_png(&s)?)?;
    }
    Ok(())
}
