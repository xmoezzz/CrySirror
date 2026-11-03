//! Native derived-image policy over engine-owned immutable image jobs.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource};
use crysirror_image::{
    effects::ImageEffect,
    integer_raster::{IntegerRaster, IntegerRasterOperation as Raster, display::DisplayRaster},
};
#[derive(Clone)]
pub(super) enum GraphInput {
    Bitmap(i32),
    Frozen(Arc<dyn ImageResource>),
    Solid([u32; 2], [u8; 4]),
    Prefix([u32; 2], [i32; 2]),
}
#[derive(Clone)]
pub(super) struct GraphPlan {
    pub inputs: Vec<GraphInput>,
    pub operation: Option<Raster>,
    pub quad: Option<[u32; 4]>,
    pub source_format: Option<i32>,
    pub compound: Option<super::graph_backf::BackFPlan>,
    pub projection: Option<super::graph_projection::ProjectionPlan>,
    pub warp: Option<([u32; 2], crysirror_core::image_processing::FieldWarpSpec)>,
    pub format: i32,
    pub force_opaque: bool,
    pub filters: Vec<(u8, i32, u16)>,
}
impl GraphPlan {
    pub fn bitmap(id: i32, format: i32) -> Self {
        Self {
            inputs: vec![GraphInput::Bitmap(id)],
            operation: None,
            quad: None,
            source_format: None,
            warp: None,
            compound: None,
            projection: None,
            format,
            force_opaque: format == 1,
            filters: Vec::new(),
        }
    }
}
enum Prepared {
    Ready(Arc<dyn ImageResource>),
    Job(ImageJob),
    Solid([u32; 2], [u8; 4]),
}
struct Work {
    inputs: Vec<Prepared>,
    operation: Option<Raster>,
    quad: Option<[u32; 4]>,
    source_format: Option<i32>,
    compound: Option<super::graph_backf::BackFPlan>,
    projection: Option<super::graph_projection::ProjectionPlan>,
    warp: Option<([u32; 2], crysirror_core::image_processing::FieldWarpSpec)>,
    filters: Vec<(u8, i32, u16)>,
    phase: Option<ImageJob>,
    mask: Option<(Prepared, DisplayRaster)>,
    force_opaque: bool,
    format: i32,
    stage: u8,
}
pub(super) struct ImagePaint {
    pub path: String,
    pub image: Arc<dyn ImageResource>,
}
#[derive(Default)]
pub(super) struct GraphImages {
    pub plans: BTreeMap<i32, GraphPlan>,
    pub painted: BTreeMap<i32, ImagePaint>,
    pub dirty: std::collections::BTreeSet<i32>,
    jobs: BTreeMap<i32, Work>,
    pub masks: BTreeMap<i32, Arc<dyn ImageResource>>,
    pub mode3_generation: BTreeMap<i32, (u32, u64)>,
    pub expired: std::collections::BTreeSet<i32>,
    pub retired: Vec<String>,
    pub serial: u64,
    pub revisions: BTreeMap<i32, u64>,
}
impl GraphImages {
    pub fn remove(&mut self, object: i32) {
        self.plans.remove(&object);
        if let Some(old) = self.painted.remove(&object) {
            self.retired.push(old.path);
        }
        self.jobs.remove(&object);
        self.masks.remove(&object);
        self.mode3_generation.remove(&object);
        self.expired.remove(&object);
        self.dirty.remove(&object);
    }
    pub fn plan(&mut self, object: i32, plan: GraphPlan) {
        self.plans.insert(object, plan);
        self.jobs.remove(&object);
        self.dirty.insert(object);
        self.serial = self.serial.wrapping_add(1);
        self.revisions.insert(object, self.serial);
    }
    pub fn dirty(&mut self, object: i32) {
        if self.plans.contains_key(&object) {
            self.jobs.remove(&object);
            self.dirty.insert(object);
            self.serial = self.serial.wrapping_add(1);
            self.revisions.insert(object, self.serial);
        }
    }
}
fn image_input(input: &Prepared) -> Result<ImageInput> {
    Ok(match input {
        Prepared::Ready(image) => ImageInput::Resource(image.clone()),
        Prepared::Solid(size, color) => ImageInput::Solid {
            size: *size,
            color: *color,
        },
        Prepared::Job(_) => bail!("BGI pending image input"),
    })
}
fn job_image(job: &mut ImageJob) -> Result<Option<Arc<dyn ImageResource>>> {
    Ok(match job.poll()? {
        ImageProcessState::Pending => None,
        ImageProcessState::Failed(error) => bail!("BGI graph image: {error}"),
        ImageProcessState::Ready(image) => Some(image),
    })
}
impl BgiVm {
    pub(crate) fn graph_bitmap_dependencies(
        &self,
        call: &NativeCall,
        vm: &Machine,
    ) -> Result<Vec<u32>> {
        if !super::handles(call.group, call.id) {
            return Ok(Vec::new());
        }
        let a = &call.args;
        let object = if call.group == 0x90 && matches!(call.id, 0x40..=0x4d) {
            0
        } else if call.group == 0x91 && matches!(call.id, 0x38 | 0x3d) {
            a.get(1).copied().unwrap_or(0)
        } else {
            a.first().copied().unwrap_or(0)
        };
        let mut ids = Vec::new();
        let indices: &[usize] = match (call.group, call.id) {
            (0x90, 0x58 | 0x5a) => &[3, 4],
            (0x90, 0x40 | 0x46 | 0x48 | 0x49) => &[0],
            (0x90, 0x41 | 0x4a) => &[0, 1],
            (0x90, 0x42) => &[0, 1, 2, 3],
            (0x90, 0x75) => &[3],
            (0x90, 0x66) => &[3],
            (0x90, 0x45) => &[0, 1, 2],
            (0x90, 0x43) => &[2, 5, 6],
            (0x90, 0x47) => &[0, 1],
            (0x90, 0x5b) => &[3, 4],
            (0x90, 0x5c | 0x5d) => &[4, 5],
            (0x90, 0x55) => &[1],
            (0x91, 0x65) => &[1, 2],
            (0x91, 0x67) => &[1],
            (0x91, 0x10..=0x17) | (0x92, 0x10 | 0x11) => &[0],
            _ => &[],
        };
        for &index in indices {
            if let Some(&id) = a.get(index) {
                if (id as u32) < 0x4000 {
                    ids.push(id as u32);
                }
            }
        }
        for object in self.graph.descendants(object) {
            if let Some(recipe) = self.sprite_mask_recipe(object)? {
                ids.push(recipe.bitmap as u32);
            }

            if let Some(effector) = self
                .graph
                .filters
                .get(&object)
                .and_then(|f| f.effector.as_ref())
            {
                ids.extend(effector.bitmap_dependencies());
            }

            if let Some(plan) = self.graph.images.plans.get(&object) {
                for input in &plan.inputs {
                    if let GraphInput::Bitmap(id) = input {
                        ids.push(*id as u32);
                    }
                }
            }
            if let Some(bitmap) = self
                .graph
                .objects
                .get(&object)
                .and_then(|p| p.format_resource)
            {
                if (bitmap as u32) < 0x4000 {
                    ids.push(bitmap as u32);
                }
            }
        }
        if call.group == 0x90 && call.id == 0x44 {
            let count = usize::try_from(a[0])?;
            ensure!((2..=32).contains(&count), "BGI frame table count");
            let bytes = vm.memory(a[1] as u32, count * 4)?;
            for bytes in bytes.chunks_exact(4) {
                let id = u32::from_le_bytes(bytes.try_into().unwrap());
                if id < 0x4000 {
                    ids.push(id);
                }
            }
        }
        if let Some(frames) = self.graph.frames.get(&object) {
            ids.extend(frames.iter().map(|&(id, _)| id as u32));
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }
    pub(crate) fn invalidate_graph_bitmap(&mut self, bitmap: u32) {
        self.graph.bitmap_clock = self.graph.bitmap_clock.wrapping_add(1);
        self.graph
            .bitmap_versions
            .insert(bitmap, self.graph.bitmap_clock);
        let raw = self
            .sprites
            .iter()
            .filter_map(|(&id, s)| (s.bitmap == Some(bitmap)).then_some(id as i32))
            .collect::<Vec<_>>();
        for id in raw {
            if self.graph.sprite_affine.contains_key(&id) {
                self.graph.affine_rebuild.insert(id);
            }
            self.graph.mark_visual(id);
        }
        let masks = self
            .graph
            .filters
            .iter()
            .filter_map(|(&id, f)| {
                (f.mask.is_some_and(|(b, _)| b as u32 == bitmap)
                    || f.effector.as_ref().is_some_and(|e| e.uses_bitmap(bitmap)))
                .then_some(id)
            })
            .collect::<Vec<_>>();
        for id in masks {
            self.graph.filters.get_mut(&id).unwrap().signature = None;
        }

        let affected = self
            .graph
            .images
            .plans
            .iter()
            .filter_map(|(&object, plan)| {
                plan.inputs
                    .iter()
                    .any(|input| matches!(input,GraphInput::Bitmap(id)if *id as u32==bitmap))
                    .then_some(object)
            })
            .collect::<Vec<_>>();
        let mut affected = affected;
        affected.extend(self.graph.frames.iter().filter_map(|(&object, frames)| {
            frames
                .iter()
                .any(|&(id, _)| id as u32 == bitmap)
                .then_some(object)
        }));
        affected.sort_unstable();
        affected.dedup();
        for object in affected {
            if self
                .graph
                .background_args
                .get(&object)
                .is_some_and(|(id, _)| matches!(id, 0x45 | 0x47))
            {
                self.graph.field_rebuild.insert(object);
            }
            if self.graph.sprite_wave_args.contains_key(&object) {
                self.graph.field_rebuild.insert(object);
            }

            // CDspObjMap retains a copied tile cache. Atlas pixel changes do
            // not repaint it until the native invalidation/viewport operation.
            if !self.graph.maps.contains_key(&object) {
                self.graph.images.dirty(object);
            }
        }
    }
    pub(crate) fn release_graph_retired(&mut self, host: &mut dyn HostServices) -> Result<()> {
        for path in std::mem::take(&mut self.graph.images.retired) {
            host.release_image_resource(self.client, &path)?;
        }
        Ok(())
    }
    pub(crate) fn graph_tick(&mut self, host: &mut dyn HostServices) -> Result<()> {
        self.refresh_native_sprite_masks()?;
        self.release_graph_retired(host)?;
        for object in std::mem::take(&mut self.graph.affine_rebuild) {
            self.rebuild_native_sprite_affine(object)?;
        }
        for object in std::mem::take(&mut self.graph.field_rebuild) {
            if self.graph.sprite_wave_args.contains_key(&object) {
                self.rebuild_native_sprite_wave(object)?;
            } else {
                self.rebuild_backdrop_plan(object)?;
            }
        }
        let objects = self.graph.images.dirty.iter().copied().collect::<Vec<_>>();
        let mut changed = false;
        for object in objects {
            changed |= self.refresh_graph_image(host, object)?;
        }
        if changed || self.graph.needs_publish || !self.graph.compositions.is_empty() {
            self.publish_graph(host)?;
        }
        self.refresh_native_filters(host)?;
        Ok(())
    }
    pub(super) fn set_graph_plan(&mut self, object: i32, mut plan: GraphPlan) {
        plan.filters = self
            .graph
            .objects
            .get(&object)
            .map(|p| {
                (0..16)
                    .filter_map(|index| p.properties.get(&(0x1100 | index)).copied())
                    .filter_map(|(packed, color)| {
                        let kind = ((packed as u32 >> 8) & 255) as u8;
                        (kind != 0).then_some((kind, color, (packed as u32 >> 16) as u16))
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.graph.images.plan(object, plan);
    }
    fn expire_graph_image(&mut self, host: &mut dyn HostServices, object: i32) -> Result<bool> {
        self.hide_graph_object(host, object)?;
        let painted = self.graph.images.painted.remove(&object);
        if let Some(painted) = painted {
            self.graph.images.retired.push(painted.path);
        }
        self.graph.images.jobs.remove(&object);
        self.graph.images.dirty.remove(&object);
        self.graph.images.expired.insert(object);
        self.release_graph_retired(host)?;
        Ok(true)
    }
    pub(super) fn refresh_graph_image(
        &mut self,
        host: &mut dyn HostServices,
        object: i32,
    ) -> Result<bool> {
        if let Some(&(bitmap, generation)) = self.graph.images.mode3_generation.get(&object) {
            if self
                .bitmaps
                .get(&bitmap)
                .is_none_or(|b| b.generation != generation)
            {
                return self.expire_graph_image(host, object);
            }
        }
        if self.graph.frames.get(&object).is_some_and(|frames| {
            frames.iter().any(|&(id, generation)| {
                self.bitmaps
                    .get(&(id as u32))
                    .is_none_or(|bitmap| bitmap.generation != generation)
            })
        }) {
            return self.expire_graph_image(host, object);
        }
        if self.graph.mask_unready.contains(&object) {
            self.graph.images.jobs.remove(&object);
            self.graph.images.dirty.remove(&object);
            return Ok(true);
        }
        if self.graph.filters.contains_key(&object) && !self.graph_prefix_ready(object) {
            return Ok(false);
        }
        if !self.graph.images.dirty.contains(&object) {
            return Ok(false);
        }
        let mut work = if let Some(work) = self.graph.images.jobs.remove(&object) {
            work
        } else {
            let Some(plan) = self.graph.images.plans.get(&object).cloned() else {
                self.graph.images.dirty.remove(&object);
                return Ok(false);
            };
            let mut inputs = Vec::new();
            for input in plan.inputs {
                inputs.push(match input {
                    GraphInput::Frozen(image) => Prepared::Ready(image),
                    GraphInput::Solid(size, color) => Prepared::Solid(size, color),
                    GraphInput::Prefix(size, cut) => {
                        self.publish_graph(host)?;
                        if !self.graph_prefix_ready(object) {
                            return Ok(false);
                        }
                        Prepared::Job(host.begin_layer_prefix_image_over(
                            self.client,
                            size,
                            cut,
                            [0, 0, 0, 255],
                        )?)
                    }
                    GraphInput::Bitmap(id) => {
                        let (_, bitmap) = self.bitmap(id)?;
                        if let Some(image) = &bitmap.image {
                            Prepared::Ready(image.clone())
                        } else {
                            Prepared::Job(host.begin_canvas_image(self.client, id as u32)?)
                        }
                    }
                });
            }
            let mask = if let Some(recipe) = self.sprite_mask_recipe(object)? {
                let bitmap = self.bitmap(recipe.bitmap)?.1;
                let input = if let Some(image) = &bitmap.image {
                    Prepared::Ready(image.clone())
                } else {
                    Prepared::Job(host.begin_canvas_image(self.client, recipe.bitmap as u32)?)
                };
                Some((input, recipe.raster))
            } else {
                None
            };
            let force_opaque = plan.force_opaque && mask.is_none();
            Work {
                inputs,
                mask,
                operation: plan.operation,
                quad: plan.quad,
                source_format: plan.source_format,
                compound: plan.compound,
                projection: plan.projection,
                warp: plan.warp,
                filters: plan.filters,
                phase: None,
                force_opaque,
                format: plan.format,
                stage: 0,
            }
        };
        if let Some((Prepared::Job(job), _)) = &mut work.mask {
            let Some(image) = job_image(job)? else {
                self.graph.images.jobs.insert(object, work);
                return Ok(false);
            };
            work.mask.as_mut().unwrap().0 = Prepared::Ready(image);
        }
        loop {
            let current = if let Some(job) = &mut work.phase {
                let Some(image) = job_image(job)? else {
                    self.graph.images.jobs.insert(object, work);
                    return Ok(false);
                };
                work.phase = None;
                image
            } else {
                for input in &mut work.inputs {
                    if let Prepared::Job(job) = input {
                        let Some(image) = job_image(job)? else {
                            self.graph.images.jobs.insert(object, work);
                            return Ok(false);
                        };
                        *input = Prepared::Ready(image);
                    }
                }
                if let Some(plan) = work.projection.as_ref().cloned() {
                    let sources = work
                        .inputs
                        .iter()
                        .map(|input| match input {
                            Prepared::Ready(image) => Ok(image.clone()),
                            _ => bail!("BGI projected source lease"),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let Some(input) = self.projection_image(object, &plan, sources)? else {
                        self.graph.images.jobs.insert(object, work);
                        return Ok(false);
                    };
                    work.projection = None;
                    work.phase = Some(
                        host.image_metadata(self.client)?
                            .begin_process(input, Arc::from(b"[]".as_slice()))?,
                    );
                    work.stage = 1;
                    continue;
                }
                if let Some(compound) = work.compound.take() {
                    let input = compound.input(
                        work.inputs
                            .iter()
                            .map(image_input)
                            .collect::<Result<Vec<_>>>()?,
                    )?;
                    work.phase = Some(
                        host.image_metadata(self.client)?
                            .begin_process(input, Arc::from(b"[]".as_slice()))?,
                    );
                    work.stage = 1;
                    continue;
                }
                if let Some((size, spec)) = work.warp.take() {
                    ensure!(work.inputs.len() == 1, "BGI field warp source count");
                    work.phase = Some(host.image_metadata(self.client)?.begin_process(
                        ImageInput::FieldWarp {
                            source: Box::new(image_input(&work.inputs[0])?),
                            size,
                            spec,
                        },
                        Arc::from(b"[]".as_slice()),
                    )?);
                    work.stage = 1;
                    continue;
                }
                if let Some([x, y, w, h]) = work.quad.take() {
                    ensure!(work.inputs.len() == 4, "BGI four-cell backdrop image count");
                    let rects = [
                        ([x, y, w - x, h - y], [0, 0]),
                        ([0, y, x, h - y], [w - x, 0]),
                        ([x, 0, w - x, y], [0, h - y]),
                        ([0, 0, x, y], [w - x, h - y]),
                    ];
                    let mut layers = Vec::new();
                    for (index, (rect, destination)) in rects.into_iter().enumerate() {
                        layers.push(crysirror_core::image_processing::ImageCompositeLayer {
                            input: Box::new(image_input(&work.inputs[index])?),
                            source: rect.map(|n| n as i32),
                            destination: destination.map(|n| n as i32),
                            clip: None,
                            opacity: 255,
                            math: crysirror_core::image_processing::ImageCompositeMath::Copy,
                        });
                    }
                    work.phase = Some(host.image_metadata(self.client)?.begin_process(
                        ImageInput::Composite {
                            base: Box::new(ImageInput::Solid {
                                size: [w, h],
                                color: [0; 4],
                            }),
                            layers,
                        },
                        Arc::from(b"[]".as_slice()),
                    )?);
                    work.stage = 1;
                    continue;
                }
                if matches!(
                    &work.operation,
                    Some(Raster::Display {
                        spec: DisplayRaster::ThresholdMask { .. }
                    })
                ) {
                    if let Some(Prepared::Ready(mask)) = work.inputs.get(1) {
                        self.graph.images.masks.insert(object, mask.clone());
                        if let Some(plan) = self.graph.images.plans.get_mut(&object) {
                            plan.inputs[1] = GraphInput::Frozen(mask.clone());
                        }
                    }
                }
                if let Some(operation) = work.operation.take() {
                    ensure!(
                        (1..=2).contains(&work.inputs.len()),
                        "BGI graph raster input count"
                    );
                    let spec = IntegerRaster {
                        destination_format: work.format as u32,
                        source_format: work.source_format.unwrap_or(work.format) as u32,
                        raster: operation,
                    };
                    let effects =
                        Arc::from(serde_json::to_vec(&vec![ImageEffect::IntegerRaster {
                            spec,
                        }])?);
                    work.phase = Some(host.image_metadata(self.client)?.begin_process(
                        ImageInput::Raster {
                            destination: Box::new(image_input(&work.inputs[0])?),
                            source: Box::new(image_input(&work.inputs[work.inputs.len() - 1])?),
                        },
                        effects,
                    )?);
                    work.stage = 1;
                    continue;
                }
                match &work.inputs[0] {
                    Prepared::Ready(image) => image.clone(),
                    Prepared::Solid(size, color) => {
                        work.phase = Some(host.image_metadata(self.client)?.begin_process(
                            ImageInput::Solid {
                                size: *size,
                                color: *color,
                            },
                            Arc::from(b"[]".as_slice()),
                        )?);
                        work.stage = 1;
                        continue;
                    }
                    _ => unreachable!(),
                }
            };
            if let Some((mask, raster)) = work.mask.take() {
                let spec = IntegerRaster {
                    destination_format: 2,
                    source_format: 2,
                    raster: Raster::Display { spec: raster },
                };
                work.phase = Some(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Raster {
                        destination: Box::new(ImageInput::Resource(current)),
                        source: Box::new(image_input(&mask)?),
                    },
                    serde_json::to_vec(&vec![ImageEffect::IntegerRaster { spec }])?.into(),
                )?);
                work.stage = 1;
                continue;
            }
            if work.stage <= 1 && !work.filters.is_empty() {
                let spec = IntegerRaster {
                    destination_format: 2,
                    source_format: 2,
                    raster: Raster::Display {
                        spec: DisplayRaster::ColorFilters {
                            filters: std::mem::take(&mut work.filters),
                        },
                    },
                };
                let effects = Arc::from(serde_json::to_vec(&vec![ImageEffect::IntegerRaster {
                    spec,
                }])?);
                work.phase = Some(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Raster {
                        destination: Box::new(ImageInput::Resource(current.clone())),
                        source: Box::new(ImageInput::Resource(current)),
                    },
                    effects,
                )?);
                work.stage = 2;
                continue;
            }
            if work.force_opaque && !current.opaque() {
                work.phase = Some(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Resource(current),
                    Arc::from(serde_json::to_vec(&vec![ImageEffect::OverrideAlpha {
                        alpha: 255,
                    }])?),
                )?);
                work.force_opaque = false;
                work.stage = 3;
                continue;
            }
            if let Some(effector) = self
                .graph
                .filters
                .get_mut(&object)
                .and_then(|filter| filter.effector.as_mut())
            {
                if effector.mode == 4 {
                    effector.feedback = Some(current.clone());
                }
            }
            let uploaded = host.publish_image_resource(
                self.client,
                0xd0000000 ^ (object as u32),
                current.clone(),
            )?;
            if let Some(old) = self.graph.images.painted.insert(
                object,
                ImagePaint {
                    path: uploaded.path.relative().to_string(),
                    image: current,
                },
            ) {
                self.graph.images.retired.push(old.path);
            }
            self.release_graph_retired(host)?;
            self.graph.images.dirty.remove(&object);
            self.graph.mark_visual(object);
            return Ok(true);
        }
    }
    pub(crate) fn graph_alpha(&mut self, object: i32, value: i32) -> Result<()> {
        let objects = self.graph.descendants(object);
        let mut backdrops = Vec::new();
        let mut sprites = Vec::new();
        let mut effectors = Vec::new();
        for object in objects {
            let p = self.graph.objects.entry(object).or_default();
            if self
                .graph
                .filters
                .get(&object)
                .is_some_and(|f| f.effector.is_some())
            {
                p.set_alpha_parameter(value);
                effectors.push(object);
                continue;
            }
            if let Some(background) = p.background {
                let changed = p.alpha_parameter() != value;
                p.set_alpha_parameter(value);
                if changed && !matches!(background.class, BackClass::BackN | BackClass::BackS) {
                    backdrops.push(object);
                }
                continue;
            }
            let mode = p
                .named_properties
                .get("target-object-mode")
                .copied()
                .unwrap_or(0);
            if mode == 4 {
                p.set_alpha_parameter(value);
                sprites.push(object);
                continue;
            }
            if mode == 1 && p.background.is_none() {
                let selector = p
                    .named_properties
                    .get("mode1-selector")
                    .copied()
                    .unwrap_or(0);
                if matches!(selector, -1 | 0 | 2 | 3) {
                    p.set_alpha_parameter(value);
                }
                if matches!(selector, 1 | 2) {
                    p.named_properties.insert("mode1-amount".into(), value);
                    if let Some(plan) = self.graph.images.plans.get_mut(&object) {
                        if let Some(Raster::Display {
                            spec: DisplayRaster::Crossfade { amount },
                        }) = &mut plan.operation
                        {
                            *amount = value.clamp(0, 256);
                        }
                    }
                    self.graph.images.dirty(object);
                }
            } else {
                // Ordinary alpha is a retained Layer gate. In particular a
                // mode-3 mask depends on the fixed parameter, not base alpha.
                p.set_alpha_parameter(value);
            }
        }
        for object in backdrops {
            self.rebuild_backdrop_plan(object)?;
        }
        for object in sprites {
            self.rebuild_native_sprite_wave(object)?;
        }
        for object in effectors {
            self.sync_native_effector(object)?;
        }
        Ok(())
    }
}

impl BgiVm {
    /// CProc control writes raw 8.24 progress, whereas Graph90:35 first
    /// shifts its wrapper operand. Both reach the same native virtual state.
    pub(crate) fn set_control_parameters(
        &mut self,
        _host: &mut dyn HostServices,
        object: i32,
        alpha: Option<i32>,
        fixed_raw: Option<i32>,
    ) -> Result<()> {
        self.graph.require(object)?;
        if let Some(alpha) = alpha {
            self.graph_alpha(object, alpha)?;
        }
        let objects = self.graph.descendants(object);
        for id in objects {
            if let Some(raw) = fixed_raw {
                let p = self.graph.objects.entry(id).or_default();
                p.native.fixed_parameter_16_16 = raw;
                p.properties.insert(0x35, (raw, 0));
                if let Some(plan) = self.graph.images.plans.get_mut(&id) {
                    if let Some(Raster::Display {
                        spec: DisplayRaster::ThresholdMask { threshold, .. },
                    }) = &mut plan.operation
                    {
                        *threshold = (raw as u32) >> 16;
                        self.graph.images.dirty(id);
                    }
                }
                if self
                    .graph
                    .objects
                    .get(&id)
                    .and_then(|p| p.background)
                    .is_some_and(|b| b.class == BackClass::BackF)
                {
                    self.rebuild_native_backf(id)?;
                }
                self.sync_native_effector(id)?;
                self.sync_native_projection_fixed(id, raw)?;
            }
            self.graph.mark_visual(id);
        }
        Ok(())
    }
}
