// SPDX-License-Identifier: MPL-2.0
//! Modular VFX through the same public object API available to JS/TS and tools.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::{json, Value as Json};

fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&json!({
            "schema":2,"name":"vfx-api","content_revision":"1","canvas":[64,64],"assets":"assets",
            "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"vfx"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&dir.path().join("project.json"), &dir.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn effect() -> Json {
    json!({"parameters":{"wind":{"kind":"vector","value":[1,0,0]},"rate":{"kind":"float","value":0}},
        "emitters":[{"id":"sparks","capacity":32,"spawn":{"rate":{"parameter":"rate"},"bursts":[{"time":0,"count":4}],"looping":false},
        "initialize":{"lifetime":[2,2],"speed":[0,0]},"update":[{"op":"acceleration","value":{"parameter":"wind"}}]}]})
}
fn particles(app: &App) -> Vec<crysirror_vfx::Particle> {
    app.runtime()
        .unwrap()
        .render_pbr()
        .unwrap()
        .unwrap()
        .vfx
        .unwrap()
        .emitters
        .iter()
        .flat_map(|e| e.particles.iter().cloned())
        .collect()
}

#[test]
fn script_vfx_controls_mutate_shared_state_without_a_renderer_or_vm() {
    let (_dir, mut app) = app();
    let vfx = app.context().singleton("Vfx").unwrap();
    let plan = app
        .context()
        .call(vfx, "compile", &[Value::from_json(effect())])
        .unwrap()
        .to_json();
    assert_eq!(plan["emitters"][0]["id"], "sparks");
    let entity = app
        .context()
        .call(
            vfx,
            "play",
            &[
                Value::from_json(effect()),
                Value::from_json(json!({"position":[2,0,0]})),
            ],
        )
        .unwrap();
    let id = entity.as_object().unwrap();
    app.prepare_render_offline().unwrap();
    let initial = particles(&app);
    assert_eq!(initial.len(), 4);
    assert!(initial
        .iter()
        .all(|p| p.age == 0. && (p.position[0] - 2.).abs() < 1e-5));
    app.frame(100, InputFrame::default()).unwrap();
    app.prepare_render_offline().unwrap();
    let moving = particles(&app);
    assert!(moving[0].velocity[0] > 0.);
    app.context()
        .call(vfx, "pause", &[entity.clone(), Value::Bool(true)])
        .unwrap();
    app.prepare_render_offline().unwrap();
    let frozen = particles(&app);
    app.frame(200, InputFrame::default()).unwrap();
    app.prepare_render_offline().unwrap();
    assert_eq!(particles(&app), frozen, "pause must freeze existing state");
    app.context()
        .call(
            vfx,
            "setParameter",
            &[
                entity.clone(),
                Value::Str("wind".into()),
                Value::from_json(json!([0, 2, 0])),
            ],
        )
        .unwrap();
    assert!(app
        .context()
        .call(
            vfx,
            "setParameter",
            &[entity.clone(), Value::Str("wind".into()), Value::Float(4.)]
        )
        .is_err());
    assert!(app
        .context()
        .call(
            vfx,
            "setParameter",
            &[
                entity.clone(),
                Value::Str("missing".into()),
                Value::Float(1.)
            ]
        )
        .is_err());
    app.context()
        .call(vfx, "pause", &[entity.clone(), Value::Bool(false)])
        .unwrap();
    app.context().call(vfx, "stop", &[entity.clone()]).unwrap();
    app.prepare_render_offline().unwrap();
    app.frame(100, InputFrame::default()).unwrap();
    app.prepare_render_offline().unwrap();
    let stopped = particles(&app);
    assert_eq!(stopped.len(), 4);
    assert_eq!(stopped[0].id, initial[0].id);
    assert!(
        stopped[0].velocity[1] > 0.,
        "parameter update must affect existing particles"
    );
    app.context().call(vfx, "restart", &[entity]).unwrap();
    app.prepare_render_offline().unwrap();
    let restarted = particles(&app);
    assert_eq!(restarted.len(), 4);
    assert_eq!(restarted[0].age, 0.);
    assert_eq!(
        app.context().get(id, "vfx").unwrap().to_json()["revision"],
        1
    );
    let stats = app.context().call(vfx, "stats", &[]).unwrap().to_json();
    assert_eq!(stats["cpu_live_particles"], 4);
    assert!(stats["gpu_live_particles"].is_null());
    let events = app
        .context()
        .call(vfx, "drainEvents", &[])
        .unwrap()
        .to_json();
    assert!(!events.as_array().unwrap().is_empty());
    assert_eq!(
        app.context()
            .call(vfx, "drainEvents", &[])
            .unwrap()
            .to_json(),
        json!([])
    );
}

#[test]
fn unsupported_gpu_programs_and_competing_legacy_emitter_fail_before_device_creation() {
    let (_dir, mut app) = app();
    let vfx = app.context().singleton("Vfx").unwrap();
    let mut bad = effect();
    bad["emitters"][0]["target"] = json!("gpu");
    bad["emitters"][0]["render"] = json!({"kind":"ribbon","width":0.1});
    assert!(app
        .context()
        .call(vfx, "compile", &[Value::from_json(bad)])
        .is_err());
    let world = app.context().singleton("World").unwrap();
    assert!(app
        .context()
        .call(
            world,
            "spawn",
            &[Value::from_json(
                json!({"name":"ambiguous","vfx":effect(),"particles":{}})
            )]
        )
        .is_err());
}
