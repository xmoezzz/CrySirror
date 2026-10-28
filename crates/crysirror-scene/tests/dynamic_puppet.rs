use crysirror_core::state::ClientId;
use crysirror_puppet::PuppetFrame;
use crysirror_scene::layer::{LayerCommand, LayerSpec, LayerSystem};
use std::sync::Arc;
fn layer() -> LayerSpec {
    serde_json::from_str(
        r#"{"id":7,"rect":[0,0,32,16],"paint":{"kind":"solid","color":[0,0,0,0]}}"#,
    )
    .unwrap()
}
#[test]
fn dynamic_mesh_publication_is_owned_derived_and_validated() {
    let mut layers = LayerSystem::default();
    let client = ClientId(1);
    layers
        .apply_batch(client, &[LayerCommand::Put { layer: layer() }])
        .unwrap();
    let frame = Arc::new(PuppetFrame {
        size: [32., 16.],
        ..Default::default()
    });
    assert!(layers.set_dynamic_puppet(2, 7, frame.clone()).is_err());
    layers.set_dynamic_puppet(1, 7, frame.clone()).unwrap();
    assert_eq!(
        layers.frame([32, 16]).unwrap()[0].puppet,
        Some(frame.clone())
    );
    layers
        .apply_batch(client, &[LayerCommand::Put { layer: layer() }])
        .unwrap();
    assert_eq!(layers.frame([32, 16]).unwrap()[0].puppet, Some(frame));
    assert!(layers
        .set_dynamic_puppet(1, 7, Arc::new(PuppetFrame::default()))
        .is_err());
    let restored: LayerSystem =
        serde_json::from_slice(&serde_json::to_vec(&layers).unwrap()).unwrap();
    assert!(restored.frame([32, 16]).unwrap()[0].puppet.is_none());
    layers
        .apply_batch(client, &[LayerCommand::Remove { id: 7 }])
        .unwrap();
    assert!(layers.frame([32, 16]).unwrap().is_empty());
}
