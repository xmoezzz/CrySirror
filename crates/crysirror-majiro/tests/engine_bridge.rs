#![cfg(feature = "runtime")]
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::time::{Duration, Instant};
fn short(b: &mut Vec<u8>, n: u16) {
    b.extend(n.to_le_bytes());
}
fn word(b: &mut Vec<u8>, n: u32) {
    b.extend(n.to_le_bytes());
}
fn module(code: &[u8]) -> Vec<u8> {
    let mut b = b"MajiroObjV1.000\0".to_vec();
    for n in [0, 0, 1, 0x12345678, 0, code.len() as u32] {
        word(&mut b, n);
    }
    b.extend(code);
    b
}
fn constant(b: &mut Vec<u8>, n: i32) {
    short(b, 0x800);
    word(b, n as u32);
}
#[test]
fn original_native_page_becomes_shared_canvas_and_unknown_stays_at_pc() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("assets")).unwrap();
    let mut code = vec![];
    // Native arguments push last parameter first: optionalcolor,height,width.
    constant(&mut code, 1);
    constant(&mut code, 24);
    constant(&mut code, 32);
    short(&mut code, 0x834);
    word(&mut code, 0x83d81f59);
    short(&mut code, 3);
    short(&mut code, 0x82f);
    let unknown_pc = code.len();
    short(&mut code, 0x835);
    word(&mut code, 0x12344321);
    short(&mut code, 0);
    short(&mut code, 0x82b);
    std::fs::write(temp.path().join("assets/start.mjo"), module(&code)).unwrap();
    let p = serde_json::json!({"schema":2,"name":"native Majiro shared page","content_revision":"1","canvas":[320,180],"assets":"assets","clients":[{"name":"native","adapter":"majiro","game":"authored","config":{"script":"start.mjo"}}]});
    let path = temp.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_majiro::adapter::register(&mut registry).unwrap();
    let mut rt = Runtime::with_registry(
        Project::load(&path).unwrap(),
        &path,
        &temp.path().join("state"),
        registry,
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Err(e) = rt.tick(16, InputFrame::default()) {
            assert!(format!("{e:#}").contains("0x12344321"));
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let inspect = rt.inspect();
    let canvas = &inspect["engine"]["canvases"]["clients"]["1"]["1"];
    assert_eq!(canvas["width"], 32);
    assert_eq!(canvas["height"], 24);
    assert_eq!(
        rt.host
            .engine
            .canvases
            .sample_pixel(crysirror_core::state::ClientId(1), 1, [0, 0])
            .unwrap(),
        [0, 0, 0, 255]
    );
    assert_eq!(
        inspect["sessions"][0]["native"]["vm"],
        "original-majiro-mjo"
    );
    assert_eq!(inspect["sessions"][0]["native"]["pc"], unknown_pc);
    assert!(!rt.all_halted());
}
