//! A surface edit updates all its presentations, while unrelated presentations
//! retain their physical identities and layer-side state.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{CanvasCommand as C, CanvasSystem},
    layer::{LayerCommand as L, LayerSystem},
};
use serde_json::Value;

fn present(id: u32, layer: u32) -> C {
    C::Present {
        id,
        layer,
        parent: None,
        order: 0,
        position: [0., 0.],
    }
}
fn generated(canvases: &CanvasSystem, root: u32) -> Vec<u32> {
    let value = serde_json::to_value(canvases).unwrap();
    value["presentations"]["1"][root.to_string()]["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id: &Value| id.as_u64().unwrap() as u32)
        .collect()
}
fn setup() -> (CanvasSystem, LayerSystem) {
    let mut canvases = CanvasSystem::default();
    let mut layers = LayerSystem::default();
    canvases
        .apply_batch(
            ClientId(1),
            &[
                C::Create {
                    id: 1,
                    width: 8,
                    height: 8,
                },
                C::Create {
                    id: 2,
                    width: 8,
                    height: 8,
                },
                present(1, 10),
                present(1, 11),
                present(2, 20),
            ],
            &mut layers,
        )
        .unwrap();
    (canvases, layers)
}

#[test]
fn source_edit_updates_all_aliases_but_preserves_unrelated_layer_state() {
    let (mut canvases, mut layers) = setup();
    let before = [10, 11, 20].map(|root| generated(&canvases, root));
    layers
        .apply_batch(
            ClientId(1),
            &[L::SetOpacity {
                id: 20,
                opacity: 99,
            }],
        )
        .unwrap();
    canvases
        .apply_batch(
            ClientId(1),
            &[C::Clear {
                id: 1,
                color: [12, 34, 56, 255],
            }],
            &mut layers,
        )
        .unwrap();
    assert_ne!(generated(&canvases, 10), before[0]);
    assert_ne!(generated(&canvases, 11), before[1]);
    assert_eq!(generated(&canvases, 20), before[2]);
    assert_eq!(layers.get(ClientId(1), 20).unwrap().opacity, 99);
    for root in [10, 11] {
        let physical = generated(&canvases, root);
        let node = layers.get(ClientId(1), physical[0]).unwrap();
        assert_eq!(
            serde_json::to_value(&node.paint).unwrap()["color"],
            serde_json::json!([12, 34, 56, 255])
        );
    }
}

#[test]
fn all_copy_operations_only_invalidate_the_destination() {
    let (mut canvases, mut layers) = setup();
    for command in [
        C::Copy {
            source: 1,
            destination: 2,
            rect: [0., 0., 8., 8.],
            position: [0., 0.],
            opacity: 255,
            replace: true,
            clip: None, atop: None
        },
        C::TransformCopy {
            source: 1,
            destination: 2,
            rect: [0., 0., 8., 8.],
            matrix: [1., 0., 0., 1., 0., 0.],
            replace: true,
            clip: None, opacity: 255,
        },
        C::CopySubtree {
            source: 10,
            destination: 2,
            rect: [0., 0., 8., 8.],
            position: [0., 0.],
            clip: None,
        },
    ] {
        let before = [10, 11, 20].map(|root| generated(&canvases, root));
        canvases
            .apply_batch(ClientId(1), &[command], &mut layers)
            .unwrap();
        assert_eq!(generated(&canvases, 10), before[0]);
        assert_eq!(generated(&canvases, 11), before[1]);
        assert_ne!(generated(&canvases, 20), before[2]);
    }
}

#[test]
fn present_and_hide_do_not_rebuild_other_bindings_even_for_the_same_source() {
    let (mut canvases, mut layers) = setup();
    let before = [10, 11, 20].map(|root| generated(&canvases, root));
    canvases
        .apply_batch(ClientId(1), &[present(1, 10)], &mut layers)
        .unwrap();
    assert_ne!(generated(&canvases, 10), before[0]);
    assert_eq!(generated(&canvases, 11), before[1]);
    assert_eq!(generated(&canvases, 20), before[2]);
    canvases
        .apply_batch(ClientId(1), &[C::Hide { layer: 10 }], &mut layers)
        .unwrap();
    assert!(layers.get(ClientId(1), 10).is_none());
    assert_eq!(generated(&canvases, 11), before[1]);
    assert_eq!(generated(&canvases, 20), before[2]);
}
