// SPDX-License-Identifier: MPL-2.0
//! RLD v1 image select items. Binding copies the idle cell into the parent
//! bitmap's colour plane, preserving its independent alpha. A button atlas
//! contains three horizontal cells. The v1 selection wait consumes primary
//! pointer activations for the attached, unmasked image-item profile.
use super::{branch::number, panel_animation::Kind, surfaces::Surface};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    image_processing::{
        ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState,
    },
    path::{Mount, VfsPath},
    service::{AssetReader, HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Button {
    enable_count: i32,
    group: u8,
    parent: u8,
    id: u8,
    pos: [i32; 2],
    extent: [u32; 2],
    atlas: VfsPath,
}
impl Button {
    pub fn parse(raw: &[u8]) -> Result<Self> {
        let f: Vec<_> = raw.splitn(25, |b| *b == b',').collect();
        ensure!(f.len() == 25, "short ExHibit select item record");
        let n = |i: usize| number(f[i]);
        let (group, parent) = (n(0)?, n(2)?);
        ensure!(
            (0..8).contains(&group) && (0..127).contains(&parent) && n(1)? == 1,
            "unsupported ExHibit select item group, parent, or kind"
        );
        ensure!(
            n(7)? == -2 && n(9)? == 0 && n(10)? == -1 && n(11)? == -1,
            "unsupported ExHibit select item key, flags, sound or tooltip"
        );
        let pos = [n(4)?, n(5)?];
        ensure!(
            pos.iter().all(|v| (-32768..=32767).contains(v)),
            "unsupported ExHibit select item coordinates"
        );
        Ok(Self {
            enable_count: 0,
            group: group as u8,
            parent: parent as u8,
            id: u8::try_from(n(3)?)?,
            pos,
            extent: [0; 2], // Published together with the composed button.
            atlas: VfsPath::new(Mount::Asset, &std::str::from_utf8(f[6])?.replace('\\', "/"))?,
        })
    }
}

pub(super) struct Selection {
    group: u8,
    pub destination: i32,
    pressed: Option<(u8, u8)>,
}
impl Selection {
    pub fn new(group: u8, destination: i32) -> Self {
        Self { group, destination, pressed: None }
    }

    /// The native normal-item result is the low byte of the layer/item
    /// result (cmdExecSelectItem, RVA 0x66570). A host primary activation
    /// captures an item and completes only when the pointer releases on it.
    /// Do not translate other native event classes into ordinary item IDs.
    pub fn poll(
        &mut self,
        panels: &Panels,
        input: InputFrame,
        pointer: Option<[i32; 2]>,
    ) -> Option<u8> {
        let hit = pointer.and_then(|p| panels.hit(self.group, p));
        let down = input.buttons_down & buttons::MOUSE_LEFT != 0;
        if input.buttons_pressed & buttons::MOUSE_LEFT != 0 {
            self.pressed = hit;
        } else if self.pressed != hit {
            self.pressed = None;
        }
        if down {
            None
        } else {
            self.pressed.take().map(|(_, id)| id)
        }
    }
}
fn button_cell(
    pos: [i32; 2],
    atlas: ImageInput,
    [w, h]: [u32; 2],
    enabled: bool,
) -> Result<ImageCompositeLayer> {
    ensure!(w >= 3 && h > 0, "empty ExHibit select item");
    Ok(ImageCompositeLayer {
        input: Box::new(atlas),
        source: [
            if enabled { 0 } else { 2 * (w / 3) as i32 },
            0,
            (w / 3) as i32,
            h as i32,
        ],
        destination: pos,
        clip: None,
        opacity: 255,
        math: ImageCompositeMath::CopyRgb,
    })
}
pub(super) fn palette(reader: &dyn AssetReader) -> Result<([u32; 2], [u8; 1024])> {
    let mut header = [0; 36];
    crysirror_resource::read_exact_at(reader, 0, &mut header)?;
    let word = |o: usize| u32::from_le_bytes(header[o..o + 4].try_into().expect("header field"));
    ensure!(
        header.starts_with(b"GYU\x1a") && word(12) == 8 && word(32) == 256,
        "unsupported ExHibit select item bitmap palette"
    );
    let size = [word(16), word(20)];
    ensure!(
        size.iter().all(|n| (1..=8192).contains(n)),
        "invalid ExHibit select bitmap dimensions"
    );
    let mut palette = [0; 1024];
    crysirror_resource::read_exact_at(reader, 36, &mut palette)?;
    Ok((size, palette))
}

struct Layer {
    surface: Surface,
    size: [u32; 2],
    published: Option<String>,
    items: BTreeMap<u8, Button>,
}
enum Pending {
    Preparing(Task<(ImageJob, [u32; 2])>),
    Composing(ImageJob),
}
#[derive(Default)]
pub(super) struct Panels {
    layers: BTreeMap<u8, Layer>,
    character_layers: Vec<Option<u8>>,
    input_blocks: BTreeMap<u8, i32>,
    pending: Option<(Button, Pending)>,
}
impl Panels {
    fn hit(&self, group: u8, point: [i32; 2]) -> Option<(u8, u8)> {
        use crysirror_core::region_map::contains_rect;
        let point = point.map(i64::from);
        for (&slot, layer) in self.layers.iter().rev() {
            if (61..=71).contains(&slot)
                || layer.surface.level == 0
                || self.input_blocks.get(&slot).copied().unwrap_or(0) != 0
            {
                continue;
            }
            let [x, y] = layer.surface.origin.map(i64::from);
            let [w, h] = layer.size.map(i64::from);
            if !contains_rect([x, y, w, h], point) {
                continue;
            }
            for (&id, button) in layer.items.iter().rev() {
                let [dx, dy] = button.pos.map(i64::from);
                let [bw, bh] = button.extent.map(i64::from);
                if id != 0 && button.group == group && button.enable_count >= 0
                    && contains_rect([x + dx, y + dy, bw, bh], point)
                {
                    return Some((slot, id));
                }
            }
        }
        None
    }
    pub fn for_characters(count: usize) -> Result<Self> {
        ensure!(count <= 1_000_000, "ExHibit character layer count");
        // RetouchAdvCharacter's constructor initializes the layer field to -1.
        // enterEngine changes it when a character is actually put on a layer.
        Ok(Self {
            character_layers: vec![None; count],
            ..Self::default()
        })
    }

    pub fn resolve_slot(&self, layer: i32) -> Option<u8> {
        if layer >= 1000 {
            self.character_layers
                .get((layer - 1000) as usize)
                .copied()
                .flatten()
        } else {
            u8::try_from(layer).ok().filter(|&slot| slot < 127)
        }
    }
    /// cmdInputLayerControl's single-layer range or its complement. These
    /// counters belong to all 127 native slots even before a bitmap is bound.
    pub fn control_input(&mut self, selected: i32, enable: bool, complement: bool) {
        for slot in 0..127_u8 {
            if (i32::from(slot) == selected) != complement {
                let count = self.input_blocks.entry(slot).or_default();
                if enable {
                    if *count > 0 {
                        *count -= 1;
                    }
                } else {
                    *count = count.wrapping_add(1);
                }
            }
        }
    }
    pub fn input_blocked(&self, slot: u8) -> bool {
        self.input_blocks.get(&slot).copied().unwrap_or(0) != 0
    }
    pub fn scene_empty(&self) -> bool {
        self.layers.is_empty() && self.pending.is_none()
            && self.character_layers.iter().all(Option::is_none)
    }
    pub fn is_bound(&self, slot: u8) -> bool {
        self.layers.contains_key(&slot)
    }
    pub fn exit_group(&mut self, group: u8) {
        // cmdExitSelectItem destroys the group's objects. Image binding copies
        // into the parent bitmap, which lives until the later billboard removal.
        for layer in self.layers.values_mut() {
            layer.items.retain(|_, button| button.group != group);
        }
    }
    pub fn remove(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        slot: u8,
    ) -> Result<()> {
        let Some(layer) = self.layers.get(&slot) else {
            return Ok(());
        };
        ensure!(layer.items.is_empty(), "unsupported ExHibit removal with attached controls");
        let id = 256 + u32::from(slot);
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&serde_json::json!({"layers":[{"op":"remove","id":id}]}))?,
        )?;
        let layer = self.layers.remove(&slot).expect("removed layer");
        if let Some(path) = layer.published {
            h.release_image_resource(c, &path)?;
        }
        Ok(())
    }
    pub fn values(&self, slot: u8, kind: Kind) -> Result<[i32; 2]> {
        let surface = &self
            .layers
            .get(&slot)
            .context("unavailable ExHibit animation surface")?
            .surface;
        Ok(match kind {
            Kind::Fade => [i32::from(surface.level), 0],
            Kind::Move => surface.origin,
        })
    }
    pub fn set_values(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        slot: u8,
        kind: Kind,
        value: [i32; 2],
    ) -> Result<()> {
        if self.values(slot, kind)? == value {
            return Ok(());
        }
        let layer = self
            .layers
            .get_mut(&slot)
            .context("missing ExHibit animation surface")?;
        let id = 256 + u32::from(slot);
        let op = match kind {
            Kind::Fade => {
                serde_json::json!({"op":"set_opacity","id":id,"opacity":u8::try_from(value[0])?.saturating_mul(16)})
            }
            Kind::Move => {
                serde_json::json!({"op":"set_rect","id":id,"rect":[value[0],value[1],layer.size[0],layer.size[1]]})
            }
        };
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&serde_json::json!({"layers":[op]}))?,
        )?;
        match kind {
            Kind::Fade => layer.surface.level = value[0] as u8,
            Kind::Move => layer.surface.origin = value,
        }
        Ok(())
    }

    pub fn disable(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        pool: &TaskPool,
        layer: i32,
        item: i32,
    ) -> Result<()> {
        let slot = self.resolve_slot(layer);
        let Some(button) = slot
            .and_then(|slot| self.layers.get_mut(&slot))
            .and_then(|l| u8::try_from(item).ok().and_then(|id| l.items.get_mut(&id)))
        else {
            return Ok(());
        };
        let count = button.enable_count.wrapping_sub(1);
        if count != -1 {
            button.enable_count = count;
            return Ok(());
        }
        // Only the 0 -> -1 crossing redraws. Commit that count with its pixels.
        let mut next = button.clone();
        next.enable_count = count;
        self.prepare_cell(h, c, pool, next)
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn define(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        surface: Surface,
    ) -> Result<()> {
        ensure!(
            self.layers
                .get(&surface.slot)
                .is_none_or(|p| p.items.is_empty()),
            "unsupported ExHibit rebind of attached controls"
        );
        let size = h.image_size(c, &surface.asset)?;
        surface.put(h, c, size, &surface.asset)?;
        self.layers.insert(
            surface.slot,
            Layer {
                surface,
                size,
                published: None,
                items: BTreeMap::new(),
            },
        );
        Ok(())
    }
    pub fn bind(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        pool: &TaskPool,
        button: Button,
    ) -> Result<()> {
        let layer = self
            .layers
            .get(&button.parent)
            .context("unavailable ExHibit select item parent")?;
        ensure!(
            !layer.items.contains_key(&button.id),
            "unsupported ExHibit select item replacement"
        );
        self.prepare_cell(h, c, pool, button)
    }
    fn prepare_cell(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        pool: &TaskPool,
        button: Button,
    ) -> Result<()> {
        let layer = self
            .layers
            .get(&button.parent)
            .context("missing ExHibit select item parent")?;
        let orig = VfsPath::new(Mount::Asset, &layer.surface.asset)?;
        let base = if let Some(path) = &layer.published {
            ImageInput::Resource(
                h.published_image_resource(c, path)?
                    .context("missing ExHibit parent image")?,
            )
        } else {
            ImageInput::Asset {
                path: layer.surface.asset.clone(),
                size: layer.size,
                fingerprint: None,
            }
        };
        let meta = h.image_metadata(c)?;
        let storage = h.storage(c)?;
        let atlas = button.atlas.clone();
        let pos = button.pos;
        let enabled = button.enable_count >= 0;
        let task = pool.spawn(move |cancel| {
            cancel.check()?;
            let parent_reader = storage.open_read(&orig)?;
            let atlas_reader = storage.open_read(&atlas)?;
            let (_, parent_palette) = palette(parent_reader.as_ref())?;
            let (size, atlas_palette) = palette(atlas_reader.as_ref())?;
            // The native attachment copies indices, never RGB. Expand those
            // indices through the parent's palette in the shared GYU decoder.
            let source = if parent_palette == atlas_palette {
                ImageInput::ScopedAsset {
                    path: atlas,
                    size,
                    fingerprint: None,
                }
            } else {
                let mut encoded = crysirror_resource::read_all(atlas_reader.as_ref(), 64 << 20)?;
                encoded
                    .get_mut(36..1060)
                    .context("truncated select item palette")?
                    .copy_from_slice(&parent_palette);
                cancel.check()?;
                ImageInput::Encoded {
                    name: atlas.relative().to_owned(),
                    bytes: encoded.into(),
                    size,
                }
            };
            let cell = button_cell(pos, source, size, enabled)?;
            Ok((
                meta.begin_process(base.with_composite_layer(cell), Arc::from(&b"[]"[..]))?,
                [size[0] / 3, size[1]],
            ))
        })?;
        self.pending = Some((button, Pending::Preparing(task)));
        Ok(())
    }
    pub fn poll(&mut self, h: &mut dyn HostServices, c: ClientId) -> Result<bool> {
        let Some((button, pending)) = &mut self.pending else {
            return Ok(true);
        };
        if let Pending::Preparing(task) = pending {
            let Some((job, extent)) = task.try_take()? else {
                return Ok(false);
            };
            button.extent = extent;
            *pending = Pending::Composing(job);
        }
        let Pending::Composing(job) = pending else {
            unreachable!()
        };
        let image = match job.poll()? {
            ImageProcessState::Pending => return Ok(false),
            ImageProcessState::Failed(e) => bail!("ExHibit select bitmap: {e}"),
            ImageProcessState::Ready(image) => image,
        };
        let parent = self
            .layers
            .get_mut(&button.parent)
            .context("lost ExHibit select item parent")?;
        let upload = h.publish_image_resource(c, 256 + u32::from(button.parent), image)?;
        let path = upload.path.relative();
        if let Err(e) = parent.surface.put(h, c, parent.size, path) {
            h.release_image_resource(c, path)?;
            return Err(e);
        }
        if let Some(old) = parent.published.replace(path.to_owned()) {
            h.release_image_resource(c, &old)?;
        }
        let (button, _) = self.pending.take().expect("completed button job");
        parent.items.insert(button.id, button);
        Ok(true)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn layer_input_control_nests_across_slots_and_preserves_the_selected_parent() {
        let mut panels = Panels::default();
        panels.control_input(42, false, true);
        panels.control_input(42, false, true);
        assert_eq!(panels.input_blocks.len(), 126);
        assert_eq!(panels.input_blocks.get(&42), None);
        assert_eq!(panels.input_blocks[&126], 2);
        panels.control_input(42, true, true);
        assert_eq!(panels.input_blocks[&0], 1);
        panels.control_input(42, true, false);
        assert_eq!(panels.input_blocks[&42], 0);
    }
    pub(crate) fn bitmap(indices: &[u8], alpha: &[u8]) -> Vec<u8> {
        let stride = indices.len().next_multiple_of(4);
        let mut b = b"GYU\x1a\x03\x00\x00\x01".to_vec();
        for word in [
            u32::MAX,
            8,
            indices.len() as u32,
            1,
            stride as u32,
            if alpha.is_empty() { 0 } else { stride as u32 },
            256,
        ] {
            b.extend(word.to_le_bytes());
        }
        for i in 0..=255 {
            b.extend([1, 2, i, 0]);
        }
        b.extend(indices);
        b.resize(b.len() + stride - indices.len(), 0);
        if !alpha.is_empty() {
            b.extend(alpha);
            b.resize(b.len() + stride - indices.len(), 0);
        }
        b
    }

    #[test]
    fn binding_two_buttons_publishes_idle_pixels_with_original_parent_alpha() -> Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "crysirror-exhibit-buttons-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        fs::create_dir(&dir)?;
        let result = (|| -> Result<()> {
            let assets = dir.join("assets");
            fs::create_dir(&assets)?;
            fs::write(
                assets.join("panel.gyu"),
                bitmap(&[1, 1, 1, 1], &[9, 15, 80, 255]),
            )?;
            let mut atlas = bitmap(&[4, 5, 6, 7, 8, 9], &[]);
            fs::write(assets.join("atlas.gyu"), &atlas)?;
            atlas[36 + 4 * 4..36 + 4 * 6].fill(191);
            fs::write(assets.join("other.gyu"), atlas)?;
            let project = dir.join("project.json");
            fs::write(&project,br#"{"schema":2,"name":"buttons","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#)?;
            let mut rt = crysirror_runtime::Runtime::load(&project, &dir.join("state"))?;
            let c = ClientId(1);
            let pool = TaskPool::new("exhibit-button-test", 1, 1)?;
            let mut panels = Panels::default();
            panels.define(
                &mut rt.host,
                c,
                Surface {
                    slot: 42,
                    asset: "panel.gyu".into(),
                    origin: [0, 0],
                    level: 16,
                },
            )?;
            for (id, x, atlas) in [(10, 0, "atlas.gyu"), (11, 2, "other.gyu")] {
                let csv =
                    format!("0,1,42,{id},{x},0,{atlas},-2,1,0,-1,-1,,0,0,0,0,0,0,0,0,0,0,0,0");
                panels.bind(&mut rt.host, c, &pool, Button::parse(csv.as_bytes())?)?;
                let deadline = Instant::now() + Duration::from_secs(10);
                while !panels.poll(&mut rt.host, c)? {
                    ensure!(Instant::now() < deadline, "button job timed out");
                    std::thread::yield_now();
                }
            }
            let layer = &panels.layers[&42];
            assert_eq!(layer.items[&10].group, 0);
            assert_eq!(layer.items[&11].pos, [2, 0]);
            let image = rt
                .host
                .published_image_resource(c, layer.published.as_deref().unwrap())?
                .unwrap();
            assert_eq!(
                &*image.rgba(),
                &[4, 2, 1, 9, 5, 2, 1, 15, 4, 2, 1, 80, 5, 2, 1, 255]
            );
            panels.character_layers = vec![Some(42), None];
            panels.disable(&mut rt.host, c, &pool, 1001, 10)?;
            panels.disable(&mut rt.host, c, &pool, 42, 999)?;
            assert!(!panels.pending());
            panels.disable(&mut rt.host, c, &pool, 1000, 11)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while !panels.poll(&mut rt.host, c)? {
                ensure!(Instant::now() < deadline, "disabled button job timed out");
                std::thread::yield_now();
            }
            let layer = &panels.layers[&42];
            assert_eq!(layer.items[&11].enable_count, -1);
            let image = rt
                .host
                .published_image_resource(c, layer.published.as_deref().unwrap())?
                .unwrap();
            assert_eq!(
                &*image.rgba(),
                &[4, 2, 1, 9, 5, 2, 1, 15, 8, 2, 1, 80, 9, 2, 1, 255]
            );
            let mut selection = Selection::new(0, 1300);
            let press = InputFrame {
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                ..InputFrame::default()
            };
            let release = InputFrame::default();
            assert_eq!(selection.poll(&panels, release, Some([1, 0])), None);
            assert_eq!(selection.poll(&panels, press, Some([2, 0])), None);
            assert_eq!(selection.poll(&panels, release, Some([2, 0])), None);
            assert_eq!(selection.poll(&panels, press, Some([1, 0])), None);
            assert_eq!(selection.poll(&panels, release, Some([4, 0])), None);
            assert_eq!(selection.poll(&panels, press, Some([1, 0])), None);
            assert_eq!(selection.poll(&panels, release, None), None);
            panels.control_input(42, false, false);
            assert_eq!(selection.poll(&panels, press, Some([1, 0])), None);
            assert_eq!(selection.poll(&panels, release, Some([1, 0])), None);
            panels.control_input(42, true, false);
            assert_eq!(selection.poll(&panels, press, Some([1, 0])), None);
            assert_eq!(selection.poll(&panels, release, Some([1, 0])), Some(10));
            panels.disable(&mut rt.host, c, &pool, 42, 11)?;
            assert!(!panels.pending());
            assert_eq!(panels.layers[&42].items[&11].enable_count, -2);
            // Animate the composed parent and verify the shared render state
            // still points at the same image throughout the move and fade.
            use crate::panel_animation::{Animations, Spec};
            let mut animations = Animations::default();
            let fade = format!("0,1,42,0,800,0{}", ",-1,0,0,0".repeat(7));
            let movement = format!("0,800,0,0,42,80,40,0{}", ",-1,0,0,0".repeat(5));
            animations.begin(
                Spec::parse(Kind::Fade, fade.as_bytes())?,
                &panels,
                100,
                false,
            )?;
            animations.begin(
                Spec::parse(Kind::Move, movement.as_bytes())?,
                &panels,
                100,
                false,
            )?;
            assert!(animations.waiting());
            animations.tick(&mut panels, &mut rt.host, c, 500)?;
            let scene = rt.inspect();
            let shown = &scene["engine"]["layers"]["clients"]["1"]["298"];
            assert_eq!(shown["opacity"], 128);
            assert_eq!(shown["rect"], serde_json::json!([40.0, 20.0, 4.0, 1.0]));
            assert_eq!(
                shown["paint"]["asset"].as_str(),
                panels.layers[&42].published.as_deref()
            );
            animations.tick(&mut panels, &mut rt.host, c, 901)?;
            animations.leave(Kind::Fade, 0, true);
            assert!(!animations.waiting());
            assert_eq!(
                rt.inspect()["engine"]["layers"]["clients"]["1"]["298"]["opacity"],
                0
            );
            panels.set_values(&mut rt.host, c, 42, Kind::Fade, [16, 0])?;
            let tail = ",-1,0,0,0".repeat(6);
            let mut exact = Animations::default();
            for invalid in [
                "0,1,42,0,500,0,44,0,500,0", // Asynchronous.
                "0,0,42,0,499,0,44,0,500,0", // Different timing.
                "0,0,42,0,500,0,44,1,500,0", // Nonzero target.
            ] {
                let spec = Spec::parse(Kind::Fade, format!("{invalid}{tail}").as_bytes())?;
                assert!(exact.begin(spec, &panels, 1000, false).is_err());
            }
            let spec = Spec::parse(
                Kind::Fade,
                format!("0,0,42,0,500,0,44,0,500,0{tail}").as_bytes(),
            )?;
            exact.begin(spec, &panels, 1000, false)?;
            exact.tick(&mut panels, &mut rt.host, c, 1468)?;
            assert!(exact.waiting());
            assert_eq!(panels.values(42, Kind::Fade)?, [1, 0]);
            exact.tick(&mut panels, &mut rt.host, c, 1469)?;
            assert!(!exact.waiting());
            assert_eq!(panels.values(42, Kind::Fade)?, [0, 0]);
            assert!(!panels.is_bound(44));
            assert!(panels.remove(&mut rt.host, c, 42).is_err());
            panels.exit_group(7);
            assert_eq!(panels.layers[&42].items.len(), 2);
            let published = panels.layers[&42].published.clone().unwrap();
            panels.exit_group(0);
            assert!(panels.layers[&42].items.is_empty());
            assert_eq!(panels.layers[&42].published.as_deref(), Some(published.as_str()));
            panels.remove(&mut rt.host, c, 42)?;
            panels.remove(&mut rt.host, c, 42)?;
            assert!(!panels.is_bound(42));
            assert!(rt.inspect()["engine"]["layers"]["clients"]["1"]["298"].is_null());
            Ok(())
        })();
        fs::remove_dir_all(dir)?;
        result
    }
}
