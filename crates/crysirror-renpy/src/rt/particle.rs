// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.particle`, after Ren'Py 8.3 `particle.py` (MIT):
//! SpriteManager/Sprite, Particles and the SnowBlossom factory.
use super::{
    layout::rargs,
    py::{self, *},
    render,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.display.particle";

fn mget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, MODULE, n)
}

fn fget(vm: &mut Vm, o: &Value, n: &str) -> PyResult<f64> {
    let v = py::get_or_none(vm, o, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn bget(vm: &mut Vm, o: &Value, n: &str) -> PyResult<bool> {
    let v = py::get_or_none(vm, o, n)?;
    ops::truthy(vm, &v)
}

fn uniform(vm: &mut Vm, a: f64, b: f64) -> PyResult<f64> {
    let u = py::import_attr(vm, "random", "uniform")?;
    let r = vm.call(&u, vec![Value::Float(a), Value::Float(b)], Vec::new())?;
    py::f64_of(vm, &r)
}

// Sprite ---------------------------------------------------------------------------------------------

fn sprite_set_child(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (this, d) = (&args[0], &args[1]);
    let manager = py::get(vm, this, "manager")?;
    let map = py::get(vm, &manager, "displayable_map")?;
    let id = Value::Int(d.id());
    let mut sc = vm.call_method(&map, "get", vec![id.clone(), Value::None])?;
    if py::is_none(&sc) {
        let dd = super::easy::displayable(vm, d, &Value::None, true)?;
        let scc = mget(vm, "SpriteCache")?;
        sc = vm.call(&scc, Vec::new(), Vec::new())?;
        py::set(vm, &sc, "render", Value::None)?;
        py::set(vm, &sc, "child", dd.clone())?;
        py::set(vm, &sc, "st", Value::None)?;
        if bget(vm, &dd, "_duplicatable")? {
            let c = vm.call_method(&dd, "_duplicate", vec![Value::None])?;
            vm.call_method(&c, "_unique", Vec::new())?;
            py::set(vm, &sc, "child_copy", c)?;
        } else {
            py::set(vm, &sc, "child_copy", dd)?;
            ops::setitem(vm, &map, id, sc.clone())?;
        }
    }
    py::set(vm, this, "cache", sc)?;
    Ok(Value::None)
}

fn sprite_destroy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let m = py::get(vm, &args[0], "manager")?;
    py::set(vm, &m, "dead_child", Value::Bool(true))?;
    py::set(vm, &args[0], "live", Value::Bool(false))?;
    py::set(vm, &args[0], "events", Value::Bool(false))?;
    Ok(Value::None)
}

// SpriteManager ----------------------------------------------------------------------------------------

fn sm_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = ["update", "event", "predict", "ignore_time"];
    let mut v = vec![Value::None, Value::None, Value::None, Value::Bool(false)];
    for (i, a) in args.iter().skip(1).enumerate().take(4) {
        v[i] = a.clone();
    }
    let mut props = Vec::new();
    for (k, val) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => v[i] = val.clone(),
            None => props.push((k.clone(), val.clone())),
        }
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    py::set(vm, this, "update_function", v[0].clone())?;
    py::set(vm, this, "event_function", v[1].clone())?;
    py::set(vm, this, "predict_function", v[2].clone())?;
    py::set(vm, this, "ignore_time", v[3].clone())?;
    py::set(vm, this, "displayable_map", vm.types.new_dict())?;
    py::set(vm, this, "children", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "dead_child", Value::Bool(false))?;
    py::set(vm, this, "events", Value::Bool(false))?;
    py::set(vm, this, "width", Value::None)?;
    py::set(vm, this, "height", Value::None)?;
    Ok(Value::None)
}

fn sm_create(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let sc = mget(vm, "Sprite")?;
    let s = vm.call(&sc, Vec::new(), Vec::new())?;
    for (n, v) in [
        ("x", Value::Int(0)),
        ("y", Value::Int(0)),
        ("zorder", Value::Int(0)),
        ("live", Value::Bool(true)),
        ("manager", this.clone()),
        ("events", Value::Bool(false)),
    ] {
        py::set(vm, &s, n, v)?;
    }
    vm.call_method(&s, "set_child", vec![args[1].clone()])?;
    let c = py::get(vm, this, "children")?;
    vm.call_method(&c, "append", vec![s.clone()])?;
    Ok(s)
}

fn sm_predict_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let pf = py::get_or_none(vm, &args[0], "predict_function")?;
    if !py::is_none(&pf) {
        let l = vm.call(&pf, Vec::new(), Vec::new())?;
        let f = py::import_attr(vm, "renpy.display.predict", "displayable")?;
        for i in py::items(vm, &l)? {
            vm.call(&f, vec![i], Vec::new())?;
        }
    }
    Ok(Value::None)
}

fn sm_redraw(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 1, "delay", Value::Int(0));
    let d = py::f64_of(vm, &d)?;
    render::redraw(vm, &args[0], d);
    Ok(Value::None)
}

fn is_fast(vm: &mut Vm, r: &Value) -> PyResult<bool> {
    let Some(rd) = render::render_data(r) else {
        return Ok(false);
    };
    let (mesh, uniforms, shaders, properties, plain) = {
        let d = rd.borrow();
        (
            d.mesh.clone(),
            d.uniforms.clone(),
            d.shaders.clone(),
            d.properties.clone(),
            d.forward.is_none() && !d.xclipping && !d.yclipping,
        )
    };
    if !plain {
        return Ok(false);
    }
    for v in [mesh, uniforms, shaders, properties] {
        if ops::truthy(vm, &v)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sm_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (w, h, st, _) = rargs(vm, args)?;
    py::set(vm, this, "width", Value::Float(w))?;
    py::set(vm, this, "height", Value::Float(h))?;
    let uf = py::get_or_none(vm, this, "update_function")?;
    if !py::is_none(&uf) {
        let r = vm.call(&uf, vec![Value::Float(st)], Vec::new())?;
        if !py::is_none(&r) {
            let t = py::f64_of(vm, &r)?;
            render::redraw(vm, this, t);
        }
    }
    if !bget(vm, this, "ignore_time")? {
        let m = py::get(vm, this, "displayable_map")?;
        vm.call_method(&m, "clear", Vec::new())?;
    }
    let mut children = py::get_items(vm, this, "children")?;
    if bget(vm, this, "dead_child")? {
        let mut live = Vec::new();
        for c in children {
            if bget(vm, &c, "live")? {
                live.push(c);
            }
        }
        children = live;
    }
    let mut keyed = Vec::new();
    for c in children {
        keyed.push((fget(vm, &c, "zorder")?, c));
    }
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let children: Vec<Value> = keyed.into_iter().map(|k| k.1).collect();
    py::set(vm, this, "children", vm.types.new_list(children.clone()))?;
    let rv = super::layout::new(vm, w, h);
    let rvd = render::render_data(&rv).expect("render");
    let mut caches = Vec::new();
    let mut events = false;
    for s in &children {
        events |= bget(vm, s, "events")?;
        let cache = py::get(vm, s, "cache")?;
        let mut r = py::get_or_none(vm, &cache, "render")?;
        if py::is_none(&r) {
            let cst0 = py::get_or_none(vm, &cache, "st")?;
            let cst0 = if py::is_none(&cst0) {
                py::set(vm, &cache, "st", Value::Float(st))?;
                st
            } else {
                py::f64_of(vm, &cst0)?
            };
            let cst = st - cst0;
            let cc = py::get(vm, &cache, "child_copy")?;
            r = render::render(vm, &cc, w, h, cst, cst)?;
            py::set(vm, &cache, "render", r.clone())?;
            let fast = is_fast(vm, &r)?;
            py::set(vm, &cache, "fast", Value::Bool(fast))?;
            caches.push(cache.clone());
        }
        let (x, y) = (fget(vm, s, "x")?, fget(vm, s, "y")?);
        if bget(vm, &cache, "fast")? {
            let src = render::render_data(&r).expect("render");
            let blits: Vec<render::Blit> = src.borrow().children.clone();
            let mut dst = rvd.borrow_mut();
            for b in blits {
                dst.children.push(render::Blit {
                    source: b.source,
                    x: b.x + x,
                    y: b.y + y,
                    focus: false,
                    main: false,
                });
            }
        } else {
            let pos = vm.types.new_tuple(vec![Value::Float(x), Value::Float(y)]);
            vm.call_method(&rv, "subpixel_blit", vec![r, pos])?;
        }
    }
    py::set(vm, this, "events", Value::Bool(events))?;
    for c in caches {
        py::set(vm, &c, "render", Value::None)?;
    }
    Ok(rv)
}

fn sm_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args[1].clone();
    let x = py::f64_of(vm, &args[2])?;
    let y = py::f64_of(vm, &args[3])?;
    let st = py::f64_of(vm, &args[4])?;
    let children = py::get_items(vm, this, "children")?;
    for s in children.iter().rev() {
        if bget(vm, s, "events")? {
            let cache = py::get(vm, s, "cache")?;
            let child = py::get(vm, &cache, "child")?;
            let (sx, sy, cst) = (
                fget(vm, s, "x")?,
                fget(vm, s, "y")?,
                fget(vm, &cache, "st")?,
            );
            let rv = vm.call_method(
                &child,
                "event",
                vec![
                    ev.clone(),
                    Value::Float(x - sx),
                    Value::Float(y - sy),
                    Value::Float(st - cst),
                ],
            )?;
            if !py::is_none(&rv) {
                return Ok(rv);
            }
        }
    }
    let ef = py::get_or_none(vm, this, "event_function")?;
    if !py::is_none(&ef) {
        return vm.call(&ef, args[1..].to_vec(), Vec::new());
    }
    Ok(Value::None)
}

fn sm_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut out = Vec::new();
    let pf = py::get_or_none(vm, &args[0], "predict_function")?;
    if ops::truthy(vm, &pf)? {
        // Upstream swallows predict errors here.
        if let Ok(l) = vm.call(&pf, Vec::new(), Vec::new()) {
            if let Ok(items) = py::items(vm, &l) {
                for i in items {
                    if let Ok(d) = super::easy::displayable(vm, &i, &Value::None, true) {
                        out.push(d);
                    }
                }
            }
        }
    }
    Ok(vm.types.new_list(out))
}

fn sm_destroy_all(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "children", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

// Particles ------------------------------------------------------------------------------------------

fn new_sm(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let smc = mget(vm, "SpriteManager")?;
    let u = py::get(vm, this, "update_callback")?;
    let p = py::get(vm, this, "predict_callback")?;
    vm.call(
        &smc,
        Vec::new(),
        vec![("update".into(), u), ("predict".into(), p)],
    )
}

fn particles_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::int(vm, &args[1])? < 1 {
        let sm = new_sm(vm, &args[0])?;
        py::set(vm, &args[0], "sm", sm)?;
    }
    Ok(Value::None)
}

fn particles_after_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "particles", Value::None)?;
    Ok(Value::None)
}

fn particles_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let factory = arg_or(args, kw, 1, "factory", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "factory")
        .cloned()
        .collect();
    super::core::d_init(vm, &[this.clone()], &props)?;
    let sm = new_sm(vm, this)?;
    py::set(vm, this, "sm", sm)?;
    py::set(vm, this, "factory", factory)?;
    py::set(vm, this, "particles", Value::None)?;
    Ok(Value::None)
}

fn particles_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = py::f64_of(vm, &args[1])?;
    let mut particles = py::get_or_none(vm, this, "particles")?;
    let sm = py::get(vm, this, "sm")?;
    if st == 0. || py::is_none(&particles) {
        vm.call_method(&sm, "destroy_all", Vec::new())?;
        particles = vm.types.new_list(Vec::new());
    }
    let factory = py::get(vm, this, "factory")?;
    let add = vm.call_method(
        &factory,
        "create",
        vec![particles.clone(), Value::Float(st)],
    )?;
    let mut new = Vec::new();
    for pair in py::items(vm, &particles)? {
        let pp = py::items(vm, &pair)?;
        let (sprite, p) = (pp[0].clone(), pp[1].clone());
        let u = vm.call_method(&p, "update", vec![Value::Float(st)])?;
        if py::is_none(&u) {
            vm.call_method(&sprite, "destroy", Vec::new())?;
            continue;
        }
        let u = py::items(vm, &u)?;
        let cache = py::get(vm, &sprite, "cache")?;
        let cc = py::get(vm, &cache, "child")?;
        if !py::same(&u[3], &cc) {
            vm.call_method(&sprite, "set_child", vec![u[3].clone()])?;
        }
        py::set(vm, &sprite, "x", u[0].clone())?;
        py::set(vm, &sprite, "y", u[1].clone())?;
        new.push(vm.types.new_tuple(vec![sprite, p]));
    }
    if ops::truthy(vm, &add)? {
        for p in py::items(vm, &add)? {
            let u = vm.call_method(&p, "update", vec![Value::Float(st)])?;
            if py::is_none(&u) {
                continue;
            }
            let u = py::items(vm, &u)?;
            if py::is_none(&u[3]) {
                continue;
            }
            let sprite = vm.call_method(&sm, "create", vec![u[3].clone()])?;
            py::set(vm, &sprite, "x", u[0].clone())?;
            py::set(vm, &sprite, "y", u[1].clone())?;
            new.push(vm.types.new_tuple(vec![sprite, p]));
        }
    }
    py::set(vm, this, "particles", vm.types.new_list(new))?;
    Ok(Value::Int(0))
}

fn particles_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "factory")?;
    vm.call_method(&f, "predict", Vec::new())
}

fn particles_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (w, h, st, at) = rargs(vm, args)?;
    let sm = py::get(vm, &args[0], "sm")?;
    render::render(vm, &sm, w, h, st, at)
}

fn particles_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sm = py::get(vm, &args[0], "sm")?;
    Ok(vm.types.new_list(vec![sm]))
}

// SnowBlossom ---------------------------------------------------------------------------------------

const SBF: &[&str] = &[
    "image", "count", "xspeed", "yspeed", "border", "start", "fast", "rotate",
];

fn sbf_setstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "start", Value::Int(0))?;
    for (k, v) in py::str_items(vm, &args[1])? {
        py::set(vm, this, &k, v)?;
    }
    vm.call_method(this, "init", Vec::new())?;
    Ok(Value::None)
}

fn sbf_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut v = vec![Value::None; SBF.len()];
    v[7] = Value::Bool(false);
    for (i, a) in args.iter().skip(1).enumerate().take(SBF.len()) {
        v[i] = a.clone();
    }
    for (k, val) in kw {
        if let Some(i) = SBF.iter().position(|n| *n == &**k) {
            v[i] = val.clone();
        }
    }
    v[0] = super::easy::displayable(vm, &v[0], &Value::None, true)?;
    for (i, n) in SBF.iter().enumerate() {
        py::set(vm, this, n, v[i].clone())?;
    }
    vm.call_method(this, "init", Vec::new())?;
    Ok(Value::None)
}

fn sbf_init_starts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let start = fget(vm, this, "start")?;
    let count = {
        let v = py::get(vm, this, "count")?;
        py::int(vm, &v)?
    };
    let mut starts = Vec::new();
    for _ in 0..count {
        starts.push(uniform(vm, 0., start)?);
    }
    starts.push(start);
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let l = vm
        .types
        .new_list(starts.into_iter().map(Value::Float).collect());
    py::set(vm, this, "starts", l)?;
    Ok(Value::None)
}

fn ranged(vm: &mut Vm, n: &Value) -> PyResult<Value> {
    if matches!(n.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        let t = py::items(vm, n)?;
        let (a, b) = (py::f64_of(vm, &t[0])?, py::f64_of(vm, &t[1])?);
        return Ok(Value::Float(uniform(vm, a, b)?));
    }
    Ok(n.clone())
}

fn new_particle(vm: &mut Vm, this: &Value, st: f64, fast: bool) -> PyResult<Value> {
    let c = mget(vm, "SnowBlossomParticle")?;
    let image = py::get(vm, this, "image")?;
    let xs = py::get(vm, this, "xspeed")?;
    let xs = ranged(vm, &xs)?;
    let ys = py::get(vm, this, "yspeed")?;
    let ys = ranged(vm, &ys)?;
    let border = py::get(vm, this, "border")?;
    let off = uniform(vm, 0., 100.)?;
    let rot = py::get_or_none(vm, this, "rotate")?;
    vm.call(
        &c,
        vec![image, xs, ys, border, Value::Float(st), Value::Float(off)],
        vec![("fast".into(), Value::Bool(fast)), ("rotate".into(), rot)],
    )
}

fn sbf_create(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let particles = args[1].clone();
    let st = py::f64_of(vm, &args[2])?;
    let count = {
        let v = py::get(vm, this, "count")?;
        py::int(vm, &v)?
    };
    let has = !py::is_none(&particles) && ops::truthy(vm, &particles)?;
    if st == 0. && !has && bget(vm, this, "fast")? {
        let mut rv = Vec::new();
        for _ in 0..count {
            rv.push(new_particle(vm, this, st, true)?);
        }
        return Ok(vm.types.new_list(rv));
    }
    let n = if py::is_none(&particles) {
        0
    } else {
        ops::len(vm, &particles)? as i64
    };
    if py::is_none(&particles) || n < count {
        if has {
            let starts = py::get(vm, this, "starts")?;
            let s = ops::getitem(vm, &starts, Value::Int(n))?;
            if st < py::f64_of(vm, &s)? {
                return Ok(Value::None);
            }
        }
        let p = new_particle(vm, this, st, false)?;
        return Ok(vm.types.new_list(vec![p]));
    }
    Ok(Value::None)
}

fn sbf_predict(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::get(vm, &args[0], "image")?;
    Ok(vm.types.new_list(vec![i]))
}

fn screen_dims(vm: &mut Vm, rotate: bool) -> PyResult<(f64, f64)> {
    let sw = py::config_f64(vm, "screen_width")?;
    let sh = py::config_f64(vm, "screen_height")?;
    Ok(if rotate { (sh, sw) } else { (sw, sh) })
}

fn sbp_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "image", "xspeed", "yspeed", "border", "start", "offset", "fast", "rotate",
    ];
    let mut v = vec![Value::None; names.len()];
    for (i, a) in args.iter().skip(1).enumerate().take(names.len()) {
        v[i] = a.clone();
    }
    for (k, val) in kw {
        if let Some(i) = names.iter().position(|n| *n == &**k) {
            v[i] = val.clone();
        }
    }
    if py::f64_of(vm, &v[2])? == 0. {
        v[2] = Value::Int(1);
    }
    for (i, n) in names.iter().enumerate() {
        if *n != "fast" {
            py::set(vm, this, n, v[i].clone())?;
        }
    }
    let rotate = ops::truthy(vm, &v[7])?;
    let (sw, sh) = screen_dims(vm, rotate)?;
    let (xspeed, yspeed, border) = (
        py::f64_of(vm, &v[1])?,
        py::f64_of(vm, &v[2])?,
        py::f64_of(vm, &v[3])?,
    );
    let mut ystart = if yspeed > 0. { -border } else { sh + border };
    let travel = (2. * border + sh) / yspeed.abs();
    let xdist = xspeed * travel;
    let mut xstart = uniform(vm, (-xdist).min(0.), (sw + xdist).max(sw))?;
    if ops::truthy(vm, &v[6])? {
        ystart = uniform(vm, -border, sh + border)?;
        xstart = uniform(vm, 0., sw)?;
    }
    py::set(vm, this, "ystart", Value::Float(ystart))?;
    py::set(vm, this, "xstart", Value::Float(xstart))?;
    Ok(Value::None)
}

fn sbp_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let st = py::f64_of(vm, &args[1])?;
    let to = st - fget(vm, this, "start")?;
    let xpos = fget(vm, this, "xstart")? + to * fget(vm, this, "xspeed")?;
    let ypos = fget(vm, this, "ystart")? + to * fget(vm, this, "yspeed")?;
    let rotate = bget(vm, this, "rotate")?;
    let (_, sh) = screen_dims(vm, rotate)?;
    let border = fget(vm, this, "border")?;
    if ypos > sh + border || ypos < -border {
        return Ok(Value::None);
    }
    let image = py::get(vm, this, "image")?;
    let t = Value::Float(to + fget(vm, this, "offset")?);
    let (a, b) = if rotate { (ypos, xpos) } else { (xpos, ypos) };
    Ok(vm.types.new_tuple(vec![
        Value::Int(a.trunc() as i64),
        Value::Int(b.trunc() as i64),
        t,
        image,
    ]))
}

fn f_snow_blossom(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = arg_or(args, kw, 0, "d", Value::None);
    let count = arg_or(args, kw, 1, "count", Value::Int(10));
    let border = arg_or(args, kw, 2, "border", Value::Int(50));
    let mut xs = arg_or(
        args,
        kw,
        3,
        "xspeed",
        vm.types.new_tuple(vec![Value::Int(20), Value::Int(50)]),
    );
    let mut ys = arg_or(
        args,
        kw,
        4,
        "yspeed",
        vm.types.new_tuple(vec![Value::Int(100), Value::Int(200)]),
    );
    let start = arg_or(args, kw, 5, "start", Value::Int(0));
    let fast = arg_or(args, kw, 6, "fast", Value::Bool(false));
    let horizontal = arg_or(args, kw, 7, "horizontal", Value::Bool(false));
    if ops::truthy(vm, &horizontal)? {
        std::mem::swap(&mut xs, &mut ys);
    }
    let fc = mget(vm, "SnowBlossomFactory")?;
    let f = vm.call(
        &fc,
        Vec::new(),
        vec![
            ("image".into(), d),
            ("count".into(), count),
            ("border".into(), border),
            ("xspeed".into(), xs),
            ("yspeed".into(), ys),
            ("start".into(), start),
            ("fast".into(), fast),
            ("rotate".into(), horizontal),
        ],
    )?;
    let pc = mget(vm, "Particles")?;
    vm.call(&pc, vec![f], Vec::new())
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[("SnowBlossom", f_snow_blossom)]);
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let norollback = py::import_attr(vm, "renpy.python", "NoRollback")?;
    let d = Value::Obj(super::core::displayable_class(vm));
    let nosave = |vm: &mut Vm, names: &[&str]| -> Value {
        let l = names.iter().map(|n| vm.types.new_str(n)).collect();
        vm.types.new_list(l)
    };
    let sc_nosave = nosave(vm, &["st", "render"]);
    let p_nosave = nosave(vm, &["particles"]);
    let mut c = |vm: &mut Vm,
                 name: &str,
                 bases: &[Value],
                 methods: Methods,
                 attrs: &[(&str, Value)]|
     -> PyResult<()> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        crysirror_python::modules::set(vm, &m, name, Value::Obj(cls));
        Ok(())
    };
    c(
        vm,
        "SpriteCache",
        &[object.clone()],
        &[],
        &[
            ("nosave", sc_nosave),
            ("child", Value::None),
            ("child_copy", Value::None),
            ("st", Value::None),
            ("render", Value::None),
            ("fast", Value::Bool(false)),
        ],
    )?;
    c(
        vm,
        "Sprite",
        &[object],
        &[("set_child", sprite_set_child), ("destroy", sprite_destroy)],
        &[
            ("x", Value::Int(0)),
            ("y", Value::Int(0)),
            ("zorder", Value::Int(0)),
            ("child", Value::None),
            ("render", Value::None),
            ("live", Value::Bool(true)),
            ("manager", Value::None),
        ],
    )?;
    c(
        vm,
        "SpriteManager",
        &[d.clone()],
        &[
            ("__init__", sm_init),
            ("create", sm_create),
            ("predict_one", sm_predict_one),
            ("redraw", sm_redraw),
            ("render", sm_render),
            ("event", sm_event),
            ("visit", sm_visit),
            ("destroy_all", sm_destroy_all),
        ],
        &[],
    )?;
    c(
        vm,
        "Particles",
        &[d, norollback.clone()],
        &[
            ("after_upgrade", particles_after_upgrade),
            ("after_setstate", particles_after_setstate),
            ("__init__", particles_init),
            ("update_callback", particles_update),
            ("predict_callback", particles_predict),
            ("render", particles_render),
            ("visit", particles_visit),
        ],
        &[("__version__", Value::Int(1)), ("nosave", p_nosave)],
    )?;
    c(
        vm,
        "SnowBlossomFactory",
        &[norollback.clone()],
        &[
            ("__setstate__", sbf_setstate),
            ("__init__", sbf_init),
            ("init", sbf_init_starts),
            ("create", sbf_create),
            ("predict", sbf_predict),
        ],
        &[("rotate", Value::Bool(false))],
    )?;
    c(
        vm,
        "SnowBlossomParticle",
        &[norollback],
        &[("__init__", sbp_init), ("update", sbp_update)],
        &[],
    )?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
