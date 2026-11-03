//! BGI native BP semantic adapter over CrySirror resources, Canvas and Layer.
//! No pixel buffer, rasterizer, font algorithm or audio mixer lives here.
#![forbid(unsafe_code)]
mod audio_api;
mod audio_cd;
mod bitmap_api;
mod bitmap_checks;
mod bitmap_codec;
mod bitmap_fields;
mod bitmap_memory;
mod display_tree;
mod effect_api;
mod file_api;
pub mod fixture;
mod graph_api;
mod graph_controls;
mod graph_state;
mod input_api;
mod input_policy;
mod media_api;
mod media_registry;
mod message_api;
mod native_background;
mod native_display;
mod platform_api;
mod program_io;
mod save_api;
mod selection_api;
mod system_api;
mod system_files;
mod target_checks;
mod text_api;
mod text_policy;
mod user_api;
mod user_dialog_api;
mod window_api;
use anyhow::{Context, Result, bail, ensure};
use crysirror_bgi_vm::{Machine, NativeCall, Step};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
use crysirror_runtime::{AdapterRegistry, model::ClientSpec, runtime::VmFactory};
use crysirror_tasks::{Task, TaskPool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Capability-relative game root. Empty selects the client's own root.
    #[serde(default)]
    pub root: String,
    /// Native BP module. Buriko scenario scripts are a separate unsupported ABI.
    pub script: String,
    /// Project font alias for native font slot zero in the plain text profile.
    #[serde(default)]
    pub font: Option<String>,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if let Some(font) = &self.font {
            ensure!(
                !font.is_empty() && font.len() <= 256 && !font.chars().any(char::is_control),
                "BGI project font alias"
            );
        }
        ensure!(
            self.script.to_ascii_lowercase().ends_with("._bp"),
            "BGI profile requires a native ._bp module"
        );
        let script_parts = self.configured_script();
        for name in [script_parts.1]
            .into_iter()
            .chain((!script_parts.0.is_empty()).then_some(script_parts.0))
            .chain((!self.root.is_empty()).then_some(self.root.as_str()))
        {
            ensure!(
                VfsPath::new(Mount::Asset, name)?.relative() == name,
                "noncanonical BGI path"
            );
        }
        Ok(())
    }
    fn configured_script(&self) -> (&str, &str) {
        self.script.split_once(':').unwrap_or(("", &self.script))
    }
    fn asset(&self, name: &str) -> Result<String> {
        let name = name.replace('\\', "/");
        let full = if self.root.is_empty() {
            name
        } else {
            format!("{}/{}", self.root, name)
        };
        Ok(VfsPath::new(Mount::Asset, &full)?.relative().into())
    }
}
struct Factory;
impl VmFactory for Factory {
    fn create(&self, client: ClientId, spec: &ClientSpec) -> Result<Box<dyn VmAdapter>> {
        ensure!(
            spec.program.is_empty(),
            "BGI runs native BP bytecode, not data-VM actions"
        );
        Ok(Box::new(BgiVm::new(
            client,
            serde_json::from_value(spec.config.clone())?,
        )?))
    }
}
pub fn register(registry: &mut AdapterRegistry) -> Result<()> {
    registry.register("bgi", Arc::new(Factory))
}

#[derive(Clone)]
struct Bitmap {
    generation: u64,
    published_path: Option<String>,
    size: [u32; 2],
    format: i32,
    image: Option<Arc<dyn crysirror_core::image_processing::ImageResource>>,
    auxiliary: [i32; 2],
}
#[derive(Clone, Default)]
struct Sprite {
    bitmap: Option<u32>,
    position: [i32; 2],
    priority: i32,
    opacity: u8,
    visible: bool,
}
/// Native handles and task receipts only. Actual drawing/state belongs to engine
/// Canvas/Layer systems, and files stay behind the client storage capability.
pub struct BgiVm {
    client: ClientId,
    config: Config,
    pool: TaskPool,
    load: Option<Task<Machine>>,
    program_load: Option<Task<Vec<u8>>>,
    file_load: Option<Task<file_api::FileReply>>,
    audio: audio_api::AudioState,
    bitmap_work: Option<bitmap_api::BitmapWork>,
    memory_jobs: BTreeMap<u64, bitmap_memory::MemoryWork>,
    field_memory_jobs: BTreeMap<u64, bitmap_fields::FieldMemoryWork>,
    codec: bitmap_codec::CodecState,
    graph: graph_api::GraphState,
    controls: graph_controls::ControlState,
    effects: effect_api::EffectState,
    resource_fences: BTreeMap<u32, u64>,
    audio_fences: BTreeMap<u32, u64>,
    save_jobs: BTreeMap<u64, save_api::SaveWork>,
    dialog_work: Option<user_dialog_api::DialogWork>,
    desktop_jobs: BTreeMap<u64, platform_api::PlatformWork>,
    user: user_api::UserState,
    system: system_api::SystemState,
    windows: window_api::WindowState,
    messages: message_api::MessageState,
    media: media_api::MediaState,
    selections: selection_api::SelectionState,
    text: text_api::TextState,
    text_jobs: BTreeMap<u64, text_api::TextWork>,
    color_luts: BTreeMap<i32, [[i32; 2]; 3]>,
    bitmap_jobs: BTreeMap<u64, bitmap_api::BitmapWork>,
    file_jobs: BTreeMap<u64, Task<file_api::FileReply>>,
    program_jobs: BTreeMap<u64, Task<Vec<u8>>>,
    vm: Option<Machine>,
    bitmaps: BTreeMap<u32, Bitmap>,
    bitmap_serial: u64,
    sprites: BTreeMap<u32, Sprite>,
    timer: u64,
    wait: Option<VmYield>,
    calls: u64,
    input: InputFrame,
}
impl BgiVm {
    pub fn new(client: ClientId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client,
            config,
            pool: TaskPool::new("bgi-resource", 2, 128)?,
            load: None,
            program_load: None,
            file_load: None,
            audio: Default::default(),
            bitmap_work: None,
            memory_jobs: BTreeMap::new(),
            field_memory_jobs: BTreeMap::new(),
            codec: Default::default(),
            graph: Default::default(),
            controls: Default::default(),
            effects: Default::default(),
            resource_fences: BTreeMap::new(),
            audio_fences: BTreeMap::new(),
            save_jobs: BTreeMap::new(),
            dialog_work: None,
            desktop_jobs: BTreeMap::new(),
            user: Default::default(),
            system: Default::default(),
            windows: Default::default(),
            messages: Default::default(),
            media: Default::default(),
            selections: Default::default(),
            text: Default::default(),
            text_jobs: BTreeMap::new(),
            color_luts: BTreeMap::new(),
            bitmap_jobs: BTreeMap::new(),
            file_jobs: BTreeMap::new(),
            program_jobs: BTreeMap::new(),
            vm: None,
            bitmaps: BTreeMap::new(),
            bitmap_serial: 0,
            sprites: BTreeMap::new(),
            timer: 0,
            wait: None,
            calls: 0,
            input: InputFrame::default(),
        })
    }
    fn call_bitmap_dependencies(call: &NativeCall) -> Vec<u32> {
        let a = &call.args;
        let indices: &[usize] = match (call.group, call.id) {
            (0x90, 0x10..=0x13) | (0x92, 0x9c) | (0x91, 0x9c | 0x9d) => &[0],
            (0x90, 0xc2 | 0xc3 | 0xca | 0xcd) | (0x91, 0x18..=0x1f) => &[0, 1],
            (0x90, 0xc8) => &[0, 3],
            (0x90, 0x56 | 0x59) => &[3],
            (0x90, 0x57) => &[1],
            (0x90, 0x04 | 0x17 | 0x83 | 0xc0 | 0xc7) => &[0],
            (0x90, 0xce) => &[2],
            (0x90, 0xf6) => &[0],
            (0x91, 0xf0 | 0xf2 | 0xf3) | (0x92, 0xf2 | 0xf4 | 0xf6) => &[0],
            (0x91, 0xf1 | 0xf7) | (0x92, 0xf5) => &[1],
            (0x92, 0x89 | 0x8d) => &[0, 3],
            (0x90, 0x18 | 0x1e) => &[0, 3],
            (0x90, 0x19) | (0x92, 0x1a) => &[0, 3, 4],
            (0x90, 0x1a) => &[0, 1, 2, 3],
            (0x90, 0x1b | 0x1d) => &[0, 1],
            (0x90, 0x1c) => &[0, 5],
            (0x90, 0x1f) => &[0, 1],
            (0x90, 0x14) => &[0],
            (0x90, 0x15) => &[3],
            (0x90, 0x16) | (0x92, 0x16 | 0x17) => &[1],
            (0x92, 0x12) => &[0],
            (0x90, 0x40) => &[0],
            (0xb0, 0x00) => &[2],
            _ => &[],
        };
        indices
            .iter()
            .filter_map(|&i| a.get(i))
            .filter_map(|&id| ((id as u32) < 0x4000).then_some(id as u32))
            .collect()
    }
    fn allocate_bitmap_generation(&mut self) -> Result<u64> {
        self.bitmap_serial = self
            .bitmap_serial
            .checked_add(1)
            .context("BGI bitmap generation exhausted")?;
        Ok(self.bitmap_serial)
    }
    fn submit(&self, host: &mut dyn HostServices, batch: Value) -> Result<()> {
        host.submit_engine(self.client, 1, &serde_json::to_vec(&batch)?)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    fn bitmap(&self, id: i32) -> Result<(u32, &Bitmap)> {
        ensure!((0..0x4000).contains(&id), "BGI bitmap handle");
        Ok((
            id as u32,
            self.bitmaps
                .get(&(id as u32))
                .context("unallocated BGI bitmap")?,
        ))
    }
    fn native(
        &mut self,
        host: &mut dyn HostServices,
        vm: &Machine,
        call: &NativeCall,
    ) -> Result<Option<(Vec<i32>, Option<VmYield>)>> {
        let a = &call.args;
        let mut output = Vec::new();
        let mut wait = None;
        match (call.group, call.id) {
            (0x80, 0x04) => output.push(host.now_ms() as u32 as i32),
            (0x80, 0x11) => {
                use crysirror_core::service::buttons;
                let bit = match a[0] {
                    1 => buttons::MOUSE_LEFT,
                    2 => buttons::MOUSE_RIGHT,
                    13 => buttons::ENTER,
                    32 => buttons::SPACE,
                    27 => buttons::ESCAPE,
                    37 => buttons::LEFT,
                    38 => buttons::UP,
                    39 => buttons::RIGHT,
                    40 => buttons::DOWN,
                    _ => bail!("BGI physical key descriptor {} is unmapped", a[0]),
                };
                output.push(i32::from(
                    (self.input.buttons_down | self.input.buttons_pressed) & bit != 0,
                ));
            }

            (0x80, 0x58) => {
                ensure!(a[0] >= 0, "BGI timer duration");
                self.timer = host.now_ms().saturating_add(a[0] as u64);
            }
            (0x80, 0x5a) => {
                let remaining = self.timer.saturating_sub(host.now_ms());
                output.push(i32::from(remaining > 0));
                if remaining > 0 {
                    wait = Some(VmYield::WaitUntil(self.timer));
                }
            }
            (0x80, 0x5f) => wait = Some(VmYield::WaitUntil(host.now_ms().saturating_add(1))),
            (0x80, 0x6a) => wait = Some(VmYield::Halted),
            (0x90, 0x11) => {
                let id = a[0];
                ensure!((0..0x4000).contains(&id), "BGI bitmap handle");
                let [w, h] = [u32::try_from(a[1])?, u32::try_from(a[2])?];
                ensure!(
                    w > 0 && h > 0 && w <= 8192 && h <= 8192,
                    "BGI bitmap dimensions"
                );
                let format = if a[3] == 7 { 1 } else { a[3] };
                ensure!(
                    matches!(format, 0..=3),
                    "BGI bitmap format {format} needs an engine pixel profile"
                );
                let color = if format != 2 {
                    [0, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                };
                self.submit(host,json!({"canvases":[{"op":"create","id":id,"width":w,"height":h},{"op":"clear","id":id,"color":color}]}))?;
                if let Some(old) = self
                    .bitmaps
                    .get(&(id as u32))
                    .and_then(|b| b.published_path.as_deref())
                {
                    host.release_image_resource(self.client, old)?;
                }
                let generation = self.allocate_bitmap_generation()?;
                self.release_graph_field_bitmap(id as u32);
                self.bitmaps.insert(
                    id as u32,
                    Bitmap {
                        published_path: None,
                        generation,
                        size: [w, h],
                        format,
                        image: None,
                        auxiliary: [-1, -1],
                    },
                );
            }
            (0x90, 0x13) => {
                let (id, bitmap) = self.bitmap(a[0])?;
                ensure!(
                    matches!(bitmap.format, 0..=3),
                    "BGI fill needs the decoded resource's native bitmap format metadata"
                );
                let packed = a[1] as u32;
                let rgb = [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8];
                let color = match bitmap.format {
                    0 => [rgb[0] & 0xf8, rgb[1] & 0xf8, rgb[2] & 0xf8, 255],
                    1 => [rgb[0], rgb[1], rgb[2], 255],
                    2 => [rgb[0], rgb[1], rgb[2], (packed >> 24) as u8],
                    3 => [rgb[2], 0, 0, 255],
                    _ => unreachable!(),
                };
                self.submit(
                    host,
                    json!({"canvases":[{"op":"clear","id":id,"color":color}]}),
                )?;
                let bitmap = self.bitmaps.get_mut(&id).unwrap();
                bitmap.image = None;
                if let Some(path) = bitmap.published_path.take() {
                    host.release_image_resource(self.client, &path)?;
                }
                self.invalidate_graph_bitmap(id);
            }
            (0x90, 0x40) => {
                let (id, _) = self.bitmap(a[0])?;
                self.submit(
                    host,
                    json!({"canvases":[{"op":"present","id":id,"layer":1024u32,"order":0}]}),
                )?;
            }
            (0xb0, 0x00) => {
                let (id, _) = self.bitmap(a[2])?;
                self.submit(host,json!({"canvases":[{"op":"present","id":id,"layer":1025u32,"position":[a[0],a[1]],"order":0}]}))?;
            }
            _ => bail!(
                "unmapped BGI native call {:#04x}:{:#04x}; missing CrySirror semantic mapping",
                call.group,
                call.id
            ),
        }
        ensure!(
            output.len() == call.outputs,
            "BGI adapter native result count"
        );
        self.calls += 1;
        Ok(Some((output, wait)))
    }
    fn run(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> Result<VmYield> {
        self.input = input;
        self.tick_user(host)?;
        self.audio.tick(host, self.client)?;
        self.tick_media(host)?;
        self.graph_tick(host)?;
        self.tick_window_compositions(host)?;
        if self.vm.is_none() {
            if self.load.is_none() {
                let storage = host.storage(self.client)?;
                let config = self.config.clone();
                let client = self.client;
                self.load = Some(self.pool.spawn(move |cancel| {
                    cancel.check()?;
                    let (archive, file) = config.configured_script();
                    let bytes = program_io::load(storage.clone(), client, &config, archive, file)?;
                    let profile =
                        program_io::native_profile(storage.as_ref(), client, &config, &bytes)?;
                    cancel.check()?;
                    Machine::from_bp_with_profile(&bytes, profile)
                })?);
            }
            let Some(mut vm) = self.load.as_mut().unwrap().try_take()? else {
                return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
            };
            let mut external = Vec::new();
            for group in [0x80, 0x81] {
                for id in 0..=255 {
                    if file_api::handles(group, id) || system_api::handles(group, id) {
                        external.push((group, id));
                    }
                }
            }
            vm.set_external_native_calls(external)?;
            self.vm = Some(vm);
            self.load = None;
        }
        if let Some(wait) = self.wait {
            match wait {
                VmYield::Halted => return Ok(wait),
                VmYield::WaitUntil(t) if host.now_ms() < t => return Ok(wait),
                VmYield::WaitForInput if input.buttons_pressed == 0 => return Ok(wait),
                _ => self.wait = None,
            }
        }
        let mut vm = self.vm.take().unwrap();
        use crysirror_core::service::buttons;
        let active = input.buttons_down | input.buttons_pressed;
        let held_keys = (0..256)
            .filter_map(|key| {
                crysirror_core::service::buttons::windows_virtual_key(key)
                    .filter(|bit| active & bit != 0)
                    .map(|_| key as i32)
            })
            .collect();
        vm.update_host(host.now_ms(), held_keys, [input.pointer_x, input.pointer_y]);
        vm.update_window_state(
            host.window_focused(self.client)?,
            host.window_minimized(self.client)?,
        );
        if input.buttons_pressed & buttons::WHEEL_UP != 0 {
            vm.pulse_input(14);
        }
        if input.buttons_pressed & buttons::WHEEL_DOWN != 0 {
            vm.pulse_input(15);
        }
        let result = (|| {
            let mut pending = std::collections::BTreeSet::new();
            let mut remaining = budget as usize;
            while remaining > 0 {
                if vm.halted() {
                    return Ok(VmYield::Halted);
                }
                if vm.request_id().is_some() && !vm.select_request(&pending) {
                    if !vm.advance_scheduler_with_budget(remaining)? {
                        return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
                    }
                    remaining = remaining.saturating_sub(vm.consumed_steps().max(1));
                    continue;
                }
                let desktop = host.display_info().map(|d| d.screen);
                let outer = host
                    .window_geometry(self.client)?
                    .and_then(|g| g.outer_position)
                    .unwrap_or([0; 2]);
                vm.update_display_state(
                    self.system.display_size(),
                    self.system.effective_scale(desktop),
                    desktop.unwrap_or([0; 2]),
                    outer,
                );
                vm.update_spline_handles(self.effect_spline_handles());
                vm.update_window_handles(self.windows.records.keys().copied().collect());
                vm.update_graph_input_handles(self.graph_input_handles());
                vm.update_graph_input_states(self.graph_input_states());
                self.tick_controls(host, &mut vm)?;
                vm.update_media_processes(
                    self.media_process_snapshots(host)?,
                    self.bitmaps.keys().map(|id| *id as i32).collect(),
                );
                self.tick_graph_inputs(host, &mut vm)?;
                self.tick_selection(host, &mut vm)?;
                self.tick_messages(host, &mut vm)?;
                self.refresh_native_pointer_nodes(host, &mut vm)?;
                let step = vm.step_with_budget(remaining)?;
                vm.update_graph_input_handles(self.graph_input_handles());
                vm.update_graph_input_states(self.graph_input_states());
                self.tick_controls(host, &mut vm)?;
                vm.update_media_processes(
                    self.media_process_snapshots(host)?,
                    self.bitmaps.keys().map(|id| *id as i32).collect(),
                );
                self.tick_graph_inputs(host, &mut vm)?;
                self.tick_selection(host, &mut vm)?;
                self.tick_messages(host, &mut vm)?;
                self.refresh_native_pointer_nodes(host, &mut vm)?;
                remaining = remaining.saturating_sub(vm.consumed_steps().max(1));
                match step {
                    Step::Continue => {}
                    Step::NativePlatform { request } => {
                        let id = vm.request_id().context("BGI platform request identity")?;
                        if !self.native_platform_call(host, &mut vm, id, &request)? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::DisplayAction { action } => {
                        self.display_action(host, &mut vm, action)?;
                        vm.commit_display_action()?;
                    }
                    Step::Halted => return Ok(VmYield::Halted),
                    Step::Yielded => {
                        return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
                    }
                    Step::Input => return Ok(VmYield::WaitForInput),
                    Step::Selection { id, request } => {
                        if !self.selection_call(host, &mut vm, id, request)? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::GraphInput { request } => {
                        let id = vm.request_id().context("BGI graph input identity")?;
                        if !self.graph_input_call(host, &mut vm, id, request)? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::NativeEnumerate {
                        pattern,
                        recursive,
                        directories,
                    } => {
                        let id = vm.request_id().context("BGI enumeration identity")?;
                        if !self.native_enumerate(
                            host,
                            &mut vm,
                            id,
                            &pattern,
                            recursive,
                            directories,
                        )? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::UserDialog { request } => {
                        let id = vm.request_id().context("BGI user dialog identity")?;
                        if !self.user_dialog_call(host, &mut vm, id, &request)? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::Prompt { text, default_yes } => {
                        let id = vm.request_id().context("BGI prompt identity")?;
                        if !self.prompt_call(host, &mut vm, id, &text, default_yes)? {
                            pending.insert(id);
                            vm.postpone_request();
                        }
                    }
                    Step::NativeFileRead { path } => {
                        let request = vm.request_id().context("BGI native file read identity")?;
                        if !self.native_file_read(host, &mut vm, request, &path)? {
                            pending.insert(request);
                            vm.postpone_request();
                        }
                    }
                    Step::NativeFileWrite { path, bytes } => {
                        let request = vm.request_id().context("BGI native file write identity")?;
                        if !self.native_file_write(host, &mut vm, request, &path, bytes)? {
                            pending.insert(request);
                            vm.postpone_request();
                        }
                    }
                    Step::LoadProgram { archive, file } => {
                        let request = vm.request_id().context("BGI program request identity")?;
                        self.program_load = self.program_jobs.remove(&request);
                        if self.program_load.is_none() {
                            let storage = host.storage(self.client)?;
                            let client = self.client;
                            let config = self.config.clone();
                            self.program_load = Some(self.pool.spawn(move |cancel| {
                                cancel.check()?;
                                let bytes =
                                    program_io::load(storage, client, &config, &archive, &file)?;
                                cancel.check()?;
                                Ok(bytes)
                            })?);
                        }
                        let Some(bytes) = self.program_load.as_mut().unwrap().try_take()? else {
                            self.program_jobs
                                .insert(request, self.program_load.take().unwrap());
                            pending.insert(request);
                            vm.postpone_request();
                            continue;
                        };
                        vm.commit_program(&bytes)?;
                        self.program_load = None;
                    }
                    Step::Call(call) => {
                        let request = vm.request_id().context("BGI native request identity")?;
                        let mut resources = Self::call_bitmap_dependencies(&call);
                        resources.extend(self.graph_bitmap_dependencies(&call, &vm)?);
                        resources.sort_unstable();
                        resources.dedup();
                        if resources.iter().any(|id| {
                            self.resource_fences
                                .get(id)
                                .is_some_and(|owner| *owner != request)
                        }) {
                            pending.insert(request);
                            vm.postpone_request();
                            continue;
                        }
                        if effect_api::handles(call.group, call.id) {
                            if !self.effect_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            }
                            continue;
                        }
                        if graph_controls::handles(call.group, call.id) {
                            self.control_call(host, &mut vm, &call)?;
                            continue;
                        }
                        if media_api::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.media_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if user_api::handles(call.group, call.id) {
                            if !self.user_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            }
                            continue;
                        }
                        if system_api::handles(call.group, call.id) {
                            if let Some(wait) = self.system_call(host, &mut vm, &call)? {
                                self.wait = Some(wait);
                                return Ok(wait);
                            }
                            continue;
                        }
                        if selection_api::handles(call.group, call.id) {
                            ensure!(
                                self.selection_config_call(&mut vm, &call)?,
                                "BGI selection dispatch policy"
                            );
                            continue;
                        }
                        if window_api::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            self.bitmap_work = self.bitmap_jobs.remove(&request);
                            let complete = self.window_call(host, &mut vm, &call)?;
                            if let Some(work) = self.bitmap_work.take() {
                                self.bitmap_jobs.insert(request, work);
                            }
                            if !complete {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if self.message_call(host, &mut vm, &call)? {
                            continue;
                        }
                        if bitmap_codec::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.bitmap_codec_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if self.field_bitmap_handles(&call) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.field_bitmap_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if bitmap_memory::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.bitmap_memory_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if graph_api::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.graph_call(host, &mut vm, &call)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if text_api::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            if !self.text_call(host, &mut vm, &call, request)? {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if bitmap_api::handles(call.group, call.id) {
                            for &id in &resources {
                                self.resource_fences.insert(id, request);
                            }
                            self.bitmap_work = self.bitmap_jobs.remove(&request);
                            let complete = self.bitmap_call(host, &mut vm, &call)?;
                            if let Some(work) = self.bitmap_work.take() {
                                self.bitmap_jobs.insert(request, work);
                            }
                            if !complete {
                                pending.insert(request);
                                vm.postpone_request();
                            } else {
                                self.resource_fences.retain(|_, owner| *owner != request);
                            }
                            continue;
                        }
                        if call.group == 0xa0 {
                            let channel = match call.id {
                                0x10..=0x19 | 0x1c => call.args.first().map(|n| *n as u32),
                                0x20..=0x2f => call.args.first().map(|n| 0x100 | *n as u32),
                                0x80 | 0x81 | 0x84 | 0x85 | 0x86 => Some(0x1fe),
                                0xc0 => Some(0x13f),
                                _ => None,
                            };
                            if let Some(channel) = channel {
                                if self
                                    .audio_fences
                                    .get(&channel)
                                    .is_some_and(|owner| *owner != request)
                                {
                                    pending.insert(request);
                                    vm.postpone_request();
                                    continue;
                                }
                                self.audio_fences.insert(channel, request);
                            }
                            self.audio.enter_call(request);
                            let complete = self.audio_call(host, &mut vm, &call)?;
                            self.audio.leave_call(request, complete);
                            if complete {
                                self.audio_fences.retain(|_, owner| *owner != request);
                            }
                            if !complete {
                                pending.insert(request);
                                vm.postpone_request();
                            }
                            continue;
                        }
                        if file_api::handles(call.group, call.id) {
                            self.file_load = self.file_jobs.remove(&request);
                            let complete = self.file_call(host, &mut vm, &call)?;
                            if let Some(work) = self.file_load.take() {
                                self.file_jobs.insert(request, work);
                            }
                            if !complete {
                                pending.insert(request);
                                vm.postpone_request();
                            }
                            continue;
                        }
                        let Some((results, wait)) = self.native(host, &vm, &call)? else {
                            return Ok(VmYield::WaitUntil(host.now_ms().saturating_add(1)));
                        };
                        vm.commit_call(&results)?;
                        if let Some(wait) = wait {
                            self.wait = Some(wait);
                            return Ok(wait);
                        }
                    }
                }
            }
            Ok(VmYield::BudgetExhausted)
        })();
        if matches!(&result, Ok(VmYield::Halted)) {
            self.bitmap_jobs.clear();
            self.text_jobs.clear();
            self.memory_jobs.clear();
            self.field_memory_jobs.clear();
            self.program_jobs.clear();
            self.file_jobs.clear();
            self.resource_fences.clear();
            self.audio_fences.clear();
            self.save_jobs.clear();
            self.desktop_jobs.clear();
        }
        let result = result.and_then(|yielded| {
            self.graph_tick(host)?;
            self.tick_window_compositions(host)?;
            Ok(yielded)
        });
        self.vm = Some(vm);
        result
    }
}
impl VmAdapter for BgiVm {
    fn engine_id(&self) -> &'static str {
        "bgi"
    }
    fn snapshot_schema(&self) -> u32 {
        1
    }
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        budget: u32,
    ) -> crysirror_core::Result<VmYield> {
        if budget == 0 {
            return Ok(VmYield::BudgetExhausted);
        }
        self.run(host, input, budget)
            .map_err(|e| Error::Io(format!("{e:#}")))
    }
    fn snapshot(&self) -> crysirror_core::Result<Vec<u8>> {
        Err(Error::Unsupported("BGI native runtime snapshot"))
    }
    fn restore(&mut self, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::Unsupported("BGI native runtime snapshot"))
    }
    fn diagnostics(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({"profile":"native-bp","loaded":self.vm.is_some(),"pc":self.vm.as_ref().map(Machine::pc),"modules":self.vm.as_ref().map(Machine::module_count),"threads":self.vm.as_ref().map(Machine::thread_count),"native_calls":self.calls,"bitmaps":self.bitmaps.len(),"sprites":self.sprites.len(),"bitmap_sizes":self.bitmaps.iter().take(16).map(|(id,b)|(*id,b.size)).collect::<Vec<_>>()})).unwrap_or_default()
    }
}
