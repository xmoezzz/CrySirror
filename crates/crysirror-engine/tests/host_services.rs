//! Shared services exercised through public App APIs and native HostServices.
use crysirror_core::{
    service::{HostServices, KeyboardLockState},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn app() -> (tempfile::TempDir, App) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"shared-host","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"a"},{"name":"peer","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &t.path().join("state")).unwrap(), "app").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    (t, a)
}
fn call(a: &mut App, class: &'static str, method: &str, args: &[Value]) -> anyhow::Result<Value> {
    let cx = a.context();
    let id = cx.singleton(class)?;
    cx.call(id, method, args)
}
#[test]
fn native_and_public_logs_use_one_canonical_scope_and_owner_receipts() {
    let (_t, mut a) = app();
    call(&mut a, "Engine", "log", &["script".into()]).unwrap();
    let h = &mut a.runtime_mut().unwrap().host;
    let receipt = h.append_log(ClientId(1), "native").unwrap();
    assert_eq!(receipt.sequence, 2);
    h.append_log(ClientId(2), "private-peer").unwrap();
    assert!(h.append_log(ClientId(999), "foreign").is_err());
    let before = h.log_snapshot(ClientId(1)).unwrap();
    assert!(h.append_log(ClientId(1), &"x".repeat(4097)).is_err());
    assert_eq!(h.log_snapshot(ClientId(1)).unwrap(), before);
    a.frame(1, Default::default()).unwrap();
    assert_eq!(
        a.engine()
            .unwrap()
            .log
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["script", "native"]
    );
    let logs = call(&mut a, "Engine", "logs", &[]).unwrap().to_json();
    assert_eq!(logs["records"].as_array().unwrap().len(), 2);
    assert_eq!(logs["records"][1]["sequence"], "2");
}
#[test]
fn keyboard_locks_are_logical_scoped_atomic_and_reset_only_after_committed_restore() {
    let (_t, mut a) = app();
    let saved = a.runtime().unwrap().snapshot().unwrap();
    let h = &mut a.runtime_mut().unwrap().host;
    let locks = h
        .set_keyboard_locks(
            ClientId(1),
            KeyboardLockState {
                caps_lock: true,
                num_lock: true,
                scroll_lock: false,
            },
        )
        .unwrap();
    assert!(locks.physical.is_none());
    assert!(!locks.physical_control_supported);
    assert_eq!(locks.revision, 1);
    assert!(!h.keyboard_locks(ClientId(2)).unwrap().logical.caps_lock);
    assert!(h
        .set_keyboard_locks(ClientId(999), Default::default())
        .is_err());
    assert!(a.runtime_mut().unwrap().restore(b"not a snapshot").is_err());
    assert!(
        a.runtime()
            .unwrap()
            .host
            .keyboard_locks(ClientId(1))
            .unwrap()
            .logical
            .caps_lock
    );
    let v = call(
        &mut a,
        "Input",
        "setKeyboardLocks",
        &[Value::from_json(json!({"caps_lock":false}))],
    )
    .unwrap()
    .to_json();
    assert_eq!(v["logical"]["num_lock"], true);
    assert_eq!(v["revision"], "2");
    let before = a
        .runtime()
        .unwrap()
        .host
        .keyboard_locks(ClientId(1))
        .unwrap();
    for spec in [
        json!({"mode":"physical","caps_lock":true}),
        json!({"caps_lock":"on"}),
        json!({"unknown":true}),
    ] {
        assert!(call(
            &mut a,
            "Input",
            "setKeyboardLocks",
            &[Value::from_json(spec)]
        )
        .is_err());
        assert_eq!(
            a.runtime()
                .unwrap()
                .host
                .keyboard_locks(ClientId(1))
                .unwrap(),
            before
        );
    }
    a.runtime_mut().unwrap().restore(&saved).unwrap();
    let after = call(&mut a, "Input", "keyboardLocks", &[])
        .unwrap()
        .to_json();
    assert_eq!(after["revision"], "0");
    assert_eq!(after["physical"], json!(null));
}
#[test]
fn civil_clock_is_a_scoped_real_worker_dependency_not_virtual_time() {
    use crysirror_core::service::CivilTimeState;
    let (_t, mut a) = app();
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert!(a
        .runtime_mut()
        .unwrap()
        .host
        .begin_civil_time(ClientId(999))
        .is_err());
    let mut job = a
        .runtime_mut()
        .unwrap()
        .host
        .begin_civil_time(ClientId(1))
        .unwrap();
    assert_eq!(job.client(), ClientId(1));
    let stop = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let time = loop {
        match job.poll().unwrap() {
            CivilTimeState::Pending => {
                assert!(std::time::Instant::now() < stop);
                std::thread::yield_now();
            }
            CivilTimeState::Ready(v) => break v,
            CivilTimeState::Failed(e) => panic!("{e}"),
        }
    };
    assert_eq!(time.client, ClientId(1));
    assert!(time.unix_ms >= start);
    assert!((1..=12).contains(&time.month));
    assert!((1..=31).contains(&time.day));
    assert!(time.hour < 24 && time.minute < 60 && time.second <= 60);
    assert_eq!(job.poll().unwrap(), CivilTimeState::Ready(time));
    assert!(time.unix_ms > 10_000_000);
    assert_eq!(a.runtime().unwrap().now_ms(), 0);
}
