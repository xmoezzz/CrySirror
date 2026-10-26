// SPDX-License-Identifier: AGPL-3.0-or-later
//! Handle translation between KiriKiri native objects and engine services.
//!
//! A KiriKiri `Layer` is one engine Layer node (position, size, clip,
//! visibility, opacity and painter order) with one engine Canvas surface
//! presented beneath it for its image. Drawing calls become Canvas commands;
//! no pixels are composed here. Commands are queued while TJS runs and
//! submitted in order by the adapter after each script entry point returns.
use crate::storage::{Storage, decode_text};
use crysirror_core::service::{ImageMetadata, TextMeasurement};
use krkr_tjs2::{
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
    /// Nothing has been drawn since the surface was cleared to transparency,
    /// so source-over and replace are the same operation.
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
    pub primary: Option<usize>,
    pub inner: [i64; 2],
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct TimerState {
    pub interval: u64,
    pub enabled: bool,
    pub capacity: u64,
    pub next: u64,
}

#[derive(Debug)]
pub enum Batch {
    Layers(Vec<Value>),
    Canvases(Vec<Value>),
}

pub struct KrkrHost {
    pub storage: Option<Storage>,
    pub images: Option<Arc<dyn ImageMetadata>>,
    pub text_measurement: Option<Arc<dyn TextMeasurement>>,
    pub now: u64,
    pub canvas: [u32; 2],
    pub font: String,
    pub log: Vec<String>,
    pub batches: Vec<Batch>,
    pub layers: BTreeMap<usize, LayerState>,
    pub windows: BTreeMap<usize, WindowState>,
    pub timers: BTreeMap<usize, TimerState>,
    free_slots: BTreeSet<u32>,
    next_slot: u32,
    pub exit_requested: bool,
    pub api_calls: u64,
}

impl KrkrHost {
    pub fn new(canvas: [u32; 2], font: String) -> Self {
        Self {
            storage: None,
            images: None,
            text_measurement: None,
            now: 0,
            canvas,
            font,
            log: Vec::new(),
            batches: Vec::new(),
            layers: BTreeMap::new(),
            windows: BTreeMap::new(),
            timers: BTreeMap::new(),
            free_slots: BTreeSet::new(),
            next_slot: 1,
            exit_requested: false,
            api_calls: 0,
        }
    }

    pub fn push_log(&mut self, line: String) {
        if self.log.len() >= MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(line);
    }

    pub fn canvas_op(&mut self, op: Value) {
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
            blank: true,
            attached: true,
            major: 0,
            dirty: true,
            presented: None,
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

    /// Resize a layer's image, discarding its contents (KiriKiri semantics of
    /// `setImageSize` with a fresh transparent bitmap).
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
        let slot = {
            let l = self.layer_mut(h)?;
            l.image_width = w;
            l.image_height = height;
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

    /// Children in painter order: stable by `absolute`, then insertion order.
    pub fn sorted_children(&self, h: usize) -> Vec<usize> {
        let Some(l) = self.layers.get(&h) else {
            return vec![];
        };
        let mut children = l.children.clone();
        children.sort_by_key(|c| self.layers.get(c).map_or(0, |c| c.absolute));
        children
    }

    /// Emit the commands that bring engine nodes up to date: painter order
    /// from a depth-first walk, container geometry and image placement.
    pub fn sync(&mut self) {
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
            let present = (l.presented != Some(placement)).then(|| {
                l.presented = Some(placement);
                json!({"op":"present","id":l.slot,"layer":l.presentation(),"order":placement.0,
                    "parent":l.container(),"position":placement.1})
            });
            self.layer_op(json!({"op":"put","layer":spec}));
            if let Some(p) = present {
                self.canvas_op(p);
            }
        }
    }

    /// Front-most visible layer containing the canvas point, with local
    /// coordinates. Hit testing uses layer bounds; per-pixel alpha
    /// thresholds are not evaluated by this adapter.
    pub fn hit(&self, x: i64, y: i64) -> Option<(usize, i64, i64)> {
        let roots: Vec<usize> = self.windows.values().filter_map(|w| w.primary).collect();
        let mut best = None;
        for root in roots {
            self.hit_walk(root, 0, 0, x, y, &mut best);
        }
        best.map(|(_, h, lx, ly)| (h, lx, ly))
    }

    fn hit_walk(
        &self,
        h: usize,
        ox: i64,
        oy: i64,
        x: i64,
        y: i64,
        best: &mut Option<(i32, usize, i64, i64)>,
    ) {
        let Some(l) = self.layers.get(&h) else { return };
        if !l.visible || !l.attached {
            return;
        }
        let (lx, ly) = (x - ox - l.left, y - oy - l.top);
        if lx < 0 || ly < 0 || lx >= l.width || ly >= l.height {
            return;
        }
        if best.is_none_or(|b| l.major >= b.0) {
            *best = Some((l.major, h, lx, ly));
        }
        for c in self.sorted_children(h) {
            self.hit_walk(c, ox + l.left, oy + l.top, x, y, best);
        }
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

    fn now_millis(&mut self) -> i64 {
        self.now as i64
    }

    fn log(&mut self, message: &str) {
        self.push_log(message.to_owned());
    }
}
