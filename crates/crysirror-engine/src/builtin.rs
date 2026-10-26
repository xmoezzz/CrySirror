// SPDX-License-Identifier: MPL-2.0
//! Built-in classes: Engine, World, Entity, Fs and Client.
//!
//! These wrap shared runtime services; they add no second source of truth.
//! World mutations are submitted as transactional `ServiceBatch`es scoped to
//! the App's client, exactly like VM-originated mutations.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::FileSystem,
    state::ClientId,
};
use crysirror_scene::{
    model::Geometry,
    world::{EntityHandle, EntitySpec, Transform, Tween, WorldCommand},
    ServiceBatch,
};
use std::{collections::BTreeMap, sync::Arc, sync::OnceLock};

/// Script-visible singletons: (global name, class).
pub const GLOBALS: &[(&str, &str)] = &[
    ("engine", "Engine"),
    ("world", "World"),
    ("fs", "Fs"),
    ("layers", "Layers"),
    ("ui", "Ui"),
    ("audio", "Audio"),
    ("save", "Save"),
    ("input", "Input"),
    ("localization", "Localization"),
    ("text", "Text"),
    ("bitmap", "Bitmap"),
    ("resources", "Resources"),
    ("puppets", "Puppets"),
    ("physics", "Physics"),
    ("canvases", "Canvases"),
    ("media", "Media"),
    ("assets", "Assets"),
    ("profiler", "Profiler"),
    ("vfx", "Vfx"),
];

pub fn register(r: &mut ClassRegistry) -> Result<()> {
    r.register(Arc::new(EngineClass))?;
    r.register(Arc::new(WorldClass))?;
    r.register(Arc::new(EntityClass))?;
    r.register(Arc::new(FsClass))?;
    r.register(Arc::new(ClientClass))?;
    crate::services::register(r)?;
    crate::physics_api::register(r)?;
    crate::canvas_api::register(r)?;
    crate::assets_api::register(r)?;
    crate::profiler_api::register(r)?;
    crate::vfx_api::register(r)?;
    Ok(())
}

fn prop(name: &'static str, ty: T, writable: bool, doc: &'static str) -> PropertyInfo {
    PropertyInfo {
        name,
        ty,
        writable,
        doc,
    }
}
fn method(
    name: &'static str,
    params: Vec<(&'static str, T)>,
    returns: T,
    doc: &'static str,
) -> MethodInfo {
    MethodInfo {
        name,
        params,
        returns,
        doc,
    }
}
fn arg<'a>(args: &'a [Value], i: usize, what: &str) -> Result<&'a Value> {
    args.get(i)
        .with_context(|| format!("missing argument '{what}'"))
}
fn opt(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !matches!(v, Value::Null))
}

pub(crate) fn submit(cx: &mut EngineContext, commands: Vec<WorldCommand>) -> Result<()> {
    let s = cx.state()?;
    let client = s.client;
    s.runtime.host.engine.submit(
        client,
        &ServiceBatch {
            world: commands,
            ..Default::default()
        },
    )
}
pub(crate) fn entity_object(cx: &mut EngineContext, id: u32, generation: u32) -> Result<Value> {
    let client = cx.state()?.client.0;
    let payload = Value::Map(BTreeMap::from([
        ("client".into(), Value::Int(client.into())),
        ("id".into(), Value::Int(id.into())),
        ("generation".into(), Value::Int(generation.into())),
    ]));
    let oid = cx.state()?.objects.intern("Entity", payload)?;
    Ok(Value::object(oid, "Entity"))
}
/// Resolve an Entity object to its live world handle; stale handles fail.
pub(crate) fn handle(cx: &mut EngineContext, this: ObjectId) -> Result<EntityHandle> {
    let s = cx.state()?;
    let entry = s.objects.get(this)?;
    ensure!(
        entry.payload.field("client").and_then(|v| v.as_i64().ok()) == Some(i64::from(s.client.0)),
        "entity belongs to another client"
    );
    ensure!(
        entry.class == "Entity",
        "object is a {}, not an Entity",
        entry.class
    );
    let id = entry.payload.field("id").context("entity id")?.as_i64()?;
    let generation = entry
        .payload
        .field("generation")
        .context("entity generation")?
        .as_i64()?;
    let h = EntityHandle {
        client: s.client.0,
        id: u32::try_from(id)?,
        generation: u32::try_from(generation)?,
    };
    s.runtime.host.engine.world.entity(h)?;
    Ok(h)
}
fn spec(cx: &mut EngineContext, this: ObjectId) -> Result<EntitySpec> {
    let h = handle(cx, this)?;
    Ok(cx.state()?.runtime.host.engine.world.entity(h)?.clone())
}
fn entity_id_of(cx: &mut EngineContext, v: &Value) -> Result<u32> {
    let oid = v.as_object()?;
    Ok(handle(cx, oid)?.id)
}
pub(crate) fn next_entity_id(cx: &mut EngineContext) -> Result<u32> {
    let s = cx.state()?;
    let max = s
        .runtime
        .host
        .engine
        .world
        .entities(s.client)
        .iter()
        .map(|e| e.0)
        .max()
        .unwrap_or(0);
    max.checked_add(1).context("entity id exhausted")
}

// ---------------------------------------------------------------- Engine
pub struct EngineClass;
fn engine_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Engine",
        doc: "Application-wide services: time, input, logging, lifecycle requests.",
        properties: vec![
            prop("frame", T::Int, false, "Completed App frames."),
            prop("timeMs", T::Int, false, "Runtime clock in milliseconds."),
            prop(
                "fixedStepMs",
                T::Int,
                false,
                "Fixed update step in milliseconds.",
            ),
            prop(
                "input",
                T::Map,
                false,
                "This frame's input: buttonsDown, buttonsPressed, pointer.",
            ),
        ],
        methods: vec![
            method(
                "log",
                vec![("message", T::String)],
                T::Void,
                "Append a line to the bounded engine log.",
            ),
            method(
                "quit",
                vec![],
                T::Void,
                "Ask the host to stop after this frame.",
            ),
            method(
                "clients",
                vec![],
                T::List(Box::new(T::Object("Client"))),
                "Project clients, including native VMs.",
            ),
        ],
    })
}
impl Class for EngineClass {
    fn info(&self) -> &ClassInfo {
        engine_info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        Ok(match p {
            "frame" => s.frame.into(),
            "timeMs" => s.runtime.now_ms().into(),
            "fixedStepMs" => s.fixed_step_ms.into(),
            "input" => Value::Map(BTreeMap::from([
                ("buttonsDown".into(), s.input.buttons_down.into()),
                ("buttonsPressed".into(), s.input.buttons_pressed.into()),
                (
                    "pointer".into(),
                    Value::List(vec![
                        Value::Int(s.input.pointer_x.into()),
                        Value::Int(s.input.pointer_y.into()),
                    ]),
                ),
            ])),
            _ => unreachable!(),
        })
    }
    fn call(&self, cx: &mut EngineContext, _: ObjectId, m: &str, args: &[Value]) -> Result<Value> {
        match m {
            "log" => {
                let line = arg(args, 0, "message")?.as_str()?;
                ensure!(line.len() <= 4096, "log line exceeds 4 KiB");
                cx.log(line)?;
            }
            "quit" => cx.state()?.quit_requested = true,
            "clients" => {
                let names: Vec<String> = cx
                    .state()?
                    .runtime
                    .project
                    .clients
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                let mut out = Vec::new();
                for name in names {
                    let oid = cx.state()?.objects.intern("Client", Value::Str(name))?;
                    out.push(Value::object(oid, "Client"));
                }
                return Ok(Value::List(out));
            }
            _ => unreachable!(),
        }
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------- World
pub struct WorldClass;
fn world_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "World",
        doc: "The shared dynamic 3D world, scoped to this App's client.",
        properties: vec![
            prop("count", T::Int, false, "Entities owned by this client."),
            prop("revision", T::Int, false, "Committed world mutation revision."),
            prop("camera", T::Optional(Box::new(T::Map)), false, "Current camera or null (project camera applies)."),
            prop("environment", T::Optional(Box::new(T::Map)), false, "Current PBR environment or null (legacy renderer)."),
        ],
        methods: vec![
            method("spawn", vec![("spec", T::Map)], T::Object("Entity"),
                "Spawn {name, parent?, position?, rotation?, scale?, geometry?: 'cube'|{vertices,indices}, color?, visible?, velocity?}."),
            method("find", vec![("name", T::String)], T::Optional(Box::new(T::Object("Entity"))), "First entity with this name."),
            method("entities", vec![], T::List(Box::new(T::Object("Entity"))), "All entities of this client."),
            method("importModel", vec![("path", T::String), ("options", T::Optional(Box::new(T::Map)))], T::Map,
                "Import a model as entities: glTF/GLB/VRM, binary FBX 7.x or MMD PMX (detected by content); returns {roots, report} with unrepresented source features in report.dropped. options: {parent?, prefix?, pbr?: true for textured PBR materials, asScene?: true for one animated/skinned entity}."),
            method("setCamera", vec![("camera", T::Map)], T::Void, "Camera {eye, target, up?, fov?, near?, far?} (degrees, world units)."),
            method("setEnvironment", vec![("environment", T::Map)], T::Void,
                "Select the PBR renderer: {hdr?, intensity?, rotation_degrees?, exposure?, tonemap?: 'pbr_neutral'|'aces'|'none', show_sky?, bloom?, sun?: {direction, color, intensity, shadows}}."),
            method("pause", vec![("paused", T::Bool)], T::Void, "Pause/resume this client's world simulation."),
            method("groundPose", vec![("position", T::Vec3), ("yawDegrees", T::Number), ("halfLength", T::Number), ("maxDrop", T::Number), ("ignore", T::Optional(Box::new(T::Object("Entity"))))],
                T::Optional(Box::new(T::Map)), "Ground under a body's front and back (along yaw, +Z forward): {height (mean ground y), pitch (degrees, nose up positive), normal, front, back} or null when neither end has ground within maxDrop below position."),
            method("sphereCast", vec![("origin", T::Vec3), ("direction", T::Vec3), ("radius", T::Number), ("maxDistance", T::Number), ("ignore", T::Optional(Box::new(T::Object("Entity"))))],
                T::Optional(Box::new(T::Map)), "Sweep a sphere: {entity, point (sphere centre at impact), normal, distance} or null. Use for cameras and probes that must not clip into geometry."),
            method("raycast", vec![("origin", T::Vec3), ("direction", T::Vec3), ("maxDistance", T::Number), ("ignore", T::Optional(Box::new(T::Object("Entity"))))],
                T::Optional(Box::new(T::Map)), "Closest collider hit {entity, point, normal, distance} or null."),
            method("defineBehaviorTree", vec![("name", T::String), ("root", T::Map)], T::Void,
                "Define a behaviour tree: nodes {kind: selector|sequence|invert|cooldown|check|has_tag|near|wait|move_to|face|ability|set|perceive|succeed|fail, ...}."),
            method("buildNavigation", vec![("settings", T::Map)], T::Map,
                "Bake the navigation grid from level collision {min, max, cell?, agent_radius?, agent_height?, max_step?, max_slope?}; returns {width, height, walkable}."),
            method("streamLevel", vec![("name", T::String), ("level", T::Optional(Box::new(T::Map)))], T::Void,
                "Register a streaming volume {source, center, load_radius, unload_radius, transform?, pbr?, collide?} (null removes and unloads)."),
            method("playSequence", vec![("name", T::String), ("sequence", T::Map), ("startMs", T::Optional(Box::new(T::Number)))], T::Void,
                "Start a level sequence {duration_ms, looping?, speed?, camera?:{keys:[{time_ms, eye, target, fov_y_degrees?}], interpolation?, shake?}, tracks:[{entity, property:'translation'|'rotation'|'scale'|'light_intensity'|'light_color'|'light_range'|'visible', keys:[{time_ms, value:[..]}], interpolation?:'linear'|'step'|'smooth'|'spline', noise?:{amplitude, frequency_hz, seed?}}], events:[{time_ms, name}]}."),
            method("stopSequence", vec![("name", T::String)], T::Void, "Remove a sequence; animated properties keep their last values."),
            method("sequenceStatus", vec![("name", T::String)], T::Any, "{time_ms, playing, loops, duration_ms} or null."),
            method("sequenceEvents", vec![("name", T::String)], T::List(Box::new(T::String)), "Event names fired since the last call (consumed)."),
            method("playVideo", vec![("name", T::String), ("asset", T::String), ("options", T::Optional(Box::new(T::Map)))], T::String,
                "Play an OMV/Ogg-Theora/MPEG-1/2/WMV asset from the start; options {loop?: false, paused?: false}. Returns the surface path to use as a layer image or material texture."),
            method("pauseVideo", vec![("name", T::String)], T::Void, "Pause a video (the frame stays)."),
            method("resumeVideo", vec![("name", T::String)], T::Void, "Resume a paused video."),
            method("seekVideo", vec![("name", T::String), ("ms", T::Number)], T::Void, "Set the playback position in milliseconds."),
            method("stopVideo", vec![("name", T::String)], T::Void, "Remove a video and its decoder."),
            method("video", vec![("name", T::String)], T::Optional(Box::new(T::Map)),
                "{asset, time, playing, loop, surface, width?, height?, ended?} or null."),
            method("findPath", vec![("from", T::Vec3), ("to", T::Vec3)], T::Optional(Box::new(T::List(Box::new(T::Vec3)))), "Smoothed navigation path or null."),
            method("defineAnimation", vec![("defs", T::Map)], T::Void,
                "Merge animation definitions {machines: {Name: {entry, states, transitions, parameters}}, montages: {Name: {clip, length, sections, notifies}}}."),
            method("defineGameplay", vec![("defs", T::Map)], T::Void,
                "Merge gameplay definitions {effects: {Name: EffectDef}, abilities: {Name: AbilityDef}}."),
            method("modelInfo", vec![("path", T::String)], T::Map, "Model summary {clips:[{name,duration}], nodes, skins, meshes, morphs:[target names], bounds:{min,max} (rest pose, model units) or null} without spawning."),
        ],
    })
}
fn parse_spawn(cx: &mut EngineContext, spec: &Value, id: u32) -> Result<EntitySpec> {
    let Value::Map(m) = spec else {
        bail!("spawn expects a spec object")
    };
    for k in m.keys() {
        ensure!(
            matches!(
                k.as_str(),
                "name"
                    | "parent"
                    | "position"
                    | "rotation"
                    | "scale"
                    | "geometry"
                    | "color"
                    | "visible"
                    | "velocity"
                    | "angularVelocity"
                    | "collider"
                    | "particles"
                    | "vfx"
                    | "light"
                    | "reflectionProbe"
                    | "irradianceVolume"
                    | "material"
            ),
            "unknown spawn field '{k}'"
        );
    }
    let mut transform = Transform::identity();
    if let Some(v) = spec.field("position") {
        transform.translation = v.as_vec3()?;
    }
    if let Some(v) = spec.field("rotation") {
        transform.rotation_degrees = v.as_vec3()?;
    }
    if let Some(v) = spec.field("scale") {
        transform.scale = v.as_vec3()?;
    }
    let geometry = match spec.field("geometry") {
        None => None,
        Some(Value::Str(s)) if s == "cube" => Some(Geometry::Cube),
        // Typed geometry ({kind: terrain|model|scene|cube|triangles, ...}); a map
        // without `kind` is raw triangles {vertices, indices}.
        Some(v @ Value::Map(_)) => Some(serde_json::from_value::<Geometry>({
            let mut j = v.to_json();
            if j.get("kind").is_none() {
                j["kind"] = "triangles".into();
            }
            j
        })?),
        Some(v) => bail!("unsupported geometry {v:?}"),
    };
    let color = match spec.field("color") {
        None => [255; 4],
        Some(Value::List(l)) if l.len() == 4 => {
            let mut c = [0u8; 4];
            for (o, v) in c.iter_mut().zip(l) {
                *o = u8::try_from(v.as_i64()?).context("color channel 0..255")?;
            }
            c
        }
        Some(v) => bail!("color must be [r,g,b,a], got {v:?}"),
    };
    let parent = match spec.field("parent") {
        Some(p) => Some(entity_id_of(cx, p)?),
        None => None,
    };
    Ok(EntitySpec {
        id,
        name: spec
            .field("name")
            .map(|v| v.as_str().map(str::to_owned))
            .transpose()?
            .unwrap_or_else(|| format!("entity{id}")),
        parent,
        transform,
        geometry,
        color,
        visible: spec
            .field("visible")
            .map(Value::as_bool)
            .transpose()?
            .unwrap_or(true),
        velocity: spec
            .field("velocity")
            .map(Value::as_vec3)
            .transpose()?
            .unwrap_or_default(),
        angular_velocity: spec
            .field("angularVelocity")
            .map(Value::as_vec3)
            .transpose()?
            .unwrap_or_default(),
        tweens: vec![],
        material: spec
            .field("material")
            .filter(|v| !matches!(v, Value::Null))
            .map(|v| serde_json::from_value(v.to_json()).context("material"))
            .transpose()?,
        animation: None,
        collider: match spec.field("collider") {
            Some(c) => Some(serde_json::from_value(c.to_json()).context("collider")?),
            None => None,
        },
        gameplay: None,
        ai: None,
        particles: match spec.field("particles") {
            Some(p) => Some(serde_json::from_value(p.to_json()).context("particles")?),
            None => None,
        },
        vfx: spec
            .field("vfx")
            .filter(|v| !matches!(v, Value::Null))
            .map(|v| serde_json::from_value(v.to_json()).context("VFX system"))
            .transpose()?,
        light: match spec.field("light") {
            Some(l) => Some(parse_light(l)?),
            None => None,
        },
        reflection_probe: spec.field("reflectionProbe").map(parse_probe).transpose()?,
        irradiance_volume: spec
            .field("irradianceVolume")
            .map(parse_volume)
            .transpose()?,
    })
}
fn parse_volume(v: &Value) -> Result<crysirror_scene::components::IrradianceVolumeSpec> {
    let s: crysirror_scene::components::IrradianceVolumeSpec =
        serde_json::from_value(v.to_json()).context("irradiance volume")?;
    s.validate()?;
    Ok(s)
}
fn parse_probe(v: &Value) -> Result<crysirror_scene::components::ReflectionProbeSpec> {
    let p: crysirror_scene::components::ReflectionProbeSpec =
        serde_json::from_value(v.to_json()).context("reflection probe")?;
    p.validate()?;
    Ok(p)
}
fn parse_light(v: &Value) -> Result<crysirror_scene::components::LightSpec> {
    let l: crysirror_scene::components::LightSpec =
        serde_json::from_value(v.to_json()).context("light")?;
    l.validate()?;
    Ok(l)
}
pub(crate) fn import_model(
    cx: &mut EngineContext,
    path: &str,
    options: Option<&Value>,
) -> Result<Value> {
    let (client, bytes) = {
        let s = cx.state()?;
        let p = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
        (
            s.client,
            s.runtime
                .host
                .fs
                .read(s.client, &p)
                .map_err(|e| anyhow::anyhow!("{path}: {e}"))?,
        )
    };
    let parent = match options.and_then(|o| o.field("parent")) {
        Some(p) => Some(entity_id_of(cx, p)?),
        None => None,
    };
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or("model");
    let prefix = match options.and_then(|o| o.field("prefix")) {
        Some(v) => v.as_str()?.to_owned(),
        None => format!("{stem}:"),
    };
    let pbr = match options.and_then(|o| o.field("pbr")) {
        Some(v) => v.as_bool()?,
        None => false,
    };
    // One entity for the whole glTF scene (skins + animations), PBR materials.
    if options
        .and_then(|o| o.field("asScene"))
        .map(Value::as_bool)
        .transpose()?
        .unwrap_or(false)
    {
        let id = next_entity_id(cx)?;
        let mut spec_v = BTreeMap::from([(
            "name".to_owned(),
            Value::Str(match options.and_then(|o| o.field("prefix")) {
                Some(v) => v.as_str()?.to_owned(),
                None => path.rsplit('/').next().unwrap_or(path).to_owned(),
            }),
        )]);
        if let Some(p) = options.and_then(|o| o.field("parent")) {
            spec_v.insert("parent".into(), p.clone());
        }
        let mut entity = parse_spawn(cx, &Value::Map(spec_v), id)?;
        entity.geometry = Some(Geometry::Scene {
            asset: path.to_owned(),
        });
        submit(cx, vec![WorldCommand::Spawn { entity }])?;
        let s = cx.state()?;
        let h = s.runtime.host.engine.world.handle(s.client, id)?;
        let root = entity_object(cx, id, h.generation)?;
        return Ok(Value::Map(BTreeMap::from([
            ("roots".into(), Value::List(vec![root])),
            (
                "report".into(),
                Value::from_json(
                    serde_json::json!({"nodes":1,"primitives":0,"triangles":0,"dropped":[]}),
                ),
            ),
        ])));
    }
    let first_id = next_entity_id(cx)?;
    let dir = path
        .rsplit_once('/')
        .map(|(d, _)| format!("{d}/"))
        .unwrap_or_default();
    let model = {
        let s = cx.state()?;
        let fs = &mut s.runtime.host.fs;
        crysirror_asset::import_gltf(
            &bytes,
            &crysirror_asset::ImportOptions {
                first_id,
                parent,
                name_prefix: prefix,
                reference_asset: pbr.then(|| path.to_owned()),
            },
            |uri| {
                let p = VfsPath::new(Mount::Asset, &format!("{dir}{uri}"))
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                fs.read(client, &p).map_err(|e| anyhow::anyhow!("{e}"))
            },
        )?
    };
    let roots: Vec<(u32, u32)> = model
        .entities
        .iter()
        .filter(|e| e.parent == parent)
        .map(|e| (e.id, 0))
        .collect();
    submit(
        cx,
        model
            .entities
            .into_iter()
            .map(|entity| WorldCommand::Spawn { entity })
            .collect(),
    )?;
    let mut root_values = Vec::new();
    for (id, _) in roots {
        let h = cx
            .state()?
            .runtime
            .host
            .engine
            .world
            .handle(ClientId(client.0), id)?;
        root_values.push(entity_object(cx, id, h.generation)?);
    }
    let report = Value::from_json(serde_json::to_value(&model.report)?);
    Ok(Value::Map(BTreeMap::from([
        ("roots".into(), Value::List(root_values)),
        ("report".into(), report),
    ])))
}
impl Class for WorldClass {
    fn info(&self) -> &ClassInfo {
        world_info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        Ok(match p {
            "count" => (s.runtime.host.engine.world.entities(s.client).len() as u64).into(),
            "revision" => s.runtime.host.engine.world.revision.into(),
            "camera" => Value::from_json(serde_json::to_value(
                &s.runtime.host.engine.world.view.camera,
            )?),
            "environment" => Value::from_json(serde_json::to_value(
                s.runtime
                    .host
                    .engine
                    .world
                    .view
                    .environment
                    .as_ref()
                    .map(|(_, e)| e),
            )?),
            _ => unreachable!(),
        })
    }
    fn call(&self, cx: &mut EngineContext, _: ObjectId, m: &str, args: &[Value]) -> Result<Value> {
        match m {
            "spawn" => {
                let id = next_entity_id(cx)?;
                let entity = parse_spawn(cx, arg(args, 0, "spec")?, id)?;
                submit(cx, vec![WorldCommand::Spawn { entity }])?;
                let s = cx.state()?;
                let h = s.runtime.host.engine.world.handle(s.client, id)?;
                entity_object(cx, id, h.generation)
            }
            "find" | "entities" => {
                let name = if m == "find" {
                    Some(arg(args, 0, "name")?.as_str()?.to_owned())
                } else {
                    None
                };
                let s = cx.state()?;
                let found: Vec<(u32, u32)> = s
                    .runtime
                    .host
                    .engine
                    .world
                    .entities(s.client)
                    .into_iter()
                    .filter(|(_, _, e)| name.as_deref().is_none_or(|n| e.name == n))
                    .map(|(id, g, _)| (id, g))
                    .collect();
                if m == "find" {
                    return match found.first() {
                        Some((id, g)) => entity_object(cx, *id, *g),
                        None => Ok(Value::Null),
                    };
                }
                let mut out = Vec::new();
                for (id, g) in found {
                    out.push(entity_object(cx, id, g)?);
                }
                Ok(Value::List(out))
            }
            "importModel" => import_model(cx, arg(args, 0, "path")?.as_str()?, opt(args, 1)),
            "groundPose" => {
                let p = glam::Vec3::from(arg(args, 0, "position")?.as_vec3()?);
                let yaw = arg(args, 1, "yawDegrees")?.as_f32()?.to_radians();
                let half = arg(args, 2, "halfLength")?.as_f32()?;
                let drop = arg(args, 3, "maxDrop")?.as_f32()?;
                ensure!(
                    half.is_finite()
                        && half >= 0.
                        && drop.is_finite()
                        && drop > 0.
                        && yaw.is_finite(),
                    "groundPose arguments"
                );
                let ignore = match opt(args, 4) {
                    Some(e) => Some(handle(cx, e.as_object()?)?).map(|h| (h.client, h.id)),
                    None => None,
                };
                let s = cx.state()?;
                let physics = s.physics()?;
                let fwd = glam::Vec3::new(yaw.sin(), 0., yaw.cos());
                // Probe from slightly above the body so steps up are found too.
                let lift = drop * 0.5;
                let probe = |at: glam::Vec3| {
                    physics.raycast(
                        at + glam::Vec3::Y * lift,
                        glam::Vec3::NEG_Y,
                        drop + lift,
                        ignore,
                    )
                };
                let (front, back) = (probe(p + fwd * half), probe(p - fwd * half));
                let (f, b) = match (front, back) {
                    (None, None) => return Ok(Value::Null),
                    (Some(f), None) => (f, f),
                    (None, Some(b)) => (b, b),
                    (Some(f), Some(b)) => (f, b),
                };
                let height = (f.point.y + b.point.y) * 0.5;
                let pitch = if half > 0. {
                    ((f.point.y - b.point.y) / (2. * half)).atan().to_degrees()
                } else {
                    0.
                };
                let normal = (f.normal + b.normal).normalize_or_zero();
                return Ok(Value::from_json(serde_json::json!({
                    "height": height, "pitch": pitch, "normal": normal.to_array(),
                    "front": f.point.to_array(), "back": b.point.to_array(),
                })));
            }
            "raycast" | "sphereCast" => {
                let origin = arg(args, 0, "origin")?.as_vec3()?;
                let dir = arg(args, 1, "direction")?.as_vec3()?;
                let sphere = m == "sphereCast";
                let radius = if sphere {
                    arg(args, 2, "radius")?.as_f32()?
                } else {
                    0.
                };
                let at = usize::from(sphere);
                let max = arg(args, 2 + at, "maxDistance")?.as_f32()?;
                let ignore = match opt(args, 3 + at) {
                    Some(e) => Some(handle(cx, e.as_object()?)?),
                    None => None,
                };
                ensure!(
                    !sphere || (radius.is_finite() && radius > 0. && radius <= 1000.),
                    "sphereCast radius"
                );
                let s = cx.state()?;
                let ignore = ignore.map(|h| (h.client, h.id));
                let hit = if sphere {
                    s.physics()?
                        .sphere_cast(origin.into(), dir.into(), radius, max, ignore)
                } else {
                    s.physics()?.raycast(origin.into(), dir.into(), max, ignore)
                };
                let Some(hit) = hit else {
                    return Ok(Value::Null);
                };
                let (client, id) = hit.entity;
                let s = cx.state()?;
                let entity = if client == s.client.0 {
                    let h = s.runtime.host.engine.world.handle(s.client, id)?;
                    entity_object(cx, id, h.generation)?
                } else {
                    Value::Null
                };
                Ok(Value::Map(BTreeMap::from([
                    ("entity".into(), entity),
                    ("point".into(), Value::vec3(hit.point.to_array())),
                    ("normal".into(), Value::vec3(hit.normal.to_array())),
                    ("distance".into(), Value::Float(hit.distance as f64)),
                ])))
            }
            "defineBehaviorTree" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                let root: crysirror_scene::ai::BtNode =
                    serde_json::from_value(arg(args, 1, "root")?.to_json())
                        .context("behaviour tree")?;
                let defs = crysirror_scene::ai::AiDefs {
                    trees: BTreeMap::from([(name, root)]),
                };
                defs.validate()?;
                submit(cx, vec![WorldCommand::DefineAi { defs }])?;
                Ok(Value::Null)
            }
            "buildNavigation" => {
                let settings: crate::ai::NavSettings =
                    serde_json::from_value(arg(args, 0, "settings")?.to_json())
                        .context("navigation settings")?;
                let s = cx.state()?;
                let grid = crate::ai::NavGrid::bake(s.physics()?, settings)?;
                let out = serde_json::json!({"width": grid.w, "height": grid.h, "walkable": grid.walkable()});
                s.ai.nav = Some(grid);
                Ok(Value::from_json(out))
            }
            "playVideo" | "pauseVideo" | "resumeVideo" | "seekVideo" | "stopVideo" | "video" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                let (client, current) = {
                    let s = cx.state()?;
                    (
                        s.client,
                        s.runtime.host.engine.world.video(s.client, &name).cloned(),
                    )
                };
                let need = |v: Option<crysirror_scene::world::VideoSpec>| {
                    v.with_context(|| format!("no video named {name}"))
                };
                let spec = match m {
                    "video" => {
                        let Some(v) = current else {
                            return Ok(Value::Null);
                        };
                        let status = cx.state()?.runtime.video_status(client, &name);
                        let mut m = BTreeMap::from([
                            ("asset".into(), Value::Str(v.asset.clone())),
                            ("time".into(), Value::Int(v.time_ms as i64)),
                            ("playing".into(), Value::Bool(v.playing)),
                            ("loop".into(), Value::Bool(v.looping)),
                            (
                                "surface".into(),
                                Value::Str(format!("__surface/{}", v.slot)),
                            ),
                        ]);
                        if let Some(st) = status {
                            m.insert("width".into(), Value::Int(st.width.into()));
                            m.insert("height".into(), Value::Int(st.height.into()));
                            m.insert("ended".into(), Value::Bool(st.ended));
                        }
                        return Ok(Value::Map(m));
                    }
                    "playVideo" => {
                        let asset = arg(args, 1, "asset")?.as_str()?.to_owned();
                        let o = opt(args, 2)
                            .map(|v| v.to_json())
                            .unwrap_or(serde_json::json!({}));
                        ensure!(
                            o.as_object()
                                .is_some_and(|m| m.keys().all(|k| k == "loop" || k == "paused")),
                            "playVideo options: loop, paused"
                        );
                        Some(crysirror_scene::world::VideoSpec {
                            asset,
                            looping: o["loop"].as_bool().unwrap_or(false),
                            playing: !o["paused"].as_bool().unwrap_or(false),
                            time_ms: 0,
                            slot: 0,
                        })
                    }
                    "pauseVideo" => Some(crysirror_scene::world::VideoSpec {
                        playing: false,
                        ..need(current)?
                    }),
                    "resumeVideo" => Some(crysirror_scene::world::VideoSpec {
                        playing: true,
                        ..need(current)?
                    }),
                    "seekVideo" => {
                        let ms = arg(args, 1, "ms")?.as_f64()?;
                        ensure!(ms.is_finite() && ms >= 0. && ms <= 1e12, "video position");
                        Some(crysirror_scene::world::VideoSpec {
                            time_ms: ms as u64,
                            ..need(current)?
                        })
                    }
                    _ => {
                        need(current)?;
                        None
                    }
                };
                submit(
                    cx,
                    vec![WorldCommand::SetVideo {
                        name: name.clone(),
                        video: spec,
                    }],
                )?;
                if m == "playVideo" {
                    let slot = cx
                        .state()?
                        .runtime
                        .host
                        .engine
                        .world
                        .video(client, &name)
                        .map(|v| v.slot)
                        .context("video")?;
                    return Ok(Value::Str(format!("__surface/{slot}")));
                }
                Ok(Value::Null)
            }
            "playSequence" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                let sequence: crysirror_scene::sequence::SequenceSpec =
                    serde_json::from_value(arg(args, 1, "sequence")?.to_json())
                        .context("sequence spec")?;
                let start_ms = opt(args, 2).map(Value::as_f64).transpose()?.unwrap_or(0.) as f32;
                submit(
                    cx,
                    vec![WorldCommand::PlaySequence {
                        name,
                        sequence,
                        start_ms,
                    }],
                )?;
                Ok(Value::Null)
            }
            "stopSequence" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                submit(cx, vec![WorldCommand::StopSequence { name }])?;
                Ok(Value::Null)
            }
            "sequenceStatus" => {
                let name = arg(args, 0, "name")?.as_str()?;
                let s = cx.state()?;
                Ok(
                    match s.runtime.host.engine.world.sequence(s.client.0, name) {
                        None => Value::Null,
                        Some(p) => Value::from_json(serde_json::json!({
                            "time_ms": p.time_ms, "playing": p.playing, "loops": p.loops, "duration_ms": p.spec.duration_ms,
                        })),
                    },
                )
            }
            "sequenceEvents" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                let s = cx.state()?;
                let client = s.client.0;
                let events = s
                    .runtime
                    .host
                    .engine
                    .world
                    .take_sequence_events(client, &name)?;
                Ok(Value::List(events.into_iter().map(Value::Str).collect()))
            }
            "streamLevel" => {
                let name = arg(args, 0, "name")?.as_str()?.to_owned();
                let level = match opt(args, 1) {
                    Some(v) => {
                        let mut l: crysirror_scene::world::StreamingLevel =
                            serde_json::from_value(v.to_json()).context("streaming level")?;
                        l.loaded = None;
                        Some(l)
                    }
                    None => None,
                };
                let old = cx
                    .state()?
                    .runtime
                    .host
                    .engine
                    .world
                    .streaming
                    .get(&name)
                    .and_then(|l| l.loaded);
                let mut cmds = vec![WorldCommand::SetStreaming { name, level }];
                if let Some(root) = old {
                    cmds.insert(0, WorldCommand::Despawn { id: root });
                }
                submit(cx, cmds)?;
                Ok(Value::Null)
            }
            "findPath" => {
                let from = arg(args, 0, "from")?.as_vec3()?;
                let to = arg(args, 1, "to")?.as_vec3()?;
                let s = cx.state()?;
                let nav =
                    s.ai.nav
                        .as_ref()
                        .context("call world.buildNavigation first")?;
                Ok(match nav.find_path(from.into(), to.into()) {
                    Some(p) => {
                        Value::List(p.into_iter().map(|v| Value::vec3(v.to_array())).collect())
                    }
                    None => Value::Null,
                })
            }
            "defineAnimation" => {
                let defs: crysirror_scene::animgraph::AnimDefs =
                    serde_json::from_value(arg(args, 0, "defs")?.to_json())
                        .context("animation definitions")?;
                defs.validate()?;
                submit(cx, vec![WorldCommand::DefineAnimation { defs }])?;
                Ok(Value::Null)
            }
            "defineGameplay" => {
                let defs: crysirror_scene::gameplay::GameplayDefs =
                    serde_json::from_value(arg(args, 0, "defs")?.to_json())
                        .context("gameplay definitions")?;
                defs.validate()?;
                submit(cx, vec![WorldCommand::DefineGameplay { defs }])?;
                Ok(Value::Null)
            }
            "modelInfo" => {
                let path = arg(args, 0, "path")?.as_str()?.to_owned();
                let s = cx.state()?;
                let client = s.client;
                let read = |fs: &mut crysirror_runtime::vfs::DiskVfs, p: &str| -> Result<Vec<u8>> {
                    let vp =
                        VfsPath::new(Mount::Asset, p).map_err(|e| anyhow::anyhow!("{p}: {e}"))?;
                    fs.read(client, &vp)
                        .map_err(|e| anyhow::anyhow!("{p}: {e}"))
                };
                let fs = &mut s.runtime.host.fs;
                let bytes = read(fs, &path)?;
                let dir = path
                    .rsplit_once('/')
                    .map(|(d, _)| format!("{d}/"))
                    .unwrap_or_default();
                let m = crysirror_asset::pbr::load_geometry(&bytes, |u| {
                    read(fs, &format!("{dir}{u}"))
                })?;
                let bounds = m
                    .rest_bounds()
                    .map(|(lo, hi)| serde_json::json!({"min": lo, "max": hi}));
                Ok(Value::from_json(serde_json::json!({
                    "clips": m.animations.iter().map(|c| serde_json::json!({"name": c.name, "duration": c.duration})).collect::<Vec<_>>(),
                    "nodes": m.nodes.len(), "skins": m.skins.len(), "meshes": m.meshes.len(),
                    "morphs": m.meshes.iter().flatten().flat_map(|p| p.morphs.iter().map(|t| t.name.clone())).collect::<std::collections::BTreeSet<_>>(),
                    "bounds": bounds,
                })))
            }
            "setCamera" => {
                let c = arg(args, 0, "camera")?;
                let camera = crysirror_scene::model::Camera {
                    eye: c.field("eye").context("camera.eye")?.as_vec3()?,
                    target: c.field("target").context("camera.target")?.as_vec3()?,
                    up: c
                        .field("up")
                        .map(Value::as_vec3)
                        .transpose()?
                        .unwrap_or([0., 1., 0.]),
                    fov_y_degrees: c
                        .field("fov")
                        .map(Value::as_f32)
                        .transpose()?
                        .unwrap_or(45.),
                    near: c
                        .field("near")
                        .map(Value::as_f32)
                        .transpose()?
                        .unwrap_or(0.05),
                    far: c
                        .field("far")
                        .map(Value::as_f32)
                        .transpose()?
                        .unwrap_or(500.),
                };
                submit(cx, vec![WorldCommand::SetCamera { camera }])?;
                Ok(Value::Null)
            }
            "setEnvironment" => {
                let environment: crysirror_scene::view::Environment =
                    serde_json::from_value(arg(args, 0, "environment")?.to_json())
                        .context("environment")?;
                submit(cx, vec![WorldCommand::SetEnvironment { environment }])?;
                Ok(Value::Null)
            }
            "pause" => {
                let paused = arg(args, 0, "paused")?.as_bool()?;
                submit(cx, vec![WorldCommand::Pause { paused }])?;
                Ok(Value::Null)
            }
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------- Entity
pub struct EntityClass;
fn entity_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Entity",
        doc: "A generational handle to a world entity. Using it after destroy() fails.",
        properties: vec![
            prop("id", T::Int, false, "Client-local entity id."),
            prop("name", T::String, false, "Entity name."),
            prop("alive", T::Bool, false, "False after destroy() or cascading parent removal."),
            prop("position", T::Vec3, true, "Local translation."),
            prop("rotation", T::Vec3, true, "Local XYZ Euler rotation in degrees."),
            prop("scale", T::Vec3, true, "Local non-zero scale."),
            prop("visible", T::Bool, true, "Local visibility (inherited by children)."),
            prop("velocity", T::Vec3, true, "Units per second, integrated at the world fixed step."),
            prop("angularVelocity", T::Vec3, true, "World angular velocity in degrees per second. Rigid bodies are integrated once by physics."),
            prop("physicsState",T::Any,false,"World position, rotation quaternion [x,y,z,w], velocity, angularVelocity in degrees/s and sleeping state, or null."),
            prop("color", T::List(Box::new(T::Int)), false, "Base color [r,g,b,a] of untextured geometry."),
            prop("geometry", T::Any, false, "Geometry: cube, triangles or {kind:'model', asset, mesh}."),
            prop("parent", T::Optional(Box::new(T::Object("Entity"))), false, "Parent entity or null."),
            prop("material", T::Any, false, "Engine material override or null; paths stay client-scoped."),
            prop("animation", T::Any, false, "Animation state {layers:[{clip,time,speed,weight,...}]} or null."),
            prop("collider", T::Any, false, "Collider spec or null."),
            prop("gameplay", T::Any, false, "Gameplay component (attributes, effects, tags, abilities) or null."),
            prop("tags", T::List(Box::new(T::String)), false, "All owned gameplay tags."),
            prop("particles", T::Any, false, "Particle emitter spec or null."),
            prop("vfx", T::Any, false, "Modular VFX system specification or null; simulation belongs to the engine runtime."),
            prop("reflectionProbe", T::Any, false, "Box reflection capture or null."),
            prop("irradianceVolume", T::Any, false, "Dynamic diffuse GI volume or null."),
            prop("light", T::Any, false, "Local light {shape:{kind:'point'|'spot'|'rect',...}, color, intensity, units:'candela'|'lumens', range, shadows, enabled} or null."),
            prop("ai", T::Any, false, "Behaviour tree state {tree, blackboard, status, active_leaf} or null."),
        ],
        methods: vec![
            method("destroy", vec![], T::Void, "Despawn this entity and its descendants."),
            method("setParent", vec![("parent", T::Optional(Box::new(T::Object("Entity"))))], T::Void, "Re-parent; cycles are rejected."),
            method("children", vec![], T::List(Box::new(T::Object("Entity"))), "Direct children."),
            method("play", vec![("clip", T::String), ("options", T::Optional(Box::new(T::Map)))], T::Void,
                "Cross-fade skeletal animation: options {speed?: 1, fade?: seconds (0.2), loop?: true}."),
            method("stopAnimation", vec![], T::Void, "Clear animation (rest pose)."),
            method("setMorph", vec![("name", T::String), ("weight", T::Number)], T::Void,
                "Set a morph target (blend shape) weight by target name on a whole-scene model; 0 removes it. Names: World.modelInfo(path).morphs."),
            method("setMorphs", vec![("weights", T::Optional(Box::new(T::Map)))], T::Void,
                "Replace all morph weights with {name: weight}, or clear them with null."),
            method("setCollider", vec![("collider", T::Optional(Box::new(T::Map)))], T::Void,
                "Collider {shape:{kind:'box'|'sphere'|'capsule'|'mesh'|'convex_hull',...}, body?:'static'|'dynamic'|'kinematic'|'kinematic_position'|'character',mass?,density?,friction?,restitution?,ccd?,sensor?,groups?,gravity_scale?,linear_damping?,angular_damping?,locked_translation?,locked_rotation?,offset?} or null."),
            method("applyImpulse",vec![("impulse",T::Vec3),("point",T::Optional(Box::new(T::Vec3)))],T::Void,"Apply a world-space impulse (N·s), optionally at a world-space point; saves the resulting velocity immediately."),
            method("applyTorqueImpulse",vec![("impulse",T::Vec3)],T::Void,"Apply a world-space angular impulse to a dynamic body."),
            method("addForce",vec![("force",T::Vec3)],T::Void,"Accumulate a persistent world-space force (N); saved until clearForces()."),
            method("addTorque",vec![("torque",T::Vec3)],T::Void,"Accumulate persistent world-space torque (N·m)."),
            method("clearForces",vec![],T::Void,"Clear persistent force and torque."),
            method("wake",vec![],T::Void,"Wake a physics body."),
            method("sleep",vec![],T::Void,"Sleep a physics body when can_sleep is enabled."),
            method("setKinematicTarget",vec![("position",T::Vec3),("rotation",T::Optional(Box::new(T::Vec3)))],T::Void,"World-space target for a kinematic_position body at the next fixed step; rotation is XYZ degrees."),
            method("setVehicle", vec![("spec", T::Optional(Box::new(T::Map)))], T::Void,
                "Make this character-collider entity an arcade vehicle (ADR 0043): {wheelbase, track, ride_height, suspension, max_speed, acceleration, braking, drag, steer_degrees, steer_falloff, grip, handbrake_grip, gravity, step, max_slope} (all optional); null removes it. Vehicle state is not saved yet."),
            method("drive", vec![("input", T::Map), ("dt", T::Number)], T::Map,
                "Advance the vehicle by dt seconds with {throttle -1..1, steer -1..1 (positive toward +X of the heading), brake 0..1, handbrake}; moves and orients the entity. Returns {speed, lateral, grounded, wheels, blocked, yaw}."),
            method("moveCharacter", vec![("desired", T::Vec3), ("dt", T::Number), ("options", T::Optional(Box::new(T::Map)))], T::Map,
                "Move a character capsule with sliding, autostep and slope limits; returns {translation, grounded}. options {step?: 0.3, maxSlope?: 45}."),
            method("setMaterial", vec![("material",T::Optional(Box::new(T::Map)))],T::Void,"Replace the engine material; null restores imported materials. Alpha modes: opaque, mask, blend, additive, premultiplied."),
            method("setIrradianceVolume", vec![("volume", T::Optional(Box::new(T::Map)))], T::Void,
                "Dynamic diffuse GI volume {extent:[hx,hy,hz], counts?:[nx,ny,nz], intensity?, probes_per_frame?, revision?} centred on this entity, or null."),
            method("setReflectionProbe", vec![("probe", T::Optional(Box::new(T::Map)))], T::Void,
                "Box reflection capture {extent:[hx,hy,hz], intensity?, blend_distance?, revision?} at this entity, or null. Bump revision to recapture."),
            method("setLight", vec![("light", T::Optional(Box::new(T::Map)))], T::Void,
                "Attach/replace a local light (spot and rect emit along local -Z) or null to remove. Intensity is candela unless units:'lumens'."),
            method("setParticles", vec![("emitter", T::Optional(Box::new(T::Map)))], T::Void,
                "Particle emitter {rate, burst, lifetime, speed, direction, spread_degrees, gravity, drag, size, color_start, color_end, max, additive, enabled, seed} or null."),
            method("setVfx", vec![("system", T::Optional(Box::new(T::Map)))], T::Void,
                "Attach or replace a modular VFX system; null removes it. CPU/GPU targets, emitter modules, parameters and renderers are explicitly validated."),
            method("setVfxParameter", vec![("name",T::String),("value",T::Any)],T::Void,
                "Set an existing typed VFX parameter. Accepts a number, bool, vector3 or color4 matching its declared type; does not restart the effect."),
            method("pauseVfx",vec![("paused",T::Bool)],T::Void,"Freeze or resume this VFX system clock and all live particles."),
            method("stopVfx",vec![],T::Void,"Stop new VFX births; existing particles continue to the end of their lifetimes."),
            method("restartVfx",vec![],T::Void,"Increment the VFX revision and restart its simulation from its configured seed."),
            method("runBehaviorTree", vec![("tree", T::Optional(Box::new(T::String)))], T::Void, "Run a defined behaviour tree every fixed step (null stops)."),
            method("setBlackboard", vec![("key", T::String), ("value", T::Any)], T::Void, "Write a blackboard value (bool, number, [x,y,z], string, Entity, or null to clear)."),
            method("setStateMachine", vec![("machine", T::Optional(Box::new(T::String)))], T::Void, "Drive animation from a defined state machine (null clears)."),
            method("setAnimParam", vec![("name", T::String), ("value", T::Any)], T::Void, "Set a state machine parameter (number or bool)."),
            method("playMontage", vec![("montage", T::String), ("section", T::Optional(Box::new(T::String)))], T::Void, "Play a montage over the state machine, optionally from a section."),
            method("stopMontage", vec![], T::Void, "Blend the current montage out."),
            method("drainAnimEvents", vec![], T::List(Box::new(T::Map)), "Take notifies/state changes {kind, name}."),
            method("initAttributes", vec![("attributes", T::Map)], T::Void, "Set attributes {Name: {base, min?, max?, max_attribute?}} (keeps effects/tags)."),
            method("attribute", vec![("name", T::String)], T::Optional(Box::new(T::Number)), "Current value after active effects, or null."),
            method("setAttribute", vec![("name", T::String), ("base", T::Number)], T::Void, "Set an attribute's base value."),
            method("applyEffect", vec![("effect", T::String), ("source", T::Optional(Box::new(T::Object("Entity"))))], T::Bool, "Apply a gameplay effect; false if blocked by tags."),
            method("removeEffect", vec![("effect", T::String)], T::Bool, "Remove an active effect."),
            method("hasTag", vec![("tag", T::String)], T::Bool, "Owns the tag or a descendant (loose, effect or ability tags)."),
            method("addTag", vec![("tag", T::String)], T::Void, "Add a loose tag (reference counted)."),
            method("removeTag", vec![("tag", T::String)], T::Void, "Remove one loose tag reference."),
            method("grantAbility", vec![("ability", T::String)], T::Void, "Grant an ability."),
            method("activate", vec![("ability", T::String)], T::Map, "Try to activate: {ok, reason}."),
            method("endAbility", vec![("ability", T::String)], T::Bool, "End an active ability."),
            method("drainEvents", vec![], T::List(Box::new(T::Map)), "Take queued gameplay events {kind, name, value, source}."),
            method("tween", vec![("spec", T::Map)], T::Void,
                "Integer tween {property:'x'|'y'|'z'|'rotation_x'|..., from, to, duration_ms, delay_ms?, mode?, easing?, real_time?}."),
        ],
    })
}
impl Class for EntityClass {
    fn info(&self) -> &ClassInfo {
        entity_info()
    }
    fn get(&self, cx: &mut EngineContext, this: ObjectId, p: &str) -> Result<Value> {
        if p == "alive" {
            return Ok(handle(cx, this).is_ok().into());
        }
        if p == "physicsState" {
            let h = handle(cx, this)?;
            return crate::physics_api::entity_state(cx, h);
        }
        let e = spec(cx, this)?;
        Ok(match p {
            "id" => Value::Int(e.id.into()),
            "name" => e.name.into(),
            "position" => Value::vec3(e.transform.translation),
            "rotation" => Value::vec3(e.transform.rotation_degrees),
            "scale" => Value::vec3(e.transform.scale),
            "visible" => e.visible.into(),
            "velocity" => Value::vec3(e.velocity),
            "angularVelocity" => Value::vec3(e.angular_velocity),
            "color" => Value::List(e.color.iter().map(|c| Value::Int((*c).into())).collect()),
            "geometry" => Value::from_json(serde_json::to_value(&e.geometry)?),
            "material" => Value::from_json(serde_json::to_value(&e.material)?),
            "animation" => Value::from_json(serde_json::to_value(&e.animation)?),
            "collider" => Value::from_json(serde_json::to_value(&e.collider)?),
            "gameplay" => Value::from_json(serde_json::to_value(&e.gameplay)?),
            "ai" => Value::from_json(serde_json::to_value(&e.ai)?),
            "particles" => Value::from_json(serde_json::to_value(&e.particles)?),
            "vfx" => Value::from_json(serde_json::to_value(&e.vfx)?),
            "light" => Value::from_json(serde_json::to_value(&e.light)?),
            "reflectionProbe" => Value::from_json(serde_json::to_value(&e.reflection_probe)?),
            "irradianceVolume" => Value::from_json(serde_json::to_value(&e.irradiance_volume)?),
            "tags" => {
                let defs = cx.state()?.runtime.host.engine.world.gameplay.clone();
                Value::List(
                    e.gameplay
                        .map(|g| g.tags(&defs))
                        .unwrap_or_default()
                        .into_iter()
                        .map(Value::Str)
                        .collect(),
                )
            }
            "parent" => match e.parent {
                None => Value::Null,
                Some(pid) => {
                    let s = cx.state()?;
                    let h = s.runtime.host.engine.world.handle(s.client, pid)?;
                    entity_object(cx, pid, h.generation)?
                }
            },
            _ => unreachable!(),
        })
    }
    fn set(&self, cx: &mut EngineContext, this: ObjectId, p: &str, v: Value) -> Result<()> {
        let e = spec(cx, this)?;
        let id = e.id;
        let mut t = e.transform.clone();
        let cmd = match p {
            "position" => {
                t.translation = v.as_vec3()?;
                WorldCommand::SetTransform { id, transform: t }
            }
            "rotation" => {
                t.rotation_degrees = v.as_vec3()?;
                WorldCommand::SetTransform { id, transform: t }
            }
            "scale" => {
                t.scale = v.as_vec3()?;
                WorldCommand::SetTransform { id, transform: t }
            }
            "visible" => WorldCommand::SetVisible {
                id,
                visible: v.as_bool()?,
            },
            "velocity" => WorldCommand::SetVelocity {
                id,
                velocity: v.as_vec3()?,
            },
            "angularVelocity" => WorldCommand::SetAngularVelocity {
                id,
                velocity: v.as_vec3()?,
            },
            _ => unreachable!(),
        };
        submit(cx, vec![cmd])
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        this: ObjectId,
        m: &str,
        args: &[Value],
    ) -> Result<Value> {
        if crate::physics_api::is_entity_method(m) {
            return crate::physics_api::entity_call(cx, this, m, args);
        }
        let id = handle(cx, this)?.id;
        match m {
            "destroy" => submit(cx, vec![WorldCommand::Despawn { id }])?,
            "setParent" => {
                let parent = match opt(args, 0) {
                    Some(p) => Some(entity_id_of(cx, p)?),
                    None => None,
                };
                submit(cx, vec![WorldCommand::SetParent { id, parent }])?
            }
            "children" => {
                let s = cx.state()?;
                let kids: Vec<(u32, u32)> = s
                    .runtime
                    .host
                    .engine
                    .world
                    .entities(s.client)
                    .into_iter()
                    .filter(|(_, _, e)| e.parent == Some(id))
                    .map(|(i, g, _)| (i, g))
                    .collect();
                let mut out = Vec::new();
                for (i, g) in kids {
                    out.push(entity_object(cx, i, g)?);
                }
                return Ok(Value::List(out));
            }
            "setMaterial" => {
                let material = opt(args, 0)
                    .map(|v| serde_json::from_value(v.to_json()).context("material"))
                    .transpose()?;
                submit(cx, vec![WorldCommand::SetMaterial { id, material }])?;
            }
            "setLight" => {
                let light = opt(args, 0).map(parse_light).transpose()?;
                submit(cx, vec![WorldCommand::SetLight { id, light }])?
            }
            "setIrradianceVolume" => {
                let volume = opt(args, 0).map(parse_volume).transpose()?;
                submit(cx, vec![WorldCommand::SetIrradianceVolume { id, volume }])?
            }
            "setReflectionProbe" => {
                let probe = opt(args, 0).map(parse_probe).transpose()?;
                submit(cx, vec![WorldCommand::SetReflectionProbe { id, probe }])?
            }
            "setParticles" => {
                let particles = match opt(args, 0) {
                    Some(v) => {
                        let p: crysirror_scene::components::EmitterSpec =
                            serde_json::from_value(v.to_json()).context("particles")?;
                        p.validate()?;
                        Some(p)
                    }
                    None => None,
                };
                submit(cx, vec![WorldCommand::SetParticles { id, particles }])?
            }
            "setVfx" => {
                let vfx = opt(args, 0)
                    .map(|v| {
                        serde_json::from_value::<crysirror_vfx::SystemSpec>(v.to_json())
                            .context("VFX system")
                    })
                    .transpose()?;
                if let Some(system) = &vfx {
                    system.compile()?;
                }
                submit(cx, vec![WorldCommand::SetVfx { id, vfx }])?
            }
            "setVfxParameter" | "pauseVfx" | "stopVfx" | "restartVfx" => {
                let mut system = spec(cx, this)?.vfx.context("entity has no VFX system")?;
                match m {
                    "setVfxParameter" => {
                        let name = arg(args, 0, "parameter name")?.as_str()?;
                        let value = arg(args, 1, "parameter value")?;
                        let declared = system
                            .parameters
                            .get(name)
                            .context("VFX parameter is not declared")?;
                        use crysirror_vfx::ParameterValue as P;
                        let typed = match declared {
                            P::Float(_) => P::Float(value.as_f32()?),
                            P::Bool(_) => P::Bool(value.as_bool()?),
                            P::Vector(_) => P::Vector(value.as_vec3()?),
                            P::Color(_) => P::Color(
                                serde_json::from_value(value.to_json())
                                    .context("VFX color parameter requires four components")?,
                            ),
                        };
                        system = system
                            .with_parameters(&[(name.to_owned(), typed)].into_iter().collect())?;
                    }
                    "pauseVfx" => system.paused = arg(args, 0, "paused")?.as_bool()?,
                    "stopVfx" => system.enabled = false,
                    "restartVfx" => {
                        system.revision = system
                            .revision
                            .checked_add(1)
                            .context("VFX revision exhausted")?;
                        system.enabled = true;
                    }
                    _ => unreachable!(),
                }
                submit(
                    cx,
                    vec![WorldCommand::SetVfx {
                        id,
                        vfx: Some(system),
                    }],
                )?
            }
            "runBehaviorTree" => {
                let ai = match opt(args, 0) {
                    Some(t) => {
                        let tree = t.as_str()?.to_owned();
                        ensure!(
                            cx.state()?
                                .runtime
                                .host
                                .engine
                                .world
                                .ai
                                .trees
                                .contains_key(&tree),
                            "unknown behaviour tree '{tree}'"
                        );
                        let prev = spec(cx, this)?.ai.map(|a| a.blackboard).unwrap_or_default();
                        Some(crysirror_scene::ai::AiState {
                            tree,
                            blackboard: prev,
                            ..Default::default()
                        })
                    }
                    None => None,
                };
                submit(cx, vec![WorldCommand::SetAi { id, ai }])?
            }
            "setBlackboard" => {
                use crysirror_scene::ai::BbValue;
                let key = arg(args, 0, "key")?.as_str()?.to_owned();
                let mut ai = spec(cx, this)?.ai.context("entity has no behaviour tree")?;
                let v = match opt(args, 1) {
                    None => None,
                    Some(Value::Bool(b)) => Some(BbValue::Bool(*b)),
                    Some(Value::Str(s)) => Some(BbValue::Text(s.clone())),
                    Some(o @ Value::Object(_)) => {
                        Some(BbValue::Number(handle(cx, o.as_object()?)?.id as f32))
                    }
                    Some(l @ Value::List(_)) => Some(BbValue::Vec3(l.as_vec3()?)),
                    Some(other) => Some(BbValue::Number(other.as_f32()?)),
                };
                match v {
                    Some(v) => ai.blackboard.insert(key, v),
                    None => ai.blackboard.remove(&key),
                };
                submit(cx, vec![WorldCommand::SetAi { id, ai: Some(ai) }])?
            }
            "setStateMachine" | "setAnimParam" | "playMontage" | "stopMontage"
            | "drainAnimEvents" => {
                return anim_call(cx, this, id, m, args);
            }
            "initAttributes" | "attribute" | "setAttribute" | "applyEffect" | "removeEffect"
            | "hasTag" | "addTag" | "removeTag" | "grantAbility" | "activate" | "endAbility"
            | "drainEvents" => {
                return gameplay_call(cx, this, id, m, args);
            }
            "play" => {
                let clip = arg(args, 0, "clip")?.as_str()?.to_owned();
                let o = opt(args, 1);
                let f = |k: &str, d: f32| -> Result<f32> {
                    Ok(o.and_then(|o| o.field(k))
                        .map(Value::as_f32)
                        .transpose()?
                        .unwrap_or(d))
                };
                let looping = o
                    .and_then(|o| o.field("loop"))
                    .map(Value::as_bool)
                    .transpose()?
                    .unwrap_or(true);
                let mut a = spec(cx, this)?.animation.unwrap_or_default();
                a.play(&clip, f("speed", 1.)?, f("fade", 0.2)?, looping);
                submit(
                    cx,
                    vec![WorldCommand::SetAnimation {
                        id,
                        animation: Some(a),
                    }],
                )?
            }
            "setMorph" | "setMorphs" => {
                let mut a = spec(cx, this)?.animation.unwrap_or_default();
                if m == "setMorph" {
                    let target = arg(args, 0, "name")?.as_str()?.to_owned();
                    let w = arg(args, 1, "weight")?.as_f32()?;
                    if w == 0. {
                        a.morphs.remove(&target);
                    } else {
                        a.morphs.insert(target, w);
                    }
                } else {
                    a.morphs = match opt(args, 0) {
                        Some(v) => serde_json::from_value(v.to_json()).context("morph weights")?,
                        None => Default::default(),
                    };
                    a.morphs.retain(|_, w| *w != 0.);
                }
                submit(
                    cx,
                    vec![WorldCommand::SetAnimation {
                        id,
                        animation: Some(a),
                    }],
                )?
            }
            "stopAnimation" => submit(
                cx,
                vec![WorldCommand::SetAnimation {
                    id,
                    animation: None,
                }],
            )?,
            "setCollider" => {
                let collider = match opt(args, 0) {
                    Some(v) => Some(serde_json::from_value(v.to_json()).context("collider")?),
                    None => None,
                };
                submit(cx, vec![WorldCommand::SetCollider { id, collider }])?
            }
            "setVehicle" => {
                let h = handle(cx, this)?;
                let key = (h.client, h.id);
                let collider = spec(cx, this)?.collider;
                let s = cx.state()?;
                match opt(args, 0) {
                    None => {
                        s.vehicles.remove(&key);
                    }
                    Some(v) => {
                        ensure!(
                            collider.is_some_and(
                                |c| c.body == crysirror_scene::components::BodyKind::Character
                            ),
                            "setVehicle needs a character collider (body: 'character')"
                        );
                        let vs: crate::vehicle::VehicleSpec =
                            serde_json::from_value(v.to_json()).context("vehicle spec")?;
                        vs.validate()?;
                        let yaw = spec_rotation_y(&s.runtime.host.engine.world, key)?;
                        s.vehicles.insert(
                            key,
                            (
                                vs,
                                crate::vehicle::VehicleState {
                                    yaw,
                                    ..Default::default()
                                },
                            ),
                        );
                    }
                }
                return Ok(Value::Null);
            }
            "drive" => {
                let input: crate::vehicle::DriveInput =
                    serde_json::from_value(arg(args, 0, "input")?.to_json())
                        .context("drive input")?;
                let dt = arg(args, 1, "dt")?.as_f32()?;
                let h = handle(cx, this)?;
                let key = (h.client, h.id);
                let mut t = spec(cx, this)?.transform;
                let s = cx.state()?;
                let (vs, state) = *s.vehicles.get(&key).context("call setVehicle first")?;
                let r = crate::vehicle::step(
                    s.physics()?,
                    key,
                    &vs,
                    state,
                    glam::Vec3::from_array(t.translation),
                    input,
                    dt,
                )?;
                s.vehicles.insert(key, (vs, r.state));
                t.translation = r.position.to_array();
                t.rotation_degrees = crate::vehicle::rotation(&r.state);
                submit(cx, vec![WorldCommand::SetTransform { id, transform: t }])?;
                return Ok(Value::from_json(serde_json::json!({
                    "speed": r.forward_speed, "lateral": r.lateral_speed, "grounded": r.state.grounded,
                    "wheels": r.wheels_on_ground, "blocked": r.blocked, "yaw": r.state.yaw,
                })));
            }
            "moveCharacter" => {
                let desired = arg(args, 0, "desired")?.as_vec3()?;
                let dt = arg(args, 1, "dt")?.as_f32()?;
                let o = opt(args, 2);
                let f = |k: &str, d: f32| -> Result<f32> {
                    Ok(o.and_then(|o| o.field(k))
                        .map(Value::as_f32)
                        .transpose()?
                        .unwrap_or(d))
                };
                let (step, slope) = (f("step", 0.3)?, f("maxSlope", 45.)?);
                let h = handle(cx, this)?;
                let s = cx.state()?;
                let mv = s.physics()?.move_character(
                    (h.client, h.id),
                    desired.into(),
                    dt,
                    step,
                    slope,
                )?;
                let mut t = spec(cx, this)?.transform;
                for (k, d) in mv.translation.to_array().iter().enumerate() {
                    t.translation[k] += d;
                }
                submit(cx, vec![WorldCommand::SetTransform { id, transform: t }])?;
                return Ok(Value::Map(BTreeMap::from([
                    ("translation".into(), Value::vec3(mv.translation.to_array())),
                    ("grounded".into(), Value::Bool(mv.grounded)),
                ])));
            }
            "tween" => {
                let tween: Tween = serde_json::from_value(arg(args, 0, "spec")?.to_json())?;
                submit(cx, vec![WorldCommand::Tween { id, tween }])?
            }
            _ => unreachable!(),
        }
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------- Fs
pub struct FsClass;
fn fs_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Fs",
        doc: "This client's capability filesystem: read-only assets (incl. mounted providers) and atomic saves.",
        properties: vec![],
        methods: vec![
            method("readText", vec![("path", T::String)], T::String, "Read a UTF-8 asset."),
            method("exists", vec![("path", T::String)], T::Bool, "Whether an asset is readable."),
            method("readSave", vec![("path", T::String)], T::Optional(Box::new(T::String)), "Read a UTF-8 save file, or null."),
            method("writeSave", vec![("path", T::String), ("text", T::String)], T::Void, "Atomically replace a save file."),
        ],
    })
}
impl Class for FsClass {
    fn info(&self) -> &ClassInfo {
        fs_info()
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        bail!("Fs has no property {p}")
    }
    fn call(&self, cx: &mut EngineContext, _: ObjectId, m: &str, args: &[Value]) -> Result<Value> {
        let path = arg(args, 0, "path")?.as_str()?;
        let mount = if m.ends_with("Save") {
            Mount::Save
        } else {
            Mount::Asset
        };
        let p = VfsPath::new(mount, path).map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
        let s = cx.state()?;
        let client = s.client;
        let fs = &mut s.runtime.host.fs;
        match m {
            "readText" => Ok(String::from_utf8(
                fs.read(client, &p)
                    .map_err(|e| anyhow::anyhow!("{path}: {e}"))?,
            )?
            .into()),
            "exists" => Ok(fs.read(client, &p).is_ok().into()),
            "readSave" => match fs.read(client, &p) {
                Ok(b) => Ok(String::from_utf8(b)?.into()),
                Err(crysirror_core::Error::NotFound) => Ok(Value::Null),
                Err(e) => bail!("{path}: {e}"),
            },
            "writeSave" => {
                let text = arg(args, 1, "text")?.as_str()?;
                fs.write_atomic(client, &p, text.as_bytes())
                    .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
                Ok(Value::Null)
            }
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------- Client
pub struct ClientClass;
fn client_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Client",
        doc: "A project client: a VM peer (data VM, Siglus SceneVm, ...) with its own scheduling, waits and saves.",
        properties: vec![
            prop("name", T::String, false, "Project client name."),
            prop("engine", T::String, false, "Adapter id, e.g. 'siglus'."),
            prop("state", T::String, false, "Last VM yield: BudgetExhausted, WaitUntil(..), WaitForInput, Halted."),
            prop("diagnostics", T::Any, false, "Bounded adapter diagnostics (read-only)."),
        ],
        methods: vec![],
    })
}
impl Class for ClientClass {
    fn info(&self) -> &ClassInfo {
        client_info()
    }
    fn get(&self, cx: &mut EngineContext, this: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        let name = s.objects.get(this)?.payload.as_str()?.to_owned();
        let info = s.runtime.inspect();
        let session = info["sessions"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"] == name.as_str()))
            .with_context(|| format!("client '{name}' not found"))?;
        Ok(match p {
            "name" => name.into(),
            "engine" => Value::from_json(session["engine"].clone()),
            "state" => Value::from_json(session["yield"].clone()),
            "diagnostics" => Value::from_json(session["native"].clone()),
            _ => unreachable!(),
        })
    }
    fn call(&self, _: &mut EngineContext, _: ObjectId, m: &str, _: &[Value]) -> Result<Value> {
        bail!("Client has no method {m}")
    }
}

/// Gameplay methods: mutate a copy of the component, then submit it as one
/// transactional world command (nothing changes when a call errors).
fn gameplay_call(
    cx: &mut EngineContext,
    this: ObjectId,
    id: u32,
    m: &str,
    args: &[Value],
) -> Result<Value> {
    use crysirror_scene::gameplay::{Attribute, GameplayState};
    let defs = cx.state()?.runtime.host.engine.world.gameplay.clone();
    let mut g: GameplayState = spec(cx, this)?.gameplay.unwrap_or_default();
    let name = || -> Result<String> { Ok(arg(args, 0, "name")?.as_str()?.to_owned()) };
    let (ret, changed) = match m {
        "attribute" => {
            return Ok(g
                .value(&defs, &name()?)
                .map(|v| Value::Float(v as f64))
                .unwrap_or(Value::Null));
        }
        "hasTag" => return Ok(g.has_tag(&defs, &name()?).into()),
        "initAttributes" => {
            let attrs: BTreeMap<String, Attribute> =
                serde_json::from_value(arg(args, 0, "attributes")?.to_json())
                    .context("attributes")?;
            g.attributes.extend(attrs);
            (Value::Null, true)
        }
        "setAttribute" => {
            let n = name()?;
            let v = arg(args, 1, "base")?.as_f32()?;
            g.attributes
                .get_mut(&n)
                .with_context(|| format!("no attribute {n}"))?
                .base = v;
            (Value::Null, true)
        }
        "applyEffect" => {
            let source = match opt(args, 1) {
                Some(e) => Some(handle(cx, e.as_object()?)?.id),
                None => None,
            };
            let ok = g.apply_effect(&defs, &name()?, source)?;
            (ok.into(), ok)
        }
        "removeEffect" => {
            let ok = g.remove_effect(&defs, &name()?);
            (ok.into(), ok)
        }
        "addTag" => {
            let t = name()?;
            *g.loose_tags.entry(t).or_insert(0) += 1;
            (Value::Null, true)
        }
        "removeTag" => {
            let t = name()?;
            if let Some(c) = g.loose_tags.get_mut(&t) {
                *c -= 1;
                if *c == 0 {
                    g.loose_tags.remove(&t);
                }
            }
            (Value::Null, true)
        }
        "grantAbility" => {
            let a = name()?;
            ensure!(defs.abilities.contains_key(&a), "unknown ability '{a}'");
            if !g.granted.contains(&a) {
                g.granted.push(a);
            }
            (Value::Null, true)
        }
        "activate" => {
            let r = g.try_activate(&defs, &name()?)?;
            let ok = r.is_ok();
            let reason = r.err().map(Value::Str).unwrap_or(Value::Null);
            (
                Value::Map(BTreeMap::from([
                    ("ok".into(), Value::Bool(ok)),
                    ("reason".into(), reason),
                ])),
                ok,
            )
        }
        "endAbility" => {
            let ok = g.end_ability(&defs, &name()?);
            (ok.into(), ok)
        }
        "drainEvents" => {
            let ev = std::mem::take(&mut g.events);
            (Value::from_json(serde_json::to_value(&ev)?), !ev.is_empty())
        }
        _ => unreachable!(),
    };
    if changed {
        submit(
            cx,
            vec![WorldCommand::SetGameplay {
                id,
                gameplay: Some(g),
            }],
        )?;
    }
    Ok(ret)
}

fn anim_call(
    cx: &mut EngineContext,
    this: ObjectId,
    id: u32,
    m: &str,
    args: &[Value],
) -> Result<Value> {
    use crysirror_scene::animgraph::{MachineInstance, MontageInstance};
    let defs = cx.state()?.runtime.host.engine.world.animation.clone();
    let mut a = spec(cx, this)?.animation.unwrap_or_default();
    let mut ret = Value::Null;
    match m {
        "setStateMachine" => match opt(args, 0) {
            Some(v) => {
                let name = v.as_str()?;
                let def = defs
                    .machines
                    .get(name)
                    .with_context(|| format!("unknown state machine '{name}'"))?;
                a.machine = Some(MachineInstance::new(name, def));
            }
            None => a.machine = None,
        },
        "setAnimParam" => {
            let name = arg(args, 0, "name")?.as_str()?.to_owned();
            let v = match arg(args, 1, "value")? {
                Value::Bool(b) => f32::from(u8::from(*b)),
                other => other.as_f32()?,
            };
            a.machine
                .as_mut()
                .context("no state machine set")?
                .params
                .insert(name, v);
        }
        "playMontage" => {
            let name = arg(args, 0, "montage")?.as_str()?.to_owned();
            let def = defs
                .montages
                .get(&name)
                .with_context(|| format!("unknown montage '{name}'"))?;
            let time = match opt(args, 1) {
                Some(s) => *def
                    .sections
                    .get(s.as_str()?)
                    .with_context(|| format!("montage '{name}' has no section"))?,
                None => 0.,
            };
            // Jumping to a section of the playing montage keeps its weight (combo).
            let weight = a
                .montage
                .as_ref()
                .filter(|x| x.montage == name)
                .map(|x| x.weight)
                .unwrap_or(0.);
            a.montage = Some(MontageInstance {
                montage: name,
                time,
                weight,
                stopping: false,
            });
        }
        "stopMontage" => {
            if let Some(mo) = a.montage.as_mut() {
                mo.stopping = true;
            }
        }
        "drainAnimEvents" => {
            ret = Value::from_json(serde_json::to_value(std::mem::take(&mut a.events))?)
        }
        _ => unreachable!(),
    }
    submit(
        cx,
        vec![WorldCommand::SetAnimation {
            id,
            animation: Some(a),
        }],
    )?;
    Ok(ret)
}

/// Heading of an entity: its Y Euler angle, the vehicle yaw convention.
fn spec_rotation_y(world: &crysirror_scene::world::World, key: (u32, u32)) -> Result<f32> {
    let h = world.handle(ClientId(key.0), key.1)?;
    Ok(world.entity(h)?.transform.rotation_degrees[1])
}
