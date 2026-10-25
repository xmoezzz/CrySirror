// SPDX-License-Identifier: MPL-2.0
//! Puppet layers through the engine API (ADR 0029): name checks, motions,
//! scripted parameters, physics, saved state and the extracted meshes.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;

/// A 100×100 rig: a head deformer turned by `angle`, a breathing loop, a
/// blink and a pendulum on `sway`.
pub fn rig() -> serde_json::Value {
    let quad = |id: &str, x: f32, y: f32, w: f32, h: f32| {
        serde_json::json!({"id":id,"texture":0,"positions":[[x,y],[x+w,y],[x,y+h],[x+w,y+h]],
            "uvs":[[0,0],[1,0],[0,1],[1,1]],"indices":[0,1,2,2,1,3],"deformer":"head"})
    };
    let mut eye = quad("eye", 40., 40., 20., 10.);
    eye["masks"] = serde_json::json!(["eye_white"]);
    eye["order"] = 2.into();
    eye["keyforms"] = serde_json::json!([{"parameter":"eye_open","keys":[
        {"value":0,"offsets":[[0,5],[0,5],[0,-5],[0,-5]]},{"value":1}]}]);
    let mut white = quad("eye_white", 38., 38., 24., 14.);
    white["visible"] = false.into();
    white["order"] = 1.into();
    let mut hair = quad("hair", 20., 0., 60., 30.);
    hair["order"] = 3.into();
    hair["keyforms"] = serde_json::json!([{"parameter":"sway","keys":[{"value":-1,"offsets":[[-10,0],[-10,0],[-10,0],[-10,0]]},{"value":1,"offsets":[[10,0],[10,0],[10,0],[10,0]]}]}]);
    serde_json::json!({
        "format":"crysirror-puppet","version":1,"size":[100,100],"textures":["face.png"],
        "parameters":[{"id":"angle","min":-30,"max":30,"default":0},{"id":"eye_open","min":0,"max":1,"default":1},
            {"id":"breath","min":0,"max":1,"default":0},{"id":"sway","min":-1,"max":1,"default":0}],
        "deformers":[{"id":"head","pivot":[50,90],"keyforms":[{"parameter":"angle","keys":[{"value":-30,"rotation":-15},{"value":30,"rotation":15}]},
            {"parameter":"breath","keys":[{"value":0},{"value":1,"translation":[0,-2]}]}]}],
        "parts":[quad("face",10.,10.,80.,80.),white,eye,hair],
        "motions":[{"id":"breathe","duration_ms":2000,"looping":true,"fade_in_ms":0,"fade_out_ms":0,
            "curves":[{"parameter":"breath","keys":[[0,0],[1000,1],[2000,0]]}]},
            {"id":"blink","duration_ms":300,"fade_in_ms":0,"fade_out_ms":0,"curves":[{"parameter":"eye_open","keys":[[0,1],[150,0],[300,1]]}]}],
        "physics":[{"id":"hair","inputs":[{"parameter":"angle","weight":1}],"output":"sway"}]
    })
}

fn project(dir: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("assets/chara")).unwrap();
    std::fs::write(dir.join("assets/chara/puppet.json"), serde_json::to_vec(&rig()).unwrap()).unwrap();
    let p = serde_json::json!({"schema":2,"name":"puppets","content_revision":"1","canvas":[200,200],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"puppets",
            "systems":{"layers":[{"id":1,"rect":[0,0,200,200],"paint":{"kind":"puppet","asset":"chara/puppet.json"}}]}}]});
    std::fs::write(dir.join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    dir.join("project.json")
}

fn part(app: &App, id: &str) -> serde_json::Value {
    let frame = app.runtime().unwrap().host.engine.frame([200, 200]).unwrap();
    let f = frame.iter().find(|p| p.id == 1).unwrap().puppet.clone().expect("puppet meshes extracted");
    assert_eq!(f.textures, vec!["chara/face.png".to_string()], "textures resolve next to the rig");
    serde_json::to_value(f.parts.iter().find(|p| p.id == id).unwrap()).unwrap()
}

#[test]
fn puppet_layers_are_driven_through_the_api_and_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = App::new(Runtime::load(&project(tmp.path()), &tmp.path().join("s")).unwrap(), "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let cx = app.context();
    let puppets = cx.singleton("Puppets").unwrap();
    assert_eq!(cx.get(puppets, "count").unwrap(), Value::from(1u64));
    let params = cx.call(puppets, "parameters", &[Value::from(1u64)]).unwrap().to_json();
    assert_eq!(params.as_array().unwrap().len(), 4);
    // Unknown names and non-puppet layers fail at the call.
    let s = |v: &str| Value::from_json(serde_json::json!(v));
    assert!(cx.call(puppets, "play", &[Value::from(1u64), s("dance")]).is_err());
    assert!(cx.call(puppets, "setParameter", &[Value::from(1u64), s("nose"), Value::Float(1.)]).is_err());
    assert!(cx.call(puppets, "play", &[Value::from(2u64), s("blink")]).is_err());
    cx.call(puppets, "play", &[Value::from(1u64), s("breathe")]).unwrap();
    cx.call(puppets, "setParameter", &[Value::from(1u64), s("angle"), Value::Float(30.), Value::Float(0.)]).unwrap();
    for _ in 0..30 {
        app.frame(16, InputFrame::default()).unwrap();
    }
    let cx = app.context();
    assert_eq!(cx.call(puppets, "value", &[Value::from(1u64), s("angle")]).unwrap(), Value::Float(30.));
    let sway = cx.call(puppets, "value", &[Value::from(1u64), s("sway")]).unwrap().as_f64().unwrap();
    assert!(sway > 0.5, "the hair pendulum follows the head turn: {sway}");
    let motions = cx.call(puppets, "motions", &[Value::from(1u64)]).unwrap().to_json();
    assert_eq!(motions[0]["id"], "breathe");
    assert_eq!(motions[0]["playing"], true);
    // The extracted mesh is turned: the face's top-left corner moved.
    let face = part(&app, "face");
    let x = face["positions"][0][0].as_f64().unwrap();
    assert!((x - 10.).abs() > 5., "head rotation reaches the meshes ({x})");
    assert_eq!(part(&app, "eye")["masks"], serde_json::json!([1]));
    // State is saved with the layers and survives a JSON round trip.
    let saved = serde_json::to_value(&app.runtime().unwrap().host.engine.layers).unwrap();
    let state = &saved["puppets"]["1"]["1"]["state"];
    assert_eq!(state["motions"][0]["id"], "breathe");
    assert_eq!(state["overrides"]["angle"]["to"], 30.0);
    let restored: crysirror_scene::layer::LayerSystem = serde_json::from_value(saved).unwrap();
    restored.validate().unwrap();
    // Releasing returns the parameter to its default (no motion drives it).
    let cx = app.context();
    cx.call(puppets, "releaseParameter", &[Value::from(1u64), s("angle")]).unwrap();
    cx.call(puppets, "stop", &[Value::from(1u64)]).unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let cx = app.context();
    assert_eq!(cx.call(puppets, "value", &[Value::from(1u64), s("angle")]).unwrap(), Value::Float(0.));
    // Replacing the paint drops the puppet state.
    let layers = cx.singleton("Layers").unwrap();
    cx.call(layers, "put", &[Value::from_json(serde_json::json!({"id":1,"rect":[0,0,10,10],"paint":{"kind":"solid"}}))]).unwrap();
    assert_eq!(cx.get(puppets, "count").unwrap(), Value::from(0u64));
    // PSB is not a layer asset.
    assert!(cx
        .call(layers, "put", &[Value::from_json(serde_json::json!({"id":2,"rect":[0,0,10,10],"paint":{"kind":"puppet","asset":"m.psb"}}))])
        .is_err());
}
