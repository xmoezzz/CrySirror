// SPDX-License-Identifier: MPL-2.0
//! Shared retained text state tested without any VM, font or device.
use crysirror_core::state::ClientId;
use crysirror_scene::layer::{LayerCommand, LayerSpec, LayerSystem, Paint};
use serde_json::json;

fn spec() -> LayerSpec {
    serde_json::from_value(json!({"id":7,"rect":[10.,20.,200.,80.],"clip":[0.,0.,180.,80.],
        "paint":{"kind":"glyph_run","font":"project-font","size":24.,"color":[255,255,255,255],
            "glyphs":[{"character":"字","baseline":[2.5,30.]},
                {"character":"A","baseline":[42.5,30.],"size":30.,"color":[255,0,0,255],"bold":true}]}})).unwrap()
}
#[test]
fn glyph_runs_are_inspectable_scoped_retained_and_saved_state() {
    let mut layers = LayerSystem::default();
    layers.install(ClientId(1), &[spec()]).unwrap();
    layers.install(ClientId(2), &[spec()]).unwrap();
    let snapshot = serde_json::to_value(&layers).unwrap();
    assert_eq!(
        snapshot["clients"]["1"]["7"]["paint"]["glyphs"][0]["character"],
        "字"
    );
    let mut restored: LayerSystem = serde_json::from_value(snapshot.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), snapshot);
    let frame = restored.frame([400, 300]).unwrap();
    assert_eq!(frame.len(), 2);
    let Paint::GlyphRun { glyphs, .. } = &frame[0].paint else {
        panic!("run was flattened")
    };
    assert_eq!(glyphs.len(), 2);
    assert_eq!(glyphs[0].baseline, [2.5, 30.]);
    restored.set_visible(ClientId(1), 7, false).unwrap();
    let frame = restored.frame([400, 300]).unwrap();
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].owner, 2);
}
#[test]
fn malformed_glyph_data_rejects_without_partially_replacing_layers() {
    let mut layers = LayerSystem::default();
    layers.install(ClientId(1), &[spec()]).unwrap();
    let before = serde_json::to_value(&layers).unwrap();
    for size in [0., f32::NAN, 513.] {
        let mut bad = spec();
        if let Paint::GlyphRun { glyphs, .. } = &mut bad.paint {
            glyphs[0].size = Some(size);
        }
        assert!(
            layers
                .apply_batch(ClientId(1), &[LayerCommand::Put { layer: bad }])
                .is_err()
        );
        assert_eq!(serde_json::to_value(&layers).unwrap(), before);
    }
    let mut bad = spec();
    if let Paint::GlyphRun { glyphs, .. } = &mut bad.paint {
        glyphs[0].baseline[0] = f32::INFINITY;
    }
    assert!(
        layers
            .apply_batch(ClientId(1), &[LayerCommand::Put { layer: bad }])
            .is_err()
    );
    assert_eq!(serde_json::to_value(&layers).unwrap(), before);
    let mut bad = spec();
    bad.uv = Some([0., 0., 0.5, 0.5]);
    assert!(
        layers
            .apply_batch(ClientId(1), &[LayerCommand::Put { layer: bad }])
            .is_err()
    );
    assert!(
        serde_json::from_value::<Paint>(json!({"kind":"glyph_run","font":"f","size":24.,
        "glyphs":[{"character":"two","baseline":[0.,0.]}]}))
        .is_err()
    );
}

fn reveal_layer(initial_visible: usize, interval_ms: u64) -> LayerSpec {
    serde_json::from_value(json!({"id":7,"rect":[0,0,100,50],"paint":{
        "kind":"glyph_run","font":"project-font","size":24,
        "glyphs":[{"character":"A","baseline":[0,24]},
            {"character":"\u{301}","baseline":[12,24]},
            {"character":"B","baseline":[24,24]},
            {"character":"C","baseline":[36,24]}],
        "reveal":{"unit":"unicode_scalar","initial_visible":initial_visible,"interval_ms":interval_ms}
    }})).unwrap()
}

#[test]
fn shared_engine_clock_reveals_scalars_preserves_page_prefix_and_roundtrips_progress() {
    use crysirror_core::service::InputFrame;
    use crysirror_scene::{EngineSystems, SystemsSpec};
    let spec = SystemsSpec {
        layers: vec![reveal_layer(1, 20)],
        ..Default::default()
    };
    let mut engine = EngineSystems::default();
    engine.install(ClientId(1), &spec).unwrap();
    assert_eq!(
        engine.layers.reveal_status(ClientId(1), 7).unwrap().visible,
        1
    );
    engine.tick(19, InputFrame::default(), [100, 50]).unwrap();
    assert_eq!(
        engine.layers.reveal_status(ClientId(1), 7).unwrap().visible,
        1
    );
    engine.tick(1, InputFrame::default(), [100, 50]).unwrap();
    let status = engine.layers.reveal_status(ClientId(1), 7).unwrap();
    assert_eq!(
        (status.visible, status.total, status.complete),
        (2, 4, false),
        "the combining scalar is its own explicit unit, not a grapheme merge"
    );
    let mut restored: EngineSystems =
        serde_json::from_value(serde_json::to_value(&engine).unwrap()).unwrap();
    restored.validate().unwrap();
    engine.tick(40, InputFrame::default(), [100, 50]).unwrap();
    for _ in 0..4 {
        restored.tick(10, InputFrame::default(), [100, 50]).unwrap();
    }
    assert_eq!(
        serde_json::to_value(&engine).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );
    assert_eq!(
        restored.layers.reveal_status(ClientId(1), 7).unwrap(),
        crysirror_scene::layer::GlyphRevealStatus {
            visible: 4,
            total: 4,
            complete: true
        }
    );
}

#[test]
fn complete_reveal_is_scoped_and_works_when_paused_without_a_private_clock() {
    let mut layers = LayerSystem::default();
    let mut layer = reveal_layer(2, 20);
    if let Paint::GlyphRun {
        reveal: Some(r), ..
    } = &mut layer.paint
    {
        r.paused = true;
    }
    layers.install(ClientId(1), &[layer.clone()]).unwrap();
    layers.install(ClientId(2), &[layer]).unwrap();
    layers.tick(10_000);
    assert_eq!(layers.reveal_status(ClientId(1), 7).unwrap().visible, 2);
    layers
        .apply_batch(ClientId(1), &[LayerCommand::CompleteReveal { id: 7 }])
        .unwrap();
    assert!(layers.reveal_status(ClientId(1), 7).unwrap().complete);
    assert_eq!(layers.reveal_status(ClientId(2), 7).unwrap().visible, 2);
    let before = serde_json::to_value(&layers).unwrap();
    assert!(
        layers
            .apply_batch(
                ClientId(1),
                &[
                    LayerCommand::SetOpacity { id: 7, opacity: 2 },
                    LayerCommand::Put {
                        layer: reveal_layer(5, 20)
                    }
                ]
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&layers).unwrap(), before);
    layers
        .apply_batch(
            ClientId(1),
            &[LayerCommand::Put {
                layer: reveal_layer(0, 0),
            }],
        )
        .unwrap();
    assert!(layers.reveal_status(ClientId(1), 7).unwrap().complete);
}
