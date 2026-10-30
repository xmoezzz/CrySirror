use crysirror_core::service::{InputFrame, buttons};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn write_project(root: &Path, bp: Vec<u8>, font: Option<&Path>) -> Runtime {
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/main._bp"), bp).unwrap();
    let png = crysirror_image::encode_png_uncompressed(&crysirror_image::ImageSurface {
        width: 2,
        height: 2,
        rgba: [240, 50, 20, 255].repeat(4),
    })
    .unwrap();
    std::fs::write(root.join("assets/pixel.png"), png).unwrap();
    let samples = vec![5000i16; 8000];
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(16036u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(16000u32.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(root.join("assets/tone.wav"), wav).unwrap();
    let mut fonts = serde_json::Map::new();
    if let Some(font) = font {
        std::fs::copy(font, root.join("assets/font.bin")).unwrap();
        fonts.insert("main".into(), "font.bin".into());
    }
    let config = if fonts.is_empty() {
        serde_json::json!({"script":"main._bp"})
    } else {
        serde_json::json!({"script":"main._bp","font":"main"})
    };
    let project = serde_json::json!({"schema":2,"name":"BGI authored native BP","content_revision":"1","canvas":[320,180],"assets":"assets","fonts":fonts,"clients":[{"name":"native","adapter":"bgi","game":"authored","config":config}]});
    let path = root.join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_bgi::register(&mut registry).unwrap();
    Runtime::with_registry(
        Project::load(&path).unwrap(),
        &path,
        &root.join("state"),
        registry,
    )
    .unwrap()
}
#[test]
fn native_bytecode_uses_engine_canvases_sprites_audio_and_input() {
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = write_project(
        temp.path(),
        crysirror_bgi::fixture::scene(false, true, true),
        None,
    );
    let until = Instant::now() + Duration::from_secs(5);
    let mut peak = 0f64;
    loop {
        runtime.tick(16, InputFrame::default()).unwrap();
        let inspect = runtime.inspect();
        peak = peak.max(inspect["channel_audio"]["peak"].as_f64().unwrap_or(0.));
        if inspect["engine"]["canvases"]["clients"]["1"]["0"].is_object() && peak > 0.01 {
            break;
        }
        assert!(
            Instant::now() < until,
            "native resource preparation never completed"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!runtime.all_halted(), "native input loop was skipped");
    let inspect = runtime.inspect();
    let canvas = &inspect["engine"]["canvases"]["clients"]["1"]["0"];
    assert_eq!(canvas["width"], 320);
    assert_eq!(canvas["height"], 180);
    assert!(inspect["engine"]["layers"]["clients"]["1"].is_object());
    for _ in 0..8 {
        runtime
            .tick(
                16,
                InputFrame {
                    buttons_down: buttons::ENTER,
                    buttons_pressed: buttons::ENTER,
                    ..Default::default()
                },
            )
            .unwrap();
    }
    assert!(runtime.all_halted());
    assert!(peak > 0.01);
}
#[test]
fn unknown_native_operation_fails_instead_of_returning_a_default() {
    let mut bp = crysirror_bgi_vm::Builder::default();
    bp.int(1).call(0x80, 0xe3).ret();
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = write_project(temp.path(), bp.finish(), None);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(error) = runtime.tick(16, InputFrame::default()) {
            assert!(format!("{error:#}").contains("unmapped BGI native call"));
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
#[ignore = "requires owner/system BGI_TEST_FONT at runtime; fonts are never shipped"]
fn plain_native_text_becomes_engine_glyph_runs_without_a_private_painter() {
    let font = std::env::var_os("BGI_TEST_FONT").expect("BGI_TEST_FONT");
    let temp = tempfile::tempdir().unwrap();
    let mut runtime = write_project(
        temp.path(),
        crysirror_bgi::fixture::scene(true, false, false),
        Some(Path::new(&font)),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !runtime.all_halted() {
        runtime.tick(16, InputFrame::default()).unwrap();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let json = serde_json::to_string(&runtime.inspect()["engine"]).unwrap();
    assert!(json.contains("glyph_run"));
    assert!(json.contains("main"));
    assert!(
        !json.contains("__surface"),
        "text must remain shared glyph state, not composed bitmap upload"
    );
}
