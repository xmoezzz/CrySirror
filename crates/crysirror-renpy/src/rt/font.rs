// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.font`: font lookup, FontGroup and image fonts, after Ren'Py
//! 8.3 `font.py` (MIT).
//!
//! TrueType/OpenType faces are registered with the shared client font
//! library through the game's asset capability and measured there; glyphs
//! are drawn as retained glyph runs. Image fonts (SFont, MudgeFont, BMFont)
//! become per-glyph image cells of the source image.
use super::py::{self, *};
use crysirror_core::service::{FontJob, FontLoadSource, FontLoadSpec, FontLoadState};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const MODULE: &str = "renpy.text.font";

enum Face {
    Loading(FontJob),
    Ready { alias: String, ascent: f64, line_height: f64, descent: f64 },
    Failed(String),
}

/// One image-font cell.
#[derive(Clone, Debug)]
pub struct Cell {
    pub asset: String,
    /// Source rectangle in the page image and the page size.
    pub rect: [f64; 4],
    pub page: [f64; 2],
    pub offset: [f64; 2],
    pub width: f64,
    pub advance: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ImageFontData {
    pub height: f64,
    pub baseline: f64,
    pub default_kern: f64,
    pub kerns: HashMap<(char, char), f64>,
    pub cells: HashMap<char, Cell>,
}

#[derive(Default)]
struct FontState {
    faces: HashMap<String, Face>,
    metrics: HashMap<(String, u32, char), (f64, f64)>,
    image_fonts: HashMap<String, Rc<ImageFontData>>,
}

fn state() -> Rc<RefCell<FontState>> {
    thread_local! {
        static S: Rc<RefCell<FontState>> = Rc::new(RefCell::new(FontState::default()));
    }
    S.with(|s| s.clone())
}

/// A resolved font for layout.
#[derive(Clone, Debug)]
pub enum Font {
    Outline { alias: String, size: f64, bold: bool, ascent: f64, line_height: f64, descent: f64 },
    Image { data: Rc<ImageFontData>, scale: f64 },
}

/// Metrics of one laid-out glyph (before placement).
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    pub ascent: f64,
    pub line_spacing: f64,
    pub width: f64,
    pub advance: f64,
}

pub enum Lookup {
    Ready(Font),
    Pending,
}

fn is_zerowidth(c: char) -> bool {
    matches!(c as u32, 0x200b | 0x200c | 0x200d | 0x2060 | 0xfeff) || (0xfe00..=0xfe0f).contains(&(c as u32))
}

/// Resolves a Ren'Py font name (file name or image-font name) at a size.
pub fn get_font(vm: &mut Vm, name: &str, size: f64, bold: bool, italic: bool) -> PyResult<Lookup> {
    // config.font_replacement_map[(fn, bold, italics)]
    let (mut fname, mut bold, mut italic) = (name.to_owned(), bold, italic);
    if let Ok(map) = py::config(vm, "font_replacement_map") {
        if !py::is_none(&map) {
            let n = vm.types.new_str(&fname);
            let key = vm.types.new_tuple(vec![n, Value::Bool(bold), Value::Bool(italic)]);
            let r = vm.call_method(&map, "get", vec![key, Value::None])?;
            if !py::is_none(&r) {
                let t = py::items(vm, &r)?;
                fname = py::string(vm, &t[0])?;
                bold = ops::truthy(vm, &t[1])?;
                italic = ops::truthy(vm, &t[2])?;
            }
        }
    }
    let _ = italic;
    // Image fonts registered under (name, size, bold, italics).
    if let Some(f) = image_font(vm, &fname, size, bold, italic)? {
        return Ok(Lookup::Ready(f));
    }
    let face_index = fname.split_once('@').and_then(|(_, i)| i.parse::<u32>().ok()).unwrap_or(0);
    let file = fname.split('@').next().unwrap_or(&fname).to_owned();
    let st = state();
    let key = format!("{file}@{face_index}");
    let status = {
        let s = st.borrow();
        match s.faces.get(&key) {
            Some(Face::Ready { alias, ascent, line_height, descent }) => Some(Ok((alias.clone(), *ascent, *line_height, *descent))),
            Some(Face::Failed(e)) => Some(Err(e.clone())),
            Some(Face::Loading(_)) => None,
            None => None,
        }
    };
    match status {
        Some(Ok((alias, ascent, line_height, descent))) => {
            return Ok(Lookup::Ready(Font::Outline { alias, size, bold, ascent: ascent * size, line_height: line_height * size, descent: descent * size }));
        }
        Some(Err(e)) => return Err(py::error(vm, e)),
        None => {}
    }
    let fonts = super::host(vm).borrow().fonts.clone().ok_or_else(|| py::error(vm, "font service unavailable"))?;
    let loading = matches!(st.borrow().faces.get(&key), Some(Face::Loading(_)));
    if !loading {
        let path = font_path(vm, &file).ok_or_else(|| py::error(vm, format!("Couldn't find file '{file}'.")))?;
        let spec = FontLoadSpec { name: format!("renpy:{key}"), source: FontLoadSource::Storage(path), face_index };
        let job = fonts.begin(spec).map_err(|e| py::error(vm, format!("{file}: {e}")))?;
        st.borrow_mut().faces.insert(key.clone(), Face::Loading(job));
    }
    let job = match st.borrow().faces.get(&key) {
        Some(Face::Loading(j)) => *j,
        _ => return Ok(Lookup::Pending),
    };
    match fonts.status(job).map_err(|e| py::error(vm, e.to_string()))? {
        FontLoadState::Pending => Ok(Lookup::Pending),
        FontLoadState::Failed(e) => {
            st.borrow_mut().faces.insert(key, Face::Failed(format!("{file}: {e}")));
            Err(py::error(vm, format!("{file}: {e}")))
        }
        FontLoadState::Ready(info) => {
            let em = fonts.outline_em_metrics(&info.alias, 1.).ok();
            let m = fonts.metrics(&info.alias, 100., 'x').ok();
            let ascent = em.map(|e| e.ascent as f64).or(m.map(|m| m.ascent as f64 / 100.)).unwrap_or(0.8);
            let descent = m.map(|m| (m.descent as f64 / 100.).abs()).unwrap_or(0.2);
            let line_height = em.map(|e| e.line_height as f64).unwrap_or(ascent + descent);
            st.borrow_mut().faces.insert(key, Face::Ready { alias: info.alias.clone(), ascent, line_height, descent });
            Ok(Lookup::Ready(Font::Outline { alias: info.alias, size, bold, ascent: ascent * size, line_height: line_height * size, descent: descent * size }))
        }
    }
}

fn font_path(vm: &mut Vm, file: &str) -> Option<crysirror_core::path::VfsPath> {
    for candidate in [file.to_owned(), format!("fonts/{file}")] {
        if let Some(p) = super::asset_path(vm, &candidate) {
            return crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &p).ok();
        }
    }
    None
}

impl Font {
    /// Per-character metrics. Ren'Py's glyph ascent and line spacing are the
    /// face's line values for every glyph of the run.
    pub fn glyphs(&self, vm: &mut Vm, s: &str) -> PyResult<Vec<(char, Metrics)>> {
        let mut out = Vec::new();
        match self {
            Font::Outline { alias, size, ascent, line_height, .. } => {
                let fonts = super::host(vm).borrow().fonts.clone().ok_or_else(|| py::error(vm, "font service unavailable"))?;
                let st = state();
                let size_key = (size * 64.).round() as u32;
                for c in s.chars() {
                    let key = (alias.clone(), size_key, c);
                    let cached = st.borrow().metrics.get(&key).copied();
                    let (width, advance) = match cached {
                        Some(v) => v,
                        None => {
                            let v = if is_zerowidth(c) {
                                (0., 0.)
                            } else {
                                let m = fonts.metrics(alias, *size as f32, c).map_err(|e| py::error(vm, e.to_string()))?;
                                (m.advance as f64, m.advance as f64)
                            };
                            st.borrow_mut().metrics.insert(key, v);
                            v
                        }
                    };
                    out.push((c, Metrics { ascent: ascent.round(), line_spacing: line_height.round(), width, advance }));
                }
            }
            Font::Image { data, scale } => {
                let chars: Vec<char> = s.chars().collect();
                for (i, &c) in chars.iter().enumerate() {
                    let mut m = Metrics { ascent: (data.baseline * scale).round(), line_spacing: (data.height * scale).round(), width: 0., advance: 0. };
                    if !is_zerowidth(c) {
                        let cell = data.cells.get(&c).ok_or_else(|| py::error(vm, format!("Character {c:?} not found in image-based font.")))?;
                        m.width = (cell.width * scale).round();
                        m.advance = (cell.advance * scale).round();
                    }
                    if let Some(n) = chars.get(i + 1) {
                        m.advance += data.kerns.get(&(c, *n)).copied().unwrap_or(data.default_kern) * scale;
                    }
                    out.push((c, m));
                }
            }
        }
        Ok(out)
    }
}

/// Image fonts registered as `image_fonts[name, size, bold, italics]`.
fn image_font(vm: &mut Vm, name: &str, size: f64, bold: bool, italic: bool) -> PyResult<Option<Font>> {
    let table = py::import_attr(vm, MODULE, "image_fonts")?;
    let n = vm.types.new_str(name);
    let sz = if size.fract() == 0. { Value::Int(size as i64) } else { Value::Float(size) };
    let key = vm.types.new_tuple(vec![n.clone(), sz, Value::Bool(bold), Value::Bool(italic)]);
    let mut f = vm.call_method(&table, "get", vec![key, Value::None])?;
    let mut scale = 1.;
    if py::is_none(&f) {
        // Scaled image fonts: the registered size of the same face.
        let mut best: Option<(f64, Value)> = None;
        for (k, v) in ops::mapping_items(vm, &table)? {
            let k = py::items(vm, &k)?;
            if k.len() == 4 && k[0].as_str() == Some(name) && ops::truthy(vm, &k[2])? == bold && ops::truthy(vm, &k[3])? == italic {
                let s = py::f64_of(vm, &k[1])?;
                best = Some((s, v));
            }
        }
        match best {
            Some((s, v)) if s > 0. => {
                scale = size / s;
                f = v;
            }
            _ => return Ok(None),
        }
    }
    let key = format!("{name}|{bold}|{italic}|{}", f.id());
    if let Some(d) = state().borrow().image_fonts.get(&key) {
        return Ok(Some(Font::Image { data: d.clone(), scale }));
    }
    let data = load_image_font(vm, &f)?;
    let data = Rc::new(data);
    state().borrow_mut().image_fonts.insert(key, data.clone());
    Ok(Some(Font::Image { data, scale }))
}

fn read_text(vm: &mut Vm, name: &str) -> PyResult<String> {
    let load = py::import_attr(vm, "renpy.loader", "load")?;
    let n = vm.types.new_str(name);
    let f = vm.call(&load, vec![n], vec![("directory".into(), vm.types.new_str("fonts"))])?;
    let b = vm.call_method(&f, "read", Vec::new())?;
    let _ = vm.call_method(&f, "close", Vec::new());
    let bytes = ops::bytes_of(&b).map(|b| b.to_vec()).unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn image_asset(vm: &mut Vm, name: &str) -> PyResult<(String, [f64; 2])> {
    let asset = super::asset_path(vm, name).ok_or_else(|| py::error(vm, format!("Couldn't find file '{name}'.")))?;
    let [w, h] = super::image::image_size(vm, name).map_err(|e| py::error(vm, e))?;
    Ok((asset, [w as f64, h as f64]))
}

fn parse_bmfont_line(l: &str) -> (String, HashMap<String, String>) {
    let mut words = Vec::new();
    let mut w = String::new();
    let mut quote = false;
    for c in l.chars() {
        match c {
            '\r' | '\n' => {}
            ' ' if !quote => {
                if !w.is_empty() {
                    words.push(std::mem::take(&mut w));
                }
            }
            '"' => quote = !quote,
            c => w.push(c),
        }
    }
    if !w.is_empty() {
        words.push(w);
    }
    let kind = words.first().cloned().unwrap_or_default();
    let map = words.iter().skip(1).filter_map(|i| i.split_once('=').map(|(a, b)| (a.to_owned(), b.to_owned()))).collect();
    (kind, map)
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=");
    let i = tag.find(&pat)? + pat.len();
    let rest = &tag[i..];
    let q = rest.chars().next()?;
    if q == '"' || q == '\'' {
        let end = rest[1..].find(q)? + 1;
        Some(rest[1..end].to_owned())
    } else {
        Some(rest.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next()?.to_owned())
    }
}

fn load_image_font(vm: &mut Vm, f: &Value) -> PyResult<ImageFontData> {
    let cls = f.obj().map(|o| o.class_name()).unwrap_or_default();
    let geti = |vm: &mut Vm, n: &str| -> PyResult<f64> { let v = py::get_or_none(vm, f, n)?; if py::is_none(&v) { Ok(0.) } else { py::f64_of(vm, &v) } };
    let mut d = ImageFontData::default();
    match cls.as_str() {
        "BMFont" => {
            let filename = { let v = py::get(vm, f, "filename")?; py::string(vm, &v)? };
            let text = read_text(vm, &filename)?;
            let mut pages: HashMap<i64, (String, [f64; 2])> = HashMap::new();
            for line in text.lines() {
                let (kind, a) = parse_bmfont_line(line);
                let num = |k: &str| a.get(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.);
                match kind.as_str() {
                    "common" => {
                        d.height = num("lineHeight");
                        d.baseline = num("base");
                    }
                    "page" => {
                        let file = a.get("file").cloned().unwrap_or_default();
                        let dir = filename.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
                        let img = image_asset(vm, &format!("{dir}{file}")).or_else(|_| image_asset(vm, &file))?;
                        pages.insert(num("id") as i64, img);
                    }
                    "char" => {
                        let c = char::from_u32(num("id") as u32).unwrap_or('\u{fffd}');
                        let (asset, page) = pages.get(&(num("page") as i64)).cloned().ok_or_else(|| py::error(vm, "BMFont char refers to a missing page"))?;
                        let (xo, w) = (num("xoffset"), num("width"));
                        d.cells.insert(c, Cell { asset, rect: [num("x"), num("y"), w, num("height")], page, offset: [xo, num("yoffset")], width: w + xo, advance: num("xadvance") });
                    }
                    "kerning" => {
                        let a1 = char::from_u32(num("first") as u32).unwrap_or(' ');
                        let a2 = char::from_u32(num("second") as u32).unwrap_or(' ');
                        d.kerns.insert((a1, a2), num("amount"));
                    }
                    _ => {}
                }
            }
            if let Some(sp) = d.cells.get(&' ').cloned() {
                d.cells.entry('\u{a0}').or_insert(sp);
            }
        }
        "MudgeFont" => {
            let filename = { let v = py::get(vm, f, "filename")?; py::string(vm, &v)? };
            let xml = { let v = py::get(vm, f, "xml")?; py::string(vm, &v)? };
            let (asset, page) = image_asset(vm, &filename)?;
            let text = read_text(vm, &xml)?;
            let mut height = 0f64;
            for tag in text.split('<').filter(|t| t.starts_with("char ") || t.starts_with("char\t")) {
                let num = |n: &str| attr_value(tag, n).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.);
                let id = num("id");
                if id < 0. {
                    continue;
                }
                let c = char::from_u32(id as u32).unwrap_or('\u{fffd}');
                let (w, h) = (num("width"), num("height"));
                d.cells.insert(c, Cell { asset: asset.clone(), rect: [num("x"), num("y"), w, h], page, offset: [0., 0.], width: w, advance: w });
                height = height.max(h);
            }
            d.height = height;
            d.baseline = height;
            let space = geti(vm, "spacewidth")?;
            d.cells.entry(' ').or_insert(Cell { asset: String::new(), rect: [0.; 4], page, offset: [0., 0.], width: space, advance: space });
            let sp = d.cells[&' '].clone();
            d.cells.entry('\u{a0}').or_insert(sp);
            d.default_kern = geti(vm, "default_kern")?;
        }
        "SFont" => {
            // The glyph boundaries are marked by a key colour in the first
            // row of the image; that needs decoded pixels from the image
            // service, which the layout path does not wait for.
            return Err(py::error(vm, "SFont glyph boundaries require the image's decoded first row; SFont image fonts are not available in this runtime."));
        }
        other => return Err(py::error(vm, format!("Unknown image font class {other}."))),
    }
    d.cells.insert('\u{200b}', Cell { asset: String::new(), rect: [0.; 4], page: [1., 1.], offset: [0., 0.], width: 0., advance: 0. });
    let kerns = py::get_or_none(vm, f, "kerns")?;
    if !py::is_none(&kerns) {
        for (k, v) in ops::mapping_items(vm, &kerns)? {
            let k = py::string(vm, &k)?;
            let mut it = k.chars();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                d.kerns.insert((a, b), py::f64_of(vm, &v)?);
            }
        }
    }
    Ok(d)
}

/// Releases cached faces (`free_memory`).
pub fn free_memory() {
    let st = state();
    let mut s = st.borrow_mut();
    s.metrics.clear();
    s.image_fonts.clear();
}

// Python surface -------------------------------------------------------------------------

fn image_font_init(vm: &mut Vm, args: &[Value], kw: Kw, names: &[&str], defaults: &[Value]) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in names.iter().enumerate() {
        let v = match arg(args, kw, i + 1, n) { Some(v) => v.clone(), None => defaults.get(i).cloned().unwrap_or(Value::None) };
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn sfont_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(vm, args, kw, &["filename", "spacewidth", "default_kern", "kerns", "charset", "baseline"], &[])
}

fn mudge_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(vm, args, kw, &["filename", "xml", "spacewidth", "default_kern", "kerns"], &[])
}

fn bmfont_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(vm, args, kw, &["filename"], &[])
}

fn reg_args(vm: &mut Vm, args: &[Value], kw: Kw, names: &[&str]) -> Vec<Value> {
    let _ = vm;
    names.iter().enumerate().map(|(i, n)| arg(args, kw, i, n).cloned().unwrap_or(Value::None)).collect()
}

fn store_image_font(vm: &mut Vm, name: &Value, size: &Value, bold: &Value, italics: &Value, font: Value) -> PyResult<()> {
    let table = py::import_attr(vm, MODULE, "image_fonts")?;
    let b = Value::Bool(ops::truthy(vm, bold)?);
    let i = Value::Bool(ops::truthy(vm, italics)?);
    let key = vm.types.new_tuple(vec![name.clone(), size.clone(), b, i]);
    ops::setitem(vm, &table, key, font)
}

fn f_register_sfont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(vm, args, kw, &["name", "size", "bold", "italics", "underline", "filename", "spacewidth", "baseline", "default_kern", "kerns", "charset"]);
    if py::is_none(&a[0]) || py::is_none(&a[1]) || py::is_none(&a[5]) {
        return Err(py::error(vm, "When registering an SFont, the font name, font size, and filename are required."));
    }
    let c = py::import_attr(vm, MODULE, "SFont")?;
    let spacewidth = if py::is_none(&a[6]) { Value::Int(10) } else { a[6].clone() };
    let dk = if py::is_none(&a[8]) { Value::Int(0) } else { a[8].clone() };
    let kerns = if py::is_none(&a[9]) { vm.types.new_dict() } else { a[9].clone() };
    let charset = if py::is_none(&a[10]) { vm.types.new_str("!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~") } else { a[10].clone() };
    let f = vm.call(&c, vec![a[5].clone(), spacewidth, dk, kerns, charset, a[7].clone()], Vec::new())?;
    store_image_font(vm, &a[0], &a[1], &a[2], &a[3], f)?;
    Ok(Value::None)
}

fn f_register_mudgefont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(vm, args, kw, &["name", "size", "bold", "italics", "underline", "filename", "xml", "spacewidth", "default_kern", "kerns"]);
    if py::is_none(&a[0]) || py::is_none(&a[1]) || py::is_none(&a[5]) || py::is_none(&a[6]) {
        return Err(py::error(vm, "When registering a Mudge Font, the font name, font size, filename, and xml filename are required."));
    }
    let c = py::import_attr(vm, MODULE, "MudgeFont")?;
    let spacewidth = if py::is_none(&a[7]) { Value::Int(10) } else { a[7].clone() };
    let dk = if py::is_none(&a[8]) { Value::Int(0) } else { a[8].clone() };
    let kerns = if py::is_none(&a[9]) { vm.types.new_dict() } else { a[9].clone() };
    let f = vm.call(&c, vec![a[5].clone(), a[6].clone(), spacewidth, dk, kerns], Vec::new())?;
    store_image_font(vm, &a[0], &a[1], &a[2], &a[3], f)?;
    Ok(Value::None)
}

fn f_register_bmfont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(vm, args, kw, &["name", "size", "bold", "italics", "underline", "filename"]);
    let c = py::import_attr(vm, MODULE, "BMFont")?;
    let f = vm.call(&c, vec![a[5].clone()], Vec::new())?;
    store_image_font(vm, &a[0], &a[1], &a[2], &a[3], f)?;
    Ok(Value::None)
}

// FontGroup --------------------------------------------------------------------------------

fn fg_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "map", vm.types.new_dict())?;
    py::set(vm, &args[0], "char_map", vm.types.new_dict())?;
    Ok(Value::None)
}

fn ord_of(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        _ => {
            let s = py::string(vm, v)?;
            s.chars().next().map(|c| c as i64).ok_or_else(|| vm.value_error("empty character"))
        }
    }
}

fn fg_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let font = arg_or(args, kw, 1, "font", Value::None);
    let start = arg_or(args, kw, 2, "start", Value::None);
    let end = arg_or(args, kw, 3, "end", Value::None);
    let target = arg_or(args, kw, 4, "target", Value::None);
    let inc = match arg(args, kw, 5, "target_increment") { Some(v) => ops::truthy(vm, v)?, None => false };
    let alias_map = py::config(vm, "font_name_map")?;
    if !py::is_none(&alias_map) && font.as_str().is_some() && ops::contains(vm, &alias_map, &font)? {
        return Err(py::error(vm, "FontGroup do not accept font aliases."));
    }
    let map = py::get(vm, this, "map")?;
    if py::is_none(&start) {
        if font.obj().is_some_and(|o| o.class_name() == "FontGroup") {
            let other = py::get(vm, &font, "map")?;
            for (k, v) in ops::mapping_items(vm, &other)? {
                if !ops::contains(vm, &map, &k)? {
                    ops::setitem(vm, &map, k, v)?;
                }
            }
        } else if !ops::contains(vm, &map, &Value::None)? {
            ops::setitem(vm, &map, Value::None, font)?;
        }
        return Ok(this.clone());
    }
    let s = ord_of(vm, &start)?;
    let e = ord_of(vm, &end)?;
    let mut t = if py::is_none(&target) || matches!(target, Value::Bool(false)) { None } else { Some(ord_of(vm, &target)?) };
    if e < s {
        return Err(py::error(vm, "In FontGroup.add, the start of a character range must be before the end of the range."));
    }
    let cmap = py::get(vm, this, "char_map")?;
    for i in s..=e {
        if !ops::contains(vm, &map, &Value::Int(i))? {
            ops::setitem(vm, &map, Value::Int(i), font.clone())?;
            if let Some(tv) = t {
                ops::setitem(vm, &cmap, Value::Int(i), Value::Int(tv))?;
                if inc {
                    t = Some(tv + 1);
                }
            }
        }
    }
    Ok(this.clone())
}

fn fg_remap(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let cha = arg_or(args, kw, 1, "cha", Value::None);
    let target = arg_or(args, kw, 2, "target", Value::None);
    let target = ord_of(vm, &target)?;
    let chars: Vec<Value> = match &cha {
        Value::Int(_) => vec![cha.clone()],
        v if v.as_str().is_some_and(|s| s.chars().count() == 1) => vec![cha.clone()],
        v => py::items(vm, v)?,
    };
    let map = py::get(vm, this, "map")?;
    let cmap = py::get(vm, this, "char_map")?;
    for c in chars {
        let i = ord_of(vm, &c)?;
        if !(ops::contains(vm, &map, &Value::None)? || ops::contains(vm, &map, &Value::Int(i))?) {
            return Err(py::error(vm, format!("Character U+{i:04x} has no font in this FontGroup")));
        }
        if !ops::contains(vm, &cmap, &Value::Int(i))? {
            ops::setitem(vm, &cmap, Value::Int(i), Value::Int(target))?;
        }
    }
    Ok(this.clone())
}

/// `FontGroup.segment(s)` as (font, substring) pairs.
pub fn fontgroup_segment(vm: &mut Vm, group: &Value, s: &str) -> PyResult<Vec<(Value, String)>> {
    let map = py::get(vm, group, "map")?;
    let cmap = py::get(vm, group, "char_map")?;
    let mut text: Vec<char> = s.chars().collect();
    if ops::truthy(vm, &cmap)? {
        for c in text.iter_mut() {
            let r = vm.call_method(&cmap, "get", vec![Value::Int(*c as i64), Value::None])?;
            if let Value::Int(t) = r {
                *c = char::from_u32(t as u32).unwrap_or(*c);
            }
        }
    }
    let mut out: Vec<(Value, String)> = Vec::new();
    let mut font: Option<Value> = None;
    for c in text {
        if c == ' ' && font.is_some() {
            out.last_mut().unwrap().1.push(c);
            continue;
        }
        let mut f = vm.call_method(&map, "get", vec![Value::Int(c as i64), Value::None])?;
        if py::is_none(&f) {
            f = vm.call_method(&map, "get", vec![Value::None, Value::None])?;
            if py::is_none(&f) {
                return Err(py::error(vm, format!("Character U+{:04x} not found in FontGroup", c as u32)));
            }
        }
        let same = match &font { Some(o) => ops::eq(vm, o, &f)?, None => false };
        if same {
            out.last_mut().unwrap().1.push(c);
        } else {
            out.push((f.clone(), c.to_string()));
            font = Some(f);
        }
    }
    Ok(out)
}

fn fg_segment(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::string(vm, &args.get(1).cloned().unwrap_or(Value::None))?;
    let segs = fontgroup_segment(vm, &args[0], &s)?;
    let items = segs.into_iter().map(|(f, t)| { let t = vm.types.new_string(t); vm.types.new_tuple(vec![f, t]) }).collect();
    Ok(vm.types.new_list(items))
}

fn f_get_font(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::string(vm, &arg_or(args, kw, 0, "fn", Value::None))?;
    let size = py::f64_of(vm, &arg_or(args, kw, 1, "size", Value::Int(22)))?;
    let bold = ops::truthy(vm, &arg_or(args, kw, 2, "bold", Value::Bool(false)))?;
    let italic = ops::truthy(vm, &arg_or(args, kw, 3, "italics", Value::Bool(false)))?;
    match get_font(vm, &name, size, bold, italic)? {
        Lookup::Ready(Font::Outline { alias, .. }) => Ok(vm.types.new_string(alias)),
        Lookup::Ready(Font::Image { .. }) => Ok(vm.types.new_str("image")),
        Lookup::Pending => Ok(Value::None),
    }
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("register_sfont", f_register_sfont),
        ("register_mudgefont", f_register_mudgefont),
        ("register_bmfont", f_register_bmfont),
        ("get_font", f_get_font),
        ("free_memory", |_, _, _| {
            free_memory();
            Ok(Value::None)
        }),
        ("load_fonts", |_, _, _| Ok(Value::None)),
        ("is_zerowidth", |vm, a, _| {
            let c = ord_of(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
            Ok(Value::Bool(char::from_u32(c as u32).is_some_and(is_zerowidth)))
        }),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for n in ["image_fonts", "scaled_image_fonts", "font_cache", "face_cache"] {
        set(vm, n, vm.types.new_dict());
    }
    let c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods, attrs: &[(&str, Value)]| -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    let image_font = c(vm, "ImageFont", &[], &[], &[("height", Value::Int(0)), ("default_kern", Value::Float(0.)), ("baseline", Value::Int(0))])?;
    c(vm, "SFont", &[image_font.clone()], &[("__init__", sfont_init)], &[])?;
    c(vm, "MudgeFont", &[image_font.clone()], &[("__init__", mudge_init)], &[])?;
    c(vm, "BMFont", &[image_font.clone()], &[("__init__", bmfont_init)], &[])?;
    c(vm, "ScaledImageFont", &[image_font], &[], &[])?;
    c(vm, "FontGroup", &[], &[("__init__", fg_init), ("add", fg_add), ("remap", fg_remap), ("segment", fg_segment)], &[("char_map", vm.types.new_dict())])?;
    set(vm, "last_scale", Value::Float(1.));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
