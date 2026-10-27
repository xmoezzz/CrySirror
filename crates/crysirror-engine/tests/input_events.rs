// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use anyhow::Result;
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{Value as Json, json};
fn app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    let p = d.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"input events","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &d.path().join("state")).unwrap(), "a").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    (d, a)
}
fn call(a: &mut App, name: &str, args: Json) -> Result<Json> {
    let cx = a.context();
    let input = cx.singleton("Input")?;
    let args = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    Ok(cx.call(input, name, &args)?.to_json())
}
fn frame(a: &mut App, down: u64, pressed: u64, x: i32) {
    a.frame(
        1,
        InputFrame {
            buttons_down: down,
            buttons_pressed: pressed,
            pointer_x: x,
            ..Default::default()
        },
    )
    .unwrap();
}
#[test]
fn optional_public_journal_captures_normalized_edges_while_action_queries_remain_independent() {
    let (_d, mut a) = app();
    assert_eq!(
        call(&mut a, "eventStats", json!([])).unwrap()["enabled"],
        false
    );
    call(&mut a, "enableEvents", json!([8])).unwrap();
    call(&mut a, "bind", json!(["confirm", ["enter"]])).unwrap();
    frame(&mut a, 1, 0, 10);
    assert_eq!(call(&mut a, "isDown", json!(["confirm"])).unwrap(), true);
    frame(&mut a, 0, 0, 20);
    frame(&mut a, 0, 1, 30);
    let b = call(&mut a, "events", json!([8])).unwrap();
    assert_eq!(b["samples"].as_array().unwrap().len(), 3);
    assert_eq!(b["samples"][0]["frame"]["buttons_pressed"], "1");
    assert_eq!(b["samples"][1]["released"], "1");
    assert_eq!(b["samples"][2]["released"], "1");
    assert_eq!(call(&mut a, "events", json!([8])).unwrap(), b);
    call(&mut a, "ackEvents", json!([b["receipt"]])).unwrap();
    assert!(call(&mut a, "ackEvents", json!([b["receipt"]])).is_err());
    assert!(
        call(&mut a, "events", json!([8])).unwrap()["samples"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn client_epochs_and_portable_restore_prevent_foreign_or_stale_acknowledgements() {
    let (_d, mut a) = app();
    call(&mut a, "enableEvents", json!([8])).unwrap();
    frame(&mut a, 0, 1, 10);
    let b = call(&mut a, "events", json!([8])).unwrap();
    let saved = call(&mut a, "captureEvents", json!([])).unwrap();
    a.context().state().unwrap().client = ClientId(2);
    call(&mut a, "enableEvents", json!([8])).unwrap();
    assert!(call(&mut a, "ackEvents", json!([b["receipt"]])).is_err());
    a.context().state().unwrap().client = ClientId(1);
    call(&mut a, "restoreEvents", json!([saved])).unwrap();
    assert!(call(&mut a, "ackEvents", json!([b["receipt"]])).is_err());
    let restored = call(&mut a, "events", json!([8])).unwrap();
    assert_eq!(restored["samples"], b["samples"]);
    assert_ne!(restored["receipt"]["epoch"], b["receipt"]["epoch"]);
    let before = call(&mut a, "captureEvents", json!([])).unwrap();
    let mut bad = saved;
    bad["schema"] = json!(99);
    assert!(call(&mut a, "restoreEvents", json!([bad])).is_err());
    assert_eq!(call(&mut a, "captureEvents", json!([])).unwrap(), before);
}
#[test]
fn runtime_restore_disables_device_journals_until_explicit_logical_restore() {
    let (_d, mut a) = app();
    call(&mut a, "enableEvents", json!([8])).unwrap();
    frame(&mut a, 1, 1, 0);
    let input = call(&mut a, "captureEvents", json!([])).unwrap();
    let save = a.runtime().unwrap().snapshot().unwrap();
    a.runtime_mut().unwrap().restore(&save).unwrap();
    assert_eq!(
        call(&mut a, "eventStats", json!([])).unwrap()["enabled"],
        false
    );
    call(&mut a, "restoreEvents", json!([input])).unwrap();
    assert_eq!(call(&mut a, "eventStats", json!([])).unwrap()["pending"], 1);
}
#[test]
fn large_button_masks_remain_exact_in_the_public_json_transport() {
    let (_d, mut a) = app();
    call(&mut a, "enableEvents", json!([4])).unwrap();
    let mask = (1u64 << 60) | 1;
    frame(&mut a, mask, mask, 0);
    let b = call(&mut a, "events", json!([4])).unwrap();
    assert_eq!(b["samples"][0]["frame"]["buttons_down"], mask.to_string());
    call(&mut a, "ackEvents", json!([b["receipt"]])).unwrap();
}

#[test]
fn overflow_in_one_client_is_visible_and_does_not_partially_advance_other_journals() {
    let (_d, mut a) = app();
    call(&mut a, "enableEvents", json!([8])).unwrap();
    a.context().state().unwrap().client = ClientId(2);
    call(&mut a, "enableEvents", json!([1])).unwrap();
    a.context().state().unwrap().client = ClientId(1);
    frame(&mut a, 1, 1, 0);
    let first = call(&mut a, "captureEvents", json!([])).unwrap();
    a.context().state().unwrap().client = ClientId(2);
    let second = call(&mut a, "captureEvents", json!([])).unwrap();
    a.context().state().unwrap().client = ClientId(1);
    assert!(a.frame(1, InputFrame::default()).is_err());
    assert_eq!(call(&mut a, "captureEvents", json!([])).unwrap(), first);
    a.context().state().unwrap().client = ClientId(2);
    assert_eq!(call(&mut a, "captureEvents", json!([])).unwrap(), second);
}
