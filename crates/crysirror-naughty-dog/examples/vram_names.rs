// SPDX-License-Identifier: MPL-2.0
//! Print every `VRAM_DESC` texture name of the given packages.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, texture::VramDesc};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
            let d = VramDesc::read(&source, item.payload_offset)?;
            println!("{:08x}\t{:?}\t{}x{}\t{}", d.id, d.format, d.width, d.height, d.name);
        }
    }
    Ok(())
}
