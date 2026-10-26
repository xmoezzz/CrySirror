// SPDX-License-Identifier: MPL-2.0
//! Retained fallback state. Exact native layout is required by the adapter.
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct UiRuntime {
    pub message: Option<String>,
    pub name: Option<String>,
    pub reveal_start: Option<crate::platform_time::Instant>,
    pub visible_chars: usize,
    pub window: Option<crate::message::MessageWindow>,
    pub cursor: crysirror_text::grid::CellCursor,
    pub glyphs: Vec<crysirror_text::grid::CellGlyph>,
    pub name_glyphs: Vec<crysirror_text::grid::CellGlyph>,
    pub reveal: Option<crysirror_scene::layer::GlyphReveal>,
    pub published_message: bool,
}
impl UiRuntime {
    pub fn message_waiting(&self) -> bool {
        self.reveal
            .as_ref()
            .is_some_and(|reveal| !reveal.status(self.glyphs.len()).complete)
    }
    pub fn message_visible_chars(&self) -> usize {
        self.visible_chars
    }
    pub fn message_wait_message_len(&self) -> usize {
        self.message.as_deref().unwrap_or("").chars().count()
    }
    pub fn message_text(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub fn set_message(&mut self, msg: String) {
        let new = if msg.is_empty() { None } else { Some(msg) };
        if self.message != new {
            self.message = new;
            self.visible_chars = 0;
            self.reveal_start = Some(crate::platform_time::Instant::now());
        }
    }
    pub fn set_name(&mut self, name: String) {
        self.name = if name.is_empty() { None } else { Some(name) }
    }
}
