// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! layerExDraw semantic classes/state only. Geometry, bounds and rasterization
//! are shared engine operations. Profile: ordered solid fills and line paths.
use crate::natives::{Rt, methods, object_arg, properties, read_only, this_of, unsupported};
use crysirror_runtime::vector::{self, Affine2, IDENTITY, PathCommand as C, VectorPath};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::BTreeMap;
fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}
#[derive(Clone)]
struct Brush {
    color: [u8; 4],
    offset: [f32; 2],
}
#[derive(Clone, Default)]
struct Path {
    commands: Vec<C>,
    new_figure: bool,
}
#[derive(Clone)]
struct Layer {
    transform: Affine2,
    view: Affine2,
    smoothing: i64,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            transform: IDENTITY,
            view: IDENTITY,
            smoothing: 4,
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    builtin_update: Option<ObjectHandle>,
    apps: BTreeMap<usize, Vec<Brush>>,
    paths: BTreeMap<usize, Path>,
    layers: BTreeMap<usize, Layer>,
}
impl State {
    pub(crate) fn invalidate(&mut self, id: usize) {
        self.apps.remove(&id);
        self.paths.remove(&id);
        self.layers.remove(&id);
    }
}
const MAX_OBJECTS: usize = 256;
// object_member is intentionally a raw own-member lookup in the original VM.
// Follow native superclass links without evaluating script getters/callbacks,
// and compare the resolved method to the builtin captured before any script executes.
pub(crate) fn capture_base_update(rt:&mut Rt) {
    let update=rt.global_member("Layer").object_handle().map(|h|rt.object_member(h,"update"));
    rt.host_mut().draws.builtin_update=update.and_then(|v|rt.variant_is_native_function(&v).then(||v.object_handle()).flatten());
}
fn has_builtin_update(rt: &Rt, h: ObjectHandle) -> bool {
    let Some(expected) = rt.host().draws.builtin_update else {
        return false;
    };
    let mut current = Some(h);
    for _ in 0..64 {
        let Some(object) = current else { return false };
        if rt.has_object_member(object, "update") {
            let value = rt.object_member(object, "update");
            return value.object_handle() == Some(expected)
                && rt.variant_is_native_function(&value);
        }
        current = rt.object_super_class(object);
    }
    false
}

fn fresh(rt: &mut Rt, t: Option<ObjectHandle>) -> ObjectHandle {
    t.map(|h| rt.bound_this(h).unwrap_or(h))
        .filter(|h| *h != rt.global_handle())
        .unwrap_or_else(|| rt.alloc_ordinary_object())
}
fn arg(a: &[Variant], i: usize) -> Result<f32> {
    let v = a.get(i).ok_or_else(TjsError::bad_param_count)?.to_real()? as f32;
    if !v.is_finite() {
        return Err(err("nonfinite drawing argument"));
    }
    Ok(v)
}
fn argc(a: &[Variant], n: usize) -> Result<()> {
    if a.len() < n {
        Err(TjsError::bad_param_count())
    } else {
        Ok(())
    }
}
fn member(rt: &Rt, h: ObjectHandle, name: &str) -> Result<f32> {
    let v = rt.object_member(h, name).to_real()? as f32;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(err("nonfinite geometry member"))
    }
}
fn data(rt: &mut Rt, h: ObjectHandle) -> Result<&mut Layer> {
    rt.host().layer(h)?;
    let live = rt
        .host()
        .layers
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let state = &mut rt.host_mut().draws;
    state.layers.retain(|k, _| live.contains(k));
    Ok(state.layers.entry(h.0).or_default())
}
fn rect(rt: &mut Rt, [x, y, w, h]: [f32; 4]) -> Result<Variant> {
    let object = rt.alloc_ordinary_object();
    rt.add_object_class_info(object, "RectF");
    for (k, v) in [("x", x), ("y", y), ("width", w), ("height", h)] {
        rt.set_object_member(object, k, Variant::Real(v as f64));
    }
    properties(
        rt,
        object,
        &[
            (
                "left",
                |rt, t| {
                    let h = this_of(rt, t, "RectF.left")?;
                    Ok(rt.object_member(h, "x"))
                },
                read_only,
            ),
            (
                "top",
                |rt, t| {
                    let h = this_of(rt, t, "RectF.top")?;
                    Ok(rt.object_member(h, "y"))
                },
                read_only,
            ),
            (
                "right",
                |rt, t| {
                    let h = this_of(rt, t, "RectF.right")?;
                    Ok(Variant::Real(
                        (member(rt, h, "x")? + member(rt, h, "width")?) as f64,
                    ))
                },
                read_only,
            ),
            (
                "bottom",
                |rt, t| {
                    let h = this_of(rt, t, "RectF.bottom")?;
                    Ok(Variant::Real(
                        (member(rt, h, "y")? + member(rt, h, "height")?) as f64,
                    ))
                },
                read_only,
            ),
        ],
    );
    // The original RectF converter returns (dispatch, dispatch). Preserve ObjThis
    // so own native edge properties read this RectF, not the caller frame.
    Ok(Variant::self_bound(object))
}
fn points(rt: &Rt, v: &Variant) -> Result<Vec<[f32; 2]>> {
    let h = object_arg(v).ok_or_else(|| err("point array required"))?;
    let values = rt
        .array_elements(h)
        .ok_or_else(|| err("point array required"))?;
    if values.len() > 8190 {
        return Err(err("drawing point budget"));
    }
    values
        .iter()
        .map(|v| {
            let h = object_arg(v).ok_or_else(|| err("point required"))?;
            if let Some(a) = rt.array_elements(h) {
                argc(a, 2)?;
                Ok([arg(a, 0)?, arg(a, 1)?])
            } else {
                Ok([member(rt, h, "x")?, member(rt, h, "y")?])
            }
        })
        .collect()
}
fn shape(rt: &Rt, name: &str, a: &[Variant], skip: usize) -> Result<Vec<C>> {
    let a = &a[skip..];
    match name {
        "drawRectangle" => {
            argc(a, 4)?;
            vector::rectangle([arg(a, 0)?, arg(a, 1)?, arg(a, 2)?, arg(a, 3)?]).map_err(err)
        }
        "drawPolygon" => {
            argc(a, 1)?;
            vector::polygon(&points(rt, &a[0])?, true).map_err(err)
        }
        "drawLine" => {
            argc(a, 4)?;
            vector::polygon(&[[arg(a, 0)?, arg(a, 1)?], [arg(a, 2)?, arg(a, 3)?]], false)
                .map_err(err)
        }
        "drawLines" => {
            argc(a, 1)?;
            vector::polygon(&points(rt, &a[0])?, false).map_err(err)
        }
        _ => Err(unsupported(
            name,
            "solid rectangle/polygon/line path profile",
        )),
    }
}
fn draw(rt: &mut Rt, t: Option<ObjectHandle>, name: &str, a: Vec<Variant>) -> Result<Variant> {
    argc(
        &a,
        if name == "drawPath" {
            2
        } else {
            if name == "drawRectangle" || name == "drawLine" {
                5
            } else {
                2
            }
        },
    )?;
    let h = this_of(rt, t, name)?;
    let state = data(rt, h)?.clone();
    if !has_builtin_update(rt, h) {
        return Err(unsupported(
            "overridden Layer.update",
            "post-draw script callback continuation",
        ));
    }
    let app = object_arg(&a[0]).ok_or_else(|| err("Appearance required"))?;
    let brushes = rt
        .host()
        .draws
        .apps
        .get(&app.0)
        .cloned()
        .ok_or_else(|| err("Appearance is not constructed"))?;
    let commands = if name == "drawPath" {
        let p = object_arg(&a[1]).ok_or_else(|| err("Path required"))?;
        rt.host()
            .draws
            .paths
            .get(&p.0)
            .ok_or_else(|| err("Path is not constructed"))?
            .commands
            .clone()
    } else {
        shape(rt, name, &a, 1)?
    };
    if brushes.is_empty() || commands.is_empty() {
        return rect(rt, [0.; 4]);
    }
    let transform = vector::append_affine(state.transform, state.view).map_err(err)?;
    let mut bounds = None;
    let mut effects = vec![];
    for b in brushes {
        let transform = vector::append_affine(
            vector::translation(b.offset[0], b.offset[1]).map_err(err)?,
            transform,
        )
        .map_err(err)?;
        let found = vector::fill_bounds(&commands, transform).map_err(err)?;
        bounds = Some(bounds.map_or(found, |old| vector::union_bounds(old, found)));
        let spec = VectorPath {
            commands: commands.clone(),
            fill: Some(b.color),
            stroke: None,
            transform,
            even_odd: true,
        };
        effects.push(serde_json::json!({"kind":"vector","path":spec}));
    }
    let result = rect(rt, bounds.unwrap_or([0.; 4]))?;
    crate::image_api::begin_chain(rt, Some(h), effects)?;
    Ok(result)
}
fn install_appearance(rt: &mut Rt, h: ObjectHandle) {
    methods(
        rt,
        h,
        &[
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "Appearance.finalize")?;
                rt.host_mut().draws.apps.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("clear", |rt, t, _| {
                let h = this_of(rt, t, "Appearance.clear")?;
                rt.host_mut()
                    .draws
                    .apps
                    .get_mut(&h.0)
                    .ok_or_else(|| err("Appearance is not constructed"))?
                    .clear();
                Ok(Variant::Void)
            }),
            ("addBrush", |rt, t, a| {
                argc(&a, 3)?;
                let h = this_of(rt, t, "Appearance.addBrush")?;
                if object_arg(&a[0]).is_some() {
                    return Err(unsupported("GDI brush object", "solid ARGB brush"));
                }
                let c = a[0].to_integer()? as u32;
                let offset = [arg(&a, 1)?, arg(&a, 2)?];
                vector::translation(offset[0], offset[1]).map_err(err)?;
                let b = rt
                    .host_mut()
                    .draws
                    .apps
                    .get_mut(&h.0)
                    .ok_or_else(|| err("Appearance is not constructed"))?;
                if b.len() >= 16 {
                    return Err(err("Appearance brush budget"));
                }
                b.push(Brush {
                    color: [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8],
                    offset,
                });
                Ok(Variant::Void)
            }),
            ("addPen", |_, _, a| {
                argc(&a, 4)?;
                Err(unsupported(
                    "Appearance.addPen",
                    "shared GDI stroke join/cap/bounds profile",
                ))
            }),
        ],
    );
}
fn install_path(rt: &mut Rt, h: ObjectHandle) {
    methods(
        rt,
        h,
        &[
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "Path.finalize")?;
                rt.host_mut().draws.paths.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("startFigure", |rt, t, _| {
                let h = this_of(rt, t, "Path.startFigure")?;
                rt.host_mut()
                    .draws
                    .paths
                    .get_mut(&h.0)
                    .ok_or_else(|| err("Path is not constructed"))?
                    .new_figure = true;
                Ok(Variant::Void)
            }),
            ("closeFigure", |rt, t, _| {
                let h = this_of(rt, t, "Path.closeFigure")?;
                let p = rt
                    .host_mut()
                    .draws
                    .paths
                    .get_mut(&h.0)
                    .ok_or_else(|| err("Path is not constructed"))?;
                if !p.commands.is_empty() && !matches!(p.commands.last(), Some(C::Close)) {
                    if p.commands.len() >= 8192 {
                        return Err(err("Path command budget"));
                    }
                    p.commands.push(C::Close)
                }
                p.new_figure = true;
                Ok(Variant::Void)
            }),
        ],
    );
    for name in ["drawRectangle", "drawPolygon", "drawLine", "drawLines"] {
        rt.register_object_native(h, name, move |rt: &mut Rt, t, a: Vec<Variant>| {
            let who = this_of(rt, t, name)?;
            let commands = shape(rt, name, &a, 0)?;
            let p = rt
                .host_mut()
                .draws
                .paths
                .get_mut(&who.0)
                .ok_or_else(|| err("Path is not constructed"))?;
            if name == "drawLine" || name == "drawLines" {
                let points = commands
                    .iter()
                    .filter_map(|c| match c {
                        C::Move { to } | C::Line { to } => Some(*to),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                vector::append_polyline(&mut p.commands, &points, p.new_figure).map_err(err)?;
                p.new_figure = false;
            } else {
                if p.commands.len() + commands.len() > 8192 {
                    return Err(err("Path command budget"));
                }
                p.commands.extend(commands);
                p.new_figure = true;
            }
            Ok(Variant::Void)
        });
    }
}
fn matrix(rt: &Rt, v: &Variant) -> Result<Affine2> {
    let h = object_arg(v).ok_or_else(|| err("matrix array/dictionary required"))?;
    let m = if let Some(a) = rt.array_elements(h) {
        argc(a, 6)?;
        [
            arg(a, 0)?,
            arg(a, 1)?,
            arg(a, 2)?,
            arg(a, 3)?,
            arg(a, 4)?,
            arg(a, 5)?,
        ]
    } else {
        [
            member(rt, h, "m11")?,
            member(rt, h, "m12")?,
            member(rt, h, "m21")?,
            member(rt, h, "m22")?,
            member(rt, h, "dx")?,
            member(rt, h, "dy")?,
        ]
    };
    vector::validate_affine(m).map_err(err)?;
    Ok(m)
}
pub fn install(rt: &mut Rt) {
    let g = rt
        .global_member("GdiPlus")
        .object_handle()
        .unwrap_or_else(|| rt.alloc_ordinary_object());
    rt.add_object_class_info(g, "GdiPlus");
    rt.set_global_member("GdiPlus", Variant::Object(g));
    let a = rt.alloc_native_constructor(|rt: &mut Rt, t, _a: Vec<Variant>| {
        if rt.host().draws.apps.len() >= MAX_OBJECTS {
            return Err(err("Appearance object budget"));
        }
        let h = fresh(rt, t);
        rt.host_mut().draws.apps.insert(h.0, vec![]);
        rt.add_object_class_info(h, "Appearance");
        install_appearance(rt, h);
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(a, "Appearance");
    rt.set_object_member(g, "Appearance", Variant::Object(a));
    let p = rt.alloc_native_constructor(|rt: &mut Rt, t, _a: Vec<Variant>| {
        if rt.host().draws.paths.len() >= MAX_OBJECTS {
            return Err(err("Path object budget"));
        }
        let h = fresh(rt, t);
        rt.host_mut().draws.paths.insert(h.0, Path::default());
        rt.add_object_class_info(h, "Path");
        install_path(rt, h);
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(p, "Path");
    rt.set_object_member(g, "Path", Variant::Object(p));
    let r = rt.alloc_native_constructor(|rt: &mut Rt, _t, a: Vec<Variant>| {
        argc(&a, 4)?;
        rect(rt, [arg(&a, 0)?, arg(&a, 1)?, arg(&a, 2)?, arg(&a, 3)?])
    });
    rt.add_object_class_info(r, "RectF");
    rt.set_object_member(g, "RectF", Variant::Object(r));
    for (k, v) in [
        ("SmoothingModeAntiAlias", 4),
        ("SmoothingModeHighQuality", 2),
        ("TextRenderingHintAntiAlias", 4),
        ("MatrixOrderAppend", 1),
    ] {
        rt.set_object_member(g, k, Variant::Integer(v));
    }
    let Some(layer) = rt.global_member("Layer").object_handle() else {
        return;
    };


    for name in [
        "drawPath",
        "drawRectangle",
        "drawPolygon",
        "drawLine",
        "drawLines",
    ] {
        rt.register_object_native(layer, name, move |rt: &mut Rt, t, a: Vec<Variant>| {
            draw(rt, t, name, a)
        });
    }
    rt.register_object_native(layer, "clear", |rt: &mut Rt, t, a: Vec<Variant>| {
        argc(&a, 1)?;
        let h = this_of(rt, t, "Layer.clear")?;
        let l = rt.host().layer(h)?.clone();
        if !has_builtin_update(rt, h) {
            return Err(unsupported(
                "overridden Layer.update",
                "post-clear script callback continuation",
            ));
        }
        if !l.has_image {
            return Err(err("not drawable layer type"));
        }
        let c = a[0].to_integer()? as u32;
        let color = [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8];
        rt.host_mut()
            .canvas_op(serde_json::json!({"op":"clear","id":l.slot,"color":color}));
        rt.host_mut().layer_mut(h)?.blank = color[3] == 0;
        Ok(Variant::Void)
    });
    for name in [
        "setTransform",
        "setViewTransform",
        "resetTransform",
        "resetViewTransform",
        "translateTransform",
        "translateViewTransform",
        "scaleTransform",
        "scaleViewTransform",
        "rotateTransform",
        "rotateViewTransform",
    ] {
        rt.register_object_native(layer, name, move |rt: &mut Rt, t, a: Vec<Variant>| {
            let h = this_of(rt, t, name)?;
            let m = if name.starts_with("reset") {
                IDENTITY
            } else if name.starts_with("set") {
                argc(&a, 1)?;
                matrix(rt, &a[0])?
            } else if name.starts_with("translate") {
                argc(&a, 2)?;
                vector::translation(arg(&a, 0)?, arg(&a, 1)?).map_err(err)?
            } else if name.starts_with("scale") {
                argc(&a, 2)?;
                vector::scaling(arg(&a, 0)?, arg(&a, 1)?).map_err(err)?
            } else {
                argc(&a, 1)?;
                vector::rotation(arg(&a, 0)?).map_err(err)?
            };
            let d = data(rt, h)?;
            let dest = if name.contains("View") {
                &mut d.view
            } else {
                &mut d.transform
            };
            *dest = if name.starts_with("set") || name.starts_with("reset") {
                m
            } else {
                vector::append_affine(*dest, m).map_err(err)?
            };
            Ok(Variant::Void)
        });
    }
    properties(
        rt,
        layer,
        &[
            (
                "updateWhenDraw",
                |_, _| Ok(Variant::Integer(1)),
                |_, _, v| {
                    if v.to_integer()? != 0 {
                        Ok(())
                    } else {
                        Err(unsupported(
                            "updateWhenDraw=false",
                            "deferred vector publication",
                        ))
                    }
                },
            ),
            (
                "smoothingMode",
                |rt, t| {
                    let h = this_of(rt, t, "smoothingMode")?;
                    Ok(Variant::Integer(data(rt, h)?.smoothing))
                },
                |rt, t, v| {
                    let mode = v.to_integer()?;
                    if !matches!(mode, 2 | 4) {
                        return Err(unsupported(
                            "non-AA GDI mode",
                            "shared vector antialias policy",
                        ));
                    }
                    let h = this_of(rt, t, "smoothingMode")?;
                    data(rt, h)?.smoothing = mode;
                    Ok(())
                },
            ),
            (
                "record",
                |_, _| Ok(Variant::Integer(0)),
                |_, _, v| {
                    if v.to_integer()? == 0 {
                        Ok(())
                    } else {
                        Err(unsupported(
                            "GDI metafile record",
                            "shared vector command recording",
                        ))
                    }
                },
            ),
        ],
    );
}

impl State {
    pub(crate) fn gc_roots(&self, out: &mut Vec<crysirror_tjs2::runtime::ObjectHandle>) {
        use crysirror_tjs2::runtime::ObjectHandle;
        out.extend(self.builtin_update);
        out.extend(self.apps.keys().chain(self.paths.keys()).chain(self.layers.keys()).map(|k| ObjectHandle(*k)));
    }
}
