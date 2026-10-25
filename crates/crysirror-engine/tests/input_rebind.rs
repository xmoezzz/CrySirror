// SPDX-License-Identifier: MPL-2.0
//! Synthetic physical events: these tests neither open a device nor need fonts.
use crysirror_core::service::{buttons, InputFrame};
use crysirror_engine::{
    input_api::{
        ActionBinding, ActionMap, Binding, BindingProfile, GamepadEvent, GamepadFamily, PadAxis,
        PadButton,
    },
    App, Value,
};
use crysirror_runtime::Runtime;
use serde_json::json;

fn frame(down: u64, pressed: u64) -> InputFrame {
    InputFrame {
        buttons_down: down,
        buttons_pressed: pressed,
        ..Default::default()
    }
}
fn action(bindings: Vec<Binding>) -> ActionBinding {
    ActionBinding {
        bindings,
        threshold: 0.5,
    }
}
fn connect(map: &mut ActionMap, id: u32, family: GamepadFamily) {
    map.inject_gamepad(GamepadEvent::Connected { id, family })
        .unwrap();
}

#[test]
fn keyboard_edges_analog_deadzone_and_signed_axis_are_distinct() {
    let mut map = ActionMap::default();
    map.bind("jump", &["space".into(), "e".into()]).unwrap();
    let first = frame(buttons::SPACE, buttons::SPACE);
    map.begin_frame(first);
    assert!(map.is_down("jump").unwrap());
    assert!(map.pressed("jump").unwrap());
    map.end_frame(first);
    let held = frame(buttons::SPACE, 0);
    map.begin_frame(held);
    assert!(!map.pressed("jump").unwrap());
    map.end_frame(held);
    map.begin_frame(InputFrame::default());
    assert!(map.released("jump").unwrap());
    map.end_frame(InputFrame::default());
    map.rebind(
        "move_x",
        action(vec![Binding::Axis {
            axis: PadAxis::LeftX,
            scale: 1.,
            dead_zone: 0.2,
        }]),
    )
    .unwrap();
    connect(&mut map, 7, GamepadFamily::Xbox);
    map.inject_gamepad(GamepadEvent::Axis {
        id: 7,
        axis: PadAxis::LeftX,
        value: 0.1,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert_eq!(map.value("move_x").unwrap(), 0.);
    map.end_frame(InputFrame::default());
    map.inject_gamepad(GamepadEvent::Axis {
        id: 7,
        axis: PadAxis::LeftX,
        value: 0.8,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert!((map.value("move_x").unwrap() - 0.75).abs() < 1e-6);
    assert!(map.pressed("move_x").unwrap());
    map.end_frame(InputFrame::default());
    map.inject_gamepad(GamepadEvent::Axis {
        id: 7,
        axis: PadAxis::LeftX,
        value: -0.8,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert!((map.value("move_x").unwrap() + 0.75).abs() < 1e-6);
    assert!(!map.pressed("move_x").unwrap());
    assert!(map
        .inject_gamepad(GamepadEvent::Axis {
            id: 7,
            axis: PadAxis::LeftX,
            value: f32::NAN
        })
        .is_err());
    assert!(map
        .inject_gamepad(GamepadEvent::Button {
            id: 999,
            button: PadButton::South,
            value: 1.
        })
        .is_err());
}

#[test]
fn capture_profile_roundtrip_rejection_and_prompt_family_follow_activity() {
    let mut map = ActionMap::default();
    map.rebind(
        "confirm",
        action(vec![
            Binding::Key {
                key: "enter".into(),
            },
            Binding::Button {
                button: PadButton::South,
            },
        ]),
    )
    .unwrap();
    assert_eq!(map.prompt("confirm").unwrap().label, "ENTER");
    connect(&mut map, 1, GamepadFamily::Playstation);
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::South,
        value: 1.,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    let prompt = map.prompt("confirm").unwrap();
    assert_eq!(prompt.label, "Cross");
    assert_eq!(prompt.icon, "input/playstation/south");
    map.end_frame(InputFrame::default());
    map.set_prompt_family(Some(GamepadFamily::Nintendo));
    assert_eq!(map.prompt("confirm").unwrap().label, "B");
    map.set_prompt_family(None);
    map.begin_frame(frame(buttons::ENTER, buttons::ENTER));
    assert_eq!(map.prompt("confirm").unwrap().label, "ENTER");
    map.end_frame(frame(buttons::ENTER, buttons::ENTER));
    map.start_rebind("confirm").unwrap();
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::East,
        value: 1.,
    })
    .unwrap();
    let result = map.take_rebind().unwrap();
    assert_eq!(result.action, "confirm");
    assert_eq!(
        result.binding,
        Binding::Button {
            button: PadButton::East
        }
    );
    map.begin_frame(InputFrame::default());
    assert!(
        !map.pressed("confirm").unwrap(),
        "capture must not also activate the action"
    );
    map.end_frame(InputFrame::default());
    let profile = map.export();
    let bytes = serde_json::to_vec(&profile).unwrap();
    let mut restored = ActionMap::default();
    restored
        .import(serde_json::from_slice(&bytes).unwrap())
        .unwrap();
    assert_eq!(restored.export(), profile);
    assert!(!restored.is_down("confirm").unwrap());
    let before = restored.export();
    let mut invalid = profile.clone();
    invalid.actions.get_mut("confirm").unwrap().threshold = 0.;
    assert!(restored.import(invalid).is_err());
    assert_eq!(restored.export(), before);
    assert!(
        serde_json::from_value::<BindingProfile>(json!({"schema":1,"actions":{},"typo":1}))
            .is_err()
    );
    map.inject_gamepad(GamepadEvent::Disconnected { id: 1 })
        .unwrap();
    map.begin_frame(InputFrame::default());
    assert!(map.released("confirm").unwrap());
}

#[test]
fn active_pad_switch_does_not_leak_other_pad_press_and_navigation_pulses_once() {
    let mut map = ActionMap::default();
    map.rebind(
        "south",
        action(vec![Binding::Button {
            button: PadButton::South,
        }]),
    )
    .unwrap();
    connect(&mut map, 1, GamepadFamily::Xbox);
    connect(&mut map, 2, GamepadFamily::Nintendo);
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::South,
        value: 1.,
    })
    .unwrap();
    map.inject_gamepad(GamepadEvent::Button {
        id: 2,
        button: PadButton::East,
        value: 1.,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert!(!map.pressed("south").unwrap());
    assert!(!map.is_down("south").unwrap());
    map.end_frame(InputFrame::default());
    map.inject_gamepad(GamepadEvent::Button {
        id: 2,
        button: PadButton::South,
        value: 1.,
    })
    .unwrap();
    map.inject_gamepad(GamepadEvent::Button {
        id: 2,
        button: PadButton::South,
        value: 0.,
    })
    .unwrap();
    assert_eq!(map.navigation_buttons(), (0, buttons::ENTER));
    assert_eq!(map.navigation_buttons(), (0, 0));
    map.inject_gamepad(GamepadEvent::Axis {
        id: 2,
        axis: PadAxis::DpadX,
        value: -1.,
    })
    .unwrap();
    assert_eq!(map.navigation_buttons(), (buttons::LEFT, buttons::LEFT));
    assert_eq!(map.navigation_buttons(), (buttons::LEFT, 0));
    map.reset_devices();
    assert_eq!(map.navigation_buttons(), (0, 0));
}

#[test]
fn reflective_api_rebinding_profile_and_hints_reach_real_app() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"input-test","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"input-test"}]})).unwrap()).unwrap();
    let runtime = Runtime::load(&path, &dir.path().join("state")).unwrap();
    let mut app = App::new(runtime, "game").unwrap();
    let input = app.context().singleton("Input").unwrap();
    app.context()
        .call(
            input,
            "bind",
            &["jump".into(), Value::from_json(json!(["space"]))],
        )
        .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, frame(buttons::SPACE, buttons::SPACE))
        .unwrap();
    // End-of-frame polling is not a lifecycle callback; pressed edges are scoped
    // to the frame. The held state remains inspectable through the same API.
    assert!(app
        .context()
        .call(input, "isDown", &["jump".into()])
        .unwrap()
        .as_bool()
        .unwrap());
    app.context()
        .call(
            input,
            "rebind",
            &[
                "jump".into(),
                Value::from_json(json!({"bindings":[{"kind":"button","button":"south"}]})),
            ],
        )
        .unwrap();
    let profile = app.context().call(input, "exportBindings", &[]).unwrap();
    assert_eq!(profile.field("schema").unwrap().as_i64().unwrap(), 1);
    app.context()
        .state()
        .unwrap()
        .actions
        .inject_gamepad(GamepadEvent::Connected {
            id: 4,
            family: GamepadFamily::Xbox,
        })
        .unwrap();
    app.context()
        .state()
        .unwrap()
        .actions
        .inject_gamepad(GamepadEvent::Button {
            id: 4,
            button: PadButton::South,
            value: 1.,
        })
        .unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let hint = app
        .context()
        .call(input, "prompt", &["jump".into()])
        .unwrap();
    assert_eq!(hint.field("label").unwrap().as_str().unwrap(), "A");
    assert!(app
        .context()
        .call(input, "isDown", &["jump".into()])
        .unwrap()
        .as_bool()
        .unwrap());
    assert!(app
        .context()
        .call(
            input,
            "rebind",
            &[
                "jump".into(),
                Value::from_json(
                    json!({"bindings":[{"kind":"axis","axis":"left_x","dead_zone":1}]})
                )
            ]
        )
        .is_err());
    assert_eq!(
        app.context()
            .call(input, "exportBindings", &[])
            .unwrap()
            .to_json(),
        profile.to_json()
    );
    let declarations = app
        .engine()
        .unwrap()
        .classes
        .typescript_declarations(crysirror_engine::builtin::GLOBALS);
    assert!(declarations.contains("rebind("));
    assert!(declarations.contains("exportBindings("));
}

#[test]
fn analog_button_edges_respect_action_threshold_even_for_a_short_pulse() {
    let mut map = ActionMap::default();
    map.rebind(
        "trigger",
        ActionBinding {
            bindings: vec![Binding::Button {
                button: PadButton::RightTrigger,
            }],
            threshold: 0.8,
        },
    )
    .unwrap();
    connect(&mut map, 1, GamepadFamily::Xbox);
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::RightTrigger,
        value: 0.6,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert!(!map.pressed("trigger").unwrap());
    assert!(!map.is_down("trigger").unwrap());
    map.end_frame(InputFrame::default());
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::RightTrigger,
        value: 0.9,
    })
    .unwrap();
    map.inject_gamepad(GamepadEvent::Button {
        id: 1,
        button: PadButton::RightTrigger,
        value: 0.,
    })
    .unwrap();
    map.begin_frame(InputFrame::default());
    assert!(map.pressed("trigger").unwrap());
    assert!(!map.is_down("trigger").unwrap());
    map.end_frame(InputFrame::default());
    map.begin_frame(InputFrame::default());
    assert!(!map.pressed("trigger").unwrap());
}
