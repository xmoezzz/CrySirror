// SPDX-License-Identifier: MPL-2.0
//! Read-only placement check: cast a ray grid from the first package's
//! start camera through the placed triangles of all given packages and
//! report hit fraction and median distance.
use anyhow::{Context, Result};
use crysirror_core::service::MemoryAssetReader;
use crysirror_naughty_dog::{mesh, package::Directory, scene};

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut camera = None;
    let mut tris: Vec<[[f32; 3]; 3]> = Vec::new();
    for path in &args {
        let source = MemoryAssetReader(std::fs::read(path)?.into());
        let directory = Directory::read(&source)?;
        if camera.is_none() {
            if let Some(item) = directory.items.iter().find(|i| i.kind == "CAMERA_TABLE_1") {
                let loaded = mesh::Loaded::read(&source, 256 << 20)?;
                camera = scene::start_camera(&loaded, item.payload_offset as usize)?;
            }
        }
        for m in scene::package_meshes(&source, &directory, 256 << 20)? {
            for (i, inst) in m.instances.iter().enumerate() {
                let t = scene::placement(&m, i);
                for f in &m.fragments {
                    for tri in &f.triangles {
                        tris.push(tri.map(|v| {
                            let p = f.positions[v as usize].map(|c| c);
                            std::array::from_fn(|c| p[0] * t[c] + p[1] * t[4 + c] + p[2] * t[8 + c] + t[12 + c])
                        }));
                    }
                }
            }
        }
    }
    let cam = camera.context("no start camera")?;
    let f = cam.forward;
    let r = [f[1] * cam.up[2] - f[2] * cam.up[1], f[2] * cam.up[0] - f[0] * cam.up[2], f[0] * cam.up[1] - f[1] * cam.up[0]];
    let (tx, ty) = ((cam.fov[0] / 2.).tan(), (cam.fov[1] / 2.).tan() * 9. / 16.);
    let mut dists = Vec::new();
    let (cols, rows) = (32, 18);
    for j in 0..rows {
        for i in 0..cols {
            let (u, v) = ((i as f32 + 0.5) / cols as f32 * 2. - 1., 1. - (j as f32 + 0.5) / rows as f32 * 2.);
            let d: [f32; 3] = std::array::from_fn(|k| f[k] + r[k] * u * tx + cam.up[k] * v * ty);
            let mut best = f32::INFINITY;
            for t in &tris {
                if let Some(h) = hit(cam.position, d, t) {
                    best = best.min(h);
                }
            }
            if best.is_finite() {
                let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                dists.push(best * n);
            }
        }
    }
    dists.sort_by(f32::total_cmp);
    println!("{} triangles; {} of {} rays hit; median distance {:.1} m", tris.len(), dists.len(), cols * rows, dists.get(dists.len() / 2).copied().unwrap_or(f32::NAN));
    Ok(())
}

fn hit(o: [f32; 3], d: [f32; 3], t: &[[f32; 3]; 3]) -> Option<f32> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-9 {
        return None;
    }
    let s = sub(o, t[0]);
    let u = dot(s, p) / det;
    if !(0. ..=1.).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    if v < 0. || u + v > 1. {
        return None;
    }
    let t = dot(e2, q) / det;
    (t > 1e-4).then_some(t)
}
