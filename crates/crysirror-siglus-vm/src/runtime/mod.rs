// SPDX-License-Identifier: MPL-2.0
use crate::{
    assets::RgbaImage,
    audio::{AudioHub, BgmEngine, KoeEngine, PcmEngine, SeEngine},
    image_manager::ImageManager,
    layer::RenderFrame,
};
use anyhow::{Result, bail};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};
pub mod constants;
pub mod forms;
pub mod gan;
pub mod globals;
pub mod opcode;
pub mod string_semantics;
pub mod unknown;
pub mod wait;
pub mod input {
    pub use crysirror_scene::input::*;
}
pub mod int_event {
    pub use crysirror_scene::animation::*;
}
mod scene_metadata;
pub mod tables;
pub mod twitter;
pub mod ui;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Value {
    Int(i64),
    Str(String),
    /// An element chain as raw i32 codes (as stored on the VM int stack).
    Element(Vec<i32>),
    /// A nested list value (FM_LIST).
    List(Vec<Value>),
    /// A named argument (id -> value), used by some engine commands.
    NamedArg {
        id: i32,
        value: Box<Value>,
    },
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::NamedArg { value, .. } => value.as_i64(),
            _ => None,
        }
    }

    pub fn named_id(&self) -> Option<i32> {
        match self {
            Value::NamedArg { id, .. } => Some(*id),
            _ => None,
        }
    }

    pub fn unwrap_named(&self) -> &Value {
        match self {
            Value::NamedArg { value, .. } => value.as_ref(),
            _ => self,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            Value::NamedArg { value, .. } => value.as_str(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Command {
    pub name: String,
    /// Optional numeric code for VM forms.
    pub code: Option<opcode::OpCode>,
    pub args: Vec<Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExcallCompatState {
    pub ready: bool,
    /// C_elm_excall::free() runs frame-action end callbacks before clearing
    /// its child lists/stages. Rust drains those callbacks immediately after
    /// command dispatch, so resource release is deferred until that drain.
    pub pending_free: bool,
    pub ex_call_flag: bool,
    pub flag_204: bool,
    pub flag_2148: bool,
    pub script_proc_requested: bool,
    pub script_proc_pop_requested: bool,

    // C_elm_excall owns an independent set of SCRIPT font overrides.
    // They are not aliases of Gp_local: cmd_script.cpp routes
    // EXCALL.SCRIPT.{87..95} to Gp_excall->m_font_name / m_pod.
    pub font_name: String,
    pub font_bold: i64,
    pub font_shadow: i64,
    // Newer C_elm_excall POD field corresponding to SCRIPT.98/99/100.
    pub joypad_mode_override: i64,
}

impl Default for ExcallCompatState {
    fn default() -> Self {
        Self {
            ready: false,
            pending_free: false,
            ex_call_flag: false,
            flag_204: false,
            flag_2148: false,
            script_proc_requested: false,
            script_proc_pop_requested: false,
            font_name: String::new(),
            font_bold: -1,
            font_shadow: -1,
            joypad_mode_override: -1,
        }
    }
}

pub trait ExternalFormHandler: Send + Sync {
    /// Return true if the form ID was handled.
    fn dispatch_form(
        &self,
        ctx: &mut CommandContext,
        form_id: u32,
        args: &[Value],
    ) -> anyhow::Result<bool>;
}
/// Cooperative script-process boundary, mirroring Siglus' `TNM_PROC_TYPE_*`
/// model at the VM/runtime boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProcKind {
    Script,
    Disp,
    Frame,
    Command,
    MessageBlock,
    MessageWait,
    KeyWait,
    TimeWait,
    MovieWait,
    WipeWait,
    AudioWait,
    EventWait,
    Selection,
    SystemModal,
}

/// Spare `Vec<i32>` buffers for element paths and object chains, which the
/// VM makes and drops for every property access: reusing them saves an
/// allocation and a free each time (a tenth of the VM's frame time in
/// particle scripts, more with the console allocators).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IntVecPool {
    #[serde(skip)]
    spare: Vec<Vec<i32>>,
}

impl IntVecPool {
    const MAX_SPARE: usize = 32;
    const MAX_CAPACITY: usize = 64;

    /// An empty vector, reused when one is spare.
    pub fn take(&mut self) -> Vec<i32> {
        self.spare.pop().unwrap_or_default()
    }

    /// A copy of `items` in a reused vector.
    pub fn take_copy(&mut self, items: &[i32]) -> Vec<i32> {
        let mut v = self.take();
        v.extend_from_slice(items);
        v
    }

    /// Keeps `v` for reuse (small ones, a few at a time).
    pub fn give(&mut self, mut v: Vec<i32>) {
        if self.spare.len() < Self::MAX_SPARE && v.capacity() <= Self::MAX_CAPACITY {
            v.clear();
            self.spare.push(v);
        }
    }
}

impl Clone for IntVecPool {
    /// Spare buffers are not state: a copy starts without them.
    fn clone(&self) -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VmCallMeta {
    pub element: Vec<i32>,
    pub al_id: i64,
    pub ret_form: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RuntimeSaveKind {
    Normal,
    Quick,
    End,
    Inner,
}

/// GPU service for original-engine capture commands. Captures must pass through
/// the same render graph as presentation so meshes, depth, fog and shader state
/// are preserved.
pub trait FrameCaptureBackend {
    fn capture_render_frame(
        &mut self,
        images: &ImageManager,
        frame: &RenderFrame,
        logical_width: u32,
        logical_height: u32,
    ) -> Result<RgbaImage>;
}

pub type FrameCaptureBackendRef = Rc<RefCell<dyn FrameCaptureBackend>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeSaveRequest {
    pub kind: RuntimeSaveKind,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeLoadRequest {
    pub kind: RuntimeSaveKind,
    pub index: usize,
}

/// Engine-side cache mirroring the C++ `S_tnm_local_save` (a.k.a. `Gp_eng->m_local_save`).
/// Populated by GLOBAL_SAVEPOINT (`tnm_save_local`) and after a successful load
/// (`tnm_load_local_on_file`). All save-to-file paths (normal / quick / end) must
/// pull from this cache instead of re-serializing the live runtime.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LocalSaveSnapshot {
    pub save_id: [u16; 7],
    pub append_dir: String,
    pub append_name: String,
    pub save_scene_title: String,
    pub save_msg: String,
    pub save_full_msg: String,
    pub local_stream: Vec<u8>,
    pub local_ex_stream: Vec<u8>,
    pub sel_saves: Vec<crate::original_save::OriginalLocalSaveEnvelope>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CommandContext {
    pub int_vec_pool: IntVecPool,
    pub project_dir: PathBuf,
    pub ids: constants::RuntimeConstants,
    pub globals: globals::GlobalState,
    pub tables: tables::AssetTables,
    pub stack: Vec<Value>,
    pub unknown: unknown::UnknownOpRecorder,
    pub ui: ui::UiRuntime,
    pub input: input::InputState,
    pub script_input: input::InputState,
    pub wait: wait::VmWait,
    pub screen_w: u32,
    pub screen_h: u32,
    pub current_scene_no: Option<i64>,
    pub current_scene_name: Option<String>,
    pub current_line_no: i64,
    pub vm_call: Option<VmCallMeta>,
    pub excall_state: ExcallCompatState,
    pub local_save_snapshot: Option<LocalSaveSnapshot>,
    pub pending_scene_restart: Option<(String, i32)>,
    pub images: ImageManager,
    pub audio: AudioHub,
    pub bgm: BgmEngine,
    pub koe: KoeEngine,
    pub pcm: PcmEngine,
    pub se: SeEngine,
    pub movie: crate::movie::MovieManager,
    #[serde(skip)]
    pub external_forms: Option<Arc<dyn ExternalFormHandler>>,
    proc_generation: u64,
    last_proc_kind: ProcKind,
    pending_auto_savepoint: bool,
    pending_read_flag_no: bool,
    runtime_load_completed: bool,
    pub backend_faults: RefCell<Vec<String>>,
    scene_names: RefCell<Option<Arc<scene_metadata::SceneMetadata>>>,
    last_ms: u64,
}
impl CommandContext {
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            int_vec_pool: Default::default(),
            project_dir,
            ids: Default::default(),
            globals: Default::default(),
            tables: Default::default(),
            stack: vec![],
            unknown: Default::default(),
            ui: Default::default(),
            input: Default::default(),
            script_input: Default::default(),
            wait: Default::default(),
            screen_w: 1280,
            screen_h: 720,
            current_scene_no: None,
            current_scene_name: None,
            current_line_no: -1,
            vm_call: None,
            excall_state: Default::default(),
            local_save_snapshot: None,
            pending_scene_restart: None,
            images: Default::default(),
            audio: Default::default(),
            bgm: Default::default(),
            koe: Default::default(),
            pcm: Default::default(),
            se: Default::default(),
            movie: Default::default(),
            external_forms: None,
            proc_generation: 0,
            last_proc_kind: ProcKind::Script,
            pending_auto_savepoint: false,
            pending_read_flag_no: false,
            runtime_load_completed: false,
            backend_faults: Default::default(),
            scene_names: Default::default(),
            last_ms: crate::platform_time::Instant::now().0,
        }
    }
    pub fn push(&mut self, v: Value) {
        self.stack.push(v)
    }
    pub fn pop(&mut self) -> Option<Value> {
        self.stack.pop()
    }
    pub fn proc_generation(&self) -> u64 {
        self.proc_generation
    }
    pub fn last_proc_kind(&self) -> ProcKind {
        self.last_proc_kind
    }
    pub fn request_proc_boundary(&mut self, kind: ProcKind) {
        self.proc_generation = self.proc_generation.wrapping_add(1);
        self.last_proc_kind = kind;
    }
    pub fn request_wait_proc_boundary(&mut self, kind: ProcKind) {
        self.request_proc_boundary(kind)
    }
    pub fn begin_frame_main_proc_pass(&mut self) {}
    pub fn begin_input_frame(&mut self) {}
    pub fn sync_script_input_from_runtime(&mut self) {
        self.script_input = self.input.clone();
    }
    pub fn wait_poll(&mut self) -> bool {
        self.wait.poll(
            &mut self.stack,
            &mut self.bgm,
            &mut self.koe,
            &mut self.se,
            &mut self.pcm,
            &mut self.globals,
            &self.ids,
            false,
        )
    }
    pub fn tick_frame(&mut self) {
        let now = crate::platform_time::Instant::now().0;
        let delta = now.saturating_sub(self.last_ms).min(i32::MAX as u64) as i32;
        self.last_ms = now;
        self.globals.tick_frame(delta, delta, &[], None);
    }
    pub fn on_mouse_down(&mut self, b: input::VmMouseButton) {
        self.input.on_mouse_down(b);
        self.wait.notify_key(&mut self.globals, &self.ids);
    }
    pub fn on_mouse_up(&mut self, b: input::VmMouseButton) {
        self.input.on_mouse_up(b)
    }
    pub fn on_mouse_move(&mut self, x: i32, y: i32) {
        self.input.on_mouse_move(x, y)
    }
    pub fn take_runtime_load_completed(&mut self) -> bool {
        std::mem::take(&mut self.runtime_load_completed)
    }
    pub fn mark_runtime_load_completed(&mut self) {
        self.runtime_load_completed = true;
    }
    pub fn begin_runtime_load_apply(&mut self) {
        self.wait.clear();
        self.input.clear_all();
        self.script_input.clear_all();
    }
    pub fn take_runtime_save_request(&mut self) -> Option<RuntimeSaveRequest> {
        None
    }
    pub fn take_runtime_load_request(&mut self) -> Option<RuntimeLoadRequest> {
        None
    }
    pub fn request_auto_savepoint(&mut self) {
        self.pending_auto_savepoint = true;
    }
    pub fn take_pending_auto_savepoint(&mut self) -> bool {
        std::mem::take(&mut self.pending_auto_savepoint)
    }
    pub fn request_message_auto_savepoint(&mut self) {
        self.local_save_snapshot = None;
        self.pending_auto_savepoint = true;
    }
    pub fn take_pending_sel_point_result(&mut self) -> Option<i32> {
        None
    }
    pub fn request_read_flag_no(&mut self) {
        self.pending_read_flag_no = true;
    }
    pub fn take_read_flag_no_request(&mut self) -> bool {
        std::mem::take(&mut self.pending_read_flag_no)
    }
    pub fn discard_read_flag_no_request(&mut self) {
        self.pending_read_flag_no = false;
    }
    pub fn submit_read_flag_no(&mut self, flag: i32) {
        if let Some(s) = self.current_scene_no {
            self.globals.set_read_flag(s, i64::from(flag));
        }
    }
    pub fn install_scene_metadata(&self, _: &str, p: &crate::scene_pck::ScenePck) -> Result<()> {
        *self.scene_names.borrow_mut() =
            Some(Arc::new(scene_metadata::SceneMetadata::from_pack(p)?));
        Ok(())
    }
    pub(crate) fn scene_metadata(&self) -> Result<Arc<scene_metadata::SceneMetadata>> {
        self.scene_names
            .borrow()
            .clone()
            .ok_or_else(|| anyhow::anyhow!("scene metadata not installed"))
    }
    pub fn lookup_scene_no(&self, s: &str) -> Option<i64> {
        self.scene_names
            .borrow()
            .as_ref()?
            .find_scene_no(s)
            .map(|v| v as i64)
    }
    pub fn set_focused_editbox(&mut self, f: Option<(u32, usize)>) {
        self.globals.focused_editbox = f;
    }
    pub fn set_active_append(&mut self, s: String, name: String) {
        self.globals.append_dir = s;
        self.globals.append_name = name;
    }
    pub fn reset_for_scene_restart(&mut self) {
        self.wait.clear();
        self.stack.clear();
    }
    pub fn render_list_with_effects(&mut self) -> Vec<crate::layer::RenderSprite> {
        vec![]
    }
    pub fn finish_wipe_runtime(&mut self) {
        self.globals.finish_wipe();
    }
}
pub fn dispatch_form_code(ctx: &mut CommandContext, id: u32, args: &[Value]) -> Result<bool> {
    use constants::{elm_value as ev, global_form as gf};
    if id == ev::GLOBAL_NOP as u32 {
        return Ok(true);
    }
    if id == ev::GLOBAL_GET_LINE_NO as u32 {
        ctx.push(Value::Int(ctx.current_line_no));
        return Ok(true);
    }
    if id == ev::GLOBAL_GET_SCENE_NAME as u32 {
        ctx.push(Value::Str(
            ctx.current_scene_name.clone().unwrap_or_default(),
        ));
        return Ok(true);
    }
    if constants::matches_form_id(id, ctx.ids.form_global_bgm, gf::BGM) {
        return crate::bgm_bridge::dispatch(ctx, args);
    }
    if constants::matches_form_id(id, ctx.ids.form_global_bgm_table, gf::BGMTABLE) {
        return crate::bgm_bridge::dispatch_table(ctx, args);
    }
    if constants::matches_form_id(id, ctx.ids.form_global_pcm, gf::PCM) {
        return crate::sfx_bridge::pcm(ctx, args);
    }
    if constants::matches_form_id(id, ctx.ids.form_global_se, gf::SE) {
        return crate::sfx_bridge::se(ctx, args);
    }
    if crate::koe_bridge::dispatch(ctx, id, args)? || crate::koe_bridge::config(ctx, id, args)? {
        return Ok(true);
    }
    if id == gf::TIMEWAIT {
        return forms::timewait::dispatch(ctx, false, args);
    }
    if id == gf::TIMEWAIT_KEY {
        return forms::timewait::dispatch(ctx, true, args);
    }
    if constants::global_form::INT_LIST_FORMS.contains(&id) {
        return forms::int_list::dispatch(ctx, id, args);
    }
    if constants::global_form::STR_LIST_FORMS.contains(&id) {
        return forms::str_list::dispatch(ctx, id, args);
    }
    if id == gf::COUNTER {
        return forms::counter::dispatch(ctx, id, args);
    }
    if forms::stage::is_stage_form_id(ctx, id as i32) {
        return forms::stage::dispatch(ctx, args);
    }
    if id == ctx.ids.form_global_input {
        return forms::input::dispatch(ctx, id, args);
    }
    if id == ctx.ids.form_global_mouse {
        return forms::mouse::dispatch(ctx, args);
    }
    if id == ctx.ids.form_global_key {
        return forms::key::dispatch(ctx, args);
    }
    if id == ctx.ids.form_global_math {
        return forms::math::dispatch(ctx, id, args);
    }
    if id == forms::codes::FM_INTEVENT as u32 || id == forms::codes::FM_INTEVENTLIST as u32 {
        return forms::int_event::dispatch(ctx, id, args);
    }

    bail!(
        "unsupported Siglus native form {id} chain {:?}",
        ctx.vm_call.as_ref().map(|v| &v.element)
    )
}
