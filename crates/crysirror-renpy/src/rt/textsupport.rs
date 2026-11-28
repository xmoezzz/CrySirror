// SPDX-License-Identifier: MPL-2.0
//! `renpy.text.textsupport`, after Ren'Py 6.18 - 8.6 `textsupport.pyx`
//! (MIT): the `Glyph` and `Line` records and the glyph-list passes of text
//! layout (break annotation, greedy/no-break wrapping, horizontal and
//! vertical placement, ruby, alignment/justification, slow-text times,
//! hyperlink areas, spacing adjustment).
//!
//! The passes run on [`Glyph`] slices; `renpy.text.text` lays out with them
//! directly and the Python functions convert lists of Python `Glyph`
//! objects to and from them, so both paths share one implementation.
//! UAX #14 classes and pair rules come from the shared
//! `crysirror_text::pair_linebreak` tables: the 65536-codepoint edition up
//! to 8.4, the Unicode 17 edition from 8.5. Release differences follow the
//! upstream sources of each release: CB handling (7.3+), closing
//! punctuation ignored for breaking (7.7/8.2+), the hyphen-aware 8.5
//! algorithm (including 8.5's handling of U+E0000 and above as CM), altruby
//! (7.0+), ruby line leading (7.7/8.2+), line baselines (7.8/8.3+),
//! `tweak_glyph_spacing`/`offset_glyphs` (to 7.6/8.1) and
//! `adjust_glyph_spacing`/`move_glyphs` (7.7/8.2+).
use super::{
    font::Font,
    py::{self, *},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use crysirror_text::pair_linebreak::{self, Edition, PairTable, Strictness};
use std::{cell::RefCell, collections::HashMap, ops::Range};

const MODULE: &str = "renpy.text.textsupport";

pub const SPLIT_NONE: u8 = 0;
pub const SPLIT_BEFORE: u8 = 1;
pub const SPLIT_INSTEAD: u8 = 2;
pub const SPLIT_IGNORE: u8 = 3;
pub const RUBY_NONE: u8 = 0;
pub const RUBY_BOTTOM: u8 = 1;
pub const RUBY_TOP: u8 = 2;
pub const RUBY_ALT: u8 = 3;
pub const MAX_WIDTH: f64 = 8192.;

/// The release whose semantics the passes follow.
#[derive(Clone, Copy, Debug)]
pub struct Rel {
    pub v: (u32, u32),
}

impl Rel {
    pub fn of(vm: &Vm) -> Rel {
        let v = super::state::try_rt(vm)
            .map(|r| (r.profile.version[0], r.profile.version[1]))
            .unwrap_or((8, 6));
        Rel { v }
    }
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        self.v >= (major, minor)
    }
    /// 7.`py2` / 8.`py3`.
    pub fn since_pair(&self, py2: u32, py3: u32) -> bool {
        self.at_least(8, py3) || (self.at_least(7, py2) && !self.at_least(8, 0))
    }
}

/// One glyph (upstream `Glyph`, plus the layout's piece and resolved font).
#[derive(Clone, Debug)]
pub struct Glyph {
    pub ch: char,
    pub variation: u32,
    pub glyph: u32,
    pub x: f64,
    pub y: f64,
    pub delta_x_adjustment: i64,
    pub split: u8,
    pub ruby: u8,
    pub ascent: f64,
    pub descent: f64,
    pub line_spacing: f64,
    pub width: f64,
    pub advance: f64,
    pub x_offset: f64,
    pub y_offset: f64,
    pub time: f64,
    pub duration: f64,
    pub hyperlink: i64,
    pub draw: bool,
    pub add_left: i64,
    pub add_top: i64,
    pub add_right: i64,
    pub add_bottom: i64,
    pub rtl: bool,
    pub index: i64,
    pub shader: Option<Value>,
    pub piece: usize,
    pub font: Option<Font>,
}

impl Default for Glyph {
    fn default() -> Self {
        Glyph {
            ch: '\0',
            variation: 0,
            glyph: 0,
            x: 0.,
            y: 0.,
            delta_x_adjustment: 0,
            split: SPLIT_NONE,
            ruby: RUBY_NONE,
            ascent: 0.,
            descent: 0.,
            line_spacing: 0.,
            width: 0.,
            advance: 0.,
            x_offset: 0.,
            y_offset: 0.,
            time: 0.,
            duration: -1.,
            hyperlink: 0,
            draw: true,
            add_left: 0,
            add_top: 0,
            add_right: 0,
            add_bottom: 0,
            rtl: false,
            index: 0,
            shader: None,
            piece: 0,
            font: None,
        }
    }
}

impl Glyph {
    fn is_ruby_top(&self) -> bool {
        self.ruby == RUBY_TOP || self.ruby == RUBY_ALT
    }
}

/// One line (upstream `Line`); `glyphs` indexes the layout's glyph list.
#[derive(Clone, Debug, Default)]
pub struct Line {
    pub y: f64,
    pub baseline: f64,
    pub height: f64,
    pub glyphs: Range<usize>,
    pub eop: bool,
    pub max_time: f64,
}

// Line breaking ------------------------------------------------------------------------------

pub fn annotate_western(gs: &mut [Glyph]) {
    for g in gs {
        if g.ruby != RUBY_NONE {
            continue;
        }
        g.split = match g.ch {
            '\0' => SPLIT_BEFORE,
            ' ' | '\u{200b}' => SPLIT_INSTEAD,
            _ => SPLIT_NONE,
        };
    }
}

pub fn annotate_anywhere(gs: &mut [Glyph]) {
    for g in gs {
        if g.ruby != RUBY_NONE {
            continue;
        }
        g.split = if g.ch == ' ' || g.ch == '\u{200b}' {
            SPLIT_INSTEAD
        } else {
            SPLIT_BEFORE
        };
    }
}

fn edition(rel: Rel) -> Edition {
    if rel.at_least(8, 5) {
        Edition::Unicode17
    } else {
        Edition::Bmp
    }
}

thread_local! {
    /// `language_tailor` overrides per table edition (class XX = none).
    static TAILOR: RefCell<HashMap<u32, u8>> = RefCell::new(HashMap::new());
}

/// `language_tailor(chars, cls)`.
pub fn language_tailor(rel: Rel, chars: &str, cls: &str) -> Result<(), String> {
    let t = pair_linebreak::table(edition(rel));
    let xx = t.class_id("XX").unwrap_or(0);
    let ncls = t.class_id(cls).unwrap_or(xx);
    for c in chars.chars() {
        if c as u32 >= t.char_count() {
            return Err(format!(
                "Character U+{:04x} is out of range for language_tailor.",
                c as u32
            ));
        }
        TAILOR.with(|m| {
            m.borrow_mut().insert(c as u32, ncls);
        });
    }
    Ok(())
}

fn tailor(c: u32) -> Option<u8> {
    TAILOR.with(|m| m.borrow().get(&c).copied())
}

struct Classes {
    sp: u8,
    cm: u8,
    cb: u8,
    cl: u8,
    cp: u8,
    wj: u8,
    gl: u8,
    al: u8,
    id: u8,
    nu: u8,
    hy: u8,
    hh: Option<u8>,
    xx: u8,
    pitch: u8,
    h: [u8; 6],
}

fn classes(t: &PairTable) -> Classes {
    let c = |n: &str| t.class_id(n).expect("line-break class");
    Classes {
        sp: c("SP"),
        cm: c("CM"),
        cb: c("CB"),
        cl: c("CL"),
        cp: c("CP"),
        wj: c("WJ"),
        gl: c("GL"),
        al: c("AL"),
        id: c("ID"),
        nu: c("NU"),
        hy: c("HY"),
        hh: t.class_id("HH"),
        xx: c("XX"),
        pitch: t.pitch(),
        h: [c("H2"), c("H3"), c("ID"), c("JL"), c("JV"), c("JT")],
    }
}

/// `annotate_unicode(glyphs, no_ideographs, cjk)`: cjk 0 western,
/// 1 loose, 2 normal, 3 strict.
pub fn annotate_unicode(rel: Rel, gs: &mut [Glyph], no_ideographs: bool, cjk: i64) {
    if gs.is_empty() {
        return;
    }
    let t = pair_linebreak::table(edition(rel));
    let k = classes(t);
    let strict = match cjk {
        1 => Strictness::CjkLoose,
        2 => Strictness::CjkNormal,
        3 => Strictness::CjkStrict,
        _ => Strictness::Western,
    };
    let modern = rel.at_least(8, 5);
    let cb_rules = rel.at_least(7, 3);
    let ignore_close = rel.since_pair(7, 2);
    let mut new_type = k.wj;
    let mut old_type = k.wj;
    let mut new_hyphen = false;
    let mut space_pos = 0usize;
    for pos in 1..gs.len() {
        let c = gs[pos].ch as u32;
        let tailor_type;
        if modern {
            old_type = new_type;
            let old_hyphen = new_hyphen;
            if c < t.char_count() {
                new_type = t.class(strict, c).unwrap_or(k.xx);
                tailor_type = tailor(c).unwrap_or(k.xx);
            } else if c < 0xE0000 {
                new_type = k.id;
                tailor_type = k.xx;
            } else if !rel.at_least(8, 6) || c < 0xF0000 {
                // 8.5 tests the previous class here, which always passes.
                new_type = k.cm;
                tailor_type = k.xx;
            } else {
                new_type = k.xx;
                tailor_type = k.xx;
            }
            if no_ideographs && k.h.contains(&new_type) {
                new_type = k.al;
            }
            if tailor_type != k.xx {
                new_type = tailor_type;
            }
            new_hyphen = (new_type == k.hy || Some(new_type) == k.hh) && old_type != k.sp;
            if new_type >= k.pitch && new_type != k.sp && new_type != k.cb {
                new_type = k.al;
            }
            let g = &mut gs[pos];
            if new_type == k.sp {
                g.split = SPLIT_NONE;
                space_pos = pos;
                new_type = old_type;
                continue;
            }
            if new_type == k.cm {
                g.split = SPLIT_NONE;
                new_type = old_type;
                continue;
            }
            if new_type == k.cb {
                g.split = if old_type == k.wj || old_type == k.gl {
                    SPLIT_NONE
                } else {
                    SPLIT_BEFORE
                };
                new_type = old_type;
                continue;
            }
            if old_type == k.cb {
                g.split = if new_type == k.wj || new_type == k.gl {
                    SPLIT_NONE
                } else {
                    SPLIT_BEFORE
                };
                continue;
            }
            if new_type == k.cl || new_type == k.cp {
                g.split = SPLIT_IGNORE;
                new_type = old_type;
                continue;
            }
            apply_rule(t, gs, pos, old_type, new_type, &mut space_pos);
            if old_hyphen
                && gs[pos].split == SPLIT_NONE
                && (old_type != k.hy || new_type != k.nu)
            {
                gs[pos].split = SPLIT_BEFORE;
            }
            space_pos = 0;
        } else {
            if (0x20000..=0x2ffff).contains(&c) {
                new_type = k.id;
                tailor_type = k.xx;
            } else if c > 65535 {
                new_type = k.al;
                tailor_type = k.xx;
            } else {
                new_type = t.class(strict, c).unwrap_or(k.xx);
                tailor_type = tailor(c).unwrap_or(k.xx);
            }
            if no_ideographs && k.h.contains(&new_type) {
                new_type = k.al;
            }
            if new_type >= k.pitch && new_type != k.sp && (!cb_rules || new_type != k.cb) {
                new_type = k.al;
            }
            if tailor_type != k.xx {
                new_type = tailor_type;
            }
            let g = &mut gs[pos];
            if new_type == k.sp {
                g.split = SPLIT_NONE;
                space_pos = pos;
                continue;
            }
            if new_type == k.cm {
                g.split = SPLIT_NONE;
                continue;
            }
            if cb_rules {
                if new_type == k.cb {
                    g.split = if old_type == k.wj || old_type == k.gl {
                        SPLIT_NONE
                    } else {
                        SPLIT_BEFORE
                    };
                    continue;
                }
                if old_type == k.cb {
                    g.split = if new_type == k.wj || new_type == k.gl {
                        SPLIT_NONE
                    } else {
                        SPLIT_BEFORE
                    };
                    continue;
                }
            }
            if ignore_close && (new_type == k.cl || new_type == k.cp) {
                g.split = SPLIT_IGNORE;
                continue;
            }
            apply_rule(t, gs, pos, old_type, new_type, &mut space_pos);
            old_type = new_type;
            space_pos = 0;
        }
    }
    gs[0].split = SPLIT_NONE;
    let mut old_ruby = gs[0].ruby;
    for g in gs.iter_mut() {
        if g.ch == '\0' {
            g.split = SPLIT_BEFORE;
        }
        if g.is_ruby_top() {
            g.split = SPLIT_NONE;
        } else if g.ruby == RUBY_BOTTOM && old_ruby == RUBY_BOTTOM {
            g.split = SPLIT_NONE;
        }
        old_ruby = g.ruby;
    }
}

fn apply_rule(t: &PairTable, gs: &mut [Glyph], pos: usize, old: u8, new: u8, space_pos: &mut usize) {
    match t.rule(old, new) {
        b'%' => {
            if *space_pos != 0 {
                gs[*space_pos].split = SPLIT_INSTEAD;
            }
            gs[pos].split = SPLIT_NONE;
        }
        b'_' => {
            if *space_pos != 0 {
                gs[*space_pos].split = SPLIT_INSTEAD;
                gs[pos].split = SPLIT_NONE;
            } else {
                gs[pos].split = SPLIT_BEFORE;
            }
        }
        _ => gs[pos].split = SPLIT_NONE,
    }
}

/// Cython `float` accumulation (single precision), as upstream computes.
fn f32r(x: f64) -> f64 {
    x as f32 as f64
}

pub fn linebreak_greedy(gs: &mut [Glyph], first_width: f64, rest_width: f64) {
    let mut width = first_width.trunc();
    let rest = rest_width.trunc();
    let mut split_g: Option<usize> = None;
    let (mut x, mut splitx, mut ignored) = (0f64, 0f64, 0f64);
    for i in 0..gs.len() {
        if gs[i].is_ruby_top() {
            continue;
        }
        if gs[i].split == SPLIT_IGNORE {
            ignored = f32r(ignored + gs[i].advance);
            continue;
        }
        if x > width && split_g.is_some() {
            x = splitx;
            split_g = None;
            width = rest;
        }
        let adv = gs[i].advance;
        x = f32r(x + adv + ignored);
        splitx = f32r(splitx + adv + ignored);
        ignored = 0.;
        match gs[i].split {
            SPLIT_INSTEAD => {
                if let Some(s) = split_g {
                    gs[s].split = SPLIT_NONE;
                }
                split_g = Some(i);
                splitx = 0.;
            }
            SPLIT_BEFORE => {
                if let Some(s) = split_g {
                    gs[s].split = SPLIT_NONE;
                }
                split_g = Some(i);
                splitx = adv;
            }
            _ => {}
        }
    }
    if x > width {
        split_g = None;
    }
    if let Some(s) = split_g {
        gs[s].split = SPLIT_NONE;
    }
}

pub fn linebreak_nobreak(gs: &mut [Glyph]) {
    for g in gs {
        g.split = SPLIT_NONE;
    }
}

fn chr(c: char) -> char {
    c
}

pub fn linebreak_debug(gs: &[Glyph]) -> String {
    let mut rv = String::new();
    for g in gs {
        match g.split {
            SPLIT_INSTEAD => rv.push('|'),
            SPLIT_BEFORE => {
                rv.push('[');
                rv.push(chr(g.ch));
            }
            _ => rv.push(chr(g.ch)),
        }
    }
    rv
}

pub fn linebreak_list(gs: &[Glyph]) -> Vec<String> {
    let mut rv = Vec::new();
    let mut line = String::new();
    for g in gs {
        match g.split {
            SPLIT_INSTEAD => rv.push(std::mem::take(&mut line)),
            SPLIT_BEFORE => {
                rv.push(std::mem::take(&mut line));
                line.push(g.ch);
            }
            _ => line.push(g.ch),
        }
    }
    if !line.is_empty() {
        rv.push(line);
    }
    rv
}

// Placement ----------------------------------------------------------------------------------

/// Returns the width of the laid-out text.
pub fn place_horizontal(gs: &mut [Glyph], start_x: f64, first_indent: f64, rest_indent: f64) -> f64 {
    if gs.is_empty() {
        return 0.;
    }
    let mut x = f32r(start_x + first_indent);
    let mut maxx = 0f64;
    let mut old: Option<usize> = None;
    for i in 0..gs.len() {
        if gs[i].is_ruby_top() {
            continue;
        }
        if gs[i].split == SPLIT_IGNORE {
            gs[i].split = SPLIT_NONE;
        }
        if gs[i].split != SPLIT_NONE
            && let Some(o) = old
        {
            gs[o].advance = gs[o].width;
        }
        if gs[i].split == SPLIT_INSTEAD {
            x = f32r(start_x + rest_indent);
            continue;
        } else if gs[i].split == SPLIT_BEFORE {
            x = f32r(start_x + rest_indent);
        }
        gs[i].x = (x + 0.5).trunc();
        maxx = maxx.max(f32r(x + gs[i].width)).max(f32r(x + gs[i].advance));
        x = f32r(x + gs[i].advance).min(MAX_WIDTH);
        old = Some(i);
    }
    maxx
}

/// Places lines of the glyphs (indexed from `base` in the layout) and
/// returns them with the y of the next line.
pub fn place_vertical(
    rel: Rel,
    gs: &mut [Glyph],
    base: usize,
    y: f64,
    spacing: f64,
    leading: f64,
    ruby_line_leading: f64,
) -> (Vec<Line>, f64) {
    let mut rv = Vec::new();
    if gs.is_empty() {
        return (rv, y.trunc());
    }
    let (spacing, leading, ruby_ll) = (spacing.trunc(), leading.trunc(), ruby_line_leading.trunc());
    let n = gs.len();
    let (mut pos, mut sol) = (0usize, 0usize);
    let (mut ascent, mut line_spacing) = (0f64, 0f64);
    let mut y = y.trunc() + leading;
    loop {
        let end_line = pos >= n || gs[pos].split != SPLIT_NONE;
        if end_line {
            let mut has_ruby = false;
            for g in gs[sol..pos].iter_mut() {
                if g.ruby == RUBY_TOP || (g.ruby == RUBY_ALT && rel.at_least(7, 0)) {
                    has_ruby = true;
                    continue;
                }
                if g.ascent != 0. {
                    g.y = y + ascent;
                } else {
                    g.y = y;
                    if rel.at_least(6, 99) {
                        g.ascent = ascent;
                    }
                }
            }
            let mut line_leading = leading;
            if has_ruby && rel.since_pair(7, 2) {
                y += ruby_ll;
                line_leading += ruby_ll;
                for g in gs[sol..pos].iter_mut() {
                    if !g.is_ruby_top() {
                        g.y += ruby_ll;
                    }
                }
            }
            rv.push(Line {
                y: y - line_leading,
                baseline: y + ascent,
                height: line_leading + line_spacing + spacing,
                glyphs: base + sol..base + pos,
                eop: false,
                max_time: 0.,
            });
            y += line_spacing + spacing + leading;
            sol = pos;
            ascent = 0.;
            line_spacing = 0.;
            if pos < n && gs[pos].split == SPLIT_INSTEAD {
                sol += 1;
                pos += 1;
                if pos >= n {
                    break;
                }
                continue;
            }
        }
        if pos >= n {
            break;
        }
        ascent = ascent.max(gs[pos].ascent.trunc());
        line_spacing = line_spacing.max(gs[pos].line_spacing.trunc());
        pos += 1;
    }
    if let Some(l) = rv.last_mut() {
        l.eop = true;
    }
    (rv, y - leading)
}

pub fn kerning(gs: &mut [Glyph], amount: f64) {
    for g in gs {
        g.advance = f32r(g.advance + amount);
    }
}

/// Assigns show times from `t` at `gps` glyphs per second; returns the
/// time after the last glyph.
pub fn assign_times(rel: Rel, mut t: f64, gps: f64, gs: &mut [Glyph]) -> f64 {
    let tpg = if gps == 0. { 0. } else { f32r(1. / gps) };
    for g in gs {
        let ruby = if rel.at_least(7, 0) {
            g.is_ruby_top()
        } else {
            g.ruby == RUBY_TOP
        };
        if ruby {
            g.time = if rel.at_least(7, 3) { -1. } else { t };
            continue;
        }
        t = f32r(t + tpg);
        g.time = t;
        if rel.since_pair(8, 3) {
            g.duration = tpg;
        }
    }
    t
}

pub fn max_times(gs: &[Glyph], lines: &mut [Line]) -> f64 {
    let mut max_time = 0f64;
    for l in lines {
        for g in &gs[l.glyphs.clone()] {
            if g.time > max_time {
                max_time = g.time;
            }
        }
        l.max_time = max_time;
    }
    max_time
}

pub fn assign_index(mut index: i64, gs: &mut [Glyph]) -> i64 {
    for g in gs {
        if g.time != -1. {
            g.index = index;
            index += 1;
        }
    }
    index
}

/// (hyperlink, x, y, w, h, first shown time) rectangles.
pub fn hyperlink_areas(gs: &[Glyph], lines: &[Line]) -> Vec<(i64, f64, f64, f64, f64, f64)> {
    let mut rv = Vec::new();
    for l in lines {
        let mut hyperlink = 0i64;
        let (mut min_x, mut max_x, mut ht) = (1_000_000f64, 0f64, 86400f64);
        for g in &gs[l.glyphs.clone()] {
            if hyperlink != 0 && g.hyperlink != hyperlink {
                rv.push((hyperlink, min_x, l.y, max_x - min_x, l.height, ht));
                ht = 86400.;
                max_x = 0.;
                min_x = 1_000_000.;
            }
            hyperlink = g.hyperlink;
            if hyperlink != 0 {
                if g.x < min_x {
                    min_x = g.x;
                }
                if g.x + g.width > max_x {
                    max_x = g.x + g.width.trunc();
                }
                ht = ht.min(g.time);
            }
        }
        if hyperlink != 0 {
            rv.push((hyperlink, min_x, l.y, max_x - min_x, l.height, ht));
        }
    }
    rv
}

/// `place_ruby`; 6.18 raises when ruby leaves the surface.
pub fn place_ruby(
    rel: Rel,
    gs: &mut [Glyph],
    ruby_offset: f64,
    altruby_offset: f64,
    surf: (f64, f64),
) -> Result<(), String> {
    let n = gs.len();
    let (ruby_offset, altruby_offset) = (ruby_offset.trunc(), altruby_offset.trunc());
    let (mut min_x, mut max_x, mut y) = (0f64, 0f64, 0f64);
    let mut last = RUBY_NONE;
    let mut pos = 0;
    while pos < n {
        let r = gs[pos].ruby;
        if r == RUBY_NONE {
            min_x = gs[pos].x;
            max_x = f32r(gs[pos].x + gs[pos].width);
            y = gs[pos].y;
            last = RUBY_NONE;
            pos += 1;
            continue;
        }
        if r == RUBY_BOTTOM {
            if last != RUBY_BOTTOM {
                min_x = gs[pos].x;
            }
            max_x = f32r(gs[pos].x + gs[pos].width);
            y = gs[pos].y;
            last = RUBY_BOTTOM;
            pos += 1;
            continue;
        }
        let start = pos;
        last = if rel.at_least(7, 0) { r } else { RUBY_TOP };
        while pos < n && gs[pos].ruby == last {
            pos += 1;
        }
        if pos == start {
            // A RUBY_ALT glyph before altruby existed: not a ruby run.
            pos += 1;
            continue;
        }
        let mut width = 0f64;
        for g in &gs[start..pos] {
            width = f32r(width + g.advance);
        }
        width = f32r(width - gs[pos - 1].advance);
        width = f32r(width + gs[pos - 1].width);
        let mut x = f32r((max_x + min_x) / 2. - width / 2.);
        for g in &mut gs[start..pos] {
            g.x = (x + 0.5).trunc();
            if !rel.at_least(6, 99) {
                let fail = g.x < 0.
                    || g.x + g.width > surf.0
                    || g.y - g.ascent < 0.
                    || g.y - g.ascent + g.line_spacing > surf.1;
                if fail {
                    return Err("Ruby glyph out of bounds.".into());
                }
            }
            g.y = y + if g.ruby == RUBY_TOP {
                ruby_offset
            } else {
                altruby_offset
            };
            x = f32r(x + g.advance);
        }
    }
    Ok(())
}

pub fn align_and_justify(gs: &mut [Glyph], lines: &[Line], width: f64, text_align: f64, justify: bool) {
    let width = width.trunc();
    if !justify && text_align == 0. {
        return;
    }
    for l in lines {
        let mut spaces = 0;
        let mut max_x = 0f64;
        for g in &gs[l.glyphs.clone()] {
            if g.is_ruby_top() {
                continue;
            }
            if g.ch == ' ' {
                spaces += 1;
            }
            max_x = (g.x + g.width).trunc();
        }
        if max_x >= MAX_WIDTH {
            continue;
        }
        if justify && spaces > 0 && !l.eop {
            let per = f32r((width - max_x) / spaces as f64);
            let mut off = 0.5f64;
            for g in &mut gs[l.glyphs.clone()] {
                if g.is_ruby_top() {
                    continue;
                }
                if g.ch == ' ' {
                    off = f32r(off + per);
                }
                g.x += off.trunc();
            }
        } else {
            let off = f32r((width - max_x) * text_align).trunc();
            for g in &mut gs[l.glyphs.clone()] {
                if !g.is_ruby_top() {
                    g.x += off;
                }
            }
        }
    }
}

/// Reverses each line of `gs` in place, keeping line order.
pub fn reverse_lines(rel: Rel, gs: Vec<Glyph>) -> Vec<Glyph> {
    let mut rv = Vec::with_capacity(gs.len());
    let mut block = Vec::new();
    for mut g in gs {
        if g.split == SPLIT_INSTEAD {
            block.reverse();
            rv.append(&mut block);
            rv.push(g);
            continue;
        }
        if rel.since_pair(8, 3) {
            g.rtl = true;
        }
        block.push(g);
    }
    block.reverse();
    rv.append(&mut block);
    rv
}

pub fn copy_splits(source: &[Glyph], dest: &mut [Glyph]) {
    for (d, s) in dest.iter_mut().zip(source) {
        d.split = s.split;
    }
}

/// `tweak_glyph_spacing` (to 7.6/8.1) and `adjust_glyph_spacing` (7.7/8.2+).
pub fn adjust_glyph_spacing(rel: Rel, gs: &mut [Glyph], lines: &mut [Line], dx: f64, dy: f64, w: f64, h: f64) {
    if w <= 0. || h <= 0. {
        return;
    }
    // 6.99 - 7.5 store the offsets in C shorts.
    let short = rel.at_least(7, 0) && !rel.since_pair(6, 1);
    let cast = |v: f64| -> f64 {
        let t = v.trunc();
        if short { t as i64 as i16 as f64 } else { t }
    };
    let mut old = 0f64;
    for g in gs.iter_mut() {
        let xa = cast(dx * g.x / w);
        g.x += xa;
        g.y += cast(dy * g.y / h);
        if xa > old {
            g.delta_x_adjustment = (xa - old) as i64;
        }
        old = xa;
    }
    let cap = rel.since_pair(6, 1);
    for l in lines {
        let mut end = l.y + l.height;
        if cap && end > 32767. {
            break;
        }
        l.y += (dy * l.y / h).trunc();
        end += (dy * end / h).trunc();
        l.height = end - l.y;
    }
}

pub fn move_glyphs(gs: &mut [Glyph], x: f64, y: f64) {
    if x == 0. && y == 0. {
        return;
    }
    for g in gs {
        g.x += x.trunc();
        g.y += y.trunc();
    }
}

// Python surface ------------------------------------------------------------------------------

fn delta_name(rel: Rel) -> &'static str {
    if rel.since_pair(7, 2) {
        "delta_x_adjustment"
    } else {
        "delta_x_offset"
    }
}

fn glyph_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    write_glyph(vm, rel, &args[0], &Glyph::default())?;
    Ok(Value::None)
}

fn i(v: f64) -> Value {
    Value::Int(v as i64)
}

/// Stores every field of `g` on a Python `Glyph`.
pub fn write_glyph(vm: &mut Vm, rel: Rel, o: &Value, g: &Glyph) -> PyResult<()> {
    let ch = if g.ch == '\0' { 0 } else { g.ch as i64 };
    let mut fields: Vec<(&str, Value)> = vec![
        ("x", i(g.x)),
        ("y", i(g.y)),
        (delta_name(rel), Value::Int(g.delta_x_adjustment)),
        ("character", Value::Int(ch)),
        ("split", Value::Int(g.split as i64)),
        ("ruby", Value::Int(g.ruby as i64)),
        ("ascent", i(g.ascent)),
        ("line_spacing", i(g.line_spacing)),
        ("width", Value::Float(f32r(g.width))),
        ("advance", Value::Float(f32r(g.advance))),
        ("time", Value::Float(f32r(g.time))),
        ("hyperlink", Value::Int(g.hyperlink)),
        ("draw", Value::Bool(g.draw)),
    ];
    if rel.at_least(7, 4) {
        fields.push(("variation", Value::Int(g.variation as i64)));
    }
    if rel.since_pair(7, 2) {
        fields.push(("glyph", Value::Int(g.glyph as i64)));
        fields.push(("x_offset", Value::Float(f32r(g.x_offset))));
        fields.push(("y_offset", Value::Float(f32r(g.y_offset))));
    }
    if rel.since_pair(8, 3) {
        fields.extend([
            ("descent", i(g.descent)),
            ("add_left", Value::Int(g.add_left)),
            ("add_top", Value::Int(g.add_top)),
            ("add_right", Value::Int(g.add_right)),
            ("add_bottom", Value::Int(g.add_bottom)),
            ("rtl", Value::Bool(g.rtl)),
            ("duration", Value::Float(f32r(g.duration))),
            ("shader", g.shader.clone().unwrap_or(Value::None)),
            ("index", Value::Int(g.index)),
        ]);
    }
    for (n, v) in fields {
        py::set(vm, o, n, v)?;
    }
    Ok(())
}

fn num(vm: &mut Vm, o: &Value, n: &str) -> PyResult<f64> {
    match py::get_opt(vm, o, n)? {
        Some(v) if !py::is_none(&v) => py::f64_of(vm, &v),
        _ => Ok(0.),
    }
}

/// Reads a Python `Glyph` (unknown or missing fields keep their defaults).
pub fn read_glyph(vm: &mut Vm, rel: Rel, o: &Value) -> PyResult<Glyph> {
    let mut g = Glyph::default();
    let c = num(vm, o, "character")? as u32;
    g.ch = char::from_u32(c).unwrap_or('\u{fffd}');
    g.x = num(vm, o, "x")?.trunc();
    g.y = num(vm, o, "y")?.trunc();
    g.delta_x_adjustment = num(vm, o, delta_name(rel))? as i64;
    g.split = num(vm, o, "split")? as u8;
    g.ruby = num(vm, o, "ruby")? as u8;
    g.ascent = num(vm, o, "ascent")?.trunc();
    g.descent = num(vm, o, "descent")?.trunc();
    g.line_spacing = num(vm, o, "line_spacing")?.trunc();
    g.width = num(vm, o, "width")?;
    g.advance = num(vm, o, "advance")?;
    g.x_offset = num(vm, o, "x_offset")?;
    g.y_offset = num(vm, o, "y_offset")?;
    g.time = num(vm, o, "time")?;
    g.hyperlink = num(vm, o, "hyperlink")? as i64;
    g.variation = num(vm, o, "variation")? as u32;
    g.glyph = num(vm, o, "glyph")? as u32;
    g.add_left = num(vm, o, "add_left")? as i64;
    g.add_top = num(vm, o, "add_top")? as i64;
    g.add_right = num(vm, o, "add_right")? as i64;
    g.add_bottom = num(vm, o, "add_bottom")? as i64;
    g.index = num(vm, o, "index")? as i64;
    if let Some(v) = py::get_opt(vm, o, "draw")? {
        g.draw = ops::truthy(vm, &v)?;
    }
    if let Some(v) = py::get_opt(vm, o, "rtl")? {
        g.rtl = ops::truthy(vm, &v)?;
    }
    if rel.since_pair(8, 3) {
        g.duration = num(vm, o, "duration")?;
        g.shader = py::get_opt(vm, o, "shader")?.filter(|v| !py::is_none(v));
    }
    Ok(g)
}

fn glyph_list(vm: &mut Vm, rel: Rel, v: &Value) -> PyResult<(Vec<Value>, Vec<Glyph>)> {
    let objs = py::items(vm, v)?;
    let mut gs = Vec::with_capacity(objs.len());
    for o in &objs {
        gs.push(read_glyph(vm, rel, o)?);
    }
    Ok((objs, gs))
}

fn write_back(vm: &mut Vm, rel: Rel, objs: &[Value], gs: &[Glyph]) -> PyResult<()> {
    for (o, g) in objs.iter().zip(gs) {
        write_glyph(vm, rel, o, g)?;
    }
    Ok(())
}

/// Runs `f` over a Python glyph list and stores the results back.
fn with_glyphs<R>(
    vm: &mut Vm,
    v: &Value,
    f: impl FnOnce(Rel, &mut Vec<Glyph>) -> R,
) -> PyResult<R> {
    let rel = Rel::of(vm);
    let (objs, mut gs) = glyph_list(vm, rel, v)?;
    let r = f(rel, &mut gs);
    write_back(vm, rel, &objs, &gs)?;
    Ok(r)
}

fn a(args: &[Value], kw: Kw, i: usize, n: &str) -> Value {
    arg_or(args, kw, i, n, Value::None)
}

fn af(vm: &mut Vm, args: &[Value], kw: Kw, i: usize, n: &str) -> PyResult<f64> {
    let v = a(args, kw, i, n);
    py::f64_of(vm, &v)
}

fn glyph_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let g = read_glyph(vm, rel, &args[0])?;
    let time = py::get(vm, &args[0], "time")?;
    let time = ops::repr(vm, &time)?.to_string();
    let c = g.ch as u32;
    let s = if rel.at_least(8, 5) {
        let split = ["SPLIT_NONE", "SPLIT_BEFORE", "SPLIT_INSTEAD", "SPLIT_IGNORE"]
            .get(g.split as usize)
            .copied()
            .unwrap_or("?");
        if g.variation == 0 {
            format!("<Glyph U+{c:4x} {} time={time} {split}>", g.ch)
        } else {
            format!("<Glyph U+{c:4x} {} vs={} time={time} {split}>", g.ch, g.variation)
        }
    } else if g.variation == 0 {
        format!("<Glyph {c} time={time}>")
    } else {
        format!("<Glyph {c} vs={} time={time}>", g.variation)
    };
    Ok(vm.types.new_string(s))
}

fn line_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let this = &args[0];
    let rest = &args[1..];
    let (y, baseline, height, glyphs) = if rel.since_pair(8, 3) {
        (a(rest, kw, 0, "y"), Some(a(rest, kw, 1, "baseline")), a(rest, kw, 2, "height"), a(rest, kw, 3, "glyphs"))
    } else {
        (a(rest, kw, 0, "y"), None, a(rest, kw, 1, "height"), a(rest, kw, 2, "glyphs"))
    };
    let int = |vm: &mut Vm, v: &Value| -> PyResult<Value> { Ok(Value::Int(py::f64_of(vm, v)? as i64)) };
    let y = int(vm, &y)?;
    py::set(vm, this, "y", y)?;
    if let Some(b) = baseline {
        let b = int(vm, &b)?;
        py::set(vm, this, "baseline", b)?;
    }
    let h = int(vm, &height)?;
    py::set(vm, this, "height", h)?;
    py::set(vm, this, "glyphs", glyphs)?;
    py::set(vm, this, "max_time", Value::Float(0.))?;
    py::set(vm, this, "eop", Value::Bool(false))?;
    Ok(Value::None)
}

fn line_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let y = num(vm, &args[0], "y")? as i64;
    let h = num(vm, &args[0], "height")? as i64;
    let gs = py::get(vm, &args[0], "glyphs")?;
    let gs = ops::repr(vm, &gs)?.to_string();
    Ok(vm.types.new_string(format!("<Line y={y}, height={h}, glyphs={gs}>")))
}

/// Python `Line` objects of a layout's lines over Python glyph objects.
fn py_lines(vm: &mut Vm, rel: Rel, lines: &[Line], objs: &[Value], start: usize) -> PyResult<Value> {
    let cls = py::import_attr(vm, MODULE, "Line")?;
    let mut out = Vec::new();
    for l in lines {
        let gl = py::list(vm, objs[l.glyphs.start - start..l.glyphs.end - start].to_vec());
        let mut a = vec![Value::Int(l.y as i64)];
        if rel.since_pair(8, 3) {
            a.push(Value::Int(l.baseline as i64));
        }
        a.push(Value::Int(l.height as i64));
        a.push(gl);
        let o = vm.call(&cls, a, Vec::new())?;
        py::set(vm, &o, "eop", Value::Bool(l.eop))?;
        out.push(o);
    }
    Ok(py::list(vm, out))
}

/// Reads Python `Line`s whose glyphs are objects of `objs` (by identity),
/// or appends their glyphs when they are not.
fn read_lines(vm: &mut Vm, rel: Rel, v: &Value, objs: &mut Vec<Value>, gs: &mut Vec<Glyph>) -> PyResult<(Vec<Value>, Vec<Line>)> {
    let lobjs = py::items(vm, v)?;
    let mut lines = Vec::new();
    for lo in &lobjs {
        let gl = py::get(vm, lo, "glyphs")?;
        let items = py::items(vm, &gl)?;
        let start = objs.len();
        for o in &items {
            objs.push(o.clone());
            gs.push(read_glyph(vm, rel, o)?);
        }
        let eop = py::get_opt(vm, lo, "eop")?.map(|e| ops::truthy(vm, &e)).transpose()?.unwrap_or(false);
        lines.push(Line {
            y: num(vm, lo, "y")?,
            baseline: num(vm, lo, "baseline")?,
            height: num(vm, lo, "height")?,
            glyphs: start..objs.len(),
            eop,
            max_time: num(vm, lo, "max_time")?,
        });
    }
    Ok((lobjs, lines))
}

fn f_annotate_western(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| annotate_western(g))?;
    Ok(Value::None)
}
fn f_annotate_anywhere(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| annotate_anywhere(g))?;
    Ok(Value::None)
}
fn f_annotate_unicode(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let ni = a(args, kw, 1, "no_ideographs");
    let ni = ops::truthy(vm, &ni)?;
    let cjk = py::int(vm, &a(args, kw, 2, "cjk"))?;
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |r, g| annotate_unicode(r, g, ni, cjk))?;
    Ok(Value::None)
}
fn f_language_tailor(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let chars = py::string(vm, &a(args, kw, 0, "chars"))?;
    let cls = py::string(vm, &a(args, kw, 1, "cls"))?;
    let rel = Rel::of(vm);
    language_tailor(rel, &chars, &cls).map_err(|e| py::error(vm, e))?;
    Ok(Value::None)
}
fn f_linebreak_greedy(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let fw = af(vm, args, kw, 1, "first_width")?;
    let rw = af(vm, args, kw, 2, "rest_width")?;
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| linebreak_greedy(g, fw, rw))?;
    Ok(Value::None)
}
fn f_linebreak_nobreak(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| linebreak_nobreak(g))?;
    Ok(Value::None)
}
fn f_linebreak_debug(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| linebreak_debug(g))?;
    Ok(vm.types.new_string(s))
}
fn f_linebreak_list(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let l = with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| linebreak_list(g))?;
    let items = l.into_iter().map(|s| vm.types.new_string(s)).collect();
    Ok(py::list(vm, items))
}
fn f_place_horizontal(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let sx = af(vm, args, kw, 1, "start_x")?;
    let fi = af(vm, args, kw, 2, "first_indent")?;
    let ri = af(vm, args, kw, 3, "rest_indent")?;
    let w = with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| place_horizontal(g, sx, fi, ri))?;
    Ok(Value::Float(f32r(w)))
}
fn f_place_vertical(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let y = af(vm, args, kw, 1, "y")?;
    let sp = af(vm, args, kw, 2, "spacing")?;
    let le = af(vm, args, kw, 3, "leading")?;
    let rl = if rel.since_pair(7, 2) {
        af(vm, args, kw, 4, "ruby_line_leading")?
    } else {
        0.
    };
    let (objs, mut gs) = glyph_list(vm, rel, &a(args, kw, 0, "glyphs"))?;
    let (lines, ny) = place_vertical(rel, &mut gs, 0, y, sp, le, rl);
    write_back(vm, rel, &objs, &gs)?;
    if lines.is_empty() {
        let l = py::list(vm, Vec::new());
        return Ok(py::tuple(vm, vec![l, Value::Int(ny as i64)]));
    }
    let l = py_lines(vm, rel, &lines, &objs, 0)?;
    Ok(py::tuple(vm, vec![l, Value::Int(ny as i64)]))
}
fn f_kerning(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let amount = af(vm, args, kw, 1, "amount")?;
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| kerning(g, amount))?;
    Ok(Value::None)
}
fn f_assign_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let t = af(vm, args, kw, 0, "t")?;
    let gps = af(vm, args, kw, 1, "gps")?;
    let r = with_glyphs(vm, &a(args, kw, 2, "glyphs"), |rel, g| assign_times(rel, t, gps, g))?;
    Ok(Value::Float(r))
}
fn f_max_times(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let (mut objs, mut gs) = (Vec::new(), Vec::new());
    let (lobjs, mut lines) = read_lines(vm, rel, &a(args, kw, 0, "l"), &mut objs, &mut gs)?;
    let m = max_times(&gs, &mut lines);
    for (lo, l) in lobjs.iter().zip(&lines) {
        py::set(vm, lo, "max_time", Value::Float(f32r(l.max_time)))?;
    }
    Ok(Value::Float(f32r(m)))
}
fn f_assign_index(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let index = py::int(vm, &a(args, kw, 0, "index"))?;
    let r = with_glyphs(vm, &a(args, kw, 1, "glyphs"), |_, g| assign_index(index, g))?;
    Ok(Value::Int(r))
}
fn f_hyperlink_areas(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let (mut objs, mut gs) = (Vec::new(), Vec::new());
    let (_, lines) = read_lines(vm, rel, &a(args, kw, 0, "l"), &mut objs, &mut gs)?;
    let six = rel.since_pair(6, 1);
    let out = hyperlink_areas(&gs, &lines)
        .into_iter()
        .map(|(h, x, y, w, hh, t)| {
            let mut v = vec![Value::Int(h), i(x), i(y), i(w), i(hh)];
            if six {
                v.push(Value::Float(f32r(t)));
            }
            py::tuple(vm, v)
        })
        .collect();
    Ok(py::list(vm, out))
}
fn mark(vm: &mut Vm, args: &[Value], kw: Kw, r: u8) -> PyResult<Value> {
    with_glyphs(vm, &a(args, kw, 0, "l"), |_, g| g.iter_mut().for_each(|g| g.ruby = r))?;
    Ok(Value::None)
}
fn f_place_ruby(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let ro = af(vm, args, kw, 1, "ruby_offset")?;
    let (ao, sw, sh) = if rel.at_least(7, 0) {
        (af(vm, args, kw, 2, "altruby_offset")?, af(vm, args, kw, 3, "surf_width")?, af(vm, args, kw, 4, "surf_height")?)
    } else {
        (0., af(vm, args, kw, 2, "surf_width")?, af(vm, args, kw, 3, "surf_height")?)
    };
    let r = with_glyphs(vm, &a(args, kw, 0, "glyphs"), |rel, g| place_ruby(rel, g, ro, ao, (sw, sh)))?;
    r.map_err(|e| py::error(vm, e))?;
    Ok(Value::None)
}
fn f_align_and_justify(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let w = af(vm, args, kw, 1, "width")?;
    let ta = af(vm, args, kw, 2, "text_align")?;
    let j = a(args, kw, 3, "justify");
    let j = ops::truthy(vm, &j)?;
    let (mut objs, mut gs) = (Vec::new(), Vec::new());
    let (_, lines) = read_lines(vm, rel, &a(args, kw, 0, "lines"), &mut objs, &mut gs)?;
    align_and_justify(&mut gs, &lines, w, ta, j);
    write_back(vm, rel, &objs, &gs)?;
    Ok(Value::None)
}
fn f_reverse_lines(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let (objs, gs) = glyph_list(vm, rel, &a(args, kw, 0, "glyphs"))?;
    // Reorder the objects the same way: tag each glyph with its index.
    let tagged: Vec<Glyph> = gs
        .into_iter()
        .enumerate()
        .map(|(i, mut g)| {
            g.piece = i;
            g
        })
        .collect();
    let out = reverse_lines(rel, tagged);
    let mut rv = Vec::with_capacity(out.len());
    for g in &out {
        let o = objs[g.piece].clone();
        if rel.since_pair(8, 3) && g.rtl {
            py::set(vm, &o, "rtl", Value::Bool(true))?;
        }
        rv.push(o);
    }
    Ok(py::list(vm, rv))
}
fn f_copy_splits(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let (_, src) = glyph_list(vm, rel, &a(args, kw, 0, "source"))?;
    let dest = a(args, kw, 1, "dest");
    let d = py::items(vm, &dest)?;
    if d.len() > src.len() {
        return Err(vm.exc_msg("IndexError", "list index out of range"));
    }
    with_glyphs(vm, &dest, |_, g| copy_splits(&src, g))?;
    Ok(Value::None)
}
fn f_adjust_glyph_spacing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let dx = af(vm, args, kw, 2, "dx")?;
    let dy = af(vm, args, kw, 3, "dy")?;
    let w = af(vm, args, kw, 4, "w")?;
    let h = af(vm, args, kw, 5, "h")?;
    let (objs, mut gs) = glyph_list(vm, rel, &a(args, kw, 0, "glyphs"))?;
    let lv = a(args, kw, 1, "lines");
    let lobjs = py::items(vm, &lv)?;
    let mut lines = Vec::new();
    for lo in &lobjs {
        lines.push(Line {
            y: num(vm, lo, "y")?,
            height: num(vm, lo, "height")?,
            ..Default::default()
        });
    }
    adjust_glyph_spacing(rel, &mut gs, &mut lines, dx, dy, w, h);
    write_back(vm, rel, &objs, &gs)?;
    for (lo, l) in lobjs.iter().zip(&lines) {
        py::set(vm, lo, "y", i(l.y))?;
        py::set(vm, lo, "height", i(l.height))?;
    }
    Ok(Value::None)
}
fn f_move_glyphs(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let rel = Rel::of(vm);
    let x = af(vm, args, kw, 1, "x")?;
    let y = af(vm, args, kw, 2, "y")?;
    // offset_glyphs takes C shorts before 7.6/8.1.
    let (x, y) = if !rel.since_pair(6, 1) {
        (x.trunc() as i64 as i16 as f64, y.trunc() as i64 as i16 as f64)
    } else {
        (x, y)
    };
    with_glyphs(vm, &a(args, kw, 0, "glyphs"), |_, g| move_glyphs(g, x, y))?;
    Ok(Value::None)
}
fn f_get_textshader_set(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let items = py::items(vm, &a(args, kw, 0, "glyphs"))?;
    let set = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    let mut shader: Option<Value> = Some(Value::None);
    for o in items {
        let s = py::get(vm, &o, "shader")?;
        let same = shader.as_ref().is_some_and(|p| py::same(p, &s));
        if !same {
            vm.call_method(&set, "add", vec![s.clone()])?;
            shader = Some(s);
        }
    }
    Ok(set)
}
fn f_lenticular(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let s = py::string(vm, &a(args, kw, 0, "s"))?;
    let rel = Rel::of(vm);
    let t = super::text::lenticular_bracket_ruby(&s, rel.at_least(8, 6));
    Ok(super::text::tokens_to_py(vm, &t))
}

fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let rel = Rel::of(vm);
    let mut fns: Vec<(&'static str, crysirror_python::object::NativeFn)> = vec![
        ("tokenize", super::text::f_tokenize),
        ("annotate_western", f_annotate_western),
        ("language_tailor", f_language_tailor),
        ("annotate_unicode", f_annotate_unicode),
        ("linebreak_greedy", f_linebreak_greedy),
        ("linebreak_nobreak", f_linebreak_nobreak),
        ("linebreak_debug", f_linebreak_debug),
        ("place_horizontal", f_place_horizontal),
        ("place_vertical", f_place_vertical),
        ("kerning", f_kerning),
        ("assign_times", f_assign_times),
        ("max_times", f_max_times),
        ("hyperlink_areas", f_hyperlink_areas),
        ("mark_ruby_top", |vm, a, k| mark(vm, a, k, RUBY_TOP)),
        ("mark_ruby_bottom", |vm, a, k| mark(vm, a, k, RUBY_BOTTOM)),
        ("place_ruby", f_place_ruby),
        ("align_and_justify", f_align_and_justify),
        ("reverse_lines", f_reverse_lines),
    ];
    if rel.at_least(6, 99) {
        fns.push(("linebreak_list", f_linebreak_list));
        fns.push(("copy_splits", f_copy_splits));
    }
    if rel.at_least(7, 0) {
        fns.push(("mark_altruby_top", |vm, a, k| mark(vm, a, k, RUBY_ALT)));
    }
    if rel.since_pair(5, 0) {
        fns.push(("annotate_anywhere", f_annotate_anywhere));
    }
    if rel.since_pair(6, 1) {
        fns.push(("lenticular_bracket_ruby", f_lenticular));
    }
    if rel.since_pair(7, 2) {
        fns.push(("adjust_glyph_spacing", f_adjust_glyph_spacing));
        fns.push(("move_glyphs", f_move_glyphs));
    } else {
        if rel.at_least(6, 99) {
            fns.push(("tweak_glyph_spacing", f_adjust_glyph_spacing));
        }
        if rel.at_least(7, 3) {
            fns.push(("offset_glyphs", f_move_glyphs));
        }
    }
    if rel.since_pair(8, 3) {
        fns.push(("assign_index", f_assign_index));
        fns.push(("get_textshader_set", f_get_textshader_set));
    }
    let m = crysirror_python::modules::module_with(vm, MODULE, &fns);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    for (n, v) in [
        ("TEXT", super::text::TEXT),
        ("TAG", super::text::TAG),
        ("PARAGRAPH", super::text::PARAGRAPH),
        ("DISPLAYABLE", super::text::DISPLAYABLE),
    ] {
        set(vm, n, Value::Int(v));
    }
    set(vm, "MAX_WIDTH", Value::Int(MAX_WIDTH as i64));
    if rel.at_least(8, 4) {
        let d = vm.types.new_dict();
        for (k, n) in ["SPLIT_NONE", "SPLIT_BEFORE", "SPLIT_INSTEAD", "SPLIT_IGNORE"].iter().enumerate() {
            let s = vm.types.new_str(n);
            ops::setitem(vm, &d, Value::Int(k as i64), s)?;
        }
        set(vm, "split_name", d);
    }
    let g = py::class(vm, MODULE, "Glyph", &[], &[("__init__", glyph_init), ("__repr__", glyph_repr)], &[])?;
    set(vm, "Glyph", Value::Obj(g));
    let l = py::class(vm, MODULE, "Line", &[], &[("__init__", line_init), ("__repr__", line_repr)], &[])?;
    set(vm, "Line", Value::Obj(l));
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init);
}
