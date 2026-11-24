// SPDX-License-Identifier: MPL-2.0
//! Print every `SPAWNER_GROUP` spawner of the given packages with its
//! process type and schema properties (raw).
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, package::Directory};

fn main() -> Result<()> {
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        let p = mesh::Loaded::read(&source, 256 << 20)?;
        let text = |at: usize| -> String {
            let b = p.data.get(at..).unwrap_or_default();
            let e = b.iter().take(256).position(|c| *c == 0).unwrap_or(0);
            String::from_utf8_lossy(&b[..e]).into_owned()
        };
        for item in directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP") {
            let g = item.payload_offset as usize;
            let n = p.u32(g + 8)? as usize;
            let table = p.ptr(g + 4)?;
            for i in 0..n {
                let s = p.ptr(table + 4 * i)?;
                let name = p.ptr(s + 0x20).map(text).unwrap_or_default();
                let art = p.ptr(s + 0x34).map(text).unwrap_or_default();
                print!("{path}: {name} type={:08x} art={art} flags={:08x} pos=({:.2},{:.2},{:.2})", p.u32(s + 0x30)?, p.u32(s + 0x38)?, p.f32(s)?, p.f32(s + 4)?, p.f32(s + 8)?);
                if let Ok(schema) = p.ptr(s + 0x44) {
                    let count = p.u32(schema + 4)? as usize;
                    if let Ok(props) = p.ptr(schema) {
                        for k in 0..count.min(64) {
                            let id = p.u32(props + 8 * k)?;
                            let Ok(v) = p.ptr(props + 8 * k + 4) else { continue };
                            let head = p.u32(v)?;
                            let data = p.ptr(v + 4).map(|d| format!("->{:08x} {:08x}", p.u32(d).unwrap_or(0), p.u32(d + 4).unwrap_or(0))).unwrap_or_else(|_| format!("={:08x}", p.u32(v + 4).unwrap_or(0)));
                            print!(" {id:08x}:{head:08x}{data}");
                        }
                    }
                }
                println!();
            }
        }
    }
    Ok(())
}
