// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.text`, `renpy.text.textsupport`, `renpy.text.texwrap` and
//! `renpy.text.extras`, after Ren'Py 8.3 (MIT).
//!
//! Layout follows upstream: tokens become styled segments; glyph metrics
//! come from the shared font service; paragraphs are annotated for line
//! breaking (UAX #14 opportunities through `unicode-linebreak`, or the
//! western/anywhere rules), broken greedily or with the Knuth-Plass "tex"
//! wrapper, placed, aligned/justified and timed for slow text. Rendering
//! emits retained glyph runs, one per outline pass and the face pass, so
//! text is never rasterized here.
use super::{
    font::{self, Font, Lookup},
    layout::{style as sty, style_b},
    py::{self, *},
    render::{self, Mat, RenderData, TexKind, Texture, new_render, new_texture, render_data},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use serde_json::{Value as J, json};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

pub const TEXT: i64 = 1;
pub const TAG: i64 = 2;
pub const PARAGRAPH: i64 = 3;
pub const DISPLAYABLE: i64 = 4;

#[allow(unused_imports)]
use super::textsupport::{
    MAX_WIDTH, RUBY_ALT, RUBY_BOTTOM, RUBY_NONE, RUBY_TOP, Rel, SPLIT_BEFORE, SPLIT_IGNORE,
    SPLIT_INSTEAD, SPLIT_NONE,
};

// Tokenizer --------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Token {
    Text(String),
    Tag(String),
    Paragraph,
    Displayable(Value),
}

thread_local! {
    static KEEP_DANGLING: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// 8.6 emits a dangling `【` that earlier releases drop.
fn keep_dangling() -> bool {
    KEEP_DANGLING.with(|k| k.get())
}

pub fn tokenize(s: &str, lenticular: bool) -> Result<Vec<Token>, String> {
    let mut rv = Vec::new();
    if s.is_empty() {
        return Ok(rv);
    }
    let finish = |buf: &str, rv: &mut Vec<Token>| {
        if lenticular && buf.contains('【') {
            rv.extend(lenticular_bracket_ruby(buf, keep_dangling()));
        } else {
            rv.push(Token::Text(buf.to_owned()));
        }
    };
    if !s.contains('{') && !s.contains('\n') && !s.contains('【') {
        rv.push(Token::Text(s.to_owned()));
        return Ok(rv);
    }
    #[derive(PartialEq)]
    enum St {
        Text,
        Brace,
        Tag,
    }
    let mut state = St::Text;
    let mut buf = String::new();
    for c in s.chars() {
        match state {
            St::Text => match c {
                '\n' => {
                    if !buf.is_empty() {
                        finish(&buf, &mut rv);
                    }
                    rv.push(Token::Paragraph);
                    buf.clear();
                }
                '{' => state = St::Brace,
                c => buf.push(c),
            },
            St::Brace => match c {
                '{' => {
                    buf.push(c);
                    state = St::Text;
                }
                '}' => return Err(format!("Empty text tag in {s:?}.")),
                c => {
                    if !buf.is_empty() {
                        finish(&buf, &mut rv);
                    }
                    buf = c.to_string();
                    state = St::Tag;
                }
            },
            St::Tag => {
                if c == '}' {
                    rv.push(Token::Tag(std::mem::take(&mut buf)));
                    state = St::Text;
                } else {
                    buf.push(c);
                }
            }
        }
    }
    if state != St::Text {
        return Err(format!("Open text tag at end of string {s:?}."));
    }
    if !buf.is_empty() {
        finish(&buf, &mut rv);
    }
    Ok(rv)
}

pub(crate) fn lenticular_bracket_ruby(s: &str, keep_dangling: bool) -> Vec<Token> {
    let mut rv = Vec::new();
    let mut buf = String::new();
    let mut state = 0;
    for c in s.chars() {
        match state {
            0 => {
                if c == '【' {
                    if !buf.is_empty() {
                        rv.push(Token::Text(std::mem::take(&mut buf)));
                    }
                    state = 1;
                } else {
                    buf.push(c);
                }
            }
            1 => {
                if c == '【' && buf.is_empty() {
                    buf.push(c);
                    state = 0;
                } else if c == '】' {
                    rv.push(Token::Text(format!("【{buf}】")));
                    buf.clear();
                    state = 0;
                } else if c == '｜' || c == '|' {
                    rv.push(Token::Tag("rb".into()));
                    rv.push(Token::Text(std::mem::take(&mut buf)));
                    rv.push(Token::Tag("/rb".into()));
                    state = 2;
                } else {
                    buf.push(c);
                }
            }
            _ => {
                if c == '】' {
                    rv.push(Token::Tag("rt".into()));
                    rv.push(Token::Text(std::mem::take(&mut buf)));
                    rv.push(Token::Tag("/rt".into()));
                    state = 0;
                } else {
                    buf.push(c);
                }
            }
        }
    }
    if !buf.is_empty() || (keep_dangling && state == 1) {
        match state {
            0 => rv.push(Token::Text(buf)),
            1 => rv.push(Token::Text(format!("【{buf}"))),
            _ => {
                rv.push(Token::Tag("rt".into()));
                rv.push(Token::Text(buf));
                rv.push(Token::Tag("/rt".into()));
            }
        }
    }
    rv
}

pub(crate) fn tokens_to_py(vm: &mut Vm, tokens: &[Token]) -> Value {
    let items = tokens
        .iter()
        .map(|t| {
            let (k, v) = match t {
                Token::Text(s) => (TEXT, vm.types.new_str(s)),
                Token::Tag(s) => (TAG, vm.types.new_str(s)),
                Token::Paragraph => (PARAGRAPH, vm.types.new_str("")),
                Token::Displayable(d) => (DISPLAYABLE, d.clone()),
            };
            vm.types.new_tuple(vec![Value::Int(k), v])
        })
        .collect();
    vm.types.new_list(items)
}

fn tokens_from_py(vm: &mut Vm, v: &Value) -> PyResult<Vec<Token>> {
    let mut out = Vec::new();
    for t in py::items(vm, v)? {
        let t = py::items(vm, &t)?;
        let k = py::int(vm, &t[0])?;
        out.push(match k {
            TEXT => Token::Text(py::string(vm, &t[1])?),
            TAG => Token::Tag(py::string(vm, &t[1])?),
            PARAGRAPH => Token::Paragraph,
            _ => Token::Displayable(t[1].clone()),
        });
    }
    Ok(out)
}

fn lenticular(vm: &mut Vm) -> bool {
    py::config_bool(vm, "lenticular_bracket_ruby").unwrap_or(false)
}

pub(crate) fn f_tokenize(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args.first().cloned().unwrap_or(Value::None))?;
    let l = lenticular(vm);
    let t = tokenize(&s, l).map_err(|e| py::error(vm, e))?;
    Ok(tokens_to_py(vm, &t))
}

// Segments ---------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Seg {
    font: Value,
    size: f64,
    bold: bool,
    italic: bool,
    underline: i64,
    strikethrough: bool,
    color: Option<[u8; 4]>,
    black_color: Option<[u8; 4]>,
    hyperlink: i64,
    kerning: f64,
    cps: f64,
    ruby_top: u8,
    ruby_bottom: bool,
    outline_color: Option<[u8; 4]>,
    ignore: bool,
    vertical: bool,
}

#[derive(Clone, Debug)]
enum Piece {
    Text(Seg, String),
    Space(Seg, f64, f64),
    Disp(Seg, Value, f64, f64),
    Start,
    End,
}

fn color_opt(vm: &mut Vm, v: &Value) -> PyResult<Option<[u8; 4]>> {
    super::color::rgba_opt(vm, v)
}

fn take_style(vm: &mut Vm, seg: &mut Seg, style: &Value, text_cps: f64) -> PyResult<()> {
    let g = |vm: &mut Vm, n: &str| py::get(vm, style, n);
    let font = g(vm, "font")?;
    let fmap = py::config(vm, "font_name_map")?;
    seg.font = if py::is_none(&fmap) {
        font.clone()
    } else {
        vm.call_method(&fmap, "get", vec![font.clone(), font])?
    };
    let size = g(vm, "size")?;
    seg.size = py::f64_of(vm, &size)?;
    let b = g(vm, "bold")?;
    seg.bold = ops::truthy(vm, &b)?;
    let i = g(vm, "italic")?;
    seg.italic = ops::truthy(vm, &i)?;
    let u = g(vm, "underline")?;
    seg.underline = match u {
        Value::Int(n) => n,
        Value::Bool(b) => b as i64,
        v => ops::truthy(vm, &v)? as i64,
    };
    let s = g(vm, "strikethrough")?;
    seg.strikethrough = ops::truthy(vm, &s)?;
    let c = g(vm, "color")?;
    if let Some(c) = color_opt(vm, &c)? {
        seg.color = Some(c);
    }
    let bc = g(vm, "black_color")?;
    if let Some(c) = color_opt(vm, &bc)? {
        seg.black_color = Some(c);
    }
    seg.hyperlink = 0;
    let k = g(vm, "kerning")?;
    seg.kerning = if py::is_none(&k) {
        0.
    } else {
        py::f64_of(vm, &k)?
    };
    seg.outline_color = None;
    let sc = g(vm, "slow_cps")?;
    if matches!(sc, Value::Bool(true)) {
        seg.cps = text_cps;
    }
    let mul = g(vm, "slow_cps_multiplier")?;
    if !py::is_none(&mul) {
        seg.cps *= py::f64_of(vm, &mul)?;
    }
    let v = g(vm, "vertical")?;
    seg.vertical = ops::truthy(vm, &v)?;
    Ok(())
}

fn text_cps_pref(vm: &mut Vm) -> f64 {
    let p = py::import_attr(vm, "renpy.game", "preferences").unwrap_or(Value::None);
    if py::is_none(&p) {
        return 0.;
    }
    let v = py::get_or_none(vm, &p, "text_cps").unwrap_or(Value::Int(0));
    py::f64_of(vm, &v).unwrap_or(0.)
}

struct Segmented {
    paragraphs: Vec<Vec<Piece>>,
    hyperlink_targets: Vec<Value>,
    has_ruby: bool,
}

fn segment(
    vm: &mut Vm,
    tokens: &[Token],
    style: &Value,
    text: &Value,
    renders: &HashMap<i64, Value>,
    cps: f64,
) -> PyResult<Segmented> {
    let text_cps = text_cps_pref(vm);
    let mut base = Seg {
        font: Value::None,
        size: 22.,
        bold: false,
        italic: false,
        underline: 0,
        strikethrough: false,
        color: None,
        black_color: None,
        hyperlink: 0,
        kerning: 0.,
        cps,
        ruby_top: RUBY_NONE,
        ruby_bottom: false,
        outline_color: None,
        ignore: false,
        vertical: false,
    };
    take_style(vm, &mut base, style, text_cps)?;
    let mut tss = vec![base];
    let mut paragraphs = Vec::new();
    let mut line: Vec<Piece> = Vec::new();
    let mut targets: Vec<Value> = Vec::new();
    let mut has_ruby = false;
    let mask = py::get_or_none(vm, text, "mask")?;
    let mask = if py::is_none(&mask) {
        None
    } else {
        Some(py::string(vm, &mask)?)
    };
    let fill_empty = |line: &mut Vec<Piece>, tss: &Vec<Seg>| {
        if !line.iter().any(|p| matches!(p, Piece::Text(..))) {
            line.push(Piece::Text(tss.last().unwrap().clone(), "\u{200b}".into()));
        }
    };
    let mut done = false;
    for t in tokens {
        let r: PyResult<()> = (|| {
            match t {
                Token::Paragraph => {
                    fill_empty(&mut line, &tss);
                    paragraphs.push(std::mem::take(&mut line));
                }
                Token::Text(s) => {
                    let s = match &mask {
                        Some(m) => m.repeat(s.chars().count()),
                        None => s.clone(),
                    };
                    line.push(Piece::Text(tss.last().unwrap().clone(), s));
                }
                Token::Displayable(d) => {
                    let (w, h) = renders
                        .get(&d.id())
                        .map(super::layout::size)
                        .unwrap_or((0., 0.));
                    line.push(Piece::Disp(tss.last().unwrap().clone(), d.clone(), w, h));
                }
                Token::Tag(text_tag) => {
                    let (tag, value) = text_tag
                        .split_once('=')
                        .map(|(a, b)| (a.to_owned(), b.to_owned()))
                        .unwrap_or((text_tag.clone(), String::new()));
                    let push = |tss: &mut Vec<Seg>| {
                        let n = tss.last().unwrap().clone();
                        tss.push(n);
                    };
                    let need = |vm: &mut Vm| -> PyResult<()> {
                        if value.is_empty() {
                            return Err(py::error(
                                vm,
                                format!("empty value supplied for tag {tag:?}"),
                            ));
                        }
                        Ok(())
                    };
                    if let Some(closed) = tag.strip_prefix('/') {
                        tss.pop();
                        if tss.is_empty() {
                            let tt = py::import_attr(vm, "renpy.text.extras", "text_tags")?;
                            let k = vm.types.new_str(closed);
                            let takes = vm.call_method(&tt, "get", vec![k, Value::Bool(true)])?;
                            if !ops::truthy(vm, &takes)? {
                                return Err(py::error(
                                    vm,
                                    format!(
                                        "{{/{closed}}} isn't valid, since the {closed} text tag doesn't take a close tag."
                                    ),
                                ));
                            }
                            return Err(py::error(
                                vm,
                                format!("{text_tag:?} closes a text tag that isn't open."),
                            ));
                        }
                        return Ok(());
                    }
                    match tag.as_str() {
                        "_start" => line.push(Piece::Start),
                        "_end" => line.push(Piece::End),
                        "p" => {
                            fill_empty(&mut line, &tss);
                            paragraphs.push(std::mem::take(&mut line));
                        }
                        "space" => {
                            need(vm)?;
                            let w: f64 = value
                                .parse()
                                .map_err(|_| vm.value_error(format!("invalid space {value:?}")))?;
                            line.push(Piece::Space(tss.last().unwrap().clone(), w, 0.));
                        }
                        "vspace" => {
                            need(vm)?;
                            let h: f64 = value
                                .parse()
                                .map_err(|_| vm.value_error(format!("invalid vspace {value:?}")))?;
                            if !line.is_empty() {
                                paragraphs.push(std::mem::take(&mut line));
                            }
                            paragraphs.push(vec![Piece::Space(tss.last().unwrap().clone(), 0., h)]);
                        }
                        "w" | "fast" | "nw" | "noalt" => {
                            if tag == "noalt" {
                                push(&mut tss);
                            }
                        }
                        "done" => done = true,
                        "a" => {
                            let funcs = sty(vm, text, "hyperlink_functions")?;
                            let styler = ops::getitem(vm, &funcs, Value::Int(0))?;
                            let v = vm.types.new_str(&value);
                            let hls = if !py::is_none(&styler) {
                                vm.call(&styler, vec![v.clone()], Vec::new())?
                            } else {
                                style.clone()
                            };
                            let old_prefix = py::get(vm, &hls, "prefix")?;
                            targets.push(v.clone());
                            let link = targets.len() as i64;
                            let sensitive = vm.call_method(text, "hyperlink_sensitive", vec![v])?;
                            let focused = super::focus::get_focused(vm)?;
                            let arg = py::import_attr(vm, "renpy.display.focus", "argument")?;
                            let p = if !ops::truthy(vm, &sensitive)? {
                                "insensitive_"
                            } else if py::same(&focused, text)
                                && matches!(arg, Value::Int(a) if a == link)
                            {
                                "hover_"
                            } else {
                                "idle_"
                            };
                            let pv = vm.types.new_str(p);
                            vm.call_method(&hls, "set_prefix", vec![pv])?;
                            push(&mut tss);
                            let mut ts = tss.pop().unwrap();
                            let vert = ts.vertical;
                            let size = ts.size;
                            take_style(vm, &mut ts, &hls, text_cps)?;
                            ts.vertical = vert;
                            ts.hyperlink = link;
                            if py::config_bool(vm, "hyperlink_inherit_size").unwrap_or(true) {
                                ts.size = size;
                            }
                            tss.push(ts);
                            vm.call_method(&hls, "set_prefix", vec![old_prefix])?;
                        }
                        "b" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().bold = true;
                        }
                        "i" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().italic = true;
                        }
                        "u" => {
                            push(&mut tss);
                            let n = if value.is_empty() {
                                1
                            } else {
                                value.parse().unwrap_or(1)
                            };
                            tss.last_mut().unwrap().underline = n;
                        }
                        "s" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().strikethrough = true;
                        }
                        "plain" => {
                            push(&mut tss);
                            let ts = tss.last_mut().unwrap();
                            ts.bold = false;
                            ts.italic = false;
                            ts.underline = 0;
                            ts.strikethrough = false;
                        }
                        "" => {
                            let sm = py::import_attr(vm, "renpy.store", "style")?;
                            let s = py::get(vm, &sm, &value)?;
                            push(&mut tss);
                            let mut ts = tss.pop().unwrap();
                            take_style(vm, &mut ts, &s, text_cps)?;
                            tss.push(ts);
                        }
                        "font" => {
                            let fmap = py::config(vm, "font_name_map")?;
                            let v = vm.types.new_str(&value);
                            let f = if py::is_none(&fmap) {
                                v.clone()
                            } else {
                                vm.call_method(&fmap, "get", vec![v.clone(), v])?
                            };
                            push(&mut tss);
                            tss.last_mut().unwrap().font = f;
                        }
                        "size" => {
                            need(vm)?;
                            push(&mut tss);
                            let ts = tss.last_mut().unwrap();
                            if value.starts_with('+') || value.starts_with('-') {
                                ts.size += value
                                    .parse::<f64>()
                                    .map_err(|_| vm.value_error("invalid size"))?;
                            } else if let Some(m) = value.strip_prefix('*') {
                                ts.size = (m
                                    .parse::<f64>()
                                    .map_err(|_| vm.value_error("invalid size"))?
                                    * ts.size)
                                    .trunc();
                            } else {
                                ts.size = value
                                    .parse::<f64>()
                                    .map_err(|_| vm.value_error("invalid size"))?;
                            }
                        }
                        "color" | "outlinecolor" => {
                            need(vm)?;
                            let c = super::color::parse_hex(&value).ok_or_else(|| {
                                vm.value_error(format!(
                                    "Color string {value:?} must be in hex format."
                                ))
                            })?;
                            push(&mut tss);
                            if tag == "color" {
                                tss.last_mut().unwrap().color = Some(c);
                            } else {
                                tss.last_mut().unwrap().outline_color = Some(c);
                            }
                        }
                        "alpha" => {
                            need(vm)?;
                            push(&mut tss);
                            let ts = tss.last_mut().unwrap();
                            let mut c = ts.color.unwrap_or([255; 4]);
                            let cur = c[3] as f64 / 255.;
                            let a = if value.starts_with('+') || value.starts_with('-') {
                                cur + value.parse::<f64>().unwrap_or(0.)
                            } else if let Some(m) = value.strip_prefix('*') {
                                cur * m.parse::<f64>().unwrap_or(1.)
                            } else {
                                value.parse::<f64>().unwrap_or(1.)
                            };
                            c[3] = (a.clamp(0., 1.) * 255.) as u8;
                            ts.color = Some(c);
                        }
                        "k" => {
                            need(vm)?;
                            push(&mut tss);
                            tss.last_mut().unwrap().kerning = value.parse().unwrap_or(0.);
                        }
                        "rt" | "art" => {
                            let rs = sty(
                                vm,
                                text,
                                if tag == "rt" {
                                    "ruby_style"
                                } else {
                                    "altruby_style"
                                },
                            )?;
                            push(&mut tss);
                            let mut ts = tss.pop().unwrap();
                            let vert = ts.vertical;
                            take_style(vm, &mut ts, &rs, text_cps)?;
                            ts.vertical = vert;
                            ts.ruby_top = if tag == "rt" { RUBY_TOP } else { RUBY_ALT };
                            tss.push(ts);
                            has_ruby = true;
                        }
                        "rb" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().ruby_bottom = true;
                        }
                        "cps" => {
                            need(vm)?;
                            push(&mut tss);
                            let ts = tss.last_mut().unwrap();
                            if let Some(m) = value.strip_prefix('*') {
                                ts.cps *= m.parse::<f64>().unwrap_or(1.);
                            } else {
                                ts.cps = value.parse().unwrap_or(0.);
                            }
                        }
                        "vert" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().vertical = true;
                        }
                        "horiz" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().vertical = false;
                        }
                        "alt" => {
                            push(&mut tss);
                            tss.last_mut().unwrap().ignore = true;
                        }
                        "instance" => push(&mut tss),
                        "shader" => {
                            return Err(py::error(
                                vm,
                                "The shader tag was given, but text shaders are not in use. Consider adding \"define config.default_textshader = 'typewriter'\" to your game.",
                            ));
                        }
                        t if t.starts_with("axis:") => push(&mut tss),
                        t if t.starts_with('#') => {}
                        _ => return Err(py::error(vm, format!("Unknown text tag {text_tag:?}"))),
                    }
                }
            }
            Ok(())
        })();
        if let Err(e) = r {
            if done {
                break;
            }
            let all = vm
                .call_method(text, "get_all_text", Vec::new())
                .ok()
                .and_then(|v| ops::repr(vm, &v).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let tt = match t {
                Token::Tag(s) => s.clone(),
                _ => String::new(),
            };
            let msg = vm
                .types
                .new_string(format!("While processing text tag {{{tt}}} in {all}.:"));
            let _ =
                py::import(vm, "renpy.game").and_then(|g| py::set(vm, &g, "exception_info", msg));
            return Err(e);
        }
    }
    fill_empty(&mut line, &tss);
    paragraphs.push(line);
    Ok(Segmented {
        paragraphs,
        hyperlink_targets: targets,
        has_ruby,
    })
}

// Glyphs -------------------------------------------------------------------------------------

pub use super::textsupport::{Glyph as G, Line};

pub struct TextLayout {
    width: f64,
    height: f64,
    size: (f64, f64),
    baseline: f64,
    glyphs: Vec<G>,
    lines: Vec<Line>,
    pieces: Vec<Piece>,
    outlines: Vec<(f64, Option<[u8; 4]>, f64, f64)>,
    xoffset: f64,
    yoffset: f64,
    xborder: f64,
    yborder: f64,
    max_time: f64,
    hyperlinks: Vec<(i64, f64, f64, f64, f64, f64)>,
    hyperlink_targets: Vec<Value>,
    pending: bool,
    end_index: Option<usize>,
}

fn piece_glyphs(vm: &mut Vm, piece: &Piece, index: usize, pending: &mut bool) -> PyResult<Vec<G>> {
    let mk =
        |ch: char, ascent: f64, ls: f64, w: f64, adv: f64, hyperlink: i64, font: Option<Font>| G {
            ch,
            x: 0.,
            y: 0.,
            ascent,
            line_spacing: ls,
            width: w,
            advance: adv,
            split: SPLIT_NONE,
            ruby: RUBY_NONE,
            time: 0.,
            hyperlink,
            piece: index,
            font,
            ..G::default()
        };
    let mut rv = match piece {
        Piece::Text(seg, s) => {
            if seg.ignore {
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            let parts: Vec<(Value, String)> = if seg
                .font
                .obj()
                .is_some_and(|o| o.class_name() == "FontGroup")
            {
                font::fontgroup_segment(vm, &seg.font, s)?
            } else {
                vec![(seg.font.clone(), s.clone())]
            };
            for (fv, text) in parts {
                let fname = py::string(vm, &fv)?;
                match font::get_font(vm, &fname, seg.size, seg.bold, seg.italic)? {
                    Lookup::Pending => {
                        *pending = true;
                        for c in text.chars() {
                            out.push(mk(
                                c,
                                seg.size * 0.8,
                                seg.size * 1.2,
                                0.,
                                0.,
                                seg.hyperlink,
                                None,
                            ));
                        }
                    }
                    Lookup::Ready(f) => {
                        for (c, m) in f.glyphs(vm, &text)? {
                            out.push(mk(
                                c,
                                m.ascent,
                                m.line_spacing,
                                m.width,
                                m.advance + seg.kerning,
                                seg.hyperlink,
                                Some(f.clone()),
                            ));
                        }
                    }
                }
            }
            out
        }
        Piece::Space(seg, w, h) => vec![mk('\0', 1., *h, *w, *w, seg.hyperlink, None)],
        Piece::Disp(seg, d, w, h) => {
            let null = d.obj().is_some_and(|o| o.class_name() == "Null");
            if null && *w == 0. && *h == 0. {
                return Ok(Vec::new());
            }
            let caret = d.obj().is_some_and(|o| o.class_name() == "CaretBlink");
            vec![mk(
                '\u{fffc}',
                0.,
                *h,
                *w,
                if caret { 0. } else { *w },
                seg.hyperlink,
                None,
            )]
        }
        Piece::Start | Piece::End => Vec::new(),
    };
    let ruby = match piece {
        Piece::Text(s, _) | Piece::Disp(s, ..) => {
            if s.ruby_bottom {
                RUBY_BOTTOM
            } else {
                s.ruby_top
            }
        }
        _ => RUBY_NONE,
    };
    if ruby != RUBY_NONE {
        for g in &mut rv {
            g.ruby = ruby;
        }
    }
    Ok(rv)
}

fn piece_cps(p: &Piece) -> f64 {
    match p {
        Piece::Text(s, _) | Piece::Space(s, ..) | Piece::Disp(s, ..) => s.cps,
        _ => 0.,
    }
}

fn opt_f(vm: &mut Vm, text: &Value, n: &str) -> PyResult<f64> {
    let v = sty(vm, text, n)?;
    if py::is_none(&v) {
        Ok(0.)
    } else {
        py::f64_of(vm, &v)
    }
}

fn figure_outlines(
    vm: &mut Vm,
    text: &Value,
) -> PyResult<(Vec<(f64, Option<[u8; 4]>, f64, f64)>, f64, f64, f64, f64)> {
    let outlines_v = sty(vm, text, "outlines")?;
    let ds = sty(vm, text, "drop_shadow")?;
    let has_o = ops::truthy(vm, &outlines_v)?;
    let has_ds = ops::truthy(vm, &ds)?;
    if !has_o && !has_ds {
        return Ok((vec![(0., None, 0., 0.)], 0., 0., 0., 0.));
    }
    let mut outlines = Vec::new();
    if has_ds {
        let list = if matches!(ds.payload(), Some(crysirror_python::Payload::List(_))) {
            py::items(vm, &ds)?
        } else {
            vec![ds.clone()]
        };
        let dsc = sty(vm, text, "drop_shadow_color")?;
        let c = color_opt(vm, &dsc)?;
        for d in list {
            let d = py::items(vm, &d)?;
            outlines.push((0., c, py::f64_of(vm, &d[0])?, py::f64_of(vm, &d[1])?));
        }
    }
    if has_o {
        for o in py::items(vm, &outlines_v)? {
            let o = py::items(vm, &o)?;
            let c = color_opt(vm, &o[1])?;
            outlines.push((
                py::f64_of(vm, &o[0])?,
                c,
                py::f64_of(vm, &o[2])?,
                py::f64_of(vm, &o[3])?,
            ));
        }
    }
    let (mut l, mut r, mut t, mut b) = (0f64, 0f64, 0f64, 0f64);
    for (o, _, x, y) in &outlines {
        l = l.min(x - o);
        r = r.max(x + o);
        t = t.min(y - o);
        b = b.max(y + o);
    }
    outlines.push((0., None, 0., 0.));
    Ok((outlines, r - l, b - t, -l, -t))
}

#[allow(clippy::too_many_arguments)]
fn build_layout(
    vm: &mut Vm,
    text: &Value,
    tokens: &[Token],
    width: f64,
    height: f64,
    renders: &HashMap<i64, Value>,
    splits_from: Option<&TextLayout>,
) -> PyResult<TextLayout> {
    let rel = Rel::of(vm);
    let width = width.min(32767.);
    let height = height.min(32767.);
    let mut cps = {
        let v = sty(vm, text, "slow_cps")?;
        if py::is_none(&v) || matches!(v, Value::Bool(true)) {
            text_cps_pref(vm)
        } else {
            py::f64_of(vm, &v)?
        }
    };
    let mul = sty(vm, text, "slow_cps_multiplier")?;
    if !py::is_none(&mul) {
        cps *= py::f64_of(vm, &mul)?;
    }
    let (outlines, xborder, yborder, xoffset, yoffset) = figure_outlines(vm, text)?;
    let w = width - xborder;
    let style_v = py::get(vm, text, "style")?;
    let seg = segment(vm, tokens, &style_v, text, renders, cps)?;
    let first_indent0 = opt_f(vm, text, "first_indent")?;
    let rest_indent = opt_f(vm, text, "rest_indent")?;
    let mut first_indent = first_indent0;
    let language = {
        let v = sty(vm, text, "language")?;
        if py::is_none(&v) {
            "unicode".to_owned()
        } else {
            py::string(vm, &v)?
        }
    };
    let layout_mode = {
        let v = sty(vm, text, "layout")?;
        if py::is_none(&v) {
            "tex".to_owned()
        } else {
            py::string(vm, &v)?
        }
    };
    let line_spacing = opt_f(vm, text, "line_spacing")?;
    let line_leading = opt_f(vm, text, "line_leading")?;
    let ruby_line_leading = opt_f(vm, text, "ruby_line_leading")?;
    let newline_indent = style_b(vm, text, "newline_indent")?;
    let mut pieces: Vec<Piece> = Vec::new();
    let mut glyphs: Vec<G> = Vec::new();
    let mut lines: Vec<Line> = Vec::new();
    let mut maxx = 0f64;
    let mut y = 0f64;
    let mut gt = 0f64;
    let mut pending = false;
    let mut started = !seg
        .paragraphs
        .iter()
        .flatten()
        .any(|p| matches!(p, Piece::Start));
    let mut ended = false;
    let mut end_index = None;
    let split_info: Option<Vec<u8>> =
        splits_from.map(|l| l.glyphs.iter().map(|g| g.split).collect());
    let mut split_cursor = 0usize;
    for para in seg.paragraphs {
        let base = glyphs.len();
        let mut pg: Vec<G> = Vec::new();
        let mut piece_ranges = Vec::new();
        for p in para {
            let idx = pieces.len();
            if matches!(p, Piece::End) {
                end_index = Some(glyphs.len() + pg.len());
            }
            let gs = piece_glyphs(vm, &p, idx, &mut pending)?;
            let r = pg.len()..pg.len() + gs.len();
            pg.extend(gs);
            piece_ranges.push((idx, r));
            pieces.push(p);
        }
        if let Some(si) = &split_info {
            for g in pg.iter_mut() {
                g.split = si.get(split_cursor).copied().unwrap_or(SPLIT_NONE);
                split_cursor += 1;
            }
        } else {
            use super::textsupport as ts;
            match language.as_str() {
                "unicode" | "eastasian" | "thaic90" => {
                    ts::annotate_unicode(rel, &mut pg, false, 0)
                }
                "korean-with-spaces" => ts::annotate_unicode(rel, &mut pg, true, 0),
                "japanese-loose" => ts::annotate_unicode(rel, &mut pg, false, 1),
                "japanese-normal" => ts::annotate_unicode(rel, &mut pg, false, 2),
                "japanese-strict" => ts::annotate_unicode(rel, &mut pg, false, 3),
                "western" => ts::annotate_western(&mut pg),
                "anywhere" if rel.since_pair(5, 0) => ts::annotate_anywhere(&mut pg),
                other => return Err(py::error(vm, format!("Unknown language: {other}"))),
            }
            let tex = super::texwrap::linebreak_tex;
            match layout_mode.as_str() {
                "tex" => tex(rel, &mut pg, w - first_indent, w - rest_indent, false),
                "subtitle" | "tex-subtitle" => {
                    tex(rel, &mut pg, w - first_indent, w - rest_indent, true)
                }
                "greedy" => ts::linebreak_greedy(&mut pg, w - first_indent, w - rest_indent),
                "nobreak" => ts::linebreak_nobreak(&mut pg),
                other => return Err(py::error(vm, format!("Unknown layout: {other}"))),
            }
        }
        for (idx, r) in &piece_ranges {
            let p = &pieces[*idx];
            if !started {
                if matches!(p, Piece::Start) {
                    started = true;
                } else {
                    for g in &mut pg[r.clone()] {
                        g.time = -3600.;
                    }
                    continue;
                }
            }
            if matches!(p, Piece::End) {
                ended = true;
            }
            let c = if ended { 0. } else { piece_cps(p) };
            let tpg = if c == 0. { 0. } else { 1. / c };
            for g in &mut pg[r.clone()] {
                if g.ruby == RUBY_TOP || g.ruby == RUBY_ALT {
                    g.time = -1.;
                    continue;
                }
                gt += tpg;
                g.time = gt;
            }
        }
        let wline = super::textsupport::place_horizontal(&mut pg, 0., first_indent, rest_indent);
        maxx = maxx.max(wline);
        let (mut l, ny) = super::textsupport::place_vertical(
            rel,
            &mut pg,
            base,
            y,
            line_spacing,
            line_leading,
            ruby_line_leading,
        );
        y = ny;
        glyphs.extend(pg);
        lines.append(&mut l);
        if !newline_indent {
            first_indent = rest_indent;
        }
    }
    if line_spacing < 0. && !lines.is_empty() {
        if py::config_bool(vm, "broken_line_spacing").unwrap_or(false) {
            y += -line_spacing * lines.len() as f64;
        } else {
            y += -line_spacing;
        }
        let last = lines.last_mut().unwrap();
        last.height = y - last.y;
    }
    let min_width = opt_f(vm, text, "min_width")?;
    if min_width > maxx + xborder {
        maxx = min_width - xborder;
    }
    maxx = maxx.ceil();
    let text_align = opt_f(vm, text, "text_align")?;
    let justify = style_b(vm, text, "justify")?;
    super::textsupport::align_and_justify(&mut glyphs, &lines, maxx, text_align, justify);
    let adjust = sty(vm, text, "adjust_spacing")?;
    if let (Some(sf), true) = (splits_from, ops::truthy(vm, &adjust)?) {
        let tx = sf.size.0 - sf.xborder;
        let ty = sf.size.1 - sf.yborder;
        let (mut dx, mut dy) = (tx - maxx, ty - y);
        match adjust.as_str() {
            Some("horizontal") => dy = 0.,
            Some("vertical") => dx = 0.,
            _ => {}
        }
        super::textsupport::adjust_glyph_spacing(rel, &mut glyphs, &mut lines, dx, dy, maxx, y);
        maxx = tx;
        y = ty;
    }
    if seg.has_ruby {
        let rs = sty(vm, text, "ruby_style")?;
        let ro = py::get(vm, &rs, "yoffset")?;
        let ars = sty(vm, text, "altruby_style")?;
        let ao = py::get(vm, &ars, "yoffset")?;
        let ro = if py::is_none(&ro) {
            0.
        } else {
            py::f64_of(vm, &ro)?
        };
        let ao = if py::is_none(&ao) {
            0.
        } else {
            py::f64_of(vm, &ao)?
        };
        let surf = (maxx + xborder, y + yborder);
        super::textsupport::place_ruby(rel, &mut glyphs, ro, ao, surf)
            .map_err(|e| py::error(vm, e))?;
    }
    let baseline = glyphs
        .iter()
        .find(|g| g.ascent != 0.)
        .map(|g| g.y + yoffset)
        .unwrap_or(0.);
    let max_time = super::textsupport::max_times(&glyphs, &mut lines);
    let hyperlinks = if seg.hyperlink_targets.is_empty() {
        Vec::new()
    } else {
        super::textsupport::hyperlink_areas(&glyphs, &lines)
    };
    Ok(TextLayout {
        width,
        height,
        size: (maxx + xborder, y + yborder),
        baseline,
        glyphs,
        lines,
        pieces,
        outlines,
        xoffset,
        yoffset,
        xborder,
        yborder,
        max_time,
        hyperlinks,
        hyperlink_targets: seg.hyperlink_targets,
        pending,
        end_index,
    })
}

// Layout cache --------------------------------------------------------------------------------

#[derive(Default)]
struct Caches {
    layouts: HashMap<i64, Rc<TextLayout>>,
    virtual_layouts: HashMap<i64, Rc<TextLayout>>,
    slow_text: Vec<Value>,
}

fn caches() -> Rc<RefCell<Caches>> {
    thread_local! {
        static C: Rc<RefCell<Caches>> = Rc::new(RefCell::new(Caches::default()));
    }
    C.with(|c| c.clone())
}

fn kill_layout(id: i64) {
    let c = caches();
    let mut c = c.borrow_mut();
    c.layouts.remove(&id);
    c.virtual_layouts.remove(&id);
}

pub fn layout_cache_clear() {
    let c = caches();
    let mut c = c.borrow_mut();
    c.layouts.clear();
    c.virtual_layouts.clear();
}

pub fn text_tick() {
    caches().borrow_mut().slow_text.clear();
}

// Rendering ------------------------------------------------------------------------------------

fn glyph_json(
    g: &G,
    seg: &Seg,
    x: f64,
    y: f64,
    override_color: Option<[u8; 4]>,
    outline: f64,
    default_color: [u8; 4],
) -> Option<J> {
    if g.ch == '\0' || g.ch == '\u{fffc}' || g.ch == '\u{200b}' {
        return None;
    }
    let color = override_color.or(seg.color).unwrap_or(default_color);
    match &g.font {
        Some(Font::Image { data, scale }) => {
            let cell = data.cells.get(&g.ch)?;
            if cell.asset.is_empty() || cell.rect[2] <= 0. {
                return None;
            }
            let [sx, sy, sw, sh] = cell.rect;
            let rect = [
                x + cell.offset[0] * scale,
                y - g.ascent + cell.offset[1] * scale,
                sw * scale,
                sh * scale,
            ];
            let tint = if override_color.is_some() {
                color
            } else {
                [255; 4]
            };
            Some(json!({
                "character": g.ch.to_string(),
                "baseline": [x as f32, y as f32],
                "image": {
                    "asset": cell.asset,
                    "rect": rect.map(|v| v as f32),
                    "uv": [(sx / cell.page[0]) as f32, (sy / cell.page[1]) as f32, ((sx + sw) / cell.page[0]) as f32, ((sy + sh) / cell.page[1]) as f32],
                    "tint": tint,
                }
            }))
        }
        Some(Font::Outline {
            alias, size, bold, ..
        }) => {
            let mut style = serde_json::Map::new();
            if seg.underline != 0 {
                style.insert("underline".into(), json!(true));
            }
            if seg.strikethrough {
                style.insert("strikeout".into(), json!(true));
            }
            if seg.bold && !bold {
                style.insert("bold".into(), json!(true));
            }
            if outline > 0. {
                style.insert("outline".into(), json!({"radius": (outline.round() as u8).min(16), "color": color, "kernel": "disk"}));
            }
            let mut o = json!({
                "character": g.ch.to_string(),
                "baseline": [x as f32, y as f32],
                "font": alias,
                "size": *size as f32,
                "color": color,
            });
            if seg.bold && !bold {
                o["bold"] = json!(true);
            }
            if !style.is_empty() {
                o["style"] = J::Object(style);
            }
            Some(o)
        }
        None => None,
    }
}

fn render_layout(
    vm: &mut Vm,
    this: &Value,
    layout: &TextLayout,
    rv: &RenderData,
    st: f64,
    slow: bool,
) -> PyResult<Vec<render::Blit>> {
    let mut blits = Vec::new();
    let default_color = [255u8, 255, 255, 255];
    let (w, h) = layout.size;
    for (o, color, xo, yo) in &layout.outlines {
        let mut glyphs = Vec::new();
        for (i, g) in layout.glyphs.iter().enumerate() {
            if layout.end_index.is_some_and(|e| i >= e) {
                break;
            }
            if g.time < -1000. {
                continue;
            }
            if slow && g.time > st {
                continue;
            }
            let Piece::Text(seg, _) = &layout.pieces[g.piece] else {
                continue;
            };
            let oc = if color.is_some() {
                seg.outline_color.or(*color)
            } else {
                None
            };
            if let Some(j) = glyph_json(
                g,
                seg,
                g.x + layout.xoffset + xo,
                g.y + layout.yoffset + yo,
                oc,
                *o,
                default_color,
            ) {
                glyphs.push(j);
            }
        }
        if glyphs.is_empty() {
            continue;
        }
        let first_font = glyphs
            .iter()
            .find_map(|g| g.get("font").and_then(|f| f.as_str()).map(str::to_owned))
            .unwrap_or_else(|| "default".into());
        let first_size = glyphs
            .iter()
            .find_map(|g| g.get("size").and_then(|s| s.as_f64()))
            .unwrap_or(22.);
        let run = json!({"kind": "glyph_run", "font": first_font, "size": first_size as f32, "color": default_color, "glyphs": glyphs});
        let t = new_texture(
            vm,
            Texture {
                kind: TexKind::Glyphs(Rc::new(run)),
                width: w.max(1.),
                height: h.max(1.),
                crop: None,
                full: None,
                opaque: false,
            },
        );
        let source = render::source_of(vm, &t)?;
        blits.push(render::Blit {
            source,
            x: 0.,
            y: 0.,
            focus: false,
            main: true,
        });
    }
    let _ = (this, rv);
    Ok(blits)
}

// Text displayable ------------------------------------------------------------------------------

fn text_items(vm: &mut Vm, v: &Value) -> PyResult<Vec<Value>> {
    if matches!(v.payload(), Some(crysirror_python::Payload::List(_))) {
        py::items(vm, v)
    } else {
        Ok(vec![v.clone()])
    }
}

fn t_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let names = [
        "text",
        "slow",
        "scope",
        "substitute",
        "slow_done",
        "replaces",
        "mask",
        "tokenized",
    ];
    let mut vals = vec![
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::None,
        Value::Bool(false),
    ];
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|n| *n == &**k) {
            Some(i) => vals[i] = v.clone(),
            None => props.push((k.clone(), v.clone())),
        }
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    let tokenized = ops::truthy(vm, &vals[7])?;
    let mut text = vals[0].clone();
    if !tokenized {
        let items = text_items(vm, &text)?;
        let mut ok = true;
        for i in &items {
            if i.as_str().is_none()
                && !super::core::is_displayable(vm, i)
                && ops::bytes_of(i).is_none()
            {
                ok = false;
            }
        }
        if !ok {
            if py::config_bool(vm, "developer").unwrap_or(false) {
                let r = ops::repr(vm, &items[0])?;
                return Err(py::error(vm, format!("Cannot display {r} as text.")));
            }
            text = vm.types.new_list(vec![vm.types.new_str("")]);
        } else {
            text = vm.types.new_list(items);
        }
    }
    py::set(vm, this, "substitute", vals[3].clone())?;
    py::set(vm, this, "dirty", Value::Bool(true))?;
    py::set(vm, this, "text", Value::None)?;
    py::set(vm, this, "mask", vals[6].clone())?;
    py::set(vm, this, "tokenized", Value::Bool(tokenized))?;
    vm.call_method(
        this,
        "set_text",
        vec![text, vals[2].clone(), vals[3].clone()],
    )?;
    let less = py::import_attr(vm, "renpy.game", "less_updates")
        .map(|v| ops::truthy(vm, &v).unwrap_or(false))
        .unwrap_or(false);
    let slow = if less {
        Value::Bool(false)
    } else {
        vals[1].clone()
    };
    py::set(vm, this, "slow", slow.clone())?;
    py::set(vm, this, "slow_done", vals[4].clone())?;
    py::set(vm, this, "slow_done_time", Value::None)?;
    py::set(vm, this, "ctc", Value::None)?;
    py::set(vm, this, "start", Value::None)?;
    py::set(vm, this, "end", Value::None)?;
    let rep = &vals[5];
    if rep.obj().is_some_and(|o| o.class_name() == "Text") {
        for n in ["slow", "slow_done", "slow_done_time", "ctc", "start", "end"] {
            let v = py::get_or_none(vm, rep, n)?;
            py::set(vm, this, n, v)?;
        }
    }
    py::set(vm, this, "displayables", Value::None)?;
    let s = py::get_or_none(vm, this, "slow")?;
    let dup = ops::truthy(vm, &s)?;
    py::set(vm, this, "_duplicatable", Value::Bool(dup))?;
    py::set(
        vm,
        this,
        "displayable_offsets",
        vm.types.new_list(Vec::new()),
    )?;
    Ok(Value::None)
}

fn t_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(4)))?;
    if v < 3 {
        py::set(vm, this, "ctc", Value::None)?;
    }
    if v < 4 {
        let t = py::get_or_none(vm, this, "text")?;
        if !matches!(t.payload(), Some(crysirror_python::Payload::List(_))) {
            py::set(vm, this, "text", vm.types.new_list(vec![t]))?;
        }
        for n in ["scope", "start", "end"] {
            py::set(vm, this, n, Value::None)?;
        }
        py::set(vm, this, "substitute", Value::Bool(false))?;
        py::set(vm, this, "dirty", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn t_duplicate(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = arg_or(args, kw, 1, "args", Value::None);
    if !py::is_none(&a) {
        let x = py::get(vm, &a, "args")?;
        if ops::truthy(vm, &x)? {
            vm.call_method(&a, "extraneous", Vec::new())?;
        }
    }
    let d = py::get_or_none(vm, this, "_duplicatable")?;
    if ops::truthy(vm, &d)? {
        let rv = vm.call_method(this, "_copy", vec![a])?;
        vm.call_method(&rv, "_unique", Vec::new())?;
        return Ok(rv);
    }
    Ok(this.clone())
}

fn t_in_current_store(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let u = py::get_or_none(vm, this, "_uses_scope")?;
    if !ops::truthy(vm, &u)? {
        return Ok(this.clone());
    }
    let rv = vm.call_method(this, "_copy", Vec::new())?;
    let ds = py::get_or_none(vm, &rv, "displayables")?;
    if !py::is_none(&ds) {
        let mut out = Vec::new();
        for d in py::items(vm, &ds)? {
            out.push(vm.call_method(&d, "_in_current_store", Vec::new())?);
        }
        py::set(vm, &rv, "displayables", vm.types.new_list(out))?;
    }
    py::set(vm, &rv, "slow_done", Value::None)?;
    py::set(vm, &rv, "locked", Value::Bool(true))?;
    Ok(rv)
}

fn all_text(vm: &mut Vm, this: &Value) -> PyResult<String> {
    let t = py::get_or_none(vm, this, "text")?;
    let mut s = String::new();
    if py::is_none(&t) {
        return Ok(s);
    }
    for i in text_items(vm, &t)? {
        if let Some(x) = i.as_str() {
            s.push_str(x);
        }
    }
    Ok(s)
}

fn t_repr_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let mut s = all_text(vm, &args[0])?;
    if s.chars().count() > 25 {
        s = s.chars().take(24).collect::<String>() + "…";
    }
    let v = vm.types.new_string(s);
    let r = ops::repr(vm, &v)?;
    Ok(vm.types.new_string(r.to_string()))
}

fn t_get_all_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = all_text(vm, &args[0])?;
    Ok(vm.types.new_string(s))
}

fn t_scope(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let scope = arg_or(args, kw, 1, "scope", Value::None);
    let update = arg_or(args, kw, 2, "update", Value::Bool(true));
    let locked = py::get_or_none(vm, this, "locked")?;
    if ops::truthy(vm, &locked)? {
        return Ok(Value::Bool(false));
    }
    let tp = py::get_or_none(vm, this, "text_parameter")?;
    let sub = py::get_or_none(vm, this, "substitute")?;
    vm.call_method(this, "set_text", vec![tp, scope, sub, update])
}

fn in_init_phase(vm: &mut Vm) -> bool {
    let ctx = py::import_attr(vm, "renpy.game", "context")
        .and_then(|f| vm.call(&f, Vec::new(), Vec::new()));
    match ctx {
        Ok(c) if !py::is_none(&c) => py::get_or_none(vm, &c, "init_phase")
            .map(|v| ops::truthy(vm, &v).unwrap_or(false))
            .unwrap_or(false),
        _ => true,
    }
}

fn pref_language(vm: &mut Vm) -> Value {
    let p = py::import_attr(vm, "renpy.game", "preferences").unwrap_or(Value::None);
    if py::is_none(&p) {
        return Value::None;
    }
    py::get_or_none(vm, &p, "language").unwrap_or(Value::None)
}

fn t_set_text(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let text = arg_or(args, kw, 1, "text", Value::None);
    let scope = arg_or(args, kw, 2, "scope", Value::None);
    let substitute = arg_or(args, kw, 3, "substitute", Value::Bool(false));
    let update = match arg(args, kw, 4, "update") {
        Some(v) => ops::truthy(vm, v)?,
        None => true,
    };
    let locked = py::get_or_none(vm, this, "locked")?;
    if ops::truthy(vm, &locked)? {
        return Ok(Value::Bool(false));
    }
    let lang = if in_init_phase(vm) {
        Value::None
    } else {
        pref_language(vm)
    };
    py::set(vm, this, "language", lang)?;
    let tokenized = {
        let v = py::get_or_none(vm, this, "tokenized")?;
        ops::truthy(vm, &v)?
    };
    if tokenized {
        let old = py::get_or_none(vm, this, "text")?;
        if update && !ops::eq(vm, &old, &text)? {
            py::set(vm, this, "dirty", Value::Bool(true))?;
            render::redraw(vm, this, 0.);
        }
        py::set(vm, this, "text", text.clone())?;
        py::set(vm, this, "text_parameter", text)?;
        py::set(vm, this, "_uses_scope", Value::Bool(false))?;
        return Ok(Value::Bool(true));
    }
    let old = py::get_or_none(vm, this, "text")?;
    let items = text_items(vm, &text)?;
    let tl = vm.types.new_list(items.clone());
    py::set(vm, this, "text_parameter", tl)?;
    let mut new_text = Vec::new();
    let mut uses = false;
    let sub_f = py::import_attr(vm, "renpy.substitutions", "substitute")?;
    for i in items {
        let mut i = i;
        if i.as_str().is_some() && !matches!(substitute, Value::Bool(false)) {
            let r = vm.call(
                &sub_f,
                vec![i.clone(), scope.clone(), substitute.clone()],
                Vec::new(),
            )?;
            let r = py::items(vm, &r)?;
            i = r[0].clone();
            uses = uses || ops::truthy(vm, &r[1])?;
        }
        if let Some(b) = ops::bytes_of(&i) {
            i = vm
                .types
                .new_string(String::from_utf8_lossy(&b).into_owned());
        }
        new_text.push(i);
    }
    py::set(vm, this, "_uses_scope", Value::Bool(uses))?;
    let nt = vm.types.new_list(new_text);
    if ops::eq(vm, &nt, &old)? {
        return Ok(Value::Bool(false));
    }
    if update {
        py::set(vm, this, "text", nt)?;
        let dirty = py::get_or_none(vm, this, "dirty")?;
        if !ops::truthy(vm, &dirty)? {
            py::set(vm, this, "dirty", Value::Bool(true))?;
            if !py::is_none(&old) {
                render::redraw(vm, this, 0.);
            }
        }
    }
    Ok(Value::Bool(true))
}

fn t_per_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let lang = py::get_or_none(vm, this, "language")?;
    let pl = pref_language(vm);
    let uses = py::get_or_none(vm, this, "_uses_scope")?;
    if !ops::eq(vm, &lang, &pl)? && !ops::truthy(vm, &uses)? {
        let tp = py::get_or_none(vm, this, "text_parameter")?;
        let sub = py::get_or_none(vm, this, "substitute")?;
        let f = py::get(vm, this, "set_text")?;
        vm.call(
            &f,
            vec![tp],
            vec![
                ("substitute".into(), sub),
                ("update".into(), Value::Bool(true)),
            ],
        )?;
    }
    if style_b(vm, this, "slow_abortable")? {
        caches().borrow_mut().slow_text.push(this.clone());
    }
    Ok(Value::None)
}

fn t_set_ctc(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "ctc",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    py::set(vm, &args[0], "dirty", Value::Bool(true))?;
    Ok(Value::None)
}

fn t_set_last_ctc(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(
        vm,
        &args[0],
        "last_ctc",
        args.get(1).cloned().unwrap_or(Value::None),
    )?;
    py::set(vm, &args[0], "dirty", Value::Bool(true))?;
    Ok(Value::None)
}

fn get_tokens(vm: &mut Vm, this: &Value) -> PyResult<Vec<Token>> {
    let t = py::get(vm, this, "tokens")?;
    tokens_from_py(vm, &t)
}

fn t_update(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "dirty", Value::Bool(false))?;
    kill_layout(this.id());
    let tokenized = {
        let v = py::get_or_none(vm, this, "tokenized")?;
        ops::truthy(vm, &v)?
    };
    let mut tokens = if !tokenized {
        let text = py::get(vm, this, "text")?;
        let mut items = text_items(vm, &text)?;
        let start = py::get_or_none(vm, this, "start")?;
        let ctc = py::get_or_none(vm, this, "ctc")?;
        let push_ctc = |vm: &mut Vm, out: &mut Vec<Value>, c: &Value| -> PyResult<()> {
            if py::is_none(c) {
                return Ok(());
            }
            if matches!(c.payload(), Some(crysirror_python::Payload::List(_))) {
                out.extend(py::items(vm, c)?);
            } else {
                out.push(c.clone());
            }
            Ok(())
        };
        if !py::is_none(&start) {
            let s = py::int(vm, &start)? as usize;
            let end = py::get_or_none(vm, this, "end")?;
            let first: Vec<char> = items
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .collect();
            let e = if py::is_none(&end) {
                first.len()
            } else {
                (py::int(vm, &end)? as usize).min(first.len())
            };
            let s = s.min(first.len());
            let mut start_string: String = first[..s].iter().collect();
            let mid: String = first[s..e].iter().collect();
            let mut end_string: String = first[e..].iter().collect();
            if !start_string.is_empty() {
                start_string.push_str("{_start}");
            }
            if !end_string.is_empty() {
                end_string = format!("{{_end}}{end_string}");
            }
            let mut split = Vec::new();
            if !start_string.is_empty() {
                split.push(vm.types.new_string(start_string));
            }
            split.push(vm.types.new_string(mid));
            push_ctc(vm, &mut split, &ctc)?;
            if !end_string.is_empty() {
                split.push(vm.types.new_string(end_string));
            }
            split.extend(items.iter().skip(1).cloned());
            items = split;
        } else {
            push_ctc(vm, &mut items, &ctc)?;
        }
        let last = py::get_or_none(vm, this, "last_ctc")?;
        push_ctc(vm, &mut items, &last)?;
        let lent = lenticular(vm);
        let mut toks = Vec::new();
        for i in &items {
            if let Some(s) = i.as_str() {
                toks.extend(tokenize(s, lent).map_err(|e| py::error(vm, e))?);
            } else if super::core::is_displayable(vm, i) {
                toks.push(Token::Displayable(i.clone()));
            } else {
                let r = ops::repr(vm, i)?;
                return Err(py::error(vm, format!("Can't display {r} as Text.")));
            }
        }
        let ct = py::config(vm, "custom_text_tags")?;
        let sct = py::config(vm, "self_closing_custom_text_tags")?;
        let rt = py::config(vm, "replace_text").unwrap_or(Value::None);
        if ops::truthy(vm, &ct)? || ops::truthy(vm, &sct)? || !py::is_none(&rt) {
            toks = apply_custom_tags(vm, toks)?;
        }
        toks
    } else {
        let t = py::get(vm, this, "text")?;
        tokens_from_py(vm, &t)?
    };
    let mut displayables = Vec::new();
    for t in tokens.iter_mut() {
        match t {
            Token::Displayable(d) => displayables.push(d.clone()),
            Token::Tag(s) => {
                if let Some(v) = s.strip_prefix("image=") {
                    let v = vm.types.new_str(v);
                    let d = super::easy::displayable(vm, &v, &Value::None, true)?;
                    displayables.push(d.clone());
                    *t = Token::Displayable(d);
                }
            }
            _ => {}
        }
    }
    let focusable = tokens
        .iter()
        .any(|t| matches!(t, Token::Tag(s) if s.starts_with("a=")));
    let tp = tokens_to_py(vm, &tokens);
    py::set(vm, this, "tokens", tp)?;
    let mut uniq: Vec<Value> = Vec::new();
    for d in displayables {
        if !uniq.iter().any(|u| py::same(u, &d)) {
            uniq.push(d);
        }
    }
    py::set(vm, this, "displayables", vm.types.new_list(uniq))?;
    py::set(vm, this, "focusable", Value::Bool(focusable))?;
    Ok(Value::None)
}

fn apply_custom_tags(vm: &mut Vm, tokens: Vec<Token>) -> PyResult<Vec<Token>> {
    let ct = py::config(vm, "custom_text_tags")?;
    let sct = py::config(vm, "self_closing_custom_text_tags")?;
    let replace = py::config(vm, "replace_text").unwrap_or(Value::None);
    let mut tokens: std::collections::VecDeque<Token> = tokens.into();
    let mut rv = Vec::new();
    while let Some(t) = tokens.pop_front() {
        match &t {
            Token::Text(s) if !py::is_none(&replace) => {
                let v = vm.types.new_str(s);
                let r = vm.call(&replace, vec![v], Vec::new())?;
                rv.push(Token::Text(py::string(vm, &r)?));
            }
            Token::Tag(text) => {
                let (tag, value) = text
                    .split_once('=')
                    .map(|(a, b)| (a.to_owned(), b.to_owned()))
                    .unwrap_or((text.clone(), String::new()));
                let k = vm.types.new_str(&tag);
                let mut f = vm.call_method(&ct, "get", vec![k.clone(), Value::None])?;
                let mut self_closing = false;
                if py::is_none(&f) {
                    f = vm.call_method(&sct, "get", vec![k.clone(), Value::None])?;
                    self_closing = true;
                }
                if py::is_none(&f) {
                    rv.push(t.clone());
                    continue;
                }
                let v = vm.types.new_str(&value);
                let new = if !self_closing {
                    let mut contents = Vec::new();
                    let close = format!("/{tag}");
                    let mut count = 1;
                    while let Some(t2) = tokens.pop_front() {
                        if let Token::Tag(s2) = &t2 {
                            let tag2 = s2.split('=').next().unwrap_or("");
                            if tag2 == tag {
                                count += 1;
                            } else if tag2 == close {
                                count -= 1;
                                if count == 0 {
                                    break;
                                }
                            }
                        }
                        contents.push(t2);
                    }
                    let c = tokens_to_py(vm, &contents);
                    vm.call(&f, vec![k, v, c], Vec::new())?
                } else {
                    vm.call(&f, vec![k, v], Vec::new())?
                };
                let mut nt = tokens_from_py(vm, &new)?;
                nt.extend(tokens.drain(..));
                tokens = nt.into();
            }
            _ => rv.push(t.clone()),
        }
    }
    Ok(rv)
}

fn ensure_updated(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let dirty = py::get_or_none(vm, this, "dirty")?;
    let ds = py::get_or_none(vm, this, "displayables")?;
    if ops::truthy(vm, &dirty)? || py::is_none(&ds) {
        vm.call_method(this, "update", Vec::new())?;
    }
    Ok(())
}

fn t_visit(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    ensure_updated(vm, &args[0])?;
    let ds = py::get(vm, &args[0], "displayables")?;
    let items = py::items(vm, &ds)?;
    Ok(vm.types.new_list(items))
}

fn t_tts(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = all_text(vm, this)?;
    let s = s.split("{done}").next().unwrap_or("").to_owned();
    let s = s.rsplit("{fast}").next().unwrap_or("").to_owned();
    let f = py::import_attr(vm, "renpy.text.extras", "filter_alt_text")?;
    let v = vm.types.new_string(s);
    let rv = vm.call(&f, vec![v], Vec::new())?;
    let alt = sty(vm, this, "alt")?;
    if !py::is_none(&alt) {
        let sub = py::import_attr(vm, "renpy.substitutions", "substitute")?;
        let scope = py::dict(vm);
        py::dict_set(vm, &scope, "text", rv)?;
        let r = vm.call(&sub, vec![alt], vec![("scope".into(), scope)])?;
        return ops::getitem(vm, &r, Value::Int(0));
    }
    Ok(rv)
}

fn t_kill_layout(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    kill_layout(args[0].id());
    Ok(Value::None)
}

fn layout_value(vm: &mut Vm, l: &TextLayout) -> PyResult<Value> {
    let ns = py::import_attr(vm, "types", "SimpleNamespace")?;
    let size = vm
        .types
        .new_tuple(vec![Value::Float(l.size.0), Value::Float(l.size.1)]);
    let targets = vm.types.new_dict();
    for (i, t) in l.hyperlink_targets.iter().enumerate() {
        ops::setitem(vm, &targets, Value::Int(i as i64 + 1), t.clone())?;
    }
    vm.call(
        &ns,
        Vec::new(),
        vec![
            ("size".into(), size),
            ("width".into(), Value::Float(l.width)),
            ("height".into(), Value::Float(l.height)),
            ("baseline".into(), Value::Float(l.baseline)),
            ("max_time".into(), Value::Float(l.max_time)),
            ("hyperlink_targets".into(), targets),
            ("xborder".into(), Value::Float(l.xborder)),
            ("yborder".into(), Value::Float(l.yborder)),
        ],
    )
}

/// `Layout(text, width, height, renders, size_only, splits_from)` built
/// from Python: runs the same layout as `Text.render` and exposes its
/// fields on `this`. The layout joins the text's layout cache entry, which
/// a later `splits_from` resolves through `_text_id`.
pub fn fill_layout_object(
    vm: &mut Vm,
    this: &Value,
    text: &Value,
    width: f64,
    height: f64,
    renders: &Value,
    splits_from: &Value,
) -> PyResult<()> {
    ensure_updated(vm, text)?;
    let tokens = get_tokens(vm, text)?;
    let mut map = HashMap::new();
    if !py::is_none(renders) {
        for k in py::items(vm, renders)? {
            let r = ops::getitem(vm, renders, k.clone())?;
            map.insert(k.id(), r);
        }
    }
    let from = match py::get_opt(vm, splits_from, "_text_id")? {
        Some(Value::Int(id)) => caches().borrow().layouts.get(&id).cloned(),
        _ => None,
    };
    let l = build_layout(vm, text, &tokens, width, height, &map, from.as_deref())?;
    let ns = layout_value(vm, &l)?;
    for n in ["size", "width", "height", "baseline", "max_time", "hyperlink_targets", "xborder", "yborder"] {
        let v = py::get(vm, &ns, n)?;
        py::set(vm, this, n, v)?;
    }
    py::set(vm, this, "_text_id", Value::Int(text.id()))?;
    caches().borrow_mut().layouts.insert(text.id(), Rc::new(l));
    Ok(())
}

fn t_get_layout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = caches().borrow().layouts.get(&args[0].id()).cloned();
    match l {
        Some(l) => layout_value(vm, &l),
        None => Ok(Value::None),
    }
}

fn t_get_virtual_layout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let l = caches()
        .borrow()
        .virtual_layouts
        .get(&args[0].id())
        .cloned();
    match l {
        Some(l) => layout_value(vm, &l),
        None => Ok(Value::None),
    }
}

fn t_set_style_prefix(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let prefix = args.get(1).cloned().unwrap_or(Value::None);
    let s = py::get(vm, this, "style")?;
    let cur = py::get(vm, &s, "prefix")?;
    if !ops::eq(vm, &prefix, &cur)? {
        kill_layout(this.id());
    }
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "set_style_prefix")?;
    vm.call(&f, args.to_vec(), Vec::new())
}

fn child_renders(vm: &mut Vm, this: &Value, width: f64) -> PyResult<HashMap<i64, Value>> {
    let ds = py::get(vm, this, "displayables")?;
    let size = opt_f(vm, this, "size")?;
    let mut renders = HashMap::new();
    for d in py::items(vm, &ds)? {
        let r = render::render(vm, &d, width, size, 0., 0.)?;
        renders.insert(d.id(), r);
    }
    Ok(renders)
}

fn t_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = super::core::displayable_class(vm);
    let f = py::get(vm, &Value::Obj(d), "get_placement")?;
    let rv = vm.call(&f, vec![this.clone()], Vec::new())?;
    let mut p = py::items(vm, &rv)?;
    let is_baseline = p.get(3).is_some_and(|v| {
        v.obj().is_some_and(|o| o.class_name() == "Sentinel") || v.as_str() == Some("baseline")
    });
    if !is_baseline {
        return Ok(rv);
    }
    let cached = caches()
        .borrow()
        .virtual_layouts
        .get(&this.id())
        .map(|l| l.baseline);
    let baseline = match cached {
        Some(b) => b,
        None => {
            ensure_updated(vm, this)?;
            let renders = child_renders(vm, this, 4096.)?;
            let tokens = get_tokens(vm, this)?;
            build_layout(vm, this, &tokens, 4096., 4096., &renders, None)?.baseline
        }
    };
    p[3] = Value::Float(baseline);
    Ok(vm.types.new_tuple(p))
}

fn hyperlink_funcs(vm: &mut Vm, this: &Value) -> PyResult<Vec<Value>> {
    let f = sty(vm, this, "hyperlink_functions")?;
    if py::is_none(&f) {
        return Ok(Vec::new());
    }
    py::items(vm, &f)
}

fn current_target(vm: &mut Vm, this: &Value) -> PyResult<Option<Value>> {
    let l = caches().borrow().layouts.get(&this.id()).cloned();
    let Some(l) = l else { return Ok(None) };
    let arg = py::import_attr(vm, "renpy.display.focus", "argument")?;
    Ok(match arg {
        Value::Int(i) if i >= 1 => l.hyperlink_targets.get(i as usize - 1).cloned(),
        _ => None,
    })
}

fn t_focus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let target = current_target(vm, this)?;
    let had = caches().borrow().layouts.contains_key(&this.id());
    kill_layout(this.id());
    render::redraw(vm, this, 0.);
    if !had {
        return Ok(Value::None);
    }
    if !default {
        let snd = sty(vm, this, "hover_sound")?;
        if !py::is_none(&snd) {
            let play = py::import_attr(vm, "renpy.exports", "play")?;
            vm.call(&play, vec![snd], Vec::new())?;
        }
    }
    let funcs = hyperlink_funcs(vm, this)?;
    if let (Some(hf), false, Some(t)) = (funcs.get(2), default, target) {
        if !py::is_none(hf) {
            return vm.call(hf, vec![t], Vec::new());
        }
    }
    Ok(Value::None)
}

fn t_unfocus(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let default = match arg(args, kw, 1, "default") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    kill_layout(this.id());
    render::redraw(vm, this, 0.);
    let funcs = hyperlink_funcs(vm, this)?;
    if let Some(hf) = funcs.get(2) {
        if !py::is_none(hf) && !default {
            return vm.call(hf, vec![Value::None], Vec::new());
        }
    }
    Ok(Value::None)
}

fn t_hyperlink_sensitive(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let funcs = hyperlink_funcs(vm, &args[0])?;
    if funcs.len() < 4 {
        return Ok(Value::Bool(true));
    }
    vm.call(
        &funcs[3],
        vec![args.get(1).cloned().unwrap_or(Value::None)],
        Vec::new(),
    )
}

fn map_event(vm: &mut Vm, ev: &Value, name: &str) -> PyResult<bool> {
    let f = py::import_attr(vm, "renpy.display.behavior", "map_event")?;
    let n = vm.types.new_str(name);
    let r = vm.call(&f, vec![ev.clone(), n], Vec::new())?;
    ops::truthy(vm, &r)
}

fn ignore_event(vm: &mut Vm) -> crysirror_python::PyErr {
    match py::import_attr(vm, "renpy.display.core", "IgnoreEvent")
        .and_then(|c| vm.call(&c, Vec::new(), Vec::new()))
    {
        Ok(e) => crysirror_python::PyErr::Exc(e.obj().cloned().unwrap()),
        Err(e) => e,
    }
}

fn t_event(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let ev = args.get(1).cloned().unwrap_or(Value::None);
    let x = py::f64_of(vm, &args.get(2).cloned().unwrap_or(Value::Int(0)))?;
    let y = py::f64_of(vm, &args.get(3).cloned().unwrap_or(Value::Int(0)))?;
    let st = args.get(4).cloned().unwrap_or(Value::Int(0));
    let slow_done = py::get_or_none(vm, this, "slow_done")?;
    let slow = {
        let v = py::get_or_none(vm, this, "slow")?;
        ops::truthy(vm, &v)?
    };
    if !py::is_none(&slow_done) && !slow {
        if super::events::is_type(vm, &ev, super::events::TIMEEVENT)? {
            let m = py::get_or_none(vm, &ev, "modal")?;
            if ops::truthy(vm, &m)? {
                let iface = py::import_attr(vm, "renpy.game", "interface")?;
                vm.call_method(&iface, "timeout", vec![Value::Float(0.05)])?;
                return Ok(Value::None);
            }
        }
        py::set(vm, this, "slow_done", Value::None)?;
        vm.call(&slow_done, Vec::new(), Vec::new())?;
    }
    if slow && map_event(vm, &ev, "dismiss")? && style_b(vm, this, "slow_abortable")? {
        let list = caches().borrow().slow_text.clone();
        for i in list {
            let s = py::get_or_none(vm, &i, "slow")?;
            if ops::truthy(vm, &s)? {
                py::set(vm, &i, "slow", Value::Bool(false))?;
            }
            let sd = py::get_or_none(vm, &i, "slow_done")?;
            if !py::is_none(&sd) {
                py::set(vm, &i, "slow_done", Value::None)?;
                vm.call(&sd, Vec::new(), Vec::new())?;
            }
        }
        return Err(ignore_event(vm));
    }
    if !caches().borrow().layouts.contains_key(&this.id()) {
        return Ok(Value::None);
    }
    let offs = py::get_or_none(vm, this, "displayable_offsets")?;
    for o in py::items(vm, &offs).unwrap_or_default() {
        let o = py::items(vm, &o)?;
        let (xo, yo) = (py::f64_of(vm, &o[1])?, py::f64_of(vm, &o[2])?);
        let rv = vm.call_method(
            &o[0],
            "event",
            vec![
                ev.clone(),
                Value::Float(x - xo),
                Value::Float(y - yo),
                st.clone(),
            ],
        )?;
        if !py::is_none(&rv) {
            return Ok(rv);
        }
    }
    let focused = vm.call_method(this, "is_focused", Vec::new())?;
    if ops::truthy(vm, &focused)? && map_event(vm, &ev, "button_select")? {
        let snd = sty(vm, this, "activate_sound")?;
        if !py::is_none(&snd) {
            let play = py::import_attr(vm, "renpy.exports", "play")?;
            vm.call(&play, vec![snd], Vec::new())?;
        }
        let funcs = hyperlink_funcs(vm, this)?;
        if let Some(clicked) = funcs.get(1).filter(|c| !py::is_none(c)) {
            let target = current_target(vm, this)?.unwrap_or(Value::None);
            let sens = vm.call_method(this, "hyperlink_sensitive", vec![target.clone()])?;
            if !ops::truthy(vm, &sens)? {
                return Ok(Value::None);
            }
            let rv = vm.call(clicked, vec![target], Vec::new())?;
            if py::is_none(&rv) {
                return Err(ignore_event(vm));
            }
            return Ok(rv);
        }
    }
    Ok(Value::None)
}

fn t_size(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let width = py::f64_of(vm, &arg_or(args, kw, 1, "width", Value::Int(4096)))?;
    let height = py::f64_of(vm, &arg_or(args, kw, 2, "height", Value::Int(4096)))?;
    ensure_updated(vm, this)?;
    let renders = child_renders(vm, this, width)?;
    let tokens = get_tokens(vm, this)?;
    let l = build_layout(vm, this, &tokens, width, height, &renders, None)?;
    Ok(vm
        .types
        .new_tuple(vec![Value::Float(l.size.0), Value::Float(l.size.1)]))
}

fn t_get_time(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let _ = vm;
    Ok(Value::Float(
        caches()
            .borrow()
            .layouts
            .get(&args[0].id())
            .map(|l| l.max_time)
            .unwrap_or(0.),
    ))
}

fn t_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let (mut width, mut height, st, at) = super::layout::rargs(vm, args)?;
    let vertical = style_b(vm, this, "vertical")?;
    if vertical {
        std::mem::swap(&mut width, &mut height);
    }
    let slow_v = py::get_or_none(vm, this, "slow")?;
    if py::is_none(&slow_v) {
        let sc = sty(vm, this, "slow_cps")?;
        let s = ops::truthy(vm, &sc)?;
        py::set(vm, this, "slow", Value::Bool(s))?;
    }
    let mut slow = {
        let v = py::get_or_none(vm, this, "slow")?;
        ops::truthy(vm, &v)?
    };
    let sdt = py::get_or_none(vm, this, "slow_done_time")?;
    if !slow && !py::is_none(&sdt) {
        py::set(vm, this, "slow_done_time", Value::Float(st))?;
    }
    ensure_updated(vm, this)?;
    let id = this.id();
    let tokens = get_tokens(vm, this)?;
    let cached_v = caches().borrow().virtual_layouts.get(&id).cloned();
    let virtual_layout =
        match cached_v.filter(|l| l.width == width && l.height == height && !l.pending) {
            Some(l) => l,
            None => {
                let renders = child_renders(vm, this, width)?;
                let l = Rc::new(build_layout(
                    vm, this, &tokens, width, height, &renders, None,
                )?);
                caches().borrow_mut().virtual_layouts.insert(id, l.clone());
                l
            }
        };
    let cached = caches().borrow().layouts.get(&id).cloned();
    let layout = match cached.filter(|l| l.width == width && l.height == height && !l.pending) {
        Some(l) => l,
        None => {
            let renders = child_renders(vm, this, width)?;
            let l = Rc::new(build_layout(
                vm,
                this,
                &tokens,
                width,
                height,
                &renders,
                Some(&virtual_layout),
            )?);
            caches().borrow_mut().layouts.insert(id, l.clone());
            l
        }
    };
    if layout.pending {
        render::redraw(vm, this, 0.);
    }
    let (vw, vh) = virtual_layout.size;
    let mut rv = RenderData::new(vw, vh);
    if slow && st > layout.max_time {
        slow = false;
        py::set(vm, this, "slow", Value::Bool(false))?;
        let iface = py::import_attr(vm, "renpy.game", "interface")?;
        if !py::is_none(&iface) {
            vm.call_method(&iface, "timeout", vec![Value::Int(0)])?;
        }
    }
    let blits = render_layout(vm, this, &layout, &rv, st, slow)?;
    rv.children.extend(blits);
    // Inline displayables.
    let mut offsets = Vec::new();
    for g in &layout.glyphs {
        let Piece::Disp(_, d, _, _) = &layout.pieces[g.piece] else {
            continue;
        };
        if g.ch != '\u{fffc}' {
            continue;
        }
        if slow && g.time > st {
            continue;
        }
        let sdt = py::get_or_none(vm, this, "slow_done_time")?;
        let cst = if py::is_none(&sdt) {
            st - g.time
        } else {
            st - py::f64_of(vm, &sdt)?.min(g.time)
        };
        let pl = vm.call_method(d, "get_placement", Vec::new())?;
        let (pxo, pyo) = super::core::place(vm, &pl, g.width, g.ascent, g.width, g.line_spacing)?;
        let xo = g.x + pxo + layout.xoffset;
        let yo = g.y + pyo + layout.yoffset;
        let size = opt_f(vm, this, "size")?;
        let cr = render::render(vm, d, width, size, cst, at)?;
        let source = render::source_of(vm, &cr)?;
        rv.children.push(render::Blit {
            source,
            x: xo,
            y: yo,
            focus: true,
            main: true,
        });
        offsets.push(
            vm.types
                .new_tuple(vec![d.clone(), Value::Float(xo), Value::Float(yo)]),
        );
    }
    py::set(vm, this, "displayable_offsets", vm.types.new_list(offsets))?;
    for (link, hx, hy, hw, hh, valid) in &layout.hyperlinks {
        if st >= *valid || !slow {
            rv.focuses.push(render::Focus {
                d: this.clone(),
                arg: Value::Int(*link),
                rect: Some([hx + layout.xoffset, hy + layout.yoffset, *hw, *hh]),
                mask: None,
            });
        }
    }
    if slow {
        render::redraw(vm, this, 0.);
    }
    let mut out = new_render(vm, rv);
    if vertical {
        let (rw, rh) = super::layout::size(&out);
        let mut vd = RenderData::new(rh, rw);
        vd.forward = Some(Mat([
            0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]));
        vd.reverse = Some(Mat([
            0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]));
        let src = render::source_of(vm, &out)?;
        vd.children.push(render::Blit {
            source: src,
            x: rh,
            y: 0.,
            focus: true,
            main: true,
        });
        out = new_render(vm, vd);
    }
    let _ = render_data(&out);
    Ok(out)
}

fn t_tokenize(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let text = args.get(1).cloned().unwrap_or(Value::None);
    let lent = lenticular(vm);
    let mut toks = Vec::new();
    for i in text_items(vm, &text)? {
        if let Some(s) = i.as_str() {
            toks.extend(tokenize(s, lent).map_err(|e| py::error(vm, e))?);
        } else if super::core::is_displayable(vm, &i) {
            toks.push(Token::Displayable(i));
        } else {
            let r = ops::repr(vm, &i)?;
            return Err(py::error(vm, format!("Can't display {r} as Text.")));
        }
    }
    Ok(tokens_to_py(vm, &toks))
}

fn f_apply_custom_tags(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = tokens_from_py(vm, &args.first().cloned().unwrap_or(Value::None))?;
    let r = apply_custom_tags(vm, t)?;
    Ok(tokens_to_py(vm, &r))
}

fn t_get_displayables(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let tokens = tokens_from_py(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let mut ds = Vec::new();
    let mut out = Vec::new();
    for t in tokens {
        match t {
            Token::Displayable(d) => {
                ds.push(d.clone());
                out.push(Token::Displayable(d));
            }
            Token::Tag(s) if s.starts_with("image=") => {
                let v = vm.types.new_str(&s[6..]);
                let d = super::easy::displayable(vm, &v, &Value::None, true)?;
                ds.push(d.clone());
                out.push(Token::Displayable(d));
            }
            t => out.push(t),
        }
    }
    let tp = tokens_to_py(vm, &out);
    let set = vm.call(
        &Value::Obj(vm.types.set.clone()),
        vec![vm.types.new_list(ds)],
        Vec::new(),
    )?;
    Ok(vm.types.new_tuple(vec![tp, set]))
}

// extras -------------------------------------------------------------------------------------

const TEXT_TAGS: &[(&str, bool)] = &[
    ("alpha", true),
    ("alt", true),
    ("art", true),
    ("axis", true),
    ("done", false),
    ("instance", true),
    ("image", false),
    ("p", false),
    ("w", false),
    ("fast", false),
    ("b", true),
    ("i", true),
    ("u", true),
    ("a", true),
    ("plain", true),
    ("font", true),
    ("color", true),
    ("outlinecolor", true),
    ("size", true),
    ("noalt", true),
    ("nw", false),
    ("s", true),
    ("shader", true),
    ("rt", true),
    ("rb", true),
    ("k", true),
    ("cps", true),
    ("space", false),
    ("vspace", false),
    ("", true),
];

fn f_check_text_tags(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &arg_or(args, kw, 0, "s", Value::None))?;
    let check_unclosed = ops::truthy(
        vm,
        &arg_or(args, kw, 1, "check_unclosed", Value::Bool(false)),
    )?;
    let mut all: HashMap<String, bool> = TEXT_TAGS
        .iter()
        .map(|(k, v)| ((*k).to_owned(), *v))
        .collect();
    let ct = py::config(vm, "custom_text_tags")?;
    if ops::truthy(vm, &ct)? {
        for (k, _) in ops::mapping_items(vm, &ct)? {
            all.insert(py::string(vm, &k)?, true);
        }
    }
    let sct = py::config(vm, "self_closing_custom_text_tags")?;
    if ops::truthy(vm, &sct)? {
        for (k, _) in ops::mapping_items(vm, &sct)? {
            all.insert(py::string(vm, &k)?, false);
        }
    }
    let tokens = match tokenize(&s, false) {
        Ok(t) => t,
        Err(e) => return Ok(vm.types.new_string(e)),
    };
    let mut stack: Vec<String> = Vec::new();
    for t in tokens {
        let Token::Tag(text) = t else { continue };
        if text.starts_with('#') {
            continue;
        }
        let text = text
            .split('=')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_owned();
        if let Some(c) = text.strip_prefix('/') {
            match stack.last() {
                None => {
                    return Ok(vm.types.new_string(format!(
                        "Close text tag '{{{text}}}' does not match an open text tag."
                    )));
                }
                Some(top) if top != c => {
                    return Ok(vm.types.new_string(format!(
                        "Close text tag '{{{text}}}' does not match open text tag '{{{top}}}'."
                    )));
                }
                _ => {
                    stack.pop();
                }
            }
            continue;
        }
        match all.get(&text) {
            None => {
                return Ok(vm
                    .types
                    .new_string(format!("Text tag '{text}' is not known.")));
            }
            Some(true) => stack.push(text),
            Some(false) => {}
        }
    }
    if check_unclosed && !stack.is_empty() {
        let l: Vec<String> = stack.iter().map(|s| format!("'{s}'")).collect();
        return Ok(vm.types.new_string(format!(
            "One or more text tags were left open at the end of the string: {}",
            l.join(", ")
        )));
    }
    Ok(Value::None)
}

fn f_filter_text_tags(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &arg_or(args, kw, 0, "s", Value::None))?;
    let allow = arg_or(args, kw, 1, "allow", Value::None);
    let deny = arg_or(args, kw, 2, "deny", Value::None);
    if py::is_none(&allow) == py::is_none(&deny) {
        return Err(py::error(
            vm,
            "Only one of the allow and deny keyword arguments should be given to filter_text_tags.",
        ));
    }
    let tokens = tokenize(&s, false).map_err(|e| py::error(vm, e))?;
    let mut rv = String::new();
    for t in tokens {
        match t {
            Token::Paragraph => rv.push('\n'),
            Token::Tag(text) => {
                let kind = text
                    .split('=')
                    .next()
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("");
                let kind = kind.strip_prefix('/').unwrap_or(kind);
                let k = vm.types.new_str(kind);
                let keep = if !py::is_none(&allow) {
                    ops::contains(vm, &allow, &k)?
                } else {
                    !ops::contains(vm, &deny, &k)?
                };
                if keep {
                    rv.push('{');
                    rv.push_str(&text);
                    rv.push('}');
                }
            }
            Token::Text(t) => rv.push_str(&t.replace('{', "{{")),
            Token::Displayable(_) => {}
        }
    }
    Ok(vm.types.new_string(rv))
}

fn f_filter_alt_text(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args.first().cloned().unwrap_or(Value::None))?;
    let mut tokens = tokenize(&s, false).map_err(|e| py::error(vm, e))?;
    let ct = py::config(vm, "custom_text_tags")?;
    let sct = py::config(vm, "self_closing_custom_text_tags")?;
    if ops::truthy(vm, &ct)? || ops::truthy(vm, &sct)? {
        tokens = apply_custom_tags(vm, tokens)?;
    }
    let filter = py::config(vm, "tts_filter_tags").unwrap_or(Value::None);
    let mut active: Vec<String> = Vec::new();
    let mut rv = String::new();
    for t in tokens {
        match t {
            Token::Paragraph => rv.push('\n'),
            Token::Tag(text) => {
                let kind = text.split('=').next().unwrap_or("");
                let (kind, end) = match kind.strip_prefix('/') {
                    Some(k) => (k, true),
                    None => (kind, false),
                };
                let k = vm.types.new_str(kind);
                if !py::is_none(&filter) && ops::contains(vm, &filter, &k)? {
                    if end {
                        active.retain(|a| a != kind);
                    } else {
                        active.push(kind.to_owned());
                    }
                }
            }
            Token::Displayable(d) => {
                let s = vm.call_method(&d, "_tts", Vec::new())?;
                rv.push_str(&py::string(vm, &s)?);
            }
            Token::Text(t) => {
                if active.is_empty() {
                    rv.push_str(&t);
                }
            }
        }
    }
    Ok(vm.types.new_string(rv))
}

fn pt_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let style = arg_or(args, kw, 1, "style", vm.types.new_str("default"));
    let props: Vec<_> = kw
        .iter()
        .filter(|(k, _)| &**k != "style")
        .cloned()
        .collect();
    py::set(vm, this, "style", style)?;
    let d = py::kwargs_dict(vm, &props)?;
    py::set(vm, this, "properties", d)?;
    Ok(Value::None)
}

fn pt_duplicate(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let a = args.get(1).cloned().unwrap_or(Value::None);
    let style = py::get(vm, this, "style")?;
    let props = py::get(vm, this, "properties")?;
    let mut kw = py::str_items(vm, &props)?;
    kw.push(("style".into(), style));
    let text_c = py::import_attr(vm, "renpy.text.text", "Text")?;
    let lint = py::get_or_none(vm, &a, "lint")?;
    if ops::truthy(vm, &lint)? {
        let e = vm.types.new_str("");
        return vm.call(&text_c, vec![e], kw);
    }
    let aa = py::get(vm, &a, "args")?;
    let parts = py::items(vm, &aa)?;
    if parts.is_empty() {
        let n = py::get(vm, &a, "name")?;
        let n: Vec<String> = py::items(vm, &n)?
            .iter()
            .map(|x| x.as_str().unwrap_or("").to_owned())
            .collect();
        return Err(py::error(
            vm,
            format!("'{}' takes a single string parameter.", n.join(" ")),
        ));
    }
    let param: String = parts
        .iter()
        .map(|p| p.as_str().unwrap_or("").to_owned())
        .collect();
    let e = py::import_attr(vm, "renpy.python", "py_eval")?;
    let p = vm.types.new_string(param);
    let s = vm.call(&e, vec![p], Vec::new())?;
    vm.call(&text_c, vec![s], kw)
}

fn f_textwrap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = arg_or(args, kw, 0, "s", Value::None);
    let s = py::string(vm, &s)?;
    let width = py::f64_of(vm, &arg_or(args, kw, 1, "width", Value::Int(78)))?;
    let asian = arg_or(args, kw, 2, "asian", Value::Bool(false));
    let asian = ops::truthy(vm, &asian)?;
    let eaw = py::import_attr(vm, "unicodedata", "east_asian_width")?;
    let rel = Rel::of(vm);
    let mut gs = Vec::new();
    for c in s.chars() {
        let cv = vm.types.new_string(c.to_string());
        let e = vm.call(&eaw, vec![cv], Vec::new())?;
        let e = py::string(vm, &e)?;
        let w = match e.as_str() {
            "F" | "W" => 20.,
            "A" if asian => 20.,
            _ => 10.,
        };
        gs.push(G {
            ch: c,
            ascent: 10.,
            line_spacing: 10.,
            width: w,
            advance: w,
            ..G::default()
        });
    }
    super::textsupport::annotate_unicode(rel, &mut gs, false, 2);
    super::texwrap::linebreak_tex(rel, &mut gs, width * 10., width * 10., false);
    let l = super::textsupport::linebreak_list(&gs)
        .into_iter()
        .map(|s| vm.types.new_string(s))
        .collect();
    Ok(vm.types.new_list(l))
}

fn f_thaic90(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &arg_or(args, kw, 0, "s", Value::None))?;
    let out = if Rel::of(vm).at_least(8, 6) {
        crysirror_text::thai_c90::reencode(&s)
    } else {
        crysirror_text::thai_c90::legacy(&s)
    };
    Ok(vm.types.new_string(out))
}

/// `unmap_arabic_presentation_forms(s)` (7.8/8.3): base letters for the
/// presentation forms when `config.reverse_arabic_presentation_forms`.
fn f_unmap_arabic(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = arg_or(args, kw, 0, "s", Value::None);
    let on = py::config(vm, "reverse_arabic_presentation_forms").unwrap_or(Value::Bool(false));
    if !ops::truthy(vm, &on)? {
        return Ok(v);
    }
    let s = py::string(vm, &v)?;
    Ok(vm.types.new_string(crysirror_text::arabic::unmap_presentation_forms(&s)))
}

fn init_extras(vm: &mut Vm) -> PyResult<ObjRef> {
    let rel = Rel::of(vm);
    // filter_text_tags (7.0+) and filter_alt_text (7.4+) stay in every
    // release: this runtime's say behaviour and self-voicing paths call
    // them for all releases.
    let mut fns: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("check_text_tags", f_check_text_tags),
        ("filter_text_tags", f_filter_text_tags),
        ("filter_alt_text", f_filter_alt_text),
    ];
    if rel.at_least(6, 99) {
        fns.push(("textwrap", f_textwrap));
    }
    if rel.since_pair(6, 1) {
        fns.push(("thaic90", f_thaic90));
    }
    let arabic = rel.since_pair(8, 3) && !rel.at_least(8, 4);
    if arabic {
        fns.push(("unmap_arabic_presentation_forms", f_unmap_arabic));
    }
    let m = crysirror_python::modules::module_with(vm, "renpy.text.extras", &fns);
    if arabic {
        let d = vm.types.new_dict();
        let mut rows: Vec<(char, &str)> = crysirror_text::arabic::reverse_mappings()
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        rows.sort();
        for (k, v) in rows {
            let k = vm.types.new_string(k.to_string());
            let v = vm.types.new_str(v);
            ops::setitem(vm, &d, k, v)?;
        }
        crysirror_python::modules::set(vm, &m, "arabic_reverse_mappings", d);
    }
    let tt = vm.types.new_dict();
    for (k, v) in TEXT_TAGS {
        py::dict_set(vm, &tt, k, Value::Bool(*v))?;
    }
    crysirror_python::modules::set(vm, &m, "text_tags", tt);
    for (n, v, on) in [
        ("TEXT", TEXT, true),
        ("TAG", TAG, true),
        ("PARAGRAPH", PARAGRAPH, rel.at_least(7, 0)),
        ("DISPLAYABLE", DISPLAYABLE, rel.since_pair(5, 0)),
    ] {
        if on {
            crysirror_python::modules::set(vm, &m, n, Value::Int(v));
        }
    }
    let pt = py::class(
        vm,
        "renpy.text.extras",
        "ParameterizedText",
        &[],
        &[("__init__", pt_init), ("_duplicate", pt_duplicate)],
        &[("_duplicatable", Value::Bool(true))],
    )?;
    crysirror_python::modules::set(vm, &m, "ParameterizedText", Value::Obj(pt));
    Ok(m)
}

fn init_text(vm: &mut Vm) -> PyResult<ObjRef> {
    let keep = Rel::of(vm).at_least(8, 6);
    KEEP_DANGLING.with(|k| k.set(keep));
    let m = crysirror_python::modules::module_with(
        vm,
        "renpy.text.text",
        &[
            ("layout_cache_clear", |_, _, _| {
                layout_cache_clear();
                Ok(Value::None)
            }),
            ("text_tick", |_, _, _| {
                text_tick();
                Ok(Value::None)
            }),
        ],
    );
    let d = Value::Obj(super::core::displayable_class(vm));
    let cls = py::class(
        vm,
        "renpy.text.text",
        "Text",
        &[d],
        &[
            ("__init__", t_init),
            ("after_upgrade", t_after_upgrade),
            ("_duplicate", t_duplicate),
            ("_in_current_store", t_in_current_store),
            ("_repr_info", t_repr_info),
            ("get_all_text", t_get_all_text),
            ("_scope", t_scope),
            ("set_text", t_set_text),
            ("per_interact", t_per_interact),
            ("set_ctc", t_set_ctc),
            ("set_last_ctc", t_set_last_ctc),
            ("update", t_update),
            ("visit", t_visit),
            ("_tts", t_tts),
            ("_tts_all", t_tts),
            ("kill_layout", t_kill_layout),
            ("get_layout", t_get_layout),
            ("get_virtual_layout", t_get_virtual_layout),
            ("set_style_prefix", t_set_style_prefix),
            ("get_placement", t_get_placement),
            ("focus", t_focus),
            ("unfocus", t_unfocus),
            ("hyperlink_sensitive", t_hyperlink_sensitive),
            ("event", t_event),
            ("size", t_size),
            ("get_time", t_get_time),
            ("render", t_render),
            ("tokenize", t_tokenize),
            ("get_displayables", t_get_displayables),
        ],
        &[
            ("__version__", Value::Int(4)),
            ("_uses_scope", Value::Bool(true)),
            ("_duplicatable", Value::Bool(false)),
            ("locked", Value::Bool(false)),
            ("language", Value::None),
            ("mask", Value::None),
            ("last_ctc", Value::None),
            ("tokenized", Value::Bool(false)),
            ("slow_done_time", Value::None),
        ],
    )?;
    let act = vm
        .types
        .new_builtin("apply_custom_tags", f_apply_custom_tags);
    let act = py::staticmethod(vm, act);
    vm.types.set_attr(&cls, "apply_custom_tags", act);
    crysirror_python::modules::set(vm, &m, "Text", Value::Obj(cls));
    for (n, v) in [
        ("TEXT", TEXT),
        ("TAG", TAG),
        ("PARAGRAPH", PARAGRAPH),
        ("DISPLAYABLE", DISPLAYABLE),
    ] {
        crysirror_python::modules::set(vm, &m, n, Value::Int(v));
    }
    let pt = py::import_attr(vm, "renpy.text.extras", "ParameterizedText")?;
    crysirror_python::modules::set(vm, &m, "ParameterizedText", pt);
    let lt = vm
        .types
        .new_builtin("language_tailor", |_, _, _| Ok(Value::None));
    crysirror_python::modules::set(vm, &m, "language_tailor", lt);
    crysirror_python::modules::set(vm, &m, "BASELINE", Value::Int(-65536));
    super::textclasses::install_names(vm, &m)?;
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    super::textsupport::install(vm);
    super::texwrap::install(vm);
    super::textunicode::install(vm);
    vm.register_native_module("renpy.text.extras", init_extras);
    vm.register_native_module("renpy.text.text", init_text);
}
