// SPDX-License-Identifier: MPL-2.0
//! Construction of the script-thread VM: an interpreter host serving the
//! runtime's Python shells (`py/rt`) and the game's own Python modules, the
//! native Ren'Py modules and the runtime state.
use super::state::{self, Files, Profile, Rt};
use crate::{script::Script, sl2::Registry};
use crysirror_python::{Dialect, Vm};
use std::{any::Any, cell::RefCell, collections::HashMap, sync::Arc};

/// Interpreter host of the script thread. Engine shells compile as
/// Python 3; game modules (`game/**.py`) use the release's dialect.
pub struct RtHost {
    files: Arc<dyn Files>,
    dialect: Dialect,
    stdout: Vec<String>,
    /// `add_python_directory` prefixes, most recent first.
    prefixes: Vec<String>,
}

impl RtHost {
    fn game_module_path(&self, name: &str) -> Option<String> {
        let base = name.replace('.', "/");
        let list = self.files.list();
        for prefix in &self.prefixes {
            for cand in [format!("{prefix}{base}.py"), format!("{prefix}{base}/__init__.py")] {
                if list.iter().any(|n| *n == cand) {
                    return Some(cand);
                }
            }
        }
        None
    }
}

impl crysirror_python::Host for RtHost {
    fn write_stdout(&mut self, s: &str) {
        if self.stdout.len() < 4096 {
            self.stdout.push(s.to_owned());
        }
    }

    fn find_module_source(&mut self, name: &str) -> Option<(String, String)> {
        if let Some(src) = crate::glue::rt_source(name) {
            return Some((src.to_owned(), format!("<crysirror renpy {name}>")));
        }
        if name == "renpy" || name.starts_with("renpy.") {
            return None;
        }
        let path = self.game_module_path(name)?;
        let bytes = self.files.read(&path)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return None;
        }
        let src = std::str::from_utf8(&bytes).ok()?.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        Some((src, format!("game/{path}")))
    }

    fn module_is_package(&mut self, name: &str) -> bool {
        crate::glue::rt_is_package(name) || self.game_module_path(name).is_some_and(|p| p.ends_with("/__init__.py"))
    }

    fn module_dialect(&mut self, name: &str) -> Option<Dialect> {
        Some(if crate::glue::rt_source(name).is_some() { Dialect::Py3 } else { self.dialect })
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let p = path.trim_start_matches('/');
        let p = p.strip_prefix("game/").unwrap_or(p);
        self.files.read(p)
    }

    fn stat(&mut self, path: &str) -> Option<bool> {
        let p = path.trim_start_matches('/');
        let p = p.strip_prefix("game/").unwrap_or(p);
        if self.files.read(p).is_some() {
            return Some(false);
        }
        let prefix = format!("{}/", p.trim_end_matches('/'));
        self.files.list().iter().any(|n| n.starts_with(&prefix)).then_some(true)
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// `add_python_directory(path)`: searched before earlier prefixes.
pub fn add_python_prefix(vm: &mut Vm, prefix: String) {
    if let Some(h) = vm.host.as_any().downcast_mut::<RtHost>() {
        h.prefixes.retain(|p| *p != prefix);
        h.prefixes.insert(0, prefix);
    }
}

/// A VM with every native Ren'Py module registered and the runtime state
/// installed. Nothing is imported yet.
pub fn new_vm(profile: Profile, files: Arc<dyn Files>) -> Vm {
    let dialect = profile.dialect;
    // `init_importer`: "python-packages/" then "" (each inserted first).
    let host = RtHost { files: files.clone(), dialect, stdout: Vec::new(), prefixes: vec![String::new(), "python-packages/".into()] };
    let mut vm = Vm::with_host(Box::new(host));
    crysirror_python::modules::sys::set_dialect(&mut vm, dialect);
    state::install(&mut vm, Rt {
        profile,
        script: RefCell::new(Script::default()),
        registry: RefCell::new(Registry::default()),
        files,
        handles: RefCell::new(HashMap::new()),
        defaults: RefCell::new(Vec::new()),
    });
    super::install(&mut vm).expect("native Ren'Py modules");
    vm
}

fn exc_is(vm: &mut Vm, e: &crysirror_python::PyErr, name: &str) -> bool {
    let crysirror_python::PyErr::Exc(o) = e else { return false };
    match super::py::import_attr(vm, "renpy.game", name) {
        Ok(c) => crysirror_python::ops::exc_matches(vm, &crysirror_python::Value::Obj(o.clone()), &c).unwrap_or(false),
        Err(_) => false,
    }
}

/// How a session's VM ended.
enum Ended {
    Exit(Result<(), String>),
    UtterRestart,
}

/// One start of the engine on `vm`: upstream `bootstrap()` after
/// `import_all`, then `renpy.main.main()`.
fn run_once(vm: &mut Vm) -> Ended {
    let start = r#"
import renpy
renpy.import_all()
renpy.loader.init_importer()
renpy.game.args = renpy.arguments.bootstrap()
renpy.config.renpy_base = "/"
renpy.config.basedir = "/"
renpy.config.gamedir = "/game"
renpy.config.args = []
renpy.config.logdir = None
renpy.main.main()
"#;
    let r = (|| -> Result<(), crysirror_python::PyErr> {
        let code = vm.compile_exec(start, "<crysirror renpy bootstrap>", Dialect::Py3).map_err(|e| vm.runtime_error(e.to_string()))?;
        let m = vm.new_module("__main__");
        let crysirror_python::Payload::Module(_, g) = &m.payload else { unreachable!() };
        let g = g.clone();
        let b = crysirror_python::Value::Obj(vm.builtins_module.clone());
        g.dict_set_raw(&vm.types, vm.types.intern("__builtins__"), b);
        vm.run_code(code, g, None).map(|_| ())
    })();
    let err = match r {
        Ok(()) => return Ended::Exit(Ok(())),
        Err(e) => e,
    };
    if exc_is(vm, &err, "UtterRestartException") {
        return Ended::UtterRestart;
    }
    if exc_is(vm, &err, "QuitException") || exc_is(vm, &err, "ParseErrorException") {
        return Ended::Exit(Ok(()));
    }
    let crysirror_python::PyErr::Exc(o) = &err else {
        return Ended::Exit(Err("the script thread suspended outside a task".into()));
    };
    let text = crysirror_python::format_exception(vm, o);
    if let Ok(rep) = super::py::import_attr(vm, "renpy.error", "report_exception") {
        let _ = vm.call(&rep, vec![crysirror_python::Value::Obj(o.clone())], Vec::new());
    }
    Ended::Exit(Err(text))
}

/// The script thread's body: runs the game until it quits, rebuilding the
/// VM on `renpy.utter_restart()` as upstream's bootstrap loop reloads
/// Ren'Py. `setup` fills the host services of each new VM.
pub fn run_session(profile: Profile, files: Arc<dyn Files>, link: super::link::Link, setup: &dyn Fn(&mut Vm)) {
    let mut link = link;
    loop {
        let mut vm = new_vm(profile.clone(), files.clone());
        setup(&mut vm);
        super::link::install(&mut vm, link);
        let ended = run_once(&mut vm);
        match ended {
            Ended::UtterRestart => {
                let l = super::link::link(&vm).expect("installed link");
                link = l.borrow_mut().renew();
            }
            Ended::Exit(result) => {
                super::link::finish(&vm, result);
                return;
            }
        }
    }
}
