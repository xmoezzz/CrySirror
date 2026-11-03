// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free immutable image leases and owner-polled processing completion.
use crate::{Result, state::ClientId};
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
/// Explicit positioned regular-face coverage rendered into an immutable image.
/// White RGB is retained even outside glyphs; alpha is the rounded /255 union
/// of coverage in request order. No native font weight, wrapping or bitmap
/// mutation policy is inferred. Clients can request one glyph when per-glyph
/// composition/order matters.
#[derive(Clone, Debug)]
pub struct GlyphImageSpec {
    pub font: String,
    pub size: u16,
    pub canvas: [u32; 2],
    pub glyphs: Vec<crate::service::Glyph>,
}
/// Positioned glyph records composed using the integer stored-byte bitmap profile.
/// Layout, ruby policy and native message state remain with the caller.
#[derive(Clone, Debug)]
pub struct BitmapGlyphRecord {
    pub character: char,
    pub position: [f32; 2],
    pub color: [u8; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct BitmapTextShadow {
    pub mode: u8,
    pub offset: [u16; 2],
    pub color: [u8; 3],
    pub concentration: u16,
}
#[derive(Clone, Debug)]
pub struct BitmapTextSpec {
    pub font: String,
    pub size: u16,
    pub destination_format: u8,
    pub glyphs: Vec<BitmapGlyphRecord>,
    pub shadow: Option<BitmapTextShadow>,
}
/// Pixel-size interpretation for positioned glyph rasterization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphSizeBasis {
    LineHeight,
    Em,
}
/// Vertical origin of positioned coverage, independent of line layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphImageOrigin {
    Baseline,
    Top,
}
/// Explicit coverage pipeline; it does not claim native GDI font matching.
#[derive(Clone, Debug)]
pub struct GlyphRasterOptions {
    pub size_basis: GlyphSizeBasis,
    pub origin: GlyphImageOrigin,
    /// Integer working raster scale1..4, reduced to the requested canvas.
    pub supersample: u8,
    /// Convert working coverage to0/255 before morphology/reduction.
    pub binary_cutoff: Option<u8>,
    /// Symmetric disk maximum on working pixels0..16; same kernel as rich bold.
    pub embolden_radius: u8,
    /// Optional exact lookup indexed by binary hit count in a reduction block.
    /// Requires binary_cutoff and length supersample^2+1 (at most17 bytes).
    pub coverage_lut: Option<Arc<[u8]>>,
    pub downsample_nearest: bool,
    /// Use actual selected outline face underline/strikeout design metrics.
    /// Missing metrics/precomputed-only profiles fail explicitly.
    pub underline: bool,
    pub strikeout: bool,
}
impl Default for GlyphRasterOptions {
    fn default() -> Self {
        Self {
            size_basis: GlyphSizeBasis::LineHeight,
            origin: GlyphImageOrigin::Baseline,
            supersample: 1,
            binary_cutoff: None,
            embolden_radius: 0,
            coverage_lut: None,
            downsample_nearest: true,
            underline: false,
            strikeout: false,
        }
    }
}
impl GlyphRasterOptions {
    pub fn validate(&self) -> Result<()> {
        if !(1..=4).contains(&self.supersample) || self.embolden_radius > 16 {
            return Err(crate::Error::Invalid("styled glyph supersample/embolden"));
        }
        if self.coverage_lut.as_ref().is_some_and(|t| {
            self.binary_cutoff.is_none() || t.len() != usize::from(self.supersample).pow(2) + 1
        }) {
            return Err(crate::Error::Invalid("styled glyph coverage lookup"));
        }
        Ok(())
    }
}
/// One immutable positioned-glyph image with explicit coverage processing.
#[derive(Clone, Debug)]
pub struct StyledGlyphImageSpec {
    pub glyphs: GlyphImageSpec,
    pub raster: GlyphRasterOptions,
}

/// Separate immutable glyph cells prepared by one worker against one frozen face.
#[derive(Clone, Debug)]
pub struct GlyphImageCell {
    pub canvas: [u32; 2],
    pub glyphs: Vec<crate::service::Glyph>,
}
/// A bounded list of independent cells prepared against one frozen face.
#[derive(Clone, Debug)]
pub struct GlyphImageBatchSpec {
    pub font: String,
    pub size: u16,
    pub cells: Vec<GlyphImageCell>,
    pub raster: GlyphRasterOptions,
}
/// Cells retain independent output leases even after the batch is dropped.
pub trait GlyphImageBatchResource: Send + Sync {
    fn client(&self) -> ClientId;
    fn images(&self) -> &[Arc<dyn ImageResource>];
}
/// Immutable signed field resource, separate from colour image allocations.
#[derive(Clone, Debug)]
pub enum VectorFieldSamples {
    Displacement(Arc<[[i16; 2]]>),
    DirectionDistance(Arc<[DirectionDistanceSample]>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirectionDistanceSample {
    pub x: i16,
    pub y: i16,
    pub phase: u16,
}
#[derive(Clone, Debug)]
pub struct VectorFieldResource {
    pub client: ClientId,
    pub size: [u32; 2],
    pub samples: VectorFieldSamples,
}
impl VectorFieldResource {
    pub fn validate(&self, owner: ClientId) -> Result<()> {
        if self.client != owner {
            return Err(crate::Error::Invalid("foreign vector field resource"));
        }
        let n = u64::from(self.size[0]) * u64::from(self.size[1]);
        let actual = match &self.samples {
            VectorFieldSamples::Displacement(v) => v.len(),
            VectorFieldSamples::DirectionDistance(v) => v.len(),
        };
        if n == 0 || n > 4_194_304 || self.size.iter().any(|v| *v > 16_384) || n != actual as u64 {
            return Err(crate::Error::Limit("vector field layout"));
        }
        Ok(())
    }
    pub fn byte_len(&self) -> usize {
        match &self.samples {
            VectorFieldSamples::Displacement(v) => v.len() * 4,
            VectorFieldSamples::DirectionDistance(v) => v.len() * 6,
        }
    }
}
#[derive(Clone, Debug)]
pub enum FieldWarpSpec {
    Displacement {
        first: Arc<VectorFieldResource>,
        second: Option<Arc<VectorFieldResource>>,
        amount: u16,
        bilinear: bool,
    },
    Wave {
        field: Arc<VectorFieldResource>,
        samples: Arc<[i16]>,
        origin: [i32; 2],
        wrap: bool,
        bilinear: bool,
    },
}
impl FieldWarpSpec {
    pub fn validate(&self, owner: ClientId, size: [u32; 2]) -> Result<usize> {
        let check = |field: &VectorFieldResource| -> Result<()> {
            field.validate(owner)?;
            if size[0] > field.size[0] || size[1] > field.size[1] {
                return Err(crate::Error::Invalid("warp exceeds vector field"));
            }
            Ok(())
        };
        match self {
            Self::Displacement {
                first,
                second,
                amount,
                ..
            } => {
                check(first)?;
                if *amount > 256 || !matches!(first.samples, VectorFieldSamples::Displacement(_)) {
                    return Err(crate::Error::Invalid("displacement field format/amount"));
                }
                if let Some(second) = second {
                    check(second)?;
                    if !matches!(second.samples, VectorFieldSamples::Displacement(_)) {
                        return Err(crate::Error::Invalid("secondary displacement field format"));
                    }
                }
                Ok(first.byte_len() + second.as_ref().map_or(0, |f| f.byte_len()))
            }
            Self::Wave {
                field,
                samples,
                origin,
                ..
            } => {
                check(field)?;
                if !matches!(field.samples, VectorFieldSamples::DirectionDistance(_))
                    || samples.is_empty()
                    || samples.len() > 4_194_304
                    || origin.iter().any(|v| v.unsigned_abs() > 1_000_000)
                {
                    return Err(crate::Error::Invalid("wave field format/samples/origin"));
                }
                Ok(field.byte_len() + samples.len() * 2)
            }
        }
    }
}
/// Retained atomic batch completion; errors and cancellation expose no partial cells.
#[derive(Clone)]
pub enum GlyphImageBatchState {
    Pending,
    Ready(Arc<dyn GlyphImageBatchResource>),
    Failed(crate::Error),
    Cancelled,
}
/// One atomic completion; no partial cell list is visible. Drop/cancel never joins.
pub trait GlyphImageBatchQuery: Send {
    fn client(&self) -> ClientId;
    fn poll(&mut self) -> Result<GlyphImageBatchState>;
    fn cancel(&mut self) -> Result<()>;
}

/// Explicit byte arithmetic for immutable bitmap composition. These profiles
/// operate on stored RGBA, independently of the eventual display alpha policy.
#[derive(Clone, Copy, Debug)]
pub enum ImageCompositeMath {
    /// Explicit stored-byte bitmap arithmetic. Extra layer opacity is forbidden;
    /// the native integer parameter selects its own 0..256 coverage semantics.
    IntegerStored {
        destination_format: u8,
        source_format: u8,
        mode: i32,
        parameter: i32,
    },

    /// Explicit normalized fragment/factor state; layer opacity must be255.
    Normalized(crate::image_blend::RgbaBlendState),
    Copy,
    /// Copy the colour channels, keep the destination alpha.
    CopyRgb,
    /// Copy/opacity-lerp the stored alpha byte, retaining destination RGB.
    /// Opacity uses the same rounded /255 interpolation as Copy/CopyRgb.
    CopyAlpha,
    /// Encoded RGB d + floor((s-d)*opacity/256), destination alpha zero.
    /// Full opacity copies all four channels exactly.
    OpaqueScalar256,
    /// Encoded RGB interpolation with source alpha (times opacity/256 when
    /// opacity < 255). Writes alpha zero, including transparent source pixels.
    MaskScalar256,
    /// MaskScalar256's RGB interpolation, keeping the destination alpha
    /// (krkr `_HDA` hold-destination-alpha blend).
    MaskHoldAlphaScalar256,
    /// mask0 skips; mask255 uses weight/(256-weight), partial mask uses
    /// a=mask*weight>>8 and255-a; sum then/256, keeping destination alpha.
    /// Composite-layer opacity must be255; weight is0..=256.
    MaskPiecewise256 {
        weight: u16,
    },
    /// Mask0 skips, mask255 copies RGB; partial RGB is floor(s*a/256) +
    /// floor(d*(256-a)/256). Retains destination alpha; opacity must be255.
    MaskSeparate256,
    /// Opaque input over a straight-alpha destination with scalar /256 tables.
    OpaqueOnAlphaScalar256,
    /// Straight-alpha input over straight-alpha destination, scalar /256 tables.
    AlphaOnAlphaScalar256,
    /// Photoshop-like colour blend (krkr `tvpps.c` tables); `hold_alpha`
    /// keeps the destination alpha, otherwise alpha is written as zero.
    Ps {
        blend: PsBlend,
        hold_alpha: bool,
    },
}
/// Photoshop-like colour operations with exact byte-table forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsBlend {
    Normal,
    Overlay,
    HardLight,
    SoftLight,
    ColorDodge,
    ColorBurn,
    Lighten,
    Darken,
    Additive,
    Subtractive,
    Multiplicative,
    Screen,
    /// Photoshop 5 colour dodge: source faded first, no destination lerp.
    ColorDodge5,
    Difference,
    /// Photoshop 5 difference: source faded first, destination alpha kept.
    Difference5,
    Exclusion,
}
#[derive(Clone, Debug)]
pub struct ImageCompositeLayer {
    pub input: Box<ImageInput>,
    pub source: [i32; 4],
    pub destination: [i32; 2],
    /// Destination-space rectangle limiting this operation. Clipping retains
    /// the original source-to-destination mapping; `None` uses the full image.
    pub clip: Option<[i32; 4]>,
    pub opacity: u8,
    pub math: ImageCompositeMath,
}
/// A CPU resource owned by the engine; no decoder or mutation logic lives in a VM.
pub trait ImageResource: Send + Sync + core::fmt::Debug {
    fn client(&self) -> ClientId;
    fn size(&self) -> [u32; 2];
    fn rgba(&self) -> Arc<[u8]>;
    fn opaque(&self) -> bool;
    /// Opt-in checked lookup on resident storage. The default deliberately
    /// does not call rgba(): third-party providers may materialize full images
    /// there. None means out of bounds; malformed/unsupported storage is an error.
    fn pixel_at(&self, _point: [u32; 2]) -> Result<Option<[u8; 4]>> {
        Err(crate::Error::Unsupported("resident image pixel lookup"))
    }
    /// Extract immutable row-major coverage for a checked [x,y,width,height]
    /// source region. This copies only alpha once, without applying display
    /// filters or caller-specific hit policy. Work is bounded to 16 Mi pixels.
    fn alpha_region(&self, rect: [u32; 4]) -> Result<Arc<[u8]>> {
        let [x, y, w, h] = rect;
        let [iw, ih] = self.size();
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|v| v > iw)
            || y.checked_add(h).is_none_or(|v| v > ih)
        {
            return Err(crate::Error::Invalid("image alpha region bounds"));
        }
        let count = u64::from(w) * u64::from(h);
        if count > 16 * 1024 * 1024 {
            return Err(crate::Error::Limit("image alpha region"));
        }
        let rgba = self.rgba();
        let expected = u64::from(iw)
            .checked_mul(u64::from(ih))
            .and_then(|v| v.checked_mul(4));
        if expected != Some(rgba.len() as u64) {
            return Err(crate::Error::Invalid("image alpha resource layout"));
        }
        let mut coverage = Vec::with_capacity(count as usize);
        for row in y..y + h {
            let start = ((u64::from(row) * u64::from(iw) + u64::from(x)) * 4) as usize;
            for pixel in rgba[start..start + w as usize * 4].chunks_exact(4) {
                coverage.push(pixel[3]);
            }
        }
        Ok(coverage.into())
    }
    /// Read one stored coverage byte. This does not apply display opacity,
    /// filters or a VM's hit policy. Out-of-bounds/malformed resources return None.
    fn alpha_at(&self, point: [u32; 2]) -> Option<u8> {
        let [w, h] = self.size();
        if point[0] >= w || point[1] >= h {
            return None;
        }
        let index = u64::from(point[1])
            .checked_mul(u64::from(w))?
            .checked_add(u64::from(point[0]))?
            .checked_mul(4)?
            .checked_add(3)?;
        self.rgba().get(usize::try_from(index).ok()?).copied()
    }
}
mod composition;
mod pyramid;
pub use pyramid::{ImagePyramidLevel, ImagePyramidResource};

/// One retained sprite of an integer particle plane. These are engine image
/// leases, never foreign texture pointers. Projection/simulation is separate.
#[derive(Clone, Debug)]
pub struct ImageParticleSprite {
    pub primary: Arc<dyn ImageResource>,
    pub secondary: Option<Arc<dyn ImageResource>>,
    pub primary_format: u8,
    pub secondary_format: u8,
    pub shrink: u8,
    pub mix: u16,
    pub center: [i32; 2],
    pub mode: i32,
    pub parameter: i32,
}
#[derive(Clone, Debug)]
pub enum ImageInput {
    /// Bounded bulk sprite composition with shared exact fixed-point mip
    /// sampling. Equal resource/format/shrink tuples are sampled once per job.
    ParticlePlane {
        size: [u32; 2],
        sprites: Vec<ImageParticleSprite>,
    },
    PyramidLevel {
        pyramid: Arc<ImagePyramidResource>,
        level: u8,
    },
    /// Signed-field sampling evaluated by the shared image worker. The
    /// immutable field leases are owner-checked and included in admission.
    FieldWarp {
        source: Box<ImageInput>,
        size: [u32; 2],
        spec: FieldWarpSpec,
    },
    /// A bounded, immutable subgraph with its own typed effect context. The
    /// shared worker evaluates it inside the same admission and cancellation
    /// scope; no nested worker or VM state is created.
    Process {
        input: Box<ImageInput>,
        effects: Arc<[u8]>,
    },
    /// Capability-resolved encoded bytes, for archive members and embedded
    /// resources. Shared workers decode using the same registry as file assets.
    /// Declared dimensions bound admission and must match the decoded result.
    Encoded {
        name: alloc::string::String,
        bytes: Arc<[u8]>,
        size: [u32; 2],
    },
    /// Immutable ordered bitmap operation graph, evaluated by shared CPU workers.
    /// Base and descendants retain original channel bytes; no screen is captured.
    Composite {
        base: Box<ImageInput>,
        layers: Vec<ImageCompositeLayer>,
    },
    Solid {
        size: [u32; 2],
        color: [u8; 4],
    },
    Raster {
        destination: Box<ImageInput>,
        source: Box<ImageInput>,
    },
    /// Root-only three-input context. The first effect must be MaskedPairBlend;
    /// all inputs use the same scope, graph, decode and memory admission rules.
    Ternary {
        destination: Box<ImageInput>,
        first: Box<ImageInput>,
        second: Box<ImageInput>,
    },
    Transition {
        from: Arc<dyn ImageResource>,
        to: Arc<dyn ImageResource>,
        mask: Option<Arc<dyn ImageResource>>,
    },
    /// Expected dimensions bound admission; the worker verifies the decoded size.
    Asset {
        path: String,
        size: [u32; 2],
        fingerprint: Option<[u8; 32]>,
    },
    Resource(Arc<dyn ImageResource>),
}
#[derive(Clone, Debug)]
pub enum ImageProcessState {
    Pending,
    Ready(Arc<dyn ImageResource>),
    Failed(String),
}
/// Poll never joins a worker. Ready/error are retained; dropping requests cancel.
pub trait ImageProcess: Send {
    fn poll(&mut self) -> Result<ImageProcessState>;
}
pub type ImageJob = Box<dyn ImageProcess>;

/// Read one immutable row-major RGBA pixel without allocation or image decoding.
/// Layout is checked before addressing; a valid out-of-bounds point returns None.
pub fn rgba_pixel_at(size: [u32; 2], rgba: &[u8], point: [u32; 2]) -> Result<Option<[u8; 4]>> {
    let [w, h] = size;
    let expected = u64::from(w)
        .checked_mul(u64::from(h))
        .and_then(|v| v.checked_mul(4));
    if w == 0 || h == 0 || expected != Some(rgba.len() as u64) {
        return Err(crate::Error::Invalid("resident RGBA resource layout"));
    }
    if point[0] >= w || point[1] >= h {
        return Ok(None);
    }
    let index = (u64::from(point[1]) * u64::from(w) + u64::from(point[0])) * 4;
    let index = usize::try_from(index).map_err(|_| crate::Error::Limit("resident RGBA address"))?;
    Ok(Some(rgba[index..index + 4].try_into().map_err(|_| {
        crate::Error::Invalid("resident RGBA pixel")
    })?))
}
impl ImageInput {
    /// Query only already-resident Solid/Resource inputs. This never decodes an
    /// asset, evaluates a graph or falls back to a provider's full rgba() image.
    /// Provider-specific resident access may explicitly be Unsupported.
    pub fn resident_pixel(&self, owner: ClientId, point: [u32; 2]) -> Result<Option<[u8; 4]>> {
        match self {
            Self::Solid {
                size: [w, h],
                color,
            } => {
                if *w == 0 || *h == 0 {
                    return Err(crate::Error::Invalid("resident solid dimensions"));
                }
                Ok((point[0] < *w && point[1] < *h).then_some(*color))
            }
            Self::Resource(resource) => {
                if resource.client() != owner {
                    return Err(crate::Error::Invalid("foreign resident image resource"));
                }
                resource.pixel_at(point)
            }
            _ => Err(crate::Error::Unsupported(
                "image input must be prepared before resident pixel lookup",
            )),
        }
    }
}
#[cfg(test)]
mod resident_pixel_tests {
    use super::*;
    #[derive(Debug)]
    struct Resident {
        owner: ClientId,
    }
    impl ImageResource for Resident {
        fn client(&self) -> ClientId {
            self.owner
        }
        fn size(&self) -> [u32; 2] {
            [2, 1]
        }
        fn rgba(&self) -> Arc<[u8]> {
            panic!("pixel lookup must not materialize full image")
        }
        fn opaque(&self) -> bool {
            false
        }
        fn pixel_at(&self, p: [u32; 2]) -> Result<Option<[u8; 4]>> {
            rgba_pixel_at([2, 1], &[10, 20, 30, 40, 50, 60, 70, 80], p)
        }
    }
    #[derive(Debug)]
    struct Unsupported;
    impl ImageResource for Unsupported {
        fn client(&self) -> ClientId {
            ClientId(1)
        }
        fn size(&self) -> [u32; 2] {
            [1, 1]
        }
        fn rgba(&self) -> Arc<[u8]> {
            panic!("default lookup must not call rgba")
        }
        fn opaque(&self) -> bool {
            false
        }
    }
    #[test]
    fn exact_stored_rgba_has_checked_layout_and_coordinates() {
        assert_eq!(
            rgba_pixel_at([2, 1], &[10, 20, 30, 40, 50, 60, 70, 80], [1, 0]).unwrap(),
            Some([50, 60, 70, 80])
        );
        assert_eq!(rgba_pixel_at([2, 1], &[0; 8], [2, 0]).unwrap(), None);
        assert_eq!(rgba_pixel_at([2, 1], &[0; 8], [0, u32::MAX]).unwrap(), None);
        assert!(rgba_pixel_at([0, 1], &[], [0, 0]).is_err());
        assert!(rgba_pixel_at([2, 1], &[0; 7], [0, 0]).is_err());
        assert!(rgba_pixel_at([u32::MAX, u32::MAX], &[], [0, 0]).is_err());
    }
    #[test]
    fn resident_inputs_never_decode_materialize_or_cross_client_scope() {
        let source = ImageInput::Resource(Arc::new(Resident { owner: ClientId(1) }));
        assert_eq!(
            source.resident_pixel(ClientId(1), [0, 0]).unwrap(),
            Some([10, 20, 30, 40])
        );
        assert!(source.resident_pixel(ClientId(2), [0, 0]).is_err());
        let unsupported = ImageInput::Resource(Arc::new(Unsupported));
        assert!(matches!(
            unsupported.resident_pixel(ClientId(1), [0, 0]),
            Err(crate::Error::Unsupported(_))
        ));
        let graph = ImageInput::Composite {
            base: Box::new(source),
            layers: Vec::new(),
        };
        assert!(matches!(
            graph.resident_pixel(ClientId(1), [0, 0]),
            Err(crate::Error::Unsupported(_))
        ));
        let solid = ImageInput::Solid {
            size: [2, 1],
            color: [3, 4, 5, 6],
        };
        assert_eq!(
            solid.resident_pixel(ClientId(1), [1, 0]).unwrap(),
            Some([3, 4, 5, 6])
        );
        assert_eq!(solid.resident_pixel(ClientId(1), [2, 0]).unwrap(), None);
    }
}

/// Explicit little-endian byte layout for engine pixel interchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedPixelLayout {
    Bgr555,
    Bgr8,
    /// Four-byte BGR storage with an unused high byte and opaque coverage.
    Bgrx8,
    Bgra8,
    Red8,
}
impl PackedPixelLayout {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgr555 => 2,
            Self::Bgr8 => 3,
            Self::Bgrx8 | Self::Bgra8 => 4,
            Self::Red8 => 1,
        }
    }
}
#[derive(Clone, Debug)]
pub struct PackedImageSpec {
    pub size: [u32; 2],
    pub layout: PackedPixelLayout,
    pub bytes: Arc<[u8]>,
}
