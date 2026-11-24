// SPDX-License-Identifier: MPL-2.0
//! Float-positioned glyph coverage cells. Native layout and reveal stay with callers.
use crate::TextSystem;
use ab_glyph::{Font, GlyphId, PxScale, ScaleFont, point};
use anyhow::{Context, Result, ensure};
pub mod vertical;

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
    pub fn outline_em_metrics(&self, alias: &str, em: f32) -> Result<crysirror_core::service::OutlineEmMetrics> {
        let line_height = self.line_height_for_em(alias, em)?;
        let font = self.font(alias)?;
        let (bytes, index) = self.font_source(alias)?;
        let face = rustybuzz::ttf_parser::Face::parse(&bytes, index).context("outline font face")?;
        let ascent = font.as_scaled(line_height).ascent();
        let typographic_ascent = face.typographic_ascender().filter(|v| *v > 0)
            .map_or(ascent, |v| em * f32::from(v) / f32::from(face.units_per_em()));
        Ok(crysirror_core::service::OutlineEmMetrics { line_height, ascent, typographic_ascent })
    }

    pub fn line_height_for_em(&self, font: &str, em: f32) -> Result<f32> {
        ensure!(em.is_finite() && em > 0. && em <= 512., "font em size");
        ensure!(
            self.precomputed_face(font)?.is_none(),
            "bitmap font has no em design scale"
        );
        let face = self.font(font)?;
        let scale =
            em * face.height_unscaled() / face.units_per_em().context("font units per em")?;
        ensure!(
            scale.is_finite() && scale > 0. && scale <= 2048.,
            "font line-height scale"
        );
        Ok(scale)
    }
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
        let digest = self.font_digest(font)?;
        Ok(format!(
            "glyph:{digest}:{}:{:08x}:{:08x}:{:08x}",
            request.glyph_id,
            request.size.to_bits(),
            request.baseline[0].to_bits(),
            request.baseline[1].to_bits()
        ))
    }
    pub fn glyph_id(&self, font: &str, ch: char) -> Result<u32> {
        if let Some(face) = self.precomputed_face(font)? {
            return face
                .glyph_id(ch)
                .with_context(|| format!("precomputed font '{font}' missing U+{:04X}", ch as u32));
        }
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
        if let Some(face) = self.precomputed_face(font)? {
            face.check_size(size)?;
            return Ok(GlyphMetrics {
                ascent: face.ascent(),
                descent: face.descent(),
                advance: f32::from(face.glyph(glyph_id)?.advance[0]),
            });
        }
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
        self.rasterize_glyph_cell_scaled(font, request, 1.)
    }
    /// Rasterize actual outline geometry at an explicit horizontal scale;
    /// coverage is produced at its final width before any outline kernel.
    pub fn rasterize_glyph_cell_scaled(
        &self,
        font: &str,
        request: GlyphRaster,
        horizontal_scale: f32,
    ) -> Result<GlyphCell> {
        self.rasterize_glyph_cell_scaled_limit(font, request, horizontal_scale, 1024.)
    }
    pub(crate) fn rasterize_glyph_cell_scaled_limit(
        &self,
        font: &str,
        request: GlyphRaster,
        horizontal_scale: f32,
        max_size: f32,
    ) -> Result<GlyphCell> {
        ensure!(
            horizontal_scale.is_finite() && (0.03125..=32.).contains(&horizontal_scale),
            "glyph horizontal scale"
        );
        ensure!(
            request.size.is_finite() && request.size > 0. && request.size <= max_size,
            "invalid glyph raster size"
        );
        self.glyph_metrics(font, request.glyph_id, request.size.min(1024.))?;
        ensure!(
            request
                .baseline
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid glyph baseline"
        );
        if let Some(face) = self.precomputed_face(font)? {
            ensure!(
                horizontal_scale == 1.,
                "precomputed font has no outline width transform"
            );
            return face.cell(request);
        }
        let font = self.font(font)?;
        let glyph = GlyphId(request.glyph_id as u16).with_scale_and_position(
            PxScale {
                x: request.size * horizontal_scale,
                y: request.size,
            },
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
    /// Scale an outline face so its OS/2 average-character width reaches the
    /// requested logical pixel width. No system font substitution is involved.
    pub fn font_width_scale(&self, font: &str, size: f32, width: f32) -> Result<f32> {
        ensure!(
            size.is_finite()
                && (1.0..=512.).contains(&size)
                && width.is_finite()
                && (0.01..=4096.).contains(&width),
            "font width request"
        );
        ensure!(
            self.precomputed_face(font)?.is_none(),
            "precomputed font has no design average width"
        );
        let (bytes, index) = self.font_source(font)?;
        let face = rustybuzz::Face::from_slice(&bytes, index).context("font width face")?;
        let table = face
            .raw_face()
            .table(rustybuzz::ttf_parser::Tag::from_bytes(b"OS/2"))
            .context("font lacks OS/2 average width")?;
        let raw = i16::from_be_bytes(
            table
                .get(2..4)
                .context("font OS/2 average width field")?
                .try_into()?,
        );
        ensure!(raw > 0, "font average width is unspecified");
        let outline = self.font(font)?;
        let average = f32::from(raw) * outline.as_scaled(PxScale::from(size)).h_scale_factor();
        let scale = width / average;
        ensure!(
            scale.is_finite() && (0.03125..=32.).contains(&scale),
            "requested font width exceeds supported scale"
        );
        Ok(scale)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outline_horizontal_scale_is_validated_before_font_or_coverage_work() {
        let text = TextSystem::default();
        let request = GlyphRaster {
            glyph_id: 1,
            size: 22.,
            baseline: [0., 0.],
        };
        for scale in [f32::NAN, 0., -1., 33.] {
            assert!(
                text.rasterize_glyph_cell_scaled("missing", request, scale)
                    .is_err()
            );
        }
        assert!(text.font_width_scale("missing", 22., 0.).is_err());
    }
    #[test]
    #[ignore = "reads caller-selected outline font; no proprietary or system font bytes are committed"]
    fn average_width_uses_actual_face_metadata_and_rasterizes_wider_outlines() {
        let path =
            std::env::var_os("CRYSIRROR_TEST_FONT").expect("supply a local outline font path");
        let mut text = TextSystem::default();
        text.add_font("test", std::fs::read(path).unwrap()).unwrap();
        let glyph_id = text.glyph_id("test", 'A').unwrap();
        let request = GlyphRaster {
            glyph_id,
            size: 22.,
            baseline: [0., 0.],
        };
        let normal = text.rasterize_glyph_cell("test", request).unwrap();
        let wide = text
            .rasterize_glyph_cell_scaled("test", request, 2.)
            .unwrap();
        assert!(wide.width > normal.width);
        assert_eq!(wide.height, normal.height);
        let factor = text.font_width_scale("test", 22., 44.).unwrap();
        assert!(factor.is_finite() && factor > 0.);
        assert!((text.font_width_scale("test", 22., 22.).unwrap() * 2. - factor).abs() < 1e-5);
    }
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
