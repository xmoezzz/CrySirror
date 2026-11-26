// SPDX-License-Identifier: MPL-2.0
//! Print `particle.pak` materials: blend factors, soft-particle distance,
//! parameter bindings and, with `--code`, pass 0's fragment program.
//!
//! `particle_materials particle.pak [NAME-SUBSTRING] [--code]`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh::Loaded, package::Directory, particle, rsx};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let reader = MemoryAssetReader(std::fs::read(args.first().context("particle.pak")?)?.into());
    let directory = Directory::read(&reader)?;
    let p = Loaded::read(&reader, 1 << 30)?;
    let item = directory.items.iter().find(|i| i.kind == "MATERIAL_TABLE_1").context("no material table")?;
    let payload = item.payload_offset as usize;
    let filter = args.get(1).filter(|a| !a.starts_with("--"));
    let code = args.iter().any(|a| a == "--code");
    let materials = particle::materials(&p, payload)?;
    let count = p.u32(payload + 0xc)? as usize;
    let array = p.ptr(payload + 0x20)?;
    for i in 0..count {
        let Ok(m) = p.ptr(array + 4 * i) else { continue };
        let Some(material) = materials.get(&p.u32(m + 0x18)?) else { continue };
        if filter.is_some_and(|f| !material.name.contains(f.as_str())) {
            continue;
        }
        let data = p.ptr(p.ptr(m + 0x10)?).and_then(|pass| p.ptr(pass + 0x10))?;
        println!("{} blend {:#x}/{:#x} soft {:?}", material.name, material.blend.0, material.blend.1, material.depth_blend);
        println!("  bindings {:?}", rsx::binding_names(&p, data)?);
        println!("  parameters {:?}", rsx::pass_parameters(&p, data)?);
        if args.iter().any(|a| a == "--raw") {
            let table = p.ptr(data + 0x14)?;
            let n = (p.u32(data + 8)? & 0xffff) as usize;
            let names: Vec<String> = (0..n).map(|r| Ok(format!("{:#x}/{:#x}", p.u32(table + 16 * r + 4)?, p.u32(table + 16 * r + 12)?))).collect::<Result<_>>()?;
            println!("  table {table:#x} rows {names:?}");
        }
        if code {
            for ins in rsx::pass_program(&p, data)? {
                println!("    {ins:?}");
            }
        }
    }
    Ok(())
}
