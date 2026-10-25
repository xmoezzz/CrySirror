// SPDX-License-Identifier: MPL-2.0
//! CPU rich glyph rasterization with straight sRGB RGBA output.
use crate::{
    layout::{GlyphPlacement, Layout},
    TextSystem,
};
use ab_glyph::{point, Font, Glyph, GlyphId, PxScale};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outline {
    pub width: f32,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    pub offset: [f32; 2],
    pub blur_radius: u32,
    pub color: [u8; 4],
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RasterOptions {
    pub outline: Option<Outline>,
    pub shadow: Option<Shadow>,
    pub visible_graphemes: Option<usize>,
}
impl RasterOptions {
    pub fn validate(&self) -> Result<()> {
        if let Some(o) = &self.outline {
            ensure!(
                o.width.is_finite() && (0.0..=16.).contains(&o.width),
                "text outline width"
            );
        }
        if let Some(s) = &self.shadow {
            ensure!(
                s.offset.iter().all(|n| n.is_finite() && n.abs() <= 512.) && s.blur_radius <= 32,
                "text shadow bounds"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct IconBitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug)]
pub(crate) struct GlyphMask {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
}

fn linear(c: u8) -> f32 {
    let c = c as f32 / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}
fn encoded(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    ((if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    }) * 255.)
        .round() as u8
}
fn blend(p: &mut [f32; 4], color: [u8; 4], coverage: f32) {
    let a = coverage.clamp(0., 1.) * color[3] as f32 / 255.;
    for i in 0..3 {
        p[i] = linear(color[i]) * a + p[i] * (1. - a);
    }
    p[3] = a + p[3] * (1. - a);
}
fn compose(
    pixels: &mut [[f32; 4]],
    canvas: [u32; 2],
    m: &GlyphMask,
    color: [u8; 4],
    offset: [f32; 2],
) {
    let (x, y) = (
        m.x + offset[0].round() as i32,
        m.y + offset[1].round() as i32,
    );
    for yy in 0..m.height {
        let py = y + yy as i32;
        if !(0..canvas[1] as i32).contains(&py) {
            continue;
        }
        for xx in 0..m.width {
            let px = x + xx as i32;
            if !(0..canvas[0] as i32).contains(&px) {
                continue;
            }
            let a = m.alpha[yy * m.width + xx];
            if a != 0 {
                blend(
                    &mut pixels[py as usize * canvas[0] as usize + px as usize],
                    color,
                    a as f32 / 255.,
                );
            }
        }
    }
}
pub(crate) fn dilate(m: &GlyphMask, radius: usize) -> GlyphMask {
    if radius == 0 {
        return m.clone();
    }
    let (w, h) = (m.width + 2 * radius, m.height + 2 * radius);
    let mut out = GlyphMask {
        x: m.x - radius as i32,
        y: m.y - radius as i32,
        width: w,
        height: h,
        alpha: vec![0; w * h],
    };
    // Disk dilation with monotonic row windows: O(radius * pixels), rather
    // than testing every disk point at every destination pixel.
    for sy in 0..m.height {
        let source = &m.alpha[sy * m.width..(sy + 1) * m.width];
        let mut rows = BTreeMap::new();
        for dy in -(radius as i32)..=radius as i32 {
            let rx = ((radius * radius) as f64 - f64::from(dy * dy))
                .sqrt()
                .floor() as i32;
            let row = rows.entry(rx).or_insert_with(|| {
                let mut row = vec![0; w];
                let mut queue = std::collections::VecDeque::new();
                let mut next = 0usize;
                for (x, a) in row.iter_mut().enumerate() {
                    let center = x as i32 - radius as i32;
                    let high = (center + rx).min(m.width as i32 - 1);
                    while next < m.width && (next as i32) <= high {
                        while queue.back().is_some_and(|&i| source[i] <= source[next]) {
                            queue.pop_back();
                        }
                        queue.push_back(next);
                        next += 1;
                    }
                    while queue.front().is_some_and(|&i| (i as i32) < center - rx) {
                        queue.pop_front();
                    }
                    *a = queue.front().map_or(0, |&i| source[i]);
                }
                row
            });
            let dst = (sy as i32 + radius as i32 + dy) as usize * w;
            for (x, a) in row.iter().enumerate() {
                out.alpha[dst + x] = out.alpha[dst + x].max(*a);
            }
        }
    }
    out
}
fn blur(m: &GlyphMask, r: usize) -> GlyphMask {
    if r == 0 {
        return m.clone();
    }
    let (w, h) = (m.width + 2 * r, m.height + 2 * r);
    let mut a = vec![0.; w * h];
    for y in 0..m.height {
        for x in 0..m.width {
            a[(y + r) * w + x + r] = m.alpha[y * m.width + x] as f32;
        }
    }
    let mut b = vec![0.; w * h];
    let n = (2 * r + 1) as f32;
    for y in 0..h {
        let mut sum = 0.;
        for x in 0..w + r {
            if x < w {
                sum += a[y * w + x];
            }
            if x > 2 * r {
                sum -= a[y * w + x - 2 * r - 1];
            }
            if x >= r && x - r < w {
                b[y * w + x - r] = sum / n;
            }
        }
    }
    let mut out = GlyphMask {
        x: m.x - r as i32,
        y: m.y - r as i32,
        width: w,
        height: h,
        alpha: vec![0; w * h],
    };
    for x in 0..w {
        let mut sum = 0.;
        for y in 0..h + r {
            if y < h {
                sum += b[y * w + x];
            }
            if y > 2 * r {
                sum -= b[(y - 2 * r - 1) * w + x];
            }
            if y >= r && y - r < h {
                out.alpha[(y - r) * w + x] = (sum / n).round() as u8;
            }
        }
    }
    out
}
impl TextSystem {
    pub(crate) fn rich_glyph_mask(&self, g: &GlyphPlacement) -> Result<GlyphMask> {
        let font = self.font(&g.font)?;
        ensure!(
            self.fonts[&g.font].1 == g.font_digest,
            "stale shaped font resource; rebuild the layout"
        );
        ensure!(
            (g.glyph_id as usize) < font.glyph_count(),
            "rich glyph range"
        );
        let scale =
            g.size * font.height_unscaled() / font.units_per_em().context("font units per em")?;
        let glyph = Glyph {
            id: GlyphId(g.glyph_id),
            scale: PxScale::from(scale),
            position: point(g.position[0], g.position[1]),
        };
        let Some(outline) = font.outline_glyph(glyph) else {
            ensure!(
                g.whitespace,
                "glyph has no CPU outline; bitmap/SVG/color font rasterization is unsupported"
            );
            return Ok(GlyphMask {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                alpha: Vec::new(),
            });
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as usize, bounds.height() as usize);
        ensure!(w * h <= 4 * 1024 * 1024, "glyph mask limit");
        let mut m = GlyphMask {
            x: bounds.min.x as i32,
            y: bounds.min.y as i32,
            width: w,
            height: h,
            alpha: vec![0; w * h],
        };
        outline.draw(|x, y, c| {
            m.alpha[y as usize * w + x as usize] = (c.clamp(0., 1.) * 255.).round() as u8
        });
        if g.bold {
            m = dilate(&m, (g.size * 0.025).ceil().clamp(1., 16.) as usize);
        }
        Ok(m)
    }
    /// Caller resolves icon assets through its own client capability root.
    /// All passes are CPU only; shadow and outline run before colored fills.
    pub fn rasterize_rich(
        &self,
        layout: &Layout,
        options: &RasterOptions,
        mut icons: impl FnMut(&str) -> Result<IconBitmap>,
    ) -> Result<Vec<u8>> {
        options.validate()?;
        layout.validate()?;
        ensure!(
            layout.canvas.iter().all(|n| *n > 0 && *n <= 8192)
                && u64::from(layout.canvas[0]) * u64::from(layout.canvas[1]) <= 4 * 1024 * 1024,
            "rich text canvas limit"
        );
        let visible = options.visible_graphemes.unwrap_or(layout.grapheme_count);
        ensure!(visible <= layout.grapheme_count, "text reveal range");
        let mut pixels = vec![[0.; 4]; layout.canvas[0] as usize * layout.canvas[1] as usize];
        let mut jobs = Vec::new();
        let mut total = 0usize;
        for g in layout.glyphs.iter().filter(|g| g.graphemes.end <= visible) {
            let m = self.rich_glyph_mask(g)?;
            total += m.alpha.len();
            ensure!(total <= 64 * 1024 * 1024, "glyph masks exceed 64 MiB");
            jobs.push((m, g.color));
        }
        if let Some(shadow) = &options.shadow {
            for (m, _) in &jobs {
                compose(
                    &mut pixels,
                    layout.canvas,
                    &blur(m, shadow.blur_radius as usize),
                    shadow.color,
                    shadow.offset,
                );
            }
        }
        if let Some(outline) = options.outline.as_ref().filter(|o| o.width > 0.) {
            for (m, _) in &jobs {
                compose(
                    &mut pixels,
                    layout.canvas,
                    &dilate(m, outline.width.ceil() as usize),
                    outline.color,
                    [0.; 2],
                );
            }
        }
        for (m, color) in &jobs {
            compose(&mut pixels, layout.canvas, m, *color, [0.; 2]);
        }
        let mut cache = BTreeMap::new();
        let mut icon_bytes = 0usize;
        for i in layout.icons.iter().filter(|i| i.graphemes.end <= visible) {
            if !cache.contains_key(&i.asset) {
                let image = icons(&i.asset)?;
                ensure!(
                    image.width > 0
                        && image.height > 0
                        && image.width <= 8192
                        && image.height <= 8192
                        && u64::from(image.width) * u64::from(image.height) * 4
                            == image.rgba.len() as u64
                        && image.rgba.len() <= 16 * 1024 * 1024,
                    "inline icon RGBA bounds"
                );
                icon_bytes = icon_bytes
                    .checked_add(image.rgba.len())
                    .context("inline icon byte overflow")?;
                ensure!(
                    icon_bytes <= 64 * 1024 * 1024,
                    "inline icon cache exceeds 64 MiB"
                );
                cache.insert(i.asset.clone(), image);
            }
            let image = &cache[&i.asset];
            let [x, y, w, h] = i.rect;
            for py in
                y.floor().max(0.) as u32..(y + h).ceil().min(layout.canvas[1] as f32).max(0.) as u32
            {
                for px in x.floor().max(0.) as u32
                    ..(x + w).ceil().min(layout.canvas[0] as f32).max(0.) as u32
                {
                    let sx = (((px as f32 + 0.5 - x) / w).clamp(0., 0.999999) * image.width as f32)
                        as usize;
                    let sy = (((py as f32 + 0.5 - y) / h).clamp(0., 0.999999) * image.height as f32)
                        as usize;
                    let at = (sy * image.width as usize + sx) * 4;
                    let c = std::array::from_fn(|n| {
                        ((image.rgba[at + n] as u16 * i.color[n] as u16 + 127) / 255) as u8
                    });
                    blend(
                        &mut pixels[py as usize * layout.canvas[0] as usize + px as usize],
                        c,
                        1.,
                    );
                }
            }
        }
        Ok(pixels
            .into_iter()
            .flat_map(|p| {
                let a = p[3];
                [
                    encoded(if a > 0. { p[0] / a } else { 0. }),
                    encoded(if a > 0. { p[1] / a } else { 0. }),
                    encoded(if a > 0. { p[2] / a } else { 0. }),
                    (a.clamp(0., 1.) * 255.).round() as u8,
                ]
            })
            .collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dilation_blur_and_linear_alpha_composition_are_measurable() {
        let m = GlyphMask {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            alpha: vec![255],
        };
        let d = dilate(&m, 1);
        assert_eq!((d.x, d.width), (-1, 3));
        assert_eq!(d.alpha.iter().filter(|&&x| x > 0).count(), 5);
        let b = blur(&m, 1);
        assert_eq!(b.alpha, [28; 9]);
        let mut p = [0.; 4];
        blend(&mut p, [255, 0, 0, 128], 1.);
        blend(&mut p, [0, 0, 255, 128], 1.);
        assert!((p[3] - 0.752).abs() < 0.002);
        assert!(encoded(p[2] / p[3]) > 200);
    }
    #[test]
    fn optimized_disk_dilation_matches_independent_brute_force() {
        let m = GlyphMask {
            x: 3,
            y: 7,
            width: 5,
            height: 3,
            alpha: vec![0, 32, 100, 0, 17, 100, 255, 12, 200, 0, 17, 0, 128, 45, 70],
        };
        for r in 1..=5 {
            let out = dilate(&m, r);
            for y in 0..out.height {
                for x in 0..out.width {
                    let mut expected = 0;
                    for sy in 0..m.height {
                        for sx in 0..m.width {
                            let dx = x as i32 - r as i32 - sx as i32;
                            let dy = y as i32 - r as i32 - sy as i32;
                            if dx * dx + dy * dy <= (r * r) as i32 {
                                expected = expected.max(m.alpha[sy * m.width + sx]);
                            }
                        }
                    }
                    assert_eq!(out.alpha[y * out.width + x], expected);
                }
            }
        }
    }
}
