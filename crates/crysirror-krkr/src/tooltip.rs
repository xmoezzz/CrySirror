// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native hint strings and front-most native hit policy over shared passive UI.
use crate::natives::*;
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;
pub(crate) fn set(rt: &mut Rt, h: ObjectHandle, value: Variant) -> Result<()> {
    let text = value.to_tjs_string()?;
    let old = rt.host().layer(h)?.hint.clone();
    let id = rt.host().layer(h)?.slot;
    let spec = if !text.is_empty() && text != old {
        let spec: crysirror_scene::ui::tooltip::TooltipSpec =
            serde_json::from_value(json!({"id":id,"text":text,"font":rt.host().font}))
                .map_err(|e| TjsError::runtime(format!("Layer.hint: {e}")))?;
        spec.validate()
            .map_err(|e| TjsError::runtime(format!("Layer.hint: {e}")))?;
        Some(spec)
    } else {
        None
    };
    // Native SetHint resets both sensing switches even if its string is unchanged.
    rt.host_mut().layer_mut(h)?.show_parent_hint = false;
    rt.host_mut().layer_mut(h)?.ignore_hint_sensing = false;
    if let Some(spec) = spec {
        rt.host_mut()
            .ui_op(json!({"op":"put_tooltip","tooltip":spec}));
    } else if text.is_empty() && !old.is_empty() {
        rt.host_mut().ui_op(json!({"op":"remove_tooltip","id":id}));
    }
    rt.host_mut().layer_mut(h)?.hint = text;
    if rt.host().hint_hover == Some(h.0) {
        let input = rt.host().input;
        pointer(
            rt,
            Some(h.0),
            [input.pointer_x as f32, input.pointer_y as f32],
            input.buttons_down
                & (crysirror_core::service::buttons::MOUSE_LEFT
                    | crysirror_core::service::buttons::MOUSE_RIGHT)
                != 0,
        )?;
    }
    Ok(())
}
pub(crate) fn pointer(
    rt: &mut Rt,
    hit: Option<usize>,
    position: [f32; 2],
    held: bool,
) -> Result<()> {
    let mut target = hit.and_then(|id| rt.host().layers.get(&id));
    if target.is_some_and(|l| l.ignore_hint_sensing) && !held {
        return Ok(());
    }
    let mut depth = 0;
    while let Some(layer) = target {
        if !layer.show_parent_hint {
            break;
        }
        let Some(parent) = layer.parent else {
            break;
        };
        target = rt.host().layers.get(&parent);
        depth += 1;
        if depth > 64 {
            return Err(TjsError::runtime("native hint ancestry limit"));
        }
    }
    let command = match target.filter(|l| !l.hint.is_empty() && l.attached) {
        Some(layer) if !held => json!({"op":"show_tooltip","id":layer.slot,"position":position}),
        _ => json!({"op":"hide_tooltip"}),
    };
    rt.host_mut().hint_hover = if held { None } else { hit };
    rt.host_mut().ui_op(command);
    Ok(())
}

/// Native target eligibility only. Shared UI owns the actual visible tooltip.
pub(crate) fn sync_liveness(host: &mut crate::host::KrkrHost) -> Result<()> {
    let Some(target) = host.hint_hover else {
        return Ok(());
    };
    let mut node = Some(target);
    let mut depth = 0;
    let mut alive = true;
    while let Some(id) = node {
        let Some(layer) = host.layers.get(&id) else {
            alive = false;
            break;
        };
        if !layer.attached
            || !layer.visible
            || layer.opacity <= 0
            || !host.windows.get(&layer.window).is_some_and(|w| w.visible)
        {
            alive = false;
            break;
        }
        node = layer.parent;
        depth += 1;
        if depth > 64 {
            return Err(TjsError::runtime("native hint ancestry limit"));
        }
    }
    if !alive {
        host.hint_hover = None;
        host.ui_op(json!({"op":"hide_tooltip"}));
    }
    Ok(())
}
