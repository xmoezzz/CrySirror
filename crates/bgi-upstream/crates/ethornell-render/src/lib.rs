//! CPU-only font and draw data. CrySirror owns presentation; no GPU backend is linked.
mod font;
mod shared;
pub use font::FontProvider;
pub(crate) use shared::SYSTEM_CJK_FONT_CANDIDATES;
pub use shared::{PresentationScale,RenderCommand,TextGlow,TextStyleSpan,TextureHandle,load_system_cjk_font,load_text_font};
pub type TextureId=u64;
