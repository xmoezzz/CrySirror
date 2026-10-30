// SPDX-License-Identifier: MPL-2.0
//! Native object policy -> checked engine layers. No renderer/device ownership.
use crate::runtime::CommandContext;
use anyhow::{Context, Result, ensure};
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_scene::{
    ServiceBatch,
    layer::{Blend, LayerCommand, LayerSpec, Paint, SpriteTransform},
};
use std::collections::BTreeSet;
#[path = "export_tree.rs"]
mod tree;
pub fn publish(
    ctx: &mut CommandContext,
    client: ClientId,
    old: &mut BTreeSet<u32>,
    old_widgets: &mut BTreeSet<u32>,
    movie_pictures: &crate::adapter::object_movie::Pictures,
    host: &mut dyn HostServices,
) -> Result<()> {
    ensure!(
        ctx.globals.wipe.is_none(),
        "native wipe rendering is not supported by the layer bridge"
    );
    ensure!(
        ctx.ui.window.is_some() || (ctx.ui.message.is_none() && ctx.ui.name.is_none()),
        "native message layout must be implemented before CD_TEXT/CD_NAME can render"
    );
    ensure!(
        ctx.globals.stage_forms.len() <= 1,
        "native multi-stage namespace composition unsupported"
    );
    let mut layers = Vec::new();
    let mut needed = Vec::new();
    let mut ids = BTreeSet::new();
    for stage in ctx.globals.stage_forms.values() {
        let mut out = tree::Output {
            layers: &mut layers,
            images: &mut needed,
            ids: &mut ids,
            next: 0x8100_0000,
            movies: None,
            path: Vec::new(),
        };
        if let Some(windows) = stage.mwnd_lists.get(&1) {
            for (index, window) in windows.iter().enumerate() {
                ensure!(
                    window
                        .face_list
                        .iter()
                        .all(|o| o.object_type == 0 && o.runtime.child_objects.is_empty()),
                    "native MWND face/button mapping required"
                );
                if window
                    .object_list
                    .iter()
                    .all(|o| o.object_type == 0 && o.runtime.child_objects.is_empty())
                {
                    continue;
                }
                ensure!(index == 0, "native embedded MWND supports window zero only");
                let visible = window.open || ctx.ui.window_tween.is_some();
                ensure!(
                    !visible
                        || (window.window_pos.is_some()
                            && window.window_size.is_some()
                            && ctx.ui.window.is_some()),
                    "native visible embedded MWND requires configured geometry"
                );
                let pos = window.window_pos.unwrap_or((0, 0));
                let (offset, alpha) = if ctx.ui.window.is_some() {
                    crate::message::embedded_window_visual(ctx)
                } else {
                    ([0., 0.], 0)
                };
                let order = [
                    i32::try_from(if window.order <= 0 {
                        ctx.tables.mwnd_render.order.max(1)
                    } else {
                        window.order
                    })?,
                    i32::try_from(window.layer)?,
                ];
                let parent = out.window_group(
                    [pos.0 as f32 + offset[0], pos.1 as f32 + offset[1], 1., 1.],
                    if visible { alpha } else { 0 },
                    order,
                )?;
                for object in &window.object_list {
                    out.object(&mut ctx.images, object, Some(parent), order, 0)?;
                }
            }
        }
        if let Some(objects) = stage.object_lists.get(&1) {
            for (index, obj) in objects.iter().enumerate() {
                if (!obj.used || obj.object_type == 0) && obj.runtime.child_objects.is_empty() {
                    continue;
                }
                if !obj.runtime.child_objects.is_empty() || obj.object_type == 9 {
                    tree::Output {
                        layers: &mut layers,
                        images: &mut needed,
                        ids: &mut ids,
                        next: 0x8200_0000 + (index as u32) * 4096,
                        movies: Some(movie_pictures),
                        path: vec![index],
                    }
                    .object(&mut ctx.images, obj, None, [0, 0], 0)?;
                    continue;
                }
                let b = &effective_base(obj)?;
                ensure!(
                    !obj.button.enabled,
                    "native button rendering requires shared UI mapping"
                );
                ensure!(
                    b.world == -1
                        && b.clip_use == 0
                        && b.src_clip_use == 0
                        && b.mask_no == -1
                        && b.tonecurve_no == -1
                        && b.light_no == -1
                        && b.fog_use == 0
                        && (0..=4).contains(&b.blend)
                        && b.alpha_blend == 1,
                    "unsupported native object visual state at slot {index}"
                );
                let id = 0x8000_0000u32 + index as u32;
                ids.insert(id);
                let (paint, width, height, cx, cy, image) = match obj.object_type {
                    1 => {
                        let p = &obj.rect_param;
                        let c = p.color_argb;
                        let rgba = [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8];
                        (
                            Paint::Solid { color: rgba },
                            p.left.abs_diff(p.right).clamp(1, 32767) as u32,
                            p.top.abs_diff(p.bottom).clamp(1, 32767) as u32,
                            0,
                            0,
                            None,
                        )
                    }
                    2 => {
                        let file = obj
                            .file_name
                            .as_deref()
                            .context("native PCT name missing")?;
                        let image = ctx
                            .images
                            .load_g00(file, usize::try_from(b.patno).context("negative PATNO")?)?;
                        let img = &ctx.images.images[image];
                        (
                            Paint::Image {
                                asset: "__crysirror/pending".into(),
                                tint: [255; 4],
                            },
                            img.width,
                            img.height,
                            img.center_x,
                            img.center_y,
                            Some(image),
                        )
                    }
                    other => anyhow::bail!("unsupported native object type {other}"),
                };
                let transform = SpriteTransform {
                    // The native collector adds center_rep to the anchor after
                    // configuring the sprite. XY also include it in the pivot;
                    // configure_sprite_3d resets the Z pivot to center_z alone.
                    z: b.z as f32 + b.center_rep_z as f32,
                    scale_x: b.scale_x as f32 / 1000.,
                    scale_y: b.scale_y as f32 / 1000.,
                    scale_z: b.scale_z as f32 / 1000.,
                    rotate: b.rotate_z as f32 * std::f32::consts::PI / 1800.,
                    rotate_x: b.rotate_x as f32 * std::f32::consts::PI / 1800.,
                    rotate_y: b.rotate_y as f32 * std::f32::consts::PI / 1800.,
                    pivot_x: (b.center_x + b.center_rep_x) as f32,
                    pivot_y: (b.center_y + b.center_rep_y) as f32,
                    pivot_z: b.center_z as f32,
                    object_anchor: true,
                    texture_center_x: cx as f32,
                    texture_center_y: cy as f32,
                    culling: b.culling != 0,
                    ..Default::default()
                };
                needed.push(image);
                layers.push(LayerSpec {
                    id,
                    parent: None,
                    rect: [
                        native_anchor_component(b.x, b.center_rep_x),
                        native_anchor_component(b.y, b.center_rep_y),
                        width as f32,
                        height as f32,
                    ],
                    paint,
                    transform,
                    order: [
                        i32::try_from(b.order).context("native order overflow")?,
                        i32::try_from(b.layer).context("native layer overflow")?,
                    ],
                    clip: None,
                    opacity: ((b.alpha.clamp(0, 255) * b.tr.clamp(0, 255)) / 255) as u8,
                    visible: b.disp != 0,
                    blend: [
                        Blend::Normal,
                        Blend::Add,
                        Blend::Subtract,
                        Blend::Multiply,
                        Blend::Screen,
                    ][b.blend as usize],
                    uv: None,
                    sampling: Default::default(),
                    filter: object_filter(b),
                });
            }
        }
    }
    // Shared text layout/metrics validate before any visible scene mutation.
    let mut message_layers = crate::message::project(ctx, host)?;
    message_layers.extend(crate::message::project_frame(ctx, client, host)?);
    ids.extend(message_layers.iter().map(|layer| layer.id));
    // Complete decoding/validation before any visible scene mutation.
    for (layer, image) in layers.iter_mut().zip(needed) {
        if let Some(image) = image {
            let path = if let Some(path) = ctx.images.published.get(&image) {
                path.clone()
            } else {
                let img = &ctx.images.images[image];
                let path = host.upload_rgba(
                    client,
                    u32::try_from(image)?,
                    img.width,
                    img.height,
                    &img.rgba,
                )?;
                ctx.images.published.insert(image, path.relative().into());
                path.relative().to_owned()
            };
            if let Paint::Image { asset, .. } = &mut layer.paint {
                *asset = path;
            }
        }
    }
    layers.extend(message_layers);
    if ctx.globals.mov.playing && ctx.movie.ready {
        const MOV_LAYER: u32 = 0xA000_0000;
        let mov = &ctx.globals.mov;
        ids.insert(MOV_LAYER);
        layers.push(LayerSpec {
            id: MOV_LAYER,
            parent: None,
            rect: [
                mov.x as f32,
                mov.y as f32,
                mov.width as f32,
                mov.height as f32,
            ],
            paint: Paint::Image {
                asset: ctx
                    .movie
                    .surface
                    .clone()
                    .context("native MOV surface missing")?,
                tint: [255; 4],
            },
            transform: SpriteTransform::default(),
            order: [i32::MAX - 16, 0],
            clip: None,
            opacity: 255,
            visible: true,
            blend: Blend::Normal,
            uv: None,
            sampling: Default::default(),
            filter: None,
        });
    }
    let (button_layers, mut button_widgets, widget_ids) =
        crate::message::buttons::project(ctx, client, host)?;
    ids.extend(button_layers.iter().map(|l| l.id));
    layers.extend(button_layers);
    let mut batch = ServiceBatch::default();
    batch.ui.extend(
        old_widgets
            .difference(&widget_ids)
            .map(|id| crysirror_scene::ui::UiCommand::Remove { id: *id }),
    );
    batch.ui.append(&mut button_widgets);
    batch.layers.extend(
        old.iter()
            // Tree ids are assigned in preorder. Remove descendants first:
            // shared Layer removal also removes its still-attached subtree.
            .rev()
            .filter(|id| !ids.contains(*id))
            .map(|id| LayerCommand::Remove { id: *id }),
    );
    batch
        .layers
        .extend(layers.into_iter().map(|layer| LayerCommand::Put { layer }));
    if !batch.layers.is_empty() || !batch.ui.is_empty() {
        host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)?;
    }
    *old = ids;
    *old_widgets = widget_ids;
    ctx.ui.published_message = old.contains(&crate::message::MESSAGE_LAYER);
    Ok(())
}

/// Native additive/multiplicative offset-list policy; values/timing are owned
/// by the shared IntEvent implementation, matrices by the shared layer service.
fn effective_base(
    object: &crate::runtime::globals::ObjectState,
) -> Result<crate::runtime::globals::ObjectBaseState> {
    let mut b = object.base.clone();
    // Pinned native collector reads engine event totals even after completion.
    let event = &object.runtime.prop_events;
    macro_rules! value { ($($field:ident),* $(,)?) => { $(b.$field=i64::from(event.$field.get_total_value());)* }; }
    value!(
        patno,
        x,
        y,
        z,
        center_x,
        center_y,
        center_z,
        center_rep_x,
        center_rep_y,
        center_rep_z,
        scale_x,
        scale_y,
        scale_z,
        rotate_x,
        rotate_y,
        rotate_z,
        clip_left,
        clip_top,
        clip_right,
        clip_bottom,
        src_clip_left,
        src_clip_top,
        src_clip_right,
        src_clip_bottom,
        tr,
        mono,
        reverse,
        bright,
        dark,
        color_r,
        color_g,
        color_b,
        color_rate,
        color_add_r,
        color_add_g,
        color_add_b
    );
    let lists = &object.runtime.prop_event_lists;
    for (target, list) in [
        (&mut b.x, &lists.x_rep),
        (&mut b.y, &lists.y_rep),
        (&mut b.z, &lists.z_rep),
    ] {
        for value in list {
            *target = target
                .checked_add(i64::from(value.get_total_value()))
                .context("native offset sum overflow")?;
        }
    }
    for value in &lists.tr_rep {
        b.tr = b.tr.clamp(0, 255) * i64::from(value.get_total_value()).clamp(0, 255) / 255;
    }
    Ok(b)
}

/// Match apply_object_render_info_to_sprite followed by
/// finalize_object_center_rep_to_sprite in the pinned native collector.
fn native_anchor_component(position: i64, center_rep: i64) -> f32 {
    position
        .clamp(i32::MIN as i64, i32::MAX as i64)
        .saturating_add(center_rep)
        .clamp(i32::MIN as i64, i32::MAX as i64) as f32
}

/// Siglus object color effects as an engine layer filter. Units follow
/// siglus_scene_vm's render plan: every channel/rate is value / 255.
pub(crate) fn object_filter(
    b: &crate::runtime::globals::ObjectBaseState,
) -> Option<crysirror_scene::layer::LayerFilter> {
    let u = |v: i64| (v.clamp(0, 255) as f32) / 255.;
    let f = crysirror_scene::layer::LayerFilter {
        mono: u(b.mono),
        invert: u(b.reverse),
        bright: u(b.bright),
        dark: u(b.dark),
        color_rate: u(b.color_rate),
        color: [u(b.color_r), u(b.color_g), u(b.color_b)],
        add: [u(b.color_add_r), u(b.color_add_g), u(b.color_add_b)],
        tone_curve: None,
        mask: None,
        alpha_test: b.alpha_test != 0,
        distortion: None,
    };
    // Objects default to alpha test (discard alpha < 1/255). Without depth
    // writes that is invisible for every 2D blend mode (alpha 0 contributes
    // nothing), so it alone does not require the filtered path.
    let identity = f.mono == 0.
        && f.invert == 0.
        && f.bright == 0.
        && f.dark == 0.
        && f.color_rate == 0.
        && f.add == [0.; 3];
    (!identity).then_some(f)
}

#[cfg(test)]
mod filter_tests {
    use super::object_filter;
    use crate::runtime::globals::ObjectBaseState;
    #[test]
    fn siglus_object_effects_map_to_layer_filter_units() {
        let mut b = ObjectBaseState::default();
        assert_eq!(b.alpha_test, 1, "Siglus objects alpha-test by default");
        assert!(
            object_filter(&b).is_none(),
            "default object needs no filter"
        );
        b.mono = 255;
        b.reverse = 51;
        b.color_rate = 128;
        b.color_r = 255;
        b.color_add_b = 255;
        b.alpha_test = 1;
        let f = object_filter(&b).unwrap();
        assert_eq!((f.mono, f.invert, f.alpha_test), (1., 0.2, true));
        assert!((f.color_rate - 128. / 255.).abs() < 1e-6);
        assert_eq!((f.color, f.add), ([1., 0., 0.], [0., 0., 1.]));
        f.validate().unwrap();
        // Out-of-range native values clamp to Siglus's 0..255 domain.
        b.bright = 1000;
        assert_eq!(object_filter(&b).unwrap().bright, 1.);
    }
}
