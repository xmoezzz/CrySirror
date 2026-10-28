//! Minimal independent reproducers for stale Canvas presentation IDs after
//! recursive Layer removal. No VM, renderer or game assets are involved.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{CanvasCommand as C, CanvasSystem},
    layer::{LayerCommand as L, LayerSpec, LayerSystem},
};

fn create(id: u32) -> C {
    C::Create {
        id,
        width: 8,
        height: 8,
    }
}
fn present(id: u32, layer: u32, parent: Option<u32>) -> C {
    C::Present {
        id,
        layer,
        parent,
        order: 0,
        position: [0., 0.],
    }
}
fn root(id: u32) -> LayerSpec {
    serde_json::from_value(serde_json::json!({
        "id": id, "rect": [0,0,8,8], "paint": {"kind":"solid", "color":[0,0,0,0]}
    }))
    .unwrap()
}

#[test]
fn hiding_parent_presentation_does_not_fail_on_deleted_child_ids() {
    let client = ClientId(1);
    let mut canvases = CanvasSystem::default();
    let mut layers = LayerSystem::default();
    canvases
        .apply_batch(
            client,
            &[create(1), create(2), present(1, 10, None)],
            &mut layers,
        )
        .unwrap();
    canvases
        .apply_batch(client, &[present(2, 11, Some(10))], &mut layers)
        .unwrap();
    assert!(layers.get(client, 11).is_some());
    canvases
        .apply_batch(client, &[C::Hide { layer: 10 }], &mut layers)
        .expect(
            "hiding an ancestor must reconcile nested canvas bindings with recursive Layer removal",
        );
    assert!(layers.get(client, 10).is_none());
    assert!(layers.get(client, 11).is_none());
    assert!(canvases.surface(client, 1).is_some());
    assert!(canvases.surface(client, 2).is_some());
    canvases
        .apply_batch(
            client,
            &[C::Clear {
                id: 2,
                color: [255; 4],
            }],
            &mut layers,
        )
        .unwrap();
    assert!(
        layers.get(client, 11).is_none(),
        "drawing to a retained surface must not resurrect a hidden binding"
    );
}

#[test]
fn explicit_represent_after_ancestor_removal_rebuilds_from_current_layers() {
    let client = ClientId(1);
    let mut canvases = CanvasSystem::default();
    let mut layers = LayerSystem::default();
    layers
        .apply_batch(client, &[L::Put { layer: root(9) }])
        .unwrap();
    canvases
        .apply_batch(client, &[create(1), present(1, 10, Some(9))], &mut layers)
        .unwrap();
    layers.apply_batch(client, &[L::Remove { id: 9 }]).unwrap();
    assert!(layers.get(client, 10).is_none());
    layers
        .apply_batch(client, &[L::Put { layer: root(9) }])
        .unwrap();
    canvases
        .apply_batch(client, &[present(1, 10, Some(9))], &mut layers)
        .expect("explicit Present must rebuild after external removal without deleting stale IDs");
    assert!(layers.get(client, 10).is_some());
}

#[test]
fn hide_after_external_removal_only_tolerates_its_own_stale_generated_nodes() {
    let client = ClientId(1);
    let mut canvases = CanvasSystem::default();
    let mut layers = LayerSystem::default();
    canvases
        .apply_batch(client, &[create(1), present(1, 10, None)], &mut layers)
        .unwrap();
    layers.apply_batch(client, &[L::Remove { id: 10 }]).unwrap();
    canvases
        .apply_batch(client, &[C::Hide { layer: 10 }], &mut layers)
        .unwrap();
    assert!(canvases.surface(client, 1).is_some());
    assert!(
        layers.apply_batch(client, &[L::Remove { id: 10 }]).is_err(),
        "ordinary Layer Remove must still reject missing explicit targets"
    );
}
