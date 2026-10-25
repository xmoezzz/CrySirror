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
pub fn publish(
    ctx: &mut CommandContext,
    client: ClientId,
    old: &mut BTreeSet<u32>,
    host: &mut dyn HostServices,
) -> Result<()> {
    ensure!(
        ctx.globals.wipe.is_none(),
        "native wipe rendering is not supported by the layer bridge"
    );
    ensure!(
        ctx.ui.message.is_none() && ctx.ui.name.is_none(),
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
        if let Some(objects) = stage.object_lists.get(&1) {
            for (index, obj) in objects.iter().enumerate() {
                if !obj.used || obj.object_type == 0 {
                    continue;
                }
                let b = &obj.base;
                ensure!(
                    obj.runtime.child_objects.is_empty() && !obj.button.enabled,
                    "native child/button rendering unsupported"
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
                    blend: [Blend::Normal, Blend::Add, Blend::Subtract, Blend::Multiply, Blend::Screen][b.blend as usize],
                    uv: None,
                    filter: object_filter(b),
                });
            }
        }
    }
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
    let mut batch = ServiceBatch::default();
    batch.layers.extend(
        old.difference(&ids)
            .map(|id| LayerCommand::Remove { id: *id }),
    );
    batch
        .layers
        .extend(layers.into_iter().map(|layer| LayerCommand::Put { layer }));
    if !batch.layers.is_empty() {
        host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)?;
    }
    *old = ids;
    Ok(())
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
fn object_filter(b: &crate::runtime::globals::ObjectBaseState) -> Option<crysirror_scene::layer::LayerFilter> {
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
    let identity = f.mono == 0. && f.invert == 0. && f.bright == 0. && f.dark == 0. && f.color_rate == 0. && f.add == [0.; 3];
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
        assert!(object_filter(&b).is_none(), "default object needs no filter");
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
