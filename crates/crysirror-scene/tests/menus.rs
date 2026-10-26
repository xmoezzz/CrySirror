// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_core::{
    service::{InputFrame, buttons},
    state::ClientId,
};
use crysirror_scene::ui::{UiCommand, UiSystem, menu::MenuSpec};
use serde_json::json;
fn menu() -> MenuSpec {
    serde_json::from_value(json!({"id":1,"rect":[0,0,640,24],"font":"default","items":[{"id":2,"label":"File","children":[{"id":3,"label":"Fast","radio":true,"group":1,"checked":true},{"id":4,"label":"Slow","radio":true,"group":1},{"id":5,"label":"Disabled","enabled":false}]}]})).unwrap()
}
fn click(ui: &mut UiSystem, x: i32, y: i32) {
    ui.tick(
        InputFrame {
            pointer_x: x,
            pointer_y: y,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [640, 480],
    )
    .unwrap();
}
#[test]
fn native_free_menu_layout_activation_and_radio_state_are_engine_owned() {
    let mut ui = UiSystem::default();
    ui.apply_batch(ClientId(1), &[UiCommand::SetMenu { menu: menu() }])
        .unwrap();
    assert_eq!(ui.frame([640, 480]).unwrap().len(), 2);
    click(&mut ui, 20, 10);
    assert!(ui.frame([640, 480]).unwrap().len() > 2);
    click(&mut ui, 20, 60);
    let event = ui.events.last().unwrap();
    assert_eq!(event.kind, "menu_activate");
    assert_eq!(event.widget.id, 4);
    assert_eq!(event.menu_bar, Some(1));
    let spec = ui.menus.get(1, 1).unwrap();
    assert!(!spec.items[0].children[0].checked);
    assert!(spec.items[0].children[1].checked);
    assert_eq!(ui.frame([640, 480]).unwrap().len(), 2);
    for _ in 0..260 {
        click(&mut ui, 20, 10);
        click(&mut ui, 20, 60);
    }
    assert_eq!(ui.events.len(), 256);
    ui.validate().unwrap();
    assert_eq!(ui.drain_events(1, true).len(), 256);
    assert!(ui.events.is_empty());
}
#[test]
fn bad_menu_updates_roll_back_and_contexts_stay_client_scoped() {
    let mut ui = UiSystem::default();
    ui.apply_batch(ClientId(1), &[UiCommand::SetMenu { menu: menu() }])
        .unwrap();
    let original = serde_json::to_value(&ui).unwrap();
    let mut bad = menu();
    bad.items[0].children[1].id = 3;
    assert!(
        ui.apply_batch(ClientId(1), &[UiCommand::SetMenu { menu: bad }])
            .is_err()
    );
    assert_eq!(serde_json::to_value(&ui).unwrap(), original);
    assert!(
        ui.apply_batch(
            ClientId(2),
            &[UiCommand::OpenMenu {
                id: 1,
                item: Some(2),
                position: None
            }]
        )
        .is_err()
    );
    let mut context = menu();
    context.id = 6;
    context.context = true;
    ui.apply_batch(ClientId(2), &[UiCommand::SetMenu { menu: context }])
        .unwrap();
    assert_eq!(ui.frame([640, 480]).unwrap().len(), 2);
    ui.apply_batch(
        ClientId(2),
        &[UiCommand::OpenMenu {
            id: 6,
            item: None,
            position: Some([20., 40.]),
        }],
    )
    .unwrap();
    assert!(ui.frame([640, 480]).unwrap().len() > 2);
    ui.tick(
        InputFrame {
            buttons_pressed: buttons::ESCAPE,
            ..Default::default()
        },
        [640, 480],
    )
    .unwrap();
    assert_eq!(ui.frame([640, 480]).unwrap().len(), 2);
    let mut engine = crysirror_scene::EngineSystems::default();
    engine.ui = ui;
    assert!(engine.validate_clients(&[1].into()).is_err());
    engine.validate_clients(&[1, 2].into()).unwrap();
    engine.ui.events.push(crysirror_scene::ui::UiEvent {
        widget: crysirror_scene::ui::WidgetId { owner: 999, id: 1 },
        kind: "menu_activate".into(),
        value: 0.,
        menu_bar: Some(6),
    });
    assert!(engine.validate_clients(&[1, 2].into()).is_err());
}
#[test]
fn menu_state_roundtrips_but_restore_discards_pressed_and_popup_input() {
    let mut ui = UiSystem::default();
    ui.apply_batch(ClientId(1), &[UiCommand::SetMenu { menu: menu() }])
        .unwrap();
    click(&mut ui, 20, 10);
    let mut restored: UiSystem = serde_json::from_slice(&serde_json::to_vec(&ui).unwrap()).unwrap();
    restored.reset_input_after_restore();
    assert_eq!(restored.frame([640, 480]).unwrap().len(), 2);
    assert!(restored.menus.get(1, 1).unwrap().items[0].children[0].checked);
    let mut malformed = serde_json::to_value(&ui).unwrap();
    malformed["menus"]["clients"]["1"]["1"]["open"] = json!([999]);
    let malformed: UiSystem = serde_json::from_value(malformed).unwrap();
    assert!(malformed.validate().is_err());
}

#[test]
fn dismissing_a_menu_does_not_synthesize_a_second_press_from_a_held_button() {
    let mut ui = UiSystem::default();
    ui.apply_batch(ClientId(1), &[UiCommand::SetMenu { menu: menu() }])
        .unwrap();
    click(&mut ui, 20, 10);
    ui.tick(
        InputFrame {
            pointer_x: 600,
            pointer_y: 400,
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [640, 480],
    )
    .unwrap();
    ui.tick(
        InputFrame {
            pointer_x: 20,
            pointer_y: 10,
            buttons_down: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [640, 480],
    )
    .unwrap();
    ui.tick(
        InputFrame {
            pointer_x: 20,
            pointer_y: 10,
            ..Default::default()
        },
        [640, 480],
    )
    .unwrap();
    assert_eq!(ui.frame([640, 480]).unwrap().len(), 2);
    assert!(ui.events.is_empty());
}
