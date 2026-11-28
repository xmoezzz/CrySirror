// SPDX-License-Identifier: MPL-2.0
//! Baked light probes of `LIGHT_TABLE_1`: the level's precomputed indirect
//! lighting, evaluated per vertex.
//!
//! The table holds the probe count at +0x18 and the probe array at +0x30.
//! A probe is 240 bytes: a position in metres, then two sets of nine
//! spherical-harmonic coefficients with interleaved RGB (2 × 27 floats),
//! then padding. On `ice-cave-temple` (635 probes) the probes lie on a 2 m
//! grid. Both sets are summed; which lighting each set holds (for example
//! sky and bounce) is not established. The coefficients are treated as
//! radiance in the standard real SH basis (y up) and convolved with the
//! cosine lobe; this interpretation and the resulting brightness are not
//! calibrated against the original renderer (SATD-nd-retail-runtime).
use crate::mesh::Loaded;
use anyhow::{Result, ensure};
use std::collections::HashMap;

const STRIDE: usize = 240;
const CELL: f32 = 2.;

#[derive(Clone, Debug)]
pub struct Probe {
    pub position: [f32; 3],
    /// Summed coefficients: `sh[k]` is the RGB of basis function `k`.
    pub sh: [[f32; 3]; 9],
}

#[derive(Default)]
pub struct ProbeField {
    probes: Vec<Probe>,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
    /// Mean coefficients, for points away from every probe.
    mean: [[f32; 3]; 9],
}

fn cell(p: [f32; 3]) -> (i32, i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32, (p[2] / CELL).floor() as i32)
}

/// The probes of a `LIGHT_TABLE_1` payload.
pub fn read(p: &Loaded, payload: usize) -> Result<Vec<Probe>> {
    let count = p.u32(payload + 0x18)? as usize;
    ensure!(count <= 1 << 16, "ND probe count");
    let Ok(array) = p.ptr(payload + 0x30) else { return Ok(Vec::new()) };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let at = array + STRIDE * i;
        let f = |k: usize| p.f32(at + 4 * k);
        let position = [f(0)?, f(1)?, f(2)?];
        let mut sh = [[0f32; 3]; 9];
        for set in 0..2 {
            for (k, c) in sh.iter_mut().enumerate() {
                for (ch, v) in c.iter_mut().enumerate() {
                    *v += f(3 + 27 * set + 3 * k + ch)?;
                }
            }
        }
        if position.iter().chain(sh.iter().flatten()).all(|v| v.is_finite()) {
            out.push(Probe { position, sh });
        }
    }
    Ok(out)
}

impl ProbeField {
    pub fn is_empty(&self) -> bool {
        self.probes.is_empty()
    }
    pub fn len(&self) -> usize {
        self.probes.len()
    }

    /// Mean irradiance over the six axis directions.
    pub fn mean_irradiance(&self) -> [f32; 3] {
        let dirs = [[1., 0., 0.], [-1., 0., 0.], [0., 1., 0.], [0., -1., 0.], [0., 0., 1.], [0., 0., -1.]];
        let e: Vec<[f32; 3]> = dirs.iter().map(|d| evaluate(&self.mean, *d)).collect();
        std::array::from_fn(|c| e.iter().map(|v| v[c]).sum::<f32>() / 6.)
    }

    pub fn add(&mut self, probes: impl IntoIterator<Item = Probe>) {
        for p in probes {
            self.cells.entry(cell(p.position)).or_default().push(self.probes.len() as u32);
            self.probes.push(p);
        }
        let n = self.probes.len().max(1) as f32;
        self.mean = std::array::from_fn(|k| std::array::from_fn(|ch| self.probes.iter().map(|p| p.sh[k][ch]).sum::<f32>() / n));
    }

    /// Cosine-convolved irradiance (linear RGB) at `p` for unit normal `n`,
    /// interpolated by inverse squared distance over the probes within two
    /// cells; farther points use the mean of all probes. `None` without
    /// probes.
    pub fn irradiance(&self, p: [f32; 3], n: [f32; 3]) -> Option<[f32; 3]> {
        self.coefficients(p).map(|sh| evaluate(&sh, n))
    }

    fn coefficients(&self, p: [f32; 3]) -> Option<[[f32; 3]; 9]> {
        if self.probes.is_empty() {
            return None;
        }
        let c = cell(p);
        for r in 1..=2i32 {
            let mut sum = [[0f32; 3]; 9];
            let mut wsum = 0f32;
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r && r > 1 {
                            continue;
                        }
                        for id in self.cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)).into_iter().flatten() {
                            let probe = &self.probes[*id as usize];
                            let d2: f32 = (0..3).map(|k| (probe.position[k] - p[k]).powi(2)).sum();
                            let w = 1. / (d2 + 0.25);
                            for (s, v) in sum.iter_mut().zip(&probe.sh) {
                                for ch in 0..3 {
                                    s[ch] += v[ch] * w;
                                }
                            }
                            wsum += w;
                        }
                    }
                }
            }
            if wsum > 0. {
                return Some(sum.map(|c| c.map(|v| v / wsum)));
            }
        }
        Some(self.mean)
    }

    /// Low-frequency incident radiance for a level without a sky image.
    /// A fixed camera uses its surrounding probes, a roaming camera the
    /// level mean. The host prefilters this image for specular reflections.
    pub fn reflection_environment(&self, fixed_eye: Option<[f32; 3]>) -> Option<crysirror_asset::pbr::HdrImage> {
        use std::f32::consts::{PI, TAU};
        if self.is_empty() {
            return None;
        }
        let sh = fixed_eye.and_then(|eye| self.coefficients(eye)).unwrap_or(self.mean);
        let (width, height) = (64, 32);
        let mut rgb = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            let theta = PI * (y as f32 + 0.5) / height as f32;
            for x in 0..width {
                let phi = TAU * ((x as f32 + 0.5) / width as f32 - 0.5);
                let weights = basis([theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()]);
                rgb.push(project(&sh, &weights));
            }
        }
        Some(crysirror_asset::pbr::HdrImage { width, height, rgb })
    }
}

/// Irradiance of radiance coefficients `sh` for normal `n` (Ramamoorthi
/// and Hanrahan: band weights π, 2π/3, π/4).
fn evaluate(sh: &[[f32; 3]; 9], n: [f32; 3]) -> [f32; 3] {
    use std::f32::consts::PI;
    let band = [PI, 2. * PI / 3., 2. * PI / 3., 2. * PI / 3., PI / 4., PI / 4., PI / 4., PI / 4., PI / 4.];
    let b = basis(n);
    project(sh, &std::array::from_fn(|k| b[k] * band[k]))
}

fn basis([x, y, z]: [f32; 3]) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * y,
        0.488_603 * z,
        0.488_603 * x,
        1.092_548 * x * y,
        1.092_548 * y * z,
        0.315_392 * (3. * z * z - 1.),
        1.092_548 * x * z,
        0.546_274 * (x * x - y * y),
    ]
}

fn project(sh: &[[f32; 3]; 9], weights: &[f32; 9]) -> [f32; 3] {
    std::array::from_fn(|ch| (0..9).map(|k| sh[k][ch] * weights[k]).sum::<f32>().max(0.))
}

/// Engine material of a baked primitive: the base-colour image and the
/// `baked_vertex_irradiance` effect, with an upper layer when the material
/// has one: its image in the emissive slot, and for a height-mask layer the
/// mask image in the occlusion slot.
pub fn baked_material(base_color: Option<usize>, layer: Option<&Layer>) -> crysirror_asset::pbr::PbrMaterial {
    let (slope_layer, mask_layer) = match layer {
        Some(Layer { mask: None, tint, .. }) => (Some(*tint), None),
        Some(Layer { mask: Some((_, softness)), tint, .. }) => (None, Some(crysirror_scene::effect::MaskLayer { tint: *tint, softness: *softness })),
        None => (None, None),
    };
    crysirror_asset::pbr::PbrMaterial {
        base_color_texture: base_color,
        emissive_texture: layer.and_then(|l| l.top),
        occlusion_texture: layer.and_then(|l| l.mask).and_then(|m| m.0),
        metallic_factor: 0.,
        roughness_factor: 1.,
        effect: Some(crysirror_scene::effect::ShaderEffect::BakedVertexIrradiance { intensity: 1., slope_layer, mask_layer }),
        ..Default::default()
    }
}

/// An upper albedo layer: its image and tint, and for a height-mask blend
/// the mask image and softness (else the layer follows the slope).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub top: Option<usize>,
    pub tint: [f32; 3],
    pub mask: Option<(Option<usize>, f32)>,
}

/// Scale of the probe irradiance in the renderer, calibrated with the
/// render-settings exposure against the original title screen.
pub const INTENSITY: f32 = 2.;

/// Per-vertex irradiance (`_AUX`: RGB, 1) of a fragment placed by the
/// row-major metre matrix `m`. Vertices without
/// stored normals use area-weighted triangle normals.
pub fn bake(field: &ProbeField, m: &[f32; 16], positions: &[[f32; 3]], normals: &[[f32; 3]], triangles: &[[u16; 3]]) -> Vec<[f32; 4]> {
    let normals: Vec<[f32; 3]> = if normals.len() == positions.len() {
        normals.to_vec()
    } else {
        let mut acc = vec![[0f32; 3]; positions.len()];
        for t in triangles {
            let [a, b, c] = t.map(|i| positions[i as usize]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            for i in t {
                for k in 0..3 {
                    acc[*i as usize][k] += n[k];
                }
            }
        }
        acc
    };
    positions
        .iter()
        .zip(&normals)
        .map(|(p, n)| {
            let w: [f32; 3] = std::array::from_fn(|c| p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c]);
            let wn: [f32; 3] = std::array::from_fn(|c| n[0] * m[c] + n[1] * m[4 + c] + n[2] * m[8 + c]);
            let len = (wn[0] * wn[0] + wn[1] * wn[1] + wn[2] * wn[2]).sqrt();
            let wn = if len > 1e-8 { wn.map(|v| v / len) } else { [0., 1., 0.] };
            let e = field.irradiance(w, wn).unwrap_or([0.; 3]);
            [e[0], e[1], e[2], 1.]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indoor_reflection_uses_local_unconvolved_directional_colour() {
        let mut field = ProbeField::default();
        assert!(field.reflection_environment(None).is_none());
        let mut warm = [[0.; 3]; 9];
        warm[0] = [2., 1., 0.];
        warm[1] = [0.5, 0., 0.];
        let mut cold = [[0.; 3]; 9];
        cold[0] = [0., 0., 8.];
        field.add([Probe { position: [0.; 3], sh: warm }, Probe { position: [100.; 3], sh: cold }]);
        let image = field.reflection_environment(Some([0.; 3])).unwrap();
        let lower = image.rgb[(image.height as usize - 1) * image.width as usize];
        assert!(image.rgb[0][0] > lower[0] && lower[0] > 0.);
        assert!(image.rgb.iter().all(|c| (c[1] - 0.282095).abs() < 1e-5 && c[2] == 0.));
        assert!(field.reflection_environment(None).unwrap().rgb[0][2] > 1.);
    }

    #[test]
    fn constant_probe_gives_constant_irradiance() {
        let mut f = ProbeField::default();
        let mut sh = [[0.; 3]; 9];
        sh[0] = [1., 0.5, 0.25];
        f.add([Probe { position: [0.; 3], sh }]);
        let e = f.irradiance([0.5, 0.5, 0.5], [0., 1., 0.]).unwrap();
        assert!((e[0] - std::f32::consts::PI * 0.282_095).abs() < 1e-4);
        assert!((e[1] * 2. - e[0]).abs() < 1e-4);
        // Away from the probes the mean applies.
        assert_eq!(f.irradiance([100., 0., 0.], [0., 1., 0.]), Some(e));
        assert!(ProbeField::default().irradiance([0.; 3], [0., 1., 0.]).is_none());
    }
}
