//! Shared sampler state is usable without a native VM or GPU.
use crysirror_core::state::ClientId;
use crysirror_scene::layer::{
    ImageSampling, LayerCommand, LayerSpec, LayerSystem, TextureAddressMode,
};
use serde_json::json;
fn layer() -> LayerSpec {
    serde_json::from_value(json!({"id":1,"rect":[0,0,16,8],"paint":{"kind":"image","asset":"atlas"},"uv":[-1,0,2,1],"sampling":{"address_u":"repeat"}})).unwrap()
}
#[test]
fn explicit_repeat_is_axis_scoped_and_survives_engine_state() {
    let mut engine = LayerSystem::default();
    let layer = layer();
    engine.install(ClientId(1), &[layer.clone()]).unwrap();
    let frame = engine.frame([16, 8]).unwrap();
    assert_eq!(frame[0].sampling.address_u, TextureAddressMode::Repeat);
    assert_eq!(frame[0].sampling.address_v, TextureAddressMode::ClampToEdge);
    assert_eq!(frame[0].uv, [-1., 0., 2., 1.]);
    let restored: LayerSystem =
        serde_json::from_slice(&serde_json::to_vec(&engine).unwrap()).unwrap();
    assert_eq!(
        restored.frame([16, 8]).unwrap()[0].sampling,
        frame[0].sampling
    );
    let mut invalid = layer;
    invalid.uv = Some([-1., -0.01, 2., 1.]);
    let before = serde_json::to_value(&engine).unwrap();
    assert!(engine
        .apply_batch(ClientId(1), &[LayerCommand::Put { layer: invalid }])
        .is_err());
    assert_eq!(serde_json::to_value(&engine).unwrap(), before);
}
#[test]
fn default_contract_and_nonimage_rejection_remain_explicit() {
    let mut engine = LayerSystem::default();
    let mut image = layer();
    image.sampling = ImageSampling::default();
    assert!(engine.install(ClientId(1), &[image]).is_err());
    let mut solid:LayerSpec=serde_json::from_value(json!({"id":1,"rect":[0,0,16,8],"paint":{"kind":"solid"},"sampling":{"address_u":"repeat"}})).unwrap();
    assert!(engine.install(ClientId(1), &[solid.clone()]).is_err());
    solid.sampling = ImageSampling::default();
    engine.install(ClientId(1), &[solid]).unwrap();
    let sampling = ImageSampling {
        address_u: TextureAddressMode::Repeat,
        address_v: TextureAddressMode::MirrorRepeat,
    };
    for uv in [
        [f32::NAN, 0., 1., 1.],
        [-4097., 0., 1., 1.],
        [0., 0., 1., 4097.],
        [1., 0., 0., 1.],
    ] {
        assert!(!sampling.valid_uv(uv));
    }
}
