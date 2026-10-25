// SPDX-License-Identifier: MPL-2.0
//! Explicit rich text data and bounded, strict bracket markup.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_GRAPHEMES: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyle {
    pub fonts: Vec<String>,
    pub size: f32,
    pub color: [u8; 4],
    pub bold: bool,
}
impl TextStyle {
    pub fn new(font: impl Into<String>, size: f32) -> Self {
        Self {
            fonts: vec![font.into()],
            size,
            color: [255; 4],
            bold: false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.size.is_finite() && (1.0..=512.0).contains(&self.size),
            "rich text size"
        );
        ensure!(
            !self.fonts.is_empty()
                && self.fonts.len() <= 16
                && self
                    .fonts
                    .iter()
                    .all(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)),
            "font fallback chain"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RichNode {
    Text {
        text: String,
        style: TextStyle,
    },
    /// An unbreakable base with its reading, displayed above / to the right.
    Ruby {
        base: String,
        reading: String,
        style: TextStyle,
    },
    Icon {
        asset: String,
        size: [f32; 2],
        color: [u8; 4],
    },
    Event {
        name: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichText {
    pub nodes: Vec<RichNode>,
}
impl RichText {
    pub fn plain(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            nodes: vec![RichNode::Text {
                text: text.into(),
                style,
            }],
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.nodes.len() <= MAX_GRAPHEMES, "rich text node limit");
        let mut bytes = 0usize;
        for node in &self.nodes {
            match node {
                RichNode::Text { text, style } => {
                    style.validate()?;
                    bytes += text.len();
                }
                RichNode::Ruby {
                    base,
                    reading,
                    style,
                } => {
                    style.validate()?;
                    ensure!(
                        !base.is_empty()
                            && !reading.is_empty()
                            && !base.contains(['\n', '\r'])
                            && !reading.contains(['\n', '\r']),
                        "ruby requires a nonempty single-line base/reading"
                    );
                    bytes += base.len() + reading.len();
                }
                RichNode::Icon { asset, size, .. } => {
                    ensure!(
                        !asset.is_empty()
                            && asset.len() <= 1024
                            && !asset.starts_with('/')
                            && !asset.contains(['\\', '\0'])
                            && asset
                                .split('/')
                                .all(|x| !x.is_empty() && x != "." && x != ".."),
                        "icon logical path"
                    );
                    ensure!(
                        size.iter()
                            .all(|n| n.is_finite() && (1.0..=1024.0).contains(n)),
                        "icon size"
                    );
                }
                RichNode::Event { name } => ensure!(
                    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control),
                    "text event name"
                ),
            }
            ensure!(bytes <= MAX_TEXT_BYTES, "rich text byte limit");
        }
        ensure!(
            self.logical_text().graphemes(true).count() <= MAX_GRAPHEMES,
            "rich text grapheme limit"
        );
        Ok(())
    }
    /// Logical text for shaping / typewriter indexing; an icon occupies U+FFFC.
    pub fn logical_text(&self) -> String {
        let mut out = String::new();
        for n in &self.nodes {
            match n {
                RichNode::Text { text, .. } => out.push_str(text),
                RichNode::Ruby { base, .. } => out.push_str(base),
                RichNode::Icon { .. } => out.push('\u{fffc}'),
                RichNode::Event { .. } => {}
            }
        }
        out
    }
    /// `[b]`, `[color=#rrggbb[aa]]`, `[size=24]`, `[font=alias]`,
    /// `[ruby=reading]base[/ruby]`, `[icon=path,16,16]`, `[event=name]`.
    /// `[[` quotes `[`. Unknown, mismatched and unclosed tags are errors.
    pub fn parse(markup: &str, default: TextStyle) -> Result<Self> {
        ensure!(markup.len() <= MAX_TEXT_BYTES, "markup byte limit");
        default.validate()?;
        let mut nodes = Vec::new();
        let mut style = default;
        let mut stack: Vec<(String, TextStyle)> = Vec::new();
        let mut text = String::new();
        let mut at = 0;
        fn flush(text: &mut String, nodes: &mut Vec<RichNode>, style: &TextStyle) {
            if !text.is_empty() {
                nodes.push(RichNode::Text {
                    text: std::mem::take(text),
                    style: style.clone(),
                });
            }
        }
        while at < markup.len() {
            let rest = &markup[at..];
            if rest.starts_with("[[") {
                text.push('[');
                at += 2;
                continue;
            }
            if !rest.starts_with('[') {
                let ch = rest.chars().next().unwrap();
                text.push(ch);
                at += ch.len_utf8();
                continue;
            }
            flush(&mut text, &mut nodes, &style);
            let end = rest.find(']').context("unterminated rich text tag")?;
            let tag = &rest[1..end];
            at += end + 1;
            if let Some(close) = tag.strip_prefix('/') {
                let (name, old) = stack.pop().context("unmatched closing rich text tag")?;
                ensure!(close == name, "mismatched rich text tag");
                style = old;
                continue;
            }
            let (name, arg) = tag
                .split_once('=')
                .map_or((tag, None), |(n, a)| (n, Some(a)));
            match name {
                "b" | "color" | "size" | "font" => {
                    ensure!(stack.len() < 32, "rich text nesting limit");
                    let old = style.clone();
                    match name {
                        "b" => {
                            ensure!(arg.is_none(), "bold tag takes no argument");
                            style.bold = true;
                        }
                        "color" => {
                            let hex = arg
                                .context("color argument")?
                                .strip_prefix('#')
                                .context("color needs #rrggbb or #rrggbbaa")?;
                            ensure!(
                                matches!(hex.len(), 6 | 8) && hex.is_ascii(),
                                "color hexadecimal length"
                            );
                            let mut c = [255; 4];
                            for (i, part) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
                                c[i] = u8::from_str_radix(std::str::from_utf8(part)?, 16)
                                    .context("color hexadecimal")?;
                            }
                            style.color = c;
                        }
                        "size" => {
                            style.size = arg
                                .context("size argument")?
                                .parse()
                                .context("size number")?
                        }
                        "font" => style.fonts = vec![arg.context("font argument")?.into()],
                        _ => unreachable!(),
                    }
                    style.validate()?;
                    stack.push((name.into(), old));
                }
                "ruby" => {
                    let reading = arg.context("ruby reading")?;
                    let next = markup[at..].find("[/ruby]").context("unclosed ruby")?;
                    let base = &markup[at..at + next];
                    ensure!(
                        !base.contains('['),
                        "ruby base markup must be expressed as separate nodes"
                    );
                    nodes.push(RichNode::Ruby {
                        base: base.into(),
                        reading: reading.into(),
                        style: style.clone(),
                    });
                    at += next + 7;
                }
                "icon" => {
                    let p: Vec<_> = arg.context("icon argument")?.split(',').collect();
                    ensure!(p.len() == 3, "icon expects asset,width,height");
                    nodes.push(RichNode::Icon {
                        asset: p[0].into(),
                        size: [p[1].parse()?, p[2].parse()?],
                        color: style.color,
                    });
                }
                "event" => nodes.push(RichNode::Event {
                    name: arg.context("event argument")?.into(),
                }),
                _ => bail!("unknown rich text tag '{name}'"),
            }
            ensure!(nodes.len() <= MAX_GRAPHEMES, "rich text node limit");
        }
        flush(&mut text, &mut nodes, &style);
        ensure!(stack.is_empty(), "unclosed rich text tag");
        let out = Self { nodes };
        out.validate()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markup_styles_ruby_icons_and_strict_errors() {
        let d=RichText::parse("A[color=#ff000080][b]B[/b][/color][ruby=かな]仮名[/ruby][icon=icons/star.png,16,18][event=voice]C[[",TextStyle::new("base",20.)).unwrap();
        assert_eq!(d.logical_text(), "AB仮名\u{fffc}C[");
        assert!(
            matches!(&d.nodes[1],RichNode::Text{style,..} if style.bold&&style.color==[255,0,0,128])
        );
        for bad in [
            "[b]x",
            "[/b]",
            "[b]x[/size]",
            "[size=NaN]x[/size]",
            "[unknown]",
            "[ruby=x]a[b]b[/ruby]",
            "[icon=../secret.png,1,1]",
        ] {
            assert!(
                RichText::parse(bad, TextStyle::new("base", 20.)).is_err(),
                "{bad}"
            );
        }
    }
}
