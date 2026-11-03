//! Native window handles and three-pass policy over shared Canvas/Layer state.
use super::*;
mod composition;
mod raster;
use crate::display_tree::NativeDisplayKind;
use crysirror_bgi_vm::HostMemoryWrite;
pub(crate) use raster::{WindowBlendOwner, WindowBlit};

#[derive(Clone, Debug)]
pub(crate) struct NativeIcon {
    pub bitmap: i32,
    pub position: [i32; 2],
}
#[derive(Clone, Debug)]
pub(crate) struct NativeWindow {
    pub size: [u32; 2],
    pub content_version: u64,
    pub valid: [i32; 4],
    pub order: [usize; 3],
    pub isolated: bool,
    pub font: i32,
    pub font_size: i32,
    pub width_percent: i32,
    pub font_style: i32,
    pub layout_option: i32,
    pub render_option: i32,
    pub layout_mode: i32,
    pub variant: i32,
    pub line_spacing: i32,
    pub cursor: [i32; 2],
    pub color: u32,
    pub shadow: Option<crysirror_core::image_processing::BitmapTextShadow>,
    pub shadow_configured: bool,
    pub page: crysirror_text::message::ParsedMessageMarkup,
    pub page_origin: [i32; 2],
    pub background: Option<u32>,
    pub backing_raw: Option<Arc<dyn crysirror_core::image_processing::ImageResource>>,
    pub backing_path: Option<String>,
    pub dirty: bool,
    pub icons: BTreeMap<i32, NativeIcon>,
    pub text_overlays: BTreeMap<i32, Value>,
    pub control_layers: BTreeMap<i32, Vec<Value>>,
}
impl NativeWindow {
    fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            content_version: 0,
            valid: [0, 0, size[0] as i32 - 1, size[1] as i32 - 1],
            order: [0, 1, 2],
            isolated: false,
            font: 0,
            font_size: 0,
            width_percent: 100,
            font_style: 0,
            layout_option: 0,
            render_option: 0,
            layout_mode: 0,
            variant: 0,
            line_spacing: 0,
            cursor: [0; 2],
            color: 0xffffff,
            shadow: None,
            shadow_configured: false,
            page: Default::default(),
            page_origin: [0; 2],
            background: None,
            backing_raw: None,
            backing_path: None,
            dirty: true,
            icons: BTreeMap::new(),
            text_overlays: BTreeMap::new(),
            control_layers: BTreeMap::new(),
        }
    }
    pub fn reset_cursor(&mut self) {
        self.cursor = [
            if self.variant == 1 {
                self.valid[2]
            } else {
                self.valid[0]
            },
            self.valid[1],
        ];
        self.page = Default::default();
        self.page_origin = self.cursor;
    }
    pub fn canvas(handle: i32, plane: u32) -> u32 {
        0xb200_0000 | ((handle as u32 & 15) << 3) | plane
    }
    pub fn layer(handle: i32) -> u32 {
        0xb300_0000 | (handle as u32 & 15)
    }
}
#[derive(Default)]
pub(crate) struct WindowState {
    pub records: BTreeMap<i32, NativeWindow>,
    pub input_locks: std::collections::BTreeSet<i32>,
    blends: BTreeMap<i32, raster::WindowRasterJob>,
    compositions: BTreeMap<i32, composition::WindowComposition>,
    captures: BTreeMap<u64, crysirror_core::image_processing::ImageJob>,
    capture_sources: BTreeMap<u64, i32>,
}
impl WindowState {
    pub fn has_window(&self, id: i32) -> bool {
        self.records.contains_key(&id)
    }
    pub fn valid_region(&self, id: i32) -> Option<[i32; 4]> {
        self.records.get(&id).map(|w| w.valid)
    }
    pub fn get(&self, id: i32) -> Result<&NativeWindow> {
        self.records.get(&id).context("BGI invalid window handle")
    }
    pub fn get_mut(&mut self, id: i32) -> Result<&mut NativeWindow> {
        self.records
            .get_mut(&id)
            .context("BGI invalid window handle")
    }
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    matches!(
        (group, id),
        (0x90, 0x80..=0x89) | (0x91, 0x88..=0x8e) | (0x92, 0x88 | 0x89 | 0x8c | 0x8d | 0x8e)
    )
}
impl BgiVm {
    pub(crate) fn publish_windows(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let mut handles = self.windows.records.keys().copied().collect::<Vec<_>>();
        handles.sort_by_key(|handle| self.graph.tree.chain_position(*handle).unwrap_or(0));
        for handle in handles {
            let w = self.windows.records.get(&handle).unwrap();
            let p = self
                .graph
                .objects
                .get(&handle)
                .context("BGI window display object")?;
            let n = p.native;
            let canvas = NativeWindow::canvas(handle, 3);
            let layer = NativeWindow::layer(handle);
            let mut canvases = Vec::new();
            if w.dirty {
                canvases.push(json!({"op":"clear","id":canvas,"color":[0,0,0,0]}));
                for &plane in &w.order {
                    let clip = if plane == 0 {
                        Value::Null
                    } else {
                        json!([
                            w.valid[0],
                            w.valid[1],
                            (w.valid[2] - w.valid[0] + 1).max(0),
                            (w.valid[3] - w.valid[1] + 1).max(0)
                        ])
                    };
                    canvases.push(json!({"op":"copy","source":NativeWindow::canvas(handle,plane as u32),"destination":canvas,"rect":[0,0,w.size[0],w.size[1]],"position":[0,0],"opacity":255,"clip":clip}));
                }
            }
            let position = self
                .graph
                .draw_position(handle)
                .context("BGI window has no native position")?;
            let visible = self.graph.global_enabled != 0
                && n.draw_enabled != 0
                && n.suppress_draw == 0
                && p.mask_alpha < 256
                && p.alpha_multiplier != 0
                && self.graph.tree.chain_drawable(handle);
            let serial = self.graph.tree.chain_position(handle).unwrap_or(0) as i32;
            let mode = p.blend_mode;
            let opacity = (p.opacity() * 255.).round() as u8;
            let isolated = w.isolated;
            let content_changed = w.dirty;
            let descriptor = json!({"position":position,"size":w.size,"isolated":isolated,"visible":visible,"opacity":opacity,"blend":mode,"order":[n.priority,serial]});
            if content_changed {
                let w = self.windows.get_mut(handle)?;
                w.content_version = w.content_version.wrapping_add(1);
            }
            if mode == 128 {
                self.release_window_composition(host, handle)?;
                canvases.push(json!({"op":if isolated{"present_isolated"}else{"present"},"id":canvas,"layer":layer,"order":n.priority,"position":position}));
                self.submit(host,json!({"canvases":canvases,"layers":[{"op":"set_visible","id":layer,"visible":visible},{"op":"set_opacity","id":layer,"opacity":opacity},{"op":"set_blend","id":layer,"blend":"normal"},{"op":"set_order","id":layer,"order":[n.priority,serial]}]}))?;
            } else {
                canvases.push(json!({"op":"hide","layer":layer}));
                self.submit(host, json!({"canvases":canvases}))?;
                let (paint, size) =
                    self.window_composition_paint(host, handle, position, visible)?;
                self.submit(host,json!({"layers":[{"op":"put","layer":{"id":layer,"rect":[0,0,size[0],size[1]],"order":[n.priority,serial],"visible":visible,"paint":paint}}]}))?;
            }
            self.windows.records.get_mut(&handle).unwrap().dirty = false;
            if content_changed {
                self.graph.mark_visual(handle);
            }
            self.graph.visual_descriptor(handle, descriptor);
        }
        Ok(())
    }
    pub(crate) fn window_image_source(&self, bitmap: i32) -> Result<(u32, [u32; 2])> {
        if let Some(window) = self.windows.records.get(&bitmap) {
            Ok((NativeWindow::canvas(bitmap, 0), window.size))
        } else {
            let (id, info) = self.bitmap(bitmap)?;
            Ok((id, info.size))
        }
    }
    pub(crate) fn rebuild_window_icons(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
    ) -> Result<()> {
        let w = self.windows.get(window)?;
        let canvas = NativeWindow::canvas(window, 2);
        let mut commands = vec![json!({"op":"clear","id":canvas,"color":[0,0,0,0]})];
        for (_, icon) in w.icons.iter().filter(|(key, _)| **key != -1) {
            let (source, size) = self.window_image_source(icon.bitmap)?;
            commands.push(json!({"op":"copy","source":source,"destination":canvas,"rect":[0,0,size[0],size[1]],"position":icon.position,"opacity":255}));
        }
        let mut controls = w.control_layers.values().flatten().collect::<Vec<_>>();
        controls.sort_by_key(|layer| {
            (
                layer["order"][0].as_i64().unwrap_or(0),
                layer["order"][1].as_i64().unwrap_or(0),
            )
        });
        for (id, layer) in controls.into_iter().enumerate() {
            let mut layer = layer.clone();
            layer["id"] = json!(0x10000 + id as u32);
            commands.push(json!({"op":"draw","id":canvas,"layer":layer}));
        }
        for layer in w.text_overlays.values() {
            commands.push(json!({"op":"draw","id":canvas,"layer":layer}));
        }
        if let Some(icon) = w.icons.get(&-1) {
            let (source, size) = self.window_image_source(icon.bitmap)?;
            commands.push(json!({"op":"copy","source":source,"destination":canvas,"rect":[0,0,size[0],size[1]],"position":icon.position,"opacity":255}));
        }
        self.submit(host, json!({"canvases":commands}))?;
        self.windows.get_mut(window)?.dirty = true;
        Ok(())
    }
    pub(crate) fn archive_window_text(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
    ) -> Result<()> {
        let size = self.windows.get(window)?.size;
        self.submit(host,json!({"canvases":[{"op":"clear","id":NativeWindow::canvas(window,4),"color":[0,0,0,0]},
            {"op":"copy","source":NativeWindow::canvas(window,1),"destination":NativeWindow::canvas(window,4),"replace":true,"rect":[0,0,size[0],size[1]],"position":[0,0],"opacity":255}]}))?;
        let w = self.windows.get_mut(window)?;
        w.page = Default::default();
        w.page_origin = w.cursor;
        Ok(())
    }
    pub(crate) fn reset_window_text(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
    ) -> Result<()> {
        let w = self.windows.get_mut(window)?;
        w.reset_cursor();
        w.dirty = true;
        self.submit(host,json!({"canvases":[{"op":"clear","id":NativeWindow::canvas(window,1),"color":[0,0,0,0]},{"op":"clear","id":NativeWindow::canvas(window,4),"color":[0,0,0,0]}]}))?;
        Ok(())
    }
    pub(crate) fn window_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if !handles(call.group, call.id) {
            return Ok(false);
        }
        let a = &call.args;
        let request = vm.request_id().context("BGI window request identity")?;
        if call.group == 0x90 && call.id == 0x83 {
            target_checks::bitmap(a[0])?;
            self.windows.get(a[1])?;
            self.publish_windows(host)?;
            self.windows.capture_sources.insert(request, a[1]);
            let mut capture = if let Some(job) = self.windows.captures.remove(&request) {
                job
            } else {
                host.begin_canvas_image(self.client, NativeWindow::canvas(a[1], 3))?
            };
            match capture.poll()? {
                crysirror_core::image_processing::ImageProcessState::Pending => {
                    self.windows.captures.insert(request, capture);
                    return Ok(false);
                }
                crysirror_core::image_processing::ImageProcessState::Failed(error) => {
                    bail!("BGI window bitmap capture: {error}")
                }
                crysirror_core::image_processing::ImageProcessState::Ready(image) => {
                    self.windows.capture_sources.remove(&request);
                    self.publish_bitmap_reallocated(
                        host,
                        a[0] as u32,
                        2,
                        [-1; 2],
                        image.clone(),
                        image,
                    )?;
                    vm.commit_call(&[])?;
                    self.calls += 1;
                    return Ok(true);
                }
            }
        }
        if matches!((call.group, call.id), (0x92, 0x89 | 0x8d)) {
            if !self.windows.has_window(a[0]) && self.bitmaps.contains_key(&(a[0] as u32)) {
                let equivalent = NativeCall {
                    group: 0x90,
                    id: 0x18,
                    args: call.args.clone(),
                    outputs: call.outputs,
                };
                return self.bitmap_call(host, vm, &equivalent);
            }
            let size = self.windows.get(a[0])?.size;
            let source = self.bitmap(a[3])?.1;
            let overlaps = (a[1] as i64) < i64::from(size[0])
                && (a[2] as i64) < i64::from(size[1])
                && i64::from(a[1]) + i64::from(source.size[0]) > 0
                && i64::from(a[2]) + i64::from(source.size[1]) > 0;
            ensure!(
                !overlaps || matches!(source.format, 1 | 2),
                "BGI window bitmap is not screen-compatible"
            );
            target_checks::bitmap(a[3])?;
            target_checks::effect_mode(a[4])?;
            target_checks::effect_level(a[5])?;
            self.window_image_source(a[3])?;
            if !self.window_blend(
                host,
                a[0],
                WindowBlendOwner::Native(request),
                false,
                vec![WindowBlit {
                    bitmap: a[3],
                    position: [a[1], a[2]],
                    mode: a[4],
                    parameter: a[5],
                }],
            )? {
                return Ok(false);
            }
            vm.commit_call(&[])?;
            self.calls += 1;
            return Ok(true);
        }
        let mut outputs = Vec::new();
        let mut writes = Vec::new();
        match (call.group, call.id) {
            (0x90, 0x80) => {
                ensure!(
                    self.windows.records.len() < 16,
                    "これ以上、ウィンドウオブジェクトを生成することはできません"
                );
                let width = if (a[0] as u32) < 32 {
                    (a[0] as u32).wrapping_mul(32)
                } else {
                    a[0] as u32
                };
                let height = if (a[1] as u32) < 20 {
                    (a[1] as u32).wrapping_mul(32)
                } else {
                    a[1] as u32
                };
                ensure!(
                    width.wrapping_sub(1) <= 0x77f && height.wrapping_sub(1) <= 0x7fff,
                    "BGI invalid window dimensions [{},{}]",
                    a[0],
                    a[1]
                );
                let handle = self
                    .graph
                    .allocate(0xb0000000, 16, NativeDisplayKind::Window);
                ensure!(handle != 0, "BGI window pool exhausted");
                let commands=(0..5).flat_map(|plane|[json!({"op":"create","id":NativeWindow::canvas(handle,plane),"width":width,"height":height}),json!({"op":"clear","id":NativeWindow::canvas(handle,plane),"color":[0,0,0,0]})]).collect::<Vec<_>>();
                self.submit(host, json!({"canvases":commands}))?;
                self.windows
                    .records
                    .insert(handle, NativeWindow::new([width, height]));
                // CDspObjWindow constructor sub_42AE20 overrides the base
                // CDspObj blend (128) with source-alpha mode 1.
                self.graph.objects.entry(handle).or_default().blend_mode = 1;
                self.graph
                    .objects
                    .entry(handle)
                    .or_default()
                    .native
                    .blend_mode = 1;
                let n = &mut self.graph.objects.entry(handle).or_default().native;
                n.enabled = 1;
                n.draw_enabled = 1;
                outputs.push(handle);
            }
            (0x90, 0x81) => {
                let handle = a[0];
                if self.windows.blends.contains_key(&handle)
                    || self
                        .windows
                        .capture_sources
                        .values()
                        .any(|source| *source == handle)
                {
                    return Ok(false);
                }
                self.windows.get(handle)?;
                self.release_window_inputs(host, vm, handle)?;
                ensure!(
                    self.graph.tree.parent(handle).is_none(),
                    "指定されたウィンドウにはオーナーが存在します"
                );
                ensure!(
                    !self.windows.input_locks.contains(&handle),
                    "指定されたウィンドウには協調しているプロシージャが存在します"
                );
                ensure!(
                    !self.message_window_busy(handle),
                    "指定されたウィンドウには協調しているプロシージャが存在します"
                );
                let mut commands = vec![json!({"op":"hide","layer":NativeWindow::layer(handle)})];
                for plane in 0..5 {
                    commands.push(json!({"op":"destroy","id":NativeWindow::canvas(handle,plane)}));
                }
                let layers = if self.windows.compositions.contains_key(&handle) {
                    vec![json!({"op":"remove","id":NativeWindow::layer(handle)})]
                } else {
                    Vec::new()
                };
                self.submit(host, json!({"canvases":commands,"layers":layers}))?;
                self.release_window_composition(host, handle)?;
                if let Some(path) = self.windows.get(handle)?.backing_path.clone() {
                    host.release_image_resource(self.client, &path)?;
                }
                self.windows.records.remove(&handle);
                self.selections.columns.remove(&handle);
                self.graph.objects.remove(&handle);
                self.graph.tree.remove(handle);
            }
            (0x90, 0x82) => {
                let order = match a[1] {
                    0 => [0, 1, 2],
                    1 => [0, 2, 1],
                    2 => [1, 0, 2],
                    3 => [1, 2, 0],
                    4 => [2, 0, 1],
                    5 => [2, 1, 0],
                    _ => bail!("BGI invalid window composition order {}", a[1]),
                };
                let w = self.windows.get_mut(a[0])?;
                w.order = order;
                w.dirty = true;
            }
            (0x90, 0x84) => {
                self.windows.get(a[0])?;
                for id in self.graph.descendants(a[0]) {
                    self.graph
                        .objects
                        .entry(id)
                        .or_default()
                        .native
                        .draw_enabled = u32::from(a[1] != 0);
                }
            }
            (0x90, 0x85) => {
                target_checks::priority(a[6])?;
                target_checks::effect_level(a[5])?;
                target_checks::effect_level(a[4])?;
                target_checks::effect_mode(a[3])?;
                self.windows.get(a[0])?;
                self.graph.position(a[0], a[1], a[2]);
                let p = self.graph.objects.entry(a[0]).or_default();
                p.blend_mode = a[3];
                p.native.blend_mode = a[3];
                p.native.priority = a[6] as u32;
                self.graph_alpha(a[0], a[4])?;
                self.graph.sort(a[0]);
            }
            (0x90, 0x86) => {
                self.windows.get(a[0])?;
                let operations = if a[3] == -1 {
                    Vec::new()
                } else {
                    self.window_image_source(a[3])?;
                    vec![WindowBlit {
                        bitmap: a[3],
                        position: [0; 2],
                        mode: 128,
                        parameter: 0,
                    }]
                };
                if !self.window_blend(
                    host,
                    a[0],
                    WindowBlendOwner::Native(request),
                    true,
                    operations,
                )? {
                    return Ok(false);
                }
                self.windows.get_mut(a[0])?.background = (a[3] != -1).then_some(a[3] as u32);
            }
            (0x90, 0x87) => {
                let w = self.windows.get_mut(a[0])?;
                w.isolated = a[1] != 0;
                w.dirty = true;
            }
            (0x90, 0x88) => {
                let w = self.windows.get(a[0])?;
                let right = a[1].wrapping_add(a[3]).wrapping_sub(1);
                let bottom = a[2].wrapping_add(a[4]).wrapping_sub(1);
                let inside = |v: i32, max: u32| v >= 0 && (v as u32) < max;
                ensure!(
                    inside(a[1], w.size[0])
                        && inside(right, w.size[0])
                        && inside(a[2], w.size[1])
                        && inside(bottom, w.size[1]),
                    "BGI invalid window valid region"
                );
                self.archive_window_text(host, a[0])?;
                let w = self.windows.get_mut(a[0])?;
                w.valid = [a[1], a[2], right, bottom];
                w.reset_cursor();
            }
            (0x90, 0x89) => {
                let valid = self.windows.get(a[1])?.valid;
                writes.push(HostMemoryWrite {
                    address: a[0] as u32,
                    bytes: valid.into_iter().flat_map(i32::to_le_bytes).collect(),
                });
                outputs.push(1);
            }
            (0x91, 0x88) => {
                ensure!(
                    self.native_font_known(a[1]),
                    "BGI invalid native font {}",
                    a[1]
                );
                ensure!((4..=200).contains(&a[2]), "BGI invalid native font height");
                ensure!((25..=200).contains(&a[3]), "BGI invalid native font width");
                if !self.windows.get(a[0])?.page.text.is_empty() {
                    self.archive_window_text(host, a[0])?;
                }
                let w = self.windows.get_mut(a[0])?;
                w.font = a[1];
                w.font_size = a[2];
                w.width_percent = a[3];
                w.font_style = a[4];
                w.layout_option = a[5];
                w.render_option = a[6];
                if a[6] != 0 {
                    let limit = w.size[0] as i32 - 1 - a[3] * a[2] / 100;
                    if w.valid[2] > limit
                        && w.valid[0] >= 0
                        && (w.valid[0] as u32) < w.size[0]
                        && limit >= 0
                        && (limit as u32) < w.size[0]
                    {
                        w.valid[2] = limit;
                        w.reset_cursor();
                    }
                }
            }
            (0x91, 0x89) => {
                ensure!(a[1] as u32 <= 800, "BGI window line spacing");
                self.windows.get_mut(a[0])?.line_spacing = a[1];
            }
            (0x91, 0x8a) => {
                ensure!(a[1] as u32 <= 1, "BGI message variant");
                self.archive_window_text(host, a[0])?;
                let w = self.windows.get_mut(a[0])?;
                w.variant = a[1];
                w.reset_cursor();
            }
            (0x91, 0x8b) => {
                ensure!(a[1] as u32 <= 2, "BGI message layout mode");
                self.windows.get_mut(a[0])?.layout_mode = a[1];
            }
            (0x91, 0x8c) => {
                self.archive_window_text(host, a[0])?;
                let w = self.windows.get_mut(a[0])?;
                w.cursor = [a[1], a[2]];
                w.page = Default::default();
                w.page_origin = w.cursor;
            }
            (0x91, 0x8d) => {
                let w = self.windows.get(a[0])?;
                outputs.extend([1, w.cursor[0], w.cursor[1]]);
            }
            (0x91, 0x8e) => {
                let w = self.windows.get(a[0])?;
                let reached = if w.variant == 1 {
                    w.cursor[1] == w.valid[1].wrapping_add(self.text.layout.boundary_offset)
                } else {
                    w.cursor[0] == w.valid[0].wrapping_add(self.text.layout.boundary_offset)
                };
                outputs.push(i32::from(reached));
            }
            (0x92, 0x88 | 0x8c) => {
                self.windows.get(a[0])?;
                self.graph
                    .objects
                    .entry(a[0])
                    .or_default()
                    .properties
                    .insert(call.id as u32, (a[1], 0));
            }
            (0x92, 0x8e) => {
                self.reset_window_text(host, a[0])?;
                self.finish_message_for_window(a[0]);
            }
            _ => unreachable!(),
        }
        ensure!(
            outputs.len() == call.outputs,
            "BGI window native result count"
        );
        self.publish_graph(host)?;
        vm.commit_call_writes(&outputs, writes)?;
        self.calls += 1;
        Ok(true)
    }
}
