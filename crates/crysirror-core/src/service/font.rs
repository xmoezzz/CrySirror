//! Client-scoped asynchronous font resources, independent of text backends.
use crate::{path::VfsPath, state::ClientId, Result};
use alloc::{string::String, sync::Arc, vec::Vec};
/// Client-owned asynchronous registration ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontJob {
    /// Client capability namespace.
    pub owner: ClientId,
    /// Monotonic resource identifier, never a pointer.
    pub id: u64,
}
/// Client-owned immutable font face lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontHandle {
    /// Client capability namespace.
    pub owner: ClientId,
    /// Monotonic resource identifier, never a pointer.
    pub id: u64,
}
/// Input to a font worker. Storage paths use the caller's capability root.
#[derive(Clone)]
pub enum FontLoadSource {
    /// Read through the client capability on an I/O worker.
    Storage(VfsPath),
    /// Immutable caller bytes, charged before admission.
    Bytes(Arc<[u8]>),
    /// Fixed-size precomputed coverage face (currently TFT v0/v1), read on an I/O worker.
    PrecomputedStorage { path: VfsPath, pixel_size: u16 },
    /// Caller-owned precomputed bytes; pixel size is explicit because TFT has no size header.
    PrecomputedBytes { bytes: Arc<[u8]>, pixel_size: u16 },
}
/// A logical font binding and selected collection face.
#[derive(Clone)]
pub struct FontLoadSpec {
    /// Client-local binding name (reserved canonical prefix is forbidden).
    pub name: String,
    /// Capability path or immutable encoded source.
    pub source: FontLoadSource,
    /// Selected zero-based face index in a TTC/OTC source.
    pub face_index: u32,
}
/// Published font metadata. The canonical alias pins immutable content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontInfo {
    /// Lease used to explicitly release the registered face.
    pub handle: FontHandle,
    /// Caller-supplied client-local binding name.
    pub logical_alias: String,
    /// Immutable canonical alias for retained layers and shaped layouts.
    pub alias: String,
    /// Content and collection-face digest shared by rendering and measurement.
    pub digest: String,
    /// Stable publication generation; duplicate registration retains it.
    pub revision: u64,
    /// Unicode family, full-name and PostScript names recorded by the font.
    pub families: Vec<String>,
    /// Number of faces in the source collection (one for standalone fonts).
    pub collection_faces: u32,
    /// Selected zero-based face index in a TTC/OTC source.
    pub face_index: u32,
    /// Encoded source bytes; decoded backend storage has a separate host budget.
    pub byte_len: usize,
}
/// Worker progress. Ready means measurement and rendering can use the face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontLoadState {
    /// I/O or CPU worker has not completed publication.
    Pending,
    /// Atomically published in the shared client font catalog.
    Ready(FontInfo),
    /// Terminal worker or publication error. Release the ticket to reclaim it.
    Failed(String),
}
/// Bounded font registration capability. Handles cannot cross clients.
pub trait FontLibrary: Send + Sync {
    fn begin(&self, spec: FontLoadSpec) -> Result<FontJob>;
    fn status(&self, job: FontJob) -> Result<FontLoadState>;
    fn resolve(&self, name: &str) -> Result<Option<FontInfo>>;
    fn is_loaded(&self, alias: &str) -> Result<bool>;
    fn release_job(&self, job: FontJob) -> Result<()>;
    fn release_font(&self, handle: FontHandle) -> Result<()>;
}
