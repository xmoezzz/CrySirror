//! Native spline IDs and BWEF guest tables over engine math/resource services.
use super::*;
use crysirror_bgi_vm::HostMemoryWrite;
use crysirror_scene::animation::NaturalCubicPath;
#[derive(Default)]
pub(crate) struct EffectState {
    next_spline: i32,
    splines: BTreeMap<i32, Spline>,
    jobs: BTreeMap<u64, Task<Option<Vec<u8>>>>,
}
struct Spline {
    duration: i32,
    path: Option<NaturalCubicPath>,
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    group == 0xc0 && matches!(id, 0xc0..=0xc3 | 0xf0)
}
impl BgiVm {
    pub(crate) fn effect_spline_handles(&self) -> Vec<i32> {
        self.effects.splines.keys().copied().collect()
    }
    pub(crate) fn effect_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        let mut writes = Vec::new();
        let status = match call.id {
            0xc0 => {
                ensure!(
                    self.effects.splines.len() < 65536,
                    "BGI spline handle budget"
                );
                let handle = i32::MIN.wrapping_add(self.effects.next_spline);
                self.effects.next_spline = self.effects.next_spline.wrapping_add(1);
                ensure!(
                    !self.effects.splines.contains_key(&handle),
                    "BGI spline handle collision"
                );
                self.effects.splines.insert(
                    handle,
                    Spline {
                        duration: 0,
                        path: None,
                    },
                );
                handle
            }
            0xc1 => i32::from(self.effects.splines.remove(&a[0]).is_none()),
            0xc2 => {
                if (a[1] as u32) < 2 {
                    2
                } else if (a[3] as u32) < 2 {
                    3
                } else if !self.effects.splines.contains_key(&a[0]) {
                    1
                } else {
                    let count = (a[1] as u32).min(100) as usize;
                    let bytes = vm.memory(a[2] as u32, count * 16)?;
                    let points = bytes
                        .chunks_exact(16)
                        .map(|record| {
                            std::array::from_fn(|i| {
                                i32::from_le_bytes(record[i * 4..i * 4 + 4].try_into().unwrap())
                            })
                        })
                        .collect::<Vec<[i32; 3]>>();
                    if a[3] < 2 {
                        3
                    } else {
                        self.effects.splines.insert(
                            a[0],
                            Spline {
                                duration: a[3],
                                path: Some(NaturalCubicPath::new(&points)?),
                            },
                        );
                        0
                    }
                }
            }
            0xc3 => {
                if let Some(spline) = self.effects.splines.get(&a[1]) {
                    if a[2] < 0 || a[2] >= spline.duration {
                        4
                    } else if let Some(path) = &spline.path {
                        let point = path.sample(f64::from(a[2]) / f64::from(spline.duration))?;
                        writes.push(HostMemoryWrite {
                            address: a[0] as u32,
                            bytes: point.into_iter().flat_map(i32::to_le_bytes).collect(),
                        });
                        0
                    } else {
                        -1
                    }
                } else {
                    1
                }
            }
            0xf0 => {
                let mut task = if let Some(task) = self.effects.jobs.remove(&request) {
                    task
                } else {
                    let archive = vm.string(a[2])?;
                    let file = vm.string(a[3])?;
                    let storage = host.storage(self.client)?;
                    let config = self.config.clone();
                    let client = self.client;
                    self.pool.spawn(move |cancel| {
                        cancel.check()?;
                        program_io::read_raw(storage.as_ref(), client, &config, &archive, &file)?
                            .map(program_io::decode)
                            .transpose()
                    })?
                };
                let Some(bytes) = task.try_take()? else {
                    self.effects.jobs.insert(request, task);
                    return Ok(false);
                };
                match bytes {
                    None => i32::MIN + 1,
                    Some(bytes) => match crysirror_resource::bwef::OffsetTable::parse(&bytes) {
                        Err(crysirror_resource::bwef::ParseError::Header) => i32::MIN + 2,
                        Err(crysirror_resource::bwef::ParseError::Length) => i32::MIN + 3,
                        Ok(table) => {
                            writes.push(HostMemoryWrite {
                                address: a[0] as u32,
                                bytes: table.relocate(a[4] as u32),
                            });
                            writes.push(HostMemoryWrite {
                                address: a[1] as u32,
                                bytes: (table.offsets.len() as u32).to_le_bytes().to_vec(),
                            });
                            0
                        }
                    },
                }
            }
            _ => bail!("BGI native effect dispatch"),
        };
        vm.update_spline_handles(self.effect_spline_handles());
        vm.commit_call_writes(&[status], writes)?;
        self.calls += 1;
        Ok(true)
    }
}
