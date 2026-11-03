//! Native field allocation policy; storage and numerical work are engine-owned.
use super::*;
use crysirror_bgi_vm::HostMemoryWrite;
use crysirror_core::image_processing::VectorFieldResource;
use crysirror_image::vector_field;

pub(crate) enum FieldMemoryWork {
    Import(Task<Arc<VectorFieldResource>>),
    Export(Task<Arc<[u8]>>),
}
impl BgiVm {
    pub(crate) fn field_bitmap_handles(&self, call: &NativeCall) -> bool {
        let a = &call.args;
        match (call.group, call.id) {
            (0x90, 0x11 | 0x14) => matches!(a.get(3), Some(4 | 6)),
            (0x90, 0x13) => self
                .bitmaps
                .get(&(a[0] as u32))
                .is_some_and(|b| matches!(b.format, 4 | 6)),
            (0x90, 0x15) => self
                .bitmaps
                .get(&(a[3] as u32))
                .is_some_and(|b| matches!(b.format, 4 | 6)),
            (0x92, 0x17) | (0x91, 0x1f) => self
                .bitmaps
                .get(&(a[1] as u32))
                .is_some_and(|b| matches!(b.format, 4 | 6)),
            _ => false,
        }
    }
    pub(crate) fn install_field_bitmap(
        &mut self,
        host: &mut dyn HostServices,
        id: u32,
        field: Arc<VectorFieldResource>,
        format: i32,
        auxiliary: [i32; 2],
        reallocate: bool,
    ) -> Result<()> {
        field.validate(self.client)?;
        if let Some(old) = self.bitmaps.get(&id) {
            if !matches!(old.format, 4 | 6) {
                self.submit(host, json!({"canvases":[{"op":"retire","id":id}]}))?;
            }
        }
        let generation = if !reallocate && self.bitmaps.contains_key(&id) {
            self.bitmaps[&id].generation
        } else {
            self.allocate_bitmap_generation()?
        };
        let previous = self.bitmaps.insert(
            id,
            Bitmap {
                generation,
                published_path: None,
                size: field.size,
                format,
                image: None,
                auxiliary,
            },
        );
        self.replace_graph_field_bitmap(id, field)?;
        if let Some(path) = previous.and_then(|b| b.published_path) {
            host.release_image_resource(self.client, &path)?;
        }
        Ok(())
    }
    pub(crate) fn field_bitmap_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        match (call.group, call.id) {
            (0x90, 0x11 | 0x14 | 0x13) => {
                let id = a[0] as u32;
                target_checks::bitmap(a[0])?;
                let (size, format, auxiliary, reallocate) = if call.id == 0x13 {
                    let b = self.bitmap(a[0])?.1;
                    if a[1] != 0 {
                        vm.commit_call(&[])?;
                        self.calls += 1;
                        return Ok(true);
                    }
                    (b.size, b.format, b.auxiliary, false)
                } else {
                    (
                        [u32::try_from(a[1])?, u32::try_from(a[2])?],
                        a[3],
                        [-1; 2],
                        true,
                    )
                };
                let mut task = if let Some(work) = self.field_memory_jobs.remove(&request) {
                    let FieldMemoryWork::Import(task) = work else {
                        bail!("BGI field import continuation kind");
                    };
                    task
                } else {
                    let client = self.client;
                    let bytes = if call.id == 0x14 {
                        let bpp = if format == 6 { 6 } else { 4 };
                        let len = (size[0] as usize)
                            .checked_mul(size[1] as usize)
                            .and_then(|n| n.checked_mul(bpp))
                            .context("BGI packed field size")?;
                        Some(vm.memory(a[4] as u32, len)?)
                    } else {
                        None
                    };
                    self.pool.spawn(move |cancel| {
                        cancel.check()?;
                        match bytes {
                            Some(bytes) => {
                                vector_field::unpack(client, size, format, &bytes, &|| {
                                    cancel.check()
                                })
                            }
                            None => vector_field::blank(client, size, format),
                        }
                    })?
                };
                let Some(field) = task.try_take()? else {
                    self.field_memory_jobs
                        .insert(request, FieldMemoryWork::Import(task));
                    return Ok(false);
                };
                self.install_field_bitmap(host, id, field, format, auxiliary, reallocate)?;
                vm.commit_call(&[])?;
            }
            (0x91, 0x1f) => {
                target_checks::bitmap(a[0])?;
                let b = self.bitmap(a[1])?.1.clone();
                let field = self
                    .graph_field_bitmap(a[1] as u32)
                    .context("BGI field source allocation")?;
                self.install_field_bitmap(host, a[0] as u32, field, b.format, b.auxiliary, true)?;
                vm.commit_call(&[])?;
            }
            (0x90, 0x15) | (0x92, 0x17) => {
                let id = if call.group == 0x90 { a[3] } else { a[1] };
                let b = self.bitmap(id)?.1.clone();
                if call.group == 0x92 {
                    let status = if b.format == 6 {
                        Some(2)
                    } else if a[2] < 0
                        || a[3] < 0
                        || a[2] as u32 >= b.size[0]
                        || a[3] as u32 >= b.size[1]
                    {
                        Some(3)
                    } else {
                        None
                    };
                    if let Some(status) = status {
                        vm.commit_call(&[status])?;
                        self.calls += 1;
                        return Ok(true);
                    }
                }
                let mut task = if let Some(work) = self.field_memory_jobs.remove(&request) {
                    let FieldMemoryWork::Export(task) = work else {
                        bail!("BGI field export continuation kind");
                    };
                    task
                } else {
                    let field = self
                        .graph_field_bitmap(id as u32)
                        .context("BGI field export allocation")?;
                    if call.group == 0x90 {
                        ensure!(
                            field.byte_len() <= a[2] as u32 as usize,
                            "BGI field output capacity"
                        );
                    }
                    self.pool
                        .spawn(move |cancel| vector_field::pack(&field, &|| cancel.check()))?
                };
                let Some(bytes) = task.try_take()? else {
                    self.field_memory_jobs
                        .insert(request, FieldMemoryWork::Export(task));
                    return Ok(false);
                };
                if call.group == 0x90 {
                    vm.commit_call_writes(
                        &[],
                        vec![
                            HostMemoryWrite {
                                address: a[1] as u32,
                                bytes: (bytes.len() as u32).to_le_bytes().to_vec(),
                            },
                            HostMemoryWrite {
                                address: a[0] as u32,
                                bytes: bytes.to_vec(),
                            },
                        ],
                    )?;
                } else {
                    let offset = (a[3] as usize * b.size[0] as usize + a[2] as usize) * 4;
                    vm.commit_call_writes(
                        &[0],
                        vec![HostMemoryWrite {
                            address: a[0] as u32,
                            bytes: bytes[offset..offset + 4].to_vec(),
                        }],
                    )?;
                }
            }
            _ => bail!("BGI field bitmap dispatch"),
        }
        self.calls += 1;
        Ok(true)
    }
}
