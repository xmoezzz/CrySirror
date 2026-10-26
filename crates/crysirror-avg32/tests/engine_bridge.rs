// SPDX-License-Identifier: MPL-2.0
//! Self-authored TPC32 bytecode; no game data or bundled fonts.
use crysirror_core::{
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::{Value, json};

fn number(value: u32) -> Vec<u8> {
    let mut len = 1usize;
    while len < 5 && (value >> 4) >> (8 * (len - 1)) != 0 {
        len += 1;
    }
    let mut out = vec![((len as u8) << 4) | (value as u8 & 15)];
    for i in 0..len - 1 {
        out.push((value >> (4 + 8 * i)) as u8);
    }
    out
}
fn scene(code: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x63];
    bytes[..5].copy_from_slice(b"TPC32");
    bytes[0x28..0x2c].copy_from_slice(&1i32.to_le_bytes());
    bytes[0x5f..0x63].copy_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(code);
    bytes
}
fn wav() -> Vec<u8> {
    let frames = 4800;
    let bytes = frames * 2;
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36u32 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&48000u32.to_le_bytes());
    out.extend_from_slice(&96000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for _ in 0..frames {
        out.extend_from_slice(&1000i16.to_le_bytes());
    }
    out
}
fn fixture(code: &[u8], extra: Value) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("SEEN001.TXT"), scene(code)).unwrap();
    image::RgbaImage::from_pixel(640, 480, image::Rgba([40u8, 80, 120, 255]))
        .save(assets.join("bg.png"))
        .unwrap();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255u8, 255, 255, 255]))
        .save(assets.join("icon.png"))
        .unwrap();
    std::fs::write(assets.join("sound.wav"), wav()).unwrap();
    let mut config = json!({"scenes":{"1":"SEEN001.TXT"},"message_interval_ms":10,"resources":{"bg":"bg.png","music":"sound.wav","se":"sound.wav"},"sound_effects":{"3":"sound.wav"},"message_icon":"icon.png"});
    for (key, value) in extra.as_object().unwrap() {
        config[key] = value.clone();
    }
    let project=serde_json::from_value(json!({"schema":2,"name":"avg32-api","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"avg32","game":"synthetic","config":config}]})).unwrap();
    std::fs::write(
        temp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_avg32::register(&mut registry).unwrap();
    let runtime = Runtime::with_registry(
        project,
        &temp.path().join("project.json"),
        &temp.path().join("state"),
        registry,
    )
    .unwrap();
    (temp, runtime)
}
fn diag(r: &Runtime) -> Value {
    r.inspect()["sessions"][0]["native"].clone()
}
#[test]
fn original_native_arithmetic_images_audio_and_waits_drive_shared_state() {
    let mut code = vec![0x3b];
    code.extend(number(1));
    code.extend(number(7));
    code.extend([0x0b, 0x09, b'b', b'g', 0]);
    code.extend(number(0));
    code.extend([0x0e, 0x01]);
    code.extend(b"music\0");
    code.extend([0x19, 0x01]);
    code.extend(number(40));
    code.push(0x3c);
    code.extend(number(1));
    code.extend(number(2));
    code.extend([0x60, 0x20]);
    let (_temp, mut r) = fixture(&code, json!({}));
    r.tick(0, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 7);
    assert_eq!(diag(&r)["owns_framebuffer"], false);
    let canvas = r.host.engine.canvases.surface(ClientId(1), 0).unwrap();
    let nodes = serde_json::to_string(&canvas.nodes).unwrap();
    assert!(nodes.contains("bg.png"), "{nodes}");
    assert!(r.host.audio_playing(ClientId(1), 20_000).unwrap());
    r.tick(0, InputFrame::default()).unwrap();
    r.tick(39, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 7);
    r.tick(1, InputFrame::default()).unwrap();
    r.tick(1, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 9);
    assert!(diag(&r)["engine_api_calls"].as_u64().unwrap() >= 3);
}
#[test]
fn unsupported_effect_save_and_movie_fail_instead_of_hosting_or_hiding_them() {
    for bytes in [&[0x13, 1][..], &[0x60, 0x03, 0x11][..], &[0x0e, 0x50][..]] {
        let (_temp, mut r) = fixture(bytes, json!({}));
        let e = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
        assert!(e.contains("AVG32"), "{e}");
        assert!(r.host.engine.layers.frame([640, 480]).unwrap().is_empty());
    }
}

#[test]
fn native_timer_starts_when_its_vm_boots_instead_of_inheriting_the_host_age() {
    let (_temp, mut r) = fixture(&[0x19, 0x06, 0x11, 0x60, 0x20], json!({}));
    r.tick(5_000, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 0);
}
#[test]
fn partial_images_fail_before_mutating_the_engine_canvas() {
    let code = [0x0b, 0x09, b'b', b'g', 0, 0x10, 0x60, 0x20];
    let (temp, mut r) = fixture(&code, json!({}));
    image::RgbaImage::from_pixel(1, 1, image::Rgba([1u8, 2, 3, 255]))
        .save(temp.path().join("assets/bg.png"))
        .unwrap();
    let error = format!("{:#}", r.tick(0, InputFrame::default()).unwrap_err());
    assert!(error.contains("partial/masked"), "{error}");
    let nodes = &r
        .host
        .engine
        .canvases
        .surface(ClientId(1), 0)
        .unwrap()
        .nodes;
    assert_eq!(nodes.len(), 1);
    assert!(matches!(
        &nodes[0],
        crysirror_scene::canvas::CanvasNode::Draw {
            layer: crysirror_scene::layer::LayerSpec {
                paint: crysirror_scene::layer::Paint::Solid {
                    color: [0, 0, 0, 255]
                },
                ..
            }
        }
    ));
}
#[test]
fn native_hidden_choice_indices_are_preserved_in_engine_ui() {
    // Choose V1: first entry is visible, second condition false, third visible.
    let code = [
        0x58, 0x02, 0x11, 0x22, 0xfe, b'O', b'n', b'e', 0, 0, 0x28, 0x3b, 0x10, 0x11, 0x29, 0xfe,
        b'H', 0, 0, 0xfe, b'T', b'w', b'o', 0, 0, 0x23, 0x60, 0x20,
    ];
    let (_temp, mut r) = fixture(&code, json!({}));
    r.tick(0, InputFrame::default()).unwrap();
    assert!(r.host.engine.ui.get(1, 41_000).is_some());
    assert!(r.host.engine.ui.get(1, 41_001).is_none());
    assert!(r.host.engine.ui.get(1, 41_002).is_some());
    // Pointer press/release is routed by the engine UI before the VM polls its event.
    let input = InputFrame {
        pointer_x: 50,
        pointer_y: 380,
        buttons_down: buttons::MOUSE_LEFT,
        ..Default::default()
    };
    r.tick(1, input).unwrap();
    r.tick(
        1,
        InputFrame {
            pointer_x: 50,
            pointer_y: 380,
            ..Default::default()
        },
    )
    .unwrap();
    r.tick(1, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 3);
}
#[test]
#[ignore = "reads an explicitly supplied licensed system font at runtime; no font is bundled"]
fn native_text_uses_shared_grid_glyph_resources_and_native_clocked_reveal() {
    let (_temp, mut r) = fixture(&[0xfe, b'A', b'B', 0, 0x03, 0x60, 0x20], json!({}));
    let font = std::env::var_os("CRYSIRROR_TEST_FONT")
        .expect("set CRYSIRROR_TEST_FONT to a licensed system font");
    let bytes = std::fs::read(font).unwrap();
    r.host.text.add_font("default", bytes.clone()).unwrap();
    r.assets.text.add_font("default", bytes).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let text = r.host.engine.layers.get(ClientId(1), 40_000).unwrap();
    let crysirror_scene::layer::Paint::GlyphRun { glyphs, reveal, .. } = &text.paint else {
        panic!("not engine-owned glyphs")
    };
    assert_eq!(glyphs.iter().map(|g| g.character).collect::<String>(), "AB");
    assert_eq!(reveal.as_ref().unwrap().initial_visible, 0);
    r.tick(10, InputFrame::default()).unwrap();
    assert_eq!(
        r.host
            .engine
            .layers
            .reveal_status(ClientId(1), 40_000)
            .unwrap()
            .visible,
        1
    );
    r.tick(10, InputFrame::default()).unwrap();
    assert_eq!(
        r.host
            .engine
            .layers
            .reveal_status(ClientId(1), 40_000)
            .unwrap()
            .visible,
        2
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(r.host.engine.layers.get(ClientId(1), 40_001).is_some());
    r.tick(
        1,
        InputFrame {
            buttons_down: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
    r.tick(1, InputFrame::default()).unwrap();
    assert!(r.host.engine.layers.get(ClientId(1), 40_001).is_none());
    assert!(format!("{:#}", r.snapshot().unwrap_err()).contains("AVG32 portable checkpoints"));
}

use crysirror_tasks::{Task, TaskPool};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
struct Gate {
    gate: Arc<(Mutex<bool>, Condvar)>,
    tasks: Vec<Task<()>>,
}
impl Gate {
    fn block(p: &TaskPool) -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut tasks = vec![];
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..p.stats().workers {
            let g = gate.clone();
            let tx = tx.clone();
            tasks.push(
                p.spawn(move |_| {
                    tx.send(())?;
                    let (v, _) = g
                        .1
                        .wait_timeout_while(g.0.lock().unwrap(), Duration::from_secs(5), |v| !*v)
                        .unwrap();
                    anyhow::ensure!(*v, "gate timeout");
                    Ok(())
                })
                .unwrap(),
            );
        }
        for _ in 0..tasks.len() {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Self { gate, tasks }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
        for t in self.tasks.drain(..) {
            t.wait().unwrap();
        }
    }
}

#[test]
fn original_avg32_audio_event_waits_for_engine_workers_without_running_following_arithmetic() {
    let mut code = vec![0x0e, 0x02];
    code.extend(b"music\0");
    code.push(0x3b);
    code.extend(number(1));
    code.extend(number(9));
    code.extend([0x60, 0x20]);
    let (_temp, mut r) = fixture(&code, json!({}));
    r.set_audio_nonblocking(true);
    assert!(!r.assets.is_nonblocking());
    let gate = Gate::block(&r.assets.io_tasks());
    r.tick(0, InputFrame::default()).unwrap();
    let first = diag(&r)["instructions"].clone();
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
        assert_eq!(diag(&r)["instructions"], first);
        assert_eq!(diag(&r)["values"][1], 0);
    }
    drop(gate);
    let end = Instant::now() + Duration::from_secs(5);
    while !r.host.audio_playing(ClientId(1), 20_000).unwrap() {
        r.tick(0, InputFrame::default()).unwrap();
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    r.tick(99, InputFrame::default()).unwrap();
    assert_eq!(diag(&r)["values"][1], 0);
    r.tick(1, InputFrame::default()).unwrap();
    for _ in 0..3 {
        r.tick(0, InputFrame::default()).unwrap();
    }
    assert_eq!(diag(&r)["values"][1], 9);
}
#[test]
fn original_avg32_audio_failure_keeps_native_event_and_following_arithmetic_pending() {
    let mut code = vec![0x0e, 0x01];
    code.extend(b"music\0");
    code.push(0x3b);
    code.extend(number(1));
    code.extend(number(9));
    code.extend([0x60, 0x20]);
    let (temp, mut r) = fixture(&code, json!({}));
    std::fs::write(temp.path().join("assets/sound.wav"), b"invalid wave").unwrap();
    r.assets.set_nonblocking(true);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(e) = r.tick(0, InputFrame::default()) {
            assert!(format!("{e:#}").contains("shared audio preparation failed"));
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(diag(&r)["values"][1], 0);
}
