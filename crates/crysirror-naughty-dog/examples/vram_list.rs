// SPDX-License-Identifier: MPL-2.0
//! List `VRAM_DESC` textures: ID, format, size and whether level 0 holds a
//! single colour (with that colour).
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, texture::VramDesc};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
            let d = VramDesc::read(&source, item.payload_offset)?;
            let flat = d.decode(&source, directory.texture_base).ok().filter(|s| s.rgba.chunks_exact(4).all(|t| t == &s.rgba[..4])).map(|s| s.rgba[..4].to_vec());
            println!("{:08x} {:?} {}x{} {:?}", d.id, d.format, d.width, d.height, flat);
        }
    }
    Ok(())
}
