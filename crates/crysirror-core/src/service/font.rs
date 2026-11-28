//! Client-scoped asynchronous font resources, independent of text backends.
use crate::{Result, path::VfsPath, state::ClientId};
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
/// Actual outline face style prepared from its OS/2 metadata. Missing metadata
/// and precomputed-only faces return None, never a guessed regular weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontFaceStyle {
    pub weight: u16,
    pub italic: bool,
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
/// One glyph outline request, independent of shaping or native text syntax.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphBoundsRequest {
    pub character: char,
    pub size: f32,
    pub baseline: [f32; 2],
}
#[derive(Clone, Debug)]
pub struct GlyphBoundsSpec {
    pub font: String,
    pub glyphs: Vec<GlyphBoundsRequest>,
}
/// Bounds of the same coverage cell used by the engine glyph rasterizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphBounds {
    pub glyph_id: u32,
    pub origin: [i32; 2],
    pub size: [u32; 2],
}
/// Immutable scoped worker result. Keeping this resource retains accounting
/// and the admitted font snapshot; it is not a project or device pointer.
pub trait GlyphBoundsResource: Send + Sync {
    fn client(&self) -> ClientId;
    fn font(&self) -> &str;
    fn digest(&self) -> &str;
    fn glyphs(&self) -> &[GlyphBounds];
}
#[derive(Clone)]
pub enum GlyphBoundsProgress {
    Pending,
    Ready(Arc<dyn GlyphBoundsResource>),
    Failed(crate::Error),
    Cancelled,
}
/// Owner-polled completion; cancel/drop never joins a font worker.
pub trait GlyphBoundsQuery: Send {
    fn poll(&mut self) -> Result<GlyphBoundsProgress>;
    fn cancel(&mut self) -> Result<()>;
}
/// Outline design metrics in pixels at a requested em size.
#[derive(Clone, Copy, Debug)]
pub struct OutlineEmMetrics {
    /// Scale used by the engine's line-height-based rasterizer.
    pub line_height: f32,
    pub ascent: f32,
    /// OS/2 sTypoAscender, or the face ascent when absent.
    pub typographic_ascent: f32,
}

/// One variable-font design axis (OpenType `fvar`), in user coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct FontVariationAxis {
    pub tag: [u8; 4],
    /// Axis name from the font's `name` table (the tag when absent).
    pub name: String,
    pub minimum: f32,
    pub default: f32,
    pub maximum: f32,
}
/// A named instance: one coordinate per axis, in axis order.
#[derive(Clone, Debug, PartialEq)]
pub struct FontNamedInstance {
    pub name: String,
    pub coordinates: Vec<f32>,
}
/// Variation data of a face; both lists are empty for static fonts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontVariations {
    pub axes: Vec<FontVariationAxis>,
    pub instances: Vec<FontNamedInstance>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
}
/// OpenType shaping of one run of text in an authorized face.
#[derive(Clone, Debug)]
pub struct ShapeRequest {
    pub font: String,
    /// Pixels per em.
    pub size: f32,
    pub text: String,
    pub direction: ShapeDirection,
    /// Variation axis coordinates (user units) by tag.
    pub variations: Vec<([u8; 4], f32)>,
    /// Feature settings by tag, applied to the whole run.
    pub features: Vec<([u8; 4], u32)>,
}
/// One shaped glyph in pixels; `cluster` is the byte offset in the text of
/// the first character the glyph represents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// Bounded font registration capability. Handles cannot cross clients.
pub trait FontLibrary: Send + Sync {
    /// Actual metadata of the authorized, currently published font catalogue.
    fn catalog(&self) -> Result<Vec<FontCatalogFace>> {
        Err(crate::Error::Unsupported("font catalogue metadata"))
    }
    /// Cached aliases currently authorized for this client. This enumerates
    /// published faces only and performs no filesystem or system-font scan.
    fn aliases(&self) -> Result<Vec<String>> {
        Err(crate::Error::Unsupported("scoped font enumeration"))
    }
    fn begin(&self, spec: FontLoadSpec) -> Result<FontJob>;
    fn status(&self, job: FontJob) -> Result<FontLoadState>;
    fn resolve(&self, name: &str) -> Result<Option<FontInfo>>;
    /// Resolve a client-local binding, canonical alias or installed family to
    /// the exact engine alias used for retained drawing. Static faces have no
    /// dynamic lease and are therefore not fabricated as FontInfo handles.
    fn resolve_alias(&self, _name: &str) -> Result<Option<String>> {
        Err(crate::Error::Unsupported("scoped font alias resolution"))
    }
    /// Bounded immutable character-to-glyph lookup in the authorized face.
    fn glyph_id(&self, _alias: &str, _character: char) -> Result<u32> {
        Err(crate::Error::Unsupported("scoped glyph identifier"))
    }
    /// Cached actual style; this performs no font parsing or I/O on the owner.
    fn face_style(&self, _alias: &str) -> Result<Option<FontFaceStyle>> {
        Err(crate::Error::Unsupported("scoped font style metadata"))
    }
    /// Synchronous immutable metrics using this client's authorized font view.
    fn metrics(&self, _alias: &str, _size: f32, _character: char) -> Result<super::FontMetrics> {
        Err(crate::Error::Unsupported("scoped font metrics"))
    }
    /// Convert an outline em size to the exact line-height scale used by glyph
    /// bounds/metrics. Uses an authorized immutable face, with no host I/O.
    fn line_height_for_em(&self, _alias: &str, _em: f32) -> Result<f32> {
        Err(crate::Error::Unsupported("scoped font em scale"))
    }
    fn outline_em_metrics(&self, _alias: &str, _em: f32) -> Result<OutlineEmMetrics> {
        Err(crate::Error::Unsupported("scoped outline em metrics"))
    }
    fn begin_glyph_bounds(
        &self,
        _spec: GlyphBoundsSpec,
    ) -> Result<alloc::boxed::Box<dyn GlyphBoundsQuery>> {
        Err(crate::Error::Unsupported("scoped glyph bounds worker"))
    }
    fn is_loaded(&self, alias: &str) -> Result<bool>;
    /// Pair kerning (legacy `kern` table) between two characters, in pixels
    /// at `size` pixels per em; zero when the face has no such pair.
    fn kerning(&self, _alias: &str, _size: f32, _left: char, _right: char) -> Result<f32> {
        Err(crate::Error::Unsupported("scoped font kerning"))
    }
    /// OpenType shaping (GSUB/GPOS, variations, features) of one run.
    fn shape(&self, _request: &ShapeRequest) -> Result<Vec<ShapedGlyph>> {
        Err(crate::Error::Unsupported("scoped text shaping"))
    }
    /// Variable-font axes and named instances of an authorized face.
    fn variations(&self, _alias: &str) -> Result<FontVariations> {
        Err(crate::Error::Unsupported("scoped font variation metadata"))
    }
    fn release_job(&self, job: FontJob) -> Result<()>;
    fn release_font(&self, handle: FontHandle) -> Result<()>;
}
/// Portable face metadata, without GDI handles or invented installed fonts.
#[derive(Clone, Debug)]
pub struct FontCatalogFace {
    pub alias: String,
    pub names: Vec<String>,
    pub fixed_pitch: bool,
    pub outline: bool,
    pub code_pages: Option<[u32; 2]>,
}
