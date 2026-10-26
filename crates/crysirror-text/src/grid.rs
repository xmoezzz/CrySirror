// SPDX-License-Identifier: MPL-2.0
//! Font-independent cell layout for terminals and fixed-cell message windows.
//! Callers supply their line-break and indentation policy. Glyph rasterization
//! and reveal clocks remain separate engine services.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellLayoutOptions {
    pub bounds: [i32; 2],
    pub size: i32,
    pub spacing: [i32; 2],
    /// ASCII and half-width katakana occupy half a cell before spacing.
    pub half_width: bool,
    /// Apply spacing before halving (useful for independently sized name runs).
    pub half_spacing: bool,
    /// Maximum extra width allowed for forbidden line-start punctuation.
    pub hanging: i32,
    pub forbidden_line_start: String,
    pub indent_pairs: Vec<[char; 2]>,
}
impl CellLayoutOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.bounds.iter().all(|v| (1..=32767).contains(v)),
            "cell layout bounds"
        );
        ensure!((1..=512).contains(&self.size), "cell layout size");
        ensure!(
            self.spacing.iter().all(|v| (-511..=4096).contains(v))
                && self.size + self.spacing[1] > 0,
            "cell layout spacing"
        );
        ensure!(
            (0..=512).contains(&self.hanging)
                && self.forbidden_line_start.len() <= 4096
                && self.indent_pairs.len() <= 32,
            "cell layout break policy"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellCursor {
    pub position: [i32; 2],
    pub indent: i32,
    pub opening: Option<char>,
    pub nesting: u32,
    pub line_head: bool,
}
impl CellCursor {
    pub fn at_start() -> Self {
        Self {
            line_head: true,
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellGlyph {
    pub character: char,
    pub position: [i32; 2],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellLayout {
    pub glyphs: Vec<CellGlyph>,
    /// UTF-8 byte boundary of the first scalar which did not fit.
    pub consumed: usize,
}

/// Append cells from an immutable policy and a caller-owned logical cursor.
/// Returns overflow explicitly rather than truncating the caller's text.
/// ASCII BEL is an explicit line break which suppresses hanging indentation.
pub fn layout_cells(
    text: &str,
    options: &CellLayoutOptions,
    cursor: &mut CellCursor,
) -> Result<CellLayout> {
    options.validate()?;
    ensure!(
        text.len() <= 1024 * 1024 && text.chars().count() <= 65536,
        "cell text limit"
    );
    ensure!(
        cursor.position.iter().all(|v| *v >= 0)
            && (0..=32767).contains(&cursor.indent)
            && cursor.nesting <= 65536,
        "cell cursor limit"
    );
    let newline = |state: &mut CellCursor, indented: bool| {
        state.position = [
            if indented { state.indent } else { 0 },
            state.position[1].saturating_add(options.size + options.spacing[1]),
        ];
        state.line_head = true;
    };
    let mut glyphs = Vec::new();
    for (byte, character) in text.char_indices() {
        if character == '\r' {
            continue;
        }
        if character == '\n' || character == '\u{7}' {
            newline(cursor, character == '\n');
            continue;
        }
        let half = options.half_width
            && (character.is_ascii() || ('\u{ff61}'..='\u{ff9f}').contains(&character));
        let cell = if half {
            (options.size / 2).max(1)
        } else {
            options.size
        };
        let advance = if half && options.half_spacing {
            (options.size + options.spacing[0]) / 2
        } else {
            cell + options.spacing[0]
        };
        let next = cursor.position[0].saturating_add(advance);
        let wrap = next > options.bounds[0].saturating_add(options.hanging)
            || (next > options.bounds[0] && !options.forbidden_line_start.contains(character));
        if wrap {
            newline(cursor, true);
            if character == ' ' || character == '\u{3000}' {
                continue;
            }
        }
        if cursor.position[1] >= options.bounds[1] {
            return Ok(CellLayout {
                glyphs,
                consumed: byte,
            });
        }
        glyphs.push(CellGlyph {
            character,
            position: cursor.position,
        });
        cursor.position[0] = cursor.position[0].saturating_add(advance.max(1));
        if let Some(pair) = options
            .indent_pairs
            .iter()
            .find(|pair| pair[0] == character)
        {
            if cursor.line_head {
                cursor.indent = cursor.position[0];
                cursor.opening = Some(pair[0]);
                cursor.nesting = 1;
            } else if cursor.opening == Some(pair[0]) {
                cursor.nesting = cursor.nesting.saturating_add(1);
            }
        }
        if cursor.opening.is_some_and(|open| {
            options
                .indent_pairs
                .iter()
                .any(|pair| pair == &[open, character])
        }) {
            cursor.nesting = cursor.nesting.saturating_sub(1);
            if cursor.nesting == 0 {
                cursor.indent = 0;
                cursor.opening = None;
            }
        }
        cursor.line_head = false;
    }
    Ok(CellLayout {
        glyphs,
        consumed: text.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> CellLayoutOptions {
        CellLayoutOptions {
            bounds: [39, 60],
            size: 20,
            spacing: [-1, 10],
            half_width: true,
            half_spacing: false,
            hanging: 20,
            forbidden_line_start: "。、」".into(),
            indent_pairs: vec![['「', '」']],
        }
    }
    #[test]
    fn half_cells_hanging_and_append_share_the_same_cursor() {
        let mut cursor = CellCursor::at_start();
        let a = layout_cells("Aあ。", &options(), &mut cursor).unwrap();
        assert_eq!(
            a.glyphs.iter().map(|g| g.position).collect::<Vec<_>>(),
            [[0, 0], [9, 0], [28, 0]]
        );
        let b = layout_cells("う", &options(), &mut cursor).unwrap();
        assert_eq!(b.glyphs[0].position, [0, 30]);
    }
    #[test]
    fn nested_hanging_indent_and_explicit_breaks() {
        let mut cursor = CellCursor::at_start();
        let result = layout_cells(
            "「あ\nい」\u{7}う",
            &CellLayoutOptions {
                bounds: [100, 100],
                ..options()
            },
            &mut cursor,
        )
        .unwrap();
        assert_eq!(result.glyphs[2].position, [19, 30]);
        assert_eq!(result.glyphs[4].position, [0, 60]);
        assert_eq!(cursor.indent, 0);
    }
    #[test]
    fn overflow_is_a_valid_utf8_suffix_and_does_not_emit_that_scalar() {
        let mut cursor = CellCursor::at_start();
        let input = "あいうえお";
        let result = layout_cells(
            input,
            &CellLayoutOptions {
                bounds: [39, 30],
                ..options()
            },
            &mut cursor,
        )
        .unwrap();
        assert_eq!(result.consumed, 6);
        assert_eq!(&input[result.consumed..], "うえお");
        assert_eq!(result.glyphs.len(), 2);
    }
}
