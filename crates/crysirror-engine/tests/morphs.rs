// SPDX-License-Identifier: MPL-2.0
//! Morph target weights: script API → entity state → PBR extraction, and the
//! CPU blend the renderer uploads. No GPU or third-party asset.
use crysirror_asset::interchange::glb;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use crysirror_scene::view::PbrDrawable;

fn morph_model() -> Vec<u8> {
    let mut doc = glb::Document { generator: "morph test".into(), ..Default::default() };
    doc.materials.push(glb::Material::default());
    doc.meshes.push((
        "face".into(),
        vec![glb::Primitive {
            positions: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            normals: vec![[0., 0., 1.]; 3],
            uvs: vec![[0., 0.]; 3],
            indices: vec![0, 1, 2],
            material: Some(0),
            targets: vec![vec![[0., 0., 0.], [0., 0., 0.], [0., 1., 0.]]],
            target_names: vec!["smile".into()],
            ..Default::default()
        }],
    ));
    let mut node = glb::Node::named("face");
    node.mesh = Some(0);
    doc.nodes.push(node);
    doc.roots.push(0);
    doc.to_glb().unwrap()
}

#[test]
fn set_morph_reaches_entity_state_extraction_and_blend() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("assets")).unwrap();
    std::fs::write(tmp.path().join("assets/face.glb"), morph_model()).unwrap();
    let p = serde_json::json!({"schema":2,"name":"morph","content_revision":"1","canvas":[64,64],"assets":"assets",
        "clients":[{"name":"game","adapter":"crysirror-script-v1","game":"morph"}]});
    std::fs::write(tmp.path().join("project.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let mut app = App::new(Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("s")).unwrap(), "game").unwrap();
    let cx = app.context();
    let world = cx.singleton("World").unwrap();
    let info = cx.call(world, "modelInfo", &[Value::Str("face.glb".into())]).unwrap().to_json();
    assert_eq!(info["morphs"], serde_json::json!(["smile"]));
    cx.call(world, "setEnvironment", &[Value::from_json(serde_json::json!({"sun": null}))]).unwrap();
    let imported = cx
        .call(world, "importModel", &[Value::Str("face.glb".into()), Value::from_json(serde_json::json!({"asScene": true}))])
        .unwrap();
    let Some(Value::List(roots)) = imported.field("roots").cloned() else { panic!("roots") };
    let e = roots[0].as_object().unwrap();
    cx.call(e, "setMorph", &[Value::Str("smile".into()), Value::Float(0.5)]).unwrap();
    assert_eq!(cx.get(e, "animation").unwrap().to_json()["morphs"], serde_json::json!({"smile": 0.5}));
    // Unknown names are kept as data (they may match a later model revision);
    // invalid weights are rejected by validation.
    assert!(cx.call(e, "setMorph", &[Value::Str("smile".into()), Value::Float(f64::NAN)]).is_err());

    let scene = app.runtime().unwrap().host.engine.world.pbr_scene(None).unwrap().unwrap();
    let morphs = scene
        .instances
        .iter()
        .find_map(|i| match &i.drawable {
            PbrDrawable::Scene { morphs, .. } => Some(morphs.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(morphs, vec![("smile".to_string(), 0.5)]);

    // The renderer's CPU blend for that weight.
    let model = crysirror_asset::pbr::load_pbr(&morph_model(), |_| anyhow::bail!("no files")).unwrap();
    let v = model.meshes[0][0].morphed(|n| if n == "smile" { 0.5 } else { 0. }).unwrap();
    assert_eq!(v[2].position, [0., 1.5, 0.]);

    let cx = app.context();
    cx.call(e, "setMorphs", &[Value::Null]).unwrap();
    assert!(cx.get(e, "animation").unwrap().to_json().get("morphs").is_none());
}
