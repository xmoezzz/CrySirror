//! BG cache namespaces and native encoding ABI over engine codecs/image jobs.
use super::*;
use crysirror_bgi_vm::HostMemoryWrite;
use crysirror_core::image_processing::{
    ImageInput, ImageJob, ImageProcessState, ImageResource, VectorFieldResource,
};
use crysirror_image::{
    cbg::{CbgEncoding, NativeImageHeader},
    effects::ImageEffect,
};
#[derive(Default)]
pub(crate) struct CodecState {
    cache: BTreeMap<(String, String), Cached>,
    jobs: BTreeMap<u64, CodecWork>,
}
#[derive(Clone)]
struct Cached {
    image: Arc<dyn ImageResource>,
    format: i32,
    point: [i32; 2],
}
enum CodecWork {
    Header(Task<NativeImageHeader>),
    Blank {
        header: NativeImageHeader,
        job: ImageJob,
    },
    BlankField {
        header: NativeImageHeader,
        task: Task<Arc<VectorFieldResource>>,
    },
    Register {
        key: (String, String),
        job: ImageJob,
        format: i32,
        point: [i32; 2],
    },
    CachedPublish {
        raw: Arc<dyn ImageResource>,
        job: ImageJob,
        format: i32,
        point: [i32; 2],
    },
    Capture(ImageJob),
    Encode(Task<Vec<u8>>),
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    group == 0x90 && matches!(id, 0xc0 | 0xc6 | 0xc7 | 0xce)
}
fn point_format(bytes: &[u8]) -> (i32, [i32; 2]) {
    if let Ok(meta) = crysirror_image::cbg::parse_cbg_metadata(bytes) {
        return (
            meta.native_bitmap_format().unwrap_or(2),
            if meta.embedded_point_flag == 1 {
                [i32::from(meta.embedded_x), i32::from(meta.embedded_y)]
            } else {
                [-1; 2]
            },
        );
    }
    (
        crysirror_image::cbg::bmp_native_bitmap_format(bytes).unwrap_or(2),
        [-1; 2],
    )
}
impl BgiVm {
    pub(crate) fn bitmap_codec_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        let mut work = self.codec.jobs.remove(&request);
        match call.id {
            0xc0 => {
                target_checks::bitmap(a[0])?;
                if work.is_none() {
                    let storage = host.storage(self.client)?;
                    let config = self.config.clone();
                    let client = self.client;
                    let archive = vm.string(a[1])?;
                    let file = vm.string(a[2])?;
                    work = Some(CodecWork::Header(self.pool.spawn(move |cancel| {
                        cancel.check()?;
                        let bytes = program_io::read_raw(
                            storage.as_ref(),
                            client,
                            &config,
                            &archive,
                            &file,
                        )?
                        .context("BGI header image is missing")?;
                        let bytes = if bytes.starts_with(b"DSC FORMAT 1.00") {
                            crysirror_resource::dsc::decode(&bytes)?
                        } else {
                            bytes
                        };
                        crysirror_image::cbg::native_image_header(&bytes)
                    })?));
                }
                loop {
                    match work.take().context("BGI header continuation")? {
                        CodecWork::Header(mut task) => {
                            let Some(header) = task.try_take()? else {
                                self.codec.jobs.insert(request, CodecWork::Header(task));
                                return Ok(false);
                            };
                            ensure!(
                                header.size.iter().all(|n| *n > 0),
                                "BGI header bitmap dimensions"
                            );
                            work = Some(if matches!(header.format, 4 | 6) {
                                let client = self.client;
                                CodecWork::BlankField {
                                    header,
                                    task: self.pool.spawn(move |cancel| {
                                        cancel.check()?;
                                        crysirror_image::vector_field::blank(
                                            client,
                                            header.size,
                                            header.format,
                                        )
                                    })?,
                                }
                            } else {
                                ensure!(
                                    matches!(header.format, 0..=3),
                                    "BGI header allocation pixel format {}",
                                    header.format
                                );
                                CodecWork::Blank {
                                    header,
                                    job: host.image_metadata(self.client)?.begin_process(
                                        ImageInput::Solid {
                                            size: header.size,
                                            color: if header.format == 2 {
                                                [0; 4]
                                            } else {
                                                [0, 0, 0, 255]
                                            },
                                        },
                                        serde_json::to_vec(&Vec::<ImageEffect>::new())?.into(),
                                    )?,
                                }
                            });
                        }
                        CodecWork::Blank { header, mut job } => match job.poll()? {
                            ImageProcessState::Pending => {
                                self.codec
                                    .jobs
                                    .insert(request, CodecWork::Blank { header, job });
                                return Ok(false);
                            }
                            ImageProcessState::Failed(error) => {
                                bail!("BGI header allocation: {error}")
                            }
                            ImageProcessState::Ready(image) => {
                                self.publish_bitmap_reallocated(
                                    host,
                                    a[0] as u32,
                                    header.format,
                                    header.reference_point,
                                    image.clone(),
                                    image,
                                )?;
                                vm.commit_call(&[])?;
                                break;
                            }
                        },
                        CodecWork::BlankField { header, mut task } => {
                            let Some(field) = task.try_take()? else {
                                self.codec
                                    .jobs
                                    .insert(request, CodecWork::BlankField { header, task });
                                return Ok(false);
                            };
                            self.install_field_bitmap(
                                host,
                                a[0] as u32,
                                field,
                                header.format,
                                header.reference_point,
                                true,
                            )?;
                            vm.commit_call(&[])?;
                            break;
                        }
                        _ => bail!("BGI header allocation continuation kind"),
                    }
                }
            }
            0xc6 => {
                if work.is_none() {
                    let bytes = vm.memory(a[2] as u32, a[3].max(0) as usize)?;
                    let (format, point) = point_format(&bytes);
                    let key = (
                        vm.string(a[0])?.to_ascii_lowercase(),
                        vm.string(a[1])?.to_ascii_lowercase(),
                    );
                    work = Some(CodecWork::Register {
                        key,
                        format,
                        point,
                        job: host
                            .image_metadata(self.client)?
                            .begin_encoded("native-bg-cache", bytes.into())?,
                    });
                }
                let Some(CodecWork::Register {
                    key,
                    mut job,
                    format,
                    point,
                }) = work
                else {
                    bail!("BGI BG registration continuation kind");
                };
                match job.poll()? {
                    ImageProcessState::Pending => {
                        self.codec.jobs.insert(
                            request,
                            CodecWork::Register {
                                key,
                                job,
                                format,
                                point,
                            },
                        );
                        return Ok(false);
                    }
                    ImageProcessState::Failed(_) => vm.commit_call(&[0])?,
                    ImageProcessState::Ready(image) => {
                        ensure!(
                            self.codec.cache.len() < 4096 || self.codec.cache.contains_key(&key),
                            "BGI BG cache entry budget"
                        );
                        self.codec.cache.entry(key).or_insert(Cached {
                            image,
                            format,
                            point,
                        });
                        vm.commit_call(&[1])?;
                    }
                }
            }
            0xc7 => {
                target_checks::bitmap(a[0])?;
                if let Some(CodecWork::CachedPublish {
                    raw,
                    mut job,
                    format,
                    point,
                }) = work
                {
                    match job.poll()? {
                        ImageProcessState::Pending => {
                            self.codec.jobs.insert(
                                request,
                                CodecWork::CachedPublish {
                                    raw,
                                    job,
                                    format,
                                    point,
                                },
                            );
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => {
                            bail!("BGI cached BG presentation: {error}")
                        }
                        ImageProcessState::Ready(display) => {
                            self.publish_bitmap_reallocated(
                                host,
                                a[0] as u32,
                                format,
                                point,
                                raw,
                                display,
                            )?;
                            let key = (
                                vm.string(a[1])?.to_ascii_lowercase(),
                                vm.string(a[2])?.to_ascii_lowercase(),
                            );
                            if a[3] != 0 {
                                self.codec.cache.remove(&key);
                            }
                            vm.commit_call(&[1])?;
                        }
                    }
                } else {
                    let key = (
                        vm.string(a[1])?.to_ascii_lowercase(),
                        vm.string(a[2])?.to_ascii_lowercase(),
                    );
                    if let Some(cached) = self.codec.cache.get(&key).cloned() {
                        ensure!(
                            matches!(cached.format, 0..=3),
                            "BGI cached BG field format requires a field resource"
                        );
                        if cached.format != 2 && !cached.image.opaque() {
                            let job = host.image_metadata(self.client)?.begin_process(
                                ImageInput::Resource(cached.image.clone()),
                                serde_json::to_vec(&vec![ImageEffect::OverrideAlpha {
                                    alpha: 255,
                                }])?
                                .into(),
                            )?;
                            self.codec.jobs.insert(
                                request,
                                CodecWork::CachedPublish {
                                    raw: cached.image,
                                    job,
                                    format: cached.format,
                                    point: cached.point,
                                },
                            );
                            return Ok(false);
                        }
                        self.publish_bitmap_reallocated(
                            host,
                            a[0] as u32,
                            cached.format,
                            cached.point,
                            cached.image.clone(),
                            cached.image,
                        )?;
                        if a[3] != 0 {
                            self.codec.cache.remove(&key);
                        }
                        vm.commit_call(&[1])?;
                    } else {
                        vm.commit_call(&[0])?;
                    }
                }
            }
            0xce => {
                let bitmap = self.bitmap(a[2])?.1.clone();
                ensure!(
                    matches!(bitmap.format, 1 | 2),
                    "BGI bitmap encoding requires truecolor"
                );
                ensure!(
                    u64::from(bitmap.size[0]) * u64::from(bitmap.size[1]) <= 0x400000,
                    "BGI encode pixel count"
                );
                let encoding = match a[3] {
                    0 => CbgEncoding::LosslessV1,
                    1 => {
                        ensure!((0..=100).contains(&a[4]), "BGI bitmap quality");
                        CbgEncoding::LossyV2 {
                            quality: a[4] as u8,
                        }
                    }
                    _ => bail!("BGI bitmap encoding format"),
                };
                if work.is_none() {
                    if let Some(image) = bitmap.image {
                        work = Some(self.begin_bitmap_encode(
                            image,
                            bitmap.format == 2,
                            encoding,
                            vm.native_call_tick(),
                        )?);
                    } else {
                        work = Some(CodecWork::Capture(
                            host.begin_canvas_image(self.client, a[2] as u32)?,
                        ));
                    }
                }
                work = Some(match work.take().context("BGI encode continuation")? {
                    CodecWork::Capture(mut job) => match job.poll()? {
                        ImageProcessState::Pending => {
                            self.codec.jobs.insert(request, CodecWork::Capture(job));
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => bail!("BGI encode capture: {error}"),
                        ImageProcessState::Ready(image) => self.begin_bitmap_encode(
                            image,
                            bitmap.format == 2,
                            encoding,
                            vm.native_call_tick(),
                        )?,
                    },
                    other => other,
                });
                let Some(CodecWork::Encode(mut task)) = work else {
                    bail!("BGI bitmap encode continuation kind");
                };
                let Some(bytes) = task.try_take()? else {
                    self.codec.jobs.insert(request, CodecWork::Encode(task));
                    return Ok(false);
                };
                let mut writes = vec![HostMemoryWrite {
                    address: a[1] as u32,
                    bytes: (bytes.len() as u32).to_le_bytes().to_vec(),
                }];
                if a[0] != 0 {
                    writes.push(HostMemoryWrite {
                        address: a[0] as u32,
                        bytes,
                    });
                }
                vm.commit_call_writes(&[], writes)?;
            }
            _ => bail!("BGI codec dispatch"),
        }
        self.calls += 1;
        Ok(true)
    }
    fn begin_bitmap_encode(
        &self,
        image: Arc<dyn ImageResource>,
        alpha: bool,
        encoding: CbgEncoding,
        key: u32,
    ) -> Result<CodecWork> {
        Ok(CodecWork::Encode(self.pool.spawn(move |cancel| {
            let [width, height] = image.size();
            let surface = crysirror_image::ImageSurface {
                width,
                height,
                rgba: image.rgba().to_vec(),
            };
            crysirror_image::cbg::encode_surface(&surface, alpha, encoding, key, &|| cancel.check())
        })?))
    }
}
