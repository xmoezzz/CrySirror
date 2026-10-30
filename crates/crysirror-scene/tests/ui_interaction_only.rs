use crysirror_core::{
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_scene::{
    ui::{UiCommand, WidgetSpec},
    EngineSystems, ServiceBatch,
};
fn widget(value: serde_json::Value) -> UiCommand {
    UiCommand::Put {
        widget: serde_json::from_value::<WidgetSpec>(value).unwrap(),
    }
}
#[test]
fn image_layer_controls_receive_pointer_without_paints_or_keyboard_focus_and_edges_are_stable() {
    let mut engine = EngineSystems::default();
    let batch = ServiceBatch {
        ui: vec![
            widget(
                serde_json::json!({"id":1,"rect":[0,0,50,20],"kind":"text_field","text":"edit","font":"engine-font"}),
            ),
            widget(
                serde_json::json!({"id":2,"rect":[60,0,50,20],"kind":"button","interaction_only":true,"keyboard_focusable":false}),
            ),
            UiCommand::Focus { id: 1 },
        ],
        ..Default::default()
    };
    engine.submit(ClientId(7), &batch).unwrap();
    engine
        .tick(
            0,
            InputFrame {
                pointer_x: 70,
                pointer_y: 5,
                ..Default::default()
            },
            [200, 100],
        )
        .unwrap();
    assert_eq!(engine.ui.focus.unwrap().id, 1);
    assert!(engine.ui.interaction(7, 2).unwrap().hovered);
    assert!(engine.frame([200, 100]).unwrap().iter().all(|p| p.id != 2));
    engine
        .tick(
            1,
            InputFrame {
                pointer_x: 70,
                pointer_y: 5,
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                ..Default::default()
            },
            [200, 100],
        )
        .unwrap();
    assert_eq!(engine.ui.focus.unwrap().id, 1);
    assert!(engine.ui.interaction(7, 2).unwrap().pressed);
    engine
        .tick(
            1,
            InputFrame {
                pointer_x: 70,
                pointer_y: 5,
                ..Default::default()
            },
            [200, 100],
        )
        .unwrap();
    let activation = engine.ui.interaction(7, 2).unwrap();
    assert!(activation.activated && activation.activation_serial > 0);
    assert_eq!(
        engine.ui.interaction(7, 2).unwrap(),
        activation,
        "observation does not consume or replay an edge"
    );
    let restored: EngineSystems =
        serde_json::from_slice(&serde_json::to_vec(&engine).unwrap()).unwrap();
    assert!(
        !restored.ui.interaction(7, 2).unwrap().activated,
        "input edge is not a saved action"
    );
    engine
        .tick(
            1,
            InputFrame {
                pointer_x: 70,
                pointer_y: 5,
                ..Default::default()
            },
            [200, 100],
        )
        .unwrap();
    assert!(!engine.ui.interaction(7, 2).unwrap().activated);
    assert_eq!(engine.ui.interaction(7, 2).unwrap().activation_serial, 0);
    assert!(
        engine.ui.interaction(8, 2).is_none(),
        "client capability boundary"
    );
}
