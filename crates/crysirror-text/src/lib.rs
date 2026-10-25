// SPDX-License-Identifier: MPL-2.0
//! Engine-owned CPU text service, independent of renderer, window and native VMs.
//! Basic layout is explicit; native frontends may submit positioned glyphs.
#![forbid(unsafe_code)]
use ab_glyph::{Font, FontArc, Glyph, GlyphId, PxScale, ScaleFont, point};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    service::{Glyph as PositionedGlyph, SIMPLE_TEXT_LAYOUT, TextService},
    state::TextRun,
};
use std::{collections::BTreeMap, sync::Arc};

pub mod glyph;
pub mod layout;
pub mod raster;
pub mod reveal;
pub mod rich;
pub mod sdf;

#[derive(Default)]
pub struct TextSystem {
    fonts: BTreeMap<String, (FontArc, String)>,
    // Font bytes remain CPU resources, never project/save data or atlas output.
    font_sources: BTreeMap<String, (Arc<[u8]>, u32)>,
}
impl TextSystem {
    /// Metrics from the same engine font resource used by retained glyph runs.
    pub fn metrics(
        &self,
        alias: &str,
        size: f32,
        character: char,
    ) -> Result<crysirror_core::service::FontMetrics> {
        ensure!(
            size.is_finite() && (1.0..=512.0).contains(&size),
            "font metric size"
        );
        let face = self.font(alias)?;
        let scaled = face.as_scaled(PxScale::from(size));
        Ok(crysirror_core::service::FontMetrics {
            ascent: scaled.ascent(),
            descent: scaled.descent(),
            advance: scaled.h_advance(face.glyph_id(character)),
        })
    }
    pub fn add_font(&mut self, alias: &str, bytes: Vec<u8>) -> Result<()> {
        self.add_font_face(alias, bytes, 0)
    }
    /// Load a face in a TTC/OTC collection. No font bytes are bundled by the engine.
    pub fn add_font_face(&mut self, alias: &str, bytes: Vec<u8>, face_index: u32) -> Result<()> {
        ensure!(
            !alias.is_empty()
                && alias.len() <= 256
                && !alias.chars().any(char::is_control)
                && bytes.len() <= 32 * 1024 * 1024,
            "invalid font alias/size"
        );
        let digest = if face_index == 0 {
            blake3::hash(&bytes).to_hex().to_string()
        } else {
            let mut hash = blake3::Hasher::new();
            hash.update(&bytes);
            hash.update(&face_index.to_le_bytes());
            hash.finalize().to_hex().to_string()
        };
        ensure!(
            self.fonts.contains_key(alias) || self.fonts.len() < 64,
            "font resource limit"
        );
        rustybuzz::Face::from_slice(&bytes, face_index).context("invalid shaping face/index")?;
        let font: FontArc = ab_glyph::FontVec::try_from_vec_and_index(bytes.clone(), face_index)
            .map_err(|e| anyhow::anyhow!("font {alias}: {e}"))?
            .into();
        self.font_sources
            .insert(alias.into(), (Arc::from(bytes), face_index));
        self.fonts.insert(alias.into(), (font, digest));
        Ok(())
    }
    fn font(&self, alias: &str) -> Result<&FontArc> {
        Ok(&self
            .fonts
            .get(alias)
            .with_context(|| {
                format!("font alias '{alias}' is not loaded; provide --font or project.fonts")
            })?
            .0)
    }
    pub fn key(&self, text: &TextRun) -> Result<String> {
        ensure!(
            text.layout_profile == SIMPLE_TEXT_LAYOUT,
            "layout '{}' is not implemented; native RFVP/Siglus layout must be provided by its adapter",
            text.layout_profile
        );
        let (_, digest) = self.fonts.get(&text.font).with_context(|| {
            format!(
                "font alias '{}' is not loaded; provide --font or project.fonts",
                text.font
            )
        })?;
        Ok(format!(
            "text:{}",
            blake3::hash(format!("{digest}:{text:?}").as_bytes()).to_hex()
        ))
    }
    fn validate_surface(size: u16, canvas: [u32; 2]) -> Result<()> {
        ensure!((1..=512).contains(&size), "text size must be 1..512");
        ensure!(
            canvas[0] > 0
                && canvas[1] > 0
                && u64::from(canvas[0]) * u64::from(canvas[1]) <= 4 * 1024 * 1024,
            "text surface exceeds four million pixels or is empty"
        );
        Ok(())
    }
    fn positioned(&self, text: &TextRun) -> Result<Vec<Glyph>> {
        self.key(text)?;
        Self::validate_surface(text.size_px, [text.bounds.width, text.bounds.height])?;
        ensure!(text.text.len() <= 1024 * 1024, "text exceeds 1 MiB");
        let font = self.font(&text.font)?;
        let size = f32::from(text.size_px);
        let scaled = font.as_scaled(size);
        let line_height = (scaled.ascent() - scaled.descent() + scaled.line_gap()).max(1.);
        let mut x = 0.;
        let mut baseline = scaled.ascent();
        let mut previous = None;
        let mut glyphs = Vec::new();
        for ch in text.text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                x = 0.;
                baseline += line_height;
                previous = None;
                continue;
            }
            if ch == '\t' {
                x += scaled.h_advance(font.glyph_id(' ')) * 4.;
                previous = None;
                continue;
            }
            let id = font.glyph_id(ch);
            ensure!(
                id.0 != 0,
                "font '{}' has no glyph for U+{:04X}",
                text.font,
                ch as u32
            );
            let kern = previous.map(|p| scaled.kern(p, id)).unwrap_or(0.);
            let advance = scaled.h_advance(id);
            if x > 0. && x + kern + advance > text.bounds.width as f32 {
                x = 0.;
                baseline += line_height;
                previous = None;
            }
            if previous.is_some() {
                x += kern;
            }
            if baseline - scaled.ascent() >= text.bounds.height as f32 {
                break;
            }
            glyphs.push(Glyph {
                id,
                scale: PxScale::from(size),
                position: point(x, baseline),
            });
            x += advance;
            previous = Some(id);
        }
        Ok(glyphs)
    }
    pub fn layout_glyphs(&self, text: &TextRun) -> Result<Vec<PositionedGlyph>> {
        let glyphs = self.positioned(text)?;
        let scaled = self.font(&text.font)?.as_scaled(f32::from(text.size_px));
        Ok(glyphs
            .into_iter()
            .map(|g| PositionedGlyph {
                glyph_id: u32::from(g.id.0),
                x_26_6: (g.position.x * 64.).round() as i32,
                y_26_6: (g.position.y * 64.).round() as i32,
                advance_26_6: (scaled.h_advance(g.id) * 64.).round() as i32,
            })
            .collect())
    }
    /// Preserve the original simple-layout float placements for pixel stability.
    pub fn rasterize(&self, text: &TextRun) -> Result<Vec<u8>> {
        let glyphs = self.positioned(text)?;
        Self::paint(
            self.font(&text.font)?,
            [text.bounds.width, text.bounds.height],
            &glyphs,
        )
    }
    /// Shared drawing primitive for explicit glyph positions from a shaper or a
    /// compatibility frontend. Does not guess line breaking/ruby/reveal policy.
    /// Callers must include the glyph data in any cache identity they create.
    pub fn rasterize_glyphs(
        &self,
        font: &str,
        size: u16,
        canvas: [u32; 2],
        glyphs: &[PositionedGlyph],
    ) -> Result<Vec<u8>> {
        Self::validate_surface(size, canvas)?;
        ensure!(glyphs.len() <= 1024 * 1024, "too many positioned glyphs");
        let font = self.font(font)?;
        let mut positioned = Vec::with_capacity(glyphs.len());
        for g in glyphs {
            ensure!(
                (g.glyph_id as usize) < font.glyph_count(),
                "glyph id out of font range"
            );
            let id = u16::try_from(g.glyph_id).context("glyph id exceeds font representation")?;
            positioned.push(Glyph {
                id: GlyphId(id),
                scale: PxScale::from(f32::from(size)),
                position: point(g.x_26_6 as f32 / 64., g.y_26_6 as f32 / 64.),
            });
        }
        Self::paint(font, canvas, &positioned)
    }
    fn paint(font: &FontArc, canvas: [u32; 2], glyphs: &[Glyph]) -> Result<Vec<u8>> {
        let (w, h) = (canvas[0], canvas[1]);
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[..3].fill(255);
        }
        for glyph in glyphs {
            if let Some(outline) = font.outline_glyph(glyph.clone()) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                        let i = ((py as usize) * (w as usize) + px as usize) * 4 + 3;
                        let alpha = (coverage.clamp(0., 1.) * 255.).round() as u16;
                        let old = u16::from(pixels[i]);
                        pixels[i] = (alpha + (old * (255 - alpha) + 127) / 255) as u8;
                    }
                });
            }
        }
        Ok(pixels)
    }
}
impl TextService for TextSystem {
    fn layout(&mut self, text: &TextRun) -> crysirror_core::Result<Vec<PositionedGlyph>> {
        if text.layout_profile != SIMPLE_TEXT_LAYOUT {
            return Err(crysirror_core::Error::Unsupported("text layout profile"));
        }
        self.layout_glyphs(text)
            .map_err(|_| crysirror_core::Error::Invalid("text layout request/font"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_font_is_an_error() {
        assert!(
            TextSystem::default()
                .add_font("bad", vec![0, 1, 2])
                .is_err()
        );
    }
    #[test]
    fn legacy_layout_is_not_silently_approximated() {
        let t = TextRun {
            text: "x".into(),
            font: "default".into(),
            size_px: 16,
            bounds: crysirror_core::state::Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            color: crysirror_core::state::Color([255; 4]),
            layout_profile: "siglus-ruby".into(),
        };
        let mut system = TextSystem::default();
        assert!(
            system
                .key(&t)
                .unwrap_err()
                .to_string()
                .contains("not implemented")
        );
        assert_eq!(
            TextService::layout(&mut system, &t),
            Err(crysirror_core::Error::Unsupported("text layout profile"))
        );
    }
    #[test]
    fn explicit_glyph_surface_is_bounded_before_allocation() {
        assert!(
            TextSystem::default()
                .rasterize_glyphs("x", 16, [u32::MAX, u32::MAX], &[])
                .is_err()
        );
        assert!(
            TextSystem::default()
                .rasterize_glyphs("x", 0, [1, 1], &[])
                .is_err()
        );
    }
    #[test]
    #[ignore = "requires a caller-supplied licensed font via CRYSIRROR_TEST_FONT; none bundled"]
    fn supplied_font_layout_and_positioned_raster_are_cpu_only() {
        let file = std::env::var_os("CRYSIRROR_TEST_FONT")
            .expect("set CRYSIRROR_TEST_FONT to a licensed local font");
        let mut system = TextSystem::default();
        system
            .add_font("fixture", std::fs::read(file).unwrap())
            .unwrap();
        let text = TextRun {
            text: "AB\nC".into(),
            font: "fixture".into(),
            size_px: 24,
            bounds: crysirror_core::state::Rect {
                x: 0,
                y: 0,
                width: 128,
                height: 96,
            },
            color: crysirror_core::state::Color([255; 4]),
            layout_profile: SIMPLE_TEXT_LAYOUT.into(),
        };
        let glyphs = TextService::layout(&mut system, &text).unwrap();
        assert_eq!(glyphs.len(), 3);
        assert!(glyphs[1].x_26_6 > glyphs[0].x_26_6);
        assert!(glyphs[2].y_26_6 > glyphs[0].y_26_6);
        for pixels in [
            system.rasterize(&text).unwrap(),
            system
                .rasterize_glyphs("fixture", 24, [128, 96], &glyphs)
                .unwrap(),
        ] {
            assert_eq!(pixels.len(), 128 * 96 * 4);
            assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        }
        let mut invalid = glyphs;
        invalid[0].glyph_id = u32::MAX;
        assert!(
            system
                .rasterize_glyphs("fixture", 24, [128, 96], &invalid)
                .is_err()
        );
    }
}
