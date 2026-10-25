// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_scene::{
    layer::{Blend, LayerSystem, Paint},
    ui::{UiSystem, WidgetSpec},
    world::{Property, Transform, Tween},
    EngineSystems, SystemsSpec, WorldCommand,
};
use serde_json::json;
fn spec() -> SystemsSpec {
    serde_json::from_value(json!({"entities":[{"id":1,"name":"parent","geometry":{"kind":"cube"},"velocity":[1,0,0]},{"id":2,"name":"child","parent":1,"transform":{"translation":[2,0,0]},"geometry":{"kind":"cube"}}]})).unwrap()
}
#[test]
fn initial_environment_is_scoped_validated_transactional_and_saved() {
    let mut systems = EngineSystems::default();
    let initial: SystemsSpec =
        serde_json::from_value(json!({"environment":{"intensity":0.7,"sun":{"intensity":5}}}))
            .unwrap();
    systems.install(ClientId(1), &initial).unwrap();
    systems
        .install(ClientId(2), &SystemsSpec::default())
        .unwrap();
    assert_eq!(systems.world.view.environment.as_ref().unwrap().0, 1);
    let before = serde_json::to_value(&systems).unwrap();
    let invalid:SystemsSpec=serde_json::from_value(json!({"entities":[{"id":7,"name":"rollback"}],"environment":{"sun":{"direction":[0,0,0]}}})).unwrap();
    assert!(systems.install(ClientId(3), &invalid).is_err());
    assert_eq!(serde_json::to_value(&systems).unwrap(), before);
    systems.install(ClientId(2), &initial).unwrap();
    let restored: EngineSystems =
        serde_json::from_value(serde_json::to_value(&systems).unwrap()).unwrap();
    restored.validate().unwrap();
    let (owner, env) = restored.world.view.environment.unwrap();
    assert_eq!(owner, 2);
    assert_eq!(env.intensity, 0.7);
    assert_eq!(env.sun.unwrap().intensity, 5.);
}
#[test]
fn world_scoping_generation_hierarchy_and_atomic_rollback() {
    let mut e = EngineSystems::default();
    e.install(ClientId(1), &spec()).unwrap();
    e.install(ClientId(2), &spec()).unwrap();
    let h = e.world.handle(ClientId(1), 2).unwrap();
    assert_eq!(e.world.global_matrix(h).unwrap().w_axis.x, 2.);
    let before = serde_json::to_value(&e).unwrap();
    assert!(e
        .world
        .apply_batch(
            ClientId(1),
            &[WorldCommand::SetParent {
                id: 1,
                parent: Some(2)
            }]
        )
        .is_err());
    assert_eq!(serde_json::to_value(&e).unwrap(), before);
    e.world
        .apply_batch(ClientId(1), &[WorldCommand::Despawn { id: 1 }])
        .unwrap();
    assert!(e.world.entity(h).is_err());
    assert!(e.world.handle(ClientId(2), 2).is_ok());
    e.world.install(ClientId(1), &spec().entities).unwrap();
    assert_ne!(
        e.world.handle(ClientId(1), 2).unwrap().generation,
        h.generation
    );
    assert!(e.world.entity(h).is_err());
}
#[test]
fn fixed_step_chunking_and_state_roundtrip() {
    let mut a = EngineSystems::default();
    a.install(ClientId(1), &spec()).unwrap();
    let mut b = a.clone();
    a.tick(1000, InputFrame::default(), [100, 100]).unwrap();
    for _ in 0..100 {
        b.tick(10, InputFrame::default(), [100, 100]).unwrap();
    }
    let h = a.world.handle(ClientId(1), 1).unwrap();
    assert_eq!(
        a.world.entity(h).unwrap().transform.translation,
        b.world.entity(h).unwrap().transform.translation
    );
    assert_eq!(a.world.elapsed_ms, 992);
    let mut restored: EngineSystems =
        serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
    restored.validate().unwrap();
    a.tick(24, InputFrame::default(), [100, 100]).unwrap();
    restored
        .tick(24, InputFrame::default(), [100, 100])
        .unwrap();
    assert_eq!(
        serde_json::to_value(a).unwrap(),
        serde_json::to_value(restored).unwrap()
    );
}
#[test]
fn game_pause_keeps_real_time_animation_running() {
    let mut e = EngineSystems::default();
    let mut s = spec();
    s.entities.truncate(1);
    s.entities[0].velocity = [0.; 3];
    s.entities[0].tweens = vec![
        Tween {
            property: Property::X,
            from: 0,
            to: 1000,
            duration_ms: 1000,
            delay_ms: 0,
            mode: 0,
            easing: 0,
            real_time: true,
        },
        Tween {
            property: Property::Y,
            from: 0,
            to: 1000,
            duration_ms: 1000,
            delay_ms: 0,
            mode: 0,
            easing: 0,
            real_time: false,
        },
    ];
    e.install(ClientId(1), &s).unwrap();
    e.world
        .apply_batch(ClientId(1), &[WorldCommand::Pause { paused: true }])
        .unwrap();
    e.tick(512, InputFrame::default(), [100, 100]).unwrap();
    let t = &e
        .world
        .entity(e.world.handle(ClientId(1), 1).unwrap())
        .unwrap()
        .transform;
    assert!((t.translation[0] - 0.512).abs() < 1e-5);
    assert_eq!(t.translation[1], 0.);
}
#[test]
fn negative_parent_scale_retains_face_orientation() {
    let mut e = EngineSystems::default();
    e.install(ClientId(1), &spec()).unwrap();
    e.world
        .apply_batch(
            ClientId(1),
            &[WorldCommand::SetTransform {
                id: 1,
                transform: Transform {
                    scale: [-1., 1., 1.],
                    ..Transform::default()
                },
            }],
        )
        .unwrap();
    let scene = e.world.render_scene(None).unwrap().unwrap();
    if let crysirror_scene::Geometry::Triangles { vertices, indices } = &scene.meshes[0].geometry {
        assert_eq!(indices[0..3], [4, 6, 5]);
        assert_eq!(vertices[4][0], 0.5);
    } else {
        panic!("world extraction must bake parent transforms")
    }
}
fn widgets(modal: bool) -> Vec<WidgetSpec> {
    serde_json::from_value(json!([
 {"id":1,"rect":[0,0,100,100],"kind":"panel","flow":"column","padding":10,"gap":5,"clip_children":true,"modal":modal},
 {"id":2,"parent":1,"rect":[0,0,80,20],"kind":"button"},
 {"id":3,"parent":1,"rect":[0,0,80,20],"kind":"toggle"},
 {"id":4,"parent":1,"rect":[0,0,80,10],"kind":"slider","value":0.5}
])).unwrap()
}
#[test]
fn ui_capture_release_focus_modal_and_slider() {
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &widgets(false)).unwrap();
    let f = |x, y, down, pressed| InputFrame {
        pointer_x: x,
        pointer_y: y,
        buttons_down: down,
        buttons_pressed: pressed,
    };
    ui.tick(f(15, 15, 1 << 16, 1 << 16), [200, 200]).unwrap();
    assert!(ui.events.is_empty());
    ui.tick(f(150, 150, 0, 0), [200, 200]).unwrap();
    assert!(ui.events.is_empty(), "release outside cannot activate");
    ui.tick(f(15, 15, 1 << 16, 1 << 16), [200, 200]).unwrap();
    ui.tick(f(15, 15, 0, 0), [200, 200]).unwrap();
    assert_eq!(ui.events.len(), 1);
    assert_eq!(ui.events[0].widget.id, 2);
    ui.tick(f(-1, -1, 0, 64), [200, 200]).unwrap();
    ui.tick(f(-1, -1, 0, 1), [200, 200]).unwrap();
    assert_eq!(ui.events.last().unwrap().widget.id, 3);
    assert_eq!(ui.events.last().unwrap().value, 1.);
    ui.tick(f(70, 63, 1 << 16, 1 << 16), [200, 200]).unwrap();
    assert_eq!(ui.events.last().unwrap().widget.id, 4);
    assert!((ui.events.last().unwrap().value - 0.75).abs() < 1e-6);
    ui.install(ClientId(2), &widgets(true)).unwrap();
    let (_, consumed) = ui.tick(f(180, 180, 0, 0), [200, 200]).unwrap();
    assert!(consumed);
    ui.tick(f(-1, -1, 0, 1), [200, 200]).unwrap();
    assert_eq!(ui.events.last().unwrap().widget.owner, 2);
    let saved = serde_json::to_value(&ui).unwrap();
    let mut restored: UiSystem = serde_json::from_value(saved).unwrap();
    restored.validate().unwrap();
    let count = restored.events.len();
    restored.tick(f(15, 15, 0, 0), [200, 200]).unwrap();
    assert_eq!(
        restored.events.len(),
        count,
        "restore must discard pointer capture"
    );
}
#[test]
fn layers_inherit_visibility_opacity_transform_and_rotated_clip() {
    let layers:Vec<crysirror_scene::LayerSpec>=serde_json::from_value(json!([
 {"id":1,"rect":[20,20,100,100],"paint":{"kind":"solid"},"opacity":128,"clip":[0,0,100,100],"transform":{"rotate":0.7853982}},
 {"id":2,"parent":1,"rect":[20,20,40,40],"paint":{"kind":"solid","color":[255,0,0,255]},"order":[1,0],"opacity":128,"blend":"add"}
 ])).unwrap();
    let mut system = LayerSystem::default();
    system.install(ClientId(1), &layers).unwrap();
    let f = system.frame([400, 400]).unwrap();
    assert_eq!(f.len(), 2);
    let child = &f[1];
    assert_eq!(child.opacity, 64);
    assert_eq!(child.blend, Blend::Add);
    assert!((child.quad[0][0] - 20.).abs() < 1e-4);
    assert!(
        child.clip.unwrap()[2] > 70.,
        "all four rotated clip corners must be used"
    );
    system.set_visible(ClientId(1), 1, false).unwrap();
    assert!(system.frame([400, 400]).unwrap().is_empty());
}
#[test]
fn invalid_ui_world_action_rejected_during_install() {
    let mut e = EngineSystems::default();
    let mut s = spec();
    s.widgets = widgets(false);
    s.widgets[1].on_activate = vec![WorldCommand::SetVisible {
        id: 99,
        visible: false,
    }];
    assert!(e.install(ClientId(1), &s).is_err());
    assert!(e.world.is_empty());
}
#[test]
fn layer_text_unsupported_native_layout_fails_visibly() {
    assert!(Paint::Text {
        text: "x".into(),
        font: "default".into(),
        size: 16,
        color: [255; 4],
        layout: "siglus-ruby".into()
    }
    .validate()
    .is_err());
}
#[test]
fn retained_mutations_are_scoped_transactional_and_cascade() {
    use crysirror_scene::{
        layer::{LayerCommand, LayerSpec},
        ui::UiCommand,
    };
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &widgets(false)).unwrap();
    ui.tick(InputFrame::default(), [100, 100]).unwrap();
    let before = serde_json::to_value(&ui).unwrap();
    assert!(ui
        .apply_batch(
            ClientId(1),
            &[UiCommand::SetValue {
                id: 4,
                value: f32::NAN
            }]
        )
        .is_err());
    assert_eq!(serde_json::to_value(&ui).unwrap(), before);
    assert!(ui
        .apply_batch(ClientId(2), &[UiCommand::Remove { id: 1 }])
        .is_err());
    ui.apply_batch(ClientId(1), &[UiCommand::Remove { id: 1 }])
        .unwrap();
    assert!(ui.frame([100, 100]).unwrap().is_empty());
    assert!(ui.focus.is_none());
    let s: LayerSpec =
        serde_json::from_value(json!({"id":1,"rect":[0,0,20,20],"paint":{"kind":"solid"}}))
            .unwrap();
    let mut layers = LayerSystem::default();
    layers
        .apply_batch(ClientId(1), &[LayerCommand::Put { layer: s.clone() }])
        .unwrap();
    let before = serde_json::to_value(&layers).unwrap();
    let mut bad = s;
    bad.parent = Some(1);
    assert!(layers
        .apply_batch(ClientId(1), &[LayerCommand::Put { layer: bad }])
        .is_err());
    assert_eq!(serde_json::to_value(layers).unwrap(), before);
}
#[test]
fn held_pointer_after_restore_requires_release_before_activation() {
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &widgets(false)).unwrap();
    ui.reset_input_after_restore();
    let held = InputFrame {
        buttons_down: 1 << 16,
        pointer_x: 15,
        pointer_y: 15,
        ..InputFrame::default()
    };
    ui.tick(held, [100, 100]).unwrap();
    ui.tick(
        InputFrame {
            pointer_x: 15,
            pointer_y: 15,
            ..InputFrame::default()
        },
        [100, 100],
    )
    .unwrap();
    assert!(ui.events.is_empty());
}
#[test]
fn object_anchor_and_rotated_leaf_clips_follow_sprite_projection() {
    let specs:Vec<crysirror_scene::LayerSpec>=serde_json::from_value(json!([
 {"id":1,"rect":[100,100,80,80],"paint":{"kind":"solid"},"transform":{"object_anchor":true,"texture_center_x":40,"texture_center_y":40,"rotate":0.7853982},"clip":[0,0,80,80]},
 {"id":2,"parent":1,"rect":[0,0,10,10],"paint":{"kind":"solid"},"order":[1,0]}
 ])).unwrap();
    let mut layers = LayerSystem::default();
    layers.install(ClientId(1), &specs).unwrap();
    let f = layers.frame([400, 400]).unwrap();
    assert!((f[0].quad[0][0] - f[1].quad[0][0]).abs() < 1e-4);
    assert!((f[0].quad[0][1] - f[1].quad[0][1]).abs() < 1e-4);
    assert!(
        f[0].clip.unwrap()[2] > 110.,
        "leaf clip must follow all four rotated corners"
    );
    // Deserialized invalid trees must report errors rather than recurse forever.
    let mut v = serde_json::to_value(layers).unwrap();
    v["clients"]["1"]["1"]["parent"] = json!(2);
    let invalid: LayerSystem = serde_json::from_value(v).unwrap();
    assert!(invalid.frame([400, 400]).is_err());
}
