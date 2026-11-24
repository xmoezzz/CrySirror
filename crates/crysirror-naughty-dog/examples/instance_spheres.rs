// SPDX-License-Identifier: MPL-2.0
//! Per prototype mesh: grid bits, unit, instance count, instance 0
//! translation and culling sphere, and the decoded bounds (centre and
//! half extents in metres) to check placement. `instance_spheres PAK…`
use anyhow::Result;
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{package::Directory, scene};

fn main() -> Result<()> {
    let r1 = |v: &[f32]| v.iter().map(|x| (x * 10.).round() / 10.).collect::<Vec<_>>();
    for path in std::env::args().skip(1) {
        let source = MemoryAssetReader(std::fs::read(&path)?.into());
        let directory = Directory::read(&source)?;
        for (i, m) in scene::package_meshes(&source, &directory, 1 << 30)?.iter().enumerate() {
            let Some((lo, hi)) = scene::bounds(std::slice::from_ref(m)) else { continue };
            let c: Vec<f32> = (0..3).map(|k| (lo[k] + hi[k]) * 0.5).collect();
            let h: Vec<f32> = (0..3).map(|k| (hi[k] - lo[k]) * 0.5).collect();
            println!(
                "{i:3} inst {:3} t {:?} sphere {:?} centre {:?} half {:?} {}",
                m.instances.len(),
                m.instances.first().map(|t| r1(&t[12..15])),
                m.spheres.first().map(|s| r1(s)),
                r1(&c),
                r1(&h),
                m.groups.first().map_or("", |g| g.material.name.as_str())
            );
        }
    }
    Ok(())
}
