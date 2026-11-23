// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! KiriKiri exchange semantics over engine-owned isolated subtree crossfades.
//! Reference: krkrz fd5c4baa, visual/LayerIntf.cpp Start/InternalStopTransition.
use crate::natives::*;
use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{Closure, ObjectHandle, Variant},
};
use serde_json::json;
use std::collections::BTreeMap;
/// Active transitions: destination → source (the destination itself for a
/// simple transition without source), the destinations whose transition
/// excludes children (only the destination's own image changes),
/// destinations clocked by a script `callback` → (callback, time ms),
/// simple (ttSimple, no exchange) destinations, and destinations whose
/// handler finishes at its first update.
#[derive(Default)]
pub struct State(
    pub BTreeMap<usize, usize>,
    pub std::collections::BTreeSet<usize>,
    pub BTreeMap<usize, (Variant, u64)>,
    pub std::collections::BTreeSet<usize>,
    pub std::collections::BTreeSet<usize>,
);

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
    let _span = crysirror_tasks::profile::span_current("krkr.transition.prepare");
    let dest = this_of(rt, t, "Layer.beginTransition")?;
    let name = string(&args, 0, "Layer.beginTransition")?;
    let kaicho=matches!(name.as_str(),"blur"|"dim");
    let glitch = matches!(name.as_str(), "glitch" | "fadeglitch" | "loopglitch");
    if name != "crossfade" && name != "universal" && !kaicho && !glitch {
        return Err(unsupported(
            &format!("transition {name}"),
            "an engine transition implementation",
        ));
    }
    // krkrz LayerIntf.cpp:7795-7797: `withchildren` defaults to true when
    // omitted or void.
    let with_children = match args.get(1) {
        None | Some(Variant::Void) => true,
        Some(v) => v.is_truthy(),
    };
    let given_source = args
        .get(2)
        .and_then(object_arg)
        .map(|h| rt.bound_this(h).unwrap_or(h));
    let options = args
        .get(3)
        .and_then(object_arg)
        .ok_or_else(|| TjsError::runtime("transition options required"))?;
    let glitch = if glitch {
        Some(crate::layer_ex::glitch_transition(rt, &name, options, given_source.is_some())?)
    } else {
        None
    };
    // ttSimple providers take an optional source; ttExchange needs one
    // (krkrz TVPSpecifyTransitionSource).
    let simple = matches!(glitch, Some(crate::layer_ex::GlitchStart::Immediate)
        | Some(crate::layer_ex::GlitchStart::Effect { exchange: false, .. }));
    let source = match given_source {
        Some(h) => h,
        None if simple => dest,
        None => return Err(TjsError::runtime("transition source Layer required")),
    };
    let single = source == dest;
    let immediate = matches!(glitch, Some(crate::layer_ex::GlitchStart::Immediate));
    let (effect, glitch_time) = match glitch {
        Some(crate::layer_ex::GlitchStart::Effect { effect, time, .. }) => (Some(effect), Some(time)),
        Some(crate::layer_ex::GlitchStart::Immediate) => (None, None),
        None if kaicho => (Some(crate::kaicho_plugin::effect(rt, &name, options)?), None),
        None => (None, None),
    };
    // krkrz `callback`: the transition's tick source (GetTransTick calls it
    // each update; the handler phase is tick / time, starting at tick 0).
    // `selfupdate` alone (updates only on script Update calls) is not mapped.
    let callback = match rt.object_member(options, "callback") {
        Variant::Void | Variant::Null => None,
        v => Some(v),
    };
    if rt.object_member(options, "selfupdate").is_truthy() && callback.is_none() {
        return Err(unsupported(
            &format!("transition {name} selfupdate without callback"),
            "script-driven transition updates",
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
        // vague 0 is a hard edge (TVPInitUnivTransBlendTable has no blend band).
        if !(0..=4096).contains(&vague) {
            return Err(unsupported(
                &format!("universal transition vague {vague}"),
                "a vague of 0..=4096",
            ));
        }
        Some(json!({"asset": asset, "vague": vague}))
    } else {
        None
    };
    let requested_duration = match glitch_time {
        Some(time) => time,
        None if immediate => 1,
        None => rt.resolve_object_member(options, "time")?.to_integer()?,
    };
    if kaicho && requested_duration<0 {return Err(TjsError::runtime("negative Kaicho transition duration"));}
    let duration=if kaicho {requested_duration.max(2)}else{requested_duration};
    if !(1..=600_000).contains(&duration) {
        return Err(TjsError::runtime("transition time must be 1..600000 ms"));
    }
    let a = rt.host().layer(dest)?.clone();
    let b = rt.host().layer(source)?.clone();
    // krkrz allows endpoints anywhere in the tree (KAG transitions a fore
    // page layer with its back page twin under another parent).
    if with_children
        && !immediate
        && ((dest == source && !single)
            || a.width != b.width
            || a.height != b.height)
    {
        let label = |h: ObjectHandle| rt.object_member(h, "name").to_tjs_string().unwrap_or_default();
        return Err(unsupported(
            &format!(
                "Layer.beginTransition({name}) dest `{}` {}x{} parent {:?} / source `{}` {}x{} parent {:?}",
                label(dest), a.width, a.height, a.parent, label(source), b.width, b.height, b.parent
            ),
            "equally sized subtree endpoints",
        ));
    }
    // krkrz: without children both layers must have an image; only the
    // destination's own image is transitioned (children draw normally).
    if !with_children
        && !immediate
        && ((dest == source && !single)
            || !a.has_image
            || !b.has_image
            || a.image_width != b.image_width
            || a.image_height != b.image_height)
    {
        return Err(unsupported(
            "Layer.beginTransition without children",
            "equally sized images",
        ));
    }
    if a.window!=b.window || !a.constructed || !b.constructed || (!single && rt.host().is_ancestor_or_self(dest.0,source.0)) {
        return Err(unsupported("Layer.beginTransition topology","bound same-window endpoints whose source does not contain the destination"));
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
    if simple {
        rt.host_mut().transitions.3.insert(dest.0);
    }
    if immediate {
        if !with_children {
            rt.host_mut().transitions.1.insert(dest.0);
        }
        rt.host_mut().transitions.4.insert(dest.0);
        rt.host_mut().transitions.0.insert(dest.0, source.0);
        return Ok(Variant::Void);
    }
    let (id, src) = if single {
        let a = rt.host().layer(dest)?.clone();
        if with_children {
            (a.container(), a.container())
        } else {
            rt.host_mut().transitions.1.insert(dest.0);
            (a.presentation(), a.presentation())
        }
    } else if with_children {
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
    let external = callback.is_some();
    if let Some(cb) = callback {
        rt.host_mut().transitions.2.insert(dest.0, (cb, duration as u64));
    }
    if let Some(effect)=effect {
        rt.host_mut().layer_op(json!({"op":"begin_crossfade_effect","id":id,"source":src,"duration_ms":duration,"effect":effect,"external":external}));
    } else {
    rt.host_mut().layer_op(json!({"op":"begin_crossfade","id":id,"source":src,
        "duration_ms":duration,"rule":rule,"external":external}));    }

    Ok(Variant::Void)
}
fn finish(rt: &mut Rt, dest: ObjectHandle) -> Result<()> {
    let Some(source) = rt.host_mut().transitions.0.remove(&dest.0) else {
        return Ok(());
    };
    rt.host_mut().transitions.2.remove(&dest.0);
    let simple = rt.host_mut().transitions.3.remove(&dest.0);
    let immediate = rt.host_mut().transitions.4.remove(&dest.0);
    let source = ObjectHandle(source);
    let a = rt.host().layer(dest)?.clone();
    let b = rt.host().layer(source)?.clone();
    if simple {
        // krkrz InternalStopTransition: no exchange for ttSimple; the
        // completion event needs a (live) transition source.
        let surface_only = rt.host_mut().transitions.1.remove(&dest.0);
        if !immediate {
            let id = if surface_only { a.presentation() } else { a.container() };
            rt.host_mut().layer_op(json!({"op":"end_crossfade","id":id}));
            if surface_only && source != dest {
                rt.host_mut().canvas_op(json!({"op":"hide","layer":temp_presentation(a.slot)}));
            }
        }
        if source != dest {
            rt.call_variant_method(
                Variant::Object(dest),
                "onTransitionCompleted",
                vec![Variant::Closure(Closure::new(dest, Some(dest))), Variant::Closure(Closure::new(source, Some(source)))],
            )?;
        }
        return Ok(());
    }
    if rt.host_mut().transitions.1.remove(&dest.0) {
        rt.host_mut().layer_op(json!({"op":"end_crossfade","id":a.presentation()}));
        rt.host_mut().canvas_op(json!({"op":"hide","layer":temp_presentation(a.slot)}));
        swap_keep_children(rt, dest, source)?;
        return completed(rt, dest, source, &a, &b);
    }
    rt.host_mut()
        .layer_op(json!({"op":"end_crossfade","id":a.container()}));
    exchange_tree(rt,dest,source,false)?;
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
fn swap_keep_children(rt:&mut Rt,dest:ObjectHandle,source:ObjectHandle)->Result<()> {
    exchange_tree(rt,dest,source,true)
}
/// LayerIntf::Exchange: both root ownership and ancestor/descendant joins are
/// native semantic state. Rendering remains shared Layer/Canvas operations.
fn exchange_tree(rt:&mut Rt,a:ObjectHandle,b:ObjectHandle,keep:bool)->Result<()> {
    use std::collections::BTreeMap;
    let host=rt.host_mut();
    let la=host.layer(a)?;let lb=host.layer(b)?;
    if a==b{return Ok(());}
    if la.window!=lb.window || !host.same_manager(a.0,b.0){return Err(unsupported("Layer exchange","one layer manager"));}
    if host.windows.get(&la.window).is_some_and(|w| w.views.iter().any(|v| v.layer==a.0 || v.layer==b.0)){
        return Err(unsupported("exchange of a draw-device view root","view-state reassignment"));
    }
    let old:BTreeMap<usize,(Option<usize>,Vec<usize>)>=host.layers.iter().map(|(id,l)|(*id,(l.parent,l.children.clone()))).collect();
    let swap=|id|if id==a.0 {b.0}else if id==b.0 {a.0}else{id};
    let mut next=old.clone();
    if keep {
        // Exchanging vertex identities preserves the children at each place,
        // including direct-parent and deeper ancestor relationships.
        for (&id,entry) in &mut next {
            let original=&old[&swap(id)];
            *entry=(original.0.map(swap),original.1.iter().copied().map(swap).collect());
        }
    } else {
        let ancestor_child=|ancestor:usize,descendant:usize|->Result<Option<usize>>{
            let mut child=descendant;
            for _ in 0..64 {
                match old.get(&child).and_then(|v|v.0) {
                    Some(parent) if parent==ancestor=>return Ok(Some(child)),
                    Some(parent)=>child=parent,
                    None=>return Ok(None),
                }
            }
            Err(TjsError::runtime("Layer exchange ancestry limit"))
        };
        let (pa,pb)=(old[&a.0].0,old[&b.0].0);
        let mut moved=vec![a.0,b.0];
        if let Some(child)=ancestor_child(a.0,b.0)? {
            next.get_mut(&a.0).unwrap().0=if pb==Some(a.0){Some(b.0)}else{pb};
            next.get_mut(&b.0).unwrap().0=pa;
            if child!=b.0 {next.get_mut(&child).unwrap().0=Some(b.0);moved.push(child);}
        } else if let Some(child)=ancestor_child(b.0,a.0)? {
            next.get_mut(&a.0).unwrap().0=pb;
            next.get_mut(&b.0).unwrap().0=if pa==Some(b.0){Some(a.0)}else{pa};
            if child!=a.0 {next.get_mut(&child).unwrap().0=Some(a.0);moved.push(child);}
        } else {
            next.get_mut(&a.0).unwrap().0=pb;next.get_mut(&b.0).unwrap().0=pa;
        }
        for (&id,original) in &old {
            let mut children=Vec::with_capacity(original.1.len()+3);
            for &child in &original.1 {
                let replaced=swap(child);
                let chosen=if next[&replaced].0==Some(id){Some(replaced)}else if next[&child].0==Some(id){Some(child)}else{None};
                if let Some(child)=chosen {if !children.contains(&child){children.push(child);}}
            }
            for &child in &moved {if next[&child].0==Some(id)&&!children.contains(&child){children.push(child);}}
            next.get_mut(&id).unwrap().1=children;
        }
    }
    for (id,(parent,children)) in next {
        let l=host.layers.get_mut(&id).unwrap();
        if l.parent!=parent||l.children!=children {l.parent=parent;l.children=children;l.dirty=true;l.presented=None;}
    }
    for window in host.windows.values_mut(){window.primary=window.primary.map(swap);}
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
        if rt.host().transitions.4.contains(&dest) {
            finish(rt, ObjectHandle(dest)).map_err(|e| anyhow::anyhow!("transition completion: {e}"))?;
            continue;
        }
        let id = if rt.host().transitions.1.contains(&dest) {
            layer.presentation()
        } else {
            layer.container()
        };
        if let Some((callback, time)) = rt.host().transitions.2.get(&dest).cloned() {
            let tick = rt
                .call_function(callback, vec![])
                .and_then(|v| v.to_integer())
                .map_err(|e| anyhow::anyhow!("transition callback: {e}"))?;
            let elapsed = u64::try_from(tick).unwrap_or(0).min(time);
            rt.host_mut()
                .layer_op(json!({"op":"set_crossfade_elapsed","id":id,"elapsed_ms":elapsed}));
            if elapsed >= time {
                finish(rt, ObjectHandle(dest))
                    .map_err(|e| anyhow::anyhow!("transition completion: {e}"))?;
            }
            continue;
        }
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
