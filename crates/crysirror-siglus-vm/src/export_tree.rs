// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original object ownership policy translated to shared retained layer trees.
//! No rasterization, transform composition or hit-testing is performed here.
use super::*;
use crate::{image_manager::ImageManager, runtime::globals::ObjectState};

pub(super) struct Output<'a> {
    pub layers: &'a mut Vec<LayerSpec>,
    pub images: &'a mut Vec<Option<usize>>,
    pub ids: &'a mut BTreeSet<u32>,
    pub next: u32,
    pub movies: Option<&'a crate::adapter::object_movie::Pictures>,
    pub path: Vec<usize>,
}
impl Output<'_> {
    fn push(&mut self, mut layer: LayerSpec, image: Option<usize>) -> Result<u32> {
        ensure!(
            self.layers.len() < 1900,
            "native layer-tree publication budget"
        );
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .context("native tree layer id overflow")?;
        layer.id = id;
        self.ids.insert(id);
        self.layers.push(layer);
        self.images.push(image);
        Ok(id)
    }
    pub(super) fn window_group(
        &mut self,
        rect: [f32; 4],
        opacity: u8,
        order: [i32; 2],
    ) -> Result<u32> {
        self.push(
            LayerSpec {
                id: 0,
                parent: None,
                rect,
                paint: Paint::Solid { color: [0; 4] },
                transform: Default::default(),
                order,
                clip: None,
                opacity,
                visible: opacity > 0,
                blend: Blend::Normal,
                uv: None,
                sampling: Default::default(),
                filter: None,
            },
            None,
        )
    }
    pub(super) fn object(
        &mut self,
        images: &mut ImageManager,
        object: &ObjectState,
        parent: Option<u32>,
        order: [i32; 2],
        depth: usize,
    ) -> Result<()> {
        ensure!(depth <= 16, "native render-tree depth");
        if object.object_type == 0 && object.runtime.child_objects.is_empty() {
            return Ok(());
        }
        ensure!(
            !object.button.enabled,
            "native button requires shared UI interaction mapping"
        );
        let b = &effective_base(object)?;
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
            "native nested object visual profile"
        );
        let order = [
            order[0]
                .checked_add(i32::try_from(b.order)?)
                .context("native tree order overflow")?,
            order[1]
                .checked_add(i32::try_from(b.layer)?)
                .context("native tree layer overflow")?,
        ];
        let children = &object.runtime.child_objects;
        ensure!(
            b.rotate_x == 0
                && b.rotate_y == 0
                && b.z == 0
                && b.center_rep_z == 0
                && b.scale_z == 1000,
            "native nested 3D objects require shared world hierarchy mapping"
        );
        if !children.is_empty() {
            ensure!(
                b.rotate_x == 0
                    && b.rotate_y == 0
                    && b.z == 0
                    && b.center_rep_z == 0
                    && b.scale_z == 1000
                    && b.blend == 0
                    && object_filter(b).is_none(),
                "native nested parent requires a shared inherited 3D/filter/blend profile"
            );
            ensure!(
                (0..=1).contains(&b.child_sort_type)
                    && children
                        .iter()
                        .all(|c| c.base.order == 0 && c.base.layer == 0),
                "native non-default subtree ordering requires an explicit shared sort scope"
            );
        }
        // CHILD inherits the parent's native transform around center_rep, not
        // the parent's image-center/payload pivot. The transparent group keeps
        // those distinct while CrySirror composes the matrices and opacity.
        let group = self.push(
            LayerSpec {
                id: 0,
                parent,
                rect: [
                    native_anchor_component(b.x, b.center_rep_x),
                    native_anchor_component(b.y, b.center_rep_y),
                    1.,
                    1.,
                ],
                paint: Paint::Solid { color: [0; 4] },
                transform: SpriteTransform {
                    scale_x: b.scale_x as f32 / 1000.,
                    scale_y: b.scale_y as f32 / 1000.,
                    rotate: b.rotate_z as f32 * std::f32::consts::PI / 1800.,
                    object_anchor: true,
                    pivot_x: b.center_rep_x as f32,
                    pivot_y: b.center_rep_y as f32,
                    ..Default::default()
                },
                order,
                clip: None,
                opacity: b.tr.clamp(0, 255) as u8,
                visible: b.disp != 0,
                blend: Blend::Normal,
                uv: None,
                sampling: Default::default(),
                filter: None,
            },
            None,
        )?;
        let mut payload_visible = true;
        let payload = match object.object_type {
            0 => None,
            1 => {
                let p = &object.rect_param;
                let c = p.color_argb;
                Some((
                    Paint::Solid {
                        color: [(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8],
                    },
                    p.left.abs_diff(p.right).clamp(1, 32767) as f32,
                    p.top.abs_diff(p.bottom).clamp(1, 32767) as f32,
                    0,
                    0,
                    None,
                ))
            }
            2 => {
                let file = object
                    .file_name
                    .as_deref()
                    .context("native nested PCT filename")?;
                let image = images.load_g00(
                    file,
                    usize::try_from(b.patno).context("negative native nested PATNO")?,
                )?;
                let bitmap = &images.images[image];
                Some((
                    Paint::Image {
                        asset: "__crysirror/pending".into(),
                        tint: [255; 4],
                    },
                    bitmap.width as f32,
                    bitmap.height as f32,
                    bitmap.center_x,
                    bitmap.center_y,
                    Some(image),
                ))
            }
            9 => {
                let picture = self
                    .movies
                    .and_then(|m| m.get(&self.path))
                    .context("native object movie lacks engine Media resource receipt")?;
                payload_visible = picture.ready;
                let paint = if picture.ready {
                    Paint::Image {
                        asset: picture.surface.clone(),
                        tint: [255; 4],
                    }
                } else {
                    Paint::Solid { color: [0; 4] }
                };
                Some((
                    paint,
                    picture.size[0] as f32,
                    picture.size[1] as f32,
                    0,
                    0,
                    None,
                ))
            }
            other => anyhow::bail!(
                "native nested payload type {other} requires shared rendering mapping"
            ),
        };
        if let Some((paint, w, h, cx, cy, image)) = payload {
            self.push(
                LayerSpec {
                    id: 0,
                    parent: Some(group),
                    rect: [
                        -(b.center_x as f32) - cx as f32,
                        -(b.center_y as f32) - cy as f32,
                        w,
                        h,
                    ],
                    paint,
                    transform: SpriteTransform {
                        z: b.z as f32 + b.center_rep_z as f32,
                        rotate_x: b.rotate_x as f32 * std::f32::consts::PI / 1800.,
                        rotate_y: b.rotate_y as f32 * std::f32::consts::PI / 1800.,
                        culling: b.culling != 0,
                        ..Default::default()
                    },
                    order,
                    clip: None,
                    opacity: b.alpha.clamp(0, 255) as u8,
                    visible: payload_visible,
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
                },
                image,
            )?;
        }
        for (index, child) in children.iter().enumerate() {
            self.path.push(index);
            let result = self.object(images, child, Some(group), order, depth + 1);
            self.path.pop();
            result?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn engine_composes_child_origin_without_inheriting_parent_payload_center() {
        let mut parent = ObjectState::default();
        parent.used = true;
        parent.object_type = 1;
        parent.base.disp = 1;
        let constants = crate::runtime::constants::RuntimeConstants::default();
        for (op, value) in [
            (constants.obj_x, 100),
            (constants.obj_center_rep_x, 10),
            (constants.obj_center_x, 30),
            (constants.obj_scale_x, 2000),
        ] {
            parent.set_int_prop(&constants, op, value);
        }
        parent.rect_param.right = 5;
        parent.rect_param.bottom = 5;
        parent.rect_param.color_argb = 0xffff0000;
        let mut child = ObjectState::default();
        child.used = true;
        child.object_type = 1;
        child.base.disp = 1;
        child.set_int_prop(&constants, constants.obj_x, 7);
        child.rect_param.right = 10;
        child.rect_param.bottom = 5;
        child.rect_param.color_argb = 0xff00ff00;
        parent.runtime.child_objects.push(child);
        let (mut layers, mut needed, mut ids) = (vec![], vec![], BTreeSet::new());
        Output {
            layers: &mut layers,
            images: &mut needed,
            ids: &mut ids,
            next: 0x81000000,
            movies: None,
            path: Vec::new(),
        }
        .object(&mut ImageManager::default(), &parent, None, [0, 0], 0)
        .unwrap();
        assert_eq!((layers.len(), ids.len(), needed.len()), (4, 4, 4));
        assert_eq!(layers[1].parent, Some(layers[0].id));
        assert_eq!(layers[2].parent, Some(layers[0].id));
        assert_eq!(layers[3].parent, Some(layers[2].id));
        let mut engine = crysirror_scene::EngineSystems::default();
        let batch = ServiceBatch {
            layers: layers
                .into_iter()
                .map(|layer| LayerCommand::Put { layer })
                .collect(),
            ..Default::default()
        };
        engine.submit(ClientId(1), &batch).unwrap();
        let frame = engine.frame([320, 180]).unwrap();
        let red = frame
            .iter()
            .find(|p| {
                matches!(
                    p.paint,
                    Paint::Solid {
                        color: [255, 0, 0, 255]
                    }
                )
            })
            .unwrap();
        let green = frame
            .iter()
            .find(|p| {
                matches!(
                    p.paint,
                    Paint::Solid {
                        color: [0, 255, 0, 255]
                    }
                )
            })
            .unwrap();
        assert_eq!(red.quad[0][..2], [30., 0.]);
        assert_eq!(green.quad[0][..2], [104., 0.]);
        assert_eq!(green.quad[2][..2], [124., 5.]);
        let mut hide = batch.clone();
        let LayerCommand::Put { layer } = &mut hide.layers[0] else {
            unreachable!()
        };
        layer.visible = false;
        engine.submit(ClientId(1), &hide).unwrap();
        assert!(engine.frame([320, 180]).unwrap().is_empty());
        assert!(
            engine.layers.get(ClientId(1), 0x81000003).is_some(),
            "hidden child remains an engine-owned retained resource"
        );
    }
}
