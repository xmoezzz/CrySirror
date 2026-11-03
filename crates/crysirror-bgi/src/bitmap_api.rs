//! Native bitmap handles and operation plans; engine jobs own all pixel work.
use super::*;
use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource};
use crysirror_image::{
    effects::ImageEffect,
    integer_raster::{
        IntegerRaster, IntegerRasterOperation as Raster, perspective_blit::PerspectiveBlit,
    },
};

pub(crate) fn handles(group: u8, id: u16) -> bool {
    matches!(
        (group, id),
        (
            0x90,
            0x04 | 0x10 | 0x12 | 0x17 | 0x18..=0x1f | 0xc2 | 0xc3 | 0xc8 | 0xca | 0xcc | 0xcd
        ) | (0x91, 0x18..=0x1f)
            | (0x92, 0x1a)
    )
}
enum Prepared {
    Ready(Arc<dyn ImageResource>),
    Job(ImageJob),
}
enum AuxiliaryOperation {
    AlphaPlanes {
        size: [u32; 2],
        spec: crysirror_image::alpha_planes::AlphaPlanePair,
    },
    Mask(crysirror_image::stored_mask::StoredMaskBlend),
    Warp {
        size: [u32; 2],
        spec: crysirror_core::image_processing::FieldWarpSpec,
    },
}
enum Phase {
    Auxiliary {
        inputs: Vec<Prepared>,
        operation: AuxiliaryOperation,
    },
    Read(Task<(String, Vec<u8>)>),
    Decode(ImageJob),
    Capture {
        inputs: Vec<Prepared>,
        base: Option<ImageInput>,
        operation: Option<IntegerRaster>,
    },
    Process(ImageJob),
    Opaque {
        raw: Arc<dyn ImageResource>,
        job: ImageJob,
    },
}
pub(crate) struct BitmapWork {
    reallocate: bool,
    outputs: Vec<i32>,
    destination: u32,
    format: i32,
    auxiliary: [i32; 2],
    phase: Phase,
}
impl BgiVm {
    pub(super) fn bitmap_info(&self, bitmap: i32) -> Option<crysirror_bgi_vm::BitmapInfo> {
        self.bitmaps
            .get(&(bitmap as u32))
            .map(|bitmap| crysirror_bgi_vm::BitmapInfo {
                row_stride: bitmap.size[0]
                    * match bitmap.format {
                        0 => 2,
                        3 => 1,
                        6 => 6,
                        _ => 4,
                    },
                width: bitmap.size[0],
                height: bitmap.size[1],
                format: bitmap.format as u32,
                bytes_per_pixel: match bitmap.format {
                    0 => 2,
                    3 => 1,
                    6 => 6,
                    _ => 4,
                },
            })
    }
    fn prepare_bitmap(&self, host: &mut dyn HostServices, id: i32) -> Result<Prepared> {
        let (id, bitmap) = self.bitmap(id)?;
        Ok(if let Some(image) = &bitmap.image {
            Prepared::Ready(image.clone())
        } else {
            Prepared::Job(host.begin_canvas_image(self.client, id)?)
        })
    }
    fn raster_plan(
        &self,
        host: &mut dyn HostServices,
        destination: i32,
        source: i32,
        raster: Raster,
        recreate: Option<[u32; 2]>,
    ) -> Result<BitmapWork> {
        ensure!((0..0x4000).contains(&destination), "BGI destination bitmap");
        let (_, bitmap) = self.bitmap(source)?;
        let source_format = bitmap.format;
        let mut inputs = Vec::new();
        let (base, format) = if let Some(size) = recreate {
            crysirror_image::rgba_len(size[0], size[1])?;
            (
                Some(ImageInput::Solid {
                    size,
                    color: if source_format == 1 {
                        [0, 0, 0, 255]
                    } else {
                        [0; 4]
                    },
                }),
                source_format,
            )
        } else {
            inputs.push(self.prepare_bitmap(host, destination)?);
            (None, self.bitmap(destination)?.1.format)
        };
        inputs.push(self.prepare_bitmap(host, source)?);
        let operation = IntegerRaster {
            destination_format: format as u32,
            source_format: source_format as u32,
            raster,
        };
        operation.validate()?;
        Ok(BitmapWork {
            reallocate: recreate.is_some(),
            outputs: Vec::new(),
            destination: destination as u32,
            format,
            auxiliary: [-1, -1],
            phase: Phase::Capture {
                inputs,
                base,
                operation: Some(operation),
            },
        })
    }
    fn bitmap_plan(
        &mut self,
        host: &mut dyn HostServices,
        vm: &Machine,
        call: &NativeCall,
    ) -> Result<Option<BitmapWork>> {
        let a = &call.args;
        match (call.group, call.id) {
            (0x90, 0xc2 | 0xc3 | 0xca) => {
                target_checks::bitmap(a[0])?;
                target_checks::bitmap(a[1])?;
                let source = self.bitmap(a[1])?.1;
                let (raster, recreate) = match call.id {
                    0xc2 => {
                        ensure!(a[0] != a[1], "BGI flip destination aliases source");
                        ensure!(matches!(a[2], 0 | 1), "BGI invalid flip direction");
                        (
                            Raster::Mirror {
                                vertical: a[2] == 1,
                            },
                            Some(source.size),
                        )
                    }
                    0xc3 => {
                        ensure!(
                            source.size.iter().all(|v| *v >= 2),
                            "BGI downsample source smaller than2x2"
                        );
                        (Raster::HalfBox, Some(source.size.map(|n| n.div_ceil(2))))
                    }
                    _ => {
                        self.bitmap(a[0])?;
                        (Raster::AspectFit, None)
                    }
                };
                Ok(Some(self.raster_plan(host, a[0], a[1], raster, recreate)?))
            }
            (0x90, 0x19..=0x1d) => self.immediate_bitmap_plan(host, call),
            (0x92, 0x1a) => self.alpha_plane_plan(host, call),
            (0x90, 0x04) => {
                target_checks::bitmap(a[0])?;
                Ok(Some(BitmapWork {
                    reallocate: true,
                    outputs: Vec::new(),
                    destination: a[0] as u32,
                    format: 2,
                    auxiliary: [-1; 2],
                    phase: Phase::Decode(host.begin_frame_capture(self.client)?),
                }))
            }
            (0x90, 0x10) => {
                let destination = a[0];
                target_checks::bitmap(destination)?;
                let archive = vm.string(a[1])?;
                let file = vm.string(a[2])?;
                let storage = host.storage(self.client)?;
                let client = self.client;
                let config = self.config.clone();
                let task = self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    let bytes =
                        program_io::read_raw(storage.as_ref(), client, &config, &archive, &file)?
                            .with_context(|| format!("BGI image is missing: {archive}:{file}"))?;
                    let bytes = if bytes.starts_with(b"DSC FORMAT 1.00") {
                        crysirror_resource::dsc::decode(&bytes)?
                    } else {
                        bytes
                    };
                    cancel.check()?;
                    Ok((file, bytes))
                })?;
                Ok(Some(BitmapWork {
                    reallocate: true,
                    outputs: Vec::new(),
                    destination: destination as u32,
                    format: 2,
                    auxiliary: [-1, -1],
                    phase: Phase::Read(task),
                }))
            }
            (0x90, 0x12) => {
                let destination = a[0];
                target_checks::bitmap(destination)?;
                if self.bitmaps.contains_key(&(destination as u32)) {
                    // Keep native display objects alive, but detach their released
                    // source before the engine destroys the canvas resource.
                    let affected = self
                        .sprites
                        .iter()
                        .filter_map(|(&id, sprite)| {
                            (sprite.bitmap == Some(destination as u32)).then_some(id)
                        })
                        .collect::<Vec<_>>();
                    for id in affected {
                        let sprite = self.sprites.get_mut(&id).unwrap();
                        sprite.bitmap = None;
                        sprite.visible = false;
                        self.submit(host,json!({"layers":[{"op":"set_visible","id":(id&0x1ff)+1,"visible":false}]}))?;
                    }
                    if !matches!(self.bitmaps[&(destination as u32)].format, 4 | 6) {
                        self.submit(host, json!({"canvases":[{"op":"retire","id":destination}]}))?;
                    }
                    self.release_graph_field_bitmap(destination as u32);
                    if let Some(bitmap) = self.bitmaps.remove(&(destination as u32)) {
                        if let Some(path) = bitmap.published_path {
                            host.release_image_resource(self.client, &path)?;
                        }
                    }
                    self.invalidate_graph_bitmap(destination as u32);
                }
                Ok(None)
            }
            (0x90, 0xcc) => {
                if a[1] == 0 {
                    self.color_luts.remove(&a[0]);
                } else {
                    let bytes = vm.memory(a[1] as u32, 24)?;
                    let words = bytes
                        .chunks_exact(4)
                        .map(|bytes| i32::from_le_bytes(bytes.try_into().unwrap()))
                        .collect::<Vec<_>>();
                    let points = [
                        [words[0], words[1]],
                        [words[2], words[3]],
                        [words[4], words[5]],
                    ];
                    ensure!(crysirror_image::integer_raster::graph_color::RuntimeColorLut::from_control_points(points).is_some(),"BGI tone curve points");
                    self.color_luts.insert(a[0], points);
                }
                Ok(None)
            }
            (0x90, 0x18) => {
                for id in [a[0], a[3]] {
                    target_checks::bitmap(id)?;
                }
                target_checks::effect_mode(a[4])?;
                target_checks::effect_level(a[5])?;
                let target = self.bitmap(a[0])?.1;
                let source = self.bitmap(a[3])?.1;
                let overlaps = (a[1] as i64) < i64::from(target.size[0])
                    && (a[2] as i64) < i64::from(target.size[1])
                    && i64::from(a[1]) + i64::from(source.size[0]) > 0
                    && i64::from(a[2]) + i64::from(source.size[1]) > 0;
                if !overlaps {
                    return Ok(None);
                }
                ensure!(
                    target.format == source.format
                        || matches!((target.format, source.format), (1, 2) | (2, 1)),
                    "BGI bitmap blit pixel formats"
                );
                Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    a[3],
                    Raster::Blit {
                        position: [a[1], a[2]],
                        mode: a[4],
                        parameter: a[5],
                    },
                    None,
                )?))
            }
            (0x90, 0x1e) => Ok(Some(self.raster_plan(
                host,
                a[0],
                a[3],
                Raster::CopyRect {
                    source: [a[4], a[5], a[6], a[7]],
                    position: [a[1], a[2]],
                },
                None,
            )?)),
            (0x90, 0x1f) => {
                ensure!(a[4] > 0 && a[5] > 0, "BGI bitmap region size");
                Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    a[1],
                    Raster::Blit {
                        position: [a[2].wrapping_neg(), a[3].wrapping_neg()],
                        mode: 128,
                        parameter: 0,
                    },
                    Some([a[4] as u32, a[5] as u32]),
                )?))
            }
            (0x90, 0xcd) => {
                let source = a[1];
                let destination = a[0];
                let points = *self
                    .color_luts
                    .get(&a[4])
                    .context("BGI tone curve identifier")?;
                ensure!(
                    matches!(a[6], 8 | 38) && a[3] as u32 <= 256 && a[7] as u32 <= 256,
                    "BGI tone curve mode/level"
                );
                let size = self.bitmap(source)?.1.size;
                Ok(Some(self.raster_plan(
                    host,
                    destination,
                    source,
                    Raster::ToneCurve {
                        points,
                        mono_color: a[2] as u32,
                        mono_level: a[3] as u32,
                        film_color: a[5] as u32,
                        film_level: a[7] as u32,
                    },
                    Some(size),
                )?))
            }
            (0x90, 0xc8) => {
                let transform = PerspectiveBlit {
                    dest_x: a[1],
                    dest_y: a[2],
                    anchor_x: a[4],
                    anchor_y: a[5],
                    scale_x: a[6],
                    scale_y: a[7],
                    translate_x: a[8],
                    translate_y: a[9],
                    translate_z: a[10],
                    pitch: a[11],
                    heading: a[12],
                    bank: a[13],
                    order: a[14],
                    distance: a[15],
                    mode: a[16],
                    parameter: a[17],
                };
                Ok(Some(self.raster_plan(
                    host,
                    a[0],
                    a[3],
                    Raster::Perspective { transform },
                    None,
                )?))
            }
            (0x91, 0x18..=0x1e) => {
                self.graph91_check_bitmap_operation(call.id, a)?;
                let mut recreate = None;
                let source = if matches!(call.id, 0x18 | 0x19) {
                    a[3]
                } else {
                    a[1]
                };
                let raster = match call.id {
                    0x18 | 0x19 => Raster::Rotate {
                        positions: [a[1], a[2], a[4], a[5]],
                        angle: a[6],
                        factors: [a[7] as u32, a[8] as u32],
                        alpha: a[9] as u32,
                        filtered: a[10] != 0,
                        copy: call.id == 0x19,
                    },
                    0x1a => {
                        if self.bitmap(a[0])?.1.format != 2 || self.bitmap(a[1])?.1.format != 2 {
                            return Ok(None);
                        }
                        Raster::ReplaceRgb { color: a[2] as u32 }
                    }
                    0x1b => {
                        let target = self.bitmap(a[0])?.1;
                        let source = self.bitmap(a[1])?.1;
                        if target.format != 1 || source.format != 1 || target.size != source.size {
                            return Ok(None);
                        }
                        Raster::Concentrate {
                            attenuation: a[4] as u32,
                            factors: [a[2] as u32, a[3] as u32],
                        }
                    }
                    0x1c => {
                        let source = self.bitmap(a[1])?.1;
                        let size = [
                            (a[2] as u32).wrapping_mul(source.size[0]) >> 16,
                            (a[3] as u32).wrapping_mul(source.size[1]) >> 16,
                        ];
                        recreate = Some(size);
                        Raster::Scale {
                            factors: [a[2] as u32, a[3] as u32],
                            filtered: a[4] != 0,
                        }
                    }
                    0x1d => Raster::Process {
                        kind: a[2] as u32,
                        color: a[3] as u32,
                        alpha: a[4] as u32,
                    },
                    0x1e => Raster::GrayMask {
                        offset: [a[2], a[3]],
                    },
                    _ => unreachable!(),
                };
                Ok(Some(
                    self.raster_plan(host, a[0], source, raster, recreate)?,
                ))
            }
            (0x91, 0x1f) => {
                target_checks::bitmap(a[0])?;
                let source = self.bitmap(a[1])?.1;
                Ok(Some(BitmapWork {
                    reallocate: true,
                    outputs: Vec::new(),
                    destination: a[0] as u32,
                    format: source.format,
                    auxiliary: source.auxiliary,
                    phase: Phase::Capture {
                        inputs: vec![self.prepare_bitmap(host, a[1])?],
                        base: None,
                        operation: None,
                    },
                }))
            }
            _ => bail!("BGI bitmap operation {:02x}:{:02x}", call.group, call.id),
        }
    }
    pub(super) fn bitmap_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if self.bitmap_work.is_none() {
            let released = call.group == 0x90
                && call.id == 0x12
                && self.bitmaps.contains_key(&(call.args[0] as u32));
            if call.group == 0x90 && call.id == 0x17 {
                let status = match self.bitmaps.get(&(call.args[0] as u32)) {
                    None => 1,
                    Some(bitmap) if bitmap.format == call.args[1] => 0,
                    Some(bitmap) if bitmap.format == 2 && call.args[1] == 1 => -1,
                    _ => 2,
                };
                if status >= 0 {
                    vm.commit_call(&[status])?;
                    self.calls += 1;
                    return Ok(true);
                }
                let bitmap = self.bitmap(call.args[0])?.1.clone();
                self.bitmap_work = Some(BitmapWork {
                    reallocate: false,
                    outputs: vec![0],
                    destination: call.args[0] as u32,
                    format: 1,
                    auxiliary: bitmap.auxiliary,
                    phase: Phase::Capture {
                        inputs: vec![self.prepare_bitmap(host, call.args[0])?],
                        base: None,
                        operation: None,
                    },
                });
            } else {
                self.bitmap_work = self.bitmap_plan(host, vm, call)?;
            }
            if self.bitmap_work.is_none() {
                vm.commit_call(if call.group == 0x90 && call.id == 0x12 {
                    if released { &[1] } else { &[0] }
                } else {
                    &[]
                })?;
                self.calls += 1;
                return Ok(true);
            }
        }
        let mut work = self.bitmap_work.take().unwrap();
        loop {
            let raw = match &mut work.phase {
                Phase::Read(task) => {
                    let Some((name, bytes)) = task.try_take()? else {
                        self.bitmap_work = Some(work);
                        return Ok(false);
                    };
                    if let Ok(metadata) = crysirror_image::cbg::parse_cbg_metadata(&bytes) {
                        work.format = metadata.native_bitmap_format().unwrap_or(2);
                        if metadata.embedded_point_flag == 1 {
                            work.auxiliary = [
                                i32::from(metadata.embedded_x),
                                i32::from(metadata.embedded_y),
                            ];
                        }
                    } else if matches!(
                        crysirror_image::cbg::detect_image_format(&bytes),
                        crysirror_image::cbg::ImageFormat::RawBgiImage
                    ) {
                        work.format = if u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == 24 {
                            1
                        } else {
                            2
                        };
                    } else if bytes.starts_with(b"BM") {
                        work.format = crysirror_image::cbg::bmp_native_bitmap_format(&bytes)?;
                    }
                    let name = program_io::relative(&self.config, &name)?;
                    work.phase = Phase::Decode(
                        host.image_metadata(self.client)?
                            .begin_encoded(&name, Arc::from(bytes))?,
                    );
                    continue;
                }
                Phase::Auxiliary { inputs, operation } => {
                    for input in inputs.iter_mut() {
                        if let Prepared::Job(job) = input {
                            match job.poll()? {
                                ImageProcessState::Pending => {
                                    self.bitmap_work = Some(work);
                                    return Ok(false);
                                }
                                ImageProcessState::Failed(error) => {
                                    bail!("BGI auxiliary image capture: {error}")
                                }
                                ImageProcessState::Ready(image) => *input = Prepared::Ready(image),
                            }
                        }
                    }
                    let images = inputs
                        .iter()
                        .map(|input| match input {
                            Prepared::Ready(image) => ImageInput::Resource(image.clone()),
                            _ => unreachable!(),
                        })
                        .collect::<Vec<_>>();
                    let (input, effects) = match operation {
                        AuxiliaryOperation::AlphaPlanes { size, spec } => (
                            ImageInput::Ternary {
                                destination: Box::new(ImageInput::Solid {
                                    size: *size,
                                    color: [0, 0, 0, 255],
                                }),
                                first: Box::new(images[0].clone()),
                                second: Box::new(images[1].clone()),
                            },
                            vec![ImageEffect::AlphaPlanePair { spec: spec.clone() }],
                        ),
                        AuxiliaryOperation::Mask(spec) => (
                            ImageInput::Ternary {
                                destination: Box::new(images[0].clone()),
                                first: Box::new(images[1].clone()),
                                second: Box::new(images[2].clone()),
                            },
                            vec![ImageEffect::StoredMaskBlend { spec: spec.clone() }],
                        ),
                        AuxiliaryOperation::Warp { size, spec } => (
                            ImageInput::FieldWarp {
                                source: Box::new(images[0].clone()),
                                size: *size,
                                spec: spec.clone(),
                            },
                            Vec::new(),
                        ),
                    };
                    work.phase = Phase::Process(
                        host.image_metadata(self.client)?
                            .begin_process(input, serde_json::to_vec(&effects)?.into())?,
                    );
                    continue;
                }
                Phase::Capture {
                    inputs,
                    base,
                    operation,
                } => {
                    for input in inputs.iter_mut() {
                        if let Prepared::Job(job) = input {
                            match job.poll()? {
                                ImageProcessState::Pending => {
                                    self.bitmap_work = Some(work);
                                    return Ok(false);
                                }
                                ImageProcessState::Failed(error) => {
                                    bail!("BGI bitmap capture: {error}")
                                }
                                ImageProcessState::Ready(image) => *input = Prepared::Ready(image),
                            }
                        }
                    }
                    let images = inputs
                        .iter()
                        .map(|input| match input {
                            Prepared::Ready(image) => image.clone(),
                            _ => unreachable!(),
                        })
                        .collect::<Vec<_>>();
                    if let Some(operation) = operation.take() {
                        let (destination, source) = if let Some(base) = base.take() {
                            (base, ImageInput::Resource(images[0].clone()))
                        } else {
                            (
                                ImageInput::Resource(images[0].clone()),
                                ImageInput::Resource(images[1].clone()),
                            )
                        };
                        let effects =
                            Arc::from(serde_json::to_vec(&vec![ImageEffect::IntegerRaster {
                                spec: operation,
                            }])?);
                        work.phase =
                            Phase::Process(host.image_metadata(self.client)?.begin_process(
                                ImageInput::Raster {
                                    destination: Box::new(destination),
                                    source: Box::new(source),
                                },
                                effects,
                            )?);
                        continue;
                    } else {
                        images[0].clone()
                    }
                }
                Phase::Decode(job) | Phase::Process(job) => match job.poll()? {
                    ImageProcessState::Pending => {
                        self.bitmap_work = Some(work);
                        return Ok(false);
                    }
                    ImageProcessState::Failed(error) => bail!("BGI bitmap operation: {error}"),
                    ImageProcessState::Ready(image) => image,
                },
                Phase::Opaque { raw, job } => match job.poll()? {
                    ImageProcessState::Pending => {
                        self.bitmap_work = Some(work);
                        return Ok(false);
                    }
                    ImageProcessState::Failed(error) => bail!("BGI bitmap presentation: {error}"),
                    ImageProcessState::Ready(display) => {
                        let raw = raw.clone();
                        self.publish_bitmap_allocation(
                            host,
                            work.destination,
                            work.format,
                            work.auxiliary,
                            raw,
                            display,
                            work.reallocate,
                        )?;
                        vm.commit_call(&work.outputs)?;
                        self.calls += 1;
                        return Ok(true);
                    }
                },
            };
            if matches!(work.format, 0 | 1) && !raw.opaque() {
                let effects = Arc::from(serde_json::to_vec(&vec![ImageEffect::OverrideAlpha {
                    alpha: 255,
                }])?);
                let job = host
                    .image_metadata(self.client)?
                    .begin_process(ImageInput::Resource(raw.clone()), effects)?;
                work.phase = Phase::Opaque { raw, job };
                continue;
            }
            self.publish_bitmap_allocation(
                host,
                work.destination,
                work.format,
                work.auxiliary,
                raw.clone(),
                raw,
                work.reallocate,
            )?;
            vm.commit_call(&work.outputs)?;
            self.calls += 1;
            return Ok(true);
        }
    }
    pub(super) fn publish_bitmap(
        &mut self,
        host: &mut dyn HostServices,
        id: u32,
        format: i32,
        auxiliary: [i32; 2],
        raw: Arc<dyn ImageResource>,
        display: Arc<dyn ImageResource>,
    ) -> Result<()> {
        self.publish_bitmap_allocation(host, id, format, auxiliary, raw, display, false)
    }
    pub(super) fn publish_bitmap_reallocated(
        &mut self,
        host: &mut dyn HostServices,
        id: u32,
        format: i32,
        auxiliary: [i32; 2],
        raw: Arc<dyn ImageResource>,
        display: Arc<dyn ImageResource>,
    ) -> Result<()> {
        self.publish_bitmap_allocation(host, id, format, auxiliary, raw, display, true)
    }
    fn publish_bitmap_allocation(
        &mut self,
        host: &mut dyn HostServices,
        id: u32,
        format: i32,
        auxiliary: [i32; 2],
        raw: Arc<dyn ImageResource>,
        display: Arc<dyn ImageResource>,
        reallocate: bool,
    ) -> Result<()> {
        let generation = if reallocate || !self.bitmaps.contains_key(&id) {
            self.allocate_bitmap_generation()?
        } else {
            self.bitmaps[&id].generation
        };
        let previous = self.bitmaps.get(&id).and_then(|b| b.published_path.clone());
        let uploaded = host.publish_image_resource(self.client, 0xb100_0000 | id, display)?;
        let size = raw.size();
        let asset = uploaded.path.relative();
        let published_path = Some(asset.to_owned());
        self.submit(host,json!({"canvases":[{"op":"create","id":id,"width":size[0],"height":size[1]},{"op":"set","id":id,"layer":{"id":0,"rect":[0,0,size[0],size[1]],"paint":{"kind":"image","asset":asset}}}]}))?;
        self.release_graph_field_bitmap(id);
        self.bitmaps.insert(
            id,
            Bitmap {
                published_path,
                generation,
                size,
                format,
                image: Some(raw),
                auxiliary,
            },
        );
        if let Some(path) = previous {
            host.release_image_resource(self.client, &path)?;
        }
        self.invalidate_graph_bitmap(id);
        Ok(())
    }
}

#[path = "bitmap_immediate.rs"]
mod bitmap_immediate;
