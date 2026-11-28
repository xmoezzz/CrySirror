// SPDX-License-Identifier: MPL-2.0
//! Converts a finished Ren'Py Render tree into CrySirror retained layers.
//!
//! Each Render becomes a group layer at its blit offset whose
//! `local_matrix` is the parent's `reverse` matrix (Ren'Py applies a
//! render's reverse to each child after the child's offset). Textures
//! become image, solid, glyph-run or vector layers. Alpha and clipping are
//! inherited group properties; dissolve operations set child opacity and
//! image dissolves use the shared coverage mask. Draw order is the tree's
//! painter order.
//!
//! A clip over a single-child chain of plain blits (subsurfaces, Frame
//! tiles) crops the leaf image's own geometry instead, as upstream crops
//! texture coordinates: adjacent tiles then share rasterized edges, where
//! two half-open clips meeting at a pixel centre could both miss it.
use super::render::{self, Mat, RenderData, RenderRef, Source, TexKind, Texture};
use crysirror_python::{PyResult, Vm};
use serde_json::{Value as J, json};

/// One frame of layer operations.
pub struct Frame {
    pub layers: Vec<J>,
}

pub struct Builder<'a> {
    pub next_id: u32,
    pub order: i32,
    pub out: Vec<J>,
    pub assets: &'a dyn Fn(&str) -> Option<String>,
    pub errors: Vec<String>,
}

impl Builder<'_> {
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn put(&mut self, mut layer: J) {
        // Ren'Py renders may be empty (a Null, an unsized Fixed) yet carry
        // children; the host requires positive layer sizes. A group only
        // carries a transform and opacity, so its size is nominal; an empty
        // leaf draws nothing.
        let empty = |l: &J, i: usize| l["rect"][i].as_f64().is_none_or(|v| !(v > 0.));
        if empty(&layer, 2) || empty(&layer, 3) {
            if layer["paint"]["kind"] != "group" {
                return;
            }
            for i in [2, 3] {
                if empty(&layer, i) {
                    layer["rect"][i] = json!(1.0e-3f32);
                }
            }
        }
        self.order += 1;
        layer["order"] = json!([0, self.order]);
        self.out.push(json!({"op": "put", "layer": layer}));
    }

    /// Emits `r` placed at (x, y) inside `parent`, with the parent's reverse
    /// matrix applied to this child.
    pub fn render(
        &mut self,
        r: &RenderData,
        parent: Option<u32>,
        x: f64,
        y: f64,
        matrix: Option<&Mat>,
        opacity: f64,
        mut crop: Option<[f64; 4]>,
    ) {
        let id = self.id();
        let alpha = (r.alpha * opacity).clamp(0., 1.);
        let mut layer = json!({
            "id": id,
            "rect": [x as f32, y as f32, r.width.max(0.) as f32, r.height.max(0.) as f32],
            "paint": {"kind": "group"},
            "opacity": (alpha * 255.).round() as u8,
        });
        if let Some(p) = parent {
            layer["parent"] = json!(p);
        }
        if let Some(m) = matrix.filter(|m| !m.is_identity()) {
            layer["transform"] = json!({"local_matrix": m.column_major()});
        }
        if r.xclipping || r.yclipping {
            // A clip spans at most 16384 units: the unclipped axis opens
            // that far, centred on the render.
            const SPAN: f64 = 16384.;
            let axis = |clip: bool, size: f64| {
                if clip {
                    (0., size.clamp(1.0e-3, SPAN))
                } else {
                    (-(SPAN - size.clamp(0., SPAN)) / 2., SPAN)
                }
            };
            let (cx, cw) = axis(r.xclipping, r.width);
            let (cy, ch) = axis(r.yclipping, r.height);
            crop = intersect(crop, [cx, cy, cw, ch]);
        }
        let pass_crop = crop.is_some()
            && r.children.len() == 1
            && r.operation == render::BLIT
            && r.reverse.as_ref().is_none_or(axis_aligned);
        if let Some(c) = crop.filter(|_| !pass_crop) {
            layer["clip"] = json!([c[0] as f32, c[1] as f32, c[2].max(1.0e-3) as f32, c[3].max(1.0e-3) as f32]);
        }
        if r.over < 0.5 {
            layer["blend"] = json!("add");
        }
        self.put(layer);
        let reverse = r.reverse.as_ref();
        let n = r.children.len();
        for (i, c) in r.children.iter().enumerate() {
            let child_opacity = match r.operation {
                render::DISSOLVE if n >= 2 => {
                    if i == n - 1 {
                        r.operation_complete.clamp(0., 1.)
                    } else {
                        1.
                    }
                }
                render::IMAGEDISSOLVE if n >= 3 => {
                    if i == 0 {
                        // The mask itself is not drawn.
                        continue;
                    }
                    if i == n - 1 {
                        r.operation_complete.clamp(0., 1.)
                    } else {
                        1.
                    }
                }
                _ => 1.,
            };
            if r.operation == render::IMAGEDISSOLVE && n >= 3 && i == n - 1 {
                if let Some(mask) = self.mask_asset(&r.children[0].source) {
                    let inverted = r
                        .properties
                        .obj()
                        .and_then(|d| d.dict_get_str_raw("imagedissolve_reverse"))
                        .is_some_and(|v| matches!(v, crysirror_python::Value::Bool(true)));
                    self.masked(
                        c,
                        Some(id),
                        reverse,
                        child_opacity,
                        &mask,
                        r.operation_complete,
                        r.operation_parameter,
                        inverted,
                    );
                    continue;
                }
            }
            let child_crop = crop.filter(|_| pass_crop).map(|k| crop_into_child(k, c, reverse));
            self.child(c, Some(id), reverse, child_opacity, child_crop);
        }
    }

    fn child(&mut self, c: &render::Blit, parent: Option<u32>, matrix: Option<&Mat>, opacity: f64, crop: Option<[f64; 4]>) {
        match &c.source {
            Source::Render(cr, _) => {
                let cr = cr.borrow();
                self.render(&cr, parent, c.x, c.y, matrix, opacity, crop);
            }
            Source::Texture(t, _) => self.texture(t, parent, c.x, c.y, matrix, opacity, crop),
        }
    }

    fn mask_asset(&self, s: &Source) -> Option<(String, [f64; 2])> {
        fn find(s: &Source) -> Option<Texture> {
            match s {
                Source::Texture(t, _) => Some((**t).clone()),
                Source::Render(r, _) => {
                    let r = r.borrow();
                    if r.children.len() == 1 && r.reverse.is_none() {
                        find(&r.children[0].source)
                    } else {
                        None
                    }
                }
            }
        }
        let t = find(s)?;
        let asset = match &t.kind {
            TexKind::Asset(a) => (self.assets)(a)?,
            TexKind::Published(p) => p.clone(),
            _ => return None,
        };
        Some((asset, [t.width, t.height]))
    }

    #[allow(clippy::too_many_arguments)]
    fn masked(
        &mut self,
        c: &render::Blit,
        parent: Option<u32>,
        matrix: Option<&Mat>,
        _opacity: f64,
        mask: &(String, [f64; 2]),
        complete: f64,
        ramp: f64,
        inverted: bool,
    ) {
        let start = self.out.len();
        self.child(c, parent, matrix, 1., None);
        // Upstream: the ramp width over the 0-255 mask range.
        let feather = (ramp / 256.).clamp(0., 1.) as f32;
        for op in &mut self.out[start..] {
            if op["layer"]["parent"] == json!(parent) {
                op["layer"]["filter"] = json!({
                    "mask": {
                        "asset": mask.0,
                        "channel": "red",
                        "scale": if inverted { -1.0 } else { 1.0 },
                        "offset": if inverted { 255.0 } else { 0.0 },
                        "coverage": {"progress": complete.clamp(0., 1.) as f32, "feather": feather, "canvas": [mask.1[0] as u32, mask.1[1] as u32]},
                    }
                });
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn texture(
        &mut self,
        t: &Texture,
        parent: Option<u32>,
        x: f64,
        y: f64,
        matrix: Option<&Mat>,
        opacity: f64,
        crop: Option<[f64; 4]>,
    ) {
        let crop = match crop {
            Some(k) => match intersect(Some(k), [0., 0., t.width, t.height]) {
                Some(k) if k[2] > 0. && k[3] > 0. => Some(k),
                _ => return,
            },
            None => None,
        };
        let id = self.id();
        let paint = match &t.kind {
            TexKind::Asset(name) => match (self.assets)(name) {
                Some(a) => json!({"kind": "image", "asset": a}),
                None => {
                    self.errors.push(format!("image asset not found: {name}"));
                    return;
                }
            },
            TexKind::Published(path) => json!({"kind": "image", "asset": path}),
            TexKind::Solid(c) => json!({"kind": "solid", "color": c}),
            TexKind::Glyphs(run) => (**run).clone(),
            TexKind::Video(channel) => json!({"kind": "renpy_video", "channel": channel}),
            TexKind::Canvas(ops) => {
                if ops.borrow().is_empty() {
                    return;
                }
                match super::image::canvas_asset(t) {
                    Some(a) => json!({"kind": "image", "asset": a}),
                    None => {
                        self.errors
                            .push("Render.canvas() drawing has no published vector image".into());
                        return;
                    }
                }
            }
        };
        let mut layer = json!({
            "id": id,
            "rect": [x as f32, y as f32, t.width as f32, t.height as f32],
            "paint": paint,
            "opacity": (opacity.clamp(0., 1.) * 255.).round() as u8,
        });
        if let Some(p) = parent {
            layer["parent"] = json!(p);
        }
        if let Some(m) = matrix.filter(|m| !m.is_identity()) {
            layer["transform"] = json!({"local_matrix": m.column_major()});
        }
        if let (Some(crop), Some(full)) = (t.crop, full_size(t)) {
            layer["uv"] = json!([
                (crop[0] / full[0]) as f32,
                (crop[1] / full[1]) as f32,
                ((crop[0] + crop[2]) / full[0]) as f32,
                ((crop[1] + crop[3]) / full[1]) as f32
            ]);
        }
        if let Some(k) = crop.filter(|k| *k != [0., 0., t.width, t.height]) {
            layer["source_crop"] = json!([k[0] as f32, k[1] as f32, k[2] as f32, k[3] as f32]);
        }
        self.put(layer);
    }
}

fn intersect(a: Option<[f64; 4]>, b: [f64; 4]) -> Option<[f64; 4]> {
    let Some(a) = a else { return Some(b) };
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let r = (a[0] + a[2]).min(b[0] + b[2]);
    let d = (a[1] + a[3]).min(b[1] + b[3]);
    Some([x, y, (r - x).max(0.), (d - y).max(0.)])
}

/// A scale/translate matrix: rectangles stay rectangles.
fn axis_aligned(m: &Mat) -> bool {
    m.is_2d_affine() && m.0[1] == 0. && m.0[4] == 0. && m.0[0] != 0. && m.0[5] != 0.
}

/// A crop in a render's local space, in the local space of its child at
/// (c.x, c.y) under `matrix` (the scene places the child at
/// `offset * matrix * q`).
fn crop_into_child(k: [f64; 4], c: &render::Blit, matrix: Option<&Mat>) -> [f64; 4] {
    let inv = matrix.map(|m| m.inverse());
    let map = |x: f64, y: f64| match &inv {
        Some(i) => i.transform2(x - c.x, y - c.y),
        None => (x - c.x, y - c.y),
    };
    let (x0, y0) = map(k[0], k[1]);
    let (x1, y1) = map(k[0] + k[2], k[1] + k[3]);
    [x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()]
}

fn full_size(t: &Texture) -> Option<[f64; 2]> {
    t.full.filter(|f| f[0] > 0. && f[1] > 0.)
}

/// Builds the layer list for the screen render.
/// The layer operations of a frame. `fit` places the virtual screen in
/// the host canvas (a scale on the root layer).
pub fn build(
    root: &RenderRef,
    first_id: u32,
    assets: &dyn Fn(&str) -> Option<String>,
    fit: super::events::Fit,
) -> (Vec<J>, u32, Vec<String>) {
    let mut b = Builder {
        next_id: first_id,
        order: 0,
        out: Vec::new(),
        assets,
        errors: Vec::new(),
    };
    let r = root.borrow();
    let m = (fit != super::events::Fit::IDENTITY).then(|| Mat::scale(fit.scale, fit.scale, 1.));
    b.render(&r, None, fit.x, fit.y, m.as_ref(), 1., None);
    // The window shows the virtual screen only (upstream draws into the
    // letterboxed viewport), so the root clips to it.
    if let Some(layer) = b.out.first_mut().and_then(|op| op.get_mut("layer")) {
        layer["clip"] = json!([0f32, 0f32, r.width.max(1.0e-3) as f32, r.height.max(1.0e-3) as f32]);
    }
    (b.out, b.next_id, b.errors)
}

/// The screen rectangle of the last text-input render drawn (gldraw sets
/// `interface.text_rect` to `rend.screen_rect(...)` while drawing).
pub fn text_input_rect(root: &RenderData) -> Option<[f64; 4]> {
    fn walk(r: &RenderData, m: &Mat, out: &mut Option<[f64; 4]>) {
        if r.text_input {
            let pts = [(0., 0.), (r.width, 0.), (0., r.height), (r.width, r.height)].map(|(x, y)| m.transform2(x, y));
            let x0 = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            let y0 = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let x1 = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max);
            let y1 = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max);
            *out = Some([x0, y0, x1 - x0, y1 - y0]);
        }
        for c in &r.children {
            if let Source::Render(cr, _) = &c.source {
                let cm = m.mul(&Mat::offset(c.x, c.y, 0.));
                let cm = match &r.reverse {
                    Some(rev) => cm.mul(rev),
                    None => cm,
                };
                walk(&cr.borrow(), &cm, out);
            }
        }
    }
    let mut out = None;
    walk(root, &Mat::IDENTITY, &mut out);
    out
}

/// Whether the render paints a non-transparent pixel at (x, y). Image
/// alpha needs decoded pixels from the shared image service; leaves that
/// cannot be resolved count as opaque inside their bounds.
pub fn pixel_opaque(vm: &mut Vm, r: &RenderRef, x: f64, y: f64) -> PyResult<bool> {
    fn walk(vm: &mut Vm, r: &RenderData, x: f64, y: f64) -> bool {
        if r.alpha <= 0. {
            return false;
        }
        if (r.xclipping && (x < 0. || x >= r.width)) || (r.yclipping && (y < 0. || y >= r.height)) {
            return false;
        }
        for c in r.children.iter().rev() {
            let (mut cx, mut cy) = (x - c.x, y - c.y);
            if let Some(f) = &r.forward {
                (cx, cy) = f.transform2(cx, cy);
            }
            match &c.source {
                Source::Render(cr, _) => {
                    if walk(vm, &cr.borrow(), cx, cy) {
                        return true;
                    }
                }
                Source::Texture(t, _) => {
                    if cx < 0. || cy < 0. || cx >= t.width || cy >= t.height {
                        continue;
                    }
                    match &t.kind {
                        TexKind::Solid(c) => {
                            if c[3] > 0 {
                                return true;
                            }
                        }
                        TexKind::Asset(_) | TexKind::Published(_) => {
                            let (ox, oy) = t.crop.map_or((0., 0.), |c| (c[0], c[1]));
                            if super::image::texture_alpha(vm, t, cx + ox, cy + oy).unwrap_or(255)
                                > 0
                            {
                                return true;
                            }
                        }
                        TexKind::Glyphs(_) | TexKind::Canvas(_) | TexKind::Video(_) => return true,
                    }
                }
            }
        }
        false
    }
    let r = r.borrow();
    Ok(walk(vm, &r, x, y))
}
