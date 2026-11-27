// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared image processing for bitmap, UI, sprite and texture resources. These
//! are destructive resource operations, separate from display-only layer filters.
use crate::ImageSurface;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
/// CPU pixel limit bounds scratch memory independently of the decoder budget.
pub const MAX_EFFECT_PIXELS: u64 = 4_194_304;
/// Reusable image operations. All channels are straight RGBA bytes; RGB-only
/// transforms preserve alpha. Effects never silently select a different effect.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImageEffect {
    /// Remove alpha-marked border and resize nine retained image cells.
    NinePatch {width:u32,height:u32},
    /// Import bounded monochrome coverage or packed BGRA glyph bytes.
    CoverageBitmap {spec:crate::coverage_bitmap::CoverageBitmap},
    /// Native-compatible circular coverage blur, preserving stored RGB.
    GlyphCoverageBlur {radius:u32,level:i32},
    RadialRgbBrushes{spec:crate::radial_rgb_brushes::RadialRgbBrushes},
    PackedPairBatch{spec:crate::packed_pair_batch::PackedPairBatch},
    RadialWavePair{spec:crate::radial_wave_pair::RadialWavePair},
    LinearMaskOffset{spec:crate::linear_mask_offset::LinearMaskOffset},
    StoredWordSpans{spec:crate::stored_word_spans::StoredWordSpans},
    StoredGradientSpans{spec:crate::stored_gradient_spans::StoredGradientSpans},
    /// Saturated RGB offsets over squared-distance circular/cutout spans.
    CircularOffset{spec:crate::circular_offset::CircularOffset},
    /// Quantized outward channel lifts from independently positioned row edges.
    RowEdgeLift{spec:crate::row_edge_lift::RowEdgeLift},
    /// Inclusive ellipse/cutout spans with explicit stored-byte RGB arithmetic.
    EllipticPaint { spec: crate::elliptic_paint::EllipticPaint },
    /// Stored RGB neighbour differences with independently truncated weights.
    NeighborRelief { spec: crate::neighbor_relief::NeighborRelief },
    /// Three-input RGB-mask coverage with explicit packed-word arithmetic.
    MaskFieldPair { spec: crate::mask_field_pair::MaskFieldPair },
    PackedColorMask { spec: crate::packed_color_mask::PackedColorMask },
    /// Two independent alpha masks, optional RGB transform and packed products.
    PackedDualMask { spec: crate::packed_dual_mask::PackedDualMask },
    /// Single-precision half-biased DDA with signed stored-colour interpolation.
    FloatingLines { lines:Vec<crate::floating_lines::FloatingLine> },
    ///One checked lookup for the stored coverage byte; RGB remains unchanged.
    AlphaLookup {
        table: Vec<u8>,
    },
    /// Look up one stored channel and copy the result into selected channels.
    /// RGBA write bits are1/2/4/8; every lookup reads the original pixel.
    ChannelLookup { channel: u8, write_mask: u8, table: Vec<u8> },
    /// Normalize RGB only where stored alpha is exactly zero.
    ClearTransparentRgb,
    /// Binary alpha from a reference channel byte. Spec: reference[x,y],
    /// channel0..3, equal0..255, different0..255. RGB remains unchanged.
    CoverageReferenceKey {
        spec: crate::coverage_key::CoverageReferenceKey,
    },
    /// Ordered integer rectangle/scanline mapping with a defined clear plane.
    RectMap {
        spec: crate::rect_map::RectMap,
    },
    /// Byte-accurate two-texture channel composition in a raster context.
    DualTexture {
        spec: crate::dual_texture::DualTextureByte128,
    },
    AffineBlend {
        spec: crate::affine_blend::AffineBlendByte128,
    },
    FixedAffineNearest {
        spec: crate::fixed_affine::FixedAffineNearest,
    },
    ScanlineAffineNearest {
        spec: crate::scanline_affine::ScanlineAffineNearest,
    },
    TileReveal {
        spec: crate::tile_reveal::TileReveal,
    },
    RadialAngleLookup {
        spec: crate::radial_angle::RadialAngleLookup,
    },
    GradientOverlay {
        spec: crate::gradient_overlay::GradientOverlayByte,
    },
    LinearGradient {
        spec: crate::linear_gradient::LinearGradientByte,
    },
    /// Alpha from a byte table indexed by offset + x*step0 + y*step1.
    LinearAlphaLookup {
        spec: crate::linear_alpha_lookup::LinearAlphaLookup,
    },
    SequentialBoxBlur {
        spec: crate::sequential_box_blur::SequentialBoxBlur,
    },
    MaskedBlur {
        spec: crate::masked_blur::MaskedBlur,
    },
    /// Sequential 3x3 stored RGBA averages with row feedback and one final
    /// top/bottom/left border repair; the right visible column stays intact.
    FeedbackBoxBlur {
        spec: crate::feedback_box_blur::FeedbackBoxBlur,
    },
    BoxBlurClamped {
        spec: crate::box_blur_clamped::BoxBlurClamped,
    },
    /// Repeat-edge average over an explicit half-open axis window.
    BoxBlurSpan {
        spec: crate::box_blur_span::BoxBlurSpan,
    },
    /// Full-cell sample replication. Spec: region[x,y,w,h], cell[w,h], and
    /// offsets[[dx,dy],...]. Partial cells and the outside region remain intact.
    CyclicCellSample {
        spec: crate::cyclic_cell_sample::CyclicCellSample,
    },
    NearestCellSample {
        spec: crate::nearest_cell_sample::NearestCellSample,
    },
    WaveDisplace {
        spec: crate::wave_displace::WaveDisplaceByte128,
    },
    ResizeByte {
        spec: crate::resize_byte::ResizeByte128,
    },
    ShapeMask {
        spec: crate::shape_mask::ShapeMask,
    },
    RectResample {
        spec: crate::rect_resample::RectResample1024,
    },
    /// Stored-byte8.8 rectangle sampling without a half-texel offset.
    RectResampleByte256 {
        spec: crate::resample_byte256::RectResampleByte256,
    },
    CosineLens {
        spec: crate::cosine_lens::CosineLensByte128,
    },
    IntegerColorMatrix {
        spec: crate::integer_color::ColorMatrixFixed,
    },
    IntegerColorModulation {
        spec: crate::integer_color::IntegerColorModulation,
    },
    /// Straight RGBA 4x5 matrix including alpha rows and columns.
    RgbaColorMatrix {
        spec: crate::integer_color::RgbaColorMatrix,
    },
    /// RGBA colour table indexed by stored alpha or explicit integer luminance.
    LumaLookup {
        spec: crate::luma_lookup::LumaLookup,
    },
    LumaOffsetLookup {
        spec: crate::tonal_balance::LumaOffsetLookup,
    },
    RowWordAdd {spec:crate::row_color::RowWordAdd},
    ShiftRows {spec:crate::shift_rows::ShiftRows},
    /// Source stretched to `size`, tiled and redrawn under a centred affine.
    TiledTransform {
        spec: crate::tiled_transform::TiledTransform,
    },
    /// One channel's observed range mapped onto 0..=255.
    ChannelRangeStretch {
        spec: crate::tiled_transform::ChannelRangeStretch,
    },
    /// Truncated straight RGBA means, one uniform colour per rectangular cell.
    BlockAverage {
        cell: [u16; 2],
    },
    /// Scaled sprite/tile composition onto a new transparent canvas.
    TileCompose {
        spec: crate::tile_compose::TileCompose,
    },
    /// Vertical alpha-ramp streaks stored into the image (Rain 438E90).
    StreakField {
        spec: crate::streak_field::StreakField,
    },
    /// One signed horizontal shift per row, transparent fill (Wave 433E80).
    RowShift {
        offsets: Vec<i32>,
    },
    /// Truncated straight RGBA means over complete square cells only; the
    /// right/bottom remainder keeps its source pixels (N-System Mosaic 433630).
    FullBlockAverage {
        cell: u16,
    },
    /// Horizontal mean over a row-truncated window (optionally one past).
    TruncatedRowBlur {
        spec: crate::truncated_row_blur::TruncatedRowBlur,
    },
    /// Random-length horizontal runs filled from the source run.
    RandomRuns {
        spec: crate::random_runs::RandomRuns,
    },
    /// Checked box-window reduction with explicit integer channel arithmetic.
    BoxReduce {
        spec: crate::box_reduce::BoxReduce,
    },
    /// Signed left/top/right/bottom expansion or crop, preserving stored bytes.
    CanvasMargins {
        margins: [i32; 4],
        fill: [u8; 4],
    },
    /// Modulate stored alpha using an immutable original-alpha plane.
    AlphaMaskPlane {
        spec: crate::alpha_mask::AlphaMaskPlane,
    },
    /// Ordered atlas alpha replacement, retaining stored destination RGB.
    AlphaStamps {
        spec: crate::alpha_stamps::AlphaStamps,
    },
    /// Replace selected stored RGBA channels from one channel of an equally
    /// sized second image. Destination bits R/G/B/A are 1/2/4/8; inversion is
    /// exactly 255-value. Unselected bytes, including hidden RGB, are retained.
    CopyChannel {
        source_channel: u8,
        destination_channels: u8,
        invert: bool,
    },
    /// Saturating alpha neighborhoods with explicit sample offsets and edge colour.
    CoverageSum {
        spec: crate::coverage_sum::CoverageSum,
    },
    /// Fixed-point alpha-aware triangular kernel; expands only explicit padding.
    TriangularBlur {
        spec: crate::triangular_blur::TriangularBlur,
    },
    RowDeform {
        spec: crate::pyramid::RowDeform,
    },
    AlphaPlanePair {
        spec: crate::alpha_planes::AlphaPlanePair,
    },
    StoredMaskBlend {
        spec: crate::stored_mask::StoredMaskBlend,
    },
    /// Exact stored-channel bitmap kernels over two immutable resources.
    IntegerRaster {
        spec: crate::integer_raster::IntegerRaster,
    },
    /// Checked byte drawing and sampling over two immutable engine images.
    ByteRaster { spec: crate::byte_raster::ByteRaster },
    /// Replace coverage while retaining all stored RGB bytes. Useful for
    /// opaque XRGB resources and explicitly authored mask normalisation.
    OverrideAlpha {
        alpha: u8,
    },
    /// Arbitrary straight RGB tables; premultiplied profiles fail before admission.
    RgbLookup {
        spec: crate::rgb_lut::RgbLookup,
    },
    /// Explicit grouped luminance rounding and integer /256 colour mixture.
    FixedLuma {
        spec: crate::fixed_luma::FixedLumaSpec,
    },
    /// Fixed16.16 piecewise nearest sampling and explicit /256 mixtures.
    PiecewiseResample {
        spec: crate::piecewise_resample::PiecewiseResampleSpec,
    },
    /// Bounded generic stripe/stagger mask for retained transitions.
    StripeReveal {
        spec: crate::sprite_alpha::StripeReveal,
    },
    /// Extract reusable sprite transparency from decoded resource pixels.
    SpriteAlpha {
        spec: crate::sprite_alpha::SpriteAlpha,
    },
    /// Two-input whole-pixel row copy; preserves uncovered destination pixels.
    /// Midpoint-circle discs copied from the raster source (second input).
    DiscCopies {
        spec: crate::disc_copy::DiscCopies,
    },
    SineRowCopy {
        spec: crate::row_copy::SineRowCopy,
    },
    /// Two-input clipped coverage multiplication with optional exterior clear.
    AlphaClipRect {
        spec: crate::alpha_clip::AlphaClipRect,
    },
    /// Second-half blue into first-half alpha (layerExBTOA).
    BlueToAlpha { spec: crate::layer_ex::BlueToAlpha },
    /// Opaque alpha inside a rectangle (layerExBTOA fillAlpha).
    FillAlpha { spec: crate::layer_ex::FillAlpha },
    /// Integer box shrink to an opaque image (shrinkCopyFast).
    ShrinkFast { spec: crate::layer_ex::ShrinkFast },
    /// Two-input area-average shrink onto a fractional rectangle (shrinkCopy).
    ShrinkCopy { spec: crate::layer_ex::ShrinkCopy },
    /// Two-input threshold overwrite (layerExBTOA overwrapRect).
    OverwrapRect { spec: crate::layer_ex::OverwrapRect },
    /// Two-input horizontally wrapped row copy (layerStwCopy).
    StitchWrappedCopy { spec: crate::layer_ex::StitchWrappedCopy },
    /// Two-input block/line displacement with channel splitting (GlitchEffect).
    GlitchCopy { spec: crate::layer_ex::GlitchCopy },
    /// Three immutable inputs, two independent masks and exact14bit products.
    PackedPairBlend {spec:crate::packed_pair_blend::PackedPairBlend},
    MaskedPairBlend {
        spec: crate::masked_pair::MaskedPairBlend,
    },
    Transition {
        spec: crate::transition::ImageTransition,
    },
    /// Engine vector path on the existing image with source-over composition.
    Vector {
        path: crate::vector::VectorPath,
    },
    /// Stored RGBA integer DDA strokes with explicit signed half-bias rounding.
    IntegerLines { lines:Vec<crate::integer_lines::IntegerLine> },
    /// Map the whole source into a convex quad, keeping canvas dimensions.
    Perspective {
        quad: [[f32; 2]; 4],
    },

    /// Per-channel pow(value/255,1/gamma), floor/ceiling in byte units.
    /// Fully transparent pixels keep hidden RGB, matching the Krkr gamma profile.
    Gamma {
        gamma: [f32; 3],
        floor: [i32; 3],
        ceiling: [i32; 3],
    },
    /// Integer luminance: (54R+183G+19B)/256, truncated.
    Grayscale,
    /// CxImage brightness/contrast LUT (brightness -255..255, contrast -100..100).
    BrightnessContrast {
        brightness: i32,
        contrast: i32,
    },
    /// CxImage HSL colorize: replace hue/saturation (0..255), keep lightness.
    Colorize {
        hue: u8,
        saturation: u8,
        blend: f64,
    },
    /// HSL modulate: hue degrees, saturation/luminance percent (-100..100).
    HslModulate {
        hue: i32,
        saturation: i32,
        luminance: i32,
    },
    /// Grey white noise replacing RGB; alpha is unchanged.
    WhiteNoise {
        seed: u32,
    },
    Invert,
    /// Every stored byte, alpha included, becomes floor(byte * factor / 255),
    /// factors in RGBA order (CatSystem2 68BEC0 multiplies a bitmap by ARGB).
    ChannelScale {
        factors: [u8; 4],
    },
    /// Box blur of `rect` (x, y, w, h; whole image when absent): each pixel
    /// becomes the truncated mean of the window `area` = [left, top, right,
    /// bottom] offsets (inclusive, containing 0) around it, with the window
    /// clipped to the image and the divisor counting only covered pixels.
    /// `alpha_weighted` averages colour premultiplied by alpha and
    /// un-premultiplies (krkrz `DoBoxBlurForAlpha`); otherwise all four
    /// channels average independently (`DoBoxBlur`).
    BoxBlur {
        #[serde(default)]
        rect: Option<[u32; 4]>,
        area: [i32; 4],
        #[serde(default)]
        alpha_weighted: bool,
    },
    /// Separable Gaussian on premultiplied floating RGBA, clamped image edges.
    GaussianBlur {
        sigma: f32,
    },
    /// Deterministic signed uniform RGB noise. Alpha is unchanged.
    Noise {
        seed: u32,
        amplitude: u8,
        monochrome: bool,
    },
    Flip {
        horizontal: bool,
        vertical: bool,
    },
    /// codeX RScript image-layer load conversions (mirror, rotate/zoom, tone).
    CelConvert { spec: crate::cel_convert::CelConvert },
    /// Top-left proportional nearest sampling using exact integer quotients.
    ResizeNearest { width: u32, height: u32 },
    /// Bilinear resize of straight RGBA to a new size.
    Resize {
        width: u32,
        height: u32,
    },
    /// Clockwise quarter turns; 90 and 270 swap width and height.
    Rotate {
        degrees: u16,
    },
    /// Whole-pixel row displacement: round(sin(phase+y*frequency)*amplitude).
    /// Source edges clamp. Phase/frequency are radians, amplitude is pixels.
    Raster {
        amplitude: f32,
        frequency: f32,
        phase: f32,
    },
}
impl ImageEffect {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::NinePatch{width,height}=>{crate::rgba_len(*width,*height)?;ensure!(u64::from(*width)*u64::from(*height)<=MAX_EFFECT_PIXELS,"nine-patch output");},
            Self::CoverageBitmap{spec}=>spec.validate()?,
            Self::GlyphCoverageBlur{radius,level}=>ensure!(*radius<=64&&(0..=65535).contains(level),"glyph coverage blur"),
            Self::CopyChannel { source_channel, destination_channels, .. } => ensure!(
                *source_channel < 4 && (1..=15).contains(destination_channels),
                "stored channel copy selection"
            ),
            Self::AlphaLookup { table } => ensure!(table.len() == 256, "alpha lookup table length"),
            Self::ChannelLookup { channel, write_mask, table } => ensure!(
                *channel < 4 && (1..=15).contains(write_mask) && table.len() == 256,
                "channel lookup selection/table"
            ),
            Self::CoverageReferenceKey { spec } => spec.validate()?,
            Self::RectMap { spec } => spec.validate()?,
            Self::DualTexture { spec } => spec.validate()?,
            Self::AffineBlend { spec } => spec.validate()?,
            Self::FixedAffineNearest { spec } => spec.validate()?,
            Self::ScanlineAffineNearest { spec } => spec.validate()?,
            Self::TileReveal { spec } => spec.validate()?,
            Self::RadialAngleLookup { spec } => spec.validate()?,
            Self::GradientOverlay { spec } => spec.validate()?,
            Self::LinearGradient { spec } => spec.validate()?,
            Self::LinearAlphaLookup { spec } => spec.validate()?,
            Self::SequentialBoxBlur { spec } => spec.validate()?,
            Self::MaskedBlur { spec } => spec.validate()?,
            Self::FeedbackBoxBlur { spec } => spec.validate()?,
            Self::BoxBlurClamped { spec } => spec.validate()?,
            Self::BoxBlurSpan { spec } => spec.validate()?,
            Self::CyclicCellSample { spec } => spec.validate()?,
            Self::NearestCellSample { spec } => spec.validate()?,
            Self::WaveDisplace { spec } => spec.validate()?,
            Self::ResizeByte { spec } => spec.validate()?,
            Self::ShapeMask { spec } => spec.validate()?,
            Self::RectResample { spec } => spec.validate()?,
            Self::RectResampleByte256 { spec } => spec.validate()?,
            Self::CosineLens { spec } => spec.validate()?,
            Self::IntegerColorMatrix { spec } => spec.validate()?,
            Self::RgbaColorMatrix { spec } => spec.validate()?,
            Self::IntegerColorModulation { spec } => spec.validate()?,
            Self::LumaLookup { spec } => spec.validate()?,
            Self::LumaOffsetLookup { spec } => spec.validate()?,
            Self::RowWordAdd{spec}=>spec.validate()?,
            Self::ShiftRows{spec}=>spec.validate()?,
            Self::BoxReduce { spec } => spec.validate()?,
            Self::RandomRuns { spec } => spec.validate()?,
            Self::TruncatedRowBlur { spec } => spec.validate()?,
            Self::TiledTransform { spec } => spec.validate()?,
            Self::ChannelRangeStretch { spec } => spec.validate()?,
            Self::TileCompose { spec } => spec.validate()?,
            Self::StreakField { spec } => spec.validate()?,
            Self::RowShift { offsets } => ensure!(
                offsets.len() <= 16_384 && offsets.iter().all(|v| v.unsigned_abs() <= 1 << 20),
                "row shift offsets"
            ),
            Self::FullBlockAverage { cell } => ensure!(
                (1..=4096).contains(cell),
                "full block average cell must be 1..4096"
            ),
            Self::BlockAverage { cell } => ensure!(
                cell.iter().all(|v| (1..=1024).contains(v)),
                "block average cell must be1..1024"
            ),
            Self::CanvasMargins { margins, .. } => ensure!(
                margins.iter().all(|v| v.unsigned_abs() <= 8192),
                "canvas margin bounds"
            ),
            Self::AlphaMaskPlane { spec } => spec.validate()?,
            Self::AlphaStamps { spec } => spec.validate()?,
            Self::RowDeform { spec } => spec.validate()?,
            Self::IntegerRaster { spec } => spec.validate()?,
            Self::ByteRaster { spec } => spec.validate()?,
            Self::StoredGradientSpans{spec}=>spec.validate()?,
            Self::StoredWordSpans{spec}=>spec.validate()?,
            Self::MaskFieldPair{spec}=>spec.validate()?,
            Self::PackedColorMask{spec}=>spec.validate()?,
            Self::PackedDualMask{spec}=>spec.validate()?,
            Self::LinearMaskOffset{spec}=>spec.validate()?,
            Self::RadialWavePair{spec}=>spec.validate()?,
            Self::FloatingLines{lines}=>crate::floating_lines::validate(lines)?,
            Self::NeighborRelief{spec}=>spec.validate()?,
            Self::CircularOffset{spec}=>spec.validate()?,
            Self::RowEdgeLift{spec}=>spec.validate()?,
            Self::EllipticPaint{spec}=>spec.validate()?,
            Self::StoredMaskBlend { spec } => spec.validate()?,
            Self::AlphaPlanePair { spec } => spec.validate()?,
            Self::RgbLookup { spec } => spec.validate()?,
            Self::FixedLuma { spec } => spec.validate()?,
            Self::PiecewiseResample { spec } => spec.validate()?,
            Self::StripeReveal { spec } => spec.validate()?,
            Self::SpriteAlpha { spec } => spec.validate()?,
            Self::SineRowCopy { .. } => (),
            Self::DiscCopies { spec } => spec.validate()?,
            Self::AlphaClipRect { spec } => spec.validate()?,
            Self::ShrinkCopy { spec } => spec.validate()?,
            Self::ShrinkFast { spec } => ensure!(spec.step[0] > 0 && spec.step[1] > 0, "shrink factor"),
            Self::BlueToAlpha { .. } | Self::FillAlpha { .. } | Self::OverwrapRect { .. } | Self::StitchWrappedCopy { .. } | Self::GlitchCopy { .. } => {}
            Self::PackedPairBlend{spec}=>spec.validate()?,
            Self::PackedPairBatch{spec}=>spec.validate()?,
            Self::RadialRgbBrushes{spec}=>spec.validate()?,
            Self::MaskedPairBlend { spec } => spec.validate()?,
            Self::Transition { spec } => spec.validate()?,
            Self::Vector { path } => path.validate()?,
            Self::IntegerLines { lines } => crate::integer_lines::validate(lines)?,
            Self::Perspective { quad } => crate::warp::validate_quad(quad)?,
            Self::Gamma {
                gamma,
                floor,
                ceiling,
            } => {
                ensure!(
                    gamma
                        .iter()
                        .all(|g| g.is_finite() && (0.01..=100.).contains(g)),
                    "gamma must be finite in 0.01..100"
                );
                ensure!(
                    floor
                        .iter()
                        .chain(ceiling)
                        .all(|v| (-65535..=65535).contains(v)),
                    "gamma ramp bounds"
                );
            }
            Self::BrightnessContrast {
                brightness,
                contrast,
            } => ensure!(
                (-65535..=65535).contains(brightness) && (-65535..=65535).contains(contrast),
                "brightness/contrast bounds"
            ),
            Self::Colorize { blend, .. } => ensure!(blend.is_finite(), "colorize blend"),
            Self::HslModulate {
                hue,
                saturation,
                luminance,
            } => ensure!(
                [hue, saturation, luminance]
                    .iter()
                    .all(|v| (-65535..=65535).contains(*v)),
                "modulate bounds"
            ),
            Self::BoxBlur { area, .. } => {
                ensure!(
                    area[0] <= 0 && area[1] <= 0 && area[2] >= 0 && area[3] >= 0,
                    "box blur area must contain the centre pixel"
                );
                ensure!(area.iter().all(|v| v.abs() <= 8192), "box blur area bounds");
            }
            Self::Resize { width, height } | Self::ResizeNearest {width,height} => ensure!(
                *width > 0
                    && *height > 0
                    && u64::from(*width) * u64::from(*height) <= MAX_EFFECT_PIXELS,
                "resize extent"
            ),
            Self::Rotate { degrees } => ensure!(
                matches!(degrees, 90 | 180 | 270),
                "rotation must be a quarter turn"
            ),
            Self::GaussianBlur { sigma } => ensure!(
                sigma.is_finite() && (0.0..=16.).contains(sigma),
                "blur sigma must be 0..16"
            ),
            Self::Raster {
                amplitude,
                frequency,
                phase,
            } => ensure!(
                [amplitude, frequency, phase]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 65536.),
                "raster parameters"
            ),
            _ => (),
        }
        Ok(())
    }
}
impl ImageSurface {
    pub fn apply_effect(&self, effect: &ImageEffect) -> Result<Self> {
        self.apply_effect_cancellable(effect, &|| Ok(()))
    }
    /// The input remains immutable on error/cancellation. Jobs call the fence
    /// between rows/passes; no worker ever mutates a published surface in place.
    pub fn apply_effect_cancellable(
        &self,
        effect: &ImageEffect,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        self.validate()?;
        effect.validate()?;
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= MAX_EFFECT_PIXELS,
            "image effect pixel budget"
        );
        check()?;
        if let ImageEffect::GradientOverlay { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::LinearGradient { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::LinearAlphaLookup { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::SequentialBoxBlur { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::MaskedBlur { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::FeedbackBoxBlur { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::BoxBlurClamped { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::BoxBlurSpan { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::CyclicCellSample { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::NearestCellSample { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::CelConvert { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RadialAngleLookup { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::WaveDisplace { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::ResizeByte { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RectResampleByte256 { spec } = effect {
            return spec.apply(self, check);
        }
        if matches!(
            effect,
            ImageEffect::DualTexture { .. }
                | ImageEffect::AffineBlend { .. }
                | ImageEffect::FixedAffineNearest { .. }
                | ImageEffect::ScanlineAffineNearest { .. }
                | ImageEffect::TileReveal { .. }
                | ImageEffect::AlphaMaskPlane { .. }
                | ImageEffect::AlphaStamps { .. }
                | ImageEffect::CopyChannel { .. }
                | ImageEffect::AlphaPlanePair { .. }
                | ImageEffect::MaskFieldPair{..}
                | ImageEffect::PackedColorMask{..}
                | ImageEffect::RadialWavePair{..}
                | ImageEffect::LinearMaskOffset{..}
                | ImageEffect::PackedDualMask{..}
                | ImageEffect::StoredMaskBlend { .. }
                | ImageEffect::Transition { .. }
                | ImageEffect::SineRowCopy { .. }
                | ImageEffect::DiscCopies { .. }
                | ImageEffect::AlphaClipRect { .. }
                | ImageEffect::ShrinkCopy { .. }
                | ImageEffect::OverwrapRect { .. }
                | ImageEffect::StitchWrappedCopy { .. }
                | ImageEffect::GlitchCopy { .. }
                | ImageEffect::PackedPairBatch{..}
                | ImageEffect::PackedPairBlend{..}
                | ImageEffect::MaskedPairBlend { .. }
                | ImageEffect::IntegerRaster { .. }
                | ImageEffect::ByteRaster { .. }
        ) {
            anyhow::bail!("image effect requires its multi-image context")
        }
        if let ImageEffect::RadialRgbBrushes{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::BlueToAlpha { spec } = effect { check()?; return spec.apply(self); }
        if let ImageEffect::FillAlpha { spec } = effect { check()?; return spec.apply(self); }
        if let ImageEffect::ShrinkFast { spec } = effect { check()?; return spec.apply(self); }
        if let ImageEffect::StoredGradientSpans{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::StoredWordSpans{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::AlphaLookup { table } = effect {
            ensure!(table.len() == 256, "alpha lookup table length");
            let mut out = self.clone();
            for row in out.rgba.chunks_mut(self.width as usize * 4) {
                check()?;
                for pixel in row.chunks_exact_mut(4) {
                    pixel[3] = table[pixel[3] as usize];
                }
            }
            check()?;
            return Ok(out);
        }
        if let ImageEffect::ChannelLookup { channel, write_mask, table } = effect {
            let mut out = self.clone();
            for row in out.rgba.chunks_mut(self.width as usize * 4) {
                check()?;
                for pixel in row.chunks_exact_mut(4) {
                    let value = table[usize::from(pixel[usize::from(*channel)])];
                    for (index, byte) in pixel.iter_mut().enumerate() {
                        if *write_mask & (1 << index) != 0 { *byte = value; }
                    }
                }
            }
            check()?;
            return Ok(out);
        }
        if let ImageEffect::ClearTransparentRgb = effect {
            let mut out=self.clone();
            for row in out.rgba.chunks_mut(self.width as usize*4) {
                check()?;
                for pixel in row.chunks_exact_mut(4) {if pixel[3]==0 {pixel[..3].fill(0);}}
            }
            check()?;
            return Ok(out);
        }
        if let ImageEffect::RectMap { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RowDeform { spec } = effect {
            return crate::pyramid::row_deform(self, spec, check);
        }
        if let ImageEffect::ShapeMask { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RectResample { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::CosineLens { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::IntegerColorMatrix { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RgbaColorMatrix { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::IntegerColorModulation { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::LumaOffsetLookup { spec } = effect {
            return spec.apply(self,check);
        }
        if let ImageEffect::RowWordAdd{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::ShiftRows{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::LumaLookup { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::BoxReduce { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RandomRuns { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::TruncatedRowBlur { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::TiledTransform { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::ChannelRangeStretch { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::TileCompose { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::StreakField { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::RowShift { offsets } = effect {
            return crate::row_shift::apply(self, offsets, check);
        }
        if let ImageEffect::FullBlockAverage { cell } = effect {
            return self.full_block_average(*cell, check);
        }
        if let ImageEffect::BlockAverage { cell } = effect {
            return self.block_average(*cell, check);
        }
        if let ImageEffect::CanvasMargins { margins, fill } = effect {
            return self.canvas_margins(*margins, *fill, check);
        }
        if let ImageEffect::RgbLookup { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::FixedLuma { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::PiecewiseResample { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::StripeReveal { spec } = effect {
            return self.stripe_reveal(spec, check);
        }
        if let ImageEffect::CoverageReferenceKey { spec } = effect {
            return self.coverage_reference_key(spec, check);
        }
        if let ImageEffect::SpriteAlpha { spec } = effect {
            return self.sprite_alpha(spec, check);
        }
        if let ImageEffect::Vector { path } = effect {
            return self.draw_vector(path, check);
        }
        if let ImageEffect::FloatingLines{lines}=effect{return self.floating_lines(lines,check);}
        if let ImageEffect::NeighborRelief{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::CircularOffset{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::RowEdgeLift{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::EllipticPaint{spec}=effect{return spec.apply(self,check);}
        if let ImageEffect::IntegerLines { lines } = effect { return self.integer_lines(lines,check); }
        if let ImageEffect::Perspective { quad } = effect {
            return self.perspective(*quad, check);
        }
        if let ImageEffect::ResizeNearest {width,height} = effect {
            let mut output=ImageSurface::filled(*width,*height,[0;4])?;
            for y in 0..*height {
                check()?;let sy=u64::from(y)*u64::from(self.height)/u64::from(*height);
                for x in 0..*width {
                    let sx=u64::from(x)*u64::from(self.width)/u64::from(*width);
                    let from=((sy*u64::from(self.width)+sx)*4) as usize;
                    let to=((y as usize*(*width as usize)+x as usize)*4) as usize;
                    output.rgba[to..to+4].copy_from_slice(&self.rgba[from..from+4]);
                }
            }
            return Ok(output);
        }
        if let ImageEffect::NinePatch{width,height}=effect{return crate::nine_patch::resize(self,[*width,*height],check);}
        if let ImageEffect::CoverageBitmap{spec}=effect{return spec.apply(check);}
        if let ImageEffect::GlyphCoverageBlur{radius,level}=effect{return crate::coverage_blur::blur_image(self,*radius,*level,check);}
        if let ImageEffect::Resize { width, height } = effect {
            return self.resized(*width, *height, check);
        }
        if let ImageEffect::Rotate { degrees } = effect {
            return self.rotated(*degrees, check);
        }
        if let ImageEffect::CoverageSum { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::TriangularBlur { spec } = effect {
            return spec.apply(self, check);
        }
        if let ImageEffect::GaussianBlur { sigma } = effect {
            return self.gaussian(*sigma, check);
        }
        if let ImageEffect::BoxBlur {
            rect,
            area,
            alpha_weighted,
        } = effect
        {
            return self.box_blur(*rect, *area, *alpha_weighted, check);
        }
        let mut out = self.clone();
        let w = self.width as usize;
        let h = self.height as usize;
        let mut seed = match effect {
            ImageEffect::Noise { seed, .. } | ImageEffect::WhiteNoise { seed } => *seed,
            _ => 0,
        };
        if seed == 0 {
            seed = 0x9e3779b9;
        }
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut lut = [[0u8; 256]; 3];
        if let ImageEffect::BrightnessContrast {
            brightness,
            contrast,
        } = effect
        {
            lut = [crate::cximage::brightness_contrast_lut(*brightness, *contrast); 3];
        }

        if let ImageEffect::Gamma {
            gamma,
            floor,
            ceiling,
        } = effect
        {
            for c in 0..3 {
                for v in 0..256 {
                    lut[c][v] = (((v as f64 / 255.).powf(1. / f64::from(gamma[c]))
                        * f64::from(ceiling[c] - floor[c])
                        + 0.5
                        + f64::from(floor[c]))
                    .trunc())
                    .clamp(0., 255.) as u8;
                }
            }
        }
        for y in 0..h {
            check()?;
            for x in 0..w {
                let at = (y * w + x) * 4;
                let p = &self.rgba[at..at + 4];
                match effect {
                    ImageEffect::OverrideAlpha { alpha } => out.rgba[at + 3] = *alpha,
                    ImageEffect::Gamma { .. } => {
                        if p[3] != 0 {
                            for c in 0..3 {
                                out.rgba[at + c] = lut[c][p[c] as usize];
                            }
                        }
                    }
                    // Native operates on BGRA without consulting alpha.
                    ImageEffect::BrightnessContrast { .. } => {
                        for c in 0..3 {
                            out.rgba[at + c] = lut[c][p[c] as usize];
                        }
                    }
                    ImageEffect::Colorize {
                        hue,
                        saturation,
                        blend,
                    } => {
                        let v =
                            crate::cximage::colorize([p[0], p[1], p[2]], *hue, *saturation, *blend);
                        out.rgba[at..at + 3].copy_from_slice(&v);
                    }
                    ImageEffect::HslModulate {
                        hue,
                        saturation,
                        luminance,
                    } => {
                        let v = crate::cximage::modulate(
                            [p[0], p[1], p[2]],
                            *hue,
                            *saturation,
                            *luminance,
                        );
                        out.rgba[at..at + 3].copy_from_slice(&v);
                    }
                    ImageEffect::WhiteNoise { .. } => {
                        let n = (random() % 256) as u8;
                        out.rgba[at..at + 3].fill(n);
                    }
                    ImageEffect::Grayscale => {
                        let v =
                            ((u32::from(p[0]) * 54 + u32::from(p[1]) * 183 + u32::from(p[2]) * 19)
                                >> 8) as u8;
                        out.rgba[at..at + 3].fill(v);
                    }
                    ImageEffect::Invert => {
                        for c in 0..3 {
                            out.rgba[at + c] = 255 - p[c];
                        }
                    }
                    ImageEffect::ChannelScale { factors } => {
                        for c in 0..4 {
                            out.rgba[at + c] =
                                (u32::from(p[c]) * u32::from(factors[c]) / 255) as u8;
                        }
                    }
                    ImageEffect::Noise {
                        amplitude,
                        monochrome,
                        ..
                    } => {
                        let a = i32::from(*amplitude);
                        let mut noise = (random() % (2 * a + 1) as u32) as i32 - a;
                        for c in 0..3 {
                            if c > 0 && !*monochrome {
                                noise = (random() % (2 * a + 1) as u32) as i32 - a;
                            }
                            out.rgba[at + c] = (i32::from(p[c]) + noise).clamp(0, 255) as u8;
                        }
                    }
                    ImageEffect::Flip {
                        horizontal,
                        vertical,
                    } => {
                        let sx = if *horizontal { w - 1 - x } else { x };
                        let sy = if *vertical { h - 1 - y } else { y };
                        let from = (sy * w + sx) * 4;
                        out.rgba[at..at + 4].copy_from_slice(&self.rgba[from..from + 4]);
                    }
                    ImageEffect::Raster {
                        amplitude,
                        frequency,
                        phase,
                    } => {
                        let dx = ((f64::from(*phase) + y as f64 * f64::from(*frequency)).sin()
                            * f64::from(*amplitude))
                        .round() as i64;
                        let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                        let from = (y * w + sx) * 4;
                        out.rgba[at..at + 4].copy_from_slice(&self.rgba[from..from + 4]);
                    }
                    _ => unreachable!(),
                }
            }
        }
        Ok(out)
    }
    fn resized(&self, width: u32, height: u32, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (nw, nh) = (width as usize, height as usize);
        let mut out = vec![0u8; nw * nh * 4];
        let axis = |at: usize, new: usize, old: usize| {
            let source = ((at as f64 + 0.5) * old as f64 / new as f64 - 0.5).max(0.);
            let low = (source as usize).min(old - 1);
            (low, (low + 1).min(old - 1), source - low as f64)
        };
        for y in 0..nh {
            check()?;
            let (y0, y1, fy) = axis(y, nh, h);
            for x in 0..nw {
                let (x0, x1, fx) = axis(x, nw, w);
                let to = (y * nw + x) * 4;
                for c in 0..4 {
                    let sample = |sy: usize, sx: usize| f64::from(self.rgba[(sy * w + sx) * 4 + c]);
                    let top = sample(y0, x0) * (1. - fx) + sample(y0, x1) * fx;
                    let bottom = sample(y1, x0) * (1. - fx) + sample(y1, x1) * fx;
                    out[to + c] = (top * (1. - fy) + bottom * fy + 0.5) as u8;
                }
            }
        }
        Ok(Self {
            width,
            height,
            rgba: out,
        })
    }
    fn rotated(&self, degrees: u16, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        let (w, h) = (self.width as usize, self.height as usize);
        let (nw, nh) = if degrees == 180 { (w, h) } else { (h, w) };
        let mut out = vec![0u8; nw * nh * 4];
        for y in 0..h {
            check()?;
            for x in 0..w {
                let (dx, dy) = match degrees {
                    90 => (h - 1 - y, x),
                    180 => (w - 1 - x, h - 1 - y),
                    _ => (y, w - 1 - x),
                };
                let (from, to) = ((y * w + x) * 4, (dy * nw + dx) * 4);
                out[to..to + 4].copy_from_slice(&self.rgba[from..from + 4]);
            }
        }
        let mut result = self.clone();
        result.width = nw as u32;
        result.height = nh as u32;
        result.rgba = out.into();
        Ok(result)
    }
    fn box_blur(
        &self,
        rect: Option<[u32; 4]>,
        area: [i32; 4],
        alpha_weighted: bool,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let (w, h) = (self.width as usize, self.height as usize);
        let [rx, ry, rw, rh] = rect.unwrap_or([0, 0, self.width, self.height]);
        let x0 = (rx as usize).min(w);
        let y0 = (ry as usize).min(h);
        let x1 = (rx as usize).saturating_add(rw as usize).min(w);
        let y1 = (ry as usize).saturating_add(rh as usize).min(h);
        if area == [0; 4] || x0 >= x1 || y0 >= y1 {
            return Ok(self.clone());
        }
        // Summed-area table of the summed channel values ([R, G, B, A]; with
        // alpha weighting the colour terms are `c * (a + a/128) >> 8`).
        let stride = w + 1;
        let mut sat = vec![[0u64; 4]; stride * (h + 1)];
        for y in 0..h {
            check()?;
            let mut row = [0u64; 4];
            for x in 0..w {
                let p = &self.rgba[(y * w + x) * 4..][..4];
                let v = if alpha_weighted {
                    let adj = u32::from(p[3]) + (u32::from(p[3]) >> 7);
                    [0, 1, 2].map(|c| u64::from(u32::from(p[c]) * adj >> 8))
                } else {
                    [0, 1, 2].map(|c| u64::from(p[c]))
                };
                row[0] += v[0];
                row[1] += v[1];
                row[2] += v[2];
                row[3] += u64::from(p[3]);
                let above = sat[y * stride + x + 1];
                sat[(y + 1) * stride + x + 1] = [0, 1, 2, 3].map(|c| above[c] + row[c]);
            }
        }
        let mut out = self.clone();
        for y in y0..y1 {
            check()?;
            let top = (y as i64 + i64::from(area[1])).max(0) as usize;
            let bottom = (y as i64 + i64::from(area[3])).min(h as i64 - 1) as usize + 1;
            for x in x0..x1 {
                let left = (x as i64 + i64::from(area[0])).max(0) as usize;
                let right = (x as i64 + i64::from(area[2])).min(w as i64 - 1) as usize + 1;
                let n = ((right - left) * (bottom - top)) as u64;
                let s = |c: usize| {
                    sat[bottom * stride + right][c] + sat[top * stride + left][c]
                        - sat[top * stride + right][c]
                        - sat[bottom * stride + left][c]
                };
                let mean = [0, 1, 2, 3].map(|c| s(c) / n);
                let px = &mut out.rgba[(y * w + x) * 4..][..4];
                if alpha_weighted {
                    let a = mean[3];
                    for c in 0..3 {
                        px[c] = if a == 0 {
                            0
                        } else {
                            (mean[c] * 255 / a).min(255) as u8
                        };
                    }
                    px[3] = a as u8;
                } else {
                    for c in 0..4 {
                        px[c] = mean[c] as u8;
                    }
                }
            }
        }
        Ok(out)
    }
    fn gaussian(&self, sigma: f32, check: &dyn Fn() -> Result<()>) -> Result<Self> {
        if sigma == 0. {
            return Ok(self.clone());
        }
        let radius = (sigma * 3.).ceil() as i32;
        let mut kernel = (-radius..=radius)
            .map(|x| (-0.5 * (x as f64 / f64::from(sigma)).powi(2)).exp())
            .collect::<Vec<_>>();
        let total = kernel.iter().sum::<f64>();
        for k in &mut kernel {
            *k /= total;
        }
        let w = self.width as usize;
        let h = self.height as usize;
        let mut a = vec![[0f32; 4]; w * h];
        let mut b = a.clone();
        for (p, rgba) in a.iter_mut().zip(self.rgba.chunks_exact(4)) {
            let alpha = rgba[3] as f32 / 255.;
            *p = [
                rgba[0] as f32 * alpha,
                rgba[1] as f32 * alpha,
                rgba[2] as f32 * alpha,
                rgba[3] as f32,
            ];
        }
        for vertical in [false, true] {
            for y in 0..h {
                check()?;
                for x in 0..w {
                    let mut v = [0f64; 4];
                    for (i, &k) in kernel.iter().enumerate() {
                        let d = i as i32 - radius;
                        let sx = if vertical {
                            x
                        } else {
                            (x as i32 + d).clamp(0, w as i32 - 1) as usize
                        };
                        let sy = if vertical {
                            (y as i32 + d).clamp(0, h as i32 - 1) as usize
                        } else {
                            y
                        };
                        for c in 0..4 {
                            v[c] += f64::from(a[sy * w + sx][c]) * k;
                        }
                    }
                    b[y * w + x] = v.map(|n| n as f32);
                }
            }
            std::mem::swap(&mut a, &mut b);
        }
        let mut out = self.clone();
        for (p, v) in out.rgba.chunks_exact_mut(4).zip(a) {
            let alpha = v[3];
            p[3] = alpha.round().clamp(0., 255.) as u8;
            for c in 0..3 {
                p[c] = if alpha > 0. {
                    (v[c] * 255. / alpha).round().clamp(0., 255.) as u8
                } else {
                    0
                };
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(w: u32, h: u32, px: &[[u8; 4]]) -> ImageSurface {
        ImageSurface {
            width: w,
            height: h,
            rgba: px.concat(),
        }
    }

    #[test]
    fn box_blur_truncates_the_clipped_window_mean() {
        // 3x1 row, 3x1 window: edge pixels average two neighbours.
        let s = surface(3, 1, &[[0, 0, 0, 255], [9, 30, 100, 255], [20, 0, 0, 255]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: None,
                area: [-1, 0, 1, 0],
                alpha_weighted: false,
            })
            .unwrap();
        assert_eq!(out.rgba, [4, 15, 50, 255, 9, 10, 33, 255, 14, 15, 50, 255]);
    }

    #[test]
    fn box_blur_rect_limits_writes_not_reads() {
        let s = surface(2, 1, &[[100, 0, 0, 255], [0, 0, 0, 255]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: Some([1, 0, 1, 1]),
                area: [-1, 0, 1, 0],
                alpha_weighted: false,
            })
            .unwrap();
        assert_eq!(out.rgba, [100, 0, 0, 255, 50, 0, 0, 255]);
    }

    #[test]
    fn alpha_weighted_box_blur_ignores_transparent_colour() {
        let s = surface(2, 1, &[[255, 0, 0, 255], [0, 255, 0, 0]]);
        let out = s
            .apply_effect(&ImageEffect::BoxBlur {
                rect: None,
                area: [-1, 0, 1, 0],
                alpha_weighted: true,
            })
            .unwrap();
        // Mean alpha 127; premultiplied red 255*256>>8 = 255 -> 127 -> 255*127/127.
        assert_eq!(&out.rgba[0..4], &[255, 0, 0, 127]);
    }
}

#[cfg(test)]
mod nearest_resize_tests {
    use super::*;
    #[test]
    fn proportional_nearest_keeps_top_left_phase_and_stored_alpha() {
        let source=ImageSurface{width:3,height:1,rgba:vec![10,20,30,0,40,50,60,127,70,80,90,255]};
        let result=source.apply_effect(&ImageEffect::ResizeNearest{width:5,height:2}).unwrap();
        let row=vec![10,20,30,0,10,20,30,0,40,50,60,127,40,50,60,127,70,80,90,255];
        assert_eq!(result.rgba,[row.clone(),row].concat());
        assert!(source.apply_effect(&ImageEffect::ResizeNearest{width:0,height:2}).is_err());
    }
}
