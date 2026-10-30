//! Engine-only compaction ownership regression tests, without native VMs or GPU.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{COMPACT_NODES, CanvasCommand as C, CanvasNode, CanvasSystem},
    layer::{LayerSpec, LayerSystem, Paint},
};
use serde_json::json;

fn draw() -> C {
    C::Draw {
        id: 1,
        layer: serde_json::from_value::<LayerSpec>(json!({
            "id":0,"rect":[0,0,8,8],"paint":{"kind":"solid","color":[200,40,10,8]}
        }))
        .unwrap(),
        atop: None,
    }
}
fn fixture() -> (CanvasSystem, LayerSystem) {
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
                C::Present {
                    id: 1,
                    layer: 10,
                    parent: None,
                    order: 0,
                    position: [0., 0.],
                },
            ],
            &mut layers,
        )
        .unwrap();
    (canvases, layers)
}

#[test]
fn compaction_preserves_frozen_copies_and_drawing_after_capture() {
    let (mut c, mut l) = fixture();
    c.apply_batch(ClientId(1), &vec![draw(); COMPACT_NODES + 1], &mut l)
        .unwrap();
    assert_eq!(c.surface(ClientId(1), 1).unwrap().nodes.len(), 1);
    let request = c.materialize_requests().pop().unwrap();
    assert_eq!(request.size, [8, 8]);
    c.apply_batch(
        ClientId(1),
        &[
            C::Copy {
                source: 1,
                destination: 2,
                rect: [0., 0., 8., 8.],
                position: [0., 0.],
                opacity: 255,
                replace: true,
                clip: None,
                atop: None,
            },
            draw(),
        ],
        &mut l,
    )
    .unwrap();
    c.resolve_materialized(&request, "__crysirror/frozen.png", &mut l)
        .unwrap();
    assert!(c.materialize_requests().is_empty());
    assert_eq!(
        c.surface(ClientId(1), 1).unwrap().nodes.len(),
        2,
        "publication must keep commands issued after the frozen snapshot"
    );
    for id in [1, 2] {
        let bytes = serde_json::to_string(&c.surface(ClientId(1), id).unwrap()).unwrap();
        assert!(
            bytes.contains("__crysirror/frozen.png"),
            "copied snapshot was not substituted: {bytes}"
        );
        assert!(!bytes.contains("\"kind\":\"snapshot\""));
    }
    c.validate().unwrap();
}

#[test]
fn old_completion_cannot_discard_a_newer_materialization_request() {
    let (mut c, mut l) = fixture();
    c.apply_batch(ClientId(1), &vec![draw(); COMPACT_NODES + 1], &mut l)
        .unwrap();
    let old = c.materialize_requests().pop().unwrap();
    c.apply_batch(ClientId(1), &vec![draw(); COMPACT_NODES + 1], &mut l)
        .unwrap();
    let current = c.materialize_requests().pop().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&old.layers, &current.layers));
    c.resolve_materialized(&old, "__crysirror/older.png", &mut l)
        .unwrap();
    let pending = c.materialize_requests();
    assert_eq!(pending.len(), 1, "old result removed the newer request");
    assert!(std::sync::Arc::ptr_eq(&pending[0].layers, &current.layers));
    c.resolve_materialized(&current, "__crysirror/current.png", &mut l)
        .unwrap();
    assert!(c.materialize_requests().is_empty());
    let [CanvasNode::Draw { layer }] = c.surface(ClientId(1), 1).unwrap().nodes.as_slice() else {
        panic!("compacted surface must remain one draw");
    };
    assert!(matches!(&layer.paint,Paint::Image { asset,.. } if asset=="__crysirror/current.png"));
}
