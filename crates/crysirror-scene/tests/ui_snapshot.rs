use crysirror_core::state::ClientId;
use crysirror_scene::{
    layer::LayerSystem,
    ui::{UiCommand, UiSystem},
};
use serde_json::json;
fn apply(ui: &mut UiSystem, owner: u32, commands: serde_json::Value) {
    let commands: Vec<UiCommand> = serde_json::from_value(commands).unwrap();
    ui.apply_batch(ClientId(owner), &commands).unwrap();
}
#[test]
fn snapshot_freezes_current_text_style_clip_and_order_without_another_owner() {
    let mut ui = UiSystem::default();
    apply(
        &mut ui,
        1,
        json!([
            {"op":"put","widget":{"id":1,"rect":[10,20,200,100],"clip_children":true,"color":[12,34,56,255]}},
            {"op":"put","widget":{"id":2,"parent":1,"rect":[5,8,100,40],"kind":"text_field","font":"project-font","text_size":28,"text_color":[5,7,9,255],"text_inset":[2,1],"text":"old","z":4}},
            {"op":"put","widget":{"id":3,"parent":1,"rect":[150,80,100,40],"kind":"button","font":"project-font","text":"button","z":5}},
            {"op":"focus","id":2},{"op":"insert_text","text":"日本"}
        ]),
    );
    apply(
        &mut ui,
        2,
        json!([{"op":"put","widget":{"id":2,"rect":[0,0,32,32],"kind":"label","font":"secret-font","text":"private-other-client"}}]),
    );
    let frozen = ui.snapshot_layers(1, [320, 200], Some(1)).unwrap();
    let text = serde_json::to_string(&frozen).unwrap();
    assert!(text.contains("old日本"));
    assert!(!text.contains("private-other-client") && !text.contains("secret-font"));
    let mut expected = ui
        .frame([320, 200])
        .unwrap()
        .into_iter()
        .filter(|p| p.owner == 1)
        .collect::<Vec<_>>();
    expected.sort_by_key(|p| (p.order, p.owner, p.id, p.sub));
    let mut layers = LayerSystem::default();
    layers.install(ClientId(1), &frozen).unwrap();
    let frame = layers.frame([320, 200]).unwrap();
    assert_eq!(frame.len(), expected.len());
    for (a, b) in frame.iter().zip(&expected) {
        assert_eq!(a.quad, b.quad);
        assert_eq!(a.clip, b.clip);
        assert_eq!(a.order, b.order);
        assert_eq!(a.opacity, b.opacity);
        assert_eq!(a.blend, b.blend);
        assert_eq!(a.uv, b.uv);
        assert_eq!(
            serde_json::to_value(&a.paint).unwrap(),
            serde_json::to_value(&b.paint).unwrap()
        );
    }
    apply(&mut ui, 1, json!([{"op":"insert_text","text":"new"}]));
    assert_eq!(serde_json::to_string(&frozen).unwrap(), text);
}
#[test]
fn root_scope_is_not_another_owner_and_full_owner_snapshot_excludes_hidden_widgets() {
    let mut ui = UiSystem::default();
    apply(
        &mut ui,
        1,
        json!([{"op":"put","widget":{"id":1,"rect":[0,0,32,32]}},{"op":"put","widget":{"id":2,"rect":[0,0,32,32],"visible":false}}]),
    );
    apply(
        &mut ui,
        2,
        json!([{"op":"put","widget":{"id":9,"rect":[0,0,32,32]}}]),
    );
    assert!(ui.snapshot_layers(1, [64, 64], Some(9)).is_err());
    assert_eq!(ui.snapshot_layers(1, [64, 64], None).unwrap().len(), 1);
    assert!(ui.snapshot_layers(1, [0, 64], None).is_err());
}
#[test]
fn press_activation_is_engine_owned_and_release_does_not_activate_twice() {
    use crysirror_core::service::{buttons, InputFrame};
    let mut ui = UiSystem::default();
    apply(
        &mut ui,
        1,
        json!([{"op":"put","widget":{"id":4,"rect":[10,10,30,30],"kind":"button","activate_on_press":true}}]),
    );
    ui.tick(
        InputFrame {
            pointer_x: 20,
            pointer_y: 20,
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [64, 64],
    )
    .unwrap();
    let events = ui.drain_events(1, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "activate");
    ui.tick(
        InputFrame {
            pointer_x: 20,
            pointer_y: 20,
            ..Default::default()
        },
        [64, 64],
    )
    .unwrap();
    assert!(ui.drain_events(1, None).is_empty());
    let command:UiCommand=serde_json::from_value(json!({"op":"put","widget":{"id":5,"rect":[0,0,16,16],"kind":"text_field","activate_on_press":true}})).unwrap();
    assert!(ui.apply_batch(ClientId(1), &[command]).is_err());
}
