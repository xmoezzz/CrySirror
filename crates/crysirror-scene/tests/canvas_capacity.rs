// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. See https://mozilla.org/MPL/2.0/.
//! Logical surface capacity and rollback are usable without any native VM.
use crysirror_core::{service::MAX_CANVAS_SURFACES_PER_CLIENT, state::ClientId};
use crysirror_scene::{canvas::{CanvasCommand as C,CanvasSystem,MAX_SURFACES},layer::LayerSystem};

fn create(id:u32,width:u32)->C {C::Create{id,width,height:1}}

#[test]
fn shared_capacity_is_per_client_and_overflow_is_atomic() {
    assert_eq!(MAX_SURFACES,MAX_CANVAS_SURFACES_PER_CLIENT);
    let mut canvases=CanvasSystem::default();
    let mut layers=LayerSystem::default();
    let full=(1..=MAX_SURFACES as u32).map(|id|create(id,1)).collect::<Vec<_>>();
    canvases.apply_batch(ClientId(1),&full,&mut layers).unwrap();
    assert!(canvases.surface(ClientId(1),MAX_SURFACES as u32).is_some());
    let before=serde_json::to_value(canvases.surface(ClientId(1),1)).unwrap();
    assert!(canvases.apply_batch(ClientId(1),&[
        C::Clear{id:1,color:[1,2,3,255]},create(MAX_SURFACES as u32+1,1)
    ],&mut layers).unwrap_err().to_string().contains("canvas surface limit"));
    assert_eq!(serde_json::to_value(canvases.surface(ClientId(1),1)).unwrap(),before);
    assert!(canvases.surface(ClientId(1),MAX_SURFACES as u32+1).is_none());
    // Reusing a slot at full capacity does not consume another admission.
    canvases.apply_batch(ClientId(1),&[create(1,2)],&mut layers).unwrap();
    assert_eq!(canvases.surface(ClientId(1),1).unwrap().width,2);
    canvases.apply_batch(ClientId(2),&full,&mut layers).unwrap();
    assert_eq!(canvases.surface(ClientId(2),1).unwrap().width,1);
    let restored:CanvasSystem=serde_json::from_value(serde_json::to_value(&canvases).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored.surface(ClientId(1),1).unwrap().width,2);
}
