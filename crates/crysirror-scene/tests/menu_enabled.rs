//! Menu enabled state is engine-owned and independent of native executables.
use crysirror_core::state::ClientId;
use crysirror_scene::ui::{
    menu::{MenuItemSpec, MenuSpec},
    UiCommand, UiSystem,
};
use serde_json::json;

#[test]
fn disabling_owned_menu_closes_branch_and_blocks_native_activation() {
    let spec: MenuSpec = serde_json::from_value(json!({
        "id":1,"rect":[0,0,640,24],"font":"default",
        "items":[{"id":10,"label":"System","children":[{"id":119,"label":"Return to title","checked":true}]}]
    })).unwrap();
    let mut ui = UiSystem::default();
    for owner in [1, 2] {
        ui.apply_batch(
            ClientId(owner),
            &[
                UiCommand::SetMenu { menu: spec.clone() },
                UiCommand::OpenMenu {
                    id: 1,
                    item: Some(10),
                    position: None,
                },
            ],
        )
        .unwrap();
    }
    let before = serde_json::to_value(&ui).unwrap();
    assert!(ui
        .apply_batch(
            ClientId(1),
            &[UiCommand::SetMenuEnabled {
                id: 1,
                item: 999,
                enabled: false
            }]
        )
        .is_err());
    assert_eq!(before, serde_json::to_value(&ui).unwrap());
    assert!(ui
        .apply_batch(
            ClientId(3),
            &[UiCommand::SetMenuEnabled {
                id: 1,
                item: 10,
                enabled: false
            }]
        )
        .is_err());
    ui.apply_batch(
        ClientId(1),
        &[UiCommand::SetMenuEnabled {
            id: 1,
            item: 10,
            enabled: false,
        }],
    )
    .unwrap();
    assert!(ui.menus.activate_item(1, 1, 119).is_err());
    assert!(ui.menus.activate_item(2, 1, 119).is_ok());
    ui.menus.validate().unwrap();
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["menus"]["clients"]["1"]["1"]["open"], json!([]));
    let mut restored: UiSystem = serde_json::from_value(saved).unwrap();
    assert!(restored.menus.activate_item(1, 1, 119).is_err());
    restored
        .apply_batch(
            ClientId(1),
            &[
                UiCommand::SetMenuEnabled {
                    id: 1,
                    item: 10,
                    enabled: true,
                },
                UiCommand::SetMenuEnabled {
                    id: 1,
                    item: 119,
                    enabled: false,
                },
            ],
        )
        .unwrap();
    assert!(restored.menus.activate_item(1, 1, 119).is_err());
    restored
        .apply_batch(
            ClientId(1),
            &[UiCommand::SetMenuEnabled {
                id: 1,
                item: 119,
                enabled: true,
            }],
        )
        .unwrap();
    assert!(
        MenuItemSpec::find(&restored.menus.get(1, 1).unwrap().items, 119)
            .unwrap()
            .checked
    );
    assert!(restored.menus.activate_item(1, 1, 119).is_ok());
}
