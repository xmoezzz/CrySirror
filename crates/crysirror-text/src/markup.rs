// Portions adapted from asukaminato0721/krkrz_rs
// b4a295fabbedf9ff5579faf304b0342d7a0699a6 (crates/krkrz_runtime/src/text_render.rs),
// MIT License, Copyright (c) 2026 krkrz_rs contributors; see
// docs/third-party/krkrz_rs.txt. Modifications CrySirror contributors (MPL-2.0).
//! Inline-markup text layout (the KiriKiri TextRender profile), without a VM.
//!
//! Text is UTF-16 with `%`-format commands, `#rrggbb;` colours, `$expr;`
//! evaluation, `[ruby,n]` annotations and `\` escapes. Layout places characters
//! line by line with alignment, auto-indent, kinsoku (`following`/`leading`)
//! and per-character reveal delays. Measuring, font notifications, expression
//! evaluation and labels are delegated to [`MarkupHost`]; nothing is drawn here.
//! Unimplemented profiles (vertical ruby, graphical characters, word break,
//! width-time scaling, uneven ruby) return errors instead of approximations.
use anyhow::{bail, ensure, Context, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Real,
    Bool,
    Color,
    Int,
}
/// Field name suffix, markup/tag attribute (empty: none), kind, default.
pub const FIELDS: &[(&str, &str, Kind, f64)] = &[
    ("Align", "align", Kind::Int, -1.0),
    ("BigFontSize", "bigfontsize", Kind::Real, 48.0),
    ("Bold", "bold", Kind::Bool, 0.0),
    ("ChColor", "color", Kind::Color, 4294967295.0),
    ("Edge", "edge", Kind::Bool, 0.0),
    ("EdgeColor", "edgecolor", Kind::Color, 4278223103.0),
    ("Face", "face", Kind::Text, 0.0),
    ("FontSize", "fontsize", Kind::Real, 24.0),
    ("Italic", "", Kind::Bool, 0.0),
    ("LineSize", "linesize", Kind::Real, 24.0),
    ("LineSpacing", "linespacing", Kind::Real, 6.0),
    ("Pitch", "pitch", Kind::Real, 0.0),
    ("RubyOffset", "rubyoffset", Kind::Real, -2.0),
    ("RubySize", "rubysize", Kind::Real, 10.0),
    ("Shadow", "shadow", Kind::Bool, 1.0),
    ("ShadowColor", "shadowcolor", Kind::Color, 4278190080.0),
    ("ShadowDiff", "shadowdiff", Kind::Int, 1.0),
    ("SmallFontSize", "smallfontsize", Kind::Real, 12.0),
    ("Valign", "valign", Kind::Int, -1.0),
];
/// Options read by `setOption`; `vertical` is stored separately.
pub const OPTIONS: &[&str] = &[
    "following",
    "leading",
    "begin",
    "end",
    "kinsoku_max",
    "vertical",
    "word_break",
    "width_time_scale",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Real(f64),
    Text(Vec<u16>),
}
impl Value {
    fn number(&self) -> f32 {
        match self {
            Self::Int(v) => *v as f32,
            Self::Real(v) => *v as f32,
            Self::Text(_) => 0.0,
        }
    }
}
fn field_index(name: &str) -> Option<usize> {
    FIELDS.iter().position(|f| f.0 == name)
}
fn ix(name: &str) -> usize {
    field_index(name).expect("known TextRender field")
}

/// One complete style state, indexed like [`FIELDS`].
#[derive(Clone, Debug, PartialEq)]
pub struct Format(pub Vec<Value>);
impl Default for Format {
    fn default() -> Self {
        Self(
            FIELDS
                .iter()
                .map(|&(_, _, kind, n)| match kind {
                    Kind::Text => Value::Text("normal".encode_utf16().collect()),
                    Kind::Real => Value::Real(n),
                    _ => Value::Int(n as i64),
                })
                .collect(),
        )
    }
}
impl Format {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(field_index(name)?)
    }
    pub fn number(&self, name: &str) -> f32 {
        self.0[ix(name)].number()
    }
    fn set(&mut self, name: &str, v: Value) {
        self.0[ix(name)] = v;
    }
    fn font(&self) -> (Value, Value, Value) {
        (
            self.0[ix("Bold")].clone(),
            self.0[ix("Italic")].clone(),
            self.0[ix("Face")].clone(),
        )
    }
}
fn style_field(key: &str) -> bool {
    matches!(key, "Align" | "Valign" | "Pitch" | "LineSize" | "LineSpacing")
}
/// Validate and normalise a value for field `name` (`kind` conversion).
pub fn convert(kind: Kind, value: Value) -> Result<Value> {
    Ok(match (kind, value) {
        (Kind::Text, Value::Text(t)) => Value::Text(t),
        (Kind::Text, v) => Value::Text(format!("{}", v.number()).encode_utf16().collect()),
        (Kind::Real, v) => Value::Real(f64::from(finite(v.number())?)),
        (Kind::Bool, v) => Value::Int((int(&v)? != 0) as i64),
        (Kind::Color, v) => Value::Int(int(&v)? as u32 as i64),
        (Kind::Int, v) => Value::Int(int(&v)? as i32 as i64),
    })
}
fn int(v: &Value) -> Result<i64> {
    Ok(match v {
        Value::Int(i) => *i,
        Value::Real(r) => *r as i64,
        Value::Text(_) => bail!("TextRender numeric field requires a number"),
    })
}
pub fn finite(n: f32) -> Result<f32> {
    ensure!(n.is_finite(), "TextRender numeric value must be finite");
    ensure!(n.abs() <= 1e9, "TextRender numeric range limit exceeded");
    Ok(n)
}

/// Callbacks the native frontend supplies (script handlers in KiriKiri).
pub trait MarkupHost {
    /// Advance width of `text` at pixel `size`.
    fn text_width(&mut self, text: &[u16], size: f32) -> Result<f32>;
    /// Notified whenever bold/italic/face may have changed.
    fn font_changed(&mut self, bold: bool, italic: bool, face: &[u16]) -> Result<()>;
    /// `$expr;` substitution text.
    fn eval(&mut self, expression: &[u16]) -> Result<Vec<u16>>;
    /// `%D$label;` delay in milliseconds.
    fn label(&mut self, name: &[u16]) -> Result<f32>;
}

#[derive(Clone, Debug)]
pub struct Ruby {
    pub text: Vec<u16>,
    pub size: f32,
    pub width: f32,
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Debug)]
pub struct Character {
    pub text: Vec<u16>,
    pub format: Format,
    pub size: f32,
    pub width: f32,
    pub delay: f32,
    pub x: f32,
    pub y: f32,
    pub ruby: Option<Ruby>,
}
#[derive(Clone, Debug)]
struct Line {
    characters: Vec<Character>,
    height: f32,
    spacing: f32,
    offset: f32,
}

const MAX_UNITS: usize = 4 << 20;
const MAX_CHARACTERS: usize = 100_000;

#[derive(Clone, Debug)]
pub struct MarkupLayout {
    pub defaults: Format,
    pub current: Format,
    options: Vec<(String, Value)>,
    pub width: f32,
    pub height: f32,
    pub font_scale: f32,
    pub time_scale: f32,
    pub vertical: bool,
    lines: Vec<Line>,
    pending: Vec<Character>,
    published: Vec<Character>,
    pub render_text: Vec<u16>,
    pub count: usize,
    pub delay: f32,
    pub over: bool,
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub initialized: bool,
    key_waits: Vec<usize>,
    line_indent: f32,
    next_indent: f32,
    indent_stack: Vec<f32>,
    hanging_count: usize,
    replay_extent: f32,
}
impl Default for MarkupLayout {
    fn default() -> Self {
        Self {
            defaults: Format::default(),
            current: Format::default(),
            options: Vec::new(),
            width: 0.0,
            height: 0.0,
            font_scale: 1.0,
            time_scale: 1.0,
            vertical: false,
            lines: vec![],
            pending: vec![],
            published: vec![],
            render_text: vec![],
            count: 0,
            delay: 0.0,
            over: false,
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 0.0,
            initialized: false,
            key_waits: vec![],
            line_indent: 0.0,
            next_indent: 0.0,
            indent_stack: vec![],
            hanging_count: 0,
            replay_extent: 0.0,
        }
    }
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl MarkupLayout {
    fn notify(&self, host: &mut dyn MarkupHost) -> Result<()> {
        let (b, i, f) = self.current.font();
        let face = match f {
            Value::Text(t) => t,
            _ => vec![],
        };
        host.font_changed(b.number() != 0.0, i.number() != 0.0, &face)
    }
    fn reset_layout(&mut self) {
        self.current = self.defaults.clone();
        self.lines.clear();
        self.pending.clear();
        self.key_waits.clear();
        self.line_indent = 0.0;
        self.next_indent = 0.0;
        self.indent_stack.clear();
        self.hanging_count = 0;
        self.replay_extent = 0.0;
        self.published.clear();
        self.render_text.clear();
        self.count = 0;
        self.delay = 0.0;
        self.over = false;
        self.left = if self.vertical { self.width } else { 0.0 };
        self.right = self.left;
        self.top = 0.0;
        self.bottom = 0.0;
        self.initialized = true;
    }
    /// `clear()`; notifies only when the font signature changed.
    pub fn clear(&mut self, host: &mut dyn MarkupHost) -> Result<()> {
        let before = self.current.font();
        self.reset_layout();
        if before != self.current.font() {
            self.notify(host)?;
        }
        Ok(())
    }
    pub fn set_render_size(&mut self, w: f32, h: f32, host: &mut dyn MarkupHost) -> Result<()> {
        self.width = finite(w)?;
        self.height = finite(h)?;
        self.clear(host)
    }
    /// `resetFont` (style=false) / `resetStyle` (style=true).
    pub fn reset(&mut self, style: bool, host: &mut dyn MarkupHost) -> Result<()> {
        let before = self.current.font();
        self.reset_fields(style);
        if before != self.current.font() {
            self.notify(host)?;
        }
        Ok(())
    }
    fn reset_fields(&mut self, style: bool) {
        for (i, &(key, ..)) in FIELDS.iter().enumerate() {
            if style_field(key) == style {
                self.current.0[i] = self.defaults.0[i].clone();
            }
        }
    }
    /// `setDefault`/`setFont`/`setStyle` from attribute lookups. `lookup`
    /// returns the attribute value, `None` when absent.
    pub fn apply(
        &mut self,
        op: Apply,
        mut lookup: impl FnMut(&str) -> Result<Option<Value>>,
        host: &mut dyn MarkupHost,
    ) -> Result<()> {
        let before = self.current.font();
        for (i, &(key, attribute, kind, _)) in FIELDS.iter().enumerate() {
            if attribute.is_empty()
                || op != Apply::Default && style_field(key) != (op == Apply::Style)
            {
                continue;
            }
            if let Some(v) = lookup(attribute)? {
                let v = convert(kind, v)?;
                if op == Apply::Default {
                    self.defaults.0[i] = v;
                } else {
                    self.current.0[i] = v;
                }
            }
        }
        if op == Apply::Font && before != self.current.font() {
            self.notify(host)?;
        }
        Ok(())
    }
    pub fn set_default(&mut self, suffix: &str, v: Value) -> Result<()> {
        let i = field_index(suffix).context("unknown TextRender field")?;
        self.defaults.0[i] = convert(FIELDS[i].2, v)?;
        Ok(())
    }
    pub fn set_option(&mut self, name: &str, v: Value) -> Result<()> {
        ensure!(OPTIONS.contains(&name), "unknown TextRender option");
        if name == "vertical" {
            self.vertical = int(&v)? != 0;
            return Ok(());
        }
        self.options.retain(|(n, _)| n != name);
        self.options.push((name.into(), v));
        Ok(())
    }
    fn option(&self, name: &str) -> Option<&Value> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    fn option_contains(&self, name: &str, unit: u16) -> bool {
        matches!(self.option(name), Some(Value::Text(s)) if s.contains(&unit))
    }
    fn offset(&self) -> f32 {
        self.lines
            .last()
            .map(|l| l.offset + l.height + l.spacing)
            .unwrap_or(0.0)
    }
    fn advance(&self, ch: &Character) -> f32 {
        (if self.vertical { ch.size } else { ch.width }) + ch.format.number("Pitch")
    }
    fn pending_width(&self) -> f32 {
        self.pending.iter().map(|c| self.advance(c)).sum()
    }
    /// Characters published by the last `done()`, in reveal order.
    pub fn characters(&self, start: usize, count: usize) -> &[Character] {
        let start = start.min(self.published.len());
        let end = if count == 0 {
            self.published.len()
        } else {
            start.saturating_add(count).min(self.published.len())
        };
        &self.published[start..end]
    }
    pub fn key_waits(&self) -> &[usize] {
        &self.key_waits
    }
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
    pub fn render_delay(&self) -> f32 {
        self.delay * self.time_scale
    }
    pub fn max_scroll_offset(&self) -> f32 {
        if self.vertical {
            self.width - self.left
        } else {
            self.height - self.bottom
        }
    }
    pub fn show_count(&self, time: f32) -> usize {
        self.published
            .iter()
            .take_while(|c| c.delay * self.time_scale <= time)
            .count()
    }
    pub fn line_offset(&self, line: i64) -> f32 {
        if line <= 0 {
            return 0.0;
        }
        if line as usize >= self.lines.len() {
            return self.bottom;
        }
        // The published block fits the render area: in-range lines need no scroll.
        0.0
    }

    fn finish_line(&mut self, host: &mut dyn MarkupHost, force_empty: bool) -> Result<()> {
        if (!force_empty && self.pending.is_empty()) || self.over {
            return Ok(());
        }
        let h = self
            .pending
            .iter()
            .map(|c| c.size)
            .fold(self.current.number("LineSize") * self.font_scale, f32::max);
        let y = self.offset();
        let limit = if self.vertical { self.width } else { self.height };
        if limit > 0.0 && y + h > limit {
            self.over = true;
            self.pending.clear();
            return Ok(());
        }
        let space_size = self.current.number("FontSize") * self.font_scale;
        let measured_space = host.text_width(&[0x3000], space_size)?;
        let space = if self.vertical { space_size } else { measured_space };
        let size = self.pending_width();
        let limit = if self.vertical { self.height } else { self.width };
        let align = self.current.number("Align");
        let mut x = if align == 0.0 {
            ((limit - size) / 2.0).trunc()
        } else if align == 1.0 {
            limit - size
        } else {
            self.line_indent
        };
        let spaces = if space > 0.0 { (x / space).max(0.0) as usize } else { 0 };
        ensure!(spaces <= 100_000, "TextRender indentation limit exceeded");
        self.render_text.extend(std::iter::repeat_n(0x3000, spaces));
        let mut characters = std::mem::take(&mut self.pending);
        for ch in &mut characters {
            if self.vertical {
                ch.x = self.width - y;
                ch.y = x;
            } else {
                ch.x = x;
                ch.y = y + h - ch.size;
            }
            if let Some(ruby) = &mut ch.ruby {
                ruby.x += ch.x;
                ruby.y += ch.y;
            }
            x += self.advance(ch);
            self.render_text.extend_from_slice(&ch.text);
        }
        self.render_text.push(10);
        self.hanging_count = 0;
        self.line_indent = self.next_indent;
        self.lines.push(Line {
            characters,
            height: h,
            spacing: self.current.number("LineSpacing"),
            offset: y,
        });
        Ok(())
    }

    pub fn newline(&mut self, host: &mut dyn MarkupHost) -> Result<()> {
        self.require()?;
        self.finish_line(host, false)
    }
    fn require(&self) -> Result<()> {
        ensure!(self.initialized, "TextRender layout requires clear or setRenderSize");
        Ok(())
    }

    /// Close the current line, compute bounds and vertical alignment, publish.
    pub fn done(&mut self, host: &mut dyn MarkupHost) -> Result<()> {
        self.require()?;
        self.finish_line(host, false)?;
        self.published = self.lines.iter().flat_map(|l| l.characters.clone()).collect();
        self.left = if self.vertical { self.width } else { 0.0 };
        self.right = self.left.max(self.replay_extent);
        self.top = 0.0;
        self.bottom = 0.0;
        for ch in &self.published {
            if self.vertical {
                self.left = self.left.min(ch.x - ch.size);
                self.bottom = self.bottom.max(ch.y + ch.size);
            } else {
                self.left = self.left.min(ch.x);
                self.right = self.right.max(ch.x + ch.width);
                self.bottom = self.bottom.max(ch.y + ch.size);
            }
            if let Some(ruby) = &ch.ruby {
                self.left = self.left.min(ruby.x);
                self.right = self.right.max(ruby.x + ruby.width);
                self.top = self.top.min(ruby.y);
            }
        }
        if !self.published.is_empty() {
            let valign = self.current.number("Valign");
            let available = if self.vertical {
                -(self.width - self.left)
            } else {
                self.height - self.bottom
            };
            let offset = if valign == 0.0 {
                (available / 2.0).trunc()
            } else if valign == 1.0 {
                available
            } else {
                0.0
            };
            let vertical = self.vertical;
            for ch in &mut self.published {
                if vertical {
                    ch.x += offset;
                } else {
                    ch.y += offset;
                }
                if let Some(ruby) = &mut ch.ruby {
                    if vertical {
                        ruby.x += offset;
                    } else {
                        ruby.y += offset;
                    }
                }
            }
            if vertical {
                self.left += offset;
                self.right += offset;
            } else {
                self.top += offset;
                self.bottom += offset;
            }
            // Stored coordinates change too, including on repeated done().
            let mut published = self.published.iter();
            for ch in self.lines.iter_mut().flat_map(|l| &mut l.characters) {
                *ch = published.next().expect("published line character").clone();
            }
        }
        self.published.sort_by(|a, b| a.delay.total_cmp(&b.delay));
        Ok(())
    }

    /// `render(text, autoIndent, delay, total, same)`; returns false once over.
    pub fn render(
        &mut self,
        text: &[u16],
        auto_indent: i64,
        base_delay: f32,
        total: f32,
        same: bool,
        host: &mut dyn MarkupHost,
    ) -> Result<bool> {
        self.require()?;
        ensure!(text.len() <= MAX_UNITS, "TextRender input limit exceeded");
        let mut units = text.to_vec();
        let base_delay = finite(base_delay)?;
        let total = finite(total)?;
        if !same {
            let n = self.published.len();
            for ch in self.lines.iter_mut().flat_map(|l| &mut l.characters).take(n) {
                ch.delay = 0.0;
            }
        }
        let kinsoku_max = self
            .option("kinsoku_max")
            .map(int)
            .transpose()?
            .unwrap_or(1)
            .max(0) as usize;
        for name in ["word_break", "width_time_scale"] {
            if self.option(name).is_some_and(|v| int(v).unwrap_or(1) != 0) {
                bail!("TextRender layout option {name} is not implemented");
            }
        }
        self.notify(host)?;
        let size = self.current.number("FontSize") * self.font_scale;
        host.text_width(&[0x3000], size)?;
        let start_count = self.count;
        let (mut i, mut delay, mut step) = (0usize, 0.0f32, base_delay);
        self.delay = 0.0;
        let mut ruby_pending: Option<(Vec<u16>, usize, usize)> = None;
        while i < units.len() && units[i] != 0 && !self.over {
            let mut unit = units[i];
            i += 1;
            let escaped = unit == u16::from(b'\\');
            if escaped {
                unit = *units.get(i).context("unterminated TextRender escape")?;
                i += 1;
                match unit {
                    110 /* n */ => {
                        self.finish_line(host, false)?;
                        continue;
                    }
                    107 /* k */ => {
                        self.key_waits.push(self.count);
                        continue;
                    }
                    120 /* x */ => continue,
                    119 /* w */ => {
                        let size = self.current.number("FontSize") * self.font_scale;
                        let width = host.text_width(&[0x3000], size)?;
                        if self.pending.is_empty() {
                            self.line_indent += width;
                        }
                        if let Some(last) = self.pending.last_mut() {
                            let p = last.format.number("Pitch");
                            last.format.set("Pitch", Value::Real(f64::from(p + width)));
                        }
                        continue;
                    }
                    116 /* t */ => unit = 9,
                    105 /* i */ => {
                        self.next_indent = self.line_indent + self.pending_width();
                        continue;
                    }
                    114 /* r */ => {
                        self.next_indent = 0.0;
                        continue;
                    }
                    _ => (),
                }
            }
            if !escaped && unit == u16::from(b'%') {
                let command = *units.get(i).context("unterminated TextRender format command")?;
                i += 1;
                match command {
                    98 | 105 | 115 | 101 /* b i s e */ => {
                        let flag = *units.get(i).context("missing TextRender format flag")?;
                        i += 1;
                        let key = match command {
                            98 => "Bold",
                            105 => "Italic",
                            115 => "Shadow",
                            _ => "Edge",
                        };
                        let value = match flag {
                            48 => Value::Int(0),
                            49 => Value::Int(1),
                            _ => self.defaults.0[ix(key)].clone(),
                        };
                        self.current.set(key, value);
                        if command == 98 || command == 105 {
                            self.notify(host)?;
                        }
                    }
                    // `%n[N];` line breaks: the game's TagTextConverter emits
                    // it for KAG `[xr num=N]`. Semantics follow the krkrsdl3
                    // drop-in textrender (plugins/textrender.cpp): flush, one
                    // break, then N more (N defaults to 1). The wamsoft plugin
                    // source was not available to inspect.
                    110 /* n */ => {
                        let token = terminated(&units, &mut i, 59)?;
                        let n = if token.is_empty() {
                            1
                        } else {
                            String::from_utf16(&token)?
                                .parse::<u32>()
                                .context("invalid TextRender %n count")?
                                .max(1)
                        };
                        ensure!(n <= 64, "TextRender %n count limit");
                        for _ in 0..=n {
                            self.finish_line(host, true)?;
                        }
                    }
                    66 | 83 /* B S */ => {
                        let v = self.defaults.0[ix(if command == 66 { "BigFontSize" } else { "SmallFontSize" })].clone();
                        self.current.set("FontSize", v);
                    }
                    114 /* r */ => {
                        self.reset_fields(false);
                        self.notify(host)?;
                    }
                    67 | 82 | 76 /* C R L */ => {
                        self.current.set(
                            "Align",
                            Value::Int(match command {
                                67 => 0,
                                82 => 1,
                                _ => -1,
                            }),
                        );
                    }
                    102 /* f */ => {
                        let face = terminated(&units, &mut i, 59)?;
                        self.current.set("Face", Value::Text(face));
                        self.notify(host)?;
                    }
                    112 | 100 | 119 | 68 | 48..=57 /* p d w D digits */ => {
                        if (48..=57).contains(&command) {
                            i -= 1;
                        }
                        if command == 68 && units.get(i) == Some(&36) {
                            i += 1;
                            let name = terminated(&units, &mut i, 59)?;
                            delay = delay.max(finite(host.label(&name)?)?);
                        } else {
                            let token = terminated(&units, &mut i, 59)?;
                            let value = String::from_utf16(&token)?
                                .parse::<f32>()
                                .context("invalid TextRender numeric command")?;
                            let value = finite(value)?;
                            match command {
                                112 => self.current.set("Pitch", Value::Real(f64::from(value))),
                                100 => step = base_delay * value / 100.0,
                                119 => delay += base_delay * value / 100.0,
                                68 => delay = delay.max(value),
                                _ => {
                                    let size = self.defaults.number("FontSize") * value / 100.0;
                                    self.current.set("FontSize", Value::Real(f64::from(size)));
                                }
                            }
                        }
                    }
                    _ => bail!(
                        "TextRender format command {} is not implemented",
                        char::from_u32(u32::from(command)).unwrap_or('?')
                    ),
                }
                continue;
            }
            if !escaped && unit == u16::from(b'#') {
                let token = terminated(&units, &mut i, 59)?;
                let color = u32::from_str_radix(&String::from_utf16(&token)?, 16)
                    .context("invalid TextRender color")?;
                self.current.set("ChColor", Value::Int(i64::from(color | 0xff00_0000)));
                continue;
            }
            if !escaped && unit == u16::from(b'$') {
                let expr = terminated(&units, &mut i, 59)?;
                let value = host.eval(&expr)?;
                let mut literal = vec![];
                for ch in value {
                    if matches!(ch, 37 | 35 | 91 | 36 | 38 | 92) {
                        literal.push(92);
                    }
                    literal.push(ch);
                }
                ensure!(
                    units.len().saturating_add(literal.len()) <= MAX_UNITS,
                    "TextRender expanded text limit exceeded"
                );
                units.splice(i..i, literal);
                continue;
            }
            if !escaped && unit == u16::from(b'[') {
                ensure!(!self.vertical, "TextRender vertical ruby is not implemented");
                let mut ruby = terminated(&units, &mut i, 93)?;
                let count = if let Some(comma) = ruby.iter().rposition(|u| *u == 44) {
                    let n = String::from_utf16(&ruby[comma + 1..])?
                        .parse::<usize>()
                        .context("invalid ruby character count")?;
                    ruby.truncate(comma);
                    n
                } else {
                    1
                };
                ensure!((1..=100_000).contains(&count), "ruby character count");
                ensure!(
                    count == 1 || ruby.len() % count == 0,
                    "uneven multi-character ruby spacing is not implemented"
                );
                ruby_pending = Some((ruby, count, 0));
                continue;
            }
            ensure!(
                escaped || unit != u16::from(b'&'),
                "TextRender graphical-character layout is not implemented"
            );
            if unit == 10 {
                self.finish_line(host, false)?;
                continue;
            }
            if unit == 13 {
                continue;
            }
            let mut text = vec![unit];
            if (0xd800..=0xdbff).contains(&unit)
                && units.get(i).is_some_and(|u| (0xdc00..=0xdfff).contains(u))
            {
                text.push(units[i]);
                i += 1;
            }
            let format = self.current.clone();
            let size = self.current.number("FontSize") * self.font_scale;
            let width = finite(host.text_width(&text, size)?)?;
            ensure!(width >= 0.0, "TextRender width must be nonnegative");
            let ruby = if let Some((rt, count, index)) = &mut ruby_pending {
                let chunk = rt[*index * rt.len() / *count..(*index + 1) * rt.len() / *count].to_vec();
                let rs = self.current.number("RubySize") * self.font_scale;
                let offset = self.current.number("RubyOffset") * self.font_scale;
                let rw = host.text_width(&chunk, rs)?;
                *index += 1;
                Some(Ruby {
                    text: chunk,
                    size: rs,
                    width: rw,
                    x: (width - rw) / 2.0,
                    y: -(rs + offset),
                })
            } else {
                None
            };
            if ruby_pending.as_ref().is_some_and(|(_, c, i)| c == i) {
                ruby_pending = None;
            }
            let mut ch = Character {
                text,
                format,
                size,
                width,
                delay,
                x: 0.0,
                y: 0.0,
                ruby,
            };
            let limit = if self.vertical { self.height } else { self.width };
            if auto_indent < 0
                && self.count == start_count
                && self.pending.is_empty()
                && self.option_contains("begin", unit)
            {
                self.line_indent -= self.advance(&ch);
            }
            let following = self.option_contains("following", unit)
                || auto_indent != 0 && self.option_contains("end", unit);
            if following
                && limit > 0.0
                && self.line_indent + self.pending_width() + self.advance(&ch) >= limit
            {
                if self.hanging_count >= kinsoku_max && !self.pending.is_empty() {
                    // Only the run beyond the margin and its preceding normal
                    // character move; earlier punctuation stays on this line.
                    let normal = self
                        .pending
                        .len()
                        .checked_sub(self.hanging_count + 1)
                        .filter(|&index| {
                            self.pending[index].text.first().is_some_and(|u| {
                                !self.option_contains("following", *u)
                                    && !(auto_indent != 0 && self.option_contains("end", *u))
                            })
                        });
                    ensure!(
                        normal != Some(0),
                        "TextRender kinsoku reflow of a complete narrow line is not implemented"
                    );
                    let mut carry = normal.map(|index| self.pending.split_off(index)).unwrap_or_default();
                    // The temporary replay pen counts in renderRight even when
                    // the hanging glyphs stay on the previous line.
                    if carry.is_empty() && !self.vertical {
                        let hanging: f32 = self
                            .pending
                            .iter()
                            .rev()
                            .take(self.hanging_count)
                            .map(|c| self.advance(c))
                            .sum();
                        self.replay_extent = self
                            .replay_extent
                            .max(self.line_indent + self.pending_width() + hanging);
                    }
                    let replay = if carry.is_empty() { self.hanging_count } else { carry.len() };
                    let extra = replay as f32 * step;
                    for c in &mut carry {
                        c.delay += extra;
                    }
                    delay += extra;
                    ch.delay = delay;
                    self.finish_line(host, true)?;
                    if let Some(first) = carry.first() {
                        if self.line_indent + self.advance(first) >= limit {
                            self.finish_line(host, true)?;
                        }
                    }
                    self.pending.extend(carry);
                }
                if self.line_indent + self.pending_width() + self.advance(&ch) >= limit {
                    self.hanging_count += 1;
                }
            }
            if limit > 0.0
                && self.line_indent + self.pending_width() + self.advance(&ch) >= limit
                && !following
            {
                let mut carry = vec![];
                while self.pending.len() > 1
                    && self.pending.last().is_some_and(|c| {
                        c.text.first().is_some_and(|u| self.option_contains("leading", *u))
                    })
                {
                    carry.push(self.pending.pop().expect("nonempty line"));
                }
                self.finish_line(host, true)?;
                for mut leading in carry.into_iter().rev() {
                    leading.delay += step;
                    delay += step;
                    leading.x = 0.0;
                    leading.y = 0.0;
                    self.pending.push(leading);
                }
                ch.delay = delay;
            }
            if self.over {
                break;
            }
            self.pending.push(ch);
            if auto_indent != 0 && self.option_contains("begin", unit) {
                self.indent_stack.push(self.next_indent);
                self.next_indent = self.line_indent + self.pending_width();
            }
            if auto_indent != 0 && self.option_contains("end", unit) {
                self.next_indent = self.indent_stack.pop().unwrap_or(0.0);
            }
            self.count += 1;
            self.delay = delay + base_delay;
            delay += step;
            ensure!(self.count <= MAX_CHARACTERS, "TextRender character limit exceeded");
        }
        if total > 0.0 {
            let factor = if self.delay != 0.0 { total / self.delay } else { 0.0 };
            for (index, c) in self
                .lines
                .iter_mut()
                .flat_map(|l| l.characters.iter_mut())
                .chain(self.pending.iter_mut())
                .enumerate()
            {
                if index >= start_count {
                    c.delay *= factor;
                }
            }
            self.delay = total;
        }
        Ok(!self.over)
    }
}

/// Which `set*` operation [`MarkupLayout::apply`] performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    Default,
    Font,
    Style,
}

fn terminated(units: &[u16], i: &mut usize, end: u16) -> Result<Vec<u16>> {
    let start = *i;
    while *i < units.len() && units[*i] != end {
        *i += 1;
    }
    ensure!(*i < units.len(), "unterminated TextRender command");
    let result = units[start..*i].to_vec();
    *i += 1;
    Ok(result)
}

/// Convenience for tests and simple frontends.
pub fn text(s: &str) -> Vec<u16> {
    units(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixed;
    impl MarkupHost for Fixed {
        fn text_width(&mut self, t: &[u16], size: f32) -> Result<f32> {
            Ok(t.len() as f32 * size)
        }
        fn font_changed(&mut self, _: bool, _: bool, _: &[u16]) -> Result<()> {
            Ok(())
        }
        fn eval(&mut self, e: &[u16]) -> Result<Vec<u16>> {
            Ok(e.to_vec())
        }
        fn label(&mut self, _: &[u16]) -> Result<f32> {
            Ok(0.0)
        }
    }
    #[test]
    fn wraps_aligns_and_reveals() {
        let mut l = MarkupLayout::default();
        l.set_render_size(100.0, 200.0, &mut Fixed).unwrap();
        assert!(l.render(&text("abcdef"), 0, 10.0, 0.0, false, &mut Fixed).unwrap());
        l.done(&mut Fixed).unwrap();
        let c = l.characters(0, 0);
        assert_eq!(c.len(), 6);
        // 24px glyphs in 100px: four per line, then wrap.
        assert_eq!((c[3].x, c[4].x), (72.0, 0.0));
        assert!(c[4].y > c[3].y);
        assert_eq!(l.render_delay(), 60.0);
        assert_eq!(l.show_count(25.0), 3);
        assert_eq!(l.line_count(), 2);
    }
    #[test]
    fn markup_commands_and_errors() {
        let mut l = MarkupLayout::default();
        l.set_render_size(1000.0, 200.0, &mut Fixed).unwrap();
        l.render(&text("%b1#ff0000;x$y;[r]z\\%"), 0, 0.0, 0.0, false, &mut Fixed).unwrap();
        l.done(&mut Fixed).unwrap();
        let c = l.characters(0, 0);
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].format.get("Bold"), Some(&Value::Int(1)));
        assert_eq!(c[0].format.get("ChColor"), Some(&Value::Int(0xffff0000)));
        assert!(c[2].ruby.is_some());
        assert_eq!(c[3].text, text("%"));
        assert!(l.render(&text("&g;"), 0, 0.0, 0.0, false, &mut Fixed).is_err());
        let mut n = MarkupLayout::default();
        n.set_render_size(1000.0, 200.0, &mut Fixed).unwrap();
        n.render(&text("x%n;ab%n2;c"), 0, 0.0, 0.0, false, &mut Fixed).unwrap();
        n.done(&mut Fixed).unwrap();
        let c = n.characters(0, 0);
        assert_eq!(c.len(), 4);
        assert_eq!(c[1].text, text("a"));
        // x, one blank line, "ab", two blank lines, "c".
        assert!(c[1].y > c[0].y && c[3].y > c[2].y);
        assert_eq!(n.line_count(), 6);
        assert!(MarkupLayout::default().render(&text("a"), 0, 0.0, 0.0, false, &mut Fixed).is_err());
    }
}
