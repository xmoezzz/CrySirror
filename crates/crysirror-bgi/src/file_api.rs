//! Original native file status/offset policy over CrySirror capability jobs.
use super::*;
use crysirror_bgi_vm::HostMemoryWrite;

pub(crate) struct FileReply {
    pub(super) outputs: Vec<i32>,
    pub(super) writes: Vec<HostMemoryWrite>,
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    group == 0x80
        && matches!(
            id,
            0x24 | 0x27 | 0x28 | 0x29 | 0x2a | 0x2c | 0x2f | 0x30..=0x35
        )
}
impl BgiVm {
    pub(crate) fn file_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if self.file_load.is_none()
            && matches!(call.id, 0x24 | 0x27 | 0x28 | 0x29 | 0x2a | 0x2c | 0x2f)
        {
            let storage = host.storage(self.client)?;
            let config = self.config.clone();
            let id = call.id;
            let first = vm.string(call.args[0])?;
            let second = if matches!(id, 0x27 | 0x2f) {
                Some(vm.string(call.args[1])?)
            } else {
                None
            };
            let recursive = id == 0x24 && call.args[1] != 0;
            self.file_load = Some(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let result = match id {
                    0x24 => super::system_files::count_files(
                        storage.as_ref(),
                        &config,
                        &first,
                        recursive,
                    )?,
                    0x2a => i32::from(
                        super::system_files::metadata(storage.as_ref(), &config, &first)?
                            .is_some_and(|(_, m)| m.is_directory),
                    ),
                    0x2c => {
                        if let Some((mount, m)) =
                            super::system_files::metadata(storage.as_ref(), &config, &first)?
                        {
                            let relative = super::system_files::relative(&config, &first)?;
                            let path = if mount == Mount::Asset {
                                if relative.is_empty() {
                                    config.root.clone()
                                } else {
                                    config.asset(&relative)?
                                }
                            } else {
                                relative
                            };
                            i32::from(storage.read_only(mount, &path)?)
                                | if m.is_directory { 0x10 } else { 0x20 }
                        } else {
                            -1
                        }
                    }
                    0x28 | 0x29 => {
                        let relative = super::system_files::relative(&config, &first)?;
                        if relative.is_empty() {
                            0
                        } else {
                            let path = VfsPath::new(Mount::Save, &relative)?;
                            let result = if id == 0x28 {
                                storage.create_directory(&path)
                            } else {
                                storage.remove_directory(&path)
                            };
                            match result {
                                Ok(()) => 1,
                                Err(error @ Error::Unsupported(_)) => return Err(error.into()),
                                Err(_) => 0,
                            }
                        }
                    }
                    0x27 => {
                        let source = VfsPath::new(
                            Mount::Save,
                            &program_io::relative(&config, second.as_deref().unwrap())?,
                        )?;
                        let target =
                            VfsPath::new(Mount::Save, &program_io::relative(&config, &first)?)?;
                        match storage.rename_no_replace(&source, &target) {
                            Ok(()) => 1,
                            Err(error @ Error::Unsupported(_)) => return Err(error.into()),
                            Err(_) => 0,
                        }
                    }
                    0x2f => {
                        let source = second.as_deref().unwrap();
                        if let Some((mount, info)) =
                            super::system_files::metadata(storage.as_ref(), &config, source)?
                        {
                            if info.is_directory {
                                0
                            } else {
                                let relative = super::system_files::relative(&config, source)?;
                                let source = VfsPath::new(
                                    mount,
                                    &if mount == Mount::Asset {
                                        config.asset(&relative)?
                                    } else {
                                        relative
                                    },
                                )?;
                                let target = VfsPath::new(
                                    Mount::Save,
                                    &program_io::relative(&config, &first)?,
                                )?;
                                match storage.copy_file(&source, &target, true) {
                                    Ok(()) => 1,
                                    Err(error @ Error::Unsupported(_)) => return Err(error.into()),
                                    Err(_) => 0,
                                }
                            }
                        } else {
                            0
                        }
                    }
                    _ => unreachable!(),
                };
                cancel.check()?;
                Ok(FileReply {
                    outputs: vec![result],
                    writes: Vec::new(),
                })
            })?);
        }
        if self.file_load.is_none() {
            let a = &call.args;
            let storage = host.storage(self.client)?;
            let client = self.client;
            let config = self.config.clone();
            let id = call.id;
            let (archive, file, destination, range, write) = match id {
                0x30 | 0x31 => (
                    if a[1] == 0 {
                        String::new()
                    } else {
                        vm.string(a[1])?
                    },
                    vm.string(a[2])?,
                    a[0] as u32,
                    (id == 0x31).then(|| (a[3] as u32 as usize, a[4] as u32 as usize)),
                    None,
                ),
                0x32 => (
                    String::new(),
                    vm.string(a[0])?,
                    0,
                    None,
                    Some(vm.memory(a[1] as u32, a[2] as u32 as usize)?),
                ),
                0x33 => {
                    let directory = if a[0] == 0 {
                        String::new()
                    } else {
                        vm.string(a[0])?
                    };
                    let file = vm.string(a[1])?;
                    (
                        String::new(),
                        if directory.is_empty() {
                            file
                        } else {
                            format!("{directory}/{file}")
                        },
                        0,
                        None,
                        None,
                    )
                }
                0x34 | 0x35 => (
                    if a[0] == 0 {
                        String::new()
                    } else {
                        vm.string(a[0])?
                    },
                    vm.string(a[1])?,
                    0,
                    None,
                    None,
                ),
                _ => bail!("BGI file native {id:#x}"),
            };
            self.file_load = Some(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let mut writes = Vec::new();
                let result = match id {
                    0x32 => {
                        let path =
                            VfsPath::new(Mount::Save, &program_io::relative(&config, &file)?)?;
                        let bytes = write.as_ref().context("BGI write payload")?;
                        i32::from(
                            storage.write_atomic_existing_parent(&path, bytes).is_ok()
                                || bytes.is_empty(),
                        )
                    }
                    0x33 => {
                        let path =
                            VfsPath::new(Mount::Save, &program_io::relative(&config, &file)?)?;
                        i32::from(storage.remove(&path).is_ok())
                    }
                    _ => {
                        let bytes = program_io::read_raw(
                            storage.as_ref(),
                            client,
                            &config,
                            &archive,
                            &file,
                        )?;
                        if id == 0x34 {
                            i32::from(bytes.is_some())
                        } else {
                            let bytes = bytes.map(program_io::decode).transpose()?;
                            match (id, bytes) {
                                (0x35, bytes) => bytes.map_or(0, |bytes| bytes.len() as i32),
                                (0x30, Some(bytes)) => {
                                    writes.push(HostMemoryWrite {
                                        address: destination,
                                        bytes: vec![0; 16],
                                    });
                                    let length = bytes.len() as i32;
                                    writes.push(HostMemoryWrite {
                                        address: destination,
                                        bytes,
                                    });
                                    length
                                }
                                (0x30, None) => {
                                    bail!("BGI required file is missing: {archive}:{file}")
                                }
                                (0x31, None) => 1,
                                (0x31, Some(bytes)) => {
                                    let (offset, count) = range.unwrap();
                                    let count = if offset == 0 && count == 0 {
                                        bytes.len()
                                    } else {
                                        count
                                    };
                                    if count == 0 || count > bytes.len() {
                                        3
                                    } else if let Some(data) = offset
                                        .checked_add(count)
                                        .and_then(|end| bytes.get(offset..end))
                                    {
                                        writes.push(HostMemoryWrite {
                                            address: destination,
                                            bytes: data.to_vec(),
                                        });
                                        0
                                    } else {
                                        2
                                    }
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                };
                cancel.check()?;
                Ok(FileReply {
                    outputs: vec![result],
                    writes,
                })
            })?);
        }
        let Some(reply) = self.file_load.as_mut().unwrap().try_take()? else {
            return Ok(false);
        };
        vm.commit_call_writes(&reply.outputs, reply.writes)?;
        self.file_load = None;
        self.calls += 1;
        Ok(true)
    }
}
