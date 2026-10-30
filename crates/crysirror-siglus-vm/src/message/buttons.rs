// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Siglus WAKU declaration, frame and callback policy over shared resources/UI.
use super::*;
use crate::runtime::{
    forms::{codes, stage},
    globals::{ObjectState, PendingButtonAction, PendingButtonActionKind},
};
use crysirror_scene::ui::{UiCommand, WidgetSpec};
use std::collections::BTreeSet;
pub(crate) const WIDGET_BASE: u32 = 0xb1000000;
const LAYER_BASE: u32 = 0xb2000000;
pub(crate) fn native(ctx: &CommandContext) -> Option<&crate::runtime::globals::MwndState> {
    ctx.globals
        .stage_forms
        .get(&stage::current_stage_form_id(ctx))?
        .mwnd_lists
        .get(&1)?
        .first()
}
pub(crate) fn native_mut(
    ctx: &mut CommandContext,
) -> Option<&mut crate::runtime::globals::MwndState> {
    let family = stage::current_stage_form_id(ctx);
    ctx.globals
        .stage_forms
        .get_mut(&family)?
        .mwnd_lists
        .get_mut(&1)?
        .first_mut()
}
pub(super) fn initialize(ctx: &mut CommandContext, waku: usize) -> Result<()> {
    let cfg = ctx.tables.gameexe.clone().context("native WAKU config")?;
    let mut count = cfg
        .get_usize(&format!("WAKU.{waku:03}.BTN.CNT"))
        .unwrap_or(8);
    for entry in &cfg.entries {
        if entry.key_index("WAKU") == Some(waku)
            && entry.key_parts.get(2).is_some_and(|p| p == "BTN")
        {
            if let Some(index) = entry.key_parts.get(3).and_then(|p| p.parse::<usize>().ok()) {
                count = count.max(index.checked_add(1).context("native WAKU button count")?);
            }
        }
    }
    ensure!(count <= 256, "native WAKU button count budget");
    let mut objects = vec![];
    let mut layout = vec![];
    for index in 0..count {
        let entry = |name: &str| {
            cfg.entries.iter().rev().find(|entry| {
                entry.key_index("WAKU") == Some(waku)
                    && entry.key_parts.get(2).is_some_and(|part| part == "BTN")
                    && entry
                        .key_parts
                        .get(3)
                        .and_then(|part| part.parse::<usize>().ok())
                        == Some(index)
                    && entry.key_parts.get(4).is_some_and(|part| part == name)
            })
        };
        let field = |name: &str| entry(name).map(|entry| entry.value.as_str());
        let file = entry("FILE")
            .map(|entry| entry.scalar_unquoted())
            .unwrap_or("");
        let mut object = ObjectState::default();
        let pos = config::tuple(field("POS"), [0, 0, 0])?;
        ensure!(
            (0..=3).contains(&pos[0]),
            "native WAKU button position base"
        );
        layout.push((i64::from(pos[0]), i64::from(pos[1]), i64::from(pos[2])));
        if !file.is_empty() {
            ensure!(
                file.len() <= 4096 && !file.contains('?'),
                "native WAKU button filename/tonecurve profile"
            );
            let cut = config::scalar(field("CUT_NO"), 0)?;
            ensure!(cut >= 0, "native WAKU negative cut");
            ctx.images.load_g00(file, cut as usize)?;
            object.used = true;
            object.object_type = 2;
            object.file_name = Some(file.into());
            object.base.disp = 1;
            object.base.layer = ctx.tables.mwnd_render.moji_layer_rep;
            let b = &mut object.button;
            b.enabled = true;
            b.button_no = index as i64;
            b.group_no = -1;
            b.cut_no = i64::from(cut);
            b.action_no = i64::from(config::scalar(field("ACTION"), 0)?);
            b.se_no = i64::from(config::scalar(field("SE"), -1)?);
            if let Some(value) = field("TYPE") {
                let parts = value
                    .split(',')
                    .map(|p| p.trim().trim_matches('"'))
                    .collect::<Vec<_>>();
                b.sys_type = match parts[0].to_ascii_lowercase().as_str() {
                    "none" => 0,
                    "save" => 1,
                    "load" => 2,
                    "read_skip" => 3,
                    "auto_mode" => 4,
                    "return_sel" => 5,
                    "close_mwnd" => 6,
                    "msg_log" => 7,
                    "koe_play" => 8,
                    "qsave" => 9,
                    "qload" => 10,
                    "config" => 11,
                    "local_switch" => 12,
                    "local_mode" => 13,
                    "global_switch" => 14,
                    "global_mode" => 15,
                    other => anyhow::bail!("unsupported native WAKU button TYPE {other}"),
                };
                let num = |n: usize| parts.get(n).map(|p| p.parse::<i64>()).transpose();
                if matches!(b.sys_type, 3 | 4) {
                    b.mode = num(1)?.unwrap_or(0);
                } else if b.sys_type >= 9 {
                    b.sys_type_opt = num(1)?.unwrap_or(0);
                    b.mode = num(2)?.unwrap_or(0);
                }
            }
            if let Some(call) = field("CALL") {
                let parts = call
                    .split(',')
                    .map(|p| p.trim().trim_matches('"'))
                    .collect::<Vec<_>>();
                ensure!(
                    parts.len() == 2 && parts.iter().all(|s| s.len() <= 4096),
                    "native WAKU callback declaration"
                );
                b.decided_action_scn_name = parts[0].into();
                if let Ok(z) = parts[1].parse::<i64>() {
                    b.decided_action_z_no = z;
                } else {
                    b.decided_action_cmd_name = parts[1].into();
                }
            }
            if let Some(action) = field("FRAME_ACTION") {
                let parts = action
                    .split(',')
                    .map(|p| p.trim().trim_matches('"'))
                    .collect::<Vec<_>>();
                ensure!(
                    parts.len() == 2 && parts.iter().all(|s| s.len() <= 4096),
                    "native WAKU frame callback declaration"
                );
                object.frame_action.scn_name = parts[0].into();
                object.frame_action.cmd_name = parts[1].into();
                object.frame_action.end_time = -1;
                object.frame_action.counter.start();
            }
        }
        objects.push(object);
    }
    let window = super::ops::state_mut(ctx);
    window.button_list = objects;
    window.waku_button_layout = layout;
    Ok(())
}
fn read_skip_enabled(ctx: &CommandContext) -> bool {
    let sys = &ctx.globals.syscom;
    let script = &ctx.globals.script;
    let unread = script.skip_unread_message
        || sys
            .config_int
            .get(&codes::syscom_op::GET_SKIP_UNREAD_MESSAGE_ONOFF)
            .copied()
            .unwrap_or(i64::from(sys.original_config.skip_unread_message_flag))
            != 0;
    sys.read_skip.enable
        && sys.read_skip.exist
        && (unread
            || ctx
                .globals
                .read_flag(script.cur_read_flag_scn_no, script.cur_read_flag_flag_no))
}
pub(crate) fn enabled(ctx: &CommandContext, index: usize, o: &ObjectState) -> bool {
    let s = &ctx.globals.syscom;
    let feature = match o.button.sys_type {
        1 | 9 => s.save_feature.check_enabled() != 0,
        2 | 10 => s.load_feature.check_enabled() != 0,
        3 => read_skip_enabled(ctx),
        4 => s.auto_mode.check_enabled() != 0,
        5 => s.return_to_sel.check_enabled() != 0,
        6 => s.hide_mwnd.check_enabled() != 0,
        7 => s.msg_back.check_enabled() != 0,
        12 => s.local_extra_switch.check_enabled() != 0,
        13 => s.local_extra_mode.check_enabled() != 0,
        _ => true,
    };
    o.button.enabled
        && !o.button.is_disabled()
        && !s.mwnd_btn_disable_all
        && !s
            .mwnd_btn_disable
            .get(&(index as i64))
            .copied()
            .unwrap_or(false)
        && feature
}
pub(crate) fn real_state(ctx: &CommandContext, index: usize, o: &ObjectState) -> i64 {
    if !o.button.enabled {
        return o.button.state;
    }
    if !enabled(ctx, index, o) {
        return 4;
    }
    if matches!(o.button.state, 3 | 4) {
        return o.button.state;
    }
    if ctx.globals.syscom.mwnd_btn_touch_disable {
        return 0;
    }
    if o.button.pushed {
        2
    } else if o.button.hit {
        1
    } else {
        0
    }
}
fn mode_visible(ctx: &CommandContext, o: &ObjectState) -> bool {
    let s = &ctx.globals.syscom;
    let mode = match o.button.sys_type {
        3 => i64::from(s.read_skip.onoff),
        4 => i64::from(s.auto_mode.onoff),
        12 => i64::from(s.local_extra_switch.onoff),
        13 => s.local_extra_mode.value,
        _ => 0,
    };
    o.button.sys_type == 0 || o.button.mode == mode
}
fn pattern(ctx: &CommandContext, action: i64, state: i64) -> Result<[i32; 6]> {
    let mut fallback = [0, 0, 0, 255, 0, 0];
    if matches!(state, 1 | 2) {
        fallback[4] = 32;
    }
    if state == 2 {
        fallback[1] = 1;
        fallback[2] = 1;
    }
    let name = ["NORMAL", "HIT", "PUSH", "SELECT", "DISABLE"][state.clamp(0, 4) as usize];
    let raw = usize::try_from(action).ok().and_then(|action| {
        ctx.tables
            .gameexe
            .as_ref()?
            .get_indexed_field("BUTTON.ACTION", action, name)
    });
    config::tuple(raw, fallback)
}
pub(crate) fn project(
    ctx: &mut CommandContext,
    client: ClientId,
    host: &mut dyn HostServices,
) -> Result<(Vec<LayerSpec>, Vec<UiCommand>, BTreeSet<u32>)> {
    let Some(window) = native(ctx).cloned() else {
        return Ok((vec![], vec![], BTreeSet::new()));
    };
    if window.button_list.iter().all(|o| o.object_type == 0) {
        return Ok((vec![], vec![], BTreeSet::new()));
    }
    ensure!(
        ctx.ui.window.is_some(),
        "native WAKU buttons require configured message window"
    );
    let (pos, size) = window
        .window_pos
        .zip(window.window_size)
        .context("native WAKU button window geometry")?;
    let (offset, alpha) = super::ops::visual(ctx);
    let mut layers = vec![];
    let mut widgets = vec![];
    let mut ids = BTreeSet::new();
    for (index, object) in window.button_list.iter().enumerate() {
        if object.object_type == 0 {
            continue;
        }
        let b = &object.base;
        ensure!(
            object.object_type == 2
                && object.runtime.child_objects.is_empty()
                && !object.button.alpha_test
                && b.world == -1
                && b.rotate_x == 0
                && b.rotate_y == 0
                && b.rotate_z == 0
                && b.scale_x > 0
                && b.scale_y > 0
                && b.z == 0
                && b.clip_use == 0
                && b.src_clip_use == 0
                && b.mask_no == -1
                && b.tonecurve_no == -1,
            "native WAKU button requires bounded 2D rectangle interaction profile"
        );
        let active = enabled(ctx, index, object);
        let state = real_state(ctx, index, object);
        let pat = pattern(ctx, object.button.action_no, state)?;
        let cut = b
            .patno
            .checked_add(object.button.cut_no)
            .and_then(|v| v.checked_add(i64::from(pat[0])))
            .context("native button frame overflow")?
            .max(0);
        let image = ctx.images.load_g00(
            object.file_name.as_deref().context("native WAKU image")?,
            cut as usize,
        )?;
        // Native interaction uses the unmodified PCT backing. ACTION's
        // rep_pos/pattern are render-only; moving a pressed visual must not
        // move the pointer target out from under the held gesture.
        let base_cut = b
            .patno
            .checked_add(object.button.cut_no)
            .context("native hit frame overflow")?
            .max(0);
        let hit_image = ctx.images.load_g00(
            object.file_name.as_deref().context("native WAKU image")?,
            base_cut as usize,
        )?;
        let hit_bitmap = &ctx.images.images[hit_image];
        let hit_size = [hit_bitmap.width, hit_bitmap.height];
        let hit_center = [hit_bitmap.center_x, hit_bitmap.center_y];
        let bitmap = &ctx.images.images[image];
        let asset = if let Some(path) = ctx.images.published.get(&image) {
            path.clone()
        } else {
            let path = host
                .upload_rgba(
                    client,
                    u32::try_from(image)?,
                    bitmap.width,
                    bitmap.height,
                    &bitmap.rgba,
                )?
                .relative()
                .to_owned();
            ctx.images.published.insert(image, path.clone());
            path
        };
        let place = window
            .waku_button_layout
            .get(index)
            .copied()
            .unwrap_or((0, 0, 0));
        let x = pos.0
            + if place.0 == 1 || place.0 == 3 {
                size.0 - place.1
            } else {
                place.1
            };
        let y = pos.1
            + if place.0 == 2 || place.0 == 3 {
                size.1 - place.2
            } else {
                place.2
            };
        let sx = b.scale_x as f32 / 1000.;
        let sy = b.scale_y as f32 / 1000.;
        let anchor = [
            x as f32 + b.x as f32 + b.center_rep_x as f32 + offset[0] + pat[1] as f32,
            y as f32 + b.y as f32 + b.center_rep_y as f32 + offset[1] + pat[2] as f32,
        ];
        let rect = [
            anchor[0] - (bitmap.center_x as f32 + b.center_x as f32 + b.center_rep_x as f32) * sx,
            anchor[1] - (bitmap.center_y as f32 + b.center_y as f32 + b.center_rep_y as f32) * sy,
            bitmap.width as f32 * sx,
            bitmap.height as f32 * sy,
        ];
        let hit_rect = [
            anchor[0]
                - pat[1] as f32
                - (hit_center[0] as f32 + b.center_x as f32 + b.center_rep_x as f32) * sx,
            anchor[1]
                - pat[2] as f32
                - (hit_center[1] as f32 + b.center_y as f32 + b.center_rep_y as f32) * sy,
            hit_size[0] as f32 * sx,
            hit_size[1] as f32 * sy,
        ];
        let visible = alpha > 0
            && b.disp != 0
            && !ctx.globals.script.mwnd_disp_off_flag
            && !ctx.globals.syscom.hide_mwnd.onoff
            && mode_visible(ctx, object);
        let id = WIDGET_BASE + index as u32;
        ids.insert(id);
        let widget: WidgetSpec = serde_json::from_value(
            serde_json::json!({"id":id,"rect":hit_rect,"kind":"button","z":i32::try_from(b.layer.checked_mul(256).and_then(|v|v.checked_add(index as i64)).context("native WAKU interaction order")?)?,"visible":visible,"enabled":active&&window.open&&!ctx.globals.syscom.mwnd_btn_touch_disable,"interaction_only":true,"keyboard_focusable":false}),
        )?;
        widgets.push(UiCommand::Put { widget });
        let mut visual = b.clone();
        visual.bright = (visual.bright + i64::from(pat[4])).clamp(0, 255);
        visual.dark = (visual.dark + i64::from(pat[5])).clamp(0, 255);
        layers.push(LayerSpec {
            id: LAYER_BASE + index as u32,
            parent: None,
            rect,
            paint: Paint::Image {
                asset,
                tint: [255; 4],
            },
            transform: SpriteTransform::default(),
            order: [
                i32::try_from(if window.order <= 0 {
                    ctx.tables.mwnd_render.order.max(1)
                } else {
                    window.order
                })?,
                i32::try_from(window.layer + ctx.tables.mwnd_render.waku_layer_rep + b.layer)?,
            ],
            clip: None,
            opacity: ((i64::from(alpha) * b.alpha.clamp(0, 255) / 255) * b.tr.clamp(0, 255) / 255
                * i64::from(pat[3].clamp(0, 255))
                / 255) as u8,
            visible,
            blend: Blend::Normal,
            uv: None,
            sampling: Default::default(),
            filter: crate::export::object_filter(&visual),
        });
    }
    Ok((layers, widgets, ids))
}
pub(crate) fn decided(object: &ObjectState) -> Option<PendingButtonAction> {
    let b = &object.button;
    let kind = if !b.decided_action_scn_name.is_empty() {
        PendingButtonActionKind::UserCall {
            scn_name: b.decided_action_scn_name.clone(),
            cmd_name: b.decided_action_cmd_name.clone(),
            z_no: b.decided_action_z_no,
        }
    } else if b.sys_type != 0 {
        PendingButtonActionKind::Syscom {
            sys_type: b.sys_type,
            sys_type_opt: b.sys_type_opt,
            mode: b.mode,
        }
    } else {
        return None;
    };
    Some(PendingButtonAction { kind })
}
