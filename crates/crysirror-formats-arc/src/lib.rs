// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit GPL archive plugins translated from arc_unpacker. This is not a CLI
//! wrapper. Archive storage, budgets, transforms, keys and mounts are shared APIs.
#![forbid(unsafe_code)]
mod common;
mod nscripter;
mod simple;
mod xp3;
pub mod images_b;
pub mod images_c;
pub mod images_a;
pub mod images_d;
mod images;
pub use images::{register_images,image_format_ids};
pub use nscripter::{FilenameEncoding, Nscripter};
pub use simple::{BgiArc, MajiroArc};
pub use xp3::Xp3;

/// Only these concrete Rust decoders are installed. The upstream format inventory
/// includes many pending formats and is not used to claim or register support.
pub fn register(registry: &mut crysirror_resource::FormatRegistry) -> anyhow::Result<()> {
    registry.register(std::sync::Arc::new(BgiArc))?;
    registry.register(std::sync::Arc::new(MajiroArc))?;
    registry.register(std::sync::Arc::new(Nscripter::sar()))?;
    registry.register(std::sync::Arc::new(Nscripter::nsa()))?;
    registry.register(std::sync::Arc::new(Xp3))?;
    Ok(())
}
