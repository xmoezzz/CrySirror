// SPDX-License-Identifier: MPL-2.0
use crysirror_core::state::ClientId;
use crysirror_scene::layer::{
    LayerCommand, LayerSystem, ORDERED_DITHER_TRANSITION_CODE, TransitionEffect, TransitionSpec,
    transition_code_supported,
};
use serde_json::json;

#[test]
fn ordered_dither_is_a_typed_shared_effect_with_its_own_code_and_options() {
    let spec: TransitionSpec = serde_json::from_value(json!({"duration_ms":2000,
        "effect":{"kind":"ordered_dither","cell":[2,3],"diagonal":true,"steps":4}}))
    .unwrap();
    spec.validate().unwrap();
    let (code, options, mask) = spec.resolve();
    assert_eq!(code, ORDERED_DITHER_TRANSITION_CODE);
    assert_eq!(&options[..4], &[2, 3, 1, 4]);
    assert!(mask.is_none());
    assert!(transition_code_supported(code));
    let mut layers = LayerSystem::default();
    layers
        .apply_batch(
            ClientId(5),
            &[LayerCommand::Transition {
                transition: spec.clone(),
            }],
        )
        .unwrap();
    layers.tick(500);
    let frame = layers.transition_frame().unwrap();
    assert_eq!(frame.code, code);
    assert_eq!(frame.progress, 0.25);
    assert!(!frame.generated_mask);
    let roundtrip: LayerSystem =
        serde_json::from_value(serde_json::to_value(&layers).unwrap()).unwrap();
    roundtrip.validate().unwrap();
    assert_eq!(roundtrip.transition_frame().unwrap(), frame);
    let mut invalid = spec.clone();
    invalid.effect = TransitionEffect::Siglus {
        code,
        options: vec![2, 3, 1, 4],
        mask: None,
    };
    assert!(
        invalid.validate().is_err(),
        "new engine effect must not masquerade as a Siglus opcode"
    );
    for (cell, steps) in [
        ([0, 1], 1),
        ([1, 0], 1),
        ([8193, 1], 1),
        ([1, 1], 0),
        ([1, 1], 65_537),
    ] {
        let mut bad = spec.clone();
        bad.effect = TransitionEffect::OrderedDither {
            cell,
            diagonal: false,
            steps,
        };
        assert!(
            layers
                .apply_batch(ClientId(5), &[LayerCommand::Transition { transition: bad }])
                .is_err()
        );
        assert_eq!(
            layers.transition_frame().unwrap(),
            frame,
            "bad parameters must not replace the active effect"
        );
    }
}
