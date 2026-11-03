//! Native CProcSelect policy with engine-owned widgets, hit masks and text.
use super::*;
use crate::window_api::{NativeIcon, NativeWindow, WindowBlendOwner, WindowBlit};
use crysirror_bgi_vm::native_selection::{
    NativeSelectionCompletion, NativeSelectionData, NativeSelectionRequest,
};
use crysirror_core::{
    image_processing::{ImageJob, ImageProcessState, ImageResource},
    service::EngineQuery,
};
use crysirror_scene::ui::{
    WidgetHitMask, WidgetInteraction,
    hit_mask::CoverageChannel,
    navigation::{GridDirection, grid_neighbor},
};
pub(crate) struct SelectionState {
    pub(crate) inputs: crate::input_api::InputState,
    pending: BTreeMap<u64, Pending>,
    active: BTreeMap<u64, Selection>,
    next_widget: u32,
    pub styles: [u32; 2],
    pub blink_interval: i32,
    pub navigation: [i32; 4],
    pub poll_gate: i32,
    pub(crate) columns: BTreeMap<i32, [i32; 16]>,
    callbacks: BTreeMap<u64, (Vec<[i32; 2]>, bool)>,
}
impl Default for SelectionState {
    fn default() -> Self {
        Self {
            inputs: Default::default(),
            pending: BTreeMap::new(),
            active: BTreeMap::new(),
            next_widget: 0,
            styles: [0xff0000, 0xffff00],
            blink_interval: 600,
            navigation: [0; 4],
            poll_gate: 0,
            columns: BTreeMap::new(),
            callbacks: BTreeMap::new(),
        }
    }
}
struct Selection {
    id: u64,
    visual_current: Option<i32>,
    visual_target: Option<i32>,
    finish_requested: bool,
    cancelled: bool,
    request: NativeSelectionRequest,
    base: u32,
    rects: Vec<[i32; 4]>,
    current: i32,
    previous_hover: i32,
    local: [i32; 2],
    blink: usize,
    deadline: u64,
    confirmation: Option<u32>,
    last_tick: Option<u64>,
    serials: Vec<u32>,
    scope: i32,
    root_position: [i32; 2],
    keyboard_scope: bool,
    pointer_scope: bool,
}
struct Pending {
    request: NativeSelectionRequest,
    rects: Vec<[i32; 4]>,
    sources: Vec<Option<MaskSource>>,
    task: Option<Task<Vec<Option<WidgetHitMask>>>>,
}
struct MaskSource {
    size: [u32; 2],
    format: i32,
    image: Option<Arc<dyn ImageResource>>,
    job: Option<ImageJob>,
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    group == 0x90 && matches!(id, 0xa4..=0xa6 | 0xaf)
}
impl BgiVm {
    pub(crate) fn selection_config_call(
        &mut self,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if !handles(call.group, call.id) {
            return Ok(false);
        }
        let a = &call.args;
        match call.id {
            0xa4 => self.selections.styles = [a[0] as u32, a[1] as u32],
            0xa5 => self.selections.blink_interval = a[0],
            0xa6 => self.selections.navigation = [a[0], a[1], a[2], a[3]],
            0xaf => self.selections.poll_gate = a[0],
            _ => unreachable!(),
        }
        vm.commit_call(&[])?;
        self.calls += 1;
        Ok(true)
    }
    pub(crate) fn set_selection_columns(
        &mut self,
        window: i32,
        columns: Option<[i32; 16]>,
    ) -> Result<()> {
        self.windows.get(window)?;
        if let Some(columns) = columns {
            self.selections.columns.insert(window, columns);
        } else {
            self.selections.columns.remove(&window);
        }
        Ok(())
    }
    pub(crate) fn draw_item_grid(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        text: &[String],
        columns: i32,
        centered: bool,
        color: i32,
    ) -> Result<()> {
        let request = NativeSelectionRequest {
            owner_thread_id: 0,
            opcode: crysirror_bgi_vm::NativeOpcode {
                group: 0x90,
                id: 0xa1,
            },
            window,
            allow_cancel: false,
            data: NativeSelectionData::Items {
                text: text.to_vec(),
                columns,
                centered,
                color,
                initial: 0,
                overlays: None,
                blink_confirmation: false,
            },
        };
        let rects = self.selection_rects(&request)?;
        self.draw_item_grid_rects(host, window, text, &rects, color)
    }
    fn draw_item_grid_rects(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        text: &[String],
        rects: &[[i32; 4]],
        color: i32,
    ) -> Result<()> {
        let canvas = NativeWindow::canvas(window, 1);
        let mut commands = vec![json!({"op":"clear","id":canvas,"color":[0,0,0,0]})];
        for (index, text) in text.iter().enumerate() {
            let r = rects[index];
            let color = self
                .selections
                .columns
                .get(&window)
                .map_or(color, |colors| colors[index]);
            commands.push(json!({"op":"draw","id":canvas,"layer":self.draw_selection_text(host,window,text,[r[0],r[1]],color as u32,index as u32)?}));
        }
        self.submit(host, json!({"canvases":commands}))?;
        self.windows.get_mut(window)?.dirty = true;
        self.publish_windows(host)
    }
    fn selection_rects(&self, request: &NativeSelectionRequest) -> Result<Vec<[i32; 4]>> {
        let w = self.windows.get(request.window)?;
        match &request.data {
            NativeSelectionData::Items {
                text,
                columns,
                centered,
                initial,
                ..
            } => {
                ensure!(
                    !text.is_empty()
                        && text.len() <= 16
                        && (1..=16).contains(columns)
                        && (0..text.len() as i32).contains(initial),
                    "BGI item selection arguments"
                );
                ensure!(
                    w.font_size > 0,
                    "指定されたウィンドウにはフォントが設定されていません"
                );
                let area = w.valid[2] - w.valid[0] + 1;
                let size = w.font_size;
                let pitch = crysirror_text::bitmap_layout::line_pitch(size as f32, w.line_spacing)
                    .round() as i32;
                text.iter()
                    .enumerate()
                    .map(|(index, text)| {
                        let (_, width) = crysirror_text::grid::encoded_cell_run(
                            text,
                            [0; 2],
                            size,
                            encoding_rs::SHIFT_JIS,
                        )?;
                        let column = index as i32 % columns;
                        let left = w.valid[0] + column * area / columns;
                        let x = if *centered {
                            left + area / columns / 2 - (width as u32 >> 1) as i32
                        } else {
                            left
                        };
                        let y = w.valid[1] + pitch * (index as i32 / columns);
                        Ok([
                            x,
                            y,
                            x.wrapping_add(width).wrapping_sub(1),
                            y.wrapping_add(size).wrapping_sub(1),
                        ])
                    })
                    .collect()
            }
            NativeSelectionData::Icons {
                records,
                input_mode,
                ..
            } => {
                ensure!(
                    !records.is_empty() && records.len() <= 64 && (*input_mode as u32) <= 3,
                    "BGI icon selection arguments"
                );
                records
                    .iter()
                    .map(|record| {
                        let (_, size) = self.window_image_source(record[2])?;
                        let x = w.valid[0].wrapping_add(record[0]);
                        let y = w.valid[1].wrapping_add(record[1]);
                        Ok([
                            x,
                            y,
                            x.wrapping_add(size[0] as i32).wrapping_sub(1),
                            y.wrapping_add(size[1] as i32).wrapping_sub(1),
                        ])
                    })
                    .collect()
            }
        }
    }
    fn selection_source(
        &self,
        host: &mut dyn HostServices,
        bitmap: i32,
        size: [u32; 2],
    ) -> Result<MaskSource> {
        let (canvas, _) = self.window_image_source(bitmap)?;
        if let Some(b) = self.bitmaps.get(&(bitmap as u32)) {
            Ok(MaskSource {
                size,
                format: b.format,
                image: b.image.clone(),
                job: if b.image.is_none() {
                    Some(host.begin_canvas_image(self.client, canvas)?)
                } else {
                    None
                },
            })
        } else {
            Ok(MaskSource {
                size,
                format: 2,
                image: self.windows.get(bitmap)?.backing_raw.clone(),
                job: if self.windows.get(bitmap)?.backing_raw.is_none() {
                    Some(host.begin_canvas_image(self.client, canvas)?)
                } else {
                    None
                },
            })
        }
    }
    pub(crate) fn selection_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        id: u64,
        request: NativeSelectionRequest,
    ) -> Result<bool> {
        let mut pending = if let Some(pending) = self.selections.pending.remove(&id) {
            pending
        } else {
            let rects = self.selection_rects(&request)?;
            ensure!(
                !self.windows.input_locks.contains(&request.window),
                "BGI window already owns an input procedure"
            );
            self.windows.input_locks.insert(request.window);
            if let NativeSelectionData::Items {
                overlays: Some(overlays),
                ..
            } = &request.data
            {
                for bracket in overlays {
                    if bracket[0] != -1 {
                        self.window_image_source(bracket[0])?;
                    }
                }
            }
            let mut sources = Vec::with_capacity(rects.len());
            let mut bytes = 0u64;
            for (index, rect) in rects.iter().enumerate() {
                let size = [
                    (rect[2] - rect[0] + 1).max(1) as u32,
                    (rect[3] - rect[1] + 1).max(1) as u32,
                ];
                if let NativeSelectionData::Icons {
                    records,
                    extended,
                    alpha_hit_test: true,
                    ..
                } = &request.data
                {
                    let bitmap = if *extended && records[index][15] != -1 {
                        records[index][15]
                    } else {
                        records[index][2]
                    };
                    bytes = bytes
                        .checked_add(u64::from(size[0]) * u64::from(size[1]))
                        .context("BGI selection coverage overflow")?;
                    ensure!(bytes <= 64 * 1024 * 1024, "BGI selection coverage budget");
                    sources.push(Some(self.selection_source(host, bitmap, size)?));
                } else {
                    sources.push(None);
                }
            }
            Pending {
                request,
                rects,
                sources,
                task: None,
            }
        };
        if pending.task.is_none() {
            for source in pending.sources.iter_mut().flatten() {
                if let Some(job) = &mut source.job {
                    match job.poll()? {
                        ImageProcessState::Pending => {
                            self.selections.pending.insert(id, pending);
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => {
                            bail!("BGI selection hit mask capture: {error}")
                        }
                        ImageProcessState::Ready(image) => {
                            source.image = Some(image);
                            source.job = None;
                        }
                    }
                }
            }
            if pending.sources.iter().any(Option::is_some) {
                let sources = std::mem::take(&mut pending.sources);
                pending.task = Some(self.pool.spawn(move |cancel| {
                    sources
                        .into_iter()
                        .map(|source| {
                            let Some(source) = source else {
                                return Ok(None);
                            };
                            cancel.check()?;
                            let image = source.image.context("BGI selection mask has no image")?;
                            let channel = match source.format {
                                0 | 1 => CoverageChannel::MaxRgb,
                                2 => CoverageChannel::Alpha,
                                3 => CoverageChannel::Red,
                                _ => CoverageChannel::Zero,
                            };
                            Ok(Some(WidgetHitMask::from_rgba_channels(
                                source.size,
                                image.size(),
                                &image.rgba(),
                                channel,
                                1,
                                &|| cancel.check(),
                            )?))
                        })
                        .collect()
                })?);
            }
        }
        let masks = if let Some(task) = &mut pending.task {
            let Some(masks) = task.try_take()? else {
                self.selections.pending.insert(id, pending);
                return Ok(false);
            };
            masks
        } else {
            (0..pending.rects.len()).map(|_| None).collect()
        };
        self.selections.next_widget = self
            .selections
            .next_widget
            .checked_add(128)
            .context("BGI selection widget serial exhausted")?;
        let base = 0xb4000000u32
            .checked_add(self.selections.next_widget)
            .context("BGI selection widget namespace exhausted")?;
        let window = pending.request.window;
        let root_position = self.selection_window_position(window)?;
        let w = self.windows.get(window)?;
        let modal = matches!(
            &pending.request.data,
            NativeSelectionData::Items { .. } | NativeSelectionData::Icons { input_mode: 3, .. }
        );
        let root = json!({"op":"put","widget":{"id":base,"rect":[root_position[0],root_position[1],w.size[0],w.size[1]],"kind":"panel","interaction_only":true,"keyboard_focusable":false,"modal":modal,"clip_children":false,"z":self.graph.tree.chain_position(window).unwrap_or(0)as i32+1000}});
        let mut widgets = vec![root];
        for (index, (rect, mask)) in pending.rects.iter().zip(masks).enumerate() {
            let empty = rect[2] < rect[0] || rect[3] < rect[1];
            let size = [
                (rect[2] - rect[0] + 1).max(1),
                (rect[3] - rect[1] + 1).max(1),
            ];
            let mask = if empty {
                Some(WidgetHitMask::from_rgba_channels(
                    [size[0] as u32, size[1] as u32],
                    [0; 2],
                    &[],
                    CoverageChannel::Zero,
                    1,
                    &|| Ok(()),
                )?)
            } else {
                mask
            };
            widgets.push(json!({"op":"put","widget":{"id":base+index as u32+1,"parent":base,"rect":[rect[0],rect[1],size[0],size[1]],"kind":"button","interaction_only":true,"keyboard_focusable":false,"activate_on_press":true,"hit_mask":mask,"z":-(index as i32)}}));
        }
        self.submit(host, json!({"ui":widgets}))?;
        let scope = self
            .graph
            .tree
            .native_sort_key(window)
            .context("BGI window has no native sort key")? as i32;
        let (keyboard_scope, pointer_scope) = match &pending.request.data {
            NativeSelectionData::Items { .. } => (true, true),
            NativeSelectionData::Icons {
                input_mode,
                key_assignments,
                ..
            } => (
                *input_mode == 3 || key_assignments.is_some(),
                *input_mode == 3,
            ),
        };
        vm.register_selection_input(id, scope, keyboard_scope, pointer_scope);
        if let NativeSelectionData::Icons {
            key_assignments: Some(keys),
            ..
        } = &pending.request.data
        {
            for key in keys {
                let _ = vm.query_selection_descriptor(*key, scope);
            }
        }
        let _ = vm.query_selection_input(if keyboard_scope { scope } else { 0 }, scope);
        if matches!(&pending.request.data, NativeSelectionData::Icons { .. }) {
            let _ = vm.query_selection_input(0, scope.wrapping_add(0x8000));
        }
        let current = match &pending.request.data {
            NativeSelectionData::Items { initial, .. } => *initial,
            _ => -1,
        };
        let confirmation = matches!(
            &pending.request.data,
            NativeSelectionData::Items {
                blink_confirmation: true,
                ..
            }
        )
        .then_some(0);
        let state = Selection {
            id,
            visual_current: None,
            visual_target: None,
            finish_requested: false,
            cancelled: false,
            keyboard_scope,
            pointer_scope,
            request: pending.request,
            base,
            rects: pending.rects,
            current,
            previous_hover: -1,
            local: [0; 2],
            blink: 0,
            deadline: host.now_ms(),
            confirmation,
            last_tick: Some(host.now_ms()),
            serials: Vec::new(),
            scope,
            root_position,
        };
        let mut state = state;
        state.serials = vec![0; state.rects.len()];
        self.render_selection(host, &mut state)?;
        self.selections.active.insert(id, state);
        vm.commit_selection_start(id)?;
        Ok(true)
    }
    pub(crate) fn selection_window_position(&self, window: i32) -> Result<[i32; 2]> {
        self.graph
            .draw_position(window)
            .context("BGI selection window object missing")
    }
    fn draw_selection_text(
        &self,
        host: &mut dyn HostServices,
        window: i32,
        text: &str,
        position: [i32; 2],
        color: u32,
        id: u32,
    ) -> Result<Value> {
        let w = self.windows.get(window)?;
        let alias = self.font_alias(host, w.font)?;
        let library = host.font_library(self.client)?;
        let size = w.font_size as f32;
        let ascent = library.metrics(&alias, size, '字')?.ascent;
        let mut x = position[0] as f32;
        let mut glyphs = Vec::new();
        for ch in text.chars() {
            glyphs.push(json!({"character":ch,"baseline":[x,position[1]as f32+ascent]}));
            x += library.metrics(&alias, size, ch)?.advance.max(size * 0.5);
        }
        Ok(
            json!({"id":id,"rect":[0,0,w.size[0],w.size[1]],"paint":{"kind":"glyph_run","font":alias,"size":size,"bold":w.font_style!=0,"color":[(color>>16)as u8,(color>>8)as u8,color as u8,255],"glyphs":glyphs}}),
        )
    }
    fn render_selection(
        &mut self,
        host: &mut dyn HostServices,
        state: &mut Selection,
    ) -> Result<bool> {
        let window = state.request.window;
        let mut text_layers = Vec::new();
        let mut icons = BTreeMap::new();
        match &state.request.data {
            NativeSelectionData::Items {
                text,
                color,
                overlays,
                ..
            } => {
                self.draw_item_grid_rects(host, window, text, &state.rects, *color)?;
                if state.current >= 0 {
                    let i = state.current as usize;
                    let rect = state.rects[i];
                    let style = self.selections.styles[state.blink];
                    if style != 0 {
                        text_layers.push((
                            0,
                            self.draw_selection_text(
                                host,
                                window,
                                &text[i],
                                [rect[0], rect[1]],
                                style,
                                0,
                            )?,
                        ));
                    }
                    if let Some(overlays) = overlays {
                        if overlays[0][0] != -1 {
                            for (index, &[bitmap, dx, dy]) in overlays.iter().enumerate() {
                                if bitmap != -1 {
                                    icons.insert(
                                        index as i32 + 1,
                                        NativeIcon {
                                            bitmap,
                                            position: [
                                                rect[0].wrapping_add(dx).wrapping_add(
                                                    if index == 1 {
                                                        rect[2] - rect[0] + 1
                                                    } else {
                                                        0
                                                    },
                                                ),
                                                rect[1].wrapping_add(dy),
                                            ],
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
            NativeSelectionData::Icons {
                records, extended, ..
            } => {
                let w = self.windows.get(window)?;
                let target = *state.visual_target.get_or_insert(state.current);
                let mut operations = Vec::new();
                if state.visual_current.is_none() {
                    for r in records {
                        operations.push(WindowBlit {
                            bitmap: r[2],
                            position: [r[0], r[1]],
                            mode: 0,
                            parameter: 0,
                        });
                    }
                }
                let mut blit = |bitmap, r: &[i32; 16], mode, parameter| {
                    operations.push(WindowBlit {
                        bitmap,
                        position: [w.valid[0].wrapping_add(r[0]), w.valid[1].wrapping_add(r[1])],
                        mode,
                        parameter,
                    })
                };
                if !*extended && state.visual_current != Some(target) {
                    if let Some(old) = state.visual_current.filter(|old| *old >= 0) {
                        let r = &records[old as usize];
                        if self.window_image_source(r[3]).is_ok() {
                            blit(r[3], r, 0x40, 1);
                            blit(r[2], r, 0, 0);
                        }
                    }
                    if target >= 0 {
                        let r = &records[target as usize];
                        if self.window_image_source(r[3]).is_ok() {
                            blit(r[2], r, 0x40, 1);
                            blit(r[3], r, 0, 0);
                        }
                    }
                }
                let _ = w;
                if !operations.is_empty()
                    && !self.window_blend(
                        host,
                        window,
                        WindowBlendOwner::Selection(state.id),
                        false,
                        operations,
                    )?
                {
                    return Ok(false);
                }
                state.visual_current = Some(target);
                state.visual_target = None;
                if target != state.current {
                    return Ok(false);
                }
                if *extended {
                    let w = self.windows.get(window)?;
                    if state.current >= 0 {
                        let words = |i: usize| records.get(i / 16).map_or(0, |r| r[i % 16]);
                        for slot in 0..8 {
                            let at = state.current as usize * 16 + 3 + slot * 3;
                            let bitmap = words(at + 2);
                            if self.window_image_source(bitmap).is_ok() {
                                icons.insert(
                                    100 + slot as i32,
                                    NativeIcon {
                                        bitmap,
                                        position: [
                                            w.valid[0].wrapping_add(words(at)),
                                            w.valid[1].wrapping_add(words(at + 1)),
                                        ],
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        let w = self.windows.get_mut(window)?;
        w.icons.retain(|key, _| *key == -1);
        w.icons.extend(icons);
        w.text_overlays = text_layers.into_iter().collect();
        w.dirty = true;
        self.rebuild_window_icons(host, window)?;
        self.publish_windows(host)?;
        state.visual_current = Some(state.current);
        Ok(true)
    }
    pub(crate) fn refresh_native_pointer_nodes(
        &self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
    ) -> Result<()> {
        let mut nodes = self.graph_input_pointer_nodes(host)?;
        for state in self.selections.active.values() {
            let NativeSelectionData::Icons { input_mode, .. } = &state.request.data else {
                continue;
            };
            let mut covers = false;
            for index in 0..state.rects.len() {
                let item = state.base + index as u32 + 1;
                let interaction: Option<WidgetInteraction> = serde_json::from_slice(
                    &host.query_engine(self.client, EngineQuery::UiInteraction(item))?,
                )?;
                covers |= interaction.is_some_and(|i| i.hovered);
            }
            nodes.push((state.scope.wrapping_add(0x8000) as u32, covers));
            if matches!(input_mode, 1 | 2) {
                let origin = self.selection_window_position(state.request.window)?;
                let w = self.windows.get(state.request.window)?;
                nodes.push((
                    state.scope as u32,
                    self.input.pointer_x >= origin[0]
                        && self.input.pointer_y >= origin[1]
                        && self.input.pointer_x < origin[0].saturating_add(w.size[0] as i32)
                        && self.input.pointer_y < origin[1].saturating_add(w.size[1] as i32)
                        && self.graph.tree.chain_drawable(state.request.window),
                ));
            }
        }
        vm.update_native_pointer_nodes(nodes);
        Ok(())
    }
    pub(crate) fn tick_selection(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
    ) -> Result<()> {
        for poll in vm.take_selection_polls() {
            let entry = self.selections.callbacks.entry(poll.id).or_default();
            entry.0.extend(poll.callbacks);
            entry.1 |= poll.cancelled;
        }
        let ids = self.selections.active.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let mut state = self.selections.active.remove(&id).unwrap();
            let now = host.now_ms();
            let (callbacks, cancelled) = self.selections.callbacks.remove(&id).unwrap_or_default();
            state.cancelled |= cancelled;
            let mut finish = cancelled || state.finish_requested;
            let icons = matches!(&state.request.data, NativeSelectionData::Icons { .. });
            let mut changed = false;
            for [code, value] in callbacks {
                if icons && code == 512 {
                    state.current = if (value as usize) < state.rects.len() && value >= 0 {
                        value
                    } else {
                        -1
                    };
                    state.local = [0; 2];
                    finish = true;
                }
            }
            if !finish && state.last_tick != Some(now) {
                state.last_tick = Some(now);
                let position = self.selection_window_position(state.request.window)?;
                let w = self.windows.get(state.request.window)?;
                let modal = matches!(
                    &state.request.data,
                    NativeSelectionData::Items { .. }
                        | NativeSelectionData::Icons { input_mode: 3, .. }
                );
                let drawable = self.graph.global_enabled != 0
                    && self.graph.tree.chain_drawable(state.request.window);
                self.submit(host,json!({"ui":[{"op":"put","widget":{"id":state.base,"rect":[position[0],position[1],w.size[0],w.size[1]],"enabled":drawable,"kind":"panel","interaction_only":true,"keyboard_focusable":false,"modal":modal,"clip_children":false,"z":self.graph.tree.chain_position(state.request.window).unwrap_or(0)as i32+1000}}]}))?;
                state.root_position = position;
                let eligible: Option<bool> = serde_json::from_slice(
                    &host
                        .query_engine(self.client, EngineQuery::UiInputEligible(state.base + 1))?,
                )?;
                let eligible = eligible.unwrap_or(false)
                    && drawable
                    && (self.selections.poll_gate == 0
                        || host.window_focused(self.client)?.unwrap_or(true));
                let mut hover = -1;
                let mut activation = None;
                for index in 0..state.rects.len() {
                    let interaction: Option<WidgetInteraction> =
                        serde_json::from_slice(&host.query_engine(
                            self.client,
                            EngineQuery::UiInteraction(state.base + index as u32 + 1),
                        )?)?;
                    if let Some(interaction) = interaction {
                        if interaction.hovered {
                            hover = index as i32;
                        }
                        if interaction.activated
                            && interaction.activation_serial != state.serials[index]
                        {
                            state.serials[index] = interaction.activation_serial;
                            activation = Some(index as i32);
                        }
                    }
                }
                let full = match &state.request.data {
                    NativeSelectionData::Items { .. } => true,
                    NativeSelectionData::Icons { input_mode, .. } => *input_mode == 3,
                };
                let mut bits = if eligible {
                    let window_input = match &state.request.data {
                        NativeSelectionData::Items { .. } => true,
                        NativeSelectionData::Icons { input_mode, .. } => *input_mode != 0,
                    };
                    let bits = if window_input {
                        vm.query_selection_input(if full { state.scope } else { 0 }, state.scope)
                            as u32
                    } else {
                        0
                    };
                    let virtual_bits = if icons {
                        vm.query_selection_input(0, state.scope.wrapping_add(0x8000)) as u32
                    } else {
                        0
                    };
                    (bits | virtual_bits) & 0x7fffffff
                } else {
                    0
                };
                if !state.request.allow_cancel {
                    bits &= !0x202;
                }
                if eligible && state.confirmation.is_none() {
                    let key_selection = if let NativeSelectionData::Icons {
                        key_assignments: Some(keys),
                        ..
                    } = &state.request.data
                    {
                        keys.iter().position(|key| {
                            vm.query_selection_descriptor(*key, state.scope) != 0
                                || descriptor_bit(bits, *key)
                        })
                    } else {
                        None
                    };
                    if let Some(index) = key_selection {
                        state.current = index as i32;
                        state.local = [0; 2];
                        finish = true;
                        changed = true;
                    } else if let Some(index) = activation {
                        state.current = index;
                        finish = true;
                        changed = true;
                    } else if bits & 0x202 != 0 {
                        state.current = -1;
                        finish = true;
                        changed = true;
                    } else if icons {
                        if hover != state.current {
                            state.current = hover;
                            changed = true;
                            vm.post_message_event([
                                0x20000001,
                                state.request.owner_thread_id,
                                (hover as u16 as i32)
                                    | ((match &state.request.data {
                                        NativeSelectionData::Icons {
                                            records, extended, ..
                                        } => {
                                            hover >= 0
                                                && (*extended || records[hover as usize][3] != -1)
                                        }
                                        _ => false,
                                    } as i32)
                                        << 16),
                            ])?;
                        }
                        if state.current >= 0 {
                            let rect = state.rects[state.current as usize];
                            let origin = self.selection_window_position(state.request.window)?;
                            state.local = [
                                self.input
                                    .pointer_x
                                    .wrapping_sub(origin[0])
                                    .wrapping_sub(rect[0]),
                                self.input
                                    .pointer_y
                                    .wrapping_sub(origin[1])
                                    .wrapping_sub(rect[1]),
                            ];
                            if bits & 1 != 0 {
                                finish = true;
                            }
                        }
                    } else {
                        if state.confirmation.is_none() {
                            let old = state.current;
                            let mut picked = None;
                            for index in 0..10 {
                                if bits & (0x10000u32 << index) != 0 && index < state.rects.len() {
                                    picked = Some(index as i32);
                                    break;
                                }
                            }
                            if let Some(index) = picked {
                                state.current = index;
                                finish = true;
                            } else if bits & 0x100 != 0 {
                                finish = true;
                            } else {
                                let alternate =
                                    self.selections.navigation[0] != 0 && bits & 0xc0 != 0;
                                let direction = if alternate && bits & 0x40 != 0 {
                                    Some(GridDirection::Up)
                                } else if alternate && bits & 0x80 != 0 {
                                    Some(GridDirection::Down)
                                } else if bits & 0x1000 != 0 {
                                    Some(GridDirection::Up)
                                } else if bits & 0x2000 != 0 {
                                    Some(GridDirection::Down)
                                } else if bits & 0x4000 != 0 {
                                    Some(GridDirection::Left)
                                } else if bits & 0x8000 != 0 {
                                    Some(GridDirection::Right)
                                } else {
                                    None
                                };
                                if let Some(direction) = direction {
                                    let columns = match &state.request.data {
                                        NativeSelectionData::Items { columns, .. } => *columns,
                                        _ => 1,
                                    };
                                    let next = grid_neighbor(
                                        state.current,
                                        state.rects.len() as u32,
                                        columns as u32,
                                        direction,
                                    )?;
                                    if alternate && next >= 0 {
                                        let r = state.rects[next as usize];
                                        let origin =
                                            self.selection_window_position(state.request.window)?;
                                        let nav = self.selections.navigation;
                                        self.configure_cursor_motion(
                                            host,
                                            0,
                                            nav[3],
                                            nav[2],
                                            nav[1],
                                            origin[1].wrapping_add(
                                                r[1].wrapping_add((r[3] - r[1] + 1) >> 1),
                                            ),
                                            origin[0].wrapping_add(
                                                r[0].wrapping_add((r[2] - r[0] + 1) >> 1),
                                            ),
                                        )?;
                                    } else {
                                        state.current = next;
                                    }
                                } else if hover != state.previous_hover {
                                    let extended = matches!(
                                        &state.request.data,
                                        NativeSelectionData::Items {
                                            overlays: Some(_),
                                            ..
                                        }
                                    );
                                    if extended || hover >= 0 {
                                        state.current = hover;
                                    }
                                    state.previous_hover = hover;
                                    vm.post_message_event([
                                        0x10000002,
                                        state.request.owner_thread_id,
                                        hover,
                                    ])?;
                                }
                            }
                            if old != state.current {
                                changed = true;
                                state.blink = 0;
                                vm.post_message_event([
                                    0x10000001,
                                    state.request.owner_thread_id,
                                    state.current,
                                ])?;
                            }
                        }
                    }
                }
                if !icons {
                    let blink_confirm = matches!(
                        &state.request.data,
                        NativeSelectionData::Items {
                            blink_confirmation: true,
                            ..
                        }
                    );
                    if finish && blink_confirm && state.current >= 0 && !cancelled {
                        finish = false;
                        state.confirmation = Some(0);
                        state.deadline = now;
                    }
                    if let Some(ticks) = state.confirmation {
                        if ticks >= 20 || state.current == -1 {
                            finish = true;
                        } else if now >= state.deadline {
                            state.blink ^= usize::from(ticks != 0);
                            state.confirmation = Some(ticks + 1);
                            state.deadline = now.saturating_add(50);
                            changed = true;
                        }
                    } else if now >= state.deadline
                        && matches!(
                            &state.request.data,
                            NativeSelectionData::Items { overlays: None, .. }
                        )
                    {
                        state.blink ^= 1;
                        state.deadline =
                            now.wrapping_add(self.selections.blink_interval as u32 as u64);
                        changed = true;
                    }
                }
            }
            state.finish_requested |= finish;
            let visual_ready = if changed
                || state.visual_target.is_some()
                || state.visual_current.is_none()
                || finish
            {
                self.render_selection(host, &mut state)?
            } else {
                true
            };
            if state.finish_requested && visual_ready {
                let values = if icons {
                    [
                        state.local[0],
                        state.local[1],
                        if state.cancelled { -1 } else { state.current },
                    ]
                } else {
                    [
                        state.current,
                        if state.cancelled { -1 } else { state.current },
                        0,
                    ]
                };
                vm.complete_selection(
                    id,
                    NativeSelectionCompletion {
                        values,
                        count: if icons { 3 } else { 2 },
                    },
                )?;
                vm.unregister_selection_input(
                    id,
                    state.scope,
                    state.keyboard_scope,
                    state.pointer_scope,
                );
                self.submit(host, json!({"ui":[{"op":"remove","id":state.base}]}))?;
                self.windows.input_locks.remove(&state.request.window);
            } else {
                self.selections.active.insert(id, state);
            }
        }
        Ok(())
    }
}

pub(crate) fn descriptor_bit(bits: u32, descriptor: i32) -> bool {
    let shift = match descriptor {
        1 => 0,
        2 => 1,
        4 => 2,
        5 => 4,
        6 => 5,
        9 => 30,
        14 => 6,
        15 => 7,
        37 => 14,
        38 => 12,
        39 => 15,
        40 => 13,
        48 | 96 => 25,
        49..=57 => 16 + descriptor - 49,
        97..=105 => 16 + descriptor - 97,
        193 => 8,
        194 => 9,
        _ => return false,
    };
    bits & (1 << shift) != 0
}
