// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Handle translation between KiriKiri native objects and engine services.
//!
//! A KiriKiri `Layer` is one engine Layer node (position, size, clip,
//! visibility, opacity and painter order) with one engine Canvas surface
//! presented beneath it for its image. Drawing calls become Canvas commands;
//! no pixels are composed here. Commands are queued while TJS runs and
//! submitted in order by the adapter after each script entry point returns.
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
/// Canvas surface ids 1..=MAX_LAYERS; engine layer ids are derived from them.
pub const MAX_LAYERS: u32 = 1000;
const MAX_LOG: usize = 4096;

#[derive(Clone, Debug)]
pub struct LayerState {
    pub kind: i64,
    /// Drawing clip (krkr setClip) in image pixels; `None` is the whole image.
    pub draw_clip: Option<[i64; 4]>,
    pub image_source: Option<crysirror_core::image_processing::ImageInput>,
    pub source_lost: String,
    pub regions: crysirror_core::region_map::RegionMap,
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
    pub menu: Option<usize>,
    pub primary: Option<usize>,
    pub inner: [i64; 2],
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct TimerState {
    pub mode: u8,
    pub interval: u64,
    pub enabled: bool,
    pub capacity: u64,
    pub next: u64,
}

#[derive(Debug)]
pub enum Batch {
    Ui(Vec<Value>),
    Layers(Vec<Value>),
    Canvases(Vec<Value>),
}

pub struct KrkrHost {
    pub(crate) transitions: crate::transitions::State,
    pub(crate) protect: crate::protect_plugin::State,
    /// Companion executable name reported as `System.exeName`.
    pub executable: Option<String>,
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
    pub(crate) draws: crate::draw_plugin::State,
    pub image_job: Option<(usize, crysirror_core::image_processing::ImageJob, [bool; 2])>,
    pub io_barrier: bool,
    pub writes: std::collections::VecDeque<Box<dyn crysirror_core::service::StorageWrite>>,
    pub can_suspend: bool,
    pub sounds: crate::sound::Sounds,
    pub videos: crate::video::Videos,
    pub alpha_movies: crate::alpha_movie::AlphaMovies,
    pub(crate) cursors: crate::pointer::Cursors,
    pub client: u32,
    pub storage: Option<Storage>,
    pub images: Option<Arc<dyn ImageMetadata>>,
    pub text_measurement: Option<Arc<dyn TextMeasurement>>,
    pub(crate) fonts:crate::font_resources::Fonts,
    pub input: crysirror_core::service::InputFrame,
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
    /// A user close (window close button) waiting for its onCloseQuery.
    pub user_close_pending: bool,
    /// The onCloseQuery being answered is a user close (`!ProgramClosing`).
    pub closing_by_user: bool,
    pub api_calls: u64,
}

impl KrkrHost {
    pub fn new(canvas: [u32; 2], font: String) -> Self {
        Self {
            protect: Default::default(),
            executable: None,
            kag_ex: false,
            audio_codecs: BTreeSet::new(),
            ext_kag: false,
            text_render: Default::default(),
            motion: Default::default(),
            transitions: Default::default(),
            csv: Default::default(),
            line_parser: Default::default(),
            psbfile: Default::default(),
            image_job: None,
            draws:Default::default(),
            io_barrier: false,
            writes: Default::default(),
            can_suspend: true,
            sounds: Default::default(),
            videos: Default::default(),
            alpha_movies: Default::default(),
            cursors: Default::default(),
            client: 1,
            storage: None,
            images: None,
            text_measurement: None,
            fonts:Default::default(),
            input: Default::default(),
            now: 0,
            display: None,
            canvas,
            font,
            font_rasterizer_preference: 1,
            log: Vec::new(),
            batches: Vec::new(),
            layers: BTreeMap::new(),
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
            dialog_serial: 0,
            menus_synced: None,
            loaded_plugins: BTreeSet::new(),
            plugin_shells: BTreeSet::new(),
            free_slots: BTreeSet::new(),
            next_slot: 1,
            exit_requested: false,
            modal_layers: Vec::new(),
            close_decision: false,
            user_close_pending: false,
            closing_by_user: false,
            api_calls: 0,
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
                let _ = writeln!(f, "{line}");
            }
        }
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(line);
    }

    pub fn canvas_op(&mut self, op: Value) {
        if matches!(
            op["op"].as_str(),
            Some("clear" | "set" | "draw" | "copy" | "create")
        ) {
            if let Some(id) = op
                .get("destination")
                .or_else(|| op.get("id"))
                .and_then(Value::as_u64)
            {
                if let Some(l) = self.layers.values_mut().find(|l| l.slot as u64 == id) {
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
        self.modal_layers
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

    /// Register a layer and create its engine container, surface and
    /// presentation. The parent's container already exists, so the queued
    /// order of these commands is valid.
    pub fn create_layer(
        &mut self,
        handle: ObjectHandle,
        window: usize,
        parent: Option<usize>,
    ) -> TjsResult<()> {
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
        let primary = parent.is_none();
        let size = DEFAULT_LAYER_SIZE as i64;
        let state = LayerState {
            kind: if primary { 1 } else { 2 },
            image_source: None,
            source_lost: String::new(),
            regions: crysirror_core::region_map::RegionMap::new(
                self.client,
                [size as u32, size as u32],
            )
            .map_err(|e| TjsError::runtime(e.to_string()))?,
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
        self.layer_op(
            json!({"op":"put","layer":container_spec(&state, 0, parent_container, shown)}),
        );
        self.canvas_op(json!({"op":"create","id":slot,"width":size,"height":size}));
        self.canvas_op(json!({"op":"clear","id":slot,"color":[255,255,255,0]}));
        self.canvas_op(
            json!({"op":"present","id":slot,"layer":state.presentation(),
            "order":0,"parent":state.container(),"position":[0,0]}),
        );
        let mut state = state;
        state.presented = Some((0, [0, 0]));
        self.layers.insert(handle.0, state);
        Ok(())
    }

    /// krkr image resize: contents stay anchored at the top-left.
    pub fn resize_surface(&mut self, h: ObjectHandle, w: i64, height: i64) -> TjsResult<()> {
        if !(1..=8192).contains(&w) || !(1..=8192).contains(&height) {
            return Err(TjsError::runtime(format!(
                "invalid layer image size {w}x{height}"
            )));
        }
        let regions =
            crysirror_core::region_map::RegionMap::new(self.client, [w as u32, height as u32])
                .map_err(|e| TjsError::runtime(e.to_string()))?;
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
        if !(1..=8192).contains(&w) || !(1..=8192).contains(&height) {
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
            l.full_alpha = Some(fill[3]);
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
        let cancelled: Vec<_> = self.transitions.0.iter()
            .filter(|(dest, source)| **dest == h || **source == h)
            .map(|(&dest, _)| dest).collect();
        for dest in cancelled {
            self.transitions.0.remove(&dest);
            self.transitions.2.remove(&dest);
            let surface_only = self.transitions.1.remove(&dest);
            if let Some(layer) = self.layers.get(&dest) {
                let (id, temp) = (
                    if surface_only { layer.presentation() } else { layer.container() },
                    crate::transitions::temp_presentation(layer.slot),
                );
                self.layer_op(json!({"op":"end_crossfade", "id":id}));
                if surface_only {
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
        let (presentation, container) = (l.presentation(), l.container());
        self.canvas_op(json!({"op":"hide","layer":presentation}));
        self.layer_op(json!({"op":"remove","id":container}));
    }

    pub fn invalidate_layer(&mut self, h: usize) {
        self.detach_subtree(h);
        if let Some(l) = self.layers.remove(&h) {
            if let Some(p) = l.parent.and_then(|p| self.layers.get_mut(&p)) {
                p.children.retain(|c| *c != h);
            }
            for c in &l.children {
                if let Some(child) = self.layers.get_mut(c) {
                    child.parent = None;
                }
            }
            self.free_slots.insert(l.slot);
        }
        for w in self.windows.values_mut() {
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
        crate::menus::sync(self)?;
        let roots: Vec<usize> = self.windows.values().filter_map(|w| w.primary).collect();
        let mut order = Vec::new();
        for root in roots {
            let mut stack = vec![root];
            while let Some(h) = stack.pop() {
                order.push(h);
                let mut children = self.sorted_children(h);
                children.reverse();
                stack.extend(children);
            }
        }
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
            let spec = container_spec(l, major, parent_container, shown);
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
            self.layer_op(json!({"op":"put","layer":spec}));
            if let Some(p) = present {
                self.canvas_op(p);
            }
        }
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
        for w in self.windows.values().filter(|w| w.visible) {
            if let Some(root) = w.primary {
                visit(self, root, [0, 0], [x, y], &mut out)
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
        "visible": l.visible && shown,
    })
}

impl TjsHost for KrkrHost {
    /// Everything with native state or queued work keyed by an object.
    fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        let ids = self.layers.keys()
            .chain(self.windows.keys())
            .chain(self.timers.keys())
            .chain(self.parsers.keys())
            .chain(self.menus.keys())
            .chain(self.transitions.0.keys())
            .chain(self.transitions.0.values())
            .chain(self.menu_popups.iter().map(|(h, _)| h))
            .chain(self.image_job.iter().map(|(h, ..)| h));
        out.extend(ids.map(|h| ObjectHandle(*h)));
        out.extend(self.deferred.targets(self.client).map(|t| ObjectHandle((t >> 2) as usize)));
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
        // Registries that do not root their instances (see gc_roots).
        self.psbfile.invalidate(handle.0);
        self.csv.invalidate(handle.0);
        self.line_parser.invalidate(handle.0);
        self.text_render.invalidate(handle.0);
    }

    fn invalidate_object(&mut self, handle: ObjectHandle) {
        self.modal_layers.retain(|l| *l != handle.0);
        self.motion.invalidate(handle.0);
        self.sounds.invalidate(handle.0);
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
            if let Some(primary) = window.primary {
                self.invalidate_layer(primary);
            }
        }
    }

    fn read_text(&mut self, name: &str, _mode: &str) -> TjsResult<String> {
        let bytes = self
            .storage()?
            .read_named(name)
            .map_err(|e| TjsError::runtime(format!("{e:#}")))?;
        decode_text(&bytes).map_err(|e| TjsError::runtime(format!("{name}: {e:#}")))
    }

    fn read_binary(&mut self, name: &str, _mode: &str) -> TjsResult<Vec<u8>> {
        self.storage()?
            .read_named(name)
            .map_err(|e| TjsError::runtime(format!("{e:#}")))
    }

    fn write_text(&mut self, name: &str, mode: &str, text: &str) -> TjsResult<()> {
        let data = encode_native_text(text,mode)?;
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
