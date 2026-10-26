// SPDX-License-Identifier: MPL-2.0
//! Retained Canvas behavior, without a VM, font, device or game data.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{CanvasCommand as C, CanvasNode, CanvasSystem},
    layer::{LayerSpec, LayerSystem},
};
use serde_json::{json, to_value};
fn draw(color: [u8; 4]) -> LayerSpec {
    serde_json::from_value(
        json!({"id":0,"rect":[0,0,100,100],"paint":{"kind":"solid","color":color}}),
    )
    .unwrap()
}
fn create(id: u32) -> C {
    C::Create {
        id,
        width: 10,
        height: 10,
    }
}
fn copy(rect: [f32; 4], position: [f32; 2], replace: bool) -> C {
    C::Copy {
        source: 1,
        destination: 2,
        rect,
        position,
        opacity: 255,
        replace,
    }
}
fn setup() -> (CanvasSystem, LayerSystem) {
    let mut c = CanvasSystem::default();
    let mut l = LayerSystem::default();
    c.apply_batch(
        ClientId(1),
        &[
            create(1),
            create(2),
            C::Clear {
                id: 1,
                color: [255, 0, 0, 255],
            },
            C::Draw {
                id: 1,
                layer: draw([0, 255, 0, 255]),
            },
            C::Present {
                id: 2,
                layer: 7,
                order: 0,
                parent: None,
                position: [0., 0.],
            },
        ],
        &mut l,
    )
    .unwrap();
    (c, l)
}
#[test]
fn copies_are_independent_snapshots_and_roundtrip() {
    let (mut c, mut l) = setup();
    c.apply_batch(
        ClientId(1),
        &[copy([0., 0., 10., 10.], [0., 0.], true)],
        &mut l,
    )
    .unwrap();
    let snapshot = to_value(c.surface(ClientId(1), 2)).unwrap();
    c.apply_batch(
        ClientId(1),
        &[C::Clear {
            id: 1,
            color: [0, 0, 255, 255],
        }],
        &mut l,
    )
    .unwrap();
    assert_eq!(to_value(c.surface(ClientId(1), 2)).unwrap(), snapshot);
    let saved = to_value(&c).unwrap();
    let restored: CanvasSystem = serde_json::from_value(saved.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(to_value(restored).unwrap(), saved);
}
#[test]
fn client_handles_and_presentations_are_isolated() {
    let (mut c, mut l) = setup();
    let before = to_value(&c).unwrap();
    assert!(
        c.apply_batch(
            ClientId(2),
            &[copy([0., 0., 10., 10.], [0., 0.], true)],
            &mut l
        )
        .is_err()
    );
    assert_eq!(to_value(&c).unwrap(), before);
    c.apply_batch(
        ClientId(2),
        &[
            create(1),
            C::Clear {
                id: 1,
                color: [0, 0, 255, 255],
            },
            C::Present {
                id: 1,
                layer: 7,
                order: 0,
                parent: None,
                position: [0., 0.],
            },
        ],
        &mut l,
    )
    .unwrap();
    assert_eq!(
        to_value(c.surface(ClientId(1), 1)).unwrap(),
        before["clients"]["1"]["1"]
    );
    assert!(l.get(ClientId(1), 7).is_some());
    assert!(l.get(ClientId(2), 7).is_some());
}
#[test]
fn source_and_destination_clipping_preserve_the_requested_mapping() {
    let (mut c, mut l) = setup();
    c.apply_batch(
        ClientId(1),
        &[copy([-2., -3., 10., 10.], [0., 0.], false)],
        &mut l,
    )
    .unwrap();
    match c.surface(ClientId(1), 2).unwrap().nodes.last().unwrap() {
        CanvasNode::Group { offset, clip, .. } => {
            assert_eq!(*offset, [2., 3.]);
            assert_eq!(*clip, [0., 0., 8., 7.]);
        }
        _ => panic!(),
    };
    c.apply_batch(
        ClientId(1),
        &[copy([0., 0., 10., 10.], [-5., -6.], false)],
        &mut l,
    )
    .unwrap();
    match c.surface(ClientId(1), 2).unwrap().nodes.last().unwrap() {
        CanvasNode::Group { clip, .. } => assert_eq!(*clip, [5., 6., 5., 4.]),
        _ => panic!(),
    };
    let frame = l.frame([50, 50]).unwrap();
    assert!(frame.iter().all(|p| p.clip.is_some_and(|clip| clip[0] >= 0.
        && clip[1] >= 0.
        && clip[0] + clip[2] <= 10.
        && clip[1] + clip[3] <= 10.)));
}
#[test]
fn empty_intersections_are_true_noops_even_for_replace() {
    let (mut c, mut l) = setup();
    let before = (to_value(&c).unwrap(), to_value(&l).unwrap());
    for command in [
        copy([30., 0., 10., 10.], [0., 0.], false),
        copy([-20., 0., 10., 10.], [0., 0.], true),
        copy([0., 0., 10., 10.], [40., 0.], false),
    ] {
        c.apply_batch(ClientId(1), &[command], &mut l).unwrap();
        assert_eq!((to_value(&c).unwrap(), to_value(&l).unwrap()), before);
    }
}
#[test]
fn errors_roll_back_canvas_and_generated_layers_atomically() {
    let (mut c, mut l) = setup();
    let before = (to_value(&c).unwrap(), to_value(&l).unwrap());
    for command in [
        C::Copy {
            source: 999,
            destination: 2,
            rect: [0., 0., 10., 10.],
            position: [0., 0.],
            opacity: 255,
            replace: false,
        },
        C::Copy {
            source: 1,
            destination: 2,
            rect: [0., 0., 10., 10.],
            position: [0., 0.],
            opacity: 128,
            replace: false,
        },
        copy([-2., 0., 10., 10.], [0., 0.], true),
    ] {
        assert!(
            c.apply_batch(
                ClientId(1),
                &[
                    C::Clear {
                        id: 1,
                        color: [1, 2, 3, 4]
                    },
                    command
                ],
                &mut l
            )
            .is_err()
        );
        assert_eq!((to_value(&c).unwrap(), to_value(&l).unwrap()), before);
    }
}
#[test]
fn dynamic_reveal_is_rejected_in_commands_and_restored_groups() {
    let (mut c, mut l) = setup();
    let layer:LayerSpec=serde_json::from_value(json!({"id":0,"rect":[0,0,10,10],"paint":{"kind":"glyph_run","font":"font","size":8,"glyphs":[{"character":"字","baseline":[0,8]}],"reveal":{"interval_ms":20}}})).unwrap();
    let before = (to_value(&c).unwrap(), to_value(&l).unwrap());
    for command in [
        C::Draw {
            id: 1,
            layer: layer.clone(),
        },
        C::Set {
            id: 1,
            layer: layer.clone(),
        },
    ] {
        assert!(
            c.apply_batch(ClientId(1), &[command], &mut l)
                .unwrap_err()
                .to_string()
                .contains("canonical canvas clock")
        );
        assert_eq!((to_value(&c).unwrap(), to_value(&l).unwrap()), before);
    }
    let mut saved = to_value(&c).unwrap();
    saved["clients"]["1"]["1"]["nodes"] = json!([{"kind":"group","offset":[0,0],"clip":[0,0,10,10],"opacity":255,"nodes":[{"kind":"draw","layer":layer}]}]);
    let invalid: CanvasSystem = serde_json::from_value(saved.clone()).unwrap();
    assert!(invalid.validate().is_err());
    saved["clients"]["1"]["1"]["nodes"][0]["nodes"] = json!([]);
    saved["clients"]["1"]["1"]["nodes"][0]["opacity"] = json!(128);
    assert!(
        serde_json::from_value::<CanvasSystem>(saved)
            .unwrap()
            .validate()
            .is_err()
    );
}
#[test]
fn presentation_attaches_under_parent_layer_and_follows_it() {
    use crysirror_scene::layer::LayerCommand;
    let mut c = CanvasSystem::default();
    let mut l = LayerSystem::default();
    let client = ClientId(1);
    let mut parent = draw([0, 0, 0, 0]);
    parent.id = 5;
    parent.rect = [100., 50., 64., 32.];
    l.apply_batch(client, &[LayerCommand::Put { layer: parent }]).unwrap();
    c.apply_batch(
        client,
        &[
            C::Create { id: 3, width: 16, height: 8 },
            C::Clear { id: 3, color: [255, 0, 0, 255] },
            C::Present { id: 3, layer: 9, order: 2, parent: Some(5), position: [4., 2.] },
        ],
        &mut l,
    )
    .unwrap();
    let root = l.get(client, 9).unwrap();
    assert_eq!(root.parent, Some(5));
    assert_eq!(root.rect, [4., 2., 16., 8.]);
    let frame = l.frame([640, 480]).unwrap();
    let fill = frame
        .iter()
        .find(|p| matches!(p.paint, crysirror_scene::layer::Paint::Solid { color } if color == [255, 0, 0, 255]))
        .unwrap();
    assert_eq!(fill.quad[0][0], 104.);
    assert_eq!(fill.quad[0][1], 52.);
    // A presentation cannot reference a parent that does not exist.
    assert!(c
        .apply_batch(
            client,
            &[C::Present { id: 3, layer: 10, order: 0, parent: Some(77), position: [0., 0.] }],
            &mut l,
        )
        .is_err());
}
