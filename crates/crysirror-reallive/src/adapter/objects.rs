// SPDX-License-Identifier: MPL-2.0
//! RealLive objects on engine layers. The VM keeps the original object state
//! (`vm::object`, copied from upstream); this file resolves its resources
//! and turns each visible foreground object into retained engine layers with
//! the original placement (`Object::bounds`/`blit_region` geometry), alpha,
//! blend, clip and colour filters. Pixels are composed by the engine.
use super::*;
use crate::vm::graphics::{AssetLoader, Image};
use crate::vm::object::{Object, ObjectData, ObjectParams, TextParams, digit_patterns};
use crysirror_core::service::{ImageMetadata, TextMeasurement};
use std::{cell::RefCell, collections::HashMap};

/// Engine layer ids: one block per object slot, 16 per object (digits).
const OBJECT_LAYER_BASE: u32 = 1_000_000;
/// Objects draw over DC 0 and under the text windows (order 999+).
const OBJECT_ORDER: i32 = 100;

pub(super) struct ObjectAssets {
    pub resources: Rc<RefCell<Resources>>,
    pub exe: Rc<Gameexe>,
    pub metadata: Arc<dyn ImageMetadata>,
    pub text: Option<Arc<dyn TextMeasurement>>,
    pub font: String,
    pub images: RefCell<HashMap<String, Rc<Image>>>,
}

impl AssetLoader for ObjectAssets {
    fn image(&self, name: &str) -> Result<Rc<Image>> {
        let key = name.trim().to_lowercase();
        if let Some(image) = self.images.borrow().get(&key) {
            return Ok(image.clone());
        }
        let asset = self.resources.borrow_mut().find(&self.exe, "image", name.trim())?;
        let image = if asset.to_ascii_lowercase().ends_with(".g00") {
            let bytes = self.resources.borrow().read(&asset)?;
            let (width, height, regions) = crysirror_image::g00::g00_regions(&bytes)
                .with_context(|| format!("g00 regions of {asset}"))?;
            Image { asset, width, height, regions }
        } else {
            let [w, h] = self.metadata.size(&asset).map_err(err)?;
            let (width, height) = (i32::try_from(w)?, i32::try_from(h)?);
            Image {
                asset,
                width,
                height,
                regions: vec![crysirror_image::g00::Region::full(width, height)],
            }
        };
        let image = Rc::new(image);
        let mut cache = self.images.borrow_mut();
        if cache.len() > 512 {
            cache.clear();
        }
        cache.insert(key, image.clone());
        Ok(image)
    }

    fn gan(&self, name: &str) -> Result<Vec<u8>> {
        let mut r = self.resources.borrow_mut();
        let name = name.trim();
        let file = if name.to_ascii_lowercase().ends_with(".gan") { name.to_owned() } else { format!("{name}.gan") };
        let folder = self
            .exe
            .get("FOLDNAME.GAN")
            .and_then(|e| e.str(0).map(|d| d.replace('\\', "/")))
            .unwrap_or_else(|| "gan".into());
        let path = match r.file(&folder, &file)? {
            Some(p) => p,
            None => r.file("", &file)?.with_context(|| format!("reallive: GAN file {name:?} not found"))?,
        };
        r.read(&path)
    }

    fn text_size(&self, p: &TextParams) -> (i32, i32) {
        let Some(text) = &self.text else {
            return (0, 0);
        };
        let size = p.size.clamp(1, 256) as f32;
        let mut width = 0f32;
        let mut lines = 0;
        for line in text_lines(p) {
            lines += 1;
            let extent = text.measure_simple(&self.font, size, &line).map(|e| e.advance).unwrap_or(0.);
            width = width.max(extent + (p.xspace * line.chars().count() as i32) as f32);
        }
        let height = lines as f32 * (size + p.yspace as f32);
        (width.ceil() as i32, height.ceil() as i32)
    }
}

/// Text-object lines: `objTextOpts` characters per line, `\n` breaks.
/// The control syntax (`#c`, `#d`, ...) is not interpreted yet.
fn text_lines(p: &TextParams) -> Vec<String> {
    let per = usize::try_from(p.char_count).unwrap_or(0);
    let mut out = Vec::new();
    for line in p.text.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if per == 0 || chars.is_empty() {
            out.push(line.to_owned());
        } else {
            out.extend(chars.chunks(per).map(|c| c.iter().collect()));
        }
    }
    out
}

/// What a parent contributes to its children (upstream `render`).
struct Inherited {
    x: i32,
    y: i32,
    alpha: i32,
    scale: (f64, f64),
    clip: Option<crate::vm::rect::Rect>,
}

impl RealLiveVm {
    /// Install the resource loader the VM's object functions use.
    pub(super) fn install_object_assets(&mut self, h: &mut dyn HostServices, m: &mut Machine) -> Result<()> {
        let resources = self.resources.clone().context("RealLive resources")?;
        let metadata = h.image_metadata(self.client).map_err(err)?;
        let text = h.text_measurement(self.client).ok();
        m.sys.gfx.width = self.width as i32;
        m.sys.gfx.height = self.height as i32;
        m.sys.gfx.loader = Some(Rc::new(ObjectAssets {
            resources,
            exe: m.gameexe.clone(),
            metadata,
            text,
            font: self.config.font_alias.clone(),
            images: RefCell::new(HashMap::new()),
        }));
        Ok(())
    }

    /// Bring the engine layers in line with the VM's foreground objects.
    pub(super) fn sync_objects(&mut self, m: &mut Machine, h: &mut dyn HostServices) -> Result<()> {
        let now = m.sys.now();
        let animating = m.sys.gfx.update(now)
            || m.sys.gfx.fg.iter().flatten().any(|o| o.animation.as_ref().is_some_and(|a| !a.finished));
        if !m.sys.gfx.dirty && !animating {
            return Ok(());
        }
        m.sys.gfx.dirty = false;
        let mut order: Vec<(i32, i32, i32, usize)> = Vec::new();
        for (index, object) in m.sys.gfx.fg.iter().enumerate() {
            let Some(object) = object else { continue };
            if !object.params.visible || object.data.is_none() {
                continue;
            }
            let p = &object.params;
            order.push((p.z_order, p.z_layer, p.z_depth, index));
        }
        order.sort();
        let mut layers: BTreeMap<u32, Value> = BTreeMap::new();
        for (rank, (.., index)) in order.into_iter().enumerate() {
            let Some(object) = &m.sys.gfx.fg[index] else { continue };
            let at = m.sys.gfx.paused_animations.get(&index).copied().unwrap_or(now);
            let base = OBJECT_LAYER_BASE + index as u32 * 257 * 16;
            self.project(m, object, at, None, base, rank as i32, &mut layers)?;
        }
        let mut ops = Vec::new();
        for id in self.object_layers.keys() {
            if !layers.contains_key(id) {
                ops.push(json!({"op":"remove","id":id}));
            }
        }
        for (id, layer) in &layers {
            if self.object_layers.get(id) != Some(layer) {
                ops.push(json!({"op":"put","layer":layer}));
            }
        }
        self.object_layers = layers;
        if !ops.is_empty() {
            self.submit(h, json!({ "layers": ops }))?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn project(
        &self,
        m: &Machine,
        object: &Object,
        now: u64,
        parent: Option<&Inherited>,
        id: u32,
        rank: i32,
        out: &mut BTreeMap<u32, Value>,
    ) -> Result<()> {
        let p = &object.params;
        if !p.visible {
            return Ok(());
        }
        let Some(data) = object.data.as_ref() else { return Ok(()) };
        let (px, py, parent_alpha, parent_scale) = match parent {
            Some(i) => (i.x, i.y, i.alpha, i.scale),
            None => (0, 0, 255, (1.0, 1.0)),
        };
        let alpha = p.computed_alpha() * parent_alpha / 255;
        let mut clip = p.clip;
        if let Some(own) = parent.and_then(|i| i.clip) {
            let own = crate::vm::rect::Rect::new(px + own.x, py + own.y, own.w, own.h);
            clip = Some(clip.map_or(own, |c| c.intersect(&own)));
        }
        let (ax, ay) = p.adjust_sum();
        match data {
            ObjectData::Rect(_) => {
                ensure!(!p.has_effects(), "RealLive filter-area objects need an engine region filter");
            }
            ObjectData::Parent(children) => {
                let inherited = Inherited {
                    x: p.x + px,
                    y: p.y + py,
                    alpha,
                    scale: parent_scale,
                    clip: p.own_clip,
                };
                for (i, child) in children.iter().enumerate() {
                    if let Some(child) = child {
                        self.project(m, child, now, Some(&inherited), id + 16 * (i as u32 + 1), rank, out)?;
                    }
                }
            }
            ObjectData::Drift { .. } => bail!("RealLive drift (weather) objects need an engine particle mapping"),
            ObjectData::Digits { image, .. } => {
                let mut x = p.x + ax + px;
                let y = p.y + ay + py;
                for (k, pattern) in digit_patterns(&p.digits).into_iter().enumerate().take(16) {
                    let region = image.region(pattern);
                    if pattern >= 0 {
                        out.insert(id + k as u32, self.pattern_layer(p, image, region, (x, y), (0, 0), alpha, clip, parent_scale, id + k as u32, rank)?);
                    }
                    x += if p.digits.space > 0 { p.digits.space } else { region.width() };
                }
            }
            ObjectData::Text => {
                if p.text.text.is_empty() || alpha <= 0 {
                    return Ok(());
                }
                let (sx, sy) = p.scale();
                let colour = m
                    .gameexe
                    .ints(&format!("COLOR_TABLE.{:03}", p.text.colour))
                    .into_iter()
                    .map(|v| v.clamp(0, 255) as u8)
                    .chain([255; 3])
                    .take(3)
                    .collect::<Vec<_>>();
                let size = p.text.size.clamp(1, 256);
                let lines = text_lines(&p.text).join("\n");
                let (w, hgt) = m.sys.gfx.text_size()(&p.text);
                let mut layer = json!({
                    "id": id,
                    "rect": [(p.x + ax + px) as f32, (p.y + ay + py) as f32, w.max(1) as f32, hgt.max(1) as f32],
                    "paint": {"kind":"text","text":lines,"font":self.config.font_alias,"size":size,"color":[colour[0],colour[1],colour[2],255],"layout":crysirror_core::service::SIMPLE_TEXT_LAYOUT},
                    "order": [OBJECT_ORDER, rank],
                    "parent": SCENE_ROOT,
                    "opacity": alpha.clamp(0, 255),
                    "transform": {"scale_x": sx * parent_scale.0, "scale_y": sy * parent_scale.1},
                });
                if let Some(c) = clip {
                    layer["clip"] = json!([c.x, c.y, c.w, c.h]);
                }
                out.insert(id, layer);
            }
            ObjectData::File { image, .. } | ObjectData::Gan { image, .. } => {
                let (pattern, offset, frame_alpha) = object.current_frame(now);
                if pattern < 0 {
                    return Ok(());
                }
                let pattern = pattern + p.button.state.max(0) * i32::from(p.button.is_button != 0);
                let alpha = alpha * frame_alpha.clamp(0, 255) / 255;
                let region = image.region(pattern);
                out.insert(
                    id,
                    self.pattern_layer(
                        p,
                        image,
                        region,
                        (p.x + ax + px + offset.0, p.y + ay + py + offset.1),
                        (region.origin_x, region.origin_y),
                        alpha,
                        clip,
                        parent_scale,
                        id,
                        rank,
                    )?,
                );
            }
        }
        Ok(())
    }

    /// One pattern with the object's transform: upstream `blit_region`
    /// geometry expressed as an engine sprite (pivot in pattern pixels).
    #[allow(clippy::too_many_arguments)]
    fn pattern_layer(
        &self,
        p: &ObjectParams,
        image: &Image,
        mut region: crysirror_image::g00::Region,
        position: (i32, i32),
        pattern_origin: (i32, i32),
        alpha: i32,
        clip: Option<crate::vm::rect::Rect>,
        parent_scale: (f64, f64),
        id: u32,
        rank: i32,
    ) -> Result<Value> {
        if let Some(own) = p.own_clip {
            region.x1 = (region.x1 + own.x).max(region.x1);
            region.y1 = (region.y1 + own.y).max(region.y1);
            region.x2 = region.x2.min(region.x1 + own.w - 1);
            region.y2 = region.y2.min(region.y1 + own.h - 1);
        }
        let (w, h) = (region.width(), region.height());
        let anchor = if p.origin != (0, 0) { p.origin } else { pattern_origin };
        let (sx, sy) = p.scale();
        let scale = (sx * parent_scale.0, sy * parent_scale.1);
        let pivot = (anchor.0 + p.rep_origin.0, anchor.1 + p.rep_origin.1);
        // The pivot lands at position + pivot - anchor (upstream `dest`).
        let dest = (position.0 + pivot.0 - anchor.0, position.1 + pivot.1 - anchor.1);
        let (iw, ih) = (image.width.max(1) as f32, image.height.max(1) as f32);
        let uv = [
            region.x1 as f32 / iw,
            region.y1 as f32 / ih,
            (region.x2 + 1) as f32 / iw,
            (region.y2 + 1) as f32 / ih,
        ];
        let mut layer = json!({
            "id": id,
            "rect": [(dest.0 - pivot.0) as f32, (dest.1 - pivot.1) as f32, w.max(0) as f32, h.max(0) as f32],
            "paint": {"kind":"image","asset":image.asset},
            "uv": uv,
            "order": [OBJECT_ORDER, rank],
                    "parent": SCENE_ROOT,
            "opacity": alpha.clamp(0, 255),
            "blend": match p.composite { 1 => "add", 2 => "subtract", _ => "normal" },
            "transform": {
                "scale_x": scale.0,
                "scale_y": scale.1,
                "rotate": (f64::from(p.rotation) / 10.0).to_radians(),
                "pivot_x": pivot.0 as f32,
                "pivot_y": pivot.1 as f32,
            },
        });
        if w <= 0 || h <= 0 || alpha <= 0 {
            layer["visible"] = json!(false);
            layer["rect"] = json!([0, 0, 1, 1]);
        }
        if let Some(c) = clip {
            layer["clip"] = json!([c.x, c.y, c.w.max(0), c.h.max(0)]);
        }
        if p.has_effects() {
            layer["filter"] = filter(p)?;
        }
        Ok(layer)
    }
}

/// Upstream `ObjectParams::shade` as an engine filter: colour mix, light,
/// mono, invert. Per-channel tint has no exact engine filter yet.
fn filter(p: &ObjectParams) -> Result<Value> {
    ensure!(
        p.tint[0] == p.tint[1] && p.tint[1] == p.tint[2],
        "RealLive per-channel object tint needs an engine filter"
    );
    let light = (p.tint[0] + p.light).clamp(-255, 255) as f32 / 255.0;
    let level = p.colour[3].clamp(0, 255) as f32 / 255.0;
    Ok(json!({
        "color_rate": level,
        "color": [p.colour[0].clamp(0,255) as f32/255.0, p.colour[1].clamp(0,255) as f32/255.0, p.colour[2].clamp(0,255) as f32/255.0],
        "bright": light.max(0.0),
        "dark": (-light).max(0.0),
        "mono": p.mono.clamp(0, 255) as f32 / 255.0,
        "invert": p.invert.clamp(0, 255) as f32 / 255.0,
    }))
}
