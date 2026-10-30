// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native frame resource and origin policy on engine Image/Solid/Filter layers.
use super::*;
pub const WAKU_LAYER: u32 = MESSAGE_LAYER + 2;
pub const FILTER_LAYER: u32 = MESSAGE_LAYER + 3;
pub(crate) fn project(
    ctx: &mut CommandContext,
    client: ClientId,
    host: &mut dyn HostServices,
) -> Result<Vec<LayerSpec>> {
    let Some(m) = ops::state(ctx).cloned() else {
        return Ok(vec![]);
    };
    if m.waku_file.is_empty() && m.filter_file.is_empty() && m.filter_color.is_none() {
        return Ok(vec![]);
    }
    let (x, y) = m.window_pos.context("native MWND frame window position")?;
    let (w, h) = m.window_size.context("native MWND frame window size")?;
    ensure!(
        w > 0 && h > 0 && m.waku_extend_type == 0,
        "native extended MWND frame layout is not mapped"
    );
    let (offset, alpha) = ops::visual(ctx);
    let margin = m.filter_margin.unwrap_or((0, 0, 0, 0));
    let filter_pos = [
        x.checked_add(margin.0)
            .context("native filter X overflow")?,
        y.checked_add(margin.1)
            .context("native filter Y overflow")?,
    ];
    let filter_size = [
        w.checked_sub(margin.0)
            .and_then(|n| n.checked_sub(margin.2))
            .context("native filter width overflow")?
            .max(1),
        h.checked_sub(margin.1)
            .and_then(|n| n.checked_sub(margin.3))
            .context("native filter height overflow")?
            .max(1),
    ];
    let cfg = &ctx.globals.syscom.config_int;
    let configured = |key, default| cfg.get(&key).copied().unwrap_or(default).clamp(0, 255) as u8;
    let color = m.filter_color.unwrap_or((0, 0, 255, 128));
    let tr = if m.filter_config_tr {
        configured(93, 128)
    } else if !m.filter_file.is_empty() {
        255
    } else {
        color.3
    };
    let add = if m.filter_config_color {
        [configured(84, 0), configured(91, 0), configured(92, 0)]
    } else {
        [0; 3]
    };
    let mut out = vec![];
    for (id, file, pos, bounds, tint, opacity, order) in [
        (WAKU_LAYER, m.waku_file, [x, y], [w, h], [255; 4], alpha, 1),
        (
            FILTER_LAYER,
            m.filter_file,
            filter_pos,
            filter_size,
            [color.0, color.1, color.2, 255],
            ((u16::from(alpha) * u16::from(tr)) / 255) as u8,
            0,
        ),
    ] {
        if id == WAKU_LAYER && file.is_empty() {
            continue;
        }
        let mut rect = [
            pos[0] as f32 + offset[0],
            pos[1] as f32 + offset[1],
            bounds[0] as f32,
            bounds[1] as f32,
        ];
        let paint = if file.is_empty() {
            Paint::Solid { color: tint }
        } else {
            let image = ctx.images.load_g00(&file, 0)?;
            let bitmap = &ctx.images.images[image];
            rect[0] -= bitmap.center_x as f32;
            rect[1] -= bitmap.center_y as f32;
            rect[2] = bitmap.width as f32;
            rect[3] = bitmap.height as f32;
            let asset = if let Some(path) = ctx.images.published.get(&image) {
                path.clone()
            } else {
                let path = host.upload_rgba(
                    client,
                    u32::try_from(image)?,
                    bitmap.width,
                    bitmap.height,
                    &bitmap.rgba,
                )?;
                let path = path.relative().to_owned();
                ctx.images.published.insert(image, path.clone());
                path
            };
            Paint::Image {
                asset,
                tint: [255; 4],
            }
        };
        let native_order = if order == 1 {
            ctx.tables.mwnd_render.waku_layer_rep
        } else {
            ctx.tables.mwnd_render.filter_layer_rep
        };
        out.push(LayerSpec {
            id,
            parent: None,
            rect,
            paint,
            transform: Default::default(),
            order: [
                i32::try_from(ctx.tables.mwnd_render.order)?,
                i32::try_from(native_order)?,
            ],
            clip: None,
            opacity,
            visible: opacity > 0,
            blend: Blend::Normal,
            uv: None,
            sampling: Default::default(),
            filter: if id == FILTER_LAYER && add != [0; 3] {
                Some(crysirror_scene::layer::LayerFilter {
                    add: add.map(|n| f32::from(n) / 255.),
                    mono: 0.,
                    invert: 0.,
                    bright: 0.,
                    dark: 0.,
                    color_rate: 0.,
                    color: [0.; 3],
                    tone_curve: None,
                    mask: None,
                    alpha_test: false,
                    distortion: None,
                })
            } else {
                None
            },
        });
    }
    Ok(out)
}
