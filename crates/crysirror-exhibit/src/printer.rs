// SPDX-License-Identifier: MPL-2.0
//! V1 ordinary character-say layout and hit-wait marker.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    service::{FontLibrary, FontMetrics, InputFrame, buttons},
    state::ClientId,
};
use crysirror_resource::text_stream::decode_shift_jis;
use serde_json::{Value, json};

use crate::v1_printer_page::{BODY_SLOT, NAME_SLOT};
const BORDER_PX: i32 = 1;
const MAX_ROW_BYTES: usize = 1024;

fn cannot_start_line(c: char) -> bool {
    // Retouch's default CP932 line-start table (0x106410) includes a
    // subset of its engine-defined symbols in the user-defined area.
    "。」』】・）]｝－―ー‥！？、～　ょっゃゅぇェャュョッ^;:)♪-_ぁぃぅぇぉゎァィゥォヵ".contains(c)
        || matches!(c as u32,
            0xe000..=0xe006 | 0xe008..=0xe00b | 0xe00d..=0xe012 |
            0xe014..=0xe017 | 0xe019 | 0xe01b..=0xe027 | 0xe02d..=0xe030 |
            0xe032 | 0xe034 | 0xe043..=0xe047 | 0xe056 | 0xe05b..=0xe067)
}

#[derive(Debug, PartialEq)]
pub(super) struct OrdinarySay {
    speaker: Option<String>,
    body: String,
}

/// The first ordinary opening form, not the range of native say variants.
#[cfg(test)]
pub(super) const FIRST_WORDS: [i32; 7] = [5, 1, 1, 1, 0, 0, 0];

#[derive(Debug, PartialEq)]
pub(super) struct Line {
    pub character: u8,
    pub ordinal: i32,
    pub wait: bool,
    pub say: OrdinarySay,
}

pub(super) const MARKER: &str = "res/g/gn/hit.gyu";
const MARKER_SLOT: u32 = 55;
pub(super) const MARKER_LAYER: u32 = 256 + MARKER_SLOT;
const MARKER_FRAME_MS: u64 = 140;
const MARKER_CELL: [i32; 2] = [14, 28];

pub(super) fn marker(origin: [i32; 2], age_ms: u64) -> Value {
    // The resident sets ten columns. Its animation advances at 20 ms and
    // each of the nine specified pictures occupies seven sequence ticks.
    let frame = (age_ms / MARKER_FRAME_MS) % 9;
    let u0 = frame as f32 / 10.;
    let u1 = (frame + 1) as f32 / 10.;
    json!({"op":"put","layer":{
        "id": MARKER_LAYER, "order": [2, MARKER_SLOT],
        "rect": [origin[0],origin[1],MARKER_CELL[0],MARKER_CELL[1]],
        "uv": [u0,0.,u1,1.], "paint":{"kind":"image","asset":MARKER}
    }})
}

pub(super) fn advances(input: InputFrame, canvas: [u32; 2]) -> bool {
    let keys = buttons::ENTER | buttons::SPACE | buttons::CTRL | buttons::PAGE_DOWN;
    input.buttons_pressed & keys != 0
        || (input.buttons_pressed & buttons::MOUSE_LEFT != 0
            && (input.pointer_x as u32) < canvas[0]
            && (input.pointer_y as u32) < canvas[1])
}

pub(super) struct Wait {
    pub origin: [i32; 2],
    pub since: u64,
    pub frame: u8,
}

impl Wait {
    pub fn update(
        &mut self,
        host: &mut dyn crysirror_core::service::HostServices,
        client: ClientId,
    ) -> Result<()> {
        let age = host.now_ms().saturating_sub(self.since);
        let frame = (age / MARKER_FRAME_MS % 9) as u8;
        if frame != self.frame {
            host.submit_engine(
                client,
                1,
                &serde_json::to_vec(&json!({
                    "layers": [marker(self.origin, age)]
                }))?,
            )?;
            self.frame = frame;
        }
        Ok(())
    }
}

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

    fn rows(&self, inline_name: bool) -> impl Iterator<Item = &str> {
        self.speaker
            .as_deref()
            .filter(move |_| inline_name)
            .into_iter()
            .chain(self.body.split('\n'))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plane {
    Inline,
    FramedBody,
    FramedName,
}

pub(super) struct PageStyle {
    pub body: Style,
    pub name: Option<Style>,
}

impl PageStyle {
    pub fn layout(&self, say: &OrdinarySay, fonts: &dyn FontLibrary) -> Result<(Value, [i32; 2])> {
        let (body, cursor) = self.body.layout(say.rows(self.name.is_none()), fonts)?;
        let mut layers = vec![body];
        if let Some(style) = &self.name {
            layers.push(style.layout(say.speaker.as_deref().into_iter(), fonts)?.0);
        }
        Ok((json!({"layers": layers}), cursor))
    }
}

#[derive(Clone, Debug)]
pub(super) struct Style {
    pub rect: [i32; 4],
    pub family: String,
    pub font_px: i32,
    pub line_px: i32,
    pub plane: Plane,
}

impl Style {
    fn layout<'a>(
        &self,
        rows: impl Iterator<Item = &'a str>,
        fonts: &dyn FontLibrary,
    ) -> Result<(Value, [i32; 2])> {
        let alias = match fonts.resolve_alias(&self.family)? {
            Some(alias) => alias,
            None => fonts
                .resolve_alias("default")?
                .context("no scoped ExHibit printer font")?,
        };
        self.build(rows, &alias, |character| {
            if !character.is_whitespace() {
                ensure!(
                    fonts.glyph_id(&alias, character)? != 0,
                    "missing ExHibit printer glyph"
                );
            }
            Ok(fonts.metrics(&alias, self.font_px as f32, character)?)
        })
    }

    /// For an inline printer 0x54da0 emits a centered
    /// name on its own line only when present and inline naming is enabled.
    /// It then submits the body with zero reveal delay.
    /// 0xaba50 wraps to a complete character boundary, moves prohibited
    /// initial characters to the next row with their predecessor, and
    /// centers only the final piece of an explicit line.
    fn build<'a>(
        &self,
        rows: impl Iterator<Item = &'a str>,
        alias: &str,
        mut measure: impl FnMut(char) -> Result<FontMetrics>,
    ) -> Result<(Value, [i32; 2])> {
        let [_, _, width, height] = self.rect;
        let outlined = self.plane == Plane::Inline;
        let margin = if outlined { 4 } else { 0 };
        ensure!(
            !alias.is_empty() && width > margin && height > 0,
            "invalid ExHibit printer destination or font binding"
        );
        let mut glyphs = Vec::new();
        let mut cursor = [self.rect[0], self.rect[1]];
        let mut top = 0;
        for text in rows {
            let mut chars = Vec::new();
            for character in text.chars() {
                let metric = measure(character)?;
                ensure!(
                    metric.ascent.is_finite()
                        && metric.descent.is_finite()
                        && metric.ascent > 0.0
                        && metric.descent <= 0.0
                        && metric.advance.is_finite()
                        && (0.0..=128.0).contains(&metric.advance),
                    "invalid ExHibit printer font metric"
                );
                chars.push((
                    character,
                    metric.advance.round() as i32,
                    metric.ascent.round() as i32,
                    (-metric.descent).round() as i32,
                ));
            }
            let mut remaining = chars.as_slice();
            loop {
                ensure!(
                    top + self.line_px <= height,
                    "unsupported ExHibit printer vertical clipping"
                );
                let mut advance = 0;
                let fitted = remaining
                    .iter()
                    .take_while(|glyph| {
                        advance += glyph.1;
                        advance + margin <= width
                    })
                    .count();
                ensure!(
                    fitted > 0 || remaining.is_empty(),
                    "ExHibit printer glyph is wider than its destination"
                );
                let complete = fitted == remaining.len() || self.plane == Plane::FramedName;
                let end = if complete {
                    fitted
                } else {
                    (1..=fitted)
                        .rev()
                        .find(|&i| !cannot_start_line(remaining[i].0))
                        .unwrap_or(fitted)
                };
                let row = &remaining[..end];
                let width_used: i32 = row.iter().map(|glyph| glyph.1).sum();
                let mut x = if complete && self.plane != Plane::FramedBody {
                    (width - width_used) / 2
                } else {
                    0
                };
                for &(character, advance, ascent, descent) in row {
                    ensure!(
                        top + ascent + descent + i32::from(outlined) * BORDER_PX <= height,
                        "unsupported ExHibit printer vertical clipping"
                    );
                    glyphs.push(json!({"character": character, "baseline": [x, top + ascent]}));
                    x += advance;
                }
                cursor = [self.rect[0] + x, self.rect[1] + top];
                top += self.line_px;
                if complete {
                    break;
                }
                remaining = &remaining[end..];
            }
        }
        let slot = if self.plane == Plane::FramedName {
            NAME_SLOT
        } else {
            BODY_SLOT
        };
        let mut paint = if glyphs.is_empty() {
            json!({"kind":"solid","color":[0,0,0,0]})
        } else {
            let intensity = if outlined { 255 } else { 0 };
            json!({"kind":"glyph_run","font":alias,"size":self.font_px,"glyphs":glyphs,
                "color":[intensity,intensity,intensity,255]})
        };
        if outlined && paint["kind"] == "glyph_run" {
            paint["outline"] = json!({"radius":BORDER_PX,"kernel":"disk","color":[0,0,0,255]});
        }
        Ok((
            json!({"op":"put","layer":{
                "id":256+slot,"order":[2,slot],"rect":self.rect,"paint":paint,
            }}),
            cursor,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::{
        Error, Result as CoreResult,
        service::{FontHandle, FontInfo, FontJob, FontLoadSpec, FontLoadState},
    };

    #[test]
    fn marker_animation_uses_native_cells_and_requires_a_fresh_advance_edge() {
        assert_eq!(
            marker([10, 20], 0)["layer"]["rect"],
            json!([10, 20, 14, 28])
        );
        assert_eq!(marker([10, 20], 139), marker([10, 20], 0));
        assert_ne!(marker([10, 20], 140), marker([10, 20], 0));
        assert_eq!(marker([10, 20], 1260), marker([10, 20], 0));
        let mut frame = InputFrame {
            buttons_down: buttons::ENTER,
            ..InputFrame::default()
        };
        assert!(!advances(frame, [800, 600]));
        frame.buttons_pressed = buttons::ENTER;
        assert!(advances(frame, [800, 600]));
        frame.buttons_pressed = buttons::MOUSE_LEFT;
        frame.pointer_x = -1;
        assert!(!advances(frame, [800, 600]));
        frame.pointer_x = 4;
        assert!(advances(frame, [800, 600]));
    }

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
            assert!(matches!(name, "Synthetic Gothic" | "default"));
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
            font_px: 26,
            line_px: 30,
            plane: Plane::Inline,
        };
        assert!(style.layout(say.rows(true), &Fonts(false)).is_err());
        let (cmd, cursor) = style.layout(say.rows(true), &Fonts(true))?;
        let layer = &cmd["layer"];
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
        assert_eq!(cursor, [435, 348]);
        assert_eq!(layer["paint"]["outline"]["kernel"], "disk");
        assert!(layer["paint"].get("reveal").is_none());
        Ok(())
    }

    #[test]
    fn narration_sentinel_puts_body_on_first_row_without_hidden_name_gap() -> Result<()> {
        let style = Style {
            rect: [32, 288, 728, 404],
            family: "Synthetic Gothic".into(),
            font_px: 26,
            line_px: 30,
            plane: Plane::Inline,
        };
        let say = OrdinarySay::decode(&[0x8b, 0x4c, 0x8f, 0x71], &[0x93, 0xfa, 0x96, 0x7b])?;
        assert!(say.speaker.is_none());
        let (cmd, _) = style.layout(say.rows(true), &Fonts(true))?;
        let glyphs = cmd["layer"]["paint"]["glyphs"].as_array().unwrap();
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
            font_px: 26,
            line_px: 30,
            plane: Plane::Inline,
        };
        assert!(
            style
                .build(say.rows(true), "fixture", |_| Ok(FontMetrics {
                    ascent: 15.0,
                    descent: -3.0,
                    advance: 12.0
                }))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn authored_large_glyphs_keep_the_independent_row_pitch() -> Result<()> {
        let say = OrdinarySay::decode(b"", b"A\nB")?;
        let style = Style {
            rect: [32, 288, 728, 404],
            family: "fixture".into(),
            font_px: 75,
            line_px: 30,
            plane: Plane::Inline,
        };
        let (commands, cursor) = style.build(say.rows(true), "fixture", |_| {
            Ok(FontMetrics {
                ascent: 63.0,
                descent: -12.0,
                advance: 75.0,
            })
        })?;
        let paint = &commands["layer"]["paint"];
        assert_eq!(paint["size"], 75);
        assert_eq!(paint["glyphs"][0]["baseline"], json!([326, 63]));
        assert_eq!(paint["glyphs"][1]["baseline"], json!([326, 93]));
        assert_eq!(cursor, [433, 318]);
        Ok(())
    }

    #[test]
    fn long_lines_wrap_before_a_forbidden_start_and_center_the_last_piece() -> Result<()> {
        let say = OrdinarySay::decode(b"", b"ABCD)E")?;
        let style = Style {
            rect: [16, 4, 52, 72],
            family: "fixture".into(),
            font_px: 14,
            line_px: 24,
            plane: Plane::Inline,
        };
        let (commands, cursor) = style.build(say.rows(true), "fixture", |_| {
            Ok(FontMetrics {
                ascent: 10.0,
                descent: -3.0,
                advance: 12.0,
            })
        })?;
        let glyphs = &commands["layer"]["paint"]["glyphs"];
        assert_eq!(glyphs[2], json!({"character":"C","baseline":[24,10]}));
        assert_eq!(glyphs[3], json!({"character":"D","baseline":[8,34]}));
        assert_eq!(glyphs[4], json!({"character":")","baseline":[20,34]}));
        assert_eq!(cursor, [60, 28]);
        Ok(())
    }

    #[test]
    fn framed_body_and_name_use_independent_origins_and_clear_the_previous_label() -> Result<()> {
        let style = PageStyle {
            body: Style {
                rect: [32, 472, 728, 100],
                family: "Synthetic Gothic".into(),
                font_px: 26,
                line_px: 30,
                plane: Plane::FramedBody,
            },
            name: Some(Style {
                rect: [16, 446, 52, 30],
                family: "Synthetic Gothic".into(),
                font_px: 26,
                line_px: 30,
                plane: Plane::FramedName,
            }),
        };
        let dialogue = OrdinarySay::decode(b"ABC", b"Line")?;
        let (commands, cursor) = style.layout(&dialogue, &Fonts(true))?;
        let body = &commands["layers"][0]["layer"];
        let name = &commands["layers"][1]["layer"];
        assert_eq!(body["id"], 308);
        assert_eq!(body["paint"]["color"], json!([0, 0, 0, 255]));
        assert_eq!(
            body["paint"]["glyphs"][0],
            json!({"character":"L","baseline":[0,21]})
        );
        assert_eq!(name["id"], 310);
        assert_eq!(name["paint"]["glyphs"].as_array().unwrap().len(), 2);
        assert_eq!(
            name["paint"]["glyphs"][0],
            json!({"character":"A","baseline":[0,21]})
        );
        assert_eq!(cursor, [136, 472]);

        let narration = OrdinarySay::decode(b"", b"Next")?;
        let (commands, _) = style.layout(&narration, &Fonts(true))?;
        assert_eq!(
            commands["layers"][1]["layer"]["paint"],
            json!({"kind":"solid","color":[0,0,0,0]})
        );
        Ok(())
    }
}
