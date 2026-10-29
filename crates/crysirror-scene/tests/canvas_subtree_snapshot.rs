//! Frozen retained subtree capture: no VM or GPU, exact primitive semantics.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{CanvasCommand as C, CanvasSystem},
    layer::{Blend, LayerCommand as L, LayerSpec, LayerSystem, Paint, Primitive},
};
use serde_json::json;
const CLIENT: ClientId = ClientId(1);
fn solid(id: u32, parent: Option<u32>, rect: [f32; 4], color: [u8; 4], order: i32) -> LayerSpec {
    serde_json::from_value(json!({"id":id,"parent":parent,"rect":rect,
        "paint":{"kind":"solid","color":color},"order":[0,order]}))
    .unwrap()
}
fn destination(layers: &mut LayerSystem) -> CanvasSystem {
    let mut c = CanvasSystem::default();
    c.apply_batch(
        CLIENT,
        &[
            C::Create {
                id: 1,
                width: 16,
                height: 16,
            },
            C::Clear {
                id: 1,
                color: [0; 4],
            },
            C::Present {
                id: 1,
                layer: 10,
                order: 0,
                parent: None,
                position: [0., 0.],
            },
        ],
        layers,
    )
    .unwrap();
    c
}
fn copy(source: u32) -> C {
    C::CopySubtree {
        source,
        destination: 1,
        rect: [0., 0., 16., 16.],
        position: [0., 0.],
        clip: None,
    }
}
fn painted(layers: &LayerSystem) -> Vec<Primitive> {
    layers
        .frame([16, 16])
        .unwrap()
        .into_iter()
        .filter(|p| matches!(&p.paint,Paint::Solid{color} if color[3]!=0))
        .collect()
}
fn snapshot_primitive(layers: &LayerSystem) -> Primitive {
    let mut found: Vec<_> = layers
        .frame([16, 16])
        .unwrap()
        .into_iter()
        .filter(|p| matches!(p.paint, Paint::Snapshot { .. }))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "snapshot must be one externally composited draw"
    );
    found.remove(0)
}
fn snapshot_contents(layers: &LayerSystem) -> Vec<Primitive> {
    let snapshot = snapshot_primitive(layers);
    let Paint::Snapshot { size, layers } = &snapshot.paint else {
        unreachable!()
    };
    assert_eq!(*size, [16, 16]);
    let mut frozen = LayerSystem::default();
    frozen.install(CLIENT, layers).unwrap();
    painted(&frozen)
}
fn assert_primitives(actual: &[Primitive], expected: &[Primitive]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.iter().zip(expected) {
        assert_eq!(
            serde_json::to_value(&a.paint).unwrap(),
            serde_json::to_value(&b.paint).unwrap()
        );
        assert_eq!(a.opacity, b.opacity);
        for (x, y) in a.quad.iter().flatten().zip(b.quad.iter().flatten()) {
            assert!((x - y).abs() < 0.001, "quad {x} vs {y}");
        }
        for (x, y) in a.clip.unwrap().iter().zip(b.clip.unwrap().iter()) {
            assert!((x - y).abs() < 0.001, "clip {x} vs {y}");
        }
    }
}
#[test]
fn snapshot_preserves_global_order_transform_clip_and_outlives_source_tree() {
    let mut root = solid(100, None, [0., 0., 16., 16.], [0; 4], 90);
    let mut parent = solid(101, Some(100), [1., 2., 6., 6.], [255, 0, 0, 255], 30);
    parent.transform.rotate = 0.25;
    parent.transform.scale_x = 1.5;
    parent.transform.pivot_x = 0.5;
    parent.clip = Some([0., 0., 5., 5.]);
    let mut child = solid(103, Some(101), [1., 1., 2., 2.], [0, 0, 255, 128], 10);
    child.transform.scale_y = 1.2;
    child.transform.pivot_y = 0.25;
    let sibling = solid(102, Some(100), [0., 0., 2., 2.], [0, 255, 0, 255], 20);
    let mut oracle = LayerSystem::default();
    oracle
        .install(
            CLIENT,
            &[root.clone(), parent.clone(), child.clone(), sibling.clone()],
        )
        .unwrap();
    let expected = painted(&oracle);
    assert!(matches!(
        expected[0].paint,
        Paint::Solid {
            color: [0, 0, 255, 128]
        }
    ));
    let mut layers = LayerSystem::default();
    let mut outside = solid(99, None, [100., 100., 16., 16.], [255; 4], 0);
    outside.visible = false;
    outside.opacity = 23;
    root.parent = Some(99);
    root.rect[0] = 85.;
    root.rect[1] = 75.;
    root.visible = false;
    root.opacity = 9;
    root.transform.scale_x = 2.;
    root.transform.rotate = 0.7;
    layers
        .install(CLIENT, &[outside, root, parent, child, sibling])
        .unwrap();
    let mut canvases = destination(&mut layers);
    canvases
        .apply_batch(CLIENT, &[copy(100)], &mut layers)
        .unwrap();
    let frozen = serde_json::to_value(canvases.surface(CLIENT, 1)).unwrap();
    assert_primitives(&snapshot_contents(&layers), &expected);
    layers.apply_batch(CLIENT, &[L::Remove { id: 99 }]).unwrap();
    assert_eq!(
        serde_json::to_value(canvases.surface(CLIENT, 1)).unwrap(),
        frozen
    );
    assert_primitives(&snapshot_contents(&layers), &expected);
}

#[test]
fn hidden_dynamic_children_are_skipped_but_visible_motion_meshes_fail_atomically() {
    let mut root = solid(100, None, [0., 0., 16., 16.], [255, 0, 0, 255], 0);
    root.visible = false;
    let mut child = solid(101, Some(100), [0., 0., 4., 4.], [0; 4], 1);
    child.visible = false;
    let mut layers = LayerSystem::default();
    layers.install(CLIENT, &[root, child]).unwrap();
    layers
        .set_dynamic_puppet(
            1,
            101,
            std::sync::Arc::new(crysirror_puppet::PuppetFrame {
                size: [4., 4.],
                ..Default::default()
            }),
        )
        .unwrap();
    let mut canvases = destination(&mut layers);
    canvases
        .apply_batch(CLIENT, &[copy(100)], &mut layers)
        .unwrap();
    layers
        .apply_batch(
            CLIENT,
            &[L::SetVisible {
                id: 101,
                visible: true,
            }],
        )
        .unwrap();
    let before = (
        serde_json::to_value(&canvases).unwrap(),
        serde_json::to_value(&layers).unwrap(),
    );
    let error = canvases
        .apply_batch(CLIENT, &[copy(100)], &mut layers)
        .unwrap_err();
    assert!(error.to_string().contains("dynamic puppet"), "{error:#}");
    assert_eq!(
        (
            serde_json::to_value(&canvases).unwrap(),
            serde_json::to_value(&layers).unwrap()
        ),
        before
    );
}

#[test]
fn same_batch_canvas_edits_are_captured_and_later_edits_do_not_alias() {
    let mut layers = LayerSystem::default();
    let mut c = destination(&mut layers);
    c.apply_batch(
        CLIENT,
        &[
            C::Create {
                id: 2,
                width: 16,
                height: 16,
            },
            C::Clear {
                id: 2,
                color: [255, 0, 0, 255],
            },
            C::Present {
                id: 2,
                layer: 20,
                order: 1,
                parent: None,
                position: [0., 0.],
            },
            C::CopySubtree {
                source: 20,
                destination: 1,
                rect: [0., 0., 16., 16.],
                position: [0., 0.],
                clip: None,
            },
            C::Clear {
                id: 2,
                color: [0, 0, 255, 255],
            },
        ],
        &mut layers,
    )
    .unwrap();
    c.apply_batch(CLIENT, &[C::Hide { layer: 20 }], &mut layers)
        .unwrap();
    let pixels = snapshot_contents(&layers);
    assert_eq!(pixels.len(), 1);
    assert!(matches!(
        pixels[0].paint,
        Paint::Solid {
            color: [255, 0, 0, 255]
        }
    ));
}

fn sample(layers: &LayerSystem, x: f32, y: f32) -> [f32; 4] {
    let mut out = [0f32; 4];
    for p in layers.frame([16, 16]).unwrap() {
        let clip = p.clip.unwrap();
        if x < clip[0] || y < clip[1] || x >= clip[0] + clip[2] || y >= clip[1] + clip[3] {
            continue;
        }
        let x0 = p.quad.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min);
        let y0 = p.quad.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min);
        let x1 = p
            .quad
            .iter()
            .map(|q| q[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let y1 = p
            .quad
            .iter()
            .map(|q| q[1])
            .fold(f32::NEG_INFINITY, f32::max);
        if x < x0 || y < y0 || x >= x1 || y >= y1 {
            continue;
        }
        let src = match &p.paint {
            Paint::Solid { color } => {
                let alpha = f32::from(color[3]) / 255.;
                [
                    f32::from(color[0]) / 255. * alpha,
                    f32::from(color[1]) / 255. * alpha,
                    f32::from(color[2]) / 255. * alpha,
                    alpha,
                ]
            }
            Paint::Snapshot { size, layers } => {
                let mut frozen = LayerSystem::default();
                frozen.install(CLIENT, layers).unwrap();
                let u = p.uv[0] + (x - x0) / (x1 - x0) * (p.uv[2] - p.uv[0]);
                let v = p.uv[1] + (y - y0) / (y1 - y0) * (p.uv[3] - p.uv[1]);
                sample(&frozen, u * size[0] as f32, v * size[1] as f32)
            }
            _ => panic!("test only covers solid fixtures and snapshot transport"),
        };
        let opacity = f32::from(p.opacity) / 255.;
        let alpha = src[3] * opacity;
        for i in 0..3 {
            out[i] = src[i] * opacity + out[i] * (1. - alpha);
        }
        out[3] = alpha + out[3] * (1. - alpha);
    }
    out
}
#[test]
fn clipped_replace_copies_transparency_instead_of_blending_over_old_pixels() {
    let mut layers = LayerSystem::default();
    let mut root = solid(100, None, [50., 50., 16., 16.], [0; 4], 0);
    root.visible = false;
    layers.install(CLIENT, &[root]).unwrap();
    let mut c = destination(&mut layers);
    c.apply_batch(
        CLIENT,
        &[
            C::Clear {
                id: 1,
                color: [0, 255, 0, 255],
            },
            C::CopySubtree {
                source: 100,
                destination: 1,
                rect: [1., 1., 4., 4.],
                position: [2., 2.],
                clip: Some([3., 3., 2., 2.]),
            },
        ],
        &mut layers,
    )
    .unwrap();
    let captured = snapshot_primitive(&layers);
    assert_eq!(captured.extent, [2., 2.]);
    assert_eq!(captured.uv, [2. / 16., 2. / 16., 4. / 16., 4. / 16.]);
    assert_eq!(captured.quad[0][..2], [3., 3.]);
    assert_eq!(sample(&layers, 3.5, 3.5), [0.; 4]);
    assert_eq!(sample(&layers, 2.5, 3.5), [0., 1., 0., 1.]);
    assert_eq!(sample(&layers, 5.5, 3.5), [0., 1., 0., 1.]);
}

#[test]
fn unsupported_blend_group_opacity_and_active_composite_fail_explicitly() {
    for mode in [0, 1, 2] {
        let mut root = solid(100, None, [0., 0., 16., 16.], [255; 4], 0);
        let mut parent = solid(101, Some(100), [0., 0., 8., 8.], [255; 4], 1);
        let child = solid(102, Some(101), [0., 0., 4., 4.], [255; 4], 2);
        if mode == 0 {
            root.blend = Blend::Add;
        }
        if mode == 1 {
            parent.opacity = 128;
        }
        let sibling = solid(200, None, [0., 0., 16., 16.], [255; 4], 0);
        let mut layers = LayerSystem::default();
        layers
            .install(CLIENT, &[root, parent, child, sibling])
            .unwrap();
        if mode == 2 {
            layers
                .apply_batch(
                    CLIENT,
                    &[L::BeginCrossfade {
                        id: 100,
                        source: 200,
                        duration_ms: 500,
                        rule: None,
                        external: false,
                    }],
                )
                .unwrap();
        }
        let mut c = destination(&mut layers);
        assert!(c.apply_batch(CLIENT, &[copy(100)], &mut layers).is_err());
    }
}

#[test]
fn ancestor_chains_are_stored_once_without_flattening_expansion() {
    let mut specs = Vec::new();
    for id in 100..120 {
        specs.push(solid(
            id,
            if id == 100 { None } else { Some(id - 1) },
            [0., 0., 16., 16.],
            [0; 4],
            0,
        ));
    }
    for id in 200..410 {
        specs.push(solid(id, Some(119), [0., 0., 1., 1.], [255; 4], id as i32));
    }
    let mut layers = LayerSystem::default();
    layers.install(CLIENT, &specs).unwrap();
    let mut c = destination(&mut layers);
    c.apply_batch(CLIENT, &[copy(100)], &mut layers).unwrap();
    let snapshot = snapshot_primitive(&layers);
    let Paint::Snapshot { layers: frozen, .. } = snapshot.paint else {
        unreachable!()
    };
    assert_eq!(frozen.len(), specs.len());
    assert_eq!(frozen.len(), 230);
}

#[test]
fn later_opacity_and_scale_apply_once_to_single_snapshot_not_frozen_inputs() {
    let mut layers = LayerSystem::default();
    layers
        .install(
            CLIENT,
            &[
                solid(100, None, [0., 0., 16., 16.], [255; 4], 0),
                solid(101, Some(100), [0., 0., 8., 8.], [255, 0, 0, 128], 1),
            ],
        )
        .unwrap();
    let mut c = destination(&mut layers);
    c.apply_batch(CLIENT, &[copy(100)], &mut layers).unwrap();
    let before = snapshot_primitive(&layers);
    let frozen = serde_json::to_value(&before.paint).unwrap();
    let transform = crysirror_scene::layer::SpriteTransform {
        scale_x: 2.,
        scale_y: 2.,
        ..Default::default()
    };
    layers
        .apply_batch(
            CLIENT,
            &[
                L::SetOpacity {
                    id: 10,
                    opacity: 127,
                },
                L::SetTransform { id: 10, transform },
            ],
        )
        .unwrap();
    let after = snapshot_primitive(&layers);
    assert_eq!(after.opacity, 127);
    assert_eq!(serde_json::to_value(&after.paint).unwrap(), frozen);
    assert_eq!(after.extent, [16., 16.]);
    assert_eq!(
        after
            .quad
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max),
        32.
    );
    assert_eq!(
        after
            .quad
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max),
        32.
    );
}

#[test]
fn serialized_snapshot_bounds_nesting_and_rejects_external_duplicate_and_oversized_inputs() {
    let root = solid(1, None, [0., 0., 16., 16.], [255; 4], 0);
    let inner = Paint::Snapshot {
        size: [16, 16],
        layers: vec![root.clone()].into(),
    };
    assert!(inner.validate().is_ok());
    // Nesting is bounded: payloads within MAX_SNAPSHOT_NESTING levels are
    // valid (renderers rasterize inner payloads first); deeper ones fail.
    let mut paint = inner;
    for level in 1..=crysirror_scene::layer::MAX_SNAPSHOT_NESTING {
        let mut nested = root.clone();
        nested.paint = paint;
        paint = Paint::Snapshot { size: [16, 16], layers: vec![nested].into() };
        let result = paint.validate();
        if level < crysirror_scene::layer::MAX_SNAPSHOT_NESTING {
            assert!(result.is_ok(), "level {level}: {result:?}");
        } else {
            assert!(result.unwrap_err().to_string().contains("nesting"), "level {level}");
        }
    }
    let external = solid(2, Some(999), [0., 0., 2., 2.], [255; 4], 0);
    assert!(
        Paint::Snapshot {
            size: [16, 16],
            layers: vec![root.clone(), external].into()
        }
        .validate()
        .is_err()
    );
    assert!(
        Paint::Snapshot {
            size: [16, 16],
            layers: vec![root.clone(), root.clone()].into()
        }
        .validate()
        .is_err()
    );
    assert!(
        Paint::Snapshot {
            size: [16, 16],
            layers: vec![root.clone(); 4097].into()
        }
        .validate()
        .is_err()
    );
    assert!(
        Paint::Snapshot {
            size: [8193, 16],
            layers: vec![root].into()
        }
        .validate()
        .is_err()
    );
}
