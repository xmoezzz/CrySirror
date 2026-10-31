//! Actual shared Runtime owner services; no original engine, window or renderer.
use crysirror_core::{
    service::{HostServices, InputFrame},
    state::ClientId,
    vm::VmAdapter,
};
use crysirror_runtime::Runtime;
use crysirror_system4::worker_adapter::{Config, NativeAdapter};
use serde_json::json;
#[test]
#[ignore = "reads owner-installed original Rance7 assets; no original executable runs"]
fn owner_constructor_output_commits_to_shared_log_before_pc_advance() {
    let root = std::env::var("CRYSIRROR_SYSTEM4_ROOT").unwrap();
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("project.json");
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"system4-shared-log","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"native","adapter":"crysirror-script-v1","game":"owner"}]})).unwrap()).unwrap();
    let mut runtime = Runtime::load(&p, &t.path().join("state")).unwrap();
    let mut files = crysirror_core::vfs::MemoryProvider::new();
    for name in ["戦国ランス.ain", "DLL/OutputLog.dll"] {
        files.insert(name, std::fs::read(std::path::Path::new(&root).join(name)).unwrap()).unwrap();
    }
    runtime.mount_provider(ClientId(1), "", std::sync::Arc::new(files)).unwrap();
    let mut vm = NativeAdapter::new(
        ClientId(1),
        Config {
            canvas: Some([800,600]), // Original System40.ini logical view.
            font_aliases: ["default".into(), "default".into()],
            program: "戦国ランス.ain".into(),
            reviewed_output_log_dll: Some("DLL/OutputLog.dll".into()),
        },
    )
    .unwrap();
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut error = None;
    while std::time::Instant::now() < limit {
        match vm.step(&mut runtime.host, InputFrame::default(), 10000) {
            Ok(_) => {
                runtime.tick(1, Default::default()).unwrap();
                std::thread::yield_now();
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    let native: serde_json::Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    let log = runtime.host.log_snapshot(ClientId(1)).unwrap();
    println!(
        "native={native} log_lines={} last={:?} error={error:?}",
        log.records.len(),
        log.records.last()
    );
    assert!(
        !log.records.is_empty(),
        "original output was never committed"
    );
    assert!(native["log_calls"].as_u64().unwrap() > 0);
    assert_ne!(native["native_location"].as_str().unwrap(), "pc=0xa4a");
    assert!(
        error.is_some(),
        "profile unexpectedly claims game completion"
    );
}
