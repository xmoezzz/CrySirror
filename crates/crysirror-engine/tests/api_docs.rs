// SPDX-License-Identifier: MPL-2.0
//! docs/api is generated from the live registry and specs. This test fails
//! when the checked-in reference is stale or an API signature change has no
//! CHANGES.md entry. Regenerate with
//! `CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs`.
use crysirror_engine::{
    api_doc::{self, ApiInput, DataType, sample},
    builtin,
    object::ClassRegistry,
    services::COMMAND_API,
};
use crysirror_scene::{canvas, components as c, layer, material, physics, ui, view, world};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}
fn variants<T: DeserializeOwned>() -> Vec<String> {
    let err = serde_json::from_value::<T>(json!({"op": "__probe__"}))
        .err()
        .expect("probe accepted")
        .to_string();
    err.split("expected one of ")
        .nth(1)
        .expect("serde variant list")
        .split(',')
        .map(|s| s.trim().trim_matches('`').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
fn d(name: &'static str, used_by: &'static str, sample: Option<serde_json::Value>) -> DataType {
    DataType {
        name,
        used_by,
        sample,
    }
}

fn generate() -> api_doc::ApiDocs {
    let mut classes = ClassRegistry::default();
    builtin::register(&mut classes).unwrap();
    let mut items = Vec::new();
    for f in [
        "crates/crysirror-scene/src/view.rs",
        "crates/crysirror-scene/src/components.rs",
        "crates/crysirror-scene/src/world.rs",
        "crates/crysirror-scene/src/layer.rs",
        "crates/crysirror-scene/src/canvas.rs",
        "crates/crysirror-core/src/service.rs",
        "crates/crysirror-runtime/src/media.rs",
        "crates/crysirror-runtime/src/assets_async.rs",
        "crates/crysirror-tasks/src/lib.rs",
        "crates/crysirror-scene/src/ui.rs",
        "crates/crysirror-scene/src/material.rs",
        "crates/crysirror-scene/src/sequence.rs",
        "crates/crysirror-scene/src/physics.rs",
        "crates/crysirror-scene/src/lib.rs",
        "crates/crysirror-puppet/src/lib.rs",
    ] {
        items.extend(api_doc::parse_rust_items(&read(f)));
    }
    let config = |f: &str| {
        api_doc::parse_rust_items(&read(f))
            .into_iter()
            .find(|i| i.name == "Config")
    };
    let e = json!({});
    let extent = json!({"extent": [1, 1, 1]});
    let data = vec![
        d("AssetKind", "`Assets.load(path, kind)`", Some(json!("model"))),
        d("AssetState", "`Assets.status(id)`", None),
        d("AssetTicket", "Rust async asset handle; store/client checked", None),
        d("AssetStatus", "`Assets.status(id)`", None),
        d("TaskStats", "`Assets.stats()` CPU and I/O pools", None),
        d(
            "Environment",
            "`World.setEnvironment`, `world.set_environment`",
            sample::<view::Environment>(e.clone()),
        ),
        d("Sun", "`Environment.sun`", sample::<view::Sun>(e.clone())),
        d("Fog", "`Environment.fog`", sample::<view::Fog>(e.clone())),
        d(
            "AutoExposure",
            "`Environment.auto_exposure`",
            sample::<view::AutoExposure>(e.clone()),
        ),
        d(
            "ScreenSpaceReflections",
            "`Environment.reflections` (ADR 0023)",
            sample::<view::ScreenSpaceReflections>(e.clone()),
        ),
        d(
            "GeometryLod",
            "`Environment.geometry_lod` (ADR 0026)",
            sample::<view::GeometryLod>(e.clone()),
        ),
        d(
            "VirtualShadows",
            "`Environment.virtual_shadows` (ADR 0027)",
            sample::<view::VirtualShadows>(e.clone()),
        ),
        d("Tonemap", "`Environment.tonemap`", None),
        d(
            "EntitySpec",
            "`World.spawn`, `ClientSpec.systems.entities`",
            sample::<world::EntitySpec>(json!({"id": 1, "name": "e"})),
        ),
        d(
            "Transform",
            "`EntitySpec.transform`",
            sample::<world::Transform>(e.clone()),
        ),
        d(
            "Material",
            "`EntitySpec.material`, `Entity.setMaterial`",
            sample::<material::MaterialSpec>(e.clone()),
        ),
        d("AlphaMode", "`Material.alpha_mode`", None),
        d("NormalEncoding", "`Material.normal_encoding`", None),
        d(
            "LightSpec",
            "`EntitySpec.light`, `Entity.setLight` (ADR 0018)",
            sample::<c::LightSpec>(e.clone()),
        ),
        d("LightShape", "`LightSpec.shape`", None),
        d("LightUnits", "`LightSpec.units`", None),
        d("Falloff", "`LightSpec.falloff`", None),
        d(
            "ReflectionProbeSpec",
            "`EntitySpec.reflection_probe`, `Entity.setReflectionProbe` (ADR 0024/0025)",
            sample::<c::ReflectionProbeSpec>(extent.clone()),
        ),
        d(
            "IrradianceVolumeSpec",
            "`EntitySpec.irradiance_volume`, `Entity.setIrradianceVolume` (ADR 0028)",
            sample::<c::IrradianceVolumeSpec>(extent.clone()),
        ),
        d(
            "ColliderSpec",
            "`EntitySpec.collider`, `Entity.setCollider` (ADR 0012)",
            sample::<c::ColliderSpec>(json!({"shape": {"kind": "sphere", "radius": 1}})),
        ),
        d("ColliderShape", "`ColliderSpec.shape`", None),
        d("BodyKind", "`ColliderSpec.body`", None),
        d(
            "CollisionGroups",
            "`ColliderSpec.groups`",
            sample::<c::CollisionGroups>(e.clone()),
        ),
        d(
            "SystemsSpec",
            "`ClientSpec.systems`",
            sample::<crysirror_scene::SystemsSpec>(e.clone()),
        ),
        d(
            "PhysicsScene",
            "World snapshot physics declarations (ADR 0045)",
            sample::<physics::PhysicsScene>(e.clone()),
        ),
        d(
            "ClientPhysicsScene",
            "`ClientSpec.systems.physics`, `Physics.gravity`, `WorldCommand.set_physics`",
            sample::<physics::ClientPhysicsScene>(e.clone()),
        ),
        d(
            "BodyForces",
            "`ClientPhysicsScene.forces`",
            sample::<physics::BodyForces>(json!({"generation":1})),
        ),
        d(
            "KinematicTarget",
            "`ClientPhysicsScene.kinematic_targets`",
            sample::<physics::KinematicTarget>(json!({"generation":1,"pose":{}})),
        ),
        d(
            "JointFrame",
            "`JointSpec` local frames and `KinematicTarget.pose`",
            sample::<physics::JointFrame>(e.clone()),
        ),
        d(
            "JointMotor",
            "`JointSpec.motor`",
            sample::<physics::JointMotor>(e.clone()),
        ),
        d(
            "JointSpec",
            "`Physics.createJoint`, `PhysicsJoint.setSpec`",
            None,
        ),
        d(
            "JointHandle",
            "Client-scoped generational physics joint identity",
            None,
        ),
        d("JointState", "`ClientPhysicsScene.joints`", None),
        d(
            "EmitterSpec",
            "`EntitySpec.particles`, `Entity.setParticles` (ADR 0014)",
            sample::<c::EmitterSpec>(e.clone()),
        ),
        d(
            "LayerSpec",
            "`Layers.put`, `ClientSpec.systems.layers`",
            None,
        ),
        d("Paint", "`LayerSpec.paint`", None),
        d(
            "PositionedGlyph",
            "`Paint.glyph_run.glyphs`",
            sample::<layer::PositionedGlyph>(json!({"character":"あ","baseline":[0,20]})),
        ),
        d("GlyphRevealUnit", "`GlyphReveal.unit`", None),
        d(
            "GlyphReveal",
            "`Paint.glyph_run.reveal`",
            sample::<layer::GlyphReveal>(json!({"interval_ms":40})),
        ),
        d("GlyphRevealStatus", "Engine glyph reveal query", None),
        d(
            "CanvasSurface",
            "`Canvases.get`",
            sample::<canvas::CanvasSurface>(json!({"width":640,"height":480,"nodes":[]})),
        ),
        d("CanvasNode", "`CanvasSurface.nodes`", None),
        d(
            "CanvasCommand",
            "`Canvases.apply`, `ServiceBatch.canvases`",
            None,
        ),
        d(
            "ServiceBatch",
            "Engine service mutation batch",
            sample::<crysirror_scene::ServiceBatch>(e.clone()),
        ),
        d(
            "MediaRequest",
            "`HostServices.media_request` (script `Media.play` options use `loop`, `audio`, `gain`)",
            None,
        ),
        d("MediaPhase", "`HostServices.media_status`", None),
        d("MediaInfo", "`HostServices.media_status`", None),
        d(
            "MediaOptions",
            "Rust shared media service options (script uses `loop` instead of `looping`)",
            sample::<crysirror_runtime::media::MediaOptions>(e.clone()),
        ),
        d("MediaState", "`MediaStatus.state`", None),
        d("MediaStatus", "`Media.status`", None),
        d(
            "LayerFilter",
            "`Layers.setFilter` (ADR 0021)",
            sample::<layer::LayerFilter>(e.clone()),
        ),
        d("Blend", "`LayerSpec.blend`", None),
        d("ToneCurve", "`LayerFilter.tone_curve`", None),
        d("LayerMask", "`LayerFilter.mask`", None),
        d("Distortion", "`LayerFilter.distortion`", None),
        d("TransitionSpec", "`Layers.transition` (ADR 0022)", None),
        d("TransitionEffect", "`TransitionSpec.effect`", None),
        d("Easing", "`TransitionSpec.easing`", None),
        d(
            "WidgetSpec",
            "`Ui.put`, `ClientSpec.systems.widgets`",
            sample::<ui::WidgetSpec>(json!({"id": 1, "rect": [0, 0, 1, 1]})),
        ),
        d("WidgetKind", "`WidgetSpec.kind`", None),
        d("Flow", "`WidgetSpec.flow`", None),
        d("PuppetCommand", "`Puppets.*`, `layer.puppet`", None),
        d(
            "PuppetRig",
            "`Paint.puppet.asset` (engine rig JSON, ADR 0029)",
            None,
        ),
        d("Parameter", "`PuppetRig.parameters`", None),
        d("Deformer", "`PuppetRig.deformers`", None),
        d("DeformerKey", "`Deformer.keyforms[].keys`", None),
        d("Part", "`PuppetRig.parts`", None),
        d("PartKey", "`Part.keyforms[].keys`", None),
        d("PartBlend", "`Part.blend`", None),
        d("Motion", "`PuppetRig.motions`", None),
        d("Curve", "`Motion.curves`", None),
        d("Pendulum", "`PuppetRig.physics`", None),
        d("SequenceSpec", "`World.playSequence` (ADR 0040)", None),
        d("Track", "`SequenceSpec.tracks`", None),
        d("TrackProperty", "`Track.property`", None),
        d(
            "KeyInterpolation",
            "`Track.interpolation`, `CameraTrack.interpolation`",
            None,
        ),
        d("TrackNoise", "`Track.noise`, `CameraTrack.shake`", None),
        d("CameraTrack", "`SequenceSpec.camera`", None),
        d("CameraKey", "`CameraTrack.keys`", None),
        d("SequenceEvent", "`SequenceSpec.events`", None),
        d(
            "WorldCommand",
            "World mutation batches (`ServiceBatch.world`)",
            None,
        ),
        d(
            "LayerCommand",
            "Layer mutation batches (`ServiceBatch.layers`)",
            None,
        ),
        d("UiCommand", "UI mutation batches (`ServiceBatch.ui`)", None),
    ];
    api_doc::generate(&ApiInput {
        classes: &classes,
        globals: builtin::GLOBALS,
        commands: COMMAND_API,
        command_variants: vec![
            ("canvas", variants::<canvas::CanvasCommand>()),
            ("world", variants::<world::WorldCommand>()),
            ("layer", variants::<layer::LayerCommand>()),
            ("ui", variants::<ui::UiCommand>()),
        ],
        command_enums: vec![
            ("canvas", "CanvasCommand"),
            ("world", "WorldCommand"),
            ("layer", "LayerCommand"),
            ("ui", "UiCommand"),
        ],
        data,
        adapters: vec![
            (
                "reallive",
                config("crates/crysirror-reallive/src/adapter.rs"),
            ),
            (
                "siglus",
                config("crates/crysirror-siglus-vm/src/adapter.rs"),
            ),
        ],
        items,
    })
}

#[test]
fn api_reference_is_current_and_every_signature_change_is_logged() {
    let docs = generate();
    let dir = root().join("docs/api");
    let files = [
        ("REFERENCE.md", &docs.markdown),
        ("api.json", &docs.json),
        ("crysirror.d.ts", &docs.typescript),
    ];
    if std::env::var_os("CRYSIRROR_BLESS").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        for (f, s) in files {
            std::fs::write(dir.join(f), s).unwrap();
        }
    }
    let mut stale = Vec::new();
    for (f, s) in files {
        if std::fs::read_to_string(dir.join(f)).ok().as_deref() != Some(s.as_str()) {
            stale.push(f);
        }
    }
    assert!(
        stale.is_empty(),
        "docs/api {stale:?} are stale; run CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs"
    );
    let changes = std::fs::read_to_string(dir.join("CHANGES.md")).unwrap_or_default();
    assert!(
        changes.contains(&format!("`{}`", docs.fingerprint)),
        "API signature changed: add an entry with fingerprint `{}` to docs/api/CHANGES.md",
        docs.fingerprint
    );
}

#[test]
fn every_class_property_and_method_is_documented() {
    let mut classes = ClassRegistry::default();
    builtin::register(&mut classes).unwrap();
    let missing = api_doc::undocumented(&classes);
    assert!(missing.is_empty(), "undocumented API: {missing:?}");
}

#[test]
fn every_listed_data_type_has_source_docs_and_samples_where_given() {
    let docs = generate();
    assert!(
        !docs.markdown.contains("(no source declaration found)"),
        "a data type or adapter config was not found in source"
    );
    let v: serde_json::Value = serde_json::from_str(&docs.json).unwrap();
    // Specs given a minimal sample must parse it: a renamed field or a new
    // required field shows up here instead of as an empty defaults column.
    for name in [
        "Environment",
        "EntitySpec",
        "IrradianceVolumeSpec",
        "ColliderSpec",
        "WidgetSpec",
    ] {
        let entry = v["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap();
        assert!(
            entry["fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| !f["default"].is_null()),
            "{name}: sample did not parse"
        );
    }
}

#[test]
fn native_capabilities_and_config_are_in_public_reference() {
    let docs = generate();
    for term in [
        "Canvases",
        "Media",
        "glyph_run",
        "GlyphReveal",
        "ordered_dither",
        "set_title",
        "set_cursor_visible",
        "text_field",
        "insert_text",
        "backspace",
        "focus",
        "font_alias",
        "start_scene",
    ] {
        assert!(docs.markdown.contains(term), "public API missing {term}");
    }
    for op in variants::<canvas::CanvasCommand>() {
        assert!(
            COMMAND_API
                .iter()
                .any(|(system, command, _)| *system == "canvas" && *command == op),
            "canvas command missing: {op}"
        );
    }
    let config = api_doc::parse_rust_items(&read("crates/crysirror-reallive/src/adapter.rs"))
        .into_iter()
        .find(|i| i.name == "Config")
        .expect("RealLive Config declaration");
    assert_eq!(
        config
            .members
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["root", "font_alias", "encoding", "start_scene"]
    );
}

#[test]
fn documented_text_input_methods_mutate_engine_owned_state_without_native_vm() {
    use crysirror_engine::{App, Value};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    std::fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&json!({
            "schema":2,"name":"ui-api","content_revision":"1","canvas":[64,64],"assets":"assets",
            "clients":[{"name":"main","adapter":"crysirror-script-v1","game":"test"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        crysirror_runtime::Runtime::load(
            &root.path().join("project.json"),
            &root.path().join("state"),
        )
        .unwrap(),
        "main",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let ui = app.context().singleton("Ui").unwrap();
    let mut call = |name: &str, args: serde_json::Value| {
        let args = args
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(Value::from_json)
            .collect::<Vec<_>>();
        app.context().call(ui, name, &args)
    };
    call(
        "put",
        json!([{"id":7,"rect":[0,0,60,20],"kind":"text_field","font":"default","max_length":2}]),
    )
    .unwrap();
    call("focusTextField", json!([7])).unwrap();
    call("insertText", json!(["あ😀x"])).unwrap();
    assert!(call("insertText", json!(["\n"])).is_err());
    call("backspace", json!([7])).unwrap();
    call("insertText", json!(["界"])).unwrap();
    call("setTitle", json!(["engine UI"])).unwrap();
    call("setCursorVisible", json!([false])).unwrap();
    let ui = &app.runtime().unwrap().host.engine.ui;
    let owner = ui.focus.unwrap().owner;
    assert_eq!(ui.get(owner, 7).unwrap().text, "あ界");
    assert_eq!(ui.title, Some((owner, "engine UI".into())));
    assert_eq!(ui.cursor, Some((owner, false)));
}
