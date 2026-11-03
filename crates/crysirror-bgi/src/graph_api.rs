//! BGI handle classes, native member relations and display-chain ordering.
//! Canvas/Layer retain all drawable content; this file owns no renderer.
use super::*;
use crate::{
    display_tree::{NativeDisplayKind as Kind, NativeDisplayTree},
    graph_state::RuntimeGraphObjectProperties as Properties,
};
use crysirror_scene::layer::{Blend, SpriteTransform};
#[path = "graph_backf.rs"]
mod graph_backf;
#[path = "graph_composition.rs"]
mod graph_composition;
#[path = "graph_configure.rs"]
mod graph_configure;
#[path = "graph_effector.rs"]
mod graph_effector;
#[path = "graph_field.rs"]
mod graph_field;
#[path = "graph_filter.rs"]
mod graph_filter;
#[path = "graph_images.rs"]
mod graph_images;
#[path = "graph_map.rs"]
mod graph_map;
#[path = "graph_mask.rs"]
mod graph_mask;
#[path = "graph_projection.rs"]
mod graph_projection;
use crate::native_background::{NativeBackgroundClass as BackClass, NativeBackgroundState};
use crysirror_image::integer_raster::{IntegerRasterOperation as Raster, display::DisplayRaster};
use graph_images::{GraphImages, GraphInput, GraphPlan};
pub(crate) struct GraphState {
    pub screen: [u32; 2],
    pub renderer_options: [i32; 2],
    pub global_enabled: i32,
    pub default_priority: i32,
    images: GraphImages,
    compositions: BTreeMap<i32, graph_composition::SpriteComposition>,
    projections: BTreeMap<i32, graph_projection::ProjectionState>,
    fixed_vectors: BTreeMap<i32, [[i32; 3]; 2]>,
    sprite_relations: BTreeMap<i32, i32>,
    sprite_mask_owners: BTreeMap<i32, i32>,
    sprite_link_masks: std::collections::BTreeSet<i32>,
    aux_bindings: BTreeMap<i32, (i32, u64)>,
    mask_signatures: BTreeMap<i32, Value>,
    mask_unready: std::collections::BTreeSet<i32>,
    sprite_affine: BTreeMap<i32, [i32; 6]>,
    raster_offsets: BTreeMap<i32, [i32; 2]>,
    affine_rebuild: std::collections::BTreeSet<i32>,
    bitmap_versions: BTreeMap<u32, u64>,
    bitmap_clock: u64,
    filters: BTreeMap<i32, graph_filter::NativeFilter>,
    fields: BTreeMap<u32, Arc<crysirror_core::image_processing::VectorFieldResource>>,
    waves: BTreeMap<i32, crysirror_image::vector_field::WaveTable>,
    field_jobs: BTreeMap<u64, graph_field::FieldWork>,
    field_rebuild: std::collections::BTreeSet<i32>,
    sprite_wave_args: BTreeMap<i32, Vec<i32>>,
    needs_publish: bool,
    visual_clock: u64,
    visual_versions: BTreeMap<i32, u64>,
    visual_descriptors: BTreeMap<i32, Value>,
    maps: BTreeMap<i32, graph_map::NativeMap>,
    frames: BTreeMap<i32, Vec<(i32, u64)>>,
    crops: BTreeMap<i32, [u32; 4]>,
    live: BTreeMap<i32, bool>,
    background_args: BTreeMap<i32, (u16, Vec<i32>)>,
    pending: BTreeMap<u64, (Vec<i32>, Vec<crysirror_bgi_vm::HostMemoryWrite>, Vec<i32>)>,
    pub tree: NativeDisplayTree,
    pub objects: BTreeMap<i32, Properties>,
    serials: BTreeMap<Kind, u32>,
    members: BTreeMap<i32, BTreeMap<i32, [i32; 2]>>,
    pub display_offset: [i32; 2],
    pub shake_offset: [i32; 2],
    pub center: [i32; 2],
    pub transforms: BTreeMap<i32, SpriteTransform>,
}
impl Default for GraphState {
    fn default() -> Self {
        Self {
            screen: [1280, 720],
            renderer_options: [1, 0],
            global_enabled: 1,
            default_priority: 0,
            images: Default::default(),
            compositions: BTreeMap::new(),
            projections: BTreeMap::new(),
            fixed_vectors: BTreeMap::new(),
            sprite_relations: BTreeMap::new(),
            sprite_mask_owners: BTreeMap::new(),
            sprite_link_masks: Default::default(),
            aux_bindings: BTreeMap::new(),
            mask_signatures: BTreeMap::new(),
            mask_unready: Default::default(),
            sprite_affine: BTreeMap::new(),
            raster_offsets: BTreeMap::new(),
            affine_rebuild: Default::default(),
            bitmap_versions: BTreeMap::new(),
            bitmap_clock: 0,
            filters: BTreeMap::new(),
            fields: BTreeMap::new(),
            waves: BTreeMap::new(),
            field_jobs: BTreeMap::new(),
            field_rebuild: Default::default(),
            sprite_wave_args: BTreeMap::new(),
            needs_publish: false,
            visual_clock: 0,
            visual_versions: BTreeMap::new(),
            visual_descriptors: BTreeMap::new(),
            maps: BTreeMap::new(),
            frames: BTreeMap::new(),
            crops: BTreeMap::new(),
            live: BTreeMap::new(),
            background_args: BTreeMap::new(),
            pending: BTreeMap::new(),
            tree: Default::default(),
            objects: BTreeMap::new(),
            serials: BTreeMap::new(),
            members: BTreeMap::new(),
            display_offset: [0; 2],
            shake_offset: [0; 2],
            center: [-1; 2],
            transforms: BTreeMap::new(),
        }
    }
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    graph_effector::handles(group, id)
        || graph_field::handles(group, id)
        || matches!(
            (group, id),
            (
                0x90,
                0x60 | 0x61
                    | 0x64
                    | 0x65
                    | 0x66
                    | 0x70
                    | 0x71
                    | 0x74
                    | 0x75
                    | 0x76
                    | 0x78
                    | 0x79
                    | 0x7a
                    | 0x06
                    | 0x08
                    | 0x09
                    | 0x30
                    | 0x31
                    | 0x33
                    ..=0x3a
                        | 0x3f
                        | 0x44
                        | 0x43
                        | 0x45
                        | 0x47
                        | 0x40
                        | 0x42
                        | 0x41
                        | 0x46
                        | 0x48
                        | 0x49
                        | 0x4a
                        | 0x4c
                        | 0x4d
                        | 0x50
                        | 0x51
                        | 0x53
                        | 0x54
                        | 0x55
                        | 0x56
                        | 0x57
                        | 0x58
                        | 0x59
                        | 0x5a
                        | 0x5b
                        | 0x5c
                        | 0x5d
                        | 0xe0
                        | 0xe1
                        | 0xe4
                        | 0xe5
                        | 0xe8
                        | 0xe9
            ) | (
                0x91,
                0x06 | 0x31 | 0x33 | 0x36 | 0x37 | 0x38 | 0x3d | 0x3e | 0x3f | 0x55
            )
        )
}
impl GraphState {
    pub(crate) fn draw_position(&self, object: i32) -> Option<[i32; 2]> {
        let n = &self.objects.get(&object)?.native;
        let raster = self.raster_offsets.get(&object).copied().unwrap_or([0; 2]);
        let offset = if n.global_display_offset_enabled != 0 {
            self.display_offset
        } else {
            [0; 2]
        };
        Some([
            n.position_x
                .wrapping_add(n.primary_offset_x)
                .wrapping_add(n.secondary_offset_x)
                .wrapping_add(offset[0])
                .wrapping_add(self.shake_offset[0])
                .wrapping_add(raster[0]),
            n.position_y
                .wrapping_add(n.primary_offset_y)
                .wrapping_add(n.secondary_offset_y)
                .wrapping_add(offset[1])
                .wrapping_add(self.shake_offset[1])
                .wrapping_add(raster[1]),
        ])
    }

    pub(crate) fn visual_prefix(&self, object: i32) -> Vec<(i32, u64)> {
        let Some(cut) = self.tree.chain_position(object) else {
            return Vec::new();
        };
        self.tree
            .chain_handles()
            .iter()
            .take(cut as usize)
            .map(|id| (*id, self.visual_versions.get(id).copied().unwrap_or(0)))
            .collect()
    }

    pub(crate) fn mark_visual(&mut self, object: i32) {
        self.visual_clock = self.visual_clock.wrapping_add(1);
        self.visual_versions.insert(object, self.visual_clock);
        self.needs_publish = true;
    }
    pub(crate) fn visual_descriptor(&mut self, object: i32, descriptor: Value) {
        if self.visual_descriptors.get(&object) != Some(&descriptor) {
            self.visual_descriptors.insert(object, descriptor);
            self.mark_visual(object);
        }
    }
    pub(crate) fn require(&self, id: i32) -> Result<()> {
        ensure!(self.tree.contains(id), "BGI invalid display object {id:#x}");
        Ok(())
    }
    pub(crate) fn allocate(&mut self, tag: u32, slots: u32, kind: Kind) -> i32 {
        let Some(id) = (0..slots)
            .map(|n| (tag | n) as i32)
            .find(|&id| !self.tree.contains(id))
        else {
            return 0;
        };
        self.tree.register(id, kind);
        let native = &mut self.objects.entry(id).or_default().native;
        native.sort_class = match kind {
            Kind::Map | Kind::Landscape => 1,
            Kind::Sprite => 2,
            Kind::Window => 3,
            Kind::ParticleScreen => 4,
            Kind::RainScreen => 5,
            Kind::Effector => 6,
            Kind::Filter => 7,
            Kind::Group => 9,
            Kind::Knob => 10,
            _ => 0,
        };
        let serial = self.serials.entry(kind).or_default();
        native.sort_index = *serial;
        *serial = serial.wrapping_add(1);
        self.sort(id);
        id
    }
    pub(crate) fn sort(&mut self, id: i32) {
        let Some(properties) = self.objects.get(&id) else {
            return;
        };
        let n = &properties.native;
        let low = if n.fixed_position_updates_integer_position != 0 {
            n.sort_index
        } else {
            let vectors = self.fixed_vectors.get(&id).copied().unwrap_or([[0; 3]; 2]);
            let z = n
                .fixed_position_z_16_16
                .wrapping_add(vectors[0][2])
                .wrapping_add(vectors[1][2]);
            4095i32.wrapping_sub(z >> 19) as u32
        } & 0x1fff;
        let key = n
            .sort_class
            .min(7)
            .wrapping_add(n.priority.wrapping_mul(8))
            .wrapping_shl(13)
            .wrapping_add(n.sort_bias as u32)
            .wrapping_add(low);
        self.tree.set_chain_depth(id, n.priority as i32);
        self.tree.set_native_sort_key(id, key);
    }
    pub(crate) fn descendants(&self, id: i32) -> Vec<i32> {
        self.tree.descendants_inclusive(id)
    }
    pub(crate) fn position(&mut self, id: i32, x: i32, y: i32) {
        // Native setters update an attached member's local record as well.
        if let Some(parent) = self.tree.parent(id) {
            let p = self
                .objects
                .get(&parent)
                .map(|p| [p.native.position_x, p.native.position_y])
                .unwrap_or([0; 2]);
            self.members
                .entry(parent)
                .or_default()
                .insert(id, [x.wrapping_sub(p[0]), y.wrapping_sub(p[1])]);
        }
        let mut pending = vec![(id, [x, y])];
        while let Some((id, point)) = pending.pop() {
            let n = &mut self.objects.entry(id).or_default().native;
            let changed = n.position_x != point[0] || n.position_y != point[1];
            n.position_x = point[0];
            n.position_y = point[1];
            if changed {
                self.mark_visual(id);
            }
            if let Some(children) = self.members.get(&id) {
                for (&child, &offset) in children {
                    pending.push((
                        child,
                        [
                            point[0].wrapping_add(offset[0]),
                            point[1].wrapping_add(offset[1]),
                        ],
                    ));
                }
            }
        }
    }
}
impl BgiVm {
    pub(crate) fn set_screen_shake_offset(&mut self, offset: [i32; 2]) {
        if self.graph.shake_offset != offset {
            self.graph.shake_offset = offset;
            let ids = self.graph.tree.chain_handles().to_vec();
            for id in ids {
                self.graph.mark_visual(id);
            }
        }
    }
    pub(crate) fn graph_prefix_ready(&self, object: i32) -> bool {
        let Some(cut) = self.graph.tree.chain_position(object) else {
            return false;
        };
        !self.graph.images.dirty.iter().any(|id| {
            *id != object
                && self
                    .graph
                    .tree
                    .chain_position(*id)
                    .is_some_and(|position| position < cut)
        }) && self.window_prefix_ready(object)
            && self.sprite_prefix_ready(object)
    }
    pub(crate) fn native_graph_layer(id: i32) -> u32 {
        if (id as u32 >> 24) == 0x80 {
            (id as u32 & 0x1ff) + 1
        } else {
            0x60000000 ^ (id as u32)
        }
    }
    pub(super) fn hide_graph_object(&mut self, host: &mut dyn HostServices, id: i32) -> Result<()> {
        if let Some(image) = self.graph.live.remove(&id) {
            self.graph.mark_visual(id);
            let layer = if self
                .graph
                .objects
                .get(&id)
                .is_some_and(|p| p.background.is_some())
            {
                1024
            } else {
                Self::native_graph_layer(id)
            };
            let layers = if image {
                vec![json!({"op":"remove","id":layer})]
            } else {
                Vec::new()
            };
            self.submit(
                host,
                json!({"canvases":[{"op":"hide","layer":layer}],"layers":layers}),
            )?;
        }
        Ok(())
    }
    pub(crate) fn publish_graph(&mut self, host: &mut dyn HostServices) -> Result<()> {
        self.release_graph_retired(host)?;
        let ids = self.graph.tree.chain_handles().to_vec();
        for (order, id) in ids.into_iter().enumerate() {
            if self.graph.images.expired.contains(&id) {
                self.hide_graph_object(host, id)?;
                continue;
            }
            // Windows own their three-pass publication below. Bookkeeping
            // objects have no pixels and must not churn filter signatures.
            if self.graph.tree.kind(id) == Some(Kind::Window)
                || (!self.graph.images.painted.contains_key(&id)
                    && self
                        .sprites
                        .get(&(id as u32))
                        .and_then(|s| s.bitmap)
                        .is_none())
            {
                continue;
            }
            let Some(p) = self.graph.objects.get(&id) else {
                continue;
            };
            if self.graph.images.expired.contains(&id) {
                continue;
            }
            let n = p.native;
            let background = p.background;
            let is_back = background.is_some();
            let is_filter = self.graph.filters.contains_key(&id);
            let visible = !self.graph.images.expired.contains(&id)
                && self.graph.global_enabled != 0
                && n.draw_enabled != 0
                && n.suppress_draw == 0
                && p.mask_alpha < 256
                && p.alpha_multiplier != 0
                && self.graph.tree.chain_drawable(id)
                && !self.graph.sprite_mask_owners.contains_key(&id)
                && !self.graph.mask_unready.contains(&id);
            let position = if background.is_some_and(|b| b.class == BackClass::BackF) {
                [0; 2]
            } else {
                self.graph
                    .draw_position(id)
                    .context("BGI drawable object position")?
            };
            let layer = if is_back {
                1024
            } else {
                Self::native_graph_layer(id)
            };
            let transform = self.graph.transforms.get(&id).cloned().unwrap_or_default();
            let is_composited_sprite =
                matches!(self.graph.tree.kind(id), Some(Kind::Sprite | Kind::Map))
                    && p.blend_mode != 128;
            let native_blend = p.blend_mode;
            let blend = if is_back || is_filter || is_composited_sprite {
                Blend::Normal
            } else {
                match p.blend_mode {
                    0 | 1 | 128 => Blend::Normal,
                    2 => Blend::Add,
                    3 => Blend::Subtract,
                    _ => bail!(
                        "BGI display blend selector {} requires its engine composition profile",
                        p.blend_mode
                    ),
                }
            };
            let opacity = if is_composited_sprite
                || is_filter
                || (is_back
                    && p.background
                        .is_some_and(|b| !matches!(b.class, BackClass::BackN | BackClass::BackS)))
            {
                255
            } else {
                (p.opacity() * 255.).round() as u8
            };
            // Compare only reusable layer state; hit masks and VM bookkeeping
            // do not invalidate destination-reading effects.
            self.graph.visual_descriptor(id,json!({"position":position,"visible":visible,"opacity":opacity,"blend":blend,"transform":transform,"order":[n.priority,order],"crop":self.graph.crops.get(&id),"native_blend":native_blend}));
            if is_composited_sprite {
                let (paint, size) = self.sprite_composition_paint(host, id, position, visible)?;
                self.submit(host,json!({"canvases":[{"op":"hide","layer":layer}],"layers":[{"op":"put","layer":{"id":layer,"rect":[0,0,size[0],size[1]],"paint":paint,"order":[n.priority,order as i32],"opacity":255,"visible":visible,"blend":"normal"}}]}))?;
                self.graph.live.insert(id, true);
                continue;
            }
            self.remove_sprite_composition(host, id)?;
            if let Some(image) = self.graph.images.painted.get(&id) {
                let size = image.image.size();
                let crop = self
                    .graph
                    .crops
                    .get(&id)
                    .copied()
                    .unwrap_or([0, 0, size[0], size[1]]);
                let uv = [
                    crop[0] as f32 / size[0] as f32,
                    crop[1] as f32 / size[1] as f32,
                    (crop[0] + crop[2]) as f32 / size[0] as f32,
                    (crop[1] + crop[3]) as f32 / size[1] as f32,
                ];
                let paint = if background.is_some_and(|b| b.active == 0) {
                    json!({"kind":"solid","color":[0,0,0,255]})
                } else {
                    json!({"kind":"image","asset":image.path})
                };
                self.submit(host,json!({"canvases":[{"op":"hide","layer":layer}],"layers":[{"op":"put","layer":{"id":layer,"rect":[position[0],position[1],crop[2],crop[3]],"uv":uv,"paint":paint,"order":[n.priority,order as i32],"transform":transform,"opacity":opacity,"visible":visible,"blend":blend}}]}))?;
                self.graph.live.insert(id, true);
            } else if let Some(bitmap) = self.sprites.get(&(id as u32)).and_then(|s| s.bitmap) {
                self.submit(host,json!({"canvases":[{"op":"present","id":bitmap,"layer":layer,"order":n.priority,"position":position}],"layers":[{"op":"set_opacity","id":layer,"opacity":opacity},{"op":"set_visible","id":layer,"visible":visible},{"op":"set_order","id":layer,"order":[n.priority,order as i32]},{"op":"set_transform","id":layer,"transform":transform},{"op":"set_blend","id":layer,"blend":blend}]}))?;
                self.graph.live.insert(id, false);
            }
        }
        self.publish_windows(host)?;
        self.graph.needs_publish = false;
        Ok(())
    }
    pub(crate) fn graph_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if graph_field::handles(call.group, call.id) {
            return self.native_field_call(host, vm, call);
        }
        let request = vm.request_id().context("BGI graph continuation identity")?;
        if let Some((output, writes, objects)) = self.graph.pending.remove(&request) {
            for &object in &objects {
                self.refresh_graph_image(host, object)?;
            }
            if objects
                .iter()
                .any(|object| self.graph.images.dirty.contains(object))
            {
                self.graph
                    .pending
                    .insert(request, (output, writes, objects));
                return Ok(false);
            }
            self.publish_graph(host)?;
            vm.commit_call_writes(&output, writes)?;
            self.calls += 1;
            return Ok(true);
        }
        let a = &call.args;
        let target = if call.group == 0x90 && matches!(call.id, 0x40..=0x4d) {
            0
        } else if call.group == 0x91 && matches!(call.id, 0x38 | 0x3d) {
            a.get(1).copied().unwrap_or(0)
        } else {
            a.first().copied().unwrap_or(0)
        };
        let affected = self.graph.descendants(target);
        if self
            .graph
            .pending
            .values()
            .any(|(_, _, objects)| objects.iter().any(|id| affected.contains(id)))
        {
            return Ok(false);
        }
        let before = self.graph.images.serial;
        let mut output = Vec::new();
        let mut writes = Vec::new();
        match (call.group, call.id) {
            (0x90, 0x55) | (0x91, 0x55) => {
                if let Some(result) = self.sprite_mask_call(call)? {
                    output.push(result);
                }
            }
            (0x91, 0x60 | 0x61 | 0x64..=0x69) => {
                if let Some(result) = self.native_effector_call(host, call)? {
                    output.push(result);
                }
            }
            (0x90, 0x60 | 0x61 | 0x64 | 0x65 | 0x66) => {
                if let Some(result) = self.native_filter_call(host, call)? {
                    output.push(result);
                }
            }
            (0x90, 0x70 | 0x71 | 0x74 | 0x75 | 0x76 | 0x78 | 0x79 | 0x7a) => {
                if let Some(result) = self.native_map_call(host, vm, call)? {
                    output.push(result);
                }
            }
            (0x90, 0x06) => {
                self.graph.center = [a[0], a[1]];
                let objects = self.graph.projections.keys().copied().collect::<Vec<_>>();
                for id in objects {
                    self.rebuild_native_projection(id)?;
                }
            }
            (0x90, 0x08) => self.graph.global_enabled = a[0],
            (0x90, 0x09) => {
                target_checks::priority(a[0])?;
                self.graph.default_priority = a[0];
            }
            (0x90, 0x43) => self.configure_native_backf(call)?,
            (0x90, 0x44) => self.configure_native_frame_background(vm, call)?,
            (0x90, 0x40 | 0x41 | 0x42 | 0x45 | 0x46 | 0x47 | 0x48 | 0x49 | 0x4a) => {
                self.configure_native_backdrop(call)?
            }
            (0x90, 0x4c) => {
                self.graph.renderer_options = [a[0], a[1]];
                for id in self.graph.descendants(0) {
                    self.graph
                        .objects
                        .entry(id)
                        .or_default()
                        .native
                        .draw_enabled = u32::from(a[0] != 0);
                }
                if let Some(background) =
                    self.graph.objects.entry(0).or_default().background.as_mut()
                {
                    background.active = a[1];
                }
            }
            (0x90, 0x4d) => output.push(
                self.graph
                    .objects
                    .get(&0)
                    .and_then(|p| p.background)
                    .map(|b| b.class as i32)
                    .unwrap_or(0),
            ),
            (0x90, 0x58 | 0x5a) => self.configure_native_sprite_pair(call)?,
            (0x90, 0x5b) => self.configure_native_sprite_wave(call)?,
            (0x90, 0x5c | 0x5d) => self.configure_native_projection(call)?,
            (0x90, 0x50) => {
                let id = self.graph.allocate(0x80000000, 512, Kind::Sprite);
                ensure!(id != 0, "BGI sprite pool exhausted");
                if id != 0 {
                    self.sprites.insert(
                        id as u32,
                        Sprite {
                            opacity: 255,
                            visible: false,
                            ..Default::default()
                        },
                    );
                }
                output.push(id);
            }
            (0x90, 0xe0) => {
                let id = self.graph.allocate(0xf1000000, 8, Kind::Group);
                ensure!(id != 0, "BGI group pool exhausted");
                output.push(id);
            }
            (0x90, 0x51 | 0xe1) => {
                let id = a[0];
                self.graph.require(id)?;
                let expected = if call.id == 0x51 {
                    Kind::Sprite
                } else {
                    Kind::Group
                };
                ensure!(
                    self.graph.tree.kind(id) == Some(expected),
                    "BGI object class mismatch"
                );
                if call.id == 0x51 {
                    ensure!(
                        self.graph.tree.parent(id).is_none(),
                        "BGI object still has an owner"
                    );
                }
                if let Some(parent) = self.graph.tree.parent(id) {
                    if let Some(members) = self.graph.members.get_mut(&parent) {
                        members.remove(&id);
                    }
                }
                if let Some(children) = self.graph.members.remove(&id) {
                    for child in children.keys() {
                        self.graph
                            .tree
                            .set_parent(*child, 0)
                            .map_err(|s| anyhow::anyhow!("BGI detach status {s}"))?;
                    }
                }
                self.hide_graph_object(host, id)?;
                self.forget_sprite_mask_links(id);
                self.remove_sprite_composition(host, id)?;
                self.graph.sprite_affine.remove(&id);
                self.graph.projections.remove(&id);
                self.graph.fixed_vectors.remove(&id);
                self.graph.raster_offsets.remove(&id);
                self.sprites.remove(&(id as u32));
                self.graph.frames.remove(&id);
                self.graph.sprite_wave_args.remove(&id);
                self.graph.images.remove(id);
                self.graph.tree.remove(id);
                self.graph.objects.remove(&id);
                self.graph.transforms.remove(&id);
            }
            (0x90, 0x56 | 0x59) => {
                let id = a[0];
                self.graph.require(id)?;
                ensure!(
                    self.graph.tree.kind(id) == Some(Kind::Sprite),
                    "BGI sprite class"
                );
                let (bitmap, info) = self.bitmap(a[3])?;
                let size = info.size;
                let (mode, blend, alpha, priority) = if call.id == 0x56 {
                    (0, a[4], a[5], a[6])
                } else {
                    (2, a[10], a[11], a[12])
                };
                target_checks::effect_mode(blend)?;
                target_checks::effect_level(alpha)?;
                target_checks::priority(priority)?;
                let transform = if mode == 2 {
                    let w = i64::from(size[0]) * i64::from(a[7]) >> 16;
                    let h = i64::from(size[1]) * i64::from(a[8]) >> 16;
                    ensure!(w >= 2 && h >= 2, "BGI sprite scale extent");
                    SpriteTransform {
                        pivot_x: a[4] as f32,
                        pivot_y: a[5] as f32,
                        rotate: (a[6] as f32 / 65536.).to_radians(),
                        scale_x: a[7] as f32 / 65536.,
                        scale_y: a[8] as f32 / 65536.,
                        ..Default::default()
                    }
                } else {
                    Default::default()
                };
                self.graph.images.remove(id);
                self.graph.projections.remove(&id);
                self.graph.frames.remove(&id);
                self.graph.sprite_wave_args.remove(&id);
                if mode == 2 {
                    self.graph
                        .sprite_affine
                        .insert(id, [a[4], a[5], a[6], a[7], a[8], a[9]]);
                } else {
                    self.graph.sprite_affine.remove(&id);
                }
                self.graph.transforms.insert(
                    id,
                    if mode == 2 {
                        Default::default()
                    } else {
                        transform
                    },
                );
                self.graph.raster_offsets.remove(&id);
                self.graph.position(id, a[1], a[2]);
                let p = self.graph.objects.entry(id).or_default();
                p.format_resource = Some(bitmap as i32);
                p.blend_mode = blend;
                p.native.blend_mode = blend;
                p.set_alpha_parameter(alpha);
                p.native.priority = priority as u32;
                p.named_properties.insert("target-object-mode".into(), mode);
                let sprite = self
                    .sprites
                    .get_mut(&(id as u32))
                    .context("BGI sprite slot")?;
                sprite.bitmap = Some(bitmap);
                self.graph.sort(id);
                if mode == 2 {
                    self.rebuild_native_sprite_affine(id)?;
                }
            }
            (0x90, 0x57) => {
                self.graph.require(a[0])?;
                target_checks::bitmap(a[1])?;
                self.replace_native_sprite_source(a[0], a[1])?;
            }
            (0x90, 0x53) => {
                self.graph.require(a[0])?;
                ensure!(
                    self.graph.tree.kind(a[0]) == Some(Kind::Sprite),
                    "BGI invalid sprite handle"
                );
                let mode = self
                    .graph
                    .objects
                    .get(&a[0])
                    .and_then(|p| p.named_properties.get("target-object-mode"))
                    .copied()
                    .unwrap_or(0);
                if matches!(mode, 0 | 1 | 3) {
                    ensure!(a[3] > 0 && a[4] > 0, "BGI invalid sprite update rectangle");
                }
                if matches!(mode, 5 | 6) {
                    self.graph
                        .projections
                        .get_mut(&a[0])
                        .context("BGI projected cache")?
                        .dirty_rect = Some([a[1], a[2], a[3], a[4]]);
                }
                self.graph.images.dirty(a[0]);
            }
            (0x90, 0x30 | 0x54 | 0xe4) => {
                self.graph.require(a[0])?;
                if call.id == 0x54 {
                    ensure!(
                        self.graph.tree.kind(a[0]) == Some(Kind::Sprite),
                        "BGI invalid sprite handle"
                    );
                }
                if call.id == 0xe4 {
                    ensure!(
                        self.graph.tree.kind(a[0]) == Some(Kind::Group),
                        "BGI invalid group handle"
                    );
                }
                for id in self.graph.descendants(a[0]) {
                    self.graph
                        .objects
                        .entry(id)
                        .or_default()
                        .native
                        .draw_enabled = u32::from(a[1] != 0);
                }
            }
            (0x90, 0x31) => {
                self.graph.require(a[0])?;
                self.graph.tree.set_enabled(a[0], a[1] != 0);
                self.graph.objects.entry(a[0]).or_default().native.enabled = u32::from(a[1] != 0);
            }
            (0x90, 0x33) => {
                self.graph.require(a[0])?;
                if let Some(background) = self
                    .graph
                    .objects
                    .get_mut(&a[0])
                    .and_then(|p| p.background.as_mut())
                {
                    background.set_position(
                        a[1],
                        a[2],
                        self.graph.screen[0] as i32,
                        self.graph.screen[1] as i32,
                    );
                    self.rebuild_backdrop_plan(a[0])?;
                } else {
                    self.graph.position(a[0], a[1], a[2]);
                }
            }
            (0x90, 0x34 | 0x35 | 0x39) => {
                self.graph.require(a[0])?;
                target_checks::effect_level(a[1])?;
                for id in self.graph.descendants(a[0]) {
                    let p = self.graph.objects.entry(id).or_default();
                    match call.id {
                        0x34 => {
                            p.mask_alpha = a[1];
                            p.native.mask_alpha = a[1];
                        }
                        0x35 => {
                            p.native.fixed_parameter_16_16 = a[1].wrapping_shl(16);
                            p.properties
                                .insert(0x35, (p.native.fixed_parameter_16_16, 0));
                        }
                        _ => {
                            p.alpha_multiplier = a[1];
                            p.native.alpha_multiplier = a[1];
                        }
                    }
                }
                if call.id == 0x35 {
                    for id in self.graph.descendants(a[0]) {
                        self.sync_native_effector(id)?;
                        self.sync_native_projection_fixed(id, a[1].wrapping_shl(16))?;
                        if let Some(plan) = self.graph.images.plans.get_mut(&id) {
                            if let Some(Raster::Display {
                                spec: DisplayRaster::ThresholdMask { threshold, .. },
                            }) = &mut plan.operation
                            {
                                *threshold = a[1] as u32;
                            }
                        }
                        self.graph.images.dirty(id);
                    }
                }
            }
            (0x90, 0x36 | 0x37) => {
                self.graph.require(a[0])?;
                for id in self.graph.descendants(a[0]) {
                    let n = &mut self.graph.objects.entry(id).or_default().native;
                    if call.id == 0x36 {
                        n.primary_offset_x = a[1];
                        n.primary_offset_y = a[2];
                    } else {
                        n.secondary_offset_x = a[1];
                        n.secondary_offset_y = a[2];
                    }
                }
            }
            (0x90, 0x3a) => {
                self.graph.require(a[0])?;
                target_checks::priority(a[1])?;
                self.graph.objects.entry(a[0]).or_default().native.priority = a[1] as u32;
                self.graph.sort(a[0]);
            }
            (0x90, 0x38) => {
                self.graph.require(a[0])?;
                match a[1] as u32 {
                    0 => self.graph.position(a[0], a[2], a[3]),
                    1 => {
                        target_checks::effect_mode(a[2])?;
                        let p = self.graph.objects.entry(a[0]).or_default();
                        p.blend_mode = a[2];
                        p.native.blend_mode = a[2];
                    }
                    2 => self.graph_alpha(a[0], a[2])?,
                    0xc0 => {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .native
                            .property_c0_value = a[2] as u32
                    }
                    0xc1 => {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .native
                            .property_c1_value = a[2] as u32
                    }
                    0x8001 => {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .native
                            .property_8001_value = a[2] as u32
                    }
                    0x7fff0000 => {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .native
                            .auto_update = a[2] as u32
                    }
                    0x7fffffff => {
                        let slot = usize::try_from(a[2])?;
                        ensure!(slot < 16, "BGI native user slot");
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .native
                            .user_slots[slot] = a[3];
                    }
                    196 | 0x8000 | 0x8100 => self
                        .graph
                        .objects
                        .entry(a[0])
                        .or_default()
                        .set_property(a[1] as u32, a[2], a[3]),
                    0x80 | 0x101 | 0x102
                        if self
                            .graph
                            .objects
                            .get(&a[0])
                            .is_some_and(|p| p.background.is_some()) =>
                    {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .properties
                            .insert(a[1] as u32, (a[2], a[3]));
                        if a[1] == 0x101 {
                            self.graph
                                .objects
                                .entry(a[0])
                                .or_default()
                                .properties
                                .remove(&0x80);
                        }
                        self.rebuild_backdrop_plan(a[0])?;
                    }
                    property @ (0x10 | 0x40 | 0x41 | 0x42 | 0x43 | 0x60 | 0x80 | 0x81 | 0x82
                    | 0x83 | 0x8f)
                        if self.graph.tree.kind(a[0]) == Some(Kind::Sprite) =>
                    {
                        self.native_sprite_property(a[0], property, a[2], a[3])?
                    }
                    0x40000000
                        if self
                            .graph
                            .objects
                            .get(&a[0])
                            .and_then(|p| p.background)
                            .is_some_and(|b| b.class == BackClass::BackF) =>
                    {
                        let backf = self
                            .graph
                            .objects
                            .get_mut(&a[0])
                            .unwrap()
                            .background
                            .as_mut()
                            .unwrap()
                            .backf
                            .as_mut()
                            .unwrap();
                        ensure!(
                            backf.mask_control_enabled == 0 || (a[3] as u32) < 4,
                            "BGI invalid BackF mask control mode (254)"
                        );
                        backf.mask_control_enabled = a[2];
                        backf.mask_control_mode = a[3];
                        self.rebuild_native_backf(a[0])?;
                    }
                    0x100 => {
                        if self
                            .graph
                            .filters
                            .get(&a[0])
                            .is_some_and(|f| f.effector.is_some())
                        {
                            self.graph_effector_property(a[0], 0x100, a[2], a[3])?;
                        } else {
                            self.graph_wave_window(a[0], a[2], a[3])?;
                        }
                    }
                    property @ (0x80 | 0x81 | 0x82 | 0x8f | 0xff)
                        if self
                            .graph
                            .filters
                            .get(&a[0])
                            .is_some_and(|f| f.effector.is_some()) =>
                    {
                        self.graph_effector_property(a[0], property, a[2], a[3])?
                    }
                    0xff if self.graph.objects.get(&a[0]).is_some_and(|p| {
                        p.background.is_some_and(|b| {
                            matches!(b.class, BackClass::BackRpl | BackClass::BackDst)
                        })
                    }) =>
                    {
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .properties
                            .insert(255, (a[2], a[3]));
                        self.rebuild_backdrop_plan(a[0])?;
                    }
                    0x11 => {
                        ensure!(
                            self.graph.tree.kind(a[0]) == Some(Kind::Sprite),
                            "BGI color-filter property requires a sprite"
                        );
                        let index = a[2] as u32 & 255;
                        let kind = (a[2] as u32 >> 8) & 255;
                        ensure!(index < 16 && kind <= 5, "BGI color-filter index/kind");
                        self.graph
                            .objects
                            .entry(a[0])
                            .or_default()
                            .properties
                            .insert(0x1100 | index, (a[2], a[3]));
                        let primary = self.graph.objects[&a[0]]
                            .format_resource
                            .context("BGI unconfigured sprite color filter")?;
                        let mut plan =
                            self.graph.images.plans.get(&a[0]).cloned().unwrap_or(
                                GraphPlan::bitmap(primary, self.bitmap(primary)?.1.format),
                            );
                        plan.filters.clear();
                        self.set_graph_plan(a[0], plan);
                    }
                    _ => bail!("BGI unsupported native object property {:#x}", a[1]),
                }
                self.graph.sort(a[0]);
            }
            (0x90, 0x3f) => {
                self.graph.require(a[0])?;
                bail!(
                    "指定されたディスプレイオブジェクトはイメージキャッシュをサポートしていません"
                );
            }
            (0x91, 0x06) => self.graph.display_offset = [a[0], a[1]],

            (0x91, 0x31) => {
                self.graph.require(a[0])?;
                self.graph
                    .objects
                    .entry(a[0])
                    .or_default()
                    .native
                    .suppress_draw = u32::from(a[1] != 0);
            }
            (0x91, 0x38 | 0x3d) => {
                self.graph.require(a[1])?;
                let n = self
                    .graph
                    .objects
                    .get(&a[1])
                    .map(|p| p.native)
                    .unwrap_or_default();
                let values = if call.id == 0x3d {
                    let offset = if n.global_display_offset_enabled != 0 {
                        self.graph.display_offset
                    } else {
                        [0; 2]
                    };
                    vec![
                        n.position_x
                            .wrapping_add(n.primary_offset_x)
                            .wrapping_add(n.secondary_offset_x)
                            .wrapping_add(offset[0]),
                        n.position_y
                            .wrapping_add(n.primary_offset_y)
                            .wrapping_add(n.secondary_offset_y)
                            .wrapping_add(offset[1]),
                    ]
                } else {
                    match a[2] as u32 {
                        0 => vec![n.position_x, n.position_y],
                        1 => vec![n.blend_mode],
                        2 => vec![
                            self.graph
                                .objects
                                .get(&a[1])
                                .map_or(n.alpha_parameter, |p| p.alpha_virtual()),
                        ],
                        3 => vec![n.priority as i32],
                        32 => vec![
                            n.fixed_position_x_16_16,
                            n.fixed_position_y_16_16,
                            n.fixed_position_z_16_16,
                        ],
                        0x7fffffff => {
                            let bytes = vm.memory(a[0] as u32, 4)?;
                            let input = i32::from_le_bytes(bytes.try_into().unwrap());
                            let slot = usize::try_from(input)?;
                            vec![*n.user_slots.get(slot).context("BGI native user slot")?]
                        }
                        0x41 => {
                            let p = self.graph.objects.get(&a[1]).context("BGI native sprite")?;
                            let mode = p
                                .named_properties
                                .get("target-object-mode")
                                .copied()
                                .unwrap_or(0);
                            ensure!(
                                matches!(mode, 2 | 5 | 6),
                                "BGI object rotation property is unsupported by this class"
                            );
                            vec![p.properties.get(&0x41).map(|p| p.0).unwrap_or_else(|| {
                                if let Some(state) = self.graph.projections.get(&a[1]) {
                                    return state.args[if mode == 6 { 12 } else { 10 }];
                                }
                                self.graph
                                    .transforms
                                    .get(&a[1])
                                    .map(|t| (t.rotate.to_degrees() * 65536.).round() as i32)
                                    .unwrap_or(0)
                            })]
                        }
                        0x10000000 => {
                            let p = self
                                .graph
                                .objects
                                .get(&a[1])
                                .context("BGI projected transform")?;
                            ensure!(
                                self.graph.projections.contains_key(&a[1]),
                                "BGI object does not support projected transform query"
                            );
                            (0..5)
                                .map(|i| {
                                    p.named_properties
                                        .get(&format!("projected-transform-{i}"))
                                        .copied()
                                        .context("BGI projected transform state")
                                })
                                .collect::<Result<Vec<_>>>()?
                        }
                        0x10000100 => {
                            ensure!(
                                self.graph.tree.kind(a[1]) == Some(Kind::Sprite),
                                "BGI bitmap extent property requires a sprite"
                            );
                            let p = self
                                .graph
                                .objects
                                .get(&a[1])
                                .context("BGI sprite properties")?;
                            let size = p
                                .format_resource
                                .and_then(|id| self.bitmaps.get(&(id as u32)))
                                .map(|b| b.size)
                                .unwrap_or([0; 2]);
                            let mode = p
                                .named_properties
                                .get("target-object-mode")
                                .copied()
                                .unwrap_or(0);
                            let extent = if matches!(mode, 5 | 6) {
                                [
                                    *p.named_properties
                                        .get(&format!("mode{mode}-projected-width"))
                                        .context("BGI projected width")?,
                                    *p.named_properties
                                        .get(&format!("mode{mode}-projected-height"))
                                        .context("BGI projected height")?,
                                ]
                            } else {
                                [size[0] as i32, size[1] as i32]
                            };
                            vec![size[0] as i32, size[1] as i32, extent[0], extent[1]]
                        }
                        _ => bail!("BGI unsupported native object query {:#x}", a[2]),
                    }
                };
                writes.push(crysirror_bgi_vm::HostMemoryWrite {
                    address: a[0] as u32,
                    bytes: values.into_iter().flat_map(i32::to_le_bytes).collect(),
                });
            }
            (0x91, 0x36 | 0x37) => {
                self.graph.require(a[0])?;
                self.graph.fixed_vectors.entry(a[0]).or_insert([[0; 3]; 2])
                    [if call.id == 0x37 { 0 } else { 1 }] = [a[1], a[2], a[3]];
                self.rebuild_native_projection(a[0])?;
                self.graph.sort(a[0]);
            }
            (0x91, 0x33) => {
                self.graph.require(a[0])?;
                let n = &mut self.graph.objects.entry(a[0]).or_default().native;
                let round = |value: i32| {
                    if n.fixed_position_rounding_enabled != 0
                        && (a[3] == 0 || n.fixed_position_rounding_mode == 1)
                    {
                        value.wrapping_add(0x8000) & !0xffff
                    } else {
                        value
                    }
                };
                let x = round(a[1]);
                let y = round(a[2]);
                n.fixed_position_x_16_16 = x;
                n.fixed_position_y_16_16 = y;
                n.fixed_position_z_16_16 = a[3];
                if n.fixed_position_updates_integer_position != 0 {
                    self.graph.position(a[0], x >> 16, y >> 16);
                }
                self.rebuild_native_projection(a[0])?;
                self.graph.sort(a[0]);
            }
            (0x90, 0xe5) => {
                self.graph.require(a[0])?;
                ensure!(
                    self.graph.tree.kind(a[0]) == Some(Kind::Group),
                    "BGI group class"
                );
                target_checks::effect_level(a[3])?;
                self.graph.position(a[0], a[1], a[2]);
                self.graph_alpha(a[0], a[3])?;
            }
            (0x91, 0x3e) | (0x90, 0xe8) => {
                let (master, child, x, y) = (a[0], a[1], a[2], a[3]);
                self.graph.require(master)?;
                self.graph.require(child)?;
                if call.group == 0x90 {
                    ensure!(
                        self.graph.tree.kind(master) == Some(Kind::Group),
                        "BGI invalid group handle"
                    );
                }
                ensure!(master != child, "BGI cannot attach object to itself");
                ensure!(
                    self.graph.tree.parent(child).is_none(),
                    "BGI member already has an owner"
                );
                self.graph
                    .tree
                    .set_parent(child, master)
                    .map_err(|s| anyhow::anyhow!("BGI member relation status {s}"))?;
                self.graph
                    .members
                    .entry(master)
                    .or_default()
                    .insert(child, [x, y]);
                let p = &self.graph.objects.entry(master).or_default().native;
                let position = [p.position_x.wrapping_add(x), p.position_y.wrapping_add(y)];
                self.graph.position(child, position[0], position[1]);
            }
            (0x91, 0x3f) | (0x90, 0xe9) => {
                let (master, child) = (a[0], a[1]);
                self.graph.require(master)?;
                self.graph.require(child)?;
                if call.group == 0x90 {
                    ensure!(
                        self.graph.tree.kind(master) == Some(Kind::Group),
                        "BGI invalid group handle"
                    );
                }
                ensure!(
                    self.graph.tree.parent(child) == Some(master),
                    "BGI object is not a member of this owner"
                );
                self.graph
                    .tree
                    .set_parent(child, 0)
                    .map_err(|s| anyhow::anyhow!("BGI member detach status {s}"))?;
                if let Some(members) = self.graph.members.get_mut(&master) {
                    members.remove(&child);
                }
            }
            _ => bail!(
                "BGI graph selector {:02x}:{:02x} requires its native subclass mapping",
                call.group,
                call.id
            ),
        }
        self.refresh_native_sprite_masks()?;
        ensure!(output.len() == call.outputs, "BGI graph result count");
        let objects = self
            .graph
            .images
            .dirty
            .iter()
            .copied()
            .filter(|object| {
                self.graph
                    .images
                    .revisions
                    .get(object)
                    .is_some_and(|revision| *revision > before)
            })
            .collect::<Vec<_>>();
        for &object in &objects {
            self.refresh_graph_image(host, object)?;
        }
        if objects
            .iter()
            .any(|object| self.graph.images.dirty.contains(object))
        {
            self.graph
                .pending
                .insert(request, (output, writes, objects));
            return Ok(false);
        }
        self.publish_graph(host)?;
        vm.commit_call_writes(&output, writes)?;
        self.calls += 1;
        Ok(true)
    }
}
