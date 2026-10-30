// SPDX-License-Identifier: MPL-2.0
//! CPU R8 signed-distance atlas. Derived pixels/UVs only, no GPU objects/fonts.
use crate::{
    layout::{IconPlacement, Layout},
    TextSystem,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SdfOptions {
    pub page_size: u32,
    pub spread: u32,
    pub max_pages: u32,
}
impl Default for SdfOptions {
    fn default() -> Self {
        Self {
            page_size: 1024,
            spread: 8,
            max_pages: 4,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlyphKey {
    pub font: String,
    pub digest: String,
    pub glyph_id: u16,
    pub size_bits: u32,
    pub bold: bool,
}
#[derive(Clone, Debug)]
pub struct SdfPage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct SdfGlyph {
    pub page: usize,
    pub atlas_rect: [u32; 4],
    pub uv: [f32; 4],
    pub plane_bounds: [f32; 4],
    pub spread: u32,
}
#[derive(Clone, Debug)]
pub struct SdfPlacement {
    pub glyph: GlyphKey,
    pub rect: [f32; 4],
    pub color: [u8; 4],
    pub graphemes: Range<usize>,
    pub ruby: bool,
}
#[derive(Clone, Debug)]
pub struct SdfAtlas {
    pub pages: Vec<SdfPage>,
    pub glyphs: BTreeMap<GlyphKey, SdfGlyph>,
    pub placements: Vec<SdfPlacement>,
    pub icons: Vec<IconPlacement>,
}

// Exact separable squared Euclidean distance transform; lower envelope of
// parabolas. Finite sentinel avoids inf-inf for empty rows/columns.
fn distance_1d(f: &[f64]) -> Vec<f64> {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0.; n + 1];
    let mut k = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2. * (q - p) as f64);
            if s > z[k] || k == 0 {
                break;
            }
            k -= 1;
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    let mut out = vec![0.; n];
    k = 0;
    for (q, value) in out.iter_mut().enumerate() {
        while z[k + 1] < (q as f64) {
            k += 1;
        }
        let d = q as f64 - v[k] as f64;
        *value = d * d + f[v[k]];
    }
    out
}
fn distances(width: usize, height: usize, inside: &[bool], target: bool) -> Vec<f64> {
    let mut scratch = vec![0.; width * height];
    for y in 0..height {
        let f = (0..width)
            .map(|x| {
                if inside[y * width + x] == target {
                    0.
                } else {
                    1e20
                }
            })
            .collect::<Vec<_>>();
        scratch[y * width..(y + 1) * width].copy_from_slice(&distance_1d(&f));
    }
    let mut out = vec![0.; width * height];
    for x in 0..width {
        let f = (0..height)
            .map(|y| scratch[y * width + x])
            .collect::<Vec<_>>();
        let d = distance_1d(&f);
        for y in 0..height {
            out[y * width + x] = d[y];
        }
    }
    out
}
/// Alpha≥128 defines the contour. Output includes `spread` transparent padding;
/// R8 128 is the edge, >128 inside. Decode distance as (r/255−0.5)·2·spread.
pub fn from_alpha(width: u32, height: u32, alpha: &[u8], spread: u32) -> Result<SdfPage> {
    ensure!(
        width > 0
            && height > 0
            && width <= 2048
            && height <= 2048
            && spread > 0
            && spread <= 64
            && u64::from(width) * u64::from(height) == alpha.len() as u64,
        "SDF mask bounds"
    );
    let (w, h) = (
        (width + 2 * spread) as usize,
        (height + 2 * spread) as usize,
    );
    let mut inside = vec![false; w * h];
    for y in 0..height as usize {
        for x in 0..width as usize {
            inside[(y + spread as usize) * w + x + spread as usize] =
                alpha[y * width as usize + x] >= 128;
        }
    }
    let to_inside = distances(w, h, &inside, true);
    let to_outside = distances(w, h, &inside, false);
    let pixels = to_inside
        .iter()
        .zip(to_outside)
        .map(|(a, b)| {
            ((0.5 + (b.sqrt() - a.sqrt()) / (2. * spread as f64)).clamp(0., 1.) * 255.).round()
                as u8
        })
        .collect();
    Ok(SdfPage {
        width: w as u32,
        height: h as u32,
        pixels,
    })
}
impl TextSystem {
    /// Create a shelf-packed atlas for the given shaped layout. Glyph identity
    /// includes actual face digest, size and synthetic-bold policy. Glyphs share
    /// entries; icon placements are forwarded for a separate image atlas.
    pub fn build_sdf_atlas(&self, layout: &Layout, options: &SdfOptions) -> Result<SdfAtlas> {
        layout.validate()?;
        ensure!(
            options.page_size.is_power_of_two()
                && (64..=4096).contains(&options.page_size)
                && (1..=64).contains(&options.spread)
                && (1..=16).contains(&options.max_pages)
                && u64::from(options.page_size).pow(2) * u64::from(options.max_pages)
                    <= 64 * 1024 * 1024,
            "SDF atlas page/spread/byte limits"
        );
        let mut atlas = SdfAtlas {
            pages: Vec::new(),
            glyphs: BTreeMap::new(),
            placements: Vec::new(),
            icons: layout.icons.clone(),
        };
        let (mut x, mut y, mut row) = (1, 1, 0);
        for g in &layout.glyphs {
            let digest = self.font_digest(&g.font)?;
            let key = GlyphKey {
                font: g.font.clone(),
                digest: digest.clone(),
                glyph_id: g.glyph_id,
                size_bits: g.size.to_bits(),
                bold: g.bold,
            };
            if !atlas.glyphs.contains_key(&key) {
                let mut origin = g.clone();
                origin.position = [0., 0.];
                let m = self.rich_glyph_mask(&origin)?;
                if m.width == 0 || m.height == 0 {
                    continue;
                }
                let field = from_alpha(m.width as u32, m.height as u32, &m.alpha, options.spread)?;
                ensure!(
                    field.width + 2 <= options.page_size && field.height + 2 <= options.page_size,
                    "glyph does not fit SDF atlas page"
                );
                if x + field.width + 1 > options.page_size {
                    x = 1;
                    y += row + 1;
                    row = 0;
                }
                if atlas.pages.is_empty() || y + field.height + 1 > options.page_size {
                    ensure!(
                        atlas.pages.len() < options.max_pages as usize,
                        "SDF atlas page budget exhausted"
                    );
                    atlas.pages.push(SdfPage {
                        width: options.page_size,
                        height: options.page_size,
                        pixels: vec![0; options.page_size as usize * options.page_size as usize],
                    });
                    x = 1;
                    y = 1;
                    row = 0;
                }
                let page = atlas.pages.len() - 1;
                let pixels = &mut atlas.pages[page].pixels;
                for yy in 0..field.height as usize {
                    let dst = (y as usize + yy) * options.page_size as usize + x as usize;
                    pixels[dst..dst + field.width as usize].copy_from_slice(
                        &field.pixels[yy * field.width as usize..(yy + 1) * field.width as usize],
                    );
                }
                let d = options.page_size as f32;
                atlas.glyphs.insert(
                    key.clone(),
                    SdfGlyph {
                        page,
                        atlas_rect: [x, y, field.width, field.height],
                        uv: [
                            x as f32 / d,
                            y as f32 / d,
                            (x + field.width) as f32 / d,
                            (y + field.height) as f32 / d,
                        ],
                        plane_bounds: [
                            m.x as f32 - options.spread as f32,
                            m.y as f32 - options.spread as f32,
                            field.width as f32,
                            field.height as f32,
                        ],
                        spread: options.spread,
                    },
                );
                x += field.width + 1;
                row = row.max(field.height);
            }
            let entry = &atlas.glyphs[&key];
            let b = entry.plane_bounds;
            atlas.placements.push(SdfPlacement {
                glyph: key,
                rect: [g.position[0] + b[0], g.position[1] + b[1], b[2], b[3]],
                color: g.color,
                graphemes: g.graphemes.clone(),
                ruby: g.ruby,
            });
        }
        Ok(atlas)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_distance_field_has_inside_outside_and_padding() {
        let field = from_alpha(3, 3, &[255; 9], 2).unwrap();
        assert_eq!((field.width, field.height), (7, 7));
        assert!(field.pixels[3 * 7 + 3] > field.pixels[2 * 7 + 3]);
        assert!(field.pixels[2 * 7 + 3] > 128);
        assert!(field.pixels[7 + 3] < 128);
        assert_eq!(field.pixels[0], 0);
        let empty = from_alpha(1, 1, &[0], 1).unwrap();
        assert!(empty.pixels.iter().all(|&p| p == 0));
        assert!(from_alpha(u32::MAX, 1, &[], 2).is_err());
        assert_eq!(distance_1d(&[0., 1e20, 1e20]), [0., 1., 4.]);
    }
}
