//! DCIPIcon/Ex policy over engine-owned UI widgets and window composition.
use super::*;
use crate::{
    input_policy::NativeGraphInputObject,
    window_api::{NativeWindow, WindowBlendOwner, WindowBlit},
};
use crysirror_bgi_vm::{
    GraphInputDescriptor, GraphInputRegion, HostMemoryWrite, NativeGraphPayload, NativeGraphRequest,
};
use crysirror_core::{
    image_processing::{ImageJob, ImageProcessState, ImageResource},
    service::{EngineQuery, buttons},
};
use crysirror_scene::ui::{WidgetHitMask, WidgetInteraction, hit_mask::CoverageChannel};
#[derive(Default)]
pub(crate) struct InputState {
    pub(crate) processors: BTreeMap<i32, NativeGraphInputObject>,
    live: BTreeMap<i32, Live>,
    pending: BTreeMap<u64, Configure>,
    surface: BTreeMap<i32, i32>,
    next: i32,
    widgets: u32,
    keys: [Option<[i32; 24]>; 4],
    dynamic_masks: BTreeMap<(i32, usize), DynamicMask>,
    mask_work: BTreeMap<(i32, usize), MaskRefresh>,
    last_tick: Option<u64>,
}
struct DynamicMask {
    bitmap: i32,
    size: [u32; 2],
    image: Arc<dyn ImageResource>,
    mask: WidgetHitMask,
}
struct MaskRefresh {
    bitmap: i32,
    size: [u32; 2],
    format: i32,
    image: Option<Arc<dyn ImageResource>>,
    capture: Option<ImageJob>,
    task: Option<Task<WidgetHitMask>>,
}
struct Live {
    base: u32,
    scope: i32,
    full: bool,
    window_input: bool,
    masks: BTreeMap<usize, WidgetHitMask>,
    widgets: BTreeMap<usize, Value>,
    rects: BTreeMap<usize, [f32; 4]>,
    serials: BTreeMap<usize, u32>,
    press_point: [i32; 2],
    initialized: bool,
}
struct Configure {
    object: i32,
    descriptor: GraphInputDescriptor,
    masks: Vec<(
        usize,
        [u32; 2],
        i32,
        Option<Arc<dyn ImageResource>>,
        Option<ImageJob>,
    )>,
    task: Option<Task<Vec<(usize, WidgetHitMask)>>>,
}
impl BgiVm {
    pub(crate) fn graph_input_pointer_nodes(
        &self,
        host: &mut dyn HostServices,
    ) -> Result<Vec<(u32, bool)>> {
        let mut nodes = Vec::new();
        for (&object, live) in &self.selections.inputs.live {
            let input = &self.selections.inputs.processors[&object];
            if input.enabled == 0
                || !self.graph.tree.chain_drawable(input.layer)
                || self.graph.global_enabled == 0
            {
                continue;
            }
            let mut covers = false;
            for &index in live.rects.keys() {
                let item = live.base + index as u32 + 1;
                let interaction: Option<WidgetInteraction> = serde_json::from_slice(
                    &host.query_engine(self.client, EngineQuery::UiInteraction(item))?,
                )?;
                covers |= interaction.is_some_and(|i| i.hovered);
            }
            nodes.push((live.scope.wrapping_add(0x8000) as u32, covers));
            if !live.full && live.window_input {
                let origin = self.selection_window_position(input.layer)?;
                let w = self.windows.get(input.layer)?;
                nodes.push((
                    live.scope as u32,
                    self.input.pointer_x >= origin[0]
                        && self.input.pointer_y >= origin[1]
                        && self.input.pointer_x < origin[0].saturating_add(w.size[0] as i32)
                        && self.input.pointer_y < origin[1].saturating_add(w.size[1] as i32),
                ));
            }
        }
        Ok(nodes)
    }
    pub(crate) fn graph_input_states(&self) -> Vec<(i32, i32)> {
        self.selections
            .inputs
            .processors
            .iter()
            .filter(|(id, _)| **id > 0)
            .map(|(&id, p)| (id, p.enabled))
            .collect()
    }
    pub(crate) fn graph_input_handles(&self) -> Vec<(i32, bool)> {
        self.selections
            .inputs
            .processors
            .iter()
            .filter(|(id, _)| **id > 0)
            .map(|(&id, p)| (id, p.extended))
            .collect()
    }
    pub(crate) fn release_window_inputs(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        window: i32,
    ) -> Result<()> {
        ensure!(
            !self
                .selections
                .inputs
                .processors
                .iter()
                .any(|(&id, p)| id > 0 && p.is_live() && p.layer == window),
            "指定されたウィンドウには協調しているプロシージャが存在します"
        );
        if let Some(object) = self.selections.inputs.surface.remove(&window) {
            self.release_input_live(host, vm, object)?;
            self.selections.inputs.processors.remove(&object);
        }
        Ok(())
    }
    fn input_create(&mut self, window: i32, extended: bool) -> Result<i32> {
        let state = &mut self.selections.inputs;
        state.next = state
            .next
            .checked_add(1)
            .context("BGI input processor handle exhausted")?;
        ensure!(state.next > 0, "BGI input processor handle overflow");
        state.processors.insert(
            state.next,
            if extended {
                NativeGraphInputObject::new_extended(window)
            } else {
                NativeGraphInputObject::new(window)
            },
        );
        Ok(state.next)
    }
    fn release_input_live(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        object: i32,
    ) -> Result<()> {
        self.selections
            .inputs
            .dynamic_masks
            .retain(|(id, _), _| *id != object);
        self.selections
            .inputs
            .mask_work
            .retain(|(id, _), _| *id != object);
        if let Some(live) = self.selections.inputs.live.remove(&object) {
            vm.unregister_selection_input(object as u64, live.scope, live.full, live.full);

            self.submit(host, json!({"ui":[{"op":"remove","id":live.base}]}))?;
        }
        if let Some(input) = self.selections.inputs.processors.get(&object) {
            if let Some(w) = self.windows.records.get_mut(&input.layer) {
                w.control_layers.remove(&object);
                w.dirty = true;
                let window = input.layer;
                self.rebuild_window_icons(host, window)?;
            }
        }
        Ok(())
    }
    fn input_configure(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        id: u64,
        object: i32,
        descriptor: GraphInputDescriptor,
    ) -> Result<bool> {
        let mut pending = if let Some(work) = self.selections.inputs.pending.remove(&id) {
            work
        } else {
            let input = self
                .selections
                .inputs
                .processors
                .get(&object)
                .context("BGI unknown icon processor")?;
            self.windows.get(input.layer)?;
            ensure!(
                descriptor.regions.len() <= 4095,
                "BGI icon controls exceed the shared UI widget budget"
            );
            let mut masks = Vec::new();
            let mut bytes = 0u64;
            for (index, region) in descriptor.regions.iter().enumerate() {
                let configured = if region.normal_resource != -1
                    && region.selected
                    && region.selected_resource != -1
                {
                    region.selected_resource
                } else {
                    region.normal_resource
                };
                let Ok((_, size)) = self.window_image_source(configured) else {
                    continue;
                };
                let (format, image, canvas) =
                    if let Some(b) = self.bitmaps.get(&(region.mask_resource as u32)) {
                        (b.format, b.image.clone(), region.mask_resource as u32)
                    } else if let Some(w) = self.windows.records.get(&region.mask_resource) {
                        (
                            2,
                            w.backing_raw.clone(),
                            NativeWindow::canvas(region.mask_resource, 0),
                        )
                    } else {
                        continue;
                    };
                bytes += u64::from(size[0]) * u64::from(size[1]);
                ensure!(bytes <= 64 * 1024 * 1024, "BGI icon mask byte budget");
                let job = if image.is_none() {
                    Some(host.begin_canvas_image(self.client, canvas)?)
                } else {
                    None
                };
                masks.push((index, size, format, image, job));
            }
            Configure {
                object,
                descriptor,
                masks,
                task: None,
            }
        };
        if pending.task.is_none() {
            for (_, _, _, image, job) in &mut pending.masks {
                if let Some(work) = job {
                    match work.poll()? {
                        ImageProcessState::Pending => {
                            self.selections.inputs.pending.insert(id, pending);
                            return Ok(false);
                        }
                        ImageProcessState::Failed(error) => {
                            bail!("BGI icon hit coverage capture: {error}")
                        }
                        ImageProcessState::Ready(result) => {
                            *image = Some(result);
                            *job = None;
                        }
                    }
                }
            }
            let masks = std::mem::take(&mut pending.masks);
            pending.task = Some(self.pool.spawn(move |cancel| {
                masks
                    .into_iter()
                    .map(|(index, size, format, image, _)| {
                        cancel.check()?;
                        let image = image.context("BGI icon mask image missing")?;
                        let channel = match format {
                            0 | 1 => CoverageChannel::MaxRgb,
                            2 => CoverageChannel::Alpha,
                            3 => CoverageChannel::Red,
                            _ => CoverageChannel::Zero,
                        };
                        Ok((
                            index,
                            WidgetHitMask::from_rgba_channels(
                                size,
                                image.size(),
                                &image.rgba(),
                                channel,
                                1,
                                &|| cancel.check(),
                            )?,
                        ))
                    })
                    .collect()
            })?);
        }
        let Some(masks) = pending.task.as_mut().unwrap().try_take()? else {
            self.selections.inputs.pending.insert(id, pending);
            return Ok(false);
        };
        self.release_input_live(host, vm, pending.object)?;
        let input = self
            .selections
            .inputs
            .processors
            .get_mut(&pending.object)
            .context("BGI processor released during configure")?;
        input.configure(pending.descriptor);
        let window = input.layer;
        let full = input.descriptor.flags[0] != 0;
        let window_input = full || input.descriptor.flags[1] != 0;
        let scope = self
            .graph
            .tree
            .native_sort_key(window)
            .context("BGI input window has no display order")? as i32;
        self.selections.inputs.widgets = self
            .selections
            .inputs
            .widgets
            .checked_add(4096)
            .context("BGI input widget serial exhausted")?;
        let base = 0xba000000u32
            .checked_add(self.selections.inputs.widgets)
            .context("BGI input widget namespace exhausted")?;
        self.selections.inputs.live.insert(
            object,
            Live {
                base,
                scope,
                full,
                window_input,
                masks: masks.into_iter().collect(),
                widgets: BTreeMap::new(),
                rects: BTreeMap::new(),
                serials: BTreeMap::new(),
                press_point: [0; 2],
                initialized: false,
            },
        );
        vm.register_selection_input(object as u64, scope, full, full);

        let _ = vm.query_selection_input(scope, scope);
        let _ = vm.query_selection_input(0, scope.wrapping_add(0x8000));
        self.refresh_input_controls(host, object)?;
        Ok(true)
    }
    fn input_image(&self, bitmap: i32) -> Option<(String, [u32; 2])> {
        if let Some(w) = self.windows.records.get(&bitmap) {
            return Some((w.backing_path.clone()?, w.size));
        }
        let b = self.bitmaps.get(&(bitmap as u32))?;
        Some((b.published_path.clone()?, b.size))
    }
    fn refresh_input_controls(&mut self, host: &mut dyn HostServices, object: i32) -> Result<()> {
        let Some(input) = self.selections.inputs.processors.get(&object) else {
            return Ok(());
        };
        let Some(live) = self.selections.inputs.live.get(&object) else {
            return Ok(());
        };
        let window = input.layer;
        let w = self.windows.get(window)?;
        let position = self.selection_window_position(window)?;
        let visible = self.graph.global_enabled != 0 && self.graph.tree.chain_drawable(window);
        let input_enabled = input.enabled != 0;
        let mut commands = vec![
            json!({"op":"put","widget":{"id":live.base,"rect":[position[0],position[1],w.size[0],w.size[1]],"enabled":visible&&input_enabled,"kind":"panel","interaction_only":true,"keyboard_focusable":false,"modal":live.full,"clip_children":false,"z":self.graph.tree.chain_position(window).unwrap_or(0)as i32+1000}}),
        ];
        let mut widgets = BTreeMap::new();
        let mut rects = BTreeMap::new();
        let mut layers = Vec::new();
        for (index, region) in input.descriptor.regions.iter().enumerate() {
            if region.enabled_depth == 0 {
                continue;
            }
            let configured = if region.normal_resource != -1
                && region.selected
                && region.selected_resource != -1
            {
                region.selected_resource
            } else {
                region.normal_resource
            };
            let Ok((_, hit_size)) = self.window_image_source(configured) else {
                continue;
            };
            let selected = input.is_current_selection(region.group, region.index);
            let hovered = input.hovered == Some((region.group, region.index));
            let preferred =
                if input.extended && selected && hovered && region.hover_selected_resource >= 0 {
                    region.hover_selected_resource
                } else if selected && region.selected_resource >= 0 {
                    region.selected_resource
                } else if hovered && region.hover_resource >= 0 {
                    region.hover_resource
                } else {
                    region.normal_resource
                };
            let Some((asset, size)) = self
                .input_image(preferred)
                .or_else(|| self.input_image(region.normal_resource))
            else {
                continue;
            };
            let origin = if input.descriptor.compact_uses_valid_region_origin {
                [w.valid[0], w.valid[1]]
            } else {
                [0; 2]
            };
            let x = region.x.saturating_add(origin[0]);
            let y = region.y.saturating_add(origin[1]);
            let z = input
                .item_depths
                .get(&(region.group, region.index))
                .copied()
                .unwrap_or(0);
            let (x, y, scale) = icon_projection(self.graph.screen, [x, y], z);
            let rect = [x, y, size[0] as f32 * scale, size[1] as f32 * scale];
            let depth = if region.flags & 0x10 != 0 {
                region.enabled_depth
            } else if region.flags & 2 != 0 {
                region.y
            } else {
                region.ordinal
            };
            let enabled = input.item_enabled(region.group, region.index);
            if enabled {
                layers.push(json!({"id":index as u32,"rect":rect,"order":[depth,index as i32],"paint":{"kind":"image","asset":asset}}));
            }
            let mask = if region.mask_resource == -2 {
                Some(WidgetHitMask::from_rgba_channels(
                    hit_size,
                    [0; 2],
                    &[],
                    CoverageChannel::Zero,
                    1,
                    &|| Ok(()),
                )?)
            } else {
                live.masks.get(&index).cloned().or_else(|| {
                    self.selections
                        .inputs
                        .dynamic_masks
                        .get(&(object, index))
                        .filter(|m| m.bitmap == region.mask_resource)
                        .map(|m| m.mask.clone())
                })
            };
            let mask_ready = live.masks.contains_key(&index)
                || region.mask_resource < 0
                || self.window_image_source(region.mask_resource).is_err()
                || mask.is_some();
            let value = json!({"op":"put","widget":{"id":live.base+index as u32+1,"parent":live.base,"rect":[x,y,hit_size[0],hit_size[1]],"enabled":enabled&&mask_ready&&input.pointer_hit_eligible(*region)&&input.descriptor.pointer_processing_enabled,"kind":"button","interaction_only":true,"keyboard_focusable":false,"activate_on_press":true,"hit_mask":mask,"z":depth}});
            if live.widgets.get(&index) != Some(&value) {
                commands.push(value.clone());
            }
            widgets.insert(index, value);
            rects.insert(index, rect);
        }
        for index in live.widgets.keys().filter(|i| !widgets.contains_key(i)) {
            commands.push(json!({"op":"remove","id":live.base+*index as u32+1}));
        }
        self.submit(host, json!({"ui":commands}))?;
        let live = self.selections.inputs.live.get_mut(&object).unwrap();
        live.widgets = widgets;
        live.rects = rects;
        let w = self.windows.get_mut(window)?;
        let changed = w.control_layers.get(&object) != Some(&layers);
        if changed {
            w.control_layers.insert(object, layers);
            w.dirty = true;
            self.rebuild_window_icons(host, window)?;
            self.publish_windows(host)?;
        }
        Ok(())
    }
    pub(crate) fn graph_input_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        id: u64,
        request: NativeGraphRequest,
    ) -> Result<bool> {
        let a = &request.args;
        let mut outputs = Vec::new();
        let mut writes = Vec::new();
        match (&request.payload, request.opcode.group, request.opcode.id) {
            (NativeGraphPayload::Grid(text), 0x90, 0xa1) => {
                self.draw_item_grid(host, a[0], text, a[3], a[4] != 0, a[5])?
            }
            (NativeGraphPayload::Columns(columns), 0x90, 0xa7) => {
                self.set_selection_columns(a[0], *columns)?
            }
            (NativeGraphPayload::Icons(records), 0x90, 0xb4 | 0xb5) => {
                let operations = records
                    .iter()
                    .filter(|r| self.window_image_source(r.bitmap).is_ok())
                    .map(|r| WindowBlit {
                        bitmap: r.bitmap,
                        position: [r.x, r.y],
                        mode: 0,
                        parameter: 0,
                    })
                    .collect();
                if !self.window_blend(host, a[0], WindowBlendOwner::Native(id), true, operations)? {
                    return Ok(false);
                }
            }
            (NativeGraphPayload::Descriptor { status, descriptor }, group, selector) => {
                if let Some(descriptor) = descriptor {
                    let object = if selector == 0xba {
                        a[0]
                    } else if let Some(object) = self.selections.inputs.surface.get(&a[0]) {
                        *object
                    } else {
                        let object = a[0];
                        self.selections.inputs.processors.insert(
                            object,
                            if selector == 0xb7 || group == 0x91 {
                                NativeGraphInputObject::new_extended(a[0])
                            } else {
                                NativeGraphInputObject::new(a[0])
                            },
                        );
                        self.selections.inputs.surface.insert(a[0], object);
                        object
                    };
                    if !self.input_configure(host, vm, id, object, descriptor.clone())? {
                        return Ok(false);
                    }
                }
                outputs.push(*status);
            }
            (NativeGraphPayload::None, group, 0xb8) => {
                outputs.push(self.input_create(a[0], group == 0x91)?)
            }
            (NativeGraphPayload::None, 0x90, 0xb9) => {
                self.release_input_live(host, vm, a[0])?;
                outputs.push(i32::from(
                    self.selections.inputs.processors.remove(&a[0]).is_some(),
                ));
                self.selections.inputs.surface.retain(|_, v| *v != a[0]);
                self.publish_windows(host)?;
            }
            (NativeGraphPayload::None, 0x91, 0xbb) => {
                let status = if let Some(input) = self.selections.inputs.processors.get_mut(&a[0]) {
                    input.set_item_state(a[1], a[2], a[3]).err().unwrap_or(0)
                } else {
                    1
                };
                if status == 0 {
                    self.refresh_input_controls(host, a[0])?;
                }
                outputs.push(status);
            }
            (NativeGraphPayload::KeyAssignment(keys), 0x91, 0xbf) => {
                ensure!(
                    (4..=7).contains(&a[0]),
                    "無効なキーアサインメント [ {} ] が指定されました",
                    a[0]
                );
                self.selections.inputs.keys[(a[0] - 4) as usize] = Some(*keys);
            }
            (NativeGraphPayload::None, 0x90, 0xbc..=0xbf) => {
                let values = if request.opcode.id == 0xbf {
                    self.selections
                        .inputs
                        .processors
                        .get_mut(&a[1])
                        .map(|p| p.pop_event().unwrap_or([0; 3]).to_vec())
                } else {
                    self.input_query_values(host, a[1], request.opcode.id)?
                };
                if let Some(values) = values {
                    writes.push(HostMemoryWrite {
                        address: a[0] as u32,
                        bytes: values.into_iter().flat_map(i32::to_le_bytes).collect(),
                    });
                    outputs.push(1);
                } else {
                    outputs.push(0);
                }
            }
            _ => bail!(
                "BGI unsupported typed graph request {:02x}:{:02x}",
                request.opcode.group,
                request.opcode.id
            ),
        }
        vm.update_graph_input_handles(self.graph_input_handles());
        vm.update_graph_input_states(self.graph_input_states());
        vm.commit_graph_input(id, &outputs, writes)?;
        self.calls += 1;
        Ok(true)
    }
    fn refresh_input_masks(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let mut entries = Vec::new();
        for (&object, live) in &self.selections.inputs.live {
            let input = &self.selections.inputs.processors[&object];
            for (index, r) in input.descriptor.regions.iter().enumerate() {
                if live.masks.contains_key(&index) || r.mask_resource < 0 {
                    continue;
                }
                let configured =
                    if r.normal_resource != -1 && r.selected && r.selected_resource != -1 {
                        r.selected_resource
                    } else {
                        r.normal_resource
                    };
                let Ok((_, size)) = self.window_image_source(configured) else {
                    continue;
                };
                let Ok((canvas, _)) = self.window_image_source(r.mask_resource) else {
                    continue;
                };
                let (format, image) = if let Some(b) = self.bitmaps.get(&(r.mask_resource as u32)) {
                    (b.format, b.image.clone())
                } else {
                    (2, self.windows.get(r.mask_resource)?.backing_raw.clone())
                };
                entries.push((
                    (object, index),
                    r.mask_resource,
                    size,
                    canvas,
                    format,
                    image,
                ));
            }
        }
        let bytes = entries
            .iter()
            .try_fold(0u64, |sum, (_, _, size, _, _, _)| {
                sum.checked_add(u64::from(size[0]) * u64::from(size[1]))
            })
            .context("BGI dynamic mask size overflow")?;
        ensure!(
            bytes <= 64 * 1024 * 1024,
            "BGI dynamic pointer mask byte budget"
        );
        for (key, bitmap, size, canvas, format, image) in entries {
            if self
                .selections
                .inputs
                .dynamic_masks
                .get(&key)
                .is_some_and(|cached| {
                    cached.bitmap == bitmap
                        && cached.size == size
                        && image
                            .as_ref()
                            .is_some_and(|image| Arc::ptr_eq(image, &cached.image))
                })
            {
                continue;
            }
            let existing = self
                .selections
                .inputs
                .mask_work
                .remove(&key)
                .filter(|work| work.bitmap == bitmap && work.size == size);
            let mut work = if let Some(work) = existing {
                work
            } else {
                let capture = if image.is_none() {
                    Some(host.begin_canvas_image(self.client, canvas)?)
                } else {
                    None
                };
                MaskRefresh {
                    bitmap,
                    size,
                    format,
                    image,
                    capture,
                    task: None,
                }
            };
            if let Some(capture) = &mut work.capture {
                match capture.poll()? {
                    ImageProcessState::Pending => {
                        self.selections.inputs.mask_work.insert(key, work);
                        continue;
                    }
                    ImageProcessState::Failed(error) => {
                        bail!("BGI dynamic pointer mask capture: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        work.image = Some(image);
                        work.capture = None;
                    }
                }
            }
            if work.task.is_none() {
                let image = work
                    .image
                    .clone()
                    .context("BGI dynamic pointer mask missing pixels")?;
                let size = work.size;
                let format = work.format;
                work.task = Some(self.pool.spawn(move |cancel| {
                    let channel = match format {
                        0 | 1 => CoverageChannel::MaxRgb,
                        2 => CoverageChannel::Alpha,
                        3 => CoverageChannel::Red,
                        _ => CoverageChannel::Zero,
                    };
                    WidgetHitMask::from_rgba_channels(
                        size,
                        image.size(),
                        &image.rgba(),
                        channel,
                        1,
                        &|| cancel.check(),
                    )
                })?);
            }
            if let Some(mask) = work.task.as_mut().unwrap().try_take()? {
                self.selections.inputs.dynamic_masks.insert(
                    key,
                    DynamicMask {
                        bitmap: work.bitmap,
                        size: work.size,
                        image: work.image.unwrap(),
                        mask,
                    },
                );
            } else {
                self.selections.inputs.mask_work.insert(key, work);
            }
        }
        Ok(())
    }
    fn input_query_values(
        &self,
        host: &mut dyn HostServices,
        object: i32,
        selector: u16,
    ) -> Result<Option<Vec<i32>>> {
        let Some(input) = self.selections.inputs.processors.get(&object) else {
            return Ok(None);
        };
        let values = match selector {
            0xbd => vec![input.registered_state],
            0xbe => input.selected_region_values(),
            0xbc => {
                let mut state = input.state_record();
                if state[3] != 0 {
                    let point = host.input_observation(self.client)?;
                    let held = input.group_pointer_activation_enabled(state[1])
                        && point.buttons_down & buttons::MOUSE_LEFT != 0;
                    if let Some(live) = self.selections.inputs.live.get(&object) {
                        let point = if held {
                            point.pointer.unwrap_or([0; 2])
                        } else {
                            live.press_point
                        };
                        let ordinal = input
                            .descriptor
                            .regions
                            .iter()
                            .position(|r| r.group == state[1] && r.index == state[2]);
                        let origin = self.selection_window_position(input.layer)?;
                        let local = ordinal
                            .and_then(|ordinal| live.rects.get(&ordinal))
                            .and_then(|rect| {
                                let x = point[0] as f32 - origin[0] as f32 - rect[0];
                                let y = point[1] as f32 - origin[1] as f32 - rect[1];
                                (x >= 0. && y >= 0. && x < rect[2] && y < rect[3])
                                    .then_some([x as i32, y as i32])
                            })
                            .unwrap_or([0; 2]);
                        state[4] = local[0];
                        state[5] = local[1];
                    }
                }
                state.to_vec()
            }
            _ => bail!("BGI invalid input state selector {selector:#x}"),
        };
        Ok(Some(values))
    }
    pub(crate) fn tick_graph_inputs(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
    ) -> Result<()> {
        for (id, value) in vm.take_graph_input_enable() {
            if let Some(input) = self.selections.inputs.processors.get_mut(&id) {
                input.enabled = value;
            }
        }
        for (id, words) in vm.take_graph_input_messages() {
            if let Some(input) = self.selections.inputs.processors.get_mut(&id) {
                input.queue_indirect_message(words);
            }
        }
        vm.update_graph_input_states(self.graph_input_states());
        let now = host.now_ms();
        if self.selections.inputs.last_tick == Some(now) {
            return Ok(());
        }
        self.selections.inputs.last_tick = Some(now);
        self.refresh_input_masks(host)?;
        let observation = host.input_observation(self.client)?;
        let mut objects = self
            .selections
            .inputs
            .processors
            .keys()
            .copied()
            .collect::<Vec<_>>();
        objects.reverse();
        for object in objects {
            self.refresh_input_controls(host, object)?;
            let Some(live) = self.selections.inputs.live.get(&object) else {
                continue;
            };
            let input = &self.selections.inputs.processors[&object];
            if !input.is_live() || input.enabled == 0 {
                continue;
            }
            let running = input.is_running();
            let mut hit = None;
            let mut clicked = false;
            let mut serials = Vec::new();
            for (&index, rect) in &live.rects {
                let state: Option<WidgetInteraction> =
                    serde_json::from_slice(&host.query_engine(
                        self.client,
                        EngineQuery::UiInteraction(live.base + index as u32 + 1),
                    )?)?;
                if let Some(state) = state {
                    if state.hovered {
                        let r = input.descriptor.regions[index];
                        hit = Some((
                            r,
                            ((self.input.pointer_x as f32 - live_position(self, input.layer)?[0])
                                - rect[0]) as i32,
                            ((self.input.pointer_y as f32 - live_position(self, input.layer)?[1])
                                - rect[1]) as i32,
                        ));
                    }
                    if state.activated
                        && live.serials.get(&index).copied().unwrap_or(0) != state.activation_serial
                    {
                        clicked = true;
                        serials.push((index, state.activation_serial));
                    }
                }
            }
            let mut eligible = false;
            for index in live.rects.keys() {
                eligible |= serde_json::from_slice::<Option<bool>>(&host.query_engine(
                    self.client,
                    EngineQuery::UiInputEligible(live.base + *index as u32 + 1),
                )?)?
                .unwrap_or(false);
                if eligible {
                    break;
                }
            }
            let (scope, full, window_input) = (live.scope, live.full, live.window_input);
            let mut bits = if eligible && input.descriptor.pointer_processing_enabled {
                let bits = if window_input {
                    vm.query_selection_input(if full { scope } else { 0 }, scope) as u32
                } else {
                    0
                };
                bits | vm.query_selection_input(0, scope.wrapping_add(0x8000)) as u32
            } else {
                0
            };
            if !full {
                bits &= 0x2c3;
            }
            if eligible
                && (clicked
                    || (observation.buttons_down & buttons::MOUSE_LEFT != 0
                        && hit.is_some_and(|(r, _, _)| {
                            input.group_pointer_activation_enabled(r.group)
                        })))
            {
                bits |= 1;
            }
            bits &= !(input.descriptor.flags[2] as u32);
            let mode = (input.descriptor.flags[3] & 7) as usize;
            let table = if mode < 4 {
                ACTION_TABLES[mode]
            } else {
                self.selections.inputs.keys[mode - 4].unwrap_or([0; 24])
            };
            let mut action = ACTION_BITS
                .iter()
                .position(|b| bits & b != 0)
                .map_or(0, |i| table[i]);
            if action == 19 {
                action = if observation.buttons_down & buttons::SHIFT != 0 {
                    7
                } else {
                    8
                };
            }
            let key_activation = if running && eligible && full && action == 0 && !input.extended {
                input
                    .descriptor
                    .regions
                    .iter()
                    .copied()
                    .filter(|r| r.enabled_depth != 0 && r.flags & i32::MAX != 0)
                    .find(|r| {
                        let descriptor = r.flags & i32::MAX;
                        (r.flags < 0 && vm.input_descriptor_repeat_ready(descriptor))
                            || crate::selection_api::descriptor_bit(bits, descriptor)
                            || vm.query_selection_descriptor(descriptor, scope) != 0
                    })
            } else {
                None
            };
            let live = self.selections.inputs.live.get_mut(&object).unwrap();
            for (i, serial) in serials {
                live.serials.insert(i, serial);
            }
            if clicked {
                live.press_point = [self.input.pointer_x, self.input.pointer_y];
            }
            let initialized = live.initialized;
            live.initialized = true;
            let native_origin = if full {
                [0; 2]
            } else {
                self.selection_window_position(self.selections.inputs.processors[&object].layer)?
            };
            let input = self.selections.inputs.processors.get_mut(&object).unwrap();
            while input.enabled != 0 && input.is_live() {
                let Some(words) = input.indirect_messages.pop_front() else {
                    break;
                };
                if words.first() == Some(&0) {
                    if words.len() == 2 {
                        input.enabled = words[1];
                    }
                    continue;
                }
                input.handle_message(&words, &|group, item| {
                    hit.filter(|(r, _, _)| r.group == group && r.index == item)
                        .map_or((0, 0), |(_, x, y)| (x, y))
                });
            }
            if input.enabled == 0 {
                self.refresh_input_controls(host, object)?;
                continue;
            }
            let raw = hit.map(|(r, _, _)| (r.group, r.index));
            if initialized && input.raw_hit != raw {
                input.queue_event(raw.map_or([0x10000001, -1, -1], |(g, i)| [0x10000001, g, i]));
            }
            input.raw_hit = raw;
            let old = input.hovered;
            input.observe_hit(hit);
            if old != input.hovered {
                let (payload, parameter, suppress) = hit.map_or((-1, 0, false), |(r, _, _)| {
                    (
                        crate::input_policy::pack_words(r.group, r.index),
                        i32::from(r.hover_resource != -1),
                        r.flags & 4 != 0,
                    )
                });
                if !input.extended || !suppress || hit.is_none() {
                    input.queue_event([0x10000002, payload, parameter]);
                }
                if initialized && let Some((r, _, _)) = hit {
                    if input.select_pointer_item(r.group, r.index) {
                        input.queue_event([
                            0x10000004,
                            crate::input_policy::pack_words(r.group, r.index),
                            0,
                        ]);
                    }
                }
            }
            let mut released = false;
            if let Some(key) = input.deferred_pointer_activation() {
                if action != 0 {
                    input.clear_deferred_pointer_activation();
                } else if observation.buttons_down & buttons::MOUSE_LEFT == 0 {
                    if eligible && hit.is_some_and(|(r, _, _)| (r.group, r.index) == key) {
                        action = 1;
                        released = true;
                    } else {
                        input.clear_deferred_pointer_activation();
                    }
                }
            }
            if !running {
                action = 0;
            }
            if action == 1 {
                if let Some((r, x, y)) = hit {
                    if released || input.pointer_activation_is_immediate(r) {
                        input.activate_native_action_item(r, (x, y), true);
                        input.clear_deferred_pointer_activation();
                    } else {
                        input.defer_pointer_activation(r);
                    }
                } else if input.extended {
                    let point = observation.pointer.unwrap_or([0; 2]);
                    let origin = native_origin;
                    input.queue_event([
                        0x10000007,
                        -1,
                        (point[0].wrapping_sub(origin[0]) & 0xffff)
                            | (point[1].wrapping_sub(origin[1]) << 16),
                    ]);
                }
            } else if (2..=18).contains(&action) {
                input.native_action(action);
            } else if let Some(region) = key_activation {
                input.activate_native_action_item(region, (0, 0), false);
            }
            self.refresh_input_controls(host, object)?;
        }
        vm.update_graph_input_handles(self.graph_input_handles());
        vm.update_graph_input_states(self.graph_input_states());
        Ok(())
    }
}
fn live_position(vm: &BgiVm, window: i32) -> Result<[f32; 2]> {
    Ok(vm.selection_window_position(window)?.map(|p| p as f32))
}
fn icon_projection(screen: [u32; 2], point: [i32; 2], z: i32) -> (f32, f32, f32) {
    if z == 0 {
        return (point[0] as f32, point[1] as f32, 1.);
    }
    let p = i64::from((screen[0] / 2).max(1));
    let z = i64::from(z) << 16;
    let scale = if z < 0 {
        ((p << 16) - z) / p
    } else {
        (p << 32) / (z + (p << 16))
    } as f32
        / 65536.;
    let cx = (screen[0] / 2) as f32;
    let cy = (screen[1] / 2) as f32;
    (
        cx + (point[0] as f32 - cx) * scale,
        cy + (point[1] as f32 - cy) * scale,
        scale,
    )
}
const ACTION_BITS: [u32; 24] = [
    1, 2, 4, 16, 32, 64, 128, 256, 512, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288,
    1048576, 2097152, 4194304, 8388608, 16777216, 33554432, 0x40000000,
];
const ACTION_TABLES: [[i32; 24]; 4] = [
    [
        1, 2, 0, 0, 0, 0, 0, 3, 4, 7, 8, 5, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 19,
    ],
    [
        1, 2, 0, 0, 0, 0, 0, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 19,
    ],
    [
        1, 2, 0, 0, 0, 0, 0, 3, 4, 11, 12, 9, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 19,
    ],
    [
        1, 2, 0, 0, 0, 0, 0, 3, 4, 9, 10, 11, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 19,
    ],
];
