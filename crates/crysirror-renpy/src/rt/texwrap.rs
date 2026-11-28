// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.texwrap`, after Ren'Py 6.18 - 8.6 `texwrap.pyx` (MIT):
//! Knuth-Plass line breaking over annotated glyphs. Words end at
//! SPLIT_INSTEAD/SPLIT_BEFORE glyphs; each line costs 100000 plus the
//! squared unused width (except a non-subtitle last line), overlong
//! single-word lines 100000 per pixel over. Release differences kept:
//! 6.18/6.99 use the rest width for the first line and vice versa (their
//! `if i:` test), altruby is skipped from 7.3, SPLIT_IGNORE advances are
//! carried from 7.7/8.2 (after the split test), 8.4 (before it, counting
//! the glyph's own advance into the word end) and 8.5+ (before it).
use super::{
    py::{self, *},
    textsupport::{self, Glyph, Rel, RUBY_ALT, RUBY_TOP, SPLIT_BEFORE, SPLIT_IGNORE, SPLIT_INSTEAD, SPLIT_NONE},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

const MODULE: &str = "renpy.text.texwrap";

struct Word {
    glyph: usize,
    start_x: f64,
    end_x: f64,
}

fn make_word_list(rel: Rel, gs: &[Glyph]) -> Vec<Word> {
    let mut words = Vec::new();
    let (mut start_x, mut x, mut ignored) = (0f64, gs[0].advance, 0f64);
    let mut start_glyph = 0usize;
    let ignore = rel.since_pair(7, 2);
    let early = rel.at_least(8, 4);
    for (i, g) in gs.iter().enumerate().skip(1) {
        if g.ruby == RUBY_TOP || (g.ruby == RUBY_ALT && rel.at_least(7, 3)) {
            continue;
        }
        if ignore && early {
            if g.split == SPLIT_IGNORE {
                ignored += g.advance;
                continue;
            }
            if rel.at_least(8, 5) {
                x += ignored;
            } else {
                x += g.advance + ignored;
            }
            ignored = 0.;
        }
        if g.split == SPLIT_INSTEAD {
            words.push(Word { glyph: start_glyph, start_x, end_x: x });
            start_x = x + g.advance;
            start_glyph = i;
        } else if g.split == SPLIT_BEFORE {
            words.push(Word { glyph: start_glyph, start_x, end_x: x });
            start_x = x;
            start_glyph = i;
        }
        if !ignore {
            x += g.advance;
        } else if !early {
            if g.split == SPLIT_IGNORE {
                ignored += g.advance;
            } else {
                x += g.advance + ignored;
                ignored = 0.;
            }
        } else if rel.at_least(8, 5) {
            x += g.advance;
        }
    }
    if ignore {
        x += ignored;
    }
    words.push(Word { glyph: start_glyph, start_x, end_x: x });
    words
}

/// `linebreak_tex(glyphs, first_width, rest_width, subtitle)`.
pub fn linebreak_tex(rel: Rel, gs: &mut [Glyph], first_width: f64, rest_width: f64, subtitle: bool) {
    if gs.is_empty() {
        return;
    }
    let (mut first, mut rest) = (first_width.trunc(), rest_width.trunc());
    if !rel.at_least(7, 0) {
        std::mem::swap(&mut first, &mut rest);
    }
    let words = make_word_list(rel, gs);
    let n = words.len();
    let mut scores = vec![0f64; n + 1];
    let mut splits = vec![0usize; n + 1];
    for j in 1..=n {
        let j_x = words[j - 1].end_x;
        let mut min_score = f64::INFINITY;
        let mut split = j - 1;
        let mut i = j;
        while i > 0 {
            i -= 1;
            let mut score = scores[i] + 100000.;
            let width = j_x - words[i].start_x;
            let line_width = if i == 0 { first } else { rest };
            if width > line_width {
                if i < j - 1 {
                    break;
                }
                score += 100000.0 * (width - line_width);
            } else if subtitle || j != n {
                score += (line_width - width) * (line_width - width);
            }
            if score < min_score {
                min_score = score;
                split = i;
            }
        }
        scores[j] = min_score;
        splits[j] = split;
    }
    let mut end = n;
    while end > 0 {
        let start = splits[end];
        for w in &words[start + 1..end] {
            gs[w.glyph].split = SPLIT_NONE;
        }
        end = start;
    }
}

fn wrap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<()> {
    let glyphs = arg_or(args, kw, 0, "glyphs", Value::None);
    let fw = py::f64_of(vm, &arg_or(args, kw, 1, "first_width", Value::Int(0)))?;
    let rw = py::f64_of(vm, &arg_or(args, kw, 2, "rest_width", Value::Int(0)))?;
    let st = arg_or(args, kw, 3, "subtitle", Value::Bool(false));
    let subtitle = ops::truthy(vm, &st)?;
    let rel = Rel::of(vm);
    let objs = py::items(vm, &glyphs)?;
    let mut gs = Vec::with_capacity(objs.len());
    for o in &objs {
        gs.push(textsupport::read_glyph(vm, rel, o)?);
    }
    let before: Vec<u8> = gs.iter().map(|g| g.split).collect();
    linebreak_tex(rel, &mut gs, fw, rw, subtitle);
    for ((o, g), b) in objs.iter().zip(&gs).zip(before) {
        if g.split != b {
            py::set(vm, o, "split", Value::Int(g.split as i64))?;
        }
    }
    Ok(())
}

fn f_linebreak_tex(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    wrap(vm, args, kw)?;
    Ok(Value::None)
}

fn ww_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    wrap(vm, &args[1..], kw)?;
    Ok(Value::None)
}

fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[("linebreak_tex", f_linebreak_tex)]);
    let ww = py::class(vm, MODULE, "WordWrapper", &[], &[("__init__", ww_init)], &[])?;
    crysirror_python::modules::set(vm, &m, "WordWrapper", Value::Obj(ww));
    // `times = collections.defaultdict(float)` (profiling totals).
    let dd = py::import_attr(vm, "collections", "defaultdict")?;
    let float = py::import_attr(vm, "builtins", "float")
        .or_else(|_| py::import_attr(vm, "__builtin__", "float"))?;
    let times = vm.call(&dd, vec![float], Vec::new())?;
    crysirror_python::modules::set(vm, &m, "times", times);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init);
}
