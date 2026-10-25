// SPDX-License-Identifier: MPL-2.0
//! Real Oxc -> Boa -> object API -> shared Runtime Rapier simulation.
use crysirror_core::service::InputFrame;
use crysirror_engine::App;
use crysirror_runtime::Runtime;
use crysirror_script_js::JsHost;

#[test]
fn typescript_impulse_and_queries_use_the_shared_runtime_physics_service() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let project = serde_json::json!({"schema":2,"name":"physics-ts","content_revision":"1","assets":"assets","canvas":[32,32],"clients":[{"name":"game","adapter":"crysirror-script-v1","game":"physics-ts"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    let source = r#"
        type Vector = [number, number, number];
        function onStart(): void {
            physics.gravity = [0,0,0];
            const box = world.spawn({name:'box',collider:{body:'dynamic',mass:2,shape:{kind:'box',half_extents:[0.5,0.5,0.5]}}});
            const impulse:Vector = [2,0,0];
            box.applyImpulse(impulse);
            const hit=physics.raycast([-2,0,0],[1,0,0],10);
            engine.log(hit.entity.name);
            engine.log(String(resources.transforms().includes('xor')));
        }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("physics", "physics.ts", source).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    for _ in 0..60 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let state = app.context().state().unwrap();
    assert_eq!(
        state.log.iter().map(String::as_str).collect::<Vec<_>>(),
        ["box", "true"]
    );
    let world = &state.runtime.host.engine.world;
    let h = world.handle(state.client, 1).unwrap();
    let entity = world.entity(h).unwrap();
    assert!((entity.transform.translation[0] - 0.96).abs() < 0.001);
    assert!((entity.velocity[0] - 1.).abs() < 0.0001);
    let stats = state.runtime.physics.stats(state.client.0);
    assert_eq!(stats.bodies_created, 1);
    assert_eq!(stats.colliders_created, 1);
    assert_eq!(stats.simulation_steps, 60);
}
