// SPDX-License-Identifier: MPL-2.0
//! Existing renderer-facing name for the engine-owned CPU text service.
//! No layout or rasterization implementation lives in this GPU crate.
pub use crysirror_text::{TextSystem, TextSystem as TextRasterizer};
