//! Native movie transport and BF handle policy over CrySirror media/image jobs.
use super::*;
use crate::media_registry::BurikoMovieRegistry;
use crysirror_bgi_vm::{
    HostMemoryWrite,
    native_call::{NativeProcedureClass, NativeProcedureCompletion},
};
use crysirror_core::{
    frame_image::{EncodedFrameFormat, EncodedFrameSpec},
    image_processing::{ImageJob, ImageProcessState},
    service::{ClientStorage, MediaPhase, MediaRequest},
};
const GLOBAL_MOVIE: u32 = 1023;
const GLOBAL_LAYER: u32 = 0xbf00_0001;
#[derive(Default)]
pub(crate) struct MediaState {
    registry: BurikoMovieRegistry,
    jobs: BTreeMap<u64, MediaWork>,
    movies: BTreeMap<i32, Movie>,
    global: Option<Movie>,
    global_volume: Option<i32>,
    global_layer: bool,
    global_muted: bool,
    global_opening: Option<u64>,
}
struct Movie {
    id: u32,
    asset: String,
    started: bool,
    paused: bool,
    volume: i32,
    frame: u64,
}
enum MediaWork {
    BfLoad(Task<Option<Vec<u8>>>),
    BfDecode {
        job: ImageJob,
        bitmap: u32,
        format: i32,
        auxiliary: [i32; 2],
    },
    Resolve(Task<Option<String>>),
    Opening {
        slot: Option<i32>,
        movie: Movie,
        allocate: bool,
    },
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    matches!((group,id),(0x90,0xf0..=0xf7)|(0x91,0xf0..=0xf7)|(0x92,0xf0..=0xf2|0xf4..=0xf6))
}
fn dword(address: i32, value: i32) -> HostMemoryWrite {
    HostMemoryWrite {
        address: address as u32,
        bytes: value.to_le_bytes().to_vec(),
    }
}
fn completion(class: NativeProcedureClass, status: i32) -> NativeProcedureCompletion {
    NativeProcedureCompletion {
        class,
        status,
        outputs: [0; 2],
        output_count: 0,
    }
}
fn complete_bf_frame(vm: &mut Machine, status: i32) -> Result<()> {
    if vm.native_decode_async() {
        vm.commit_call_writes_with_completion(
            &[],
            Vec::new(),
            NativeProcedureCompletion {
                class: NativeProcedureClass::DecodeBurikoMovie,
                status,
                outputs: [status, 0],
                output_count: 1,
            },
        )?;
    } else {
        vm.commit_call(&[status])?;
    }
    Ok(())
}
impl BgiVm {
    fn complete_bf_load(
        &mut self,
        vm: &mut Machine,
        call: &NativeCall,
        bytes: Option<Vec<u8>>,
    ) -> Result<()> {
        let class = if call.group == 0x92 && call.args[4] != 0 {
            NativeProcedureClass::LoadBurikoMovieHeader
        } else {
            NativeProcedureClass::LoadBurikoMovie
        };
        let archive = if call.args[2] == 0 {
            String::new()
        } else {
            vm.string(call.args[2])?
        };
        let resource = vm.string(call.args[3])?;
        let result = bytes
            .ok_or(1)
            .and_then(|bytes| self.media.registry.load(archive, resource, bytes));
        let (status, writes) = match result {
            Ok((handle, info)) => {
                let mut writes = vec![dword(call.args[0], handle)];
                for (index, value) in info.into_iter().enumerate() {
                    writes.push(dword(
                        (call.args[1] as u32).wrapping_add(index as u32 * 4) as i32,
                        value,
                    ));
                }
                (0, writes)
            }
            Err(status) => (status, Vec::new()),
        };
        vm.commit_call_writes_with_completion(&[], writes, completion(class, status))?;
        self.calls += 1;
        Ok(())
    }
    pub(crate) fn media_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        match (call.group, call.id) {
            (0x90, 0xf4) | (0x92, 0xf1) => {
                let work = if let Some(work) = self.media.jobs.remove(&request) {
                    work
                } else {
                    let archive = if a[2] == 0 {
                        String::new()
                    } else {
                        vm.string(a[2])?
                    };
                    let file = vm.string(a[3])?;
                    let storage = host.storage(self.client)?;
                    let config = self.config.clone();
                    let client = self.client;
                    MediaWork::BfLoad(self.pool.spawn(move |cancel| {
                        cancel.check()?;
                        let bytes = program_io::read_raw(
                            storage.as_ref(),
                            client,
                            &config,
                            &archive,
                            &file,
                        )?
                        .map(program_io::decode)
                        .transpose()?;
                        cancel.check()?;
                        Ok(bytes)
                    })?)
                };
                let MediaWork::BfLoad(mut task) = work else {
                    bail!("BGI BF load continuation kind")
                };
                let Some(bytes) = task.try_take()? else {
                    self.media.jobs.insert(request, MediaWork::BfLoad(task));
                    return Ok(false);
                };
                self.complete_bf_load(vm, call, bytes)?;
                return Ok(true);
            }
            (0x90, 0xf5) => {
                let status = self.media.registry.release(a[0]);
                vm.commit_call(&[status])?;
            }
            (0x90, 0xf7) => match self.media.registry.attach(a[1]) {
                Ok(handle) => vm.commit_call_writes(&[0], vec![dword(a[0], handle)])?,
                Err(status) => vm.commit_call(&[status])?,
            },
            (0x90, 0xf6) => {
                let work = if let Some(work) = self.media.jobs.remove(&request) {
                    work
                } else {
                    let source = self.media.registry.resource(a[1]);
                    let status = match source {
                        None => 3,
                        Some(source) if a[2] < 0 || a[2] >= source.metadata[4].max(0) => 4,
                        Some(source) => match self.bitmaps.get(&(a[0] as u32)) {
                            Some(bitmap)
                                if bitmap.size
                                    == [source.metadata[0] as u32, source.metadata[1] as u32]
                                    && bitmap.format == source.metadata[2] =>
                            {
                                0
                            }
                            _ => 5,
                        },
                    };
                    if status != 0 {
                        complete_bf_frame(vm, status)?;
                        self.calls += 1;
                        return Ok(true);
                    }
                    let source = source.unwrap();
                    let bitmap = self.bitmaps[&(a[0] as u32)].clone();
                    let Some(destination) = bitmap.image else {
                        complete_bf_frame(vm, 6)?;
                        self.calls += 1;
                        return Ok(true);
                    };
                    let job = host.image_metadata(self.client)?.begin_encoded_frame(
                        EncodedFrameSpec {
                            format: EncodedFrameFormat::BfMovie,
                            bytes: source.bytes.clone(),
                            frame: a[2] as u32,
                            destination,
                        },
                    )?;
                    MediaWork::BfDecode {
                        job,
                        bitmap: a[0] as u32,
                        format: bitmap.format,
                        auxiliary: bitmap.auxiliary,
                    }
                };
                let MediaWork::BfDecode {
                    mut job,
                    bitmap,
                    format,
                    auxiliary,
                } = work
                else {
                    bail!("BGI BF frame continuation kind")
                };
                let status = match job.poll()? {
                    ImageProcessState::Pending => {
                        self.media.jobs.insert(
                            request,
                            MediaWork::BfDecode {
                                job,
                                bitmap,
                                format,
                                auxiliary,
                            },
                        );
                        return Ok(false);
                    }
                    ImageProcessState::Failed(_) => 6,
                    ImageProcessState::Ready(image) => {
                        self.publish_bitmap(host, bitmap, format, auxiliary, image.clone(), image)?;
                        0
                    }
                };
                // The VM decides whether its one-shot decode flag installs a
                // DCProcDecodeBMV; the same native status is retained in either case.
                complete_bf_frame(vm, status)?;
            }
            _ => return self.standard_movie_call(host, vm, call, request),
        }
        self.calls += 1;
        Ok(true)
    }
}

/// Resolve an archive member as a live capability reader, without unpacking or
/// reading the movie into adapter memory. Archives already mounted by detection
/// participate in the same native loose-first search order.
fn movie_asset(
    storage: &dyn ClientStorage,
    config: &Config,
    archive: &str,
    file: &str,
) -> Result<Option<String>> {
    let file = program_io::relative(config, file)?;
    let mut candidates = vec![config.asset(&file)?];
    if archive.is_empty() {
        match storage.list(Mount::Asset, "__bgi") {
            Ok(mounts) => {
                for mount in mounts {
                    candidates.push(format!("__bgi/{mount}/{file}"));
                }
            }
            Err(Error::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        let source = config.asset(archive)?;
        for source in [source.clone(), format!("{source}.arc")] {
            if let Some(point) = storage.container_mount(&source)? {
                candidates.push(format!("{point}/{file}"));
            }
        }
    }
    for candidate in candidates {
        let candidate = VfsPath::new(Mount::Asset, &candidate)?;
        let resolved = match storage.metadata(Mount::Asset, candidate.relative()) {
            Ok(Some(info)) => {
                if info.is_directory {
                    None
                } else {
                    Some(candidate.clone())
                }
            }
            Ok(None) | Err(Error::NotFound) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(path) = resolved {
            return Ok(Some(path.relative().into()));
        }
        match crysirror_resource::capability_lookup::resolve_ascii_case_insensitive(
            storage, &candidate,
        ) {
            Ok(Some(path)) => {
                if storage
                    .metadata(Mount::Asset, path.relative())?
                    .is_some_and(|m| !m.is_directory)
                {
                    return Ok(Some(path.relative().into()));
                }
            }
            Ok(None) => {}
            Err(error) if matches!(error.downcast_ref::<Error>(), Some(Error::NotFound)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}
impl BgiVm {
    fn movie_slot_status(&self, slot: i32) -> i32 {
        if !(0..0x4000).contains(&slot) || !self.bitmaps.contains_key(&(slot as u32)) {
            4
        } else if !self.media.movies.contains_key(&slot) {
            1
        } else {
            0
        }
    }
    fn allocate_media_id(&self) -> Result<u32> {
        (0..GLOBAL_MOVIE)
            .find(|id| {
                !self.media.movies.values().any(|m| m.id == *id)
                    && !self
                        .media
                        .jobs
                        .values()
                        .any(|work| matches!(work,MediaWork::Opening{movie,..}if movie.id==*id))
            })
            .context("BGI shared media identities exhausted")
    }
    fn close_global_movie(&mut self, host: &mut dyn HostServices) -> Result<()> {
        if let Some(movie) = self.media.global.take() {
            host.media_request(self.client, MediaRequest::Remove { id: movie.id })?;
        }
        // The layer exists only after the media open reached readiness.
        if self.media.global_layer {
            self.submit(host, json!({"layers":[{"op":"remove","id":GLOBAL_LAYER}]}))?;
            self.media.global_layer = false;
        }
        Ok(())
    }
    fn poll_movie_open(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        let global = call.id == 0xf0 && call.group != 0x91;
        if global {
            if self
                .media
                .global_opening
                .is_some_and(|owner| owner != request)
            {
                return Ok(false);
            }
            self.media.global_opening = Some(request);
        }
        let allocate = call.group == 0x92 && call.id == 0xf2;
        let slot = (!global).then_some(a[0]);
        let volume = if global {
            self.media.global_volume.unwrap_or(128)
        } else {
            a[if allocate { 4 } else { 3 }]
        };
        if let Some(slot) = slot {
            let status = if !(0..0x4000).contains(&slot) {
                4
            } else if !(0..=128).contains(&volume) {
                3
            } else {
                0
            };
            if status != 0 {
                vm.commit_call(&[status])?;
                self.calls += 1;
                return Ok(true);
            }
        } else {
            let (w, h) = if call.group == 0x90 {
                (a[3], a[4])
            } else {
                (a[4], a[5])
            };
            ensure!(w > 0 && h > 0, "BGI invalid movie window size [{w}, {h}]");
        }
        let work = if let Some(work) = self.media.jobs.remove(&request) {
            work
        } else {
            let (archive, file) = if global && call.group == 0x90 {
                (String::new(), vm.string(a[0])?)
            } else if global {
                (
                    if a[0] == 0 {
                        String::new()
                    } else {
                        vm.string(a[0])?
                    },
                    vm.string(a[1])?,
                )
            } else if allocate {
                (
                    if a[1] == 0 {
                        String::new()
                    } else {
                        vm.string(a[1])?
                    },
                    vm.string(a[2])?,
                )
            } else {
                (String::new(), vm.string(a[1])?)
            };
            let storage = host.storage(self.client)?;
            let config = self.config.clone();
            MediaWork::Resolve(self.pool.spawn(move |cancel| {
                cancel.check()?;
                let path = movie_asset(storage.as_ref(), &config, &archive, &file)?;
                cancel.check()?;
                Ok(path)
            })?)
        };
        let work = match work {
            MediaWork::Resolve(mut task) => {
                let Some(asset) = task.try_take()? else {
                    self.media.jobs.insert(request, MediaWork::Resolve(task));
                    return Ok(false);
                };
                let Some(asset) = asset else {
                    if global && call.group == 0x90 {
                        bail!("BGI required movie is missing");
                    }
                    if global {
                        self.media.global_opening = None;
                    }
                    vm.commit_call(&[if global { 0 } else { 2 }])?;
                    self.calls += 1;
                    return Ok(true);
                };
                if global {
                    self.close_global_movie(host)?;
                } else if let Some(previous) = self.media.movies.remove(&slot.unwrap()) {
                    host.media_request(self.client, MediaRequest::Remove { id: previous.id })?;
                }
                let id = if global {
                    GLOBAL_MOVIE
                } else {
                    self.allocate_media_id()?
                };
                let looping = !global && a[if allocate { 3 } else { 2 }] != 0;
                let gain = if global && host.window_minimized(self.client)?.unwrap_or(false) {
                    0.0
                } else {
                    volume as f32 / 128.0
                };
                match host.media_request(
                    self.client,
                    MediaRequest::Play {
                        id,
                        asset: asset.clone(),
                        looping,
                        audio: true,
                        gain,
                    },
                ) {
                    Ok(_) => {}
                    Err(error @ Error::Unsupported(_)) => return Err(error.into()),
                    Err(_) => {
                        if global {
                            self.media.global_opening = None;
                        }
                        vm.commit_call(&[if global { 0 } else { 1 }])?;
                        self.calls += 1;
                        return Ok(true);
                    }
                }
                if !global {
                    host.media_request(self.client, MediaRequest::Pause { id })?;
                }
                MediaWork::Opening {
                    slot,
                    movie: Movie {
                        id,
                        asset,
                        started: global,
                        paused: !global,
                        volume,
                        frame: 0,
                    },
                    allocate,
                }
            }
            other => other,
        };
        let MediaWork::Opening {
            slot,
            movie,
            allocate,
        } = work
        else {
            bail!("BGI movie open continuation kind")
        };
        let info = host.media_info(self.client, movie.id)?;
        if info.phase == MediaPhase::Failed {
            host.media_request(self.client, MediaRequest::Remove { id: movie.id })?;
            if global {
                self.media.global_opening = None;
            }
            vm.commit_call(&[if global { 0 } else { 1 }])?;
            self.calls += 1;
            return Ok(true);
        }
        if info.phase == MediaPhase::Loading || info.size.is_none() {
            self.media.jobs.insert(
                request,
                MediaWork::Opening {
                    slot,
                    movie,
                    allocate,
                },
            );
            return Ok(false);
        }
        if global {
            let path = VfsPath::new(
                Mount::Asset,
                &format!("__surface/{}", 0x7100_0000 + movie.id),
            )?;
            let screen = self.graph.screen;
            self.submit(host,json!({"layers":[{"op":"put","layer":{"id":GLOBAL_LAYER,"rect":[0,0,screen[0],screen[1]],"order":[2147483600i32,0],"paint":{"kind":"image","asset":path.relative()}}}]}))?;
            self.media.global_layer = true;
            self.media.global_muted = host.window_minimized(self.client)?.unwrap_or(false);
            self.media.global = Some(movie);
            self.media.global_opening = None;
            vm.commit_call(&[info.duration_ms.unwrap_or(0).min(i32::MAX as u64) as i32])?;
        } else {
            let slot = slot.unwrap();
            // Graph92:F2 prepares a real first picture/bitmap before returning;
            // Graph91:F0 only prepares transport and leaves allocation alone.
            if allocate {
                let Some(image) = host.media_image(self.client, movie.id)? else {
                    self.media.jobs.insert(
                        request,
                        MediaWork::Opening {
                            slot: Some(slot),
                            movie,
                            allocate,
                        },
                    );
                    return Ok(false);
                };
                self.publish_bitmap_reallocated(
                    host,
                    slot as u32,
                    1,
                    [0; 2],
                    image.clone(),
                    image,
                )?;
            }
            self.media.movies.insert(slot, movie);
            vm.commit_call(&[0])?;
        }
        self.calls += 1;
        Ok(true)
    }
    fn standard_movie_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
        request: u64,
    ) -> Result<bool> {
        let a = &call.args;
        let mut writes = Vec::new();
        let mut out = Vec::new();
        match (call.group, call.id) {
            (0x90, 0xf0) | (0x91, 0xf0) | (0x92, 0xf0 | 0xf2) => {
                return self.poll_movie_open(host, vm, call, request);
            }
            (0x90, 0xf1) => {
                if self.media.global_opening.is_some() {
                    return Ok(false);
                }
                self.close_global_movie(host)?;
            }
            (0x90, 0xf2) => {
                let playing = if let Some(movie) = &self.media.global {
                    let info = host.media_info(self.client, movie.id)?;
                    matches!(info.phase, MediaPhase::Loading | MediaPhase::Playing)
                        && info.duration_ms.is_none_or(|end| info.position_ms < end)
                } else {
                    false
                };
                out.push(i32::from(playing));
            }
            (0x90, 0xf3) => {
                if (0..=128).contains(&a[0]) {
                    self.media.global_volume = Some(a[0]);
                    if let Some(movie) = self.media.global.as_mut() {
                        movie.volume = a[0];
                        if !host.window_minimized(self.client)?.unwrap_or(false) {
                            host.media_request(
                                self.client,
                                MediaRequest::SetGain {
                                    id: movie.id,
                                    gain: a[0] as f32 / 128.0,
                                },
                            )?;
                        }
                    }
                }
            }
            (0x91, 0xf1) => {
                let status = self.movie_slot_status(a[1]);
                out.push(status);
                if status == 0 {
                    let movie = self.media.movies.get_mut(&a[1]).unwrap();
                    host.media_request(
                        self.client,
                        MediaRequest::Seek {
                            id: movie.id,
                            position_ms: 0,
                        },
                    )?;
                    host.media_request(self.client, MediaRequest::Resume { id: movie.id })?;
                    movie.started = true;
                    movie.paused = false;
                    movie.frame = 0;
                    let info = host.media_info(self.client, movie.id)?;
                    writes.push(dword(
                        a[0],
                        info.duration_ms.unwrap_or(0).min(i32::MAX as u64) as i32,
                    ));
                }
            }
            (0x91, 0xf2) => {
                let status = self.movie_slot_status(a[0]);
                out.push(status);
                if status == 0 {
                    let movie = self.media.movies.remove(&a[0]).unwrap();
                    host.media_request(self.client, MediaRequest::Remove { id: movie.id })?;
                }
            }
            (0x91, 0xf3) => {
                let status = self.movie_slot_status(a[0]);
                out.push(status);
                if status == 0 {
                    let movie = self.media.movies.get_mut(&a[0]).unwrap();
                    if movie.started {
                        host.media_request(
                            self.client,
                            if a[1] != 0 {
                                MediaRequest::Pause { id: movie.id }
                            } else {
                                MediaRequest::Resume { id: movie.id }
                            },
                        )?;
                    }
                    movie.paused = a[1] != 0;
                }
            }
            (0x91, 0xf4..=0xf6) => {
                bail!("BGI Flash control requires a shared SWF playback and rendering capability")
            }
            (0x91, 0xf7) | (0x92, 0xf5) => {
                let valid =
                    (0..0x4000).contains(&a[1]) && self.bitmaps.contains_key(&(a[1] as u32));
                if call.group == 0x91 {
                    out.push(i32::from(valid));
                    if valid {
                        let position = if let Some(movie) = self.media.movies.get(&a[1]) {
                            if movie.started && !movie.paused {
                                let info = host.media_info(self.client, movie.id)?;
                                if matches!(info.phase, MediaPhase::Ended | MediaPhase::Stopped) {
                                    -1
                                } else {
                                    info.position_ms.min(i32::MAX as u64) as i32
                                }
                            } else {
                                -1
                            }
                        } else {
                            -1
                        };
                        writes.push(dword(a[0], position));
                    }
                } else {
                    let status = self.movie_slot_status(a[1]);
                    out.push(status);
                    if status == 0 {
                        let movie = &self.media.movies[&a[1]];
                        let position = host
                            .media_info(self.client, movie.id)?
                            .position_ms
                            .min(i32::MAX as u64) as i32;
                        writes.push(dword(a[0], position));
                    }
                }
            }
            (0x92, 0xf4) => {
                let status = self.movie_slot_status(a[0]);
                if status != 0 {
                    out.push(status);
                } else if a[1] < 0 {
                    out.push(5);
                } else {
                    let movie = &self.media.movies[&a[0]];
                    out.push(
                        if host
                            .media_request(
                                self.client,
                                MediaRequest::Seek {
                                    id: movie.id,
                                    position_ms: a[1] as u64,
                                },
                            )
                            .is_ok()
                        {
                            0
                        } else {
                            5
                        },
                    );
                }
            }
            (0x92, 0xf6) => {
                let status = self.movie_slot_status(a[0]);
                if status != 0 {
                    out.push(status);
                } else if !(0..=128).contains(&a[1]) {
                    out.push(3);
                } else {
                    let movie = self.media.movies.get_mut(&a[0]).unwrap();
                    host.media_request(
                        self.client,
                        MediaRequest::SetGain {
                            id: movie.id,
                            gain: a[1] as f32 / 128.0,
                        },
                    )?;
                    movie.volume = a[1];
                    out.push(0);
                }
            }
            _ => bail!("BGI native movie {:02x}:{:02x}", call.group, call.id),
        }
        ensure!(out.len() == call.outputs, "BGI movie output count");
        vm.commit_call_writes(&out, writes)?;
        self.calls += 1;
        Ok(true)
    }
    pub(crate) fn tick_media(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let minimized = host.window_minimized(self.client)?.unwrap_or(false);
        if let Some(movie) = &self.media.global {
            ensure!(
                host.media_info(self.client, movie.id)?.phase != MediaPhase::Failed,
                "BGI global media playback failed: {}",
                movie.asset
            );
            if minimized != self.media.global_muted {
                host.media_request(
                    self.client,
                    MediaRequest::SetGain {
                        id: movie.id,
                        gain: if minimized {
                            0.0
                        } else {
                            movie.volume as f32 / 128.0
                        },
                    },
                )?;
            }
        }
        self.media.global_muted = minimized;
        let mut frames = Vec::new();
        for (&slot, movie) in &mut self.media.movies {
            if !movie.started || self.resource_fences.contains_key(&(slot as u32)) {
                continue;
            }
            let info = host.media_info(self.client, movie.id)?;
            ensure!(
                info.phase != MediaPhase::Failed,
                "BGI bitmap media playback failed: {}",
                movie.asset
            );
            if info.published_frames == movie.frame {
                continue;
            }
            if let Some(image) = host.media_image(self.client, movie.id)? {
                frames.push((slot, image));
                movie.frame = info.published_frames;
            }
        }
        for (slot, image) in frames {
            self.publish_bitmap(host, slot as u32, 1, [0; 2], image.clone(), image)?;
        }
        Ok(())
    }
}
impl BgiVm {
    /// Original native process observations: handle, result, duration, position,
    /// active. Opening a transport leaves result -1 until its real EOF.
    pub(crate) fn media_process_snapshots(
        &self,
        host: &dyn HostServices,
    ) -> Result<Vec<(i32, i32, i32, i32, bool, bool)>> {
        self.media
            .movies
            .iter()
            .map(|(&handle, movie)| {
                let info = host.media_info(self.client, movie.id)?;
                let ended = matches!(
                    info.phase,
                    MediaPhase::Ended | MediaPhase::Stopped | MediaPhase::Failed
                );
                Ok((
                    handle,
                    if ended { 0 } else { -1 },
                    info.duration_ms.unwrap_or(0).min(i32::MAX as u64) as i32,
                    info.position_ms.min(i32::MAX as u64) as i32,
                    movie.started && !ended,
                    movie.paused,
                ))
            })
            .collect()
    }
}
