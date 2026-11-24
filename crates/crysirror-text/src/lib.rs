// SPDX-License-Identifier: MPL-2.0
//! Engine-owned CPU text service, independent of renderer, window and native VMs.
//! Basic layout is explicit; native frontends may submit positioned glyphs.
#![forbid(unsafe_code)]
use ab_glyph::{Font, FontArc, Glyph, GlyphId, PxScale, ScaleFont, point};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    service::{Glyph as PositionedGlyph, SIMPLE_TEXT_LAYOUT, TextService, UNWRAPPED_TEXT_LAYOUT},
    state::TextRun,
};
use std::{collections::BTreeMap, sync::Arc};
use outline_font::{font_arc, FontBacking};

pub mod dynamic;
pub mod legacy_width;
pub mod precomputed_font;
pub mod format;
pub mod byte_replace;
pub mod printf32;
pub mod printf_values;
pub mod glyph;
pub mod glyph_blur;
pub mod glyph_coverage;
pub mod grid;
pub mod width;
pub mod bitmap_layout;
pub mod message;
pub mod unit_flow;
pub mod layout;
pub mod markup;
pub mod raster;
mod styled_glyph;
mod glyph_coverage_offsets;
mod glyph_quality;
pub mod reveal;
pub mod rich;
pub mod sdf;
pub mod decoration;
mod outline_font;

#[derive(Clone, Default)]
pub struct TextSystem {
    fonts: Arc<BTreeMap<String, (FontArc, String)>>,
    font_families: Arc<BTreeMap<String, Vec<String>>>,
    font_styles: Arc<BTreeMap<String,Option<crysirror_core::service::FontFaceStyle>>>,
    // Font bytes remain CPU resources, never project/save data or atlas output.
    font_sources: Arc<BTreeMap<String, (Arc<FontBacking>, u32)>>,
    dynamic: dynamic::Catalog,
    scope: Option<crysirror_core::state::ClientId>,
}
impl TextSystem {
    /// Measure one unwrapped simple-layout line with the exact kerning and
    /// advances used by `positioned`. Complex shaping uses `layout_rich`.
    pub fn measure_simple(
        &self,
        alias: &str,
        size: f32,
        text: &str,
    ) -> Result<crysirror_core::service::TextExtent> {
        ensure!(
            size.is_finite() && (1.0..=512.0).contains(&size),
            "text metric size"
        );
        ensure!(
            text.len() <= 1024 * 1024 && !text.contains(['\n', '\r']),
            "single-line text metric limit"
        );
        if let Some(font)=self.precomputed_face(alias)? { return font.measure_simple(size,text); }
        let font = self.font(alias)?;
        let scaled = font.as_scaled(PxScale::from(size));
        let mut advance = 0.;
        let mut previous = None;
        for ch in text.chars() {
            if ch == '\t' {
                advance += scaled.h_advance(font.glyph_id(' ')) * 4.;
                previous = None;
                continue;
            }
            let id = font.glyph_id(ch);
            ensure!(
                id.0 != 0,
                "font '{alias}' has no glyph for U+{:04X}",
                ch as u32
            );
            advance += previous.map_or(0., |p| scaled.kern(p, id)) + scaled.h_advance(id);
            previous = Some(id);
        }
        Ok(crysirror_core::service::TextExtent {
            advance,
            line_height: (scaled.ascent() - scaled.descent() + scaled.line_gap()).max(1.),
            ascent: scaled.ascent(),
            descent: scaled.descent(),
        })
    }
    /// Reuse the exact unwrapped simple painter placements for retained runs.
    pub fn position_simple(
        &self,
        font: &str,
        size: u16,
        text: &str,
    ) -> Result<Vec<crysirror_core::service::SimpleTextPlacement>> {
        ensure!((1..=512).contains(&size), "text metric size");
        ensure!(
            text.len() <= 1024 * 1024 && !text.contains(['\n', '\r']),
            "single-line text metric limit"
        );
        if let Some(face) = self.precomputed_face(font)? {
            face.check_size(f32::from(size))?;
            return face.position_simple(text);
        }
        let glyphs = self.position_glyphs(font, size, text, false, [1, u32::from(size) * 2])?;
        Ok(text
            .chars()
            .filter(|c| *c != '\t')
            .zip(glyphs)
            .map(
                |(character, g)| crysirror_core::service::SimpleTextPlacement {
                    character,
                    baseline: [g.position.x, g.position.y],
                },
            )
            .collect())
    }
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
        if let Some(face)=self.precomputed_face(alias)? {
            face.check_size(size)?;let id=face.glyph_id(character).context("precomputed font missing character")?;
            return Ok(crysirror_core::service::FontMetrics{ascent:face.ascent(),descent:face.descent(),advance:f32::from(face.glyph(id)?.logical_advance)});
        }
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
                && !alias.starts_with("__font/")
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
        let face=rustybuzz::Face::from_slice(&bytes, face_index).context("invalid shaping face/index")?;
        let families=dynamic::family_names(&face)?;
        let style=dynamic::face_style(&face);
        let shared = FontBacking::new(bytes.into());
        let font = font_arc(shared.clone(), face_index).map_err(|e| anyhow::anyhow!("font {alias}: {e}"))?;
        Arc::make_mut(&mut self.font_sources)
            .insert(alias.into(), (shared, face_index));
        Arc::make_mut(&mut self.fonts).insert(alias.into(), (font, digest));
        Arc::make_mut(&mut self.font_families).insert(alias.into(),families);
        Arc::make_mut(&mut self.font_styles).insert(alias.into(),style);
        Ok(())
    }
    /// Resolve only preinstalled static aliases/family names. Metadata is
    /// prepared at face installation, so this bounded lookup parses no bytes.
    pub fn static_font_alias(&self,name:&str)->Option<String> {
        if self.fonts.contains_key(name) { return Some(name.into()); }
        self.font_families.iter().find(|(_,families)|families.iter().any(|f|f.eq_ignore_ascii_case(name))).map(|(alias,_)|alias.clone())
    }
    /// Cached OS/2 face style, scoped like glyph lookup; no parsing occurs here.
    pub fn font_style(&self,alias:&str)->Result<Option<crysirror_core::service::FontFaceStyle>> {
        if let Some(style)=self.font_styles.get(alias) {return Ok(*style);}
        Ok(self.dynamic_face(alias)?.style)
    }
    /// Content digest of a static or client-authorized dynamic face.
    pub fn font_digest(&self, alias: &str) -> Result<String> {
        if let Some((_, digest)) = self.fonts.get(alias) { return Ok(digest.clone()); }
        Ok(self.dynamic_face(alias)?.digest.clone())
    }
    fn font(&self, alias: &str) -> Result<FontArc> {
        if let Some((font, _)) = self.fonts.get(alias) { return Ok(font.clone()); }
        self.dynamic_face(alias)?.font.clone().context("precomputed face has no outline/shaping backend")
    }
    fn precomputed_face(&self, alias:&str) -> Result<Option<Arc<precomputed_font::PrecomputedFont>>> {
        if self.fonts.contains_key(alias){ return Ok(None); }
        Ok(self.dynamic_face(alias)?.precomputed.clone())
    }
    fn font_source(&self, alias: &str) -> Result<(Arc<FontBacking>,u32)> {
        if let Some(source) = self.font_sources.get(alias) { return Ok(source.clone()); }
        let face = self.dynamic_face(alias)?;
        Ok((face.bytes.clone(), face.face_index))
    }
    pub fn key(&self, text: &TextRun) -> Result<String> {
        ensure!(
            matches!(
                text.layout_profile.as_str(),
                SIMPLE_TEXT_LAYOUT | UNWRAPPED_TEXT_LAYOUT
            ),
            "layout '{}' is not implemented; native RFVP/Siglus layout must be provided by its adapter",
            text.layout_profile
        );
        let digest = self.font_digest(&text.font)?;
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
        self.position_glyphs(
            &text.font,
            text.size_px,
            &text.text,
            text.layout_profile == SIMPLE_TEXT_LAYOUT,
            [text.bounds.width, text.bounds.height],
        )
    }
    fn position_glyphs(
        &self,
        alias: &str,
        size: u16,
        text: &str,
        wrap: bool,
        bounds: [u32; 2],
    ) -> Result<Vec<Glyph>> {
        let font = self.font(alias)?;
        let size = f32::from(size);
        let scaled = font.as_scaled(size);
        let line_height = (scaled.ascent() - scaled.descent() + scaled.line_gap()).max(1.);
        let mut x = 0.;
        let mut baseline = scaled.ascent();
        let mut previous = None;
        let mut glyphs = Vec::new();
        for ch in text.chars() {
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
                "font '{alias}' has no glyph for U+{:04X}",
                ch as u32
            );
            let kern = previous.map(|p| scaled.kern(p, id)).unwrap_or(0.);
            let advance = scaled.h_advance(id);
            if wrap && x > 0. && x + kern + advance > bounds[0] as f32 {
                x = 0.;
                baseline += line_height;
                previous = None;
            }
            if previous.is_some() {
                x += kern;
            }
            if baseline - scaled.ascent() >= bounds[1] as f32 {
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
        if let Some(face)=self.precomputed_face(&text.font)? {
            self.key(text)?;Self::validate_surface(text.size_px,[text.bounds.width,text.bounds.height])?;
            return face.layout(text);
        }
        let glyphs = self.positioned(text)?;
        let font = self.font(&text.font)?;
        let scaled = font.as_scaled(f32::from(text.size_px));
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
        if let Some(face)=self.precomputed_face(&text.font)? {
            self.key(text)?;Self::validate_surface(text.size_px,[text.bounds.width,text.bounds.height])?;
            return face.paint([text.bounds.width,text.bounds.height],&face.layout(text)?);
        }
        let glyphs = self.positioned(text)?;
        Self::paint(
            &self.font(&text.font)?,
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
        self.rasterize_glyphs_cancellable(font, size, canvas, glyphs, &|| Ok(()))
    }
    /// The same white-RGB/coverage-alpha primitive, with cooperative cancellation
    /// and bounded actual outline work. A worker can freeze its face before
    /// calling this method; it never publishes mutable font or image state.
    pub fn rasterize_glyphs_cancellable(
        &self,
        font: &str,
        size: u16,
        canvas: [u32; 2],
        glyphs: &[PositionedGlyph],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<u8>> {
        check()?;
        Self::validate_surface(size, canvas)?;
        ensure!(glyphs.len() <= 1024 * 1024, "too many positioned glyphs");
        if let Some(face) = self.precomputed_face(font)? {
            face.check_size(f32::from(size))?;
            return face.paint_cancellable(canvas, glyphs, check);
        }
        let font = self.font(font)?;
        let mut positioned = Vec::with_capacity(glyphs.len());
        for (index, g) in glyphs.iter().enumerate() {
            if index % 256 == 0 { check()?; }
            ensure!((g.glyph_id as usize) < font.glyph_count(), "glyph id out of font range");
            let id = u16::try_from(g.glyph_id).context("glyph id exceeds font representation")?;
            positioned.push(Glyph {
                id: GlyphId(id),
                scale: PxScale::from(f32::from(size)),
                position: point(g.x_26_6 as f32 / 64., g.y_26_6 as f32 / 64.),
            });
        }
        Self::paint_cancellable(&font, canvas, &positioned, check)
    }
    fn paint(font: &FontArc, canvas: [u32; 2], glyphs: &[Glyph]) -> Result<Vec<u8>> {
        Self::paint_cancellable(font, canvas, glyphs, &|| Ok(()))
    }
    fn paint_cancellable(
        font: &FontArc,
        canvas: [u32; 2],
        glyphs: &[Glyph],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Vec<u8>> {
        check()?;
        let (w, h) = (canvas[0], canvas[1]);
        let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        for block in pixels.chunks_mut(256 * 1024) {
            check()?;
            for pixel in block.chunks_exact_mut(4) { pixel[..3].fill(255); }
        }
        let mut work = 0u64;
        for glyph in glyphs {
            check()?;
            if let Some(outline) = font.outline_glyph(glyph.clone()) {
                let bounds = outline.px_bounds();
                ensure!(bounds.min.x.is_finite() && bounds.min.y.is_finite()
                    && bounds.width().is_finite() && bounds.height().is_finite()
                    && bounds.width() >= 0. && bounds.height() >= 0., "invalid glyph outline bounds");
                let samples = (bounds.width().ceil() as u64)
                    .checked_mul(bounds.height().ceil() as u64).context("glyph outline size overflow")?;
                ensure!(samples <= 4 * 1024 * 1024, "glyph outline sample budget");
                work = work.checked_add(samples).context("glyph outline work overflow")?;
                ensure!(work <= 64 * 1024 * 1024, "glyph outline work budget");
                let mut last_row = u32::MAX;
                let mut cancelled = None;
                outline.draw(|gx, gy, coverage| {
                    if cancelled.is_some() { return; }
                    if gy != last_row {
                        last_row = gy;
                        if let Err(error) = check() { cancelled = Some(error); return; }
                    }
                    let px = bounds.min.x as i64 + i64::from(gx);
                    let py = bounds.min.y as i64 + i64::from(gy);
                    if px >= 0 && py >= 0 && px < i64::from(w) && py < i64::from(h) {
                        let i = ((py as usize) * (w as usize) + px as usize) * 4 + 3;
                        let alpha = (coverage.clamp(0., 1.) * 255.).round() as u16;
                        let old = u16::from(pixels[i]);
                        pixels[i] = (alpha + (old * (255 - alpha) + 127) / 255) as u8;
                    }
                });
                if let Some(error) = cancelled { return Err(error); }
            }
        }
        check()?;
        Ok(pixels)
    }

}
impl crysirror_core::service::TextMeasurement for TextSystem {
    fn ink_bounds_simple(&self,font:&str,size:f32,text:&str,bold:bool,underline:bool,strikeout:bool)->crysirror_core::Result<[i32;4]>{
        TextSystem::ink_bounds_simple(self,font,size,text,bold,underline,strikeout).map_err(|e|crysirror_core::Error::Io(e.to_string()))
    }
    fn position_simple(
        &self,
        font: &str,
        size: u16,
        text: &str,
    ) -> crysirror_core::Result<Vec<crysirror_core::service::SimpleTextPlacement>> {
        TextSystem::position_simple(self, font, size, text)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn measure_simple(
        &self,
        font: &str,
        size: f32,
        text: &str,
    ) -> crysirror_core::Result<crysirror_core::service::TextExtent> {
        TextSystem::measure_simple(self, font, size, text)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
impl TextService for TextSystem {
    fn layout(&mut self, text: &TextRun) -> crysirror_core::Result<Vec<PositionedGlyph>> {
        if !matches!(
            text.layout_profile.as_str(),
            SIMPLE_TEXT_LAYOUT | UNWRAPPED_TEXT_LAYOUT
        ) {
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
    fn simple_measurement_rejects_invalid_requests_before_font_lookup() {
        let system = TextSystem::default();
        assert!(
            system
                .measure_simple("none", f32::NAN, "a")
                .unwrap_err()
                .to_string()
                .contains("size")
        );
        assert!(
            system
                .measure_simple("none", 24., "a\nb")
                .unwrap_err()
                .to_string()
                .contains("single-line")
        );
    }
    #[test]
    #[ignore = "requires licensed runtime font in CRYSIRROR_TEST_FONT; no font bundled"]
    fn measurements_match_simple_glyph_positions_and_can_run_on_workers() {
        let mut system = TextSystem::default();
        system
            .add_font(
                "fixture",
                std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").unwrap()).unwrap(),
            )
            .unwrap();
        let text = TextRun {
            text: "AVATAR".into(),
            font: "fixture".into(),
            size_px: 24,
            bounds: crysirror_core::state::Rect {
                x: 0,
                y: 0,
                width: 512,
                height: 64,
            },
            color: crysirror_core::state::Color([255; 4]),
            layout_profile: SIMPLE_TEXT_LAYOUT.into(),
        };
        let glyphs = system.positioned(&text).unwrap();
        let last = glyphs.last().unwrap();
        let expected = last.position.x
            + system
                .font("fixture")
                .unwrap()
                .as_scaled(24.)
                .h_advance(last.id);
        let metrics = system.measure_simple("fixture", 24., &text.text).unwrap();
        assert!((metrics.advance - expected).abs() < 0.001);
        assert_eq!(
            system.measure_simple("fixture", 24., "").unwrap().advance,
            0.
        );
        let mut narrow = text.clone();
        narrow.bounds.width = 20;
        narrow.bounds.height = 128;
        let wrapped = system.positioned(&narrow).unwrap();
        assert!(wrapped.last().unwrap().position.y > wrapped[0].position.y);
        narrow.layout_profile = UNWRAPPED_TEXT_LAYOUT.into();
        let single_line = system.positioned(&narrow).unwrap();
        assert_eq!(single_line.len(), text.text.chars().count());
        assert!(
            single_line
                .iter()
                .all(|g| g.position.y == single_line[0].position.y)
        );
        assert!(single_line.last().unwrap().position.x > 20.);
        assert_eq!(system.rasterize(&narrow).unwrap().len(), 20 * 128 * 4);
        let handle: Arc<dyn crysirror_core::service::TextMeasurement> = Arc::new(system.clone());
        let measured =
            std::thread::spawn(move || handle.measure_simple("fixture", 24., "AVATAR").unwrap())
                .join()
                .unwrap();
        assert_eq!(metrics, measured);
        // A handle owns its font resource snapshot, not a pointer to a host.
        drop(system);
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
        let unwrapped = TextRun {
            text: "AV\tTo".into(),
            layout_profile: UNWRAPPED_TEXT_LAYOUT.into(),
            ..text
        };
        let positions = system
            .position_simple("fixture", 24, &unwrapped.text)
            .unwrap();
        let reference = system.positioned(&unwrapped).unwrap();
        assert_eq!(positions.len(), reference.len());
        for (actual, expected) in positions.iter().zip(reference) {
            assert_eq!(actual.baseline, [expected.position.x, expected.position.y]);
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

pub mod bitmap_font;

pub mod slot_color;

pub mod vn_slot;
pub mod byte_split;
pub mod byte_groups;

pub mod bitmap_raster;
pub mod line_viewport;
