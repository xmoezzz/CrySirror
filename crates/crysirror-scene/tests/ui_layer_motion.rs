use crysirror_core::{
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_scene::{
    layer::{LayerCommand, LayerSystem, MotionProperty},
    ui::{UiCommand, UiSystem},
    EngineSystems, ServiceBatch,
};
use serde_json::{json, Value};
fn layer(l: &mut LayerSystem, o: u32, c: Value) {
    let c: Vec<LayerCommand> = serde_json::from_value(c).unwrap();
    l.apply_batch(ClientId(o), &c).unwrap();
}
fn ui(u: &mut UiSystem, o: u32, c: Value) {
    let c: Vec<UiCommand> = serde_json::from_value(c).unwrap();
    u.apply_batch(ClientId(o), &c).unwrap();
}
#[test]
fn caller_clock_owner_isolation_finish_cancel_and_serialized_continuation() {
    let mut l = LayerSystem::default();
    for owner in [1, 2] {
        layer(
            &mut l,
            owner,
            json!([{"op":"put","layer":{"id":1,"rect":[1280,0,20,20],"paint":{"kind":"solid","color":[1,2,3,255]}}}]),
        );
    }
    layer(
        &mut l,
        1,
        json!([{"op":"begin_motion","id":1,"track":{"property":"x","to":0,"duration":20,"external":true}},{"op":"begin_motion","id":1,"track":{"property":"scale_x","to":1.2,"duration":15,"external":true}}]),
    );
    layer(
        &mut l,
        1,
        json!([{"op":"set_motion_elapsed","id":1,"property":"x","elapsed":5},{"op":"set_motion_elapsed","id":1,"property":"scale_x","elapsed":5}]),
    );
    assert_eq!(l.get(ClientId(1), 1).unwrap().rect[0], 960.);
    assert_eq!(l.get(ClientId(2), 1).unwrap().rect[0], 1280.);
    let before = serde_json::to_vec(&l).unwrap();
    layer(
        &mut l,
        1,
        json!([{"op":"set_motion_elapsed","id":1,"property":"x","elapsed":5}]),
    );
    assert_eq!(
        serde_json::to_vec(&l).unwrap(),
        before,
        "same elapsed is idempotent"
    );
    let bad = serde_json::from_value::<Vec<LayerCommand>>(
        json!([{"op":"set_motion_elapsed","id":1,"property":"x","elapsed":4}]),
    )
    .unwrap();
    assert!(l.apply_batch(ClientId(1), &bad).is_err());
    assert_eq!(serde_json::to_vec(&l).unwrap(), before);
    let mut restored: LayerSystem = serde_json::from_slice(&before).unwrap();
    restored.validate().unwrap();
    layer(
        &mut restored,
        1,
        json!([{"op":"set_motion_elapsed","id":1,"property":"x","elapsed":20},{"op":"end_motion","id":1,"property":"scale_x","finish":true}]),
    );
    assert_eq!(restored.get(ClientId(1), 1).unwrap().rect[0], 0.);
    assert_eq!(restored.get(ClientId(1), 1).unwrap().transform.scale_x, 1.2);
    assert!(restored.motion_status(ClientId(1), 1).unwrap().complete);
    layer(
        &mut restored,
        1,
        json!([{"op":"put","layer":{"id":1,"rect":[5,0,20,20],"paint":{"kind":"solid","color":[1,2,3,255]}}}]),
    );
    assert!(restored
        .motion_status(ClientId(1), 1)
        .unwrap()
        .tracks
        .is_empty());
    assert!(restored.motion_status(ClientId(3), 1).is_err());
}
#[test]
fn failed_outer_transaction_restores_geometry_and_clock() {
    let mut l = LayerSystem::default();
    layer(
        &mut l,
        1,
        json!([{"op":"put","layer":{"id":1,"rect":[0,0,20,20],"paint":{"kind":"solid","color":[1,2,3,255]}}},{"op":"begin_motion","id":1,"track":{"property":"x","to":100,"duration":100}}]),
    );
    let before = serde_json::to_value(&l).unwrap();
    let result: anyhow::Result<()> = l.transaction(|s| {
        s.tick(50);
        let cmds = serde_json::from_value::<Vec<LayerCommand>>(
            json!([{"op":"end_motion","id":1,"finish":true}]),
        )
        .unwrap();
        s.apply_batch(ClientId(1), &cmds)?;
        anyhow::bail!("rollback")
    });
    assert!(result.is_err());
    assert_eq!(serde_json::to_value(&l).unwrap(), before);
    l.tick(25);
    assert_eq!(l.get(ClientId(1), 1).unwrap().rect[0], 25.);
}
#[test]
fn moving_ui_parent_hit_test_uses_new_geometry_and_removal_cancels_descendants() {
    let mut e = EngineSystems::default();
    ui(
        &mut e.ui,
        1,
        json!([{"op":"put","widget":{"id":1,"rect":[100,0,100,100],"interaction_only":true}},{"op":"put","widget":{"id":2,"parent":1,"rect":[0,0,20,20],"kind":"button","interaction_only":true,"keyboard_focusable":false}},{"op":"begin_motion","id":1,"track":{"property":"x","to":0,"duration":100}}]),
    );
    e.tick(
        50,
        InputFrame {
            pointer_x: 55,
            pointer_y: 5,
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [200, 100],
    )
    .unwrap();
    assert!(
        e.ui.interaction(1, 2).unwrap().pressed,
        "advance before input layout"
    );
    e.tick(
        0,
        InputFrame {
            pointer_x: 55,
            pointer_y: 5,
            ..Default::default()
        },
        [200, 100],
    )
    .unwrap();
    assert!(e.ui.interaction(1, 2).unwrap().activated);
    ui(
        &mut e.ui,
        1,
        json!([{"op":"begin_motion","id":2,"track":{"property":"y","to":20,"duration":10}},{"op":"remove","id":1}]),
    );
    e.tick(10, InputFrame::default(), [200, 100]).unwrap();
    e.ui.validate().unwrap();
    assert!(e.ui.get(1, 2).is_none());
}
#[test]
fn invalid_scale_and_ui_scale_do_not_commit_partial_changes() {
    let mut e = EngineSystems::default();
    let put:ServiceBatch=serde_json::from_value(json!({"ui":[{"op":"put","widget":{"id":1,"rect":[0,0,20,20]}}],"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,20,20],"paint":{"kind":"solid","color":[1,2,3,255]}}}]})).unwrap();
    e.submit(ClientId(1), &put).unwrap();
    let c = serde_json::from_value::<Vec<UiCommand>>(
        json!([{"op":"begin_motion","id":1,"track":{"property":"scale_x","to":2,"duration":10}}]),
    )
    .unwrap();
    assert!(e.ui.apply_batch(ClientId(1), &c).is_err());
    let c = serde_json::from_value::<Vec<LayerCommand>>(
        json!([{"op":"begin_motion","id":1,"track":{"property":"scale_x","to":-1,"duration":10}}]),
    )
    .unwrap();
    assert!(e.layers.apply_batch(ClientId(1), &c).is_err());
    assert!(e
        .layers
        .motion_status(ClientId(1), 1)
        .unwrap()
        .tracks
        .is_empty());
    assert_eq!(MotionProperty::X, MotionProperty::X);
}

#[test]
fn linear_integer_pixel_trajectory_does_not_round_through_f32_progress() {
    let tween = crysirror_scene::animation::ScalarTween::new(
        1280.,
        0.,
        0,
        20,
        crysirror_scene::layer::Easing::Linear,
    )
    .unwrap();
    for tick in 0..=20 {
        assert_eq!(tween.sample(tick), (1280 - 64 * tick) as f32, "tick {tick}");
    }
}
