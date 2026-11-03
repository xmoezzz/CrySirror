//! Native guest-memory image exchange through immutable engine resources.
use super::*;
use crysirror_bgi_vm::HostMemoryWrite;
use crysirror_core::image_processing::*;
pub(crate) fn handles(group: u8, id: u16) -> bool {
    matches!(
        (group, id),
        (0x90, 0x14..=0x16) | (0x92, 0x12 | 0x16 | 0x17)
    )
}
pub(crate) enum MemoryWork {
    Capture(ImageJob),
    Import(ImageJob),
    Export(Task<Vec<u8>>),
}
fn layout(format: i32) -> Result<PackedPixelLayout> {
    Ok(match format {
        0 => PackedPixelLayout::Bgr555,
        1 | 7 => PackedPixelLayout::Bgrx8,
        2 => PackedPixelLayout::Bgra8,
        3 => PackedPixelLayout::Red8,
        _ => bail!("BGI packed image format {format} requires its engine pixel resource"),
    })
}
impl BgiVm {
    pub(crate) fn bitmap_memory_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        let mut writes = Vec::new();
        let mut outputs = Vec::new();
        match (call.group, call.id) {
            (0x90, 0x16) => {
                let info = self.bitmap_info(a[1]);
                writes.push(HostMemoryWrite {
                    address: a[0] as u32,
                    bytes: 0u32.to_le_bytes().to_vec(),
                });
                if let Some(info) = info {
                    let words = [
                        info.row_stride,
                        info.width,
                        info.height,
                        info.format,
                        info.bytes_per_pixel,
                    ];
                    writes.push(HostMemoryWrite {
                        address: (a[0] as u32).wrapping_add(4),
                        bytes: words.into_iter().flat_map(u32::to_le_bytes).collect(),
                    });
                    outputs.push(1);
                } else {
                    outputs.push(0);
                }
            }
            (0x92, 0x12) => {
                let found = if let Some(bitmap) = self.bitmaps.get_mut(&(a[0] as u32)) {
                    bitmap.auxiliary = [a[1], a[2]];
                    true
                } else {
                    false
                };
                outputs.push(i32::from(found));
            }
            (0x92, 0x16) => {
                if let Some(bitmap) = self.bitmaps.get(&(a[1] as u32)) {
                    writes.push(HostMemoryWrite {
                        address: a[0] as u32,
                        bytes: bitmap
                            .auxiliary
                            .into_iter()
                            .flat_map(i32::to_le_bytes)
                            .collect(),
                    });
                    outputs.push(1);
                } else {
                    outputs.push(0);
                }
            }
            (0x90, 0x14) => {
                target_checks::bitmap(a[0])?;
                let format = if a[3] == 7 { 1 } else { a[3] };
                let layout = layout(format)?;
                let work = if let Some(work) = self.memory_jobs.remove(&request) {
                    work
                } else {
                    let size = [u32::try_from(a[1])?, u32::try_from(a[2])?];
                    let len =
                        crysirror_image::rgba_len(size[0], size[1])? / 4 * layout.bytes_per_pixel();
                    let bytes = Arc::from(vm.memory(a[4] as u32, len)?);
                    MemoryWork::Import(host.image_metadata(self.client)?.begin_packed_pixels(
                        PackedImageSpec {
                            size,
                            layout,
                            bytes,
                        },
                    )?)
                };
                let MemoryWork::Import(mut job) = work else {
                    bail!("BGI stale pixel import continuation")
                };
                match job.poll()? {
                    ImageProcessState::Pending => {
                        self.memory_jobs.insert(request, MemoryWork::Import(job));
                        return Ok(false);
                    }
                    ImageProcessState::Failed(error) => bail!("BGI pixel import: {error}"),
                    ImageProcessState::Ready(image) => {
                        self.publish_bitmap_reallocated(
                            host,
                            a[0] as u32,
                            format,
                            [-1; 2],
                            image.clone(),
                            image,
                        )?;
                    }
                }
            }
            (0x90, 0x15) | (0x92, 0x17) => {
                let bitmap_id = if call.group == 0x90 { a[3] } else { a[1] };
                if call.group == 0x92 {
                    let status = if let Some(bitmap) = self.bitmaps.get(&(bitmap_id as u32)) {
                        if bitmap.format == 6 {
                            Some(2)
                        } else if a[2] < 0
                            || a[3] < 0
                            || a[2] as u32 >= bitmap.size[0]
                            || a[3] as u32 >= bitmap.size[1]
                        {
                            Some(3)
                        } else {
                            None
                        }
                    } else {
                        Some(1)
                    };
                    if let Some(status) = status {
                        vm.commit_call(&[status])?;
                        self.calls += 1;
                        return Ok(true);
                    }
                }
                let bitmap = self.bitmap(bitmap_id)?.1.clone();
                let pixel_layout = layout(bitmap.format)?;
                if call.group == 0x90 {
                    let required = bitmap.size[0] as usize
                        * bitmap.size[1] as usize
                        * pixel_layout.bytes_per_pixel();
                    ensure!(
                        required <= a[2] as u32 as usize,
                        "BGI pixel output capacity"
                    );
                }
                let mut work = self.memory_jobs.remove(&request);
                let image = if matches!(&work, Some(MemoryWork::Capture(_))) {
                    let Some(MemoryWork::Capture(mut job)) = work.take() else {
                        unreachable!()
                    };
                    match job.poll()? {
                        ImageProcessState::Pending => {
                            self.memory_jobs.insert(request, MemoryWork::Capture(job));
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => bail!("BGI pixel capture: {error}"),
                        ImageProcessState::Ready(image) => Some(image),
                    }
                } else if work.is_some() {
                    None
                } else if let Some(image) = bitmap.image {
                    Some(image)
                } else {
                    self.memory_jobs.insert(
                        request,
                        MemoryWork::Capture(
                            host.begin_canvas_image(self.client, bitmap_id as u32)?,
                        ),
                    );
                    return Ok(false);
                };
                if call.group == 0x92 {
                    let image = image.context("BGI pixel-read continuation")?;
                    let [r, g, b, alpha] = image
                        .pixel_at([a[2] as u32, a[3] as u32])?
                        .context("BGI pixel outside image")?;
                    let value = match bitmap.format {
                        0 => {
                            (u32::from(r >> 3) << 10) | (u32::from(g >> 3) << 5) | u32::from(b >> 3)
                        }
                        1 => u32::from_le_bytes([b, g, r, 0]),
                        3 => u32::from(r),
                        _ => u32::from_le_bytes([b, g, r, alpha]),
                    };
                    writes.push(HostMemoryWrite {
                        address: a[0] as u32,
                        bytes: value.to_le_bytes().to_vec(),
                    });
                    outputs.push(0);
                } else {
                    let mut task = if let Some(MemoryWork::Export(task)) = work {
                        task
                    } else {
                        let image = image.context("BGI pixel-export continuation")?;
                        self.pool.spawn(move |cancel| {
                            crysirror_image::packed_pixels::encode(
                                image.size(),
                                &image.rgba(),
                                pixel_layout,
                                &|| cancel.check(),
                            )
                        })?
                    };
                    let Some(bytes) = task.try_take()? else {
                        self.memory_jobs.insert(request, MemoryWork::Export(task));
                        return Ok(false);
                    };
                    writes.push(HostMemoryWrite {
                        address: a[1] as u32,
                        bytes: (bytes.len() as u32).to_le_bytes().to_vec(),
                    });
                    writes.push(HostMemoryWrite {
                        address: a[0] as u32,
                        bytes,
                    });
                }
            }
            _ => unreachable!(),
        }
        ensure!(
            outputs.len() == call.outputs,
            "BGI pixel exchange result count"
        );
        vm.commit_call_writes(&outputs, writes)?;
        self.calls += 1;
        Ok(true)
    }
}
