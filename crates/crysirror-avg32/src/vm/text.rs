// SPDX-License-Identifier: MPL-2.0
//! CrySirror seam replacing upstream's glyph rasterisation into buffer 0.
//! Characters the original draws onto the display buffer are retained as
//! positioned glyphs and published as an engine GlyphRun layer. A glyph is
//! removed as soon as buffer 0 is overwritten where it stands (the bank's
//! write damage), which is when the native frame would have lost it.
use crysirror_image::avg32::Rect;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextGlyph {
    pub character: char,
    pub x: i32,
    pub baseline: i32,
    pub size: i32,
    pub colour: [u8; 3],
}

impl TextGlyph {
    fn intersects(&self, rect: &Rect) -> bool {
        // The character cell above the baseline (native cells do not overlap
        // the next row, which starts `font_y` lower).
        // Inset by 2px so a neighbouring row/cell edge is not counted.
        let inset = (self.size / 4).min(2);
        let (x1, y1) = (self.x + inset, self.baseline - self.size + 1 + inset);
        let (x2, y2) = (self.x + self.size - 1 - inset, self.baseline - inset);
        x1 <= rect.x2 && rect.x1 <= x2 && y1 <= rect.y2 && rect.y1 <= y2
    }
}

/// Glyphs currently standing on buffer 0, in drawing order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TextLayer {
    pub glyphs: Vec<TextGlyph>,
    /// Bumped on every change so the adapter republishes only when needed.
    pub revision: u64,
}

pub const MAX_GLYPHS: usize = 8192;

impl TextLayer {
    /// Drops glyphs under freshly written regions of buffer 0.
    pub fn apply_damage(&mut self, damage: &[Rect]) {
        if damage.is_empty() || self.glyphs.is_empty() {
            return;
        }
        let before = self.glyphs.len();
        self.glyphs
            .retain(|glyph| !damage.iter().any(|rect| glyph.intersects(rect)));
        if self.glyphs.len() != before {
            self.revision += 1;
        }
    }

    /// Native colour inversion of a region (choice highlight): the glyphs
    /// standing there stay and take the inverted colour, as their pixels did.
    pub fn invert(&mut self, rect: &Rect) {
        let mut changed = false;
        for glyph in &mut self.glyphs {
            // Pixel ops recolour a glyph whose cell centre is in the region.
            let (cx, cy) = (glyph.x + glyph.size / 2, glyph.baseline - glyph.size / 2);
            if (rect.x1..=rect.x2).contains(&cx) && (rect.y1..=rect.y2).contains(&cy) {
                glyph.colour = glyph.colour.map(|c| 255 - c);
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    pub fn push(&mut self, glyph: TextGlyph) {
        if self.glyphs.len() >= MAX_GLYPHS {
            self.glyphs.remove(0);
        }
        self.glyphs.push(glyph);
        self.revision += 1;
    }

    pub fn clear(&mut self) {
        if !self.glyphs.is_empty() {
            self.glyphs.clear();
            self.revision += 1;
        }
    }
}
