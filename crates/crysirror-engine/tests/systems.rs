// SPDX-License-Identifier: MPL-2.0
//! Terrain collision, level streaming and spatial audio through the API.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use std::path::Path;

fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/engine/assets/lantern.glb"),
        tmp.path().join("assets/lantern.glb"),
    )
    .unwrap();
    // Runtime-authored PCM, so the shared audio mixer can really decode it.
    let samples = vec![0i16; 8000];
    let data_size = (samples.len() * 2) as u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data_size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());
    for s in samples {
        wav.extend(s.to_le_bytes())
    }
    std::fs::write(tmp.path().join("assets/hum.wav"), wav).unwrap();
    let p = serde_json::json!({"schema":2,"name":"sys","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"sys"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(),
        "game",
    )
    .unwrap();
    (tmp, app)
}
fn camera(app: &mut App, eye: [f32; 3]) {
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let target = [eye[0] + 1., eye[1], eye[2]];
    cx.call(
        world,
        "setCamera",
        &[v(serde_json::json!({"eye": eye, "target": target}))],
    )
    .unwrap();
}

#[test]
fn character_lands_on_procedural_terrain_heightfield() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let spec = serde_json::json!({"kind": "terrain", "resolution": 65, "size": [40, 40], "height": 6, "seed": 3});
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "land", "geometry": spec, "collider": {"shape": {"kind": "mesh"}}}))]).unwrap();
    let hero = cx
        .call(world, "spawn", &[v(serde_json::json!({"name": "hero", "position": [5, 20, -3],
            "collider": {"shape": {"kind": "capsule", "radius": 0.3, "half_height": 0.5}, "body": "character"}}))])
        .unwrap()
        .as_object()
        .unwrap();
    for _ in 0..300 {
        cx.call(
            hero,
            "moveCharacter",
            &[
                Value::vec3([0., -0.2, 0.]),
                Value::Float(0.016),
                Value::Null,
            ],
        )
        .unwrap();
    }
    let p = cx.get(hero, "position").unwrap().as_vec3().unwrap();
    let ground = cx
        .call(
            world,
            "raycast",
            &[
                Value::vec3([5., 50., -3.]),
                Value::vec3([0., -1., 0.]),
                Value::Float(100.),
                v(serde_json::json!({"$object": hero.0})),
            ],
        )
        .unwrap();
    let gy = ground.field("point").unwrap().as_vec3().unwrap()[1];
    let s: crysirror_scene::terrain::TerrainSpec = serde_json::from_value(
        serde_json::json!({"resolution": 65, "size": [40, 40], "height": 6, "seed": 3}),
    )
    .unwrap();
    let expect =
        crysirror_scene::terrain::height_at(&s, &crysirror_scene::terrain::procedural(&s), 5., -3.)
            .unwrap();
    assert!(
        (gy - expect).abs() < 0.15,
        "heightfield {gy} vs sampled {expect}"
    );
    assert!(
        (p[1] - (gy + 0.8)).abs() < 0.2,
        "standing on terrain: {p:?} ground {gy}"
    );
}

#[test]
fn levels_stream_in_and_out_with_hysteresis_and_survive_save_load() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(
        world,
        "streamLevel",
        &[
            Value::Str("village".into()),
            v(serde_json::json!({
        "source": "lantern.glb", "center": [50, 0, 0], "load_radius": 20, "unload_radius": 30,
        "transform": {"translation": [50, 0, 0]}, "collide": true})),
        ],
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let count = |app: &mut App| {
        let cx = app.context();
        let w = cx.singleton("World").unwrap();
        cx.get(w, "count").unwrap().as_i64().unwrap()
    };
    camera(&mut app, [0., 2., 0.]);
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(count(&mut app), 0, "far away: not loaded");
    camera(&mut app, [35., 2., 0.]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while count(&mut app) == 0 {
        app.frame(16, InputFrame::default()).unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "asynchronous level did not become ready"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        count(&mut app),
        4,
        "root + lantern hierarchy loaded atomically"
    );
    // A raycast hits the streamed level's collision.
    let cx = app.context();
    let w = cx.singleton("World").unwrap();
    let hit = cx
        .call(
            w,
            "raycast",
            &[
                Value::vec3([50., 5., 0.]),
                Value::vec3([0., -1., 0.]),
                Value::Float(10.),
                Value::Null,
            ],
        )
        .unwrap();
    assert!(!matches!(hit, Value::Null), "streamed collision");
    let save = cx.singleton("Save").unwrap();
    cx.call(save, "save", &[Value::Str("s.bin".into())])
        .unwrap();
    camera(&mut app, [25., 2., 0.]); // inside the unload radius: stays loaded
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(count(&mut app), 4);
    camera(&mut app, [10., 2., 0.]);
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(count(&mut app), 0, "unloaded beyond 30");
    let cx = app.context();
    let save = cx.singleton("Save").unwrap();
    cx.call(save, "load", &[Value::Str("s.bin".into())])
        .unwrap();
    assert_eq!(count(&mut app), 4, "restored loaded state");
}

#[test]
fn spatial_voice_attenuates_with_listener_distance_without_restarting() {
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let audio = cx.singleton("Audio").unwrap();
    let torch = cx
        .call(
            world,
            "spawn",
            &[v(
                serde_json::json!({"name": "torch", "position": [10, 0, 0]}),
            )],
        )
        .unwrap();
    cx.call(
        audio,
        "playAt",
        &[
            Value::Int(3),
            v(serde_json::json!({"asset": "hum.wav", "looping": true})),
            v(serde_json::json!({"follow": torch, "radius": 21, "min_distance": 1})),
        ],
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let gain = |app: &App| {
        let rt = app.runtime().unwrap();
        let (k, voice) = rt.host.scene.voices().iter().next().unwrap();
        (voice.gain_milli, rt.host.scene.voice_epoch(*k))
    };
    camera(&mut app, [10., 0., 1.]);
    app.frame(16, InputFrame::default()).unwrap();
    let (near, e1) = gain(&app);
    assert_eq!(near, 1000);
    camera(&mut app, [10., 0., 11.]); // halfway between min and radius
    app.frame(16, InputFrame::default()).unwrap();
    let (mid, e2) = gain(&app);
    assert_eq!((mid, e2), (250, e1), "attenuated, same voice epoch");
    let cx = app.context();
    let w = cx.singleton("World").unwrap();
    let t = cx
        .call(w, "find", &[Value::Str("torch".into())])
        .unwrap()
        .as_object()
        .unwrap();
    cx.set(t, "position", Value::vec3([10., 0., 30.])).unwrap(); // the source follows the entity
    app.frame(16, InputFrame::default()).unwrap();
    // d = 19: t = 18/20, gain = (1 - 0.9)^2 = 0.01.
    assert_eq!(gain(&app).0, 10, "follows entity");
}

#[test]
fn vertical_raycasts_hit_terrain_everywhere() {
    // Regression: exactly vertical rays used to slip between heightfield cells.
    let (_tmp, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world, "spawn", &[v(serde_json::json!({"name": "v", "geometry": {"kind": "terrain", "resolution": 161, "size": [120, 120], "height": 14, "seed": 11, "frequency": 2.2}, "collider": {"shape": {"kind": "mesh"}}}))]).unwrap();
    let mut misses = 0;
    for i in 0..40 {
        for j in 0..40 {
            let (x, z) = (-55. + i as f32 * 2.75, -55. + j as f32 * 2.75);
            if matches!(
                cx.call(
                    world,
                    "raycast",
                    &[
                        Value::vec3([x, 60., z]),
                        Value::vec3([0., -1., 0.]),
                        Value::Float(120.),
                        Value::Null
                    ]
                )
                .unwrap(),
                Value::Null
            ) {
                misses += 1;
            }
        }
    }
    assert_eq!(misses, 0);
    let nav = cx.call(world, "buildNavigation", &[v(serde_json::json!({"min": [-58, -2, -58], "max": [58, 30, 58], "cell": 0.5, "max_step": 0.6, "max_slope": 50}))]).unwrap();
    assert_eq!(nav.field("walkable").unwrap().as_i64().unwrap(), 232 * 232);
}

#[test]
fn offline_streaming_fence_preserves_logical_clock_and_lifecycle_count() {
    let (_t, mut app) = app();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    cx.call(world,"streamLevel",&[Value::Str("offline".into()),v(serde_json::json!({"source":"lantern.glb","center":[0,0,0],"load_radius":20,"unload_radius":30,"pbr":true}))]).unwrap();
    camera(&mut app, [0., 2., 0.]);
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let time = app.runtime().unwrap().now_ms();
    let frame = app.engine().unwrap().frame;
    app.prepare_render_offline().unwrap();
    assert_eq!(app.runtime().unwrap().now_ms(), time);
    assert_eq!(app.engine().unwrap().frame, frame);
    assert_eq!(app.runtime().unwrap().host.engine.world.len(), 4);
    let cx = app.context();
    let profiler = cx.singleton("Profiler").unwrap();
    let report = cx.call(profiler, "snapshot", &[]).unwrap().to_json();
    assert!(
        report["stages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "cpu:asset.Level.read_prepare")
    );
    let trace = cx.call(profiler, "trace", &[]).unwrap().to_json();
    assert!(trace["traceEvents"].as_array().unwrap().len() > 0);
    cx.call(profiler, "clear", &[]).unwrap();
    assert_eq!(
        cx.call(profiler, "snapshot", &[]).unwrap().to_json()["retained_samples"],
        0
    );
}
