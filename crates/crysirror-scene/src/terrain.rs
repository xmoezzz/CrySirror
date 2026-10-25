// SPDX-License-Identifier: MPL-2.0
//! Heightfield terrain (ADR 0014): the spec is saved; heights, meshes,
//! color maps and colliders are derived. Procedural terrain is fBm value
//! noise; image terrain reads a grayscale heightmap asset.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

fn res() -> u32 {
    129
}
fn size() -> [f32; 2] {
    [100., 100.]
}
fn height() -> f32 {
    12.
}
fn octaves() -> u32 {
    5
}
fn freq() -> f32 {
    3.
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainSpec {
    /// Grayscale PNG heightmap (8 or 16 bit); omitted = procedural noise.
    #[serde(default)]
    pub heightmap: Option<String>,
    /// Vertices per side for procedural terrain (heightmaps use their size).
    #[serde(default = "res")]
    pub resolution: u32,
    /// Extent in world units along X and Z.
    #[serde(default = "size")]
    pub size: [f32; 2],
    /// Height of the brightest heightmap value / noise amplitude.
    #[serde(default = "height")]
    pub height: f32,
    #[serde(default)]
    pub seed: u32,
    #[serde(default = "octaves")]
    pub octaves: u32,
    /// Base noise frequency across the whole terrain.
    #[serde(default = "freq")]
    pub frequency: f32,
}
impl TerrainSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (2..=2049).contains(&self.resolution)
                && self
                    .size
                    .iter()
                    .all(|s| s.is_finite() && *s > 0. && *s <= 100_000.)
                && self.height.is_finite()
                && self.height.abs() <= 10_000.
                && (1..=12).contains(&self.octaves)
                && self.frequency.is_finite()
                && self.frequency > 0.
                && self.frequency <= 1000.,
            "invalid terrain"
        );
        if let Some(h) = &self.heightmap {
            ensure!(
                !h.is_empty() && !h.starts_with('/') && !h.contains(".."),
                "invalid heightmap path"
            );
        }
        Ok(())
    }
}

fn hash(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (z as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.
}
fn value_noise(x: f32, z: f32, seed: u32) -> f32 {
    let (xi, zi) = (x.floor() as i32, z.floor() as i32);
    let (fx, fz) = (x - xi as f32, z - zi as f32);
    let s = |t: f32| t * t * (3. - 2. * t);
    let (u, v) = (s(fx), s(fz));
    let a = hash(xi, zi, seed) + (hash(xi + 1, zi, seed) - hash(xi, zi, seed)) * u;
    let b = hash(xi, zi + 1, seed) + (hash(xi + 1, zi + 1, seed) - hash(xi, zi + 1, seed)) * u;
    a + (b - a) * v
}

/// Height grid (row-major, z rows) for a procedural terrain.
pub fn procedural(spec: &TerrainSpec) -> (u32, u32, Vec<f32>) {
    let n = spec.resolution;
    let mut h = Vec::with_capacity((n * n) as usize);
    for z in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / (n - 1) as f32, z as f32 / (n - 1) as f32);
            let (mut amp, mut f, mut sum, mut norm) = (1f32, spec.frequency, 0f32, 0f32);
            for o in 0..spec.octaves {
                sum += amp * value_noise(u * f, v * f, spec.seed.wrapping_add(o * 101));
                norm += amp;
                amp *= 0.5;
                f *= 2.;
            }
            // Ridged shaping makes ranges more mountain-like.
            let t = sum / norm;
            h.push((t * t * (3. - 2. * t)) * spec.height);
        }
    }
    (n, n, h)
}

/// Bilinear height at local (x, z) relative to the terrain centre.
pub fn height_at(spec: &TerrainSpec, grid: &(u32, u32, Vec<f32>), x: f32, z: f32) -> Option<f32> {
    let (w, d, h) = grid;
    let u = (x / spec.size[0] + 0.5) * (*w - 1) as f32;
    let v = (z / spec.size[1] + 0.5) * (*d - 1) as f32;
    if !(0.0..=(*w - 1) as f32).contains(&u) || !(0.0..=(*d - 1) as f32).contains(&v) {
        return None;
    }
    let (x0, z0) = (u.floor() as u32, v.floor() as u32);
    let (x1, z1) = ((x0 + 1).min(w - 1), (z0 + 1).min(d - 1));
    let (fx, fz) = (u - x0 as f32, v - z0 as f32);
    let at = |x: u32, z: u32| h[(z * w + x) as usize];
    let a = at(x0, z0) + (at(x1, z0) - at(x0, z0)) * fx;
    let b = at(x0, z1) + (at(x1, z1) - at(x0, z1)) * fx;
    Some(a + (b - a) * fz)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn procedural_terrain_is_deterministic_bounded_and_seeded() {
        let s = TerrainSpec {
            heightmap: None,
            resolution: 65,
            size: [50., 50.],
            height: 10.,
            seed: 7,
            octaves: 5,
            frequency: 3.,
        };
        s.validate().unwrap();
        let a = procedural(&s);
        assert_eq!(a, procedural(&s));
        assert!(a.2.iter().all(|h| (0.0..=10.).contains(h)));
        let spread = a.2.iter().cloned().fold(f32::MIN, f32::max)
            - a.2.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 3., "has relief: {spread}");
        assert_ne!(
            a.2,
            procedural(&TerrainSpec {
                seed: 8,
                ..s.clone()
            })
            .2
        );
        assert_eq!(height_at(&s, &a, -25., -25.), Some(a.2[0]));
        assert_eq!(height_at(&s, &a, 30., 0.), None);
    }
}
