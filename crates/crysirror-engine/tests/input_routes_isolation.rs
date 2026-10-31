//! Routed input has one owner across behaviours, VMs and asynchronous finish.
use anyhow::Result;
use crysirror_core::{
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_engine::{App, EngineContext, Lifecycle, Value};
use crysirror_runtime::Runtime;
use crysirror_scene::{input_routes::*, ServiceBatch};
use serde_json::{json, Value as Json};
use std::sync::{Arc, Mutex};
fn app() -> (tempfile::TempDir, App) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    let p = d.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"input routes","content_revision":"1","assets":"assets","canvas":[100,100],"clients":[{"name":"app","adapter":"crysirror-script-v1","game":"a"}]})).unwrap()).unwrap();
    let a = App::new(Runtime::load(&p, &d.path().join("state")).unwrap(), "app").unwrap();
    (d, a)
}
fn put(cx: &mut EngineContext) -> Result<()> {
    cx.state()?.runtime.host.engine.submit(
        ClientId(1),
        &ServiceBatch {
            input_routes: vec![InputRouteCommand::Put {
                receiver: InputReceiverSpec {
                    id: 7,
                    priority: 0,
                    sources: 7,
                    rect: None,
                    coverage: None,
                    enabled: true,
                    hover_enabled: true,
                    drag_enabled: true,
                },
            }],
            ..Default::default()
        },
    )
}
fn input(a: &mut App, name: &str, args: Json) -> Json {
    let cx = a.context();
    let id = cx.singleton("Input").unwrap();
    cx.call(
        id,
        name,
        &args
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(Value::from_json)
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .to_json()
}
struct Observe {
    seen: Arc<Mutex<Vec<InputFrame>>>,
    register: bool,
}
impl Lifecycle for Observe {
    fn name(&self) -> &str {
        "observe routing"
    }
    fn on_update(&mut self, cx: &mut EngineContext, _: u64) -> Result<()> {
        self.seen.lock().unwrap().push(cx.state()?.input);
        if self.register {
            put(cx)?;
            self.register = false;
        }
        Ok(())
    }
    fn on_late_update(&mut self, cx: &mut EngineContext, _: u64) -> Result<()> {
        self.seen.lock().unwrap().push(cx.state()?.input);
        Ok(())
    }
}
fn frame() -> InputFrame {
    InputFrame {
        buttons_down: buttons::CTRL,
        buttons_pressed: buttons::CTRL,
        pointer_x: 50,
        pointer_y: 50,
    }
}
fn poll(a: &mut App) -> Option<InputDelivery> {
    a.runtime_mut()
        .unwrap()
        .host
        .engine
        .input_routes
        .poll(ClientId(1))
        .unwrap()
}
fn finish(a: &mut App) {
    let start = std::time::Instant::now();
    while !a.poll_frame().unwrap() {
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::yield_now();
    }
}
#[test]
fn exclusive_routes_suppress_behaviour_actions_and_journal_in_sync_and_worker_frames() {
    let (_d, mut a) = app();
    put(a.context()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    a.add_behaviour(Box::new(Observe {
        seen: seen.clone(),
        register: false,
    }))
    .unwrap();
    a.init().unwrap();
    a.start().unwrap();
    input(&mut a, "enableEvents", json!([8]));
    input(&mut a, "bind", json!(["skip", ["ctrl"]]));
    a.frame(1, frame()).unwrap();
    assert_eq!(input(&mut a, "isDown", json!(["skip"])), false);
    let down = poll(&mut a).unwrap();
    assert_eq!(
        (down.event.code, down.event.phase),
        (12, InputRoutePhase::Press)
    );
    a.runtime_mut()
        .unwrap()
        .host
        .engine
        .input_routes
        .acknowledge(ClientId(1), down.receipt, true)
        .unwrap();
    a.begin_frame(
        1,
        InputFrame {
            pointer_x: 50,
            pointer_y: 50,
            ..Default::default()
        },
    )
    .unwrap();
    finish(&mut a);
    let up = poll(&mut a).unwrap();
    assert_eq!(up.event.phase, InputRoutePhase::Release);
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|f| f.buttons_down == 0 && f.buttons_pressed == 0));
    let samples = input(&mut a, "events", json!([8]));
    assert!(samples["samples"]
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["frame"]["buttons_down"] == "0"
            && s["frame"]["buttons_pressed"] == "0"
            && s["released"] == "0"));
}
#[test]
fn registering_in_update_does_not_reoffer_behaviour_edge_or_held_state() {
    let (_d, mut a) = app();
    let seen = Arc::new(Mutex::new(Vec::new()));
    a.add_behaviour(Box::new(Observe {
        seen: seen.clone(),
        register: true,
    }))
    .unwrap();
    a.init().unwrap();
    a.start().unwrap();
    a.begin_frame(1, frame()).unwrap();
    finish(&mut a);
    assert_eq!(seen.lock().unwrap()[0].buttons_pressed, buttons::CTRL);
    assert!(poll(&mut a).is_none());
    a.frame(
        1,
        InputFrame {
            buttons_pressed: 0,
            ..frame()
        },
    )
    .unwrap();
    assert!(poll(&mut a).is_none());
    a.frame(
        1,
        InputFrame {
            pointer_x: 50,
            pointer_y: 50,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(poll(&mut a).unwrap().event.phase, InputRoutePhase::Release);
    assert!(seen.lock().unwrap()[2..]
        .iter()
        .all(|f| f.buttons_down == 0 && f.buttons_pressed == 0));
}
#[test]
fn menu_conflict_precedes_behaviour_journal_and_clock_changes() {
    let (_d, mut a) = app();
    put(a.context()).unwrap();
    let menu = serde_json::from_value(
        json!({"id":1,"rect":[0,0,100,24],"font":"default","items":[{"id":1,"label":"Menu"}]}),
    )
    .unwrap();
    a.runtime_mut()
        .unwrap()
        .host
        .engine
        .submit(
            ClientId(1),
            &ServiceBatch {
                ui: vec![crysirror_scene::ui::UiCommand::SetMenu { menu }],
                ..Default::default()
            },
        )
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    a.add_behaviour(Box::new(Observe {
        seen: seen.clone(),
        register: false,
    }))
    .unwrap();
    a.init().unwrap();
    a.start().unwrap();
    input(&mut a, "enableEvents", json!([8]));
    let before = input(&mut a, "captureEvents", json!([]));
    assert!(a
        .frame(
            1,
            InputFrame {
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                pointer_x: 5,
                pointer_y: 5
            }
        )
        .is_err());
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(a.runtime().unwrap().now_ms(), 0);
    assert_eq!(input(&mut a, "captureEvents", json!([])), before);
    assert_eq!(
        a.runtime()
            .unwrap()
            .host
            .engine
            .input_routes
            .stats(ClientId(1))
            .queued_samples,
        0
    );
}
