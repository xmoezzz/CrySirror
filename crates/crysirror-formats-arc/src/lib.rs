// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit GPL archive plugins translated from arc_unpacker. This is not a CLI
//! wrapper. Archive storage, budgets, transforms, keys and mounts are shared APIs.
#![forbid(unsafe_code)]
mod audio;
mod common;
mod files;
pub use audio::{audio_format_ids, register_audio};
pub use files::{file_format_ids, register_files};
mod images;
pub mod images_a;
pub mod images_b;
pub mod images_c;
pub mod images_d;
mod legacy;
mod nscripter;
mod simple;
mod xp3;
pub use images::{image_format_ids, register_images};
pub use nscripter::{FilenameEncoding, Nscripter};
pub use simple::{BgiArc, MajiroArc};
pub use xp3::Xp3;

/// Install concrete archive parsers. The reviewed inventory records native
/// variant/evidence limits separately; an ID never substitutes for a parser.
pub fn register(registry: &mut crysirror_resource::FormatRegistry) -> anyhow::Result<()> {
    registry.register(std::sync::Arc::new(BgiArc))?;
    registry.register(std::sync::Arc::new(MajiroArc))?;
    registry.register(std::sync::Arc::new(Nscripter::sar()))?;
    registry.register(std::sync::Arc::new(Nscripter::nsa()))?;
    registry.register(std::sync::Arc::new(Xp3))?;
    for format in legacy::formats() {
        registry.register(std::sync::Arc::new(LegacyFormat(format)))?;
    }
    Ok(())
}
/// IDs of the table-driven legacy archive plugins installed by `register`.
pub fn legacy_archive_ids() -> Vec<&'static str> {
    legacy::formats().map(|f| f.id).collect()
}
/// Look up one legacy archive plugin by its upstream decoder ID.
pub fn legacy_archive(id: &str) -> Option<std::sync::Arc<dyn crysirror_resource::ArchiveFormat>> {
    legacy::formats()
        .find(|f| f.id == id)
        .map(|f| std::sync::Arc::new(LegacyFormat(f)) as _)
}
struct LegacyFormat(&'static legacy::Legacy);
impl crysirror_resource::ArchiveFormat for LegacyFormat {
    fn id(&self) -> &'static str {
        self.0.id
    }
    fn probe(
        &self,
        source: &dyn crysirror_core::service::AssetReader,
    ) -> anyhow::Result<crysirror_resource::Probe> {
        self.0.probe(source)
    }
    fn open(
        &self,
        source: std::sync::Arc<dyn crysirror_core::service::AssetReader>,
        context: &crysirror_resource::DecodeContext,
    ) -> anyhow::Result<Box<dyn crysirror_resource::Archive>> {
        self.0.open(source, context)
    }
}
