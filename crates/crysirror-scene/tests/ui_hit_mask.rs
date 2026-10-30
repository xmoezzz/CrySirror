use crysirror_core::{
    image_processing::ImageResource,
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_scene::ui::{UiCommand, UiSystem, WidgetHitMask};
use serde_json::json;
use std::sync::Arc;
fn commands(ui: &mut UiSystem, owner: u32, v: serde_json::Value) {
    ui.apply_batch(
        ClientId(owner),
        &serde_json::from_value::<Vec<UiCommand>>(v).unwrap(),
    )
    .unwrap();
}
fn hit(ui: &mut UiSystem, x: i32, y: i32) {
    ui.tick(
        InputFrame {
            pointer_x: x,
            pointer_y: y,
            ..Default::default()
        },
        [20, 20],
    )
    .unwrap();
}
#[test]
fn coverage_passes_through_transparency_and_survives_saved_state_without_private_sampler() {
    let mut u = UiSystem::default();
    commands(
        &mut u,
        1,
        json!([{"op":"put","widget":{"id":1,"rect":[2,2,3,2],"kind":"button","interaction_only":true}},{"op":"put","widget":{"id":2,"rect":[2,2,3,2],"kind":"button","interaction_only":true,"z":1}}]),
    );
    let mask = WidgetHitMask {
        size: [3, 2],
        coverage: vec![0, 1, 255, 255, 0, 255].into(),
        threshold: 1,
        strict_bounds: false,
    };
    u.apply_batch(
        ClientId(1),
        &[UiCommand::SetHitMask {
            id: 2,
            mask: Some(mask),
        }],
    )
    .unwrap();
    hit(&mut u, 2, 2);
    assert!(u.interaction(1, 1).unwrap().hovered);
    assert!(!u.interaction(1, 2).unwrap().hovered);
    hit(&mut u, 3, 2);
    assert!(u.interaction(1, 2).unwrap().hovered);
    let mut restored: UiSystem = serde_json::from_slice(&serde_json::to_vec(&u).unwrap()).unwrap();
    restored.validate().unwrap();
    hit(&mut restored, 3, 3);
    assert!(restored.interaction(1, 1).unwrap().hovered);
    restored
        .apply_batch(ClientId(1), &[UiCommand::SetHitMask { id: 2, mask: None }])
        .unwrap();
    hit(&mut restored, 3, 3);
    assert!(restored.interaction(1, 2).unwrap().hovered);
    assert!(restored
        .apply_batch(ClientId(2), &[UiCommand::SetHitMask { id: 2, mask: None }])
        .is_err());
}
#[test]
fn strict_local_edges_and_keyboard_are_separate_and_invalid_masks_are_atomic() {
    let mut u = UiSystem::default();
    commands(
        &mut u,
        1,
        json!([{"op":"put","widget":{"id":1,"rect":[2,2,3,2],"kind":"button","interaction_only":true,"hit_mask":{"size":[3,2],"coverage":[255,255,255,255,255,255],"threshold":1,"strict_bounds":true}}}]),
    );
    hit(&mut u, 2, 3);
    assert!(!u.interaction(1, 1).unwrap().hovered);
    hit(&mut u, 3, 2);
    assert!(!u.interaction(1, 1).unwrap().hovered);
    hit(&mut u, 3, 3);
    assert!(u.interaction(1, 1).unwrap().hovered);
    u.tick(
        InputFrame {
            pointer_x: 0,
            pointer_y: 0,
            buttons_pressed: buttons::ENTER,
            buttons_down: buttons::ENTER,
            ..Default::default()
        },
        [20, 20],
    )
    .unwrap();
    assert!(u.interaction(1, 1).unwrap().activated);
    let old = serde_json::to_value(&u).unwrap();
    let bad = WidgetHitMask {
        size: [3, 2],
        coverage: vec![255].into(),
        threshold: 1,
        strict_bounds: false,
    };
    assert!(u
        .apply_batch(
            ClientId(1),
            &[UiCommand::SetHitMask {
                id: 1,
                mask: Some(bad)
            }]
        )
        .is_err());
    assert_eq!(serde_json::to_value(&u).unwrap(), old);
}
#[derive(Debug)]
struct Image {
    size: [u32; 2],
    rgba: Arc<[u8]>,
}
impl ImageResource for Image {
    fn client(&self) -> ClientId {
        ClientId(1)
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.rgba.clone()
    }
    fn opaque(&self) -> bool {
        false
    }
}
#[test]
fn image_alpha_region_checks_shape_overflow_and_stored_bytes() {
    let image = Image {
        size: [3, 2],
        rgba: vec![
            0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5,
        ]
        .into(),
    };
    assert_eq!(&*image.alpha_region([1, 0, 2, 2]).unwrap(), &[1, 2, 4, 5]);
    for rect in [
        [3, 0, 1, 1],
        [0, 0, 0, 1],
        [u32::MAX, 0, 2, 1],
        [0, 1, 1, 2],
    ] {
        assert!(image.alpha_region(rect).is_err());
    }
    let bad = Image {
        size: [3, 2],
        rgba: vec![0].into(),
    };
    assert!(bad.alpha_region([0, 0, 1, 1]).is_err());
}

#[test]
fn masked_button_activation_remains_in_engine_mailbox_while_worker_suspends_consumer() {
    let mut u = UiSystem::default();
    commands(
        &mut u,
        1,
        json!([{"op":"put","widget":{"id":1,"rect":[0,0,2,2],"kind":"button","interaction_only":true,"keyboard_focusable":false,"hit_mask":{"size":[2,2],"coverage":[0,0,0,255],"threshold":1,"strict_bounds":true}}}]),
    );
    u.tick(
        InputFrame {
            pointer_x: 1,
            pointer_y: 1,
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        },
        [20, 20],
    )
    .unwrap();
    hit(&mut u, 1, 1);
    assert!(u.interaction(1, 1).unwrap().activated);
    for _ in 0..10 {
        hit(&mut u, 0, 0);
    }
    assert!(!u.interaction(1, 1).unwrap().activated);
    let events = u.drain_events(1, None);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "activate");
    assert!(u.drain_events(1, None).is_empty());
}
