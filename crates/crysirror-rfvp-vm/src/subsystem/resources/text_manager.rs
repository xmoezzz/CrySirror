// SPDX-License-Identifier: MPL-2.0
//! RFVP font-ID, read-flag and save mapping over engine-owned slot text.
use crate::{font::Font, path::Path};
use alloc::{
    string::{String, ToString},
    vec,
    vec::Vec,
};
use anyhow::{bail, Result};
use crysirror_text::vn_slot::{FontFallbackSet, SlotFonts};
pub use crysirror_text::vn_slot::{
    SlotTextItem as TextItem, SlotTextItemSnapshotV1 as TextItemSnapshotV1, TextSlotSurfaceInfo,
    TextSlotSurfaceUpdate,
};
use serde::{Deserialize, Serialize};
pub const FONTFACE_CURRENT: i32 = -1;
pub const FONTFACE_MS_GOTHIC: i32 = -2;
pub const FONTFACE_MS_MINCHO: i32 = -3;
pub const FONTFACE_MS_PGOTHIC: i32 = -4;
pub const FONTFACE_MS_PMINCHO: i32 = -5;
#[cfg(feature = "no_std")]
pub struct FontEnumerator {
    default_font: Option<Font>,
    system_fontface_id: i32,
    current_font_name: String,
}

#[cfg(feature = "no_std")]
impl Default for FontEnumerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "no_std")]
impl FontEnumerator {
    pub fn new() -> Self {
        Self {
            default_font: None,
            system_fontface_id: FONTFACE_MS_GOTHIC,
            current_font_name: "default font".to_string(),
        }
    }

    pub fn from_default_font(default_font: Font) -> Self {
        Self {
            default_font: Some(default_font),
            system_fontface_id: FONTFACE_MS_GOTHIC,
            current_font_name: "default font".to_string(),
        }
    }

    pub fn set_default_font(&mut self, default_font: Font) {
        self.default_font = Some(default_font);
        self.current_font_name = "default font".to_string();
        self.system_fontface_id = FONTFACE_MS_GOTHIC;
    }

    pub fn set_system_font_fallback_enabled(&mut self, _enabled: bool) {}

    pub fn load_system_fallback_fonts(&mut self) {}

    pub fn add_font_file(&mut self, _path: &Path) -> Option<i32> {
        None
    }

    pub fn set_forced_font(&mut self, _id: Option<i32>) {}

    pub fn forced_font_id(&self) -> Option<i32> {
        None
    }

    pub fn init_fontface(&mut self) -> Result<()> {
        if self.default_font.is_some() {
            Ok(())
        } else {
            bail!("Required font file default.ttf was not found in the game directory.")
        }
    }

    pub fn set_current_font_name(&mut self, _name: &str) {
        self.current_font_name = "default font".to_string();
    }

    pub fn get_current_font_name(&self) -> &str {
        "default font"
    }

    pub fn get_font(&self, id: i32) -> Font {
        if !self.is_valid_fontface_id(id) {
            return self
                .default_font
                .as_ref()
                .expect("no_std default.ttf must be initialized before text rendering")
                .clone();
        }
        self.default_font
            .as_ref()
            .expect("no_std default.ttf must be initialized before text rendering")
            .clone()
    }

    fn get_font_fallback_set(&self, id: i32) -> FontFallbackSet {
        FontFallbackSet::new(self.get_font(id), Vec::new(), Vec::new())
    }

    pub fn get_font_name(&self, id: i32) -> Option<String> {
        if self.is_valid_fontface_id(id) {
            Some("default font".to_string())
        } else {
            None
        }
    }

    pub fn get_font_count(&self) -> i32 {
        0
    }

    pub fn is_valid_fontface_id(&self, id: i32) -> bool {
        matches!(
            id,
            FONTFACE_CURRENT
                | FONTFACE_MS_GOTHIC
                | FONTFACE_MS_MINCHO
                | FONTFACE_MS_PGOTHIC
                | FONTFACE_MS_PMINCHO
        )
    }

    pub fn get_system_fontface_id(&self) -> i32 {
        self.system_fontface_id
    }

    pub fn set_system_fontface_id(&mut self, id: i32) {
        if self.is_valid_fontface_id(id) {
            self.system_fontface_id = id;
        }
    }
}

impl SlotFonts for FontEnumerator {
    fn get_font_fallback_set(&self, id: i32) -> FontFallbackSet {
        FontEnumerator::get_font_fallback_set(self, id)
    }
}
pub struct TextManager {
    bank: crysirror_text::vn_slot::SlotTextBank,
    pub readed_text: Vec<u32>,
}
impl Default for TextManager {
    fn default() -> Self {
        Self::new()
    }
}
impl core::ops::Deref for TextManager {
    type Target = crysirror_text::vn_slot::SlotTextBank;
    fn deref(&self) -> &Self::Target {
        &self.bank
    }
}
impl core::ops::DerefMut for TextManager {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bank
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextManagerSnapshotV1 {
    pub items: Vec<TextItemSnapshotV1>,
    pub readed_text: Vec<u32>,
}
impl TextManager {
    pub fn new() -> Self {
        Self {
            bank: Default::default(),
            readed_text: vec![0; 0x800000 / 32],
        }
    }
    pub fn mark_readed_text_first(&mut self, addr: u32) -> bool {
        if addr >= 0x800000 {
            return false;
        }
        #[cfg(feature = "psx")]
        let addr = addr >> 3;
        let idx = (addr / 32) as usize;
        let bit = addr % 32;
        let mask = 1u32 << bit;
        #[cfg(feature = "psx")]
        if idx >= self.readed_text.len() {
            self.readed_text.resize(idx + 1, 0);
        }
        let prev = self.readed_text[idx] & mask;
        self.readed_text[idx] |= mask;
        prev == 0
    }

    pub fn capture_snapshot_v1(&self) -> TextManagerSnapshotV1 {
        TextManagerSnapshotV1 {
            items: self.bank.capture_snapshot_v1().items,
            readed_text: self.readed_text.clone(),
        }
    }
    pub fn apply_snapshot_v1(&mut self, snap: &TextManagerSnapshotV1) {
        self.bank
            .apply_snapshot_v1(&crysirror_text::vn_slot::SlotTextBankSnapshotV1 {
                items: snap.items.clone(),
            });
        self.readed_text = snap.readed_text.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_text_save_envelope_keeps_read_flags_separate_from_engine_bank() {
        let mut native = TextManager::new();
        native.set_text_buff(0, 320, 100);
        native.set_text_content(0, "[reading|base] engine text");
        assert!(native.mark_readed_text_first(127));
        assert!(!native.mark_readed_text_first(127));
        let original = native.capture_snapshot_v1();
        let bytes = postcard::to_allocvec(&original).unwrap();
        let decoded: TextManagerSnapshotV1 = postcard::from_bytes(&bytes).unwrap();
        let mut restored = TextManager::new();
        restored.apply_snapshot_v1(&decoded);
        assert!(!restored.mark_readed_text_first(127));
        assert!(restored.mark_readed_text_first(128));
        // The engine declaration carries text/reveal state; the native bitmap
        // remains a second, unchanged field in the original save envelope.
        assert_eq!(
            restored.capture_snapshot_v1().items.len(),
            original.items.len()
        );
        let mut rebuilt = original.items.clone();
        // Render caches are derived: every restored slot needs a new surface.
        for item in &mut rebuilt {
            item.dirty = true;
        }
        assert_eq!(
            postcard::to_allocvec(&restored.bank.capture_snapshot_v1().items).unwrap(),
            postcard::to_allocvec(&rebuilt).unwrap()
        );
    }
}
