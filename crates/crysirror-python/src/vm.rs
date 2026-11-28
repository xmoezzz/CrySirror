// SPDX-License-Identifier: MPL-2.0
//! Interpreter loop with heap frames. Python-to-Python calls push frames
//! without Rust recursion, so a native call made from the top-level task can
//! suspend the whole frame stack (`PyErr::Suspend`) and the host can resume
//! it later with a value. Native code that calls back into Python runs a
//! nested loop that cannot suspend.

use crate::ast::{BinOp, CmpOp, UnaryOp};
use crate::code::*;
use crate::dict::Dict;
use crate::object::*;
use crate::types::Types;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub enum PyErr {
    Exc(ObjRef),
    /// Request from a native function to suspend the running task.
    Suspend(Box<dyn Any>),
}

impl std::fmt::Debug for PyErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PyErr::Exc(e) => write!(f, "PyErr::Exc({})", e.class_name()),
            PyErr::Suspend(_) => write!(f, "PyErr::Suspend"),
        }
    }
}

pub type PyResult<T> = Result<T, PyErr>;
pub type Kwargs = Vec<(Rc<str>, Value)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Finally,
    ExceptHandler,
}

#[derive(Clone)]
pub struct Block {
    pub kind: BlockKind,
    pub handler: u32,
    pub level: usize,
    pub prev_exc: Option<Value>,
}

#[derive(Clone)]
pub struct Frame {
    pub code: Rc<Code>,
    pub ip: usize,
    pub stack: Vec<Value>,
    pub blocks: Vec<Block>,
    pub fast: Vec<Option<Value>>,
    pub cells: Vec<ObjRef>,
    pub globals: ObjRef,
    /// Namespace for module/class/exec code (a dict or any mapping).
    pub locals: Option<Value>,
    pub builtins: ObjRef,
    /// Exception state saved by an enclosing generator resume.
    pub saved_exc: Option<Value>,
    pub func_name: Rc<str>,
}

pub enum Exit {
    Return(Value),
    Yield(Value, Box<Frame>),
}

pub enum Resume {
    Send(Value),
    Throw(ObjRef),
}

pub enum GenStep {
    Yielded(Value),
    Returned(Value),
}

/// A suspended task: its frames, detached from the VM until resumed.
#[derive(Clone)]
pub struct Task {
    frames: Vec<Box<Frame>>,
    exc_state: Option<Value>,
}

impl Task {
    /// Roots reachable from suspended script frames. Global namespaces are
    /// provided by the embedding runtime.
    pub fn retained_values(&self) -> impl Iterator<Item = Value> + '_ {
        self.frames.iter().flat_map(|frame| {
            frame.stack.iter().cloned()
                .chain(frame.fast.iter().flatten().cloned())
                .chain(frame.cells.iter().cloned().map(Value::Obj))
                .chain(frame.locals.iter().cloned())
                .chain(frame.saved_exc.iter().cloned())
                .chain(frame.blocks.iter().filter_map(|block| block.prev_exc.clone()))
        }).chain(self.exc_state.iter().cloned())
    }
}

pub enum TaskOutcome {
    Done(Value),
    Suspended(Task, Box<dyn Any>),
    Error(ObjRef),
}

/// Host hooks supplied by the embedding engine.
pub trait Host {
    /// Text written by `print` and `sys.stdout.write`.
    fn write_stdout(&mut self, _s: &str) {}
    /// Returns module source for `import` of a host-provided Python module.
    fn find_module_source(&mut self, _name: &str) -> Option<(String, String)> {
        None
    }
    /// Whether a host-provided module is a package (gets `__path__`).
    fn module_is_package(&mut self, _name: &str) -> bool {
        false
    }
    /// Dialect of a host-provided module; None uses the VM default.
    fn module_dialect(&mut self, _name: &str) -> Option<crate::Dialect> {
        None
    }
    /// Scoped file access for `open()` and `os`; paths are script-visible.
    fn read_file(&mut self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    fn write_file(&mut self, _path: &str, _data: &[u8]) -> bool {
        false
    }
    /// None: missing; Some(true): directory; Some(false): file.
    fn stat(&mut self, _path: &str) -> Option<bool> {
        None
    }
    fn list_dir(&mut self, _path: &str) -> Option<Vec<String>> {
        None
    }
    fn remove_file(&mut self, _path: &str) -> bool {
        false
    }
    fn rename(&mut self, _from: &str, _to: &str) -> bool {
        false
    }
    fn make_dir(&mut self, _path: &str) -> bool {
        false
    }
    /// A virtual temporary directory. Never exposes an ambient filesystem path.
    fn temporary_directory(&self) -> Option<&str> { None }
    /// Acquire an exclusive nonblocking lease for a scoped file.
    fn try_lock_file(&mut self, _path: &str) -> Result<Box<dyn Any>, String> {
        Err("scoped file locking is unavailable".into())
    }
    fn as_any(&mut self) -> &mut dyn Any;
}

struct NoHost;
impl Host for NoHost {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

pub type ModuleInit = fn(&mut Vm) -> PyResult<ObjRef>;

pub struct Vm {
    pub types: Types,
    pub builtins: ObjRef,
    pub builtins_module: ObjRef,
    /// sys.modules
    pub modules: ObjRef,
    pub frames: Vec<Box<Frame>>,
    pub exc_state: Option<Value>,
    pub host: Box<dyn Host>,
    pub native_modules: Vec<(&'static str, ModuleInit)>,
    /// Python 2 `print` softspace of the default stdout.
    pub stdout_softspace: bool,
    /// Called once a native or source module has been initialized, before
    /// the importer returns it (embedders add module-level behavior here).
    pub module_loaded: Option<fn(&mut Vm, &str, &ObjRef) -> PyResult<()>>,
    /// Frame depth at which a running task may suspend.
    task_base: Option<usize>,
    pub recursion_limit: usize,
    pub native_depth: usize,
    /// `CRYSIRROR_PY_TRACE_NATIVE`: exceptions record the native functions
    /// they pass through as `<native>` traceback entries (diagnostics only).
    pub trace_natives: bool,
    pub dialect_default: crate::Dialect,
    /// Arbitrary engine state reachable from native functions.
    pub ext: RefCell<Vec<Box<dyn Any>>>,
    pub classes: crate::builtins::Classes,
    /// `gc.garbage` and `gc.callbacks` (lists).
    pub gc_garbage: ObjRef,
    pub gc_callbacks: ObjRef,
}

macro_rules! top {
    ($s:expr) => {
        $s.frames.last_mut().unwrap()
    };
}

impl Vm {
    pub fn new() -> Vm {
        Self::with_host(Box::new(NoHost))
    }

    pub fn with_host(host: Box<dyn Host>) -> Vm {
        let types = Types::new();
        let builtins = types.new_dict_obj(Dict::new());
        let modules = types.new_dict_obj(Dict::new());
        let builtins_module = new_obj(types.module.clone(), None, Payload::Module("builtins".into(), builtins.clone()));
        let new_list = |types: &Types| new_obj(types.list.clone(), None, Payload::List(RefCell::new(Vec::new())));
        let gc_garbage = new_list(&types);
        let gc_callbacks = new_list(&types);
        let mut vm = Vm {
            types,
            builtins,
            builtins_module,
            modules,
            frames: Vec::new(),
            exc_state: None,
            host,
            native_modules: Vec::new(),
            stdout_softspace: false,
            module_loaded: None,
            task_base: None,
            recursion_limit: 1000,
            native_depth: 0,
            trace_natives: std::env::var_os("CRYSIRROR_PY_TRACE_NATIVE").is_some(),
            dialect_default: crate::Dialect::Py3,
            ext: RefCell::new(Vec::new()),
            classes: crate::builtins::Classes::default(),
            gc_garbage,
            gc_callbacks,
        };
        crate::builtins::install(&mut vm);
        vm
    }

    // ---- errors ----

    pub fn new_exc(&self, cls: &ObjRef, args: Vec<Value>) -> ObjRef {
        let o = new_obj(cls.clone(), Some(Dict::new()), Payload::None);
        let t = self.types.new_tuple(args);
        o.dict.as_ref().unwrap().borrow_mut().set_str(self.types.intern("args"), t);
        o
    }
    pub fn exc_msg(&self, name: &str, msg: impl Into<String>) -> PyErr {
        let cls = self.types.exc(name);
        let m = self.types.new_string(msg.into());
        PyErr::Exc(self.new_exc(&cls, vec![m]))
    }
    pub fn type_error(&self, msg: impl Into<String>) -> PyErr {
        self.exc_msg("TypeError", msg)
    }
    pub fn value_error(&self, msg: impl Into<String>) -> PyErr {
        self.exc_msg("ValueError", msg)
    }
    pub fn attr_error(&self, msg: impl Into<String>) -> PyErr {
        self.exc_msg("AttributeError", msg)
    }
    pub fn key_error(&self, key: Value) -> PyErr {
        let cls = self.types.exc("KeyError");
        PyErr::Exc(self.new_exc(&cls, vec![key]))
    }
    pub fn index_error(&self, msg: impl Into<String>) -> PyErr {
        self.exc_msg("IndexError", msg)
    }
    pub fn runtime_error(&self, msg: impl Into<String>) -> PyErr {
        self.exc_msg("RuntimeError", msg)
    }
    pub fn stop_iteration(&self, v: Value) -> PyErr {
        let cls = self.types.exc("StopIteration");
        let e = self.new_exc(&cls, if v.is_none() { vec![] } else { vec![v.clone()] });
        e.dict.as_ref().unwrap().borrow_mut().set_str(self.types.intern("value"), v);
        PyErr::Exc(e)
    }
    pub fn is_exc(&self, e: &PyErr, name: &str) -> bool {
        match e {
            PyErr::Exc(o) => crate::ops::is_subclass(&o.class(), &self.types.exc(name)),
            _ => false,
        }
    }

    pub fn str_val(&self, s: &str) -> Value {
        self.types.new_str(s)
    }

    // ---- frames ----

    fn new_frame(&self, code: Rc<Code>, globals: ObjRef, locals: Option<Value>, cells: Vec<ObjRef>) -> Box<Frame> {
        let nfast = code.varnames.len();
        let builtins = match globals.dict_get_str_raw("__builtins__") {
            Some(Value::Obj(o)) if matches!(o.payload, Payload::Dict(_)) => o,
            Some(Value::Obj(o)) => match &o.payload {
                Payload::Module(_, ns) => ns.clone(),
                _ => self.builtins.clone(),
            },
            _ => self.builtins.clone(),
        };
        let func_name = code.name.clone();
        Box::new(Frame {
            code,
            ip: 0,
            stack: Vec::with_capacity(8),
            blocks: Vec::new(),
            fast: vec![None; nfast],
            cells,
            globals,
            locals,
            builtins,
            saved_exc: None,
            func_name,
        })
    }

    /// Builds the frame for a Python function call (argument binding).
    pub fn function_frame(&mut self, func_obj: &ObjRef, mut args: Vec<Value>, kwargs: Kwargs) -> PyResult<Box<Frame>> {
        let Payload::Function(f) = &func_obj.payload else { unreachable!() };
        let code = f.code.clone();
        let argc = code.argcount;
        let kwonly = code.kwonlyargcount;
        let total = argc + kwonly;
        let mut fast: Vec<Option<Value>> = vec![None; code.varnames.len()];
        let fname = f.qualname.borrow().clone();
        let nposargs = args.len();
        let varargs = if args.len() > argc {
            if code.flags & CO_VARARGS == 0 {
                return Err(self.type_error(format!(
                    "{}() takes {} positional argument{} but {} {} given",
                    fname,
                    argc,
                    if argc == 1 { "" } else { "s" },
                    nposargs,
                    if nposargs == 1 { "was" } else { "were" }
                )));
            }
            args.split_off(argc)
        } else {
            Vec::new()
        };
        let npos = args.len();
        for (i, a) in args.into_iter().enumerate() {
            fast[i] = Some(a);
        }
        let mut next = total;
        if code.flags & CO_VARARGS != 0 {
            fast[next] = Some(self.types.new_tuple(varargs));
            next += 1;
        }
        let kwdict = if code.flags & CO_VARKEYWORDS != 0 {
            let d = self.types.new_dict_obj(Dict::new());
            fast[next] = Some(Value::Obj(d.clone()));
            Some(d)
        } else {
            None
        };
        for (k, v) in kwargs {
            let pos = code.varnames[code.posonlyargcount..total].iter().position(|n| **n == *k).map(|p| p + code.posonlyargcount);
            match pos {
                Some(p) => {
                    if fast[p].is_some() {
                        return Err(self.type_error(format!("{fname}() got multiple values for argument '{k}'")));
                    }
                    fast[p] = Some(v);
                }
                None => match &kwdict {
                    Some(d) => {
                        let key = self.types.new_str(&k);
                        crate::ops::dict_set(self, d, key, v)?;
                    }
                    None => return Err(self.type_error(format!("{fname}() got an unexpected keyword argument '{k}'"))),
                },
            }
        }
        // Defaults for missing positional parameters.
        let ndef = f.defaults.len();
        for i in npos..argc {
            if fast[i].is_none() {
                if i >= argc - ndef {
                    fast[i] = Some(f.defaults[i - (argc - ndef)].clone());
                } else {
                    return Err(self.type_error(format!("{}() missing required positional argument: '{}'", fname, code.varnames[i])));
                }
            }
        }
        for i in argc..total {
            if fast[i].is_none() {
                let name = &code.varnames[i];
                let d = f.kwdefaults.as_ref().and_then(|kd| kd.dict_get_str_raw(name));
                match d {
                    Some(v) => fast[i] = Some(v),
                    None => return Err(self.type_error(format!("{fname}() missing required keyword-only argument: '{name}'"))),
                }
            }
        }
        // Cells: cellvars (possibly shadowing args) then the closure.
        let mut cells = Vec::with_capacity(code.cellvars.len() + code.freevars.len());
        for (i, _) in code.cellvars.iter().enumerate() {
            let init = code.cell2arg[i].and_then(|a| fast[a].clone());
            cells.push(self.types.new_cell(init));
        }
        cells.extend(f.closure.iter().cloned());
        let mut frame = self.new_frame(code, f.globals.clone(), None, cells);
        frame.fast = fast;
        frame.func_name = f.name.borrow().clone();
        Ok(frame)
    }

    /// Runs module-level or exec code with the given namespaces.
    pub fn run_code(&mut self, code: Rc<Code>, globals: ObjRef, locals: Option<Value>) -> PyResult<Value> {
        let ncells = code.cellvars.len();
        let cells = (0..ncells).map(|_| self.types.new_cell(None)).collect();
        let locals = locals.or_else(|| Some(Value::Obj(globals.clone())));
        let frame = self.new_frame(code, globals, locals, cells);
        self.run_frame(frame)
    }

    /// Runs a frame to completion on a nested loop.
    pub fn run_frame(&mut self, frame: Box<Frame>) -> PyResult<Value> {
        if self.frames.len() >= self.recursion_limit {
            return Err(self.exc_msg("RecursionError", "maximum recursion depth exceeded"));
        }
        let base = self.frames.len();
        self.frames.push(frame);
        match self.execute(base)? {
            Exit::Return(v) => Ok(v),
            Exit::Yield(..) => Err(self.runtime_error("yield outside generator")),
        }
    }

    // ---- tasks ----

    /// Calls `func` as a suspendable task.
    pub fn start_task(&mut self, func: &Value, args: Vec<Value>, kwargs: Kwargs) -> TaskOutcome {
        let saved_base = self.task_base;
        let base = self.frames.len();
        self.task_base = Some(base);
        let r = match func {
            Value::Obj(o) if matches!(o.payload, Payload::Function(_)) && !Self::is_gen_func(o) => match self.function_frame(o, args, kwargs) {
                Ok(fr) => {
                    self.frames.push(fr);
                    self.execute(base).map(|e| match e {
                        Exit::Return(v) => v,
                        Exit::Yield(..) => Value::None,
                    })
                }
                Err(e) => Err(e),
            },
            _ => self.call(func, args, kwargs),
        };
        self.task_base = saved_base;
        self.finish_task(base, r)
    }

    /// Runs code objects (module/exec) as a suspendable task.
    pub fn start_code_task(&mut self, code: Rc<Code>, globals: ObjRef, locals: Option<Value>) -> TaskOutcome {
        let saved_base = self.task_base;
        let base = self.frames.len();
        self.task_base = Some(base);
        let ncells = code.cellvars.len();
        let cells = (0..ncells).map(|_| self.types.new_cell(None)).collect();
        let locals = locals.or_else(|| Some(Value::Obj(globals.clone())));
        let frame = self.new_frame(code, globals, locals, cells);
        self.frames.push(frame);
        let r = self.execute(base).map(|e| match e {
            Exit::Return(v) => v,
            Exit::Yield(..) => Value::None,
        });
        self.task_base = saved_base;
        self.finish_task(base, r)
    }

    pub fn resume_task(&mut self, task: Task, result: Result<Value, ObjRef>) -> TaskOutcome {
        let saved_base = self.task_base;
        let base = self.frames.len();
        self.task_base = Some(base);
        let saved_exc = std::mem::replace(&mut self.exc_state, task.exc_state);
        self.frames.extend(task.frames);
        let r = match result {
            Ok(v) => {
                top!(self).stack.push(v);
                self.execute(base)
            }
            Err(e) => self.execute_with_exception(base, e),
        }
        .map(|e| match e {
            Exit::Return(v) => v,
            Exit::Yield(..) => Value::None,
        });
        self.task_base = saved_base;
        let out = self.finish_task(base, r);
        if !matches!(out, TaskOutcome::Suspended(..)) {
            self.exc_state = saved_exc;
        }
        out
    }

    fn finish_task(&mut self, base: usize, r: PyResult<Value>) -> TaskOutcome {
        match r {
            Ok(v) => TaskOutcome::Done(v),
            Err(PyErr::Exc(e)) => {
                self.frames.truncate(base);
                TaskOutcome::Error(e)
            }
            Err(PyErr::Suspend(req)) => {
                let frames = self.frames.split_off(base);
                let exc_state = self.exc_state.take();
                TaskOutcome::Suspended(Task { frames, exc_state }, req)
            }
        }
    }

    fn is_gen_func(o: &ObjRef) -> bool {
        matches!(&o.payload, Payload::Function(f) if f.code.is_generator())
    }

    /// Whether native code at this depth may suspend the task.
    pub fn can_suspend(&self) -> bool {
        self.task_base.is_some() && self.native_depth == 0
    }

    // ---- calls ----

    pub fn call(&mut self, func: &Value, args: Vec<Value>, kwargs: Kwargs) -> PyResult<Value> {
        let Value::Obj(o) = func else {
            return Err(self.type_error(format!("'{}' object is not callable", crate::ops::type_name(self, func))));
        };
        match &o.payload {
            Payload::Function(f) => {
                let frame = self.function_frame(o, args, kwargs)?;
                if f.code.is_generator() {
                    return Ok(self.make_generator(frame, f));
                }
                self.native_depth += 1;
                let r = self.run_frame(frame);
                self.native_depth -= 1;
                r
            }
            Payload::Builtin(b) => {
                let func = b.func;
                let r = if let Some(s) = &b.bound {
                    let mut a = Vec::with_capacity(args.len() + 1);
                    a.push(s.clone());
                    a.extend(args);
                    func(self, &a, &kwargs)
                } else {
                    func(self, &args, &kwargs)
                };
                if self.trace_natives {
                    if let Err(PyErr::Exc(e)) = &r {
                        crate::ops::add_traceback(self, e, ("<native>".into(), 0, b.name.into()));
                    }
                }
                r
            }
            Payload::Method { func, slf } => {
                let mut a = Vec::with_capacity(args.len() + 1);
                a.push(slf.clone());
                a.extend(args);
                let f = func.clone();
                self.call(&f, a, kwargs)
            }
            Payload::Type(_) => crate::ops::call_type(self, o, args, kwargs),
            _ => {
                let call = crate::ops::lookup_special(self, func, "__call__");
                match call {
                    Some(c) => {
                        let mut a = Vec::with_capacity(args.len() + 1);
                        a.push(func.clone());
                        a.extend(args);
                        self.call(&c, a, kwargs)
                    }
                    None => Err(self.type_error(format!("'{}' object is not callable", o.class_name()))),
                }
            }
        }
    }

    pub fn call0(&mut self, func: &Value) -> PyResult<Value> {
        self.call(func, Vec::new(), Vec::new())
    }
    pub fn call1(&mut self, func: &Value, a: Value) -> PyResult<Value> {
        self.call(func, vec![a], Vec::new())
    }

    pub fn call_method(&mut self, obj: &Value, name: &str, args: Vec<Value>) -> PyResult<Value> {
        let f = crate::ops::getattr(self, obj, name)?;
        self.call(&f, args, Vec::new())
    }

    fn make_generator(&self, frame: Box<Frame>, f: &Function) -> Value {
        let g = Generator { frame: Some(frame), status: GenStatus::Created, name: f.name.borrow().clone(), qualname: f.qualname.borrow().clone(), yield_from: None };
        Value::Obj(new_obj(self.types.generator.clone(), None, Payload::Generator(RefCell::new(g))))
    }

    pub fn gen_resume(&mut self, genobj: &ObjRef, r: Resume) -> PyResult<GenStep> {
        let Payload::Generator(g) = &genobj.payload else { return Err(self.type_error("not a generator")) };
        let mut frame = {
            let mut gb = g.borrow_mut();
            match gb.status {
                GenStatus::Running => return Err(self.exc_msg("ValueError", "generator already executing")),
                GenStatus::Finished => {
                    return match r {
                        Resume::Throw(e) => Err(PyErr::Exc(e)),
                        Resume::Send(_) => Err(self.stop_iteration(Value::None)),
                    };
                }
                GenStatus::Created => {
                    if let Resume::Send(v) = &r {
                        if !v.is_none() {
                            return Err(self.type_error("can't send non-None value to a just-started generator"));
                        }
                    }
                }
                GenStatus::Suspended => {}
            }
            let first = matches!(gb.status, GenStatus::Created);
            gb.status = GenStatus::Running;
            let mut fr = gb.frame.take().unwrap();
            if !first {
                if let Resume::Send(v) = &r {
                    fr.stack.push(v.clone());
                }
            }
            fr
        };
        if self.frames.len() >= self.recursion_limit {
            let mut gb = g.borrow_mut();
            gb.frame = Some(frame);
            gb.status = GenStatus::Suspended;
            return Err(self.exc_msg("RecursionError", "maximum recursion depth exceeded"));
        }
        let base = self.frames.len();
        frame.saved_exc = self.exc_state.take();
        self.frames.push(frame);
        self.native_depth += 1;
        let res = match r {
            Resume::Send(_) => self.execute(base),
            Resume::Throw(e) => self.execute_with_exception(base, e),
        };
        self.native_depth -= 1;
        match res {
            Ok(Exit::Yield(v, mut fr)) => {
                self.exc_state = fr.saved_exc.take();
                let mut gb = g.borrow_mut();
                gb.frame = Some(fr);
                gb.status = GenStatus::Suspended;
                Ok(GenStep::Yielded(v))
            }
            Ok(Exit::Return(v)) => {
                g.borrow_mut().status = GenStatus::Finished;
                Ok(GenStep::Returned(v))
            }
            Err(e) => {
                g.borrow_mut().status = GenStatus::Finished;
                if let PyErr::Exc(o) = &e {
                    // PEP 479: StopIteration leaking out of a generator.
                    if crate::ops::is_subclass(&o.class(), &self.types.exc("StopIteration")) {
                        return Err(self.runtime_error("generator raised StopIteration"));
                    }
                }
                Err(e)
            }
        }
    }

    // ---- interpreter loop ----

    fn execute_with_exception(&mut self, base: usize, exc: ObjRef) -> PyResult<Exit> {
        match self.unwind(base, PyErr::Exc(exc)) {
            Ok(()) => self.execute(base),
            Err(e) => Err(e),
        }
    }

    /// Unwinds to a handler in the frames above `base`. Ok means execution
    /// continues at a handler; Err means the exception left `base`.
    fn unwind(&mut self, base: usize, err: PyErr) -> PyResult<()> {
        let exc = match err {
            PyErr::Exc(e) => e,
            PyErr::Suspend(s) => {
                if self.task_base == Some(base) && self.native_depth == 0 {
                    return Err(PyErr::Suspend(s));
                }
                match self.runtime_error("cannot suspend inside a nested Python call") {
                    PyErr::Exc(e) => e,
                    _ => unreachable!(),
                }
            }
        };
        loop {
            let f = top!(self);
            // Record traceback position once per frame.
            let line = f.code.line_at(f.ip.saturating_sub(1));
            let entry = (f.code.filename.clone(), line, f.func_name.clone());
            crate::ops::add_traceback(self, &exc, entry);
            loop {
                let Some(b) = top!(self).blocks.pop() else { break };
                match b.kind {
                    BlockKind::ExceptHandler => {
                        self.exc_state = b.prev_exc;
                        top!(self).stack.truncate(b.level);
                    }
                    BlockKind::Finally => {
                        top!(self).stack.truncate(b.level);
                        let prev = self.exc_state.replace(Value::Obj(exc.clone()));
                        let f = top!(self);
                        f.blocks.push(Block { kind: BlockKind::ExceptHandler, handler: 0, level: b.level, prev_exc: prev });
                        f.stack.push(Value::Obj(exc));
                        f.ip = b.handler as usize;
                        return Ok(());
                    }
                }
            }
            // No handler in this frame.
            let fr = self.frames.pop().unwrap();
            if let Some(saved) = fr.saved_exc {
                self.exc_state = Some(saved);
            }
            if self.frames.len() <= base {
                return Err(PyErr::Exc(exc));
            }
        }
    }

    fn pop(&mut self) -> Value {
        top!(self).stack.pop().expect("stack underflow")
    }
    fn push(&mut self, v: Value) {
        top!(self).stack.push(v);
    }
    fn peek(&mut self, n: usize) -> Value {
        let s = &top!(self).stack;
        s[s.len() - 1 - n].clone()
    }

    pub fn execute(&mut self, base: usize) -> PyResult<Exit> {
        loop {
            match self.step_loop(base) {
                Ok(exit) => {
                    // Work queued by the last instructions of an outermost run.
                    if base == 0 && crate::gc::pending_hint() {
                        crate::gc::safe_point(self);
                    }
                    return Ok(exit);
                }
                Err(e) => {
                    if self.frames.len() <= base {
                        return Err(e);
                    }
                    self.unwind(base, e)?;
                }
            }
        }
    }

    fn name_value(&self, idx: u32) -> Value {
        self.frames.last().unwrap().code.names[idx as usize].clone()
    }

    fn step_loop(&mut self, base: usize) -> PyResult<Exit> {
        loop {
            // GC safe point: no borrows are held between instructions.
            if crate::gc::pending_hint() {
                crate::gc::safe_point(self);
            }
            let op = {
                let f = top!(self);
                let op = f.code.ops[f.ip];
                f.ip += 1;
                op
            };
            match op {
                Op::Nop => {}
                Op::LoadConst(i) => {
                    let f = top!(self);
                    let v = f.code.consts[i as usize].clone();
                    f.stack.push(v);
                }
                Op::LoadFast(i) => {
                    let f = top!(self);
                    match &f.fast[i as usize] {
                        Some(v) => {
                            let v = v.clone();
                            f.stack.push(v);
                        }
                        None => {
                            let n = f.code.varnames[i as usize].clone();
                            return Err(self.exc_msg("UnboundLocalError", format!("local variable '{n}' referenced before assignment")));
                        }
                    }
                }
                Op::StoreFast(i) => {
                    let v = self.pop();
                    top!(self).fast[i as usize] = Some(v);
                }
                Op::DelFast(i) => {
                    let f = top!(self);
                    if f.fast[i as usize].take().is_none() {
                        let n = f.code.varnames[i as usize].clone();
                        return Err(self.exc_msg("UnboundLocalError", format!("local variable '{n}' referenced before assignment")));
                    }
                }
                Op::LoadDeref(i) | Op::LoadClassDeref(i) => {
                    if matches!(op, Op::LoadClassDeref(_)) {
                        let name = top!(self).code.deref_name(i as usize).to_string();
                        if let Some(Value::Obj(l)) = top!(self).locals.clone() {
                            if let Some(v) = crate::ops::mapping_get_str(self, &l, &name)? {
                                self.push(v);
                                continue;
                            }
                        }
                    }
                    let f = top!(self);
                    let cell = f.cells[i as usize].clone();
                    let Payload::Cell(c) = &cell.payload else { unreachable!() };
                    match c.borrow().clone() {
                        Some(v) => f.stack.push(v),
                        None => {
                            let n = f.code.deref_name(i as usize).to_string();
                            return Err(self.exc_msg("NameError", format!("free variable '{n}' referenced before assignment in enclosing scope")));
                        }
                    }
                }
                Op::StoreDeref(i) => {
                    let v = self.pop();
                    let cell = top!(self).cells[i as usize].clone();
                    if let Payload::Cell(c) = &cell.payload {
                        *c.borrow_mut() = Some(v);
                    }
                }
                Op::DelDeref(i) => {
                    let cell = top!(self).cells[i as usize].clone();
                    if let Payload::Cell(c) = &cell.payload {
                        *c.borrow_mut() = None;
                    }
                }
                Op::LoadClosure(i) => {
                    let f = top!(self);
                    let c = Value::Obj(f.cells[i as usize].clone());
                    f.stack.push(c);
                }
                Op::LoadName(i) => {
                    let name = self.name_value(i);
                    let v = self.load_name(&name)?;
                    self.push(v);
                }
                Op::StoreName(i) => {
                    let name = self.name_value(i);
                    let v = self.pop();
                    let locals = { let f = top!(self); f.locals.clone().unwrap_or_else(|| Value::Obj(f.globals.clone())) };
                    crate::ops::setitem(self, &locals, name, v)?;
                }
                Op::DelName(i) => {
                    let name = self.name_value(i);
                    let locals = { let f = top!(self); f.locals.clone().unwrap_or_else(|| Value::Obj(f.globals.clone())) };
                    if let Err(e) = crate::ops::delitem(self, &locals, name.clone()) {
                        if self.is_exc(&e, "KeyError") {
                            return Err(self.exc_msg("NameError", format!("name '{}' is not defined", name.as_str().unwrap_or(""))));
                        }
                        return Err(e);
                    }
                }
                Op::LoadGlobal(i) => {
                    let name = self.name_value(i);
                    let v = self.load_global(&name)?;
                    self.push(v);
                }
                Op::StoreGlobal(i) => {
                    let name = self.name_value(i);
                    let v = self.pop();
                    let g = top!(self).globals.clone();
                    crate::ops::dict_set(self, &g, name, v)?;
                }
                Op::DelGlobal(i) => {
                    let name = self.name_value(i);
                    let g = top!(self).globals.clone();
                    crate::ops::dict_del(self, &g, name)?;
                }
                Op::LoadAttr(i) => {
                    let name = self.name_value(i);
                    let o = self.pop();
                    let v = crate::ops::getattr_v(self, &o, &name)?;
                    self.push(v);
                }
                Op::StoreAttr(i) => {
                    let name = self.name_value(i);
                    let o = self.pop();
                    let v = self.pop();
                    crate::ops::setattr(self, &o, &name, v)?;
                }
                Op::DelAttr(i) => {
                    let name = self.name_value(i);
                    let o = self.pop();
                    crate::ops::delattr(self, &o, &name)?;
                }
                Op::Subscr => {
                    let k = self.pop();
                    let o = self.pop();
                    let v = crate::ops::getitem(self, &o, k)?;
                    self.push(v);
                }
                Op::StoreSubscr => {
                    let k = self.pop();
                    let o = self.pop();
                    let v = self.pop();
                    crate::ops::setitem(self, &o, k, v)?;
                }
                Op::DelSubscr => {
                    let k = self.pop();
                    let o = self.pop();
                    crate::ops::delitem(self, &o, k)?;
                }
                Op::BuildSlice(n) => {
                    let step = if n == 3 { self.pop() } else { Value::None };
                    let stop = self.pop();
                    let start = self.pop();
                    let s = Value::Obj(new_obj(self.types.slice.clone(), None, Payload::Slice(start, stop, step)));
                    self.push(s);
                }
                Op::Binary(bop) => {
                    let b = self.pop();
                    let a = self.pop();
                    let classic = top!(self).code.classic_division;
                    let v = crate::ops::binary_op(self, &a, &b, bop, false, classic)?;
                    self.push(v);
                }
                Op::Inplace(bop) => {
                    let b = self.pop();
                    let a = self.pop();
                    let classic = top!(self).code.classic_division;
                    let v = crate::ops::binary_op(self, &a, &b, bop, true, classic)?;
                    self.push(v);
                }
                Op::Unary(uop) => {
                    let a = self.pop();
                    let v = crate::ops::unary_op(self, &a, uop)?;
                    self.push(v);
                }
                Op::Compare(c) => {
                    let b = self.pop();
                    let a = self.pop();
                    let v = crate::ops::compare(self, &a, &b, c)?;
                    self.push(v);
                }
                Op::Repr => {
                    let a = self.pop();
                    let s = crate::ops::repr(self, &a)?;
                    let v = self.types.new_string(s);
                    self.push(v);
                }
                Op::Pop => {
                    self.pop();
                }
                Op::Dup => {
                    let v = self.peek(0);
                    self.push(v);
                }
                Op::Dup2 => {
                    let a = self.peek(1);
                    let b = self.peek(0);
                    self.push(a);
                    self.push(b);
                }
                Op::Rot2 => {
                    let s = &mut top!(self).stack;
                    let n = s.len();
                    s.swap(n - 1, n - 2);
                }
                Op::Rot3 => {
                    let s = &mut top!(self).stack;
                    let v = s.pop().unwrap();
                    let n = s.len();
                    s.insert(n - 2, v);
                }
                Op::Rot4 => {
                    let s = &mut top!(self).stack;
                    let v = s.pop().unwrap();
                    let n = s.len();
                    s.insert(n - 3, v);
                }
                Op::Jump(t) => top!(self).ip = t as usize,
                Op::PopJumpIfFalse(t) => {
                    let v = self.pop();
                    if !crate::ops::truthy(self, &v)? {
                        top!(self).ip = t as usize;
                    }
                }
                Op::PopJumpIfTrue(t) => {
                    let v = self.pop();
                    if crate::ops::truthy(self, &v)? {
                        top!(self).ip = t as usize;
                    }
                }
                Op::JumpIfFalseOrPop(t) => {
                    let v = self.peek(0);
                    if !crate::ops::truthy(self, &v)? {
                        top!(self).ip = t as usize;
                    } else {
                        self.pop();
                    }
                }
                Op::JumpIfTrueOrPop(t) => {
                    let v = self.peek(0);
                    if crate::ops::truthy(self, &v)? {
                        top!(self).ip = t as usize;
                    } else {
                        self.pop();
                    }
                }
                Op::GetIter => {
                    let v = self.pop();
                    let it = crate::ops::iter(self, &v)?;
                    self.push(it);
                }
                Op::ForIter(t) => {
                    let it = self.peek(0);
                    match crate::ops::next(self, &it)? {
                        Some(v) => self.push(v),
                        None => {
                            self.pop();
                            top!(self).ip = t as usize;
                        }
                    }
                }
                Op::BuildTuple(n) => {
                    let items = self.pop_n(n as usize);
                    let t = self.types.new_tuple(items);
                    self.push(t);
                }
                Op::BuildList(n) => {
                    let items = self.pop_n(n as usize);
                    let l = self.types.new_list(items);
                    self.push(l);
                }
                Op::BuildSet(n) => {
                    let items = self.pop_n(n as usize);
                    let s = self.types.new_set(Dict::new());
                    for it in items {
                        crate::ops::set_add(self, &s, it)?;
                    }
                    self.push(s);
                }
                Op::BuildMap(n) => {
                    let items = self.pop_n(2 * n as usize);
                    let d = self.types.new_dict_obj(Dict::new());
                    let mut it = items.into_iter();
                    while let (Some(k), Some(v)) = (it.next(), it.next()) {
                        crate::ops::dict_set(self, &d, k, v)?;
                    }
                    self.push(Value::Obj(d));
                }
                Op::BuildString(n) => {
                    let items = self.pop_n(n as usize);
                    let mut s = String::new();
                    for it in &items {
                        s.push_str(it.as_str().unwrap_or(""));
                    }
                    let v = self.types.new_string(s);
                    self.push(v);
                }
                Op::FormatValue(conv, has_spec) => {
                    let spec = if has_spec { Some(self.pop()) } else { None };
                    let mut v = self.pop();
                    if conv != 0 {
                        let s = match conv {
                            b's' => crate::ops::str(self, &v)?,
                            b'r' => crate::ops::repr(self, &v)?,
                            _ => crate::ops::ascii(self, &v)?,
                        };
                        v = self.types.new_string(s);
                    }
                    let spec_s = spec.as_ref().and_then(|s| s.as_str().map(|x| x.to_string())).unwrap_or_default();
                    let out = crate::ops::format(self, &v, &spec_s)?;
                    let r = self.types.new_string(out);
                    self.push(r);
                }
                Op::ListAppend(d) => {
                    let v = self.pop();
                    let l = self.peek(d as usize);
                    crate::ops::list_append(self, &l, v)?;
                }
                Op::SetAdd(d) => {
                    let v = self.pop();
                    let s = self.peek(d as usize);
                    crate::ops::set_add(self, &s, v)?;
                }
                Op::MapAdd(d) => {
                    let v = self.pop();
                    let k = self.pop();
                    let m = self.peek(d as usize);
                    let Value::Obj(mo) = &m else { unreachable!() };
                    crate::ops::dict_set(self, mo, k, v)?;
                }
                Op::ListExtend(d) => {
                    let v = self.pop();
                    let l = self.peek(d as usize);
                    let items = crate::ops::collect(self, &v)?;
                    if let Some(Payload::List(lv)) = l.payload() {
                        lv.borrow_mut().extend(items);
                    }
                }
                Op::SetUpdate(d) => {
                    let v = self.pop();
                    let s = self.peek(d as usize);
                    for it in crate::ops::collect(self, &v)? {
                        crate::ops::set_add(self, &s, it)?;
                    }
                }
                Op::DictUpdate(d) | Op::DictMerge(d) => {
                    let v = self.pop();
                    let m = self.peek(d as usize);
                    let Value::Obj(mo) = &m else { unreachable!() };
                    let items = crate::ops::mapping_items(self, &v)?;
                    for (k, val) in items {
                        if matches!(op, Op::DictMerge(_)) && crate::ops::dict_get(self, mo, &k)?.is_some() {
                            let ks = crate::ops::str(self, &k)?;
                            return Err(self.type_error(format!("got multiple values for keyword argument '{ks}'")));
                        }
                        crate::ops::dict_set(self, mo, k, val)?;
                    }
                }
                Op::ListToTuple => {
                    let l = self.pop();
                    let items = match l.payload() {
                        Some(Payload::List(v)) => v.borrow().clone(),
                        _ => Vec::new(),
                    };
                    let t = self.types.new_tuple(items);
                    self.push(t);
                }
                Op::UnpackSequence(n) => {
                    let v = self.pop();
                    let items = crate::ops::collect(self, &v)?;
                    if items.len() != n as usize {
                        return Err(self.value_error(if items.len() > n as usize {
                            format!("too many values to unpack (expected {n})")
                        } else {
                            format!("not enough values to unpack (expected {n}, got {})", items.len())
                        }));
                    }
                    let f = top!(self);
                    for it in items.into_iter().rev() {
                        f.stack.push(it);
                    }
                }
                Op::UnpackEx(before, after) => {
                    let v = self.pop();
                    let mut items = crate::ops::collect(self, &v)?;
                    let (b, a) = (before as usize, after as usize);
                    if items.len() < b + a {
                        return Err(self.value_error(format!("not enough values to unpack (expected at least {}, got {})", b + a, items.len())));
                    }
                    let tail = items.split_off(items.len() - a);
                    let mid = items.split_off(b);
                    let mut out = items;
                    out.push(self.types.new_list(mid));
                    out.extend(tail);
                    let f = top!(self);
                    for it in out.into_iter().rev() {
                        f.stack.push(it);
                    }
                }
                Op::Call(n) => {
                    let args = self.pop_n(n as usize);
                    let func = self.pop();
                    self.do_call(func, args, Vec::new())?;
                }
                Op::CallKw(n) => {
                    let names = self.pop();
                    let mut args = self.pop_n(n as usize);
                    let func = self.pop();
                    let Some(Payload::Tuple(names)) = names.payload() else { unreachable!() };
                    let kw_vals = args.split_off(args.len() - names.len());
                    let kwargs: Kwargs = names.iter().zip(kw_vals).map(|(k, v)| (Rc::from(k.as_str().unwrap()), v)).collect();
                    self.do_call(func, args, kwargs)?;
                }
                Op::CallEx(has_kw) => {
                    let kw = if has_kw { Some(self.pop()) } else { None };
                    let args = self.pop();
                    let func = self.pop();
                    let args = crate::ops::collect(self, &args)?;
                    let mut kwargs = Vec::new();
                    if let Some(kw) = kw {
                        for (k, v) in crate::ops::mapping_items(self, &kw)? {
                            let Some(ks) = k.as_str() else {
                                return Err(self.type_error("keywords must be strings"));
                            };
                            kwargs.push((Rc::from(ks), v));
                        }
                    }
                    self.do_call(func, args, kwargs)?;
                }
                Op::MakeFunction(flags) => {
                    let qual = self.pop();
                    let code = self.pop();
                    let closure = if flags & 8 != 0 {
                        match self.pop().payload() {
                            Some(Payload::Tuple(t)) => t.iter().filter_map(|c| c.obj().cloned()).collect(),
                            _ => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    if flags & 4 != 0 {
                        self.pop();
                    }
                    let kwdefaults = if flags & 2 != 0 { self.pop().obj().cloned() } else { None };
                    let defaults = if flags & 1 != 0 {
                        match self.pop().payload() {
                            Some(Payload::Tuple(t)) => t.to_vec(),
                            _ => Vec::new(),
                        }
                    } else {
                        Vec::new()
                    };
                    let Some(Payload::Code(code)) = code.payload() else { unreachable!() };
                    let globals = top!(self).globals.clone();
                    let module = globals.dict_get_str_raw("__name__").unwrap_or(Value::None);
                    let doc = code.doc.clone().unwrap_or(Value::None);
                    let func = Function {
                        code: code.clone(),
                        globals,
                        defaults,
                        kwdefaults,
                        closure,
                        name: RefCell::new(code.name.clone()),
                        qualname: RefCell::new(Rc::from(qual.as_str().unwrap_or(&code.name))),
                        module: RefCell::new(module),
                        doc: RefCell::new(doc),
                    };
                    let fo = new_obj(self.types.function.clone(), Some(Dict::new()), Payload::Function(func));
                    self.push(Value::Obj(fo));
                }
                Op::Return => {
                    let v = self.pop();
                    let fr = self.frames.pop().unwrap();
                    if let Some(saved) = fr.saved_exc {
                        self.exc_state = Some(saved);
                    }
                    if self.frames.len() <= base {
                        return Ok(Exit::Return(v));
                    }
                    self.push(v);
                }
                Op::Yield => {
                    let v = self.pop();
                    let fr = self.frames.pop().unwrap();
                    return Ok(Exit::Yield(v, fr));
                }
                Op::GetYieldFromIter => {
                    let v = self.pop();
                    let it = if matches!(v.payload(), Some(Payload::Generator(_))) { v } else { crate::ops::iter(self, &v)? };
                    self.push(it);
                }
                Op::YieldFrom => {
                    let sent = self.pop();
                    let sub = self.peek(0);
                    let step = match sub.payload() {
                        Some(Payload::Generator(_)) => self.gen_resume(sub.obj().unwrap(), Resume::Send(sent))?,
                        _ => {
                            if sent.is_none() {
                                match crate::ops::next(self, &sub)? {
                                    Some(v) => GenStep::Yielded(v),
                                    None => GenStep::Returned(Value::None),
                                }
                            } else {
                                match self.call_method(&sub, "send", vec![sent]) {
                                    Ok(v) => GenStep::Yielded(v),
                                    Err(e) if self.is_exc(&e, "StopIteration") => GenStep::Returned(crate::ops::stop_value(self, &e)),
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    };
                    match step {
                        GenStep::Yielded(v) => {
                            top!(self).ip -= 1;
                            let fr = self.frames.pop().unwrap();
                            return Ok(Exit::Yield(v, fr));
                        }
                        GenStep::Returned(v) => {
                            self.pop();
                            self.push(v);
                        }
                    }
                }
                Op::SetupFinally(h) | Op::SetupWith(h) => {
                    if matches!(op, Op::SetupWith(_)) {
                        let mgr = self.pop();
                        let exit = crate::ops::lookup_special(self, &mgr, "__exit__");
                        let enter = crate::ops::lookup_special(self, &mgr, "__enter__");
                        let (Some(exit), Some(enter)) = (exit, enter) else {
                            return Err(self.attr_error("__enter__"));
                        };
                        let bound_exit = crate::ops::bind(self, &exit, &mgr);
                        self.push(bound_exit);
                        let r = self.call(&enter, vec![mgr], Vec::new())?;
                        let f = top!(self);
                        let level = f.stack.len();
                        f.blocks.push(Block { kind: BlockKind::Finally, handler: h, level, prev_exc: None });
                        f.stack.push(r);
                    } else {
                        let f = top!(self);
                        let level = f.stack.len();
                        f.blocks.push(Block { kind: BlockKind::Finally, handler: h, level, prev_exc: None });
                    }
                }
                Op::PopBlock => {
                    top!(self).blocks.pop();
                }
                Op::PopExcept => {
                    let b = top!(self).blocks.pop();
                    if let Some(b) = b {
                        if b.kind == BlockKind::ExceptHandler {
                            self.exc_state = b.prev_exc;
                        }
                    }
                }
                Op::Raise(n) => {
                    return Err(match n {
                        0 => match self.exc_state.clone() {
                            Some(Value::Obj(e)) => PyErr::Exc(e),
                            _ => self.runtime_error("No active exception to reraise"),
                        },
                        1 => {
                            let e = self.pop();
                            self.make_raise(e, None)?
                        }
                        2 => {
                            let cause = self.pop();
                            let e = self.pop();
                            self.make_raise(e, Some(cause))?
                        }
                        _ => {
                            let val = self.pop();
                            let typ = self.pop();
                            self.make_raise_py2(typ, val)?
                        }
                    });
                }
                Op::Reraise => {
                    let e = self.pop();
                    match e {
                        Value::Obj(o) => {
                            // Pop the handler's ExceptHandler block first so the
                            // previous exception state is restored.
                            return Err(PyErr::Exc(o));
                        }
                        _ => return Err(self.runtime_error("bad reraise")),
                    }
                }
                Op::CheckExcMatch => {
                    let cls = self.pop();
                    let exc = self.peek(0);
                    let m = crate::ops::exc_matches(self, &exc, &cls)?;
                    self.push(Value::Bool(m));
                }
                Op::WithExceptStart => {
                    let exc = self.peek(0);
                    let exit = self.peek(1);
                    let typ = Value::Obj(crate::ops::type_of(self, &exc));
                    let tb = crate::ops::getattr_opt(self, &exc, "__traceback__")?.unwrap_or(Value::None);
                    let r = self.call(&exit, vec![typ, exc, tb], Vec::new())?;
                    self.push(r);
                }
                Op::ImportName(i) => {
                    let name = self.name_value(i);
                    let fromlist = self.pop();
                    let level = self.pop();
                    let m = crate::import::import_name(self, &name, &fromlist, &level)?;
                    self.push(m);
                }
                Op::ImportFrom(i) => {
                    let name = self.name_value(i);
                    let m = self.peek(0);
                    let v = crate::import::import_from(self, &m, &name)?;
                    self.push(v);
                }
                Op::ImportStar => {
                    let m = self.pop();
                    crate::import::import_star(self, &m)?;
                }
                Op::LoadBuildClass => {
                    let b = self.types.new_builtin("__build_class__", crate::builtins::build_class);
                    self.push(b);
                }
                Op::SetupAnnotations => {}
                Op::PrintItem | Op::PrintItemTo => {
                    let v = self.pop();
                    let dest = if op == Op::PrintItemTo { Some(self.pop()) } else { None };
                    crate::builtins::print_item(self, dest.as_ref(), &v)?;
                }
                Op::PrintNewline | Op::PrintNewlineTo => {
                    let dest = if op == Op::PrintNewlineTo { Some(self.pop()) } else { None };
                    crate::builtins::print_write(self, dest.as_ref(), "\n", false)?;
                    crate::builtins::set_softspace(self, dest.as_ref(), false)?;
                }
                Op::PrintExpr => {
                    let v = self.pop();
                    if !v.is_none() {
                        let s = crate::ops::repr(self, &v)?;
                        self.host.write_stdout(&(s + "\n"));
                    }
                }
                Op::ExecStmt => {
                    let locals = self.pop();
                    let globals = self.pop();
                    let code = self.pop();
                    let (g, l) = if globals.is_none() {
                        let f = top!(self);
                        (Value::Obj(f.globals.clone()), f.locals.clone().unwrap_or(Value::Obj(f.globals.clone())))
                    } else {
                        let l = if locals.is_none() { globals.clone() } else { locals };
                        (globals, l)
                    };
                    crate::builtins::exec_impl(self, &code, &g, Some(l), crate::Dialect::Py2)?;
                }
                Op::MatchSequence => {
                    let v = self.peek(0);
                    let ok = crate::ops::is_sequence_for_match(self, &v);
                    self.push(Value::Bool(ok));
                }
                Op::MatchMapping => {
                    let v = self.peek(0);
                    let ok = crate::ops::is_mapping(self, &v);
                    self.push(Value::Bool(ok));
                }
                Op::GetLen => {
                    let v = self.peek(0);
                    let n = crate::ops::len(self, &v)?;
                    self.push(Value::Int(n as i64));
                }
                Op::MatchKeys => {
                    let keys = self.peek(0);
                    let subj = self.peek(1);
                    let mut vals = Vec::new();
                    let mut ok = true;
                    for k in crate::ops::collect(self, &keys)? {
                        match crate::ops::getitem(self, &subj, k) {
                            Ok(v) => vals.push(v),
                            Err(e) if self.is_exc(&e, "KeyError") => {
                                ok = false;
                                break;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    let r = if ok { self.types.new_tuple(vals) } else { Value::None };
                    self.push(r);
                }
                Op::CopyDictWithoutKeys => {
                    let keys = self.peek(0);
                    let subj = self.peek(1);
                    let d = self.types.new_dict_obj(Dict::new());
                    let ks = crate::ops::collect(self, &keys)?;
                    for (k, v) in crate::ops::mapping_items(self, &subj)? {
                        let mut skip = false;
                        for kk in &ks {
                            if crate::ops::eq(self, &k, kk)? {
                                skip = true;
                            }
                        }
                        if !skip {
                            crate::ops::dict_set(self, &d, k, v)?;
                        }
                    }
                    self.push(Value::Obj(d));
                }
                Op::MatchClass(npos) => {
                    let names = self.pop();
                    let cls = self.pop();
                    let subj = self.pop();
                    let r = crate::ops::match_class(self, &subj, &cls, npos as usize, &names)?;
                    self.push(r);
                }
                Op::IndexConst(i) => {
                    let v = self.pop();
                    let r = crate::ops::getitem(self, &v, Value::Int(i as i64))?;
                    self.push(r);
                }
            }
        }
    }

    fn pop_n(&mut self, n: usize) -> Vec<Value> {
        let s = &mut top!(self).stack;
        let at = s.len() - n;
        s.split_off(at)
    }

    fn do_call(&mut self, func: Value, args: Vec<Value>, kwargs: Kwargs) -> PyResult<()> {
        if let Value::Obj(o) = &func {
            match &o.payload {
                Payload::Function(f) if !f.code.is_generator() => {
                    if self.frames.len() >= self.recursion_limit {
                        return Err(self.exc_msg("RecursionError", "maximum recursion depth exceeded"));
                    }
                    let fr = self.function_frame(o, args, kwargs)?;
                    self.frames.push(fr);
                    return Ok(());
                }
                Payload::Method { func: inner, slf } => {
                    if let Value::Obj(io) = inner {
                        if let Payload::Function(f) = &io.payload {
                            if !f.code.is_generator() {
                                let mut a = Vec::with_capacity(args.len() + 1);
                                a.push(slf.clone());
                                a.extend(args);
                                let fr = self.function_frame(io, a, kwargs)?;
                                self.frames.push(fr);
                                return Ok(());
                            }
                        }
                    }
                }
                Payload::Function(_) | Payload::Builtin(_) | Payload::Type(_) => {}
                _ => {
                    if let Some(method) = crate::ops::lookup_special(self, &func, "__call__") {
                        if matches!(method.payload(), Some(Payload::Function(_))) {
                            let mut bound = Vec::with_capacity(args.len() + 1);
                            bound.push(func);
                            bound.extend(args);
                            return self.do_call(method, bound, kwargs);
                        }
                    }
                }
            }
        }
        let r = self.call(&func, args, kwargs)?;
        self.push(r);
        Ok(())
    }

    fn load_name(&mut self, name: &Value) -> PyResult<Value> {
        let f = top!(self);
        let locals = f.locals.clone();
        let globals = f.globals.clone();
        let builtins = f.builtins.clone();
        if let Some(Value::Obj(l)) = &locals {
            if !Rc::ptr_eq(l, &globals) {
                let key = name.as_str().unwrap();
                if let Some(v) = crate::ops::mapping_get_str(self, l, key)? {
                    return Ok(v);
                }
            }
        }
        self.load_global_from(name, &globals, &builtins)
    }

    fn load_global(&mut self, name: &Value) -> PyResult<Value> {
        let f = top!(self);
        let globals = f.globals.clone();
        let builtins = f.builtins.clone();
        self.load_global_from(name, &globals, &builtins)
    }

    fn load_global_from(&mut self, name: &Value, globals: &ObjRef, builtins: &ObjRef) -> PyResult<Value> {
        let key = name.as_str().unwrap();
        if let Some(v) = crate::ops::mapping_get_str(self, globals, key)? {
            return Ok(v);
        }
        if let Some(v) = builtins.dict_get_str_raw(key) {
            return Ok(v);
        }
        Err(self.exc_msg("NameError", format!("name '{key}' is not defined")))
    }

    fn make_raise(&mut self, e: Value, cause: Option<Value>) -> PyResult<PyErr> {
        let exc = self.instantiate_exc(e)?;
        if let Some(c) = cause {
            let c = if c.is_none() { Value::None } else { Value::Obj(self.instantiate_exc(c)?) };
            crate::ops::set_raw_attr(self, &exc, "__cause__", c);
            crate::ops::set_raw_attr(self, &exc, "__suppress_context__", Value::Bool(true));
        }
        self.set_context(&exc);
        Ok(PyErr::Exc(exc))
    }

    fn make_raise_py2(&mut self, typ: Value, val: Value) -> PyResult<PyErr> {
        let exc = if crate::ops::is_type(&typ) {
            if crate::ops::isinstance(self, &val, typ.obj().unwrap()) {
                val.obj().unwrap().clone()
            } else {
                let args = match val.payload() {
                    Some(Payload::Tuple(t)) => t.to_vec(),
                    _ if val.is_none() => Vec::new(),
                    _ => vec![val],
                };
                let r = self.call(&typ, args, Vec::new())?;
                self.instantiate_exc(r)?
            }
        } else {
            self.instantiate_exc(typ)?
        };
        self.set_context(&exc);
        Ok(PyErr::Exc(exc))
    }

    fn set_context(&mut self, exc: &ObjRef) {
        if let Some(Value::Obj(cur)) = &self.exc_state {
            if !Rc::ptr_eq(cur, exc) {
                let c = Value::Obj(cur.clone());
                crate::ops::set_raw_attr(self, exc, "__context__", c);
            }
        }
    }

    pub fn instantiate_exc(&mut self, e: Value) -> PyResult<ObjRef> {
        let base = self.types.exc("BaseException");
        if crate::ops::is_type(&e) {
            let cls = e.obj().unwrap().clone();
            if !crate::ops::is_subclass(&cls, &base) {
                return Err(self.type_error("exceptions must derive from BaseException"));
            }
            let inst = self.call(&e, Vec::new(), Vec::new())?;
            return Ok(inst.obj().unwrap().clone());
        }
        match &e {
            Value::Obj(o) if crate::ops::is_subclass(&o.class(), &base) => Ok(o.clone()),
            _ => Err(self.type_error("exceptions must derive from BaseException")),
        }
    }

    // ---- compile/exec helpers ----

    pub fn compile_exec(&self, src: &str, filename: &str, dialect: crate::Dialect) -> Result<Rc<Code>, crate::Error> {
        let body = match dialect {
            crate::Dialect::Py3 => crate::parser::parse_module(src, crate::parser::ParseOptions::py3()),
            crate::Dialect::Py2 => crate::parser::parse_module_py2_fallback(src),
        }
        .map_err(|e| crate::Error::Syntax { msg: e.msg, filename: filename.into(), line: e.line })?;
        let opts = crate::compile::CompileOptions::new(dialect, filename);
        crate::compile::compile_module(&self.types, &body, opts).map_err(|e| crate::Error::Syntax { msg: e.msg, filename: filename.into(), line: e.line })
    }

    pub fn compile_eval(&self, src: &str, filename: &str, dialect: crate::Dialect) -> Result<Rc<Code>, crate::Error> {
        let opts = match dialect {
            crate::Dialect::Py3 => crate::parser::ParseOptions::py3(),
            crate::Dialect::Py2 => crate::parser::ParseOptions::py2(),
        };
        let e = crate::parser::parse_expression(src, opts).map_err(|e| crate::Error::Syntax { msg: e.msg, filename: filename.into(), line: e.line })?;
        let opts = crate::compile::CompileOptions::new(dialect, filename);
        crate::compile::compile_eval(&self.types, &e, opts).map_err(|e| crate::Error::Syntax { msg: e.msg, filename: filename.into(), line: e.line })
    }

    pub fn syntax_error(&self, e: crate::Error) -> PyErr {
        match e {
            crate::Error::Syntax { msg, filename, line } => {
                let cls = self.types.exc("SyntaxError");
                let m = self.types.new_string(msg);
                let exc = self.new_exc(&cls, vec![m.clone()]);
                crate::ops::set_raw_attr(self, &exc, "msg", m);
                let fname = self.types.new_string(filename);
                crate::ops::set_raw_attr(self, &exc, "filename", fname);
                crate::ops::set_raw_attr(self, &exc, "lineno", Value::Int(line as i64));
                PyErr::Exc(exc)
            }
            crate::Error::Runtime(m) => self.runtime_error(m),
        }
    }

    /// New module object registered in sys.modules.
    pub fn new_module(&mut self, name: &str) -> ObjRef {
        let ns = self.types.new_dict_obj(Dict::new());
        let m = new_obj(self.types.module.clone(), None, Payload::Module(name.into(), ns));
        {
            let mut d = m.attr_table().unwrap().borrow_mut();
            d.set_str(self.types.intern("__name__"), self.types.new_str(name));
            d.set_str(self.types.intern("__doc__"), Value::None);
        }
        let key = self.types.new_str(name);
        self.modules.dict_set_raw(&self.types, key, Value::Obj(m.clone()));
        m
    }

    /// A module whose namespace is `ns` (an existing dict, which may be an
    /// instance of a dict subclass), registered in `sys.modules`.
    pub fn new_module_with_namespace(&mut self, name: &str, ns: ObjRef) -> ObjRef {
        let m = new_obj(self.types.module.clone(), None, Payload::Module(name.into(), ns));
        {
            let mut d = m.attr_table().unwrap().borrow_mut();
            if d.get_str("__name__").is_none() {
                d.set_str(self.types.intern("__name__"), self.types.new_str(name));
            }
        }
        let key = self.types.new_str(name);
        self.modules.dict_set_raw(&self.types, key, Value::Obj(m.clone()));
        m
    }

    pub fn register_native_module(&mut self, name: &'static str, init: ModuleInit) {
        self.native_modules.push((name, init));
    }

    pub fn ext_get<T: 'static>(&self) -> Option<std::cell::RefMut<'_, T>> {
        let b = self.ext.borrow_mut();
        let idx = b.iter().position(|x| x.is::<T>())?;
        Some(std::cell::RefMut::map(b, |v| v[idx].downcast_mut::<T>().unwrap()))
    }
    pub fn ext_set<T: 'static>(&self, v: T) {
        let mut b = self.ext.borrow_mut();
        if let Some(idx) = b.iter().position(|x| x.is::<T>()) {
            b[idx] = Box::new(v);
        } else {
            b.push(Box::new(v));
        }
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

/// Raw (no user `__eq__`) access helpers for dict-payload objects keyed by str.
impl PyObject {
    pub fn dict_get_str_raw(&self, key: &str) -> Option<Value> {
        match &self.payload {
            Payload::Dict(d) => d.borrow().get_str(key).cloned(),
            _ => self.attr_table()?.borrow().get_str(key).cloned(),
        }
    }
    pub fn dict_set_raw(&self, types: &Types, key: Value, v: Value) {
        let _ = types;
        match &self.payload {
            Payload::Dict(d) => d.borrow_mut().set_str(key, v),
            _ => {
                if let Some(d) = self.attr_table() {
                    d.borrow_mut().set_str(key, v)
                }
            }
        }
    }
    /// Namespace dict of a module/instance (`__dict__`) as raw Dict.
    pub fn ns(&self) -> Option<&RefCell<Dict>> {
        self.dict.as_ref()
    }
}

pub fn unary_dummy(_: UnaryOp, _: BinOp, _: CmpOp) {}
