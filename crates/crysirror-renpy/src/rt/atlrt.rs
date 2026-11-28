// SPDX-License-Identifier: MPL-2.0
//! `renpy.atl` runtime: compiling the ATL source form (`crate::atl`) in a
//! context, executing compiled blocks against a Transform each frame, and
//! the ATLTransform displayable. After Ren'Py 8.3 `atl.py` (MIT); the
//! value types, property table and interpolation rules cover 6.x-8.x.
use super::py::{self, *};
use crate::{
    ast::Signature,
    atl::{RawBlock, RawStatement},
    lexer::PyExpr,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MODULE: &str = "renpy.atl";

// Expression evaluation ------------------------------------------------------------------------

thread_local! {
    static CODE: RefCell<HashMap<(Rc<str>, u32, Rc<str>, &'static str), Rc<crysirror_python::code::Code>>> = RefCell::new(HashMap::new());
}

fn store_globals(vm: &mut Vm) -> PyResult<ObjRef> {
    crate::engine::module_namespace(vm, "store")
        .ok_or_else(|| vm.runtime_error("store is not initialized"))
}

fn run(vm: &mut Vm, e: &PyExpr, mode: &'static str, locals: &Value) -> PyResult<Value> {
    let key = (e.filename.clone(), e.line, e.text.clone(), mode);
    let code = CODE.with(|c| c.borrow().get(&key).cloned());
    let code = match code {
        Some(c) => c,
        None => {
            let dialect = super::state::dialect(vm);
            let c = crate::pycompile::compile(vm, e, mode, dialect)
                .map_err(|m| vm.syntax_error(crysirror_python::Error::Runtime(m)))?;
            CODE.with(|cc| cc.borrow_mut().insert(key, c.clone()));
            c
        }
    };
    let g = store_globals(vm)?;
    let l = if py::is_none(locals) {
        None
    } else {
        Some(locals.clone())
    };
    vm.run_code(code, g, l)
}

/// Evaluates `e` with the store as globals and `locals` (None: the store).
pub fn eval(vm: &mut Vm, e: &PyExpr, locals: &Value) -> PyResult<Value> {
    run(vm, e, "eval", locals)
}

/// Executes the statements of `e` like `exec(code, store, locals)`.
pub fn exec(vm: &mut Vm, e: &PyExpr, locals: &Value) -> PyResult<()> {
    run(vm, e, "exec", locals).map(|_| ())
}

// Compiled form --------------------------------------------------------------------------------

pub enum Stmt {
    Block(Rc<Block>),
    Interp(Rc<Interp>),
    Repeat(Option<i64>),
    Parallel(Vec<Rc<Block>>),
    Choice(Vec<(f64, Rc<Block>)>),
    Time,
    On(Vec<(String, Rc<Block>)>),
    Event(String),
    Function(Value),
    Child { child: Value, transition: Value },
}

pub struct Block {
    pub(crate) stmts: Vec<Stmt>,
    pub(crate) times: Vec<(f64, usize)>,
}

pub struct Interp {
    pub(crate) warper: Value,
    pub(crate) warper_name: Option<String>,
    pub(crate) duration: f64,
    pub(crate) properties: Vec<(String, Value)>,
    pub(crate) revolution: Option<String>,
    pub(crate) circles: f64,
    pub(crate) splines: Vec<(String, Vec<Value>)>,
}

#[derive(Clone)]
pub enum State {
    Block {
        index: usize,
        start: f64,
        loop_start: f64,
        repeats: i64,
        times: Vec<(f64, usize)>,
        child: Option<Rc<State>>,
    },
    Interp(Rc<InterpState>),
    Parallel(Vec<(Rc<Block>, Option<Rc<State>>)>),
    Choice(Rc<Block>, Option<Rc<State>>),
    On {
        name: Option<String>,
        start: f64,
        child: Option<Rc<State>>,
    },
    FunctionBlocking,
}

pub struct InterpState {
    linear: Vec<(String, Value, Value)>,
    angles: Option<(f64, f64)>,
    radii: Option<(Value, Value)>,
    anchorangles: Option<(Value, Value)>,
    anchorradii: Option<(Value, Value)>,
    splines: Vec<(String, Vec<Value>)>,
}

enum Action {
    Continue(Option<Rc<State>>),
    Next(f64),
    Repeat(Option<i64>, f64),
    Event(String, f64),
}

struct Exec {
    action: Action,
    pause: Option<f64>,
}

fn is_atl_base(vm: &mut Vm, v: &Value) -> bool {
    py::import_attr(vm, MODULE, "ATLTransformBase")
        .ok()
        .and_then(|c| c.obj().cloned())
        .is_some_and(|c| ops::isinstance(vm, v, &c))
}

fn compile_block(vm: &mut Vm, raw: &RawBlock, ctx: &Value) -> PyResult<Rc<Block>> {
    let mut stmts = Vec::new();
    for s in &raw.statements {
        stmts.push(compile_stmt(vm, s, ctx)?);
    }
    let mut times = Vec::new();
    for (i, s) in raw.statements.iter().enumerate() {
        if let RawStatement::Time { time, .. } = s {
            let v = eval(vm, time, ctx)?;
            times.push((py::f64_of(vm, &v)?, i + 1));
        }
    }
    times.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(Rc::new(Block { stmts, times }))
}

fn set_exception_info(vm: &mut Vm, what: &str, loc: &(Rc<str>, u32)) {
    if let Ok(g) = py::import(vm, "renpy.game") {
        let v = vm
            .types
            .new_string(format!("{what} ATL code at {}:{}", loc.0, loc.1));
        let _ = py::set(vm, &g, "exception_info", v);
    }
}

fn compile_stmt(vm: &mut Vm, s: &RawStatement, ctx: &Value) -> PyResult<Stmt> {
    Ok(match s {
        RawStatement::Block(b) => Stmt::Block(compile_block(vm, b, ctx)?),
        RawStatement::Repeat { loc, repeats } => {
            set_exception_info(vm, "Compiling", loc);
            let r = match repeats {
                Some(e) => {
                    let v = eval(vm, e, ctx)?;
                    Some(py::int(vm, &v)?)
                }
                None => None,
            };
            Stmt::Repeat(r)
        }
        RawStatement::Parallel { blocks, .. } => {
            let mut out = Vec::new();
            for b in blocks {
                out.push(compile_block(vm, b, ctx)?);
            }
            Stmt::Parallel(out)
        }
        RawStatement::Choice { loc, choices } => {
            set_exception_info(vm, "Compiling", loc);
            let mut out = Vec::new();
            for (chance, b) in choices {
                let c = eval(vm, chance, ctx)?;
                out.push((py::f64_of(vm, &c)?, compile_block(vm, b, ctx)?));
            }
            Stmt::Choice(out)
        }
        RawStatement::On { handlers, .. } => {
            let mut out = Vec::new();
            for (name, b) in handlers {
                out.push((name.clone(), compile_block(vm, b, ctx)?));
            }
            Stmt::On(out)
        }
        RawStatement::Time { .. } => Stmt::Time,
        RawStatement::Event { name, .. } => Stmt::Event(name.clone()),
        RawStatement::Function { loc, expr } => {
            set_exception_info(vm, "Compiling", loc);
            Stmt::Function(eval(vm, expr, ctx)?)
        }
        RawStatement::ContainsExpr { loc, expr } => {
            set_exception_info(vm, "Compiling", loc);
            let child = eval(vm, expr, ctx)?;
            let child = super::easy::displayable(vm, &child, &Value::None, true)?;
            Stmt::Child {
                child,
                transition: Value::None,
            }
        }
        RawStatement::Child { children, .. } => {
            let mb = py::import_attr(vm, "renpy.display.layout", "MultiBox")?;
            let box_ = vm.call(
                &mb,
                Vec::new(),
                vec![("layout".into(), vm.types.new_str("fixed"))],
            )?;
            for c in children {
                let t = new_atl_transform(vm, Rc::new(c.clone()), None, ctx.clone(), Value::None)?;
                vm.call_method(&box_, "add", vec![t])?;
            }
            Stmt::Child {
                child: box_,
                transition: Value::None,
            }
        }
        RawStatement::Multipurpose(m) => {
            if let Some(loc) = &m.loc {
                set_exception_info(vm, "Compiling", loc);
            }
            if m.warper.is_none()
                && m.warp_function.is_none()
                && m.properties.is_empty()
                && m.splines.is_empty()
                && m.expressions.len() == 1
            {
                let (expr, with) = &m.expressions[0];
                let child = eval(vm, expr, ctx)?;
                let transition = match with {
                    Some(w) => eval(vm, w, ctx)?,
                    None => Value::None,
                };
                if matches!(child, Value::Int(_) | Value::Float(_)) {
                    let d = py::f64_of(vm, &child)?;
                    let w = warper_value(vm, "pause")?;
                    return Ok(Stmt::Interp(Rc::new(Interp {
                        warper: w,
                        warper_name: Some("pause".into()),
                        duration: d,
                        properties: Vec::new(),
                        revolution: None,
                        circles: 0.,
                        splines: Vec::new(),
                    })));
                }
                let child = super::easy::displayable(vm, &child, &Value::None, true)?;
                if is_atl_base(vm, &child) {
                    let c = py::get_or_none(vm, &child, "child")?;
                    if py::is_none(&c) {
                        let b = compiled_block(vm, &child)?;
                        return Ok(Stmt::Block(b));
                    }
                }
                return Ok(Stmt::Child { child, transition });
            }
            let (warper, warper_name) = match &m.warp_function {
                Some(f) => (eval(vm, f, ctx)?, None),
                None => {
                    let name = m.warper.clone().unwrap_or_else(|| "instant".into());
                    (warper_value(vm, &name)?, Some(name))
                }
            };
            let mut properties = Vec::new();
            for (name, expr) in &m.properties {
                if !is_known_property(vm, name)? {
                    return Err(py::error(
                        vm,
                        format!("ATL Property {name} is unknown at runtime."),
                    ));
                }
                properties.push((name.clone(), eval(vm, expr, ctx)?));
            }
            let mut splines = Vec::new();
            for (name, exprs) in &m.splines {
                let mut vals = Vec::new();
                for e in exprs {
                    vals.push(eval(vm, e, ctx)?);
                }
                splines.push((name.clone(), vals));
            }
            for (expr, _) in &m.expressions {
                let v = eval(vm, expr, ctx)?;
                if !is_atl_base(vm, &v) {
                    return Err(py::error(
                        vm,
                        format!(
                            "Expression {:?} is not an ATL transform, and so cannot be included in an ATL interpolation.",
                            expr.text
                        ),
                    ));
                }
                compiled_block(vm, &v)?;
                let props = atl_properties(vm, &v)?.ok_or_else(|| py::error(vm, format!("ATL transform {:?} is too complicated to be included in interpolation.", expr.text)))?;
                properties.extend(props);
            }
            let duration = match &m.duration {
                Some(d) => {
                    let v = eval(vm, d, ctx)?;
                    py::f64_of(vm, &v)?
                }
                None => 0.,
            };
            let circles = match &m.circles {
                Some(c) => {
                    let v = eval(vm, c, ctx)?;
                    py::f64_of(vm, &v)?
                }
                None => 0.,
            };
            Stmt::Interp(Rc::new(Interp {
                warper,
                warper_name,
                duration,
                properties,
                revolution: m.revolution.clone(),
                circles,
                splines,
            }))
        }
    })
}

fn is_known_property(vm: &mut Vm, name: &str) -> PyResult<bool> {
    let p = py::import_attr(vm, MODULE, "PROPERTIES")?;
    let k = vm.types.new_str(name);
    Ok(ops::contains(vm, &p, &k)? || name.starts_with("u_") || name.starts_with("gl_"))
}

fn warper_value(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let w = py::import_attr(vm, MODULE, "warpers")?;
    let k = vm.types.new_str(name);
    let v = vm.call_method(&w, "get", vec![k, Value::None])?;
    if py::is_none(&v) {
        return Err(py::error(
            vm,
            format!("ATL Warper {name} is unknown at runtime."),
        ));
    }
    Ok(v)
}

// Interpolation ----------------------------------------------------------------------------------

fn prop_type(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let p = py::import_attr(vm, MODULE, "PROPERTIES")?;
    let k = vm.types.new_str(name);
    vm.call_method(&p, "get", vec![k, Value::None])
}

fn is_named(v: &Value, name: &str) -> bool {
    match v.payload() {
        Some(crysirror_python::Payload::Builtin(b)) => b.name == name,
        Some(crysirror_python::Payload::Function(f)) => &**f.name.borrow() == name,
        Some(crysirror_python::Payload::Type(_)) => {
            v.obj().is_some_and(|o| o.class_name() == "type") && false
        }
        _ => false,
    }
}

fn is_position_type(vm: &mut Vm, typ: &Value) -> PyResult<bool> {
    let pos = py::import_attr(vm, MODULE, "position")?;
    let pon = py::import_attr(vm, MODULE, "position_or_none")?;
    Ok(py::same(typ, &pos) || py::same(typ, &pon) || is_named(typ, "position_or_none"))
}

pub fn interpolate(vm: &mut Vm, t: f64, a: &Value, b: &Value, typ: &Value) -> PyResult<Value> {
    let simple = py::is_none(b)
        || matches!(b, Value::Bool(_))
        || b.as_str().is_some()
        || b.obj()
            .is_some_and(|o| matches!(o.class_name().as_str(), "Matrix" | "Camera"));
    if simple {
        return Ok(if t >= 1. { b.clone() } else { a.clone() });
    }
    if let Some(bt) =
        py::seq(b).filter(|_| matches!(b.payload(), Some(crysirror_python::Payload::Tuple(_))))
    {
        let at: Vec<Value> = if py::is_none(a) {
            vec![Value::None; bt.len()]
        } else {
            py::items(vm, a)?
        };
        let tt: Vec<Value> = match py::seq(typ)
            .filter(|_| matches!(typ.payload(), Some(crysirror_python::Payload::Tuple(_))))
        {
            Some(t) => t,
            None => vec![typ.clone(); bt.len()],
        };
        let mut out = Vec::new();
        for i in 0..bt.len() {
            out.push(interpolate(
                vm,
                t,
                at.get(i).unwrap_or(&Value::None),
                &bt[i],
                tt.get(i).unwrap_or(&Value::None),
            )?);
        }
        return Ok(vm.types.new_tuple(out));
    }
    if crysirror_python::builtins::is_callable(vm, b)
        && !matches!(b, Value::Int(_) | Value::Float(_))
        && b.obj().is_some_and(|o| {
            !matches!(
                o.payload,
                crysirror_python::Payload::Float(_) | crysirror_python::Payload::Int(_)
            )
        })
        && b.obj().is_some_and(|o| {
            o.class_name() != "position"
                && o.class_name() != "absolute"
                && o.class_name() != "DualAngle"
        })
    {
        let origin = if py::is_none(a) {
            Value::None
        } else {
            py::get_opt(vm, a, "origin")?.unwrap_or(Value::None)
        };
        let rv = vm.call(b, vec![origin, Value::Float(t)], Vec::new())?;
        let _ = py::set(vm, &rv, "origin", b.clone());
        return Ok(rv);
    }
    let a = if py::is_none(a) {
        Value::Int(0)
    } else {
        a.clone()
    };
    if is_position_type(vm, typ)? {
        if py::config_bool(vm, "mixed_position").unwrap_or(false) {
            let (aa, ar) = super::transform::pos_parts(vm, &a)?;
            let (ba, br) = super::transform::pos_parts(vm, b)?;
            let pos = py::import_attr(vm, MODULE, "position")?;
            return vm.call(
                &pos,
                vec![
                    Value::Float(aa + t * (ba - aa)),
                    Value::Float(ar + t * (br - ar)),
                ],
                Vec::new(),
            );
        }
        let ty = Value::Obj(ops::type_of(vm, b));
        let diff = ops::binary_op(vm, b, &a, crysirror_python::ast::BinOp::Sub, false, false)?;
        let scaled = ops::binary_op(
            vm,
            &Value::Float(t),
            &diff,
            crysirror_python::ast::BinOp::Mult,
            false,
            false,
        )?;
        let sum = ops::binary_op(
            vm,
            &a,
            &scaled,
            crysirror_python::ast::BinOp::Add,
            false,
            false,
        )?;
        return vm.call(&ty, vec![sum], Vec::new());
    }
    let diff = ops::binary_op(vm, b, &a, crysirror_python::ast::BinOp::Sub, false, false)?;
    let scaled = ops::binary_op(
        vm,
        &Value::Float(t),
        &diff,
        crysirror_python::ast::BinOp::Mult,
        false,
        false,
    )?;
    let sum = ops::binary_op(
        vm,
        &a,
        &scaled,
        crysirror_python::ast::BinOp::Add,
        false,
        false,
    )?;
    if py::is_none(typ) {
        return Ok(sum);
    }
    vm.call(typ, vec![sum], Vec::new())
}

fn spline_point(vm: &mut Vm, t: f64, s: &[Value]) -> PyResult<Value> {
    let mul = |vm: &mut Vm, k: f64, v: &Value| {
        ops::binary_op(
            vm,
            &Value::Float(k),
            v,
            crysirror_python::ast::BinOp::Mult,
            false,
            false,
        )
    };
    let add = |vm: &mut Vm, a: Value, b: Value| {
        ops::binary_op(vm, &a, &b, crysirror_python::ast::BinOp::Add, false, false)
    };
    let n = s.len();
    Ok(match n {
        2 => {
            let a = mul(vm, 1. - t, &s[0])?;
            let b = mul(vm, t, &s[1])?;
            add(vm, a, b)?
        }
        3 => {
            let a = mul(vm, (1. - t).powi(2), &s[0])?;
            let b = mul(vm, 2. * t * (1. - t), &s[1])?;
            let c = mul(vm, t * t, &s[2])?;
            let ab = add(vm, a, b)?;
            add(vm, ab, c)?
        }
        4 => {
            let a = mul(vm, (1. - t).powi(3), &s[0])?;
            let b = mul(vm, 3. * t * (1. - t).powi(2), &s[1])?;
            let c = mul(vm, 3. * t * t * (1. - t), &s[2])?;
            let d = mul(vm, t.powi(3), &s[3])?;
            let ab = add(vm, a, b)?;
            let cd = add(vm, c, d)?;
            add(vm, ab, cd)?
        }
        _ if t <= 0. => s[0].clone(),
        _ if t >= 1. => s[n - 1].clone(),
        _ => {
            let mut sp: Vec<Value> = vec![s[1].clone(), s[0].clone()];
            sp.extend(s[2..n - 2].iter().cloned());
            sp.push(s[n - 1].clone());
            sp.push(s[n - 2].clone());
            let inner = (n - 3) as f64;
            let sector = ((t / (1. / inner)).floor() as usize + 1).min(n - 3);
            let tt = (t % (1. / inner)) * inner;
            let tt = tt.clamp(0., 1.);
            let p = &sp[sector - 1..sector + 3];
            let c0 = tt * ((2. - tt) * tt - 1.);
            let c1 = tt * tt * (3. * tt - 5.) + 2.;
            let c2 = tt * ((4. - 3. * tt) * tt + 1.);
            let c3 = (tt - 1.) * tt * tt;
            let a = mul(vm, c0 / 2., &p[0])?;
            let b = mul(vm, c1 / 2., &p[1])?;
            let c = mul(vm, c2 / 2., &p[2])?;
            let d = mul(vm, c3 / 2., &p[3])?;
            let ab = add(vm, a, b)?;
            let cd = add(vm, c, d)?;
            add(vm, ab, cd)?
        }
    })
}

pub fn interpolate_spline(vm: &mut Vm, t: f64, spline: &[Value], typ: &Value) -> PyResult<Value> {
    let last = spline.last().cloned().unwrap_or(Value::None);
    if matches!(last.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        let cols: Vec<Vec<Value>> = spline
            .iter()
            .map(|v| py::seq(v).unwrap_or_default())
            .collect();
        let w = cols.last().map(|c| c.len()).unwrap_or(0);
        let types = py::seq(typ).unwrap_or_else(|| vec![typ.clone(); w]);
        let mut out = Vec::new();
        for i in 0..w {
            let col: Vec<Value> = cols
                .iter()
                .map(|c| c.get(i).cloned().unwrap_or(Value::None))
                .collect();
            out.push(interpolate_spline(
                vm,
                t,
                &col,
                types.get(i).unwrap_or(&Value::None),
            )?);
        }
        return Ok(vm.types.new_tuple(out));
    }
    if spline.first().is_none_or(py::is_none) {
        return Ok(last);
    }
    let mut pts = spline.to_vec();
    if py::config_bool(vm, "mixed_position").unwrap_or(false) && is_position_type(vm, typ)? {
        let pon = py::import_attr(vm, MODULE, "position_or_none")?;
        for p in pts.iter_mut() {
            *p = vm.call(&pon, vec![p.clone()], Vec::new())?;
        }
    }
    let rv = spline_point(vm, t, &pts)?;
    if py::is_none(&rv) {
        return Ok(Value::None);
    }
    let ty = Value::Obj(ops::type_of(vm, pts.last().unwrap()));
    vm.call(&ty, vec![rv], Vec::new())
}

// Execution --------------------------------------------------------------------------------------

fn trans_state(vm: &mut Vm, trans: &Value) -> PyResult<Value> {
    py::get(vm, trans, "state")
}

fn exec_block(
    vm: &mut Vm,
    b: &Rc<Block>,
    trans: &Value,
    st: f64,
    state: Option<&Rc<State>>,
    events: &[String],
) -> PyResult<Exec> {
    let (mut index, mut start, mut loop_start, mut repeats, mut times, mut child) =
        match state.map(|s| &**s) {
            Some(State::Block {
                index,
                start,
                loop_start,
                repeats,
                times,
                child,
            }) => (
                *index,
                *start,
                *loop_start,
                *repeats,
                times.clone(),
                child.clone(),
            ),
            _ => (0, 0., 0., 0, b.times.clone(), None),
        };
    let fresh = state.is_none();
    loop {
        let (target, max_pause) = match times.first() {
            Some((time, _)) => (time.min(st), time - time.min(st)),
            None => (st, 15.),
        };
        let mut result;
        loop {
            if index >= b.stmts.len() {
                return Ok(Exec {
                    action: Action::Next(target - start),
                    pause: None,
                });
            }
            let r = exec_stmt(
                vm,
                &b.stmts[index],
                trans,
                target - start,
                child.as_ref(),
                events,
            )?;
            match r.action {
                Action::Continue(arg) => {
                    let pause = r.pause.unwrap_or(max_pause).min(max_pause);
                    let ns = State::Block {
                        index,
                        start,
                        loop_start,
                        repeats,
                        times: times.clone(),
                        child: arg,
                    };
                    result = Exec {
                        action: Action::Continue(Some(Rc::new(ns))),
                        pause: Some(pause),
                    };
                    break;
                }
                Action::Event(n, t) => {
                    return Ok(Exec {
                        action: Action::Event(n, t),
                        pause: r.pause,
                    });
                }
                Action::Next(arg) => {
                    index += 1;
                    start = target - arg;
                    child = None;
                }
                Action::Repeat(count, arg) => {
                    let loop_end = target - arg;
                    let duration = loop_end - loop_start;
                    if fresh && duration <= 0. {
                        return Err(py::error(vm, "ATL appears to be in an infinite loop."));
                    }
                    let mut new_repeats = if duration != 0. {
                        ((target - loop_start) / duration) as i64
                    } else {
                        0
                    };
                    if let Some(c) = count {
                        if repeats + new_repeats >= c {
                            new_repeats = c - repeats;
                            loop_start += new_repeats as f64 * duration;
                            return Ok(Exec {
                                action: Action::Next(target - loop_start),
                                pause: None,
                            });
                        }
                    }
                    repeats += new_repeats;
                    loop_start += new_repeats as f64 * duration;
                    start = loop_start;
                    index = 0;
                    child = None;
                }
            }
        }
        if let Some((time, tindex)) = times.first().cloned() {
            if time <= target {
                times.remove(0);
                index = tindex;
                start = time;
                child = None;
                continue;
            }
        }
        if let Action::Continue(Some(s)) = &result.action {
            if let State::Block { times: t, .. } = &**s {
                if t.len() != times.len() {
                    let ns = State::Block {
                        index,
                        start,
                        loop_start,
                        repeats,
                        times: times.clone(),
                        child: child.clone(),
                    };
                    result.action = Action::Continue(Some(Rc::new(ns)));
                }
            }
        }
        return Ok(result);
    }
}

fn exec_stmt(
    vm: &mut Vm,
    s: &Stmt,
    trans: &Value,
    st: f64,
    state: Option<&Rc<State>>,
    events: &[String],
) -> PyResult<Exec> {
    match s {
        Stmt::Block(b) => exec_block(vm, b, trans, st, state, events),
        Stmt::Interp(i) => exec_interp(vm, i, trans, st, state),
        Stmt::Repeat(r) => Ok(Exec {
            action: Action::Repeat(*r, st),
            pause: Some(0.),
        }),
        Stmt::Time => Ok(Exec {
            action: Action::Continue(None),
            pause: None,
        }),
        Stmt::Event(n) => Ok(Exec {
            action: Action::Event(n.clone(), st),
            pause: None,
        }),
        Stmt::Child { child, transition } => {
            let old = py::get_or_none(vm, trans, "raw_child")?;
            let mut c = child.clone();
            let dup = py::get_or_none(vm, &c, "_duplicatable")?;
            if ops::truthy(vm, &dup)? {
                let a = py::get_or_none(vm, trans, "_args")?;
                c = vm.call_method(child, "_duplicate", vec![a])?;
                vm.call_method(&c, "_unique", Vec::new())?;
            }
            let null = py::import_attr(vm, "renpy.display.motion", "null").unwrap_or(Value::None);
            if !py::is_none(&old) && !py::same(&old, &null) && !py::is_none(transition) {
                c = vm.call(
                    transition,
                    Vec::new(),
                    vec![("old_widget".into(), old), ("new_widget".into(), c)],
                )?;
                vm.call_method(&c, "_unique", Vec::new())?;
            }
            let f = py::get(vm, trans, "set_child")?;
            vm.call(&f, vec![c], vec![("duplicate".into(), Value::Bool(false))])?;
            py::set(vm, trans, "raw_child", child.clone())?;
            Ok(Exec {
                action: Action::Next(st),
                pause: None,
            })
        }
        Stmt::Function(f) => {
            let always = py::config_bool(vm, "atl_function_always_blocks").unwrap_or(false);
            let mut block = state.is_some() || always;
            let at = py::get_or_none(vm, trans, "at")?;
            let mut fr = vm.call(
                f,
                vec![
                    trans.clone(),
                    Value::Float(if block { st } else { 0. }),
                    at.clone(),
                ],
                Vec::new(),
            )?;
            if !block && !py::is_none(&fr) {
                block = true;
                fr = vm.call(f, vec![trans.clone(), Value::Float(st), at], Vec::new())?;
            }
            if !py::is_none(&fr) {
                let p = py::f64_of(vm, &fr)?;
                Ok(Exec {
                    action: Action::Continue(Some(Rc::new(State::FunctionBlocking))),
                    pause: Some(p),
                })
            } else {
                Ok(Exec {
                    action: Action::Next(if block { 0. } else { st }),
                    pause: None,
                })
            }
        }
        Stmt::Parallel(blocks) => {
            let states: Vec<(Rc<Block>, Option<Rc<State>>)> = match state.map(|s| &**s) {
                Some(State::Parallel(v)) => v.clone(),
                _ => blocks.iter().map(|b| (b.clone(), None)).collect(),
            };
            let mut left = Vec::new();
            let mut pauses = Vec::new();
            let mut new = Vec::new();
            for (b, s) in states {
                let r = exec_block(vm, &b, trans, st, s.as_ref(), events)?;
                if let Some(p) = r.pause {
                    pauses.push(p);
                }
                match r.action {
                    Action::Continue(arg) => new.push((b, arg)),
                    Action::Next(a) => left.push(a),
                    Action::Event(n, t) => {
                        return Ok(Exec {
                            action: Action::Event(n, t),
                            pause: r.pause,
                        });
                    }
                    Action::Repeat(..) => {}
                }
            }
            if !new.is_empty() {
                let p = pauses.into_iter().fold(f64::MAX, f64::min);
                Ok(Exec {
                    action: Action::Continue(Some(Rc::new(State::Parallel(new)))),
                    pause: Some(p),
                })
            } else {
                Ok(Exec {
                    action: Action::Next(left.into_iter().fold(f64::MAX, f64::min)),
                    pause: None,
                })
            }
        }
        Stmt::Choice(choices) => {
            let (choice, cstate) = match state.map(|s| &**s) {
                Some(State::Choice(b, s)) => (b.clone(), s.clone()),
                _ => {
                    let total: f64 = choices.iter().map(|c| c.0).sum();
                    let r = py::import_attr(vm, "random", "uniform")?;
                    let n = vm.call(&r, vec![Value::Float(0.), Value::Float(total)], Vec::new())?;
                    let mut n = py::f64_of(vm, &n)?;
                    let mut pick = choices.last().map(|c| c.1.clone());
                    for (chance, b) in choices {
                        if n < *chance {
                            pick = Some(b.clone());
                            break;
                        }
                        n -= chance;
                    }
                    (
                        pick.ok_or_else(|| py::error(vm, "ATL choice without blocks"))?,
                        None,
                    )
                }
            };
            let r = exec_block(vm, &choice, trans, st, cstate.as_ref(), events)?;
            match r.action {
                Action::Continue(arg) => Ok(Exec {
                    action: Action::Continue(Some(Rc::new(State::Choice(choice, arg)))),
                    pause: r.pause,
                }),
                a => Ok(Exec {
                    action: a,
                    pause: None,
                }),
            }
        }
        Stmt::On(handlers) => {
            let (mut name, mut start, mut cstate) = match state.map(|s| &**s) {
                Some(State::On { name, start, child }) => (name.clone(), *start, child.clone()),
                _ => (Some("start".to_owned()), st, None),
            };
            let find = |n: &str| handlers.iter().find(|h| h.0 == n).map(|h| h.1.clone());
            let hide_req = {
                let v = py::get_or_none(vm, trans, "hide_request")?;
                ops::truthy(vm, &v)?
            };
            let repl_req = {
                let v = py::get_or_none(vm, trans, "replaced_request")?;
                ops::truthy(vm, &v)?
            };
            for ev in events {
                let mut e = ev.clone();
                while !e.is_empty() && find(&e).is_none() {
                    e = e
                        .split_once('_')
                        .map(|(_, r)| r.to_owned())
                        .unwrap_or_default();
                }
                if e.is_empty() {
                    continue;
                }
                let lock = (name.as_deref() == Some("hide") && hide_req)
                    || (name.as_deref() == Some("replaced") && repl_req);
                if !lock {
                    name = Some(e);
                    start = st;
                    cstate = None;
                }
            }
            loop {
                let Some(n) = name.clone() else {
                    return Ok(Exec {
                        action: Action::Continue(Some(Rc::new(State::On {
                            name,
                            start,
                            child: cstate,
                        }))),
                        pause: None,
                    });
                };
                let Some(h) = find(&n) else {
                    return Ok(Exec {
                        action: Action::Continue(Some(Rc::new(State::On {
                            name,
                            start,
                            child: cstate,
                        }))),
                        pause: None,
                    });
                };
                let r = exec_block(vm, &h, trans, st - start, cstate.as_ref(), events)?;
                match r.action {
                    Action::Continue(arg) => {
                        if n == "hide" || n == "replaced" {
                            py::set(vm, trans, "hide_response", Value::Bool(false))?;
                            py::set(vm, trans, "replaced_response", Value::Bool(false))?;
                        }
                        return Ok(Exec {
                            action: Action::Continue(Some(Rc::new(State::On {
                                name: Some(n),
                                start,
                                child: arg,
                            }))),
                            pause: r.pause,
                        });
                    }
                    Action::Next(arg) => {
                        name = if matches!(n.as_str(), "default" | "hide" | "replaced") {
                            None
                        } else {
                            Some("default".into())
                        };
                        start = st - arg;
                        cstate = None;
                    }
                    Action::Event(en, arg) => {
                        if find(&en).is_some() {
                            start = (st - arg).max(st - 30.);
                            name = Some(en);
                            cstate = None;
                            continue;
                        }
                        return Ok(Exec {
                            action: Action::Event(en, arg),
                            pause: None,
                        });
                    }
                    Action::Repeat(..) => {
                        return Ok(Exec {
                            action: Action::Next(0.),
                            pause: None,
                        });
                    }
                }
            }
        }
    }
}

fn exec_interp(
    vm: &mut Vm,
    i: &Interp,
    trans: &Value,
    st: f64,
    state: Option<&Rc<State>>,
) -> PyResult<Exec> {
    let force_frame = state.is_none()
        && i.warper_name.as_deref() == Some("pause")
        && i.duration == 0.
        && py::config_bool(vm, "atl_one_frame").unwrap_or(true);
    let mut complete = if i.duration != 0. {
        (st / i.duration).min(1.)
    } else {
        1.
    };
    complete = complete.clamp(0., 1.);
    let c = vm.call(&i.warper, vec![Value::Float(complete)], Vec::new())?;
    let complete = py::f64_of(vm, &c)?;
    let ts = trans_state(vm, trans)?;
    let istate = match state.map(|s| &**s) {
        Some(State::Interp(s)) => s.clone(),
        _ => {
            let tsc = py::import_attr(vm, "renpy.display.transform", "TransformState")?;
            let newts = vm.call(&tsc, Vec::new(), Vec::new())?;
            vm.call_method(&newts, "take_state", vec![ts.clone()])?;
            let (mut has_angle, mut has_radius, mut has_aa, mut has_ar) =
                (false, false, false, false);
            for (k, v) in &i.properties {
                py::set(vm, &newts, k, v.clone())?;
                match k.as_str() {
                    "angle" => has_angle = true,
                    "radius" => has_radius = true,
                    "anchorangle" => has_aa = true,
                    "anchorradius" => has_ar = true,
                    _ => {}
                }
            }
            let diff = vm.call_method(&ts, "diff", vec![newts.clone()])?;
            let mut linear: Vec<(String, Value, Value)> = Vec::new();
            for (k, v) in py::str_items(vm, &diff)? {
                let p = py::items(vm, &v)?;
                linear.push((k.to_string(), p[0].clone(), p[1].clone()));
            }
            let (mut angles, mut radii, mut aangles, mut aradii) = (None, None, None, None);
            let auto_polar = py::config_bool(vm, "automatic_polar_motion").unwrap_or(true);
            if i.revolution.is_some()
                || ((has_angle || has_radius) && auto_polar)
                || has_aa
                || has_ar
            {
                linear.retain(|l| {
                    !matches!(
                        l.0.as_str(),
                        "xpos"
                            | "ypos"
                            | "xanchor"
                            | "yanchor"
                            | "xaround"
                            | "yaround"
                            | "xanchoraround"
                            | "yanchoraround"
                    )
                });
                if let Some(rev) = &i.revolution {
                    for (n, d) in [("xaround", 0.), ("yaround", 0.)] {
                        let v = py::get_or_none(vm, &newts, n)?;
                        let nv = if ops::truthy(vm, &v)? {
                            v
                        } else {
                            Value::Float(d)
                        };
                        py::set(vm, &ts, n, nv)?;
                    }
                    for n in ["xanchoraround", "yanchoraround"] {
                        let v = py::get_or_none(vm, &newts, n)?;
                        py::set(vm, &ts, n, v)?;
                    }
                    let sa = py::get(vm, &ts, "angle")?;
                    let mut sa = py::f64_of(vm, &sa)?;
                    let ea = py::get(vm, &newts, "angle")?;
                    let ea = py::f64_of(vm, &ea)?;
                    let sr = py::get(vm, &ts, "radius")?;
                    let er = py::get(vm, &newts, "radius")?;
                    if rev == "clockwise" {
                        if ea < sa {
                            sa -= 360.;
                        }
                        sa -= i.circles * 360.;
                    } else if rev == "counterclockwise" {
                        if ea > sa {
                            sa += 360.;
                        }
                        sa += i.circles * 360.;
                    }
                    angles = Some((sa, ea));
                    radii = Some((sr, er));
                } else {
                    if has_angle {
                        let s = py::get(vm, &ts, "angle")?;
                        let mut s = py::f64_of(vm, &s)?;
                        let e = py::get_or_none(vm, &newts, "last_angle")?;
                        let e = py::f64_of(vm, &e)?;
                        if e - s > 180. {
                            s += 360.;
                        }
                        if e - s < -180. {
                            s -= 360.;
                        }
                        angles = Some((s, e));
                    }
                    if has_radius {
                        let a = py::get(vm, &ts, "radius")?;
                        let b = py::get(vm, &newts, "radius")?;
                        radii = Some((a, b));
                    }
                    if has_aa {
                        let a = py::get(vm, &ts, "anchorangle")?;
                        let b = py::get(vm, &newts, "anchorangle")?;
                        aangles = Some((a, b));
                    }
                    if has_ar {
                        let a = py::get(vm, &ts, "anchorradius")?;
                        let b = py::get(vm, &newts, "anchorradius")?;
                        aradii = Some((a, b));
                    }
                }
            }
            let mut splines = Vec::new();
            for (name, values) in &i.splines {
                let f = vm.call_method(&ts, "get", vec![vm.types.new_str(name)])?;
                let mut v = vec![f];
                v.extend(values.iter().cloned());
                splines.push((name.clone(), v));
            }
            let lin_names: Vec<String> = linear.iter().map(|l| l.0.clone()).collect();
            for (k, v) in &i.properties {
                if !lin_names.contains(k) {
                    py::set(vm, &ts, k, v.clone())?;
                }
            }
            Rc::new(InterpState {
                linear,
                angles,
                radii,
                anchorangles: aangles,
                anchorradii: aradii,
                splines,
            })
        }
    };
    for (k, old, new) in &istate.linear {
        let typ = prop_type(vm, k)?;
        let v = interpolate(vm, complete, old, new, &typ)?;
        py::set(vm, &ts, k, v)?;
    }
    if let Some((a, b)) = istate.angles {
        py::set(vm, &ts, "angle", Value::Float(a + complete * (b - a)))?;
    }
    if let Some((a, b)) = &istate.radii {
        let pon = py::import_attr(vm, MODULE, "position_or_none")?;
        let v = interpolate(vm, complete, a, b, &pon)?;
        py::set(vm, &ts, "radius", v)?;
    }
    if let Some((a, b)) = &istate.anchorangles {
        let v = interpolate(vm, complete, a, b, &Value::None)?;
        py::set(vm, &ts, "anchorangle", v)?;
    }
    if let Some((a, b)) = &istate.anchorradii {
        let pon = py::import_attr(vm, MODULE, "position_or_none")?;
        let v = interpolate(vm, complete, a, b, &pon)?;
        py::set(vm, &ts, "anchorradius", v)?;
    }
    for (name, values) in &istate.splines {
        let typ = prop_type(vm, name)?;
        let v = interpolate_spline(vm, complete, values, &typ)?;
        py::set(vm, &ts, name, v)?;
    }
    if st >= i.duration && !force_frame {
        return Ok(Exec {
            action: Action::Next(st - i.duration),
            pause: None,
        });
    }
    let pause = if i.properties.is_empty() && i.revolution.is_none() && i.splines.is_empty() {
        (i.duration - st).max(0.)
    } else {
        0.
    };
    Ok(Exec {
        action: Action::Continue(Some(Rc::new(State::Interp(istate)))),
        pause: Some(pause),
    })
}

// Native wrappers --------------------------------------------------------------------------------

pub struct RawRef(pub Rc<RawBlock>);
pub struct BlockRef(pub Rc<Block>);
pub struct StateRef(pub Rc<State>);

fn raw_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "RawBlock")
}

pub fn raw_value(vm: &mut Vm, raw: Rc<RawBlock>) -> PyResult<Value> {
    let c = raw_class(vm)?;
    let c = c
        .obj()
        .cloned()
        .ok_or_else(|| vm.type_error("RawBlock class"))?;
    Ok(vm.types.new_native(&c, Box::new(RawRef(raw))))
}

pub fn raw_of(v: &Value) -> Option<Rc<RawBlock>> {
    py::native::<RawRef, _>(v, |r| r.0.clone())
}

fn native_class(vm: &mut Vm, name: &str) -> PyResult<ObjRef> {
    let c = py::import_attr(vm, MODULE, name)?;
    c.obj()
        .cloned()
        .ok_or_else(|| vm.type_error(name.to_owned()))
}

fn raw_getattr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = raw_of(&args[0]).ok_or_else(|| vm.type_error("RawBlock"))?;
    let name = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
    Ok(match name {
        "loc" => {
            let f = vm.types.new_str(&r.loc.0);
            vm.types.new_tuple(vec![f, Value::Int(r.loc.1 as i64)])
        }
        "animation" => Value::Bool(r.animation),
        "statements" => return super::atlclasses::raw_block_statements(vm, &r),
        "constant" => Value::Int(0),
        "compiled_block" => Value::None,
        _ => return Err(vm.attr_error(format!("'RawBlock' object has no attribute '{name}'"))),
    })
}

fn raw_compile(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = raw_of(&args[0]).ok_or_else(|| vm.type_error("RawBlock"))?;
    let ctx = args.get(1).cloned().unwrap_or(Value::None);
    let locals = if py::is_none(&ctx) {
        Value::None
    } else {
        py::get_or_none(vm, &ctx, "context")?
    };
    let b = compile_block(vm, &r, &locals)?;
    let c = native_class(vm, "Block")?;
    Ok(vm.types.new_native(&c, Box::new(BlockRef(b))))
}

fn raw_predict(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn raw_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = raw_of(&args[0]).ok_or_else(|| vm.type_error("RawBlock"))?;
    // Saves reference the ATL by script location; the loader resolves it
    // against the loaded script.
    let f = py::import_attr(vm, MODULE, "_raw_block_at")?;
    let file = vm.types.new_str(&r.loc.0);
    let a = vm.types.new_tuple(vec![file, Value::Int(r.loc.1 as i64)]);
    Ok(vm.types.new_tuple(vec![f, a]))
}

fn f_raw_block_at(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let file = py::string(vm, &args.first().cloned().unwrap_or(Value::None))?;
    let line = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(0)))? as u32;
    let found = crate::engine::find_atl(vm, &file, line);
    match found {
        Some(r) => raw_value(vm, r),
        None => Err(py::error(
            vm,
            format!("ATL block at {file}:{line} is not in the loaded script."),
        )),
    }
}

pub fn block_of(v: &Value) -> Option<Rc<Block>> {
    py::native::<BlockRef, _>(v, |b| b.0.clone())
}

fn block_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = block_of(&args[0]).ok_or_else(|| vm.type_error("Block"))?;
    let mut out = Vec::new();
    fn walk(b: &Block, out: &mut Vec<Value>) {
        for s in &b.stmts {
            match s {
                Stmt::Block(b) => walk(b, out),
                Stmt::Parallel(bs) => bs.iter().for_each(|b| walk(b, out)),
                Stmt::Choice(cs) => cs.iter().for_each(|c| walk(&c.1, out)),
                Stmt::On(hs) => hs.iter().for_each(|h| walk(&h.1, out)),
                Stmt::Child { child, .. } => out.push(child.clone()),
                _ => {}
            }
        }
    }
    walk(&b, &mut out);
    Ok(vm.types.new_list(out))
}

fn block_handles_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = block_of(&args[0]).ok_or_else(|| vm.type_error("Block"))?;
    let ev = args
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    fn walk(b: &Block, ev: &str) -> bool {
        b.stmts.iter().any(|s| match s {
            Stmt::Block(b) => walk(b, ev),
            Stmt::Parallel(bs) => bs.iter().any(|b| walk(b, ev)),
            Stmt::Choice(cs) => cs.iter().any(|c| walk(&c.1, ev)),
            Stmt::On(hs) => hs.iter().any(|h| h.0 == ev),
            Stmt::Function(_) => true,
            _ => false,
        })
    }
    Ok(Value::Bool(walk(&b, &ev)))
}

/// `ATLTransformBase.compile()` result, cached on the transform.
fn compiled_block(vm: &mut Vm, t: &Value) -> PyResult<Rc<Block>> {
    let b = py::get_or_none(vm, t, "block")?;
    if let Some(b) = block_of(&b) {
        return Ok(b);
    }
    let r = vm.call_method(t, "compile", Vec::new())?;
    block_of(&r).ok_or_else(|| vm.type_error("ATL compile did not produce a block"))
}

fn atl_properties(vm: &mut Vm, t: &Value) -> PyResult<Option<Vec<(String, Value)>>> {
    let b = compiled_block(vm, t)?;
    let mut props = Vec::new();
    for s in &b.stmts {
        match s {
            Stmt::Interp(i) if i.duration == 0. => props.extend(i.properties.iter().cloned()),
            _ => return Ok(None),
        }
    }
    Ok(Some(props))
}

// ATLTransformBase -------------------------------------------------------------------------------

fn ctx_dict(vm: &mut Vm, t: &Value) -> PyResult<Value> {
    let c = py::get_or_none(vm, t, "context")?;
    if py::is_none(&c) {
        return Ok(vm.types.new_dict());
    }
    py::get_or_none(vm, &c, "context")
}

fn new_context(vm: &mut Vm, d: Value) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "Context")?;
    vm.call(&c, vec![d], Vec::new())
}

fn signature_of(v: &Value) -> Option<Rc<Signature>> {
    py::native::<Rc<Signature>, _>(v, |s| s.clone())
}

fn base_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let atl = arg_or(args, kw, 1, "atl", Value::None);
    let mut context = arg_or(args, kw, 2, "context", Value::None);
    let parameters = arg_or(args, kw, 3, "parameters", Value::None);
    if py::is_none(&context) {
        context = vm.types.new_dict();
    }
    if !py::is_none(&parameters) {
        context = vm.call_method(&context, "copy", Vec::new())?;
        if let (Some(sig), Some(raw)) = (signature_of(&parameters), raw_of(&atl)) {
            apply_defaults(vm, &sig, &context, &raw)?;
        }
    }
    py::set(vm, this, "parameters", parameters)?;
    py::set(vm, this, "atl", atl)?;
    let c = new_context(vm, context)?;
    py::set(vm, this, "context", c)?;
    for n in [
        "block",
        "predict_block",
        "properties",
        "atl_state",
        "transform_event",
        "last_transform_event",
        "last_child_transform_event",
        "raw_child",
        "parent_transform",
    ] {
        py::set(vm, this, n, Value::None)?;
    }
    py::set(vm, this, "done", Value::Bool(false))?;
    let start_on_show = py::config_bool(vm, "atl_start_on_show").unwrap_or(true);
    py::set(
        vm,
        this,
        "atl_st_offset",
        if start_on_show {
            Value::None
        } else {
            Value::Int(0)
        },
    )?;
    Ok(Value::None)
}

fn apply_defaults(vm: &mut Vm, sig: &Signature, context: &Value, raw: &RawBlock) -> PyResult<()> {
    for p in &sig.parameters {
        let k = vm.types.new_str(&p.name);
        if ops::contains(vm, context, &k)? {
            continue;
        }
        if let Some(d) = &p.default {
            let e = PyExpr::new(d.as_str(), raw.loc.0.clone(), raw.loc.1);
            let v = eval(vm, &e, context)?;
            ops::setitem(vm, context, k, v)?;
        }
    }
    Ok(())
}

fn base_transition(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = ctx_dict(vm, &args[0])?;
    let k = vm.types.new_str("new_widget");
    Ok(Value::Bool(ops::contains(vm, &d, &k)?))
}

fn base_handles_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let st = py::get_or_none(vm, this, "atl_state")?;
    if ev.as_str() == Some("replaced") && py::is_none(&st) {
        return Ok(Value::Bool(true));
    }
    let b = py::get_or_none(vm, this, "block")?;
    if !py::is_none(&b) {
        let r = vm.call_method(&b, "_handles_event", vec![ev.clone()])?;
        if ops::truthy(vm, &r)? {
            return Ok(Value::Bool(true));
        }
    }
    let c = py::get_or_none(vm, this, "child")?;
    if py::is_none(&c) {
        return Ok(Value::Bool(false));
    }
    vm.call_method(&c, "_handles_event", vec![ev])
}

fn base_get_block(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = py::get_or_none(vm, &args[0], "block")?;
    if ops::truthy(vm, &b)? {
        return Ok(b);
    }
    Ok(Value::None)
}

fn base_take_execution_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = args.get(1).cloned().unwrap_or(Value::None);
    let tc = py::import_attr(vm, "renpy.display.transform", "Transform")?;
    let f = py::get(vm, &tc, "take_execution_state")?;
    vm.call(&f, vec![this.clone(), t.clone()], Vec::new())?;
    py::set(vm, this, "atl_st_offset", Value::None)?;
    py::set(vm, this, "atl_state", Value::None)?;
    if py::same(this, &t) || !is_atl_base(vm, &t) {
        return Ok(Value::None);
    }
    let a = py::get_or_none(vm, this, "atl")?;
    let b = py::get_or_none(vm, &t, "atl")?;
    let same = match (raw_of(&a), raw_of(&b)) {
        (Some(x), Some(y)) => Rc::ptr_eq(&x, &y),
        _ => false,
    };
    if !same {
        return Ok(Value::None);
    }
    // Different contexts compile different blocks; keep state only when the
    // evaluated context matches.
    let ca = py::get_or_none(vm, this, "context")?;
    let cb = py::get_or_none(vm, &t, "context")?;
    if !ops::eq(vm, &ca, &cb)? {
        return Ok(Value::None);
    }
    for n in [
        "done",
        "block",
        "atl_state",
        "transform_event",
        "last_transform_event",
        "last_child_transform_event",
        "st",
        "at",
        "st_offset",
        "at_offset",
        "atl_st_offset",
    ] {
        let v = py::get_or_none(vm, &t, n)?;
        py::set(vm, this, n, v)?;
    }
    let child = py::get_or_none(vm, this, "child")?;
    let null = py::import_attr(vm, "renpy.display.motion", "null").unwrap_or(Value::None);
    if py::same(&child, &null) || py::is_none(&child) {
        let tchild = py::get_or_none(vm, &t, "child")?;
        let mut c = tchild.clone();
        let dup = py::get_or_none(vm, &tchild, "_duplicatable")?;
        if ops::truthy(vm, &tchild)? && ops::truthy(vm, &dup)? {
            c = vm.call_method(&tchild, "_duplicate", vec![Value::None])?;
        }
        py::set(vm, this, "child", c)?;
        let rc = py::get_or_none(vm, &t, "raw_child")?;
        py::set(vm, this, "raw_child", rc)?;
    }
    Ok(Value::None)
}

fn base_call(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut kw: Vec<_> = kw.to_vec();
    let a = kw
        .iter()
        .position(|(k, _)| &**k == "_args")
        .map(|i| kw.remove(i).1)
        .unwrap_or(Value::None);
    let scope = {
        let d = ctx_dict(vm, this)?;
        vm.call_method(&d, "copy", Vec::new())?
    };
    let params = py::get_or_none(vm, this, "parameters")?;
    let sig = signature_of(&params);
    let mut pos = args[1..].to_vec();
    let mut child = Value::None;
    let has_named = sig
        .as_ref()
        .is_some_and(|s| s.parameters.iter().any(|p| p.kind <= 2));
    if !pos.is_empty() && !has_named {
        if pos.len() != 1 {
            return Err(py::error(vm, "Too many arguments passed to ATL transform."));
        }
        child = pos.remove(0);
    }
    let child_param = sig
        .as_ref()
        .and_then(|s| s.parameters.iter().find(|p| p.name == "child").cloned());
    if child_param
        .as_ref()
        .is_none_or(|p| p.kind != 1 && p.kind != 3)
    {
        if let Some(i) = kw.iter().position(|(k, _)| &**k == "child") {
            child = kw.remove(i).1;
        }
    } else if let Some((_, v)) = kw.iter().find(|(k, _)| &**k == "child") {
        child = v.clone();
    }
    if let Some(s) = &sig {
        if s.parameters
            .iter()
            .any(|p| p.name == "old_widget" && (p.kind == 1 || p.kind == 3))
        {
            if let Some((_, v)) = kw.iter().find(|(k, _)| &**k == "old_widget") {
                child = v.clone();
            }
        }
    }
    // Bind positional and keyword arguments into the scope.
    let mut remaining: Vec<crate::ast::Parameter> = Vec::new();
    if let Some(s) = &sig {
        let mut it = pos.into_iter();
        for p in &s.parameters {
            match p.kind {
                0 | 1 => {
                    if let Some(v) = it.next() {
                        if p.name == "child" && py::is_none(&child) {
                            child = v.clone();
                        }
                        py::dict_set(vm, &scope, &p.name, v)?;
                        continue;
                    }
                    if let Some(i) = kw.iter().position(|(k, _)| &**k == p.name) {
                        let v = kw.remove(i).1;
                        py::dict_set(vm, &scope, &p.name, v)?;
                        continue;
                    }
                    remaining.push(p.clone());
                }
                3 => {
                    if let Some(i) = kw.iter().position(|(k, _)| &**k == p.name) {
                        let v = kw.remove(i).1;
                        py::dict_set(vm, &scope, &p.name, v)?;
                        continue;
                    }
                    remaining.push(p.clone());
                }
                _ => remaining.push(p.clone()),
            }
        }
        if it.next().is_some() {
            return Err(py::error(vm, "Too many arguments passed to ATL transform."));
        }
        for (k, v) in kw.drain(..) {
            if !s.parameters.iter().any(|p| p.kind == 4) {
                return Err(vm.type_error(format!(
                    "unexpected keyword argument {k:?} to ATL transform"
                )));
            }
            py::dict_set(vm, &scope, &k, v)?;
        }
    } else if !kw.is_empty() || !pos.is_empty() {
        return Err(py::error(vm, "Too many arguments passed to ATL transform."));
    }
    if py::is_none(&child) {
        child = py::get_or_none(vm, this, "child")?;
    }
    let dup = py::get_opt(vm, &child, "_duplicatable")?.unwrap_or(Value::Bool(false));
    if ops::truthy(vm, &dup)? {
        child = vm.call_method(&child, "_duplicate", vec![a.clone()])?;
    }
    let new_sig = Signature {
        parameters: remaining,
    };
    let atl = py::get(vm, this, "atl")?;
    let style = py::get_or_none(vm, this, "style_arg")?;
    let sigv = signature_value(vm, new_sig)?;
    let c = py::import_attr(vm, "renpy.display.motion", "ATLTransform")?;
    let rv = vm.call(
        &c,
        Vec::new(),
        vec![
            ("atl".into(), atl),
            ("child".into(), child),
            ("style".into(), style),
            ("context".into(), scope),
            ("parameters".into(), sigv),
            ("_args".into(), a),
        ],
    )?;
    py::set(vm, &rv, "parent_transform", this.clone())?;
    vm.call_method(&rv, "take_state", vec![this.clone()])?;
    Ok(rv)
}

pub fn signature_value(vm: &mut Vm, s: Signature) -> PyResult<Value> {
    let c = native_class(vm, "Signature")?;
    Ok(vm.types.new_native(&c, Box::new(Rc::new(s))))
}

fn base_compile(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let params = py::get_or_none(vm, this, "parameters")?;
    let ctx = ctx_dict(vm, this)?;
    if let Some(sig) = signature_of(&params) {
        let mut missing = Vec::new();
        for p in &sig.parameters {
            if p.kind == 2 || p.kind == 4 {
                continue;
            }
            let k = vm.types.new_str(&p.name);
            if !ops::contains(vm, &ctx, &k)? {
                missing.push(p.name.clone());
            }
        }
        if !missing.is_empty() {
            let atl = py::get(vm, this, "atl")?;
            let loc = raw_of(&atl)
                .map(|r| format!("{}:{}", r.loc.0, r.loc.1))
                .unwrap_or_default();
            return Err(py::error(
                vm,
                format!(
                    "Cannot compile ATL Transform at {loc}, as it's missing parameters {}.",
                    missing
                        .iter()
                        .map(|m| format!("'{m}'"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
    }
    let parent = py::get_or_none(vm, this, "parent_transform")?;
    if !py::is_none(&parent) {
        let pb = py::get_or_none(vm, &parent, "block")?;
        let pc = py::get_or_none(vm, &parent, "context")?;
        let c = py::get_or_none(vm, this, "context")?;
        if !py::is_none(&pb) && ops::eq(vm, &pc, &c)? {
            py::set(vm, this, "block", pb.clone())?;
            let pp = py::get_or_none(vm, &parent, "properties")?;
            py::set(vm, this, "properties", pp)?;
            py::set(vm, this, "parent_transform", Value::None)?;
            return Ok(pb);
        }
    }
    let old = py::import_attr(vm, "renpy.game", "exception_info")?;
    let atl = py::get(vm, this, "atl")?;
    let c = py::get(vm, this, "context")?;
    let block = vm.call_method(&atl, "compile", vec![c])?;
    py::set(vm, this, "block", block.clone())?;
    let b = block_of(&block).unwrap();
    let all_instant = b
        .stmts
        .iter()
        .all(|s| matches!(s, Stmt::Interp(i) if i.duration == 0.));
    if all_instant {
        let mut props = Vec::new();
        for s in &b.stmts {
            if let Stmt::Interp(i) = s {
                for (k, v) in &i.properties {
                    let kk = vm.types.new_str(k);
                    props.push(vm.types.new_tuple(vec![kk, v.clone()]));
                }
            }
        }
        py::set(vm, this, "properties", vm.types.new_list(props))?;
    }
    let g = py::import(vm, "renpy.game")?;
    py::set(vm, &g, "exception_info", old)?;
    Ok(block)
}

fn base_execute(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let trans = args.get(1).cloned().unwrap_or(Value::None);
    let st = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let at = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let done = py::get_or_none(vm, this, "done")?;
    if ops::truthy(vm, &done)? {
        return Ok(Value::None);
    }
    let block = compiled_block(vm, this)?;
    let mut events: Vec<String> = Vec::new();
    let hr = py::get_or_none(vm, &trans, "hide_request")?;
    if ops::truthy(vm, &hr)? {
        py::set(vm, this, "transform_event", vm.types.new_str("hide"))?;
    }
    let rr = py::get_or_none(vm, &trans, "replaced_request")?;
    if ops::truthy(vm, &rr)? {
        py::set(vm, this, "transform_event", vm.types.new_str("replaced"))?;
    }
    let te = |vm: &mut Vm, n: &str| py::get_or_none(vm, this, n);
    if py::config_bool(vm, "atl_multiple_events").unwrap_or(true) {
        let cur = te(vm, "transform_event")?;
        let last = te(vm, "last_transform_event")?;
        if !ops::eq(vm, &cur, &last)? {
            if let Some(s) = cur.as_str() {
                events.push(s.to_owned());
            }
            py::set(vm, this, "last_transform_event", cur)?;
        }
    }
    let child = py::get_or_none(vm, this, "child")?;
    if !py::is_none(&child) {
        let cte = py::get_or_none(vm, &child, "transform_event")?;
        let lcte = te(vm, "last_child_transform_event")?;
        if !ops::eq(vm, &cte, &lcte)? {
            py::set(vm, this, "last_child_transform_event", cte.clone())?;
            if !py::is_none(&cte) {
                py::set(vm, this, "transform_event", cte)?;
            }
        }
    }
    let cur = te(vm, "transform_event")?;
    let last = te(vm, "last_transform_event")?;
    if !ops::eq(vm, &cur, &last)? {
        if let Some(s) = cur.as_str() {
            events.push(s.to_owned());
        }
        py::set(vm, this, "last_transform_event", cur.clone())?;
    }
    let rte = py::config(vm, "repeat_transform_events").unwrap_or(Value::None);
    if !py::is_none(&rte) && !py::is_none(&cur) && ops::contains(vm, &rte, &cur)? {
        py::set(vm, this, "transform_event", Value::None)?;
        py::set(vm, this, "last_transform_event", Value::None)?;
    }
    let old = py::import_attr(vm, "renpy.game", "exception_info")?;
    let aso = te(vm, "atl_st_offset")?;
    let aso = if py::is_none(&aso) || st - py::f64_of(vm, &aso)? < 0. {
        py::set(vm, this, "atl_st_offset", Value::Float(st))?;
        st
    } else {
        py::f64_of(vm, &aso)?
    };
    let atl = py::get(vm, this, "atl")?;
    let animation = raw_of(&atl).is_some_and(|r| r.animation);
    let transition = {
        let v = vm.call_method(this, "_transition", Vec::new())?;
        ops::truthy(vm, &v)?
    };
    let timebase = if animation || transition {
        at
    } else {
        st - aso
    };
    let state = py::get_or_none(vm, this, "atl_state")?;
    let state = py::native::<StateRef, _>(&state, |s| s.0.clone());
    let r = exec_block(vm, &block, &trans, timebase, state.as_ref(), &events)?;
    let g = py::import(vm, "renpy.game")?;
    py::set(vm, &g, "exception_info", old)?;
    match r.action {
        Action::Continue(Some(s)) => {
            let c = native_class(vm, "State")?;
            let v = vm.types.new_native(&c, Box::new(StateRef(s)));
            py::set(vm, this, "atl_state", v)?;
        }
        Action::Continue(None) => py::set(vm, this, "atl_state", Value::None)?,
        _ => py::set(vm, this, "done", Value::Bool(true))?,
    }
    Ok(r.pause.map_or(Value::None, Value::Float))
}

fn base_predict_one(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn base_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let block = compiled_block(vm, this)?;
    let c = native_class(vm, "Block")?;
    let bv = vm.types.new_native(&c, Box::new(BlockRef(block)));
    let ch = py::get_or_none(vm, this, "children")?;
    let mut out = py::items(vm, &ch).unwrap_or_default();
    let extra = block_visit(vm, &[bv], &[])?;
    out.extend(py::items(vm, &extra)?);
    Ok(vm.types.new_list(out))
}

fn base_after_upgrade(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

/// Creates an ATLTransform for a script ATL block (`show ... :`,
/// `transform name:`, `image name:` and screen `at transform:`).
pub fn new_atl_transform(
    vm: &mut Vm,
    raw: Rc<RawBlock>,
    sig: Option<Signature>,
    context: Value,
    child: Value,
) -> PyResult<Value> {
    let atl = raw_value(vm, raw)?;
    let c = py::import_attr(vm, "renpy.display.motion", "ATLTransform")?;
    let sigv = match sig {
        Some(s) => signature_value(vm, s)?,
        None => Value::None,
    };
    let ctx = if py::is_none(&context) {
        vm.types.new_dict()
    } else {
        context
    };
    vm.call(
        &c,
        Vec::new(),
        vec![
            ("atl".into(), atl),
            ("child".into(), child),
            ("context".into(), ctx),
            ("parameters".into(), sigv),
        ],
    )
}

// ATLTransform (displayable) ----------------------------------------------------------------------

fn at_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let atl = arg_or(args, kw, 1, "atl", Value::None);
    let child = arg_or(args, kw, 2, "child", Value::None);
    let context = arg_or(args, kw, 3, "context", Value::None);
    let parameters = arg_or(args, kw, 4, "parameters", Value::None);
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| !matches!(&**k, "atl" | "child" | "context" | "parameters"))
        .cloned()
        .collect();
    base_init(vm, &[this.clone(), atl, context, parameters], &[])?;
    let mut tk = props;
    tk.push(("child".into(), child));
    super::transform::t_init(vm, &[this.clone()], &tk)?;
    let c = py::get_or_none(vm, this, "child")?;
    py::set(vm, this, "raw_child", c)?;
    Ok(Value::None)
}

fn at_update_state(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "hide_response", Value::Bool(true))?;
    py::set(vm, this, "replaced_response", Value::Bool(true))?;
    let s = {
        let v = py::get_or_none(vm, this, "st")?;
        py::f64_of(vm, &v).unwrap_or(0.)
    };
    let a = {
        let v = py::get_or_none(vm, this, "at")?;
        py::f64_of(vm, &v).unwrap_or(0.)
    };
    let state = py::get(vm, this, "state")?;
    let fps = py::get_or_none(vm, &state, "fps")?;
    let (fs, fa) = if ops::truthy(vm, &fps)? {
        let m = 1. / py::f64_of(vm, &fps)?;
        let (x, y) = (s + m / 2., a + m / 2.);
        (x - x.rem_euclid(m), y - y.rem_euclid(m))
    } else {
        (s, a)
    };
    let fr = vm.call_method(
        this,
        "execute",
        vec![this.clone(), Value::Float(fs), Value::Float(fa)],
    )?;
    if !py::is_none(&fr) {
        let t = py::f64_of(vm, &fr)?;
        super::render::redraw(vm, this, t);
    }
    py::set(vm, this, "active", Value::Bool(true))?;
    super::transform::update_events(vm, this)
}

fn at_repr_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get_or_none(vm, &args[0], "child")?;
    let atl = py::get_or_none(vm, &args[0], "atl")?;
    let loc = raw_of(&atl)
        .map(|r| format!("('{}', {})", r.loc.0, r.loc.1))
        .unwrap_or_default();
    let cr = ops::repr(vm, &c)?;
    Ok(vm.types.new_string(format!("({cr}, {loc})")))
}

fn at_getstate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "__dict__")?;
    let rv = vm.call_method(&d, "copy", Vec::new())?;
    for n in [
        "block",
        "atl_state",
        "predict_block",
        "parent_transform",
        "properties",
    ] {
        let k = vm.types.new_str(n);
        vm.call_method(&rv, "pop", vec![k, Value::None])?;
    }
    rv.obj().map(|_| ()).ok_or_else(|| vm.type_error("state"))?;
    py::dict_set(vm, &rv, "__version__", Value::Int(5))?;
    Ok(rv)
}

// position / DualAngle -----------------------------------------------------------------------------

fn pos_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let cls = args[0].clone();
    let a = arg_or(&args[1..], kw, 0, "absolute", Value::Int(0));
    let r = arg_or(&args[1..], kw, 1, "relative", Value::None);
    if py::is_none(&r) {
        let f = py::get(vm, &cls, "from_any")?;
        return vm.call(&f, vec![a], Vec::new());
    }
    let o = vm.call_method(&Value::Obj(vm.types.object.clone()), "__new__", vec![cls])?;
    py::set(vm, &o, "absolute", a)?;
    py::set(vm, &o, "relative", r)?;
    Ok(o)
}

fn pos_from_any(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = args[0].clone();
    let other = args.get(1).cloned().unwrap_or(Value::None);
    if other.obj().is_some_and(|o| o.class_name() == "position") {
        return Ok(other);
    }
    if matches!(other, Value::Float(_)) {
        return vm.call(&cls, vec![Value::Int(0), other], Vec::new());
    }
    vm.call(&cls, vec![other, Value::Int(0)], Vec::new())
}

fn pos_parts_of(vm: &mut Vm, v: &Value) -> PyResult<(f64, f64)> {
    super::transform::pos_parts(vm, v)
}

fn mk_pos(vm: &mut Vm, a: f64, r: f64) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "position")?;
    vm.call(&c, vec![Value::Float(a), Value::Float(r)], Vec::new())
}

fn pos_simplify(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = pos_parts_of(vm, &args[0])?;
    if r == 0. {
        if a == a.trunc() {
            return Ok(Value::Int(a as i64));
        }
        let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
        return vm.call(&abs, vec![Value::Float(a)], Vec::new());
    }
    if a == 0. {
        return Ok(Value::Float(r));
    }
    Ok(args[0].clone())
}

fn pos_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[1];
    if !o.obj().is_some_and(|x| x.class_name() == "position") {
        return Ok(vm.types.not_implemented.clone());
    }
    let (a, r) = pos_parts_of(vm, &args[0])?;
    let (b, s) = pos_parts_of(vm, o)?;
    mk_pos(vm, a + b, r + s)
}

fn pos_sub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[1];
    let (a, r) = pos_parts_of(vm, &args[0])?;
    if o.obj().is_some_and(|x| x.class_name() == "position") {
        let (b, s) = pos_parts_of(vm, o)?;
        return mk_pos(vm, a - b, r - s);
    }
    let (b, s) = pos_parts_of(vm, o)?;
    mk_pos(vm, a - b, r - s)
}

fn pos_rsub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = pos_parts_of(vm, &args[0])?;
    let (b, s) = pos_parts_of(vm, &args[1])?;
    mk_pos(vm, b - a, s - r)
}

fn pos_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[1];
    if !matches!(o, Value::Int(_) | Value::Float(_)) {
        return Ok(vm.types.not_implemented.clone());
    }
    let k = py::f64_of(vm, o)?;
    let (a, r) = pos_parts_of(vm, &args[0])?;
    mk_pos(vm, a * k, r * k)
}

fn pos_div(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[1];
    if !matches!(o, Value::Int(_) | Value::Float(_)) {
        return Ok(vm.types.not_implemented.clone());
    }
    let k = py::f64_of(vm, o)?;
    let (a, r) = pos_parts_of(vm, &args[0])?;
    mk_pos(vm, a / k, r / k)
}

fn pos_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = pos_parts_of(vm, &args[0])?;
    let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
    let av = vm.call(&abs, vec![Value::Float(a)], Vec::new())?;
    let c = py::import_attr(vm, MODULE, "position")?;
    vm.call(&c, vec![av, Value::Float(r)], Vec::new())
}

fn pos_neg(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = pos_parts_of(vm, &args[0])?;
    mk_pos(vm, -a, -r)
}

fn pos_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = py::get(vm, &args[0], "absolute")?;
    let r = py::get(vm, &args[0], "relative")?;
    let a = ops::str(vm, &a)?;
    let r = ops::str(vm, &r)?;
    Ok(vm
        .types
        .new_string(format!("position(absolute={a}, relative={r})")))
}

fn pos_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = &args[1];
    if !o.obj().is_some_and(|x| x.class_name() == "position") {
        return Ok(Value::Bool(false));
    }
    let a = pos_parts_of(vm, &args[0])?;
    let b = pos_parts_of(vm, o)?;
    Ok(Value::Bool(a == b))
}

fn pos_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = pos_parts_of(vm, &args[0])?;
    Ok(Value::Int(
        (a.to_bits() ^ r.to_bits().rotate_left(17)) as i64,
    ))
}

fn da_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "absolute",
        args.get(1).cloned().unwrap_or(Value::Int(0)),
    )?;
    py::set(
        vm,
        &args[0],
        "relative",
        args.get(2).cloned().unwrap_or(Value::Int(0)),
    )?;
    Ok(Value::None)
}

fn da_parts(vm: &mut Vm, v: &Value) -> PyResult<(f64, f64)> {
    let a = py::get(vm, v, "absolute")?;
    let r = py::get(vm, v, "relative")?;
    Ok((py::f64_of(vm, &a)?, py::f64_of(vm, &r)?))
}

fn mk_da(vm: &mut Vm, a: f64, r: f64) -> PyResult<Value> {
    let c = py::import_attr(vm, MODULE, "DualAngle")?;
    vm.call(&c, vec![Value::Float(a), Value::Float(r)], Vec::new())
}

fn da_from_any(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = args.get(1).cloned().unwrap_or(Value::None);
    if o.obj().is_some_and(|x| x.class_name() == "DualAngle") {
        return Ok(o);
    }
    if let Value::Float(f) = o {
        return mk_da(vm, f, f);
    }
    Err(vm.type_error("Cannot convert to DualAngle"))
}

fn da_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !args[1].obj().is_some_and(|x| x.class_name() == "DualAngle") {
        return Ok(vm.types.not_implemented.clone());
    }
    let (a, r) = da_parts(vm, &args[0])?;
    let (b, s) = da_parts(vm, &args[1])?;
    mk_da(vm, a + b, r + s)
}

fn da_sub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = da_parts(vm, &args[0])?;
    let (b, s) = da_parts(vm, &args[1])?;
    mk_da(vm, a - b, r - s)
}

fn da_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !matches!(args[1], Value::Int(_) | Value::Float(_)) {
        return Ok(vm.types.not_implemented.clone());
    }
    let k = py::f64_of(vm, &args[1])?;
    let (a, r) = da_parts(vm, &args[0])?;
    mk_da(vm, a * k, r * k)
}

fn da_neg(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (a, r) = da_parts(vm, &args[0])?;
    mk_da(vm, -a, -r)
}

// Type converters and module functions --------------------------------------------------------------

fn f_position_or_none(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&x) {
        return Ok(x);
    }
    let c = py::import_attr(vm, MODULE, "position")?;
    let f = py::get(vm, &c, "from_any")?;
    vm.call(&f, vec![x], Vec::new())
}

fn f_any_object(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args.first().cloned().unwrap_or(Value::None))
}

fn f_bool_or_none(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&x) {
        Ok(x)
    } else {
        Ok(Value::Bool(ops::truthy(vm, &x)?))
    }
}

fn f_float_or_none(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&x) {
        Ok(x)
    } else {
        Ok(Value::Float(py::f64_of(vm, &x)?))
    }
}

fn f_matrix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = args.first().cloned().unwrap_or(Value::None);
    if py::is_none(&x) || crysirror_python::builtins::is_callable(vm, &x) {
        return Ok(x);
    }
    let m = py::import_attr(vm, "renpy.display.matrix", "Matrix")?;
    vm.call(&m, vec![x], Vec::new())
}

fn f_mesh(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let x = args.first().cloned().unwrap_or(Value::None);
    if matches!(x.payload(), Some(crysirror_python::Payload::Tuple(_))) {
        return Ok(x);
    }
    Ok(Value::Bool(ops::truthy(vm, &x)?))
}

fn f_interpolate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::f64_of(vm, &args.first().cloned().unwrap_or(Value::Int(0)))?;
    let a = args.get(1).cloned().unwrap_or(Value::None);
    let b = args.get(2).cloned().unwrap_or(Value::None);
    let ty = args.get(3).cloned().unwrap_or(Value::None);
    interpolate(vm, t, &a, &b, &ty)
}

fn f_interpolate_spline(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = py::f64_of(vm, &args.first().cloned().unwrap_or(Value::Int(0)))?;
    let s = py::items(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let ty = args.get(2).cloned().unwrap_or(Value::None);
    interpolate_spline(vm, t, &s, &ty)
}

fn f_atl_warper(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = args.first().cloned().unwrap_or(Value::None);
    let name = py::get(vm, &f, "__name__")?;
    let w = py::import_attr(vm, MODULE, "warpers")?;
    ops::setitem(vm, &w, name.clone(), f.clone())?;
    if let Some(n) = name.as_str() {
        if let Some(st) = crate::engine::try_state(vm) {
            st.warpers.borrow_mut().insert(n.to_owned());
        }
    }
    Ok(f)
}

fn ctx_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "context",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    Ok(Value::None)
}

fn ctx_eval(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = py::get(vm, &args[0], "context")?;
    let e = py::import_attr(vm, "renpy.python", "py_eval")?;
    vm.call(
        &e,
        vec![args.get(1).cloned().unwrap_or(Value::None)],
        vec![("locals".into(), d)],
    )
}

fn ctx_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = args.get(1).cloned().unwrap_or(Value::None);
    if !o.obj().is_some_and(|x| x.class_name() == "Context") {
        return Ok(Value::Bool(false));
    }
    let a = py::get(vm, &args[0], "context")?;
    let b = py::get(vm, &o, "context")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn ctx_ne(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = ctx_eq(vm, args, kw)?;
    Ok(Value::Bool(!ops::truthy(vm, &r)?))
}

fn no_new(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.type_error("cannot create this object directly"))
}

fn native_type(vm: &mut Vm, name: &str, methods: py::Methods) -> ObjRef {
    let c = vm.types.new_builtin_type(
        name,
        &vm.types.object.clone(),
        crysirror_python::object::BaseKind::Native,
        true,
    );
    let n = vm.types.new_builtin("__new__", no_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&c, "__new__", n);
    for (n, f) in methods {
        vm.types.add_method(&c, n, *f);
    }
    let mn = vm.types.new_str(MODULE);
    vm.types.set_attr(&c, "__module__", mn);
    c
}

/// `Signature(parameters=None)`: from an iterable of `renpy.parameter`
/// `Parameter`s (valued defaults are kept as their `repr`).
fn sig_new(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let params = arg_or(&args[1..], kw, 0, "parameters", Value::None);
    let mut sig = Signature::default();
    if !py::is_none(&params) {
        let valued = py::import_attr(vm, "renpy.parameter", "ValuedParameter")?;
        for p in py::items(vm, &params)? {
            let name = py::get(vm, &p, "name")?;
            let name = py::string(vm, &name)?;
            let kind = py::get(vm, &p, "kind")?;
            let kind = py::int(vm, &kind)? as u8;
            let has = py::get(vm, &p, "has_default")?;
            let default = if py::truthy(vm, &has)? {
                let d = py::get(vm, &p, "default")?;
                let is_valued = valued.obj().is_some_and(|c| ops::isinstance(vm, &p, c));
                Some(if is_valued { ops::repr(vm, &d)? } else { py::string(vm, &d)? })
            } else {
                None
            };
            sig.parameters.push(crate::ast::Parameter { name, kind, default });
        }
    }
    signature_value(vm, sig)
}

fn sig_parameters(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    let cls = py::import_attr(vm, "renpy.parameter", "Parameter")?;
    let od = py::import_attr(vm, "collections", "OrderedDict")?;
    let rv = vm.call(&od, Vec::new(), Vec::new())?;
    for p in &sig.parameters {
        let default = p.default.as_deref().map(|d| py::s(vm, d)).unwrap_or(Value::None);
        let name = py::s(vm, &p.name);
        let param = vm.call(&cls, vec![name.clone(), Value::Int(p.kind as i64), default], Vec::new())?;
        ops::setitem(vm, &rv, name, param)?;
    }
    Ok(rv)
}

fn sig_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = py::import_attr(vm, MODULE, "Signature")?;
    let params = sig_parameters(vm, args, &[])?;
    let values = py::method(vm, &params, "values", Vec::new())?;
    let values = py::items(vm, &values)?;
    let l = vm.types.new_list(values);
    let a = vm.types.new_tuple(vec![l]);
    Ok(vm.types.new_tuple(vec![cls, a]))
}

/// `Signature.apply(args, kwargs, ignore_errors=False, partial=False,
/// apply_defaults=True)`: the bound mapping.
fn sig_apply(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    let a = py::items(vm, &arg_or(&args[1..], kw, 0, "args", vm.types.new_tuple(Vec::new())))?;
    let kd = arg_or(&args[1..], kw, 1, "kwargs", Value::None);
    let k = if py::is_none(&kd) { Vec::new() } else { py::str_items(vm, &kd)? };
    let ignore = py::truthy(vm, &arg_or(&args[1..], kw, 2, "ignore_errors", Value::Bool(false)))?;
    let loc: super::params::Loc = ("<signature>".into(), 1);
    super::params::apply(vm, &sig, &loc, &a, &k, ignore)
}

fn sig_apply_defaults(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    let loc: super::params::Loc = ("<signature>".into(), 1);
    super::params::apply_defaults(vm, &sig, &loc, &args[1])?;
    Ok(Value::None)
}

/// Upstream's `Signature.__str__`: `/` after positional-only parameters,
/// `*` before the first keyword-only one when there is no `*args`.
pub fn signature_text(sig: &Signature) -> String {
    let mut out: Vec<String> = Vec::new();
    let (mut pos_only, mut kw_sep) = (false, true);
    for p in &sig.parameters {
        let f = match (p.kind, &p.default) {
            (2, _) => format!("*{}", p.name),
            (4, _) => format!("**{}", p.name),
            (_, Some(d)) => format!("{}={}", p.name, d),
            _ => p.name.clone(),
        };
        if p.kind == 0 {
            pos_only = true;
        } else if pos_only {
            out.push("/".into());
            pos_only = false;
        }
        if p.kind == 2 {
            kw_sep = false;
        } else if p.kind == 3 && kw_sep {
            out.push("*".into());
            kw_sep = false;
        }
        out.push(f);
    }
    if pos_only {
        out.push("/".into());
    }
    format!("({})", out.join(", "))
}

fn sig_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    Ok(py::s(vm, &signature_text(&sig)))
}

fn sig_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    Ok(py::s(vm, &format!("<Signature {}>", signature_text(&sig))))
}

fn sig_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (Some(a), Some(b)) = (signature_of(&args[0]), signature_of(&args[1])) else {
        return Ok(Value::Bool(false));
    };
    let same = a.parameters.len() == b.parameters.len()
        && a.parameters.iter().zip(&b.parameters).all(|(x, y)| x.name == y.name && x.kind == y.kind && x.default == y.default);
    let _ = vm;
    Ok(Value::Bool(same))
}

fn sig_pos_only_as_pos_or_kw(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let sig = signature_of(&args[0]).ok_or_else(|| vm.type_error("Signature"))?;
    let mut s = (*sig).clone();
    for p in &mut s.parameters {
        if p.kind == 0 {
            p.kind = 1;
        }
    }
    signature_value(vm, s)
}

/// PROPERTIES: name -> type converter, after 8.3 transform.py/atl.py.
fn build_properties(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    let get = |vm: &mut Vm, n: &str| -> Value {
        crysirror_python::ops::get_raw_attr(m, n).unwrap_or(Value::None)
    };
    let pon = get(vm, "position_or_none");
    let any = get(vm, "any_object");
    let bon = get(vm, "bool_or_none");
    let fon = get(vm, "float_or_none");
    let mat = get(vm, "matrix");
    let mesh = get(vm, "mesh");
    let fl = Value::Obj(vm.types.float.clone());
    let int = Value::Obj(vm.types.int.clone());
    let bo = Value::Obj(vm.types.bool_.clone());
    let st = Value::Obj(vm.types.str_.clone());
    let abs = py::import_attr(vm, "renpy.display.core", "absolute")?;
    let t = |vm: &Vm, v: Vec<Value>| vm.types.new_tuple(v);
    let p2 = t(vm, vec![pon.clone(), pon.clone()]);
    let p4 = t(vm, vec![pon.clone(), pon.clone(), pon.clone(), pon.clone()]);
    let i2 = t(vm, vec![int.clone(), int.clone()]);
    let f2 = t(vm, vec![fl.clone(), fl.clone()]);
    let f3 = t(vm, vec![fl.clone(), fl.clone(), fl.clone()]);
    let a2 = t(vm, vec![abs.clone(), abs.clone()]);
    let da = py::import_attr(vm, MODULE, "DualAngle")
        .and_then(|c| py::get(vm, &c, "from_any"))
        .unwrap_or(any.clone());
    let table: Vec<(&str, Value)> = vec![
        ("additive", fl.clone()),
        ("alpha", fl.clone()),
        ("blend", any.clone()),
        ("blur", fon.clone()),
        ("corner1", p2.clone()),
        ("corner2", p2.clone()),
        ("crop", p4),
        ("crop_relative", bon.clone()),
        ("debug", any.clone()),
        ("delay", fl.clone()),
        ("events", bo.clone()),
        ("fit", st),
        ("fps", fon.clone()),
        ("matrixanchor", p2.clone()),
        ("matrixcolor", mat.clone()),
        ("matrixtransform", mat),
        ("maxsize", i2.clone()),
        ("mesh", mesh),
        ("mesh_pad", any.clone()),
        ("nearest", bon),
        ("perspective", any.clone()),
        ("rotate", fl.clone()),
        ("rotate_pad", bo.clone()),
        ("point_to", any.clone()),
        ("orientation", f3),
        ("xrotate", fl.clone()),
        ("yrotate", fl.clone()),
        ("zrotate", fl.clone()),
        ("shader", any.clone()),
        ("show_cancels_hide", bo.clone()),
        ("subpixel", bo.clone()),
        ("transform_anchor", bo.clone()),
        ("zoom", fl.clone()),
        ("xanchoraround", pon.clone()),
        ("xanchor", pon.clone()),
        ("xaround", pon.clone()),
        ("xoffset", abs.clone()),
        ("xpan", fon.clone()),
        ("xpos", pon.clone()),
        ("xsize", pon.clone()),
        ("xtile", int.clone()),
        ("xzoom", fl.clone()),
        ("yanchoraround", pon.clone()),
        ("yanchor", pon.clone()),
        ("yaround", pon.clone()),
        ("yoffset", abs.clone()),
        ("ypan", fon),
        ("ypos", pon.clone()),
        ("ysize", pon.clone()),
        ("ytile", int),
        ("yzoom", fl.clone()),
        ("zpos", fl.clone()),
        ("zzoom", bo),
        ("alignaround", f2.clone()),
        ("align", p2.clone()),
        ("anchor", p2.clone()),
        ("anchorangle", da),
        ("anchoraround", p2.clone()),
        ("anchorradius", pon.clone()),
        ("angle", fl.clone()),
        ("around", p2.clone()),
        ("offset", a2),
        ("pos", p2.clone()),
        ("radius", pon.clone()),
        ("size", i2),
        ("xalign", pon.clone()),
        ("xcenter", pon.clone()),
        ("xycenter", p2.clone()),
        ("xysize", p2),
        ("yalign", pon.clone()),
        ("ycenter", pon),
    ];
    let d = vm.types.new_dict();
    for (k, v) in table {
        py::dict_set(vm, &d, k, v)?;
    }
    for g in [
        "gl_anisotropic",
        "gl_blend_func",
        "gl_color_mask",
        "gl_depth",
        "gl_drawable_resolution",
        "gl_mipmap",
        "gl_pixel_perfect",
        "gl_texture_scaling",
        "gl_texture_wrap",
    ] {
        py::dict_set(vm, &d, g, any.clone())?;
    }
    crysirror_python::modules::set(vm, m, "PROPERTIES", d);
    Ok(())
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("position_or_none", f_position_or_none),
            ("any_object", f_any_object),
            ("bool_or_none", f_bool_or_none),
            ("float_or_none", f_float_or_none),
            ("matrix", f_matrix),
            ("mesh", f_mesh),
            ("interpolate", f_interpolate),
            ("interpolate_spline", f_interpolate_spline),
            ("atl_warper", f_atl_warper),
            ("compile_all", |_, _, _| Ok(Value::None)),
            ("_raw_block_at", f_raw_block_at),
            ("compiling", |_, _, _| Ok(Value::None)),
            ("executing", |_, _, _| Ok(Value::None)),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let warpers = vm.types.new_dict();
    let pause = vm.types.new_builtin("pause", |vm, a, _| {
        let t = py::f64_of(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
        Ok(Value::Float(if t < 1. { 0. } else { 1. }))
    });
    let instant = vm
        .types
        .new_builtin("instant", |_, _, _| Ok(Value::Float(1.)));
    py::dict_set(vm, &warpers, "pause", pause.clone())?;
    py::dict_set(vm, &warpers, "instant", instant.clone())?;
    set(vm, "warpers", warpers);
    set(vm, "pause", pause);
    set(vm, "instant", instant);
    set(vm, "compile_queue", vm.types.new_list(Vec::new()));
    set(vm, "GLOBAL_CONST", Value::Int(2));
    // position
    let pos = py::class(
        vm,
        MODULE,
        "position",
        &[],
        &[
            ("simplify", pos_simplify),
            ("__add__", pos_add),
            ("__radd__", pos_add),
            ("__sub__", pos_sub),
            ("__rsub__", pos_rsub),
            ("__mul__", pos_mul),
            ("__rmul__", pos_mul),
            ("__truediv__", pos_div),
            ("__div__", pos_div),
            ("__pos__", pos_pos),
            ("__neg__", pos_neg),
            ("__repr__", pos_repr),
            ("__eq__", pos_eq),
            ("__hash__", pos_hash),
        ],
        &[],
    )?;
    let n = vm.types.new_builtin("__new__", pos_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&pos, "__new__", n);
    let fa = vm.types.new_builtin("from_any", pos_from_any);
    let fa = py::classmethod(vm, fa);
    vm.types.set_attr(&pos, "from_any", fa);
    set(vm, "position", Value::Obj(pos));
    let da = py::class(
        vm,
        MODULE,
        "DualAngle",
        &[],
        &[
            ("__init__", da_init),
            ("__add__", da_add),
            ("__sub__", da_sub),
            ("__mul__", da_mul),
            ("__rmul__", da_mul),
            ("__neg__", da_neg),
        ],
        &[],
    )?;
    let fa = vm.types.new_builtin("from_any", da_from_any);
    let fa = py::classmethod(vm, fa);
    vm.types.set_attr(&da, "from_any", fa);
    set(vm, "DualAngle", Value::Obj(da));
    let ctx = py::class(
        vm,
        MODULE,
        "Context",
        &[],
        &[
            ("__init__", ctx_init),
            ("eval", ctx_eval),
            ("__eq__", ctx_eq),
            ("__ne__", ctx_ne),
        ],
        &[],
    )?;
    set(vm, "Context", Value::Obj(ctx));
    let nic = py::import_attr(vm, "renpy.object", "Sentinel").and_then(|s| {
        let n = vm.types.new_str("NotInContext");
        vm.call(&s, vec![n], Vec::new())
    })?;
    set(vm, "NotInContext", nic);
    let raw = native_type(
        vm,
        "RawBlock",
        &[
            ("__getattr__", raw_getattr),
            ("compile", raw_compile),
            ("predict", raw_predict),
            ("__reduce__", raw_reduce),
        ],
    );
    set(vm, "RawBlock", Value::Obj(raw));
    let blk = native_type(
        vm,
        "Block",
        &[
            ("visit", block_visit),
            ("_handles_event", block_handles_event),
        ],
    );
    set(vm, "Block", Value::Obj(blk));
    let stc = native_type(vm, "State", &[]);
    set(vm, "State", Value::Obj(stc));
    let sig = native_type(
        vm,
        "Signature",
        &[
            ("__reduce__", sig_reduce),
            ("apply", sig_apply),
            ("apply_defaults", sig_apply_defaults),
            ("__str__", sig_str),
            ("__repr__", sig_repr),
            ("__eq__", sig_eq),
            ("with_pos_only_as_pos_or_kw", sig_pos_only_as_pos_or_kw),
        ],
    );
    let n = vm.types.new_builtin("__new__", sig_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&sig, "__new__", n);
    let pg = vm.types.new_builtin("parameters", sig_parameters);
    let pp = py::property(vm, pg, None)?;
    vm.types.set_attr(&sig, "parameters", pp);
    set(vm, "Signature", Value::Obj(sig));
    super::atlclasses::add_to_module(vm, &m)?;
    let object = py::import_attr(vm, "renpy.object", "Object")?;
    let nosave = vm
        .types
        .new_list(vec![vm.types.new_str("parent_transform")]);
    let base = py::class(
        vm,
        MODULE,
        "ATLTransformBase",
        &[object],
        &[
            ("__init__", base_init),
            ("after_upgrade", base_after_upgrade),
            ("_transition", base_transition),
            ("_handles_event", base_handles_event),
            ("get_block", base_get_block),
            ("take_execution_state", base_take_execution_state),
            ("__call__", base_call),
            ("compile", base_compile),
            ("execute", base_execute),
            ("predict_one", base_predict_one),
            ("visit", base_visit),
        ],
        &[
            ("__version__", Value::Int(1)),
            ("parameters", Value::None),
            ("parent_transform", Value::None),
            ("atl_st_offset", Value::Int(0)),
            ("predict_block", Value::None),
            ("nosave", nosave),
        ],
    )?;
    let tr = vm.types.new_builtin("transition", base_transition);
    let tr = py::property(vm, tr, None)?;
    vm.types.set_attr(&base, "transition", tr);
    set(vm, "ATLTransformBase", Value::Obj(base));
    build_properties(vm, &m)?;
    Ok(m)
}

/// `ATLTransform(ATLTransformBase, Transform)` lives in the transform
/// module (and `renpy.display.motion`).
pub fn create_atl_transform(vm: &mut Vm, tm: &ObjRef, t: Value) -> PyResult<()> {
    let base = py::import_attr(vm, MODULE, "ATLTransformBase")?;
    let c = py::class(
        vm,
        "renpy.display.motion",
        "ATLTransform",
        &[base, t],
        &[
            ("__init__", at_init),
            ("update_state", at_update_state),
            ("_repr_info", at_repr_info),
            ("__getstate__", at_getstate),
        ],
        &[],
    )?;
    if !super::state::rt(vm).profile.at_least(6, 11) {
        // Through 6.10, `show()` runs the ATL once at time 0.
        let f = vm.types.new_builtin("show", |vm, a, _| {
            vm.call_method(&a[0], "execute", vec![a[0].clone(), Value::Int(0), Value::Int(0)])?;
            Ok(Value::None)
        });
        vm.types.set_attr(&c, "show", f);
    }
    crysirror_python::modules::set(vm, tm, "ATLTransform", Value::Obj(c));
    Ok(())
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
