// SPDX-License-Identifier: MPL-2.0
//! `renpy.error` / `renpy.display.error` per release: the 8.3 report
//! tuple and the 8.4+ `TracebackException` API.
//!
//! `data/error84_expected.txt` is the output of `data/error84_scenario.py`
//! run under CPython against upstream `renpy/error.py` of
//! 8.4.1.25072401 (with `renpy.lexer` helpers of the same tag and
//! `data/error84_t.rpy` as `game/t.rpy`).
use crysirror_python::{Dialect, Vm, ops};
use crysirror_renpy::rt::{
    boot,
    state::{Files, Profile},
};
use std::sync::Arc;

struct TestFiles;

impl Files for TestFiles {
    fn list(&self) -> Vec<String> {
        vec!["t.rpy".into()]
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        (name == "t.rpy").then(|| include_bytes!("data/error84_t.rpy").to_vec())
    }
    fn common_list(&self) -> Vec<String> {
        Vec::new()
    }
    fn common_read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn vm(version: [u32; 4]) -> Vm {
    boot::new_vm(Profile::new(version), Arc::new(TestFiles))
}

/// Runs `src` after importing the engine; returns its `RESULT` string.
fn run(vm: &mut Vm, src: &str) -> String {
    let src = format!("import renpy\nrenpy.import_all()\n{src}");
    let g = match crysirror_python::run_source(vm, &src, Dialect::Py3) {
        Ok(g) => g,
        Err(e) => panic!("{e}"),
    };
    let k = vm.types.new_str("RESULT");
    match ops::dict_get(vm, &g, &k) {
        Ok(Some(v)) => v.as_str().map(str::to_owned).unwrap_or_default(),
        Ok(None) => String::new(),
        Err(_) => panic!("RESULT lookup failed"),
    }
}

#[test]
fn tracebacks_of_8_4_match_upstream_formatting() {
    for version in [
        [8, 4, 1, 25072401],
        [8, 5, 3, 26051504],
        [8, 6, 1, 26111705],
    ] {
        let mut vm = vm(version);
        let got = run(&mut vm, include_str!("data/error84_scenario.py"));
        let want = include_str!("data/error84_expected.txt");
        for (i, (g, w)) in got.split("\n=====\n").zip(want.split("\n=====\n")).enumerate() {
            assert_eq!(g, w, "case {i} of {version:?}");
        }
        assert_eq!(got, want);
    }
}

#[test]
fn report_exception_returns_a_traceback_exception_from_8_4() {
    let mut vm = vm([8, 4, 1, 25072401]);
    let got = run(
        &mut vm,
        r#"
import renpy.error, renpy.game, renpy.config
renpy.game.exception_info = "While running game code:"
renpy.config.name = "Game"
renpy.config.version = "1.0"
def boom():
    raise NameError("name 'x' is not defined")
try:
    boom()
except Exception as e:
    te = renpy.error.report_exception(e, editor=False)
assert isinstance(te, renpy.error.TracebackException)
simple, full, fn = te
assert (simple, full, fn) == (te.simple, te.full, te.traceback_fn) and len(te) == 3
assert te[1] == full and fn == "traceback.txt"
assert simple == "While running game code:\nNameError: name 'x' is not defined\n", repr(simple)
lines = full.split("\n")
assert lines[0] == "Traceback (most recent call last):", full
assert '  File "<string>", line 11, in <module>' in lines and '  File "<string>", line 9, in boom' in lines, full
assert "NameError: name 'x' is not defined" in lines and "Game 1.0" in lines, full
renpy.config.init()
h = renpy.config.error_suggestion_handlers
assert h[AttributeError] is renpy.error.handle_attribute_error
assert h[NameError] is renpy.error.handle_name_error
assert h[ImportError] is renpy.error.handle_import_error
RESULT = "ok"
"#,
    );
    assert_eq!(got, "ok");
}

#[test]
fn report_exception_keeps_the_8_3_tuple() {
    let mut vm = vm([8, 3, 7, 25031702]);
    let got = run(
        &mut vm,
        r#"
import renpy.error, renpy.game, renpy.config, renpy.display.error
renpy.game.exception_info = "While running game code:"
renpy.config.name = "Game"
renpy.config.version = "1.0"
assert not hasattr(renpy.error, "TracebackException")
assert not hasattr(renpy.error, "TextIOExceptionPrintContext")
try:
    raise ValueError("v")
except Exception as e:
    r = renpy.error.report_exception(e, editor=False)
assert isinstance(r, tuple) and len(r) == 3
assert r[0] == "While running game code:\n  File \"<string>\", line 9, in <module>\n" or r[0].startswith("While running game code:"), r[0]
assert r[0].endswith("ValueError: v\n") and r[1].startswith("Full traceback:\n"), r
assert r[2] == "traceback.txt"
RESULT = "ok"
"#,
    );
    assert_eq!(got, "ok");
}
