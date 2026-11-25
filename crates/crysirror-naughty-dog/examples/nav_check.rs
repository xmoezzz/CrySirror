// SPDX-License-Identifier: MPL-2.0
//! Print a package's splines (length, closed/linear) and nav meshes, and
//! the nav path between two spawners when named.
//!
//! `nav_check PACKAGE.pak [FROM-SPAWNER TO-SPAWNER]`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, nav, package::Directory, scene};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source = MemoryAssetReader(std::fs::read(&args[0])?.into());
    let directory = Directory::read(&source)?;
    let p = mesh::Loaded::read(&source, 256 << 20)?;
    for s in nav::splines(&p, &directory)? {
        println!("spline {:08x} parent {:?} points {} closed {} linear {} length {:.3}", s.id, s.parent.map(|v| format!("{v:08x}")), s.points.len(), s.closed(), s.linear(), s.length());
    }
    let mut navigation = nav::Navigation::default();
    for m in nav::nav_meshes(&p, &directory)? {
        println!("nav mesh {} polys {} links {} origin {:?}", m.name, m.polys.len(), m.links.len(), m.origin);
        navigation.add(m);
    }
    if args.len() == 3 {
        let mut spawners = Vec::new();
        for item in directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP") {
            spawners.extend(scene::spawner_records(&p, item.payload_offset as usize)?);
        }
        let at = |n: &str| spawners.iter().find(|s| s.name == n).map(|s| glam::Vec3::from_array(s.position)).with_context(|| format!("no spawner {n}"));
        let (a, b) = (at(&args[1])?, at(&args[2])?);
        match navigation.path(a, b) {
            Some(path) => println!("path {:.2} m, {} points: {:?}", path.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>(), path.len(), path),
            None => println!("no path"),
        }
    }
    Ok(())
}
