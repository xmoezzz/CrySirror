// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `textrender.dll` TextRenderBase: TJS value/callback mapping only. Layout is
//! the shared `crysirror_text::markup` engine operation; script handlers
//! (`onGetTextWidth`, `onFontChange`, `onEval`, `onLabel`) are its host.
use crate::natives::{Rt, has_member, methods, need_int, object_arg, this_of};
use crysirror_text::markup::{self, Apply, FIELDS, MarkupHost, MarkupLayout, OPTIONS};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::BTreeMap;

const OUTPUTS: &[&str] = &[
    "maxScrollLine",
    "maxScrollOffset",
    "renderBottom",
    "renderCount",
    "renderDelay",
    "renderLeft",
    "renderLines",
    "renderOver",
    "renderRight",
    "renderText",
    "renderTop",
];

#[derive(Default)]
pub struct State {
    layouts: BTreeMap<usize, MarkupLayout>,
    /// Instances inside a layout operation: their entry in `layouts` is a
    /// read-only snapshot for callbacks; writes and reentry fail.
    busy: std::collections::BTreeSet<usize>,
}

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("TextRenderBase: {e:#}"))
}
fn to_value(v: &Variant) -> Result<markup::Value> {
    Ok(match v {
        Variant::Integer(i) => markup::Value::Int(*i),
        Variant::Real(r) => markup::Value::Real(*r),
        Variant::String(s) => markup::Value::Text(s.encode_utf16().collect()),
        other => markup::Value::Real(other.to_real()?),
    })
}
fn from_value(v: &markup::Value) -> Variant {
    match v {
        markup::Value::Int(i) => Variant::Integer(*i),
        markup::Value::Real(r) => Variant::Real(*r),
        markup::Value::Text(t) => Variant::String(String::from_utf16_lossy(t)),
    }
}
fn s16(t: &[u16]) -> Variant {
    Variant::String(String::from_utf16_lossy(t))
}
fn real(n: f32) -> Variant {
    Variant::Real(f64::from(n))
}

/// Script callbacks on the TextRender instance.
struct Host<'a> {
    rt: &'a mut Rt,
    this: ObjectHandle,
}
impl Host<'_> {
    fn call(&mut self, name: &str, args: Vec<Variant>) -> anyhow::Result<Option<Variant>> {
        if !has_member(self.rt, self.this, name) {
            return Ok(None);
        }
        self.rt
            .call_variant_method(Variant::Object(self.this), name, args)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}
impl MarkupHost for Host<'_> {
    fn text_width(&mut self, text: &[u16], size: f32) -> anyhow::Result<f32> {
        let v = self
            .call("onGetTextWidth", vec![s16(text), real(size)])?
            .filter(|v| !matches!(v, Variant::Void))
            .ok_or_else(|| anyhow::anyhow!("TextRender requires an onGetTextWidth callback"))?;
        Ok(v.to_real().map_err(|e| anyhow::anyhow!("{e}"))? as f32)
    }
    fn font_changed(&mut self, bold: bool, italic: bool, face: &[u16]) -> anyhow::Result<()> {
        let d = self.rt.alloc_dictionary_object();
        self.rt.set_object_member(d, "bold", Variant::Integer(bold as i64));
        self.rt.set_object_member(d, "italic", Variant::Integer(italic as i64));
        self.rt.set_object_member(d, "face", s16(face));
        self.call("onFontChange", vec![Variant::Object(d)])?;
        Ok(())
    }
    fn eval(&mut self, expression: &[u16]) -> anyhow::Result<Vec<u16>> {
        let v = self.call("onEval", vec![s16(expression)])?.unwrap_or_default();
        Ok(v.to_tjs_string()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .encode_utf16()
            .collect())
    }
    fn label(&mut self, name: &[u16]) -> anyhow::Result<f32> {
        let v = self.call("onLabel", vec![s16(name)])?.unwrap_or_default();
        Ok(v.to_real().map_err(|e| anyhow::anyhow!("{e}"))? as f32)
    }
}

fn id(rt: &Rt, t: Option<ObjectHandle>) -> Result<ObjectHandle> {
    this_of(rt, t, "TextRenderBase")
}
fn layout(rt: &Rt, h: ObjectHandle) -> Result<&MarkupLayout> {
    rt.host()
        .text_render
        .layouts
        .get(&h.0)
        .ok_or_else(|| err("instance not constructed"))
}
/// Writable layout; fails while a layout operation of this instance runs.
fn layout_mut(state: &mut State, h: ObjectHandle) -> Result<&mut MarkupLayout> {
    if state.busy.contains(&h.0) {
        return Err(err("property write from a layout callback"));
    }
    state.layouts.get_mut(&h.0).ok_or_else(|| err("instance not constructed"))
}
/// Run a layout operation with script callbacks. Callbacks read a snapshot
/// of the state from before the operation (native getters are plain reads);
/// a reentrant operation or a write from a callback fails instead of aliasing.
fn with_host<T>(
    rt: &mut Rt,
    h: ObjectHandle,
    f: impl FnOnce(&mut MarkupLayout, &mut Host) -> anyhow::Result<T>,
) -> Result<T> {
    let state = &mut rt.host_mut().text_render;
    if !state.busy.insert(h.0) {
        return Err(err("reentrant layout call"));
    }
    let Some(mut l) = state.layouts.get(&h.0).cloned() else {
        state.busy.remove(&h.0);
        return Err(err("instance not constructed"));
    };
    let result = f(&mut l, &mut Host { rt, this: h });
    let state = &mut rt.host_mut().text_render;
    state.busy.remove(&h.0);
    state.layouts.insert(h.0, l);
    result.map_err(err)
}

fn character(rt: &mut Rt, c: &markup::Character) -> Variant {
    let d = rt.alloc_dictionary_object();
    let f = |n: &str| c.format.get(n).map(from_value).unwrap_or_default();
    for (k, v) in [
        ("size", real(c.size)),
        ("shadowDiff", f("ShadowDiff")),
        ("graph", Variant::Integer(0)),
        ("italic", f("Italic")),
        ("edge", f("Edge")),
        ("x", real(c.x)),
        ("shadowColor", f("ShadowColor")),
        ("delay", real(c.delay)),
        ("text", s16(&c.text)),
        ("bold", f("Bold")),
        ("cw", real(c.width)),
        ("shadow", f("Shadow")),
        ("vertical", Variant::Integer(0)),
        ("color", f("ChColor")),
        ("edgeColor", f("EdgeColor")),
        ("y", real(c.y)),
        ("face", f("Face")),
    ] {
        rt.set_object_member(d, k, v);
    }
    if let Some(r) = &c.ruby {
        let rd = rt.alloc_dictionary_object();
        rt.set_object_member(rd, "text", s16(&r.text));
        rt.set_object_member(rd, "size", real(r.size));
        rt.set_object_member(rd, "x", real(r.x));
        rt.set_object_member(rd, "y", real(r.y));
        let a = rt.alloc_array_object(vec![Variant::Object(rd)]);
        rt.set_object_member(d, "ruby", Variant::Object(a));
    }
    Variant::Object(d)
}

fn output(rt: &mut Rt, t: Option<ObjectHandle>, name: &str) -> Result<Variant> {
    let h = id(rt, t)?;
    let l = layout(rt, h)?;
    if !l.initialized {
        return Err(err("layout requires clear or setRenderSize"));
    }
    Ok(match name {
        "renderText" => s16(&l.render_text),
        "renderCount" => Variant::Integer(l.count as i64),
        "renderLines" => Variant::Integer(l.line_count() as i64),
        "renderDelay" => real(l.render_delay()),
        "renderOver" => Variant::Integer(l.over as i64),
        "renderLeft" => real(l.left),
        "renderRight" => real(l.right),
        "renderTop" => real(l.top),
        "renderBottom" => real(l.bottom),
        "maxScrollOffset" => real(l.max_scroll_offset()),
        _ => Variant::Integer(0), // maxScrollLine: whole block always fits.
    })
}

pub fn install(rt: &mut Rt) {
    let c = crate::natives::native_class(rt, "TextRenderBase", |rt, t, _| {
        let h = id(rt, t)?;
        let state = &mut rt.host_mut().text_render;
        if state.layouts.len() >= 256 && !state.layouts.contains_key(&h.0) {
            return Err(err("instance limit"));
        }
        state.layouts.insert(h.0, MarkupLayout::default());
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", |rt, t, _| {
                let h = id(rt, t)?;
                rt.host_mut().text_render.layouts.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("onEval", |rt, _, a| {
                let e = crate::natives::string(&a, 0, "TextRenderBase.onEval")?;
                let v = crate::natives::run_source(rt, "TextRender eval", &format!("return ({e});"), None)?;
                Ok(Variant::String(v.to_tjs_string()?))
            }),
            ("clear", |rt, t, _| {
                let h = id(rt, t)?;
                with_host(rt, h, |l, host| l.clear(host))?;
                Ok(Variant::Void)
            }),
            ("setRenderSize", |rt, t, a| {
                let h = id(rt, t)?;
                let w = a.first().ok_or_else(TjsError::bad_param_count)?.to_real()? as f32;
                let hh = a.get(1).ok_or_else(TjsError::bad_param_count)?.to_real()? as f32;
                with_host(rt, h, |l, host| l.set_render_size(w, hh, host))?;
                Ok(Variant::Void)
            }),
            ("resetFont", |rt, t, _| {
                let h = id(rt, t)?;
                with_host(rt, h, |l, host| l.reset(false, host))?;
                Ok(Variant::Void)
            }),
            ("resetStyle", |rt, t, _| {
                let h = id(rt, t)?;
                with_host(rt, h, |l, host| l.reset(true, host))?;
                Ok(Variant::Void)
            }),
            ("setDefault", |rt, t, a| apply(rt, t, a, Apply::Default)),
            ("setFont", |rt, t, a| apply(rt, t, a, Apply::Font)),
            ("setStyle", |rt, t, a| apply(rt, t, a, Apply::Style)),
            ("setOption", |rt, t, a| {
                let h = id(rt, t)?;
                let o = a
                    .first()
                    .and_then(object_arg)
                    .ok_or_else(|| err("options require an object"))?;
                let mut values = vec![];
                for name in OPTIONS {
                    let v = rt.object_member(o, name);
                    if !matches!(v, Variant::Void) {
                        values.push((*name, to_value(&v)?));
                    }
                }
                let state = &mut rt.host_mut().text_render;
                let l = layout_mut(state, h)?;
                for (n, v) in values {
                    l.set_option(n, v).map_err(err)?;
                }
                Ok(Variant::Void)
            }),
            ("render", |rt, t, a| {
                let h = id(rt, t)?;
                if a.len() < 5 {
                    return Err(err("render requires five arguments"));
                }
                let text: Vec<u16> = a[0].to_tjs_string()?.encode_utf16().collect();
                let indent = a[1].to_integer()?;
                let delay = a[2].to_real()? as f32;
                let total = a[3].to_real()? as f32;
                let same = a[4].to_integer()? != 0;
                let ok = with_host(rt, h, |l, host| l.render(&text, indent, delay, total, same, host))?;
                Ok(Variant::Integer(ok as i64))
            }),
            ("done", |rt, t, _| {
                let h = id(rt, t)?;
                with_host(rt, h, |l, host| l.done(host))?;
                Ok(Variant::Void)
            }),
            ("newline", |rt, t, _| {
                let h = id(rt, t)?;
                with_host(rt, h, |l, host| l.newline(host))?;
                Ok(Variant::Void)
            }),
            ("getCharacters", |rt, t, a| {
                let h = id(rt, t)?;
                let start = need_int(&a, 0, "TextRenderBase.getCharacters")?;
                let count = need_int(&a, 1, "TextRenderBase.getCharacters")?;
                if start < 0 || count < 0 {
                    return Err(err("character range must be nonnegative"));
                }
                let chars = layout(rt, h)?.characters(start as usize, count as usize).to_vec();
                let values = chars.iter().map(|c| character(rt, c)).collect();
                Ok(Variant::Object(rt.alloc_array_object(values)))
            }),
            ("getKeyWait", |rt, t, _| {
                let h = id(rt, t)?;
                let waits = layout(rt, h)?.key_waits().to_vec();
                let values = waits
                    .into_iter()
                    .map(|pos| {
                        let d = rt.alloc_dictionary_object();
                        rt.set_object_member(d, "pos", Variant::Integer(pos as i64));
                        rt.set_object_member(d, "time", Variant::Real(0.0));
                        Variant::Object(d)
                    })
                    .collect();
                Ok(Variant::Object(rt.alloc_array_object(values)))
            }),
            ("calcShowCount", |rt, t, a| {
                let h = id(rt, t)?;
                let time = a.first().ok_or_else(TjsError::bad_param_count)?.to_real()? as f32;
                Ok(Variant::Integer(layout(rt, h)?.show_count(time) as i64))
            }),
            ("calcLineOffset", |rt, t, a| {
                let h = id(rt, t)?;
                let line = need_int(&a, 0, "TextRenderBase.calcLineOffset")?;
                Ok(real(layout(rt, h)?.line_offset(line)))
            }),
        ],
    );
    for &(suffix, ..) in FIELDS {
        let name: &'static str = Box::leak(format!("default{suffix}").into_boxed_str());
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| {
                let h = id(rt, t)?;
                let l = layout(rt, h)?;
                Ok(l.defaults.get(&name[7..]).map(from_value).unwrap_or_default())
            },
            move |rt, t, v| {
                let h = id(rt, t)?;
                let v = to_value(&v)?;
                let state = &mut rt.host_mut().text_render;
                let l = layout_mut(state, h)?;
                l.set_default(&name[7..], v).map_err(err)
            },
        );
    }
    for name in ["fontScale", "timeScale", "vertical"] {
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| {
                let h = id(rt, t)?;
                let l = layout(rt, h)?;
                Ok(match name {
                    "fontScale" => real(l.font_scale),
                    "timeScale" => real(l.time_scale),
                    _ => Variant::Integer(l.vertical as i64),
                })
            },
            move |rt, t, v| {
                let h = id(rt, t)?;
                let state = &mut rt.host_mut().text_render;
                let l = layout_mut(state, h)?;
                match name {
                    "fontScale" => l.font_scale = markup::finite(v.to_real()? as f32).map_err(err)?,
                    "timeScale" => l.time_scale = markup::finite(v.to_real()? as f32).map_err(err)?,
                    _ => l.vertical = v.to_integer()? != 0,
                }
                Ok(())
            },
        );
    }
    for name in OUTPUTS {
        rt.register_object_native_property(
            c,
            *name,
            move |rt, t| output(rt, t, name),
            crate::natives::read_only,
        );
    }
}

fn apply(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>, op: Apply) -> Result<Variant> {
    let h = id(rt, t)?;
    let o = a
        .first()
        .and_then(object_arg)
        .ok_or_else(|| err("settings require an object"))?;
    // Attribute values are read before layout detaches (lookups are plain members).
    let mut values = BTreeMap::new();
    for &(_, attribute, ..) in FIELDS {
        if attribute.is_empty() {
            continue;
        }
        let v = rt.object_member(o, attribute);
        if !matches!(v, Variant::Void) {
            values.insert(attribute, to_value(&v)?);
        }
    }
    with_host(rt, h, |l, host| {
        l.apply(op, |name| Ok(values.get(name).cloned()), host)
    })?;
    Ok(Variant::Void)
}
