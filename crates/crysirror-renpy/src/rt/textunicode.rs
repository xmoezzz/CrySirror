// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.emoji_trie` (7.7/8.2+) and `renpy.text.bidi` (8.5+), over
//! the shared `crysirror_text` emoji and bidi services.
//!
//! `emoji_trie` publishes the merged UTS #51 trie exactly as Ren'Py's
//! generated module does (`emoji` root, nodes `e1`..`eN` with the
//! generator's numbering): Emoji 15.1 data through 8.4, Emoji 17.0 from
//! 8.5. `bidi` replaces upstream's FriBidi binding: `log2vis` returns the
//! visual-order line with Arabic presentation forms and mirroring, and
//! `get_embedding_levels` the per-character UBA levels, each with the
//! resolved paragraph direction as FriBidi's type value.
use super::{
    py::{self, *},
    textsupport::Rel,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use crysirror_text::{
    bidi_visual::{self, BaseDirection},
    emoji::{self, EmojiTrie},
};

// FriBidi paragraph type values (fribidi-bidi-types.h).
pub const LTR: i64 = 0x110;
pub const RTL: i64 = 0x111;
pub const ON: i64 = 0x40;
pub const WLTR: i64 = 0x20;
pub const WRTL: i64 = 0x21;

fn emoji_version(rel: Rel) -> u16 {
    if rel.at_least(8, 5) { 170 } else { 151 }
}

fn init_emoji(vm: &mut Vm) -> PyResult<ObjRef> {
    let rel = Rel::of(vm);
    if !rel.since_pair(7, 2) {
        return Err(vm.exc_msg("ModuleNotFoundError", "No module named 'renpy.text.emoji_trie'"));
    }
    let trie = EmojiTrie::build(emoji_version(rel));
    let m = crysirror_python::modules::module_with(vm, "renpy.text.emoji_trie", &[]);
    for (n, v) in [
        ("NOT_EMOJI", emoji::NOT_EMOJI),
        ("UNQUALIFIED", emoji::UNQUALIFIED),
        ("COMPONENT", emoji::COMPONENT),
        ("MINIMALLY_QUALIFIED", emoji::MINIMALLY_QUALIFIED),
        ("QUALIFIED", emoji::QUALIFIED),
    ] {
        crysirror_python::modules::set(vm, &m, n, Value::Int(v as i64));
    }
    let dicts: Vec<Value> = trie.nodes.iter().map(|_| vm.types.new_dict()).collect();
    for (i, node) in trie.nodes.iter().enumerate() {
        let d = &dicts[i];
        let k = vm.types.new_str("");
        ops::setitem(vm, d, k, Value::Int(node.level as i64))?;
        for (c, j) in &node.children {
            let k = vm.types.new_string(c.to_string());
            ops::setitem(vm, d, k, dicts[*j].clone())?;
        }
        let name = if i == 0 { "emoji".to_string() } else { format!("e{i}") };
        crysirror_python::modules::set(vm, &m, &name, d.clone());
    }
    Ok(m)
}

/// The emoji trie root of the running release (`emoji_trie.emoji`).
pub fn emoji_root(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.text.emoji_trie", "emoji")
}

fn base_direction(vm: &mut Vm, v: &Value) -> PyResult<BaseDirection> {
    Ok(match py::int(vm, v)? {
        LTR => BaseDirection::Ltr,
        RTL => BaseDirection::Rtl,
        WLTR => BaseDirection::WeakLtr,
        WRTL => BaseDirection::WeakRtl,
        _ => BaseDirection::Neutral,
    })
}

fn resolved(rtl: bool) -> Value {
    Value::Int(if rtl { RTL } else { LTR })
}

/// `log2vis(s, direction=ON)` -> (visual string, resolved direction).
pub fn f_log2vis(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &arg_or(args, kw, 0, "s", Value::None))?;
    let d = arg_or(args, kw, 1, "direction", Value::Int(ON));
    let d = base_direction(vm, &d)?;
    let (v, rtl) = bidi_visual::log2vis(&s, d);
    let v = vm.types.new_string(v);
    Ok(py::tuple(vm, vec![v, resolved(rtl)]))
}

/// `get_embedding_levels(s, direction=ON)` -> (levels, resolved direction).
pub fn f_get_embedding_levels(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &arg_or(args, kw, 0, "s", Value::None))?;
    let d = arg_or(args, kw, 1, "direction", Value::Int(ON));
    let d = base_direction(vm, &d)?;
    let (levels, rtl) = bidi_visual::embedding_levels(&s, d);
    let l = levels.into_iter().map(|l| Value::Int(l as i64)).collect();
    let l = py::tuple(vm, l);
    Ok(py::tuple(vm, vec![l, resolved(rtl)]))
}

pub const BIDI_NAMES: &[(&str, i64)] = &[("LTR", LTR), ("RTL", RTL), ("ON", ON), ("WLTR", WLTR), ("WRTL", WRTL)];

fn init_bidi(vm: &mut Vm) -> PyResult<ObjRef> {
    if !Rel::of(vm).at_least(8, 5) {
        return Err(vm.exc_msg("ModuleNotFoundError", "No module named 'renpy.text.bidi'"));
    }
    let m = crysirror_python::modules::module_with(
        vm,
        "renpy.text.bidi",
        &[("log2vis", f_log2vis), ("get_embedding_levels", f_get_embedding_levels)],
    );
    for (n, v) in BIDI_NAMES {
        crysirror_python::modules::set(vm, &m, n, Value::Int(*v));
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module("renpy.text.emoji_trie", init_emoji);
    vm.register_native_module("renpy.text.bidi", init_bidi);
}
