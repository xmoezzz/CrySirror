// SPDX-License-Identifier: MPL-2.0
//! Reflective physics API over the runtime's one shared simulation service.
use crate::{
    builtin,
    context::EngineContext,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_physics as p;
use crysirror_scene::{
    components::{BodyKind, ColliderShape},
    physics::{BodyForces, ClientPhysicsScene, JointFrame, JointHandle, JointSpec, JointState},
    world::{EntityHandle, WorldCommand},
};
use glam::{EulerRot, Quat, Vec3};
use std::sync::{Arc, OnceLock};
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
fn property(name: &'static str, ty: T, writable: bool, doc: &'static str) -> PropertyInfo {
    PropertyInfo {
        name,
        ty,
        writable,
        doc,
    }
}
fn arg<'a>(args: &'a [Value], i: usize) -> Result<&'a Value> {
    args.get(i).context("missing physics argument")
}
fn opt(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i).filter(|v| !matches!(v, Value::Null))
}
fn current(cx: &mut EngineContext) -> Result<ClientPhysicsScene> {
    let s = cx.state()?;
    Ok(s.runtime
        .host
        .engine
        .world
        .physics
        .clients
        .get(&s.client.0)
        .cloned()
        .unwrap_or_default())
}
fn submit(cx: &mut EngineContext, physics: ClientPhysicsScene) -> Result<()> {
    builtin::submit(cx, vec![WorldCommand::SetPhysics { physics }])
}
fn joint_object(cx: &mut EngineContext, h: JointHandle) -> Result<Value> {
    let value = Value::from_json(serde_json::to_value(h)?);
    let id = cx.state()?.objects.intern("PhysicsJoint", value)?;
    Ok(Value::object(id, "PhysicsJoint"))
}
fn joint_handle(cx: &mut EngineContext, id: ObjectId) -> Result<JointHandle> {
    let s = cx.state()?;
    let e = s.objects.get(id)?;
    ensure!(e.class == "PhysicsJoint", "expected a PhysicsJoint");
    let h: JointHandle = serde_json::from_value(e.payload.to_json())?;
    ensure!(h.client == s.client.0, "joint belongs to another client");
    let entry = s
        .runtime
        .host
        .engine
        .world
        .physics
        .clients
        .get(&h.client)
        .and_then(|p| p.joints.get(&h.id))
        .context("joint was destroyed")?;
    ensure!(entry.generation == h.generation, "stale joint handle");
    Ok(h)
}
fn frame(value: &Value) -> Result<p::Pose> {
    let t: JointFrame = serde_json::from_value(value.to_json())
        .context("physics pose {translation,rotation_degrees}")?;
    let r = t.rotation_degrees.map(f32::to_radians);
    ensure!(
        t.translation
            .iter()
            .chain(&t.rotation_degrees)
            .all(|v| v.is_finite()),
        "invalid physics pose"
    );
    Ok(p::Pose {
        translation: t.translation.into(),
        rotation: Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
    })
}
fn shape(value: &Value) -> Result<p::Shape> {
    let shape: ColliderShape = serde_json::from_value(value.to_json()).context("query shape")?;
    let c: crysirror_scene::components::ColliderSpec =
        serde_json::from_value(serde_json::json!({"shape":value.to_json()}))?;
    c.validate()?;
    Ok(match shape {
        ColliderShape::Box { half_extents } => p::Shape::Cuboid {
            half_extents: half_extents.into(),
        },
        ColliderShape::Sphere { radius } => p::Shape::Ball { radius },
        ColliderShape::Capsule {
            radius,
            half_height,
        } => p::Shape::Capsule {
            radius,
            half_height,
        },
        ColliderShape::ConvexHull { points } => p::Shape::ConvexHull {
            points: Arc::new(points.into_iter().map(Vec3::from).collect()),
        },
        ColliderShape::Mesh => bail!(
            "query mesh needs explicit convex points; entity meshes remain available to collisions"
        ),
    })
}
fn options(cx: &mut EngineContext, value: Option<&Value>) -> Result<p::QueryOptions> {
    let mut ignore = None;
    let mut groups = None;
    let mut sensors = false;
    let mut only_static = false;
    if let Some(value) = value {
        let Value::Map(map) = value else {
            bail!("physics query options must be a map")
        };
        ensure!(
            map.keys()
                .all(|k| matches!(k.as_str(), "ignore" | "groups" | "sensors" | "staticOnly")),
            "unknown physics query option"
        );
        if let Some(v) = map.get("ignore").filter(|v| !matches!(v, Value::Null)) {
            ignore = Some(builtin::handle(cx, v.as_object()?)?);
        }
        if let Some(v) = map.get("groups") {
            let g: crysirror_scene::components::CollisionGroups =
                serde_json::from_value(v.to_json())?;
            groups = Some(p::CollisionGroups {
                memberships: g.memberships,
                filter: g.filter,
            });
        }
        if let Some(v) = map.get("sensors") {
            sensors = v.as_bool()?;
        }
        if let Some(v) = map.get("staticOnly") {
            only_static = v.as_bool()?;
        }
    }
    let s = cx.state()?;
    let client = s.client.0;
    s.physics()?
        .query_options(client, ignore, groups, only_static, sensors)
}
fn hit(cx: &mut EngineContext, client: u32, hit: p::QueryHit) -> Result<Value> {
    let entity = builtin::entity_object(cx, hit.key.id, hit.key.generation)?;
    ensure!(cx.state()?.client.0 == client, "query result scope");
    let mut out = std::collections::BTreeMap::new();
    out.insert("entity".into(), entity);
    out.insert("point".into(), Value::vec3(hit.point.to_array()));
    out.insert("normal".into(), Value::vec3(hit.normal.to_array()));
    out.insert("distance".into(), Value::Float(hit.distance as f64));
    Ok(Value::Map(out))
}
pub fn register(classes: &mut ClassRegistry) -> Result<()> {
    classes.register(Arc::new(PhysicsClass))?;
    classes.register(Arc::new(JointClass))?;
    Ok(())
}
pub struct PhysicsClass;
impl Class for PhysicsClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"Physics",doc:"Persistent client-isolated rigid bodies, CCD, collision queries and joints. Simulation runs in every Runtime 16 ms world step, including pure data projects. Snapshots rebuild from saved pose, velocity, forces and joint declarations; solver warm starts are not serialized.",
        properties:vec![property("gravity",T::Vec3,true,"Client gravity in world units/s²."),property("stats",T::Map,false,"Bodies/colliders/joints, sleeping counts, stable-handle updates, shape-cache and fixed-step counters for this client.")],
        methods:vec![
            method("raycast",vec![("origin",T::Vec3),("direction",T::Vec3),("maxDistance",T::Number),("options",T::Optional(Box::new(T::Map)))],T::Any,"Closest hit {entity,point,normal,distance} or null. Options {ignore?:Entity,groups?:{memberships,filter},sensors?:false,staticOnly?:false}; never queries another client."),
            method("sphereCast",vec![("origin",T::Vec3),("direction",T::Vec3),("radius",T::Number),("maxDistance",T::Number),("options",T::Optional(Box::new(T::Map)))],T::Any,"Sweep a sphere. Returns its center at impact as point, surface normal, travel distance and a scoped entity or null."),
            method("castShape",vec![("shape",T::Map),("pose",T::Map),("direction",T::Vec3),("maxDistance",T::Number),("options",T::Optional(Box::new(T::Map)))],T::Any,"Sweep box/sphere/capsule/convex_hull. Pose {translation?:Vec3,rotation_degrees?:Vec3}; dimensions are world units. Hit point is the moving shape origin at impact."),
            method("overlapShape",vec![("shape",T::Map),("pose",T::Map),("options",T::Optional(Box::new(T::Map)))],T::List(Box::new(T::Object("Entity"))),"All scoped bodies intersecting the supplied world-space shape; sensors are opt-in."),
            method("createJoint",vec![("a",T::Object("Entity")),("b",T::Object("Entity")),("spec",T::Map)],T::Object("PhysicsJoint"),"Create fixed/spherical/revolute/prismatic joint. Local anchors/frames, axes, optional limits and motor; revolute motor/limits use radians, prismatic meters. Saved with generational body identities."),
            method("events",vec![],T::List(Box::new(T::Map)),"Drain this client's contact/sensor/force/sleep events {kind,entity,other,force,forceMagnitude}. Bounded queue; counters report dropped events."),
        ]})
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, name: &str) -> Result<Value> {
        match name {
            "gravity" => Ok(Value::vec3(current(cx)?.gravity)),
            "stats" => {
                let s = cx.state()?;
                let client = s.client.0;
                let stats = s.physics()?.stats(client);
                Ok(Value::from_json(serde_json::to_value(stats)?))
            }
            _ => bail!("unknown Physics property"),
        }
    }
    fn set(&self, cx: &mut EngineContext, _: ObjectId, name: &str, value: Value) -> Result<()> {
        ensure!(name == "gravity", "unknown Physics property");
        let mut state = current(cx)?;
        state.gravity = value.as_vec3()?;
        submit(cx, state)
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        if name == "createJoint" {
            let a = builtin::handle(cx, arg(args, 0)?.as_object()?)?;
            let b = builtin::handle(cx, arg(args, 1)?.as_object()?)?;
            ensure!(
                a.client == b.client && a != b,
                "joint bodies must be distinct and owned by this client"
            );
            let spec: JointSpec = serde_json::from_value(arg(args, 2)?.to_json())?;
            spec.validate()?;
            let mut state = current(cx)?;
            let id = state
                .next_joint
                .checked_add(1)
                .context("joint id exhausted")?;
            state.next_joint = id;
            state.joints.insert(
                id,
                JointState {
                    generation: 1,
                    a,
                    b,
                    joint: spec,
                },
            );
            submit(cx, state)?;
            return joint_object(
                cx,
                JointHandle {
                    client: a.client,
                    id,
                    generation: 1,
                },
            );
        }
        if name == "events" {
            let s = cx.state()?;
            let client = s.client.0;
            let events = s.physics()?.drain_events(client);
            let mut out = Vec::new();
            for event in events {
                let entity = builtin::entity_object(cx, event.entity.id, event.entity.generation)?;
                let other = event
                    .other
                    .map(|e| builtin::entity_object(cx, e.id, e.generation))
                    .transpose()?
                    .unwrap_or(Value::Null);
                out.push(Value::Map(std::collections::BTreeMap::from([
                    (
                        "kind".into(),
                        Value::from_json(serde_json::to_value(event.kind)?),
                    ),
                    ("entity".into(), entity),
                    ("other".into(), other),
                    ("force".into(), Value::vec3(event.force)),
                    (
                        "forceMagnitude".into(),
                        Value::Float(event.force_magnitude as f64),
                    ),
                ])));
            }
            return Ok(Value::List(out));
        }
        let client = cx.state()?.client.0;
        match name {
            "raycast" => {
                let o = Vec3::from(arg(args, 0)?.as_vec3()?);
                let d = Vec3::from(arg(args, 1)?.as_vec3()?);
                let max = arg(args, 2)?.as_f32()?;
                let opts = options(cx, opt(args, 3))?;
                let hit_value = cx
                    .state()?
                    .physics()?
                    .query_raycast(client, o, d, max, opts)?;
                hit_value
                    .map(|h| hit(cx, client, h))
                    .transpose()
                    .map(|v| v.unwrap_or(Value::Null))
            }
            "sphereCast" | "castShape" => {
                let (s, pose, d, max, opts) = if name == "sphereCast" {
                    (
                        p::Shape::Ball {
                            radius: arg(args, 2)?.as_f32()?,
                        },
                        p::Pose::at(arg(args, 0)?.as_vec3()?.into()),
                        Vec3::from(arg(args, 1)?.as_vec3()?),
                        arg(args, 3)?.as_f32()?,
                        options(cx, opt(args, 4))?,
                    )
                } else {
                    (
                        shape(arg(args, 0)?)?,
                        frame(arg(args, 1)?)?,
                        Vec3::from(arg(args, 2)?.as_vec3()?),
                        arg(args, 3)?.as_f32()?,
                        options(cx, opt(args, 4))?,
                    )
                };
                let value = cx
                    .state()?
                    .physics()?
                    .query_cast(client, &s, pose, d, max, opts)?;
                value
                    .map(|h| hit(cx, client, h))
                    .transpose()
                    .map(|v| v.unwrap_or(Value::Null))
            }
            "overlapShape" => {
                let s = shape(arg(args, 0)?)?;
                let pose = frame(arg(args, 1)?)?;
                let opts = options(cx, opt(args, 2))?;
                let hits = cx
                    .state()?
                    .physics()?
                    .query_overlaps(client, &s, pose, opts)?;
                let mut out = Vec::new();
                for h in hits {
                    out.push(builtin::entity_object(cx, h.key.id, h.key.generation)?);
                }
                Ok(Value::List(out))
            }
            _ => bail!("unknown Physics method {name}"),
        }
    }
}
pub struct JointClass;
impl Class for JointClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"PhysicsJoint",doc:"Client-scoped generational joint declaration; native solver handles remain private.",properties:vec![property("id",T::Int,false,"Client-local joint id."),property("alive",T::Bool,false,"False after removal or body destruction."),property("spec",T::Map,false,"Saved joint configuration."),property("bodyA",T::Object("Entity"),false,"First body."),property("bodyB",T::Object("Entity"),false,"Second body.")],methods:vec![method("setSpec",vec![("spec",T::Map)],T::Void,"Update limits/motor/anchors while retaining the solver joint handle."),method("destroy",vec![],T::Void,"Remove this joint; stale calls fail.")]})
    }
    fn get(&self, cx: &mut EngineContext, this: ObjectId, name: &str) -> Result<Value> {
        if name == "alive" {
            return Ok(Value::Bool(joint_handle(cx, this).is_ok()));
        }
        let h = joint_handle(cx, this)?;
        let state = current(cx)?.joints[&h.id].clone();
        match name {
            "id" => Ok(Value::Int(h.id.into())),
            "spec" => Ok(Value::from_json(serde_json::to_value(state.joint)?)),
            "bodyA" => builtin::entity_object(cx, state.a.id, state.a.generation),
            "bodyB" => builtin::entity_object(cx, state.b.id, state.b.generation),
            _ => bail!("unknown PhysicsJoint property"),
        }
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        this: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        let h = joint_handle(cx, this)?;
        let mut state = current(cx)?;
        match name {
            "destroy" => {
                state.joints.remove(&h.id);
            }
            "setSpec" => {
                let spec: JointSpec = serde_json::from_value(arg(args, 0)?.to_json())?;
                spec.validate()?;
                state.joints.get_mut(&h.id).unwrap().joint = spec;
            }
            _ => bail!("unknown PhysicsJoint method"),
        };
        submit(cx, state)?;
        Ok(Value::Null)
    }
}
pub(crate) fn is_entity_method(name: &str) -> bool {
    matches!(
        name,
        "applyImpulse"
            | "applyTorqueImpulse"
            | "addForce"
            | "addTorque"
            | "clearForces"
            | "wake"
            | "sleep"
            | "setKinematicTarget"
    )
}
pub(crate) fn entity_state(cx: &mut EngineContext, h: EntityHandle) -> Result<Value> {
    let s = cx.state()?;
    if s.runtime.host.engine.world.entity(h)?.collider.is_none() {
        return Ok(Value::Null);
    }
    let state = s.physics()?.body_state(h)?;
    Ok(Value::from_json(
        serde_json::json!({"position":state.pose.translation.to_array(),"rotation":state.pose.rotation.to_array(),"velocity":state.linear_velocity.to_array(),"angularVelocity":state.angular_velocity.to_array().map(f32::to_degrees),"sleeping":state.sleeping}),
    ))
}
pub(crate) fn entity_call(
    cx: &mut EngineContext,
    this: ObjectId,
    name: &str,
    args: &[Value],
) -> Result<Value> {
    let h = builtin::handle(cx, this)?;
    let body = cx
        .state()?
        .runtime
        .host
        .engine
        .world
        .entity(h)?
        .collider
        .as_ref()
        .context("entity has no collider")?
        .body;
    if matches!(
        name,
        "applyImpulse" | "applyTorqueImpulse" | "addForce" | "addTorque"
    ) {
        ensure!(
            body == BodyKind::Dynamic,
            "forces/impulses require a dynamic body"
        );
    }
    match name {
        "addForce" | "addTorque" | "clearForces" => {
            let mut state = current(cx)?;
            if name == "clearForces" {
                state.forces.remove(&h.id);
            } else {
                let v = arg(args, 0)?.as_vec3()?;
                let force = state.forces.entry(h.id).or_insert(BodyForces {
                    generation: h.generation,
                    ..Default::default()
                });
                let to = if name == "addForce" {
                    &mut force.force
                } else {
                    &mut force.torque
                };
                for i in 0..3 {
                    to[i] += v[i];
                }
            }
            submit(cx, state)?;
        }
        "applyImpulse" | "applyTorqueImpulse" => {
            let value = Vec3::from(arg(args, 0)?.as_vec3()?);
            let point = opt(args, 1)
                .map(|v| v.as_vec3().map(Vec3::from))
                .transpose()?;
            let s = cx.state()?;
            s.physics()?;
            if name == "applyImpulse" {
                s.runtime
                    .physics
                    .impulse(h, value, point, &mut s.runtime.host.engine.world)?;
            } else {
                s.runtime
                    .physics
                    .torque_impulse(h, value, &mut s.runtime.host.engine.world)?;
            }
        }
        "wake" | "sleep" => {
            cx.state()?.physics()?.sleep(h, name == "sleep")?;
        }
        "setKinematicTarget" => {
            ensure!(
                body == BodyKind::KinematicPosition,
                "kinematic target requires kinematic_position body"
            );
            let position = arg(args, 0)?.as_vec3()?;
            let rotation = if let Some(v) = opt(args, 1) {
                v.as_vec3()?
            } else {
                let state = cx.state()?.physics()?.body_state(h)?;
                let (x, y, z) = state.pose.rotation.to_euler(EulerRot::XYZ);
                [x.to_degrees(), y.to_degrees(), z.to_degrees()]
            };
            let mut state = current(cx)?;
            state.kinematic_targets.insert(
                h.id,
                crysirror_scene::physics::KinematicTarget {
                    generation: h.generation,
                    pose: JointFrame {
                        translation: position,
                        rotation_degrees: rotation,
                    },
                },
            );
            submit(cx, state)?;
        }
        _ => bail!("unknown Entity physics method"),
    }
    Ok(Value::Null)
}
