// SPDX-License-Identifier: MPL-2.0
//! `renpy.error` and `renpy.display.error` of Ren'Py 8.4 and later (MIT),
//! which replaced the 8.3 `(simple, full, traceback_fn)` reports with
//! `TracebackException` objects formatted through exception print
//! contexts.
//!
//! The API, its text and the order of the context calls follow the
//! 8.4.1.25072401 source; 8.5.3 adds `include_screenshot` to the trace
//! save and 8.6.1 moves `raise_image_load_exceptions` to
//! `renpy.display.im.thread_local`. Formatting goes through the context's
//! Python methods, so script subclasses such as `_ExceptionPrintContext`
//! (`_errorhandling.rpym`) control the output as upstream.
//!
//! Interpreter differences, kept visible instead of invented:
//!
//! * Tracebacks are `(filename, line, name[, text])` entries without
//!   frames, column positions or locals, so frames from the interpreter
//!   carry no carets. Carets still appear for `SyntaxError` offsets and
//!   for `FrameSummary` objects given explicit columns. Anchors are
//!   computed from the crysirror-python parser, which records start
//!   positions only: `BinOp` operators are found back from the right
//!   operand and call/subscript brackets are the opener matching the
//!   final bracket of the segment.
//! * Frames of engine modules are native and therefore absent from the
//!   full traceback (upstream lists `renpy/ast.py`, `renpy/python.py`
//!   and so on before the game frame).
//! * `handle_name_error` takes the candidates of the failing frame from
//!   the store namespace and the builtins, the namespace Ren'Py script
//!   Python runs in; locals and the `self.name` hint are unavailable.
//!   `handle_attribute_error` cannot see the failing frame's `self`.
//!   All handlers read the exception's `name`/`obj`/`name_from`
//!   attributes, which interpreter-raised exceptions do not carry yet,
//!   so they return no suggestion for them, as upstream does for
//!   exceptions created without a name.
//! * `report_exception` of an exception without a traceback leaves the
//!   stack empty (upstream adds the caller's frame).
//! * There is no log directory: `traceback.txt` is the in-memory log of
//!   the same name, as in the 8.3 report; the editor is not launched.
use super::{log, py, state};
use crysirror_python::{ObjRef, PyErr, PyResult, Value, Vm, builtins::Kw, ops};
use std::rc::Rc;

const ERROR: &str = "renpy.error";
const DISPLAY_ERROR: &str = "renpy.display.error";

pub const CAUSE_MESSAGE: &str = "The above exception was the direct cause of the following exception:";
pub const CONTEXT_MESSAGE: &str = "During handling of the above exception, another exception occurred:";

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    state::try_rt(vm).is_some_and(|r| r.profile.at_least(major, minor))
}

/// True when the running release uses this module (8.4+).
pub fn active(vm: &Vm) -> bool {
    at_least(vm, 8, 4)
}

fn kwarg(name: &str, v: Value) -> (Rc<str>, Value) {
    (Rc::from(name), v)
}

fn exc_is(vm: &mut Vm, e: &PyErr, name: &str) -> bool {
    match e {
        PyErr::Exc(o) => {
            let cls = Value::Obj(vm.types.exc(name));
            ops::exc_matches(vm, &Value::Obj(o.clone()), &cls).unwrap_or(false)
        }
        PyErr::Suspend(_) => false,
    }
}

fn int_attr(vm: &mut Vm, o: &Value, name: &str) -> PyResult<i64> {
    let v = py::get(vm, o, name)?;
    py::int(vm, &v)
}

fn str_arg(vm: &mut Vm, v: &Value) -> PyResult<String> {
    py::string(vm, v)
}

fn is_int(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Bool(_))
}

/// `str.isspace()` of one character.
fn py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn rstrip(s: &str) -> &str {
    s.trim_end_matches(py_space)
}

/// The `[start:stop]` bounds Python slicing gives for a length.
fn slice_bounds(len: i64, start: i64, stop: i64) -> (usize, usize) {
    let fix = |i: i64| -> i64 {
        if i < 0 { (len + i).max(0) } else { i.min(len) }
    };
    let (a, b) = (fix(start), fix(stop));
    (a as usize, b.max(a) as usize)
}

fn chars_slice(s: &[char], start: i64, stop: i64) -> String {
    let (a, b) = slice_bounds(s.len() as i64, start, stop);
    s[a..b].iter().collect()
}

/// `str.splitlines()` over `\n`, `\r\n` and `\r`.
fn splitlines(s: &str) -> Vec<String> {
    let mut rv = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => rv.push(std::mem::take(&mut cur)),
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                rv.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        rv.push(cur);
    }
    rv
}

fn with_indent<T>(
    vm: &mut Vm,
    ctx: &Value,
    f: impl FnOnce(&mut Vm) -> PyResult<T>,
) -> PyResult<T> {
    let cm = py::method(vm, ctx, "indent", Vec::new())?;
    py::method(vm, &cm, "__enter__", Vec::new())?;
    let r = f(vm);
    let x = py::method(
        vm,
        &cm,
        "__exit__",
        vec![Value::None, Value::None, Value::None],
    );
    let v = r?;
    x?;
    Ok(v)
}

fn new_string_io(vm: &mut Vm) -> PyResult<Value> {
    let c = py::import_attr(vm, "io", "StringIO")?;
    vm.call(&c, Vec::new(), Vec::new())
}

fn environ(vm: &mut Vm, name: &str) -> PyResult<Value> {
    let env = py::import_attr(vm, "os", "environ")?;
    let k = py::s(vm, name);
    py::method(vm, &env, "get", vec![k])
}

// Exception print contexts --------------------------------------------------------

fn ec_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "indent_depth", Value::Int(0))?;
    py::set(vm, this, "exception_group_depth", Value::Int(0))?;
    py::set(vm, this, "need_close", Value::Bool(false))?;
    let opt = |n: &str, d: Value| py::kw(kw, n).cloned().unwrap_or(d);
    py::set(vm, this, "filter_private", opt("filter_private", Value::Bool(false)))?;
    py::set(vm, this, "max_group_width", opt("max_group_width", Value::Int(15)))?;
    py::set(vm, this, "max_group_depth", opt("max_group_depth", Value::Int(10)))?;
    py::set(vm, this, "emit_context", opt("emit_context", Value::Bool(true)))?;
    Ok(Value::None)
}

/// `should_filter(filename)`: with `filter_private`, hides `<...>`
/// pseudo-files, Ren'Py common and `game/libs` scripts and all `.py`.
pub fn should_filter(filter_private: bool, filename: &str) -> bool {
    if !filter_private {
        return false;
    }
    if filename.starts_with('<') && filename.ends_with('>') {
        return true;
    }
    if [".rpy", ".rpym", "_ren.py"].iter().any(|s| filename.ends_with(s)) {
        filename.starts_with("renpy/common/") || filename.starts_with("game/libs/")
    } else {
        filename.ends_with(".py")
    }
}

fn ec_should_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let fp = py::get(vm, &args[0], "filter_private")?;
    let fp = py::truthy(vm, &fp)?;
    let f = str_arg(vm, &args[1])?;
    Ok(Value::Bool(should_filter(fp, &f)))
}

fn ec_indent(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = py::import_attr(vm, ERROR, "_Indent")?;
    vm.call(&cls, vec![args[0].clone()], Vec::new())
}

fn ec_abstract(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(vm.exc_msg("NotImplementedError", "abstract ExceptionPrintContext method"))
}

fn indent_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "ctx", args[1].clone())?;
    Ok(Value::None)
}

fn indent_step(vm: &mut Vm, this: &Value, d: i64) -> PyResult<()> {
    let ctx = py::get(vm, this, "ctx")?;
    let n = int_attr(vm, &ctx, "indent_depth")?;
    py::set(vm, &ctx, "indent_depth", Value::Int(n + d))
}

fn indent_enter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    indent_step(vm, &args[0], 1)?;
    Ok(args[0].clone())
}

fn indent_exit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    indent_step(vm, &args[0], -1)?;
    Ok(Value::Bool(false))
}

fn tio_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let file = py::arg_or(&args[1..], kw, 0, "file", Value::None);
    ec_init(vm, &args[..1], kw)?;
    py::set(vm, &args[0], "file", file)?;
    Ok(Value::None)
}

fn tio_getvalue(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "file")?;
    match py::method(vm, &f, "getvalue", Vec::new()) {
        Err(e) if exc_is(vm, &e, "AttributeError") => Ok(Value::None),
        r => r,
    }
}

/// `print(text, file=self.file)` after the indentation.
fn tio_print(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let text = py::arg_or(&args[1..], kw, 0, "text", py::s(vm, ""));
    let mut text = str_arg(vm, &text)?;
    if !text.is_empty() {
        let depth = int_attr(vm, this, "indent_depth")?;
        text = format!("{}{text}", "  ".repeat(depth.max(0) as usize));
    }
    let f = py::get(vm, this, "file")?;
    let t = py::s(vm, &text);
    py::method(vm, &f, "write", vec![t])?;
    let nl = py::s(vm, "\n");
    py::method(vm, &f, "write", vec![nl])?;
    Ok(Value::None)
}

fn print_line(vm: &mut Vm, ctx: &Value, text: &str) -> PyResult<()> {
    let t = py::s(vm, text);
    py::method(vm, ctx, "_print", vec![t])?;
    Ok(())
}

fn tio_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::method(vm, &args[0], "_print", vec![args[1].clone()])
}

fn tio_chain(vm: &mut Vm, this: &Value, msg: &str) -> PyResult<Value> {
    print_line(vm, this, "")?;
    print_line(vm, this, msg)?;
    print_line(vm, this, "")?;
    Ok(Value::None)
}

fn tio_chain_cause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    tio_chain(vm, &args[0], CAUSE_MESSAGE)
}

fn tio_chain_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    tio_chain(vm, &args[0], CONTEXT_MESSAGE)
}

fn tio_exceptions_separator(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let index = py::arg_or(&args[1..], kw, 0, "index", Value::Int(0));
    let index = py::int(vm, &index)?;
    let width = int_attr(vm, this, "max_group_width")?;
    let title = if index >= width {
        "...".to_owned()
    } else {
        (index + 1).to_string()
    };
    let lead = if index == 0 { "+-" } else { "  " };
    print_line(
        vm,
        this,
        &format!("{lead}+---------------- {title} ----------------"),
    )?;
    Ok(Value::None)
}

fn tio_exceptions_close(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    print_line(vm, &args[0], "+------------------------------------")?;
    Ok(Value::None)
}

fn location_text(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<(String, String, String)> {
    let filename = py::arg_or(&args[1..], kw, 0, "filename", Value::None);
    let lineno = py::arg_or(&args[1..], kw, 1, "lineno", Value::None);
    let name = py::arg_or(&args[1..], kw, 2, "name", Value::None);
    let filename = str_arg(vm, &filename)?;
    let lineno = str_arg(vm, &lineno)?;
    let name = if py::is_none(&name) {
        String::new()
    } else {
        format!(", in {}", str_arg(vm, &name)?)
    };
    Ok((filename, lineno, name))
}

const ANSI_RESET: &str = "\x1b[0m";

const ANSI_COLORS: &[(&str, &str)] = &[
    ("RESET", "\x1b[0m"),
    ("BLACK", "\x1b[30m"),
    ("BLUE", "\x1b[34m"),
    ("CYAN", "\x1b[36m"),
    ("GREEN", "\x1b[32m"),
    ("MAGENTA", "\x1b[35m"),
    ("RED", "\x1b[31m"),
    ("WHITE", "\x1b[37m"),
    ("YELLOW", "\x1b[33m"),
    ("BOLD_BLACK", "\x1b[1;30m"),
    ("BOLD_BLUE", "\x1b[1;34m"),
    ("BOLD_CYAN", "\x1b[1;36m"),
    ("BOLD_GREEN", "\x1b[1;32m"),
    ("BOLD_MAGENTA", "\x1b[1;35m"),
    ("BOLD_RED", "\x1b[1;31m"),
    ("BOLD_WHITE", "\x1b[1;37m"),
    ("BOLD_YELLOW", "\x1b[1;33m"),
    ("INTENSE_BLACK", "\x1b[90m"),
    ("INTENSE_BLUE", "\x1b[94m"),
    ("INTENSE_CYAN", "\x1b[96m"),
    ("INTENSE_GREEN", "\x1b[92m"),
    ("INTENSE_MAGENTA", "\x1b[95m"),
    ("INTENSE_RED", "\x1b[91m"),
    ("INTENSE_WHITE", "\x1b[97m"),
    ("INTENSE_YELLOW", "\x1b[93m"),
    ("BACKGROUND_BLACK", "\x1b[40m"),
    ("BACKGROUND_BLUE", "\x1b[44m"),
    ("BACKGROUND_CYAN", "\x1b[46m"),
    ("BACKGROUND_GREEN", "\x1b[42m"),
    ("BACKGROUND_MAGENTA", "\x1b[45m"),
    ("BACKGROUND_RED", "\x1b[41m"),
    ("BACKGROUND_WHITE", "\x1b[47m"),
    ("BACKGROUND_YELLOW", "\x1b[43m"),
    ("INTENSE_BACKGROUND_BLACK", "\x1b[100m"),
    ("INTENSE_BACKGROUND_BLUE", "\x1b[104m"),
    ("INTENSE_BACKGROUND_CYAN", "\x1b[106m"),
    ("INTENSE_BACKGROUND_GREEN", "\x1b[102m"),
    ("INTENSE_BACKGROUND_MAGENTA", "\x1b[105m"),
    ("INTENSE_BACKGROUND_RED", "\x1b[101m"),
    ("INTENSE_BACKGROUND_WHITE", "\x1b[107m"),
    ("INTENSE_BACKGROUND_YELLOW", "\x1b[103m"),
];

fn color_of(vm: &mut Vm, this: &Value, name: &str) -> PyResult<String> {
    let v = py::get(vm, this, name)?;
    str_arg(vm, &v)
}

fn ac_location(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (filename, lineno, name) = location_text(vm, args, kw)?;
    let c = color_of(vm, &args[0], "LOCATION_COLOR")?;
    print_line(
        vm,
        &args[0],
        &format!("{c}File \"{filename}\", line {lineno}{ANSI_RESET}{name}"),
    )?;
    Ok(Value::None)
}

/// `zip(line, carets, strict=True)`.
fn zip_strict(vm: &Vm, line: &[char], carets: &[char]) -> PyResult<()> {
    if line.len() == carets.len() {
        return Ok(());
    }
    Err(vm.value_error(if carets.len() < line.len() {
        "zip() argument 2 is shorter than argument 1"
    } else {
        "zip() argument 2 is longer than argument 1"
    }))
}

/// `source_carets` of colouring contexts: runs of `^`/`~` carets wrap
/// their characters in `primary`/`secondary` (`escape` applies to each
/// run first).
pub fn colorize(
    vm: &Vm,
    line: &str,
    carets: &str,
    primary: &dyn Fn(&str) -> String,
    secondary: &dyn Fn(&str) -> String,
) -> PyResult<String> {
    let l: Vec<char> = line.chars().collect();
    let c: Vec<char> = carets.chars().collect();
    zip_strict(vm, &l, &c)?;
    let mut out = String::new();
    let mut i = 0;
    while i < l.len() {
        let k = c[i];
        let mut j = i;
        while j < l.len() && c[j] == k {
            j += 1;
        }
        let part: String = l[i..j].iter().collect();
        out.push_str(&match k {
            '^' => primary(&part),
            '~' => secondary(&part),
            _ => part,
        });
        i = j;
    }
    Ok(out)
}

fn ac_source_carets(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let line = py::arg_or(&args[1..], kw, 0, "line", Value::None);
    let carets = py::arg_or(&args[1..], kw, 1, "carets", Value::None);
    if py::is_none(&carets) {
        py::method(vm, this, "_print", vec![line])?;
        return Ok(Value::None);
    }
    let line = str_arg(vm, &line)?;
    let carets = str_arg(vm, &carets)?;
    let p = color_of(vm, this, "PRIMARY_CARET_COLOR")?;
    let s = color_of(vm, this, "SECONDARY_CARET_COLOR")?;
    let text = colorize(
        vm,
        &line,
        &carets,
        &|t| format!("{p}{t}{ANSI_RESET}"),
        &|t| format!("{s}{t}{ANSI_RESET}"),
    )?;
    print_line(vm, this, &text)?;
    Ok(Value::None)
}

fn ac_final_exception_line(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let exc_type = py::arg_or(&args[1..], kw, 0, "exc_type", Value::None);
    let text = py::arg_or(&args[1..], kw, 1, "text", Value::None);
    let tc = color_of(vm, this, "EXCEPTION_TYPE_COLOR")?;
    let exc_type = format!("{tc}{}{ANSI_RESET}", str_arg(vm, &exc_type)?);
    if py::is_none(&text) {
        print_line(vm, this, &exc_type)?;
    } else {
        let vc = color_of(vm, this, "EXCEPTION_VALUE_COLOR")?;
        let text = format!("{vc}{}{ANSI_RESET}", str_arg(vm, &text)?);
        print_line(vm, this, &format!("{exc_type}: {text}"))?;
    }
    Ok(Value::None)
}

/// Wide (`W`) and fullwidth (`F`) ranges of `unicodedata.east_asian_width`
/// (Unicode 17.0 data; CPython 3.12 ships 15.0, which assigns a subset).
const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F), (0x231A, 0x231B), (0x2329, 0x232A), (0x23E9, 0x23EC), (0x23F0, 0x23F0),
    (0x23F3, 0x23F3), (0x25FD, 0x25FE), (0x2614, 0x2615), (0x2630, 0x2637), (0x2648, 0x2653),
    (0x267F, 0x267F), (0x268A, 0x268F), (0x2693, 0x2693), (0x26A1, 0x26A1), (0x26AA, 0x26AB),
    (0x26BD, 0x26BE), (0x26C4, 0x26C5), (0x26CE, 0x26CE), (0x26D4, 0x26D4), (0x26EA, 0x26EA),
    (0x26F2, 0x26F3), (0x26F5, 0x26F5), (0x26FA, 0x26FA), (0x26FD, 0x26FD), (0x2705, 0x2705),
    (0x270A, 0x270B), (0x2728, 0x2728), (0x274C, 0x274C), (0x274E, 0x274E), (0x2753, 0x2755),
    (0x2757, 0x2757), (0x2795, 0x2797), (0x27B0, 0x27B0), (0x27BF, 0x27BF), (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50), (0x2B55, 0x2B55), (0x2E80, 0x2E99), (0x2E9B, 0x2EF3), (0x2F00, 0x2FD5),
    (0x2FF0, 0x303E), (0x3041, 0x3096), (0x3099, 0x30FF), (0x3105, 0x312F), (0x3131, 0x318E),
    (0x3190, 0x31E5), (0x31EF, 0x321E), (0x3220, 0x3247), (0x3250, 0xA48C), (0xA490, 0xA4C6),
    (0xA960, 0xA97C), (0xAC00, 0xD7A3), (0xF900, 0xFAFF), (0xFE10, 0xFE19), (0xFE30, 0xFE52),
    (0xFE54, 0xFE66), (0xFE68, 0xFE6B), (0xFF01, 0xFF60), (0xFFE0, 0xFFE6), (0x16FE0, 0x16FE4),
    (0x16FF0, 0x16FF6), (0x17000, 0x18CD5), (0x18CFF, 0x18D1E), (0x18D80, 0x18DF2),
    (0x1AFF0, 0x1AFF3), (0x1AFF5, 0x1AFFB), (0x1AFFD, 0x1AFFE), (0x1B000, 0x1B122),
    (0x1B132, 0x1B132), (0x1B150, 0x1B152), (0x1B155, 0x1B155), (0x1B164, 0x1B167),
    (0x1B170, 0x1B2FB), (0x1D300, 0x1D356), (0x1D360, 0x1D376), (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF), (0x1F18E, 0x1F18E), (0x1F191, 0x1F19A), (0x1F200, 0x1F202),
    (0x1F210, 0x1F23B), (0x1F240, 0x1F248), (0x1F250, 0x1F251), (0x1F260, 0x1F265),
    (0x1F300, 0x1F320), (0x1F32D, 0x1F335), (0x1F337, 0x1F37C), (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA), (0x1F3CF, 0x1F3D3), (0x1F3E0, 0x1F3F0), (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E), (0x1F440, 0x1F440), (0x1F442, 0x1F4FC), (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E), (0x1F550, 0x1F567), (0x1F57A, 0x1F57A), (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4), (0x1F5FB, 0x1F64F), (0x1F680, 0x1F6C5), (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2), (0x1F6D5, 0x1F6D8), (0x1F6DC, 0x1F6DF), (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC), (0x1F7E0, 0x1F7EB), (0x1F7F0, 0x1F7F0), (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945), (0x1F947, 0x1F9FF), (0x1FA70, 0x1FA7C), (0x1FA80, 0x1FA8A),
    (0x1FA8E, 0x1FAC6), (0x1FAC8, 0x1FAC8), (0x1FACD, 0x1FADC), (0x1FADF, 0x1FAEA),
    (0x1FAEF, 0x1FAF8), (0x20000, 0x2FFFD), (0x30000, 0x3FFFD),
];

/// `NonColoredExceptionPrintContext._display_width` of one character.
pub fn display_width(c: char) -> usize {
    let c = c as u32;
    if c < 0x1100 {
        return 1;
    }
    let wide = WIDE
        .binary_search_by(|&(a, b)| {
            if b < c {
                std::cmp::Ordering::Less
            } else if a > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok();
    if wide { 2 } else { 1 }
}

fn nc_display_width(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let line = py::arg_or(args, kw, 0, "line", Value::None);
    let line = str_arg(vm, &line)?;
    let items = line
        .chars()
        .map(|c| Value::Int(display_width(c) as i64))
        .collect();
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

fn nc_location(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (filename, lineno, name) = location_text(vm, args, kw)?;
    print_line(
        vm,
        &args[0],
        &format!("File \"{filename}\", line {lineno}{name}"),
    )?;
    Ok(Value::None)
}

/// The caret line under `line`: each caret repeated over the display
/// width of its character (`zip` stops at the shorter string).
pub fn widen_carets(line: &str, carets: &str) -> String {
    line.chars()
        .zip(carets.chars())
        .map(|(l, c)| c.to_string().repeat(display_width(l)))
        .collect()
}

fn nc_source_carets(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let line = py::arg_or(&args[1..], kw, 0, "line", Value::None);
    let carets = py::arg_or(&args[1..], kw, 1, "carets", Value::None);
    py::method(vm, this, "_print", vec![line.clone()])?;
    if !py::is_none(&carets) {
        let line = str_arg(vm, &line)?;
        let carets = str_arg(vm, &carets)?;
        print_line(vm, this, &widen_carets(&line, &carets))?;
    }
    Ok(Value::None)
}

fn nc_final_exception_line(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let exc_type = py::arg_or(&args[1..], kw, 0, "exc_type", Value::None);
    let text = py::arg_or(&args[1..], kw, 1, "text", Value::None);
    let exc_type = str_arg(vm, &exc_type)?;
    let line = if py::is_none(&text) {
        exc_type
    } else {
        format!("{exc_type}: {}", str_arg(vm, &text)?)
    };
    print_line(vm, &args[0], &line)?;
    Ok(Value::None)
}

/// `MaybeColoredExceptionPrintContext(file=None, **kwargs)`: colours a
/// terminal (`isatty`) unless `NO_COLOR`/`TERM=dumb`, or when
/// `FORCE_COLOR` is set. The Windows console-mode probe is the POSIX
/// `isatty` check here.
fn f_maybe_colored(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut file = py::arg_or(args, kw, 0, "file", Value::None);
    if py::is_none(&file) {
        file = py::import_attr(vm, "sys", "stdout")?;
        if py::is_none(&file) {
            file = new_string_io(vm)?;
        }
    }
    let kwargs: Vec<(Rc<str>, Value)> = kw
        .iter()
        .filter(|(k, _)| &**k != "file")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let make = |vm: &mut Vm, cls: &str, file: Value| -> PyResult<Value> {
        let c = py::import_attr(vm, ERROR, cls)?;
        vm.call(&c, vec![file], kwargs.clone())
    };
    const PLAIN: &str = "NonColoredExceptionPrintContext";
    const ANSI: &str = "ANSIColoredPrintContext";
    let v = environ(vm, "NO_COLOR")?;
    if py::truthy(vm, &v)? {
        return make(vm, PLAIN, file);
    }
    let v = environ(vm, "FORCE_COLOR")?;
    if py::truthy(vm, &v)? {
        return make(vm, ANSI, file);
    }
    let v = environ(vm, "TERM")?;
    if v.as_str() == Some("dumb") {
        return make(vm, PLAIN, file);
    }
    let fileno = match py::method(vm, &file, "fileno", Vec::new()) {
        Ok(n) => n,
        Err(PyErr::Exc(_)) => return make(vm, PLAIN, file),
        Err(e) => return Err(e),
    };
    let isatty = match py::method(vm, &file, "isatty", Vec::new()) {
        Ok(v) => py::truthy(vm, &v).unwrap_or(false),
        Err(_) => match py::import_attr(vm, "os", "isatty") {
            Ok(f) => match vm.call(&f, vec![fileno], Vec::new()) {
                Ok(v) => py::truthy(vm, &v).unwrap_or(false),
                Err(_) => false,
            },
            Err(_) => false,
        },
    };
    make(vm, if isatty { ANSI } else { PLAIN }, file)
}

// Line offsets ----------------------------------------------------------------------

/// Line `n` (1-based) of a game or common file as `linecache.getline`
/// returns it, without the line ending; empty when unavailable.
fn file_line(vm: &Vm, filename: &str, n: i64) -> String {
    if n < 1 {
        return String::new();
    }
    super::error::file_lines(vm, filename)
        .and_then(|l| l.into_iter().nth((n - 1) as usize))
        .unwrap_or_default()
}

/// `renpy.lexer.ren_py_to_rpy_offsets`: per `_ren.py` line, None when the
/// `.rpy` equivalent drops it, else the indentation added to it.
pub fn ren_py_to_rpy_offsets(lines: &[String]) -> Vec<Option<i64>> {
    #[derive(PartialEq)]
    enum S {
        Ignore,
        Renpy,
        Python,
    }
    let mut state = S::Ignore;
    let mut current = 0i64;
    let mut rv = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let l = if i == 0 {
            l.strip_prefix('\u{feff}').unwrap_or(l)
        } else {
            l.as_str()
        };
        if state != S::Renpy && l.starts_with("\"\"\"renpy") {
            state = S::Renpy;
            rv.push(None);
            continue;
        }
        match state {
            S::Renpy => {
                let sl = l.trim_matches(py_space);
                if sl == "\"\"\"" {
                    rv.push(None);
                    state = S::Python;
                    continue;
                }
                if sl.is_empty() || sl.starts_with('#') {
                    rv.push(Some(0));
                    continue;
                }
                current = l.chars().take_while(|c| *c == ' ').count() as i64;
                if sl.ends_with(':') {
                    current += 4;
                }
                rv.push(Some(0));
            }
            // `file_lines` drops line endings: "" is upstream's "\n".
            S::Python => rv.push(Some(if l == "\n" || l.is_empty() { 0 } else { current })),
            S::Ignore => rv.push(None),
        }
    }
    rv
}

/// `get_string_munger(prefix)(line)`: the line with `__name` replaced as
/// the script compiler munges it (`config.munge_in_strings` selects
/// every word-initial `__name`, else only after `.` or an odd run of
/// `[`).
pub fn munge_string(line: &str, prefix: &str, in_strings: bool) -> String {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let s: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let (lead_end, lead) = if in_strings {
            let boundary = i == 0 || !word(s[i - 1]);
            if !boundary {
                out.push(s[i]);
                i += 1;
                continue;
            }
            (i, 0)
        } else if s[i] == '.' {
            (i + 1, 1)
        } else if s[i] == '[' {
            let mut j = i;
            while j < s.len() && s[j] == '[' {
                j += 1;
            }
            (j, j - i)
        } else {
            out.push(s[i]);
            i += 1;
            continue;
        };
        let name_start = lead_end + 2;
        let ok = s.get(lead_end) == Some(&'_')
            && s.get(lead_end + 1) == Some(&'_')
            && s.get(name_start).is_some_and(|c| word(*c));
        if !ok {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let mut end = name_start;
        while end < s.len() && word(s[end]) {
            end += 1;
        }
        let name: String = s[name_start..end].iter().collect();
        let whole: String = s[i..end].iter().collect();
        let keep = name.contains("__")
            || if in_strings {
                name.starts_with('_')
            } else {
                lead % 2 == 0
            };
        if keep {
            out.push_str(&whole);
        } else {
            out.extend(&s[i..lead_end]);
            out.push_str(prefix);
            out.push_str(&name);
        }
        i = end;
    }
    out
}

/// `normalize_renpy_line_offset(filename, linenumber, offset, line)`:
/// a UTF-8 byte offset into `line` as a character offset, without the
/// indentation `_ren.py` conversion added and the length munged names
/// gained.
pub fn normalize_offset(
    vm: &mut Vm,
    filename: &str,
    linenumber: i64,
    mut offset: i64,
    line: &str,
) -> PyResult<i64> {
    if filename.ends_with("_ren.py") {
        let lines = super::error::file_lines(vm, filename).unwrap_or_default();
        for (i, base) in ren_py_to_rpy_offsets(&lines).into_iter().enumerate() {
            if let Some(base) = base
                && i as i64 + 1 == linenumber
            {
                offset -= base;
                break;
            }
        }
    }
    if !line.is_ascii() {
        let bytes = line.as_bytes();
        let (_, end) = slice_bounds(bytes.len() as i64, 0, offset);
        offset = String::from_utf8_lossy(&bytes[..end]).chars().count() as i64;
    }
    let prefix = crate::lexer::munge_prefix(filename);
    if line.contains(&prefix) {
        let in_strings = match py::config(vm, "munge_in_strings") {
            Ok(v) => py::truthy(vm, &v)?,
            Err(_) => true,
        };
        let munged: Vec<char> = munge_string(line, &prefix, in_strings).chars().collect();
        let p: Vec<char> = prefix.chars().collect();
        let len_prefix = p.len() as i64 - 2;
        let mut idx = 0i64;
        loop {
            let (a, b) = slice_bounds(munged.len() as i64, idx, offset + len_prefix);
            let found = (a + p.len() <= b)
                .then(|| (a..=b - p.len()).find(|&k| munged[k..k + p.len()] == p[..]))
                .flatten();
            let Some(k) = found else { break };
            offset -= len_prefix;
            idx = k as i64 + len_prefix;
        }
    }
    Ok(offset)
}

fn f_normalize_renpy_line_offset(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let filename = py::arg_or(args, kw, 0, "filename", Value::None);
    let linenumber = py::arg_or(args, kw, 1, "linenumber", Value::Int(0));
    let offset = py::arg_or(args, kw, 2, "offset", Value::Int(0));
    let line = py::arg_or(args, kw, 3, "line", Value::None);
    let filename = str_arg(vm, &filename)?;
    let linenumber = py::int(vm, &linenumber)?;
    let offset = py::int(vm, &offset)?;
    let line = str_arg(vm, &line)?;
    Ok(Value::Int(normalize_offset(
        vm, &filename, linenumber, offset, &line,
    )?))
}

// Frame summaries -------------------------------------------------------------------

/// `renpy.lexer.elide_filename`.
fn elide_filename(vm: &mut Vm, filename: &str) -> String {
    let f = filename.replace('\\', "/");
    let dir = |vm: &mut Vm, n: &str| -> String {
        let d = py::config(vm, n)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "/".into())
            .replace('\\', "/");
        let d = if d.len() > 1 { d.trim_end_matches('/') } else { &d };
        format!("{d}/")
    };
    let basedir = dir(vm, "basedir");
    let renpy_base = dir(vm, "renpy_base");
    let dirs = if renpy_base.starts_with(&basedir) {
        [renpy_base, basedir]
    } else {
        [basedir, renpy_base]
    };
    for d in dirs {
        if let Some(rest) = f.strip_prefix(&d) {
            return rest.to_owned();
        }
    }
    f
}

fn fs_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = &args[1..];
    let name = py::arg_or(a, kw, 0, "name", Value::None);
    let filename = py::arg_or(a, kw, 1, "filename", Value::None);
    let lineno = py::arg_or(a, kw, 2, "lineno", Value::None);
    let colno = py::arg_or(a, kw, 3, "colno", Value::None);
    let end_lineno = py::arg_or(a, kw, 4, "end_lineno", Value::None);
    let end_colno = py::arg_or(a, kw, 5, "end_colno", Value::None);
    let text = py::arg_or(a, kw, 6, "text", Value::None);
    let locals = py::arg_or(a, kw, 7, "locals", Value::None);
    py::set(vm, this, "name", name)?;
    let f = str_arg(vm, &filename)?;
    let f = elide_filename(vm, &f);
    let f = py::s(vm, &f);
    py::set(vm, this, "filename", f)?;
    py::set(vm, this, "lineno", lineno.clone())?;
    py::set(vm, this, "colno", colno)?;
    let end = if py::truthy(vm, &end_lineno)? {
        end_lineno
    } else {
        lineno
    };
    py::set(vm, this, "end_lineno", end)?;
    py::set(vm, this, "end_colno", end_colno)?;
    py::set(vm, this, "locals", locals)?;
    if py::is_none(&text) {
        py::set(vm, this, "_lines", Value::None)?;
        py::set(vm, this, "_anchors_value", Value::None)?;
        py::set(vm, this, "_carets", Value::None)?;
    } else {
        let l = vm.types.new_list(vec![text]);
        py::set(vm, this, "_lines", l)?;
        let a = vm.types.new_list(Vec::new());
        py::set(vm, this, "_anchors_value", a)?;
        let c = vm.types.new_list(Vec::new());
        py::set(vm, this, "_carets", c)?;
    }
    Ok(Value::None)
}

fn fs_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let f = py::get(vm, this, "filename")?;
    let l = py::get(vm, this, "lineno")?;
    let n = py::get(vm, this, "name")?;
    let s = format!(
        "<FrameSummary file {}, line {} in {}>",
        str_arg(vm, &f)?,
        str_arg(vm, &l)?,
        str_arg(vm, &n)?
    );
    Ok(py::s(vm, &s))
}

/// `FrameSummary.lines`, read once from the source file.
fn frame_lines(vm: &mut Vm, this: &Value) -> PyResult<Vec<String>> {
    let cached = py::get(vm, this, "_lines")?;
    if !py::is_none(&cached) {
        let mut rv = Vec::new();
        for l in py::items(vm, &cached)? {
            rv.push(str_arg(vm, &l)?);
        }
        return Ok(rv);
    }
    let f = py::get(vm, this, "filename")?;
    let f = str_arg(vm, &f)?;
    let start = int_attr(vm, this, "lineno")?;
    let end = int_attr(vm, this, "end_lineno")?;
    let rv: Vec<String> = (start..=end)
        .map(|n| rstrip(&file_line(vm, &f, n)).to_owned())
        .collect();
    let items = rv.iter().map(|l| py::s(vm, l)).collect();
    let l = vm.types.new_list(items);
    py::set(vm, this, "_lines", l)?;
    Ok(rv)
}

fn str_iter(vm: &mut Vm, items: &[String]) -> PyResult<Value> {
    let items = items.iter().map(|l| py::s(vm, l)).collect();
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

fn fs_lines(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = frame_lines(vm, &args[0])?;
    str_iter(vm, &l)
}

fn fs_line(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    match frame_lines(vm, &args[0])?.into_iter().next() {
        Some(l) => Ok(py::s(vm, &l)),
        None => Err(vm.exc_msg("StopIteration", "")),
    }
}

type Anchors = [(i64, i64); 4];

/// Positions of brackets in `lines` outside strings and comments, as
/// `(opener, closer)` pairs in closing order.
fn bracket_pairs(lines: &[Vec<char>]) -> Vec<((i64, i64), (i64, i64))> {
    let mut pairs = Vec::new();
    let mut stack: Vec<(i64, i64)> = Vec::new();
    let mut quote: Option<(char, bool)> = None;
    for (ln, line) in lines.iter().enumerate() {
        let mut i = 0;
        while i < line.len() {
            let c = line[i];
            if let Some((q, triple)) = quote {
                if c == '\\' {
                    i += 2;
                    continue;
                }
                if c == q {
                    if !triple {
                        quote = None;
                    } else if line.get(i + 1) == Some(&q) && line.get(i + 2) == Some(&q) {
                        quote = None;
                        i += 3;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            match c {
                '#' => break,
                '\'' | '"' => {
                    let triple = line.get(i + 1) == Some(&c) && line.get(i + 2) == Some(&c);
                    quote = Some((c, triple));
                    i += if triple { 3 } else { 1 };
                    continue;
                }
                '(' | '[' | '{' => stack.push((ln as i64, i as i64)),
                ')' | ']' | '}' => {
                    if let Some(o) = stack.pop() {
                        pairs.push((o, (ln as i64, i as i64)));
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if quote.is_some_and(|(_, triple)| !triple) {
            quote = None;
        }
    }
    pairs
}

/// The operator token of the root `BinOp` of `src` (`(\n<segment>\n)`),
/// as `(segment line, column, length)`: the last binary operator of the
/// root's precedence outside brackets (the first for right-associative
/// `**`). The parser records start positions only, so this stands in for
/// upstream's search from the end of the left operand.
fn root_operator(src: &str, op: crysirror_python::ast::BinOp) -> Option<(i64, i64, i64)> {
    use crysirror_python::ast::BinOp as B;
    use crysirror_python::lexer::Tok;
    let class = |o: &str| -> Option<u8> {
        Some(match o {
            "|" => 1,
            "^" => 2,
            "&" => 3,
            "<<" | ">>" => 4,
            "+" | "-" => 5,
            "*" | "/" | "//" | "%" | "@" => 6,
            "**" => 8,
            _ => return None,
        })
    };
    let want = match op {
        B::BitOr => 1,
        B::BitXor => 2,
        B::BitAnd => 3,
        B::LShift | B::RShift => 4,
        B::Add | B::Sub => 5,
        B::Mult | B::Div | B::FloorDiv | B::Mod | B::MatMult => 6,
        B::Pow => 8,
    };
    const NOT_OPERANDS: &[&str] = &[
        "not", "and", "or", "in", "is", "if", "else", "lambda", "await", "yield",
    ];
    let toks = crysirror_python::lexer::tokenize(src, crysirror_python::Dialect::Py3).ok()?;
    let mut depth = 0i32;
    let mut operand = false;
    let mut found = None;
    for t in &toks {
        match &t.tok {
            Tok::Op(o) => {
                if depth == 1
                    && operand
                    && class(o) == Some(want)
                    && (found.is_none() || want != 8)
                {
                    found = Some((t.line as i64 - 2, t.col as i64, o.chars().count() as i64));
                }
                match *o {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" | "}" => depth -= 1,
                    _ => {}
                }
                operand = matches!(*o, ")" | "]" | "}");
            }
            Tok::Name(n) => operand = !NOT_OPERANDS.contains(&n.as_str()),
            Tok::Int { .. } | Tok::Float(_) | Tok::Imag(_) | Tok::Str { .. } => operand = true,
            _ => {}
        }
    }
    found
}

/// `_calculate_anchors(frame_summary)`.
fn calculate_anchors(vm: &mut Vm, this: &Value) -> PyResult<Option<Anchors>> {
    let colno = py::get(vm, this, "colno")?;
    let end_colno = py::get(vm, this, "end_colno")?;
    if py::is_none(&colno) || py::is_none(&end_colno) {
        return Ok(None);
    }
    let colno = py::int(vm, &colno)?;
    let end_colno = py::int(vm, &end_colno)?;
    let all_lines = frame_lines(vm, this)?;
    let lineno = int_attr(vm, this, "lineno")?;
    let end_lineno = int_attr(vm, this, "end_lineno")?;
    let end_rel = end_lineno - lineno;
    let index = |i: i64| -> Option<&String> {
        let i = if i < 0 { all_lines.len() as i64 + i } else { i };
        all_lines.get(usize::try_from(i).ok()?)
    };
    let (Some(first_line), Some(last_line)) = (index(0), index(end_rel)) else {
        return Err(vm.exc_msg("IndexError", "list index out of range"));
    };
    let (first_line, last_line) = (first_line.clone(), last_line.clone());
    let f = py::get(vm, this, "filename")?;
    let filename = str_arg(vm, &f)?;
    let start_offset = normalize_offset(vm, &filename, lineno, colno, &first_line)?;
    let end_offset = normalize_offset(vm, &filename, end_lineno, end_colno, &last_line)?;
    let default = [
        (0, start_offset),
        (0, start_offset),
        (end_rel, end_offset),
        (end_rel, end_offset),
    ];
    let joined: Vec<char> = all_lines.join("\n").chars().collect();
    let tail = last_line.chars().count() as i64 - end_offset;
    let segment = chars_slice(&joined, start_offset, joined.len() as i64 - tail);
    use crysirror_python::ast::{ExprKind, StmtKind};
    let tree = crysirror_python::parser::parse_module(
        &format!("(\n{segment}\n)"),
        crysirror_python::parser::ParseOptions::py3(),
    )
    .map_err(|e| vm.exc_msg("SyntaxError", e.msg))?;
    if tree.len() != 1 {
        return Ok(Some(default));
    }
    let StmtKind::Expr(expression) = &tree[0].kind else {
        return Ok(Some(default));
    };
    let lines: Vec<Vec<char>> = splitlines(&segment)
        .iter()
        .map(|l| l.chars().collect())
        .collect();
    let on_first = |(l, c): (i64, i64)| if l == 0 { (l, c + start_offset) } else { (l, c) };
    let expr_end = || -> (i64, i64) {
        let l = lines.len() as i64 - 1;
        (l, lines.last().map_or(0, |x| x.len() as i64))
    };
    match &expression.kind {
        ExprKind::BinOp { op, .. } => {
            let Some((l, c, len)) = root_operator(&format!("(\n{segment}\n)"), *op) else {
                return Ok(Some(default));
            };
            Ok(Some([
                (0, start_offset),
                on_first((l, c)),
                on_first((l, c + len)),
                (end_rel, end_offset),
            ]))
        }
        ExprKind::Subscript { .. } | ExprKind::Call { .. } => {
            let (l, c) = expr_end();
            let close = (l, c - 1);
            let Some((open, _)) = bracket_pairs(&lines)
                .into_iter()
                .rev()
                .find(|(_, cl)| *cl == close)
            else {
                return Ok(Some(default));
            };
            Ok(Some([
                (0, start_offset),
                on_first(open),
                on_first((l, c)),
                (end_rel, end_offset),
            ]))
        }
        _ => Ok(Some(default)),
    }
}

/// `FrameSummary._anchors`, computed once; errors give no anchors.
fn frame_anchors(vm: &mut Vm, this: &Value) -> PyResult<Option<Anchors>> {
    let cached = py::get(vm, this, "_anchors_value")?;
    let list = if py::is_none(&cached) {
        let a = match calculate_anchors(vm, this) {
            Ok(a) => a,
            Err(PyErr::Exc(_)) => None,
            Err(e) => return Err(e),
        };
        let items = a
            .map(|a| {
                a.iter()
                    .map(|&(l, c)| vm.types.new_tuple(vec![Value::Int(l), Value::Int(c)]))
                    .collect()
            })
            .unwrap_or_default();
        let l = vm.types.new_list(items);
        py::set(vm, this, "_anchors_value", l.clone())?;
        l
    } else {
        cached
    };
    let items = py::items(vm, &list)?;
    if items.len() < 4 {
        return Ok(None);
    }
    let mut a = [(0, 0); 4];
    for (i, t) in items.iter().take(4).enumerate() {
        let p = py::items(vm, t)?;
        a[i] = (py::int(vm, &p[0])?, py::int(vm, &p[1])?);
    }
    Ok(Some(a))
}

fn fs_anchors(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(match frame_anchors(vm, &args[0])? {
        Some(a) => {
            let items = a
                .iter()
                .map(|&(l, c)| vm.types.new_tuple(vec![Value::Int(l), Value::Int(c)]))
                .collect();
            vm.types.new_list(items)
        }
        None => Value::None,
    })
}

/// The caret lines of `lines` for anchors.
pub fn caret_lines(lines: &[String], a: &Anchors) -> Vec<String> {
    let mut rv = Vec::new();
    for (lineno, line) in lines.iter().enumerate() {
        let mut carets = String::new();
        let mut whitespace = true;
        for (colno, ch) in line.chars().enumerate() {
            let pos = (lineno as i64, colno as i64);
            if !py_space(ch) {
                whitespace = false;
            }
            carets.push(if whitespace || pos >= a[3] {
                ' '
            } else if pos >= a[2] {
                '~'
            } else if pos >= a[1] {
                '^'
            } else if pos >= a[0] {
                '~'
            } else {
                ' '
            });
        }
        rv.push(carets);
    }
    rv
}

/// One pass over the `FrameSummary.carets` generator: the cached lines,
/// then (with anchors) freshly computed ones, as upstream yields them.
fn frame_carets(vm: &mut Vm, this: &Value) -> PyResult<Vec<String>> {
    let mut out = Vec::new();
    let cached = py::get(vm, this, "_carets")?;
    if !py::is_none(&cached) {
        for c in py::items(vm, &cached)? {
            out.push(str_arg(vm, &c)?);
        }
    }
    match frame_anchors(vm, this)? {
        None => {
            let l = vm.types.new_list(Vec::new());
            py::set(vm, this, "_carets", l)?;
        }
        Some(a) => {
            let lines = frame_lines(vm, this)?;
            let rv = caret_lines(&lines, &a);
            let items = rv.iter().map(|c| py::s(vm, c)).collect();
            let l = vm.types.new_list(items);
            py::set(vm, this, "_carets", l)?;
            out.extend(rv);
        }
    }
    Ok(out)
}

fn fs_carets(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = frame_carets(vm, &args[0])?;
    str_iter(vm, &c)
}

/// `FrameSummary.significant_lines`, in upstream's yield order.
fn significant_lines(len_lines: i64, anchors: Option<&Anchors>) -> Vec<i64> {
    let mut rv = vec![0];
    let mut seen = vec![-1, 0, len_lines];
    if let Some(a) = anchors
        && (a[0] != a[1] || a[2] != a[3])
    {
        for &(lineno, _) in a {
            for i in lineno - 1..lineno + 2 {
                if seen.contains(&i) {
                    continue;
                }
                seen.push(i);
                rv.push(i);
            }
        }
    }
    if !seen.contains(&(len_lines - 1)) {
        rv.push(len_lines - 1);
    }
    rv
}

fn frame_significant_lines(vm: &mut Vm, this: &Value) -> PyResult<Vec<i64>> {
    let n = frame_lines(vm, this)?.len() as i64;
    let a = frame_anchors(vm, this)?;
    Ok(significant_lines(n, a.as_ref()))
}

fn fs_significant_lines(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let items = frame_significant_lines(vm, &args[0])?
        .into_iter()
        .map(Value::Int)
        .collect();
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

/// `textwrap.dedent` (3.12).
pub fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let norm: Vec<&str> = lines
        .iter()
        .map(|l| {
            if !l.is_empty() && l.chars().all(|c| c == ' ' || c == '\t') {
                ""
            } else {
                *l
            }
        })
        .collect();
    let mut margin: Option<&str> = None;
    for l in &norm {
        let ws = l.len() - l.trim_start_matches([' ', '\t']).len();
        if ws == l.len() {
            continue;
        }
        let indent = &l[..ws];
        margin = Some(match margin {
            None => indent,
            Some(m) if indent.starts_with(m) => m,
            Some(m) if m.starts_with(indent) => indent,
            Some(m) => {
                let k = m
                    .bytes()
                    .zip(indent.bytes())
                    .take_while(|(x, y)| x == y)
                    .count();
                &m[..k]
            }
        });
    }
    let m = margin.unwrap_or("");
    norm.iter()
        .map(|l| l.strip_prefix(m).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn py_index<T: Clone>(vm: &Vm, l: &[T], i: i64) -> PyResult<T> {
    let k = if i < 0 { l.len() as i64 + i } else { i };
    usize::try_from(k)
        .ok()
        .and_then(|k| l.get(k).cloned())
        .ok_or_else(|| vm.exc_msg("IndexError", "list index out of range"))
}

/// `FrameSummary.format(ctx)`.
fn fs_format(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ctx = &args[1];
    let f = py::get(vm, this, "filename")?;
    let l = py::get(vm, this, "lineno")?;
    let n = py::get(vm, this, "name")?;
    py::method(vm, ctx, "location", vec![f, l, n])?;
    let original = frame_lines(vm, this)?;
    if !original.iter().any(|l| !l.trim_matches(py_space).is_empty()) {
        return Ok(Value::None);
    }
    let dedented: Vec<String> = dedent(&original.join("\n"))
        .split('\n')
        .map(str::to_owned)
        .collect();
    let amount = original[0].chars().count() as i64 - dedented[0].chars().count() as i64;
    let carets: Vec<Option<String>> = if !frame_carets(vm, this)?.is_empty() {
        frame_carets(vm, this)?
            .iter()
            .map(|c| {
                let c: Vec<char> = c.chars().collect();
                Some(chars_slice(&c, amount, c.len() as i64))
            })
            .collect()
    } else {
        vec![None; dedented.len()]
    };
    let sig = frame_significant_lines(vm, this)?;
    with_indent(vm, ctx, |vm| {
        let emit = |vm: &mut Vm, i: i64| -> PyResult<()> {
            let line = py_index(vm, &dedented, i)?;
            let line = py::s(vm, &line);
            let c = match py_index(vm, &carets, i)? {
                Some(c) => py::s(vm, &c),
                None => Value::None,
            };
            py::method(vm, ctx, "source_carets", vec![line, c])?;
            Ok(())
        };
        for (i, &lineno) in sig.iter().enumerate() {
            if i > 0 {
                let diff = lineno - sig[i - 1];
                if diff == 2 {
                    emit(vm, lineno - 1)?;
                } else if diff > 2 {
                    let t = py::s(vm, &format!("...<{} lines>...", diff - 1));
                    py::method(vm, ctx, "string", vec![t])?;
                }
            }
            emit(vm, lineno)?;
        }
        Ok(())
    })?;
    Ok(Value::None)
}

fn new_frame(vm: &mut Vm, entry: &Value) -> PyResult<Value> {
    let fs = py::import_attr(vm, ERROR, "FrameSummary")?;
    if let Value::Obj(c) = &fs
        && ops::isinstance(vm, entry, c)
    {
        return Ok(entry.clone());
    }
    let p = py::items(vm, entry)?;
    if p.len() < 3 {
        return Err(vm.type_error("traceback entries are (filename, line, name[, text])"));
    }
    let mut kwargs = Vec::new();
    if let Some(t) = p.get(3)
        && !py::is_none(t)
    {
        kwargs.push(kwarg("text", t.clone()));
    }
    vm.call(&fs, vec![p[2].clone(), p[0].clone(), p[1].clone()], kwargs)
}

/// `StackSummary(traceback)`: one frame per interpreter traceback entry.
/// Entries that `Context.report_traceback` contributed (as upstream's
/// `_report_traceback` replaces the `Context.run` frame) are already in
/// the list.
fn ss_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let tb = py::arg_or(&args[1..], kw, 0, "traceback", Value::None);
    if py::is_none(&tb) {
        return Ok(Value::None);
    }
    for e in py::items(vm, &tb)? {
        let f = new_frame(vm, &e)?;
        py::method(vm, this, "append", vec![f])?;
    }
    Ok(Value::None)
}

fn ctx_filters(vm: &mut Vm, ctx: &Value, frame: &Value) -> PyResult<bool> {
    let f = py::get(vm, frame, "filename")?;
    let r = py::method(vm, ctx, "should_filter", vec![f])?;
    py::truthy(vm, &r)
}

fn ss_should_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    for frame in py::items(vm, &args[0])? {
        if !ctx_filters(vm, &args[1], &frame)? {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn repeated(vm: &mut Vm, ctx: &Value, count: i64) -> PyResult<()> {
    let s = if count > 1 { "s" } else { "" };
    let t = py::s(
        vm,
        &format!("[Previous line repeated {count} more time{s}]"),
    );
    py::method(vm, ctx, "string", vec![t])?;
    Ok(())
}

/// `StackSummary.format(ctx)`: unfiltered frames, collapsing more than
/// three repetitions of a frame.
fn ss_format(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    const RECURSIVE_CUTOFF: i64 = 3;
    let ctx = &args[1];
    let mut last: Option<(Value, Value, Value)> = None;
    let mut count = 0;
    for frame in py::items(vm, &args[0])? {
        if ctx_filters(vm, ctx, &frame)? {
            continue;
        }
        let key = (
            py::get(vm, &frame, "filename")?,
            py::get(vm, &frame, "lineno")?,
            py::get(vm, &frame, "name")?,
        );
        let same = match &last {
            Some((f, l, n)) => {
                !py::is_none(f)
                    && !py::is_none(l)
                    && !py::is_none(n)
                    && ops::eq(vm, f, &key.0)?
                    && ops::eq(vm, l, &key.1)?
                    && ops::eq(vm, n, &key.2)?
            }
            None => false,
        };
        if !same {
            if count > RECURSIVE_CUTOFF {
                repeated(vm, ctx, count - RECURSIVE_CUTOFF)?;
            }
            last = Some(key);
            count = 0;
        }
        count += 1;
        if count > RECURSIVE_CUTOFF {
            continue;
        }
        with_indent(vm, ctx, |vm| {
            py::method(vm, &frame, "format", vec![ctx.clone()]).map(|_| ())
        })?;
    }
    if count > RECURSIVE_CUTOFF {
        repeated(vm, ctx, count - RECURSIVE_CUTOFF)?;
    }
    Ok(Value::None)
}

// Suggestions -----------------------------------------------------------------------

/// `compute_closest_value(value, values)`: the candidate closest by
/// CPython's suggestion distance, or None when every candidate is too far
/// or the input is too large.
pub fn compute_closest_value(value: &str, values: &[String]) -> Option<String> {
    const MAX_CANDIDATE_ITEMS: usize = 1200;
    const MAX_STRING_SIZE: usize = 40;
    const MOVE_COST: i64 = 2;
    const CASE_COST: i64 = 1;

    fn substitution_cost(a: char, b: char) -> i64 {
        if a == b {
            0
        } else if a.to_lowercase().eq(b.to_lowercase()) {
            CASE_COST
        } else {
            MOVE_COST
        }
    }

    fn levenshtein(a: &[char], b: &[char], max_cost: i64) -> i64 {
        if a == b {
            return 0;
        }
        let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let (a, b) = (&a[pre..], &b[pre..]);
        let post = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let (mut a, mut b) = (&a[..a.len() - post], &b[..b.len() - post]);
        if a.is_empty() || b.is_empty() {
            return MOVE_COST * (a.len() + b.len()) as i64;
        }
        if a.len() > MAX_STRING_SIZE || b.len() > MAX_STRING_SIZE {
            return max_cost + 1;
        }
        if b.len() < a.len() {
            std::mem::swap(&mut a, &mut b);
        }
        if (b.len() - a.len()) as i64 * MOVE_COST > max_cost {
            return max_cost + 1;
        }
        let mut row: Vec<i64> = (1..=a.len() as i64).map(|i| i * MOVE_COST).collect();
        let mut result = 0;
        for (bindex, &bchar) in b.iter().enumerate() {
            let mut distance = bindex as i64 * MOVE_COST;
            result = distance;
            let mut minimum = i64::MAX;
            for (index, &achar) in a.iter().enumerate() {
                let substitute = distance + substitution_cost(bchar, achar);
                distance = row[index];
                let insert_delete = result.min(distance) + MOVE_COST;
                result = insert_delete.min(substitute);
                row[index] = result;
                minimum = minimum.min(result);
            }
            if minimum > max_cost {
                return max_cost + 1;
            }
        }
        result
    }

    if values.len() > MAX_CANDIDATE_ITEMS {
        return None;
    }
    let v: Vec<char> = value.chars().collect();
    if v.len() > MAX_STRING_SIZE {
        return None;
    }
    let mut best = v.len() as i64;
    let mut suggestion: Option<&String> = None;
    for name in values {
        if name == value {
            continue;
        }
        let n: Vec<char> = name.chars().collect();
        let max_distance = ((n.len() + v.len() + 3) as i64 * MOVE_COST / 6).min(best - 1);
        let d = levenshtein(&v, &n, max_distance);
        if d > max_distance {
            continue;
        }
        if suggestion.is_none_or(|s| s.is_empty()) || d < best {
            suggestion = Some(name);
            best = d;
        }
    }
    suggestion.cloned()
}

fn strings_of(vm: &mut Vm, v: &Value) -> PyResult<Vec<String>> {
    let mut rv = Vec::new();
    for x in py::items(vm, v)? {
        rv.push(str_arg(vm, &x)?);
    }
    Ok(rv)
}

fn f_compute_closest_value(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let value = py::arg_or(args, kw, 0, "value", Value::None);
    let values = py::arg_or(args, kw, 1, "values", Value::None);
    let value = str_arg(vm, &value)?;
    let values = strings_of(vm, &values)?;
    Ok(match compute_closest_value(&value, &values) {
        Some(s) => py::s(vm, &s),
        None => Value::None,
    })
}

fn dir_of(vm: &mut Vm, v: &Value) -> PyResult<Vec<String>> {
    let dir = py::import_attr(vm, "builtins", "dir")?;
    let d = vm.call(&dir, vec![v.clone()], Vec::new())?;
    strings_of(vm, &d)
}

fn did_you_mean(vm: &Vm, s: Option<String>) -> Value {
    match s {
        Some(s) => py::s(vm, &format!(" Did you mean: '{s}'?")),
        None => Value::None,
    }
}

fn f_handle_attribute_error(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = py::arg_or(args, kw, 0, "exception", Value::None);
    let name = py::get_or_none(vm, &e, "name")?;
    if py::is_none(&name) {
        return Ok(Value::None);
    }
    let name = str_arg(vm, &name)?;
    let d = match py::get(vm, &e, "obj").and_then(|o| dir_of(vm, &o)) {
        Ok(d) => d,
        Err(PyErr::Exc(_)) => return Ok(Value::None),
        Err(x) => return Err(x),
    };
    let d: Vec<String> = if !name.starts_with('_') {
        d.into_iter().filter(|x| !x.starts_with('_')).collect()
    } else {
        d
    };
    Ok(did_you_mean(vm, compute_closest_value(&name, &d)))
}

fn namespace_keys(vm: &mut Vm, module: &str) -> PyResult<Vec<String>> {
    let m = py::import(vm, module)?;
    let ns = py::namespace_of(vm, &m)?;
    strings_of(vm, &ns)
}

fn f_handle_name_error(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = py::arg_or(args, kw, 0, "exception", Value::None);
    let name = py::get_or_none(vm, &e, "name")?;
    if py::is_none(&name) {
        return Ok(Value::None);
    }
    let tb = py::get_or_none(vm, &e, "__traceback__")?;
    if py::is_none(&tb) {
        return Ok(Value::None);
    }
    let name = str_arg(vm, &name)?;
    let mut d: Vec<String> = Vec::new();
    for m in ["store", "builtins"] {
        for k in namespace_keys(vm, m).unwrap_or_default() {
            if !d.contains(&k) {
                d.push(k);
            }
        }
    }
    let stdlib = match py::import_attr(vm, "sys", "stdlib_module_names") {
        Ok(s) => {
            let n = py::s(vm, &name);
            ops::contains(vm, &s, &n).unwrap_or(false)
        }
        Err(_) => false,
    };
    let s = match compute_closest_value(&name, &d) {
        Some(s) if stdlib => {
            format!(" Did you mean: '{s}' or did you forget to import '{name}'?")
        }
        Some(s) => format!(" Did you mean: '{s}'?"),
        None if stdlib => format!(" Did you forget to import '{name}'?"),
        None => return Ok(Value::None),
    };
    Ok(py::s(vm, &s))
}

fn f_handle_import_error(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let e = py::arg_or(args, kw, 0, "exception", Value::None);
    let wrong = py::get_or_none(vm, &e, "name_from")?;
    let module = py::get_or_none(vm, &e, "name")?;
    if py::is_none(&wrong) || py::is_none(&module) {
        return Ok(Value::None);
    }
    let wrong = str_arg(vm, &wrong)?;
    let imp = py::import_attr(vm, "builtins", "__import__")?;
    let d = match vm
        .call(&imp, vec![module], Vec::new())
        .and_then(|m| dir_of(vm, &m))
    {
        Ok(d) => d,
        Err(PyErr::Exc(_)) => return Ok(Value::None),
        Err(x) => return Err(x),
    };
    let d: Vec<String> = if !wrong.starts_with('_') {
        d.into_iter().filter(|x| !x.starts_with('_')).collect()
    } else {
        d
    };
    Ok(did_you_mean(vm, compute_closest_value(&wrong, &d)))
}

// TracebackException ----------------------------------------------------------------

/// `_safe_string(value, what, func)`.
fn safe_string(vm: &mut Vm, v: &Value, what: &str, repr: bool) -> PyResult<String> {
    let r = if repr { ops::repr(vm, v) } else { ops::str(vm, v) };
    match r {
        Ok(s) => Ok(s.to_string()),
        Err(PyErr::Exc(_)) => Ok(format!(
            "<{what} {}() failed>",
            if repr { "repr" } else { "str" }
        )),
        Err(e) => Err(e),
    }
}

fn exc_id(vm: &mut Vm, e: &Value) -> PyResult<Value> {
    let id = py::import_attr(vm, "builtins", "id")?;
    vm.call(&id, vec![e.clone()], Vec::new())
}

fn base_exception_group(vm: &mut Vm) -> Option<ObjRef> {
    match py::import_attr(vm, "builtins", "BaseExceptionGroup") {
        Ok(Value::Obj(c)) => Some(c),
        _ => None,
    }
}

fn te_suggest(vm: &mut Vm, this: &Value, e: &Value) -> PyResult<()> {
    let config = py::import(vm, "renpy.config")?;
    let Some(handlers) = py::get_opt(vm, &config, "error_suggestion_handlers")? else {
        return Ok(());
    };
    if !py::truthy(vm, &handlers)? {
        return Ok(());
    }
    let items = py::method(vm, &handlers, "items", Vec::new())?;
    let mut suggestion = Value::None;
    for item in py::items(vm, &items)?.into_iter().rev() {
        let p = py::items(vm, &item)?;
        let Value::Obj(typ) = &p[0] else { continue };
        if ops::isinstance(vm, e, typ) {
            suggestion = vm.call(&p[1], vec![e.clone()], Vec::new())?;
            if py::truthy(vm, &suggestion)? {
                break;
            }
        }
    }
    if py::truthy(vm, &suggestion)? {
        let s = str_arg(vm, &suggestion)?;
        let s = s.trim_start_matches(py_space);
        let cur = py::get(vm, this, "_str")?;
        let cur = str_arg(vm, &cur)?;
        let new = if cur.ends_with('.') {
            format!("{cur} {s}")
        } else {
            format!("{cur}. {s}")
        };
        let v = py::s(vm, &new);
        py::set(vm, this, "_str", v)?;
    }
    Ok(())
}

fn te_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let e = py::arg_or(&args[1..], kw, 0, "exception", Value::None);
    let seen = py::kw(kw, "_seen").cloned().unwrap_or(Value::None);
    let tb = py::get_or_none(vm, &e, "__traceback__")?;
    let ss = py::import_attr(vm, ERROR, "StackSummary")?;
    let stack = vm.call(&ss, vec![tb], Vec::new())?;
    py::set(vm, this, "stack", stack)?;
    let s = safe_string(vm, &e, "exception", false)?;
    let s = py::s(vm, &s);
    py::set(vm, this, "_str", s)?;

    let notes = match py::get_or_none(vm, &e, "__notes__") {
        Ok(n) if py::is_none(&n) => Vec::new(),
        Ok(n) => match (n.as_str(), py::seq(&n)) {
            (None, Some(items)) => {
                let mut rv = Vec::new();
                for note in items {
                    rv.push(safe_string(vm, &note, "note", false)?);
                }
                rv
            }
            _ => vec![safe_string(vm, &n, "__notes__", false)?],
        },
        Err(PyErr::Exc(o)) => {
            let err = safe_string(vm, &Value::Obj(o), "__notes__", true)?;
            vec![format!("Ignored error getting __notes__: {err}")]
        }
        Err(x) => return Err(x),
    };
    let notes = notes.iter().map(|n| py::s(vm, n)).collect();
    let notes = vm.types.new_list(notes);
    py::set(vm, this, "__notes__", notes)?;

    py::set(vm, this, "_is_syntax_error", Value::Bool(false))?;
    let typ = Value::Obj(ops::type_of(vm, &e));
    py::set(vm, this, "_exc_type", typ.clone())?;
    let q = py::get(vm, &typ, "__qualname__")?;
    py::set(vm, this, "_exc_type_qualname", q)?;
    let m = py::get_or_none(vm, &typ, "__module__")?;
    py::set(vm, this, "_exc_type_module", m)?;

    let syntax = vm.types.exc("SyntaxError");
    if ops::isinstance(vm, &e, &syntax) {
        for k in [
            "filename",
            "lineno",
            "end_lineno",
            "text",
            "offset",
            "end_offset",
            "msg",
        ] {
            let v = py::get_or_none(vm, &e, k)?;
            py::set(vm, this, k, v)?;
        }
        py::set(vm, this, "_is_syntax_error", Value::Bool(true))?;
    } else {
        te_suggest(vm, this, &e)?;
    }

    let sc = py::get_or_none(vm, &e, "__suppress_context__")?;
    py::set(vm, this, "__suppress_context__", sc)?;
    for k in [
        "__cause__",
        "__context__",
        "exceptions",
        "simple",
        "full",
        "traceback_fn",
    ] {
        py::set(vm, this, k, Value::None)?;
    }

    let recursive = !py::is_none(&seen);
    let seen = if recursive {
        seen
    } else {
        let set = py::import_attr(vm, "builtins", "set")?;
        vm.call(&set, Vec::new(), Vec::new())?
    };
    let id = exc_id(vm, &e)?;
    py::method(vm, &seen, "add", vec![id])?;
    if recursive {
        return Ok(Value::None);
    }
    let cls = py::import_attr(vm, ERROR, "TracebackException")?;
    let child = |vm: &mut Vm, x: &Value| -> PyResult<Value> {
        vm.call(&cls, vec![x.clone()], vec![kwarg("_seen", seen.clone())])
    };
    let unseen = |vm: &mut Vm, x: &Value| -> PyResult<bool> {
        if py::is_none(x) {
            return Ok(false);
        }
        let id = exc_id(vm, x)?;
        Ok(!ops::contains(vm, &seen, &id)?)
    };
    let group = base_exception_group(vm);
    let mut queue = vec![(this.clone(), e.clone())];
    while let Some((mut te, e)) = queue.pop() {
        if py::is_none(&e) {
            continue;
        }
        let cause = py::get_or_none(vm, &e, "__cause__")?;
        if unseen(vm, &cause)? {
            let c = child(vm, &cause)?;
            py::set(vm, &te, "__cause__", c.clone())?;
            queue.push((c, cause));
        }
        let context = py::get_or_none(vm, &e, "__context__")?;
        if unseen(vm, &context)? {
            let c = child(vm, &context)?;
            py::set(vm, &te, "__context__", c.clone())?;
            queue.push((c, context));
        }
        if let Some(g) = &group
            && ops::isinstance(vm, &e, g)
        {
            let mut exceptions = Vec::new();
            let excs = py::get(vm, &e, "exceptions")?;
            for exc in py::items(vm, &excs)? {
                // Upstream rebinds `te` here, so the list lands on the
                // last nested exception.
                te = child(vm, &exc)?;
                exceptions.push(te.clone());
                queue.push((te.clone(), exc));
            }
            let l = vm.types.new_list(exceptions);
            py::set(vm, &te, "exceptions", l)?;
        }
    }
    Ok(Value::None)
}

fn exc_type_str(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let q = py::get(vm, this, "_exc_type_qualname")?;
    let q = str_arg(vm, &q)?;
    let m = py::get(vm, this, "_exc_type_module")?;
    let m = match m.as_str() {
        Some(s) => s.to_owned(),
        None => "<unknown>".to_owned(),
    };
    Ok(if m == "__main__" || m == "builtins" {
        q
    } else {
        format!("{m}.{q}")
    })
}

fn te_exc_type_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = exc_type_str(vm, &args[0])?;
    Ok(py::s(vm, &s))
}

fn te_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = py::import_attr(vm, ERROR, "TracebackException")?;
    let Value::Obj(c) = &cls else {
        return Ok(vm.types.not_implemented.clone());
    };
    if !ops::isinstance(vm, &args[1], c) {
        return Ok(vm.types.not_implemented.clone());
    }
    let a = py::get(vm, &args[0], "__dict__")?;
    let b = py::get(vm, &args[1], "__dict__")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn te_str(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get(vm, &args[0], "_str")
}

fn default_ctx(vm: &mut Vm, ctx: Value) -> PyResult<Value> {
    if !py::is_none(&ctx) {
        return Ok(ctx);
    }
    let io = new_string_io(vm)?;
    let c = py::import_attr(vm, ERROR, "NonColoredExceptionPrintContext")?;
    vm.call(&c, vec![io], Vec::new())
}

fn te_format_exception_only(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ctx = py::arg_or(&args[1..], kw, 0, "ctx", Value::None);
    let ctx = default_ctx(vm, ctx)?;
    let show_group = py::kw(kw, "show_group").cloned().unwrap_or(Value::Bool(false));
    let syntax = py::get(vm, this, "_is_syntax_error")?;
    if py::truthy(vm, &syntax)? {
        py::method(vm, this, "_format_syntax_error", vec![ctx.clone()])?;
    } else {
        let t = exc_type_str(vm, this)?;
        let t = py::s(vm, &t);
        let s = py::get(vm, this, "_str")?;
        let s = if py::truthy(vm, &s)? { s } else { Value::None };
        py::method(vm, &ctx, "final_exception_line", vec![t, s])?;
    }
    for note in py::get_items(vm, this, "__notes__")? {
        py::method(vm, &ctx, "string", vec![note])?;
    }
    let exceptions = py::get(vm, this, "exceptions")?;
    if py::truthy(vm, &show_group)? && py::truthy(vm, &exceptions)? {
        with_indent(vm, &ctx, |vm| {
            for ex in py::items(vm, &exceptions)? {
                py::method_kw(
                    vm,
                    &ex,
                    "format_exception_only",
                    vec![ctx.clone()],
                    vec![kwarg("show_group", show_group.clone())],
                )?;
            }
            Ok(())
        })?;
    }
    py::method(vm, &ctx, "getvalue", Vec::new())
}

/// `TracebackException._format_syntax_error(ctx)`.
fn te_format_syntax_error(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ctx = &args[1];
    let attr = |vm: &mut Vm, k: &str| py::get_or_none(vm, this, k);
    with_indent(vm, ctx, |vm| {
        let f = attr(vm, "filename")?;
        let filename = if py::truthy(vm, &f)? {
            str_arg(vm, &f)?
        } else {
            "<string>".to_owned()
        };
        let lineno = attr(vm, "lineno")?;
        if !py::is_none(&lineno) {
            let fv = py::s(vm, &filename);
            py::method(vm, ctx, "location", vec![fv, lineno.clone(), Value::None])?;
        }
        with_indent(vm, ctx, |vm| {
            let text = attr(vm, "text")?;
            let Some(text) = text.as_str().map(str::to_owned) else {
                return Ok(());
            };
            let rtext = text.trim_end_matches('\n');
            let ltext = rtext.trim_start_matches([' ', '\n', '\x0c']);
            let spaces = rtext.chars().count() as i64 - ltext.chars().count() as i64;
            let offset = attr(vm, "offset")?;
            let lt = py::s(vm, ltext);
            if py::is_none(&offset) {
                py::method(vm, ctx, "source_carets", vec![lt, Value::None])?;
                return Ok(());
            }
            if !is_int(&offset) {
                return Ok(());
            }
            let raw = py::int(vm, &offset)?;
            let mut offset = if py::is_none(&lineno) {
                raw
            } else {
                let l = py::int(vm, &lineno)?;
                normalize_offset(vm, &filename, l, raw, rtext)?
            };
            let end_lineno = attr(vm, "end_lineno")?;
            let same_line = match (&lineno, &end_lineno) {
                (Value::None, Value::None) => true,
                (a, b) => ops::eq(vm, a, b)?,
            };
            let rlen = rtext.chars().count() as i64;
            let mut end_offset = if same_line {
                let eo = attr(vm, "end_offset")?;
                if py::truthy(vm, &eo)? {
                    py::int(vm, &eo)?
                } else {
                    offset
                }
            } else {
                rlen + 1
            };
            if !py::is_none(&end_lineno) {
                let l = py::int(vm, &end_lineno)?;
                end_offset = normalize_offset(vm, &filename, l, end_offset, rtext)?;
            }
            let tlen = text.chars().count() as i64;
            if !text.is_empty() && offset > tlen {
                offset = rlen + 1;
            }
            if !text.is_empty() && end_offset > tlen {
                end_offset = rlen + 1;
            }
            if offset >= end_offset || end_offset < 0 {
                end_offset = offset + 1;
            }
            let colno = offset - 1 - spaces;
            let end_colno = end_offset - 1 - spaces;
            let carets = if colno >= 0 {
                let c: String = ltext
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        let i = i as i64;
                        if colno <= i && i < end_colno {
                            '^'
                        } else if py_space(c) {
                            c
                        } else {
                            ' '
                        }
                    })
                    .collect();
                py::s(vm, &c)
            } else {
                Value::None
            };
            py::method(vm, ctx, "source_carets", vec![lt, carets])?;
            Ok(())
        })
    })?;
    let msg = attr(vm, "msg")?;
    let msg = if py::truthy(vm, &msg)? {
        msg
    } else {
        py::s(vm, "<no detail available>")
    };
    let t = exc_type_str(vm, this)?;
    let t = py::s(vm, &t);
    py::method(vm, ctx, "final_exception_line", vec![t, msg])?;
    Ok(Value::None)
}

/// `TracebackException.format(ctx=None, *, chain=True, show_group=False)`.
fn te_format(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ctx = py::arg_or(&args[1..], kw, 0, "ctx", Value::None);
    let ctx = default_ctx(vm, ctx)?;
    let chain = py::kw(kw, "chain").cloned().unwrap_or(Value::Bool(true));
    let chain_b = py::truthy(vm, &chain)?;
    let mut output: Vec<(Option<&str>, Value)> = Vec::new();
    let mut exc = args[0].clone();
    if chain_b {
        while py::truthy(vm, &exc)? {
            let cause = py::get(vm, &exc, "__cause__")?;
            let context = py::get(vm, &exc, "__context__")?;
            let suppress = py::get(vm, &exc, "__suppress_context__")?;
            let (meth, next) = if !py::is_none(&cause) {
                (Some("chain_cause"), cause)
            } else if !py::is_none(&context) && !py::truthy(vm, &suppress)? {
                (Some("chain_context"), context)
            } else {
                (None, Value::None)
            };
            output.push((meth, exc));
            exc = next;
        }
    } else {
        output.push((None, exc));
    }
    let int = |vm: &mut Vm, k: &str| int_attr(vm, &ctx, k);
    let set = |vm: &mut Vm, k: &str, v: Value| py::set(vm, &ctx, k, v);
    for (meth, exc) in output.into_iter().rev() {
        if let Some(m) = meth {
            py::method(vm, &ctx, m, Vec::new())?;
            set(vm, "emit_context", Value::Bool(true))?;
        }
        let exceptions = py::get(vm, &exc, "exceptions")?;
        let stack = py::get(vm, &exc, "stack")?;
        let stack_shown = |vm: &mut Vm| -> PyResult<bool> {
            let r = py::method(vm, &stack, "should_filter", vec![ctx.clone()])?;
            Ok(!py::truthy(vm, &r)?)
        };
        let emit = |vm: &mut Vm| -> PyResult<bool> {
            let e = py::get(vm, &ctx, "emit_context")?;
            py::truthy(vm, &e)
        };
        if py::is_none(&exceptions) {
            if stack_shown(vm)? {
                if emit(vm)? {
                    let t = py::s(vm, "Traceback (most recent call last):");
                    py::method(vm, &ctx, "string", vec![t])?;
                }
                py::method(vm, &stack, "format", vec![ctx.clone()])?;
            }
            py::method(vm, &exc, "format_exception_only", vec![ctx.clone()])?;
            continue;
        }
        let depth = int(vm, "exception_group_depth")?;
        let max_depth = int(vm, "max_group_depth")?;
        if depth > max_depth {
            let t = py::s(vm, &format!("... (max group depth is {max_depth})"));
            py::method(vm, &ctx, "string", vec![t])?;
            continue;
        }
        let toplevel = depth == 0;
        if toplevel {
            set(vm, "exception_group_depth", Value::Int(depth + 1))?;
        }
        if stack_shown(vm)? {
            if emit(vm)? {
                let t = py::s(vm, "Exception Group Traceback (most recent call last):");
                py::method(vm, &ctx, "string", vec![t])?;
            }
            py::method(vm, &stack, "format", vec![ctx.clone()])?;
            py::method(vm, &exc, "format_exception_only", vec![ctx.clone()])?;
        }
        let excs = py::items(vm, &exceptions)?;
        let num = excs.len() as i64;
        let width = int(vm, "max_group_width")?;
        let n = if num <= width { num } else { width + 1 };
        set(vm, "need_close", Value::Bool(false))?;
        for i in 0..n {
            let last_exc = i == n - 1;
            if last_exc {
                set(vm, "need_close", Value::Bool(true))?;
            }
            let truncated = i >= width;
            with_indent(vm, &ctx, |vm| {
                py::method(
                    vm,
                    &ctx,
                    "exceptions_separator",
                    vec![Value::Int(i), Value::Int(n)],
                )?;
                let d = int(vm, "exception_group_depth")?;
                set(vm, "exception_group_depth", Value::Int(d + 1))?;
                if !truncated {
                    py::method_kw(
                        vm,
                        &excs[i as usize],
                        "format",
                        vec![ctx.clone()],
                        vec![kwarg("chain", chain.clone())],
                    )?;
                } else {
                    let remaining = num - width;
                    let plural = if remaining > 1 { "s" } else { "" };
                    let t = py::s(vm, &format!("and {remaining} more exception{plural}"));
                    py::method(vm, &ctx, "string", vec![t])?;
                }
                let nc = py::get(vm, &ctx, "need_close")?;
                if last_exc && py::truthy(vm, &nc)? {
                    py::method(vm, &ctx, "exceptions_close", Vec::new())?;
                    set(vm, "need_close", Value::Bool(false))?;
                }
                Ok(())
            })?;
            let d = int(vm, "exception_group_depth")?;
            set(vm, "exception_group_depth", Value::Int(d - 1))?;
        }
        if toplevel {
            set(vm, "exception_group_depth", Value::Int(0))?;
        }
    }
    py::method(vm, &ctx, "getvalue", Vec::new())
}

fn te_triple(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let s = py::get(vm, this, "simple")?;
    let f = py::get(vm, this, "full")?;
    let t = py::get(vm, this, "traceback_fn")?;
    Ok(vm.types.new_tuple(vec![s, f, t]))
}

fn te_getitem(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = te_triple(vm, &args[0])?;
    ops::getitem(vm, &t, args[1].clone())
}

fn te_iter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = te_triple(vm, &args[0])?;
    let items = py::items(vm, &t)?;
    let l = vm.types.new_list(items);
    ops::iter(vm, &l)
}

fn te_len(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(3))
}

// report_exception ------------------------------------------------------------------

fn format_into(vm: &mut Vm, te: &Value, cls: &str, file: &Value, kw: &[(&str, bool)]) -> PyResult<()> {
    let c = py::import_attr(vm, ERROR, cls)?;
    let kwargs = kw.iter().map(|(k, v)| kwarg(k, Value::Bool(*v))).collect();
    let ctx = vm.call(&c, vec![file.clone()], kwargs)?;
    py::method(vm, te, "format", vec![ctx])?;
    Ok(())
}

fn write(vm: &mut Vm, file: &Value, text: &str) -> PyResult<()> {
    let t = py::s(vm, text);
    py::method(vm, file, "write", vec![t])?;
    Ok(())
}

fn getvalue(vm: &mut Vm, io: &Value) -> PyResult<String> {
    let v = py::method(vm, io, "getvalue", Vec::new())?;
    str_arg(vm, &v)
}

/// Swallows Python exceptions as upstream's `try: ... except Exception:
/// pass` blocks do; suspension still propagates.
fn quietly(r: PyResult<()>) -> PyResult<()> {
    match r {
        Err(PyErr::Suspend(s)) => Err(PyErr::Suspend(s)),
        _ => Ok(()),
    }
}

/// `report_exception(e, editor=True)` -> `TracebackException` carrying
/// `simple`, `full` and `traceback_fn`.
fn f_report_exception(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut e = py::arg_or(args, kw, 0, "e", Value::None);
    let exception = vm.types.exc("Exception");
    if !ops::isinstance(vm, &e, &exception) {
        e = vm.call(&Value::Obj(exception), vec![e], Vec::new())?;
    }
    let flag = environ(vm, "RENPY_REPORT_EXCEPTIONS")?;
    if !py::is_none(&flag) {
        let int = py::import_attr(vm, "builtins", "int")?;
        let n = vm.call(&int, vec![flag], Vec::new())?;
        if !py::truthy(vm, &n)? {
            return match e {
                Value::Obj(o) => Err(PyErr::Exc(o)),
                _ => Ok(Value::None),
            };
        }
    }
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "debug_sound", Value::Bool(false))?;
    let cls = py::import_attr(vm, ERROR, "TracebackException")?;
    let te = vm.call(&cls, vec![e], Vec::new())?;

    let info = py::import_attr(vm, "renpy.game", "exception_info")?;
    let info = ops::str(vm, &info)?.to_string();
    const PLAIN: &str = "NonColoredExceptionPrintContext";
    const MAYBE: &str = "MaybeColoredExceptionPrintContext";
    let simple_io = new_string_io(vm)?;
    let full_io = new_string_io(vm)?;
    write(vm, &simple_io, &format!("{info}\n"))?;
    format_into(
        vm,
        &te,
        PLAIN,
        &simple_io,
        &[("filter_private", true), ("emit_context", false)],
    )?;
    format_into(vm, &te, PLAIN, &full_io, &[("filter_private", false)])?;

    quietly((|| {
        let out = py::import_attr(vm, "sys", "stdout")?;
        write(vm, &out, "\nFull traceback:\n")?;
        format_into(
            vm,
            &te,
            MAYBE,
            &out,
            &[("filter_private", false), ("emit_context", false)],
        )?;
        write(vm, &out, &format!("\n{info}\n"))?;
        format_into(
            vm,
            &te,
            MAYBE,
            &out,
            &[("filter_private", true), ("emit_context", false)],
        )
    })())?;

    write(vm, &full_io, "\n")?;
    quietly((|| {
        let v = py::import_attr(vm, "renpy", "version")?;
        let v = py::string(vm, &v)?;
        let gname = py::config(vm, "name")?;
        let gver = py::config(vm, "version")?;
        let game = format!("{} {}", ops::str(vm, &gname)?, ops::str(vm, &gver)?);
        let t = ctime(vm)?;
        write(vm, &full_io, &format!("CrySirror\n{v}\n{game}\n{t}\n"))
    })())?;

    let simple = getvalue(vm, &simple_io)?;
    let full = getvalue(vm, &full_io)?;
    let sv = py::s(vm, &simple);
    py::set(vm, &te, "simple", sv)?;
    let fv = py::s(vm, &full);
    py::set(vm, &te, "full", fv.clone())?;

    let file = format!(
        "I'm sorry, but an uncaught exception occurred.\n\n{simple}\n-- Full Traceback ------------------------------------------------------------\n\n{full}"
    );
    log::push(vm, "traceback", &file);
    super::link::diagnostic(vm, format!("renpy exception: {simple}\n{full}"));
    let n = py::s(vm, "traceback.txt");
    py::set(vm, &te, "traceback_fn", n)?;

    // Keeps the state at the error as `_tracesave-1` (8.4+).
    quietly((|| {
        let session = py::import_attr(vm, "renpy", "session")?;
        let k = py::s(vm, "traceback_load");
        let loading = py::method(vm, &session, "get", vec![k, Value::Bool(false)])?;
        if py::truthy(vm, &loading)? {
            return Ok(());
        }
        let p = py::s(vm, "_tracesave-");
        call_attr(
            vm,
            "renpy.loadsave",
            "cycle_saves",
            vec![p, Value::Int(10)],
            Vec::new(),
        )?;
        let extra = vm.types.new_dict();
        py::dict_set(vm, &extra, "_traceback", fv.clone())?;
        let mut kwargs = vec![kwarg("extra_json", extra)];
        if at_least(vm, 8, 5) {
            let shot = py::config(vm, "tracesave_screenshot")?;
            kwargs.push(kwarg("include_screenshot", shot));
        }
        let slot = py::s(vm, "_tracesave-1");
        call_attr(vm, "renpy.loadsave", "save", vec![slot], kwargs)?;
        Ok(())
    })())?;
    Ok(te)
}

fn ctime(vm: &mut Vm) -> PyResult<String> {
    let c = py::import_attr(vm, "time", "ctime")?;
    let v = vm.call(&c, Vec::new(), Vec::new())?;
    py::string(vm, &v)
}

fn call_attr(
    vm: &mut Vm,
    module: &str,
    name: &str,
    args: Vec<Value>,
    kwargs: Vec<(Rc<str>, Value)>,
) -> PyResult<Value> {
    let f = py::import_attr(vm, module, name)?;
    vm.call(&f, args, kwargs)
}

/// Whether `config.exception_handler` takes the `TracebackException`
/// (8.4) rather than the three report strings: the
/// `inspect.signature(handler).bind(te)` check over the handler's code.
pub fn handler_takes_te(vm: &mut Vm, handler: &Value) -> PyResult<bool> {
    let (func, bound) = match py::get_opt(vm, handler, "__func__")? {
        Some(f) => (f, 1),
        None => (handler.clone(), 0),
    };
    let Some(code) = py::get_opt(vm, &func, "__code__")? else {
        return Ok(true);
    };
    let argc = int_attr(vm, &code, "co_argcount")? - bound;
    let flags = int_attr(vm, &code, "co_flags")?;
    let defaults = py::get_or_none(vm, &func, "__defaults__")?;
    let ndefaults = if py::is_none(&defaults) {
        0
    } else {
        ops::len(vm, &defaults)? as i64
    };
    let varargs = flags & 0x04 != 0;
    let required = (argc - ndefaults).max(0);
    let kwonly = py::get_or_none(vm, &code, "co_kwonlyargcount")?;
    let kwonly = if py::is_none(&kwonly) { 0 } else { py::int(vm, &kwonly)? };
    let kwdefaults = py::get_or_none(vm, &func, "__kwdefaults__")?;
    let nkwdefaults = if py::is_none(&kwdefaults) {
        0
    } else {
        ops::len(vm, &kwdefaults)? as i64
    };
    Ok(required <= 1 && (argc >= 1 || varargs) && kwonly <= nkwdefaults)
}

/// `Context.handle_exception()` of 8.4+: the exception (already carrying
/// the context's `report_traceback` entries) goes to the context's
/// handler, `config.exception_handler` or the error screen. Errors the
/// handlers raise propagate, chained to the original exception.
pub fn handle_exception(vm: &mut Vm, context: &Value, e: PyErr) -> PyResult<()> {
    let PyErr::Exc(orig) = &e else {
        return Err(e);
    };
    let orig = orig.clone();
    let chain = |vm: &mut Vm, err: PyErr| -> PyErr {
        if let PyErr::Exc(o) = &err
            && !Rc::ptr_eq(o, &orig)
        {
            let ev = Value::Obj(o.clone());
            if py::get_or_none(vm, &ev, "__context__").is_ok_and(|c| py::is_none(&c)) {
                let _ = py::set(vm, &ev, "__context__", Value::Obj(orig.clone()));
            }
        }
        err
    };
    let r = (|| -> PyResult<()> {
        let report = py::import_attr(vm, ERROR, "report_exception")?;
        let te = vm.call(
            &report,
            vec![Value::Obj(orig.clone())],
            vec![kwarg("editor", Value::Bool(false))],
        )?;
        let h = py::get_or_none(vm, context, "exception_handler")?;
        if !py::is_none(&h) {
            vm.call(&h, vec![te], Vec::new())?;
            return Ok(());
        }
        let ch = py::config(vm, "exception_handler")?;
        if !py::is_none(&ch) {
            let r = if handler_takes_te(vm, &ch)? {
                vm.call(&ch, vec![te.clone()], Vec::new())?
            } else {
                let parts = py::items(vm, &te)?;
                vm.call(&ch, parts, Vec::new())?
            };
            if py::truthy(vm, &r)? {
                return Ok(());
            }
        }
        call_attr(vm, DISPLAY_ERROR, "report_exception", vec![te], Vec::new())?;
        Ok(())
    })();
    r.map_err(|err| chain(vm, err))
}

// renpy.display.error ---------------------------------------------------------------

/// `init_display()` of 8.4+: no `config.gl2` from the persistent, and the
/// loader's `index_files` (the host mounts files, so the runtime's
/// `index_archives` is the same no-op index).
fn f_init_display(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let prefs = py::import_attr(vm, "renpy.game", "preferences")?;
    py::method(vm, &prefs, "check", Vec::new())?;
    let iss = py::config(vm, "init_system_styles")?;
    if !py::is_none(&iss) {
        vm.call(&iss, Vec::new(), Vec::new())?;
    }
    let interface = py::import_attr(vm, "renpy.game", "interface")?;
    if !py::truthy(vm, &interface)? {
        call_attr(vm, "renpy.display.core", "Interface", Vec::new(), Vec::new())?;
        let loader = py::import(vm, "renpy.loader")?;
        let index = match py::get_opt(vm, &loader, "index_files")? {
            Some(f) => f,
            None => py::get(vm, &loader, "index_archives")?,
        };
        vm.call(&index, Vec::new(), Vec::new())?;
        let cache = py::import_attr(vm, "renpy.display.im", "cache")?;
        py::method(vm, &cache, "init", Vec::new())?;
    }
    let interface = py::import_attr(vm, "renpy.game", "interface")?;
    py::method(vm, &interface, "start", Vec::new())?;
    call_attr(vm, "renpy.ui", "reset", Vec::new(), Vec::new())?;
    Ok(Value::None)
}

/// The checks before showing an error screen: true when it cannot be
/// shown.
fn cannot_show(vm: &mut Vm, screen: &str) -> PyResult<bool> {
    let m = py::import(vm, DISPLAY_ERROR)?;
    py::set(vm, &m, "error_handled", Value::Bool(true))?;
    call_attr(vm, "renpy.dump", "dump", vec![Value::Bool(true)], Vec::new())?;
    let args = py::import_attr(vm, "renpy.game", "args")?;
    let cmd = py::get_or_none(vm, &args, "command")?;
    if cmd.as_str() != Some("run") {
        return Ok(true);
    }
    let v = environ(vm, "RENPY_SIMPLE_EXCEPTIONS")?;
    if !py::is_none(&v) {
        return Ok(true);
    }
    let s = py::s(vm, screen);
    let has = call_attr(vm, "renpy.exports", "has_screen", vec![s], Vec::new())?;
    Ok(!py::truthy(vm, &has)?)
}

/// `report_exception(traceback_exception)`: shows `_exception` with the
/// `TracebackException`; True when the screen cannot be shown.
fn f_report_exception_screen(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let te = py::arg_or(args, kw, 0, "traceback_exception", Value::None);
    if cannot_show(vm, "_exception")? {
        return Ok(Value::Bool(true));
    }
    if quietly_failed(f_init_display(vm, &[], &[]))? {
        return Ok(Value::Bool(true));
    }
    let draw = py::import_attr(vm, "renpy.display", "draw")?;
    if py::is_none(&draw) {
        return Ok(Value::Bool(true));
    }
    let store = py::import(vm, "store")?;
    py::set(vm, &store, "te", te.clone())?;
    let (mut ignore, mut rollback, mut reload) = (Value::None, Value::None, Value::None);
    quietly((|| {
        let ctx = call_attr(vm, "renpy.game", "context", Vec::new(), Vec::new())?;
        let init_phase = py::get(vm, &ctx, "init_phase")?;
        if !py::truthy(vm, &init_phase)? {
            if py::config_bool(vm, "rollback_enabled")? {
                rollback = py::import_attr(vm, DISPLAY_ERROR, "rollback_action")?;
            }
            let l = py::s(vm, "_save_reload_game");
            reload = call_attr(
                vm,
                "renpy.exports",
                "curried_call_in_new_context",
                vec![l],
                Vec::new(),
            )?;
        } else {
            reload = py::import_attr(vm, "renpy.exports", "utter_restart")?;
        }
        let nn = py::get(vm, &ctx, "next_node")?;
        if !py::is_none(&nn) {
            ignore = call_attr(
                vm,
                "renpy.ui",
                "returns",
                vec![Value::Bool(false)],
                Vec::new(),
            )?;
        }
        Ok(())
    })())?;
    let screen = py::import_attr(vm, DISPLAY_ERROR, "call_exception_screen")?;
    let name = py::s(vm, "_exception");
    call_attr(
        vm,
        "renpy.game",
        "invoke_in_new_context",
        vec![screen, name],
        vec![
            kwarg("traceback_exception", te),
            kwarg("rollback_action", rollback),
            kwarg("reload_action", reload),
            kwarg("ignore_action", ignore),
        ],
    )?;
    let im = py::import(vm, "renpy.display.im")?;
    let ignored = py::get(vm, &im, "ignored_images")?;
    let to_ignore = py::get(vm, &im, "images_to_ignore")?;
    py::method(vm, &ignored, "update", vec![to_ignore])?;
    let config = py::import(vm, "renpy.config")?;
    py::set(vm, &config, "raise_image_exceptions", Value::Bool(false))?;
    // 8.6 keeps the flag in `renpy.display.im.thread_local`; the runtime's
    // image loader reads `config.raise_image_load_exceptions`.
    match py::get_opt(vm, &im, "thread_local")? {
        Some(tl) if at_least(vm, 8, 6) => {
            py::set(vm, &tl, "raise_image_load_exceptions", Value::Bool(false))?
        }
        _ => py::set(
            vm,
            &config,
            "raise_image_load_exceptions",
            Value::Bool(false),
        )?,
    }
    let ia = py::get_or_none(vm, &store, "_ignore_action")?;
    if !py::is_none(&ia) {
        call_attr(vm, "renpy.display.behavior", "run", vec![ia], Vec::new())?;
    }
    Ok(Value::Bool(false))
}

fn quietly_failed(r: PyResult<Value>) -> PyResult<bool> {
    match r {
        Ok(_) => Ok(false),
        Err(PyErr::Suspend(s)) => Err(PyErr::Suspend(s)),
        Err(_) => Ok(true),
    }
}

fn is_control(vm: &mut Vm, e: &PyErr) -> bool {
    let PyErr::Exc(o) = e else { return true };
    match py::import_attr(vm, "renpy.game", "CONTROL_EXCEPTIONS") {
        Ok(c) => ops::exc_matches(vm, &Value::Obj(o.clone()), &c).unwrap_or(false),
        Err(_) => false,
    }
}

fn f_report_parse_errors(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let errors = py::arg_or(args, kw, 0, "errors", Value::None);
    let error_fn = py::arg_or(args, kw, 1, "error_fn", Value::None);
    if cannot_show(vm, "_parse_errors")? {
        return Ok(Value::Bool(true));
    }
    if py::is_none(&py::config(vm, "savedir")?) {
        return Ok(Value::Bool(true));
    }
    f_init_display(vm, &[], &[])?;
    let reload = py::import_attr(vm, "renpy.exports", "utter_restart")?;
    let screen = py::import_attr(vm, DISPLAY_ERROR, "call_exception_screen")?;
    let name = py::s(vm, "_parse_errors");
    let kwargs = vec![
        kwarg("reload_action", reload),
        kwarg("errors", errors),
        kwarg("error_fn", error_fn),
    ];
    match call_attr(
        vm,
        "renpy.game",
        "invoke_in_new_context",
        vec![screen, name],
        kwargs,
    ) {
        Ok(_) => Ok(Value::Bool(false)),
        Err(e) if is_control(vm, &e) => Err(e),
        Err(e) => {
            if let Ok(d) = py::import(vm, "renpy.display")
                && let Ok(Some(l)) = py::get_opt(vm, &d, "log")
            {
                let s = py::s(vm, "While handling exception:");
                let _ = py::method(vm, &l, "write", vec![s]);
                let _ = py::method(vm, &l, "exception", Vec::new());
            }
            Err(e)
        }
    }
}

// Installation ----------------------------------------------------------------------

fn set(vm: &Vm, m: &ObjRef, name: &str, v: Value) {
    crysirror_python::modules::set(vm, m, name, v);
}

/// Adds the 8.4+ API to `renpy.error` (`m`), replacing `report_exception`.
pub fn install_error(vm: &mut Vm, m: &ObjRef) -> PyResult<()> {
    for (n, f) in [
        ("report_exception", f_report_exception as crysirror_python::object::NativeFn),
        ("MaybeColoredExceptionPrintContext", f_maybe_colored),
        ("normalize_renpy_line_offset", f_normalize_renpy_line_offset),
        ("compute_closest_value", f_compute_closest_value),
        ("handle_attribute_error", f_handle_attribute_error),
        ("handle_name_error", f_handle_name_error),
        ("handle_import_error", f_handle_import_error),
    ] {
        let b = vm.types.new_builtin(n, f);
        set(vm, m, n, b);
    }
    let c = py::s(vm, CAUSE_MESSAGE);
    set(vm, m, "CAUSE_MESSAGE", c);
    let c = py::s(vm, CONTEXT_MESSAGE);
    set(vm, m, "CONTEXT_MESSAGE", c);

    let indent = py::class(
        vm,
        ERROR,
        "_Indent",
        &[],
        &[
            ("__init__", indent_init),
            ("__enter__", indent_enter),
            ("__exit__", indent_exit),
        ],
        &[],
    )?;
    set(vm, m, "_Indent", Value::Obj(indent));

    let epc = py::class(
        vm,
        ERROR,
        "ExceptionPrintContext",
        &[],
        &[
            ("__init__", ec_init),
            ("should_filter", ec_should_filter),
            ("indent", ec_indent),
            ("getvalue", ec_abstract),
            ("location", ec_abstract),
            ("source_carets", ec_abstract),
            ("final_exception_line", ec_abstract),
            ("string", ec_abstract),
            ("chain_cause", ec_abstract),
            ("chain_context", ec_abstract),
            ("exceptions_separator", ec_abstract),
            ("exceptions_close", ec_abstract),
        ],
        &[],
    )?;
    set(vm, m, "ExceptionPrintContext", Value::Obj(epc.clone()));

    let tio = py::class(
        vm,
        ERROR,
        "TextIOExceptionPrintContext",
        &[Value::Obj(epc)],
        &[
            ("__init__", tio_init),
            ("getvalue", tio_getvalue),
            ("_print", tio_print),
            ("string", tio_string),
            ("chain_cause", tio_chain_cause),
            ("chain_context", tio_chain_context),
            ("exceptions_separator", tio_exceptions_separator),
            ("exceptions_close", tio_exceptions_close),
        ],
        &[],
    )?;
    set(vm, m, "TextIOExceptionPrintContext", Value::Obj(tio.clone()));

    let colors: Vec<(&str, Value)> = ANSI_COLORS.iter().map(|(n, c)| (*n, py::s(vm, c))).collect();
    let ansi = py::class(vm, ERROR, "ANSIColors", &[], &[], &colors)?;
    set(vm, m, "ANSIColors", Value::Obj(ansi));

    let color = |vm: &Vm, n: &str| {
        py::s(
            vm,
            ANSI_COLORS.iter().find(|(k, _)| *k == n).map_or("", |(_, c)| c),
        )
    };
    let attrs = [
        ("LOCATION_COLOR", color(vm, "INTENSE_BLUE")),
        ("PRIMARY_CARET_COLOR", color(vm, "RED")),
        ("SECONDARY_CARET_COLOR", color(vm, "INTENSE_RED")),
        ("EXCEPTION_TYPE_COLOR", color(vm, "INTENSE_WHITE")),
        ("EXCEPTION_VALUE_COLOR", color(vm, "RESET")),
    ];
    let ac = py::class(
        vm,
        ERROR,
        "ANSIColoredPrintContext",
        &[Value::Obj(tio.clone())],
        &[
            ("location", ac_location),
            ("source_carets", ac_source_carets),
            ("final_exception_line", ac_final_exception_line),
        ],
        &attrs,
    )?;
    set(vm, m, "ANSIColoredPrintContext", Value::Obj(ac));

    let dw = vm.types.new_builtin("_display_width", nc_display_width);
    let dw = py::staticmethod(vm, dw);
    let nc = py::class(
        vm,
        ERROR,
        "NonColoredExceptionPrintContext",
        &[Value::Obj(tio)],
        &[
            ("location", nc_location),
            ("source_carets", nc_source_carets),
            ("final_exception_line", nc_final_exception_line),
        ],
        &[("_display_width", dw)],
    )?;
    set(vm, m, "NonColoredExceptionPrintContext", Value::Obj(nc));

    let mut props = Vec::new();
    for (n, f) in [
        ("line", fs_line as crysirror_python::object::NativeFn),
        ("lines", fs_lines),
        ("_anchors", fs_anchors),
        ("carets", fs_carets),
        ("significant_lines", fs_significant_lines),
    ] {
        let g = vm.types.new_builtin(n, f);
        props.push((n, py::property(vm, g, None)?));
    }
    let fs = py::class(
        vm,
        ERROR,
        "FrameSummary",
        &[],
        &[
            ("__init__", fs_init),
            ("__repr__", fs_repr),
            ("format", fs_format),
        ],
        &props,
    )?;
    set(vm, m, "FrameSummary", Value::Obj(fs));

    let list = Value::Obj(vm.types.list.clone());
    let ss = py::class(
        vm,
        ERROR,
        "StackSummary",
        &[list],
        &[
            ("__init__", ss_init),
            ("should_filter", ss_should_filter),
            ("format", ss_format),
        ],
        &[],
    )?;
    set(vm, m, "StackSummary", Value::Obj(ss));

    let ets = vm.types.new_builtin("exc_type_str", te_exc_type_str);
    let ets = py::property(vm, ets, None)?;
    let te = py::class(
        vm,
        ERROR,
        "TracebackException",
        &[],
        &[
            ("__init__", te_init),
            ("__eq__", te_eq),
            ("__str__", te_str),
            ("format_exception_only", te_format_exception_only),
            ("_format_syntax_error", te_format_syntax_error),
            ("format", te_format),
            ("__getitem__", te_getitem),
            ("__iter__", te_iter),
            ("__len__", te_len),
        ],
        &[("exc_type_str", ets), ("__hash__", Value::None)],
    )?;
    set(vm, m, "TracebackException", Value::Obj(te));
    Ok(())
}

/// Replaces the 8.3 entry points of `renpy.display.error` (`m`).
pub fn install_display_error(vm: &mut Vm, m: &ObjRef) {
    for (n, f) in [
        ("init_display", f_init_display as crysirror_python::object::NativeFn),
        ("report_exception", f_report_exception_screen),
        ("report_parse_errors", f_report_parse_errors),
    ] {
        let b = vm.types.new_builtin(n, f);
        set(vm, m, n, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_values_follow_cpython_suggestions() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            compute_closest_value("fooo", &v(&["foo", "bar"])).as_deref(),
            Some("foo")
        );
        assert_eq!(
            compute_closest_value("Foo", &v(&["foo", "fo"])).as_deref(),
            Some("foo")
        );
        assert_eq!(compute_closest_value("abc", &v(&["xyz"])), None);
        assert_eq!(compute_closest_value("abc", &v(&["abc"])), None);
    }

    #[test]
    fn munging_matches_the_string_munger() {
        let p = "_m1_script__";
        assert_eq!(munge_string("x = __a + b.__c", p, true), "x = _m1_script__a + b._m1_script__c");
        assert_eq!(munge_string("x = __a__ + ___b", p, true), "x = __a__ + ___b");
        assert_eq!(munge_string("a.__b [__c] [[__d]]", p, false), "a._m1_script__b [_m1_script__c] [[__d]]");
    }

    #[test]
    fn helpers_follow_python() {
        assert_eq!(dedent("    a\n      b\n"), "a\n  b\n");
        assert_eq!(dedent("\ta\n  b"), "\ta\n  b");
        assert_eq!(widen_carets("aあb", "~^~"), "~^^~");
        assert_eq!(significant_lines(1, None), vec![0]);
        assert_eq!(significant_lines(5, None), vec![0, 4]);
        assert_eq!(
            significant_lines(9, Some(&[(0, 1), (0, 4), (6, 2), (8, 3)])),
            vec![0, 1, 5, 6, 7, 8]
        );
        assert_eq!(
            ren_py_to_rpy_offsets(&[
                "x = 1\n".into(),
                "\"\"\"renpy\n".into(),
                "init python:\n".into(),
                "\"\"\"\n".into(),
                "y = 2\n".into(),
            ]),
            vec![None, None, Some(0), None, Some(4)]
        );
    }
}
