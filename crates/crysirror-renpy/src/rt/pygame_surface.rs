// SPDX-License-Identifier: MPL-2.0
//! The pixel modules of the pygame layer (`pygame_sdl2` / `renpy.pygame`):
//! `surface`, `image`, `transform`, `draw` and `gfxdraw`, after the
//! vendored `renpy/pygame/*.pyx` of Ren'Py 8.5/8.6 (MIT/zlib).
//!
//! A `Surface` is a CPU RGBA8 pixel buffer owned here (32 bits per pixel,
//! little-endian RGBA masks, alpha channel only with `SRCALPHA`).
//! Subsurfaces share their parent's buffer. Blits follow SDL's blend
//! rules (`SDL_BLENDMODE_BLEND`, colour keys, surface alpha and the
//! `BLEND_*` special flags). `image.load` decodes through
//! `crysirror_image::decode_standard` (the shared decoder) and
//! `image.save` writes PNG through `crysirror_image::encode_png`; other
//! file types raise `error`. The `gfxdraw` primitives rasterize on the
//! CPU; the `aa*` variants draw the same pixels without coverage-based
//! anti-aliasing (SDL_gfx's AA ramps are not reproduced). Surfaces are not
//! presented anywhere: CrySirror composes frames from Ren'Py renders.
use super::py::{self, *};
use super::pygame::{self as pg, color_like, err, new_mod, new_rect, rect_like, sdl_rect, set};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, rc::Rc};

pub const SRCALPHA: i64 = 0x10000;
pub const BLEND_RGB_ADD: i64 = 0x1;
pub const BLEND_RGB_SUB: i64 = 0x2;
pub const BLEND_RGB_MULT: i64 = 0x3;
pub const BLEND_RGB_MIN: i64 = 0x4;
pub const BLEND_RGB_MAX: i64 = 0x5;
pub const BLEND_RGBA_ADD: i64 = 0x6;
pub const BLEND_RGBA_SUB: i64 = 0x7;
pub const BLEND_RGBA_MULT: i64 = 0x8;
pub const BLEND_RGBA_MIN: i64 = 0x9;
pub const BLEND_RGBA_MAX: i64 = 0x10;
pub const BLEND_PREMULTIPLIED: i64 = 0x11;

pub struct Buf {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

/// The native part of a `Surface`.
pub struct SurfData {
    pub buf: Rc<RefCell<Buf>>,
    /// Absolute offset of this view in `buf`.
    pub ox: i32,
    pub oy: i32,
    pub w: i32,
    pub h: i32,
    pub alpha: bool,
    pub colorkey: Option<[u8; 4]>,
    pub surface_alpha: Option<u8>,
    pub clip: [i32; 4],
    pub parent: Value,
    /// Offset relative to the parent.
    pub rel: (i32, i32),
    pub locks: Vec<Value>,
    pub ready: bool,
}

impl SurfData {
    pub fn new(w: i32, h: i32, alpha: bool) -> SurfData {
        SurfData {
            buf: Rc::new(RefCell::new(Buf {
                w,
                h,
                px: vec![0; (w.max(0) as usize) * (h.max(0) as usize) * 4],
            })),
            ox: 0,
            oy: 0,
            w,
            h,
            alpha,
            colorkey: None,
            surface_alpha: None,
            clip: [0, 0, w, h],
            parent: Value::None,
            rel: (0, 0),
            locks: Vec::new(),
            ready: true,
        }
    }
    fn idx(&self, b: &Buf, x: i32, y: i32) -> usize {
        (((self.oy + y) * b.w + self.ox + x) * 4) as usize
    }
    pub fn get(&self, x: i32, y: i32) -> [u8; 4] {
        let b = self.buf.borrow();
        let i = self.idx(&b, x, y);
        let mut p = [b.px[i], b.px[i + 1], b.px[i + 2], b.px[i + 3]];
        if !self.alpha {
            p[3] = 255;
        }
        p
    }
    pub fn put(&self, x: i32, y: i32, c: [u8; 4]) {
        let mut b = self.buf.borrow_mut();
        let i = self.idx(&b, x, y);
        b.px[i..i + 4].copy_from_slice(&c);
        if !self.alpha {
            b.px[i + 3] = 255;
        }
    }
    /// Plot inside the clip rectangle.
    pub fn plot(&self, x: i32, y: i32, c: [u8; 4], blend: bool) {
        let [cx, cy, cw, ch] = self.clip;
        if x < cx || y < cy || x >= cx + cw || y >= cy + ch || x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        if blend && c[3] != 255 {
            let d = self.get(x, y);
            self.put(x, y, blend_px(c, d, self.alpha));
        } else {
            self.put(x, y, c);
        }
    }
    pub fn pixels(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.w * self.h * 4).max(0) as usize);
        for y in 0..self.h {
            for x in 0..self.w {
                out.extend_from_slice(&self.get(x, y));
            }
        }
        out
    }
}

/// `SDL_BLENDMODE_BLEND`: dstRGB = srcRGB*srcA + dstRGB*(1-srcA),
/// dstA = srcA + dstA*(1-srcA).
fn blend_px(s: [u8; 4], d: [u8; 4], dst_alpha: bool) -> [u8; 4] {
    let a = s[3] as u32;
    let mix = |sc: u8, dc: u8| ((sc as u32 * a + dc as u32 * (255 - a) + 127) / 255) as u8;
    let da = if dst_alpha {
        (a + (d[3] as u32 * (255 - a) + 127) / 255).min(255) as u8
    } else {
        255
    };
    [mix(s[0], d[0]), mix(s[1], d[1]), mix(s[2], d[2]), da]
}

fn special(flag: i64, s: [u8; 4], d: [u8; 4]) -> [u8; 4] {
    let op = |f: &dyn Fn(u32, u32) -> u32, n: usize| {
        let mut o = d;
        for i in 0..n {
            o[i] = f(s[i] as u32, d[i] as u32).min(255) as u8;
        }
        o
    };
    match flag {
        BLEND_RGB_ADD => op(&|a, b| a + b, 3),
        BLEND_RGB_SUB => op(&|a, b| b.saturating_sub(a), 3),
        BLEND_RGB_MULT => op(&|a, b| (a * b + 255) >> 8, 3),
        BLEND_RGB_MIN => op(&|a, b| a.min(b), 3),
        BLEND_RGB_MAX => op(&|a, b| a.max(b), 3),
        BLEND_RGBA_ADD => op(&|a, b| a + b, 4),
        BLEND_RGBA_SUB => op(&|a, b| b.saturating_sub(a), 4),
        BLEND_RGBA_MULT => op(&|a, b| (a * b + 255) >> 8, 4),
        BLEND_RGBA_MIN => op(&|a, b| a.min(b), 4),
        BLEND_RGBA_MAX => op(&|a, b| a.max(b), 4),
        BLEND_PREMULTIPLIED => {
            let a = s[3] as u32;
            let mut o = [0u8; 4];
            for i in 0..4 {
                o[i] = (s[i] as u32 + (d[i] as u32 * (255 - a) + 127) / 255).min(255) as u8;
            }
            o
        }
        _ => blend_px(s, d, true),
    }
}

pub fn surf_of(v: &Value) -> Option<SurfRef> {
    py::native::<SurfData, _>(v, |s| SurfRef {
        buf: s.buf.clone(),
        ox: s.ox,
        oy: s.oy,
        w: s.w,
        h: s.h,
        alpha: s.alpha,
        colorkey: s.colorkey,
        surface_alpha: s.surface_alpha,
        clip: s.clip,
    })
}

/// A cheap snapshot of a surface's view (the pixels stay shared).
#[derive(Clone)]
pub struct SurfRef {
    pub buf: Rc<RefCell<Buf>>,
    pub ox: i32,
    pub oy: i32,
    pub w: i32,
    pub h: i32,
    pub alpha: bool,
    pub colorkey: Option<[u8; 4]>,
    pub surface_alpha: Option<u8>,
    pub clip: [i32; 4],
}

impl SurfRef {
    /// The view's bytes as tightly packed rows (as stored; the fourth byte
    /// of a surface without alpha is 255).
    pub fn packed(&self) -> Vec<u8> {
        self.data().pixels()
    }
    /// Replaces the view's bytes from tightly packed rows.
    pub fn store(&self, px: &[u8]) {
        let mut b = self.buf.borrow_mut();
        let bw = b.w;
        for y in 0..self.h.max(0) {
            let row = ((self.oy + y) * bw + self.ox) as usize * 4;
            let src = (y * self.w) as usize * 4;
            let n = self.w.max(0) as usize * 4;
            b.px[row..row + n].copy_from_slice(&px[src..src + n]);
            if !self.alpha {
                for x in 0..self.w.max(0) as usize {
                    b.px[row + x * 4 + 3] = 255;
                }
            }
        }
    }
    fn data(&self) -> SurfData {
        SurfData {
            buf: self.buf.clone(),
            ox: self.ox,
            oy: self.oy,
            w: self.w,
            h: self.h,
            alpha: self.alpha,
            colorkey: self.colorkey,
            surface_alpha: self.surface_alpha,
            clip: self.clip,
            parent: Value::None,
            rel: (0, 0),
            locks: Vec::new(),
            ready: true,
        }
    }
}

pub fn surf(vm: &Vm, v: &Value) -> PyResult<SurfRef> {
    surf_of(v)
        .filter(|s| s.w >= 0)
        .ok_or_else(|| vm.type_error("Argument must be a Surface."))
}

fn with<R>(v: &Value, f: impl FnOnce(&mut SurfData) -> R) -> Option<R> {
    py::native::<SurfData, _>(v, f)
}

fn surface_class(vm: &mut Vm) -> PyResult<ObjRef> {
    pg::class_obj(vm, "surface", "Surface")
}

pub fn new_surface(vm: &mut Vm, d: SurfData) -> PyResult<Value> {
    let c = surface_class(vm)?;
    Ok(vm.types.new_native(&c, Box::new(d)))
}

/// A surface built from straight RGBA pixels.
pub fn surface_from_rgba(vm: &mut Vm, w: i32, h: i32, rgba: Vec<u8>, alpha: bool) -> PyResult<Value> {
    let mut d = SurfData::new(w, h, alpha);
    d.buf.borrow_mut().px = rgba;
    new_surface(vm, d)
}

fn int_of(vm: &mut Vm, v: &Value) -> PyResult<i32> {
    pg::i32_of(vm, v)
}

fn xy(vm: &mut Vm, v: &Value) -> PyResult<(i32, i32)> {
    let it = py::items(vm, v)?;
    if it.len() < 2 {
        return Err(vm.type_error("position must be a pair of numbers"));
    }
    Ok((int_of(vm, &it[0])?, int_of(vm, &it[1])?))
}

fn this(vm: &Vm, a: &[Value]) -> PyResult<SurfRef> {
    surf(vm, a.first().unwrap_or(&Value::None))
}

fn rgba_tuple(vm: &Vm, c: [u8; 4]) -> Value {
    vm.types.new_tuple(c.iter().map(|x| Value::Int(*x as i64)).collect())
}

fn pg_color(vm: &mut Vm, c: [u8; 4]) -> PyResult<Value> {
    pg::new_color(vm, c)
}

fn map_rgb(s: &SurfRef, c: [u8; 4]) -> i64 {
    let a = if s.alpha { c[3] } else { 0 };
    (c[0] as i64) | (c[1] as i64) << 8 | (c[2] as i64) << 16 | (a as i64) << 24
}

fn unmap(s: &SurfRef, p: i64) -> [u8; 4] {
    let a = if s.alpha { (p >> 24) as u8 } else { 255 };
    [p as u8, (p >> 8) as u8, (p >> 16) as u8, a]
}

/// A colour argument: a mapped integer or anything `Color()` accepts.
fn color_arg(vm: &mut Vm, s: &SurfRef, v: &Value) -> PyResult<[u8; 4]> {
    if let Value::Int(i) = v {
        return Ok(unmap(s, *i));
    }
    let mut c = color_like(vm, v)?;
    if py::seq(v).is_some_and(|t| t.len() == 3) {
        c[3] = 255;
    }
    Ok(c)
}

// Surface ------------------------------------------------------------------------

fn s_new(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let c = a
        .first()
        .and_then(|c| c.obj().cloned())
        .ok_or_else(|| vm.type_error("Surface.__new__(X): X is not a type"))?;
    let mut d = SurfData::new(0, 0, false);
    d.ready = false;
    Ok(vm.types.new_native(&c, Box::new(d)))
}

fn s_init(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let size = arg_or(&a[1..], kw, 0, "size", Value::None);
    if py::seq(&size).is_some_and(|s| s.is_empty()) {
        return Ok(Value::None);
    }
    let flags = arg_or(&a[1..], kw, 1, "flags", Value::Int(0));
    let flags = py::int(vm, &flags)?;
    let depth = arg_or(&a[1..], kw, 2, "depth", Value::Int(32));
    let masks = arg_or(&a[1..], kw, 3, "masks", Value::None);
    let (w, h) = xy(vm, &size)?;
    if w < 0 || h < 0 {
        return Err(vm.exc_msg("AssertionError", ""));
    }
    let alpha = if !py::is_none(&masks) {
        let m = py::items(vm, &masks)?;
        m.len() == 4 && py::int(vm, &m[3])? != 0
    } else if let Some(sample) = surf_of(&depth).filter(|_| !matches!(depth, Value::Int(_))) {
        sample.alpha
    } else {
        flags & SRCALPHA != 0
    };
    with(&a[0], |d| *d = SurfData::new(w, h, alpha));
    Ok(Value::None)
}

fn s_repr(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(vm.types.new_string(format!("<Surface({}x{}x32)>", s.w, s.h)))
}

/// `SDL_UpperBlit` with clipping: copies `area` of `src` to `(dx, dy)`.
pub fn blit_into(dst: &SurfRef, src: &SurfRef, dx: i32, dy: i32, area: Option<[i32; 4]>, flags: i64) -> [i32; 4] {
    let [mut sx, mut sy, mut w, mut h] = area.unwrap_or([0, 0, src.w, src.h]);
    let (mut dx, mut dy) = (dx, dy);
    if sx < 0 {
        w += sx;
        dx -= sx;
        sx = 0;
    }
    if sy < 0 {
        h += sy;
        dy -= sy;
        sy = 0;
    }
    w = w.min(src.w - sx);
    h = h.min(src.h - sy);
    let [cx, cy, cw, ch] = dst.clip;
    let x0 = dx.max(cx).max(0);
    let y0 = dy.max(cy).max(0);
    let x1 = (dx + w).min(cx + cw).min(dst.w);
    let y1 = (dy + h).min(cy + ch).min(dst.h);
    if x1 <= x0 || y1 <= y0 {
        return [dx, dy, 0, 0];
    }
    // Read the source first: it may share pixels with the destination.
    let s = src.data();
    let d = dst.data();
    let mut rows = Vec::with_capacity(((y1 - y0) * (x1 - x0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            rows.push(s.get(sx + x - dx, sy + y - dy));
        }
    }
    let mut i = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let mut p = rows[i];
            i += 1;
            if !src.alpha {
                if let Some(k) = src.colorkey {
                    if p[..3] == k[..3] {
                        continue;
                    }
                }
                p[3] = src.surface_alpha.unwrap_or(255);
            } else if let Some(sa) = src.surface_alpha {
                p[3] = ((p[3] as u32 * sa as u32 + 127) / 255) as u8;
            }
            let o = if flags != 0 {
                special(flags, p, d.get(x, y))
            } else if p[3] == 255 {
                p
            } else {
                blend_px(p, d.get(x, y), dst.alpha)
            };
            d.put(x, y, o);
        }
    }
    [x0, y0, x1 - x0, y1 - y0]
}

fn s_blit(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let dst = this(vm, a)?;
    let src_v = arg_or(&a[1..], kw, 0, "source", Value::None);
    let src = surf(vm, &src_v)?;
    let dest = arg_or(&a[1..], kw, 1, "dest", Value::None);
    let area = arg_or(&a[1..], kw, 2, "area", Value::None);
    let flags = arg_or(&a[1..], kw, 3, "special_flags", Value::Int(0));
    let flags = py::int(vm, &flags)?;
    let [dx, dy, _, _] = sdl_rect(vm, &dest, Some("dest"))?;
    let area = if py::is_none(&area) {
        None
    } else {
        Some(sdl_rect(vm, &area, Some("area"))?)
    };
    blit_into(&dst, &src, dx, dy, area, flags);
    let dirty = super::pygame::clip_rect([dx, dy, src.w, src.h], [0, 0, dst.w, dst.h]);
    new_rect(vm, dirty)
}

fn copy_of(s: &SurfRef) -> SurfData {
    let mut d = SurfData::new(s.w, s.h, s.alpha);
    d.buf.borrow_mut().px = s.data().pixels();
    d.colorkey = s.colorkey;
    d.surface_alpha = s.surface_alpha;
    d
}

fn s_copy(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    new_surface(vm, copy_of(&s))
}

fn s_convert(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let mut d = copy_of(&s);
    // `convert()` targets the display format, which has no alpha channel.
    d.alpha = false;
    for p in d.buf.borrow_mut().px.chunks_mut(4) {
        p[3] = 255;
    }
    new_surface(vm, d)
}

fn s_convert_alpha(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let mut d = copy_of(&s);
    if !s.alpha {
        for p in d.buf.borrow_mut().px.chunks_mut(4) {
            p[3] = 255;
        }
    }
    d.alpha = true;
    new_surface(vm, d)
}

fn s_fill(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let color = arg_or(&a[1..], kw, 0, "color", Value::None);
    let c = color_arg(vm, &s, &color)?;
    let rect = arg_or(&a[1..], kw, 1, "rect", Value::None);
    let flags = arg_or(&a[1..], kw, 2, "special_flags", Value::Int(0));
    let flags = py::int(vm, &flags)?;
    let r = if py::is_none(&rect) {
        [0, 0, s.w, s.h]
    } else {
        rect_like(vm, &rect)?
    };
    let r = super::pygame::clip_rect(super::pygame::clip_rect(r, s.clip), [0, 0, s.w, s.h]);
    let d = s.data();
    for y in r[1]..r[1] + r[3] {
        for x in r[0]..r[0] + r[2] {
            let o = if flags != 0 { special(flags, c, d.get(x, y)) } else { c };
            d.put(x, y, o);
        }
    }
    new_rect(vm, r)
}

fn s_scroll(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let dx = arg_or(&a[1..], kw, 0, "dx", Value::Int(0));
    let dy = arg_or(&a[1..], kw, 1, "dy", Value::Int(0));
    let (dx, dy) = (int_of(vm, &dx)?, int_of(vm, &dy)?);
    let d = s.data();
    let src = d.pixels();
    let [cx, cy, cw, ch] = s.clip;
    for y in cy..cy + ch {
        for x in cx..cx + cw {
            let (fx, fy) = (x - dx, y - dy);
            if fx >= cx && fy >= cy && fx < cx + cw && fy < cy + ch {
                let i = ((fy * s.w + fx) * 4) as usize;
                d.put(x, y, [src[i], src[i + 1], src[i + 2], src[i + 3]]);
            }
        }
    }
    Ok(Value::None)
}

fn s_set_colorkey(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let c = a.get(1).cloned().unwrap_or(Value::None);
    let key = if py::is_none(&c) {
        None
    } else {
        Some(color_arg(vm, &s, &c)?)
    };
    with(&a[0], |d| d.colorkey = key);
    Ok(Value::None)
}

fn s_get_colorkey(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    match s.colorkey {
        Some(c) => Ok(rgba_tuple(vm, c)),
        None => Ok(Value::None),
    }
}

fn s_set_alpha(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let v = a.get(1).cloned().unwrap_or(Value::None);
    let alpha = if py::is_none(&v) {
        None
    } else {
        Some(py::int(vm, &v)?.clamp(0, 255) as u8)
    };
    with(&a[0], |d| d.surface_alpha = alpha);
    Ok(Value::None)
}

fn s_get_alpha(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(match (s.alpha, s.surface_alpha) {
        (true, _) => Value::Int(s.surface_alpha.unwrap_or(255) as i64),
        (false, Some(v)) => Value::Int(v as i64),
        (false, None) => Value::None,
    })
}

fn s_lock(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let who = a.get(1).cloned().unwrap_or_else(|| a[0].clone());
    with(&a[0], |d| d.locks.push(who));
    Ok(Value::None)
}

fn s_unlock(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let who = a.get(1).cloned().unwrap_or_else(|| a[0].clone());
    with(&a[0], |d| {
        if let Some(i) = d.locks.iter().position(|l| py::same(l, &who)) {
            d.locks.remove(i);
        }
    });
    Ok(Value::None)
}

fn s_get_locked(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(with(&a[0], |d| !d.locks.is_empty()).unwrap_or(false)))
}

fn s_get_locks(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let l = with(&a[0], |d| d.locks.clone()).unwrap_or_default();
    Ok(vm.types.new_tuple(l))
}

fn checked_pos(vm: &mut Vm, s: &SurfRef, pos: &Value) -> PyResult<(i32, i32)> {
    let (x, y) = xy(vm, pos)?;
    if x < 0 || y < 0 || x >= s.w || y >= s.h {
        return Err(vm.index_error("Position outside surface."));
    }
    Ok((x, y))
}

fn s_get_at(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let (x, y) = checked_pos(vm, &s, a.get(1).unwrap_or(&Value::None))?;
    let p = s.data().get(x, y);
    pg_color(vm, p)
}

fn s_set_at(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let (x, y) = checked_pos(vm, &s, a.get(1).unwrap_or(&Value::None))?;
    let c = color_arg(vm, &s, a.get(2).unwrap_or(&Value::None))?;
    let [cx, cy, cw, ch] = s.clip;
    if x >= cx && y >= cy && x < cx + cw && y < cy + ch {
        s.data().put(x, y, c);
    }
    Ok(Value::None)
}

fn s_get_at_mapped(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let (x, y) = checked_pos(vm, &s, a.get(1).unwrap_or(&Value::None))?;
    Ok(Value::Int(map_rgb(&s, s.data().get(x, y))))
}

fn s_map_rgb(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let c = color_arg(vm, &s, a.get(1).unwrap_or(&Value::None))?;
    Ok(Value::Int(map_rgb(&s, c)))
}

fn s_unmap_rgb(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let p = py::int(vm, a.get(1).unwrap_or(&Value::Int(0)))?;
    pg_color(vm, unmap(&s, p))
}

fn s_set_clip(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let r = a.get(1).cloned().unwrap_or(Value::None);
    let clip = if py::is_none(&r) {
        [0, 0, s.w, s.h]
    } else {
        super::pygame::clip_rect(rect_like(vm, &r)?, [0, 0, s.w, s.h])
    };
    with(&a[0], |d| d.clip = clip);
    Ok(Value::None)
}

fn s_get_clip(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    new_rect(vm, s.clip)
}

fn s_subsurface(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let r = super::pygame::rect_from_args(vm, &a[1..])?;
    let [x, y, w, h] = r;
    if x < 0 || y < 0 || w < 0 || h < 0 || x + w > s.w || y + h > s.h {
        return Err(err(vm, "subsurface rectangle outside surface area."));
    }
    let mut d = s.data();
    d.ox = s.ox + x;
    d.oy = s.oy + y;
    d.w = w;
    d.h = h;
    d.clip = [0, 0, w, h];
    d.parent = a[0].clone();
    d.rel = (x, y);
    new_surface(vm, d)
}

fn s_get_parent(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(with(&a[0], |d| d.parent.clone()).unwrap_or(Value::None))
}

fn s_get_abs_parent(_: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let mut cur = a[0].clone();
    loop {
        let p = with(&cur, |d| d.parent.clone()).unwrap_or(Value::None);
        if py::is_none(&p) {
            return Ok(cur);
        }
        cur = p;
    }
}

fn s_get_offset(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let (x, y) = with(&a[0], |d| d.rel).unwrap_or((0, 0));
    Ok(vm.types.new_tuple(vec![Value::Int(x as i64), Value::Int(y as i64)]))
}

fn s_get_abs_offset(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(vm.types.new_tuple(vec![Value::Int(s.ox as i64), Value::Int(s.oy as i64)]))
}

fn s_get_size(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(vm.types.new_tuple(vec![Value::Int(s.w as i64), Value::Int(s.h as i64)]))
}

fn s_get_rect(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let r = new_rect(vm, [0, 0, s.w, s.h])?;
    for (k, v) in kw {
        py::set(vm, &r, k, v.clone())?;
    }
    Ok(r)
}

fn s_get_flags(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(Value::Int(if s.alpha { SRCALPHA } else { 0 }))
}

fn s_get_masks(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let am = if s.alpha { 0xff00_0000 } else { 0 };
    Ok(vm.types.new_tuple(vec![
        Value::Int(0xff),
        Value::Int(0xff00),
        Value::Int(0xff_0000),
        Value::Int(am),
    ]))
}

fn s_get_shifts(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let ash = if s.alpha { 24 } else { 0 };
    Ok(vm.types.new_tuple(vec![Value::Int(0), Value::Int(8), Value::Int(16), Value::Int(ash)]))
}

fn s_get_losses(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let al = if s.alpha { 0 } else { 8 };
    Ok(vm.types.new_tuple(vec![Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(al)]))
}

fn s_fixed_format(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(err(vm, "Surface pixel formats are fixed to 32-bit RGBA in CrySirror."))
}

fn s_get_bounding_rect(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let min_alpha = arg_or(&a[1..], kw, 0, "min_alpha", Value::Int(1));
    let min_alpha = py::int(vm, &min_alpha)?;
    let d = s.data();
    let (mut x0, mut y0, mut x1, mut y1) = (s.w, s.h, -1, -1);
    for y in 0..s.h {
        for x in 0..s.w {
            let p = d.get(x, y);
            let keyed = !s.alpha && s.colorkey.is_some_and(|k| k[..3] == p[..3]);
            if (p[3] as i64) >= min_alpha && !keyed {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x1 < 0 {
        return new_rect(vm, [0, 0, 0, 0]);
    }
    new_rect(vm, [x0, y0, x1 - x0 + 1, y1 - y0 + 1])
}

fn s_get_view(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("NotImplementedError", "Surface.get_view is not implemented."))
}

fn s_get_buffer(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(vm.types.new_bytes(s.data().pixels()))
}

fn s_from_data(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    let data = a.get(1).cloned().unwrap_or(Value::None);
    let b = ops::bytes_of(&data).ok_or_else(|| vm.type_error("from_data requires bytes"))?;
    if b.len() as i64 != (s.w as i64) * (s.h as i64) * 4 {
        return Err(vm.value_error("The data must be the same size as the surface."));
    }
    let d = s.data();
    for y in 0..s.h {
        for x in 0..s.w {
            let i = ((y * s.w + x) * 4) as usize;
            d.put(x, y, [b[i], b[i + 1], b[i + 2], b[i + 3]]);
        }
    }
    Ok(Value::None)
}

fn s_sizeof(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = this(vm, a)?;
    Ok(Value::Int((s.w as i64) * (s.h as i64) * 4))
}

/// `total_size`: bytes of live top-level surfaces (approximated at module
/// level by the value upstream starts from).
fn init_surface_impl(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut fns: Vec<(&'static str, crysirror_python::object::NativeFn)> = Vec::new();
    if pg::sdl3(vm) {
        fns.push(("_init_surface_deallocation", |_, _, _| Ok(Value::None)));
        fns.push(("_quit_surface_deallocation", |_, _, _| Ok(Value::None)));
    }
    let m = new_mod(vm, "surface", &fns);
    let mut methods: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("__init__", s_init),
        ("__repr__", s_repr),
        ("__sizeof__", s_sizeof),
        ("blit", s_blit),
        ("convert", s_convert),
        ("convert_alpha", s_convert_alpha),
        ("copy", s_copy),
        ("fill", s_fill),
        ("scroll", s_scroll),
        ("lock", s_lock),
        ("unlock", s_unlock),
        ("mustlock", |_, _, _| Ok(Value::Bool(false))),
        ("get_locked", s_get_locked),
        ("get_locks", s_get_locks),
        ("get_at", s_get_at),
        ("set_at", s_set_at),
        ("get_at_mapped", s_get_at_mapped),
        ("map_rgb", s_map_rgb),
        ("unmap_rgb", s_unmap_rgb),
        ("set_clip", s_set_clip),
        ("get_clip", s_get_clip),
        ("subsurface", s_subsurface),
        ("get_parent", s_get_parent),
        ("get_abs_parent", s_get_abs_parent),
        ("get_offset", s_get_offset),
        ("get_abs_offset", s_get_abs_offset),
        ("get_size", s_get_size),
        ("get_width", |vm, a, _| Ok(Value::Int(this(vm, a)?.w as i64))),
        ("get_height", |vm, a, _| Ok(Value::Int(this(vm, a)?.h as i64))),
        ("get_rect", s_get_rect),
        ("get_bitsize", |_, _, _| Ok(Value::Int(32))),
        ("get_bytesize", |_, _, _| Ok(Value::Int(4))),
        ("get_flags", s_get_flags),
        ("get_pitch", |vm, a, _| {
            let s = this(vm, a)?;
            Ok(Value::Int(s.buf.borrow().w as i64 * 4))
        }),
        ("get_masks", s_get_masks),
        ("get_shifts", s_get_shifts),
        ("get_losses", s_get_losses),
        ("get_bounding_rect", s_get_bounding_rect),
        ("get_view", s_get_view),
        ("get_buffer", s_get_buffer),
        ("from_data", s_from_data),
    ];
    if !pg::sdl3(vm) {
        methods.extend([
            ("set_colorkey", s_set_colorkey as crysirror_python::object::NativeFn),
            ("get_colorkey", s_get_colorkey),
            ("set_alpha", s_set_alpha),
            ("get_alpha", s_get_alpha),
            ("set_masks", s_fixed_format),
            ("set_shifts", s_fixed_format),
        ]);
    }
    let new = py::builtin(vm, "__new__", s_new);
    let attrs = [("__new__", py::staticmethod(vm, new))];
    let mn = pg::modname(vm, "surface");
    let c = py::class(vm, &mn, "Surface", &[], &methods, &attrs)?;
    set(vm, &m, "Surface", Value::Obj(c));
    let rect = pg::attr(vm, "rect", "Rect")?;
    set(vm, &m, "Rect", rect);
    let e = pg::attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    set(vm, &m, "total_size", Value::Int(0));
    if pg::sdl3(vm) {
        for (n, v) in [
            ("BLEND_ADD", BLEND_RGB_ADD),
            ("BLEND_SUB", BLEND_RGB_SUB),
            ("BLEND_MULT", BLEND_RGB_MULT),
            ("BLEND_MIN", BLEND_RGB_MIN),
            ("BLEND_MAX", BLEND_RGB_MAX),
            ("BLEND_RGBA_ADD", BLEND_RGBA_ADD),
            ("BLEND_RGBA_SUB", BLEND_RGBA_SUB),
            ("BLEND_RGBA_MULT", BLEND_RGBA_MULT),
            ("BLEND_RGBA_MIN", BLEND_RGBA_MIN),
            ("BLEND_RGBA_MAX", BLEND_RGBA_MAX),
            ("BLEND_PREMULTIPLIED", BLEND_PREMULTIPLIED),
            ("BLEND_ALPHA_SDL2", 0x12),
        ] {
            set(vm, &m, n, Value::Int(v));
        }
    } else {
        set(vm, &m, "SRCALPHA", Value::Int(SRCALPHA));
    }
    Ok(m)
}

pub fn init_surface(vm: &mut Vm) -> PyResult<ObjRef> {
    init_surface_impl(vm)
}

// image --------------------------------------------------------------------------

fn read_source(vm: &mut Vm, fi: &Value) -> PyResult<(Vec<u8>, String)> {
    if let Some(name) = fi.as_str() {
        let f = py::import_attr(vm, "builtins", "open")?;
        let fo = vm.call(&f, vec![fi.clone(), vm.types.new_str("rb")], Vec::new())?;
        let data = py::method(vm, &fo, "read", Vec::new())?;
        let _ = py::method(vm, &fo, "close", Vec::new());
        let b = ops::bytes_of(&data).ok_or_else(|| vm.type_error("read() must return bytes"))?;
        return Ok((b.to_vec(), name.to_owned()));
    }
    let data = py::method(vm, fi, "read", Vec::new())?;
    let b = ops::bytes_of(&data).ok_or_else(|| vm.type_error("read() must return bytes"))?;
    let name = py::get_opt(vm, fi, "name")?
        .and_then(|n| n.as_str().map(str::to_owned))
        .unwrap_or_default();
    Ok((b.to_vec(), name))
}

fn i_load(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let fi = arg_or(a, kw, 0, "fi", Value::None);
    let (bytes, name) = read_source(vm, &fi)?;
    let img = crysirror_image::decode_standard(&bytes)
        .map_err(|e| err(vm, format!("Couldn't load image {name}: {e}")))?;
    let alpha = img.rgba.chunks(4).any(|p| p[3] != 255);
    let s = surface_from_rgba(vm, img.width as i32, img.height as i32, img.rgba, alpha)?;
    let size = arg_or(a, kw, 2, "size", Value::None);
    if !py::is_none(&size) {
        let (w, h) = xy(vm, &size)?;
        return scale_surface(vm, &s, w, h, true);
    }
    Ok(s)
}

fn i_save(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let sv = arg_or(a, kw, 0, "surface", Value::None);
    let s = surf(vm, &sv)?;
    let filename = arg_or(a, kw, 1, "filename", Value::None);
    let name = py::string(vm, &filename)?;
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let img = crysirror_image::ImageSurface {
        width: s.w as u32,
        height: s.h as u32,
        rgba: s.data().pixels(),
    };
    let bytes = match ext.as_str() {
        "png" => crysirror_image::encode_png(&img),
        "jpg" | "jpeg" => crysirror_image::encode_jpeg(&img, 90),
        _ => return Err(err(vm, format!("Saving to {ext} files is not supported (use PNG or JPEG)."))),
    }
    .map_err(|e| err(vm, e.to_string()))?;
    let f = py::import_attr(vm, "builtins", "open")?;
    let fo = vm.call(&f, vec![filename, vm.types.new_str("wb")], Vec::new())?;
    let b = vm.types.new_bytes(bytes);
    py::method(vm, &fo, "write", vec![b])?;
    py::method(vm, &fo, "close", Vec::new())?;
    Ok(Value::None)
}

pub fn init_image(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "image",
        &[
            ("init", |_, _, _| Ok(Value::None)),
            ("quit", |_, _, _| Ok(Value::None)),
            ("load", i_load),
            ("save", i_save),
            ("get_extended", |_, _, _| Ok(Value::Bool(true))),
        ],
    );
    let e = pg::attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if !pg::sdl3(vm) {
        for (n, v) in [
            ("INIT_JPG", 1),
            ("INIT_PNG", 2),
            ("INIT_TIF", 4),
            ("INIT_WEBP", 8),
            ("INIT_JXL", 16),
            ("INIT_AVIF", 32),
        ] {
            set(vm, &m, n, Value::Int(v));
        }
        // The decoder handles every format at once.
        let hi = py::builtin(vm, "has_init", |_, _, _| Ok(Value::Bool(true)));
        set(vm, &m, "has_init", hi);
        for n in ["bytes_", "unicode_", "filesystem_encode"] {
            let v = pg::attr(vm, "compat", n)?;
            set(vm, &m, n, v);
        }
    }
    Ok(m)
}

// transform ----------------------------------------------------------------------

fn sample_bilinear(src: &[u8], w: i32, h: i32, x: f64, y: f64) -> [u8; 4] {
    let x = x.clamp(0., (w - 1).max(0) as f64);
    let y = y.clamp(0., (h - 1).max(0) as f64);
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let px = |x: i32, y: i32, c: usize| src[((y * w + x) * 4) as usize + c] as f64;
    let mut o = [0u8; 4];
    for (c, out) in o.iter_mut().enumerate() {
        let top = px(x0, y0, c) * (1. - fx) + px(x1, y0, c) * fx;
        let bot = px(x0, y1, c) * (1. - fx) + px(x1, y1, c) * fx;
        *out = (top * (1. - fy) + bot * fy).round().clamp(0., 255.) as u8;
    }
    o
}

/// Nearest (`scale`, SDL_BlitScaled) or bilinear (`smoothscale`) scaling.
fn scale_surface(vm: &mut Vm, sv: &Value, w: i32, h: i32, smooth: bool) -> PyResult<Value> {
    let s = surf(vm, sv)?;
    if w < 0 || h < 0 {
        return Err(vm.value_error("Cannot scale to negative size"));
    }
    let src = s.data().pixels();
    let mut out = Vec::with_capacity((w * h * 4).max(0) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = if smooth {
                let sx = (x as f64 + 0.5) * s.w as f64 / w as f64 - 0.5;
                let sy = (y as f64 + 0.5) * s.h as f64 / h as f64 - 0.5;
                sample_bilinear(&src, s.w, s.h, sx, sy)
            } else {
                let sx = (x as i64 * s.w as i64 / w.max(1) as i64) as i32;
                let sy = (y as i64 * s.h as i64 / h.max(1) as i64) as i32;
                let i = ((sy * s.w + sx) * 4) as usize;
                [src[i], src[i + 1], src[i + 2], src[i + 3]]
            };
            out.extend_from_slice(&p);
        }
    }
    let mut d = SurfData::new(w, h, s.alpha);
    d.buf.borrow_mut().px = out;
    d.colorkey = s.colorkey;
    new_surface(vm, d)
}

fn into_dest(vm: &mut Vm, made: Value, dest: &Value) -> PyResult<Value> {
    if py::is_none(dest) {
        return Ok(made);
    }
    let d = surf(vm, dest)?;
    let m = surf(vm, &made)?;
    if (d.w, d.h) != (m.w, m.h) {
        return Err(vm.value_error("Destination surface not the given width or height."));
    }
    let px = m.data().pixels();
    let dd = d.data();
    for y in 0..d.h {
        for x in 0..d.w {
            let i = ((y * d.w + x) * 4) as usize;
            dd.put(x, y, [px[i], px[i + 1], px[i + 2], px[i + 3]]);
        }
    }
    Ok(dest.clone())
}

fn t_scale(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let size = arg_or(a, kw, 1, "size", Value::None);
    let dest = arg_or(a, kw, 2, "DestSurface", Value::None);
    let (w, h) = xy(vm, &size)?;
    let made = scale_surface(vm, &s, w, h, false)?;
    into_dest(vm, made, &dest)
}

fn t_smoothscale(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let size = arg_or(a, kw, 1, "size", Value::None);
    let dest = arg_or(a, kw, 2, "DestSurface", Value::None);
    let (w, h) = xy(vm, &size)?;
    let made = scale_surface(vm, &s, w, h, true)?;
    into_dest(vm, made, &dest)
}

fn t_flip(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let sv = arg_or(a, kw, 0, "surface", Value::None);
    let s = surf(vm, &sv)?;
    let fx = arg_or(a, kw, 1, "xbool", Value::Bool(false));
    let fy = arg_or(a, kw, 2, "ybool", Value::Bool(false));
    let (fx, fy) = (ops::truthy(vm, &fx)?, ops::truthy(vm, &fy)?);
    let src = s.data();
    let d = SurfData::new(s.w, s.h, s.alpha);
    for y in 0..s.h {
        for x in 0..s.w {
            let tx = if fx { s.w - 1 - x } else { x };
            let ty = if fy { s.h - 1 - y } else { y };
            d.put(tx, ty, src.get(x, y));
        }
    }
    new_surface(vm, d)
}

/// SDL_gfx `rotozoomSurface`: rotation by `angle` degrees
/// (counter-clockwise) and zoom, expanding to the rotated bounds.
fn rotozoom(vm: &mut Vm, sv: &Value, angle: f64, zoom: f64, smooth: bool) -> PyResult<Value> {
    let s = surf(vm, sv)?;
    let (sin, cos) = angle.to_radians().sin_cos();
    let (w, h) = (s.w as f64 * zoom, s.h as f64 * zoom);
    let nw = (w * cos.abs() + h * sin.abs()).ceil().max(1.) as i32;
    let nh = (w * sin.abs() + h * cos.abs()).ceil().max(1.) as i32;
    let src = s.data().pixels();
    let d = SurfData::new(nw, nh, true);
    let (cx, cy) = (nw as f64 / 2., nh as f64 / 2.);
    let (scx, scy) = (s.w as f64 / 2., s.h as f64 / 2.);
    for y in 0..nh {
        for x in 0..nw {
            let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            // Inverse rotation (screen y points down).
            let sx = (dx * cos - dy * sin) / zoom + scx - 0.5;
            let sy = (dx * sin + dy * cos) / zoom + scy - 0.5;
            if sx < -0.5 || sy < -0.5 || sx > s.w as f64 - 0.5 || sy > s.h as f64 - 0.5 {
                continue;
            }
            let mut p = if smooth {
                sample_bilinear(&src, s.w, s.h, sx, sy)
            } else {
                let (ix, iy) = (sx.round() as i32, sy.round() as i32);
                let i = ((iy.clamp(0, s.h - 1) * s.w + ix.clamp(0, s.w - 1)) * 4) as usize;
                [src[i], src[i + 1], src[i + 2], src[i + 3]]
            };
            if !s.alpha {
                p[3] = 255;
            }
            d.put(x, y, p);
        }
    }
    new_surface(vm, d)
}

fn t_rotate(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let angle = arg_or(a, kw, 1, "angle", Value::Int(0));
    let angle = py::f64_of(vm, &angle)?;
    rotozoom(vm, &s, angle, 1.0, false)
}

fn t_rotozoom(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let angle = arg_or(a, kw, 1, "angle", Value::Int(0));
    let scale = arg_or(a, kw, 2, "scale", Value::Float(1.));
    let smooth = arg_or(a, kw, 3, "smooth", Value::Int(1));
    let (angle, scale) = (py::f64_of(vm, &angle)?, py::f64_of(vm, &scale)?);
    let smooth = ops::truthy(vm, &smooth)?;
    rotozoom(vm, &s, angle, scale, smooth)
}

/// AdvMAME2x (`scale2x`).
fn t_scale2x(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let sv = arg_or(a, kw, 0, "surface", Value::None);
    let dest = arg_or(a, kw, 1, "DestSurface", Value::None);
    let s = surf(vm, &sv)?;
    let src = s.data();
    let d = SurfData::new(s.w * 2, s.h * 2, s.alpha);
    let at = |x: i32, y: i32| src.get(x.clamp(0, s.w - 1), y.clamp(0, s.h - 1));
    for y in 0..s.h {
        for x in 0..s.w {
            let (b, dd, e, f, hh) = (at(x, y - 1), at(x - 1, y), at(x, y), at(x + 1, y), at(x, y + 1));
            let (e0, e1, e2, e3) = if b != hh && dd != f {
                (
                    if dd == b { dd } else { e },
                    if b == f { f } else { e },
                    if dd == hh { dd } else { e },
                    if hh == f { f } else { e },
                )
            } else {
                (e, e, e, e)
            };
            d.put(2 * x, 2 * y, e0);
            d.put(2 * x + 1, 2 * y, e1);
            d.put(2 * x, 2 * y + 1, e2);
            d.put(2 * x + 1, 2 * y + 1, e3);
        }
    }
    let made = new_surface(vm, d)?;
    into_dest(vm, made, &dest)
}

/// Ren'Py's `_diff(dest, a, b, same_color, different_color)`.
fn t_diff(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    if a.len() < 5 {
        return Err(vm.type_error("_diff() takes exactly 5 arguments"));
    }
    let (d, sa, sb) = (surf(vm, &a[0])?, surf(vm, &a[1])?, surf(vm, &a[2])?);
    if (d.w, d.h) != (sa.w, sa.h) || (d.w, d.h) != (sb.w, sb.h) {
        return Err(err(vm, "Surface sizes do not match."));
    }
    let same = color_arg(vm, &d, &a[3])?;
    let diff = color_arg(vm, &d, &a[4])?;
    let (dd, da, db) = (d.data(), sa.data(), sb.data());
    let mut n = 0;
    for y in 0..d.h {
        for x in 0..d.w {
            if map_rgb(&sa, da.get(x, y)) == map_rgb(&sb, db.get(x, y)) {
                dd.put(x, y, same);
            } else {
                dd.put(x, y, diff);
                n += 1;
            }
        }
    }
    Ok(Value::Int(n))
}

pub fn init_transform(vm: &mut Vm) -> PyResult<ObjRef> {
    let mut fns: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("_diff", t_diff),
        ("flip", t_flip),
        ("scale", t_scale),
        ("rotate", t_rotate),
        ("rotozoom", t_rotozoom),
        ("smoothscale", t_smoothscale),
    ];
    if !pg::sdl3(vm) {
        fns.push(("scale2x", t_scale2x));
    }
    let m = new_mod(vm, "transform", &fns);
    let e = pg::attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    Ok(m)
}

// gfxdraw ------------------------------------------------------------------------

fn target(vm: &mut Vm, v: &Value) -> PyResult<SurfData> {
    Ok(surf(vm, v)?.data())
}

fn pcolor(vm: &mut Vm, s: &SurfData, v: &Value) -> PyResult<[u8; 4]> {
    let r = SurfRef {
        buf: s.buf.clone(),
        ox: s.ox,
        oy: s.oy,
        w: s.w,
        h: s.h,
        alpha: s.alpha,
        colorkey: None,
        surface_alpha: None,
        clip: s.clip,
    };
    color_arg(vm, &r, v)
}

fn hline(s: &SurfData, x1: i32, x2: i32, y: i32, c: [u8; 4]) {
    for x in x1.min(x2)..=x1.max(x2) {
        s.plot(x, y, c, true);
    }
}

fn line(s: &SurfData, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut e = dx + dy;
    loop {
        s.plot(x0, y0, c, true);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * e;
        if e2 >= dy {
            e += dy;
            x0 += sx;
        }
        if e2 <= dx {
            e += dx;
            y0 += sy;
        }
    }
}

/// Even-odd scanline fill of a polygon (SDL_gfx `filledPolygonColor`).
fn fill_polygon(s: &SurfData, pts: &[(f64, f64)], mut pixel: impl FnMut(i32, i32) -> [u8; 4]) {
    if pts.len() < 3 {
        return;
    }
    let miny = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor() as i32;
    let maxy = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max).ceil() as i32;
    for y in miny..=maxy {
        let yc = y as f64;
        let mut xs = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let (lo, hi) = if a.1 < b.1 { (a, b) } else { (b, a) };
            if yc >= lo.1 && yc < hi.1 && hi.1 > lo.1 {
                xs.push(lo.0 + (yc - lo.1) * (hi.0 - lo.0) / (hi.1 - lo.1));
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        for pair in xs.chunks(2) {
            if let [a, b] = pair {
                for x in a.round() as i32..=b.round() as i32 {
                    let c = pixel(x, y);
                    s.plot(x, y, c, true);
                }
            }
        }
    }
}

fn ellipse_points(cx: i32, cy: i32, rx: i32, ry: i32) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let n = ((rx.max(ry) as f64) * 8.).max(16.) as i32;
    for i in 0..n {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        out.push((cx + (rx as f64 * t.cos()).round() as i32, cy + (ry as f64 * t.sin()).round() as i32));
    }
    out
}

fn ints(vm: &mut Vm, a: &[Value], from: usize, n: usize) -> PyResult<Vec<i32>> {
    (from..from + n)
        .map(|i| int_of(vm, a.get(i).unwrap_or(&Value::Int(0))))
        .collect()
}

fn points(vm: &mut Vm, v: &Value) -> PyResult<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for p in py::items(vm, v)? {
        let xy = py::items(vm, &p)?;
        if xy.len() < 2 {
            return Err(vm.type_error("points must be pairs"));
        }
        out.push((py::f64_of(vm, &xy[0])?, py::f64_of(vm, &xy[1])?));
    }
    Ok(out)
}

fn arc_points(cx: i32, cy: i32, r: i32, start: f64, end: f64) -> Vec<(i32, i32)> {
    let mut end = end;
    while end < start {
        end += 360.;
    }
    let n = ((end - start).abs() * r.max(1) as f64 / 20.).max(8.) as i32;
    (0..=n)
        .map(|i| {
            let t = (start + (end - start) * i as f64 / n as f64).to_radians();
            (cx + (r as f64 * t.cos()).round() as i32, cy + (r as f64 * t.sin()).round() as i32)
        })
        .collect()
}

fn g_dispatch(vm: &mut Vm, a: &[Value], _: Kw, name: &str) -> PyResult<Value> {
    let s = target(vm, a.first().unwrap_or(&Value::None))?;
    let last = a.last().cloned().unwrap_or(Value::None);
    let poly = |s: &SurfData, pts: &[(i32, i32)], c: [u8; 4], closed: bool| {
        for i in 0..pts.len().saturating_sub(1) {
            line(s, pts[i].0, pts[i].1, pts[i + 1].0, pts[i + 1].1, c);
        }
        if closed && pts.len() > 2 {
            let (f, l) = (pts[0], pts[pts.len() - 1]);
            line(s, l.0, l.1, f.0, f.1, c);
        }
    };
    match name {
        "pixel" => {
            let v = ints(vm, a, 1, 2)?;
            let c = pcolor(vm, &s, &last)?;
            s.plot(v[0], v[1], c, true);
        }
        "hline" => {
            let v = ints(vm, a, 1, 3)?;
            let c = pcolor(vm, &s, &last)?;
            hline(&s, v[0], v[1], v[2], c);
        }
        "vline" => {
            let v = ints(vm, a, 1, 3)?;
            let c = pcolor(vm, &s, &last)?;
            for y in v[1].min(v[2])..=v[1].max(v[2]) {
                s.plot(v[0], y, c, true);
            }
        }
        "rectangle" | "box" | "rounded_rectangle" | "rounded_box" => {
            let r = rect_like(vm, &a[1])?;
            let c = pcolor(vm, &s, &last)?;
            let rad = if name.starts_with("rounded") {
                int_of(vm, &a[2])?.min(r[2] / 2).min(r[3] / 2).max(0)
            } else {
                0
            };
            let [x, y, w, h] = r;
            let inside = |px: i32, py: i32| {
                if rad == 0 {
                    return true;
                }
                let cx = px.clamp(x + rad, x + w - 1 - rad);
                let cy = py.clamp(y + rad, y + h - 1 - rad);
                let (dx, dy) = ((px - cx) as f64, (py - cy) as f64);
                dx * dx + dy * dy <= (rad as f64 + 0.5).powi(2)
            };
            let filled = name.ends_with("box");
            for py_ in y..y + h {
                for px in x..x + w {
                    let edge = px == x || py_ == y || px == x + w - 1 || py_ == y + h - 1;
                    let on = if filled {
                        inside(px, py_)
                    } else if rad == 0 {
                        edge
                    } else {
                        inside(px, py_)
                            && !(inside(px - 1, py_) && inside(px + 1, py_) && inside(px, py_ - 1) && inside(px, py_ + 1))
                            || (edge && inside(px, py_))
                    };
                    if on {
                        s.plot(px, py_, c, true);
                    }
                }
            }
        }
        "line" | "aaline" => {
            let v = ints(vm, a, 1, 4)?;
            let c = pcolor(vm, &s, &last)?;
            line(&s, v[0], v[1], v[2], v[3], c);
        }
        "thick_line" => {
            let v = ints(vm, a, 1, 5)?;
            let c = pcolor(vm, &s, &last)?;
            let (x1, y1, x2, y2, w) = (v[0] as f64, v[1] as f64, v[2] as f64, v[3] as f64, v[4].max(1) as f64);
            if w <= 1. {
                line(&s, v[0], v[1], v[2], v[3], c);
            } else {
                let (dx, dy) = (x2 - x1, y2 - y1);
                let len = (dx * dx + dy * dy).sqrt().max(1e-9);
                let (nx, ny) = (-dy / len * w / 2., dx / len * w / 2.);
                let pts = [(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)];
                fill_polygon(&s, &pts, |_, _| c);
            }
        }
        "circle" | "aacircle" => {
            let v = ints(vm, a, 1, 3)?;
            let c = pcolor(vm, &s, &last)?;
            let pts = ellipse_points(v[0], v[1], v[2], v[2]);
            poly(&s, &pts, c, true);
        }
        "ellipse" | "aaellipse" => {
            let v = ints(vm, a, 1, 4)?;
            let c = pcolor(vm, &s, &last)?;
            let pts = ellipse_points(v[0], v[1], v[2], v[3]);
            poly(&s, &pts, c, true);
        }
        "filled_circle" | "filled_ellipse" => {
            let n = if name == "filled_circle" { 3 } else { 4 };
            let v = ints(vm, a, 1, n)?;
            let (rx, ry) = if n == 3 { (v[2], v[2]) } else { (v[2], v[3]) };
            let c = pcolor(vm, &s, &last)?;
            for dy in -ry..=ry {
                let span = if ry == 0 {
                    rx
                } else {
                    (rx as f64 * (1. - (dy as f64 / ry as f64).powi(2)).max(0.).sqrt()).round() as i32
                };
                hline(&s, v[0] - span, v[0] + span, v[1] + dy, c);
            }
        }
        "arc" | "pie" | "filled_pie" => {
            let v = ints(vm, a, 1, 5)?;
            let c = pcolor(vm, &s, &last)?;
            let pts = arc_points(v[0], v[1], v[2], v[3] as f64, v[4] as f64);
            if name == "arc" {
                poly(&s, &pts, c, false);
            } else {
                let mut all = vec![(v[0], v[1])];
                all.extend(pts);
                if name == "pie" {
                    poly(&s, &all, c, true);
                } else {
                    let f: Vec<(f64, f64)> = all.iter().map(|p| (p.0 as f64, p.1 as f64)).collect();
                    fill_polygon(&s, &f, |_, _| c);
                }
            }
        }
        "trigon" | "aatrigon" | "filled_trigon" => {
            let v = ints(vm, a, 1, 6)?;
            let c = pcolor(vm, &s, &last)?;
            let pts = [(v[0], v[1]), (v[2], v[3]), (v[4], v[5])];
            if name == "filled_trigon" {
                let f: Vec<(f64, f64)> = pts.iter().map(|p| (p.0 as f64, p.1 as f64)).collect();
                fill_polygon(&s, &f, |_, _| c);
            } else {
                poly(&s, &pts, c, true);
            }
        }
        "polygon" | "aapolygon" | "filled_polygon" => {
            let pts = points(vm, &a[1])?;
            let c = pcolor(vm, &s, &last)?;
            if name == "filled_polygon" {
                fill_polygon(&s, &pts, |_, _| c);
            } else {
                let p: Vec<(i32, i32)> = pts.iter().map(|p| (p.0 as i32, p.1 as i32)).collect();
                poly(&s, &p, c, true);
            }
        }
        "textured_polygon" => {
            let pts = points(vm, &a[1])?;
            let tex = surf(vm, &a[2])?;
            let (tx, ty) = (int_of(vm, &a[3])?, int_of(vm, &a[4])?);
            let td = tex.data();
            let (tw, th) = (tex.w.max(1), tex.h.max(1));
            fill_polygon(&s, &pts, |x, y| td.get((x - tx).rem_euclid(tw), (y - ty).rem_euclid(th)));
        }
        "bezier" => {
            let pts = points(vm, &a[1])?;
            let steps = int_of(vm, &a[2])?;
            let c = pcolor(vm, &s, &last)?;
            if pts.len() < 3 || steps < 2 {
                return Err(err(vm, "bezier requires at least 3 points and 2 steps"));
            }
            let mut out = Vec::new();
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                let mut p = pts.clone();
                while p.len() > 1 {
                    p = p.windows(2).map(|w| (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t)).collect();
                }
                out.push((p[0].0.round() as i32, p[0].1.round() as i32));
            }
            poly(&s, &out, c, false);
        }
        _ => unreachable!(),
    }
    Ok(Value::None)
}

macro_rules! gfx {
    ($($name:literal),* $(,)?) => {
        &[$(($name, |vm: &mut Vm, a: &[Value], k: Kw| g_dispatch(vm, a, k, $name))),*]
    };
}

const GFX: &[(&str, crysirror_python::object::NativeFn)] = gfx!(
    "pixel", "hline", "vline", "rectangle", "rounded_rectangle", "box", "rounded_box", "line", "aaline",
    "thick_line", "circle", "arc", "aacircle", "filled_circle", "ellipse", "aaellipse", "filled_ellipse",
    "pie", "filled_pie", "trigon", "aatrigon", "filled_trigon", "polygon", "aapolygon", "filled_polygon",
    "textured_polygon", "bezier",
);

fn canvas_init(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let s = a.get(1).cloned().unwrap_or(Value::None);
    surf(vm, &s)?;
    py::set(vm, &a[0], "surface", s)?;
    Ok(Value::None)
}

fn canvas_call(vm: &mut Vm, a: &[Value], kw: Kw, name: &str) -> PyResult<Value> {
    let s = py::get(vm, &a[0], "surface")?;
    let mut args = vec![s];
    args.extend_from_slice(&a[1..]);
    g_dispatch(vm, &args, kw, name)
}

macro_rules! canvas_methods {
    ($($name:literal),* $(,)?) => {
        &[$(($name, |vm: &mut Vm, a: &[Value], k: Kw| canvas_call(vm, a, k, $name))),*]
    };
}

const CANVAS: &[(&str, crysirror_python::object::NativeFn)] = canvas_methods!(
    "pixel", "hline", "vline", "rectangle", "rounded_rectangle", "box", "rounded_box", "line", "aaline",
    "thick_line", "circle", "arc", "aacircle", "filled_circle", "ellipse", "aaellipse", "filled_ellipse",
    "pie", "filled_pie", "trigon", "aatrigon", "filled_trigon", "polygon", "aapolygon", "filled_polygon",
    "textured_polygon", "bezier",
);

pub fn init_gfxdraw(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(vm, "gfxdraw", GFX);
    let rect = pg::attr(vm, "rect", "Rect")?;
    set(vm, &m, "Rect", rect);
    let e = pg::attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if pg::sdl3(vm) {
        let mut methods = CANVAS.to_vec();
        methods.push(("__init__", canvas_init));
        let mn = pg::modname(vm, "gfxdraw");
        let c = py::class(vm, &mn, "GFXCanvas", &[], &methods, &[])?;
        set(vm, &m, "GFXCanvas", Value::Obj(c));
    }
    Ok(m)
}

// draw ---------------------------------------------------------------------------

fn gfx(vm: &mut Vm, name: &str, args: Vec<Value>) -> PyResult<()> {
    g_dispatch(vm, &args, &[], name)?;
    Ok(())
}

fn clip_dirty(vm: &mut Vm, s: &Value, r: [i32; 4]) -> PyResult<Value> {
    let sr = surf(vm, s)?;
    new_rect(vm, super::pygame::clip_rect(r, [0, 0, sr.w, sr.h]))
}

fn iv(x: i32) -> Value {
    Value::Int(x as i64)
}

fn d_rect(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let r = arg_or(a, kw, 2, "rect", Value::None);
    let width = arg_or(a, kw, 3, "width", Value::Int(0));
    let width = int_of(vm, &width)?;
    let [x, y, w, h] = rect_like(vm, &r)?;
    if width == 0 {
        let rv = new_rect(vm, [x, y, w, h])?;
        gfx(vm, "box", vec![s.clone(), rv, color])?;
    } else {
        let rv = new_rect(vm, [x, y, w, h])?;
        gfx(vm, "rectangle", vec![s.clone(), rv, color.clone()])?;
        for n in 1..width {
            let r1 = new_rect(vm, [x - n, y - n, w + 2 * n, h + 2 * n])?;
            gfx(vm, "rectangle", vec![s.clone(), r1, color.clone()])?;
            let r2 = new_rect(vm, [x + n, y + n, w - 2 * n, h - 2 * n])?;
            gfx(vm, "rectangle", vec![s.clone(), r2, color.clone()])?;
        }
    }
    clip_dirty(vm, &s, [x - width, y - width, w + 2 * width, h + 2 * width])
}

fn d_line(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let p0 = arg_or(a, kw, 2, "start_pos", Value::None);
    let p1 = arg_or(a, kw, 3, "end_pos", Value::None);
    let width = arg_or(a, kw, 4, "width", Value::Int(1));
    let (x0, y0) = xy(vm, &p0)?;
    let (x1, y1) = xy(vm, &p1)?;
    let w = int_of(vm, &width)?;
    gfx(vm, "thick_line", vec![s.clone(), iv(x0), iv(y0), iv(x1), iv(y1), iv(w), color])?;
    let r = super::pygame::union([x0, y0, w, w], [x1, y1, w, w]);
    clip_dirty(vm, &s, r)
}

fn d_lines(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let closed = arg_or(a, kw, 2, "closed", Value::Bool(false));
    let pl = arg_or(a, kw, 3, "pointlist", Value::None);
    let width = arg_or(a, kw, 4, "width", Value::Int(1));
    let pts = py::items(vm, &pl)?;
    let w = int_of(vm, &width)?;
    let (fx, fy) = xy(vm, pts.first().ok_or_else(|| vm.index_error("list index out of range"))?)?;
    let mut dirty = [fx, fy, w, w];
    for i in 0..pts.len().saturating_sub(1) {
        d_line(vm, &[s.clone(), color.clone(), pts[i].clone(), pts[i + 1].clone(), width.clone()], &[])?;
        let (x, y) = xy(vm, &pts[i + 1])?;
        dirty = super::pygame::union(dirty, [x, y, w, w]);
    }
    if ops::truthy(vm, &closed)? && pts.len() > 1 {
        d_line(vm, &[s.clone(), color, pts[pts.len() - 1].clone(), pts[0].clone(), width], &[])?;
    }
    clip_dirty(vm, &s, dirty)
}

fn d_polygon(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let pl = arg_or(a, kw, 2, "pointlist", Value::None);
    let width = arg_or(a, kw, 3, "width", Value::Int(0));
    if int_of(vm, &width)? != 0 {
        return d_lines(vm, &[s, color, Value::Bool(true), pl, width], &[]);
    }
    gfx(vm, "filled_polygon", vec![s.clone(), pl.clone(), color])?;
    let mut dirty: Option<[i32; 4]> = None;
    for p in py::items(vm, &pl)? {
        let (x, y) = xy(vm, &p)?;
        dirty = Some(dirty.map_or([x, y, 1, 1], |d| super::pygame::union(d, [x, y, 1, 1])));
    }
    clip_dirty(vm, &s, dirty.unwrap_or([0, 0, 0, 0]))
}

fn d_circle(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let pos = arg_or(a, kw, 2, "pos", Value::None);
    let radius = arg_or(a, kw, 3, "radius", Value::Int(0));
    let width = arg_or(a, kw, 4, "width", Value::Int(0));
    let (x, y) = xy(vm, &pos)?;
    let (r, w) = (int_of(vm, &radius)?, int_of(vm, &width)?);
    if w == 0 {
        gfx(vm, "filled_circle", vec![s.clone(), iv(x), iv(y), iv(r), color])?;
        return clip_dirty(vm, &s, [x - r, y - r, r * 2, r * 2]);
    }
    gfx(vm, "circle", vec![s.clone(), iv(x), iv(y), iv(r), color.clone()])?;
    for n in 1..w {
        for dx in [0, 1, -1] {
            gfx(vm, "circle", vec![s.clone(), iv(x + dx), iv(y), iv(r - n), color.clone()])?;
        }
    }
    clip_dirty(vm, &s, [x - r - w, y - r - w, r * 2 + w, r * 2 + w])
}

fn d_ellipse(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let r = arg_or(a, kw, 2, "rect", Value::None);
    let width = arg_or(a, kw, 3, "width", Value::Int(0));
    let [x, y, rx, ry] = rect_like(vm, &r)?;
    let w = int_of(vm, &width)?;
    if w == 0 {
        gfx(vm, "filled_ellipse", vec![s.clone(), iv(x), iv(y), iv(rx), iv(ry), color])?;
        return clip_dirty(vm, &s, [x - rx, y - ry, rx * 2, ry * 2]);
    }
    gfx(vm, "ellipse", vec![s.clone(), iv(x), iv(y), iv(rx), iv(ry), color.clone()])?;
    for n in 1..w {
        for dx in [0, 1, -1] {
            gfx(vm, "ellipse", vec![s.clone(), iv(x + dx), iv(y), iv(rx - n), iv(ry - n), color.clone()])?;
        }
    }
    clip_dirty(vm, &s, [x - rx - w, y - ry - w, rx * 2 + w, ry * 2 + w])
}

fn d_aaline(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let p0 = arg_or(a, kw, 2, "startpos", Value::None);
    let p1 = arg_or(a, kw, 3, "endpos", Value::None);
    let (x1, y1) = xy(vm, &p0)?;
    let (x2, y2) = xy(vm, &p1)?;
    gfx(vm, "aaline", vec![s.clone(), iv(x1), iv(y1), iv(x2), iv(y2), color])?;
    clip_dirty(vm, &s, [x1, y1, x2 - x1, y2 - y1])
}

fn d_aalines(vm: &mut Vm, a: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(a, kw, 0, "surface", Value::None);
    let color = arg_or(a, kw, 1, "color", Value::None);
    let closed = arg_or(a, kw, 2, "closed", Value::Bool(false));
    let pl = arg_or(a, kw, 3, "pointlist", Value::None);
    let pts = py::items(vm, &pl)?;
    let (fx, fy) = xy(vm, pts.first().ok_or_else(|| vm.index_error("list index out of range"))?)?;
    let mut dirty = [fx, fy, 1, 1];
    for i in 0..pts.len().saturating_sub(1) {
        let r = d_aaline(vm, &[s.clone(), color.clone(), pts[i].clone(), pts[i + 1].clone()], &[])?;
        if let Some(r) = super::pygame::rect_of(&r) {
            dirty = super::pygame::union(dirty, r);
        }
    }
    if ops::truthy(vm, &closed)? && pts.len() > 1 {
        d_aaline(vm, &[s.clone(), color, pts[pts.len() - 1].clone(), pts[0].clone()], &[])?;
    }
    clip_dirty(vm, &s, dirty)
}

pub fn init_draw(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = new_mod(
        vm,
        "draw",
        &[
            ("rect", d_rect),
            ("polygon", d_polygon),
            ("circle", d_circle),
            ("ellipse", d_ellipse),
            // Upstream pygame_sdl2 leaves `draw.arc` unimplemented.
            ("arc", |vm, _, _| Err(err(vm, "Not implemented."))),
            ("line", d_line),
            ("lines", d_lines),
            ("aaline", d_aaline),
            ("aalines", d_aalines),
        ],
    );
    let rect = pg::attr(vm, "rect", "Rect")?;
    set(vm, &m, "Rect", rect);
    let e = pg::attr(vm, "error", "error")?;
    set(vm, &m, "error", e);
    if pg::sdl3(vm) {
        let g = py::import(vm, "pygame_sdl2.gfxdraw")?;
        set(vm, &m, "gfxdraw", g);
    }
    Ok(m)
}
