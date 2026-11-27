// SPDX-License-Identifier: MPL-2.0
//! The Ren'Py runtime: Python environment, script load/init phases, and the
//! statement executor (`renpy/main.py`, `renpy/execution.py`,
//! `renpy/ast.py` execute methods).
//!
//! Python code runs as suspendable tasks. A native call that needs player
//! interaction (say, menu, pause, call_screen) suspends its task with a
//! `Request`; the engine keeps the task until the interaction finishes and
//! resumes it with the result. Control-flow exceptions raised by Python
//! (`renpy.game.JumpException`, …) are mapped to executor operations.

use crate::ast::*;
use crate::files::GameFiles;
use crate::lexer::PyExpr;
use crate::script::Script;
use crysirror_python::{ops, ObjRef, Payload, PyErr, PyResult, Task, TaskOutcome, Value, Vm};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Ren'Py release family selected from the installation.
#[derive(Clone, Debug)]
pub struct Profile {
    pub version: (u32, u32, u32),
    pub dialect: crysirror_python::Dialect,
}

impl Profile {
    pub fn parse_version(source: &str) -> Option<[u32; 4]> {
        let (_, tail) = source.split_once("version_tuple = (")?;
        let mut fields = tail.split(')').next()?.split(',');
        let mut version = [0; 4];
        for value in &mut version {
            *value = fields.next()?.trim().parse().ok()?;
        }
        Some(version)
    }

    pub fn from_version(version: [u32; 4]) -> Option<Self> {
        if version != [6, 99, 12, 4] { return None; }
        Some(Self { version: (version[0], version[1], version[2]),
                    dialect: crysirror_python::Dialect::Py2 })
    }
}

/// Something the script engine waits for while a Python task is suspended.
#[derive(Debug)]
pub enum Request {
    /// `ui.interact()`: run an interaction with the current screens.
    Interact(InteractRequest),
    /// `renpy.call_in_new_context(label, ...)`.
    CallInNewContext { label: String, args: Vec<Value>, kwargs: Vec<(Rc<str>, Value)> },
    /// `renpy.invoke_in_new_context(callable, ...)`.
    InvokeInNewContext { callable: Value, args: Vec<Value>, kwargs: Vec<(Rc<str>, Value)> },
}

#[derive(Clone, Debug)]
pub struct InteractRequest {
    pub suppress_overlay: bool,
    pub suppress_underlay: bool,
    pub mouse: Option<String>,
    pub type_: Option<String>,
    pub roll_forward: Value,
    pub pause: Option<f64>,
}

impl Default for InteractRequest {
    fn default() -> Self {
        Self { suppress_overlay: false, suppress_underlay: false, mouse: None, type_: None,
               roll_forward: Value::None, pause: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Interaction {
    Say { speaker: Option<String>, text: String, screen: Option<String> },
    Menu { caption: Option<String>, options: Vec<(String, i64)> },
    Pause { seconds: Option<f64>, hard: bool },
    Transition { duration_ms: u64 },
    Screen { name: String, timeout_ms: Option<u64>, suppress_overlay: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress { Running, Waiting, Finished }


/// One script execution context (`renpy.execution.Context`).
#[derive(Clone)]
pub struct Context {
    pub current: Option<NodeId>,
    pub next_node: Option<NodeId>,
    pub return_stack: Vec<Option<NodeId>>,
    pub call_location_stack: Vec<NodeId>,
    pub dynamic_stack: Vec<Vec<(String, Option<Value>)>>,
    pub rollback: bool,
    pub init_phase: bool,
    pub translate_language: Option<String>,
    pub translate_identifier: Option<String>,
    pub seen: bool,
    pub abnormal: bool,
    pub last_abnormal: bool,
    /// Python object exposed as `renpy.context()` / `renpy.game.context()`.
    pub py: Value,
    /// When this context was entered from a suspended task.
    pub return_value: Value,
    pub scene: crate::scene::SceneLists,
    pub(crate) screens: Vec<crate::screen_runtime::Instance>,
}

/// What the executor waits on.
#[derive(Clone)]
pub enum Wait {
    /// A suspended Python task and what to do with its result.
    Task { task: Task, then: Then },
    NestedCall { task: Task, then: Then, label: String, args: Vec<Value>, kwargs: Vec<(Rc<str>, Value)> },
    /// An interaction requested by `task`.
    Interaction { task: Task, then: Then, request: InteractRequest },
}

/// Continuation after a Python task completes.
#[derive(Clone)]
pub enum Then {
    /// Continue at the node's `next_node`.
    Node(NodeId),
    /// The node evaluates a value (e.g. `who`), then calls this step.
    Discard,
    /// The task was started by a nested context; its result is returned to
    /// the waiting task below.
    NestedReturn,
}

pub struct State {
    pub profile: Profile,
    pub files: Rc<dyn GameFiles>,
    pub script: RefCell<Script>,
    pub store: RefCell<HashMap<String, ObjRef>>,
    store_defaults: RefCell<Vec<(Value, Value)>>,
    pub code_cache: RefCell<HashMap<(usize, u8), Rc<crysirror_python::code::Code>>>,
    pub user_statements: RefCell<HashMap<Vec<String>, crate::statements::Definition>>,
    pub warpers: RefCell<HashSet<String>>,
    pub warper_functions: RefCell<HashMap<String, Value>>,
    pub sl2: RefCell<crate::sl2::Registry>,
    pub contexts: RefCell<Vec<Context>>,
    pub errors: RefCell<Vec<String>>,
    pub log: Rc<RefCell<Vec<String>>>,
    pub images: RefCell<HashMap<Vec<String>, Value>>,
    pub(crate) screen_refresh: Cell<bool>,
    pub(crate) clock_ms: Cell<u64>,
    pub screen_displayables: RefCell<HashMap<String, Value>>,
    pub screens: RefCell<HashMap<String, Rc<crate::sl2::Screen>>>,
    pub interaction: RefCell<Option<Interaction>>,
    pub audio_commands: RefCell<Vec<crate::audio::Command>>,
    pub(crate) rollback_blocked: Cell<bool>,
    pub(crate) rollback_suspended: Cell<bool>,
    pub(crate) rollback_available: Cell<usize>,
    pub(crate) in_rollback: Cell<bool>,
    pub(crate) roll_forward: RefCell<Value>,
    pub(crate) audio_loops: RefCell<std::collections::BTreeMap<u32, crate::audio::Command>>,
    pub(crate) audio_reset: RefCell<Option<std::collections::BTreeMap<u32, crate::audio::Command>>>,
    pub audio_playing: RefCell<std::collections::BTreeMap<u32, String>>,
}

pub fn state(vm: &Vm) -> Rc<State> {
    vm.ext_get::<Rc<State>>().expect("Ren'Py state missing from Python VM").clone()
}

pub fn try_state(vm: &Vm) -> Option<Rc<State>> {
    vm.ext_get::<Rc<State>>().map(|s| s.clone())
}

/// The ATL block whose source location is `file:line` in the loaded script
/// (resolves ATL references from saves).
pub fn find_atl(vm: &Vm, file: &str, line: u32) -> Option<Rc<crate::atl::RawBlock>> {
    let st = try_state(vm)?;
    let script = st.script.borrow();
    script.find_atl(file, line)
}

pub(crate) fn module_namespace(vm: &Vm, name: &str) -> Option<ObjRef> {
    let module = crysirror_python::import::get_module(vm, name)?;
    let module = module.obj()?;
    match &module.payload {
        Payload::Module(_, namespace) => Some(namespace.clone()),
        _ => module.attr_table().map(|_| module.clone()),
    }
}

pub struct Engine {
    pub vm: Vm,
    pub st: Rc<State>,
    pub waits: Vec<Wait>,
    pub halted: bool,
    pub(crate) cursor: Option<NodeId>,
    pub(crate) history: crate::rollback::History,
    nested: Vec<(Task, NodeId)>,
}

struct GlueHost {
    out: Rc<RefCell<Vec<String>>>,
    files: Rc<dyn GameFiles>,
    dialect: crysirror_python::Dialect,
}

impl GlueHost {
    fn module_path(&self, name: &str) -> Option<String> {
        if name == "renpy" || name.starts_with("renpy.") || name == "store" || name.starts_with("store.") {
            return None;
        }
        if name.len() > 512 || name.split('.').any(|part| part.is_empty()
            || !part.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_'))
        {
            return None;
        }
        let slash_name = name.replace('.', "/");
        for prefix in ["", "python-packages/"] {
            for suffix in [".py", "/__init__.py"] {
                let path = format!("{prefix}{slash_name}{suffix}");
                if self.files.exists(&path) { return Some(path); }
            }
        }
        None
    }
}

impl crysirror_python::Host for GlueHost {
    fn write_stdout(&mut self, s: &str) {
        let mut o = self.out.borrow_mut();
        o.push(s.to_string());
        if o.len() > 4096 {
            o.drain(..2048);
        }
    }
    fn find_module_source(&mut self, name: &str) -> Option<(String, String)> {
        if let Some(source) = crate::glue::source(name) {
            return Some((source.to_owned(), format!("<renpy glue {name}>")));
        }
        let path = self.module_path(name)?;
        let bytes = self.files.read(&path).or_else(|| self.files.common_read(&path))?;
        if bytes.len() > 4 * 1024 * 1024 { return None; }
        let source = std::str::from_utf8(&bytes).ok()?.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        Some((source, format!("game/{path}")))
    }
    fn module_is_package(&mut self, name: &str) -> bool {
        name == "renpy" || self.module_path(name).is_some_and(|path| path.ends_with("/__init__.py"))
    }
    fn module_dialect(&mut self, name: &str) -> Option<crysirror_python::Dialect> {
        Some(if crate::glue::source(name).is_some() { crysirror_python::Dialect::Py3 } else { self.dialect })
    }
    fn temporary_directory(&self) -> Option<&str> {
        self.files.temporary_storage().map(|_| "/tmp")
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if let Some(path) = virtual_temp_path(path) {
            let storage = self.files.temporary_storage()?;
            return storage.read(&path).ok();
        }
        let path = path.strip_prefix("/game/").or_else(|| path.strip_prefix("game/")).unwrap_or(path);
        self.files.read(path)
    }
    fn write_file(&mut self, path: &str, bytes: &[u8]) -> bool {
        virtual_temp_path(path).zip(self.files.temporary_storage())
            .is_some_and(|(path, storage)| storage.write_atomic(&path, bytes).is_ok())
    }
    fn remove_file(&mut self, path: &str) -> bool {
        virtual_temp_path(path).zip(self.files.temporary_storage())
            .is_some_and(|(path, storage)| storage.remove(&path).is_ok())
    }
    fn stat(&mut self, path: &str) -> Option<bool> {
        if matches!(path, "/tmp" | "/game" | "game" | "/") { return Some(true); }
        if let Some(path) = virtual_temp_path(path) {
            let storage = self.files.temporary_storage()?;
            return storage.metadata(path.mount(), path.relative()).ok()?.map(|m| m.is_directory);
        }
        let path = path.strip_prefix("/game/").or_else(|| path.strip_prefix("game/")).unwrap_or(path);
        self.files.exists(path).then_some(false)
    }
    fn try_lock_file(&mut self, path: &str) -> Result<Box<dyn std::any::Any>, String> {
        let path = virtual_temp_path(path).ok_or("locks require a virtual temporary file")?;
        let storage = self.files.temporary_storage().ok_or("temporary storage unavailable")?;
        // Hex encoding preserves distinct logical names and the lease API's
        // restricted alphabet without passing any ambient OS path.
        use std::fmt::Write;
        let mut key = String::from("python-");
        for byte in path.relative().bytes() {
            write!(key, "{byte:02x}").expect("write to String");
        }
        let lease = storage.acquire_lease(&key).map_err(|error| error.to_string())?;
        Ok(Box::new(lease))
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn virtual_temp_path(path: &str) -> Option<crysirror_core::path::VfsPath> {
    use crysirror_core::path::{Mount, VfsPath};
    let leaf = path.strip_prefix("/tmp/")?;
    if leaf.is_empty() || leaf.len() > 48 || leaf.contains(['/', '\\', ':']) || matches!(leaf, "." | "..") {
        return None;
    }
    VfsPath::new(Mount::Cache, &format!("python-tmp-{leaf}")).ok()
}

pub type EResult<T> = Result<T, String>;

impl Engine {
    pub fn new(files: Rc<dyn GameFiles>, profile: Profile) -> EResult<Engine> {
        let out = Rc::new(RefCell::new(Vec::new()));
        let mut vm = Vm::with_host(Box::new(GlueHost { out: out.clone(), files: files.clone(), dialect: profile.dialect }));
        crysirror_python::modules::sys::set_dialect(&mut vm, profile.dialect);
        let st = Rc::new(State {
            profile,
            files,
            script: RefCell::new(Script::default()),
            store: RefCell::new(HashMap::new()),
            store_defaults: RefCell::new(Vec::new()),
            code_cache: RefCell::new(HashMap::new()),
            user_statements: RefCell::new(HashMap::new()),
            warpers: RefCell::new(crate::atl::BUILTIN_WARPERS.iter().map(|s| s.to_string()).collect()),
            warper_functions: RefCell::new(HashMap::new()),
            sl2: RefCell::new(Default::default()),
            contexts: RefCell::new(Vec::new()),
            errors: RefCell::new(Vec::new()),
            log: out,
            images: RefCell::new(HashMap::new()),
            screen_refresh: Cell::new(false),
            clock_ms: Cell::new(0),
            screen_displayables: RefCell::new(HashMap::new()),
            screens: RefCell::new(HashMap::new()),
            interaction: RefCell::new(None),
            audio_commands: RefCell::new(Vec::new()),
            rollback_blocked: Cell::new(false),
            rollback_suspended: Cell::new(false),
            rollback_available: Cell::new(0),
            in_rollback: Cell::new(false),
            roll_forward: RefCell::new(Value::None),
            audio_loops: RefCell::new(Default::default()),
            audio_reset: RefCell::new(None),
            audio_playing: RefCell::new(Default::default()),
        });
        vm.ext_set(st.clone());
        crate::natives::install(&mut vm);
        let mut e = Engine { vm, st, waits: Vec::new(), halted: false, cursor: None, history: Default::default(), nested: Vec::new() };
        e.boot_python()?;
        Ok(e)
    }

    pub(crate) fn py_err(&mut self, e: PyErr) -> String {
        crate::rpyc::py_err(&mut self.vm, e)
    }

    /// Imports the glue `renpy` package and creates the stores.
    fn boot_python(&mut self) -> EResult<()> {
        self.vm.new_module("store");
        let v = &self.st.profile.version;
        let src = format!(
            "import renpy\nrenpy._bootstrap(({}, {}, {}), {:?})\n",
            v.0, v.1, v.2, std::env::consts::OS
        );
        crysirror_python::run_source(&mut self.vm, &src, crysirror_python::Dialect::Py3)?;
        crate::persistent::restore(self)?;
        let store = Value::Obj(self.store_dict("store"));
        let defaults = ops::mapping_items(&mut self.vm, &store).map_err(|err| self.py_err(err))?;
        self.st.store_defaults.replace(defaults);
        Ok(())
    }

    pub fn store_dict(&self, name: &str) -> ObjRef {
        if let Some(d) = self.st.store.borrow().get(name) {
            return d.clone();
        }
        module_namespace(&self.vm, name).unwrap_or_else(|| self.vm.builtins.clone())
    }

    pub fn ensure_definition_store(&mut self, name: &str) {
        let special = crysirror_python::import::get_module(&self.vm, "renpy.config")
            .and_then(|module| ops::get_raw_attr(module.obj()?, "special_namespaces"))
            .and_then(|value| value.obj()?.dict_get_str_raw(name)).is_some();
        if !special && !matches!(name, "store.config" | "store.persistent" | "store.preferences") {
            self.ensure_store(name);
        }
    }

    pub fn ensure_store(&mut self, name: &str) -> ObjRef {
        if self.st.store.borrow().contains_key(name) || name == "store" {
            return self.store_dict(name);
        }
        let Some((parent, leaf)) = name.rsplit_once('.') else {
            return self.store_dict("store");
        };
        let parent = self.ensure_store(parent);
        let module = self.vm.new_module(name);
        let Payload::Module(_, namespace) = &module.payload else { unreachable!() };
        let namespace = namespace.clone();
        for (key, value) in &*self.st.store_defaults.borrow() {
            namespace.dict_set_raw(&self.vm.types, key.clone(), value.clone());
        }
        namespace.dict_set_raw(&self.vm.types, self.vm.types.new_str("__name__"), self.vm.types.new_str(name));
        parent.dict_set_raw(&self.vm.types, self.vm.types.new_str(leaf), Value::Obj(module));
        self.st.store.borrow_mut().insert(name.to_owned(), namespace.clone());
        namespace
    }

    // ---- Python code ----

    fn compile(&mut self, id: NodeId, slot: u8, code: &PyCode) -> EResult<Rc<crysirror_python::code::Code>> {
        if let Some(c) = self.st.code_cache.borrow().get(&(id, slot)) {
            return Ok(c.clone());
        }
        let c = crate::pycompile::compile(&self.vm, &code.source, code.mode, self.dialect_for(code))?;
        self.st.code_cache.borrow_mut().insert((id, slot), c.clone());
        Ok(c)
    }

    fn dialect_for(&self, code: &PyCode) -> crysirror_python::Dialect {
        if code.py3 { crysirror_python::Dialect::Py3 } else { self.st.profile.dialect }
    }

    /// Evaluates an expression in the store (synchronously; no suspension).
    pub fn eval(&mut self, e: &PyExpr) -> EResult<Value> {
        let code = crate::pycompile::compile(&self.vm, e, "eval", self.st.profile.dialect)?;
        let g = self.store_dict("store");
        self.vm.run_code(code, g, None).map_err(|e| self.py_err(e))
    }

    pub fn eval_cached(&mut self, id: NodeId, slot: u8, e: &PyExpr) -> PyResult<Value> {
        let pc = PyCode { source: e.clone(), mode: "eval", py3: false };
        let code = self.compile(id, slot, &pc).map_err(|m| self.vm.syntax_error(crysirror_python::Error::Runtime(m)))?;
        let g = self.store_dict("store");
        self.vm.run_code(code, g, None)
    }

    // ---- load + init ----

    /// Loads every script file; runs early python as files load.
    pub fn load_script(&mut self) -> EResult<()> {
        let modules = crate::script::bootstrap_modules(&*self.st.files)?;
        let files = crate::script::discover(&*self.st.files)?;
        self.load_files(modules, files)
    }

    pub(crate) fn load_files(
        &mut self, modules: Vec<crate::script::ScriptFile>, files: Vec<crate::script::ScriptFile>,
    ) -> EResult<()> {
        let mut shells = crate::rpyc::Shells::new(&mut self.vm).map_err(|e| e.to_string())?;
        if !modules.is_empty() {
            for file in &modules {
                self.load_file(&mut shells, file)?;
            }
            self.st.script.borrow_mut().finish();
            let init = std::mem::take(&mut self.st.script.borrow_mut().initcode);
            self.run_init_nodes(init)?;
            self.call_glue("_init_system_styles", Vec::new())?;
        }
        for file in &files {
            self.load_file(&mut shells, file)?;
        }
        self.st.script.borrow_mut().finish();
        Ok(())
    }

    fn load_file(&mut self, shells: &mut crate::rpyc::Shells, file: &crate::script::ScriptFile) -> EResult<()> {
        let registry = std::mem::take(&mut *self.st.sl2.borrow_mut());
        let cx = crate::natives::ParseCx { st: self.st.clone(), registry };
        let stmts = self.st.script.borrow_mut().load_file(&mut self.vm, shells, &cx, &cx.registry, file);
        *self.st.sl2.borrow_mut() = cx.registry;
        let errors = self.st.script.borrow().errors.join("\n");
        if !errors.is_empty() { return Err(errors); }
        let early = self.st.script.borrow_mut().register(&stmts);
        for id in early {
            self.run_early(id)?;
        }
        Ok(())
    }

    fn run_early(&mut self, id: NodeId) -> EResult<()> {
        let kind = self.st.script.borrow().arena.get(id).kind.clone();
        match kind {
            NodeKind::EarlyPython { code, hide, store } => self.exec_python_sync(id, &code, hide, &store),
            NodeKind::TranslateEarlyBlock { block, .. } => {
                for b in block {
                    self.run_early(b)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn exec_python_sync(&mut self, id: NodeId, code: &PyCode, hide: bool, store: &str) -> EResult<()> {
        let c = self.compile(id, 0, code)?;
        let g = self.ensure_store(store);
        let locals = if hide { Some(self.vm.types.new_dict()) } else { None };
        match self.vm.start_code_task(c, g, locals) {
            TaskOutcome::Done(_) => Ok(()),
            TaskOutcome::Error(e) => {
                let n = self.st.script.borrow().arena.get(id).clone();
                Err(format!("{}:{}: {}", n.filename, n.linenumber, crysirror_python::format_exception(&mut self.vm, &e)))
            }
            TaskOutcome::Suspended(..) => Err("interaction requested during script load".into()),
        }
    }

    /// Runs the sorted init code (`for prio, node in initcode: run(node)`).
    pub fn run_init(&mut self) -> EResult<()> {
        let init = self.st.script.borrow().initcode.clone();
        self.run_init_nodes(init)?;
        self.call_glue("_post_init", Vec::new())?;
        Ok(())
    }

    fn run_init_nodes(&mut self, init: Vec<(i32, NodeId)>) -> EResult<()> {
        self.push_context(false, true)?;
        let result = self.run_init_context(init);
        self.st.contexts.borrow_mut().pop();
        result
    }

    fn run_init_context(&mut self, init: Vec<(i32, NodeId)>) -> EResult<()> {
        for (_, id) in init {
            let first = {
                let script = self.st.script.borrow();
                match &script.arena.get(id).kind {
                    NodeKind::Init { block, .. } => block.first().copied(),
                    _ => Some(id),
                }
            };
            if let Some(first) = first {
                self.run_to_end(first)?;
            }
        }
        Ok(())
    }

    /// Executes from `id` until the chain ends (init phase: no interaction).
    fn run_to_end(&mut self, id: NodeId) -> EResult<()> {
        let mut node = Some(id);
        let mut steps = 0u64;
        while let Some(n) = node {
            steps += 1;
            if steps > 10_000_000 {
                return Err("init code did not terminate".into());
            }
            match self.execute(n) {
                Ok(Flow::Next(next)) => node = next,
                Ok(Flow::Jump(t)) => {
                    self.mark_transfer();
                    node = Some(t);
                }
                Ok(Flow::Wait(_)) => {
                    let nd = self.st.script.borrow().arena.get(n).clone();
                    return Err(format!("{}:{}: interaction during init", nd.filename, nd.linenumber));
                }
                Ok(Flow::Halt) => return Ok(()),
                Ok(Flow::Rewind(_)) => return Err("rollback during Ren'Py initialization".into()),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn call_glue(&mut self, name: &str, args: Vec<Value>) -> EResult<Value> {
        let m = crysirror_python::import::import_module(&mut self.vm, "renpy").map_err(|e| self.py_err(e))?;
        let f = ops::getattr(&mut self.vm, &m, name).map_err(|e| self.py_err(e))?;
        self.vm.call(&f, args, Vec::new()).map_err(|e| self.py_err(e))
    }

    pub fn push_context(&mut self, rollback: bool, init: bool) -> EResult<()> {
        let py = self.call_glue("_new_context_object", vec![Value::Bool(rollback), Value::Bool(init)])?;
        let scene = match self.st.contexts.borrow().last() {
            Some(c) => c.scene.clone_for_new_context(),
            None => crate::scene::SceneLists::default(),
        };
        self.st.contexts.borrow_mut().push(Context {
            current: None,
            next_node: None,
            return_stack: Vec::new(),
            call_location_stack: Vec::new(),
            dynamic_stack: vec![Vec::new(), Vec::new()],
            rollback,
            init_phase: init,
            translate_language: None,
            translate_identifier: None,
            seen: false,
            abnormal: true,
            last_abnormal: false,
            py,
            return_value: Value::None,
            scene,
            screens: Vec::new(),
        });
        Ok(())
    }

    // ---- statement execution ----

    pub fn start(&mut self, label: &str) -> EResult<()> {
        let id = self.lookup_label(label)?;
        self.push_context(true, false)?;
        if label != "_start" { self.call_glue("execute_default_statement", vec![Value::Bool(true)])?; }
        self.cursor = Some(id);
        self.halted = false;
        Ok(())
    }

    pub fn interaction(&self) -> Option<Interaction> {
        self.st.interaction.borrow().clone()
    }

    pub fn resume(&mut self, choice: Option<usize>) -> EResult<()> {
        self.finish_interaction(choice, false, false)
    }

    pub fn finish_pause_timeout(&mut self) -> EResult<()> {
        self.finish_interaction(None, true, false)
    }

    pub(crate) fn finish_interaction(&mut self, choice: Option<usize>, timeout: bool, replay: bool) -> EResult<()> {
        let value = match self.interaction() {
            Some(Interaction::Menu { options, .. }) if !timeout => {
                let index = choice.ok_or("a menu requires a choice")?;
                Value::Int(options.get(index).ok_or("invalid menu choice")?.1)
            }
            Some(Interaction::Pause { seconds, hard }) if choice.is_none() => {
                if timeout && seconds.is_none() || !timeout && hard {
                    return Err("pause cannot complete from this event".into());
                }
                Value::Bool(!timeout)
            }
            Some(Interaction::Transition { .. }) if choice.is_none() => Value::Bool(!timeout),
            Some(Interaction::Screen { timeout_ms: Some(_), .. }) if choice.is_none() && timeout => Value::None,
            Some(Interaction::Say { .. }) if choice.is_none() && !timeout => Value::None,
            _ => return Err("no matching Ren'Py interaction".into()),
        };
        self.remember_decision(choice, timeout, value.clone(), replay);
        self.resume_value(value)
    }

    fn resume_value(&mut self, value: Value) -> EResult<()> {
        let Some(Wait::Interaction { task, then: Then::Node(id), .. }) = self.waits.pop() else {
            return Err("interaction without a task continuation".into());
        };
        if matches!(self.interaction(), Some(Interaction::Screen { .. } | Interaction::Say { screen: Some(_), .. })) {
            if let Some(context) = self.st.contexts.borrow_mut().last_mut() {
                context.screens.retain(|screen| !screen.transient);
            }
        }
        self.st.interaction.replace(None);
        let flow = self.resume_task(task, id, Ok(value))?;
        self.advance(flow)
    }

    fn resume_task(&mut self, task: Task, id: NodeId, result: Result<Value, ObjRef>) -> EResult<Flow> {
        Ok(match self.vm.resume_task(task, result) {
            TaskOutcome::Done(value) => self.complete_task(id, value),
            TaskOutcome::Error(error) => self.control_flow(id, PyErr::Exc(error))?,
            TaskOutcome::Suspended(task, request) => {
                Flow::Wait(Box::new(crate::exec::suspended(self, task, request, Then::Node(id))))
            }
        })
    }

    fn pop_nested_context(&mut self) -> Option<(Task, NodeId)> {
        let suspended = self.nested.pop()?;
        let child = self.st.contexts.borrow_mut().pop().expect("nested context");
        crate::exec::restore_scope(self, child.dynamic_stack.into_iter().flatten().rev());
        Some(suspended)
    }

    pub(crate) fn invoke_screen_action(&mut self, action: Value) -> EResult<()> {
        let module = crysirror_python::import::import_module(&mut self.vm, "renpy").map_err(|e| self.py_err(e))?;
        let invoke = ops::getattr(&mut self.vm, &module, "_invoke_screen_action").map_err(|e| self.py_err(e))?;
        match self.vm.call1(&invoke, action) {
            Ok(value) if !value.is_none() && matches!(self.interaction(), Some(Interaction::Screen { .. })) => {
                self.resume_value(value)
            }
            Ok(_) => Ok(()),
            Err(PyErr::Exc(error)) if matches!(error.class_name().as_str(), "JumpException" | "JumpOutException" | "CallException" | "RollbackException") => {
                let id = self.cursor.ok_or("screen action outside execution")?;
                self.st.interaction.replace(None);
                self.waits.clear();
                if let Some(context) = self.st.contexts.borrow_mut().last_mut() {
                    context.screens.retain(|screen| !screen.transient);
                }
                let flow = self.control_flow(id, PyErr::Exc(error))?;
                self.advance(flow)
            }
            Err(error) => Err(self.py_err(error)),
        }
    }

    pub fn tick(&mut self, budget: usize) -> EResult<Progress> {
        for _ in 0..budget {
            if !self.waits.is_empty() { return Ok(Progress::Waiting); }
            let Some(id) = self.cursor else { return Ok(Progress::Finished); };
            let flow = self.execute(id)?;
            self.advance(flow)?;
        }
        Ok(Progress::Running)
    }

    pub(crate) fn complete_task(&self, id: NodeId, result: Value) -> Flow {
        let script = self.st.script.borrow();
        let node = script.arena.get(id);
        if let (NodeKind::Menu { items, .. }, Value::Int(choice)) = (&node.kind, result) {
            let selected = items.get(choice as usize)
                .and_then(|item| item.block.as_ref())
                .and_then(|block| block.first()).copied();
            return Flow::Next(selected.or(node.next));
        }
        Flow::Next(self.next_node(node.next))
    }

    fn advance(&mut self, flow: Flow) -> EResult<()> {
        if self.st.rollback_blocked.replace(false) { self.reset_history(); }
        match flow {
            Flow::Next(Some(next)) => self.cursor = Some(next),
            Flow::Next(None) | Flow::Halt if !self.nested.is_empty() => {
                let value = self.store_dict("store").dict_get_str_raw("_return").unwrap_or(Value::None);
                let (parent, id) = self.pop_nested_context().expect("nested context");
                let flow = self.resume_task(parent, id, Ok(value))?;
                return self.advance(flow);
            }
            Flow::Next(None) => self.cursor = None,
            Flow::Jump(id) => {
                self.mark_transfer();
                self.cursor = Some(id);
            }
            Flow::Wait(wait) => match *wait {
                Wait::Interaction { .. } => {
                    self.waits.push(*wait);
                    self.push_checkpoint();
                },
                Wait::Task { .. } => return Err("unsupported Ren'Py task request".into()),
                Wait::NestedCall { task, then: Then::Node(id), label, args, kwargs } => {
                    if self.nested.len() >= 32 { return Err("Ren'Py context depth limit".into()); }
                    let label_id = self.lookup_label(&label)?;
                    self.push_context(false, false)?;
                    let store = self.store_dict("store");
                    let scope = ["_args", "_kwargs", "_return"].into_iter()
                        .map(|name| (name.to_owned(), store.dict_get_str_raw(name)))
                        .collect();
                    self.st.contexts.borrow_mut().last_mut().unwrap().dynamic_stack[0] = scope;
                    let named = self.vm.types.new_dict();
                    for (name, value) in kwargs {
                        named.obj().unwrap().dict_set_raw(&self.vm.types, self.vm.types.new_str(&name), value);
                    }
                    self.set_store("_args", self.vm.types.new_tuple(args));
                    self.set_store("_kwargs", named);
                    self.set_store("_return", Value::None);
                    self.nested.push((task, id));
                    self.cursor = Some(label_id);
                }
                Wait::NestedCall { .. } => return Err("nested Ren'Py context without a script continuation".into()),
            },
            Flow::Rewind(count) => {
                if !self.rewind(count) { return Err("Ren'Py rollback target is unavailable".into()); }
            }
            Flow::Halt => {
                self.cursor = None;
                self.halted = true;
            }
        }
        Ok(())
    }

    fn mark_transfer(&self) {
        if let Some(context) = self.st.contexts.borrow_mut().last_mut() {
            context.abnormal = true;
        }
    }

    /// Executes one node.
    pub fn execute(&mut self, id: NodeId) -> EResult<Flow> {
        let node = self.st.script.borrow().arena.get(id).clone();
        if let Some(c) = self.st.contexts.borrow_mut().last_mut() {
            c.current = Some(id);
            c.next_node = node.next;
            c.last_abnormal = std::mem::take(&mut c.abnormal);
        }
        let r = crate::exec::execute(self, id, &node);
        match r {
            Err(ExecError::Python(e)) => self.control_flow(id, e),
            Err(ExecError::Msg(m)) => Err(format!("{}:{}: {}", node.filename, node.linenumber, m)),
            Ok(f) => Ok(f),
        }
    }

    /// Maps a Python exception raised while executing `id` to control flow.
    pub fn control_flow(&mut self, id: NodeId, e: PyErr) -> EResult<Flow> {
        let PyErr::Exc(exc) = e else { return Err("suspension outside a task".into()) };
        let cls = exc.class_name();
        let args = crysirror_python::format::exc_args(&exc);
        match cls.as_str() {
            "RollbackException" => {
                let count = match args.first() {
                    Some(Value::Int(count)) => usize::try_from(*count).map_err(|_| "invalid rollback count")?,
                    _ => return Err("invalid rollback count".into()),
                };
                Ok(Flow::Rewind(count))
            }
            "JumpException" => {
                let label = args.first().map(|a| ops::str(&mut self.vm, a).unwrap_or_default()).unwrap_or_default();
                let t = self.lookup_label(&label)?;
                Ok(Flow::Jump(t))
            }
            "JumpOutException" => {
                let label = args.first().ok_or("context jump requires a label")?.clone();
                if self.nested.is_empty() {
                    return Err("context jump requires a suspended parent".into());
                }
                let jump = self.call_glue("_parent_jump", vec![label])?;
                let exception = jump.obj().ok_or("invalid parent jump exception")?.clone();
                let (task, origin) = self.pop_nested_context().expect("nested context");
                self.resume_task(task, origin, Err(exception))
            }
            "CallException" => {
                let label = ops::get_raw_attr(&exc, "label").map(|a| ops::str(&mut self.vm, &a).unwrap_or_default()).unwrap_or_default();
                let next = self.st.script.borrow().arena.get(id).next;
                let Some(ret) = next else { return Err("renpy.call can't be used when the next node is undefined.".into()) };
                let a = ops::get_raw_attr(&exc, "args").unwrap_or(Value::None);
                let k = ops::get_raw_attr(&exc, "kwargs").unwrap_or(Value::None);
                let target = self.call_label(&label, Some(ret))?;
                self.set_store("_args", a);
                self.set_store("_kwargs", k);
                Ok(Flow::Jump(target))
            }
            _ => {
                let node = self.st.script.borrow().arena.get(id).clone();
                Err(format!("{}:{}: {}", node.filename, node.linenumber, crysirror_python::format_exception(&mut self.vm, &exc)))
            }
        }
    }

    pub fn lookup_label(&self, label: &str) -> EResult<NodeId> {
        self.st.script.borrow().lookup_label(label).ok_or_else(|| format!("could not find label '{label}'"))
    }

    pub fn set_store(&mut self, name: &str, v: Value) {
        let g = self.store_dict("store");
        g.dict_set_raw(&self.vm.types, self.vm.types.intern(name), v);
    }

    /// `Context.call`: pushes the return site and a dynamic scope.
    pub fn call_label(&mut self, label: &str, return_site: Option<NodeId>) -> EResult<NodeId> {
        let target = self.lookup_label(label)?;
        let mut cs = self.st.contexts.borrow_mut();
        let c = cs.last_mut().unwrap();
        c.return_stack.push(return_site);
        if let Some(cur) = c.current {
            c.call_location_stack.push(cur);
        }
        c.dynamic_stack.push(Vec::new());
        Ok(target)
    }
}

pub enum Flow {
    Next(Option<NodeId>),
    Jump(NodeId),
    Rewind(usize),
    Wait(Box<Wait>),
    Halt,
}

pub enum ExecError {
    Python(PyErr),
    Msg(String),
}

impl From<PyErr> for ExecError {
    fn from(e: PyErr) -> Self {
        ExecError::Python(e)
    }
}

impl From<String> for ExecError {
    fn from(e: String) -> Self {
        ExecError::Msg(e)
    }
}
