// SPDX-License-Identifier: MPL-2.0
//! Float-positioned glyph coverage cells. Native layout and reveal stay with callers.
use crate::TextSystem;
use ab_glyph::{Font, GlyphId, PxScale, ScaleFont, point};
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug)]
pub struct GlyphRaster {
    pub glyph_id: u32,
    pub size: f32,
    /// Exact baseline coordinates in pixels; no fixed-point quantization.
    pub baseline: [f32; 2],
}
#[derive(Clone, Debug)]
pub struct GlyphCell {
    pub origin: [i32; 2],
    pub width: u32,
    pub height: u32,
    /// Unquantized outline coverage, in row-major order. Consumers choose their
    /// alpha rounding after applying opacity. Empty outlines return an empty cell.
    pub coverage: Vec<f32>,
}
#[derive(Clone, Copy, Debug)]
pub struct GlyphMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub advance: f32,
}
impl TextSystem {
    /// Stable identity for a positioned glyph resource. Consumers may subtract
    /// integer baseline translation to share raster cells across placements.
    pub fn glyph_cell_key(&self, font: &str, request: GlyphRaster) -> Result<String> {
        self.glyph_metrics(font, request.glyph_id, request.size)?;
        ensure!(
            request
                .baseline
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid glyph baseline"
        );
        let (_, digest) = self.fonts.get(font).expect("glyph_metrics checked font");
        Ok(format!(
            "glyph:{digest}:{}:{:08x}:{:08x}:{:08x}",
            request.glyph_id,
            request.size.to_bits(),
            request.baseline[0].to_bits(),
            request.baseline[1].to_bits()
        ))
    }
    pub fn glyph_id(&self, font: &str, ch: char) -> Result<u32> {
        let id = self.font(font)?.glyph_id(ch);
        ensure!(
            id.0 != 0 || ch.is_control(),
            "font '{font}' has no glyph for U+{:04X}",
            ch as u32
        );
        Ok(u32::from(id.0))
    }
    pub fn glyph_metrics(&self, font: &str, glyph_id: u32, size: f32) -> Result<GlyphMetrics> {
        ensure!(
            size.is_finite() && (0.0..=1024.0).contains(&size) && size > 0.,
            "invalid glyph size"
        );
        let font = self.font(font)?;
        ensure!(
            (glyph_id as usize) < font.glyph_count(),
            "glyph id out of font range"
        );
        let scaled = font.as_scaled(PxScale::from(size));
        Ok(GlyphMetrics {
            ascent: scaled.ascent(),
            descent: scaled.descent(),
            advance: scaled.h_advance(GlyphId(glyph_id as u16)),
        })
    }
    pub fn rasterize_glyph_cell(&self, font: &str, request: GlyphRaster) -> Result<GlyphCell> {
        self.glyph_metrics(font, request.glyph_id, request.size)?;
        ensure!(
            request
                .baseline
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid glyph baseline"
        );
        let font = self.font(font)?;
        let glyph = GlyphId(request.glyph_id as u16).with_scale_and_position(
            PxScale::from(request.size),
            point(request.baseline[0], request.baseline[1]),
        );
        let Some(outlined) = font.outline_glyph(glyph) else {
            return Ok(GlyphCell {
                origin: [0, 0],
                width: 0,
                height: 0,
                coverage: Vec::new(),
            });
        };
        let bounds = outlined.px_bounds();
        ensure!(
            [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_100_000.),
            "invalid glyph bounds"
        );
        let width = (bounds.max.x as i32 - bounds.min.x as i32).max(0) as u32;
        let height = (bounds.max.y as i32 - bounds.min.y as i32).max(0) as u32;
        ensure!(
            u64::from(width) * u64::from(height) <= 4 * 1024 * 1024,
            "glyph coverage exceeds four million pixels"
        );
        let mut coverage = vec![0.; width as usize * height as usize];
        outlined.draw(|x, y, value| {
            if x < width && y < height {
                coverage[(y * width + x) as usize] = value;
            }
        });
        Ok(GlyphCell {
            origin: [bounds.min.x as i32, bounds.min.y as i32],
            width,
            height,
            coverage,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unloaded_font_and_invalid_float_requests_fail_without_allocation() {
        let text = TextSystem::default();
        assert!(
            text.rasterize_glyph_cell(
                "missing",
                GlyphRaster {
                    glyph_id: 1,
                    size: 24.,
                    baseline: [0., 0.]
                }
            )
            .is_err()
        );
        assert!(text.glyph_metrics("missing", 1, f32::NAN).is_err());
    }
}
