// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! `AlphaMovie.dll` facade over the shared media service. Movie data is
//! local only (`CRYSIRROR_AMV_SAMPLE`); nothing is committed.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn runtime(files: &[(&str, Vec<u8>)], source: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    for (name, bytes) in files {
        std::fs::write(d.path().join("assets").join(name), bytes).unwrap();
    }
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let p = serde_json::from_value(json!({"schema":2,"name":"amv","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-amv","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    (d, r)
}

fn log(r: &Runtime) -> String {
    r.inspect()["sessions"][0]["native"]["log_tail"].to_string()
}

#[test]
fn linking_installs_the_class_and_non_movies_fail_visibly() {
    let (_d, mut r) = runtime(
        &[("bad.amv", b"not a movie".to_vec())],
        r#"
Plugins.link("AlphaMovie.dll");
var m = new AlphaMovie();
Debug.message("class " + (typeof global.AlphaMovie));
try { m.open("bad.amv"); } catch (e) { Debug.message("open failed: " + e.message); }
"#,
    );
    let result = r.tick(1, InputFrame::default());
    let log = log(&r);
    assert!(result.is_ok(), "{result:?} {log}");
    assert!(log.contains("class Object"), "{log}");
    assert!(log.contains("open failed") && log.contains("AlphaMovie header"), "{log}");
}

#[test]
#[ignore = "needs CRYSIRROR_AMV_SAMPLE (a local .amv file)"]
fn local_sample_plays_through_shared_media() {
    let bytes = std::fs::read(std::env::var_os("CRYSIRROR_AMV_SAMPLE").unwrap()).unwrap();
    let info = crysirror_video::amv::info(&mut std::io::Cursor::new(&bytes)).unwrap();
    assert!(info.frames > 1, "the playback sample must contain multiple pictures");
    let (_d, mut r) = runtime(
        &[("op.amv", bytes)],
        r#"
Plugins.link("AlphaMovie.dll");
var w = new Window();
var target = new Layer(w, null);
var m = new AlphaMovie();
m.open("op.amv");
Debug.message("info " + m.numOfFrame + " " + m.FPSRate + "/" + m.FPSScale + " " + m.screenWidth + "x" + m.screenHeight);
m.play();
global.last = -1;
System.addContinuousHandler(function(tick) {
    var f = m.showNextImage(target);
    if (f >= 0 && global.last < 0) Debug.message("picture size " + target.imageWidth + "x" + target.imageHeight);
    if (f > 0 && global.last <= 0) Debug.message("picture advanced");
    if (f >= 0) global.last = f;
});
"#,
    );
    for _ in 0..400 {
        r.tick(17, InputFrame::default()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let log = log(&r);
    assert!(log.contains("info "), "{log}");
    assert!(log.contains(&format!("picture size {}x{}", info.width, info.height)), "{log}");
    assert!(log.contains("picture advanced"), "picture index never advanced: {log}");
    println!("{log}");
}

#[test]
#[ignore = "needs CRYSIRROR_AMV_SAMPLE (a local .amv file)"]
fn copied_alpha_movie_picture_outlives_the_movie_and_is_reclaimed_after_the_copy() {
    use crysirror_core::{service::buttons, state::ClientId};
    use crysirror_scene::{canvas::CanvasNode, layer::Paint};
    let bytes = std::fs::read(std::env::var_os("CRYSIRROR_AMV_SAMPLE").unwrap()).unwrap();
    let (_dir, mut runtime) = runtime(
        &[("movie.amv", bytes)],
        r#"
        Plugins.link("AlphaMovie.dll");
        var w=new Window(), root=new Layer(w,null), target=new Layer(w,root), saved=new Layer(w,root);
        var movie=new AlphaMovie();
        movie.open("movie.amv");movie.play();
        var phase=0;
        System.addContinuousHandler(function(tick) {
            var index=movie.showNextImage(target);
            if(phase==0 && index>=2) {
                saved.setImageSize(target.imageWidth,target.imageHeight);
                saved.setSizeToImageSize();
                saved.piledCopy(0,0,target,0,0,target.imageWidth,target.imageHeight);
                phase=1;
                Debug.message("movie-copy-ready");
            } else if(phase==1 && index>=8) {
                invalidate movie;
                movie=new AlphaMovie();movie.open("movie.amv");movie.play();
                phase=2;
            } else if(phase==2 && index>=2) {
                phase=3;
                Debug.message("movie-reopened");
            } else if(phase==3 && System.getKeyState(65)) {
                invalidate saved;
                phase=4;
                Debug.message("movie-copy-released");
            }
        });
        "#,
    );
    let owner = ClientId(1);
    let asset = |runtime: &Runtime, slot| {
        let surface = runtime.host.engine.canvases.surface(owner, slot).unwrap();
        let [CanvasNode::Draw { layer }] = surface.nodes.as_slice() else {
            panic!("expected one Canvas draw");
        };
        let Paint::Image { asset, .. } = &layer.paint else { panic!("expected a movie picture") };
        asset.clone()
    };
    let advance = |runtime: &mut Runtime, marker, input| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !log(runtime).contains(marker) {
            runtime.tick(17, input).unwrap();
            assert!(std::time::Instant::now()<deadline, "waiting for {marker}: {}", log(runtime));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    advance(&mut runtime, "movie-copy-ready", InputFrame::default());
    let saved_asset = asset(&runtime, 3);
    assert!(saved_asset.starts_with("__image_resource/"));
    let saved_pixels = runtime.assets.surface(owner, &saved_asset).unwrap().rgba;

    advance(&mut runtime, "movie-reopened", InputFrame::default());
    assert_ne!(asset(&runtime, 2), saved_asset);
    assert_eq!(runtime.assets.surface(owner, &saved_asset).unwrap().rgba, saved_pixels);

    advance(&mut runtime, "movie-copy-released", InputFrame {
        buttons_down: buttons::KEY_A,
        buttons_pressed: buttons::KEY_A,
        ..InputFrame::default()
    });
    assert!(runtime.assets.surface(owner, &saved_asset).is_none());
}
