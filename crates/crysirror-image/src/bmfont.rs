// SPDX-License-Identifier: MPL-2.0
//! Metrics and coverage planes for a single-page, channel-packed BMFont text descriptor.
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub texel: [u32; 4],
    pub offset: [i32; 2],
    pub advance: i32,
    pub plane: usize,
}

pub struct PackedFont {
    pub page: String,
    pub dimensions: [u32; 2],
    pub size: u32,
    pub base: i32,
    pub line_height: u32,
    glyphs: HashMap<char, Glyph>,
    kern: HashMap<(char, char), i32>,
}

impl PackedFont {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 1 << 20, "BMFont descriptor size");
        let mut font = Self {
            page: String::new(),
            dimensions: [0; 2],
            size: 0,
            base: 0,
            line_height: 0,
            glyphs: HashMap::new(),
            kern: HashMap::new(),
        };
        let mut char_count = None;
        let mut kern_count = None;
        for line in text.lines() {
            let mut words = line.split_ascii_whitespace();
            let Some(kind) = words.next() else { continue };
            let fields: HashMap<_, _> = words.filter_map(|word| word.split_once('=')).collect();
            let field = |key: &str| -> Result<&str> {
                Ok(fields
                    .get(key)
                    .copied()
                    .with_context(|| format!("BMFont {kind}.{key}"))?)
            };
            let n = |key: &str| -> Result<u32> { Ok(field(key)?.parse()?) };
            let number = |key: &str| -> Result<i32> { Ok(field(key)?.parse()?) };
            let scalar =
                |key: &str| -> Result<char> { char::from_u32(n(key)?).context("BMFont scalar") };
            match kind {
                "info" => font.size = n("size")?,
                "common" => {
                    ensure!(
                        n("pages")? == 1 && n("packed")? == 1,
                        "BMFont requires one packed page"
                    );
                    font.base = number("base")?;
                    font.line_height = n("lineHeight")?;
                    font.dimensions = [n("scaleW")?, n("scaleH")?];
                }
                "page" => {
                    ensure!(
                        n("id")? == 0 && font.page.is_empty(),
                        "unexpected BMFont page"
                    );
                    font.page = field("file")?.trim_matches('"').to_owned();
                }
                "chars" => char_count = Some(n("count")? as usize),
                "kernings" => kern_count = Some(n("count")? as usize),
                "char" => {
                    ensure!(n("page")? == 0, "unexpected BMFont glyph page");
                    let ch = scalar("id")?;
                    let channel = n("chnl")?;
                    ensure!(
                        matches!(channel, 1 | 2 | 4 | 8),
                        "invalid BMFont coverage plane"
                    );
                    let g = Glyph {
                        texel: [n("x")?, n("y")?, n("width")?, n("height")?],
                        offset: [number("xoffset")?, number("yoffset")?],
                        advance: number("xadvance")?,
                        plane: channel.trailing_zeros() as usize,
                    };
                    ensure!(
                        font.glyphs.insert(ch, g).is_none(),
                        "duplicate BMFont character"
                    );
                }
                "kerning" => {
                    let pair = (scalar("first")?, scalar("second")?);
                    ensure!(
                        font.kern.insert(pair, number("amount")?).is_none(),
                        "duplicate BMFont pair"
                    );
                }
                _ => {}
            }
        }
        ensure!(
            font.size > 0
                && font.line_height > 0
                && !font.page.is_empty()
                && font.dimensions.iter().all(|v| (1..=4096).contains(v)),
            "invalid BMFont metrics"
        );
        ensure!(
            Some(font.glyphs.len()) == char_count && kern_count.unwrap_or(0) == font.kern.len(),
            "BMFont record count"
        );
        for g in font.glyphs.values() {
            let [x, y, w, h] = g.texel;
            ensure!(
                x.checked_add(w).is_some_and(|n| n <= font.dimensions[0])
                    && y.checked_add(h).is_some_and(|n| n <= font.dimensions[1]),
                "BMFont glyph outside page"
            );
        }
        Ok(font)
    }

    pub fn glyph(&self, ch: char) -> Option<Glyph> {
        self.glyphs.get(&ch).copied()
    }
    pub fn kerning(&self, left: char, right: char) -> i32 {
        self.kern.get(&(left, right)).copied().unwrap_or(0)
    }

    /// Return the four coverage planes vertically stacked as a white RGBA
    /// image. `channels` maps descriptor bit order (1,2,4,8) to input RGBA.
    pub fn coverage_atlas(
        &self,
        rgba: &[u8],
        channels: [usize; 4],
        bottom_up: bool,
    ) -> Result<Vec<u8>> {
        let [width, height] = self.dimensions.map(|n| n as usize);
        let pixels = width * height;
        ensure!(
            rgba.len() == pixels * 4 && channels.iter().all(|i| *i < 4),
            "BMFont source image"
        );
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(pixels * 16)?;
        for channel in channels {
            for y in 0..height {
                let row = if bottom_up { height - 1 - y } else { y };
                for p in rgba[row * width * 4..(row + 1) * width * 4].chunks_exact(4) {
                    bytes.extend_from_slice(&[255, 255, 255, p[channel]]);
                }
            }
        }
        Ok(bytes)
    }
}
