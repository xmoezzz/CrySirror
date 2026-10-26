// SPDX-License-Identifier: MPL-2.0
//! Terrain meshing, heightmap decoding and color-map baking (ADR 0014).
use crate::pbr::{mip_chain, MipImage, PbrMaterial, PbrPrimitive, PbrVertex};
use anyhow::{ensure, Context, Result};
use crysirror_scene::terrain::{procedural, TerrainSpec};
use glam::Vec3;

pub type Grid = (u32, u32, Vec<f32>);

/// Heights from the spec (procedural) or a grayscale PNG heightmap.
pub fn heights(spec: &TerrainSpec, heightmap: Option<&[u8]>) -> Result<Grid> {
    let Some(bytes) = heightmap else {
        return Ok(procedural(spec));
    };
    let img = image::load_from_memory(bytes)
        .context("decode heightmap")?
        .into_luma16();
    let (w, h) = img.dimensions();
    ensure!(
        (2..=4097).contains(&w) && (2..=4097).contains(&h),
        "heightmap dimensions"
    );
    Ok((
        w,
        h,
        img.into_raw()
            .into_iter()
            .map(|v| v as f32 / 65535. * spec.height)
            .collect(),
    ))
}

/// Smooth-normal grid mesh centred on the origin plus a baked color map
/// (sand / grass / rock / snow by height and slope) used as base color.
pub fn mesh(spec: &TerrainSpec, grid: &Grid) -> (PbrPrimitive, MipImage, PbrMaterial) {
    let (w, d, h) = grid;
    let (w, d) = (*w as usize, *d as usize);
    let (sx, sz) = (spec.size[0] / (w - 1) as f32, spec.size[1] / (d - 1) as f32);
    let at = |x: usize, z: usize| h[z.min(d - 1) * w + x.min(w - 1)];
    let mut vertices = Vec::with_capacity(w * d);
    let mut colors = Vec::with_capacity(w * d * 4);
    // Bands are relative to the actual height range of this terrain.
    let lo_h = h.iter().cloned().fold(f32::MAX, f32::min);
    let range = (h.iter().cloned().fold(f32::MIN, f32::max) - lo_h).max(1e-3);
    for z in 0..d {
        for x in 0..w {
            let hl = at(x.saturating_sub(1), z);
            let hr = at(x + 1, z);
            let hd = at(x, z.saturating_sub(1));
            let hu = at(x, z + 1);
            let n = Vec3::new((hl - hr) / (2. * sx), 1., (hd - hu) / (2. * sz)).normalize();
            let p = Vec3::new(
                x as f32 * sx - spec.size[0] / 2.,
                at(x, z),
                z as f32 * sz - spec.size[1] / 2.,
            );
            let t = Vec3::X - n * n.x;
            vertices.push(PbrVertex {
                position: p.to_array(),
                normal: n.to_array(),
                uv: [x as f32 / (w - 1) as f32, z as f32 / (d - 1) as f32],
                tangent: [t.x, t.y, t.z, 1.],
            });
            let rel = (p.y - lo_h) / range;
            let slope = 1. - n.y;
            let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ]
            };
            // Linear albedos in physically plausible ranges.
            let sand = [0.42, 0.34, 0.21];
            let grass = [0.07, 0.13, 0.035];
            let rock = [0.16, 0.15, 0.14];
            let snow = [0.78, 0.80, 0.84];
            let mut c = mix(sand, grass, ((rel - 0.12) / 0.08).clamp(0., 1.));
            c = mix(c, rock, ((slope - 0.18) / 0.15).clamp(0., 1.));
            c = mix(
                c,
                snow,
                ((rel - 0.88) / 0.06).clamp(0., 1.) * (1. - ((slope - 0.35) / 0.2).clamp(0., 1.)),
            );
            // Small per-texel variation breaks banding.
            let jitter = 0.92 + 0.16 * (((x * 73856093) ^ (z * 19349663)) % 1000) as f32 / 1000.;
            for k in c {
                let v = (k * jitter).clamp(0., 1.);
                let srgb = if v <= 0.003_130_8 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1. / 2.4) - 0.055
                };
                colors.push((srgb * 255. + 0.5) as u8);
            }
            colors.push(255);
        }
    }
    let mut indices = Vec::with_capacity((w - 1) * (d - 1) * 6);
    for z in 0..d - 1 {
        for x in 0..w - 1 {
            let i = (z * w + x) as u32;
            let (r, u) = (i + 1, i + w as u32);
            indices.extend_from_slice(&[i, u, r, r, u, u + 1]);
        }
    }
    let lo = h.iter().cloned().fold(f32::MAX, f32::min);
    let hi = h.iter().cloned().fold(f32::MIN, f32::max);
    let prim = PbrPrimitive {
        skin_attrs: None,
        morphs: Vec::new(),
        vertices,
        indices,
        material: 0,
        bounds: (
            [-spec.size[0] / 2., lo, -spec.size[1] / 2.],
            [spec.size[0] / 2., hi, spec.size[1] / 2.],
        ),
    };
    let image = MipImage {
        width: w as u32,
        height: d as u32,
        levels: mip_chain(w as u32, d as u32, colors, true),
        srgb: true,
    };
    let material = PbrMaterial {
        roughness_factor: 0.92,
        base_color_texture: Some(0),
        ..Default::default()
    };
    (prim, image, material)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mesh_has_upward_normals_and_matching_counts() {
        let s = TerrainSpec {
            heightmap: None,
            resolution: 33,
            size: [10., 10.],
            height: 2.,
            seed: 1,
            octaves: 3,
            frequency: 2.,
        };
        let g = heights(&s, None).unwrap();
        let (p, img, _) = mesh(&s, &g);
        assert_eq!(p.vertices.len(), 33 * 33);
        assert_eq!(p.indices.len(), 32 * 32 * 6);
        assert!(p.vertices.iter().all(|v| v.normal[1] > 0.));
        assert_eq!((img.width, img.levels[0].len()), (33, 33 * 33 * 4));
        // Counter-clockwise from above: the first triangle's normal points up.
        let v = |i: u32| Vec3::from_array(p.vertices[i as usize].position);
        let n = (v(p.indices[1]) - v(p.indices[0])).cross(v(p.indices[2]) - v(p.indices[0]));
        assert!(n.y > 0.);
    }
}
