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
    Ready {
        alias: String,
        ascent: f64,
        line_height: f64,
        descent: f64,
    },
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
    kerns: HashMap<(String, u32, char, char), f64>,
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
    Outline {
        alias: String,
        size: f64,
        bold: bool,
        ascent: f64,
        line_height: f64,
        descent: f64,
    },
    Image {
        data: Rc<ImageFontData>,
        scale: f64,
    },
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

pub fn is_zerowidth(c: char) -> bool {
    matches!(c as u32, 0x200b | 0x200c | 0x200d | 0x2060 | 0xfeff)
        || (0xfe00..=0xfe0f).contains(&(c as u32))
}

/// Resolves a Ren'Py font name (file name or image-font name) at a size.
pub fn get_font(vm: &mut Vm, name: &str, size: f64, bold: bool, italic: bool) -> PyResult<Lookup> {
    // config.font_replacement_map[(fn, bold, italics)]
    let (mut fname, mut bold, mut italic) = (name.to_owned(), bold, italic);
    if let Ok(map) = py::config(vm, "font_replacement_map") {
        if !py::is_none(&map) {
            let n = vm.types.new_str(&fname);
            let key = vm
                .types
                .new_tuple(vec![n, Value::Bool(bold), Value::Bool(italic)]);
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
    // `index@file` selects a face of a collection (upstream `load_face`).
    let (face_index, file) = split_face_index(&fname)?
        .ok_or_else(|| vm.value_error(format!("invalid literal for int() in font name {fname:?}")))?;
    let st = state();
    let key = format!("{file}@{face_index}");
    let status = {
        let s = st.borrow();
        match s.faces.get(&key) {
            Some(Face::Ready {
                alias,
                ascent,
                line_height,
                descent,
            }) => Some(Ok((alias.clone(), *ascent, *line_height, *descent))),
            Some(Face::Failed(e)) => Some(Err(e.clone())),
            Some(Face::Loading(_)) => None,
            None => None,
        }
    };
    match status {
        Some(Ok((alias, ascent, line_height, descent))) => {
            return Ok(Lookup::Ready(Font::Outline {
                alias,
                size,
                bold,
                ascent: ascent * size,
                line_height: line_height * size,
                descent: descent * size,
            }));
        }
        Some(Err(e)) => return Err(py::error(vm, e)),
        None => {}
    }
    let fonts = super::host(vm)
        .borrow()
        .fonts
        .clone()
        .ok_or_else(|| py::error(vm, "font service unavailable"))?;
    let loading = matches!(st.borrow().faces.get(&key), Some(Face::Loading(_)));
    if !loading {
        let path = font_path(vm, &file)
            .ok_or_else(|| py::error(vm, format!("Couldn't find file '{file}'.")))?;
        let spec = FontLoadSpec {
            name: format!("renpy:{key}"),
            source: FontLoadSource::Storage(path),
            face_index,
        };
        let job = fonts
            .begin(spec)
            .map_err(|e| py::error(vm, format!("{file}: {e}")))?;
        st.borrow_mut()
            .faces
            .insert(key.clone(), Face::Loading(job));
    }
    let job = match st.borrow().faces.get(&key) {
        Some(Face::Loading(j)) => *j,
        _ => return Ok(Lookup::Pending),
    };
    match fonts
        .status(job)
        .map_err(|e| py::error(vm, e.to_string()))?
    {
        FontLoadState::Pending => Ok(Lookup::Pending),
        FontLoadState::Failed(e) => {
            st.borrow_mut()
                .faces
                .insert(key, Face::Failed(format!("{file}: {e}")));
            Err(py::error(vm, format!("{file}: {e}")))
        }
        FontLoadState::Ready(info) => {
            let em = fonts.outline_em_metrics(&info.alias, 1.).ok();
            let m = fonts.metrics(&info.alias, 100., 'x').ok();
            let ascent = em
                .map(|e| e.ascent as f64)
                .or(m.map(|m| m.ascent as f64 / 100.))
                .unwrap_or(0.8);
            let descent = m.map(|m| (m.descent as f64 / 100.).abs()).unwrap_or(0.2);
            let line_height = em.map(|e| e.line_height as f64).unwrap_or(ascent + descent);
            st.borrow_mut().faces.insert(
                key,
                Face::Ready {
                    alias: info.alias.clone(),
                    ascent,
                    line_height,
                    descent,
                },
            );
            Ok(Lookup::Ready(Font::Outline {
                alias: info.alias,
                size,
                bold,
                ascent: ascent * size,
                line_height: line_height * size,
                descent: descent * size,
            }))
        }
    }
}

/// `"index@file"` -> (index, file); a name without `@` is face 0.
pub fn split_face_index(name: &str) -> PyResult<Option<(u32, String)>> {
    Ok(match name.split_once('@') {
        Some((i, f)) => i.trim().parse::<u32>().ok().map(|i| (i, f.to_owned())),
        None => Some((0, name.to_owned())),
    })
}

fn font_path(vm: &mut Vm, file: &str) -> Option<crysirror_core::path::VfsPath> {
    for candidate in [file.to_owned(), format!("fonts/{file}")] {
        let candidate = super::find_file(vm, &candidate);
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
            Font::Outline {
                alias,
                size,
                ascent,
                line_height,
                ..
            } => {
                let fonts = super::host(vm)
                    .borrow()
                    .fonts
                    .clone()
                    .ok_or_else(|| py::error(vm, "font service unavailable"))?;
                let st = state();
                let size_key = (size * 64.).round() as u32;
                let chars: Vec<char> = s.chars().collect();
                // FreeType kerning (`ftfont.glyphs`): the rounded pair kern is
                // added to the advance, never below the previous glyph's
                // minimum so a negative kern cannot pull glyphs backwards.
                let mut kerns = Vec::with_capacity(chars.len());
                for i in 0..chars.len() {
                    let k = match chars.get(i + 1) {
                        Some(&n) if !is_zerowidth(chars[i]) && !is_zerowidth(n) => {
                            let key = (alias.clone(), size_key, chars[i], n);
                            let cached = st.borrow().kerns.get(&key).copied();
                            match cached {
                                Some(k) => k,
                                None => {
                                    let k = fonts
                                        .kerning(alias, *size as f32, chars[i], n)
                                        .map(|k| k.round() as f64)
                                        .unwrap_or(0.);
                                    st.borrow_mut().kerns.insert(key, k);
                                    k
                                }
                            }
                        }
                        _ => 0.,
                    };
                    kerns.push(k);
                }
                let mut next_min_advance = 0f64;
                for (i, &c) in chars.iter().enumerate() {
                    let key = (alias.clone(), size_key, c);
                    let cached = st.borrow().metrics.get(&key).copied();
                    let (width, advance) = match cached {
                        Some(v) => v,
                        None => {
                            let v = if is_zerowidth(c) {
                                (0., 0.)
                            } else {
                                let m = fonts
                                    .metrics(alias, *size as f32, c)
                                    .map_err(|e| py::error(vm, e.to_string()))?;
                                (m.advance as f64, m.advance as f64)
                            };
                            st.borrow_mut().metrics.insert(key, v);
                            v
                        }
                    };
                    let min_advance = next_min_advance;
                    let advance = if i + 1 < chars.len() {
                        let a = if advance + kerns[i] > min_advance {
                            advance + kerns[i]
                        } else {
                            min_advance
                        };
                        next_min_advance = advance - a;
                        a
                    } else {
                        advance
                    };
                    out.push((
                        c,
                        Metrics {
                            ascent: ascent.round(),
                            line_spacing: line_height.round(),
                            width,
                            advance,
                        },
                    ));
                }
            }
            Font::Image { data, scale } => {
                let chars: Vec<char> = s.chars().collect();
                for (i, &c) in chars.iter().enumerate() {
                    let mut m = Metrics {
                        ascent: (data.baseline * scale).round(),
                        line_spacing: (data.height * scale).round(),
                        width: 0.,
                        advance: 0.,
                    };
                    if !is_zerowidth(c) {
                        let cell = data.cells.get(&c).ok_or_else(|| {
                            py::error(
                                vm,
                                format!("Character {c:?} not found in image-based font."),
                            )
                        })?;
                        m.width = (cell.width * scale).round();
                        m.advance = (cell.advance * scale).round();
                    }
                    if let Some(n) = chars.get(i + 1) {
                        m.advance += data
                            .kerns
                            .get(&(c, *n))
                            .copied()
                            .unwrap_or(data.default_kern)
                            * scale;
                    }
                    out.push((c, m));
                }
            }
        }
        Ok(out)
    }
}

/// Image fonts registered as `image_fonts[name, size, bold, italics]`.
fn image_font(
    vm: &mut Vm,
    name: &str,
    size: f64,
    bold: bool,
    italic: bool,
) -> PyResult<Option<Font>> {
    let table = py::import_attr(vm, MODULE, "image_fonts")?;
    let n = vm.types.new_str(name);
    let sz = if size.fract() == 0. {
        Value::Int(size as i64)
    } else {
        Value::Float(size)
    };
    let key = vm
        .types
        .new_tuple(vec![n.clone(), sz, Value::Bool(bold), Value::Bool(italic)]);
    let mut f = vm.call_method(&table, "get", vec![key, Value::None])?;
    let mut scale = 1.;
    if py::is_none(&f) {
        // Scaled image fonts: the registered size of the same face.
        let mut best: Option<(f64, Value)> = None;
        for (k, v) in ops::mapping_items(vm, &table)? {
            let k = py::items(vm, &k)?;
            if k.len() == 4
                && k[0].as_str() == Some(name)
                && ops::truthy(vm, &k[2])? == bold
                && ops::truthy(vm, &k[3])? == italic
            {
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
        return Ok(Some(Font::Image {
            data: d.clone(),
            scale,
        }));
    }
    let data = load_image_font(vm, &f)?;
    let data = Rc::new(data);
    state().borrow_mut().image_fonts.insert(key, data.clone());
    Ok(Some(Font::Image { data, scale }))
}

fn read_text(vm: &mut Vm, name: &str) -> PyResult<String> {
    let load = py::import_attr(vm, "renpy.loader", "load")?;
    let n = vm.types.new_str(name);
    let f = vm.call(
        &load,
        vec![n],
        vec![("directory".into(), vm.types.new_str("fonts"))],
    )?;
    let b = vm.call_method(&f, "read", Vec::new())?;
    let _ = vm.call_method(&f, "close", Vec::new());
    let bytes = ops::bytes_of(&b).map(|b| b.to_vec()).unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn image_asset(vm: &mut Vm, name: &str) -> PyResult<(String, [f64; 2])> {
    let name = &super::find_file(vm, name);
    let asset = super::asset_path(vm, name)
        .ok_or_else(|| py::error(vm, format!("Couldn't find file '{name}'.")))?;
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
    let map = words
        .iter()
        .skip(1)
        .filter_map(|i| i.split_once('=').map(|(a, b)| (a.to_owned(), b.to_owned())))
        .collect();
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
        Some(
            rest.split(|c: char| c.is_whitespace() || c == '/' || c == '>')
                .next()?
                .to_owned(),
        )
    }
}

fn load_image_font(vm: &mut Vm, f: &Value) -> PyResult<ImageFontData> {
    let cls = f.obj().map(|o| o.class_name()).unwrap_or_default();
    let geti = |vm: &mut Vm, n: &str| -> PyResult<f64> {
        let v = py::get_or_none(vm, f, n)?;
        if py::is_none(&v) {
            Ok(0.)
        } else {
            py::f64_of(vm, &v)
        }
    };
    let mut d = ImageFontData::default();
    match cls.as_str() {
        "BMFont" => {
            let filename = {
                let v = py::get(vm, f, "filename")?;
                py::string(vm, &v)?
            };
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
                        let dir = filename
                            .rsplit_once('/')
                            .map(|(d, _)| format!("{d}/"))
                            .unwrap_or_default();
                        let img = image_asset(vm, &format!("{dir}{file}"))
                            .or_else(|_| image_asset(vm, &file))?;
                        pages.insert(num("id") as i64, img);
                    }
                    "char" => {
                        let c = char::from_u32(num("id") as u32).unwrap_or('\u{fffd}');
                        let (asset, page) = pages
                            .get(&(num("page") as i64))
                            .cloned()
                            .ok_or_else(|| py::error(vm, "BMFont char refers to a missing page"))?;
                        let (xo, w) = (num("xoffset"), num("width"));
                        d.cells.insert(
                            c,
                            Cell {
                                asset,
                                rect: [num("x"), num("y"), w, num("height")],
                                page,
                                offset: [xo, num("yoffset")],
                                width: w + xo,
                                advance: num("xadvance"),
                            },
                        );
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
            let filename = {
                let v = py::get(vm, f, "filename")?;
                py::string(vm, &v)?
            };
            let xml = {
                let v = py::get(vm, f, "xml")?;
                py::string(vm, &v)?
            };
            let (asset, page) = image_asset(vm, &filename)?;
            let text = read_text(vm, &xml)?;
            let mut height = 0f64;
            for tag in text
                .split('<')
                .filter(|t| t.starts_with("char ") || t.starts_with("char\t"))
            {
                let num = |n: &str| {
                    attr_value(tag, n)
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.)
                };
                let id = num("id");
                if id < 0. {
                    continue;
                }
                let c = char::from_u32(id as u32).unwrap_or('\u{fffd}');
                let (w, h) = (num("width"), num("height"));
                d.cells.insert(
                    c,
                    Cell {
                        asset: asset.clone(),
                        rect: [num("x"), num("y"), w, h],
                        page,
                        offset: [0., 0.],
                        width: w,
                        advance: w,
                    },
                );
                height = height.max(h);
            }
            d.height = height;
            d.baseline = height;
            let space = geti(vm, "spacewidth")?;
            d.cells.entry(' ').or_insert(Cell {
                asset: String::new(),
                rect: [0.; 4],
                page,
                offset: [0., 0.],
                width: space,
                advance: space,
            });
            let sp = d.cells[&' '].clone();
            d.cells.entry('\u{a0}').or_insert(sp);
            d.default_kern = geti(vm, "default_kern")?;
        }
        "SFont" => {
            // The glyph boundaries are marked by a key colour in the first
            // row of the image; that needs decoded pixels from the image
            // service, which the layout path does not wait for.
            return Err(py::error(
                vm,
                "SFont glyph boundaries require the image's decoded first row; SFont image fonts are not available in this runtime.",
            ));
        }
        other => return Err(py::error(vm, format!("Unknown image font class {other}."))),
    }
    d.cells.insert(
        '\u{200b}',
        Cell {
            asset: String::new(),
            rect: [0.; 4],
            page: [1., 1.],
            offset: [0., 0.],
            width: 0.,
            advance: 0.,
        },
    );
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
    s.kerns.clear();
    s.image_fonts.clear();
}

// Python surface -------------------------------------------------------------------------

fn image_font_init(
    vm: &mut Vm,
    args: &[Value],
    kw: Kw,
    names: &[&str],
    defaults: &[Value],
) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in names.iter().enumerate() {
        let v = match arg(args, kw, i + 1, n) {
            Some(v) => v.clone(),
            None => defaults.get(i).cloned().unwrap_or(Value::None),
        };
        py::set(vm, this, n, v)?;
    }
    Ok(Value::None)
}

fn sfont_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(
        vm,
        args,
        kw,
        &[
            "filename",
            "spacewidth",
            "default_kern",
            "kerns",
            "charset",
            "baseline",
        ],
        &[],
    )
}

fn mudge_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(
        vm,
        args,
        kw,
        &["filename", "xml", "spacewidth", "default_kern", "kerns"],
        &[],
    )
}

fn bmfont_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    image_font_init(vm, args, kw, &["filename"], &[])
}

fn reg_args(vm: &mut Vm, args: &[Value], kw: Kw, names: &[&str]) -> Vec<Value> {
    let _ = vm;
    names
        .iter()
        .enumerate()
        .map(|(i, n)| arg(args, kw, i, n).cloned().unwrap_or(Value::None))
        .collect()
}

fn store_image_font(
    vm: &mut Vm,
    name: &Value,
    size: &Value,
    bold: &Value,
    italics: &Value,
    font: Value,
) -> PyResult<()> {
    let table = py::import_attr(vm, MODULE, "image_fonts")?;
    let b = Value::Bool(ops::truthy(vm, bold)?);
    let i = Value::Bool(ops::truthy(vm, italics)?);
    let key = vm.types.new_tuple(vec![name.clone(), size.clone(), b, i]);
    let m = py::import(vm, MODULE)?;
    if let Some(names) = py::get_opt(vm, &m, "image_font_names")? {
        vm.call_method(&names, "add", vec![name.clone()])?;
    }
    ops::setitem(vm, &table, key, font)
}

fn f_register_sfont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(
        vm,
        args,
        kw,
        &[
            "name",
            "size",
            "bold",
            "italics",
            "underline",
            "filename",
            "spacewidth",
            "baseline",
            "default_kern",
            "kerns",
            "charset",
        ],
    );
    if py::is_none(&a[0]) || py::is_none(&a[1]) || py::is_none(&a[5]) {
        return Err(py::error(
            vm,
            "When registering an SFont, the font name, font size, and filename are required.",
        ));
    }
    let c = py::import_attr(vm, MODULE, "SFont")?;
    let spacewidth = if py::is_none(&a[6]) {
        Value::Int(10)
    } else {
        a[6].clone()
    };
    let dk = if py::is_none(&a[8]) {
        Value::Int(0)
    } else {
        a[8].clone()
    };
    let kerns = if py::is_none(&a[9]) {
        vm.types.new_dict()
    } else {
        a[9].clone()
    };
    let charset = if py::is_none(&a[10]) {
        vm.types.new_str("!\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~")
    } else {
        a[10].clone()
    };
    let f = vm.call(
        &c,
        vec![a[5].clone(), spacewidth, dk, kerns, charset, a[7].clone()],
        Vec::new(),
    )?;
    store_image_font(vm, &a[0], &a[1], &a[2], &a[3], f)?;
    Ok(Value::None)
}

fn f_register_mudgefont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(
        vm,
        args,
        kw,
        &[
            "name",
            "size",
            "bold",
            "italics",
            "underline",
            "filename",
            "xml",
            "spacewidth",
            "default_kern",
            "kerns",
        ],
    );
    if py::is_none(&a[0]) || py::is_none(&a[1]) || py::is_none(&a[5]) || py::is_none(&a[6]) {
        return Err(py::error(
            vm,
            "When registering a Mudge Font, the font name, font size, filename, and xml filename are required.",
        ));
    }
    let c = py::import_attr(vm, MODULE, "MudgeFont")?;
    let spacewidth = if py::is_none(&a[7]) {
        Value::Int(10)
    } else {
        a[7].clone()
    };
    let dk = if py::is_none(&a[8]) {
        Value::Int(0)
    } else {
        a[8].clone()
    };
    let kerns = if py::is_none(&a[9]) {
        vm.types.new_dict()
    } else {
        a[9].clone()
    };
    let f = vm.call(
        &c,
        vec![a[5].clone(), a[6].clone(), spacewidth, dk, kerns],
        Vec::new(),
    )?;
    store_image_font(vm, &a[0], &a[1], &a[2], &a[3], f)?;
    Ok(Value::None)
}

fn f_register_bmfont(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = reg_args(
        vm,
        args,
        kw,
        &["name", "size", "bold", "italics", "underline", "filename"],
    );
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
            s.chars()
                .next()
                .map(|c| c as i64)
                .ok_or_else(|| vm.value_error("empty character"))
        }
    }
}

fn fg_add(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let font = arg_or(args, kw, 1, "font", Value::None);
    let start = arg_or(args, kw, 2, "start", Value::None);
    let end = arg_or(args, kw, 3, "end", Value::None);
    let target = arg_or(args, kw, 4, "target", Value::None);
    let inc = match arg(args, kw, 5, "target_increment") {
        Some(v) => ops::truthy(vm, v)?,
        None => false,
    };
    let alias_map = py::config(vm, "font_name_map")?;
    if !py::is_none(&alias_map) && font.as_str().is_some() && ops::contains(vm, &alias_map, &font)?
    {
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
    let mut t = if py::is_none(&target) || matches!(target, Value::Bool(false)) {
        None
    } else {
        Some(ord_of(vm, &target)?)
    };
    if e < s {
        return Err(py::error(
            vm,
            "In FontGroup.add, the start of a character range must be before the end of the range.",
        ));
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
            return Err(py::error(
                vm,
                format!("Character U+{i:04x} has no font in this FontGroup"),
            ));
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
                return Err(py::error(
                    vm,
                    format!("Character U+{:04x} not found in FontGroup", c as u32),
                ));
            }
        }
        let same = match &font {
            Some(o) => ops::eq(vm, o, &f)?,
            None => false,
        };
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
    let items = segs
        .into_iter()
        .map(|(f, t)| {
            let t = vm.types.new_string(t);
            vm.types.new_tuple(vec![f, t])
        })
        .collect();
    Ok(vm.types.new_list(items))
}

fn rel(vm: &Vm) -> super::textsupport::Rel {
    super::textsupport::Rel::of(vm)
}

/// `load_face(fn[, shaper])`: the cached FTFace/HBFace of a font file.
fn f_load_face(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = rel(vm);
    let fname = arg_or(args, kw, 0, "fn", Value::None);
    let shaper = if r.since_pair(7, 2) {
        arg_or(args, kw, 1, "shaper", Value::None)
    } else {
        Value::None
    };
    let cache = py::import_attr(vm, MODULE, "face_cache")?;
    let key = if r.since_pair(7, 2) {
        py::tuple(vm, vec![fname.clone(), shaper.clone()])
    } else {
        fname.clone()
    };
    if ops::contains(vm, &cache, &key)? {
        return ops::getitem(vm, &cache, key);
    }
    let name = py::string(vm, &fname)?;
    let (index, file) = split_face_index(&name)?
        .ok_or_else(|| vm.value_error(format!("invalid literal for int() with base 10: {name:?}")))?;
    let load = py::import_attr(vm, "renpy.loader", "load")?;
    let fv = vm.types.new_string(file.clone());
    let opened = if r.at_least(7, 0) {
        let d = vm.types.new_str("fonts");
        vm.call(&load, vec![fv], vec![("directory".into(), d)])
    } else {
        vm.call(&load, vec![fv], Vec::new())
    };
    let f = match opened {
        Ok(f) => f,
        Err(_) => return Err(py::error(vm, format!("Could not find font {name:?}."))),
    };
    let hb = shaper.as_str() == Some("harfbuzz");
    let cls = if hb {
        py::import_attr(vm, "renpy.text.hbfont", "HBFace")?
    } else {
        py::import_attr(vm, "renpy.text.ftfont", "FTFace")?
    };
    let _ = vm.call_method(&f, "close", Vec::new());
    let rv = vm.call(&cls, vec![f, Value::Int(index as i64), fname], Vec::new())?;
    ops::setitem(vm, &cache, key, rv.clone())?;
    Ok(rv)
}

/// `get_font(fn, size, bold, italics, outline, antialias, vertical,
/// hinting, scale, shaper, instance, axis, features)` with each release's
/// parameters: an image font, or an FTFont/HBFont of the loaded face.
fn f_get_font(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let r = rel(vm);
    let mut names = vec!["fn", "size", "bold", "italics", "outline", "antialias", "vertical"];
    if r.at_least(6, 99) {
        names.extend(["hinting", "scale"]);
    }
    if r.since_pair(7, 2) {
        names.extend(["shaper", "instance", "axis"]);
    }
    if r.at_least(8, 4) {
        names.push("features");
    }
    let mut a: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| arg_or(args, kw, i, n, Value::None))
        .collect();
    a.resize(13, Value::None);
    let scale = if r.at_least(6, 99) && !py::is_none(&a[8]) {
        py::f64_of(vm, &a[8])?
    } else {
        1.
    };
    if r.at_least(6, 99) {
        let last = py::import_attr(vm, MODULE, "last_scale")?;
        let last = py::f64_of(vm, &last)?;
        if scale != 1. && scale != last {
            for n in ["scaled_image_fonts", "font_cache"] {
                let d = py::import_attr(vm, MODULE, n)?;
                vm.call_method(&d, "clear", Vec::new())?;
            }
            let m = py::import(vm, MODULE)?;
            py::set(vm, &m, "last_scale", Value::Float(scale))?;
        }
    }
    let t = py::tuple(vm, vec![a[0].clone(), a[2].clone(), a[3].clone()]);
    let map = py::config(vm, "font_replacement_map")?;
    let t = vm.call_method(&map, "get", vec![t.clone(), t])?;
    let t = py::items(vm, &t)?;
    let (fname, bold, italics) = (t[0].clone(), t[1].clone(), t[2].clone());
    let key = py::tuple(vm, vec![fname.clone(), a[1].clone(), bold.clone(), italics.clone()]);
    let image_fonts = py::import_attr(vm, MODULE, "image_fonts")?;
    let rv = vm.call_method(&image_fonts, "get", vec![key.clone(), Value::None])?;
    if !py::is_none(&rv) {
        if scale != 1. {
            let scaled = py::import_attr(vm, MODULE, "scaled_image_fonts")?;
            if ops::contains(vm, &scaled, &key)? {
                return ops::getitem(vm, &scaled, key);
            }
            let c = py::import_attr(vm, MODULE, "ScaledImageFont")?;
            let s = vm.call(&c, vec![rv, Value::Float(scale)], Vec::new())?;
            ops::setitem(vm, &scaled, key, s.clone())?;
            return Ok(s);
        }
        return Ok(rv);
    }
    let mut size = a[1].clone();
    if r.at_least(8, 6) {
        let adj = py::config(vm, "font_size_adjust")?;
        let f = vm.call_method(&adj, "get", vec![fname.clone(), Value::None])?;
        if !py::is_none(&f) {
            size = if crysirror_python::builtins::is_callable(vm, &f) {
                vm.call(&f, vec![fname.clone(), size], Vec::new())?
            } else {
                ops::binary_op(vm, &size, &f, crysirror_python::ast::BinOp::Mult, false, false)?
            };
        }
    }
    let features = if py::is_none(&a[12]) {
        Value::None
    } else {
        let items = vm.call_method(&a[12], "items", Vec::new())?;
        let sorted = py::import_attr(vm, "builtins", "sorted")?;
        let l = vm.call(&sorted, vec![items], Vec::new())?;
        let tc = Value::Obj(vm.types.tuple.clone());
        vm.call(&tc, vec![l], Vec::new())?
    };
    let axis_key = if py::is_none(&a[11]) {
        Value::None
    } else {
        let items = vm.call_method(&a[11], "items", Vec::new())?;
        let sorted = py::import_attr(vm, "builtins", "sorted")?;
        let l = vm.call(&sorted, vec![items], Vec::new())?;
        let tc = Value::Obj(vm.types.tuple.clone());
        vm.call(&tc, vec![l], Vec::new())?
    };
    let key = py::tuple(
        vm,
        vec![
            fname.clone(), size.clone(), bold.clone(), italics.clone(), a[4].clone(), a[5].clone(),
            a[6].clone(), a[7].clone(), Value::Float(scale), a[9].clone(), a[10].clone(), axis_key,
            features.clone(),
        ],
    );
    let font_cache = py::import_attr(vm, MODULE, "font_cache")?;
    let rv = vm.call_method(&font_cache, "get", vec![key.clone(), Value::None])?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let shaper = a[9].clone();
    let face = {
        let lf = py::import_attr(vm, MODULE, "load_face")?;
        let mut la = vec![fname.clone()];
        if r.since_pair(7, 2) {
            la.push(shaper.clone());
        }
        match vm.call(&lf, la, Vec::new()) {
            Ok(f) => f,
            Err(e) => {
                let names = py::import_attr(vm, MODULE, "image_font_names").ok();
                if let Some(n) = names.filter(|_| r.at_least(8, 6)) {
                    if ops::contains(vm, &n, &fname)? {
                        let s = ops::repr(vm, &fname)?;
                        let sz = ops::str(vm, &size)?;
                        let b = ops::str(vm, &bold)?;
                        let it = ops::str(vm, &italics)?;
                        return Err(py::error(vm, format!("Font {s} is an image font, but (size={sz}, bold={b}, italics={it}) was not registered.")));
                    }
                }
                return Err(e);
            }
        }
    };
    let sz = py::f64_of(vm, &size)?;
    let isz = Value::Int((sz * scale) as i64);
    let hb = shaper.as_str() == Some("harfbuzz");
    let mut fa = vec![face, if r.at_least(6, 99) { isz } else { size.clone() }, bold, italics, a[4].clone(), a[5].clone(), a[6].clone()];
    if r.at_least(6, 99) {
        fa.push(a[7].clone());
    }
    let cls = if hb {
        fa.extend([a[10].clone(), a[11].clone()]);
        if r.at_least(8, 4) {
            fa.push(a[12].clone());
        }
        py::import_attr(vm, "renpy.text.hbfont", "HBFont")?
    } else {
        py::import_attr(vm, "renpy.text.ftfont", "FTFont")?
    };
    let rv = vm.call(&cls, fa, Vec::new())?;
    ops::setitem(vm, &font_cache, key, rv.clone())?;
    Ok(rv)
}

/// `parse_bmfont_line(l)` -> (kind, {key: value}).
fn f_parse_bmfont_line(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let l = py::string(vm, &arg_or(args, kw, 0, "l", Value::None))?;
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
    if words.is_empty() {
        words.push(String::new());
    }
    let d = vm.types.new_dict();
    for i in &words[1..] {
        let Some((k, v)) = i.split_once('=') else {
            return Err(vm.value_error("dictionary update sequence element has length 1; 2 is required"));
        };
        let k = vm.types.new_string(k.to_owned());
        let v = vm.types.new_string(v.to_owned());
        ops::setitem(vm, &d, k, v)?;
    }
    let kind = vm.types.new_string(words[0].clone());
    Ok(py::tuple(vm, vec![kind, d]))
}

/// `ImageFont.load()`: fills the per-character tables from the font file.
fn imagefont_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let d = load_image_font(vm, this)?;
    let num = |v: f64| if v.fract() == 0. { Value::Int(v as i64) } else { Value::Float(v) };
    py::set(vm, this, "height", num(d.height))?;
    py::set(vm, this, "baseline", num(d.baseline))?;
    let (width, advance, offsets, kerns) =
        (vm.types.new_dict(), vm.types.new_dict(), vm.types.new_dict(), vm.types.new_dict());
    let mut cells: Vec<_> = d.cells.iter().collect();
    cells.sort_by_key(|(c, _)| **c);
    for (c, cell) in cells {
        let k = vm.types.new_string(c.to_string());
        ops::setitem(vm, &width, k.clone(), num(cell.width))?;
        ops::setitem(vm, &advance, k.clone(), num(cell.advance))?;
        let o = py::tuple(vm, vec![num(cell.offset[0]), num(cell.offset[1])]);
        ops::setitem(vm, &offsets, k, o)?;
    }
    for ((a, b), v) in &d.kerns {
        let k = vm.types.new_string(format!("{a}{b}"));
        ops::setitem(vm, &kerns, k, num(*v))?;
    }
    py::set(vm, this, "width", width)?;
    py::set(vm, this, "advance", advance)?;
    py::set(vm, this, "offsets", offsets)?;
    py::set(vm, this, "kerns", kerns)?;
    if py::get_opt(vm, this, "chars")?.is_none() {
        let c = vm.types.new_dict();
        py::set(vm, this, "chars", c)?;
    }
    Ok(Value::None)
}

/// `ImageFont.glyphs(s[, level])` from the per-character tables.
fn imagefont_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let s = py::string(vm, &arg_or(&args[1..], kw, 0, "s", Value::None))?;
    let r = rel(vm);
    let cls = py::import_attr(vm, "renpy.text.textsupport", "Glyph")?;
    let baseline = py::get(vm, this, "baseline")?;
    let height = py::get(vm, this, "height")?;
    let width = py::get(vm, this, "width")?;
    let advance = py::get(vm, this, "advance")?;
    let kerns = py::get(vm, this, "kerns")?;
    let dk = py::get(vm, this, "default_kern")?;
    let chars: Vec<char> = s.chars().collect();
    let mut rv = Vec::new();
    for &c in &chars {
        let g = vm.call(&cls, Vec::new(), Vec::new())?;
        py::set(vm, &g, "character", Value::Int(c as i64))?;
        let asc = Value::Int(py::f64_of(vm, &baseline)? as i64);
        py::set(vm, &g, "ascent", asc)?;
        let ls = Value::Int(py::f64_of(vm, &height)? as i64);
        py::set(vm, &g, "line_spacing", ls)?;
        let zw = r.at_least(7, 3) && is_zerowidth(c);
        if !zw {
            let k = vm.types.new_string(c.to_string());
            let w = vm.call_method(&width, "get", vec![k.clone(), Value::None])?;
            if py::is_none(&w) {
                let rc = ops::repr(vm, &k)?;
                return Err(py::error(vm, format!("Character {rc} not found in image-based font.")));
            }
            let a = ops::getitem(vm, &advance, k)?;
            let (w, a) = (py::f64_of(vm, &w)?, py::f64_of(vm, &a)?);
            py::set(vm, &g, "width", Value::Float(w))?;
            py::set(vm, &g, "advance", Value::Float(a))?;
        }
        rv.push(g);
    }
    for i in 0..chars.len().saturating_sub(1) {
        let k = vm.types.new_string(format!("{}{}", chars[i], chars[i + 1]));
        let kern = vm.call_method(&kerns, "get", vec![k, dk.clone()])?;
        let kern = py::f64_of(vm, &kern)?;
        let a = py::get(vm, &rv[i], "advance")?;
        let a = py::f64_of(vm, &a)?;
        py::set(vm, &rv[i], "advance", Value::Float((a + kern) as f32 as f64))?;
    }
    Ok(py::list(vm, rv))
}

fn imagefont_bounds(_: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    Ok(arg_or(&args[1..], kw, 1, "bounds", Value::None))
}

fn imagefont_draw(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(
        vm,
        "ImageFont.draw: CrySirror renders image-font glyphs as retained image cells; drawing into a pixel surface is not available.",
    ))
}

/// `ScaledImageFont(parent, factor)`: the parent's tables scaled and
/// rounded.
fn scaled_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let parent = arg_or(&args[1..], kw, 0, "parent", Value::None);
    let factor = py::f64_of(vm, &arg_or(&args[1..], kw, 1, "factor", Value::Float(1.)))?;
    if py::get_opt(vm, &parent, "width")?.is_none_or(|w| !ops::truthy(vm, &w).unwrap_or(false)) {
        vm.call_method(&parent, "load", Vec::new())?;
    }
    let sc = |vm: &mut Vm, v: &Value| -> PyResult<Value> {
        Ok(Value::Int((py::f64_of(vm, v)? * factor).round_ties_even() as i64))
    };
    for n in ["height", "baseline", "default_kern"] {
        let v = py::get(vm, &parent, n)?;
        let v = sc(vm, &v)?;
        py::set(vm, this, n, v)?;
    }
    for n in ["width", "advance", "kerns"] {
        let src = py::get(vm, &parent, n)?;
        let d = vm.types.new_dict();
        for (k, v) in ops::mapping_items(vm, &src)? {
            let v = sc(vm, &v)?;
            ops::setitem(vm, &d, k, v)?;
        }
        py::set(vm, this, n, d)?;
    }
    let src = py::get(vm, &parent, "offsets")?;
    let d = vm.types.new_dict();
    for (k, v) in ops::mapping_items(vm, &src)? {
        let p = py::items(vm, &v)?;
        let (a, b) = (sc(vm, &p[0])?, sc(vm, &p[1])?);
        let t = py::tuple(vm, vec![a, b]);
        ops::setitem(vm, &d, k, t)?;
    }
    py::set(vm, this, "offsets", d)?;
    py::set(vm, this, "chars", vm.types.new_dict())?;
    py::set(vm, this, "parent", parent)?;
    py::set(vm, this, "factor", Value::Float(factor))?;
    Ok(Value::None)
}

fn f_load_image_fonts(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let fonts = py::import_attr(vm, MODULE, "image_fonts")?;
    let vals = vm.call_method(&fonts, "values", Vec::new())?;
    for f in py::items(vm, &vals)? {
        // SFont glyph boundaries need decoded image rows, which the image
        // service does not hand to the script thread; such fonts fail when
        // text first uses them rather than at startup.
        if f.obj().is_some_and(|o| o.class_name() == "SFont") {
            continue;
        }
        vm.call_method(&f, "load", Vec::new())?;
    }
    Ok(Value::None)
}

/// `load_fonts()`: loads the image fonts and preloads `config.preload_fonts`.
fn f_load_fonts(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    f_load_image_fonts(vm, args, kw)?;
    let r = rel(vm);
    let pre = py::config(vm, "preload_fonts").unwrap_or(Value::None);
    if py::is_none(&pre) {
        return Ok(Value::None);
    }
    let lf = py::import_attr(vm, MODULE, "load_face")?;
    for f in py::items(vm, &pre)? {
        let mut a = vec![f.clone()];
        if r.since_pair(7, 2) {
            a.push(vm.types.new_str("harfbuzz"));
        }
        vm.call(&lf, a, Vec::new())?;
        // Start the face's load in the font service.
        let name = py::string(vm, &f)?;
        let _ = get_font(vm, &name, 16., false, false)?;
    }
    Ok(Value::None)
}

/// `variable_font_info(font)`: the face's Variations, None when static.
fn f_variable_font_info(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let f = arg_or(args, kw, 0, "font", Value::None);
    let lf = py::import_attr(vm, MODULE, "load_face")?;
    let hb = vm.types.new_str("harfbuzz");
    let face = vm.call(&lf, vec![f, hb], Vec::new())?;
    py::get(vm, &face, "variations")
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MODULE,
        &[
            ("register_sfont", f_register_sfont),
            ("register_mudgefont", f_register_mudgefont),
            ("register_bmfont", f_register_bmfont),
            ("get_font", f_get_font),
            ("load_face", f_load_face),
            ("parse_bmfont_line", f_parse_bmfont_line),
            ("free_memory", |vm, _, _| {
                free_memory();
                // scaled_image_fonts is a 6.99+ name.
                let names: &[&str] = if rel(vm).at_least(6, 99) {
                    &["scaled_image_fonts", "font_cache", "face_cache"]
                } else {
                    &["font_cache", "face_cache"]
                };
                for n in names {
                    let d = py::import_attr(vm, MODULE, n)?;
                    vm.call_method(&d, "clear", Vec::new())?;
                }
                Ok(Value::None)
            }),
            ("is_zerowidth", |vm, a, _| {
                let c = ord_of(vm, &a.first().cloned().unwrap_or(Value::Int(0)))?;
                Ok(Value::Bool(
                    char::from_u32(c as u32).is_some_and(is_zerowidth),
                ))
            }),
        ],
    );
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let r = rel(vm);
    let mut extra: Vec<(&'static str, crysirror_python::object::NativeFn)> = Vec::new();
    // load_fonts is 7.0+; the interface calls it at startup in every
    // release, so it stays available (it loads what 6.x load_image_fonts
    // loads, plus config.preload_fonts).
    extra.push(("load_fonts", f_load_fonts));
    if !r.at_least(7, 0) {
        extra.push(("load_image_fonts", f_load_image_fonts));
    }
    if r.since_pair(7, 2) {
        extra.push(("variable_font_info", f_variable_font_info));
    }
    for (n, f) in extra {
        let b = vm.types.new_builtin(n, f);
        set(vm, n, b);
    }
    if !r.at_least(7, 3) {
        // is_zerowidth is a 7.3+ name.
        let n = vm.types.new_str("is_zerowidth");
        ops::delattr(vm, &Value::Obj(m.clone()), &n)?;
    }
    for (n, v) in [("WHITE", 255), ("BLACK", 0)] {
        let c = Value::Int(v);
        let t = py::tuple(vm, vec![c.clone(), c.clone(), c, Value::Int(255)]);
        set(vm, n, t);
    }
    if r.at_least(8, 6) {
        let s = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
        set(vm, "image_font_names", s);
    }
    for n in [
        "image_fonts",
        "scaled_image_fonts",
        "font_cache",
        "face_cache",
    ] {
        set(vm, n, vm.types.new_dict());
    }
    let c = |vm: &mut Vm,
             name: &str,
             bases: &[Value],
             methods: Methods,
             attrs: &[(&str, Value)]|
     -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    let image_font = c(
        vm,
        "ImageFont",
        &[],
        &[
            ("load", imagefont_load),
            ("glyphs", imagefont_glyphs),
            ("bounds", imagefont_bounds),
            ("draw", imagefont_draw),
        ],
        &[
            ("height", Value::Int(0)),
            ("default_kern", Value::Float(0.)),
            ("baseline", Value::Int(0)),
        ],
    )?;
    c(
        vm,
        "SFont",
        &[image_font.clone()],
        &[("__init__", sfont_init)],
        &[],
    )?;
    c(
        vm,
        "MudgeFont",
        &[image_font.clone()],
        &[("__init__", mudge_init)],
        &[],
    )?;
    c(
        vm,
        "BMFont",
        &[image_font.clone()],
        &[("__init__", bmfont_init)],
        &[],
    )?;
    if r.at_least(6, 99) {
        c(vm, "ScaledImageFont", &[image_font], &[("__init__", scaled_init)], &[])?;
    }
    c(
        vm,
        "FontGroup",
        &[],
        &[
            ("__init__", fg_init),
            ("add", fg_add),
            ("remap", fg_remap),
            ("segment", fg_segment),
        ],
        &[("char_map", vm.types.new_dict())],
    )?;
    if r.at_least(6, 99) {
        set(vm, "last_scale", Value::Float(1.));
    } else {
        let n = vm.types.new_str("scaled_image_fonts");
        ops::delattr(vm, &Value::Obj(m.clone()), &n)?;
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
    super::fontobj::install(vm);
}
