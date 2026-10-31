// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Immutable, bounded pre-rasterized glyph faces, usable without a VM or GPU.
//! TFT v0/v1 input layout and coverage runs follow the complete krkrz
//! fd5c4baa6a2ef5978db1bd043634351f48667daf visual/PrerenderedFont.cpp/.h.
//! Copyright W.Dee and contributors; notice: docs/third-party/krkrz.txt.
use anyhow::{ensure, Context, Result};
use std::{ops::Range, sync::Arc};

/// Maximum combined decoded glyph coverage for one precomputed face (64 MiB).
pub const MAX_PRECOMPUTED_COVERAGE: usize = 64 * 1024 * 1024;
/// Immutable bitmap glyph metrics; coverage is row-major, 0..=64.
#[derive(Clone, Debug)]
pub struct BitmapGlyph {
    pub character: char,
    pub size: [u16; 2],
    /// Left and top relative to a downward-positive baseline.
    pub bearing: [i32; 2],
    /// Pen displacement in the source's original pixel units.
    pub advance: [i16; 2],
    /// Native scalar measurement, distinct from the pen displacement.
    pub logical_advance: i16,
    pixels: Range<usize>,
}
/// A fixed-size face. Changing its size requires another explicit face binding.
pub struct PrecomputedFont {
    pixel_size: u16,
    glyphs: Vec<BitmapGlyph>,
    coverage: Vec<u8>,
    ascent: f32,
    descent: f32,
    retention: Option<Arc<dyn Send + Sync>>,
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .context("truncated TFT u16")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated TFT u32")?
            .try_into()?,
    ))
}
struct Header {
    version: u8,
    count: usize,
    characters: usize,
    records: usize,
}
fn header(bytes: &[u8]) -> Result<Header> {
    ensure!(
        bytes.len() >= 36 && bytes.len() <= 32 * 1024 * 1024,
        "TFT source byte limit/header"
    );
    ensure!(
        &bytes[..22] == b"TVP pre-rendered font\x1a",
        "invalid TFT signature"
    );
    ensure!(
        bytes[22] <= 1 && bytes[23] == 2,
        "unsupported TFT version/Unicode width"
    );
    let count = u32_at(bytes, 24)? as usize;
    let characters = u32_at(bytes, 28)? as usize;
    let records = u32_at(bytes, 32)? as usize;
    ensure!(count > 0 && count <= 65536, "TFT glyph count limit");
    ensure!(
        characters >= 36
            && characters
                .checked_add(count * 2)
                .is_some_and(|e| e <= bytes.len()),
        "TFT character index range"
    );
    ensure!(
        records >= 36
            && records
                .checked_add(count * 20)
                .is_some_and(|e| e <= bytes.len()),
        "TFT glyph index range"
    );
    Ok(Header {
        version: bytes[22],
        count,
        characters,
        records,
    })
}
impl PrecomputedFont {
    /// Validate table bounds and compute the reservation before glyph allocation.
    pub fn required_resident_bytes(bytes: &[u8], check: &dyn Fn() -> Result<()>) -> Result<usize> {
        check()?;
        let h = header(bytes)?;
        let mut pixels = 0usize;
        let mut previous = None;
        for i in 0..h.count {
            if i % 64 == 0 {
                check()?;
            }
            let ch = u16_at(bytes, h.characters + i * 2)?;
            ensure!(
                previous.is_none_or(|v| v < ch),
                "TFT character index must be strictly ordered"
            );
            previous = Some(ch);
            char::from_u32(u32::from(ch)).context("TFT index contains a surrogate")?;
            let at = h.records + i * 20;
            let width = usize::from(u16_at(bytes, at + 4)?);
            let height = usize::from(u16_at(bytes, at + 6)?);
            let n = width
                .checked_mul(height)
                .context("TFT glyph dimensions overflow")?;
            ensure!(n <= 4 * 1024 * 1024, "TFT glyph coverage limit");
            pixels = pixels.checked_add(n).context("TFT coverage sum overflow")?;
            ensure!(
                pixels <= MAX_PRECOMPUTED_COVERAGE,
                "TFT decoded coverage budget"
            );
            ensure!(
                n == 0 || (u32_at(bytes, at)? as usize) < bytes.len(),
                "TFT glyph source offset"
            );
        }
        check()?;
        pixels
            .checked_add(h.count * std::mem::size_of::<BitmapGlyph>())
            .context("TFT resident size overflow")
    }
    /// Decode immutable fixed-size coverage on a CPU worker after reservation.
    pub fn from_tft(bytes: &[u8], pixel_size: u16, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        ensure!(
            (1..=512).contains(&pixel_size),
            "precomputed font pixel size"
        );
        let resident = Self::required_resident_bytes(bytes, check)?;
        let h = header(bytes)?;
        let mut glyphs = Vec::with_capacity(h.count);
        let mut pixels =
            Vec::with_capacity(resident - h.count * std::mem::size_of::<BitmapGlyph>());
        let (mut ascent, mut descent) = (0f32, 0f32);
        for i in 0..h.count {
            check()?;
            let at = h.records + i * 20;
            let character = char::from_u32(u32::from(u16_at(bytes, h.characters + i * 2)?))
                .context("TFT scalar")?;
            let size = [u16_at(bytes, at + 4)?, u16_at(bytes, at + 6)?];
            let origin = [
                u16_at(bytes, at + 8)? as i16,
                u16_at(bytes, at + 10)? as i16,
            ];
            let bearing_y = -i32::from(origin[1]);
            let start = pixels.len();
            let end = start + usize::from(size[0]) * usize::from(size[1]);
            let mut pos = u32_at(bytes, at)? as usize;
            let mut next_check = start;
            while pixels.len() < end {
                if pixels.len() >= next_check {
                    check()?;
                    next_check = pixels.len() + 65536;
                }
                let value = *bytes.get(pos).context("truncated TFT glyph coverage")?;
                pos += 1;
                if (h.version == 0 && value == 0x41) || (h.version == 1 && value >= 0x41) {
                    let last = *pixels
                        .get(
                            pixels
                                .len()
                                .checked_sub(1)
                                .filter(|p| *p >= start)
                                .context("TFT run without preceding coverage")?,
                        )
                        .context("TFT run coverage")?;
                    let count = if h.version == 0 {
                        let n = *bytes.get(pos).context("truncated TFT run")?;
                        pos += 1;
                        usize::from(n)
                    } else {
                        usize::from(value - 0x40)
                    };
                    ensure!(
                        count <= end - pixels.len(),
                        "TFT coverage run exceeds glyph dimensions"
                    );
                    pixels.resize(pixels.len() + count, last);
                } else {
                    ensure!(value <= 64, "TFT literal coverage outside 0..64");
                    pixels.push(value);
                }
            }
            ascent = ascent.max(f32::from(origin[1]));
            descent = descent.min(f32::from(origin[1]) - f32::from(size[1]));
            glyphs.push(BitmapGlyph {
                character,
                size,
                bearing: [i32::from(origin[0]), bearing_y],
                advance: [
                    u16_at(bytes, at + 12)? as i16,
                    u16_at(bytes, at + 14)? as i16,
                ],
                logical_advance: u16_at(bytes, at + 16)? as i16,
                pixels: start..end,
            });
        }
        check()?;
        Ok(Self {
            pixel_size,
            glyphs,
            coverage: pixels,
            ascent,
            descent,
            retention: None,
        })
    }
    pub(crate) fn retain_storage(&mut self, lease: Arc<dyn Send + Sync>) {
        self.retention = Some(lease);
    }
    pub fn pixel_size(&self) -> u16 {
        self.pixel_size
    }
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }
    pub fn ascent(&self) -> f32 {
        self.ascent
    }
    pub fn descent(&self) -> f32 {
        self.descent
    }
    /// Stable one-based glyph IDs, with zero reserved for a missing character.
    pub fn glyph_id(&self, character: char) -> Option<u32> {
        self.glyphs
            .binary_search_by_key(&character, |g| g.character)
            .ok()
            .map(|i| i as u32 + 1)
    }
    pub fn glyph(&self, id: u32) -> Result<&BitmapGlyph> {
        self.glyphs
            .get(id.checked_sub(1).context("zero precomputed glyph id")? as usize)
            .context("precomputed glyph id out of range")
    }
    pub fn coverage(&self, id: u32) -> Result<&[u8]> {
        let g = self.glyph(id)?;
        Ok(&self.coverage[g.pixels.clone()])
    }
    pub fn check_size(&self, size: f32) -> Result<()> {
        ensure!(
            size.is_finite() && (size - f32::from(self.pixel_size)).abs() < 0.001,
            "precomputed font requires its declared pixel size"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Authored 2x2 'A': half-coverage then a run of three more pixels.
    pub(super) fn fixture(version: u8) -> Vec<u8> {
        let mut b = b"TVP pre-rendered font\x1a".to_vec();
        b.extend([version, 2]);
        b.extend(1u32.to_le_bytes());
        b.extend(36u32.to_le_bytes());
        b.extend(38u32.to_le_bytes());
        b.extend((b'A' as u16).to_le_bytes());
        b.extend(58u32.to_le_bytes());
        for v in [2i16, 2, -1, 2, 3, 0, 4, 0] {
            b.extend(v.to_le_bytes());
        }
        b.extend(if version == 0 {
            vec![32, 0x41, 3]
        } else {
            vec![32, 0x43]
        });
        b
    }
    #[test]
    fn both_versions_keep_metrics_and_exact_coverage() {
        for v in [0, 1] {
            let b = fixture(v);
            let f = PrecomputedFont::from_tft(&b, 24, &|| Ok(())).unwrap();
            assert_eq!(f.coverage(f.glyph_id('A').unwrap()).unwrap(), [32; 4]);
            let g = f.glyph(1).unwrap();
            assert_eq!(g.bearing, [-1, -2]);
            assert_eq!(g.advance, [3, 0]);
            assert_eq!(g.logical_advance, 4);
            assert!(f.check_size(23.).is_err());
            assert!(f.glyph_id('B').is_none());
        }
    }
    #[test]
    fn malformed_ranges_runs_versions_and_cancellation_fail() {
        let good = fixture(1);
        let mut cases = Vec::new();
        let mut b = good.clone();
        b[58] = 0x43;
        cases.push(b);
        let mut b = good.clone();
        b[59] = 0x44;
        cases.push(b);
        let mut b = good.clone();
        b.truncate(59);
        cases.push(b);
        let mut b = good.clone();
        b[22] = 2;
        cases.push(b);
        let mut b = good.clone();
        b[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(b);
        for b in cases {
            assert!(PrecomputedFont::from_tft(&b, 24, &|| Ok(())).is_err());
        }
        assert!(PrecomputedFont::from_tft(&good, 24, &|| anyhow::bail!("cancelled")).is_err());
    }
}

impl PrecomputedFont {
    pub(crate) fn measure_simple(
        &self,
        size: f32,
        text: &str,
    ) -> Result<crysirror_core::service::TextExtent> {
        self.check_size(size)?;
        let mut advance = 0f32;
        for ch in text.chars() {
            let tab = ch == '\t';
            let ch = if tab { ' ' } else { ch };
            let id = self
                .glyph_id(ch)
                .with_context(|| format!("precomputed font missing U+{:04X}", ch as u32))?;
            advance += f32::from(self.glyph(id)?.logical_advance) * if tab { 4. } else { 1. };
        }
        Ok(crysirror_core::service::TextExtent {
            advance,
            line_height: f32::from(self.pixel_size),
            ascent: self.ascent,
            descent: self.descent,
        })
    }
    pub(crate) fn position_simple(
        &self,
        text: &str,
    ) -> Result<Vec<crysirror_core::service::SimpleTextPlacement>> {
        let mut out = Vec::new();
        let (mut x, mut y) = (0f32, self.ascent);
        for ch in text.chars() {
            if ch == '\t' {
                let id = self
                    .glyph_id(' ')
                    .context("precomputed font missing space")?;
                x += f32::from(self.glyph(id)?.advance[0]) * 4.;
                continue;
            }
            let id = self
                .glyph_id(ch)
                .with_context(|| format!("precomputed font missing U+{:04X}", ch as u32))?;
            let g = self.glyph(id)?;
            out.push(crysirror_core::service::SimpleTextPlacement {
                character: ch,
                baseline: [x, y],
            });
            x += f32::from(g.advance[0]);
            y += f32::from(g.advance[1]);
        }
        Ok(out)
    }
    pub(crate) fn layout(
        &self,
        text: &crysirror_core::state::TextRun,
    ) -> Result<Vec<crysirror_core::service::Glyph>> {
        self.check_size(f32::from(text.size_px))?;
        ensure!(
            text.text.len() <= 1024 * 1024,
            "precomputed text byte limit"
        );
        let (mut x, mut y) = (0f32, self.ascent);
        let mut out = Vec::new();
        for ch in text.text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                x = 0.;
                y += f32::from(self.pixel_size);
                continue;
            }
            let id = self
                .glyph_id(if ch == '\t' { ' ' } else { ch })
                .with_context(|| format!("precomputed font missing U+{:04X}", ch as u32))?;
            let g = self.glyph(id)?;
            if ch == '\t' {
                x += f32::from(g.advance[0]) * 4.;
                continue;
            }
            if text.layout_profile == crysirror_core::service::SIMPLE_TEXT_LAYOUT
                && x > 0.
                && x + f32::from(g.advance[0]) > text.bounds.width as f32
            {
                x = 0.;
                y += f32::from(self.pixel_size);
            }
            if y - self.ascent >= text.bounds.height as f32 {
                break;
            }
            out.push(crysirror_core::service::Glyph {
                glyph_id: id,
                x_26_6: (x * 64.).round() as i32,
                y_26_6: (y * 64.).round() as i32,
                advance_26_6: i32::from(g.advance[0]) * 64,
            });
            x += f32::from(g.advance[0]);
            y += f32::from(g.advance[1]);
        }
        Ok(out)
    }
    pub(crate) fn cell(
        &self,
        request: crate::glyph::GlyphRaster,
    ) -> Result<crate::glyph::GlyphCell> {
        self.check_size(request.size)?;
        let g = self.glyph(request.glyph_id)?;
        Ok(crate::glyph::GlyphCell {
            origin: [
                request.baseline[0].floor() as i32 + g.bearing[0],
                request.baseline[1].floor() as i32 + g.bearing[1],
            ],
            width: u32::from(g.size[0]),
            height: u32::from(g.size[1]),
            coverage: self
                .coverage(request.glyph_id)?
                .iter()
                .map(|v| f32::from(*v) / 64.)
                .collect(),
        })
    }
    pub(crate) fn paint(
        &self,
        canvas: [u32; 2],
        glyphs: &[crysirror_core::service::Glyph],
    ) -> Result<Vec<u8>> {
        ensure!(
            canvas[0] > 0
                && canvas[1] > 0
                && u64::from(canvas[0]) * u64::from(canvas[1]) <= 4 * 1024 * 1024,
            "precomputed text surface budget"
        );
        let mut rgba = vec![0u8; canvas[0] as usize * canvas[1] as usize * 4];
        for p in rgba.chunks_exact_mut(4) {
            p[..3].fill(255);
        }
        for glyph in glyphs {
            let cell = self.cell(crate::glyph::GlyphRaster {
                glyph_id: glyph.glyph_id,
                size: f32::from(self.pixel_size),
                baseline: [glyph.x_26_6 as f32 / 64., glyph.y_26_6 as f32 / 64.],
            })?;
            for y in 0..cell.height {
                let py = cell.origin[1] + y as i32;
                if py < 0 || py >= canvas[1] as i32 {
                    continue;
                }
                for x in 0..cell.width {
                    let px = cell.origin[0] + x as i32;
                    if px < 0 || px >= canvas[0] as i32 {
                        continue;
                    }
                    let a = (cell.coverage[(y * cell.width + x) as usize] * 255.).round() as u16;
                    let index = (py as usize * canvas[0] as usize + px as usize) * 4 + 3;
                    let old = u16::from(rgba[index]);
                    rgba[index] = (a + (old * (255 - a) + 127) / 255) as u8;
                }
            }
        }
        Ok(rgba)
    }
}
