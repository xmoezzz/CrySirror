// SPDX-License-Identifier: MPL-2.0
//! List a package's items (kind, name, payload offset). With
//! `--dump KIND [WORDS]`, print that item's relocated payload words,
//! marking pointers and the strings they reach (`--at HEX` dumps from
//! that relocated offset instead).
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh::Loaded, package::Directory};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.iter().position(|a| a == "--dump").map(|i| {
        let kind = args.remove(i + 1);
        args.remove(i);
        kind
    });
    let words: usize = args.iter().position(|a| a == "--words").map(|i| { let n = args.remove(i + 1); args.remove(i); n.parse().unwrap_or(64) }).unwrap_or(64);
    let at: Option<usize> = args.iter().position(|a| a == "--at").map(|i| { let n = args.remove(i + 1); args.remove(i); usize::from_str_radix(n.trim_start_matches("0x"), 16).unwrap_or(0) });
    for path in &args {
        let source = MemoryAssetReader(std::fs::read(path)?.into());
        let directory = Directory::read(&source)?;
        let loaded = dump.as_ref().map(|_| Loaded::read(&source, 256 << 20)).transpose()?;
        for i in &directory.items {
            println!("{path}\t{}\t{}\t{:#x}", i.kind, i.name, i.payload_offset);
            let (Some(kind), Some(p)) = (&dump, &loaded) else { continue };
            if &i.kind != kind {
                continue;
            }
            let at = at.unwrap_or(i.payload_offset as usize);
            for k in 0..words {
                let o = at + 4 * k;
                let Ok(v) = p.u32(o) else { break };
                let s = match p.ptr(o) {
                    Ok(t) => {
                        let b = p.data.get(t..).unwrap_or_default();
                        let e = b.iter().take(64).position(|c| *c == 0).unwrap_or(0);
                        let text = String::from_utf8_lossy(&b[..e]);
                        format!("P -> {t:#x} {}", if e > 2 && text.chars().all(|c| c.is_ascii_graphic()) { text.into_owned() } else { String::new() })
                    }
                    Err(_) => format!("{}", f32::from_bits(v)),
                };
                println!("  +{:#05x} {v:08x} {s}", 4 * k);
            }
        }
    }
    Ok(())
}
