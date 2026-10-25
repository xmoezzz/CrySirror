// SPDX-License-Identifier: MPL-2.0
//! Default script host: JavaScript executed by Boa, TypeScript lowered by Oxc
//! (ADR 0005). Scripts reach the engine only through the reflective object
//! API; generated JS classes mirror every registered engine class.
//!
//! Lifecycle: a script may define global functions `onInit`, `onStart`,
//! `onFixedUpdate(stepMs)`, `onUpdate(dtMs)`, `onLateUpdate(dtMs)`, `onPause`,
//! `onResume` and `onShutdown`. Missing hooks are skipped. An uncaught script
//! exception is returned as an error; the App then enters `Faulted`.
//!
//! Not provided: modules/imports, a deterministic clock or RNG, persistence of
//! the JS heap (scripts persist state through `fs.writeSave`), or type checking.
#![forbid(unsafe_code)]
use anyhow::{anyhow, ensure, Context as _, Result};
use boa_engine::{
    js_string, native_function::NativeFunction, Context, JsError, JsNativeError, JsResult, JsValue,
    Source,
};
use crysirror_engine::{builtin::GLOBALS, EngineContext, Lifecycle, ObjectId, ScriptHost, Value};
use std::{cell::RefCell, fmt::Write, path::Path};

thread_local! {
    static ENGINE: RefCell<EngineContext> = RefCell::new(EngineContext::vacant());
}

/// Move the engine state into the thread-local slot for the duration of `f`
/// and always move it back, even if `f` panics.
fn with_engine<T>(cx: &mut EngineContext, f: impl FnOnce() -> T) -> T {
    struct Guard<'a>(&'a mut EngineContext);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            ENGINE.with(|slot| std::mem::swap(self.0, &mut slot.borrow_mut()));
        }
    }
    ENGINE.with(|slot| std::mem::swap(cx, &mut slot.borrow_mut()));
    let guard = Guard(cx);
    let out = f();
    drop(guard);
    out
}

fn js_err(e: anyhow::Error) -> JsError {
    JsNativeError::error().with_message(format!("{e:#}")).into()
}
fn arg_string(args: &[JsValue], i: usize, cx: &mut Context) -> JsResult<String> {
    Ok(args
        .get(i)
        .cloned()
        .unwrap_or_default()
        .to_string(cx)?
        .to_std_string_escaped())
}
fn arg_id(args: &[JsValue], cx: &mut Context) -> JsResult<ObjectId> {
    let n = args.first().cloned().unwrap_or_default().to_number(cx)?;
    if !(n.is_finite() && n >= 1. && n.fract() == 0.) {
        return Err(JsNativeError::typ()
            .with_message("invalid engine object id")
            .into());
    }
    Ok(ObjectId(n as u64))
}
fn engine_op(op: impl FnOnce(&mut EngineContext) -> Result<Value>) -> JsResult<JsValue> {
    let value = ENGINE.with(|slot| {
        let mut cx = slot
            .try_borrow_mut()
            .map_err(|_| anyhow!("re-entrant engine call"))?;
        op(&mut cx)
    });
    let json =
        serde_json::to_string(&value.map_err(js_err)?.to_json()).map_err(|e| js_err(e.into()))?;
    Ok(JsValue::from(js_string!(json)))
}
fn parse(json: &str) -> Result<Value> {
    ensure!(
        json.len() <= 16 * 1024 * 1024,
        "script value exceeds 16 MiB"
    );
    Ok(Value::from_json(serde_json::from_str(json)?))
}
fn native_call(_: &JsValue, args: &[JsValue], cx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(args, cx)?;
    let method = arg_string(args, 1, cx)?;
    let raw = arg_string(args, 2, cx)?;
    engine_op(|e| {
        let Value::List(list) = parse(&raw)? else {
            anyhow::bail!("arguments must be an array")
        };
        e.call(id, &method, &list)
    })
}
fn native_get(_: &JsValue, args: &[JsValue], cx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(args, cx)?;
    let prop = arg_string(args, 1, cx)?;
    engine_op(|e| e.get(id, &prop))
}
fn native_set(_: &JsValue, args: &[JsValue], cx: &mut Context) -> JsResult<JsValue> {
    let id = arg_id(args, cx)?;
    let prop = arg_string(args, 1, cx)?;
    let raw = arg_string(args, 2, cx)?;
    engine_op(|e| {
        e.set(id, &prop, parse(&raw)?)?;
        Ok(Value::Null)
    })
}

const RUNTIME: &str = r#""use strict";
var __cs = (function () {
  const classes = {};
  class Base {
    constructor(id) { Object.defineProperty(this, "__id", { value: id }); }
    toJSON() { return { "$object": this.__id }; }
  }
  function revive(_, v) {
    if (v !== null && typeof v === "object" && typeof v.$object === "number") {
      const C = classes[v.$class] || Base;
      return new C(v.$object);
    }
    return v;
  }
  return {
    Base, classes,
    call(id, m, args) { return JSON.parse(__cs_call(id, m, JSON.stringify(args)), revive); },
    get(id, p) { return JSON.parse(__cs_get(id, p), revive); },
    set(id, p, v) { __cs_set(id, p, JSON.stringify(v === undefined ? null : v)); },
  };
})();
"#;

/// Generate JS classes and globals from the engine class registry.
fn bindings(cx: &mut EngineContext) -> Result<String> {
    let mut js = String::from(RUNTIME);
    let mut singletons = Vec::new();
    for (global, class) in GLOBALS {
        let class: &'static str = class;
        singletons.push((*global, class, cx.singleton(class)?));
    }
    let s = cx.state()?;
    for info in s.classes.infos() {
        let n = info.name;
        if let Some(p) = info.properties.iter().find(|p| info.methods.iter().any(|m| m.name == p.name)) {
            anyhow::bail!("{n}.{} is both a property and a method", p.name);
        }
        let _ = writeln!(
            js,
            "class {n} extends __cs.Base {{}}\n__cs.classes.{n} = {n};"
        );
        for p in &info.properties {
            // Read-only properties also get a setter: the engine rejects the
            // write, so sloppy-mode scripts cannot silently drop assignments.
            let setter = format!(", set(v) {{ __cs.set(this.__id, {:?}, v); }}", p.name);
            let _ = writeln!(
                js,
                "Object.defineProperty({n}.prototype, {name:?}, {{ get() {{ return __cs.get(this.__id, {name:?}); }}{setter} }});",
                name = p.name
            );
        }
        for m in &info.methods {
            let _ = writeln!(
                js,
                "{n}.prototype.{name} = function (...a) {{ return __cs.call(this.__id, {name:?}, a); }};",
                name = m.name
            );
        }
    }
    for (global, class, id) in singletons {
        let _ = writeln!(
            js,
            "Object.defineProperty(globalThis, {global:?}, {{ value: new {class}({}), enumerable: true }});",
            id.0
        );
    }
    Ok(js)
}

const HOOKS: [&str; 8] = [
    "onInit",
    "onStart",
    "onFixedUpdate",
    "onUpdate",
    "onLateUpdate",
    "onPause",
    "onResume",
    "onShutdown",
];

pub struct JsHost {
    name: String,
    context: Option<Context>,
    pending: Vec<(String, String)>,
    /// Loop iterations allowed per entry into JS; exceeding it is an error.
    pub loop_limit: u64,
}
impl JsHost {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            context: None,
            pending: Vec::new(),
            loop_limit: 10_000_000,
        }
    }
    /// Build a host from source text; `.ts` names are lowered by Oxc first.
    pub fn with_source(name: &str, unit: &str, source: &str) -> Result<Self> {
        let mut h = Self::new(name);
        h.add_source(unit, source)?;
        Ok(h)
    }
    fn report(e: JsError, ctx: &mut Context) -> anyhow::Error {
        match e.try_native(ctx) {
            Ok(native) => anyhow!("{native}"),
            Err(_) => anyhow!("{e}"),
        }
    }
    fn run_pending(&mut self, cx: &mut EngineContext) -> Result<()> {
        let ctx = self
            .context
            .as_mut()
            .context("script host not initialized")?;
        let pending = std::mem::take(&mut self.pending);
        with_engine(cx, || {
            for (unit, js) in pending {
                ctx.eval(Source::from_bytes(js.as_bytes()))
                    .map_err(|e| Self::report(e, ctx))
                    .with_context(|| format!("script '{unit}'"))?;
            }
            Ok(())
        })
    }
    fn hook(&mut self, cx: &mut EngineContext, hook: &str, arg: Option<u64>) -> Result<()> {
        debug_assert!(HOOKS.contains(&hook));
        let ctx = self
            .context
            .as_mut()
            .context("script host not initialized")?;
        let f = ctx
            .global_object()
            .get(js_string!(hook), ctx)
            .map_err(|e| Self::report(e, ctx))?;
        let Some(f) = f.as_callable().cloned() else {
            return Ok(());
        };
        let args: Vec<JsValue> = arg.map(|a| JsValue::from(a as f64)).into_iter().collect();
        with_engine(cx, || {
            f.call(&JsValue::undefined(), &args, ctx)
                .map(|_| ())
                .map_err(|e| Self::report(e, ctx))
        })
        .with_context(|| hook.to_string())
    }
}
impl Lifecycle for JsHost {
    fn name(&self) -> &str {
        &self.name
    }
    fn on_init(&mut self, cx: &mut EngineContext) -> Result<()> {
        let mut ctx = Context::default();
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(self.loop_limit);
        ctx.runtime_limits_mut().set_recursion_limit(512);
        for (name, len, f) in [
            (
                "__cs_call",
                3,
                native_call as boa_engine::native_function::NativeFunctionPointer,
            ),
            ("__cs_get", 2, native_get),
            ("__cs_set", 3, native_set),
        ] {
            ctx.register_global_callable(js_string!(name), len, NativeFunction::from_fn_ptr(f))
                .map_err(|e| anyhow!("{e}"))?;
        }
        let prelude = bindings(cx)?;
        self.pending
            .insert(0, ("<crysirror bindings>".into(), prelude));
        self.context = Some(ctx);
        self.run_pending(cx)?;
        self.hook(cx, "onInit", None)
    }
    fn on_start(&mut self, cx: &mut EngineContext) -> Result<()> {
        self.hook(cx, "onStart", None)
    }
    fn on_fixed_update(&mut self, cx: &mut EngineContext, step_ms: u64) -> Result<()> {
        self.hook(cx, "onFixedUpdate", Some(step_ms))
    }
    fn on_update(&mut self, cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
        self.hook(cx, "onUpdate", Some(dt_ms))
    }
    fn on_late_update(&mut self, cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
        self.hook(cx, "onLateUpdate", Some(dt_ms))
    }
    fn on_pause(&mut self, cx: &mut EngineContext) -> Result<()> {
        self.hook(cx, "onPause", None)
    }
    fn on_resume(&mut self, cx: &mut EngineContext) -> Result<()> {
        self.hook(cx, "onResume", None)
    }
    fn on_shutdown(&mut self, cx: &mut EngineContext) -> Result<()> {
        if self.context.is_none() {
            return Ok(());
        }
        self.hook(cx, "onShutdown", None)
    }
}
impl ScriptHost for JsHost {
    fn language(&self) -> &'static str {
        "javascript"
    }
    fn add_source(&mut self, name: &str, source: &str) -> Result<()> {
        ensure!(self.pending.len() < 256, "script unit limit");
        ensure!(self.context.is_none(), "add sources before init");
        let js = if name.ends_with(".ts") {
            crysirror_author::transpile(source, Path::new(name))?
        } else {
            ensure!(
                source.len() <= crysirror_author::MAX_SOURCE_BYTES,
                "script exceeds 1 MiB"
            );
            source.to_owned()
        };
        self.pending.push((name.into(), js));
        Ok(())
    }
}
