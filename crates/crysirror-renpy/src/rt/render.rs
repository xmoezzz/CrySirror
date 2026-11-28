// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.render` and `renpy.display.matrix`.
//!
//! A `Render` is a retained tree node: children are blitted Renders or
//! textures at offsets, with an optional reverse matrix (child to parent),
//! alpha, clipping and a transition operation. The backend
//! (`rt::backend`) converts the finished tree into CrySirror layers; nothing
//! is rasterized here. The render cache, redraw queue and focus collection
//! follow upstream `render.pyx` (Ren'Py 8.3, MIT).
use super::py::{self, *};
use crysirror_python::{ObjRef, Payload, PyResult, Value, Vm, builtins::Kw, object::BaseKind, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub const BLIT: i64 = 0;
pub const DISSOLVE: i64 = 1;
pub const IMAGEDISSOLVE: i64 = 2;
pub const PIXELLATE: i64 = 3;
pub const FLATTEN: i64 = 4;

/// Row-major 4x4 matrix, `renpy.display.matrix.Matrix`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat(pub [f64; 16]);

impl Mat {
    pub const IDENTITY: Mat = Mat([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]);
    pub fn mul(&self, o: &Mat) -> Mat {
        let mut r = [0.; 16];
        for i in 0..4 {
            for j in 0..4 {
                r[i * 4 + j] = (0..4).map(|k| self.0[i * 4 + k] * o.0[k * 4 + j]).sum();
            }
        }
        Mat(r)
    }
    pub fn offset(x: f64, y: f64, z: f64) -> Mat {
        let mut m = Self::IDENTITY;
        m.0[3] = x;
        m.0[7] = y;
        m.0[11] = z;
        m
    }
    pub fn scale(x: f64, y: f64, z: f64) -> Mat {
        let mut m = Self::IDENTITY;
        m.0[0] = x;
        m.0[5] = y;
        m.0[10] = z;
        m
    }
    /// Euler rotation in degrees, x then y then z (upstream `Matrix.rotate`).
    pub fn rotate(x: f64, y: f64, z: f64) -> Mat {
        let (sx, cx) = x.to_radians().sin_cos();
        let (sy, cy) = y.to_radians().sin_cos();
        let (sz, cz) = z.to_radians().sin_cos();
        let rx = Mat([
            1., 0., 0., 0., 0., cx, -sx, 0., 0., sx, cx, 0., 0., 0., 0., 1.,
        ]);
        let ry = Mat([
            cy, 0., sy, 0., 0., 1., 0., 0., -sy, 0., cy, 0., 0., 0., 0., 1.,
        ]);
        let rz = Mat([
            cz, -sz, 0., 0., sz, cz, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]);
        rz.mul(&ry).mul(&rx)
    }
    pub fn perspective(w: f64, h: f64, n: f64, p: f64, f: f64) -> Mat {
        let offset = Mat::offset(-w / 2., -h / 2., -p);
        let proj = Mat([
            2. * p / w,
            0.,
            0.,
            0.,
            0.,
            2. * p / h,
            0.,
            0.,
            0.,
            0.,
            -(f + n) / (f - n),
            -2. * f * n / (f - n),
            0.,
            0.,
            -1.,
            0.,
        ]);
        proj.mul(&offset)
    }
    pub fn transform(&self, x: f64, y: f64, z: f64, w: f64) -> [f64; 4] {
        let m = &self.0;
        [
            m[0] * x + m[1] * y + m[2] * z + m[3] * w,
            m[4] * x + m[5] * y + m[6] * z + m[7] * w,
            m[8] * x + m[9] * y + m[10] * z + m[11] * w,
            m[12] * x + m[13] * y + m[14] * z + m[15] * w,
        ]
    }
    pub fn transform2(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, _, w] = self.transform(x, y, 0., 1.);
        if w != 0. && w != 1. {
            (a / w, b / w)
        } else {
            (a, b)
        }
    }
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
    pub fn is_2d_affine(&self) -> bool {
        let m = &self.0;
        m[2] == 0.
            && m[6] == 0.
            && m[8] == 0.
            && m[9] == 0.
            && m[12] == 0.
            && m[13] == 0.
            && m[14] == 0.
            && m[15] == 1.
    }
    pub fn inverse(&self) -> Mat {
        let m = &self.0;
        let mut inv = [0.; 16];
        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
            + m[9] * m[7] * m[14]
            + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
            - m[8] * m[7] * m[14]
            - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
            + m[8] * m[7] * m[13]
            + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
            - m[8] * m[6] * m[13]
            - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
            - m[9] * m[3] * m[14]
            - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
            + m[8] * m[3] * m[14]
            + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
            - m[8] * m[3] * m[13]
            - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
            + m[8] * m[2] * m[13]
            + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
            + m[5] * m[3] * m[14]
            + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
            - m[4] * m[3] * m[14]
            - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
            + m[4] * m[3] * m[13]
            + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
            - m[4] * m[2] * m[13]
            - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
            - m[5] * m[3] * m[10]
            - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
            + m[4] * m[3] * m[10]
            + m[8] * m[2] * m[7]
            - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
            - m[4] * m[3] * m[9]
            - m[8] * m[1] * m[7]
            + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
            + m[4] * m[2] * m[9]
            + m[8] * m[1] * m[6]
            - m[8] * m[2] * m[5];
        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if det == 0. {
            return Mat([0.; 16]);
        }
        Mat(inv.map(|v| v / det))
    }
    /// Column-major form used by `SpriteTransform::local_matrix`.
    pub fn column_major(&self) -> [f32; 16] {
        let m = &self.0;
        let mut c = [0f32; 16];
        for i in 0..4 {
            for j in 0..4 {
                c[j * 4 + i] = m[i * 4 + j] as f32;
            }
        }
        c
    }
}

/// What a texture leaf paints.
#[derive(Clone, Debug)]
pub enum TexKind {
    /// Image file in the game's asset namespace.
    Asset(String),
    /// Published derived image (`im.*`) path.
    Published(String),
    Solid([u8; 4]),
    /// Text: serialized `glyph_run` paint (already positioned glyphs).
    Glyphs(Rc<serde_json::Value>),
    /// Vector canvas commands (`Render.canvas()`).
    Canvas(Rc<RefCell<Vec<serde_json::Value>>>),
    /// The current frame of the movie playing on an audio channel.
    Video(String),
}

#[derive(Clone, Debug)]
pub struct Texture {
    pub kind: TexKind,
    pub width: f64,
    pub height: f64,
    /// Source crop [x, y, w, h] in texture pixels (subsurface of a texture).
    pub crop: Option<[f64; 4]>,
    /// Full source size when `crop` addresses part of an image.
    pub full: Option<[f64; 2]>,
    pub opaque: bool,
}

pub type RenderRef = Rc<RefCell<RenderData>>;

#[derive(Clone)]
pub enum Source {
    Render(RenderRef, Value),
    Texture(Rc<Texture>, Value),
}

impl Source {
    pub fn size(&self) -> (f64, f64) {
        match self {
            Source::Render(r, _) => {
                let r = r.borrow();
                (r.width, r.height)
            }
            Source::Texture(t, _) => (t.width, t.height),
        }
    }
    pub fn value(&self) -> &Value {
        match self {
            Source::Render(_, v) | Source::Texture(_, v) => v,
        }
    }
}

#[derive(Clone)]
pub struct Blit {
    pub source: Source,
    pub x: f64,
    pub y: f64,
    pub focus: bool,
    pub main: bool,
}

#[derive(Clone)]
pub struct Focus {
    pub d: Value,
    pub arg: Value,
    /// None: full-screen focus.
    pub rect: Option<[f64; 4]>,
    pub mask: Option<(f64, f64, Value)>,
}

pub struct RenderData {
    pub width: f64,
    pub height: f64,
    pub layer_name: Value,
    pub children: Vec<Blit>,
    pub forward: Option<Mat>,
    pub reverse: Option<Mat>,
    pub alpha: f64,
    pub over: f64,
    pub nearest: Value,
    pub xclipping: bool,
    pub yclipping: bool,
    pub modal: Value,
    pub text_input: bool,
    pub operation: i64,
    pub operation_complete: f64,
    pub operation_alpha: bool,
    pub operation_parameter: f64,
    pub focuses: Vec<Focus>,
    pub pass_focuses: Vec<RenderRef>,
    pub focus_screen: Value,
    pub render_of: Vec<Value>,
    pub mesh: Value,
    pub shaders: Value,
    pub uniforms: Value,
    pub properties: Value,
    /// Alpha mask applied to this render's children (`AlphaMask`) and
    /// whether it is inverted.
    pub mask: Option<(Source, bool)>,
    /// Python-visible extra attributes (`rv.something = ...`).
    pub extra: HashMap<String, Value>,
}

impl RenderData {
    pub fn new(width: f64, height: f64) -> Self {
        RenderData {
            width,
            height,
            layer_name: Value::None,
            children: Vec::new(),
            forward: None,
            reverse: None,
            alpha: 1.,
            over: 1.,
            nearest: Value::None,
            xclipping: false,
            yclipping: false,
            modal: Value::Bool(false),
            text_input: false,
            operation: BLIT,
            operation_complete: 0.,
            operation_alpha: false,
            operation_parameter: 0.,
            focuses: Vec::new(),
            pass_focuses: Vec::new(),
            focus_screen: Value::None,
            render_of: Vec::new(),
            mesh: Value::None,
            shaders: Value::None,
            uniforms: Value::None,
            properties: Value::None,
            mask: None,
            extra: HashMap::new(),
        }
    }
}

/// Classes and global render state, kept in the VM extension map.
pub struct RenderState {
    pub render_class: ObjRef,
    pub texture_class: ObjRef,
    pub matrix_class: ObjRef,
    pub canvas_class: ObjRef,
    /// id(d) -> [((w, h, st-offset, at-offset), render)]
    pub cache: HashMap<i64, Vec<([u64; 4], Value)>>,
    /// Keeps cached displayables alive while their id is a cache key.
    pub cache_keep: HashMap<i64, Value>,
    pub frame_time: f64,
    pub rendering: u32,
    pub sizing: bool,
    pub render_ready: bool,
    /// Earliest requested redraw (absolute seconds) and its displayables.
    pub redraws: Vec<(Value, f64)>,
    pub invalidated: bool,
    /// The most recent screen render (focus and hit testing).
    pub screen_render: Option<Value>,
    /// st/at of the displayable currently rendering (`Render.place`).
    pub render_st: f64,
    pub render_at: f64,
    /// `render_width`/`render_height`: the size offered to the displayable being rendered.
    pub render_size: (f64, f64),
}

pub fn rs(vm: &Vm) -> Rc<RefCell<RenderState>> {
    vm.ext_get::<Rc<RefCell<RenderState>>>()
        .expect("render state")
        .clone()
}

pub fn render_data(v: &Value) -> Option<RenderRef> {
    py::native::<RenderRef, _>(v, |r| r.clone())
}

pub fn texture_data(v: &Value) -> Option<Rc<Texture>> {
    py::native::<Rc<Texture>, _>(v, |t| t.clone())
}

pub fn matrix_data(v: &Value) -> Option<Mat> {
    py::native::<Mat, _>(v, |m| *m)
}

pub fn new_render(vm: &Vm, data: RenderData) -> Value {
    let class = rs(vm).borrow_mut().render_class.clone();
    vm.types
        .new_native(&class, Box::new(Rc::new(RefCell::new(data))))
}

pub fn new_texture(vm: &Vm, t: Texture) -> Value {
    let class = rs(vm).borrow_mut().texture_class.clone();
    vm.types.new_native(&class, Box::new(Rc::new(t)))
}

pub fn new_matrix(vm: &Vm, m: Mat) -> Value {
    let class = rs(vm).borrow_mut().matrix_class.clone();
    vm.types.new_native(&class, Box::new(m))
}

fn this(vm: &Vm, args: &[Value]) -> PyResult<RenderRef> {
    args.first()
        .and_then(render_data)
        .ok_or_else(|| vm.type_error("Render method on non-Render"))
}

/// Converts a blit source value.
pub fn source_of(vm: &mut Vm, v: &Value) -> PyResult<Source> {
    if let Some(r) = render_data(v) {
        return Ok(Source::Render(r, v.clone()));
    }
    if let Some(t) = texture_data(v) {
        return Ok(Source::Texture(t, v.clone()));
    }
    // Surfaces from renpy.display.pgrender / im.* load as textures.
    if let Some(t) = ops::getattr_opt(vm, v, "_crysirror_texture")? {
        if let Some(tex) = texture_data(&t) {
            return Ok(Source::Texture(tex, t));
        }
    }
    Err(vm.type_error(format!(
        "cannot blit {}",
        v.obj().map(|o| o.class_name()).unwrap_or_default()
    )))
}

fn blit_impl(vm: &mut Vm, args: &[Value], kw: Kw, mode: u8) -> PyResult<Value> {
    let r = this(vm, args)?;
    let source = arg(args, kw, 1, "source")
        .cloned()
        .ok_or_else(|| vm.type_error("blit() missing source"))?;
    if py::same(&source, &args[0]) {
        return Err(py::error(vm, "Blitting to self."));
    }
    let pos = arg(args, kw, 2, "pos")
        .cloned()
        .ok_or_else(|| vm.type_error("blit() missing pos"))?;
    let focus = match arg(args, kw, 3, "focus") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let main = match arg(args, kw, 4, "main") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let index = match arg(args, kw, 5, "index") {
        Some(Value::None) | None => None,
        Some(v) => Some(py::int(vm, v)?),
    };
    let p = py::items(vm, &pos)?;
    if p.len() != 2 {
        return Err(vm.type_error("blit pos must be (x, y)"));
    }
    let (mut x, mut y) = (py::f64_of(vm, &p[0])?, py::f64_of(vm, &p[1])?);
    match mode {
        0 => {
            x = x.trunc();
            y = y.trunc();
        }
        2 => {
            x = absolute(x);
            y = absolute(y);
        }
        _ => {}
    }
    let source = source_of(vm, &source)?;
    let b = Blit {
        source,
        x,
        y,
        focus,
        main,
    };
    let mut d = r.borrow_mut();
    match index {
        Some(i) => {
            let n = d.children.len() as i64;
            let i = if i < 0 { (n + i).max(0) } else { i.min(n) } as usize;
            d.children.insert(i, b);
        }
        None => d.children.push(b),
    }
    Ok(Value::Int(0))
}

/// `renpy.display.core.absolute` rounding: values are kept as floats.
fn absolute(v: f64) -> f64 {
    v
}

fn r_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let _ = (vm, args, kw);
    Ok(Value::None)
}

fn r_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    // args[0] is the class.
    let w = arg(&args[1..], kw, 0, "width")
        .cloned()
        .unwrap_or(Value::Int(0));
    let h = arg(&args[1..], kw, 1, "height")
        .cloned()
        .unwrap_or(Value::Int(0));
    let mut d = RenderData::new(py::f64_of(vm, &w)?, py::f64_of(vm, &h)?);
    if let Some(l) = arg(&args[1..], kw, 2, "layer_name") {
        d.layer_name = l.clone();
    }
    let cls = args[0]
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("Render.__new__"))?;
    Ok(vm
        .types
        .new_native(&cls, Box::new(Rc::new(RefCell::new(d)))))
}

fn r_blit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    blit_impl(vm, args, kw, 0)
}
fn r_subpixel_blit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    blit_impl(vm, args, kw, 1)
}
fn r_absolute_blit(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    blit_impl(vm, args, kw, 2)
}

fn r_get_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let r = r.borrow();
    Ok(vm
        .types
        .new_tuple(vec![Value::Float(r.width), Value::Float(r.height)]))
}

/// `subsurface(rect, focus=False, subpixel=False, bounds=None)`: a clipped
/// view of this render. Upstream crops child geometry; CrySirror layers clip
/// the retained child instead, which is equivalent for drawing and focus.
pub fn subsurface(
    vm: &mut Vm,
    this_v: &Value,
    rect: [f64; 4],
    focus: bool,
    subpixel: bool,
) -> PyResult<Value> {
    let r = render_data(this_v).ok_or_else(|| vm.type_error("subsurface on non-Render"))?;
    let [mut x, mut y, mut w, mut h] = rect;
    if !subpixel {
        x = x.trunc();
        y = y.trunc();
        w = w.trunc();
        h = h.trunc();
    }
    let mut d = RenderData::new(w, h);
    d.xclipping = true;
    d.yclipping = true;
    {
        let src = r.borrow();
        // Drop clipping on an axis that the crop does not reduce.
        if src.reverse.is_none()
            || src
                .reverse
                .is_some_and(|m| m.0[0] > 0. && m.0[1] == 0. && m.0[4] == 0. && m.0[5] > 0.)
        {
            let fwd = src.forward.unwrap_or(Mat::IDENTITY);
            let (tx, ty) = fwd.transform2(x, y);
            let (tw, th) = fwd.transform2(w + x, h + y);
            let (rw, rh) = fwd.transform2(src.width, src.height);
            if tx <= 0. && tw >= rw {
                d.xclipping = false;
            }
            if ty <= 0. && th >= rh {
                d.yclipping = false;
            }
        }
        d.render_of = src.render_of.clone();
    }
    d.children.push(Blit {
        source: Source::Render(r, this_v.clone()),
        x: -x,
        y: -y,
        focus,
        main: true,
    });
    Ok(new_render(vm, d))
}

fn r_subsurface(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = arg(args, kw, 1, "rect")
        .cloned()
        .ok_or_else(|| vm.type_error("subsurface() missing rect"))?;
    let focus = match arg(args, kw, 2, "focus") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let subpixel = match arg(args, kw, 3, "subpixel") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let rect = rect4(vm, &rect)?;
    subsurface(vm, &args[0], rect, focus, subpixel)
}

pub fn rect4(vm: &mut Vm, v: &Value) -> PyResult<[f64; 4]> {
    let p = py::items(vm, v)?;
    if p.len() != 4 {
        return Err(vm.type_error("expected (x, y, w, h)"));
    }
    Ok([
        py::f64_of(vm, &p[0])?,
        py::f64_of(vm, &p[1])?,
        py::f64_of(vm, &p[2])?,
        py::f64_of(vm, &p[3])?,
    ])
}

fn r_depends_on(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let source = arg(args, kw, 1, "source").cloned().unwrap_or(Value::None);
    if py::same(&source, &args[0]) {
        return Err(py::error(vm, "Render depends on itself."));
    }
    let focus = match arg(args, kw, 2, "focus") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    if focus {
        if let Some(s) = render_data(&source) {
            r.borrow_mut().pass_focuses.push(s);
        }
    }
    Ok(Value::None)
}

fn r_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn r_add_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let a = |i: usize, n: &str| arg(args, kw, i, n).cloned().unwrap_or(Value::None);
    let d = a(1, "d");
    let farg = a(2, "arg");
    let (x, y, w, h) = (a(3, "x"), a(4, "y"), a(5, "w"), a(6, "h"));
    let (mx, my, mask) = (a(7, "mx"), a(8, "my"), a(9, "mask"));
    let rect = if py::is_none(&x) {
        None
    } else {
        let (rw, rh) = {
            let r = r.borrow();
            (r.width, r.height)
        };
        let x = py::f64_of(vm, &x)?;
        let y = if py::is_none(&y) {
            0.
        } else {
            py::f64_of(vm, &y)?
        };
        let w = if py::is_none(&w) {
            rw
        } else {
            py::f64_of(vm, &w)?
        };
        let h = if py::is_none(&h) {
            rh
        } else {
            py::f64_of(vm, &h)?
        };
        Some([x, y, w, h])
    };
    let mask = if py::is_none(&mask) {
        None
    } else {
        let mx = if py::is_none(&mx) {
            0.
        } else {
            py::f64_of(vm, &mx)?
        };
        let my = if py::is_none(&my) {
            0.
        } else {
            py::f64_of(vm, &my)?
        };
        Some((mx, my, mask))
    };
    r.borrow_mut().focuses.push(Focus {
        d,
        arg: farg,
        rect,
        mask,
    });
    Ok(Value::None)
}

fn r_fill(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let color = arg(args, kw, 1, "color").cloned().unwrap_or(Value::None);
    let rgba = super::color::rgba(vm, &color)?;
    let (w, h) = {
        let r = r.borrow();
        (r.width, r.height)
    };
    let t = new_texture(
        vm,
        Texture {
            kind: TexKind::Solid(rgba),
            width: w,
            height: h,
            crop: None,
            full: None,
            opaque: rgba[3] == 255,
        },
    );
    let source = source_of(vm, &t)?;
    r.borrow_mut().children.push(Blit {
        source,
        x: 0.,
        y: 0.,
        focus: false,
        main: true,
    });
    Ok(Value::None)
}

fn r_canvas(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let (w, h) = {
        let r = r.borrow();
        (r.width, r.height)
    };
    let ops = Rc::new(RefCell::new(Vec::new()));
    let t = new_texture(
        vm,
        Texture {
            kind: TexKind::Canvas(ops.clone()),
            width: w,
            height: h,
            crop: None,
            full: None,
            opaque: false,
        },
    );
    let source = source_of(vm, &t)?;
    r.borrow_mut().children.push(Blit {
        source,
        x: 0.,
        y: 0.,
        focus: false,
        main: true,
    });
    let class = rs(vm).borrow_mut().canvas_class.clone();
    Ok(vm.types.new_native(&class, Box::new(ops)))
}

fn r_place(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = |i: usize, n: &str| arg(args, kw, i, n).cloned().unwrap_or(Value::None);
    let rv = args[0].clone();
    let d = a(1, "d");
    let x = if py::is_none(&a(2, "x")) {
        0.
    } else {
        py::f64_of(vm, &a(2, "x"))?
    };
    let y = if py::is_none(&a(3, "y")) {
        0.
    } else {
        py::f64_of(vm, &a(3, "y"))?
    };
    let (rw, rh) = {
        let r = this(vm, args)?;
        let r = r.borrow();
        (r.width, r.height)
    };
    let width = if py::is_none(&a(4, "width")) {
        rw
    } else {
        py::f64_of(vm, &a(4, "width"))?
    };
    let height = if py::is_none(&a(5, "height")) {
        rh
    } else {
        py::f64_of(vm, &a(5, "height"))?
    };
    let (st, at) = (a(6, "st"), a(7, "at"));
    let (rst, rat) = {
        let s_rc = rs(vm);
        let s = s_rc.borrow_mut();
        (s.render_st, s.render_at)
    };
    let st = if py::is_none(&st) {
        rst
    } else {
        py::f64_of(vm, &st)?
    };
    let at = if py::is_none(&at) {
        rat
    } else {
        py::f64_of(vm, &at)?
    };
    let child = a(8, "render");
    let child = if py::is_none(&child) {
        render(vm, &d, width, height, st, at)?
    } else {
        child
    };
    let main = match arg(args, kw, 9, "main") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let (cw, ch) = source_of(vm, &child)?.size();
    let placement = vm.call_method(&d, "get_placement", Vec::new())?;
    let (px, py_) = super::core::place(vm, &placement, width, height, cw, ch)?;
    let pos = vm
        .types
        .new_tuple(vec![Value::Float(x + px), Value::Float(y + py_)]);
    let main_v = Value::Bool(main);
    blit_impl(vm, &[rv, child, pos, Value::Bool(true), main_v], &[], 1)?;
    Ok(vm
        .types
        .new_tuple(vec![Value::Float(x + px), Value::Float(y + py_)]))
}

fn r_zoom(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let xz = py::f64_of(vm, &arg_or(args, kw, 1, "xzoom", Value::Float(1.)))?;
    let yz = py::f64_of(vm, &arg_or(args, kw, 2, "yzoom", Value::Float(1.)))?;
    let mut d = r.borrow_mut();
    d.width *= xz;
    d.height *= yz;
    let rev = Mat::scale(xz, yz, 1.);
    let fwd = Mat::scale(1. / xz, 1. / yz, 1.);
    d.reverse = Some(match d.reverse {
        Some(m) => rev.mul(&m),
        None => rev,
    });
    d.forward = Some(match d.forward {
        Some(m) => m.mul(&fwd),
        None => fwd,
    });
    Ok(Value::None)
}

fn r_add_shader(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let shader = arg(args, kw, 1, "shader").cloned().unwrap_or(Value::None);
    let mut d = r.borrow_mut();
    let mut list = match py::seq(&d.shaders) {
        Some(v) => v,
        None => Vec::new(),
    };
    if let Some(name) = shader.as_str() {
        if let Some(stripped) = name.strip_prefix('-') {
            list.retain(|s| s.as_str() != Some(stripped));
        } else if !list.iter().any(|s| s.as_str() == Some(name)) {
            list.push(shader.clone());
        }
    }
    d.shaders = vm.types.new_tuple(list);
    Ok(Value::None)
}

fn r_add_uniform(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let name = arg_or(args, kw, 1, "name", Value::None);
    let value = arg_or(args, kw, 2, "value", Value::None);
    let u = {
        let mut d = r.borrow_mut();
        if py::is_none(&d.uniforms) {
            d.uniforms = vm.types.new_dict();
        }
        d.uniforms.clone()
    };
    ops::setitem(vm, &u, name, value)?;
    Ok(Value::None)
}

fn r_add_property(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let name = arg_or(args, kw, 1, "name", Value::None);
    let value = arg_or(args, kw, 2, "value", Value::None);
    let p = {
        let mut d = r.borrow_mut();
        if py::is_none(&d.properties) {
            d.properties = vm.types.new_dict();
        }
        d.properties.clone()
    };
    ops::setitem(vm, &p, name, value)?;
    Ok(Value::None)
}

fn r_get_property(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let name = arg_or(args, kw, 1, "name", Value::None);
    let default = arg_or(args, kw, 2, "default", Value::None);
    let p = r.borrow().properties.clone();
    if py::is_none(&p) {
        return Ok(default);
    }
    vm.call_method(&p, "get", vec![name, default])
}

fn r_is_pixel_opaque(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let x = py::f64_of(vm, &arg_or(args, kw, 1, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 2, "y", Value::Int(0)))?;
    Ok(Value::Bool(super::backend::pixel_opaque(vm, &r, x, y)?))
}

fn r_is_fully_transparent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    fn empty(r: &RenderData) -> bool {
        r.alpha <= 0.
            || r.children.iter().all(|c| match &c.source {
                Source::Render(r, _) => empty(&r.borrow()),
                Source::Texture(t, _) => matches!(t.kind, TexKind::Solid(c) if c[3] == 0),
            })
    }
    Ok(Value::Bool(empty(&r.borrow())))
}

fn r_main_displayables_at_point(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let x = py::f64_of(vm, &arg_or(args, kw, 1, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 2, "y", Value::Int(0)))?;
    let mut out = Vec::new();
    fn walk(vm: &mut Vm, r: &RenderData, x: f64, y: f64, out: &mut Vec<Value>) {
        let (x, y) = match &r.forward {
            Some(f) => f.transform2(x, y),
            None => (x, y),
        };
        if x < 0. || y < 0. || x >= r.width || y >= r.height {
            return;
        }
        for d in &r.render_of {
            out.push(d.clone());
        }
        for c in &r.children {
            if !c.main {
                continue;
            }
            if let Source::Render(cr, _) = &c.source {
                walk(vm, &cr.borrow(), x - c.x, y - c.y, out);
            }
        }
    }
    walk(vm, &r.borrow(), x, y, &mut out);
    Ok(vm.types.new_list(out))
}

fn r_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let name = args
        .get(1)
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_owned();
    let d = r.borrow();
    let mat = |vm: &Vm, m: &Option<Mat>| m.map_or(Value::None, |m| new_matrix(vm, m));
    Ok(match name.as_str() {
        "width" => Value::Float(d.width),
        "height" => Value::Float(d.height),
        "layer_name" => d.layer_name.clone(),
        "forward" => mat(vm, &d.forward),
        "reverse" => mat(vm, &d.reverse),
        "alpha" => Value::Float(d.alpha),
        "over" => Value::Float(d.over),
        "nearest" => d.nearest.clone(),
        "xclipping" => Value::Bool(d.xclipping),
        "yclipping" => Value::Bool(d.yclipping),
        "clipping" => Value::Bool(d.xclipping && d.yclipping),
        "modal" => d.modal.clone(),
        "text_input" => Value::Bool(d.text_input),
        "operation" => Value::Int(d.operation),
        "operation_complete" => Value::Float(d.operation_complete),
        "operation_alpha" => Value::Bool(d.operation_alpha),
        "operation_parameter" => Value::Float(d.operation_parameter),
        "focus_screen" => d.focus_screen.clone(),
        "render_of" => vm.types.new_list(d.render_of.clone()),
        "mesh" => d.mesh.clone(),
        "shaders" => d.shaders.clone(),
        "uniforms" => d.uniforms.clone(),
        "properties" => d.properties.clone(),
        "children" => {
            let items = d
                .children
                .iter()
                .map(|c| {
                    vm.types.new_tuple(vec![
                        c.source.value().clone(),
                        Value::Float(c.x),
                        Value::Float(c.y),
                        Value::Bool(c.focus),
                        Value::Bool(c.main),
                    ])
                })
                .collect();
            vm.types.new_list(items)
        }
        "focuses" => {
            let items = d
                .focuses
                .iter()
                .map(|f| {
                    let r = f.rect.map(|r| r.map(Value::Float));
                    let (mx, my, mask) = f
                        .mask
                        .clone()
                        .map_or((Value::None, Value::None, Value::None), |(x, y, m)| {
                            (Value::Float(x), Value::Float(y), m)
                        });
                    let [x, y, w, h] =
                        r.unwrap_or([Value::None, Value::None, Value::None, Value::None]);
                    vm.types
                        .new_tuple(vec![f.d.clone(), f.arg.clone(), x, y, w, h, mx, my, mask])
                })
                .collect();
            vm.types.new_list(items)
        }
        "mark" | "cache_killed" | "killed" | "loaded" => Value::Bool(false),
        "surface" | "alpha_surface" | "half_cache" | "cached_texture" | "cached_model"
        | "pass_focuses" => Value::None,
        _ => match d.extra.get(&name) {
            Some(v) => v.clone(),
            None => return Err(vm.attr_error(format!("'Render' object has no attribute '{name}'"))),
        },
    })
}

fn bool_of(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    ops::truthy(vm, v)
}

fn r_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let name = args
        .get(1)
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_owned();
    let v = args.get(2).cloned().unwrap_or(Value::None);
    let mat = |vm: &Vm, v: &Value| -> PyResult<Option<Mat>> {
        if py::is_none(v) {
            return Ok(None);
        }
        matrix_data(v)
            .map(Some)
            .ok_or_else(|| vm.type_error("expected a Matrix"))
    };
    match name.as_str() {
        "width" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().width = f;
        }
        "height" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().height = f;
        }
        "forward" => {
            let m = mat(vm, &v)?;
            r.borrow_mut().forward = m;
        }
        "reverse" => {
            let m = mat(vm, &v)?;
            r.borrow_mut().reverse = m;
        }
        "alpha" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().alpha = f;
        }
        "over" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().over = f;
        }
        "xclipping" => {
            let b = bool_of(vm, &v)?;
            r.borrow_mut().xclipping = b;
        }
        "yclipping" => {
            let b = bool_of(vm, &v)?;
            r.borrow_mut().yclipping = b;
        }
        "clipping" => {
            let b = bool_of(vm, &v)?;
            let mut d = r.borrow_mut();
            d.xclipping = b;
            d.yclipping = b;
        }
        "text_input" => {
            let b = bool_of(vm, &v)?;
            r.borrow_mut().text_input = b;
        }
        "operation" => {
            let i = py::int(vm, &v)?;
            r.borrow_mut().operation = i;
        }
        "operation_complete" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().operation_complete = f;
        }
        "operation_alpha" => {
            let b = bool_of(vm, &v)?;
            r.borrow_mut().operation_alpha = b;
        }
        "operation_parameter" => {
            let f = py::f64_of(vm, &v)?;
            r.borrow_mut().operation_parameter = f;
        }
        "nearest" => r.borrow_mut().nearest = v,
        "modal" => r.borrow_mut().modal = v,
        "layer_name" => r.borrow_mut().layer_name = v,
        "focus_screen" => r.borrow_mut().focus_screen = v,
        "mesh" => r.borrow_mut().mesh = v,
        "shaders" => r.borrow_mut().shaders = v,
        "uniforms" => r.borrow_mut().uniforms = v,
        "properties" => r.borrow_mut().properties = v,
        "render_of" => {
            let items = py::items(vm, &v)?;
            r.borrow_mut().render_of = items;
        }
        "children" => {
            let items = py::items(vm, &v)?;
            let mut out = Vec::new();
            for c in items {
                let t = py::items(vm, &c)?;
                if t.len() < 3 {
                    return Err(vm.type_error("Render child must be (source, x, y, focus, main)"));
                }
                let source = source_of(vm, &t[0])?;
                let x = py::f64_of(vm, &t[1])?;
                let y = py::f64_of(vm, &t[2])?;
                let focus = match t.get(3) {
                    Some(v) => ops::truthy(vm, v)?,
                    None => true,
                };
                let main = match t.get(4) {
                    Some(v) => ops::truthy(vm, v)?,
                    None => true,
                };
                out.push(Blit {
                    source,
                    x,
                    y,
                    focus,
                    main,
                });
            }
            r.borrow_mut().children = out;
        }
        _ => {
            r.borrow_mut().extra.insert(name, v);
        }
    }
    Ok(Value::None)
}

fn r_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = this(vm, args)?;
    let d = r.borrow();
    Ok(vm.types.new_string(format!(
        "<Render {}x{} of {} children>",
        d.width,
        d.height,
        d.children.len()
    )))
}

fn r_kill(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let _ = this(vm, args)?;
    Ok(Value::None)
}

/// `render(d, width, height, st, at)` with the per-frame cache.
pub fn render(
    vm: &mut Vm,
    d: &Value,
    width: f64,
    height: f64,
    st: f64,
    at: f64,
) -> PyResult<Value> {
    let id = d.id();
    let frame_time = rs(vm).borrow_mut().frame_time;
    let key = |w: f64, h: f64| {
        [
            w.to_bits(),
            h.to_bits(),
            (frame_time - st).to_bits(),
            (frame_time - at).to_bits(),
        ]
    };
    let lookup = |vm: &Vm, k: [u64; 4]| {
        rs(vm)
            .borrow_mut()
            .cache
            .get(&id)
            .and_then(|v| v.iter().find(|(kk, _)| *kk == k).map(|(_, r)| r.clone()))
    };
    let orig = key(width, height);
    if let Some(r) = lookup(vm, orig) {
        return Ok(r);
    }
    let (mut w, mut h) = (width, height);
    let style = ops::getattr(vm, d, "style")?;
    if let Some(offer) = ops::getattr_opt(vm, d, "_offer_size")? {
        if !py::is_none(&offer) {
            let p = py::items(vm, &offer)?;
            if p.len() == 2 {
                w = py::f64_of(vm, &p[0])?;
                h = py::f64_of(vm, &p[1])?;
            }
        }
    }
    let xmax = ops::getattr(vm, &style, "xmaximum")?;
    if !py::is_none(&xmax) {
        w = super::core::compute_raw(vm, &xmax, w)?.min(w);
    }
    let ymax = ops::getattr(vm, &style, "ymaximum")?;
    if !py::is_none(&ymax) {
        h = super::core::compute_raw(vm, &ymax, h)?.min(h);
    }
    w = w.max(0.);
    h = h.max(0.);
    let wh = key(w, h);
    if wh != orig {
        if let Some(r) = lookup(vm, wh) {
            return Ok(r);
        }
    }
    let (old_st, old_at, old_size) = {
        let s_rc = rs(vm);
        let mut s = s_rc.borrow_mut();
        s.rendering += 1;
        let old = (s.render_st, s.render_at, s.render_size);
        s.render_st = st;
        s.render_at = at;
        s.render_size = (w, h);
        old
    };
    let result = vm.call_method(
        d,
        "render",
        vec![
            Value::Float(w),
            Value::Float(h),
            Value::Float(st),
            Value::Float(at),
        ],
    );
    {
        let s_rc = rs(vm);
        let mut s = s_rc.borrow_mut();
        s.rendering -= 1;
        s.render_st = old_st;
        s.render_at = old_at;
        s.render_size = old_size;
    }
    let mut rv = result?;
    let r = render_data(&rv).ok_or_else(|| {
        py::error(
            vm,
            format!(
                "{}.render() must return a Render.",
                d.obj().map(|o| o.class_name()).unwrap_or_default()
            ),
        )
    })?;
    r.borrow_mut().render_of.push(d.clone());
    let clipping = ops::getattr_opt(vm, d, "_clipping")?
        .map(|v| ops::truthy(vm, &v))
        .transpose()?
        .unwrap_or(false);
    if clipping {
        let (rw, rh) = {
            let r = r.borrow();
            (r.width, r.height)
        };
        rv = subsurface(vm, &rv, [0., 0., rw, rh], true, false)?;
        if let Some(r2) = render_data(&rv) {
            r2.borrow_mut().render_of.push(d.clone());
        }
    }
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    if !s.sizing {
        s.cache_keep.insert(id, d.clone());
        let e = s.cache.entry(id).or_default();
        e.push((wh, rv.clone()));
        if wh != orig {
            e.push((orig, rv.clone()));
        }
    }
    Ok(rv)
}

fn f_render(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg(args, kw, 0, "d")
        .cloned()
        .ok_or_else(|| vm.type_error("render() missing d"))?;
    let w = py::f64_of(vm, &arg_or(args, kw, 1, "width", Value::Int(0)))?;
    let h = py::f64_of(vm, &arg_or(args, kw, 2, "height", Value::Int(0)))?;
    let st = py::f64_of(vm, &arg_or(args, kw, 3, "st", Value::Int(0)))?;
    let at = py::f64_of(vm, &arg_or(args, kw, 4, "at", Value::Int(0)))?;
    render(vm, &d, w, h, st, at)
}

fn f_render_for_size(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let old = std::mem::replace(&mut rs(vm).borrow_mut().sizing, true);
    let r = f_render(vm, args, kw);
    rs(vm).borrow_mut().sizing = old;
    r
}

/// `invalidate(d)`: drop `d` from the cache and request a redraw.
pub fn invalidate(vm: &mut Vm, d: &Value) {
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    // Cached renders that contain d are stale too; clear everything that
    // depends on the frame (renders are rebuilt in full on redraw).
    s.cache.remove(&d.id());
    if s.rendering == 0 {
        s.invalidated = true;
    }
}

fn f_invalidate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if let Some(d) = args.first() {
        invalidate(vm, d);
    }
    Ok(Value::None)
}

/// `redraw(d, when)`: redraw `d` at most `when` seconds from now.
pub fn redraw(vm: &mut Vm, d: &Value, when: f64) {
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    let t = s.frame_time + when.max(0.);
    s.redraws.push((d.clone(), t));
}

fn f_redraw(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let when = py::f64_of(vm, &arg_or(args, kw, 1, "when", Value::Int(0)))?;
    redraw(vm, &d, when);
    Ok(Value::None)
}

/// Seconds until the earliest pending redraw.
pub fn redraw_time(vm: &Vm) -> Option<f64> {
    let s_rc = rs(vm);
    let s = s_rc.borrow_mut();
    if s.invalidated {
        return Some(s.frame_time);
    }
    s.redraws
        .iter()
        .map(|(_, t)| *t)
        .fold(None, |a, t| Some(a.map_or(t, |a: f64| a.min(t))))
}

fn f_redraw_time(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(redraw_time(vm).map_or(Value::None, Value::Float))
}

/// Starts a frame: drops due redraws. Returns true when a redraw is due.
pub fn begin_frame(vm: &Vm, now: f64) -> bool {
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    s.frame_time = now;
    let due = s.invalidated || s.redraws.iter().any(|(_, t)| *t <= now);
    if due {
        s.redraws.retain(|(_, t)| *t > now);
        s.invalidated = false;
        s.cache.clear();
        s.cache_keep.clear();
    }
    due
}

/// `check_redraws()`: whether a redraw is due at the host time.
pub fn check_redraws(vm: &Vm) -> bool {
    let now = super::link::now(vm);
    let s_rc = rs(vm);
    let s = s_rc.borrow();
    s.invalidated || s.redraws.iter().any(|(_, t)| *t <= now)
}

fn f_check_redraws(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(check_redraws(vm)))
}

/// `process_redraws()`: consumes the due redraws (their renders are
/// rebuilt) and reports whether any was due.
fn f_process_redraws(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let now = super::link::now(vm);
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    let due = s.invalidated || s.redraws.iter().any(|(_, t)| *t <= now);
    if due {
        s.redraws.retain(|(_, t)| *t > now);
        s.invalidated = false;
        s.cache.clear();
        s.cache_keep.clear();
    }
    Ok(Value::Bool(due))
}

/// `render_screen(root, width, height)`: renders the root at the
/// interaction's shown time and records it as the screen render.
fn f_render_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let root = arg_or(args, kw, 0, "root", Value::None);
    let w = py::f64_of(vm, &arg_or(args, kw, 1, "width", Value::Int(0)))?;
    let h = py::f64_of(vm, &arg_or(args, kw, 2, "height", Value::Int(0)))?;
    let iface = py::import_attr(vm, "renpy.display", "interface")?;
    let it = py::get_or_none(vm, &iface, "interact_time")?;
    let ft = py::get_or_none(vm, &iface, "frame_time")?;
    let ft = py::f64_of(vm, &ft)?;
    rs(vm).borrow_mut().frame_time = ft;
    let st = if py::is_none(&it) {
        0.
    } else {
        ft - py::f64_of(vm, &it)?
    };
    let rv = render(vm, &root, w, h, st, st)?;
    let m = py::import(vm, "renpy.display.render")?;
    py::set(vm, &m, "screen_render", rv.clone())?;
    {
        let s = rs(vm);
        let mut s = s.borrow_mut();
        s.screen_render = Some(rv.clone());
        s.invalidated = false;
    }
    Ok(rv)
}

/// Clears the render cache (new interaction, style rebuild).
pub fn free_memory(vm: &Vm) {
    let s_rc = rs(vm);
    let mut s = s_rc.borrow_mut();
    s.cache.clear();
    s.cache_keep.clear();
    s.invalidated = true;
}

const MATRIX_DOC_OFFSET: &str = "\n        :doc: matrix\n        :args: (x, y, z)\n\n        Returns a matrix that offsets the vertex by a fixed amount.\n        ";
const MATRIX_DOC_ROTATE: &str = "\n        :doc: matrix\n        :args: (x, y, z)\n\n        Returns a matrix that rotates the displayable around the\n        origin.\n\n        `x`, `y`, `z`\n            The amount to rotate around the origin, in degrees.\n\n\n        The rotations are applied in order:\n\n        * A clockwise rotation by `x` degrees in the Y/Z plane.\n        * A clockwise rotation by `y` degrees in the Z/X plane.\n        * A clockwise rotation by `z` degrees in the X/Y plane.\n        ";
const MATRIX_DOC_SCALE: &str = "\n        :doc: matrix\n        :args: (x, y, z)\n\n        Returns a matrix that scales the displayable.\n\n        `x`, `y`, `z`\n            The factor to scale each axis by.\n        ";
const MATRIX_DOC_PERSPECTIVE: &str = "\n        :doc: matrix\n        :args: (w, h, n, p, f)\n\n        Returns a matrix suitable for the perspective projection of an image\n        in the Ren'Py coordinate system. This is a view into the a coordinate\n        system where, where when z=0, (0, 0) corresponds to the top-left corner\n        of the screen, and (w, h) corresponds to the bottom-right corner of\n        the screen.\n\n        `w`, `h`\n            The width and height of the input plane, in pixels.\n\n        `n`\n            The distance of the near plane from the camera.\n\n        `p`\n            How far the z=0 plane is from the camera. This is also where one\n            virtual pixel is one coordinate unit in x and y.\n\n        `f`\n            The distance of the far plane from the camera.\n        ";
const MATRIX_DOC_SCREEN_PROJECTION: &str = "\n        This generates a matrix that projects the Ren'Py space, where (0, 0) is the\n        top left and (`w`, `h`) is the bottom right, into the OpenGL viewport, where\n        (-1.0, 1.0) is the top left and (1.0, -1.0) is the bottom.\n\n        Generates the matrix that projects the Ren'Py screen to the OpenGL screen.\n        ";
const MATRIX_DOC_TEXTURE_PROJECTION: &str = "\n        This generates a matrix that project the Ren'Py space, where (0, 0) is the\n        top left and (`w`, `h`) is the bottom right, into the OpenGL render-to-texture\n        space, where (-1.0, -1.0) is the top left and (1.0, 1.0) is the bottom.\n\n        Generates the matrix that projects the Ren'Py screen to the OpenGL screen.\n        ";

// ---- Matrix ---------------------------------------------------------------

const MNAMES: [&str; 16] = [
    "xdx", "xdy", "xdz", "xdw", "ydx", "ydy", "ydz", "ydw", "zdx", "zdy", "zdz", "zdw", "wdx",
    "wdy", "wdz", "wdw",
];

fn m_this(vm: &Vm, args: &[Value]) -> PyResult<Mat> {
    args.first()
        .and_then(matrix_data)
        .ok_or_else(|| vm.type_error("Matrix method on non-Matrix"))
}

fn m_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let l = arg(&args[1..], kw, 0, "l").cloned().unwrap_or(Value::None);
    let mut m = Mat::IDENTITY;
    if !py::is_none(&l) {
        let v = py::items(vm, &l)?;
        match v.len() {
            16 => {
                for (i, x) in v.iter().enumerate() {
                    m.0[i] = py::f64_of(vm, x)?;
                }
            }
            9 => {
                for r in 0..3 {
                    for c in 0..3 {
                        m.0[r * 4 + c] = py::f64_of(vm, &v[r * 3 + c])?;
                    }
                }
            }
            4 => {
                m.0[0] = py::f64_of(vm, &v[0])?;
                m.0[1] = py::f64_of(vm, &v[1])?;
                m.0[4] = py::f64_of(vm, &v[2])?;
                m.0[5] = py::f64_of(vm, &v[3])?;
            }
            _ => return Err(vm.value_error("Matrix expects 4, 9 or 16 components")),
        }
    }
    let cls = args[0]
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("Matrix.__new__"))?;
    Ok(vm.types.new_native(&cls, Box::new(m)))
}

/// `Matrix2D(xdx, xdy, ydx, ydy)`: the 2D (row-major) matrix of every release.
fn f_matrix2d(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if args.len() != 4 {
        return Err(vm.type_error("Matrix2D(xdx, xdy, ydx, ydy)"));
    }
    let mut m = Mat::IDENTITY;
    m.0[0] = py::f64_of(vm, &args[0])?;
    m.0[1] = py::f64_of(vm, &args[1])?;
    m.0[4] = py::f64_of(vm, &args[2])?;
    m.0[5] = py::f64_of(vm, &args[3])?;
    Ok(new_matrix(vm, m))
}

fn m_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let name = args.get(1).and_then(|n| n.as_str()).unwrap_or("");
    match MNAMES.iter().position(|n| *n == name) {
        Some(i) => Ok(Value::Float(m.0[i])),
        None => Err(vm.attr_error(format!("'Matrix' object has no attribute '{name}'"))),
    }
}

fn m_setattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let name = args
        .get(1)
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_owned();
    let v = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let i = MNAMES
        .iter()
        .position(|n| *n == name)
        .ok_or_else(|| vm.attr_error(format!("Matrix has no component {name}")))?;
    py::native::<Mat, _>(&args[0], |m| m.0[i] = v);
    Ok(Value::None)
}

fn m_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = m_this(vm, args)?;
    let b = args.get(1).and_then(matrix_data);
    match b {
        Some(b) => Ok(new_matrix(vm, a.mul(&b))),
        None => Ok(vm.types.not_implemented.clone()),
    }
}

fn m_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = m_this(vm, args)?;
    Ok(Value::Bool(
        args.get(1).and_then(matrix_data).is_some_and(|b| a == b),
    ))
}

fn m_transform(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let x = py::f64_of(vm, &arg_or(args, kw, 1, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 2, "y", Value::Int(0)))?;
    let z = py::f64_of(vm, &arg_or(args, kw, 3, "z", Value::Int(0)))?;
    let w = py::f64_of(vm, &arg_or(args, kw, 4, "w", Value::Int(1)))?;
    let components = py::int(vm, &arg_or(args, kw, 5, "components", Value::Int(2)))?;
    let [a, b, c, d] = m.transform(x, y, z, w);
    let v = [a, b, c, d]
        .iter()
        .take(components.clamp(1, 4) as usize)
        .map(|x| Value::Float(*x))
        .collect();
    Ok(vm.types.new_tuple(v))
}

fn m_inverse(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    Ok(new_matrix(vm, m.inverse()))
}

fn m_is_unit_aligned(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let a = m.0;
    Ok(Value::Bool(
        (a[1] == 0. && a[4] == 0.) || (a[0] == 0. && a[5] == 0.),
    ))
}

fn m_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let i = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))?;
    m.0.get(i as usize)
        .map(|v| Value::Float(*v))
        .ok_or_else(|| vm.index_error("Matrix index out of range"))
}

fn m_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let rows: Vec<String> = (0..4)
        .map(|r| {
            format!(
                "{:.5}, {:.5}, {:.5}, {:.5}",
                m.0[r * 4],
                m.0[r * 4 + 1],
                m.0[r * 4 + 2],
                m.0[r * 4 + 3]
            )
        })
        .collect();
    Ok(vm
        .types
        .new_string(format!("Matrix([{}])", rows.join(",\n        "))))
}

fn m_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = m_this(vm, args)?;
    let cls = rs(vm).borrow_mut().matrix_class.clone();
    let l = vm
        .types
        .new_list(m.0.iter().map(|v| Value::Float(*v)).collect());
    let a = vm.types.new_tuple(vec![l]);
    Ok(vm.types.new_tuple(vec![Value::Obj(cls), a]))
}

fn sm_identity(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(new_matrix(vm, Mat::IDENTITY))
}
fn sm_offset(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &arg_or(args, kw, 0, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 1, "y", Value::Int(0)))?;
    let z = py::f64_of(vm, &arg_or(args, kw, 2, "z", Value::Int(0)))?;
    Ok(new_matrix(vm, Mat::offset(x, y, z)))
}
fn sm_scale(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &arg_or(args, kw, 0, "x", Value::Int(1)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 1, "y", Value::Int(1)))?;
    let z = py::f64_of(vm, &arg_or(args, kw, 2, "z", Value::Int(1)))?;
    Ok(new_matrix(vm, Mat::scale(x, y, z)))
}
fn sm_rotate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let x = py::f64_of(vm, &arg_or(args, kw, 0, "x", Value::Int(0)))?;
    let y = py::f64_of(vm, &arg_or(args, kw, 1, "y", Value::Int(0)))?;
    let z = py::f64_of(vm, &arg_or(args, kw, 2, "z", Value::Int(0)))?;
    Ok(new_matrix(vm, Mat::rotate(x, y, z)))
}
fn sm_perspective(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut v = [0.; 5];
    for (i, n) in ["w", "h", "n", "p", "f"].iter().enumerate() {
        v[i] = py::f64_of(vm, &arg_or(args, kw, i, n, Value::Int(0)))?;
    }
    Ok(new_matrix(
        vm,
        Mat::perspective(v[0], v[1], v[2], v[3], v[4]),
    ))
}
fn sm_screen_projection(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let w = py::f64_of(vm, &arg_or(args, kw, 0, "w", Value::Int(1)))?;
    let h = py::f64_of(vm, &arg_or(args, kw, 1, "h", Value::Int(1)))?;
    Ok(new_matrix(
        vm,
        Mat([
            2. / w,
            0.,
            0.,
            -1.,
            0.,
            -2. / h,
            0.,
            1.,
            0.,
            0.,
            1.,
            0.,
            0.,
            0.,
            0.,
            1.,
        ]),
    ))
}
fn sm_texture_projection(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let w = py::f64_of(vm, &arg_or(args, kw, 0, "w", Value::Int(1)))?;
    let h = py::f64_of(vm, &arg_or(args, kw, 1, "h", Value::Int(1)))?;
    Ok(new_matrix(
        vm,
        Mat([
            2. / w,
            0.,
            0.,
            -1.,
            0.,
            2. / h,
            0.,
            -1.,
            0.,
            0.,
            1.,
            0.,
            0.,
            0.,
            0.,
            1.,
        ]),
    ))
}

// ---- Texture --------------------------------------------------------------

fn t_this(vm: &Vm, args: &[Value]) -> PyResult<Rc<Texture>> {
    args.first()
        .and_then(texture_data)
        .ok_or_else(|| vm.type_error("texture method on non-texture"))
}

fn t_get_size(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = t_this(vm, args)?;
    Ok(vm
        .types
        .new_tuple(vec![Value::Float(t.width), Value::Float(t.height)]))
}

fn t_subsurface(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = t_this(vm, args)?;
    let rect = arg_or(args, kw, 1, "rect", Value::None);
    let [x, y, w, h] = rect4(vm, &rect)?;
    let [bx, by] = match t.crop {
        Some(c) => [c[0], c[1]],
        None => [0., 0.],
    };
    let mut n = (*t).clone();
    if n.full.is_none() {
        n.full = Some([t.width, t.height]);
    }
    n.crop = Some([bx + x, by + y, w, h]);
    n.width = w;
    n.height = h;
    Ok(new_texture(vm, n))
}

fn t_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = t_this(vm, args)?;
    let name = args.get(1).and_then(|n| n.as_str()).unwrap_or("");
    Ok(match name {
        "width" => Value::Float(t.width),
        "height" => Value::Float(t.height),
        _ => return Err(vm.attr_error(format!("texture has no attribute '{name}'"))),
    })
}

// ---- Canvas ---------------------------------------------------------------

fn canvas_push(
    vm: &mut Vm,
    args: &[Value],
    op: &str,
    fields: serde_json::Value,
) -> PyResult<Value> {
    let color = super::color::rgba(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let mut o = serde_json::json!({"op": op, "color": color});
    if let (Some(o), serde_json::Value::Object(f)) = (o.as_object_mut(), fields) {
        o.extend(f);
    }
    py::native::<Rc<RefCell<Vec<serde_json::Value>>>, _>(&args[0], |ops| ops.borrow_mut().push(o));
    Ok(Value::None)
}

fn points(vm: &mut Vm, v: &Value) -> PyResult<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    for p in py::items(vm, v)? {
        let p = py::items(vm, &p)?;
        if p.len() != 2 {
            return Err(vm.type_error("point must be (x, y)"));
        }
        out.push([py::f64_of(vm, &p[0])?, py::f64_of(vm, &p[1])?]);
    }
    Ok(out)
}

fn c_rect(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = rect4(vm, &arg_or(args, kw, 2, "rect", Value::None))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 3, "width", Value::Int(0)))?;
    canvas_push(
        vm,
        args,
        "rect",
        serde_json::json!({"rect": rect, "width": width}),
    )
}
fn c_polygon(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let pts = points(vm, &arg_or(args, kw, 2, "pointlist", Value::None))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 3, "width", Value::Int(0)))?;
    canvas_push(
        vm,
        args,
        "polygon",
        serde_json::json!({"points": pts, "width": width}),
    )
}
fn c_circle(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = points(
        vm,
        &vm.types
            .new_list(vec![arg_or(args, kw, 2, "pos", Value::None)]),
    )?;
    let radius = py::f64_of(vm, &arg_or(args, kw, 3, "radius", Value::Int(0)))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 4, "width", Value::Int(0)))?;
    canvas_push(
        vm,
        args,
        "circle",
        serde_json::json!({"center": p[0], "radius": radius, "width": width}),
    )
}
fn c_ellipse(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = rect4(vm, &arg_or(args, kw, 2, "rect", Value::None))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 3, "width", Value::Int(0)))?;
    canvas_push(
        vm,
        args,
        "ellipse",
        serde_json::json!({"rect": rect, "width": width}),
    )
}
fn c_arc(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rect = rect4(vm, &arg_or(args, kw, 2, "rect", Value::None))?;
    let a0 = py::f64_of(vm, &arg_or(args, kw, 3, "start_angle", Value::Int(0)))?;
    let a1 = py::f64_of(vm, &arg_or(args, kw, 4, "stop_angle", Value::Int(0)))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 5, "width", Value::Int(1)))?;
    canvas_push(
        vm,
        args,
        "arc",
        serde_json::json!({"rect": rect, "start": a0, "stop": a1, "width": width}),
    )
}
fn c_line(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = arg_or(args, kw, 2, "start_pos", Value::None);
    let b = arg_or(args, kw, 3, "end_pos", Value::None);
    let pts = points(vm, &vm.types.new_list(vec![a, b]))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 4, "width", Value::Int(1)))?;
    canvas_push(
        vm,
        args,
        "lines",
        serde_json::json!({"points": pts, "closed": false, "width": width}),
    )
}
fn c_lines(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let closed = ops::truthy(vm, &arg_or(args, kw, 2, "closed", Value::Bool(false)))?;
    let pts = points(vm, &arg_or(args, kw, 3, "pointlist", Value::None))?;
    let width = py::f64_of(vm, &arg_or(args, kw, 4, "width", Value::Int(1)))?;
    canvas_push(
        vm,
        args,
        "lines",
        serde_json::json!({"points": pts, "closed": closed, "width": width}),
    )
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        "renpy.display.render",
        &[
            ("render", f_render),
            ("render_for_size", f_render_for_size),
            ("invalidate", f_invalidate),
            ("redraw", f_redraw),
            ("redraw_time", f_redraw_time),
            ("process_redraws", f_process_redraws),
            ("check_redraws", f_check_redraws),
            ("render_screen", f_render_screen),
            ("free_memory", |vm, _, _| {
                free_memory(vm);
                Ok(Value::None)
            }),
            ("check_at_shutdown", r_noop),
            ("mark_sweep", r_noop),
            ("mutated_surface", r_noop),
            ("adjust_render_cache_times", r_noop),
            ("render_ready", |vm, _, _| {
                rs(vm).borrow_mut().render_ready = true;
                Ok(Value::None)
            }),
            ("Matrix2D", f_matrix2d),
        ],
    );
    let rc = rs(vm).borrow_mut().render_class.clone();
    let tc = rs(vm).borrow_mut().texture_class.clone();
    let mc = rs(vm).borrow_mut().matrix_class.clone();
    let cc = rs(vm).borrow_mut().canvas_class.clone();
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "Render", Value::Obj(rc));
    set(vm, "Texture", Value::Obj(tc));
    set(vm, "Matrix", Value::Obj(mc));
    set(vm, "Canvas", Value::Obj(cc));
    for (n, v) in [
        ("BLIT", BLIT),
        ("DISSOLVE", DISSOLVE),
        ("IMAGEDISSOLVE", IMAGEDISSOLVE),
        ("PIXELLATE", PIXELLATE),
        ("FLATTEN", FLATTEN),
    ] {
        set(vm, n, Value::Int(v));
    }
    let ident = new_matrix(vm, Mat::IDENTITY);
    set(vm, "IDENTITY", ident);
    set(vm, "models", Value::Bool(true));
    set(vm, "screen_render", Value::None);
    set(vm, "per_frame", Value::Bool(false));
    set(vm, "render_cache", vm.types.new_dict());
    Ok(m)
}

pub fn init_matrix_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        "renpy.display.matrix",
        &[("Matrix2D", f_matrix2d)],
    );
    let mc = rs(vm).borrow_mut().matrix_class.clone();
    crysirror_python::modules::set(vm, &m, "Matrix", Value::Obj(mc));
    let ident = new_matrix(vm, Mat::IDENTITY);
    crysirror_python::modules::set(vm, &m, "IDENTITY", ident);
    Ok(m)
}

fn native_type(
    vm: &mut Vm,
    name: &str,
    module: &str,
    new: crysirror_python::object::NativeFn,
    methods: Methods,
) -> ObjRef {
    let c = vm
        .types
        .new_builtin_type(name, &vm.types.object.clone(), BaseKind::Native, true);
    let n = vm.types.new_builtin("__new__", new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&c, "__new__", n);
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str(module);
    vm.types.set_attr(&c, "__module__", mn);
    c
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("cannot create this object directly"))
}

pub fn install(vm: &mut Vm) {
    let render_class = native_type(
        vm,
        "Render",
        "renpy.display.render",
        r_new,
        &[
            ("__init__", r_init),
            ("blit", r_blit),
            ("subpixel_blit", r_subpixel_blit),
            ("absolute_blit", r_absolute_blit),
            ("get_size", r_get_size),
            ("subsurface", r_subsurface),
            ("depends_on", r_depends_on),
            ("kill_cache", r_noop),
            ("kill", r_kill),
            ("add_focus", r_add_focus),
            ("fill", r_fill),
            ("canvas", r_canvas),
            ("place", r_place),
            ("zoom", r_zoom),
            ("add_shader", r_add_shader),
            ("add_uniform", r_add_uniform),
            ("add_property", r_add_property),
            ("get_property", r_get_property),
            ("is_pixel_opaque", r_is_pixel_opaque),
            ("is_fully_transparent", r_is_fully_transparent),
            ("main_displayables_at_point", r_main_displayables_at_point),
            ("__getattr__", r_getattr),
            ("__setattr__", r_setattr),
            ("__repr__", r_repr),
        ],
    );
    let nmf = vm.types.new_str("NO_MOUSE_FOCUS");
    vm.types.set_attr(&render_class, "NO_MOUSE_FOCUS", nmf);
    let texture_class = native_type(
        vm,
        "Texture",
        "renpy.display.render",
        no_new,
        &[
            ("get_size", t_get_size),
            ("subsurface", t_subsurface),
            ("__getattr__", t_getattr),
        ],
    );
    let matrix_class = native_type(
        vm,
        "Matrix",
        "renpy.display.matrix",
        m_new,
        &[
            ("__getattr__", m_getattr),
            ("__setattr__", m_setattr),
            ("__mul__", m_mul),
            ("__matmul__", m_mul),
            ("__eq__", m_eq),
            ("__getitem__", m_getitem),
            ("transform", m_transform),
            ("inverse", m_inverse),
            ("is_unit_aligned", m_is_unit_aligned),
            ("__repr__", m_repr),
            ("__reduce__", m_reduce),
        ],
    );
    // Docstrings of renpy/display/matrix.pyx; 00matrixtransform.rpy
    // derives the TransformMatrix documentation from them.
    for (n, f, doc) in [
        (
            "identity",
            sm_identity as crysirror_python::object::NativeFn,
            None,
        ),
        ("offset", sm_offset, Some(MATRIX_DOC_OFFSET)),
        ("scale", sm_scale, Some(MATRIX_DOC_SCALE)),
        ("rotate", sm_rotate, Some(MATRIX_DOC_ROTATE)),
        ("perspective", sm_perspective, Some(MATRIX_DOC_PERSPECTIVE)),
        (
            "screen_projection",
            sm_screen_projection,
            Some(MATRIX_DOC_SCREEN_PROJECTION),
        ),
        (
            "texture_projection",
            sm_texture_projection,
            Some(MATRIX_DOC_TEXTURE_PROJECTION),
        ),
    ] {
        if let Some(d) = doc {
            vm.types.set_builtin_doc(f, d);
        }
        let b = vm.types.new_builtin(n, f);
        let s = py::staticmethod(vm, b);
        vm.types.set_attr(&matrix_class, n, s);
    }
    let canvas_class = native_type(
        vm,
        "Canvas",
        "renpy.display.render",
        no_new,
        &[
            ("rect", c_rect),
            ("polygon", c_polygon),
            ("circle", c_circle),
            ("ellipse", c_ellipse),
            ("arc", c_arc),
            ("line", c_line),
            ("lines", c_lines),
            ("aaline", c_line),
            ("aalines", c_lines),
        ],
    );
    vm.ext_set(Rc::new(RefCell::new(RenderState {
        render_class,
        texture_class,
        matrix_class,
        canvas_class,
        cache: HashMap::new(),
        cache_keep: HashMap::new(),
        frame_time: 0.,
        rendering: 0,
        sizing: false,
        render_ready: false,
        redraws: Vec::new(),
        invalidated: true,
        screen_render: None,
        render_st: 0.,
        render_at: 0.,
        render_size: (0., 0.),
    })));
    vm.register_native_module("renpy.display.render", init_module);
    vm.register_native_module("renpy.display.matrix", init_matrix_module);
}
