// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_scene::ui::{UiCommand, UiSystem};
use serde_json::json;
#[test]
fn retained_interaction_is_client_scoped_and_tracks_pointer_press_and_release() {
    let mut ui = UiSystem::default();
    let command:UiCommand=serde_json::from_value(json!({"op":"put","widget":{"id":7,"kind":"button","rect":[10,20,30,40],"color":[0,0,0,0]}})).unwrap();
    ui.apply_batch(ClientId(1), &[command]).unwrap();
    assert!(ui.interaction(2, 7).is_none());
    let input = InputFrame {
        pointer_x: 15,
        pointer_y: 25,
        buttons_down: buttons::MOUSE_LEFT,
        buttons_pressed: buttons::MOUSE_LEFT,
        ..Default::default()
    };
    ui.tick(input, [100, 100]).unwrap();
    let state = ui.interaction(1, 7).unwrap();
    assert!(state.hovered && state.focused && state.pressed);
    ui.tick(
        InputFrame {
            pointer_x: 90,
            pointer_y: 90,
            ..Default::default()
        },
        [100, 100],
    )
    .unwrap();
    let state = ui.interaction(1, 7).unwrap();
    assert!(!state.hovered && !state.pressed);
}
