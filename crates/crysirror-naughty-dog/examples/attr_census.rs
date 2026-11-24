// SPDX-License-Identifier: MPL-2.0
//! Census of fragment vertex attributes: `(id, format, depths)` with the
//! number of fragments carrying each, over the given packages' prototype
//! meshes.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh::Loaded, package::Directory};
use std::collections::BTreeMap;

fn main() -> Result<()> {
    let mut census: BTreeMap<(u8, u8, [u8; 4]), usize> = BTreeMap::new();
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        let p = Loaded::read(&source, 1 << 30)?;
        for item in directory.items.iter().filter(|i| i.kind == "PM_COLLECTION_2") {
            let payload = item.payload_offset as usize;
            let count = p.u32(payload)? as usize;
            if count == 0x1001 {
                continue;
            }
            for e in 0..count {
                let header = p.ptr(p.ptr(payload + 4 + 4 * e)?)?;
                let array = p.ptr(header + 0x14)?;
                for i in 0..usize::from(p.u16(header + 6)?) {
                    let f = p.ptr(array + 4 * i)?;
                    let desc = p.ptr(f + 0x28)?;
                    for a in 0..p.data[f + 0x20] as usize {
                        let r = &p.data[desc + 8 + 8 * a..desc + 16 + 8 * a];
                        *census.entry((r[0], r[1], [r[4], r[5], r[6], r[7]])).or_default() += 1;
                    }
                }
            }
        }
    }
    for ((id, format, depths), n) in census {
        println!("attr {id:#04x} format {format:#04x} depths {depths:?}: {n}");
    }
    Ok(())
}
