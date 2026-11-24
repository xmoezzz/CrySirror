// SPDX-License-Identifier: MPL-2.0
//! NGUI (the game's bundled NGUI 3.x scripts) widget layout, projected onto
//! CrySirror image layers.
//!
//! Policy kept from the C# sources: widgets live under a `UIRoot` whose
//! constrained height (`manualHeight`, 1080) defines the virtual canvas; a
//! widget's rectangle is `mWidth × mHeight` placed by `mPivot` at its
//! transform; `UIRect` anchors move sides to `lerp(target sides, relative) +
//! absolute` (`UIWidget.OnAnchor`); draw order is panel depth then widget depth
//! (`UIPanel.CompareFunc`, `UIWidget.FullCompareFunc`); colour is the widget
//! colour times the panel alpha chain. Pixels come from the atlas material's
//! `_MainTex` (`UISprite`), a Unity `Sprite` (`UI2DSprite`) or a texture
//! (`UITexture`). Sliced sprites use the atlas borders (nine patches).
//! Open scope: SATD-cm3d2-ngui.
use crate::unity_scene::{Component, ObjectKey, Scene, UnityData, UnityFiles};
use anyhow::{Context, Result};
use crysirror_unity::{
    class_id as c,
    classes::{Material, Sprite, Texture2D},
    managed::{Fields, Value},
};
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, HashMap};

/// Virtual UI canvas (UIRoot manual height 1080, 16:9).
pub const WIDTH: f32 = 1920.;
pub const HEIGHT: f32 = 1080.;

/// First layer id of NGUI widgets; ids below are adapter-owned.
pub const LAYER_BASE: u32 = 0x0100_0000;
/// Second and later lines of a label: `LABEL_LINES + (widget id − LAYER_BASE)·16 + line`.
const LABEL_LINES: u32 = 0xA000_0000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect2 {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// Native NGUI SlicedFill keeps the two edges in pixel units.
pub(super) fn slice_boundaries(start: f32, end: f32, first: f32, last: f32) -> [f32; 4] {
    [start, start + first, end - last, end]
}

#[derive(Clone, Debug)]
struct AtlasSprite {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    border: [f32; 4],  // left, right, top, bottom
    padding: [f32; 4], // left, right, top, bottom
}

#[derive(Clone, Debug)]
struct Atlas {
    texture: Option<ObjectKey>,
    sprites: HashMap<String, AtlasSprite>,
}

#[derive(Clone, Debug)]
pub struct Texture {
    pub asset: String,
    pub width: f32,
    pub height: f32,
}

/// Per-node dynamic state the game logic changes (`SetActive`, colours,
/// alpha tweens, sprite names, positions).
#[derive(Clone, Debug, Default)]
pub struct NodeState {
    pub active: Option<bool>,
    pub alpha: Option<f32>,
    pub position: Option<[f32; 3]>,
    pub sprite: Option<String>,
    /// `UISprite.atlas` set at run time (the `UIAtlas` behaviour).
    pub atlas: Option<ObjectKey>,
    pub color: Option<[f32; 4]>,
    /// `UILabel.text`.
    pub text: Option<String>,
    /// `UITexture.mainTexture` set at run time (uploaded image).
    pub texture: Option<Texture>,
    /// Rust handler of `UIButton.onClick` added at run time
    /// (`EventDelegate.Add`), as `Class.Method`.
    pub click: Option<String>,
    /// `UISlider.value`.
    pub value: Option<f32>,
    /// `BoxCollider.enabled` set at run time.
    pub collider: Option<bool>,
    /// Runtime BoxCollider.center in widget-local coordinates.
    pub collider_center: [f32; 2],
    /// `Transform.localScale` set at run time (gauges).
    pub scale: Option<[f32; 3]>,
    /// `UIWidget.SetDimensions` / `width` set at run time.
    pub size: Option<[f32; 2]>,
    /// `UIButton.isEnabled`: `false` drops the collider and draws the
    /// button's `disabledColor`.
    pub enabled: Option<bool>,
    /// `UI2DSprite.sprite2D` replaced at run time: a whole uploaded image,
    /// or `Some(None)` for `sprite2D = null`.
    pub sprite2d: Option<Option<Texture>>,
    /// `UIScrollView.ResetPosition` request (a new value each call).
    pub scroll_reset: Option<u32>,
    /// `UITexture.mainTexture` set to a Unity `Texture2D` at run time.
    pub unity_texture: Option<ObjectKey>,
}

/// A clickable widget of the last layout (`UICamera` raycast target: a
/// widget with a collider).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub scene: usize,
    pub node: usize,
    pub rect: Rect2,
    pub order: [i32; 2],
}

pub struct ButtonVisual {
    pub target: (usize, usize),
    pub color: [f32; 4],
    pub sprite: Option<String>,
}

#[derive(Default)]
pub struct Ui {
    textures: HashMap<ObjectKey, Option<Texture>>,
    atlases: HashMap<ObjectKey, Atlas>,
    sprites: HashMap<ObjectKey, (Sprite, Option<ObjectKey>)>,
    submitted: BTreeMap<u32, String>,
    next_upload: u32,
    fonts: HashMap<ObjectKey, FontSlot>,
    /// Resolved widget rectangles of the last layout (canvas pixels).
    pub rects: HashMap<(usize, usize), Rect2>,
    /// Clickable widgets of the last layout, in draw order.
    pub hits: Vec<Hit>,
    /// Transient UIButtonColor on its tweenTarget; normal tint remains widget-owned.
    pub button_visual: Option<ButtonVisual>,
    /// `UIScrollView` content offsets (root units) and the reset request
    /// they answer, per (scene, node).
    scroll: HashMap<(usize, usize), ([f32; 2], Option<u32>)>,
    /// Widget rectangles collected while measuring scroll content.
    collect: Option<Vec<Rect2>>,
    /// Scroll bars faded out by their `UIScrollView.showScrollBars`.
    faded: std::collections::HashSet<(usize, usize)>,
}

pub trait Uploader {
    fn upload(&mut self, id: u32, width: u32, height: u32, rgba: &[u8]) -> Result<String>;
    fn fonts(&mut self) -> Result<std::sync::Arc<dyn crysirror_core::service::FontLibrary>>;
    fn measurement(&mut self) -> Result<std::sync::Arc<dyn crysirror_core::service::TextMeasurement>>;
}

/// Registration of a Unity dynamic font's embedded TrueType data.
#[derive(Clone, Debug)]
enum FontSlot {
    Pending(crysirror_core::service::FontJob),
    Ready(String),
    Missing,
}

/// NGUI markup removed before shared text layout: `[rrggbb]`, `[-]`,
/// `[b]`/`[i]`/`[u]`/`[s]`/`[sub]`/`[sup]` and their closers (`NGUIText.ParseSymbol`).
pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(']') else {
            out.push_str(tail);
            return out;
        };
        let inner = &tail[1..end];
        let symbol = inner == "-"
            || (inner.len() == 6 && inner.chars().all(|c| c.is_ascii_hexdigit()))
            || matches!(inner, "b" | "/b" | "i" | "/i" | "u" | "/u" | "s" | "/s" | "sub" | "/sub" | "sup" | "/sup" | "/url");
        if symbol {
            rest = &tail[end + 1..];
        } else {
            out.push('[');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Horizontal pivot offset of `UIWidget.Pivot`.
/// Lines of `text` broken at `\n` and wrapped to `width` (NGUIText.WrapText
/// breaks at spaces for Latin text and at any character otherwise).
pub(crate) fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for c in para.chars() {
            line.push(c);
            if measure(&line) > width + 0.5 && line.chars().count() > 1 {
                line.pop();
                let rest = match line.rfind(' ').filter(|_| c.is_ascii_alphanumeric()) {
                    Some(k) if k > 0 => {
                        let tail = line[k + 1..].to_owned();
                        line.truncate(k);
                        tail
                    }
                    _ => String::new(),
                };
                out.push(std::mem::take(&mut line));
                line = rest;
                line.push(c);
            }
        }
        out.push(line);
    }
    out
}

pub fn pivot_x(p: i64) -> f32 {
    pivot(p).0
}

fn pivot(p: i64) -> (f32, f32) {
    let x = match p % 3 {
        0 => 0.,
        1 => 0.5,
        _ => 1.,
    };
    let y = match p / 3 {
        0 => 1.,
        1 => 0.5,
        _ => 0.,
    };
    (x, y)
}

fn color(fields: &Fields, name: &str) -> [f32; 4] {
    fields.floats(name).filter(|c| c.len() == 4).map(|c| [c[0], c[1], c[2], c[3]]).unwrap_or([1.; 4])
}

fn anchor(fields: &Fields, side: &str) -> Option<(ObjectKey, f32, f32)> {
    let a = fields.fields(side)?;
    let target = a.pptr("target")?;
    if target.is_null() {
        return None;
    }
    // Same-file pointers only; anchors never cross scene files.
    Some((
        ObjectKey { file: "".into(), path_id: target.path_id },
        a.float("relative").unwrap_or(0.) as f32,
        a.int("absolute").unwrap_or(0) as f32,
    ))
}

impl Ui {
    fn texture(&mut self, data: &mut UnityData, opener: &mut dyn UnityFiles, up: &mut dyn Uploader, key: &ObjectKey) -> Result<Option<Texture>> {
        if let Some(t) = self.textures.get(key) {
            return Ok(t.clone());
        }
        let (class, bytes) = data.bytes(opener, key)?;
        let t = if class == c::TEXTURE_2D {
            let tex = Texture2D::read(&bytes)?;
            if tex.width == 0 {
                None
            } else {
                let img = crysirror_unity::texture::decode(&tex).with_context(|| format!("texture {}", tex.name))?;
                self.next_upload += 1;
                let asset = up.upload(0x4000_0000 + self.next_upload, img.width, img.height, &img.rgba)?;
                Some(Texture { asset, width: img.width as f32, height: img.height as f32 })
            }
        } else {
            None
        };
        self.textures.insert(key.clone(), t.clone());
        Ok(t)
    }

    fn font(&mut self, data: &mut UnityData, opener: &mut dyn UnityFiles, up: &mut dyn Uploader, key: &ObjectKey) -> Result<Option<String>> {
        use crysirror_core::service::{FontLoadSource, FontLoadSpec, FontLoadState};
        let slot = match self.fonts.get(key) {
            Some(s) => s.clone(),
            None => {
                let (class, bytes) = data.bytes(opener, key)?;
                let slot = if class == c::FONT {
                    let f = crysirror_unity::classes::Font::read(&bytes)?;
                    if f.font_data.is_empty() {
                        FontSlot::Missing
                    } else {
                        let job = up.fonts()?.begin(FontLoadSpec {
                            name: format!("cm3d2-{}", f.name.to_ascii_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-")),
                            source: FontLoadSource::Bytes(f.font_data.into()),
                            face_index: 0,
                        })?;
                        FontSlot::Pending(job)
                    }
                } else {
                    FontSlot::Missing
                };
                self.fonts.insert(key.clone(), slot.clone());
                slot
            }
        };
        Ok(match slot {
            FontSlot::Ready(alias) => Some(alias),
            FontSlot::Missing => None,
            FontSlot::Pending(job) => match up.fonts()?.status(job)? {
                FontLoadState::Pending => None,
                FontLoadState::Ready(info) => {
                    self.fonts.insert(key.clone(), FontSlot::Ready(info.alias.clone()));
                    Some(info.alias)
                }
                FontLoadState::Failed(e) => anyhow::bail!("UILabel font registration: {e}"),
            },
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn label(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        file: &str,
        f: &Fields,
        state: Option<&NodeState>,
        rect: Rect2,
        tint: [f32; 4],
        order: [i32; 2],
        id: u32,
        layers: &mut BTreeMap<u32, Json>,
    ) -> Result<bool> {
        let text = state.and_then(|s| s.text.clone()).unwrap_or_else(|| f.string("mText").unwrap_or("").to_owned());
        let text = strip_markup(&text);
        if text.is_empty() {
            return Ok(false);
        }
        // SATD-cm3d2-ngui: bitmap `UIFont` labels.
        let Some(font_key) = f.pptr("mTrueTypeFont").map(|p| data.resolve(opener, file, p)).transpose()?.flatten() else {
            return Ok(false);
        };
        let Some(alias) = self.font(data, opener, up, &font_key)? else { return Ok(false) };
        let measure = up.measurement()?;
        // NGUIText.spacingX is added after every glyph; such lines are
        // drawn as positioned glyph runs.
        let spacing_x = f.int("mSpacingX").unwrap_or(0) as f32;
        let width_of = |size: f32, t: &str| -> f32 {
            let w = measure.measure_simple(&alias, size, t).map(|e| e.advance).unwrap_or_else(|_| t.chars().map(|c| if c.is_ascii() { size * 0.5 } else { size }).sum());
            w + spacing_x * t.chars().count() as f32
        };
        let (px, py) = pivot(f.int("mPivot").unwrap_or(4));
        // NGUIText.Alignment: Automatic follows the pivot; Justified → Left.
        let align = match f.int("mAlignment").unwrap_or(0) {
            0 => px,
            2 => 0.5,
            3 => 1.,
            _ => 0.,
        };
        let (w, h) = (rect.x1 - rect.x0, rect.y1 - rect.y0);
        let spacing = f.int("mSpacingY").unwrap_or(0) as f32;
        // mOverflow: ShrinkContent 0, ClampContent 1, ResizeFreely 2, ResizeHeight 3.
        let overflow = f.int("mOverflow").unwrap_or(0);
        let max_lines = f.int("mMaxLineCount").unwrap_or(0).max(0) as usize;
        let mut size = f.int("mFontSize").unwrap_or(16).clamp(1, 512) as f32;
        let lines = loop {
            let lines = wrap(&text, if overflow == 2 { f32::INFINITY } else { w }, |t| width_of(size, t));
            let fits = (max_lines == 0 || lines.len() <= max_lines) && lines.len() as f32 * (size + spacing) - spacing <= h + 0.5;
            if overflow != 0 || fits || size <= 1. {
                // NGUIText.WrapText: outside ResizeHeight/ResizeFreely the
                // region height bounds the line count (ClampContent drops
                // the rest).
                let by_height = if overflow >= 2 { usize::MAX } else { (((h + 0.5) / (size + spacing)).floor() as usize).max(1) };
                let limit = if max_lines > 0 { max_lines.min(by_height) } else { by_height };
                break lines.into_iter().take(limit).collect::<Vec<_>>();
            }
            size -= 1.;
        };
        // UILabel.ApplyOffset: the printed block sits at the pivot's height.
        let line_h = size + spacing;
        let block = lines.len() as f32 * line_h - spacing;
        let top = rect.y0 + (h - block) * (1. - py);
        for (i, line) in lines.iter().enumerate().take(16) {
            if line.is_empty() {
                continue;
            }
            let lw = width_of(size, line);
            let x = rect.x0 + (w - lw) * align;
            let lid = if i == 0 { id } else { LABEL_LINES + (id - LAYER_BASE) * 16 + i as u32 };
            let color = [byte(tint[0]), byte(tint[1]), byte(tint[2]), byte(tint[3])];
            if spacing_x != 0. {
                let ascent = measure.measure_simple(&alias, size, line).map(|e| e.ascent).unwrap_or(size * 0.8);
                let mut gx = 0.;
                let glyphs: Vec<Json> = line
                    .chars()
                    .map(|c| {
                        let g = json!({"character": c, "baseline": [gx, ascent]});
                        gx += width_of(size, c.encode_utf8(&mut [0; 4]));
                        g
                    })
                    .collect();
                layers.insert(lid, json!({
                    "id": lid,
                    "rect": [x, top + i as f32 * line_h, lw.max(1.) + size, size * 1.5],
                    "paint": {"kind": "glyph_run", "font": alias, "size": size, "color": color, "glyphs": glyphs},
                    "order": [order[0], order[1]],
                }));
                continue;
            }
            layers.insert(lid, json!({
                "id": lid,
                "rect": [x, top + i as f32 * line_h, lw.max(1.) + size, size * 1.5],
                "paint": {"kind": "text", "text": line, "font": alias, "size": size as u16,
                          "color": [byte(tint[0]), byte(tint[1]), byte(tint[2]), byte(tint[3])],
                          "layout": crysirror_core::service::SIMPLE_TEXT_LAYOUT},
                "order": [order[0], order[1]],
            }));
        }
        Ok(true)
    }

    fn material_texture(&mut self, data: &mut UnityData, opener: &mut dyn UnityFiles, key: &ObjectKey) -> Result<Option<ObjectKey>> {
        let (class, bytes) = data.bytes(opener, key)?;
        if class != c::MATERIAL {
            return Ok(None);
        }
        let m = Material::read(&bytes)?;
        match m.texture("_MainTex") {
            Some(t) => data.resolve(opener, &key.file, t.texture),
            None => Ok(None),
        }
    }

    fn atlas(&mut self, data: &mut UnityData, opener: &mut dyn UnityFiles, key: &ObjectKey, depth: usize) -> Result<Option<Atlas>> {
        if let Some(a) = self.atlases.get(key) {
            return Ok(Some(a.clone()));
        }
        let (script, _, _, fields) = data.behaviour(opener, key)?;
        anyhow::ensure!(script == "UIAtlas", "{script} used as an atlas");
        if let Some(r) = fields.pptr("mReplacement").filter(|p| !p.is_null()) {
            if depth < 8 {
                if let Some(rk) = data.resolve(opener, &key.file, r)? {
                    return self.atlas(data, opener, &rk, depth + 1);
                }
            }
        }
        let texture = match fields.pptr("material").map(|p| data.resolve(opener, &key.file, p)).transpose()?.flatten() {
            Some(m) => self.material_texture(data, opener, &m)?,
            None => None,
        };
        let mut sprites = HashMap::new();
        for s in fields.array("mSprites").unwrap_or(&[]) {
            let Value::Struct(s) = s else { continue };
            let i = |n: &str| s.int(n).unwrap_or(0) as f32;
            sprites.insert(
                s.string("name").unwrap_or("").to_owned(),
                AtlasSprite {
                    x: i("x"),
                    y: i("y"),
                    w: i("width"),
                    h: i("height"),
                    border: [i("borderLeft"), i("borderRight"), i("borderTop"), i("borderBottom")],
                    padding: [i("paddingLeft"), i("paddingRight"), i("paddingTop"), i("paddingBottom")],
                },
            );
        }
        let atlas = Atlas { texture, sprites };
        self.atlases.insert(key.clone(), atlas.clone());
        Ok(Some(atlas))
    }

    /// Lay out and draw every visible widget of `scenes`; returns layer ops
    /// (puts of changed layers and removes of vanished ones).
    pub fn compose(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        scenes: &[(&Scene, &HashMap<usize, NodeState>)],
    ) -> Result<Vec<Json>> {
        let mut layers: BTreeMap<u32, Json> = BTreeMap::new();
        self.rects.clear();
        self.hits.clear();
        for (si, (scene, states)) in scenes.iter().enumerate() {
            for &root in &scene.roots {
                self.walk(data, opener, up, si, scene, states, root, Walk::default(), &mut layers)?;
            }
        }
        let mut ops = Vec::new();
        let mut next = BTreeMap::new();
        for (id, spec) in layers {
            // Mirrored (negative-size) or empty source rectangles need the
            // flip/fill paths of SATD-cm3d2-ngui; they draw nothing here.
            if let Some(uv) = spec.get("uv").and_then(|v| v.as_array()) {
                let f = |i: usize| uv.get(i).and_then(Json::as_f64).unwrap_or(0.);
                if f(2) <= f(0) || f(3) <= f(1) {
                    continue;
                }
            }
            let text = spec.to_string();
            if self.submitted.get(&id) != Some(&text) {
                ops.push(json!({"op": "put", "layer": spec}));
            }
            next.insert(id, text);
        }
        for id in self.submitted.keys() {
            if !next.contains_key(id) {
                ops.push(json!({"op": "remove", "id": id}));
            }
        }
        self.submitted = next;
        Ok(ops)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        si: usize,
        scene: &Scene,
        states: &HashMap<usize, NodeState>,
        node: usize,
        mut w: Walk,
        all_layers: &mut BTreeMap<u32, Json>,
    ) -> Result<()> {
        let n = &scene.nodes[node];
        let state = states.get(&node);
        if !state.and_then(|s| s.active).unwrap_or(n.active) {
            return Ok(());
        }
        let is_root = n.components.iter().any(|(_, c)| matches!(c, Component::Script { class, .. } if class == "UIRoot"));
        let position = state.and_then(|s| s.position).unwrap_or(n.position);
        if self.faded.contains(&(si, node)) {
            w.alpha = 0.;
        }
        if is_root {
            w = Walk { in_root: true, scale: [1., 1.], offset: [0., 0.], alpha: 1., panel_depth: 0, clip: None };
        } else if w.in_root {
            w.offset = [w.offset[0] + position[0] * w.scale[0], w.offset[1] + position[1] * w.scale[1]];
            let scale = state.and_then(|s| s.scale).unwrap_or(n.scale);
            w.scale = [w.scale[0] * scale[0], w.scale[1] * scale[1]];
        }
        if w.in_root {
            for (_, comp) in &n.components {
                let Component::Script { class, enabled, fields } = comp else { continue };
                if !enabled {
                    continue;
                }
                if class == "UIPanel" {
                    w.alpha *= fields.float("mAlpha").unwrap_or(1.) as f32;
                    w.panel_depth = fields.int("mDepth").unwrap_or(0) as i32;
                    // UIPanel.mClipping (None = 0): the clip range around
                    // the panel plus its clip offset, in canvas pixels.
                    if fields.int("mClipping").unwrap_or(0) != 0 {
                        if let Some(r) = fields.floats("mClipRange").filter(|r| r.len() == 4) {
                            let off = fields.floats("mClipOffset").filter(|o| o.len() == 2).map_or([0., 0.], |o| [o[0], o[1]]);
                            let cx = w.offset[0] + (r[0] + off[0]) * w.scale[0] + WIDTH / 2.;
                            let cy = HEIGHT / 2. - (w.offset[1] + (r[1] + off[1]) * w.scale[1]);
                            let (hw, hh) = (r[2] * w.scale[0] / 2., r[3] * w.scale[1] / 2.);
                            let rect = Rect2 { x0: cx - hw, y0: cy - hh, x1: cx + hw, y1: cy + hh };
                            w.clip = Some(w.clip.map_or(rect, |c| intersect(c, rect)));
                        }
                    }
                }
            }
            if let Some(a) = state.and_then(|s| s.alpha) {
                w.alpha *= a;
            }
            for (ci, (_, comp)) in n.components.iter().enumerate() {
                let Component::Script { class, enabled, fields } = comp else { continue };
                if !enabled || !matches!(class.as_str(), "UISprite" | "UI2DSprite" | "UITexture" | "UIWidget" | "UILabel") {
                    continue;
                }
                let rect = self.widget_rect(si, scene, node, fields, state.and_then(|s| s.size), &w);
                self.rects.insert((si, node), rect);
                if let Some(c) = self.collect.as_mut() {
                    c.push(rect);
                }
                let order = [w.panel_depth, fields.int("mDepth").unwrap_or(0) as i32];
                // UICamera skips colliders of invisible widgets (finalAlpha ≤ 0.001).
                let alpha = state.and_then(|s| s.color).map(|c| c[3]).unwrap_or_else(|| color(fields, "mColor")[3]) * w.alpha;
                let disabled = state.and_then(|s| s.enabled) == Some(false);
                let collider = state.and_then(|s| s.collider).unwrap_or(true) && !disabled;
                // UICamera skips touches outside the clipping panels.
                let [cx, cy] = state.map_or([0.; 2], |s| s.collider_center);
                let (dx, dy) = (cx * w.scale[0], -cy * w.scale[1]);
                let collider_rect = Rect2 { x0: rect.x0 + dx, x1: rect.x1 + dx, y0: rect.y0 + dy, y1: rect.y1 + dy };
                let hit_rect = w.clip.map_or(collider_rect, |c| intersect(c, collider_rect));
                if collider && alpha > 0.001 && hit_rect.x1 > hit_rect.x0 && hit_rect.y1 > hit_rect.y0 && n.components.iter().any(|(_, c)| matches!(c, Component::Other { class_id: 65 })) {
                    self.hits.push(Hit { scene: si, node, rect: hit_rect, order });
                }
                if class == "UIWidget" {
                    continue;
                }
                let mut tint = state.and_then(|s| s.color).unwrap_or_else(|| color(fields, "mColor"));
                let button = self.button_visual.as_ref().filter(|button| button.target == (si, node) && !disabled);
                let button_sprite = button.and_then(|b| b.sprite.clone());
                if let Some(button) = button {
                    tint = button.color;
                }
                // UIButtonColor: the disabled state tints the target widget.
                if disabled {
                    if let Some(c) = scene.script(node, "UIButton").and_then(|b| b.floats("disabledColor")) {
                        tint = [c[0], c[1], c[2], c[3]];
                    }
                }
                tint[3] *= w.alpha;
                let id = LAYER_BASE + (si as u32) * 0x0010_0000 + (node as u32) * 4 + ci as u32 % 4;
                let mut own = BTreeMap::new();
                let layers = &mut own;
                let order = [w.panel_depth, fields.int("mDepth").unwrap_or(0) as i32];
                let drawn = match class.as_str() {
                    "UISprite" => self.sprite(data, opener, up, &n.key.file, fields, state, button_sprite.as_deref(), rect, tint, order, id, layers)?,
                    "UI2DSprite" => match state.and_then(|s| s.sprite2d.clone()) {
                        Some(Some(tex)) => {
                            layers.insert(id, image(id, rect, &tex.asset, [0., 0., 1., 1.], tint, order));
                            true
                        }
                        Some(None) => false,
                        None => self.sprite_2d(data, opener, up, &n.key.file, fields, rect, tint, order, id, layers)?,
                    },
                    "UILabel" => self.label(data, opener, up, &n.key.file, fields, state, rect, tint, order, id, layers)?,
                    _ => match state.and_then(|s| s.texture.clone()) {
                        Some(tex) => {
                            layers.insert(id, image(id, rect, &tex.asset, [0., 0., 1., 1.], tint, order));
                            true
                        }
                        None if state.is_some_and(|s| s.unity_texture.is_some()) => match self.texture(data, opener, up, state.unwrap().unity_texture.as_ref().unwrap())? {
                            Some(tex) => {
                                layers.insert(id, image(id, rect, &tex.asset, [0., 0., 1., 1.], tint, order));
                                true
                            }
                            None => false,
                        },
                        None => self.ui_texture(data, opener, up, &n.key.file, fields, rect, tint, order, id, layers)?,
                    },
                };
                let _ = drawn;
                for (k, mut v) in own {
                    if let Some(c) = w.clip {
                        v["viewport_clip"] = json!([c.x0, c.y0, c.x1 - c.x0, c.y1 - c.y0]);
                    }
                    all_layers.insert(k, v);
                }
            }
        }
        // UIScrollView: its content shifts by the scroll offset; a
        // ResetPosition request aligns the content's top-left with the clip.
        if let Some(sv) = scene.script(node, "UIScrollView").filter(|_| w.in_root) {
            let movement = sv.int("movement").unwrap_or(0);
            // showScrollBars: Always 0, OnlyIfNeeded 1, WhenDragging 2 (no
            // drag is ever in progress here). Bars fade out while the
            // content fits the clip.
            let show = sv.int("showScrollBars").unwrap_or(0);
            let bars: Vec<usize> = ["horizontalScrollBar", "verticalScrollBar"]
                .iter()
                .filter_map(|k| sv.pptr(k).filter(|p| !p.is_null()))
                .filter_map(|p| scene.find_ref(node, p.path_id))
                .collect();
            if !bars.is_empty() {
                let hide = match (show, w.clip) {
                    (0, _) => false,
                    (1, Some(clip)) => {
                        let (hits, rects) = (self.hits.len(), self.rects.clone());
                        let outer = self.collect.replace(Vec::new());
                        let mut scratch = BTreeMap::new();
                        for &child in &n.children {
                            self.walk(data, opener, up, si, scene, states, child, w, &mut scratch)?;
                        }
                        let found = std::mem::replace(&mut self.collect, outer).unwrap_or_default();
                        self.hits.truncate(hits);
                        self.rects = rects;
                        match found.into_iter().reduce(|a, b| Rect2 { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }) {
                            Some(b) => {
                                let fits_x = movement == 1 || b.x1 - b.x0 <= clip.x1 - clip.x0 + 0.5;
                                let fits_y = movement == 0 || b.y1 - b.y0 <= clip.y1 - clip.y0 + 0.5;
                                fits_x && fits_y
                            }
                            None => true,
                        }
                    }
                    _ => true,
                };
                for b in bars {
                    if hide {
                        self.faded.insert((si, b));
                    } else {
                        self.faded.remove(&(si, b));
                    }
                }
            }
            let request = state.and_then(|s| s.scroll_reset);
            let entry = self.scroll.get(&(si, node)).copied().unwrap_or(([0.; 2], None));
            let mut offset = entry.0;
            if request.is_some() && request != entry.1 {
                if let Some(clip) = w.clip {
                    let (hits, rects) = (self.hits.len(), self.rects.clone());
                    let outer = self.collect.replace(Vec::new());
                    let mut scratch = BTreeMap::new();
                    for &child in &n.children {
                        self.walk(data, opener, up, si, scene, states, child, w, &mut scratch)?;
                    }
                    let found = std::mem::replace(&mut self.collect, outer).unwrap_or_default();
                    self.hits.truncate(hits);
                    self.rects = rects;
                    if let Some(b) = found.into_iter().reduce(|a, b| Rect2 { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) }) {
                        // UIPanel.softBorderPadding keeps the content inside the soft edge.
                        let panel = scene.script(node, "UIPanel");
                        let soft = match panel {
                            Some(p) if p.bool("softBorderPadding").unwrap_or(false) => p.floats("mClipSoftness").filter(|f| f.len() == 2).map_or([0., 0.], |f| [f[0], f[1]]),
                            _ => [0., 0.],
                        };
                        offset = [0., 0.];
                        if movement != 1 {
                            offset[0] = (clip.x0 + soft[0] - b.x0) / w.scale[0].max(1e-6);
                        }
                        if movement != 0 {
                            offset[1] = -(clip.y0 + soft[1] - b.y0) / w.scale[1].max(1e-6);
                        }
                    }
                }
                self.scroll.insert((si, node), (offset, request));
            }
            w.offset = [w.offset[0] + offset[0] * w.scale[0], w.offset[1] + offset[1] * w.scale[1]];
        }
        for &child in &n.children {
            self.walk(data, opener, up, si, scene, states, child, w, all_layers)?;
        }
        Ok(())
    }

    /// The widget rectangle in canvas pixels, after anchors.
    fn widget_rect(&self, si: usize, scene: &Scene, node: usize, f: &Fields, size: Option<[f32; 2]>, w: &Walk) -> Rect2 {
        let (px, py) = pivot(f.int("mPivot").unwrap_or(4));
        let width = size.map_or(f.int("mWidth").unwrap_or(100) as f32, |s| s[0]);
        let height = size.map_or(f.int("mHeight").unwrap_or(100) as f32, |s| s[1]);
        // Local rectangle around the transform (UIRoot space, y up).
        let mut x0 = w.offset[0] - px * width * w.scale[0];
        let mut x1 = x0 + width * w.scale[0];
        let mut y0 = w.offset[1] - py * height * w.scale[1];
        let mut y1 = y0 + height * w.scale[1];
        let sides = |key: &ObjectKey| -> Option<Rect2> {
            let target = scene.find_ref(node, key.path_id)?;
            Some(self.rects.get(&(si, target)).copied().unwrap_or(Rect2 {
                // Panels, cameras and roots stand for the screen.
                x0: 0.,
                y0: 0.,
                x1: WIDTH,
                y1: HEIGHT,
            }))
        };
        let to_root_x = |cx: f32| cx - WIDTH / 2.;
        let to_root_y = |cy: f32| HEIGHT / 2. - cy;
        if let Some((k, rel, abs)) = anchor(f, "leftAnchor") {
            if let Some(s) = sides(&k) {
                x0 = to_root_x(s.x0 + (s.x1 - s.x0) * rel) + abs;
            }
        }
        if let Some((k, rel, abs)) = anchor(f, "rightAnchor") {
            if let Some(s) = sides(&k) {
                x1 = to_root_x(s.x0 + (s.x1 - s.x0) * rel) + abs;
            }
        }
        if let Some((k, rel, abs)) = anchor(f, "bottomAnchor") {
            if let Some(s) = sides(&k) {
                y0 = to_root_y(s.y1 + (s.y0 - s.y1) * rel) + abs;
            }
        }
        if let Some((k, rel, abs)) = anchor(f, "topAnchor") {
            if let Some(s) = sides(&k) {
                y1 = to_root_y(s.y1 + (s.y0 - s.y1) * rel) + abs;
            }
        }
        Rect2 { x0: x0 + WIDTH / 2., y0: HEIGHT / 2. - y1, x1: x1 + WIDTH / 2., y1: HEIGHT / 2. - y0 }
    }

    #[allow(clippy::too_many_arguments)]
    fn sprite(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        file: &str,
        f: &Fields,
        state: Option<&NodeState>,
        button_sprite: Option<&str>,
        rect: Rect2,
        tint: [f32; 4],
        order: [i32; 2],
        id: u32,
        layers: &mut BTreeMap<u32, Json>,
    ) -> Result<bool> {
        let atlas_key = match state.and_then(|s| s.atlas.clone()) {
            Some(k) => Some(k),
            None => f.pptr("mAtlas").map(|p| data.resolve(opener, file, p)).transpose()?.flatten(),
        };
        let Some(atlas_key) = atlas_key else {
            return Ok(false);
        };
        let Some(atlas) = self.atlas(data, opener, &atlas_key, 0)? else { return Ok(false) };
        let name = button_sprite.or_else(|| state.and_then(|s| s.sprite.as_deref())).or_else(|| f.string("mSpriteName")).unwrap_or("");
        let Some(s) = atlas.sprites.get(name) else { return Ok(false) };
        let Some(tk) = atlas.texture.clone() else { return Ok(false) };
        let Some(tex) = self.texture(data, opener, up, &tk)? else { return Ok(false) };
        // Trimmed sprites keep their padding as transparent margins;
        // UISprite.drawingDimensions scales it only for Simple/Filled
        // sprites (sliced and tiled ones keep it in pixels).
        let (pw, ph) = (s.w + s.padding[0] + s.padding[1], s.h + s.padding[2] + s.padding[3]);
        let (sx, sy) = match f.int("mType").unwrap_or(0) {
            0 | 3 => ((rect.x1 - rect.x0) / pw.max(1.), (rect.y1 - rect.y0) / ph.max(1.)),
            _ => (1., 1.),
        };
        let inner = Rect2 {
            x0: rect.x0 + s.padding[0] * sx,
            y0: rect.y0 + s.padding[2] * sy,
            x1: rect.x1 - s.padding[1] * sx,
            y1: rect.y1 - s.padding[3] * sy,
        };
        let sliced = f.int("mType").unwrap_or(0) == 1 && s.border.iter().any(|b| *b > 0.);
        let uv = |x: f32, y: f32, w: f32, h: f32| [x / tex.width, y / tex.height, (x + w) / tex.width, (y + h) / tex.height];
        if s.w <= 0. || s.h <= 0. || inner.x1 <= inner.x0 || inner.y1 <= inner.y0 {
            return Ok(false);
        }
        if !sliced {
            layers.insert(id, image(id, inner, &tex.asset, uv(s.x, s.y, s.w, s.h), tint, order));
            return Ok(true);
        }
        // Nine patches: borders keep pixel size, the centre stretches.
        let [bl, br, bt, bb] = s.border;
        let xs = slice_boundaries(inner.x0, inner.x1, bl, br);
        let ys = slice_boundaries(inner.y0, inner.y1, bt, bb);
        let us = slice_boundaries(s.x, s.x + s.w, bl, br);
        let vs = slice_boundaries(s.y, s.y + s.h, bt, bb);
        let fill_center = f.bool("mFillCenter").unwrap_or(true);
        for row in 0..3 {
            for col in 0..3 {
                if row == 1 && col == 1 && !fill_center {
                    continue;
                }
                let r = Rect2 { x0: xs[col], y0: ys[row], x1: xs[col + 1], y1: ys[row + 1] };
                if r.x1 - r.x0 <= 0. || r.y1 - r.y0 <= 0. {
                    continue;
                }
                // Borders that cover the whole sprite leave a zero-size
                // centre span: NGUI stretches the boundary texels there.
                if us[col + 1] < us[col] || vs[row + 1] < vs[row] {
                    continue;
                }
                let sub = id ^ (((row * 3 + col) as u32 + 1) << 28);
                let (u0, u1) = if us[col + 1] > us[col] { (us[col], us[col + 1]) } else { (us[col] - 0.5, us[col] + 0.5) };
                let (v0, v1) = if vs[row + 1] > vs[row] { (vs[row], vs[row + 1]) } else { (vs[row] - 0.5, vs[row] + 0.5) };
                layers.insert(sub, image(sub, r, &tex.asset, uv(u0, v0, u1 - u0, v1 - v0), tint, order));
            }
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn sprite_2d(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        file: &str,
        f: &Fields,
        rect: Rect2,
        tint: [f32; 4],
        order: [i32; 2],
        id: u32,
        layers: &mut BTreeMap<u32, Json>,
    ) -> Result<bool> {
        let Some(key) = f.pptr("mSprite").map(|p| data.resolve(opener, file, p)).transpose()?.flatten() else {
            return Ok(false);
        };
        let (sprite, tex_key) = match self.sprites.get(&key) {
            Some(s) => s.clone(),
            None => {
                let (class, bytes) = data.bytes(opener, &key)?;
                anyhow::ensure!(class == c::SPRITE, "UI2DSprite reference is not a Sprite");
                let s = Sprite::read(&bytes)?;
                let t = data.resolve(opener, &key.file, s.texture)?;
                self.sprites.insert(key.clone(), (s.clone(), t.clone()));
                (s, t)
            }
        };
        let Some(tk) = tex_key else { return Ok(false) };
        let Some(tex) = self.texture(data, opener, up, &tk)? else { return Ok(false) };
        // Sprite rects are bottom-up texture pixels; decoded rows are top-down.
        let [x, y, w, h] = sprite.rect;
        let top = tex.height - y - h;
        layers.insert(id, image(id, rect, &tex.asset, [x / tex.width, top / tex.height, (x + w) / tex.width, (top + h) / tex.height], tint, order));
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn ui_texture(
        &mut self,
        data: &mut UnityData,
        opener: &mut dyn UnityFiles,
        up: &mut dyn Uploader,
        file: &str,
        f: &Fields,
        rect: Rect2,
        tint: [f32; 4],
        order: [i32; 2],
        id: u32,
        layers: &mut BTreeMap<u32, Json>,
    ) -> Result<bool> {
        let Some(key) = f.pptr("mTexture").map(|p| data.resolve(opener, file, p)).transpose()?.flatten() else {
            return Ok(false);
        };
        let Some(tex) = self.texture(data, opener, up, &key)? else { return Ok(false) };
        let r = f.floats("mRect").filter(|r| r.len() == 4).map(|r| [r[0], r[1], r[2], r[3]]).unwrap_or([0., 0., 1., 1.]);
        let uv = [r[0], 1. - r[1] - r[3], r[0] + r[2], 1. - r[1]];
        layers.insert(id, image(id, rect, &tex.asset, uv, tint, order));
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Walk {
    in_root: bool,
    scale: [f32; 2],
    offset: [f32; 2],
    alpha: f32,
    panel_depth: i32,
    /// Intersection of the clipping panels above (canvas pixels).
    clip: Option<Rect2>,
}

fn intersect(a: Rect2, b: Rect2) -> Rect2 {
    Rect2 { x0: a.x0.max(b.x0), y0: a.y0.max(b.y0), x1: a.x1.min(b.x1).max(a.x0.max(b.x0)), y1: a.y1.min(b.y1).max(a.y0.max(b.y0)) }
}

fn byte(v: f32) -> u8 {
    (v.clamp(0., 1.) * 255. + 0.5) as u8
}

fn image(id: u32, r: Rect2, asset: &str, uv: [f32; 4], tint: [f32; 4], order: [i32; 2]) -> Json {
    // Atlas rectangles at the texture edge may round outside [0, 1].
    let uv = uv.map(|v| if v.is_finite() { v.clamp(0., 1.) } else { 0. });
    json!({
        "id": id,
        "rect": [r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0],
        "paint": {"kind": "image", "asset": asset, "tint": [byte(tint[0]), byte(tint[1]), byte(tint[2]), byte(tint[3])]},
        "uv": uv,
        "order": [order[0], order[1]],
        // Unity 4 renders in gamma space: NGUI's alpha blend weighs the
        // stored (gamma-encoded) bytes, not linear light.
        "blend": {"stored_byte": {"mode": "scalar256", "force_output_opaque": true}},
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn markup_is_removed_but_brackets_stay() {
        assert_eq!(super::strip_markup("[ff0000]red[-] [b]x[/b] [SF]"), "red x [SF]");
        assert_eq!(super::strip_markup("a[b"), "a[b");
    }
}

/// `UITable.Reposition` (`RepositionVariableSize`) of the active children of
/// `parent`: cells sized by each child's root widget, `columns` per row,
/// `padding` around cells, rows going down. Returns child local positions.
/// `BaseCreateViewerCtrl.Reposition`: a `UITable`, else a `UIGrid`.
pub fn reposition_positions(scene: &crate::unity_scene::Scene, states: &HashMap<usize, NodeState>, parent: usize) -> Vec<(usize, [f32; 3])> {
    if scene.script(parent, "UITable").is_some() { table_positions(scene, states, parent) } else { grid_positions(scene, states, parent) }
}

pub fn table_positions(scene: &crate::unity_scene::Scene, states: &HashMap<usize, NodeState>, parent: usize) -> Vec<(usize, [f32; 3])> {
    let Some(t) = scene.script(parent, "UITable") else { return Vec::new() };
    let columns = t.int("columns").unwrap_or(0).max(0) as usize;
    let pad = t.floats("padding").map(|p| [p[0], p[1]]).unwrap_or([0.; 2]);
    let children: Vec<usize> = scene.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|c| states.get(c).and_then(|s| s.active).unwrap_or(scene.nodes[*c].active))
        .collect();
    // NGUIMath.CalculateRelativeWidgetBounds: the active widgets of the
    // child and its descendants, in the child's space.
    let bounds = |c: usize| -> (f32, f32, f32, f32) {
        let mut acc: Option<(f32, f32, f32, f32)> = None;
        let mut stack = vec![(c, [0f32, 0f32])];
        while let Some((n, at)) = stack.pop() {
            if !states.get(&n).and_then(|s| s.active).unwrap_or(scene.nodes[n].active) {
                continue;
            }
            let f = ["UITexture", "UISprite", "UIWidget", "UILabel", "UI2DSprite"].iter().find_map(|k| scene.script(n, k));
            if let Some(f) = f {
                let (px, py) = pivot(f.int("mPivot").unwrap_or(4));
                let size = states.get(&n).and_then(|s| s.size);
                let (w, h) = size.map_or((f.int("mWidth").unwrap_or(0) as f32, f.int("mHeight").unwrap_or(0) as f32), |s| (s[0], s[1]));
                let r = (at[0] - px * w, at[1] - py * h, at[0] + (1. - px) * w, at[1] + (1. - py) * h);
                acc = Some(acc.map_or(r, |a| (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3))));
            }
            for &k in &scene.nodes[n].children {
                let p = states.get(&k).and_then(|s| s.position).unwrap_or(scene.nodes[k].position);
                stack.push((k, [at[0] + p[0], at[1] + p[1]]));
            }
        }
        acc.unwrap_or((0., 0., 0., 0.))
    };
    let cols = if columns == 0 { children.len().max(1) } else { columns };
    let mut col_w = vec![0f32; cols];
    let rows = children.len().div_ceil(cols).max(1);
    let mut row_h = vec![0f32; rows];
    for (i, c) in children.iter().enumerate() {
        let (x0, y0, x1, y1) = bounds(*c);
        col_w[i % cols] = col_w[i % cols].max(x1 - x0);
        row_h[i / cols] = row_h[i / cols].max(y1 - y0);
    }
    let mut out = Vec::with_capacity(children.len());
    let (mut x, mut y) = (0f32, 0f32);
    for (i, c) in children.iter().enumerate() {
        let (x0, y0, x1, y1) = bounds(*c);
        let (ex, ey) = ((x1 - x0) / 2., (y1 - y0) / 2.);
        let (cx, cy) = ((x0 + x1) / 2., (y0 + y1) / 2.);
        let px = x + ex - cx + pad[0];
        let py = -y - ey - cy - pad[1];
        out.push((*c, [px, py, 0.]));
        x += col_w[i % cols] + pad[0] * 2.;
        if (i + 1) % cols == 0 {
            x = 0.;
            y += row_h[i / cols] + pad[1] * 2.;
        }
    }
    out
}

/// `UIGrid.Reposition` (`ResetPosition`) of `parent`'s children: cells of
/// `cellWidth`×`cellHeight` along the arrangement, `maxPerLine` per line,
/// shifted by the grid pivot (`num3`/`num4` are the last line/cell indices
/// as in NGUI). Alphabetic sorting compares names; `hideInactive` skips
/// inactive children.
pub fn grid_positions(scene: &crate::unity_scene::Scene, states: &HashMap<usize, NodeState>, parent: usize) -> Vec<(usize, [f32; 3])> {
    let Some(g) = scene.script(parent, "UIGrid") else { return Vec::new() };
    let arrangement = g.int("arrangement").unwrap_or(0);
    let (cw, ch) = (g.float("cellWidth").unwrap_or(0.) as f32, g.float("cellHeight").unwrap_or(0.) as f32);
    let max_per_line = g.int("maxPerLine").unwrap_or(0).max(0);
    let hide_inactive = g.bool("hideInactive").unwrap_or(false);
    let mut children: Vec<usize> = scene.nodes[parent]
        .children
        .iter()
        .copied()
        .filter(|c| !hide_inactive || states.get(c).and_then(|s| s.active).unwrap_or(scene.nodes[*c].active))
        .collect();
    let mut sorting = g.int("sorting").unwrap_or(0);
    if g.bool("sorted").unwrap_or(false) && sorting == 0 {
        sorting = 1;
    }
    if sorting == 1 && arrangement != 2 {
        children.sort_by(|a, b| scene.nodes[*a].name.cmp(&scene.nodes[*b].name));
    }
    let mut out = Vec::with_capacity(children.len());
    let (mut x, mut y, mut max_x, mut max_y) = (0i64, 0i64, 0i64, 0i64);
    for c in &children {
        let p = states.get(c).and_then(|s| s.position).unwrap_or(scene.nodes[*c].position);
        let v = match arrangement {
            2 => [if cw > 0. { (p[0] / cw).round() * cw } else { p[0] }, if ch > 0. { (p[1] / ch).round() * ch } else { p[1] }, p[2]],
            0 => [cw * x as f32, -ch * y as f32, p[2]],
            _ => [cw * y as f32, -ch * x as f32, p[2]],
        };
        out.push((*c, v));
        max_x = max_x.max(x);
        max_y = max_y.max(y);
        x += 1;
        if max_per_line > 0 && x >= max_per_line {
            x = 0;
            y += 1;
        }
    }
    let pv = g.int("pivot").unwrap_or(0);
    if pv != 0 {
        let (px, py) = pivot(pv);
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let (dx, dy) = if arrangement == 0 {
            (lerp(0., max_x as f32 * cw, px), lerp(-(max_y as f32) * ch, 0., py))
        } else {
            (lerp(0., max_y as f32 * cw, px), lerp(-(max_x as f32) * ch, 0., py))
        };
        for (_, v) in &mut out {
            v[0] -= dx;
            v[1] -= dy;
        }
    }
    out
}
