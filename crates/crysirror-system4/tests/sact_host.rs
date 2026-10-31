//! Shared Runtime resource acceptance; original files/fonts are runtime inputs.
use crysirror_core::{
    service::{FontLoadSource, FontLoadSpec, FontLoadState, HostServices, InputFrame},
    state::ClientId,
    vm::VmAdapter,
};
use crysirror_runtime::Runtime;
use crysirror_system4::{
    sact::Sact,
    worker_adapter::{Config, NativeAdapter},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn runtime() -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"sact shared resources","content_revision":"1","canvas":[800,600],"assets":"assets","clients":[{"name":"data","adapter":"crysirror-script-v1","game":"test"}]}"#).unwrap();
    let r = Runtime::load(&p, &t.path().join("state")).unwrap();
    (t, r)
}
fn load_fonts(runtime: &mut Runtime) {
    let bytes: Arc<[u8]> =
        std::fs::read(std::env::var("CRYSIRROR_TEST_FONT").expect("system font runtime path"))
            .unwrap()
            .into();
    let library = runtime.host.font_library(ClientId(1)).unwrap();
    for name in ["test_gothic", "test_mincho"] {
        let job = library
            .begin(FontLoadSpec {
                name: name.into(),
                source: FontLoadSource::Bytes(bytes.clone()),
                face_index: 0,
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match library.status(job).unwrap() {
                FontLoadState::Ready(_) => {
                    library.release_job(job).unwrap();
                    break;
                }
                FontLoadState::Failed(e) => panic!("{e}"),
                FontLoadState::Pending => {
                    assert!(Instant::now() < until);
                    std::thread::yield_now();
                }
            }
        }
    }
}
#[test]
fn sact_init_rejects_unpublished_fonts_without_host_resource_commit() {
    let (_t, mut r) = runtime();
    let mut s = Sact::new([800, 600], ["test_gothic".into(), "test_mincho".into()]).unwrap();
    assert!(
        format!("{:#}", s.init(ClientId(1), &mut r.host).unwrap_err()).contains("not published")
    );
    assert!(!s.initialized && !s.font_ready && !s.audio_ready);
    assert!(r.host.engine.canvases.surface(ClientId(1), 1).is_none());
    assert!(r.host.engine.layers.get(ClientId(1), 1).is_none());
    assert!(s.init(ClientId(2), &mut r.host).is_err());
    assert!(Sact::new([0, 600], ["g".into(), "m".into()]).is_err());
}
#[test]
#[ignore = "reads caller-selected system font at runtime; no font bytes committed"]
fn sact_init_publishes_engine_owned_hidden_main_and_repeat_is_idempotent() {
    let (_t, mut r) = runtime();
    load_fonts(&mut r);
    let mut s = Sact::new([800, 600], ["test_gothic".into(), "test_mincho".into()]).unwrap();
    assert_eq!(s.init(ClientId(1), &mut r.host).unwrap(), 1);
    assert!(s.initialized && s.font_ready && s.audio_ready);
    let main = r.host.engine.canvases.surface(ClientId(1), 1).unwrap();
    assert_eq!([main.width, main.height], [800, 600]);
    assert_eq!(
        r.host
            .engine
            .canvases
            .sample_pixel(ClientId(1), 1, [799, 599])
            .unwrap(),
        [0, 0, 0, 255]
    );
    assert!(!r.host.engine.layers.get(ClientId(1), 16).unwrap().visible);
    let before = serde_json::to_value(&r.host.engine).unwrap();
    assert_eq!(s.init(ClientId(1), &mut r.host).unwrap(), 1);
    assert_eq!(serde_json::to_value(&r.host.engine).unwrap(), before);
    assert_eq!(s.calls, 2);
    assert!(s.init(ClientId(2), &mut r.host).is_err());
    assert_eq!(s.calls, 2);
    assert!(r.host.engine.canvases.surface(ClientId(2), 1).is_none());
}
#[test]
#[ignore = "reads owner Rance7 AIN/DLL plus system font, executes no original program"]
fn owner_rance7_sact_shared_init_advances_to_next_real_frontier() {
    let root = std::path::PathBuf::from(std::env::var("CRYSIRROR_SYSTEM4_ROOT").unwrap());
    let (_t, mut r) = runtime();
    load_fonts(&mut r);
    let mut files = crysirror_core::vfs::MemoryProvider::new();
    for name in ["戦国ランス.ain", "DLL/OutputLog.dll"] {
        files
            .insert(name, std::fs::read(root.join(name)).unwrap())
            .unwrap();
    }
    r.mount_provider(ClientId(1), "", Arc::new(files)).unwrap();
    let mut vm = NativeAdapter::new(
        ClientId(1),
        Config {
            program: "戦国ランス.ain".into(),
            reviewed_output_log_dll: Some("DLL/OutputLog.dll".into()),
            canvas: Some([800, 600]),
            font_aliases: ["test_gothic".into(), "test_mincho".into()],
        },
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    let mut error = None;
    while Instant::now() < until {
        match vm.step(&mut r.host, InputFrame::default(), 10000) {
            Ok(_) => {
                r.tick(1, Default::default()).unwrap();
                std::thread::yield_now();
            }
            Err(e) => {
                error = Some(e);
                break;
            }
        }
    }
    let d: serde_json::Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    println!("native={d} error={error:?}");
    assert_eq!(
        d["sact"]["initialized"], true,
        "real Init was not committed"
    );
    assert_eq!(d["sact"]["font_ready"], true);
    assert_eq!(d["sact"]["audio_ready"], true);
    assert!(r.host.engine.canvases.surface(ClientId(1), 1).is_some());
    assert!(
        error.is_some(),
        "bounded native profile must not imply complete game"
    );
    assert!(!d["native_location"]
        .as_str()
        .unwrap()
        .starts_with("pc=0x6f6 "));
}
