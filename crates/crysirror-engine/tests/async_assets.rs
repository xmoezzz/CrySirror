// SPDX-License-Identifier: MPL-2.0
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn call(app: &mut App, name: &str, args: &[Value]) -> anyhow::Result<Value> {
    let id = app.context().singleton("Assets")?;
    app.context().call(id, name, args)
}
#[test]
fn public_assets_api_loads_without_native_vm_and_rejects_foreign_ids() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(t.path().join("assets/data"), b"test").unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"asset-api","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut app = App::new(Runtime::load(&p, &t.path().join("state")).unwrap(), "a").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    assert!(app
        .typescript_declarations()
        .unwrap()
        .contains("assets: Assets"));
    let id = call(
        &mut app,
        "load",
        &[Value::Str("data".into()), Value::Str("bytes".into())],
    )
    .unwrap();
    app.context().state().unwrap().client = crysirror_core::state::ClientId(2);
    assert!(call(&mut app, "status", &[id.clone()]).is_err());
    app.context().state().unwrap().client = crysirror_core::state::ClientId(1);
    let until = std::time::Instant::now();
    loop {
        let status = call(&mut app, "status", &[id.clone()]).unwrap().to_json();
        if status["state"] == "ready" {
            assert_eq!(status["bytes"], 4);
            break;
        }
        assert!(until.elapsed() < std::time::Duration::from_secs(5));
        std::thread::yield_now();
    }
    assert!(call(
        &mut app,
        "load",
        &[Value::Str("../escape".into()), Value::Str("bytes".into())]
    )
    .is_err());
    call(&mut app, "release", &[id.clone()]).unwrap();
    assert!(call(&mut app, "status", &[id]).is_err());
}
