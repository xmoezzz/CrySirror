// SPDX-License-Identifier: MPL-2.0
//! Script-thread runtime state shared by the native Ren'Py modules: the
//! release profile, the loaded script arena, the screen-language statement
//! registry and the game's read-only files.
use crate::{ast::NodeId, script::Script, sl2::Registry};
use crysirror_python::{Dialect, ObjRef, Vm};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

/// The Ren'Py release a game was built with.
#[derive(Clone, Debug)]
pub struct Profile {
    pub version: [u32; 4],
    pub dialect: Dialect,
}

impl Profile {
    pub fn new(version: [u32; 4]) -> Profile {
        let dialect = if version[0] >= 8 {
            Dialect::Py3
        } else {
            Dialect::Py2
        };
        Profile { version, dialect }
    }

    /// The release of an installation from `renpy/__init__.py` and, when
    /// present, `renpy/vc_version.py`: 8.x reads `vc_version.version`
    /// ("8.3.7.25031702"); earlier releases write `version_tuple = (6, 99,
    /// 12, 4, vc_version)` or `(7, 4, 11, vc_version)`, whose build number
    /// is `vc_version` (0 without the file). In 7.x the first tuple is the
    /// Python 2 branch the release runs.
    pub fn detect(init_py: &str, vc_version_py: Option<&str>) -> Option<[u32; 4]> {
        let vc = vc_version_py.unwrap_or("");
        let assigned = |name: &str| -> Option<String> {
            vc.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == name)
                    .then(|| v.trim().trim_matches(|c| c == '\'' || c == '"').to_owned())
            })
        };
        let numbers = |s: &str| -> Option<[u32; 4]> {
            let mut v = [0u32; 4];
            let mut it = s.split(['.', '+']);
            for x in &mut v {
                *x = it.next()?.trim().parse().ok()?;
            }
            Some(v)
        };
        if let Some(v) = assigned("version").and_then(|v| numbers(&v)) {
            return Some(v);
        }
        // Through 6.11 the release is only a string: `version = "Ren'Py
        // 6.10.2d"`; a letter suffix orders the builds (a = 1).
        if !init_py.contains("version_tuple = (") {
            let (_, tail) = init_py.split_once("version = \"Ren'Py ")?;
            let s = tail.split('"').next()?;
            let mut v = [0u32; 4];
            let mut parts = s.split('.');
            v[0] = parts.next()?.parse().ok()?;
            v[1] = parts.next()?.parse().ok()?;
            let last = parts.next()?;
            let digits: String = last.chars().take_while(|c| c.is_ascii_digit()).collect();
            v[2] = digits.parse().ok()?;
            v[3] = last[digits.len()..].chars().next().map_or(0, |c| (c.to_ascii_lowercase() as u32).saturating_sub('a' as u32 - 1));
            return Some(v);
        }
        let (_, tail) = init_py.split_once("version_tuple = (")?;
        let items: Vec<&str> = tail.split(')').next()?.split(',').map(str::trim).collect();
        let mut v = [0u32; 4];
        let mut n = 0;
        for i in &items {
            match i.parse::<u32>() {
                Ok(x) if n < 4 => {
                    v[n] = x;
                    n += 1;
                }
                _ => break,
            }
        }
        if n < 3 {
            return None;
        }
        if n == 3 {
            v[3] = assigned("vc_version")
                .and_then(|x| x.parse().ok())
                .unwrap_or(0);
        }
        Some(v)
    }

    /// Releases this runtime implements: 6.7 through 8.6.
    pub fn supported(version: [u32; 4]) -> bool {
        let v = (version[0], version[1]);
        v >= (6, 7) && v <= (8, 6)
    }

    /// `renpy.version_tuple >= (major, minor)`.
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.version[0], self.version[1]) >= (major, minor)
    }

    /// At or after the exact build `v` (for changes inside a release line).
    pub fn at_build(&self, v: [u32; 4]) -> bool {
        self.version >= v
    }

    /// A feature released in 8.`py3` and its Python 2 twin 7.`py2`.
    pub fn since_pair(&self, py2: u32, py3: u32) -> bool {
        self.at_least(8, py3) || (self.at_least(7, py2) && !self.at_least(8, 0))
    }
}

/// The virtual screen size a game's sources set: `gui.init(w, h)` (the
/// 7.0+ GUI), else `config.screen_width` / `config.screen_height`
/// assignments (`define` or Python). Commented lines are ignored. None
/// when the sources do not say (compiled-only or archived scripts).
pub fn screen_size_hint<'a>(sources: impl IntoIterator<Item = &'a str>) -> Option<[i32; 2]> {
    fn int_after(s: &str) -> Option<i32> {
        let d: String = s
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        d.parse().ok()
    }
    let (mut gui, mut w, mut h) = (None, None, None);
    for src in sources {
        for line in src.lines() {
            let l = line.split('#').next().unwrap_or("").trim();
            if let Some(rest) = l.find("gui.init(").map(|i| &l[i + 9..]) {
                let mut parts = rest.split(',');
                if let (Some(a), Some(b)) = (
                    parts.next().and_then(int_after),
                    parts.next().and_then(int_after),
                ) {
                    gui = Some([a, b]);
                }
            }
            for (name, slot) in [
                ("config.screen_width", &mut w),
                ("config.screen_height", &mut h),
            ] {
                if let Some(i) = l.find(name) {
                    let rest = l[i + name.len()..].trim_start();
                    if let Some(v) = rest
                        .strip_prefix('=')
                        .filter(|r| !r.starts_with('='))
                        .and_then(int_after)
                    {
                        *slot = Some(v);
                    }
                }
            }
        }
    }
    gui.or(match (w, h) {
        (Some(w), Some(h)) => Some([w, h]),
        _ => None,
    })
    .filter(|s| s.iter().all(|v| (240..=16384).contains(v)))
}

/// Read-only game files usable from the script thread.
pub trait Files: Send + Sync {
    fn list(&self) -> Vec<String>;
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    fn common_list(&self) -> Vec<String>;
    fn common_read(&self, name: &str) -> Option<Vec<u8>>;
    fn temporary_storage(&self) -> Option<&dyn crysirror_core::service::ClientStorage> {
        None
    }
}

pub struct Rt {
    pub profile: Profile,
    pub script: RefCell<Script>,
    pub registry: RefCell<Registry>,
    pub files: Arc<dyn Files>,
    /// Python handle of each node, so identity comparisons hold.
    pub handles: RefCell<HashMap<NodeId, ObjRef>>,
    /// Values of `define`/`default` statements by store, for lint and
    /// `renpy.store` introspection.
    pub defaults: RefCell<Vec<(String, String, NodeId)>>,
}

pub fn rt(vm: &Vm) -> Rc<Rt> {
    vm.ext_get::<Rc<Rt>>()
        .expect("Ren'Py runtime state")
        .clone()
}

pub fn try_rt(vm: &Vm) -> Option<Rc<Rt>> {
    vm.ext_get::<Rc<Rt>>().map(|r| r.clone())
}

pub fn install(vm: &mut Vm, rt: Rt) -> Rc<Rt> {
    let rc = Rc::new(rt);
    vm.ext_set(rc.clone());
    rc
}

pub fn dialect(vm: &Vm) -> Dialect {
    try_rt(vm)
        .map(|r| r.profile.dialect)
        .or_else(|| crate::engine::try_state(vm).map(|s| s.profile.dialect))
        .unwrap_or(Dialect::Py3)
}

/// The statement registry: the new runtime's, or the legacy engine's.
pub fn statement(vm: &Vm, name: &str) -> Option<crate::sl2::StatementSpec> {
    if let Some(r) = try_rt(vm) {
        return r.registry.borrow().get(name).cloned();
    }
    crate::engine::try_state(vm).and_then(|s| s.sl2.borrow().get(name).cloned())
}

/// Generated `renpy._profiles` per release line (tools/extract_config.py).
/// 7.5–7.8 are the Python 2 twins of 8.0–8.3.
const CONFIG_PROFILES: &[(u32, u32, &str)] = &[
    (6, 7, "config_6_7"),
    (6, 9, "config_6_9"),
    (6, 10, "config_6_10"),
    (6, 11, "config_6_11"),
    (6, 12, "config_6_12"),
    (6, 13, "config_6_13"),
    (6, 14, "config_6_14"),
    (6, 15, "config_6_15"),
    (6, 16, "config_6_16"),
    (6, 17, "config_6_17"),
    (6, 18, "config_6_18"),
    (6, 99, "config_6_99"),
    (7, 0, "config_7_0"),
    (7, 1, "config_7_1"),
    (7, 2, "config_7_2"),
    (7, 3, "config_7_3"),
    (7, 4, "config_7_4"),
    (7, 5, "config_7_5"),
    (7, 6, "config_7_6"),
    (7, 7, "config_7_7"),
    (7, 8, "config_7_8"),
    (8, 0, "config_8_0"),
    (8, 1, "config_8_1"),
    (8, 2, "config_8_2"),
    (8, 3, "config_8_3"),
    (8, 4, "config_8_4"),
    (8, 5, "config_8_5"),
    (8, 6, "config_8_6"),
];

impl Profile {
    /// The generated `renpy._profiles` configuration nearest at or below
    /// this release.
    pub fn config_profile(&self) -> &'static str {
        let v = (self.version[0], self.version[1]);
        CONFIG_PROFILES
            .iter()
            .rev()
            .find(|(a, b, _)| (*a, *b) <= v)
            .map(|(_, _, n)| *n)
            .unwrap_or(CONFIG_PROFILES[0].2)
    }
}

fn host_version(
    vm: &mut Vm,
    _: &[crysirror_python::Value],
    _: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    let v = try_rt(vm)
        .map(|r| r.profile.version)
        .unwrap_or([8, 3, 0, 0]);
    let items = v
        .iter()
        .map(|x| crysirror_python::Value::Int(*x as i64))
        .collect();
    Ok(vm.types.new_tuple(items))
}

fn host_config_profile(
    vm: &mut Vm,
    _: &[crysirror_python::Value],
    _: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    let p = try_rt(vm)
        .map(|r| r.profile.config_profile())
        .unwrap_or("config_8_3");
    Ok(vm.types.new_str(p))
}

fn host_py2(
    vm: &mut Vm,
    _: &[crysirror_python::Value],
    _: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    Ok(crysirror_python::Value::Bool(matches!(
        dialect(vm),
        Dialect::Py2
    )))
}

/// Screen variants of the host, most specific first (`choose_variants`).
fn host_variants(
    vm: &mut Vm,
    _: &[crysirror_python::Value],
    _: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    let vs = super::host(vm).borrow().variants.clone();
    let items = vs.iter().map(|v| vm.types.new_str(v)).collect();
    Ok(vm.types.new_list(items))
}

/// `config.savedir` when the game sets none: saves live in the client's
/// Save mount, named by this path only in diagnostics.
fn host_savedir(
    vm: &mut Vm,
    _: &[crysirror_python::Value],
    _: crysirror_python::builtins::Kw,
) -> crysirror_python::PyResult<crysirror_python::Value> {
    Ok(vm.types.new_str("saves"))
}

/// `_crysirror_renpy`: what the runtime shells need from the host.
pub fn install_host_module(vm: &mut Vm) {
    vm.register_native_module("_crysirror_renpy", |vm| {
        Ok(crysirror_python::modules::module_with(
            vm,
            "_crysirror_renpy",
            &[
                ("version", host_version),
                ("config_profile", host_config_profile),
                ("py2", host_py2),
                ("variants", host_variants),
                ("savedir", host_savedir),
            ],
        ))
    });
}

/// 8.4.0: fixed rollback is a flag on each `Rollback` and `Forward` entry
/// (RollbackLog version 7) instead of a boundary node.
pub fn fixed_forward(vm: &Vm) -> bool {
    try_rt(vm).is_some_and(|r| r.profile.at_build([8, 4, 0, 0]))
}

/// 8.5.1: rollback and load raise `RollbackException`/`UnfreezeException`,
/// performed by `run_context`; the restart exceptions are `BaseException`s
/// in `renpy.execution` instead of `Exception`s in `renpy.game`.
pub fn deferred_rollback(vm: &Vm) -> bool {
    try_rt(vm).is_some_and(|r| r.profile.at_build([8, 5, 1, 0]))
}

/// The module that defines `RestartContext`/`RestartTopContext`.
pub fn restart_module(vm: &Vm) -> &'static str {
    if deferred_rollback(vm) { "renpy.execution" } else { "renpy.game" }
}
