// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! One shared CPU image pipeline for native adapters and public bitmap jobs.
use crysirror_core::{Error, Result, image_processing::*, service::ClientStorage, state::ClientId};
use crysirror_image::{DecoderRegistry, ImageSurface, effects::ImageEffect};
use crysirror_tasks::{Task, TaskPool};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
pub(crate) mod bitmap_text;
pub(crate) mod encoded;
pub(crate) mod frame_image;
pub(crate) mod glyph;
pub(crate) mod glyph_batch;
pub(crate) mod vector_scene;
pub(crate) mod spectrum;

/// Live decoded resources plus in-flight decode/effect scratch.
const BUDGET: usize = 1024 * 1024 * 1024;
/// Resource decoding/composition has a separate bounded budget from expensive
/// image effects. Native sprite sheets routinely exceed a4MP effect target.
const MAX_RESOURCE_PIXELS: u64 = 16 * 1024 * 1024;
// Bounded scanline maps need more than64KiB even for a native600-row canvas.
// Rectangle counts and total pixel writes retain their independent limits.
const MAX_EFFECT_SPEC_BYTES: usize = 256 * 1024;
pub(crate) struct Charge {
    memory: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    pub(crate) fn shrink(&mut self, bytes: usize) {
        assert!(bytes <= self.bytes);
        self.memory.fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        self.bytes = bytes;
    }
    pub(crate) fn new(memory: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        memory
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                v.checked_add(bytes).filter(|v| *v <= BUDGET)
            })
            .map_err(|_| Error::Limit("image processing memory budget"))?;
        Ok(Self { memory, bytes })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.memory.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Resource {
    owner: ClientId,
    size: [u32; 2],
    opaque: bool,
    scalar_coverage: bool,
    pixels: Arc<[u8]>,
    _charge: Charge,
}
impl std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageResource")
            .field("client", &self.owner)
            .field("size", &self.size())
            .finish()
    }
}
impl ImageResource for Resource {
    fn client(&self) -> ClientId {
        self.owner
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.pixels.clone()
    }
    fn pixel_at(&self, point: [u32; 2]) -> crysirror_core::Result<Option<[u8; 4]>> {
        crysirror_core::image_processing::rgba_pixel_at(self.size, &self.pixels, point)
    }
    fn opaque(&self) -> bool {
        self.opaque
    }
    fn scalar_coverage(&self) -> Option<bool> {Some(self.scalar_coverage)}
}
struct Job {
    task: Option<Task<Arc<dyn ImageResource>>>,
    state: ImageProcessState,
}
pub(crate) fn begin_packed(
    owner: ClientId,
    spec: PackedImageSpec,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> Result<ImageJob> {
    let bytes = crysirror_image::rgba_len(spec.size[0], spec.size[1])
        .map_err(|e| Error::Io(e.to_string()))?;
    if bytes > 64 * 1024 * 1024 || spec.bytes.len() != bytes / 4 * spec.layout.bytes_per_pixel() {
        return Err(Error::Limit("packed image bytes"));
    }
    let output = Charge::new(memory.clone(), bytes)?;
    let scratch = Charge::new(memory, bytes * 2 + spec.bytes.len())?;
    let task = pool
        .reserve()
        .map_err(|e| Error::Io(e.to_string()))?
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let image = crysirror_image::packed_pixels::decode(
                spec.size,
                spec.layout,
                &spec.bytes,
                &|| cancel.check(),
            )?;
            resource_from_surface(owner, image, output, &|| cancel.check())
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}
impl ImageProcess for Job {
    fn poll(&mut self) -> Result<ImageProcessState> {
        if let Some(t) = &mut self.task {
            match t.poll_owner() {
                Ok(None) => (),
                Ok(Some(r)) => {
                    self.task = None;
                    self.state = ImageProcessState::Ready(r);
                }
                Err(e) => {
                    self.task = None;
                    self.state = ImageProcessState::Failed(e.to_string());
                }
            }
        }
        Ok(self.state.clone())
    }
}
pub(crate) fn begin(
    owner: ClientId,
    input: ImageInput,
    effects: Arc<[u8]>,
    storage: Arc<dyn ClientStorage>,
    decoders: DecoderRegistry,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> Result<ImageJob> {
    if effects.len() > MAX_EFFECT_SPEC_BYTES {
        return Err(Error::Limit("image effect specification"));
    }
    let effects: Vec<ImageEffect> =
        serde_json::from_slice(&effects).map_err(|e| Error::Io(e.to_string()))?;
    // An empty chain is resource preparation: decoding/ownership/publication
    // still follows this worker's normal reservation and cancellation fence.
    // Pair inputs retain their mandatory operation/context checks below.
    if effects.len() > 16 {
        return Err(Error::Limit("image effect chain length"));
    }
    for effect in &effects {
        effect.validate().map_err(|e| Error::Io(e.to_string()))?;
    }
    let mut graph_nodes = 0;
    let mut graph_bytes = 0;
    let input_extent = input_size(owner, &input, 0, &mut graph_nodes, &mut graph_bytes)?;
    let size = effect_output_size(input_extent, &effects)?;
    let scratch_bytes = effect_scratch_bytes(input_extent, &effects)?;
    if !effects.is_empty()
        && u64::from(size[0]) * u64::from(size[1]) > crysirror_image::effects::MAX_EFFECT_PIXELS
    {
        return Err(Error::Limit("image effect pixel budget"));
    }
    validate_effect_context(&input, &effects)?;
    let bytes =
        crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
    let output = Charge::new(memory.clone(), bytes)?;
    // Effect jobs reserve Gaussian planes/products. Decode-only jobs reserve
    // decoded products and bounded encoded input, without nonexistent blur planes.
    let scratch = Charge::new(
        memory.clone(),
        scratch_bytes
            + graph_bytes
            + if matches!(
                input,
                ImageInput::FieldWarp { .. }
                    | ImageInput::Process { .. }
                    | ImageInput::Raster { .. }
                    | ImageInput::Composite { .. }
                    | ImageInput::Ternary { .. }
            ) && has_encoded_dependencies(&input)
            {
                128 * 1024 * 1024
            } else {
                0
            },
    )?;
    // Admission pressure is retryable; malformed input and memory-budget
    // failures remain distinct. Callers keep their native PC until accepted.
    let permit = pool.reserve().map_err(|e| if e.is::<crysirror_tasks::CapacityExceeded>() {
        Error::Limit("image preparation task capacity")
    } else {Error::Io(e.to_string())})?;
    let task = permit
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let image = process_input(
                owner,
                &input,
                &effects,
                &storage,
                &decoders,
                &cancel,
                Some(&memory),
            )?;
            resource_from_surface(owner, image, output, &|| cancel.check())
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}

fn parse_effect_chain(effects: &[u8]) -> Result<Vec<ImageEffect>> {
    if effects.len() > MAX_EFFECT_SPEC_BYTES {
        return Err(Error::Limit("image nested effect specification"));
    }
    let effects: Vec<ImageEffect> =
        serde_json::from_slice(effects).map_err(|e| Error::Io(e.to_string()))?;
    if effects.len() > 16 {
        return Err(Error::Limit("image nested effect chain length"));
    }
    for effect in &effects {
        effect.validate().map_err(|e| Error::Io(e.to_string()))?;
    }
    Ok(effects)
}
fn effect_scratch_factor(effects: &[ImageEffect]) -> usize {
    if effects
        .iter()
        .any(|e| matches!(e, ImageEffect::GaussianBlur { .. }))
    {
        16
    } else if effects
        .iter()
        .any(|e| matches!(e, ImageEffect::BoxBlur { .. }))
    {
        10
    } else if effects
        .iter()
        .any(|e| matches!(e, ImageEffect::IntegerRaster { .. }))
    {
        6
    } else {
        3
    }
}
fn effect_output_size(mut size: [u32; 2], effects: &[ImageEffect]) -> Result<[u32; 2]> {
    for effect in effects {
        match effect {
            ImageEffect::NinePatch{width,height}=>{size=[*width,*height];}
            ImageEffect::TileCompose{spec}=>{size=spec.canvas;}
            ImageEffect::CoverageBitmap{spec}=>{size=spec.size;}
            ImageEffect::GlyphCoverageBlur{radius,..}=>{
                size=[size[0].checked_add(radius*2).ok_or(Error::Limit("coverage width"))?,size[1].checked_add(radius*2).ok_or(Error::Limit("coverage height"))?];
            }
            ImageEffect::RadialWavePair{spec}=>{size=spec.canvas;}
            ImageEffect::RectMap { spec } => {
                size = spec.output_size.unwrap_or(size);
            }
            ImageEffect::BoxReduce { spec } => {
                size = spec
                    .output_size(size)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::ResizeByte { spec } => {
                size = spec
                    .output_size(size)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::RectResampleByte256 { spec } => {
                size = spec
                    .output_size(size)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::RectResampleByte255 { spec } => {
                size = spec
                    .output_size(size)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::CanvasMargins { margins, .. } => {
                size = crysirror_image::canvas_margins::output_size(size, *margins)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::TriangularBlur { spec } => {
                size[0] = size[0]
                    .checked_add(spec.padding[0])
                    .and_then(|v| v.checked_add(spec.padding[2]))
                    .ok_or(Error::Limit("triangular blur output width"))?;
                size[1] = size[1]
                    .checked_add(spec.padding[1])
                    .and_then(|v| v.checked_add(spec.padding[3]))
                    .ok_or(Error::Limit("triangular blur output height"))?;
            }
            ImageEffect::Resize { width, height } | ImageEffect::ResizeNearest { width, height } => {
                size = [*width, *height];
            }
            ImageEffect::RowDeform { spec } => {
                size = spec
                    .output_size(size)
                    .map_err(|e| Error::Io(e.to_string()))?
            }
            ImageEffect::Rotate { degrees } if matches!(*degrees, 90 | 270) => size.swap(0, 1),
            ImageEffect::CelConvert { spec } => size = spec.output_size(size),
            _ => (),
        }
        crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
        if u64::from(size[0]) * u64::from(size[1]) > crysirror_image::effects::MAX_EFFECT_PIXELS {
            return Err(Error::Limit("image effect output pixel budget"));
        }
    }
    Ok(size)
}
fn effect_scratch_bytes(input: [u32; 2], effects: &[ImageEffect]) -> Result<usize> {
    let peak = effect_peak_bytes(input, effects)?;
    let mut size = input;
    let mut columns = 0usize;
    let mut retained_metadata = 0usize;
    for effect in effects {
        if let ImageEffect::RadialRgbBrushes{spec}=effect {
            retained_metadata=retained_metadata.checked_add(spec.scratch_bytes().map_err(|e|Error::Io(e.to_string()))?+spec.positions.len()*std::mem::size_of::<[i32;2]>()).ok_or(Error::Limit("radial RGB brush scratch budget"))?;
        }
        if let ImageEffect::PackedPairBatch{spec}=effect {
            retained_metadata=retained_metadata.checked_add(spec.operations.len()*std::mem::size_of::<crysirror_image::packed_pair_blend::PackedPairBlend>()).ok_or(Error::Limit("packed pair batch scratch budget"))?;
        }
        if let ImageEffect::RadialWavePair{spec}=effect {
            let count=spec.radius_count().map_err(|error|Error::Io(error.to_string()))?;
            retained_metadata=retained_metadata.checked_add(count*std::mem::size_of::<i16>()).ok_or(Error::Limit("radial wave table scratch budget"))?;
        }
        if let ImageEffect::StoredWordSpans{spec}=effect {
            retained_metadata=retained_metadata.checked_add(spec.spans.len()*std::mem::size_of::<crysirror_image::stored_word_spans::StoredWordSpan>())
                .ok_or(Error::Limit("stored word metadata scratch budget"))?;
        }
        if let ImageEffect::StoredGradientSpans{spec}=effect {
            retained_metadata=retained_metadata.checked_add(spec.spans.len()*std::mem::size_of::<crysirror_image::stored_gradient_spans::StoredGradientSpan>())
                .ok_or(Error::Limit("stored gradient metadata scratch budget"))?;
        }
        if let ImageEffect::ByteRaster{spec}=effect {
            match spec {
                crysirror_image::byte_raster::ByteRaster::FloatingPolygonFragments{spec}=>{
                    let bytes=spec.scratch_bytes().map_err(|e|Error::Io(e.to_string()))?+spec.fragments.iter().map(|f|f.vertices.len()*std::mem::size_of::<[i32;2]>()+std::mem::size_of::<crysirror_image::integer_wipe::fragments::ImageFragment>()).sum::<usize>();
                    retained_metadata=retained_metadata.checked_add(bytes).ok_or(Error::Limit("floating polygon scratch budget"))?;
                }
                crysirror_image::byte_raster::ByteRaster::AlphaBrushes{spec}=>{
                    retained_metadata=retained_metadata.checked_add(spec.brushes.len()*std::mem::size_of::<crysirror_image::alpha_brushes::AlphaBrush>()).ok_or(Error::Limit("alpha brush metadata scratch budget"))?;
                }
                crysirror_image::byte_raster::ByteRaster::PackedRepeatRows{spec}=>{
                    retained_metadata=retained_metadata.checked_add(spec.rows.len()*std::mem::size_of::<crysirror_image::packed_repeat_rows::PackedRepeatRow>()).ok_or(Error::Limit("packed repeat row metadata scratch budget"))?;
                }
                crysirror_image::byte_raster::ByteRaster::RowBlitBatch{spec}=>{
                    let bytes=spec.operations.iter().map(|op|std::mem::size_of::<crysirror_image::row_blit::RowBlit>()+op.row_offsets.len()*std::mem::size_of::<i32>()+op.row_math.len()*std::mem::size_of::<crysirror_image::byte_raster::ByteRasterMath>()).sum::<usize>();
                    retained_metadata=retained_metadata.checked_add(bytes).ok_or(Error::Limit("row blit batch metadata scratch budget"))?;
                }
                crysirror_image::byte_raster::ByteRaster::AxisBoxSample{spec}=>{
                    let cross=if spec.vertical {size[0]}else{size[1]} as usize;
                    columns=columns.max(cross*std::mem::size_of::<[u16;3]>());
                }
                crysirror_image::byte_raster::ByteRaster::RepeatScaleSample{..}=>{
                    retained_metadata=retained_metadata.checked_add((size[0] as usize+size[1] as usize)*std::mem::size_of::<i64>())
                        .ok_or(Error::Limit("repeated scale index scratch budget"))?;
                }
                _=>(),
            }
        }
        if let ImageEffect::ScanlineAffineNearest { spec } = effect {
            retained_metadata = retained_metadata.checked_add(spec.row_origins.len() * std::mem::size_of::<[i64; 2]>())
                .ok_or(Error::Limit("scanline affine metadata budget"))?;
        }
        if let ImageEffect::CyclicCellSample { spec } = effect {
            retained_metadata = retained_metadata.checked_add(spec.offsets.len() * std::mem::size_of::<[u16;2]>())
                .ok_or(Error::Limit("cyclic-cell metadata budget"))?;
        }
        if let ImageEffect::TileReveal { spec } = effect {
            retained_metadata=retained_metadata.checked_add(spec.thresholds.len())
                .ok_or(Error::Limit("tile reveal metadata budget"))?;
        }
        if let ImageEffect::RadialAngleLookup { spec } = effect {
            retained_metadata=retained_metadata.checked_add(spec.auxiliary_bytes())
                .ok_or(Error::Limit("radial angle metadata budget"))?;
        }
        if matches!(effect, ImageEffect::AffineBlend { .. }) {
            columns = columns
                .max(crysirror_image::affine_blend::AffineBlendByte128::auxiliary_bytes(size));
        }
        if matches!(
            effect,
            ImageEffect::BoxBlurClamped { .. } | ImageEffect::MaskedBlur { .. }
        ) {
            // Four u64 column sums per source column. A one-row texture
            // needs more scratch per texel than a plane multiplier admits.
            columns = columns.max(size[0] as usize * std::mem::size_of::<[u64; 4]>());
        }
        size = effect_output_size(size, std::slice::from_ref(effect))?;
    }
    peak.checked_mul(effect_scratch_factor(effects))
        .and_then(|bytes| bytes.checked_add(columns))
        .and_then(|bytes| bytes.checked_add(retained_metadata))
        .ok_or(Error::Limit("image effect scratch budget"))
}
// Intermediate expansion still consumes memory when a later operation crops
// back down. Reserve the largest admitted plane, not only the final output.
fn effect_peak_bytes(size: [u32; 2], effects: &[ImageEffect]) -> Result<usize> {
    let mut peak =
        crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
    for end in 1..=effects.len() {
        let at = effect_output_size(size, &effects[..end])?;
        peak = peak
            .max(crysirror_image::rgba_len(at[0], at[1]).map_err(|e| Error::Io(e.to_string()))?);
    }
    Ok(peak)
}
fn validate_effect_context(input: &ImageInput, effects: &[ImageEffect]) -> Result<()> {
    let pair = matches!(
        input,
        ImageInput::Transition { .. } | ImageInput::Raster { .. } | ImageInput::Ternary { .. }
    );
    let pair_op = |e: &ImageEffect| {
        matches!(
            e,
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
                | ImageEffect::PackedPairBatch{..} | ImageEffect::PackedPairBlend{..}
                | ImageEffect::MaskedPairBlend { .. }
                | ImageEffect::IntegerRaster { .. }
                | ImageEffect::ByteRaster { .. }
        )
    };
    if effects.iter().skip(1).any(pair_op) || effects.first().is_some_and(pair_op) != pair {
        return Err(Error::Invalid("image input/effect context"));
    }
    if matches!(input, ImageInput::Transition { .. })
        && !matches!(effects.first(), Some(ImageEffect::Transition { .. }))
    {
        return Err(Error::Invalid("transition input context"));
    }
    if matches!(input, ImageInput::Raster { .. })
        && !matches!(
            effects.first(),
            Some(
                ImageEffect::DualTexture { .. }
                    | ImageEffect::AffineBlend { .. }
                    | ImageEffect::FixedAffineNearest { .. }
                    | ImageEffect::ScanlineAffineNearest { .. }
                    | ImageEffect::TileReveal { .. }
                    | ImageEffect::AlphaStamps { .. }
                    | ImageEffect::CopyChannel { .. }
                    | ImageEffect::SineRowCopy { .. }
                    | ImageEffect::DiscCopies { .. }
                    | ImageEffect::AlphaClipRect { .. }
                    | ImageEffect::ShrinkCopy { .. }
                    | ImageEffect::OverwrapRect { .. }
                | ImageEffect::StitchWrappedCopy { .. }
                | ImageEffect::GlitchCopy { .. }
                    | ImageEffect::IntegerRaster { .. }
                    | ImageEffect::ByteRaster { .. }
            )
        )
    {
        return Err(Error::Invalid("row-copy input context"));
    }
    if matches!(input, ImageInput::Ternary { .. })
        && !matches!(
            effects.first(),
            Some(
                ImageEffect::PackedPairBatch{..} | ImageEffect::PackedPairBlend{..}
                    | ImageEffect::MaskedPairBlend { .. }
                    | ImageEffect::MaskFieldPair{..}
                | ImageEffect::PackedColorMask{..}
                    | ImageEffect::RadialWavePair{..}
                | ImageEffect::LinearMaskOffset{..}
                | ImageEffect::PackedDualMask{..}
                    | ImageEffect::AlphaMaskPlane { .. }
                    | ImageEffect::StoredMaskBlend { .. }
                    | ImageEffect::AlphaPlanePair { .. }
            )
        )
    {
        return Err(Error::Invalid("ternary image input context"));
    }
    Ok(())
}
fn process_input(
    owner: ClientId,
    input: &ImageInput,
    effects: &[ImageEffect],
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
    encoded_memory: Option<&Arc<AtomicUsize>>,
) -> anyhow::Result<ImageSurface> {
    let load_input =
        |input: &ImageInput| load_inner(owner, input, storage, decoders, cancel, encoded_memory);
    let (mut image, other, mask) = match input {
        ImageInput::Transition { from, to, mask } => (
            load_input(&ImageInput::Resource(from.clone()))?,
            Some(load_input(&ImageInput::Resource(to.clone()))?),
            mask.as_ref()
                .map(|r| load_input(&ImageInput::Resource(r.clone())))
                .transpose()?,
        ),
        ImageInput::Raster {
            destination,
            source,
        } => (load_input(&destination)?, Some(load_input(&source)?), None),
        ImageInput::Ternary {
            destination,
            first,
            second,
        } => (
            load_input(&destination)?,
            Some(load_input(&first)?),
            Some(load_input(&second)?),
        ),
        input => (load_input(&input)?, None, None),
    };
    for effect in effects {
        cancel.check()?;
        image = if let ImageEffect::DualTexture { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("dual texture source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::AffineBlend { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("affine blend source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::FixedAffineNearest { spec } = &effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("fixed affine source absent"))?,&||cancel.check())?
        } else if let ImageEffect::ScanlineAffineNearest { spec } = &effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("scanline affine source absent"))?,&||cancel.check())?
        } else if let ImageEffect::TileReveal { spec } = &effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("tile reveal source absent"))?,&||cancel.check())?
        } else if let ImageEffect::CopyChannel { source_channel, destination_channels, invert } = &effect {
            image.copy_channel(
                other.as_ref().ok_or_else(|| anyhow::anyhow!("stored channel source absent"))?,
                *source_channel, *destination_channels, *invert, &|| cancel.check(),
            )?
        } else if let ImageEffect::AlphaStamps { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("alpha stamp source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::ByteRaster { spec } = &effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("byte raster source absent"))?,&||cancel.check())?
        } else if let ImageEffect::IntegerRaster { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("integer raster source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::AlphaMaskPlane { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("alpha baseline absent"))?,
                mask.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("alpha mask absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::AlphaPlanePair { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("first channel plane absent"))?,
                mask.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("second channel plane absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::MaskFieldPair{spec}=&effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("mask-field second image absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("mask-field image absent"))?,&||cancel.check())?
        } else if let ImageEffect::PackedColorMask{spec}=&effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("colour-mask source absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("colour-mask plane absent"))?,&||cancel.check())?
        } else if let ImageEffect::RadialWavePair{spec}=&effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("radial wave first image absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("radial wave second image absent"))?,&||cancel.check())?
        } else if let ImageEffect::LinearMaskOffset{spec}=&effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("linear mask first input absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("linear mask second input absent"))?,&||cancel.check())?
        } else if let ImageEffect::PackedDualMask{spec}=&effect {
            spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("dual-mask source absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("dual-mask plane absent"))?,&||cancel.check())?
        } else if let ImageEffect::StoredMaskBlend { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("stored mask source absent"))?,
                mask.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("stored mask resource absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::PackedPairBatch {spec}=&effect {
            spec.apply(&image,
                other.as_ref().ok_or_else(||anyhow::anyhow!("packed pair batch first input absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("packed pair batch second input absent"))?,
                &||cancel.check())?
        } else if let ImageEffect::PackedPairBlend {spec}=&effect {
            spec.apply(&image,
                other.as_ref().ok_or_else(||anyhow::anyhow!("packed pair first input absent"))?,
                mask.as_ref().ok_or_else(||anyhow::anyhow!("packed pair second input absent"))?,
                &||cancel.check())?
        } else if let ImageEffect::MaskedPairBlend { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("masked pair first input absent"))?,
                mask.as_ref()
                    .ok_or_else(|| anyhow::anyhow!("masked pair second input absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::AlphaClipRect { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("alpha clip source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::ShrinkCopy { spec } = &effect {
            cancel.check()?;
            spec.apply(&image, other.as_ref().ok_or_else(|| anyhow::anyhow!("shrinkCopy source absent"))?)?
        } else if let ImageEffect::GlitchCopy { spec } = &effect {
            cancel.check()?;
            spec.apply(&image, other.as_ref().ok_or_else(|| anyhow::anyhow!("glitchCopy source absent"))?)?
        } else if let ImageEffect::StitchWrappedCopy { spec } = &effect {
            cancel.check()?;
            spec.apply(&image, other.as_ref().ok_or_else(|| anyhow::anyhow!("stitchWrappedCopy source absent"))?)?
        } else if let ImageEffect::OverwrapRect { spec } = &effect {
            cancel.check()?;
            spec.apply(&image, other.as_ref().ok_or_else(|| anyhow::anyhow!("overwrapRect source absent"))?)?
        } else if let ImageEffect::DiscCopies { spec } = &effect {
            spec.apply(
                &image,
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("disc copy source absent"))?,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::SineRowCopy { spec } = &effect {
            image.sine_copy_from(
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("row-copy source absent"))?,
                spec,
                &|| cancel.check(),
            )?
        } else if let ImageEffect::Transition { spec } = &effect {
            image.transition_to(
                other
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("transition source absent"))?,
                mask.as_ref(),
                spec,
                &|| cancel.check(),
            )?
        } else {
            image.apply_effect_cancellable(&effect, &|| cancel.check())?
        };
    }
    Ok(image)
}

// Composition graphs with encoded plugin formats may request auxiliary sources
// during decode. Keep their existing dependency reservation. Asset inputs
// use bounded streams and reserve their actual encoded length on the worker.
fn has_encoded_dependencies(input: &ImageInput) -> bool {
    match input {
        ImageInput::FieldWarp { source, .. } => has_encoded_dependencies(source),
        ImageInput::Process { input, .. } => has_encoded_dependencies(input),
        ImageInput::Encoded { .. } => true,
        ImageInput::Composite { base, layers } => {
            has_encoded_dependencies(base)
                || layers.iter().any(|l| has_encoded_dependencies(&l.input))
        }
        ImageInput::Raster {
            destination,
            source,
        } => has_encoded_dependencies(destination) || has_encoded_dependencies(source),
        ImageInput::Ternary {
            destination,
            first,
            second,
        } => {
            has_encoded_dependencies(destination)
                || has_encoded_dependencies(first)
                || has_encoded_dependencies(second)
        }
        _ => false,
    }
}

pub(crate) fn input_size(
    owner: ClientId,
    input: &ImageInput,
    depth: usize,
    nodes: &mut usize,
    bytes: &mut usize,
) -> Result<[u32; 2]> {
    *nodes += 1;
    // A full 800x600 tile grid (25x19 layers plus its base) is one ordinary
    // composition; byte budgets still bound the work.
    if depth > crysirror_core::image_processing::IMAGE_GRAPH_MAX_DEPTH
        || *nodes > crysirror_core::image_processing::IMAGE_GRAPH_MAX_NODES
    {
        return Err(Error::Limit("image composition graph"));
    }

    let size = match input {
        ImageInput::ParticlePlane { size, sprites } => {
            let scratch = crysirror_image::particle::working_bytes(owner, sprites)
                .map_err(|e| Error::Io(e.to_string()))?;
            *bytes = bytes
                .checked_add(scratch)
                .filter(|v| *v <= 256 * 1024 * 1024)
                .ok_or(Error::Limit("particle image worker byte budget"))?;
            *size
        }
        ImageInput::PyramidLevel { pyramid, level } => {
            pyramid.validate(owner)?;
            let plane = pyramid
                .levels
                .get(usize::from(*level))
                .ok_or(Error::Invalid("image pyramid level"))?;
            *bytes = bytes
                .checked_add(pyramid.byte_len())
                .filter(|n| *n <= 256 * 1024 * 1024)
                .ok_or(Error::Limit("image pyramid graph byte budget"))?;
            plane.size
        }

        ImageInput::FieldWarp { source, size, spec } => {
            let field_bytes = spec.validate(owner, *size)?;
            if matches!(
                source.as_ref(),
                ImageInput::Raster { .. }
                    | ImageInput::Ternary { .. }
                    | ImageInput::Transition { .. }
            ) {
                return Err(Error::Invalid("unprocessed vector source context"));
            }
            input_size(owner, source, depth + 1, nodes, bytes)?;
            *bytes = bytes
                .checked_add(field_bytes)
                .filter(|n| *n <= 256 * 1024 * 1024)
                .ok_or(Error::Limit("vector field graph byte budget"))?;
            *size
        }
        ImageInput::Process { input, effects } => {
            let effects = parse_effect_chain(effects)?;
            validate_effect_context(input, &effects)?;
            let size = input_size(owner, input, depth + 1, nodes, bytes)?;
            let scratch = effect_scratch_bytes(size, &effects)?;
            let size = effect_output_size(size, &effects)?;
            if !effects.is_empty() {
                if u64::from(size[0]) * u64::from(size[1])
                    > crysirror_image::effects::MAX_EFFECT_PIXELS
                {
                    return Err(Error::Limit("nested image effect pixel budget"));
                }
                *bytes = bytes
                    .checked_add(scratch)
                    .filter(|n| *n <= 256 * 1024 * 1024)
                    .ok_or(Error::Limit("nested image scratch budget"))?;
            }
            size
        }

        ImageInput::Encoded {
            name,
            bytes: encoded,
            size,
        } => {
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, name)?;
            if encoded.is_empty() || encoded.len() > 64 * 1024 * 1024 {
                return Err(Error::Limit("image encoded byte budget"));
            }
            *bytes = bytes
                .checked_add(encoded.len())
                .filter(|n| *n <= 256 * 1024 * 1024)
                .ok_or(Error::Limit("image composition byte budget"))?;
            *size
        }
        ImageInput::Composite { base, layers } => {
            let size = input_size(owner, base, depth + 1, nodes, bytes)?;
            //Repeated layers over one immutable resident resource share its
            //pixels; they are separate graph nodes but not separate bytes.
            let mut resident = std::collections::BTreeSet::new();
            for layer in layers {
                if let ImageCompositeMath::IntegerStored {
                    destination_format,
                    source_format,
                    mode,
                    parameter,
                } = layer.math
                {
                    if layer.opacity != 255
                        || destination_format > 3
                        || source_format > 3
                        || !matches!(mode,0..=9|0x20..=0x27|0x40|0x41|0x80|0xc0|0xc1|0xf0|0xff)
                        || !(0..=256).contains(&parameter)
                    {
                        return Err(Error::Invalid("stored-byte composite mode/format/opacity"));
                    }
                }
                if let ImageCompositeMath::Normalized(state) = layer.math {
                    state.validate()?;
                    if layer.opacity != 255 {
                        return Err(Error::Invalid("normalized blend requires opacity255"));
                    }
                }
                if matches!(layer.math, ImageCompositeMath::MaskSeparate256 | ImageCompositeMath::MaskIncrement256) && layer.opacity != 255
                {
                    return Err(Error::Invalid("mask_separate256 requires opacity255"));
                }
                if let ImageCompositeMath::MaskPiecewise256 { weight } = layer.math {
                    if weight > 256 || layer.opacity != 255 {
                        return Err(Error::Invalid("piecewise mask weight/opacity"));
                    }
                }
                if layer.source[2] < 0
                    || layer.source[3] < 0
                    || layer
                        .source
                        .iter()
                        .chain(&layer.destination)
                        .any(|v| v.unsigned_abs() > 1_000_000)
                {
                    return Err(Error::Invalid("image composite coordinates"));
                }
                if layer.clip.is_some_and(|clip| {
                    clip[2] < 0 || clip[3] < 0 || clip.iter().any(|v| v.unsigned_abs() > 1_000_000)
                }) {
                    return Err(Error::Invalid("image composite clip"));
                }
                if matches!(
                    layer.input.as_ref(),
                    ImageInput::Raster { .. }
                        | ImageInput::Transition { .. }
                        | ImageInput::Ternary { .. }
                ) {
                    return Err(Error::Invalid("composite pair-effect input"));
                }
                if let ImageInput::Resource(r) = layer.input.as_ref() {
                    let key = std::sync::Arc::as_ptr(r) as *const () as usize;
                    if !resident.insert(key) {
                        *nodes += 1;
                        if *nodes > 1024 {
                            return Err(Error::Limit("image composition graph"));
                        }
                        if r.client() != owner {
                            return Err(Error::Invalid("foreign image resource"));
                        }
                        continue;
                    }
                }
                input_size(owner, &layer.input, depth + 1, nodes, bytes)?;
            }
            if matches!(
                base.as_ref(),
                ImageInput::Raster { .. }
                    | ImageInput::Transition { .. }
                    | ImageInput::Ternary { .. }
            ) {
                return Err(Error::Invalid("composite pair-effect base"));
            }
            size
        }
        ImageInput::Asset { path, size, .. } => {
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, path)?;
            *size
        }
        ImageInput::ScopedAsset { size, .. } => *size,
        ImageInput::Solid { size, .. } => *size,
        ImageInput::Resource(r) => {
            if r.client() != owner {
                return Err(Error::Invalid("foreign image resource"));
            }
            r.size()
        }
        ImageInput::Transition { from, to, mask } => {
            let size = from.size();
            for r in std::iter::once(from)
                .chain(std::iter::once(to))
                .chain(mask.iter())
            {
                if r.client() != owner {
                    return Err(Error::Invalid("foreign transition resource"));
                }
                if r.size() != size {
                    return Err(Error::Invalid("transition dimensions"));
                }
            }
            size
        }
        ImageInput::Ternary {
            destination,
            first,
            second,
        } => {
            if [destination.as_ref(), first.as_ref(), second.as_ref()]
                .iter()
                .any(|v| {
                    matches!(
                        v,
                        ImageInput::Raster { .. }
                            | ImageInput::Transition { .. }
                            | ImageInput::Ternary { .. }
                    )
                })
            {
                return Err(Error::Invalid("nested image input"));
            }
            let size = input_size(owner, destination, depth + 1, nodes, bytes)?;
            input_size(owner, first, depth + 1, nodes, bytes)?;
            input_size(owner, second, depth + 1, nodes, bytes)?;
            size
        }
        ImageInput::Raster {
            destination,
            source,
        } => {
            if [destination.as_ref(), source.as_ref()].iter().any(|v| {
                matches!(
                    v,
                    ImageInput::Raster { .. }
                        | ImageInput::Transition { .. }
                        | ImageInput::Ternary { .. }
                )
            }) {
                return Err(Error::Invalid("nested image input"));
            }
            let size = input_size(owner, destination, depth + 1, nodes, bytes)?;
            let source_size = input_size(owner, source, depth + 1, nodes, bytes)?;
            // A two-input worker may prepare successive half-size sample
            //planes while retaining the immutable full source. Their live
            //peak is bounded by half of the source plane's byte count.
            let mip_bytes = crysirror_image::rgba_len(source_size[0], source_size[1])
                .map_err(|e| Error::Io(e.to_string()))?
                / 2;
            *bytes = bytes
                .checked_add(mip_bytes)
                .filter(|v| *v <= 256 * 1024 * 1024)
                .ok_or(Error::Limit("image source sampling scratch"))?;
            size
        }
    };
    *bytes = bytes
        .checked_add(
            crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?,
        )
        .filter(|n| *n <= 256 * 1024 * 1024)
        .ok_or(Error::Limit("image composition byte budget"))?;
    if u64::from(size[0]) * u64::from(size[1]) > MAX_RESOURCE_PIXELS {
        return Err(Error::Limit("image resource pixel budget"));
    }
    Ok(size)
}
pub(crate) fn load(
    owner: ClientId,
    input: &ImageInput,
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
) -> anyhow::Result<ImageSurface> {
    load_inner(owner, input, storage, decoders, cancel, None)
}

fn load_inner(
    owner: ClientId,
    input: &ImageInput,
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
    encoded_memory: Option<&Arc<AtomicUsize>>,
) -> anyhow::Result<ImageSurface> {
    cancel.check()?;
    match input {
        ImageInput::ParticlePlane { size, sprites } => {
            crysirror_image::particle::plane(owner, *size, sprites, &|| cancel.check())
        }
        ImageInput::PyramidLevel { pyramid, level } => {
            pyramid.validate(owner)?;
            let plane = pyramid
                .levels
                .get(usize::from(*level))
                .ok_or_else(|| anyhow::anyhow!("image pyramid level"))?;
            Ok(ImageSurface {
                width: plane.size[0],
                height: plane.size[1],
                rgba: plane.pixels.to_vec(),
            })
        }
        ImageInput::FieldWarp { source, size, spec } => {
            spec.validate(owner, *size)?;
            let source = load_inner(owner, source, storage, decoders, cancel, encoded_memory)?;
            crysirror_image::vector_field::warp(&source, *size, spec, &|| cancel.check())
        }
        ImageInput::Process { input, effects } => {
            let effects = parse_effect_chain(effects)?;
            validate_effect_context(input, &effects)?;
            process_input(
                owner,
                input,
                &effects,
                storage,
                decoders,
                cancel,
                encoded_memory,
            )
        }
        ImageInput::Encoded { name, bytes, size } => {
            cancel.check()?;
            let resources =
                crysirror_resource::DecodeContext::new(owner).with_cancellation(cancel.clone());
            let source = |path: &str| {
                let path =
                    crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, path)?;
                storage.read(&path)
            };
            let image = decoders.decode_with(name, bytes, Some(&source), Some(&resources), None)?;
            cancel.check()?;
            anyhow::ensure!(
                [image.width, image.height] == *size,
                "encoded image dimensions changed during processing"
            );
            Ok(image)
        }
        ImageInput::Composite { base, layers } => {
            use crysirror_image::bitmap::{BitmapRect, BlitOptions, PixelMath};
            let mut image = load_inner(owner, base, storage, decoders, cancel, encoded_memory)?;
            for layer in layers {
                cancel.check()?;
                let (mut rect, mut destination) = (layer.source, layer.destination);
                if let Some([x, y, width, height]) = layer.clip {
                    let left = destination[0].max(x);
                    let top = destination[1].max(y);
                    let right = (destination[0] + rect[2]).min(x + width);
                    let bottom = (destination[1] + rect[3]).min(y + height);
                    if right <= left || bottom <= top {
                        continue;
                    }
                    rect[0] += left - destination[0];
                    rect[1] += top - destination[1];
                    rect[2] = right - left;
                    rect[3] = bottom - top;
                    destination = [left, top];
                }
                let source = load_inner(
                    owner,
                    &layer.input,
                    storage,
                    decoders,
                    cancel,
                    encoded_memory,
                )?;
                if let ImageCompositeMath::IntegerStored {
                    destination_format,
                    source_format,
                    mode,
                    parameter,
                } = layer.math
                {
                    crysirror_image::integer_raster::region::blit_stored_region(
                        &mut image,
                        destination_format,
                        &source,
                        source_format,
                        rect,
                        destination,
                        mode,
                        parameter,
                        &|| {
                            cancel.check()?;
                            Ok(())
                        },
                    )?;
                    continue;
                }
                if let ImageCompositeMath::Normalized(state) = layer.math {
                    image.blend_fixed_factors(
                        &source,
                        BitmapRect::new(rect[0], rect[1], rect[2] as u32, rect[3] as u32),
                        destination,
                        state,
                        &|| {
                            cancel.check()?;
                            Ok(())
                        },
                    )?;
                    continue;
                }
                let mode = match layer.math {
                    ImageCompositeMath::Normalized(_) => unreachable!("normalized handled above"),
                    ImageCompositeMath::IntegerStored { .. } => {
                        unreachable!("stored-byte blend handled above")
                    }
                    ImageCompositeMath::Copy => PixelMath::Copy,
                    ImageCompositeMath::CopyRgb => PixelMath::CopyRgb,
                    ImageCompositeMath::CopyAlpha => PixelMath::CopyAlpha,
                    ImageCompositeMath::MaskSeparate256 => PixelMath::MaskSeparate256,
                    ImageCompositeMath::MaskIncrement256 => PixelMath::MaskIncrement256,
                    ImageCompositeMath::OpaqueScalar256 => PixelMath::OpaqueScalar256,
                    ImageCompositeMath::MaskScalar256 => PixelMath::MaskScalar256,
                    ImageCompositeMath::MaskHoldAlphaScalar256 => PixelMath::MaskHoldAlphaScalar256,
                    ImageCompositeMath::MaskPiecewise256 { weight } => {
                        PixelMath::MaskPiecewise256 { weight }
                    }
                    ImageCompositeMath::Ps { blend, hold_alpha } => {
                        use crysirror_core::image_processing::PsBlend as C;
                        use crysirror_image::bitmap::PsBlend as I;
                        let blend = match blend {
                            C::Normal => I::Normal,
                            C::Overlay => I::Overlay,
                            C::HardLight => I::HardLight,
                            C::SoftLight => I::SoftLight,
                            C::ColorDodge => I::ColorDodge,
                            C::ColorBurn => I::ColorBurn,
                            C::Lighten => I::Lighten,
                            C::Darken => I::Darken,
                            C::Additive => I::Additive,
                            C::Subtractive => I::Subtractive,
                            C::Multiplicative => I::Multiplicative,
                            C::Screen => I::Screen,
                            C::ColorDodge5 => I::ColorDodge5,
                            C::Difference => I::Difference,
                            C::Difference5 => I::Difference5,
                            C::Exclusion => I::Exclusion,
                        };
                        PixelMath::Ps { blend, hold_alpha }
                    }
                    ImageCompositeMath::OpaqueOnAlphaScalar256 => PixelMath::OpaqueOnAlphaScalar256,
                    ImageCompositeMath::AlphaOnAlphaScalar256 => PixelMath::AlphaOnAlphaScalar256,
                };
                // A bounded row is the cancellation granularity; no worker owns
                // native VM state and publication occurs only after all rows.
                for row in 0..rect[3] {
                    if row % 32 == 0 {
                        cancel.check()?;
                    }
                    image.blit(
                        &source,
                        BlitOptions {
                            source_rect: BitmapRect::new(rect[0], rect[1] + row, rect[2] as u32, 1),
                            destination: [destination[0], destination[1] + row],
                            opacity: layer.opacity,
                            mode,
                            color_key: None,
                        },
                    )?;
                }
            }
            Ok(image)
        }
        ImageInput::Solid { size, color } => ImageSurface::filled(size[0], size[1], *color),
        ImageInput::Resource(r) => {
            let size = r.size();
            let pixels = r.rgba();
            anyhow::ensure!(
                pixels.len() == crysirror_image::rgba_len(size[0], size[1])?,
                "invalid image resource pixels"
            );
            Ok(ImageSurface {
                width: size[0],
                height: size[1],
                rgba: pixels.to_vec(),
            })
        }
        ImageInput::Asset {
            path,
            size,
            fingerprint,
        } => {
            use crysirror_core::path::{Mount, VfsPath};
            let path = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
            load_scoped_asset(&path, *size, fingerprint.as_ref(), storage, decoders, cancel, encoded_memory)
        }
        ImageInput::ScopedAsset {path,size,fingerprint} => {
            load_scoped_asset(path, *size, fingerprint.as_ref(), storage, decoders, cancel, encoded_memory)
        }
        _ => anyhow::bail!("nested image input is not supported"),
    }
}

fn load_scoped_asset(
    path: &crysirror_core::path::VfsPath,
    size: [u32; 2],
    fingerprint: Option<&[u8; 32]>,
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
    encoded_memory: Option<&Arc<AtomicUsize>>,
) -> anyhow::Result<ImageSurface> {
    cancel.check()?;
    let reader =
        if encoded_memory.is_some() || path.mount() != crysirror_core::path::Mount::Asset {
            storage.open_read(path)
        } else {
            storage.open_asset(path)
        }
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
    anyhow::ensure!(n <= 64 * 1024 * 1024, "image encoded byte budget");
    let _encoded_charge = encoded_memory
        .map(|memory| Charge::new(memory.clone(), n as usize))
        .transpose()?;
    let mut bytes = vec![0; n as usize];
    let mut at = 0;
    while at < bytes.len() {
        cancel.check()?;
        let end = (at + 65536).min(bytes.len());
        let n = reader
            .read_at(at as u64, &mut bytes[at..end])
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(n > 0 && n <= end - at, "truncated image asset");
        at += n;
    }
    if let Some(hash) = fingerprint {
        anyhow::ensure!(
            blake3::hash(&bytes).as_bytes() == hash,
            "image asset changed since native load"
        );
    }
    let image = decoders.decode(path.relative(), &bytes)?;
    anyhow::ensure!(
        [image.width, image.height] == size,
        "image dimensions changed during processing"
    );
    Ok(image)
}

pub(crate) fn resource_from_surface(
    owner: ClientId,
    mut image: ImageSurface,
    charge: Charge,
    check: &dyn Fn() -> anyhow::Result<()>,
) -> anyhow::Result<Arc<dyn ImageResource>> {
    image.validate()?;
    let mut opaque = true;
    let mut scalar_coverage = true;
    for block in image.rgba.chunks(256 * 1024) {
        check()?;
        opaque &= block.chunks_exact(4).all(|p| p[3] == 255);
        scalar_coverage &= block.chunks_exact(4).all(|p|p[0]==p[1]&&p[0]==p[2]&&p[0]==p[3]);
    }
    check()?;
    let pixels = Arc::from(std::mem::take(&mut image.rgba));
    Ok(Arc::new(Resource {
        owner,
        size: [image.width, image.height],
        opaque,
        scalar_coverage,
        pixels,
        _charge: charge,
    }))
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crysirror_core::{
        path::{Mount, VfsPath},
        service::{AssetReader, MemoryAssetReader},
    };
    struct Stream {
        bytes: Arc<[u8]>,
        reads: Arc<AtomicUsize>,
        declared: Option<u64>,
    }
    impl AssetReader for Stream {
        fn len(&self) -> Result<u64> {
            Ok(self.declared.unwrap_or(self.bytes.len() as u64))
        }
        fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<usize> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            MemoryAssetReader(self.bytes.clone()).read_at(offset, out)
        }
    }
    struct Storage(Arc<Stream>);
    impl ClientStorage for Storage {
        fn read(&self, _: &VfsPath) -> Result<Vec<u8>> {
            panic!("budgeted asset pipeline must not allocate via whole-file fallback")
        }
        fn open_read(&self, _: &VfsPath) -> Result<Arc<dyn AssetReader>> {
            Ok(self.0.clone())
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }
    fn wait(job: &mut ImageJob) -> ImageProcessState {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let state = job.poll().unwrap();
            if !matches!(state, ImageProcessState::Pending) {
                return state;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "image worker did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn start(
        memory: Arc<AtomicUsize>,
        stream: Arc<Stream>,
        fingerprint: Option<[u8; 32]>,
    ) -> ImageJob {
        begin(
            ClientId(7),
            ImageInput::Asset {
                path: "tiny.png".into(),
                size: [1, 1],
                fingerprint,
            },
            Arc::from(&b"[]"[..]),
            Arc::new(Storage(stream)),
            DecoderRegistry::default(),
            TaskPool::new("image-budget-test", 1, 8).unwrap(),
            memory,
        )
        .unwrap()
    }
    #[test]
    fn ordered_alpha_stamps_run_as_a_scoped_asynchronous_raster_job() {
        let memory = Arc::new(AtomicUsize::new(0));
        let stream = Arc::new(Stream {
            bytes: Arc::from([]),
            reads: Arc::new(AtomicUsize::new(0)),
            declared: None,
        });
        let input = ImageInput::Raster {
            destination: Box::new(ImageInput::Solid {
                size: [2, 1],
                color: [10, 20, 30, 255],
            }),
            source: Box::new(ImageInput::Solid {
                size: [1, 1],
                color: [4, 5, 6, 80],
            }),
        };
        let effects = serde_json::to_vec(&vec![ImageEffect::AlphaStamps {
            spec: crysirror_image::alpha_stamps::AlphaStamps {
                clear: Some(0),
                preserve_opaque: true,
                stamps: vec![crysirror_image::alpha_stamps::AlphaStamp {
                    source: [0, 0, 1, 1],
                    position: [0, 0],
                }],
            },
        }])
        .unwrap();
        let mut job = begin(
            ClientId(7),
            input,
            Arc::from(effects),
            Arc::new(Storage(stream)),
            DecoderRegistry::default(),
            TaskPool::new("alpha-stamp-test", 1, 8).unwrap(),
            memory.clone(),
        )
        .unwrap();
        let result = wait(&mut job);
        let ImageProcessState::Ready(image) = &result else {
            panic!("{result:?}");
        };
        assert_eq!(image.client(), ClientId(7));
        assert_eq!(image.size(), [2, 1]);
        assert_eq!(&*image.rgba(), &[10, 20, 30, 80, 10, 20, 30, 0]);
        drop(result);
        drop(job);
        assert_eq!(memory.load(Ordering::Acquire), 0);
    }
    #[test]
    fn asset_reserves_actual_encoded_bytes_without_flat_128_mib_penalty() {
        let memory = Arc::new(AtomicUsize::new(0));
        let occupied = Charge::new(memory.clone(), BUDGET - 1024 * 1024).unwrap();
        let png =
            crysirror_image::encode_png(&ImageSurface::filled(1, 1, [8, 16, 32, 255]).unwrap())
                .unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let stream = Arc::new(Stream {
            bytes: png.into(),
            reads: reads.clone(),
            declared: None,
        });
        let mut job = start(memory.clone(), stream, None);
        let result = wait(&mut job);
        let ImageProcessState::Ready(image) = &result else {
            panic!("{result:?}")
        };
        assert_eq!(&*image.rgba(), &[8, 16, 32, 255]);
        assert!(reads.load(Ordering::SeqCst) > 0);
        assert_eq!(memory.load(Ordering::Acquire), BUDGET - 1024 * 1024 + 4);
        drop(result);
        drop(job);
        assert_eq!(memory.load(Ordering::Acquire), BUDGET - 1024 * 1024);
        drop(occupied);
        assert_eq!(memory.load(Ordering::Acquire), 0);
    }
    #[test]
    fn encoded_budget_is_checked_before_allocating_or_reading_stream() {
        let memory = Arc::new(AtomicUsize::new(0));
        let occupied = Charge::new(memory.clone(), BUDGET - 8192).unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let stream = Arc::new(Stream {
            bytes: Arc::from([]),
            reads: reads.clone(),
            declared: Some(65536),
        });
        let mut job = start(memory.clone(), stream, None);
        let result = wait(&mut job);
        assert!(
            matches!(result,ImageProcessState::Failed(ref e) if e.contains("image processing memory budget")),
            "{result:?}"
        );
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(memory.load(Ordering::Acquire), BUDGET - 8192);
        drop(occupied);
        assert_eq!(memory.load(Ordering::Acquire), 0);
    }
    #[test]
    fn changed_stream_releases_encoded_and_scratch_reservations() {
        let memory = Arc::new(AtomicUsize::new(0));
        let png = crysirror_image::encode_png(&ImageSurface::filled(1, 1, [1, 2, 3, 255]).unwrap())
            .unwrap();
        let stream = Arc::new(Stream {
            bytes: png.into(),
            reads: Arc::new(AtomicUsize::new(0)),
            declared: None,
        });
        let mut job = start(memory.clone(), stream, Some([0; 32]));
        let result = wait(&mut job);
        assert!(
            matches!(result,ImageProcessState::Failed(ref e) if e.contains("asset changed")),
            "{result:?}"
        );
        assert_eq!(memory.load(Ordering::Acquire), 0);
    }
}
