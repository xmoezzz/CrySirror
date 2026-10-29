// SPDX-License-Identifier: MPL-2.0
//! AVG32 raster device model: 640x480 RGB+mask PDT buffers, the native copy,
//! mask, colour-key, fade, stretch and get/put primitives, and every native
//! screen transition. Copied from siglus_rs `crates/avg32/src/{buffer,pdtmgr,
//! effect}.rs` (commit 5f358ef1360b0439e92aed2b749e8f25d8721954, MPL-2.0).
//!
//! The interpreter, buffer-number policy beyond the bank itself, text,
//! presentation and audio stay with the caller. CrySirror additions are
//! limited to buffer-0 write damage (`PdtManager::mark_damage/take_damage`).
pub mod bank;
pub mod buffer;
pub mod effect;

pub use bank::{Blit, PdtManager, Rect};
pub use buffer::PdtBuffer;
pub use effect::{Effect, EffectRunner};
