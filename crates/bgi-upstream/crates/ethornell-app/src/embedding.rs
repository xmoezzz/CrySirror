//! Device-free original BGI VM host. Resources are per-client capabilities;
//! native render plans and audio commands are consumed by CrySirror services.
use super::*;
pub use crate::audio_runtime::{AudioAsset, AudioCommand, StreamBytes};
pub use crate::crysirror_frame::FramePlan;
use ab_glyph::{FontArc, FontVec};
use crysirror_core::service::ClientStorage;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct HostConfig {
    pub storage: Arc<dyn ClientStorage>,
    pub asset_root: String,
    pub font_data: Vec<u8>,
    pub font_face_index: u32,
    pub gbk: bool,
    pub strict: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum HostInput {
    MouseMove { x: f32, y: f32 },
    MousePress { x: f32, y: f32 },
    MouseRelease { x: f32, y: f32 },
    ButtonPress { button: i32, x: f32, y: f32 },
    ButtonRelease { button: i32, x: f32, y: f32 },
    KeyPress { descriptor: i32 },
    KeyRelease { descriptor: i32 },
    MouseWheel { delta_y: f32 },
    Cancel,
}

#[derive(Clone, Debug)]
pub struct HostEvent {
    pub kind: u32,
    pub request_id: u64,
    pub code: i32,
    pub text: String,
}
#[derive(Default)]
pub(super) struct HostServices {
    pub(super) enabled: bool,
    events: VecDeque<HostEvent>,
    pub(super) pending: BTreeMap<u64, usize>,
    /// Pending UserB0 input dialogs, answered with a JSON response.
    pub(super) dialogs: BTreeMap<u64, ethornell_vm::UserDialogRequest>,
    /// Pending Sys80:3C prompts: the message and the file they wait for.
    retries: BTreeMap<u64, (String, String)>,
    fatal: Option<String>,
    pub(super) input_trace: bool,
    input_trace_count: u32,
}
impl HostServices {
    pub(super) fn input_gate(&mut self, values: [u32; 9]) {
        if !self.enabled || !self.input_trace || self.input_trace_count >= 64 {
            return;
        }
        self.input_trace_count += 1;
        let [
            pc,
            frame,
            event,
            payload,
            parameter,
            group,
            item,
            running,
            gate,
        ] = values;
        self.events.push_back(HostEvent {kind:1,request_id:0,code:1002,text:format!("input_gate pc={pc} frame={frame} event={event} payload={payload} parameter={parameter} group={group} item={item} running={running} gate={gate}")});
    }
    pub(super) fn input_route(&mut self, values: [u32; 7], bits: [bool; 5]) {
        if !self.enabled || !self.input_trace || self.input_trace_count >= 64 {
            return;
        }
        let [object, group, item, root, flags, current, pending] = values;
        let [pointer, hit, press, up, event6] = bits;
        self.input_trace_count += 1;
        self.events.push_back(HostEvent {kind:1,request_id:0,code:1001,text:format!("input_route object={object} group={group} item={item} root={root} flags={flags} current={current} pending={pending} pointer={} hit={} press={} up={} event6={}",u32::from(pointer),u32::from(hit),u32::from(press),u32::from(up),u32::from(event6))});
    }
    pub(super) fn unsupported(&mut self, capability: &str) {
        if !self.enabled {
            return;
        }
        self.fatal = Some(format!(
            "unsupported or denied host capability: {capability}"
        ));
    }
    /// Event kind 5 names the dialog type; the host fetches the JSON payload
    /// with `HostRuntime::request_payload` and answers with JSON text.
    pub(super) fn request_dialog(&mut self, request: ethornell_vm::UserDialogRequest) -> u64 {
        let id = Self::next_request_id();
        self.pending.insert(id, 1);
        self.events.push_back(HostEvent {
            kind: 5,
            request_id: id,
            code: 1,
            text: super::dialog_protocol::dialog_kind(&request).to_string(),
        });
        self.dialogs.insert(id, request);
        id
    }
    /// Sys80:3C's Retry/Quit prompt for a missing required file: a kind-2
    /// confirmation (1 = Retry). Retry rechecks the file and, if it is still
    /// missing, asks again under the same id.
    pub(super) fn request_retry(&mut self, message: String, file: String) -> u64 {
        let id = self.request(message.clone(), 1);
        self.retries.insert(id, (message, file));
        id
    }
    fn next_request_id() -> u64 {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }
    pub(super) fn request(&mut self, text: String, outputs: usize) -> u64 {
        let id = Self::next_request_id();
        self.pending.insert(id, outputs);
        self.events.push_back(HostEvent {
            kind: 2,
            request_id: id,
            code: outputs as i32,
            text,
        });
        id
    }
}

pub struct HostRuntime {
    runtime: RuntimeEngine,
    policy: file_policy::FilePolicy,
    font: FontArc,
    input_queue: VecDeque<RuntimeInputEvent>,
    input_script: Option<HeadlessInputScript>,
    engine_frames: u64,
    closed: bool,
    failed: bool,
    has_presented: bool,
    vm_budget: ethornell_vm::VmResourceBudget,
}
impl HostRuntime {
    pub fn create(config: HostConfig) -> Result<Self> {
        let policy = file_policy::FilePolicy::from_client(config.storage, &config.asset_root)?;
        let font = FontVec::try_from_vec_and_index(config.font_data, config.font_face_index)
            .map(FontArc::new)
            .map_err(|_| EthornellError::Other("invalid BGI font or face".into()))?;
        let manager = policy.open_manager()?;
        let bytes = manager.read_decoded_from_archive("system.arc", "ipl._bp")?;
        let runtime = file_policy::scope(&policy, || {
            snapshot::with_font(&font, || {
                let mut runtime = RuntimeEngine::new_explicit(
                    "system.arc:ipl._bp",
                    &bytes,
                    manager,
                    policy.resource.clone(),
                    config.strict,
                    if config.gbk {
                        encoding_rs::GBK
                    } else {
                        encoding_rs::SHIFT_JIS
                    },
                );
                configure_embedded_runtime(&mut runtime);
                check_raster_errors()?;
                Ok::<_, EthornellError>(runtime)
            })
        })?;
        Ok(Self {
            runtime,
            policy,
            font,
            input_queue: VecDeque::new(),
            input_script: None,
            engine_frames: 0,
            closed: false,
            failed: false,
            has_presented: false,
            vm_budget: ethornell_vm::VmResourceBudget {
                max_vm_count: 64,
                max_memory_capacity_bytes: 768 * 1024 * 1024,
            },
        })
    }
    pub fn scope<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let policy = self.policy.clone();
        let font = self.font.clone();
        file_policy::scope(&policy, || snapshot::with_font(&font, || f(self)))
    }
    /// An explicit scripted FIFO is useful for legal fixtures and reproducible
    /// game probes. No environment variable can inject it into this host.
    pub fn set_input_script(&mut self, source: &str) -> Result<()> {
        self.input_script =
            Some(HeadlessInputScript::parse(source).map_err(EthornellError::Other)?);
        Ok(())
    }
    /// Run one original native scheduler tick. None repeats the previous image
    /// or denotes an actual host continuation which must be answered explicitly.
    pub fn frame(&mut self, elapsed_ms: u64, inputs: &[HostInput]) -> Result<Option<FramePlan>> {
        if self.failed {
            return Err(EthornellError::Other(
                "BGI host cannot resume after a failed frame".into(),
            ));
        }
        if self.closed {
            return Ok(None);
        }
        if elapsed_ms > 60_000 || inputs.len() > 4096 {
            return Err(EthornellError::Other(
                "BGI host tick/input budget exceeded".into(),
            ));
        }
        for input in inputs {
            let finite = match input {
                HostInput::MouseMove { x, y }
                | HostInput::MousePress { x, y }
                | HostInput::MouseRelease { x, y }
                | HostInput::ButtonPress { x, y, .. }
                | HostInput::ButtonRelease { x, y, .. } => {
                    x.is_finite() && y.is_finite() && x.abs() < 1_000_000.0 && y.abs() < 1_000_000.0
                }
                HostInput::MouseWheel { delta_y } => delta_y.is_finite(),
                _ => true,
            };
            if !finite {
                return Err(EthornellError::Other("invalid BGI host input".into()));
            }
        }
        let result = self.scope(|host| host.frame_inner(elapsed_ms, inputs));
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn frame_inner(&mut self, elapsed_ms: u64, inputs: &[HostInput]) -> Result<Option<FramePlan>> {
        if !self.runtime.api.host.pending.is_empty() {
            return Ok(None);
        }
        for input in inputs {
            let event = match *input {
                HostInput::MouseMove { x, y } => RuntimeInputEvent::MouseMove { x, y },
                HostInput::MousePress { x, y } => RuntimeInputEvent::MousePress { x, y },
                HostInput::MouseRelease { x, y } => RuntimeInputEvent::MouseRelease { x, y },
                HostInput::ButtonPress { button, x, y } => {
                    RuntimeInputEvent::ButtonPress { button, x, y }
                }
                HostInput::ButtonRelease { button, x, y } => {
                    RuntimeInputEvent::ButtonRelease { button, x, y }
                }
                HostInput::KeyPress { descriptor } => RuntimeInputEvent::KeyPress { descriptor },
                HostInput::KeyRelease { descriptor } => {
                    RuntimeInputEvent::KeyRelease { descriptor }
                }
                HostInput::MouseWheel { delta_y } => RuntimeInputEvent::MouseWheel { delta_y },
                HostInput::Cancel => {
                    self.clear_input();
                    continue;
                }
            };
            let consumed = match event {
                RuntimeInputEvent::MousePress { x, y } => {
                    self.runtime.api.handle_user_modeless_pointer(x, y, true)
                }
                RuntimeInputEvent::MouseRelease { x, y } => {
                    self.runtime.api.handle_user_modeless_pointer(x, y, false)
                }
                _ => false,
            };
            if !consumed {
                queue_runtime_input_event(&mut self.input_queue, event);
            }
        }
        self.runtime
            .vm
            .check_resource_budget(self.vm_budget)
            .map_err(|e| EthornellError::Other(e.to_string()))?;
        // Same FIFO and script ordering as runtime_frontend::drive_frame. Audio
        // remains queued for the shared mixer instead of being dropped/silenced.
        while let Some(event) = self.input_queue.pop_front() {
            apply_runtime_input_event(&mut self.runtime.api, event);
        }
        if let Some(event) = self
            .input_script
            .as_mut()
            .and_then(HeadlessInputScript::tick)
        {
            apply_headless_input_event(&mut self.runtime.api, event);
        }
        self.runtime.api.performance_profile.begin();
        let report = self
            .runtime
            .run_frame(RuntimeFramePacing, elapsed_ms, elapsed_ms);
        self.runtime.api.performance_profile.end(false);
        self.engine_frames += 1;
        self.runtime
            .vm
            .check_resource_budget(self.vm_budget)
            .map_err(|e| EthornellError::Other(e.to_string()))?;
        check_raster_errors()?;
        if let Some(error) = self.runtime.api.host.fatal.take() {
            return Err(EthornellError::Other(error));
        }
        if let Some(report) = report
            .last_report
            .as_ref()
            .filter(|r| r.stop_reason.is_fatal())
        {
            return Err(EthornellError::Other(format!(
                "native BGI {:?} program={} pc={} offset={:?}; trace={:?}",
                report.stop_reason, report.program, report.pc, report.offset, report.recent_trace
            )));
        }
        // The original public desktop restart opens an ambient root. An
        // embedded restart reuses this exact authorized resource manager.
        if let Some((root, archive, file)) = self.runtime.api.native_system.bootstrap_request.take()
        {
            if root.as_deref().is_some_and(|r| {
                !matches!(
                    r.replace('\\', "/").trim_end_matches('/'),
                    "" | "bgi-resource"
                )
            }) {
                return Err(EthornellError::Other(
                    "native bootstrap cannot leave the client resource capability".into(),
                ));
            }
            if !self.runtime.api.audio_requests.is_empty() {
                return Err(EthornellError::Other(
                    "native bootstrap with pending audio requires an explicit shared-service flush"
                        .into(),
                ));
            }
            let manager = self.runtime.api.manager.clone();
            let bytes = manager.read_decoded_from_archive(&archive, &file)?;
            let strict = self.runtime.options.fail_on_stub;
            let encoding = self.runtime.vm.graph_text_encoding();
            self.runtime = RuntimeEngine::new_explicit(
                &format!("{archive}:{file}"),
                &bytes,
                manager,
                self.policy.resource.clone(),
                strict,
                encoding,
            );
            configure_embedded_runtime(&mut self.runtime);
            self.has_presented = false;
        }
        if let Some(title) = self.runtime.api.pending_window_title.take() {
            self.runtime.api.host.events.push_back(HostEvent {
                kind: 4,
                request_id: 0,
                code: 0,
                text: title,
            });
        }
        if self.runtime.api.quit_requested {
            self.closed = true;
            self.runtime.api.host.events.push_back(HostEvent {
                kind: 3,
                request_id: 0,
                code: 0,
                text: String::new(),
            });
        }
        if std::mem::take(&mut self.runtime.api.pending_window_close_request)
            && self.runtime.api.handle_host_window_close("crysirror-bgi")
        {
            self.closed = true;
        }
        if !self.runtime.api.host.pending.is_empty() {
            return Ok(None);
        }
        if self.runtime.api.take_present_due() || !self.has_presented {
            let plan = crate::crysirror_frame::capture(&self.runtime.api, &self.font)?;
            check_raster_errors()?;
            self.has_presented = true;
            self.runtime.api.presentation_state =
                self.runtime
                    .api
                    .clock_base_ms
                    .wrapping_add(self.runtime.api.engine_time_ms as u32) as i32;
            Ok(Some(plan))
        } else {
            Ok(None)
        }
    }
    pub fn take_audio_commands(&mut self) -> Vec<AudioCommand> {
        self.runtime.api.audio_requests.drain(..).collect()
    }
    pub fn poll_event(&mut self) -> Option<HostEvent> {
        self.runtime.api.host.events.pop_front()
    }
    pub fn is_halted(&self) -> bool {
        self.closed || self.runtime.vm.halted
    }
    pub fn size(&self) -> [u32; 2] {
        [
            self.runtime.api.screen_width.max(1) as u32,
            self.runtime.api.screen_height.max(1) as u32,
        ]
    }
    pub fn diagnostics(&self) -> String {
        let report = self.runtime.last_report.as_ref();
        let usage = self.runtime.vm.resource_usage();
        // Bounded, read-only native visibility: no glyph rasterization, VM
        // execution or mutation occurs while collecting these diagnostics.
        let visible_texts: Vec<String> = self
            .runtime
            .api
            .text_nodes
            .values()
            .filter(|node| self.runtime.api.should_draw_text_node(node))
            .take(16)
            .map(|node| node.text.chars().take(256).collect())
            .collect();
        let text_nodes = self.runtime.api.text_nodes.len();
        let render_tree_nodes = self.runtime.api.graph_output_draw_items().len();
        serde_json::json!({"engine":"bgi","frames":self.engine_frames,"time_ms":self.runtime.api.engine_time_ms,"screen":self.size(),"pending_host_requests":self.runtime.api.host.pending.len(),"audio_pending":self.runtime.api.audio_requests.len(),"halted":self.is_halted(),"failed":self.failed,"program":report.map(|r|r.program.as_str()),"pc":report.map(|r|r.pc),"stop":report.map(|r|format!("{:?}",r.stop_reason)),"vm_count":usage.vm_count,"vm_memory_capacity_bytes":usage.memory_capacity_bytes,"device_free":true,"visible_texts":visible_texts,"text_nodes":text_nodes,"render_tree_nodes":render_tree_nodes,"visible_text_limit_chars_per_node":256}).to_string()
    }
    pub fn clear_input(&mut self) {
        self.input_queue.clear();
        self.runtime.api.input_physically_held.clear();
        self.runtime.api.input_event_counts.clear();
        self.runtime.api.input_descriptor_accumulators.clear();
        self.runtime.api.window_messages.clear();
        self.runtime.api.pending_input_state = None;
        self.runtime.api.pending_input_descriptor = None;
        self.runtime.api.pending_click = None;
        self.runtime.api.pending_object_state = None;
        self.runtime.api.input_down_descriptors.clear();
        self.runtime.api.input_down_reported.clear();
        self.runtime.api.mouse_pressed = false;
        self.runtime.api.last_button_press = None;
        self.runtime.api.suppress_message_mouse_release = false;
        for input in self.runtime.api.graph_input_objects.values_mut() {
            input.clear_deferred_pointer_activation();
        }
        self.runtime.api.finish_graph_knob_drag();
    }
    /// The JSON description of a kind-5 dialog request.
    pub fn request_payload(&self, id: u64) -> Option<String> {
        self.runtime
            .api
            .host
            .dialogs
            .get(&id)
            .map(super::dialog_protocol::request_json)
    }
    /// Answer a kind-5 dialog request with its JSON response.
    pub fn complete_dialog(&mut self, id: u64, response: &str) -> Result<()> {
        self.scope(|host| host.complete_dialog_inner(id, response))
    }
    fn complete_dialog_inner(&mut self, id: u64, response: &str) -> Result<()> {
        let request = self
            .runtime
            .api
            .host
            .dialogs
            .get(&id)
            .ok_or_else(|| EthornellError::Other("unknown/stale request".into()))?;
        let response = super::dialog_protocol::response_from_json(request, response)
            .map_err(EthornellError::Other)?;
        self.runtime
            .vm
            .complete_user_dialog(id, response)
            .map_err(|e| EthornellError::Other(e.to_string()))?;
        self.runtime.api.host.dialogs.remove(&id);
        self.runtime.api.host.pending.remove(&id);
        self.clear_input();
        Ok(())
    }
    pub fn complete_request(&mut self, id: u64, result: i32) -> Result<()> {
        self.scope(|host| host.complete_request_inner(id, result))
    }
    fn complete_request_inner(&mut self, id: u64, result: i32) -> Result<()> {
        if self.runtime.api.host.dialogs.contains_key(&id) {
            return Err(EthornellError::Other(
                "dialog request needs a JSON response".into(),
            ));
        }
        let outputs = *self
            .runtime
            .api
            .host
            .pending
            .get(&id)
            .ok_or_else(|| EthornellError::Other("unknown/stale request".into()))?;
        let mut result = result;
        if let Some((message, file)) = self.runtime.api.host.retries.get(&id).cloned() {
            if result != 0 && !self.runtime.api.required_file_present(&file) {
                self.runtime.api.host.events.push_back(HostEvent {
                    kind: 2,
                    request_id: id,
                    code: 1,
                    text: message,
                });
                return Ok(());
            }
            result = i32::from(result != 0);
            self.runtime.api.host.retries.remove(&id);
        }
        self.runtime
            .vm
            .complete_host_request(
                id,
                ethornell_vm::HostResponse {
                    outputs: if outputs == 0 {
                        vec![]
                    } else {
                        vec![ethornell_vm::Value::Int(result)]
                    },
                    writes: vec![],
                },
            )
            .map_err(|e| EthornellError::Other(e.to_string()))?;
        self.runtime.api.host.pending.remove(&id);
        self.clear_input();
        Ok(())
    }
}
fn check_raster_errors() -> Result<()> {
    if let Some(error) = snapshot::take_raster_error() {
        return Err(EthornellError::Other(error));
    }
    if snapshot::take_missing_glyphs() {
        return Err(EthornellError::Other(
            "BGI font lacks required CPU bitmap glyphs".into(),
        ));
    }
    Ok(())
}
fn configure_embedded_runtime(runtime: &mut RuntimeEngine) {
    runtime.api.host.enabled = true;
    runtime.api.native_dialog_presentation = false;
    runtime.api.debug_graph = false;
    runtime.api.trace_render_tree = false;
    runtime.api.auto_title_click = false;
    runtime.api.auto_user_click = false;
    runtime.api.title_compatibility_render_fallback = false;
    runtime.api.legacy_scenario_mode_input = false;
    runtime.api.call_coverage_enabled = false;
    runtime.api.clock_base_ms = 0;
    runtime.api.host_realtime_clock = false;
    runtime.vm.set_clock_base_ms(0);
}
impl RuntimeEngine {
    fn new_explicit(
        script: &str,
        bytes: &[u8],
        manager: ResourceManager,
        root: PathBuf,
        strict: bool,
        encoding: &'static encoding_rs::Encoding,
    ) -> Self {
        let program = ethornell_script::parse_bp_program(Some(script.into()), bytes);
        let mut vm = ethornell_vm::Vm::new();
        vm.set_native_abi_profile(confirmed_native_profile(&manager, bytes));
        vm.set_graph_text_encoding(encoding);
        vm.start(&program);
        Self {
            vm,
            api: RuntimeTraceApi::new_with_native_root(manager, root),
            options: ethornell_vm::VmRunOptions {
                max_steps: ethornell_vm::DEFAULT_COOPERATIVE_WATCHDOG_STEPS,
                trace: false,
                fail_on_stub: strict,
                collect_diagnostics: true,
            },
            last_report: None,
        }
    }
}
