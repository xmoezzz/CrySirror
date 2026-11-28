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
    /// `config.basedir` ("/") with the game's own changes.
    base: super::basedir::BaseDir,
}

impl RtHost {
    fn game_module_path(&self, name: &str) -> Option<String> {
        let base = name.replace('.', "/");
        let list = self.files.list();
        for prefix in &self.prefixes {
            for cand in [
                format!("{prefix}{base}.py"),
                format!("{prefix}{base}/__init__.py"),
            ] {
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
        let src = std::str::from_utf8(&bytes)
            .ok()?
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n");
        Some((src, format!("game/{path}")))
    }

    fn module_is_package(&mut self, name: &str) -> bool {
        crate::glue::rt_is_package(name)
            || self
                .game_module_path(name)
                .is_some_and(|p| p.ends_with("/__init__.py"))
    }

    fn module_dialect(&mut self, name: &str) -> Option<Dialect> {
        Some(if crate::glue::rt_source(name).is_some() {
            Dialect::Py3
        } else {
            self.dialect
        })
    }

    fn temporary_directory(&self) -> Option<&str> {
        self.files.temporary_storage().map(|_| "/tmp")
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if let Some(path) = virtual_temp_path(path) {
            let storage = self.files.temporary_storage()?;
            return storage.read(&path).ok();
        }
        let rel = super::basedir::relative(path)?;
        if let Some(d) = self.base.read(&rel) {
            return Some(d);
        }
        if self.base.changed(&rel) {
            return None;
        }
        // Game files, also inside archives, by their game-relative name.
        self.files.read(rel.strip_prefix("game/").unwrap_or(&rel))
    }

    fn write_file(&mut self, path: &str, data: &[u8]) -> bool {
        if let Some(path) = virtual_temp_path(path) {
            return self
                .files
                .temporary_storage()
                .is_some_and(|storage| storage.write_atomic(&path, data).is_ok());
        }
        super::basedir::relative(path).is_some_and(|rel| self.base.write(&rel, data))
    }

    fn remove_file(&mut self, path: &str) -> bool {
        if let Some(path) = virtual_temp_path(path) {
            return self
                .files
                .temporary_storage()
                .is_some_and(|storage| storage.remove(&path).is_ok());
        }
        super::basedir::relative(path).is_some_and(|rel| self.base.remove(&rel))
    }

    fn rename(&mut self, from: &str, to: &str) -> bool {
        if virtual_temp_path(from).is_some() || virtual_temp_path(to).is_some() {
            return false;
        }
        let (Some(f), Some(t)) = (super::basedir::relative(from), super::basedir::relative(to)) else {
            return false;
        };
        match self.base.read(&f) {
            Some(d) => self.base.write(&t, &d) && self.base.remove(&f),
            None => false,
        }
    }

    fn make_dir(&mut self, path: &str) -> bool {
        // Overlay directories exist implicitly once a file is written.
        super::basedir::relative(path).is_some()
    }

    fn list_dir(&mut self, path: &str) -> Option<Vec<String>> {
        let rel = super::basedir::relative(path).unwrap_or_default();
        self.base.list_dir(&rel)
    }

    fn stat(&mut self, path: &str) -> Option<bool> {
        if matches!(path, "/tmp" | "/game" | "game" | "/") {
            return Some(true);
        }
        if let Some(path) = virtual_temp_path(path) {
            let storage = self.files.temporary_storage()?;
            return storage
                .metadata(path.mount(), path.relative())
                .ok()?
                .map(|m| m.is_directory);
        }
        let rel = super::basedir::relative(path)?;
        if let Some(k) = self.base.stat(&rel) {
            return Some(k);
        }
        if self.base.changed(&rel) {
            return None;
        }
        let p = rel.strip_prefix("game/").unwrap_or(&rel);
        if self.files.read(p).is_some() {
            return Some(false);
        }
        let prefix = format!("{}/", p.trim_end_matches('/'));
        self.files
            .list()
            .iter()
            .any(|n| n.starts_with(&prefix))
            .then_some(true)
    }

    fn try_lock_file(&mut self, path: &str) -> Result<Box<dyn Any>, String> {
        let path = virtual_temp_path(path).ok_or("locks require a virtual temporary file")?;
        let storage = self
            .files
            .temporary_storage()
            .ok_or("temporary storage unavailable")?;
        use std::fmt::Write;
        let mut key = String::from("python-");
        for byte in path.relative().bytes() {
            write!(key, "{byte:02x}").expect("write to String");
        }
        let lease = storage
            .acquire_lease(&key)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(lease))
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

fn virtual_temp_path(path: &str) -> Option<crysirror_core::path::VfsPath> {
    use crysirror_core::path::{Mount, VfsPath};
    let leaf = path.strip_prefix("/tmp/")?;
    if leaf.is_empty()
        || leaf.len() > 48
        || leaf.contains(['/', '\\', ':'])
        || matches!(leaf, "." | "..")
    {
        return None;
    }
    VfsPath::new(Mount::Cache, &format!("python-tmp-{leaf}")).ok()
}

/// A base-directory file the game changed: Some(contents or None if it
/// was removed); None when the installed files decide.
pub fn changed_base_file(vm: &mut Vm, rel: &str) -> Option<Option<Vec<u8>>> {
    let h = vm.host.as_any().downcast_mut::<RtHost>()?;
    h.base.changed(rel).then(|| h.base.read(rel))
}

/// A base-directory file with the game's changes ("characters/x.chr").
pub fn base_file(vm: &mut Vm, rel: &str) -> Option<Vec<u8>> {
    vm.host.as_any().downcast_mut::<RtHost>()?.base.read(rel)
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
    let host = RtHost {
        files: files.clone(),
        dialect,
        stdout: Vec::new(),
        prefixes: vec![String::new(), "python-packages/".into()],
        base: super::basedir::BaseDir::new(files.clone()),
    };
    let mut vm = Vm::with_host(Box::new(host));
    crysirror_python::modules::sys::set_dialect(&mut vm, dialect);
    state::install(
        &mut vm,
        Rt {
            profile,
            script: RefCell::new(Script::default()),
            registry: RefCell::new(Registry::default()),
            files,
            handles: RefCell::new(HashMap::new()),
            defaults: RefCell::new(Vec::new()),
        },
    );
    super::install(&mut vm).expect("native Ren'Py modules");
    vm
}

fn exc_is(vm: &mut Vm, e: &crysirror_python::PyErr, name: &str) -> bool {
    let crysirror_python::PyErr::Exc(o) = e else {
        return false;
    };
    match super::py::import_attr(vm, "renpy.game", name) {
        Ok(c) => {
            crysirror_python::ops::exc_matches(vm, &crysirror_python::Value::Obj(o.clone()), &c)
                .unwrap_or(false)
        }
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
        let code = vm
            .compile_exec(start, "<crysirror renpy bootstrap>", Dialect::Py3)
            .map_err(|e| vm.runtime_error(e.to_string()))?;
        let m = vm.new_module("__main__");
        let crysirror_python::Payload::Module(_, g) = &m.payload else {
            unreachable!()
        };
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
        let _ = vm.call(
            &rep,
            vec![crysirror_python::Value::Obj(o.clone())],
            Vec::new(),
        );
    }
    Ended::Exit(Err(text))
}

/// The script thread's body: runs the game until it quits, rebuilding the
/// VM on `renpy.utter_restart()` as upstream's bootstrap loop reloads
/// Ren'Py. `setup` fills the host services of each new VM.
pub fn run_session(
    profile: Profile,
    files: Arc<dyn Files>,
    link: super::link::Link,
    setup: &dyn Fn(&mut Vm),
) {
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
