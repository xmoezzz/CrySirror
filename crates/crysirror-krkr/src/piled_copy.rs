// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! Layer.piledCopy semantic mapping onto the engine Canvas CopySubtree command.
//! Reference: krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf,
//! visual/LayerIntf.cpp PiledCopy (4102), Complete (6104), wrapper (7095).
//! No snapshot pixels, compositing algorithm or renderer lives in this adapter.
use crate::host::LayerState;
use crate::natives::{has_member, need_int, this_of, unsupported, Rt};
use crysirror_tjs2::{
    runtime::{ObjectHandle, Variant},
    Result, TjsError,
};
use serde_json::json;
use std::collections::BTreeSet;

pub(crate) fn copy(rt: &mut Rt, this: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    if args.len() < 7 {
        return Err(TjsError::bad_param_count());
    }
    let destination = this_of(rt, this, "Layer.piledCopy")?;
    // Native AsObjectClosureNoAddRef uses Object, not ObjThis: an
    // `source incontextof Layer` expression must still select source.
    let source = args[2]
        .object_handle()
        .map(|h| rt.bound_this(h).unwrap_or(h))
        .ok_or_else(|| TjsError::runtime("Layer.piledCopy requires a source Layer"))?;
    let values = [
        need_int(&args, 0, "piledCopy")?,
        need_int(&args, 1, "piledCopy")?,
        need_int(&args, 3, "piledCopy")?,
        need_int(&args, 4, "piledCopy")?,
        need_int(&args, 5, "piledCopy")?,
        need_int(&args, 6, "piledCopy")?,
    ];
    let [dx, dy, sx, sy, width, height] = values;
    let dest = checked_layer(rt, destination)?.clone();
    let src = checked_layer(rt, source)?.clone();
    if !dest.has_image {
        return Err(TjsError::runtime(
            "Layer.piledCopy destination has no image",
        ));
    }
    if !src.has_image {
        return Err(TjsError::runtime("Layer.piledCopy source has no image"));
    }
    if width <= 0 || height <= 0 {
        return Ok(Variant::Void);
    }
    if values.iter().any(|v| v.unsigned_abs() > 1_000_000) {
        return Err(unsupported(
            "Layer.piledCopy coordinates",
            "bounded Canvas coordinates",
        ));
    }
    if !intersects(&src, &dest, values) {
        return Ok(Variant::Void);
    }

    // Complete's raw-image fast path does not fire BeforeCompletion. The
    // composed path runs native onPaint callbacks before freezing the tree.
    let visible_children = src.children.iter().any(|id| {
        rt.host()
            .layers
            .get(id)
            .is_some_and(|l| l.visible && l.opacity != 0)
    });
    let raw_image = !visible_children
        && src.image_left == 0
        && src.image_top == 0
        && src.image_width == src.width
        && src.image_height == src.height;
    if !raw_image {
        before_completion(rt, source, &mut BTreeSet::new())?;
    }
    checked_layer(rt, destination)?;
    checked_layer(rt, source)?;
    // An opaque (ltOpaque) source's stored alpha is not displayed by krkrz.
    // The shared subtree copy composes straight alpha instead; where the
    // source subtree leaves alpha below 255 the copy shows what lies beneath
    // rather than the stored RGB. Logged so the deviation stays visible.
    if !raw_image && rt.host().layer(source)?.kind == 1 {
        let diag = profile_diagnostic(rt, source);
        rt.host_mut().push_log(format!(
            "Layer.piledCopy: opaque source composed with straight alpha (stored-RGB display of ltOpaque not mapped); {}",
            diag
        ));
    }
    let profile = if raw_image {
        if rt.host().motion.has_frame(source.0) {
            Err(unsupported(
                "Layer.piledCopy raw motion image",
                "a rasterized puppet image snapshot",
            ))
        } else {
            Ok(())
        }
    } else {
        validate_visible_profile(rt, source, true, &mut BTreeSet::new())
    };
    profile.map_err(|error| {
        TjsError::runtime(format!(
            "{error}; source [{}]; destination [{}]",
            profile_diagnostic(rt, source),
            profile_diagnostic(rt, destination)
        ))
    })?;
    if !rt.host().transitions.0.is_empty() {
        rt.host_mut().push_log(
            "Layer.piledCopy during an active transition: a transition inside the copied tree is captured at its nearest endpoint, not blended".into(),
        );
    }
    // Publish current native placement/order first. The shared Canvas command
    // snapshots preceding canvas draws in command order, including same-batch draws.
    rt.host_mut().sync()?;
    let dest = checked_layer(rt, destination)?.clone();
    let src = checked_layer(rt, source)?.clone();
    if !dest.has_image || !src.has_image {
        return Err(TjsError::runtime(
            "Layer.piledCopy image invalidated by onPaint",
        ));
    }
    let command = if raw_image {
        json!({"op":"copy","source":src.slot,"destination":dest.slot,
        "rect":[sx,sy,width,height],"position":[dx,dy],"clip":dest.draw_clip,"opacity":255,"replace":true})
    } else {
        json!({"op":"copy_subtree","source":src.container(),"destination":dest.slot,
        "rect":[sx,sy,width,height],"position":[dx,dy],"clip":dest.draw_clip})
    };
    rt.host_mut().canvas_op(command);
    let copies_whole_image = raw_image
        && dx == 0
        && dy == 0
        && sx >= 0
        && sy >= 0
        && width == dest.image_width
        && height == dest.image_height
        && sx + width <= src.image_width
        && sy + height <= src.image_height
        && dest
            .draw_clip
            .is_none_or(|[x, y, w, h]| x <= 0 && y <= 0 && x + w >= width && y + h >= height);
    let layer = rt.host_mut().layer_mut(destination)?;
    layer.image_modified = true;
    layer.image_source = None;
    layer.full_alpha = if copies_whole_image {
        src.full_alpha
    } else {
        None
    };
    layer.blank = false;
    layer.dirty = true;
    rt.host_mut().api_calls += 1;
    Ok(Variant::Void)
}
fn profile_diagnostic(rt: &Rt, h: ObjectHandle) -> String {
    let Ok(layer) = rt.host().layer(h) else {
        return "missing Layer".into();
    };
    let mut visible_types = std::collections::BTreeMap::<i64, usize>::new();
    for child in &layer.children {
        if rt
            .host()
            .layers
            .get(child)
            .is_some_and(|l| l.visible && l.opacity != 0)
        {
            let kind = rt.host().layers.get(child).map_or(-1, |l| l.kind);
            *visible_types.entry(kind).or_default() += 1;
        }
    }
    format!(
        "slot={} type={} full_alpha={:?} has_image={} size={}x{} image={}x{} offset={},{} visible_child_types={visible_types:?}",
        layer.slot,
        layer.kind,
        layer.full_alpha,
        layer.has_image,
        layer.width,
        layer.height,
        layer.image_width,
        layer.image_height,
        layer.image_left,
        layer.image_top,
    )
}
fn checked_layer(rt: &Rt, h: ObjectHandle) -> Result<&LayerState> {
    if !rt.object_valid(h) {
        return Err(TjsError::runtime("Layer.piledCopy invalidated Layer"));
    }
    let layer = rt.host().layer(h)?;
    if !layer.attached {
        return Err(TjsError::runtime("Layer.piledCopy detached Layer"));
    }
    Ok(layer)
}
/// Native no-op detection only; actual clipping and resource composition belong
/// to the shared command. i128 prevents native i64 argument overflow here.
fn intersects(src: &LayerState, dst: &LayerState, v: [i64; 6]) -> bool {
    let [dx, dy, sx, sy, w, h] = v.map(i128::from);
    let l = sx.max(0);
    let t = sy.max(0);
    let r = (sx + w).min(src.width as i128);
    let b = (sy + h).min(src.height as i128);
    if l >= r || t >= b {
        return false;
    }
    let [cl, ct, cw, ch] = dst
        .draw_clip
        .unwrap_or([0, 0, dst.image_width, dst.image_height])
        .map(i128::from);
    (dx + l - sx).max(0).max(cl) < (dx + r - sx).min(dst.image_width as i128).min(cl + cw)
        && (dy + t - sy).max(0).max(ct) < (dy + b - sy).min(dst.image_height as i128).min(ct + ch)
}
fn before_completion(rt: &mut Rt, h: ObjectHandle, visited: &mut BTreeSet<usize>) -> Result<()> {
    if !visited.insert(h.0) {
        return Err(TjsError::runtime("piledCopy completion cycle"));
    }
    checked_layer(rt, h)?;
    if rt.object_member(h, "__lp_callOnPaint").is_truthy() {
        rt.set_object_member(h, "__lp_callOnPaint", Variant::Integer(0));
        if has_member(rt, h, "onPaint") {
            let suspend = rt.host().can_suspend;
            rt.host_mut().can_suspend = false;
            let result = rt.call_object_method(h, "onPaint", vec![]);
            rt.host_mut().can_suspend = suspend;
            result?;
        }
    }
    let children = checked_layer(rt, h)?.children.clone();
    for child in children {
        before_completion(rt, ObjectHandle(child), visited)?;
    }
    Ok(())
}
fn validate_visible_profile(
    rt: &Rt,
    h: ObjectHandle,
    root: bool,
    visited: &mut BTreeSet<usize>,
) -> Result<()> {
    if !visited.insert(h.0) {
        return Err(TjsError::runtime("piledCopy layer cycle"));
    }
    let layer = checked_layer(rt, h)?;
    if !root && (!layer.visible || layer.opacity == 0) {
        return Ok(());
    }
    if rt.host().motion.pending_frame(h.0) {
        return Err(unsupported(
            "Layer.piledCopy pending visible motion",
            "an owner motion completion barrier before subtree capture",
        ));
    }
    let kind = rt.host().layer(h)?.kind;
    match kind {
        2 => {}
        1 if root => {}
        1 if layer.full_alpha == Some(255)
            && layer.image_left <= 0
            && layer.image_top <= 0
            && layer
                .image_left
                .checked_add(layer.image_width)
                .is_some_and(|r| r >= layer.width)
            && layer
                .image_top
                .checked_add(layer.image_height)
                .is_some_and(|b| b >= layer.height) => {}
        0 if !root && (layer.blank || !layer.has_image) => {}
        _ => {
            return Err(unsupported(
                &format!("Layer.piledCopy source type={kind}"),
                "normal-alpha or proven opaque source content",
            ))
        }
    }
    for child in &layer.children {
        validate_visible_profile(rt, ObjectHandle(*child), false, visited)?;
    }
    Ok(())
}
