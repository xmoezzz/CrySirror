//! Sprite display selectors over shared immutable prefix/composite operations.
use super::*;
use crysirror_core::image_processing::{
    ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState, ImageResource,
};
use crysirror_image::effects::ImageEffect;
#[derive(Default)]
pub(super) struct SpriteComposition {
    requested: Value,
    committed: Option<Value>,
    asset: Option<(String, [u32; 2])>,
    work: Option<CompositionWork>,
}
struct CompositionWork {
    signature: Value,
    size: [u32; 2],
    position: [i32; 2],
    format: i32,
    mode: i32,
    parameter: i32,
    source_rect: Option<[u32; 4]>,
    images: [Option<Arc<dyn ImageResource>>; 2],
    captures: [Option<ImageJob>; 2],
    process: Option<ImageJob>,
}
impl BgiVm {
    pub(super) fn sprite_prefix_ready(&self, object: i32) -> bool {
        let Some(cut) = self.graph.tree.chain_position(object) else {
            return false;
        };
        self.graph
            .tree
            .chain_handles()
            .iter()
            .take(cut as usize)
            .all(|id| {
                if !matches!(self.graph.tree.kind(*id), Some(Kind::Sprite | Kind::Map)) {
                    return true;
                }
                let Some(p) = self.graph.objects.get(id) else {
                    return true;
                };
                let shown = self.graph.global_enabled != 0
                    && p.native.draw_enabled != 0
                    && p.native.suppress_draw == 0
                    && p.mask_alpha < 256
                    && p.alpha_multiplier != 0
                    && self.graph.tree.chain_drawable(*id)
                    && !self.graph.images.expired.contains(id);
                let drawable = self.graph.images.painted.contains_key(id)
                    || self
                        .sprites
                        .get(&(*id as u32))
                        .is_some_and(|s| s.bitmap.is_some());
                if p.blend_mode == 128 || !shown || !drawable || p.format_resource.is_none() {
                    return true;
                }
                self.graph.compositions.get(id).is_some_and(|state| {
                    state.work.is_none() && state.committed.as_ref() == Some(&state.requested)
                })
            })
    }
    pub(super) fn remove_sprite_composition(
        &mut self,
        host: &mut dyn HostServices,
        object: i32,
    ) -> Result<()> {
        if let Some(state) = self.graph.compositions.remove(&object) {
            if let Some((path, _)) = state.asset {
                host.release_image_resource(self.client, &path)?;
            }
        }
        Ok(())
    }
    pub(super) fn sprite_composition_paint(
        &mut self,
        host: &mut dyn HostServices,
        object: i32,
        position: [i32; 2],
        visible: bool,
    ) -> Result<(Value, [u32; 2])> {
        let p = self
            .graph
            .objects
            .get(&object)
            .context("BGI composited sprite properties")?;
        let bitmap = p.format_resource.context("BGI composited sprite source")? as u32;
        let b = self.bitmap(bitmap as i32)?.1.clone();
        let native_mode = p
            .named_properties
            .get("target-object-mode")
            .copied()
            .unwrap_or(0);
        let format = if matches!(native_mode, 2 | 3 | 5 | 6) {
            2
        } else if b.format == 0 {
            1
        } else {
            b.format
        };
        let mode = p.blend_mode;
        target_checks::effect_mode(mode)?;
        let parameter = if native_mode == 4 {
            256 - (p
                .alpha_multiplier
                .wrapping_mul(256i32.wrapping_sub(p.mask_alpha))
                >> 8)
                .clamp(0, 256)
        } else {
            p.transparency_parameter()
        };
        let source_rect = self.graph.crops.get(&object).copied();
        let order = [
            p.native.priority as i32,
            self.graph
                .tree
                .chain_position(object)
                .context("BGI composited sprite order")? as i32,
        ];
        let source_version = (
            b.generation,
            self.graph
                .bitmap_versions
                .get(&bitmap)
                .copied()
                .unwrap_or(0),
            self.graph
                .images
                .revisions
                .get(&object)
                .copied()
                .unwrap_or(0),
        );
        let signature = json!({"source":source_version,"screen":self.graph.screen,"order":order,"position":position,"mode":mode,"parameter":parameter,"format":format,"visible":visible,"source_rect":source_rect,"prefix":self.graph.visual_prefix(object)});
        let ready = !self.graph.images.dirty.contains(&object) && self.graph_prefix_ready(object);
        let mut state = self.graph.compositions.remove(&object).unwrap_or_default();
        state.requested = signature.clone();
        if state.work.is_none() && state.committed.as_ref() != Some(&signature) && ready {
            if visible {
                let (source, capture) =
                    if let Some(painted) = self.graph.images.painted.get(&object) {
                        (Some(painted.image.clone()), None)
                    } else if let Some(image) = &b.image {
                        (Some(image.clone()), None)
                    } else {
                        (None, Some(host.begin_canvas_image(self.client, bitmap)?))
                    };
                state.work = Some(CompositionWork {
                    signature,
                    size: self.graph.screen,
                    position,
                    format,
                    mode,
                    parameter,
                    source_rect,
                    images: [None, source],
                    captures: [
                        Some(host.begin_layer_prefix_image_over(
                            self.client,
                            self.graph.screen,
                            order,
                            [0, 0, 0, 255],
                        )?),
                        capture,
                    ],
                    process: None,
                });
            } else {
                state.committed = Some(signature);
            }
        }
        let mut complete = false;
        if let Some(work) = &mut state.work {
            for index in 0..2 {
                if let Some(job) = &mut work.captures[index] {
                    match job.poll()? {
                        ImageProcessState::Pending => (),
                        ImageProcessState::Failed(error) => bail!("BGI sprite capture: {error}"),
                        ImageProcessState::Ready(image) => {
                            work.images[index] = Some(image);
                            work.captures[index] = None;
                        }
                    }
                }
            }
            if work.images.iter().all(Option::is_some) && work.process.is_none() {
                let source = work.images[1].as_ref().unwrap().clone();
                let size = source.size();
                let input = ImageInput::Resource(source);
                let position = work.position;
                let format = work.format;

                let composite = ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(
                        work.images[0].as_ref().unwrap().clone(),
                    )),
                    layers: vec![ImageCompositeLayer {
                        input: Box::new(input),
                        source: work
                            .source_rect
                            .map_or([0, 0, size[0] as i32, size[1] as i32], |r| {
                                r.map(|v| v as i32)
                            }),
                        destination: position,
                        clip: None,
                        opacity: 255,
                        math: ImageCompositeMath::IntegerStored {
                            destination_format: 1,
                            source_format: format as u8,
                            mode: work.mode,
                            parameter: work.parameter,
                        },
                    }],
                };
                work.process = Some(host.image_metadata(self.client)?.begin_process(
                    composite,
                    serde_json::to_vec(&vec![ImageEffect::OverrideAlpha { alpha: 255 }])?.into(),
                )?);
            }
            if let Some(job) = &mut work.process {
                match job.poll()? {
                    ImageProcessState::Pending => (),
                    ImageProcessState::Failed(error) => {
                        bail!("BGI sprite display composition: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        let image = host.publish_image_resource(
                            self.client,
                            0xbc000000 ^ object as u32,
                            image,
                        )?;
                        if let Some((old, _)) = state
                            .asset
                            .replace((image.path.relative().to_owned(), work.size))
                        {
                            host.release_image_resource(self.client, &old)?;
                        }
                        state.committed = Some(work.signature.clone());
                        complete = true;
                    }
                }
            }
        }
        if complete {
            state.work = None;
            self.graph.mark_visual(object);
        }
        let paint = state.asset.as_ref().map_or_else(
            || (json!({"kind":"solid","color":[0,0,0,0]}), self.graph.screen),
            |(path, size)| (json!({"kind":"image","asset":path}), *size),
        );
        self.graph.compositions.insert(object, state);
        Ok(paint)
    }
}

impl BgiVm {
    pub(super) fn rebuild_native_sprite_affine(&mut self, object: i32) -> Result<()> {
        let Some(&[px, py, angle, sx, sy, filter]) = self.graph.sprite_affine.get(&object) else {
            return Ok(());
        };
        let primary = self
            .graph
            .objects
            .get(&object)
            .and_then(|p| p.format_resource)
            .context("BGI affine sprite source")?;
        let b = self.bitmap(primary)?.1;
        let format = if b.format == 0 { 1 } else { b.format };
        ensure!(matches!(format, 1 | 2), "BGI affine sprite colour format");
        let [left, top, width, height] =
            crysirror_image::integer_raster::display::affine_sample_bounds(
                b.size,
                [px, py],
                angle,
                [sx as u32, sy as u32],
                filter != 0,
            )?;
        let size = [u32::try_from(width)?, u32::try_from(height)?];
        crysirror_image::rgba_len(size[0], size[1])?;
        let raster = Raster::Rotate {
            positions: [
                px.wrapping_sub(left).wrapping_shl(16),
                py.wrapping_sub(top).wrapping_shl(16),
                px.wrapping_shl(16),
                py.wrapping_shl(16),
            ],
            angle,
            factors: [sx as u32, sy as u32],
            alpha: 0,
            filtered: filter != 0,
            copy: true,
        };
        self.graph.raster_offsets.insert(object, [left, top]);
        self.set_graph_plan(
            object,
            GraphPlan {
                inputs: vec![GraphInput::Solid(size, [0; 4]), GraphInput::Bitmap(primary)],
                operation: Some(raster),
                quad: None,
                source_format: Some(format),
                compound: None,
                projection: None,
                warp: None,
                format: 2,
                force_opaque: false,
                filters: Vec::new(),
            },
        );
        Ok(())
    }
}
