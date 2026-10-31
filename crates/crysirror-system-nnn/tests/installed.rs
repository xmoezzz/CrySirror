#![cfg(feature = "runtime")]
//! Factory/resource admission checks independent of original game bytes.
use crysirror_core::{state::ClientId, vm::VmAdapter};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use crysirror_system_nnn::installed::{self, Config, Entry, InstalledAdapter};
use std::time::{Duration, Instant};
#[test]
fn installed_factory_is_dormant_and_missing_source_fails_on_worker_without_scene_mutation() {
    let config = Config {
        root: "".into(),
        module: "missing.exe".into(),
        font: "default".into(),
        entry: Entry::Controller,
    };
    let vm = InstalledAdapter::new(ClientId(1), config.clone()).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    assert_eq!(diagnostics["phase"], "dormant");
    assert_eq!(diagnostics["owns_framebuffer"], false);
    assert!(vm.snapshot().is_err());
    let root = tempfile::tempdir().unwrap();
    let project:Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"NNN dormant factory","content_revision":"1","assets":"game","canvas":[800,600],"clients":[{"name":"game","adapter":"system-nnn","game":"authored","config":config}]})).unwrap();
    let mut registry = AdapterRegistry::default();
    installed::register(&mut registry).unwrap();
    let mut rt =
        Runtime::with_asset_root(project, root.path(), &root.path().join("state"), registry)
            .unwrap();
    let before = serde_json::to_value(&rt.host.engine).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let failure = loop {
        match rt.tick(0, Default::default()) {
            Ok(()) => {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
            Err(reason) => break format!("{reason:#}"),
        }
    };
    assert!(failure.contains("missing.exe"), "{failure}");
    assert_eq!(serde_json::to_value(&rt.host.engine).unwrap(), before);
    assert!(
        rt.tick(0, Default::default()).is_err(),
        "terminal failure must be retained"
    );
    let mut bad = config.clone();
    bad.module = "../escape.exe".into();
    assert!(bad.validate().is_err());
}

#[test]
fn independent_completion_pump_never_starts_dormant_vm_or_advances_clock_and_retains_failure() {
    let config=Config{root:"".into(),module:"missing.exe".into(),font:"default".into(),entry:Entry::Controller};
    let mut adapter=InstalledAdapter::new(ClientId(1),config).unwrap();
    let root=tempfile::tempdir().unwrap();std::fs::create_dir(root.path().join("game")).unwrap();
    let project:Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"NNN maintenance","content_revision":"1","assets":"game","canvas":[16,16],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]})).unwrap();
    let mut rt=Runtime::with_asset_root(project,root.path(),&root.path().join("state"),Default::default()).unwrap();
    let before=serde_json::to_value(&rt.host.engine).unwrap();
    assert!(!adapter.dependency_pending());adapter.poll_dependency(&mut rt.host).unwrap();
    let d:serde_json::Value=serde_json::from_slice(&adapter.diagnostics()).unwrap();assert_eq!(d["phase"],"dormant");
    adapter.step(&mut rt.host,Default::default(),1).unwrap();assert!(adapter.dependency_pending());
    let deadline=Instant::now()+Duration::from_secs(10);
    let error=loop{if let Err(e)=adapter.poll_dependency(&mut rt.host){break e.to_string()}assert!(Instant::now()<deadline);std::thread::yield_now();};
    assert!(error.contains("missing.exe"),"{error}");
    use crysirror_core::service::HostServices;
    assert_eq!(rt.host.now_ms(),0);assert_eq!(serde_json::to_value(&rt.host.engine).unwrap(),before);
    assert!(!adapter.dependency_pending());assert_eq!(adapter.poll_dependency(&mut rt.host).unwrap_err().to_string(),error);
}
