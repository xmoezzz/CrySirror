// SPDX-License-Identifier: MPL-2.0
//! Shaped rich layout. UTF-8 clusters and logical grapheme ranges survive bidi.
use crate::{
    rich::{RichNode, RichText, TextStyle},
    TextSystem,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};
use unicode_bidi::{BidiInfo, Level};
use unicode_linebreak::{linebreaks, BreakOpportunity};
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WritingMode {
    #[default]
    Horizontal,
    VerticalRl,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseDirection {
    #[default]
    Auto,
    Ltr,
    Rtl,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutOptions {
    pub canvas: [u32; 2],
    pub writing: WritingMode,
    pub direction: BaseDirection,
    pub line_height: f32,
    pub ruby_scale: f32,
    pub ruby_gap: f32,
    pub language: Option<String>,
    pub alignment: TextAlign,
}
impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            canvas: [640, 480],
            writing: WritingMode::Horizontal,
            direction: BaseDirection::Auto,
            line_height: 1.3,
            ruby_scale: 0.5,
            ruby_gap: 2.,
            language: None,
            alignment: TextAlign::Start,
        }
    }
}
impl LayoutOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.canvas.iter().all(|n| *n > 0 && *n <= 8192)
                && u64::from(self.canvas[0]) * u64::from(self.canvas[1]) <= 4 * 1024 * 1024,
            "rich text canvas limit"
        );
        ensure!(
            self.line_height.is_finite()
                && (1.0..=8.).contains(&self.line_height)
                && self.ruby_scale.is_finite()
                && (0.1..=1.).contains(&self.ruby_scale)
                && self.ruby_gap.is_finite()
                && (0.0..=128.).contains(&self.ruby_gap),
            "layout spacing/ruby parameters"
        );
        if let Some(language) = &self.language {
            ensure!(
                !language.is_empty() && language.len() <= 64,
                "layout language"
            );
            let _: rustybuzz::Language = language
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid shaping language"))?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct GlyphPlacement {
    pub font: String,
    /// Reject stale placement data after an alias is replaced or face changes.
    pub font_digest: String,
    pub glyph_id: u16,
    pub size: f32,
    pub position: [f32; 2],
    pub advance: [f32; 2],
    pub color: [u8; 4],
    pub bold: bool,
    pub bytes: Range<usize>,
    pub graphemes: Range<usize>,
    pub ruby: bool,
    /// Whitespace may have a valid glyph ID with no outline.
    pub whitespace: bool,
}
#[derive(Clone, Debug)]
pub struct IconPlacement {
    pub asset: String,
    pub rect: [f32; 4],
    pub color: [u8; 4],
    pub bytes: Range<usize>,
    pub graphemes: Range<usize>,
}
#[derive(Clone, Debug)]
pub struct LayoutLine {
    pub bytes: Range<usize>,
    pub advance: f32,
    pub cross_extent: f32,
    pub overflow: bool,
}
#[derive(Clone, Debug)]
pub struct Layout {
    pub canvas: [u32; 2],
    pub writing: WritingMode,
    pub text: String,
    pub glyphs: Vec<GlyphPlacement>,
    pub icons: Vec<IconPlacement>,
    pub lines: Vec<LayoutLine>,
    pub grapheme_count: usize,
    pub clipped: bool,
}
impl Layout {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.canvas.iter().all(|n| *n > 0 && *n <= 8192)
                && u64::from(self.canvas[0]) * u64::from(self.canvas[1]) <= 4 * 1024 * 1024,
            "rich layout canvas"
        );
        ensure!(
            self.text.len() <= crate::rich::MAX_TEXT_BYTES
                && self.grapheme_count == self.text.graphemes(true).count()
                && self.grapheme_count <= crate::rich::MAX_GRAPHEMES
                && self.glyphs.len() <= 1024 * 1024,
            "rich layout text/glyph bounds"
        );
        let range = |b: &Range<usize>, g: &Range<usize>| {
            b.start < b.end
                && b.end <= self.text.len()
                && self.text.is_char_boundary(b.start)
                && self.text.is_char_boundary(b.end)
                && g.start < g.end
                && g.end <= self.grapheme_count
        };
        for g in &self.glyphs {
            ensure!(
                range(&g.bytes, &g.graphemes)
                    && g.size.is_finite()
                    && (1.0..=512.).contains(&g.size)
                    && g.position
                        .iter()
                        .chain(g.advance.iter())
                        .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
                "rich glyph metrics"
            );
        }
        for i in &self.icons {
            ensure!(
                range(&i.bytes, &i.graphemes)
                    && i.rect
                        .iter()
                        .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
                    && i.rect[2] > 0.
                    && i.rect[3] > 0.,
                "rich icon metrics"
            );
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Span {
    bytes: Range<usize>,
    node: usize,
}
#[derive(Default)]
struct Shaped {
    glyphs: Vec<GlyphPlacement>,
    icons: Vec<IconPlacement>,
    advance: f32,
    ascent: f32,
    size: f32,
    ruby_extra: f32,
}

fn ignorably_missing(c: char) -> bool {
    c.is_control()
        || matches!(c,'\u{00ad}'|'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{206f}'|'\u{fe00}'..='\u{fe0f}'|'\u{e0100}'..='\u{e01ef}')
}
fn script(text: &str) -> Script {
    text.chars()
        .map(|c| c.script())
        .find(|s| !matches!(s, Script::Common | Script::Inherited | Script::Unknown))
        .unwrap_or(Script::Common)
}
fn grapheme_range(boundaries: &[usize], r: Range<usize>) -> Range<usize> {
    boundaries
        .partition_point(|b| *b <= r.start)
        .saturating_sub(1)..boundaries.partition_point(|b| *b < r.end)
}
fn allowed_boundary(text: &str, at: usize) -> bool {
    if at == 0 || at == text.len() {
        return true;
    }
    let before = text[..at].chars().next_back().unwrap();
    let after = text[at..].chars().next().unwrap();
    if matches!(before, '\u{a0}' | '\u{202f}' | '\u{2060}')
        || matches!(after, '\u{a0}' | '\u{202f}' | '\u{2060}')
    {
        return false;
    }
    // A documented strict subset of JIS X 4051 prohibition classes, on top of
    // UAX #14. Never place a closing mark at start or an opener at line end.
    !"（〔［｛〈《「『【〘〖〝‘“([{«".contains(before)&&
    !"、。，．・：；？！‼⁇⁈⁉ーゝゞ々ヽヾ〻゛゜）〕］｝〉》」』】〙〗〟’”)]}»ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ".contains(after)
}
/// UAX #14 opportunities plus CJK prohibition tailoring. Widths correspond to
/// extended graphemes. Over-wide unbreakable ruby/marks remain together.
pub fn wrap_ranges(
    text: &str,
    widths: &[f32],
    limit: f32,
    unbreakable: &[Range<usize>],
) -> Result<Vec<Range<usize>>> {
    let gs = text.grapheme_indices(true).collect::<Vec<_>>();
    ensure!(
        widths.len() == gs.len()
            && limit.is_finite()
            && limit > 0.
            && widths.iter().all(|w| w.is_finite() && *w >= 0.),
        "line metrics"
    );
    let ends = gs.iter().map(|(at, g)| at + g.len()).collect::<Vec<_>>();
    let breaks: BTreeMap<_, _> = linebreaks(text).collect();
    let can = |at: usize| {
        allowed_boundary(text, at) && !unbreakable.iter().any(|r| r.start < at && at < r.end)
    };
    let mut lines = Vec::new();
    let mut start = 0;
    while start < gs.len() {
        let mut width = 0.;
        let mut last = None;
        let mut i = start;
        loop {
            if i == gs.len() {
                lines.push(gs[start].0..text.len());
                start = i;
                break;
            }
            let mandatory = gs[i].1.contains(['\n', '\r', '\u{2028}', '\u{2029}']);
            if mandatory {
                lines.push(gs[start].0..ends[i]);
                start = i + 1;
                break;
            }
            if width + widths[i] > limit && i > start {
                let mut end = last.unwrap_or(i);
                while end < gs.len() && !can(gs[end].0) {
                    end += 1;
                }
                if end > start {
                    lines.push(
                        gs[start].0..if end == gs.len() {
                            text.len()
                        } else {
                            gs[end].0
                        },
                    );
                    start = end;
                    break;
                }
            }
            width += widths[i];
            if can(ends[i])
                && matches!(
                    breaks.get(&ends[i]),
                    Some(BreakOpportunity::Allowed | BreakOpportunity::Mandatory)
                )
            {
                last = Some(i + 1);
            }
            i += 1;
        }
    }
    if text.is_empty() || text.ends_with(['\n', '\r', '\u{2028}', '\u{2029}']) {
        lines.push(text.len()..text.len());
    }
    Ok(lines)
}

impl TextSystem {
    fn choose_font<'a>(&self, text: &str, style: &'a TextStyle) -> Result<&'a str> {
        for alias in &style.fonts {
            let (bytes,index) = self.font_source(alias)?;
            let f = rustybuzz::Face::from_slice(&bytes,index).context("shaping face")?;
            if text
                .chars()
                .all(|c| ignorably_missing(c) || f.glyph_index(c).is_some())
            {
                return Ok(alias);
            }
        }
        anyhow::bail!(
            "no font in fallback chain {:?} covers grapheme {:?}",
            style.fonts,
            text
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn shape_text(
        &self,
        text: &str,
        range: Range<usize>,
        context: Range<usize>,
        style: &TextStyle,
        rtl: bool,
        options: &LayoutOptions,
        boundaries: &[usize],
    ) -> Result<Shaped> {
        let mut groups: Vec<(Range<usize>, String, Script)> = Vec::new();
        let mut previous_script = Script::Common;
        for (at, g) in text[range.clone()].grapheme_indices(true) {
            let at = range.start + at;
            let font = self.choose_font(g, style)?.to_string();
            let mut s = script(g);
            if s == Script::Common {
                s = previous_script
            } else {
                previous_script = s;
            }
            if let Some((r, f, old)) = groups.last_mut() {
                if *f == font && (*old == s || s == Script::Common) {
                    r.end = at + g.len();
                    continue;
                }
            }
            groups.push((at..at + g.len(), font, s));
        }
        if rtl && options.writing == WritingMode::Horizontal {
            groups.reverse();
        }
        let mut shaped = Shaped {
            size: style.size,
            ..Default::default()
        };
        for (range, font, s) in groups {
            let (bytes,index) = self.font_source(&font)?;
            let face = rustybuzz::Face::from_slice(&bytes,index).context("shaping face")?;
            let scale = style.size / face.units_per_em() as f32;
            shaped.ascent = shaped.ascent.max(face.ascender() as f32 * scale);
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(&text[range.clone()]);
            buffer.set_pre_context(&text[context.start..range.start]);
            buffer.set_post_context(&text[range.end..context.end]);
            buffer.guess_segment_properties();
            if s != Script::Common {
                buffer.set_script(
                    s.short_name()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("unknown script tag"))?,
                );
            }
            buffer.set_direction(if options.writing == WritingMode::VerticalRl {
                rustybuzz::Direction::TopToBottom
            } else if rtl {
                rustybuzz::Direction::RightToLeft
            } else {
                rustybuzz::Direction::LeftToRight
            });
            buffer.set_cluster_level(rustybuzz::BufferClusterLevel::MonotoneGraphemes);
            if let Some(lang) = &options.language {
                buffer.set_language(
                    lang.parse()
                        .map_err(|_| anyhow::anyhow!("invalid language"))?,
                );
            }
            let output = rustybuzz::shape(&face, &[], buffer);
            let mut clusters = output
                .glyph_infos()
                .iter()
                .map(|g| range.start + g.cluster as usize)
                .collect::<BTreeSet<_>>();
            clusters.insert(range.end);
            for (info, pos) in output.glyph_infos().iter().zip(output.glyph_positions()) {
                let start = range.start + info.cluster as usize;
                let end = *clusters
                    .range(start + 1..)
                    .next()
                    .context("glyph cluster end")?;
                if text[start..end].chars().all(ignorably_missing) {
                    continue;
                }
                ensure!(
                    info.glyph_id != 0 || text[start..end].chars().all(ignorably_missing),
                    "shaper produced missing glyph in {font}"
                );
                let advance = if options.writing == WritingMode::Horizontal {
                    pos.x_advance as f32 * scale
                } else {
                    -pos.y_advance as f32 * scale
                };
                let offset = if options.writing == WritingMode::Horizontal {
                    [
                        shaped.advance + pos.x_offset as f32 * scale,
                        -pos.y_offset as f32 * scale,
                    ]
                } else {
                    [
                        pos.x_offset as f32 * scale,
                        shaped.advance - pos.y_offset as f32 * scale,
                    ]
                };
                shaped.glyphs.push(GlyphPlacement {
                    font: font.clone(),
                    font_digest: self.font_digest(&font)?,
                    glyph_id: info.glyph_id as u16,
                    size: style.size,
                    position: offset,
                    advance: if options.writing == WritingMode::Horizontal {
                        [advance, 0.]
                    } else {
                        [0., advance]
                    },
                    color: style.color,
                    bold: style.bold,
                    bytes: start..end,
                    graphemes: grapheme_range(boundaries, start..end),
                    ruby: false,
                    whitespace: text[start..end].chars().all(char::is_whitespace),
                });
                shaped.advance += advance.max(0.);
            }
        }
        Ok(shaped)
    }
    #[allow(clippy::too_many_arguments)]
    fn shape_line(
        &self,
        document: &RichText,
        text: &str,
        spans: &[Span],
        range: Range<usize>,
        bidi: &BidiInfo<'_>,
        options: &LayoutOptions,
        boundaries: &[usize],
    ) -> Result<Shaped> {
        let Some(para) = bidi
            .paragraphs
            .iter()
            .find(|p| p.range.start <= range.start && range.start < p.range.end)
        else {
            return Ok(Shaped::default());
        };
        let actual = range.start..range.end.min(para.range.end);
        let (levels, runs) = bidi.visual_runs(para, actual.clone());
        let mut result = Shaped::default();
        for run in runs {
            let rtl = levels[run.start].is_rtl();
            let mut matching = spans
                .iter()
                .filter(|s| s.bytes.start < run.end && run.start < s.bytes.end)
                .collect::<Vec<_>>();
            if rtl && options.writing == WritingMode::Horizontal {
                matching.reverse();
            }
            for span in matching {
                let sub = span.bytes.start.max(run.start)..span.bytes.end.min(run.end);
                let node = &document.nodes[span.node];
                let mut part = match node {
                    RichNode::Text { style, .. } => {
                        self.shape_text(text, sub, actual.clone(), style, rtl, options, boundaries)?
                    }
                    RichNode::Ruby { reading, style, .. } => {
                        ensure!(
                            sub == span.bytes,
                            "ruby base crosses directional runs; split the ruby node"
                        );
                        let mut base = self.shape_text(
                            text,
                            sub.clone(),
                            actual.clone(),
                            style,
                            rtl,
                            options,
                            boundaries,
                        )?;
                        let small = TextStyle {
                            size: (style.size * options.ruby_scale).max(1.),
                            ..style.clone()
                        };
                        let reading_boundaries = reading
                            .grapheme_indices(true)
                            .map(|(i, _)| i)
                            .chain(std::iter::once(reading.len()))
                            .collect::<Vec<_>>();
                        let mut ruby = self.shape_text(
                            reading,
                            0..reading.len(),
                            0..reading.len(),
                            &small,
                            rtl,
                            options,
                            &reading_boundaries,
                        )?;
                        let advance = base.advance.max(ruby.advance);
                        let axis = if options.writing == WritingMode::Horizontal {
                            0
                        } else {
                            1
                        };
                        for g in &mut base.glyphs {
                            g.position[axis] += (advance - base.advance) * 0.5;
                            g.graphemes = grapheme_range(boundaries, sub.clone());
                        }
                        for g in &mut ruby.glyphs {
                            g.position[axis] += (advance - ruby.advance) * 0.5;
                            if options.writing == WritingMode::Horizontal {
                                g.position[1] -= base.ascent + options.ruby_gap + small.size * 0.25;
                            } else {
                                g.position[0] +=
                                    style.size * 0.6 + small.size * 0.5 + options.ruby_gap;
                            }
                            g.ruby = true;
                            g.bytes = sub.clone();
                            g.graphemes = grapheme_range(boundaries, sub.clone());
                        }
                        base.glyphs.extend(ruby.glyphs);
                        base.advance = advance;
                        base.ruby_extra = small.size + options.ruby_gap;
                        base
                    }
                    RichNode::Icon { asset, size, color } => Shaped {
                        icons: vec![IconPlacement {
                            asset: asset.clone(),
                            rect: if options.writing == WritingMode::Horizontal {
                                [0., -size[1], size[0], size[1]]
                            } else {
                                [-size[0] * 0.5, 0., size[0], size[1]]
                            },
                            color: *color,
                            bytes: sub.clone(),
                            graphemes: grapheme_range(boundaries, sub),
                        }],
                        advance: if options.writing == WritingMode::Horizontal {
                            size[0]
                        } else {
                            size[1]
                        },
                        ascent: size[1],
                        size: size[0].max(size[1]),
                        ..Default::default()
                    },
                    RichNode::Event { .. } => unreachable!(),
                };
                let axis = if options.writing == WritingMode::Horizontal {
                    0
                } else {
                    1
                };
                for g in &mut part.glyphs {
                    g.position[axis] += result.advance;
                }
                for i in &mut part.icons {
                    i.rect[axis] += result.advance;
                }
                result.advance += part.advance;
                result.ascent = result.ascent.max(part.ascent);
                result.size = result.size.max(part.size);
                result.ruby_extra = result.ruby_extra.max(part.ruby_extra);
                result.glyphs.extend(part.glyphs);
                result.icons.extend(part.icons);
            }
        }
        Ok(result)
    }
    pub fn layout_rich(&self, document: &RichText, options: &LayoutOptions) -> Result<Layout> {
        document.validate()?;
        options.validate()?;
        let text = document.logical_text();
        let boundaries = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        let mut spans = Vec::new();
        let mut offset = 0;
        let mut unbreakable = Vec::new();
        for (node, n) in document.nodes.iter().enumerate() {
            let len = match n {
                RichNode::Text { text, .. } => text.len(),
                RichNode::Ruby { base, .. } => base.len(),
                RichNode::Icon { .. } => 3,
                RichNode::Event { .. } => 0,
            };
            if len > 0 {
                ensure!(
                    boundaries.binary_search(&offset).is_ok()
                        && boundaries.binary_search(&(offset + len)).is_ok(),
                    "rich node boundary splits an extended grapheme"
                );
                let range = offset..offset + len;
                if matches!(n, RichNode::Ruby { .. }) {
                    unbreakable.push(range.clone());
                }
                spans.push(Span { bytes: range, node });
                offset += len;
            }
        }
        let level = match options.direction {
            BaseDirection::Auto => None,
            BaseDirection::Ltr => Some(Level::ltr()),
            BaseDirection::Rtl => Some(Level::rtl()),
        };
        let bidi = BidiInfo::new(&text, level);
        let mut widths = vec![0.; boundaries.len().saturating_sub(1)];
        for para in &bidi.paragraphs {
            let shaped = self.shape_line(
                document,
                &text,
                &spans,
                para.range.clone(),
                &bidi,
                options,
                &boundaries,
            )?;
            for g in shaped.glyphs.iter().filter(|g| !g.ruby) {
                if g.graphemes.end > g.graphemes.start + 1 {
                    unbreakable.push(g.bytes.clone());
                }
                if let Some(w) = widths.get_mut(g.graphemes.start) {
                    *w += g.advance[0].abs() + g.advance[1].abs();
                }
            }
            for i in shaped.icons {
                if let Some(w) = widths.get_mut(i.graphemes.start) {
                    *w += if options.writing == WritingMode::Horizontal {
                        i.rect[2]
                    } else {
                        i.rect[3]
                    };
                }
            }
            // Ruby's reading may be wider than the base, but it stays atomic.
            for span in spans
                .iter()
                .filter(|s| matches!(document.nodes[s.node], RichNode::Ruby { .. }))
            {
                if span.bytes.start >= para.range.start && span.bytes.end <= para.range.end {
                    let ruby = self.shape_line(
                        document,
                        &text,
                        &spans,
                        span.bytes.clone(),
                        &bidi,
                        options,
                        &boundaries,
                    )?;
                    let gs = grapheme_range(&boundaries, span.bytes.clone());
                    for w in &mut widths[gs.clone()] {
                        *w = 0.;
                    }
                    widths[gs.start] = ruby.advance;
                }
            }
        }
        let limit = if options.writing == WritingMode::Horizontal {
            options.canvas[0]
        } else {
            options.canvas[1]
        } as f32;
        let ranges = wrap_ranges(&text, &widths, limit, &unbreakable)?;
        let mut output = Layout {
            canvas: options.canvas,
            writing: options.writing,
            text: text.clone(),
            glyphs: Vec::new(),
            icons: Vec::new(),
            lines: Vec::new(),
            grapheme_count: widths.len(),
            clipped: false,
        };
        let mut cross = 0.;
        for range in ranges {
            let mut shaped = self.shape_line(
                document,
                &text,
                &spans,
                range.clone(),
                &bidi,
                options,
                &boundaries,
            )?;
            let size = shaped.size.max(1.);
            let thickness = size * options.line_height + shaped.ruby_extra;
            let remaining = (limit - shaped.advance).max(0.);
            let rtl = options.writing == WritingMode::Horizontal
                && bidi
                    .paragraphs
                    .iter()
                    .find(|p| p.range.start <= range.start && range.start < p.range.end)
                    .is_some_and(|p| p.level.is_rtl());
            let start = match options.alignment {
                TextAlign::Center => remaining * 0.5,
                TextAlign::Start => {
                    if rtl {
                        remaining
                    } else {
                        0.
                    }
                }
                TextAlign::End => {
                    if rtl {
                        0.
                    } else {
                        remaining
                    }
                }
            };
            let origin = if options.writing == WritingMode::Horizontal {
                [start, cross + shaped.ascent + shaped.ruby_extra]
            } else {
                [
                    options.canvas[0] as f32
                        - cross
                        - size * options.line_height * 0.5
                        - shaped.ruby_extra,
                    start,
                ]
            };
            for g in &mut shaped.glyphs {
                g.position[0] += origin[0];
                g.position[1] += origin[1];
            }
            for i in &mut shaped.icons {
                i.rect[0] += origin[0];
                i.rect[1] += origin[1];
            }
            let overflow = shaped.advance > limit + 0.1;
            output.clipped |= overflow
                || cross + thickness
                    > if options.writing == WritingMode::Horizontal {
                        options.canvas[1]
                    } else {
                        options.canvas[0]
                    } as f32;
            output.lines.push(LayoutLine {
                bytes: range,
                advance: shaped.advance,
                cross_extent: thickness,
                overflow,
            });
            output.glyphs.extend(shaped.glyphs);
            output.icons.extend(shaped.icons);
            cross += thickness;
        }
        output.validate()?;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uax_breaks_cjk_prohibitions_newlines_and_atomic_ruby() {
        let t = "甲（乙）丙丁";
        let r = wrap_ranges(t, &[10.; 6], 20., &[]).unwrap();
        let lines = r.iter().map(|r| &t[r.clone()]).collect::<Vec<_>>();
        assert_eq!(lines, ["甲", "（乙）", "丙丁"]);
        assert_eq!(
            wrap_ranges("aa bb", &[5.; 5], 16., &[]).unwrap(),
            [0..3, 3..5]
        );
        assert_eq!(
            wrap_ranges("A\nB", &[5., 0., 5.], 20., &[]).unwrap(),
            [0..2, 2..3]
        );
        let atomic = std::iter::once(0..2).collect::<Vec<_>>();
        assert_eq!(
            wrap_ranges("ABCD", &[30., 0., 5., 5.], 10., &atomic).unwrap(),
            [0..2, 2..4]
        );
        assert!(wrap_ranges("a", &[f32::NAN], 10., &[]).is_err());
    }
    #[test]
    fn bidi_visual_order_keeps_logical_byte_indices() {
        let t = "אבג abc";
        let b = BidiInfo::new(t, None);
        let (l, r) = b.visual_runs(&b.paragraphs[0], 0..t.len());
        assert!(r.len() >= 2);
        assert_eq!(&t[r[0].clone()], "abc");
        assert!(!l[r[0].start].is_rtl());
        assert!(l[r.last().unwrap().start].is_rtl());
    }
}
