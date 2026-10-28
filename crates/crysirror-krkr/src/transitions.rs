// Distributed under the GNU Affero General Public License, version 3 or later.
//! KiriKiri exchange semantics over engine-owned isolated subtree crossfades.
//! Reference: krkrz fd5c4baa, visual/LayerIntf.cpp Start/InternalStopTransition.
use crate::natives::*;
use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{Closure, ObjectHandle, Variant},
};
use serde_json::json;
use std::collections::BTreeMap;
/// Active transitions: destination → source, and the destinations whose
/// transition excludes children (only the destination's own image changes).
#[derive(Default)]
pub struct State(pub BTreeMap<usize, usize>, pub std::collections::BTreeSet<usize>);

/// Engine layer id of the source image presented beside a destination's own
/// image during a transition without children (below generated canvas ids).
pub fn temp_presentation(dest_slot: u32) -> u32 {
    0x1C00_0000 + dest_slot
}
pub fn install(rt: &mut Rt) {
    let c = rt.global_member("Layer").object_handle().unwrap();
    methods(
        rt,
        c,
        &[
            ("beginTransition", begin),
            ("stopTransition", |rt, t, _| {
                let h = this_of(rt, t, "Layer.stopTransition")?;
                finish(rt, h)?;
                Ok(Variant::Void)
            }),
            ("onTransitionCompleted", |_, _, _| Ok(Variant::Void)),
        ],
    );
    rt.register_object_native_property(
        c,
        "inTransition",
        |rt, t| {
            let h = this_of(rt, t, "Layer.inTransition")?;
            Ok(Variant::Integer(i64::from(
                rt.host().transitions.0.contains_key(&h.0),
            )))
        },
        read_only,
    );
}
fn begin(rt: &mut Rt, t: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    let dest = this_of(rt, t, "Layer.beginTransition")?;
    let name = string(&args, 0, "Layer.beginTransition")?;
    if name != "crossfade" && name != "universal" {
        return Err(unsupported(
            &format!("transition {name}"),
            "an engine transition implementation",
        ));
    }
    let with_children = args.get(1).is_some_and(Variant::is_truthy);
    let source = args
        .get(2)
        .and_then(object_arg)
        .map(|h| rt.bound_this(h).unwrap_or(h))
        .ok_or_else(|| TjsError::runtime("transition source Layer required"))?;
    let options = args
        .get(3)
        .and_then(object_arg)
        .ok_or_else(|| TjsError::runtime("transition options required"))?;
    if rt.object_member(options, "selfupdate").is_truthy()
        || !matches!(
            rt.object_member(options, "callback"),
            Variant::Void | Variant::Null
        )
    {
        return Err(unsupported(
            "transition callback/selfupdate",
            "external transition clocks",
        ));
    }
    // krkrz tTVPUniversalTransHandlerProvider: `rule` (required image) and
    // `vague` (default 64); the rule is stretched over the destination.
    let rule = if name == "universal" {
        let rule = rt.object_member(options, "rule");
        if matches!(rule, Variant::Void) {
            return Err(TjsError::runtime("universal transition requires a rule option"));
        }
        let rule = rule.to_tjs_string()?;
        let asset = rt
            .host()
            .storage()?
            .resolve_with(&rule, &[".tlg", ".png", ".jpg", ".jpeg", ".bmp", ".webp"])
            .ok_or_else(|| TjsError::runtime(format!("cannot load rule graphic `{rule}`")))?;
        let vague = match rt.object_member(options, "vague") {
            Variant::Void => 64,
            v => v.to_integer()?,
        };
        if !(1..=4096).contains(&vague) {
            return Err(unsupported(
                &format!("universal transition vague {vague}"),
                "a vague of 1..=4096",
            ));
        }
        Some(json!({"asset": asset, "vague": vague}))
    } else {
        None
    };
    let duration = rt.object_member(options, "time").to_integer()?;
    if !(1..=600_000).contains(&duration) {
        return Err(TjsError::runtime("transition time must be 1..600000 ms"));
    }
    let a = rt.host().layer(dest)?.clone();
    let b = rt.host().layer(source)?.clone();
    if with_children
        && (dest == source
            || a.parent.is_none()
            || a.parent != b.parent
            || a.width != b.width
            || a.height != b.height)
    {
        return Err(unsupported(
            "Layer.beginTransition",
            "equally sized sibling subtree endpoints",
        ));
    }
    // krkrz: without children both layers must have an image; only the
    // destination's own image is transitioned (children draw normally).
    if !with_children
        && (dest == source
            || !a.has_image
            || !b.has_image
            || a.image_width != b.image_width
            || a.image_height != b.image_height
            || a.parent.is_none()
            || b.parent.is_none())
    {
        return Err(unsupported(
            "Layer.beginTransition without children",
            "equally sized images on non-primary layers",
        ));
    }
    if rt
        .host()
        .transitions
        .0
        .iter()
        .any(|(&d, &s)| [d, s].contains(&dest.0) || [d, s].contains(&source.0))
    {
        return Err(TjsError::runtime("transition endpoint is already active"));
    }
    rt.host_mut().sync()?;
    let (id, src) = if with_children {
        (a.container(), b.container())
    } else {
        // Present the source image as a sibling of the destination's own
        // image; the engine transitions between those two nodes only.
        let a = rt.host().layer(dest)?.clone();
        let (order, position) = a.presented.ok_or_else(|| TjsError::runtime("transition destination not presented"))?;
        let temp = temp_presentation(a.slot);
        rt.host_mut().canvas_op(json!({"op":"present","id":b.slot,"layer":temp,"order":order,
            "parent":a.container(),"position":position}));
        rt.host_mut().transitions.1.insert(dest.0);
        (a.presentation(), temp)
    };
    rt.host_mut().transitions.0.insert(dest.0, source.0);
    rt.host_mut()
        .layer_op(json!({"op":"begin_crossfade","id":id,"source":src,"duration_ms":duration,"rule":rule}));
    Ok(Variant::Void)
}
fn finish(rt: &mut Rt, dest: ObjectHandle) -> Result<()> {
    let Some(source) = rt.host_mut().transitions.0.remove(&dest.0) else {
        return Ok(());
    };
    let source = ObjectHandle(source);
    let a = rt.host().layer(dest)?.clone();
    let b = rt.host().layer(source)?.clone();
    if rt.host_mut().transitions.1.remove(&dest.0) {
        rt.host_mut().layer_op(json!({"op":"end_crossfade","id":a.presentation()}));
        rt.host_mut().canvas_op(json!({"op":"hide","layer":temp_presentation(a.slot)}));
        swap_keep_children(rt, dest, source)?;
        return completed(rt, dest, source, &a, &b);
    }
    if a.parent != b.parent {
        return Err(TjsError::runtime("transition endpoints reparented"));
    }
    rt.host_mut()
        .layer_op(json!({"op":"end_crossfade","id":a.container()}));
    if let Some(parent) = a.parent {
        let p = rt.host_mut().layer_mut(ObjectHandle(parent))?;
        for child in &mut p.children {
            if *child == dest.0 {
                *child = source.0
            } else if *child == source.0 {
                *child = dest.0
            }
        }
        p.dirty = true;
    }
    completed(rt, dest, source, &a, &b)
}
/// Exchange position/visibility (krkrz InternalStopTransition) and fire the
/// synchronous onTransitionCompleted.
fn completed(
    rt: &mut Rt,
    dest: ObjectHandle,
    source: ObjectHandle,
    a: &crate::host::LayerState,
    b: &crate::host::LayerState,
) -> Result<()> {
    for (h, other) in [(dest, b), (source, a)] {
        let l = rt.host_mut().layer_mut(h)?;
        l.left = other.left;
        l.top = other.top;
        l.visible = other.visible;
        l.absolute = other.absolute;
        l.dirty = true;
    }
    rt.call_variant_method(
        Variant::Object(dest),
        "onTransitionCompleted",
        vec![Variant::Closure(Closure::new(dest, Some(dest))), Variant::Closure(Closure::new(source, Some(source)))],
    )?;
    Ok(())
}
/// krkrz `Swap` (Exchange with keepchildren): the two layers trade places in
/// the tree; each place keeps its children, which re-parent to the newcomer.
fn swap_keep_children(rt: &mut Rt, dest: ObjectHandle, source: ObjectHandle) -> Result<()> {
    let host = rt.host_mut();
    let (pa, pb) = (host.layer(dest)?.parent, host.layer(source)?.parent);
    let (Some(pa), Some(pb)) = (pa, pb) else {
        return Err(unsupported("transition Swap of the primary layer", "non-primary endpoints"));
    };
    if [pa, pb].contains(&dest.0) || [pa, pb].contains(&source.0) {
        return Err(unsupported("transition Swap of parent and child", "unrelated endpoints"));
    }
    for (p, from, to) in [(pa, dest.0, source.0), (pb, source.0, dest.0)] {
        let parent = host.layer_mut(ObjectHandle(p))?;
        if pa == pb {
            for c in &mut parent.children {
                if *c == dest.0 {
                    *c = source.0
                } else if *c == source.0 {
                    *c = dest.0
                }
            }
            parent.dirty = true;
            break;
        }
        if let Some(c) = parent.children.iter_mut().find(|c| **c == from) {
            *c = to;
        }
        parent.dirty = true;
    }
    host.layer_mut(dest)?.parent = Some(pb);
    host.layer_mut(source)?.parent = Some(pa);
    let ca = std::mem::take(&mut host.layer_mut(dest)?.children);
    let cb = std::mem::take(&mut host.layer_mut(source)?.children);
    for (owner, kids) in [(dest, &cb), (source, &ca)] {
        for c in kids.iter() {
            let l = host.layer_mut(ObjectHandle(*c))?;
            l.parent = Some(owner.0);
            l.dirty = true;
        }
    }
    host.layer_mut(dest)?.children = cb;
    host.layer_mut(source)?.children = ca;
    host.layer_mut(dest)?.dirty = true;
    host.layer_mut(source)?.dirty = true;
    Ok(())
}
pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    if rt.is_suspended() {
        return Ok(());
    }
    let active: Vec<_> = rt.host().transitions.0.keys().copied().collect();
    for dest in active {
        let Some(layer) = rt.host().layers.get(&dest) else {
            rt.host_mut().transitions.0.remove(&dest);
            continue;
        };
        let id = if rt.host().transitions.1.contains(&dest) {
            layer.presentation()
        } else {
            layer.container()
        };
        let bytes = host.query_engine(client, EngineQuery::LayerCrossfade(id))?;
        let status: serde_json::Value = serde_json::from_slice(&bytes)?;
        if let (Some(elapsed), Some(duration)) = (
            status["elapsed_ms"].as_u64(),
            status["duration_ms"].as_u64(),
        ) {
            if elapsed >= duration {
                finish(rt, ObjectHandle(dest))
                    .map_err(|e| anyhow::anyhow!("transition completion: {e}"))?;
            }
        }
    }
    Ok(())
}
