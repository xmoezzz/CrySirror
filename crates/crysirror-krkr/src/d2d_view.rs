// SPDX-License-Identifier: MPL-2.0
//! The D2D draw device can present additional layer managers in the same
//! window. A view rectangle is expressed as destination edges, while Layer
//! geometry and its children stay in the manager's own pixel coordinates.
use crate::host::LayerState;
use crate::natives::{Rt, int, read_only};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub(crate) struct View {
    pub layer: usize,
    pub order: i64,
    visible: bool,
    opacity: i64,
    interpolation: i64,
    rect: Option<[f64; 4]>,
}

impl View {
    pub(crate) fn present(
        &self,
        spec: &mut Value,
        layer: &LayerState,
        offset: [i64; 2],
        zoom: f64,
    ) {
        spec["visible"] = json!(self.visible && spec["visible"].as_bool() == Some(true));
        spec["opacity"] = json!((layer.opacity * self.opacity + 127) / 255);
        if let Some([left, top, right, bottom]) = self.rect {
            if layer.width <= 0 || layer.height <= 0 || right <= left || bottom <= top {
                spec["visible"] = json!(false);
                return;
            }
            // Device edges already include Window.setZoom. The shared scene is
            // in canvas pixels and the host applies display scaling at present.
            spec["rect"][0] = json!(left / zoom + offset[0] as f64);
            spec["rect"][1] = json!(top / zoom + offset[1] as f64);
            spec["transform"] = json!({
                "scale_x": (right - left) / (zoom * layer.width as f64),
                "scale_y": (bottom - top) / (zoom * layer.height as f64),
            });
        }
    }
}

pub(crate) fn sync_window_zoom(rt: &mut Rt, window: ObjectHandle) {
    let ratio = |key| match rt.object_member(window, key) {
        Variant::Integer(n) if n > 0 => n as f64,
        _ => 1.0,
    };
    let zoom = ratio("__zoomNumer") / ratio("__zoomDenom");
    let host = rt.host_mut();
    if let Some(w) = host.windows.get_mut(&window.0) {
        w.device_zoom = Some(zoom);
        for view in &w.views {
            if let Some(layer) = host.layers.get_mut(&view.layer) {
                layer.dirty = true;
            }
        }
    }
}

pub(crate) fn supported(rt: &Rt, window: ObjectHandle) -> bool {
    rt.host().loaded_plugins.contains("drawdeviced2dm.dll")
        && rt
            .object_member(window, "__drawDevice")
            .object_handle()
            .is_some_and(|device| {
                rt.object_class_infos(device)
                    .iter()
                    .any(|name| name == "DrawDeviceD2D")
            })
}

pub(crate) fn attach(rt: &mut Rt, window: ObjectHandle, layer: ObjectHandle) {
    rt.host_mut()
        .windows
        .get_mut(&window.0)
        .unwrap()
        .views
        .push(View {
            layer: layer.0,
            order: 0,
            visible: true,
            opacity: 255,
            interpolation: 0,
            rect: None,
        });
    let state = rt.alloc_ordinary_object();
    rt.add_object_class_info(state, "DrawDeviceD2D.ViewState");
    rt.set_object_member(state, "__layer", Variant::Object(layer));
    rt.set_object_member(layer, "__deviceViewState", Variant::Object(state));
    rt.register_object_native_property(
        layer,
        "viewState",
        move |rt, _| Ok(rt.object_member(layer, "__deviceViewState")),
        read_only,
    );
    for key in ["visible", "opacity", "order", "zinterp"] {
        rt.register_object_native_property(
            state,
            key,
            move |rt, _| {
                let view = rt
                    .host()
                    .windows
                    .get(&window.0)
                    .and_then(|w| w.views.iter().find(|v| v.layer == layer.0))
                    .ok_or_else(|| TjsError::runtime("draw device view is no longer attached"))?;
                Ok(Variant::Integer(match key {
                    "visible" => i64::from(view.visible),
                    "opacity" => view.opacity,
                    "order" => view.order,
                    _ => view.interpolation,
                }))
            },
            move |rt, _, value| {
                let value = value.to_integer()?;
                let view = view_mut(rt, window, layer)?;
                match key {
                    "visible" => view.visible = value != 0,
                    "opacity" => view.opacity = value.clamp(0, 255),
                    "order" => view.order = value,
                    _ => view.interpolation = value,
                }
                Ok(())
            },
        );
    }
    rt.register_object_native(
        state,
        "setRect",
        move |rt: &mut Rt, _, args: Vec<Variant>| {
            if args.len() < 4 || int(&args, 4, 0)? != 0 {
                return Err(TjsError::runtime(
                    "viewState.setRect requires four edges and mode zero",
                ));
            }
            let mut rect = [0.0; 4];
            for (edge, value) in rect.iter_mut().zip(&args) {
                *edge = value.to_real()?;
                if !edge.is_finite() || edge.abs() > i32::MAX as f64 {
                    return Err(TjsError::runtime("draw device view rectangle out of range"));
                }
            }
            view_mut(rt, window, layer)?.rect = Some(rect);
            Ok(Variant::Void)
        },
    );
}

fn view_mut(rt: &mut Rt, window: ObjectHandle, layer: ObjectHandle) -> Result<&mut View> {
    let host = rt.host_mut();
    host.layer_mut(layer)?.dirty = true;
    host.windows
        .get_mut(&window.0)
        .and_then(|w| w.views.iter_mut().find(|v| v.layer == layer.0))
        .ok_or_else(|| TjsError::runtime("draw device view is no longer attached"))
}
