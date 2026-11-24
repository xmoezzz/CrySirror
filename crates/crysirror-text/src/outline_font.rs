// SPDX-License-Identifier: MPL-2.0
//! Outline and shaping consumers borrow the same immutable font bytes.
use ab_glyph::{Font, FontArc, FontRef, GlyphId, InvalidFont, Outline};
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, Mutex, OnceLock, Weak},
};

pub(crate) struct FontBacking {
    bytes: Arc<EncodedFont>,
    reservation: Mutex<Option<Arc<dyn Send + Sync>>>,
}

// Weak catalog entries retain this small allocation. The actual font buffer
// drops when its last strong owner leaves, even while the catalog entry exists.
struct EncodedFont(Arc<[u8]>);

impl FontBacking {
    pub(crate) fn new(bytes: Arc<[u8]>) -> Arc<Self> {
        static FONTS: OnceLock<Mutex<HashMap<[u8; 32], Weak<EncodedFont>>>> = OnceLock::new();
        let digest = *blake3::hash(&bytes).as_bytes();
        let mut fonts = FONTS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bytes = fonts
            .get(&digest)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                if fonts.len() >= 256 {
                    fonts.retain(|_, entry| entry.strong_count() > 0);
                }
                let bytes = Arc::new(EncodedFont(bytes));
                fonts.insert(digest, Arc::downgrade(&bytes));
                bytes
            });
        Arc::new(Self {
            bytes,
            reservation: Mutex::new(None),
        })
    }

    pub(crate) fn retain_storage(&self, lease: Arc<dyn Send + Sync>) {
        let mut reservation = self.reservation.lock().unwrap_or_else(|e| e.into_inner());
        *reservation = Some(lease);
    }
}

impl Deref for FontBacking {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes.0
    }
}

self_cell::self_cell! {
    struct OutlineFont {
        owner: Arc<FontBacking>,

        #[covariant]
        dependent: FontRef,
    }
}

pub(crate) fn font_arc(backing: Arc<FontBacking>, index: u32) -> Result<FontArc, InvalidFont> {
    OutlineFont::try_new(backing, |bytes| {
        FontRef::try_from_slice_and_index(bytes, index)
    })
    .map(FontArc::new)
}

impl Font for OutlineFont {
    fn units_per_em(&self) -> Option<f32> {
        self.borrow_dependent().units_per_em()
    }
    fn ascent_unscaled(&self) -> f32 {
        self.borrow_dependent().ascent_unscaled()
    }
    fn descent_unscaled(&self) -> f32 {
        self.borrow_dependent().descent_unscaled()
    }
    fn line_gap_unscaled(&self) -> f32 {
        self.borrow_dependent().line_gap_unscaled()
    }
    fn italic_angle(&self) -> f32 {
        self.borrow_dependent().italic_angle()
    }
    fn glyph_id(&self, c: char) -> GlyphId {
        self.borrow_dependent().glyph_id(c)
    }
    fn h_advance_unscaled(&self, id: GlyphId) -> f32 {
        self.borrow_dependent().h_advance_unscaled(id)
    }
    fn h_side_bearing_unscaled(&self, id: GlyphId) -> f32 {
        self.borrow_dependent().h_side_bearing_unscaled(id)
    }
    fn v_advance_unscaled(&self, id: GlyphId) -> f32 {
        self.borrow_dependent().v_advance_unscaled(id)
    }
    fn v_side_bearing_unscaled(&self, id: GlyphId) -> f32 {
        self.borrow_dependent().v_side_bearing_unscaled(id)
    }
    fn kern_unscaled(&self, first: GlyphId, second: GlyphId) -> f32 {
        self.borrow_dependent().kern_unscaled(first, second)
    }
    fn outline(&self, id: GlyphId) -> Option<Outline> {
        self.borrow_dependent().outline(id)
    }
    fn glyph_count(&self) -> usize {
        self.borrow_dependent().glyph_count()
    }
    fn codepoint_ids(&self) -> ab_glyph::CodepointIdIter<'_> {
        self.borrow_dependent().codepoint_ids()
    }
    fn glyph_raster_image2(
        &self,
        id: GlyphId,
        pixel_size: u16,
    ) -> Option<ab_glyph::v2::GlyphImage<'_>> {
        self.borrow_dependent().glyph_raster_image2(id, pixel_size)
    }
    fn glyph_svg_image(&self, id: GlyphId) -> Option<ab_glyph::GlyphSvg<'_>> {
        self.borrow_dependent().glyph_svg_image(id)
    }
    fn font_data(&self) -> &[u8] {
        self.borrow_owner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_shares_live_bytes_without_retaining_released_fonts() {
        let bytes: Arc<[u8]> = Arc::from(&b"unique test font bytes"[..]);
        let source = Arc::downgrade(&bytes);
        let face = FontBacking::new(bytes);
        let alias = FontBacking::new(Arc::from(&b"unique test font bytes"[..]));
        assert!(Arc::ptr_eq(&face.bytes, &alias.bytes));

        let reservation = Arc::new(());
        let lease = Arc::downgrade(&reservation);
        face.retain_storage(reservation);
        let reader = face.clone();
        drop(face);
        assert!(lease.upgrade().is_some());
        assert!(source.upgrade().is_some());
        drop(reader);
        assert!(lease.upgrade().is_none());
        assert!(source.upgrade().is_some());
        drop(alias);
        assert!(source.upgrade().is_none());
    }
}
