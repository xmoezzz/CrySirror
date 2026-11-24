// SPDX-License-Identifier: MPL-2.0
//! World placement of the spawners of the given packages whose names
//! contain any filter word: `spawner_world PAK… -- WORD…`.
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, package::Directory, scene};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (paths, words) = (&args[..split], &args[(split + 1).min(args.len())..]);
    for path in paths {
        let source = MemoryAssetReader(std::fs::read(path)?.into());
        let directory = Directory::read(&source)?;
        let p = mesh::Loaded::read(&source, 256 << 20)?;
        for item in directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP") {
            for s in scene::spawner_records(&p, item.payload_offset as usize)? {
                if words.is_empty() || words.iter().any(|w| s.name.contains(w.as_str())) {
                    println!("{} art={:?} pos={:?} rot={:?}", s.name, s.art_group, s.position.map(|v| (v * 100.).round() / 100.), s.rotation.map(|v| (v * 1000.).round() / 1000.));
                }
            }
        }
    }
    Ok(())
}
