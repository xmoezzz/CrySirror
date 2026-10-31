// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Explicit retained-surface release is independent of native VMs/devices.
use crysirror_core::state::ClientId;
use crysirror_scene::{
    canvas::{CanvasCommand as C, CanvasSystem},
    layer::LayerSystem,
};

fn commands(value: serde_json::Value) -> Vec<C> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn destroy_is_scoped_strict_atomic_and_preserves_frozen_copy_nodes() {
    let mut canvases = CanvasSystem::default();
    let mut layers = LayerSystem::default();
    let owner = ClientId(1);
    canvases.apply_batch(owner,&commands(serde_json::json!([
        {"op":"create","id":1,"width":2,"height":2},
        {"op":"set","id":1,"layer":{"id":0,"rect":[0,0,2,2],"paint":{"kind":"image","asset":"__image_resource/authored"}}},
        {"op":"create","id":2,"width":2,"height":2},
        {"op":"copy","source":1,"destination":2,"rect":[0,0,2,2],"position":[0,0],"opacity":255,"replace":true},
        {"op":"present","id":1,"layer":10}
    ])),&mut layers).unwrap();
    let frozen = canvases.clone();
    let copy = serde_json::to_value(canvases.surface(owner, 2)).unwrap();
    assert!(copy.to_string().contains("__image_resource/authored"));
    // Another owner cannot release it, nor can an ordinary caller invalidate
    // an active presentation. Earlier operations roll back with rejection.
    assert!(canvases
        .apply_batch(ClientId(2), &[C::Destroy { id: 1 }], &mut layers)
        .is_err());
    assert!(canvases
        .apply_batch(
            owner,
            &[
                C::Clear {
                    id: 2,
                    color: [0, 0, 0, 0]
                },
                C::Destroy { id: 1 }
            ],
            &mut layers
        )
        .unwrap_err()
        .to_string()
        .contains("presented canvas"));
    assert_eq!(
        serde_json::to_value(canvases.surface(owner, 2)).unwrap(),
        copy
    );
    assert!(layers.get(owner, 10).is_some());
    canvases
        .apply_batch(
            owner,
            &[C::Hide { layer: 10 }, C::Destroy { id: 1 }],
            &mut layers,
        )
        .unwrap();
    assert!(canvases.surface(owner, 1).is_none());
    assert!(layers.get(owner, 10).is_none());
    assert_eq!(
        serde_json::to_value(canvases.surface(owner, 2)).unwrap(),
        copy
    );
    assert!(
        frozen.surface(owner, 1).is_some(),
        "admitted immutable views retain their source generation"
    );
    assert!(canvases
        .apply_batch(owner, &[C::Destroy { id: 1 }], &mut layers)
        .is_err());
    // A new logical surface may reuse the numeric id after retirement.
    canvases
        .apply_batch(
            owner,
            &[C::Create {
                id: 1,
                width: 1,
                height: 1,
            }],
            &mut layers,
        )
        .unwrap();
    assert_eq!(canvases.surface(owner, 1).unwrap().width, 1);
    assert_eq!(
        serde_json::to_value(canvases.surface(owner, 2)).unwrap(),
        copy
    );
}
