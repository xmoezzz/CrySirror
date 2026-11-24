// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Handle translation between KiriKiri native objects and engine services.
//!
//! A KiriKiri `Layer` is one engine Layer node (position, size, clip,
//! visibility, opacity and painter order) with one engine Canvas surface
//! presented beneath it for its image. Drawing calls become Canvas commands;
//! no pixels are composed here. Commands are queued while TJS runs and
//! submitted in order by the adapter after each script entry point returns.
// Native Layer ownership:
// Layer parent/child IDs describe relationships; mouse hover/capture references
// do not keep native objects alive. Retirement cancels producers/callbacks,
// detaches children before parents, hides presentations and destroys the owned
// Canvas before returning its slot. Image-only transitions reuse that Canvas
// through another presentation; native and engine capacity limits stay distinct.
// Image composition declarations use the shared full-copy history-pruning
// helper; partial clips and RGB/alpha-only writes must retain destination data.
use crate::storage::{Storage, decode_text};
use crysirror_core::service::{ImageMetadata, TextMeasurement};
use crysirror_tjs2::{
    Result as TjsResult, TjsError,
    runtime::{ObjectHandle, TjsHost},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const DEFAULT_LAYER_SIZE: u32 = 32;
/// One shared Canvas per native Layer. Image-only transitions add a temporary
/// presentation of an existing source Canvas, not another retained surface.
/// Engine Layer nodes and pixels keep their independent admission limits.
pub const MAX_LAYERS: u32 = crysirror_core::service::MAX_CANVAS_SURFACES_PER_CLIENT as u32;
const MAX_LOG: usize = 4096;

#[derive(Clone, Debug)]
pub struct LayerState {
    /// Native instance allocation precedes the script constructor. Unbound
    /// instances can own child resources but cannot be presented or hit-tested.
    pub constructed: bool,
    /// Original tooltip string; engine UI owns its display and lifecycle.
    pub hint: String,
    pub show_parent_hint: bool,
    pub ignore_hint_sensing: bool,
    pub kind: i64,
    /// Drawing clip (krkr setClip) in image pixels; `None` is the whole image.
    pub draw_clip: Option<[i64; 4]>,
    pub image_source: Option<crysirror_core::image_processing::ImageInput>,
    pub source_lost: String,
    pub regions: crysirror_core::region_map::RegionMap,
    pub province_allocated: bool,
    pub slot: u32,
    pub window: usize,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub left: i64,
    pub top: i64,
    pub width: i64,
    pub height: i64,
    pub image_left: i64,
    pub image_top: i64,
    pub image_width: i64,
    pub image_height: i64,
    pub visible: bool,
    pub opacity: i64,
    pub absolute: i64,
    pub absolute_order_mode: bool,
    /// Nothing has been drawn since the surface was cleared to transparency,
    /// so source-over and replace are the same operation.
    pub full_alpha: Option<u8>,
    pub image_modified: bool,
    pub has_image: bool,
    pub blank: bool,
    /// Engine nodes exist (false after invalidation of the layer or an ancestor).
    pub attached: bool,
    pub major: i32,
    pub dirty: bool,
    pub presented: Option<(i32, [i64; 2])>,
}

impl LayerState {
    pub fn container(&self) -> u32 {
        self.slot * 2
    }
    pub fn presentation(&self) -> u32 {
        self.slot * 2 + 1
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowState {
    /// The desktop output is selected at construction, independently of heap
    /// handle ordering (GC may reuse a lower handle for a hidden helper).
    pub output: bool,
    pub menu: Option<usize>,
    pub primary: Option<usize>,
    pub(crate) views: Vec<crate::d2d_view::View>,
    pub(crate) device_zoom: Option<f64>,
    pub inner: [i64; 2],
    pub layer_pos: [i64;2],
    pub visible: bool,
}

impl WindowState {
    pub(crate) fn append_roots(&self, roots: &mut Vec<usize>) {
        if self.views.is_empty() {
            roots.extend(self.primary);
            return;
        }
        let mut ordered: Vec<_> = self.primary.into_iter().map(|id| (0, id))
            .chain(self.views.iter().map(|view| (view.order, view.layer))).collect();
        ordered.sort_by_key(|(order, _)| *order);
        roots.extend(ordered.into_iter().map(|(_, id)| id));
    }
}

#[derive(Clone, Debug)]
pub struct TimerState {
    pub mode: u8,
    pub interval: u64,
    pub enabled: bool,
    pub capacity: u64,
    pub next: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Batch {
    Ui(Vec<Value>),
    Layers(Vec<Value>),
    Canvases(Vec<Value>),
}

/// Let the host pump input between expensive consecutive native image calls.
/// The interrupted instruction has not yet changed its destination.
#[derive(Default)]
pub(crate) struct BitmapWorkSlice {
    started: Option<web_time::Instant>,
    probe_started: Option<web_time::Instant>,
    probes: u32,
    pub parked: bool,
}
impl BitmapWorkSlice {
    fn checkpoint(&mut self) -> bool {
        let started = self.started.get_or_insert_with(web_time::Instant::now);
        self.parked = started.elapsed() >= std::time::Duration::from_millis(6);
        self.parked
    }
}

pub struct KrkrHost {
    pub(crate) bitmap_work: Option<BitmapWorkSlice>,
    pub(crate) pads:crate::native_pad::State,
    pub(crate) extra: crate::natives::native_extra::State,
    pub(crate) attention_dirty: bool,
    pub(crate) transitions: crate::transitions::State,
    pub(crate) protect: crate::protect_plugin::State,
    /// Companion executable name reported as `System.exeName`.
    pub executable: Option<String>,
    /// Configured user locale for getLangName.dll.
    pub locale: Option<String>,
    /// KAGParserEx linked: new parsers use the extended format.
    pub kag_ex: bool,
    /// Decoder capability names validated against this client's shared host.
    pub(crate) audio_codecs: BTreeSet<String>,
    pub(crate) ext_kag: bool,
    pub(crate) text_render: crate::text_render_plugin::State,
    pub(crate) motion: crate::motion_plugin::State,
    pub(crate) csv: crate::csv_plugin::State,
    pub(crate) line_parser: crate::line_plugin::State,
    pub(crate) psbfile: crate::psbfile_plugin::State,
    pub(crate) psd: crate::psd_plugin::Psd,
    pub(crate) draws: crate::draw_plugin::State,
    /// Target, owner-polled job, native flips, whether to replace Canvas paint.
    /// Unmodified Asset preparation retains its original immutable paint.
    pub image_job: Option<(usize, crysirror_core::image_processing::ImageJob, [bool; 2], bool)>,
    pub composite_pending: Vec<crate::image_api::PendingComposite>,
    pub(crate) bitmap_batch: Option<crate::image_api::BitmapBatch>,
    pub(crate) bitmap_redraw_pending: bool,
    pub(crate) image_pair:Option<crate::image_api::PairPreparation>,
    pub(crate) image_save:Option<crate::image_api::LayerImageSave>,
    /// End publication caller ownership after ordered Canvas edits commit.
    /// Engine retained draws and frozen copies then own the resource lease.
    pub(crate) retired_images: Vec<String>,
    pub io_barrier: bool,
    pub writes: std::collections::VecDeque<Box<dyn crysirror_core::service::StorageWrite>>,
    pub can_suspend: bool,
    /// `System.eventDisabled` (krkrz TVPSystemEventDisabledState): input and
    /// timer events are not delivered while set.
    pub event_disabled: bool,
    /// `System.exeBits`; KiriKiri titles without a readable PE are treated as
    /// the classic 32-bit executable.
    pub exe_bits: i64,
    pub sounds: crate::sound::Sounds,
    pub(crate) get_sample: crate::get_sample::GetSample,
    pub(crate) wave_data: crate::wv_decoder::State,
    pub(crate) multitrack: crate::multitrack::Registry,
    pub(crate) http: crate::http_plugin::Http,
    pub videos: crate::video::Videos,
    pub alpha_movies: crate::alpha_movie::AlphaMovies,
    pub layer_movies: crate::layer_movie::LayerMovies,
    pub(crate) cursors: crate::pointer::Cursors,
    pub client: u32,
    pub storage: Option<Storage>,
    pub images: Option<Arc<dyn ImageMetadata>>,
    pub(crate) pending_image_metadata: Option<String>,
    pub(crate) image_metadata_failure: Option<(String, String)>,
    pub(crate) province_load: Option<crate::province::Pending>,
    pub text_measurement: Option<Arc<dyn TextMeasurement>>,
    pub(crate) fonts:crate::font_resources::Fonts,
    pub(crate) hint_hover: Option<usize>,
    pub(crate) mouse: crate::native_mouse::MouseState,
    pub input: crysirror_core::service::InputFrame,
    /// Latest host key levels, independent of an older queued event callback.
    pub(crate) held_buttons: u64,
    pub(crate) pressed_since_query: u64,
    pub now: u64,
    /// The presenting host's display (`System.screenWidth`, windowEx monitors);
    /// `None` falls back to the canvas.
    pub display: Option<crysirror_core::service::DisplayInfo>,
    pub canvas: [u32; 2],
    pub font: String,
    /// Native Font.rasterizer preference; portable engine rendering is shared.
    pub font_rasterizer_preference: i64,
    pub log: Vec<String>,
    pub batches: Vec<Batch>,
    pub layers: BTreeMap<usize, LayerState>,
    /// Reverse index for incoming bitmap commands, including reused native slots.
    slot_owners: Vec<Option<usize>>,
    pub(crate) layer_instances: BTreeSet<usize>,
    pub(crate) fftgraph:crate::fftgraph::Graph,
    pub windows: BTreeMap<usize, WindowState>,
    pub timers: BTreeMap<usize, TimerState>,
    pub deferred: crysirror_core::deferred_dispatch::DeferredDispatch,
    pub parsers: BTreeMap<usize, crate::kag::ParserState>,
    pub menus: BTreeMap<usize, crate::menus::MenuState>,
    pub menus_last: Option<Value>,
    pub menu_popups: Vec<(usize, [f32; 2])>,
    /// Bumped by every menu state change; `menus::sync` rebuilds specs only
    /// when it (or a menu window's visibility/canvas) changed.
    pub menus_revision: u64,
    /// WIN32Dialog.messageBox waiting on an engine dialog: (dialog id).
    pub dialog_pending: Option<u32>,
    pub(crate) input_prompt: Option<crate::edit_dialog::Prompt>,
    pub(crate) prompt_release_mask: u64,
    pub dialog_serial: u32,
    pub menus_synced: Option<(u64, Vec<(usize, bool)>, [u32; 2])>,
    pub loaded_plugins: BTreeSet<String>,
    pub plugin_shells: BTreeSet<String>,
    free_slots: BTreeSet<u32>,
    next_slot: u32,
    pub exit_requested: bool,
    /// krkrz `ModalLayerVector`: layers made modal by `Layer.setMode`, the
    /// last one current.
    pub modal_layers: Vec<usize>,
    /// krkrz `CanCloseWork`: the answer `Window.onCloseQuery` gives to a
    /// script `close()` (set by the native handler, `super.onCloseQuery`).
    pub close_decision: bool,
    /// Program close awaits the complete onCloseQuery script, including a
    /// parked modal caller. Opening a confirmation is not an accepted close.
    pub(crate) program_close_pending: bool,
    /// A user close (window close button) waiting for its onCloseQuery.
    pub user_close_pending: bool,
    /// The onCloseQuery being answered is a user close (`!ProgramClosing`).
    pub closing_by_user: bool,
    pub api_calls: u64,
    /// Bound polling of the simulation clock without spinning inside a TJS
    /// timer. A parked native read resumes only after the host clock advances.
    pub clock_poll_tick: u64,
    pub clock_poll_reads: u16,
    pub clock_yield_tick: Option<u64>,
    pub(crate) modal_windows: Vec<crate::adapter::modal_window::Modal>,
    pub(crate) modal_serial: u32,
    pub(crate) pending_menu_events: std::collections::VecDeque<crysirror_scene::ui::UiEvent>,
}

impl KrkrHost {
    pub(crate) fn before_image_work(&mut self) -> TjsResult<()> {
        if self.can_suspend && self.bitmap_work.as_mut().is_some_and(BitmapWorkSlice::checkpoint) {
            let _defer = crysirror_tasks::profile::span_current("krkr.bitmap.defer");
            self.bitmap_redraw_pending = true;
            return Err(TjsError::resource_pending("continue native image work on the next host tick"));
        }
        Ok(())
    }

    pub(crate) fn before_batch_item(&mut self) -> TjsResult<()> {
        if self.can_suspend {
            if let Some(work) = &mut self.bitmap_work {
                let start = work.probe_started.get_or_insert_with(web_time::Instant::now);
                work.probes = work.probes.saturating_add(1);
                if work.probes % 8 == 0 && start.elapsed() >= std::time::Duration::from_millis(6) {
                    work.parked = true;
                    self.bitmap_redraw_pending = true;
                    return Err(TjsError::resource_pending("continue native batch on the next host tick"));
                }
            }
        }
        Ok(())
    }
    pub fn new(canvas: [u32; 2], font: String) -> Self {
        Self {
            extra: Default::default(),
            pads:Default::default(),
            attention_dirty: false,
            protect: Default::default(),
            executable: None,
            locale: None,
            kag_ex: false,
            audio_codecs: BTreeSet::new(),
            ext_kag: false,
            text_render: Default::default(),
            motion: Default::default(),
            transitions: Default::default(),
            csv: Default::default(),
            line_parser: Default::default(),
            psbfile: Default::default(),
            psd: Default::default(),
            image_job: None,
            composite_pending: Vec::new(),
            bitmap_batch: None,
            bitmap_redraw_pending: false,
            image_pair:None,
            image_save:None,
            retired_images: Vec::new(),
            draws:Default::default(),
            io_barrier: false,
            writes: Default::default(),
            can_suspend: true,
            event_disabled: false,
            exe_bits: 32,
            sounds: Default::default(),
            get_sample: Default::default(),
            wave_data: Default::default(),
            multitrack: Default::default(),
            http: Default::default(),
            videos: Default::default(),
            alpha_movies: Default::default(),
            layer_movies: Default::default(),
            cursors: Default::default(),
            client: 1,
            storage: None,
            images: None,
            pending_image_metadata: None,
            image_metadata_failure: None,
            province_load: None,
            text_measurement: None,
            fonts:Default::default(),
            hint_hover: None,
            mouse: Default::default(),
            input: Default::default(),
            held_buttons: 0,
            pressed_since_query: 0,
            now: 0,
            display: None,
            canvas,
            font,
            font_rasterizer_preference: 1,
            log: Vec::new(),
            batches: Vec::new(),
            layers: BTreeMap::new(),
            slot_owners: vec![None],
            layer_instances: BTreeSet::new(),
            fftgraph:Default::default(),
            windows: BTreeMap::new(),
            timers: BTreeMap::new(),
            parsers: BTreeMap::new(),
            deferred: crysirror_core::deferred_dispatch::DeferredDispatch::new(4096)
                .expect("fixed capacity"),
            menus: BTreeMap::new(),
            menus_last: None,
            menu_popups: vec![],
            menus_revision: 0,
            dialog_pending: None,
            input_prompt: None,
            prompt_release_mask: 0,
            dialog_serial: 0,
            menus_synced: None,
            loaded_plugins: BTreeSet::new(),
            plugin_shells: BTreeSet::new(),
            free_slots: BTreeSet::new(),
            next_slot: 1,
            exit_requested: false,
            modal_layers: Vec::new(),
            close_decision: false,
            program_close_pending: false,
            user_close_pending: false,
            closing_by_user: false,
            api_calls: 0,
            bitmap_work: None,
            clock_poll_tick: 0,
            clock_poll_reads: 0,
            clock_yield_tick: None,
            modal_windows: Vec::new(),
            modal_serial: 0,
            pending_menu_events: Default::default(),
        }
    }

    fn begin_native_write(&mut self, name: &str, data: Vec<u8>) -> TjsResult<()> {
        if !self.io_barrier || !self.can_suspend {
            return Err(TjsError::runtime(
                "native write requires a resumable saveStruct boundary",
            ));
        }
        if data.len() > 4 * 1024 * 1024 || self.writes.len() >= 64 {
            return Err(TjsError::runtime("native write budget"));
        }
        let write = self
            .storage()?
            .begin_write(name, data.into())
            .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
        self.writes.push_back(write);
        Ok(())
    }
    pub fn push_log(&mut self, line: String) {
        // Developer diagnostic: append every VM log line to a local file.
        if let Some(path) = std::env::var_os("CRYSIRROR_KRKR_LOG_FILE") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "[{} ms] {line}", self.now);
            }
        }
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(line);
    }

    pub fn canvas_op(&mut self, mut op: Value) {
        // A motion paints the source layer as a puppet mesh, not into its pixels. Scripts
        // that copy that hidden layer onto a visible one (AffineSourceMotion) must receive
        // the composed mesh, so such a copy goes through the shared subtree snapshot.
        if op["op"].as_str() == Some("copy") {
            let motion_source = op["source"].as_u64().and_then(|n| usize::try_from(n).ok())
                .and_then(|slot| self.slot_owners.get(slot).copied().flatten())
                .filter(|owner| self.motion.has_frame(*owner))
                .and_then(|owner| self.layers.get(&owner)).map(LayerState::container);
            if let Some(container) = motion_source {
                op = serde_json::json!({"op":"copy_subtree","source":container,"destination":op["destination"],
                    "rect":op["rect"],"position":op["position"],"clip":op.get("clip").cloned().unwrap_or(Value::Null)});
            }
        }
        if matches!(
            op["op"].as_str(),
            Some("clear" | "set" | "draw" | "copy" | "create")
        ) {
            if let Some(id) = op
                .get("destination")
                .or_else(|| op.get("id"))
                .and_then(Value::as_u64)
            {
                if let Some(l) = usize::try_from(id).ok().and_then(|s| self.slot_owners.get(s).copied().flatten())
                    .and_then(|owner| self.layers.get_mut(&owner)) {
                    l.image_modified = true;
                    l.image_source = None;
                    // Developer diagnostic (CRYSIRROR_KRKR_TRACE_SOURCE).
                    if trace_source() {
                        l.source_lost = op.to_string().chars().take(300).collect();
                    }
                    match op["op"].as_str() {
                        Some("clear") => {
                            if let Ok(color)=serde_json::from_value::<[u8;4]>(op["color"].clone()){
                                l.image_source=Some(crysirror_core::image_processing::ImageInput::Solid{size:[l.image_width as u32,l.image_height as u32],color});
                            }
                            l.full_alpha =
                                op["color"][3].as_u64().and_then(|v| u8::try_from(v).ok())
                        }
                        Some("draw") if l.full_alpha == Some(255) => (),
                        _ => l.full_alpha = None,
                    }
                }
            }
        }
        match self.batches.last_mut() {
            Some(Batch::Canvases(ops)) => ops.push(op),
            _ => self.batches.push(Batch::Canvases(vec![op])),
        }
    }

    pub fn layer_op(&mut self, op: Value) {
        match self.batches.last_mut() {
            Some(Batch::Layers(ops)) => ops.push(op),
            _ => self.batches.push(Batch::Layers(vec![op])),
        }
    }

    pub fn ui_op(&mut self, op: Value) {
        match self.batches.last_mut() {
            Some(Batch::Ui(ops)) => ops.push(op),
            _ => self.batches.push(Batch::Ui(vec![op])),
        }
    }
    pub fn storage(&self) -> TjsResult<&Storage> {
        self.storage
            .as_ref()
            .ok_or_else(|| TjsError::runtime("KiriKiri storage is not mounted yet"))
    }

    pub fn storage_mut(&mut self) -> TjsResult<&mut Storage> {
        self.storage
            .as_mut()
            .ok_or_else(|| TjsError::runtime("KiriKiri storage is not mounted yet"))
    }

    /// Children of different primary layers belong to different KiriKiri layer managers.
    pub(crate) fn same_manager(&self, a: usize, b: usize) -> bool {
        let (Some(la), Some(lb)) = (self.layers.get(&a), self.layers.get(&b)) else { return false };
        la.window == lb.window && self.windows.get(&la.window).is_some_and(|w| w.views.iter().all(|v|
            self.is_ancestor_or_self(a, v.layer) == self.is_ancestor_or_self(b, v.layer)))
    }

    /// krkrz `IsAncestorOrSelf(ancestor)`: `ancestor` is `h` or above it.
    pub fn is_ancestor_or_self(&self, h: usize, ancestor: usize) -> bool {
        let mut node = Some(h);
        let mut depth = 0;
        while let Some(id) = node {
            if id == ancestor {
                return true;
            }
            depth += 1;
            if depth > 256 {
                return false;
            }
            node = self.layers.get(&id).and_then(|l| l.parent);
        }
        false
    }
    /// krkrz `IsDisabledByMode`: outside the current modal layer's subtree.
    pub fn disabled_by_mode(&self, h: usize) -> bool {
        self.modal_windows.last().is_some_and(|modal|self.layers.get(&h).is_none_or(|layer|layer.window!=modal.window)) || self.modal_layers
            .last()
            .is_some_and(|current| !self.is_ancestor_or_self(h, *current))
    }
    pub fn layer(&self, h: ObjectHandle) -> TjsResult<&LayerState> {
        self.layers
            .get(&h.0)
            .ok_or_else(|| TjsError::runtime("object is not a constructed Layer"))
    }

    pub fn layer_mut(&mut self, h: ObjectHandle) -> TjsResult<&mut LayerState> {
        self.layers
            .get_mut(&h.0)
            .ok_or_else(|| TjsError::runtime("object is not a constructed Layer"))
    }

    /// WindowIntf's obsolete setLayerPos is a presentation offset, not a
    /// mutation of the native primary layer's position or its bitmap.
    pub(crate) fn set_layer_position(&mut self,window:usize,position:[i64;2])->TjsResult<()> {
        if position.iter().any(|n|i32::try_from(*n).is_err()){return Err(TjsError::runtime("native layer viewport range"));}
        let w=self.windows.get_mut(&window).ok_or_else(||TjsError::runtime("Window not constructed"))?;
        if w.layer_pos==position{return Ok(());}
        w.layer_pos=position;
        for id in w.primary.into_iter().chain(w.views.iter().map(|v|v.layer)) {
            if let Some(layer) = self.layers.get_mut(&id) { layer.dirty = true; }
        }
        Ok(())
    }
    /// Register a layer and create its engine container, surface and
    /// presentation. The parent's container already exists, so the queued
    /// order of these commands is valid.
    pub fn create_layer(
        &mut self,
        handle: ObjectHandle,
        window: usize,
        parent: Option<usize>,
    ) -> TjsResult<()> {
        self.allocate_layer(handle, window, parent, true)
    }
    pub(crate) fn reserve_layer(&mut self, handle: ObjectHandle, window: usize) -> TjsResult<()> {
        self.allocate_layer(handle, window, None, false)
    }
    pub(crate) fn bind_reserved_layer(&mut self, handle: ObjectHandle, window: usize, parent: Option<usize>) -> TjsResult<()> {
        let layer=self.layer(handle)?;
        if layer.constructed || layer.window != window {
            return Err(TjsError::runtime("invalid pending Layer binding"));
        }
        if let Some(parent)=parent {
            let mut cursor=Some(parent);
            for _ in 0..64 {
                let Some(id)=cursor else {break;};
                if id==handle.0 {return Err(TjsError::runtime("native parent cycle"));}
                let node=self.layers.get(&id).ok_or_else(||TjsError::runtime("Layer parent absent"))?;
                if node.window!=window || !node.attached {return Err(TjsError::runtime("invalid Layer parent scope"));}
                cursor=node.parent;
            }
            if cursor.is_some() {return Err(TjsError::runtime("native parent depth"));}
            self.layers.get_mut(&parent).unwrap().children.push(handle.0);
        }
        let layer=self.layer_mut(handle)?;
        layer.parent=parent;
        layer.constructed=true;
        layer.dirty=true;
        layer.presented=None;
        Ok(())
    }
    fn allocate_layer(&mut self, handle: ObjectHandle, window: usize, parent: Option<usize>, constructed: bool) -> TjsResult<()> {
        let slot = match self.free_slots.pop_first() {
            Some(slot) => slot,
            None => {
                if self.next_slot > MAX_LAYERS {
                    return Err(TjsError::runtime(format!(
                        "KiriKiri layer limit {MAX_LAYERS} reached"
                    )));
                }
                self.next_slot += 1;
                self.next_slot - 1
            }
        };
        let primary = constructed && parent.is_none();
        let size = DEFAULT_LAYER_SIZE as i64;
        let state = LayerState {
            constructed,
            hint: String::new(),
            show_parent_hint: true,
            ignore_hint_sensing: false,
            kind: if primary { 1 } else { 2 },
            image_source: None,
            source_lost: String::new(),
            regions: crysirror_core::region_map::RegionMap::new(
                self.client,
                [size as u32, size as u32],
            )
            .map_err(|e| TjsError::runtime(e.to_string()))?,
            province_allocated: false,
            slot,
            window,
            parent,
            children: Vec::new(),
            left: 0,
            top: 0,
            width: size,
            height: size,
            image_left: 0,
            image_top: 0,
            image_width: size,
            image_height: size,
            visible: primary,
            opacity: 255,
            absolute: 0,
            absolute_order_mode: false,
            full_alpha: Some(0),
            image_modified: true,
            has_image: true,
            blank: true,
            attached: true,
            major: 0,
            dirty: true,
            presented: None,
            draw_clip: None,
        };
        let mut parent_container = None;
        if let Some(p) = parent {
            let parent_state = self
                .layers
                .get_mut(&p)
                .ok_or_else(|| TjsError::runtime("Layer parent is not a constructed Layer"))?;
            if !parent_state.attached {
                return Err(TjsError::runtime("Layer parent was invalidated"));
            }
            parent_state.children.push(handle.0);
            parent_container = Some(parent_state.container());
        }
        let shown = self.windows.get(&window).is_some_and(|w| w.visible) || !primary;
        if window!=0{self.layer_op(
            json!({"op":"put","layer":container_spec(&state, 0, parent_container, shown)}),
        );}
        self.canvas_op(json!({"op":"create","id":slot,"width":size,"height":size}));
        self.canvas_op(json!({"op":"clear","id":slot,"color":[255,255,255,0]}));
        if window!=0{self.canvas_op(
            json!({"op":"present","id":slot,"layer":state.presentation(),
            "order":0,"parent":state.container(),"position":[0,0]}),
        );}
        let mut state = state;
        state.presented = (window!=0).then_some((0, [0, 0]));
        if self.slot_owners.len() <= slot as usize { self.slot_owners.resize(slot as usize + 1, None); }
        self.slot_owners[slot as usize] = Some(handle.0);
        self.layers.insert(handle.0, state);
        Ok(())
    }

    /// krkr image resize: contents stay anchored at the top-left.
    pub fn resize_surface(&mut self, h: ObjectHandle, w: i64, height: i64) -> TjsResult<()> {
        let edges = 1..=i64::from(crysirror_image::MAX_IMAGE_DIMENSION);
        if !edges.contains(&w) || !edges.contains(&height) {
            return Err(TjsError::runtime(format!(
                "invalid layer image size {w}x{height}"
            )));
        }
        let mut regions=self.layer(h)?.regions.clone();
        regions.resize(self.client,[w as u32,height as u32])
            .map_err(|e|TjsError::runtime(e.to_string()))?;
        let slot = {
            let l = self.layer_mut(h)?;
            // Canvas Resize retains the old pixels. Keep its immutable CPU
            // declaration in sync so subsequent engine image jobs see those
            // same pixels, rather than losing the source after setImageSize.
            let source = l.image_source.take().map(|input| {
                use crysirror_core::image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput};
                if l.image_width == w && l.image_height == height {
                    return input;
                }
                ImageInput::Composite {
                    base: Box::new(ImageInput::Solid { size:[w as u32,height as u32],color:[0;4] }),
                    layers: vec![ImageCompositeLayer {
                        input:Box::new(input),
                        source:[0,0,l.image_width as i32,l.image_height as i32],
                        destination:[0,0],clip:None,opacity:255,math:ImageCompositeMath::Copy,
                    }],
                }
            });
            l.regions = regions;
            l.full_alpha = None;
            l.image_modified = true;
            l.image_source = source;
            l.image_width = w;
            l.image_height = height;
            l.draw_clip = None;
            l.dirty = true;
            l.slot
        };
        self.canvas_op(json!({"op":"resize","id":slot,"width":w,"height":height}));
        self.layer_mut(h)?.presented = None;
        Ok(())
    }
    pub fn reset_surface(
        &mut self,
        h: ObjectHandle,
        w: i64,
        height: i64,
        fill: [u8; 4],
    ) -> TjsResult<()> {
        let edges = 1..=i64::from(crysirror_image::MAX_IMAGE_DIMENSION);
        if !edges.contains(&w) || !edges.contains(&height) {
            return Err(TjsError::runtime(format!(
                "invalid layer image size {w}x{height}"
            )));
        }
        let regions =
            crysirror_core::region_map::RegionMap::new(self.client, [w as u32, height as u32])
                .map_err(|e| TjsError::runtime(e.to_string()))?;
        let slot = {
            let l = self.layer_mut(h)?;
            l.regions = regions;
            l.province_allocated = false;
            l.full_alpha = Some(fill[3]);
            l.image_source=Some(crysirror_core::image_processing::ImageInput::Solid{size:[w as u32,height as u32],color:fill});
            l.has_image = true;
            l.image_modified = true;
            l.image_width = w;
            l.image_height = height;
            l.draw_clip = None;
            l.blank = fill[3] == 0;
            l.dirty = true;
            l.slot
        };
        self.canvas_op(json!({"op":"create","id":slot,"width":w,"height":height}));
        self.canvas_op(json!({"op":"clear","id":slot,"color":fill}));
        // Re-presenting after a re-create keeps the presentation bound.
        self.layer_mut(h)?.presented = None;
        Ok(())
    }

    /// Remove the engine nodes of a layer subtree (children first).
    pub fn detach_subtree(&mut self, h: usize) {
        // Invalidate marks the native object shutdown before detaching it:
        // no callback may be delivered to that retired object.
        if self.mouse.hover == Some(h) { self.mouse.hover = None; }
        if self.mouse.capture == Some(h) { self.mouse.capture = None; self.mouse.release_called = true; }
        if self.layer_movies.retire(h) { self.deferred.cancel(self.client,crate::async_trigger::target(h,3)); }
        let cancelled: Vec<_> = self.transitions.0.iter()
            .filter(|(dest, source)| **dest == h || **source == h)
            .map(|(&dest, _)| dest).collect();
        for dest in cancelled {
            let source = self.transitions.0.remove(&dest);
            self.transitions.2.remove(&dest);
            self.transitions.3.remove(&dest);
            let immediate = self.transitions.4.remove(&dest);
            let surface_only = self.transitions.1.remove(&dest);
            if immediate {
                continue;
            }
            if let Some(layer) = self.layers.get(&dest) {
                let (id, temp) = (
                    if surface_only { layer.presentation() } else { layer.container() },
                    crate::transitions::temp_presentation(layer.slot),
                );
                self.layer_op(json!({"op":"end_crossfade", "id":id}));
                if surface_only && source != Some(dest) {
                    self.canvas_op(json!({"op":"hide","layer":temp}));
                }
            }
        }

        let children = self
            .layers
            .get(&h)
            .map(|l| l.children.clone())
            .unwrap_or_default();
        for c in children {
            self.detach_subtree(c);
        }
        let Some(l) = self.layers.get_mut(&h) else {
            return;
        };
        if !l.attached {
            return;
        }
        l.attached = false;
        let hint = !std::mem::take(&mut l.hint).is_empty();
        let tooltip_id = l.slot;
        let (presentation, container) = (l.presentation(), l.container());
        if hint { self.ui_op(json!({"op":"remove_tooltip","id":tooltip_id})); }
        self.canvas_op(json!({"op":"hide","layer":presentation}));
        self.layer_op(json!({"op":"remove","id":container}));
    }

    pub fn invalidate_layer(&mut self, h: usize) {
        if self.bitmap_batch.as_ref().is_some_and(|batch| batch.target == h) {
            self.bitmap_batch = None;
        }
        self.detach_subtree(h);
        if let Some(l) = self.layers.remove(&h) {
            self.canvas_op(json!({"op":"destroy","id":l.slot}));
            if let Some(p) = l.parent.and_then(|p| self.layers.get_mut(&p)) {
                p.children.retain(|c| *c != h);
            }
            for c in &l.children {
                if let Some(child) = self.layers.get_mut(c) {
                    child.parent = None;
                }
            }
            self.slot_owners[l.slot as usize] = None;
            self.free_slots.insert(l.slot);
        }
        for w in self.windows.values_mut() {
            w.views.retain(|view| view.layer != h);
            if w.primary == Some(h) {
                w.primary = None;
            }
        }
    }

    /// Switching modes preserves painter order; entering absolute mode seeds
    /// the native absolute indices from the current sibling indices.
    pub fn set_order_mode(&mut self, h: ObjectHandle, absolute: bool) -> TjsResult<()> {
        if self.layer(h)?.absolute_order_mode == absolute {
            return Ok(());
        }
        let children = self.sorted_children(h.0);
        if absolute {
            for (index, child) in children.iter().enumerate() {
                self.layers.get_mut(child).unwrap().absolute = index as i64;
            }
        }
        let layer = self.layer_mut(h)?;
        layer.children = children;
        layer.absolute_order_mode = absolute;
        layer.dirty = true;
        Ok(())
    }

    /// Native sibling order, translated by sync into engine Layer order keys.
    pub fn sorted_children(&self, h: usize) -> Vec<usize> {
        let Some(l) = self.layers.get(&h) else {
            return vec![];
        };
        let mut children = l.children.clone();
        if l.absolute_order_mode {
            children.sort_by_key(|c| self.layers.get(c).map_or(0, |c| c.absolute));
        }
        children
    }

    /// Emit the commands that bring engine nodes up to date: painter order
    /// from a depth-first walk, container geometry and image placement.
    pub fn sync(&mut self) -> TjsResult<()> {
        let _span = crysirror_tasks::profile::span_current("krkr.layers.sync");
        crate::menus::sync(self)?;
        crate::tooltip::sync_liveness(self)?;
        let mut roots = Vec::new();
        for (_, w) in self.windows.iter().filter(|(id,_)|!self.modal_windows.iter().any(|m|m.window==**id)) {
            w.append_roots(&mut roots);
        }
        for w in self.modal_windows.iter().filter_map(|m|self.windows.get(&m.window)) {
            w.append_roots(&mut roots);
        }
        let mut order = Vec::with_capacity(self.layers.len());
        roots.reverse();
        while let Some(h) = roots.pop() {
            order.push(h);
            if let Some(layer) = self.layers.get(&h) {
                if layer.absolute_order_mode {
                    roots.extend(self.sorted_children(h).into_iter().rev());
                } else {
                    roots.extend(layer.children.iter().rev().copied());
                }
            }
        }
        let mut containers = Vec::new();
        let mut presentations = Vec::new();
        for (index, h) in order.into_iter().enumerate() {
            let major = index as i32 * 2;
            let Some(l) = self.layers.get_mut(&h) else {
                continue;
            };
            if !l.attached {
                continue;
            }
            if l.major != major {
                l.major = major;
                l.dirty = true;
            }
            if !l.dirty {
                continue;
            }
            l.dirty = false;
            let parent = l.parent;
            let window = l.window;
            let l = self.layers.get(&h).unwrap();
            let parent_container = parent
                .and_then(|p| self.layers.get(&p))
                .map(LayerState::container);
            let shown = parent.is_some() || self.windows.get(&window).is_some_and(|w| w.visible);
            let mut spec = container_spec(l, major, parent_container, shown);
            if parent.is_none() {
                if let Some(w)=self.windows.get(&window){
                    spec["rect"][0]=json!(l.left+w.layer_pos[0]);
                    spec["rect"][1]=json!(l.top+w.layer_pos[1]);
                    if let Some(view) = w.views.iter().find(|view| view.layer == h) {
                        view.present(&mut spec, l, w.layer_pos, w.device_zoom.unwrap_or(1.0));
                    }
                }
                if let Some(modal)=self.modal_windows.iter().find(|m|m.window==window){
                    spec["rect"][0]=json!(modal.offset[0]+(l.left as f32)*modal.scale);
                    spec["rect"][1]=json!(modal.offset[1]+(l.top as f32)*modal.scale);
                    spec["transform"]=json!({"scale_x":modal.scale,"scale_y":modal.scale});
                }
            }
            let l = self.layers.get_mut(&h).unwrap();
            let placement = (major + 1, [l.image_left, l.image_top]);
            // krkrz ltBinder (type 0) blits nothing of its own; its children
            // draw straight into the parent's target (LayerIntf.cpp
            // `case ltBinder: // no action`). Hiding is idempotent.
            let present = if l.kind == 0 {
                l.presented = None;
                Some(json!({"op":"hide","layer":l.presentation()}))
            } else {
                (l.presented != Some(placement)).then(|| {
                    l.presented = Some(placement);
                    json!({"op":"present","id":l.slot,"layer":l.presentation(),"order":placement.0,
                        "parent":l.container(),"position":placement.1})
                })
            };
            containers.push(json!({"op":"put","layer":spec}));
            if let Some(p) = present {
                presentations.push(p);
            }
        }
        // One owner publication must update every dirty logical container
        // before its canvas presentations. In particular the two endpoints
        // of a live subtree transition resize together: alternating layer/
        // canvas batches exposes an invalid half-updated pair to validation.
        for op in containers { self.layer_op(op); }
        for op in presentations { self.canvas_op(op); }
        Ok(())
    }

    /// Front-most visible layer containing the canvas point, with local
    /// coordinates. Hit testing uses layer bounds; per-pixel alpha
    /// thresholds are not evaluated by this adapter.
    pub fn hits(&self, x: i64, y: i64) -> Vec<(usize, i64, i64)> {
        fn visit(
            host: &KrkrHost,
            h: usize,
            offset: [i64; 2],
            point: [i64; 2],
            out: &mut Vec<(i32, usize, i64, i64)>,
        ) {
            let Some(l) = host.layers.get(&h) else { return };
            if !l.visible || !l.attached {
                return;
            }
            let local = [point[0] - offset[0] - l.left, point[1] - offset[1] - l.top];
            if local[0] < 0 || local[1] < 0 || local[0] >= l.width || local[1] >= l.height {
                return;
            }
            out.push((l.major, h, local[0], local[1]));
            for child in host.sorted_children(h) {
                visit(
                    host,
                    child,
                    [offset[0] + l.left, offset[1] + l.top],
                    point,
                    out,
                );
            }
        }
        let mut out = vec![];
        for (_,w) in self.windows.iter().filter(|(id,w)|w.visible&&self.modal_windows.last().is_none_or(|m|m.window==**id)) {
            if let Some(root) = w.primary {
                visit(self, root, w.layer_pos, [x, y], &mut out)
            }
        }
        out.sort_by_key(|v| std::cmp::Reverse(v.0));
        out.into_iter().map(|(_, h, x, y)| (h, x, y)).collect()
    }
}

pub fn container_spec(l: &LayerState, major: i32, parent: Option<u32>, shown: bool) -> Value {
    json!({
        "id": l.container(),
        "parent": parent,
        "rect": [l.left, l.top, l.width.max(1), l.height.max(1)],
        "paint": {"kind":"solid","color":[0,0,0,0]},
        "order": [major, -2],
        "clip": [0, 0, l.width.max(1), l.height.max(1)],
        "opacity": l.opacity.clamp(0, 255),
        "visible": l.constructed && l.visible && shown,
    })
}

impl TjsHost for KrkrHost {
    /// Actual owners and queued work. The Layer tree is a non-owning native
    /// index (krkrz AddChild/ObjectList::Add), not a reference to every script
    /// Layer ever constructed. Script fields and suspended frames trace theirs.
    fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        self.extra.roots(out);
        self.wave_data.roots(out);
        self.pads.roots(out);
        self.fftgraph.roots(out);
        self.http.roots(out);
        if let Some(pending)=&self.province_load {out.push(crate::province::target(pending));}
        let ids = self.windows.keys()
            .chain(self.timers.keys())
            .chain(self.parsers.keys())
            .chain(self.menus.keys())
            .chain(self.transitions.0.keys())
            .chain(self.transitions.0.values())
            .chain(self.menu_popups.iter().map(|(h, _)| h))
            .chain(self.image_job.iter().map(|(h, ..)| h));
        out.extend(ids.map(|h| ObjectHandle(*h)));
        for pending in &self.composite_pending { out.extend(pending.layers()); }
        if let Some(batch) = &self.bitmap_batch { out.push(ObjectHandle(batch.target)); }
        if let Some(pair)=&self.image_pair{out.extend([ObjectHandle(pair.target),ObjectHandle(pair.source)]);}
        if let Some(save)=&self.image_save{out.push(ObjectHandle(save.layer));}
        if let Some(prompt) = &self.input_prompt { out.push(prompt.owner); }
        out.extend(self.mouse.hover.into_iter().chain(self.mouse.capture).map(ObjectHandle));
        out.extend(self.deferred.targets(self.client).map(|t| ObjectHandle((t >> 2) as usize)));
        self.layer_movies.gc_roots(out);
        self.sounds.gc_roots(out);
        self.motion.gc_roots(out);
        for (callback, _) in self.transitions.2.values() {
            match callback {
                crysirror_tjs2::runtime::Variant::Object(h) => out.push(*h),
                crysirror_tjs2::runtime::Variant::Closure(c) => out.extend(std::iter::once(c.object).chain(c.this_obj)),
                _ => {}
            }
        }
        self.csv.gc_roots(out);
        self.line_parser.gc_roots(out);
        self.text_render.gc_roots(out);
        self.psbfile.gc_roots(out);
        self.draws.gc_roots(out);
        self.protect.gc_roots(out);
    }

    fn object_collected(&mut self, handle: ObjectHandle) {
        if let Some(widget)=self.pads.remove(handle){self.ui_op(json!({"op":"remove","id":widget}));}
        self.layer_instances.remove(&handle.0);
        // Registries that do not root their instances (see gc_roots).
        if self.layers.contains_key(&handle.0) {
            self.invalidate_layer(handle.0);
        }
        self.psbfile.invalidate(handle.0);
        self.psd.invalidate(handle.0, self.storage.as_ref());
        self.csv.invalidate(handle.0);
        self.line_parser.invalidate(handle.0);
        self.text_render.invalidate(handle.0);
    }

    fn invalidate_object(&mut self, handle: ObjectHandle) {
        self.extra.invalidate(handle);
        self.fftgraph.invalidate(handle);
        self.layer_instances.remove(&handle.0);
        if self.province_load.as_ref().is_some_and(|pending|crate::province::target(pending)==handle) {self.province_load=None;}
        self.modal_layers.retain(|l| *l != handle.0);
        self.motion.invalidate(handle.0);
        self.sounds.invalidate(handle.0);
        self.get_sample.invalidate(handle.0);
        self.wave_data.invalidate(handle.0);
        self.multitrack.retire(handle.0);
        self.http.invalidate(handle.0);
        self.draws.invalidate(handle.0);
        self.object_collected(handle);
        self.timers.remove(&handle.0);
        self.parsers.remove(&handle.0);
        for kind in 0..4 {
            self.deferred
                .cancel(self.client, crate::async_trigger::target(handle.0, kind));
        }
        self.invalidate_layer(handle.0);
        let menus = self
            .menus
            .iter()
            .filter(|(id, m)| **id == handle.0 || m.window == handle.0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in menus {
            self.menus_revision += 1;
            if let Some(menu) = self.menus.remove(&id) {
                if let Some(parent) = menu.parent.and_then(|p| self.menus.get_mut(&p)) {
                    parent.children.retain(|child| *child != id);
                }
                for child in menu.children {
                    if let Some(child) = self.menus.get_mut(&child) {
                        child.parent = None;
                    }
                }
                for window in self.windows.values_mut() {
                    if window.menu == Some(id) {
                        window.menu = None;
                    }
                }
            }
        }
        if let Some(window) = self.windows.remove(&handle.0) {
            let mut roots = Vec::new();
            window.append_roots(&mut roots);
            for root in roots {
                self.invalidate_layer(root);
            }
        }
    }

    fn read_text(&mut self, name: &str, mode: &str) -> TjsResult<String> {
        let bytes = self
            .storage()?
            .read_named(name)
            .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
        let (_, offset) = split_stream_offset(mode)?;
        let bytes = bytes.get(offset..).ok_or_else(|| {
            TjsError::runtime(format!("{name}: text stream offset {offset} is past the file end"))
        })?;
        decode_text(bytes).map_err(|e| TjsError::runtime(format!("{name}: {e:#}")))
    }

    fn read_binary(&mut self, name: &str, _mode: &str) -> TjsResult<Vec<u8>> {
        self.storage()?
            .read_named(name)
            .map_err(|e| TjsError::runtime(format!("{e:#}")))
    }

    fn write_text(&mut self, name: &str, mode: &str, text: &str) -> TjsResult<()> {
        let (codec, offset) = split_stream_offset(mode)?;
        let encoded = encode_native_text(text, &codec)?;
        if offset == 0 {
            return self.begin_native_write(name, encoded);
        }
        // `o<N>` keeps the file's first N bytes: KAG bookmarks are a thumbnail image
        // with the save data behind it. Atomic writes make this one whole-file commit.
        if !self.writes.is_empty() {
            return Err(TjsError::runtime(
                "text stream offset write requires its prefix to be committed first",
            ));
        }
        let mut data = self
            .storage()?
            .read_named(name)
            .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
        data.resize(offset, 0);
        data.extend_from_slice(&encoded);
        self.begin_native_write(name, data)
    }
    fn write_binary(&mut self, name: &str, mode: &str, bytes: &[u8]) -> TjsResult<()> {
        if mode != "b" {
            return Err(TjsError::runtime(
                "native binary stream mode is unsupported",
            ));
        }
        self.begin_native_write(name, bytes.to_vec())
    }
    fn now_millis(&mut self) -> i64 {
        self.now as i64
    }

    fn log(&mut self, message: &str) {
        self.push_log(message.to_owned());
    }
}

/// `CRYSIRROR_KRKR_TRACE_SOURCE`: record which canvas op dropped a layer's
/// CPU image source.
fn trace_source() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CRYSIRROR_KRKR_TRACE_SOURCE").is_some())
}

impl KrkrHost {
    /// Screen size and work area: the host display, else the canvas.
    pub fn display_rects(&self) -> ([i64; 2], [i64; 4]) {
        match self.display {
            Some(d) => (
                [i64::from(d.screen[0]), i64::from(d.screen[1])],
                d.work.map(i64::from),
            ),
            None => {
                let [w, h] = self.canvas.map(i64::from);
                ([w, h], [0, 0, w, h])
            }
        }
    }
}

/// Split krkr's `o<N>` stream offset out of a text mode, leaving the codec letters.
pub(crate) fn split_stream_offset(mode: &str) -> TjsResult<(String, usize)> {
    let Some((codec, rest)) = mode.split_once('o') else {
        return Ok((mode.to_owned(), 0));
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let offset = rest[..digits]
        .parse::<usize>()
        .map_err(|_| TjsError::runtime(format!("text stream offset in mode {mode:?}")))?;
    Ok((format!("{codec}{}", &rest[digits..]), offset))
}

/// Native mode-string policy around the reusable text stream codec.
pub(crate) fn encode_native_text(text:&str,mode:&str)->TjsResult<Vec<u8>> {
    use crysirror_resource::text_stream::TextWriteMode;
    let encoding=match mode {
        ""=>TextWriteMode::PlainUtf16,
        "c"|"c1"=>TextWriteMode::Cipher1,
        "c2"|"z"=>TextWriteMode::Deflate{level:6},
        mode if mode.len()==2&&mode.starts_with('z')&&mode.as_bytes()[1].is_ascii_digit()=>TextWriteMode::Deflate{level:mode.as_bytes()[1]-b'0'},
        _=>return Err(TjsError::runtime("native text offset/unknown compression mode is unsupported")),
    };
    crysirror_resource::text_stream::encode(text,encoding,64*1024*1024).map_err(|e|TjsError::runtime(format!("text stream encoding: {e:#}")))
}
#[cfg(test)]
mod stream_offset_tests {
    use super::split_stream_offset;
    #[test]
    fn offset_is_split_from_the_codec_letters() {
        assert_eq!(split_stream_offset("zo32454").unwrap(), ("z".into(), 32454));
        assert_eq!(split_stream_offset("o12z").unwrap(), ("z".into(), 12));
        assert_eq!(split_stream_offset("z9").unwrap(), ("z9".into(), 0));
        assert_eq!(split_stream_offset("").unwrap(), ("".into(), 0));
        assert!(split_stream_offset("zo").is_err());
    }
}
