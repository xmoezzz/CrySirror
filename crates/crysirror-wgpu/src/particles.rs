// SPDX-License-Identifier: MPL-2.0
//! Particle drawing is a VFX renderer; simulation belongs to crysirror-vfx.
//! Kept as a source-path migration shim for Rust renderer users.
pub use crate::vfx_render::{GpuEmitterDraw, RenderStats, VfxRenderer as Particles, VfxView};
pub const MAX_LIVE: usize = crate::vfx_render::MAX_RENDER_PARTICLES;
