// SPDX-License-Identifier: MPL-2.0
//! Staged v1 area-5 printer presentation. No 0x1c dispatch admission yet:
//! character voice, history, and native hit-wait must join this text stage.
//! See SATD-exhibit-rld.
use anyhow::{Context, Result, ensure};
use crysirror_core::service::{FontLibrary, FontMetrics};
use crysirror_resource::text_stream::decode_shift_jis;
use serde_json::{Value, json};

const SLOT: u32 = 52;
const LAYER: u32 = 256 + SLOT;
const FONT_PX: f32 = 26.0;
const LINE_PX: i32 = 30;
const BORDER_PX: i32 = 1;
const MAX_ROW_BYTES: usize = 1024;

pub(super) struct OrdinarySay {
    speaker: Option<String>,
    body: String,
}

/// The first ordinary opening form, not the range of native say variants.
pub(super) const FIRST_WORDS: [i32; 7] = [5, 1, 1, 1, 0, 0, 0];

impl OrdinarySay {
    pub fn decode(speaker: &[u8], body: &[u8]) -> Result<Self> {
        let speaker = decode_shift_jis(speaker, 128).context("ExHibit printer speaker encoding")?;
        let body =
            decode_shift_jis(body, MAX_ROW_BYTES).context("ExHibit printer text encoding")?;
        ensure!(
            !body.is_empty()
                && !speaker.contains('\n')
                && [&speaker, &body].into_iter().all(|t| {
                    t.chars()
                        .all(|c| (!c.is_control() || c == '\n') && c != '《' && c != '》')
                }),
            "unsupported ExHibit opening printer control or empty body"
        );
        // liteExec checks the raw narration sentinel at 0x107a04 and
        // substitutes an empty FCString. messageEngine consumes no name row.
        let speaker = (!speaker.is_empty() && speaker != "記述").then_some(speaker);
        Ok(Self { speaker, body })
    }
}

#[derive(Clone, Debug)]
pub(super) struct Style {
    pub rect: [i32; 4],
    pub family: String,
}

impl Style {
    #[allow(dead_code)] // awaits the scoped character-say transaction
    pub fn resolve_and_build(&self, say: &OrdinarySay, fonts: &dyn FontLibrary) -> Result<Value> {
        let alias = fonts
            .resolve_alias(&self.family)?
            .context("authored ExHibit printer font is not authorized")?;
        self.build(say, &alias, |character| {
            if !character.is_whitespace() {
                ensure!(
                    fonts.glyph_id(&alias, character)? != 0,
                    "missing ExHibit printer glyph"
                );
            }
            Ok(fonts.metrics(&alias, FONT_PX, character)?)
        })
    }

    /// Area 5 has no separate speaker surface. 0x54da0 emits a centered
    /// name on its own line only when present and inline naming is enabled.
    /// It then submits the body with zero reveal delay.
    /// Only rows that fit are planned: native wrap/control sequences require
    /// further renderer work. Font metrics must come from the same scoped
    /// face as the retained glyph run.
    fn build(
        &self,
        say: &OrdinarySay,
        alias: &str,
        mut measure: impl FnMut(char) -> Result<FontMetrics>,
    ) -> Result<Value> {
        let [_, _, width, height] = self.rect;
        ensure!(
            !alias.is_empty() && width > 0 && height > 0,
            "invalid ExHibit printer destination or font binding"
        );
        let mut glyphs = Vec::new();
        for (row, text) in say
            .speaker
            .as_deref()
            .into_iter()
            .chain(say.body.split('\n'))
            .enumerate()
        {
            let top = i32::try_from(row)?
                .checked_mul(LINE_PX)
                .context("ExHibit printer vertical position overflow")?;
            ensure!(
                top + LINE_PX <= height,
                "unsupported ExHibit printer vertical clipping"
            );
            let mut advance = 0_i32;
            let mut positions = Vec::new();
            for character in text.chars() {
                let metric = measure(character)?;
                ensure!(
                    metric.ascent.is_finite()
                        && metric.descent.is_finite()
                        && metric.ascent > 0.0
                        && metric.descent <= 0.0
                        && metric.advance.is_finite()
                        && (0.0..=64.0).contains(&metric.advance),
                    "invalid ExHibit printer font metric"
                );
                let a = metric.ascent.round() as i32;
                let d = (-metric.descent).round() as i32;
                ensure!(
                    a + d + BORDER_PX <= LINE_PX && top + a + d + BORDER_PX <= height,
                    "unsupported ExHibit printer vertical clipping"
                );
                positions.push((character, advance, top + a));
                advance = advance
                    .checked_add(metric.advance.round() as i32)
                    .context("ExHibit printer horizontal position overflow")?;
            }
            // Native 0x400 leaves four pixels of width when fitting a run.
            ensure!(advance + 4 <= width, "unsupported ExHibit printer wrapping");
            let x0 = (width - advance) / 2;
            glyphs.extend(
                positions.into_iter().map(
                    |(character, x, y)| json!({"character": character, "baseline": [x0 + x, y]}),
                ),
            );
        }
        Ok(json!({"layers": [{
            "op": "put", "layer": {
                "id": LAYER, "order": [2, SLOT], "rect": self.rect,
                "paint": {
                    "kind": "glyph_run", "font": alias, "size": FONT_PX,
                    "color": [255, 255, 255, 255], "glyphs": glyphs,
                    "outline": {"radius": BORDER_PX, "kernel": "disk", "color": [0, 0, 0, 255]}
                }
            }
        }]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        Error, Result as CoreResult,
        service::{FontHandle, FontInfo, FontJob, FontLoadSpec, FontLoadState},
    };

    struct Fonts(bool);
    impl FontLibrary for Fonts {
        fn begin(&self, _: FontLoadSpec) -> CoreResult<FontJob> {
            Err(Error::Unsupported("font registration"))
        }
        fn status(&self, _: FontJob) -> CoreResult<FontLoadState> {
            Err(Error::NotFound)
        }
        fn resolve(&self, _: &str) -> CoreResult<Option<FontInfo>> {
            Ok(None)
        }
        fn resolve_alias(&self, name: &str) -> CoreResult<Option<String>> {
            assert_eq!(name, "Synthetic Gothic");
            Ok(self.0.then(|| "scoped_gothic".into()))
        }
        fn glyph_id(&self, alias: &str, character: char) -> CoreResult<u32> {
            assert_eq!(alias, "scoped_gothic");
            Ok(character as u32)
        }
        fn metrics(&self, alias: &str, px: f32, character: char) -> CoreResult<FontMetrics> {
            assert_eq!(alias, "scoped_gothic");
            assert_eq!(px, 26.0);
            Ok(FontMetrics {
                ascent: 21.0,
                descent: -5.0,
                advance: if character == '語' { 14.0 } else { 26.0 },
            })
        }
        fn is_loaded(&self, alias: &str) -> CoreResult<bool> {
            Ok(self.0 && alias == "scoped_gothic")
        }
        fn release_job(&self, _: FontJob) -> CoreResult<()> {
            Err(Error::NotFound)
        }
        fn release_font(&self, _: FontHandle) -> CoreResult<()> {
            Err(Error::NotFound)
        }
    }

    #[test]
    fn synthetic_cp932_centered_speaker_and_two_lines_use_a_single_native_text_layer() -> Result<()>
    {
        let say = OrdinarySay::decode(
            &[0x93, 0xfa], // 日
            &[
                0x96, 0x7b, 0x8c, 0xea, b'\n', 0x83, 0x65, 0x83, 0x58, 0x83, 0x67,
            ],
        )?;
        let style = Style {
            rect: [32, 288, 728, 404],
            family: "Synthetic Gothic".into(),
        };
        assert!(style.resolve_and_build(&say, &Fonts(false)).is_err());
        let cmd = style.resolve_and_build(&say, &Fonts(true))?;
        let layer = &cmd["layers"][0]["layer"];
        let glyphs = &layer["paint"]["glyphs"];
        assert_eq!(layer["paint"]["font"], "scoped_gothic");
        assert_eq!(layer["id"], 308);
        assert_eq!(layer["order"], json!([2, 52]));
        assert_eq!(layer["rect"], json!([32, 288, 728, 404]));
        assert_eq!(glyphs[0], json!({"character":"日","baseline":[351,21]}));
        assert_eq!(glyphs[1], json!({"character":"本","baseline":[344,51]}));
        assert_eq!(glyphs[2], json!({"character":"語","baseline":[370,51]}));
        assert_eq!(glyphs[3], json!({"character":"テ","baseline":[325,81]}));
        assert_eq!(glyphs.as_array().unwrap().len(), 6);
        assert_eq!(layer["paint"]["outline"]["kernel"], "disk");
        assert!(layer["paint"].get("reveal").is_none());
        Ok(())
    }

    #[test]
    fn narration_sentinel_puts_body_on_first_row_without_hidden_name_gap() -> Result<()> {
        let style = Style {
            rect: [32, 288, 728, 404],
            family: "Synthetic Gothic".into(),
        };
        let say = OrdinarySay::decode(&[0x8b, 0x4c, 0x8f, 0x71], &[0x93, 0xfa, 0x96, 0x7b])?;
        assert!(say.speaker.is_none());
        let cmd = style.resolve_and_build(&say, &Fonts(true))?;
        let glyphs = cmd["layers"][0]["layer"]["paint"]["glyphs"]
            .as_array()
            .unwrap();
        assert_eq!(glyphs.len(), 2);
        assert_eq!(glyphs[0], json!({"character":"日","baseline":[338,21]}));
        assert_eq!(glyphs[1], json!({"character":"本","baseline":[364,21]}));
        Ok(())
    }

    #[test]
    fn invalid_cp932_markup_and_layout_overflow_do_not_silently_render() -> Result<()> {
        assert!(OrdinarySay::decode(b"x", b"").is_err());
        assert!(OrdinarySay::decode(b"x", &[0x82]).is_err());
        assert!(OrdinarySay::decode(b"x", &[0x81, 0x73, b'A']).is_err());
        let say = OrdinarySay::decode(b"X", b"unwrapped fixture")?;
        let style = Style {
            rect: [0, 0, 64, 30],
            family: "fixture".into(),
        };
        assert!(
            style
                .build(&say, "fixture", |_| Ok(FontMetrics {
                    ascent: 15.0,
                    descent: -3.0,
                    advance: 12.0
                }))
                .is_err()
        );
        Ok(())
    }
}
