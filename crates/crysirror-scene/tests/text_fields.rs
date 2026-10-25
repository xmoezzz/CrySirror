// SPDX-License-Identifier: MPL-2.0
//! Engine-owned text entry, using committed Unicode without a native VM or font file.
use crysirror_core::{
    service::{InputFrame, buttons},
    state::ClientId,
};
use crysirror_scene::ui::{UiCommand as C, UiSystem, WidgetId, WidgetSpec};
use serde_json::{json, to_value};
fn field(id: u32, text: &str) -> WidgetSpec {
    serde_json::from_value(json!({"id":id,"rect":[0,id*30,100,25],"kind":"text_field","text":text,"font":"project-font","max_length":4})).unwrap()
}
fn tick(ui: &mut UiSystem, pressed: u64) {
    ui.tick(
        InputFrame {
            buttons_pressed: pressed,
            ..Default::default()
        },
        [300, 300],
    )
    .unwrap();
}
#[test]
fn unicode_commits_backspace_programmatic_set_and_saved_state_are_shared() {
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &[field(1, "")]).unwrap();
    ui.apply_batch(
        ClientId(1),
        &[
            C::Focus { id: 1 },
            C::InsertText {
                text: "あ😀e\u{301}余分".into(),
            },
        ],
    )
    .unwrap();
    assert_eq!(ui.get(1, 1).unwrap().text, "あ😀e\u{301}");
    assert!(ui.focused_text_field());
    tick(&mut ui, buttons::BACKSPACE);
    assert_eq!(ui.get(1, 1).unwrap().text, "あ😀e");
    ui.apply_batch(
        ClientId(1),
        &[
            C::Backspace { id: 1 },
            C::SetText {
                id: 1,
                text: "花子".into(),
                font: Some("project-font".into()),
            },
        ],
    )
    .unwrap();
    assert_eq!(ui.get(1, 1).unwrap().text, "花子");
    let saved = to_value(&ui).unwrap();
    let mut restored: UiSystem = serde_json::from_value(saved.clone()).unwrap();
    restored.validate().unwrap();
    assert_eq!(to_value(&restored).unwrap(), saved);
    restored
        .apply_batch(
            ClientId(1),
            &[C::InsertText {
                text: "😊".into()
            }],
        )
        .unwrap();
    assert_eq!(restored.get(1, 1).unwrap().text, "花子😊");
    let frame = restored.frame([300, 300]).unwrap();
    assert!(frame.iter().any(
        |p| matches!(&p.paint,crysirror_scene::layer::Paint::Text{text,..} if text=="花子😊")
    ));
}
#[test]
fn enter_submits_nonempty_once_and_moves_focus_escape_cancels() {
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &[field(1, ""), field(2, "次郎")])
        .unwrap();
    tick(&mut ui, buttons::ENTER);
    assert!(ui.events.is_empty());
    ui.apply_batch(
        ClientId(1),
        &[C::InsertText {
            text: "太郎".into(),
        }],
    )
    .unwrap();
    tick(&mut ui, buttons::ENTER);
    assert_eq!(ui.events.len(), 1);
    assert_eq!(ui.events[0].kind, "submit");
    assert_eq!(ui.events[0].widget, WidgetId { owner: 1, id: 1 });
    assert_eq!(ui.focus, Some(WidgetId { owner: 1, id: 2 }));
    tick(&mut ui, buttons::ESCAPE);
    assert_eq!(ui.events.last().unwrap().kind, "cancel");
    assert_eq!(ui.get(1, 2).unwrap().text, "次郎");
    tick(&mut ui, buttons::SPACE);
    assert_eq!(ui.events.len(), 2, "space is committed input, not submit");
    tick(&mut ui, buttons::ENTER);
    assert_eq!(ui.events.last().unwrap().widget.id, 2);
    assert_eq!(ui.events.last().unwrap().kind, "submit");
}
#[test]
fn editing_is_client_scoped_and_invalid_batches_roll_back() {
    let mut ui = UiSystem::default();
    ui.install(ClientId(1), &[field(1, "太郎")]).unwrap();
    ui.install(ClientId(2), &[field(1, "花子")]).unwrap();
    ui.apply_batch(ClientId(1), &[C::Focus { id: 1 }]).unwrap();
    let before = to_value(&ui).unwrap();
    assert!(
        ui.apply_batch(ClientId(2), &[C::InsertText { text: "X".into() }])
            .is_err()
    );
    assert_eq!(to_value(&ui).unwrap(), before);
    for text in ["12345", "x\ny"] {
        assert!(
            ui.apply_batch(
                ClientId(1),
                &[
                    C::Backspace { id: 1 },
                    C::SetText {
                        id: 1,
                        text: text.into(),
                        font: Some("project-font".into())
                    }
                ]
            )
            .is_err()
        );
        assert_eq!(to_value(&ui).unwrap(), before);
    }
    ui.apply_batch(ClientId(2), &[C::Backspace { id: 1 }])
        .unwrap();
    assert_eq!(ui.get(2, 1).unwrap().text, "花");
    assert_eq!(ui.get(1, 1).unwrap().text, "太郎");
}
#[test]
fn invisible_disabled_and_modal_fields_cannot_receive_committed_input() {
    let mut ui = UiSystem::default();
    let parent: WidgetSpec =
        serde_json::from_value(json!({"id":10,"rect":[0,0,200,100],"visible":false})).unwrap();
    let mut child = field(1, "X");
    child.parent = Some(10);
    ui.install(ClientId(1), &[parent, child]).unwrap();
    assert!(ui.apply_batch(ClientId(1), &[C::Focus { id: 1 }]).is_err());
    ui.apply_batch(
        ClientId(1),
        &[
            C::SetVisible {
                id: 10,
                visible: true,
            },
            C::Focus { id: 1 },
        ],
    )
    .unwrap();
    ui.apply_batch(
        ClientId(1),
        &[C::SetEnabled {
            id: 10,
            enabled: false,
        }],
    )
    .unwrap();
    assert!(!ui.focused_text_field());
    assert!(
        ui.apply_batch(ClientId(1), &[C::InsertText { text: "Y".into() }])
            .is_err()
    );
    tick(&mut ui, buttons::ENTER);
    assert!(ui.events.is_empty());
}
