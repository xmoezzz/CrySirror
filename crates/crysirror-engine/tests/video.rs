// SPDX-License-Identifier: MPL-2.0
//! World video playback through the script API, runtime decoding into a
//! client-scoped dynamic surface, pause/seek/loop/stop (ADR 0020).
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

fn mean(rgba: &[u8]) -> [u32; 3] {
    let n = (rgba.len() / 4) as u32;
    let mut s = [0u32; 3];
    for p in rgba.chunks_exact(4) {
        for c in 0..3 {
            s[c] += p[c] as u32;
        }
    }
    s.map(|x| x / n)
}

#[test]
fn scripted_video_drives_a_surface_by_world_time() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets/movie")).unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../crysirror-video/tests/fixtures/red_blue.wmv"),
        tmp.path().join("assets/movie/op.wmv"),
    )
    .unwrap();
    let p = serde_json::json!({"schema":2,"name":"video","content_revision":"1","canvas":[64,48],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"video"}]});
    std::fs::write(tmp.path().join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut app = App::new(Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(), "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let surface = cx
        .call(world, "playVideo", &[Value::Str("op".into()), Value::Str("movie/op.wmv".into())])
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(surface, "__surface/1000");
    assert!(cx.call(world, "playVideo", &[Value::Str("x".into()), Value::Str("../escape.wmv".into())]).is_err());
    assert!(cx
        .call(world, "playVideo", &[Value::Str("x".into()), Value::Str("a.wmv".into()), Value::from_json(serde_json::json!({"speed":2}))])
        .is_err());
    let client = app.runtime().unwrap().client_id("game").unwrap();
    let color = |app: &App| mean(&app.runtime().unwrap().assets.surface(client, &surface).expect("video surface").rgba);
    app.frame(16, InputFrame::default()).unwrap();
    assert!(color(&app)[0] > 250, "red at start");
    for _ in 0..40 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    assert!(color(&app)[2] > 250, "blue after ~0.66 s");
    // Pause keeps the frame and the clock.
    let cx = app.context();
    cx.call(world, "pauseVideo", &[Value::Str("op".into())]).unwrap();
    let before = cx.call(world, "video", &[Value::Str("op".into())]).unwrap().to_json();
    for _ in 0..10 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    let after = cx.call(world, "video", &[Value::Str("op".into())]).unwrap().to_json();
    assert_eq!(before["time"], after["time"]);
    assert_eq!((after["width"].as_i64(), after["height"].as_i64(), after["playing"].as_bool()), (Some(64), Some(48), Some(false)));
    // Seeking backwards rebuilds the decoder and shows red again.
    cx.call(world, "seekVideo", &[Value::Str("op".into()), Value::Int(0)]).unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    assert!(color(&app)[0] > 250, "red after seek to 0");
    // Non-looping playback ends on its last frame.
    let cx = app.context();
    cx.call(world, "resumeVideo", &[Value::Str("op".into())]).unwrap();
    for _ in 0..120 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    let st = cx.call(world, "video", &[Value::Str("op".into())]).unwrap().to_json();
    assert_eq!(st["ended"], true);
    cx.call(world, "stopVideo", &[Value::Str("op".into())]).unwrap();
    assert!(cx.call(world, "video", &[Value::Str("op".into())]).unwrap().to_json().is_null());
    assert!(cx.call(world, "pauseVideo", &[Value::Str("op".into())]).is_err());
}
