// SPDX-License-Identifier: MPL-2.0
//! Plain native message semantics translated to shared cell layout and layers.
//! No font rasterizer, typewriter clock, surface or renderer lives here.
use crate::runtime::{CommandContext, ProcKind, globals::MwndGlyphState};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use crysirror_scene::layer::{
    Blend, GlyphReveal, LayerCommand, LayerSpec, Paint, PositionedGlyph, SpriteTransform,
};
use crysirror_text::grid::{CellCursor, CellGlyph, CellLayoutOptions, layout_cells};
use serde::{Deserialize, Serialize};
pub(crate) mod buttons;
mod config;
mod decoration;
mod frame;
mod metadata;
mod ops;
pub(crate) use config::initialize as initialize_profile;
pub(crate) use frame::project as project_frame;
pub(crate) use ops::dispatch;
pub(crate) use ops::{after_input, stock_flag};

pub const MESSAGE_LAYER: u32 = 0x9000_0000;
pub const NAME_LAYER: u32 = MESSAGE_LAYER + 1;

/// Explicit native-to-project resource mapping for the plain horizontal MWND
/// slice. This is not an automatic conversion of arbitrary Gameexe windows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageWindow {
    pub font: String,
    pub rect: [i32; 4],
    pub name_rect: [i32; 4],
    pub size: u16,
    pub name_size: u16,
    pub spacing: [i32; 2],
    pub color: [u8; 4],
    pub name_color: [u8; 4],
}
impl MessageWindow {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.font.is_empty()
                && self.font.len() <= 256
                && !self.font.chars().any(char::is_control),
            "native message font alias"
        );
        for rect in [self.rect, self.name_rect] {
            ensure!(
                rect[..2].iter().all(|n| (-32767..=32767).contains(n))
                    && rect[2..].iter().all(|n| (1..=32767).contains(n)),
                "native message rectangle"
            );
        }
        ensure!((1..=512).contains(&self.name_size), "native name size");
        self.layout(false).validate()?;
        self.layout(true).validate()?;
        Ok(())
    }
    fn layout(&self, name: bool) -> CellLayoutOptions {
        let rect = if name { self.name_rect } else { self.rect };
        CellLayoutOptions {
            bounds: [rect[2], rect[3]],
            size: i32::from(if name { self.name_size } else { self.size }),
            spacing: if name {
                [self.spacing[0], 0]
            } else {
                self.spacing
            },
            half_width: true,
            half_spacing: name,
            hanging: if name { 0 } else { i32::from(self.size) },
            // Policy data from the pinned native ABI; algorithm belongs to Text.
            forbidden_line_start: if name {
                String::new()
            } else {
                "。、！？：；」』）】〕〉》］｝ー～…‥・ゝゞヽヾ々ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ!?:;%)〕]>}'\"°′.,".into()
            },
            indent_pairs: if name {
                vec![]
            } else {
                vec![['「', '」'], ['『', '』'], ['（', '）']]
            },
        }
    }
}
fn supported_target(ctx: &CommandContext) -> Result<()> {
    use crate::runtime::forms::codes;
    ensure!(
        ctx.globals.current_mwnd_stage_idx == 1
            && ctx.globals.current_mwnd_no.unwrap_or(0) == 0
            && ctx.globals.current_mwnd_element
                == [
                    codes::ELM_GLOBAL_FRONT,
                    codes::ELM_STAGE_MWND,
                    codes::ELM_ARRAY,
                    0
                ],
        "native message profile supports FRONT.MWND[0] only"
    );
    Ok(())
}
fn plain(text: &str, name: bool) -> Result<()> {
    ensure!(
        text.len() <= 65536 && text.chars().count() <= 16384,
        "native message limit"
    );
    ensure!(
        !text.contains('＄')
            && (!name || !text.contains('#'))
            && text
                .chars()
                .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\u{7}')),
        "native message markup/emoji is not supported by the plain MWND profile"
    );
    Ok(())
}
fn native_glyph(glyph: &CellGlyph, size: u16, index: usize, name: bool) -> MwndGlyphState {
    MwndGlyphState {
        ch: glyph.character,
        code: glyph.character as i32,
        x: i64::from(glyph.position[0]),
        y: i64::from(glyph.position[1]),
        size: i64::from(size),
        reveal_index: if name { 0 } else { index + 1 },
        appeared: name,
        shadow: false,
        fuchi: false,
        ..Default::default()
    }
}
fn sync_native(ctx: &mut CommandContext) {
    let Some(window) = ctx.ui.window.as_ref() else {
        return;
    };
    let family = super::runtime::forms::stage::current_stage_form_id(ctx);
    let stage = ctx.globals.stage_forms.entry(family).or_default();
    let windows = stage.mwnd_lists.entry(1).or_default();
    if windows.is_empty() {
        windows.push(Default::default());
    }
    let native = &mut windows[0];
    native.msg_text = ctx.ui.message.clone().unwrap_or_default();
    native.name_text = ctx.ui.name.clone().unwrap_or_default();
    native.glyphs = ctx
        .ui
        .glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| native_glyph(g, window.size, i, false))
        .collect();
    native.name_glyphs = ctx
        .ui
        .name_glyphs
        .iter()
        .enumerate()
        .map(|(i, g)| native_glyph(g, window.name_size, i, true))
        .collect();
    native.cursor_pos = (
        i64::from(ctx.ui.cursor.position[0]),
        i64::from(ctx.ui.cursor.position[1]),
    );
    native.indent_pos = i64::from(ctx.ui.cursor.indent);
    native.indent_moji = ctx.ui.cursor.opening;
    native.indent_count = i64::from(ctx.ui.cursor.nesting);
    native.disp_moji_cnt = ctx.ui.visible_chars as i64;
    native.msg_block_started = true;
}
fn begin_block(ctx: &mut CommandContext) {
    let family = super::runtime::forms::stage::current_stage_form_id(ctx);
    let clear = ctx
        .globals
        .stage_forms
        .get(&family)
        .and_then(|stage| stage.mwnd_lists.get(&1))
        .and_then(|windows| windows.first())
        .is_some_and(|m| m.clear_ready);
    if clear {
        ctx.ui.message = None;
        ctx.ui.name = None;
        ctx.ui.glyphs.clear();
        ctx.ui.name_glyphs.clear();
        ctx.ui.cursor = CellCursor::at_start();
        ctx.ui.visible_chars = 0;
        ctx.ui.reveal = None;
        let m = &mut ctx
            .globals
            .stage_forms
            .get_mut(&family)
            .unwrap()
            .mwnd_lists
            .get_mut(&1)
            .unwrap()[0];
        m.clear_ready = false;
        m.msg_block_started = false;
    }
    let begun = ctx
        .globals
        .stage_forms
        .get(&family)
        .and_then(|stage| stage.mwnd_lists.get(&1))
        .and_then(|windows| windows.first())
        .is_some_and(|window| window.msg_block_started);
    if !begun {
        ctx.request_message_auto_savepoint();
    }
    ops::state_mut(ctx).msg_block_started = true;
}
pub fn append(ctx: &mut CommandContext, text: &str, read_flag: i64) -> Result<bool> {
    config::initialize(ctx)?;
    let Some(window) = ctx.ui.window.as_ref() else {
        return Ok(false);
    };
    supported_target(ctx)?;
    ensure!(read_flag >= -1, "invalid native MWND read flag");
    plain(text, false)?;
    if text.is_empty() {
        return Ok(true);
    }
    ensure!(
        ctx.ui.glyphs.len() + text.chars().count() <= 16384,
        "native message page limit"
    );
    ensure!(
        ctx.ui.message.as_ref().map_or(0, String::len) + text.len() <= 65536,
        "native message text budget"
    );
    let clear = ops::state(ctx).is_some_and(|m| m.clear_ready);
    let mut cursor = if ctx.ui.message.is_none() || clear {
        CellCursor::at_start()
    } else {
        ctx.ui.cursor.clone()
    };
    let result = layout_cells(text, &window.layout(false), &mut cursor)?;
    ensure!(
        result.consumed == text.len(),
        "native message page overflow requires MWND page semantics"
    );
    begin_block(ctx);
    ctx.ui.cursor = cursor;
    ctx.ui.glyphs.extend(result.glyphs);
    ctx.ui.message.get_or_insert_default().push_str(text);
    let script = &ctx.globals.script;
    let syscom = &ctx.globals.syscom;
    let nowait = script.msg_nowait
        || syscom
            .config_int
            .get(&crate::runtime::constants::elm_value::SYSCOM_GET_MESSAGE_NOWAIT)
            .copied()
            .unwrap_or(0)
            != 0;
    let delay = if script.msg_speed >= 0 {
        script.msg_speed
    } else {
        syscom
            .config_int
            .get(&crate::runtime::constants::elm_value::SYSCOM_GET_MESSAGE_SPEED)
            .copied()
            .unwrap_or(20)
    };
    let interval_ms = if nowait { 0 } else { delay.max(0) as u64 };
    ctx.ui.reveal = Some(GlyphReveal {
        unit: Default::default(),
        interval_ms,
        rate_per_second:None,
        initial_visible: ctx.ui.visible_chars,
        elapsed_ms: 0,
        forced_complete: false,
        paused: false,
    });
    if !script.async_msg_mode {
        ctx.wait.wait_message_reveal();
        ctx.request_proc_boundary(ProcKind::MessageWait);
    }
    sync_native(ctx);
    let scene_no = ctx.current_scene_no.unwrap_or(-1);
    if !ops::state(ctx).is_some_and(|m| m.open) {
        ops::set_open(ctx, true, false)?;
    }
    let native = ops::state_mut(ctx);
    if read_flag >= 0 && scene_no >= 0 {
        native.read_flag_stock.push((scene_no, read_flag));
    }
    Ok(true)
}
pub fn name(ctx: &mut CommandContext, text: &str) -> Result<bool> {
    config::initialize(ctx)?;
    let Some(window) = ctx.ui.window.as_ref() else {
        return Ok(false);
    };
    supported_target(ctx)?;
    plain(text, true)?;
    // Native SET_NAME takes the first name in the active message block.
    if ctx.ui.name.as_deref().is_none_or(str::is_empty)
        || ops::state(ctx).is_some_and(|m| m.clear_ready)
    {
        let mut layout = window.layout(true);
        if let Some(spacing) = ctx.ui.name_spacing {
            layout.spacing = spacing;
        }
        let result = layout_cells(text, &layout, &mut CellCursor::at_start())?;
        ensure!(result.consumed == text.len(), "native name overflow");
        ensure!(
            result.glyphs.iter().all(|glyph| glyph.position[1] == 0),
            "native name does not wrap in the plain MWND profile"
        );
        begin_block(ctx);
        ctx.ui.name = (!text.is_empty()).then(|| text.into());
        ctx.ui.name_glyphs = result.glyphs;
        sync_native(ctx);
    }
    Ok(true)
}
pub fn sync_progress(
    ctx: &mut CommandContext,
    client: ClientId,
    host: &mut dyn HostServices,
) -> Result<()> {
    if ctx.ui.window.is_none() {
        return Ok(());
    }
    if !ctx.ui.published_message {
        if ctx.ui.glyphs.is_empty() && ctx.wait.message_reveal_waiting() {
            ctx.wait.finish_message_reveal();
        }
        return Ok(());
    }
    let value = host.query_engine(client, EngineQuery::Layer(MESSAGE_LAYER))?;
    let layer: LayerSpec =
        serde_json::from_slice(&value).context("native message engine layer missing")?;
    let Paint::GlyphRun { glyphs, reveal, .. } = layer.paint else {
        anyhow::bail!("native message layer no longer GlyphRun")
    };
    ensure!(
        glyphs.len() == ctx.ui.glyphs.len(),
        "native message layer changed outside its VM"
    );
    let visible = reveal
        .as_ref()
        .map_or(glyphs.len(), |r| r.visible_count(glyphs.len()));
    ctx.ui.visible_chars = visible;
    ctx.ui.reveal = reveal;
    if visible == ctx.ui.glyphs.len() && ctx.wait.message_reveal_waiting() {
        ctx.wait.finish_message_reveal();
    }
    sync_native(ctx);
    Ok(())
}

pub fn validate(ctx: &CommandContext) -> Result<()> {
    let ui = &ctx.ui;
    if let Some(m) = ops::state(ctx) {
        if let Some((x, y)) = m.window_pos {
            ensure!(
                (-32767..=32767).contains(&x) && (-32767..=32767).contains(&y),
                "native frame position"
            );
        }
        if let Some((w, h)) = m.window_size {
            ensure!(
                (1..=32767).contains(&w) && (1..=32767).contains(&h),
                "native frame size"
            );
        }
        ensure!(
            m.waku_file.len() <= 4096 && m.filter_file.len() <= 4096,
            "native frame filename budget"
        );
    }
    if let Some(tween) = &ui.window_tween {
        tween.validate()?;
        ensure!(
            (0. ..=1.).contains(&tween.start)
                && (0. ..=1.).contains(&tween.end)
                && (0..=5).contains(&ui.window_animation_type),
            "native MWND animation snapshot state"
        );
    }
    if let Some(window) = &ui.window {
        window.validate()?;
        ensure!(
            ui.glyphs.len() <= 16384 && ui.name_glyphs.len() <= 16384,
            "native message glyph budget"
        );
        ensure!(
            ui.visible_chars <= ui.glyphs.len(),
            "native message visible prefix"
        );
        ensure!(
            ui.message.as_ref().is_none_or(|text| text.len() <= 65536)
                && ui.name.as_ref().is_none_or(|text| text.len() <= 65536),
            "native message text budget"
        );
        ensure!(
            ui.glyphs
                .iter()
                .chain(ui.name_glyphs.iter())
                .all(|glyph| !glyph.character.is_control()
                    && glyph.position.iter().all(|v| (0..=65536).contains(v))),
            "native message cell data"
        );
        if let Some(reveal) = &ui.reveal {
            reveal.validate(ui.glyphs.len())?;
        }
    }
    Ok(())
}
pub fn complete_on_decide(
    ctx: &mut CommandContext,
    client: ClientId,
    host: &mut dyn HostServices,
) -> Result<bool> {
    if ctx.ui.window.is_some() && ctx.wait.message_key_waiting() {
        let keep_voice = ctx
            .globals
            .syscom
            .config_int
            .get(&crate::runtime::forms::codes::syscom_op::GET_KOE_DONT_STOP_ONOFF)
            .copied()
            .unwrap_or(0)
            != 0
            || ctx.globals.script.koe_dont_stop_on_flag;
        if !keep_voice || ctx.globals.script.koe_dont_stop_off_flag {
            ctx.koe.stop(None)?;
        }
        ctx.wait.finish_message_key_wait();
        return Ok(true);
    }
    if !ctx.wait.message_reveal_waiting() || ctx.ui.window.is_none() || !ctx.ui.published_message {
        return Ok(false);
    }
    let batch = crysirror_scene::ServiceBatch {
        layers: vec![LayerCommand::CompleteReveal { id: MESSAGE_LAYER }],
        ..Default::default()
    };
    host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)?;
    sync_progress(ctx, client, host)?;
    Ok(true)
}
pub fn project(ctx: &CommandContext, host: &dyn HostServices) -> Result<Vec<LayerSpec>> {
    let Some(window) = ctx.ui.window.as_ref() else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    let (offset, opacity) = ops::visual(ctx);
    for (name, cells, id, rect, size, color) in [
        (
            false,
            &ctx.ui.glyphs,
            MESSAGE_LAYER,
            window.rect,
            window.size,
            window.color,
        ),
        (
            true,
            &ctx.ui.name_glyphs,
            NAME_LAYER,
            window.name_rect,
            window.name_size,
            window.name_color,
        ),
    ] {
        if cells.is_empty() {
            continue;
        }
        let ascent = host
            .font_metrics(&window.font, f32::from(size), 'A')?
            .ascent;
        let glyphs = cells
            .iter()
            .map(|cell| PositionedGlyph {
                character: cell.character,
                baseline: [cell.position[0] as f32, cell.position[1] as f32 + ascent],
                size: None,
                color: None,
                bold: None,
            })
            .collect();
        let body = LayerSpec {
            id,
            parent: None,
            rect: [
                rect[0] as f32 + offset[0],
                rect[1] as f32 + offset[1],
                rect[2] as f32,
                rect[3] as f32,
            ],
            paint: Paint::GlyphRun {
                font: window.font.clone(),
                size: f32::from(size),
                color,
                bold: false,
                glyphs,
                reveal: if name { None } else { ctx.ui.reveal.clone() },
                blur: None,
            },
            transform: SpriteTransform::default(),
            order: [
                i32::try_from(ctx.tables.mwnd_render.order)?,
                i32::try_from(
                    ctx.tables
                        .mwnd_render
                        .moji_layer_rep
                        .checked_add(i64::from(name))
                        .context("native glyph order overflow")?,
                )?,
            ],
            clip: None,
            opacity,
            visible: opacity > 0,
            blend: Blend::Normal,
            uv: None,
            sampling: Default::default(),
            filter: None,
        };
        out.extend(decoration::layers(ctx, name, body)?);
    }
    Ok(out)
}

/// Native MWND presentation policy shared by its text/frame/embedded resources.
pub(crate) fn embedded_window_visual(ctx: &CommandContext) -> ([f32; 2], u8) {
    ops::visual(ctx)
}

pub(crate) fn query_metadata(
    ctx: &mut CommandContext,
    window: usize,
    op: i32,
    args: &[crate::runtime::Value],
) -> Result<bool> {
    metadata::query(ctx, window, op, args)
}
