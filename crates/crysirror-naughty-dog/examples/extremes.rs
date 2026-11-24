// SPDX-License-Identifier: MPL-2.0
//! Art-group vertex extremes in metres after placement (lowest, highest,
//! most ±X), to compare a model's bind space with a skeleton.
//! `extremes PAK`
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh::Loaded, package::Directory, scene};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).context("PAK")?;
    let source = MemoryAssetReader(std::fs::read(&path)?.into());
    let directory = Directory::read(&source)?;
    let loaded = Loaded::read(&source, 1 << 30)?;
    let mut meshes = scene::meshes_of(&loaded, &directory)?;
    let vis = scene::vis_sphere(&loaded, &directory);
    scene::place_art_group(&mut meshes);
    let pts: Vec<[f32; 3]> = meshes.iter().flat_map(|m| m.fragments.iter().flat_map(move |f| f.positions.iter().map(move |p| p.map(|c| c)))).collect();
    let pick = |k: usize, max: bool| pts.iter().copied().fold(None::<[f32; 3]>, |a, p| match a { Some(a) if (p[k] > a[k]) != max || p[k] == a[k] => Some(a), _ => Some(p) });
    println!("vis_sphere {vis:?}");
    for (label, k, max) in [("min y", 1, false), ("max y", 1, true), ("min x", 0, false), ("max x", 0, true), ("min z", 2, false), ("max z", 2, true)] {
        println!("{label}: {:?}", pick(k, max).map(|p| p.map(|v| (v * 1000.).round() / 1000.)));
    }
    Ok(())
}
