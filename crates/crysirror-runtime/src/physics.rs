// SPDX-License-Identifier: MPL-2.0
//! Scene-to-physics bridge. Per-client persistent solvers are derived from
//! saved World data; visual revisions never recreate an unchanged collider.
use crate::vfs::DiskVfs;
use anyhow::{ensure, Context, Result};
use crysirror_asset::pbr as cpu;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::FileSystem,
    state::ClientId,
};
use crysirror_physics as backend;
use crysirror_scene::{
    components::{BodyKind, ColliderShape, ColliderSpec},
    model::Geometry,
    physics::{BodyForces, JointSpec, JointState},
    world::{geometry_data, EntityHandle, World},
};
use glam::{EulerRot, Mat4, Quat, Vec3};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
type Key = (u32, u32, u32);
type MeshData = (Arc<Vec<Vec3>>, Arc<Vec<[u32; 3]>>);
struct Entry {
    entity: EntityHandle,
    body: backend::BodyHandle,
    spec: ColliderSpec,
    geometry: Option<Geometry>,
    matrix: Mat4,
    scale: Vec3,
    linear: [f32; 3],
    angular: [f32; 3],
    forces: Option<BodyForces>,
    target: Option<crysirror_scene::physics::KinematicTarget>,
}
struct JointEntry {
    native: backend::JointHandle,
    state: JointState,
}
pub struct Physics {
    worlds: RefCell<BTreeMap<u32, backend::PhysicsWorld>>,
    entries: BTreeMap<Key, Entry>,
    joints: BTreeMap<Key, JointEntry>,
    meshes: HashMap<String, MeshData>,
    terrain_clients: HashSet<u32>,
    gravities: HashMap<u32, Vec3>,
    synced_revision: Option<u64>,
    query_client: u32,
}
impl Default for Physics {
    fn default() -> Self {
        Self {
            worlds: RefCell::new(BTreeMap::new()),
            entries: BTreeMap::new(),
            joints: BTreeMap::new(),
            meshes: HashMap::new(),
            terrain_clients: HashSet::new(),
            gravities: HashMap::new(),
            synced_revision: None,
            query_client: 1,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub entity: (u32, u32),
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct CharacterMove {
    pub translation: Vec3,
    pub grounded: bool,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PhysicsEvent {
    pub kind: backend::EventKind,
    pub entity: EntityHandle,
    pub other: Option<EntityHandle>,
    pub force: [f32; 3],
    pub force_magnitude: f32,
}
fn key(h: EntityHandle) -> Key {
    (h.client, h.id, h.generation)
}
fn bodykey(h: EntityHandle) -> backend::BodyKey {
    backend::BodyKey {
        id: h.id,
        generation: h.generation,
    }
}
// Parry 0.17 heightfield rays can miss exactly along cell seams. Preserve the
// prior bridge's tiny tilt for valid vertical rays; never repair a zero/NaN ray.
fn robust_dir(d: Vec3) -> Vec3 {
    if d.y != 0. && d.x.abs() < 1e-6 && d.z.abs() < 1e-6 {
        Vec3::new(1e-4, d.y.signum(), 7e-5).normalize()
    } else {
        d
    }
}
fn jointdesc(spec: &JointSpec) -> backend::JointDesc {
    let frame = |f: &crysirror_scene::physics::JointFrame| {
        let r = f.rotation_degrees.map(f32::to_radians);
        backend::Pose {
            translation: f.translation.into(),
            rotation: Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
        }
    };
    let motor = |m: &Option<crysirror_scene::physics::JointMotor>| {
        m.as_ref().map(|m| backend::Motor {
            target_position: m.target_position,
            target_velocity: m.target_velocity,
            stiffness: m.stiffness,
            damping: m.damping,
            max_force: m.max_force,
        })
    };
    match spec {
        JointSpec::Fixed {
            local_frame1,
            local_frame2,
            contacts_enabled,
        } => backend::JointDesc::Fixed {
            local_frame1: frame(local_frame1),
            local_frame2: frame(local_frame2),
            contacts_enabled: *contacts_enabled,
        },
        JointSpec::Spherical {
            local_anchor1,
            local_anchor2,
            contacts_enabled,
        } => backend::JointDesc::Spherical {
            local_anchor1: (*local_anchor1).into(),
            local_anchor2: (*local_anchor2).into(),
            contacts_enabled: *contacts_enabled,
        },
        JointSpec::Revolute {
            local_anchor1,
            local_anchor2,
            axis1,
            axis2,
            limits,
            motor: m,
            contacts_enabled,
        } => backend::JointDesc::Revolute {
            local_anchor1: (*local_anchor1).into(),
            local_anchor2: (*local_anchor2).into(),
            axis1: (*axis1).into(),
            axis2: (*axis2).into(),
            limits: *limits,
            motor: motor(m),
            contacts_enabled: *contacts_enabled,
        },
        JointSpec::Prismatic {
            local_anchor1,
            local_anchor2,
            axis1,
            axis2,
            limits,
            motor: m,
            contacts_enabled,
        } => backend::JointDesc::Prismatic {
            local_anchor1: (*local_anchor1).into(),
            local_anchor2: (*local_anchor2).into(),
            axis1: (*axis1).into(),
            axis2: (*axis2).into(),
            limits: *limits,
            motor: motor(m),
            contacts_enabled: *contacts_enabled,
        },
    }
}
impl Physics {
    pub fn set_query_client(&mut self, client: u32) {
        self.query_client = client;
    }
    pub fn invalidate(&mut self) {
        let client = self.query_client;
        *self = Self::default();
        self.query_client = client;
    }
    pub fn invalidate_client(&mut self, client: u32) {
        self.worlds.get_mut().remove(&client);
        self.gravities.remove(&client);
        self.entries.retain(|k, _| k.0 != client);
        self.joints.retain(|k, _| k.0 != client);
        let prefix = format!("{client}:");
        self.meshes.retain(|k, _| !k.starts_with(&prefix));
        self.synced_revision = None;
    }
    pub fn sync(&mut self, world: &World, fs: &mut DiskVfs) -> Result<()> {
        if self.synced_revision == Some(world.revision) {
            return Ok(());
        }
        let mut seen = HashSet::new();
        let wanted_joints = world
            .physics
            .clients
            .iter()
            .flat_map(|(&c, p)| {
                p.joints
                    .iter()
                    .map(move |(&id, j)| ((c, id, j.generation), j))
            })
            .collect::<BTreeMap<_, _>>();
        for stale in self
            .joints
            .keys()
            .filter(|k| !wanted_joints.contains_key(k))
            .copied()
            .collect::<Vec<_>>()
        {
            let old = self.joints.remove(&stale).unwrap();
            if let Some(w) = self.worlds.get_mut().get_mut(&stale.0) {
                w.remove_joint(old.native)?;
            }
        }
        for client in world.client_ids().collect::<Vec<_>>() {
            if !self.worlds.get_mut().contains_key(&client) {
                self.worlds.get_mut().insert(
                    client,
                    backend::PhysicsWorld::new(
                        client,
                        backend::WorldConfig {
                            fixed_dt: 0.016,
                            ..Default::default()
                        },
                    )?,
                );
            }
            let gravity = Vec3::from(
                world
                    .physics
                    .clients
                    .get(&client)
                    .map(|s| s.gravity)
                    .unwrap_or([0., -9.81, 0.]),
            );
            self.gravities.insert(client, gravity);
            let native = self.worlds.get_mut().get_mut(&client).unwrap();
            native.configure(gravity, native.fixed_dt())?;
            for (id, generation, spec) in world.entities(ClientId(client)) {
                let Some(c) = &spec.collider else { continue };
                c.validate()?;
                let h = EntityHandle {
                    client,
                    id,
                    generation,
                };
                let k = key(h);
                seen.insert(k);
                let matrix = world.global_matrix(h)?;
                let baked_static = c.body == BodyKind::Static
                    && matches!(c.shape, ColliderShape::Mesh)
                    && !matches!(spec.geometry, Some(Geometry::Terrain(_)));
                let (global_pose, mut scale) = if baked_static {
                    (backend::Pose::default(), Vec3::ONE)
                } else {
                    backend::decompose_transform(matrix)?
                };
                // A rotating unit TRS matrix can decompose to 1±1 ulp. Those
                // numerical lengths are not authored scale edits and must not
                // rebuild a dynamic collider (or wake it) every few frames.
                if !baked_static && spec.parent.is_none() {
                    scale = Vec3::from(spec.transform.scale);
                } else if let Some(previous) = self.entries.get(&k) {
                    let tolerance = previous.scale.abs().max(scale.abs()).max(Vec3::ONE) * 1e-6;
                    if (scale - previous.scale).abs().cmple(tolerance).all() {
                        scale = previous.scale;
                    }
                }
                let rebuild = self.entries.get(&k).is_none_or(|old| {
                    old.spec != *c
                        || old.geometry != spec.geometry
                        || old.scale != scale
                        || (baked_static && old.matrix != matrix)
                });
                if rebuild {
                    let shape = self.shape(
                        client,
                        c,
                        spec.geometry.as_ref(),
                        matrix,
                        scale,
                        baked_static,
                        fs,
                    )?;
                    let desc = backend::BodyDesc {
                        kind: match c.body {
                            BodyKind::Static => backend::BodyKind::Fixed,
                            BodyKind::Dynamic => backend::BodyKind::Dynamic,
                            BodyKind::Kinematic => backend::BodyKind::KinematicVelocity,
                            BodyKind::KinematicPosition | BodyKind::Character => {
                                backend::BodyKind::KinematicPosition
                            }
                        },
                        pose: global_pose,
                        linear_velocity: spec.velocity.into(),
                        angular_velocity: Vec3::from(spec.angular_velocity.map(f32::to_radians)),
                        mass: c.mass,
                        gravity_scale: c.gravity_scale,
                        linear_damping: c.linear_damping,
                        angular_damping: c.angular_damping,
                        ccd: c.ccd,
                        can_sleep: c.can_sleep,
                        enabled: c.enabled,
                        character: c.body == BodyKind::Character,
                        locked_translation: c.locked_translation,
                        locked_rotation: c.locked_rotation,
                    };
                    let collider = backend::ColliderDesc {
                        shape,
                        local_pose: if baked_static {
                            backend::Pose::default()
                        } else {
                            backend::Pose::at(Vec3::from(c.offset) * scale)
                        },
                        sensor: c.sensor,
                        friction: c.friction,
                        restitution: c.restitution,
                        density: c.density,
                        groups: backend::CollisionGroups {
                            memberships: c.groups.memberships,
                            filter: c.groups.filter,
                        },
                        enabled: c.enabled,
                        contact_force_threshold: c.contact_force_threshold,
                    };
                    let body = self.worlds.get_mut().get_mut(&client).unwrap().upsert(
                        bodykey(h),
                        &desc,
                        &[collider],
                    )?;
                    let old_force = self.entries.get(&k).and_then(|e| e.forces.clone());
                    self.entries.insert(
                        k,
                        Entry {
                            entity: h,
                            body,
                            spec: c.clone(),
                            geometry: spec.geometry.clone(),
                            matrix,
                            scale,
                            linear: spec.velocity,
                            angular: spec.angular_velocity,
                            forces: old_force,
                            target: None,
                        },
                    );
                } else {
                    let old = self.entries.get_mut(&k).unwrap();
                    let backend = self.worlds.get_mut().get_mut(&client).unwrap();
                    if old.matrix != matrix {
                        backend.set_pose(old.body, global_pose, true)?;
                        old.matrix = matrix;
                    }
                    if old.linear != spec.velocity || old.angular != spec.angular_velocity {
                        backend.set_velocity(
                            old.body,
                            spec.velocity.into(),
                            Vec3::from(spec.angular_velocity.map(f32::to_radians)),
                            true,
                        )?;
                        old.linear = spec.velocity;
                        old.angular = spec.angular_velocity;
                    }
                }
                let forces = world
                    .physics
                    .clients
                    .get(&client)
                    .and_then(|p| p.forces.get(&id))
                    .cloned();
                let entry = self.entries.get_mut(&k).unwrap();
                if entry.forces != forces {
                    let w = self.worlds.get_mut().get_mut(&client).unwrap();
                    w.clear_forces(entry.body)?;
                    if let Some(f) = &forces {
                        w.force(entry.body, f.force.into(), None)?;
                        w.torque(entry.body, f.torque.into(), false)?;
                    }
                    entry.forces = forces;
                }
                let target = world
                    .physics
                    .clients
                    .get(&client)
                    .and_then(|p| p.kinematic_targets.get(&id))
                    .cloned();
                if entry.target != target {
                    if let Some(target) = &target {
                        let r = target.pose.rotation_degrees.map(f32::to_radians);
                        self.worlds
                            .get_mut()
                            .get_mut(&client)
                            .unwrap()
                            .set_kinematic_target(
                                entry.body,
                                backend::Pose {
                                    translation: target.pose.translation.into(),
                                    rotation: Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
                                },
                            )?;
                    }
                    entry.target = target;
                }
            }
        }
        for stale in self
            .entries
            .keys()
            .filter(|k| !seen.contains(k))
            .copied()
            .collect::<Vec<_>>()
        {
            let old = self.entries.remove(&stale).unwrap();
            self.worlds
                .get_mut()
                .get_mut(&stale.0)
                .unwrap()
                .remove(bodykey(old.entity))?;
        }
        for ((client, id, generation), saved) in wanted_joints {
            let k = (client, id, generation);
            let w = self
                .worlds
                .get_mut()
                .get_mut(&client)
                .context("joint world missing")?;
            if let Some(old) = self.joints.get_mut(&k) {
                if old.state != *saved {
                    w.update_joint(old.native, &jointdesc(&saved.joint))?;
                    old.state = saved.clone();
                }
            } else {
                let a = self
                    .entries
                    .get(&key(saved.a))
                    .context("joint body A missing")?
                    .body;
                let b = self
                    .entries
                    .get(&key(saved.b))
                    .context("joint body B missing")?
                    .body;
                let native = w.create_joint(a, b, &jointdesc(&saved.joint))?;
                self.joints.insert(
                    k,
                    JointEntry {
                        native,
                        state: saved.clone(),
                    },
                );
            }
        }
        self.terrain_clients = self
            .entries
            .values()
            .filter(|e| {
                matches!(e.spec.shape, ColliderShape::Mesh)
                    && matches!(e.geometry, Some(Geometry::Terrain(_)))
            })
            .map(|e| e.entity.client)
            .collect();
        self.synced_revision = Some(world.revision);
        Ok(())
    }
    fn shape(
        &mut self,
        client: u32,
        c: &ColliderSpec,
        g: Option<&Geometry>,
        matrix: Mat4,
        scale: Vec3,
        baked: bool,
        fs: &mut DiskVfs,
    ) -> Result<backend::Shape> {
        let shape = match &c.shape {
            ColliderShape::Box { half_extents } => backend::Shape::Cuboid {
                half_extents: (*half_extents).into(),
            },
            ColliderShape::Sphere { radius } => backend::Shape::Ball { radius: *radius },
            ColliderShape::Capsule {
                radius,
                half_height,
            } => backend::Shape::Capsule {
                radius: *radius,
                half_height: *half_height,
            },
            ColliderShape::ConvexHull { points } => backend::Shape::ConvexHull {
                points: Arc::new(points.iter().copied().map(Vec3::from).collect()),
            },
            ColliderShape::Mesh if !baked && matches!(g, Some(Geometry::Terrain(_))) => {
                let Some(Geometry::Terrain(t)) = g else {
                    unreachable!()
                };
                let bytes = t
                    .heightmap
                    .as_ref()
                    .map(|p| {
                        let path =
                            VfsPath::new(Mount::Asset, p).map_err(|e| anyhow::anyhow!("{e}"))?;
                        fs.read(ClientId(client), &path)
                            .map_err(|e| anyhow::anyhow!("{e}"))
                    })
                    .transpose()?;
                let (w, d, h) = crysirror_asset::terrain::heights(t, bytes.as_deref())?;
                backend::Shape::HeightField {
                    heights: Arc::new(h),
                    rows: d as usize,
                    cols: w as usize,
                    scale: Vec3::new(t.size[0], 1., t.size[1]),
                }
            }
            ColliderShape::Mesh => {
                let (v, i) = self.mesh(client, g.context("mesh collider needs geometry")?, fs)?;
                if baked {
                    backend::Shape::TriMesh {
                        vertices: Arc::new(
                            v.iter()
                                .map(|p| matrix.transform_point3(*p + Vec3::from(c.offset)))
                                .collect(),
                        ),
                        indices: i,
                    }
                } else {
                    backend::Shape::TriMesh {
                        vertices: v,
                        indices: i,
                    }
                }
            }
        };
        if baked {
            Ok(shape)
        } else {
            shape.scaled(scale)
        }
    }
    fn mesh(&mut self, client: u32, g: &Geometry, fs: &mut DiskVfs) -> Result<MeshData> {
        let (asset, index) = match g {
            Geometry::Model { asset, mesh } => (asset.clone(), Some(*mesh as usize)),
            Geometry::Scene { asset } => (asset.clone(), None),
            Geometry::Terrain(t) => {
                let bytes = t
                    .heightmap
                    .as_ref()
                    .map(|p| {
                        let path =
                            VfsPath::new(Mount::Asset, p).map_err(|e| anyhow::anyhow!("{e}"))?;
                        fs.read(ClientId(client), &path)
                            .map_err(|e| anyhow::anyhow!("{e}"))
                    })
                    .transpose()?;
                let heights = crysirror_asset::terrain::heights(t, bytes.as_deref())?;
                let (mesh, _, _) = crysirror_asset::terrain::mesh(t, &heights);
                return Ok((
                    Arc::new(
                        mesh.vertices
                            .iter()
                            .map(|p| Vec3::from(p.position))
                            .collect(),
                    ),
                    Arc::new(
                        mesh.indices
                            .chunks_exact(3)
                            .map(|t| [t[0], t[1], t[2]])
                            .collect(),
                    ),
                ));
            }
            other => {
                let (v, i) = geometry_data(other);
                return Ok((
                    Arc::new(v.iter().copied().map(Vec3::from).collect()),
                    Arc::new(i.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect()),
                ));
            }
        };
        let cache = format!("{client}:{asset}:{index:?}");
        if let Some(mesh) = self.meshes.get(&cache) {
            return Ok(mesh.clone());
        }
        let read = |fs: &mut DiskVfs, path: &str| -> Result<Vec<u8>> {
            let p = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
            fs.read(ClientId(client), &p)
                .map_err(|e| anyhow::anyhow!("{e}"))
        };
        let bytes = read(fs, &asset)?;
        let directory = asset
            .rsplit_once('/')
            .map(|(p, _)| format!("{p}/"))
            .unwrap_or_default();
        let model = cpu::load_geometry(&bytes, |uri| read(fs, &format!("{directory}{uri}")))?;
        let (mut vertices, mut triangles) = (Vec::new(), Vec::new());
        let mut add = |prims: &[cpu::PbrPrimitive], m: Mat4| {
            for p in prims {
                let base = vertices.len() as u32;
                vertices.extend(
                    p.vertices
                        .iter()
                        .map(|p| m.transform_point3(Vec3::from(p.position))),
                );
                triangles.extend(
                    p.indices
                        .chunks_exact(3)
                        .map(|t| [base + t[0], base + t[1], base + t[2]]),
                );
            }
        };
        if let Some(index) = index {
            add(
                model.meshes.get(index).context("collider mesh index")?,
                Mat4::IDENTITY,
            );
        } else {
            let global = crysirror_asset::anim::pose(&model, &[]);
            for (ni, n) in model.nodes.iter().enumerate() {
                if let Some(mi) = n.mesh {
                    add(&model.meshes[mi], global[ni]);
                }
            }
        }
        ensure!(
            !triangles.is_empty() && triangles.len() <= 4_000_000,
            "collider triangle count"
        );
        let mesh = (Arc::new(vertices), Arc::new(triangles));
        self.meshes.insert(cache, mesh.clone());
        Ok(mesh)
    }
    pub fn step(&mut self, world: &mut World, fs: &mut DiskVfs, dt: f32) -> Result<()> {
        self.sync(world, fs)?;
        for (&client, backend) in self.worlds.get_mut() {
            if !world.client_paused(client) {
                let gravity = world
                    .physics
                    .clients
                    .get(&client)
                    .map(|p| p.gravity)
                    .unwrap_or([0., -9.81, 0.]);
                backend.configure(gravity.into(), dt)?;
                backend.step_fixed(dt)?;
            }
        }
        self.write_back(world, false)?;
        let paused = world
            .client_ids()
            .filter(|c| world.client_paused(*c))
            .collect::<HashSet<_>>();
        for (&client, settings) in &mut world.physics.clients {
            if !paused.contains(&client) {
                settings.kinematic_targets.clear();
            }
        }
        for entry in self.entries.values_mut() {
            if !paused.contains(&entry.entity.client) {
                entry.target = None;
            }
        }
        self.synced_revision = Some(world.revision);
        Ok(())
    }
    fn write_back(&mut self, world: &mut World, include_paused: bool) -> Result<()> {
        let mut states = BTreeMap::new();
        for (k, e) in &self.entries {
            if e.spec.body == BodyKind::Dynamic
                || e.spec.body == BodyKind::Kinematic
                || e.spec.body == BodyKind::KinematicPosition
            {
                if include_paused || !world.client_paused(k.0) {
                    states.insert(*k, self.worlds.get_mut().get(&k.0).unwrap().state(e.body)?);
                }
            }
        }
        let mut writes = Vec::new();
        for (k, s) in &states {
            let h = self.entries[k].entity;
            let e = world.entity(h)?;
            let mut t = e.transform.clone();
            let world_matrix = Mat4::from_rotation_translation(s.pose.rotation, s.pose.translation);
            let local = if let Some(parent) = e.parent {
                let p = world.handle(ClientId(h.client), parent)?;
                let parent_matrix = if let Some(state) = states.get(&key(p)) {
                    let scale = world.global_matrix(p)?.to_scale_rotation_translation().0;
                    Mat4::from_scale_rotation_translation(
                        scale,
                        state.pose.rotation,
                        state.pose.translation,
                    )
                } else {
                    world.global_matrix(p)?
                };
                parent_matrix.inverse() * world_matrix
            } else {
                world_matrix
            };
            let (_, rotation, position) = local.to_scale_rotation_translation();
            let (rx, ry, rz) = rotation.to_euler(EulerRot::XYZ);
            t.translation = position.to_array();
            t.rotation_degrees = [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()];
            writes.push((
                h,
                t,
                s.linear_velocity.to_array(),
                s.angular_velocity.to_array().map(f32::to_degrees),
            ));
        }
        world.write_physics(&writes)?;
        for (h, _, linear, angular) in writes {
            let e = self.entries.get_mut(&key(h)).unwrap();
            e.matrix = world.global_matrix(h)?;
            e.linear = linear;
            e.angular = angular;
        }
        Ok(())
    }
    fn entry(&self, h: EntityHandle) -> Result<&Entry> {
        self.entries
            .get(&key(h))
            .context("stale/missing physics body")
    }
    pub fn body_state(&self, h: EntityHandle) -> Result<backend::BodyState> {
        let e = self.entry(h)?;
        self.worlds.borrow().get(&h.client).unwrap().state(e.body)
    }
    pub fn query_options(
        &self,
        client: u32,
        ignore: Option<EntityHandle>,
        groups: Option<backend::CollisionGroups>,
        static_only: bool,
        sensors: bool,
    ) -> Result<backend::QueryOptions> {
        ensure!(
            ignore.is_none_or(|h| h.client == client),
            "query ignore body belongs to another client"
        );
        let ignore = ignore.map(|h| self.entry(h).map(|e| e.body)).transpose()?;
        Ok(backend::QueryOptions {
            ignore,
            static_only,
            exclude_characters: static_only,
            include_sensors: sensors,
            groups,
        })
    }
    pub fn query_raycast(
        &self,
        client: u32,
        origin: Vec3,
        dir: Vec3,
        max: f32,
        options: backend::QueryOptions,
    ) -> Result<Option<backend::QueryHit>> {
        ensure!(
            origin.is_finite()
                && dir.is_finite()
                && dir.length_squared().is_finite()
                && dir.length_squared() > 1e-12
                && max.is_finite()
                && max > 0.,
            "invalid physics ray"
        );
        let mut worlds = self.worlds.borrow_mut();
        let Some(w) = worlds.get_mut(&client) else {
            return Ok(None);
        };
        let direction = dir.normalize();
        let mut hit = w.raycast(origin, direction, max, options)?;
        if self.terrain_clients.contains(&client)
            && direction.x.abs() < 1e-6
            && direction.z.abs() < 1e-6
        {
            if let Some(fallback) = w.raycast(origin, robust_dir(direction), max, options)? {
                let terrain = self
                    .entries
                    .get(&(client, fallback.key.id, fallback.key.generation))
                    .is_some_and(|e| matches!(e.geometry, Some(Geometry::Terrain(_))));
                if terrain && hit.is_none_or(|exact| fallback.distance < exact.distance) {
                    hit = Some(fallback);
                }
            }
        }
        Ok(hit)
    }
    pub fn query_cast(
        &self,
        client: u32,
        shape: &backend::Shape,
        position: backend::Pose,
        dir: Vec3,
        max: f32,
        options: backend::QueryOptions,
    ) -> Result<Option<backend::QueryHit>> {
        ensure!(
            dir.is_finite()
                && dir.length_squared().is_finite()
                && dir.length_squared() > 1e-12
                && max.is_finite()
                && max > 0.,
            "invalid physics cast direction/distance"
        );
        let mut worlds = self.worlds.borrow_mut();
        let Some(w) = worlds.get_mut(&client) else {
            return Ok(None);
        };
        w.cast_shape(shape, position, dir.normalize_or_zero(), max, options)
    }
    pub fn query_overlaps(
        &self,
        client: u32,
        shape: &backend::Shape,
        position: backend::Pose,
        options: backend::QueryOptions,
    ) -> Result<Vec<backend::QueryHit>> {
        let mut worlds = self.worlds.borrow_mut();
        let Some(w) = worlds.get_mut(&client) else {
            return Ok(Vec::new());
        };
        w.overlaps(shape, position, options)
    }
    fn velocity_change(
        &mut self,
        h: EntityHandle,
        world: &mut World,
        change: impl FnOnce(&mut backend::PhysicsWorld, backend::BodyHandle) -> Result<()>,
    ) -> Result<()> {
        let body = self.entry(h)?.body;
        let before = self.body_state(h)?;
        change(self.worlds.get_mut().get_mut(&h.client).unwrap(), body)?;
        // A paused simulation still accepts explicit impulses. Persist their
        // resulting velocities immediately so saves cannot lose those edits.
        if let Err(error) = self.write_back(world, true) {
            let w = self.worlds.get_mut().get_mut(&h.client).unwrap();
            w.set_pose(body, before.pose, false)?;
            w.set_velocity(body, before.linear_velocity, before.angular_velocity, false)?;
            if before.sleeping {
                w.sleep(body)?;
            }
            self.synced_revision = None;
            return Err(error);
        }
        Ok(())
    }
    pub fn impulse(
        &mut self,
        h: EntityHandle,
        value: Vec3,
        point: Option<Vec3>,
        world: &mut World,
    ) -> Result<()> {
        self.velocity_change(h, world, |w, body| w.impulse(body, value, point))
    }
    pub fn torque_impulse(
        &mut self,
        h: EntityHandle,
        value: Vec3,
        world: &mut World,
    ) -> Result<()> {
        self.velocity_change(h, world, |w, body| w.torque(body, value, true))
    }
    pub fn sleep(&self, h: EntityHandle, sleep: bool) -> Result<()> {
        let body = self.entry(h)?.body;
        let mut worlds = self.worlds.borrow_mut();
        let w = worlds.get_mut(&h.client).unwrap();
        if sleep {
            w.sleep(body)
        } else {
            w.wake(body)
        }
    }
    pub fn kinematic_target(&self, h: EntityHandle, target: backend::Pose) -> Result<()> {
        let body = self.entry(h)?.body;
        self.worlds
            .borrow_mut()
            .get_mut(&h.client)
            .unwrap()
            .set_kinematic_target(body, target)
    }
    pub fn stats(&self, client: u32) -> backend::PhysicsStats {
        self.worlds
            .borrow()
            .get(&client)
            .map(|w| w.stats())
            .unwrap_or_default()
    }
    pub fn collider_count(&self) -> usize {
        self.entries.len()
    }
    pub fn drain_events(&self, client: u32) -> Vec<PhysicsEvent> {
        let mut worlds = self.worlds.borrow_mut();
        let Some(w) = worlds.get_mut(&client) else {
            return Vec::new();
        };
        w.drain_events()
            .into_iter()
            .map(|e| PhysicsEvent {
                kind: e.kind,
                entity: EntityHandle {
                    client,
                    id: e.key.id,
                    generation: e.key.generation,
                },
                other: e.other_key.map(|k| EntityHandle {
                    client,
                    id: k.id,
                    generation: k.generation,
                }),
                force: e.force.to_array(),
                force_magnitude: e.force_magnitude,
            })
            .collect()
    }
    fn options(
        &self,
        client: u32,
        ignore: Option<(u32, u32)>,
        static_only: bool,
    ) -> Option<backend::QueryOptions> {
        if ignore.is_some_and(|k| k.0 != client) {
            return None;
        }
        let skip = ignore
            .and_then(|k| {
                self.entries
                    .values()
                    .find(|e| e.entity.client == k.0 && e.entity.id == k.1)
            })
            .map(|e| e.body);
        Some(backend::QueryOptions {
            ignore: skip,
            static_only,
            exclude_characters: static_only,
            ..Default::default()
        })
    }
    pub fn raycast(
        &self,
        origin: Vec3,
        dir: Vec3,
        max: f32,
        ignore: Option<(u32, u32)>,
    ) -> Option<RayHit> {
        self.raycast_in(self.query_client, origin, dir, max, ignore, false)
    }
    pub fn raycast_static(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        self.raycast_in(self.query_client, origin, dir, max, None, true)
    }
    pub fn raycast_in(
        &self,
        client: u32,
        origin: Vec3,
        dir: Vec3,
        max: f32,
        ignore: Option<(u32, u32)>,
        static_only: bool,
    ) -> Option<RayHit> {
        let opts = self.options(client, ignore, static_only)?;
        let hit = self.query_raycast(client, origin, dir, max, opts).ok()??;
        Some(RayHit {
            entity: (client, hit.key.id),
            point: hit.point,
            normal: hit.normal,
            distance: hit.distance,
        })
    }
    pub fn sphere_cast(
        &self,
        origin: Vec3,
        dir: Vec3,
        radius: f32,
        max: f32,
        ignore: Option<(u32, u32)>,
    ) -> Option<RayHit> {
        if !origin.is_finite()
            || !dir.is_finite()
            || dir.length_squared() <= 1e-12
            || !max.is_finite()
            || max <= 0.
        {
            return None;
        }
        let client = self.query_client;
        let opts = self.options(client, ignore, false)?;
        let d = dir.normalize_or_zero();
        let hit = self
            .worlds
            .borrow_mut()
            .get_mut(&client)?
            .sphere_cast(origin, d, radius, max, opts)
            .ok()??;
        Some(RayHit {
            entity: (client, hit.key.id),
            point: origin + d * hit.distance,
            normal: hit.normal,
            distance: hit.distance,
        })
    }
    pub fn move_character(
        &self,
        k: (u32, u32),
        desired: Vec3,
        dt: f32,
        step: f32,
        max_slope_deg: f32,
    ) -> Result<CharacterMove> {
        let e = self
            .entries
            .values()
            .find(|e| e.entity.client == k.0 && e.entity.id == k.1)
            .context("entity has no collider")?;
        ensure!(
            e.spec.body == BodyKind::Character,
            "moveCharacter requires a character collider"
        );
        let mut worlds = self.worlds.borrow_mut();
        let w = worlds.get_mut(&k.0).unwrap();
        let gravity = self
            .gravities
            .get(&k.0)
            .copied()
            .unwrap_or(Vec3::new(0., -9.81, 0.));
        w.configure(gravity, dt)?;
        let m = w.move_character(
            e.body,
            desired,
            backend::CharacterConfig {
                step_height: step,
                snap_to_ground: step.max(0.05),
                max_slope_radians: max_slope_deg.to_radians(),
                ..Default::default()
            },
        )?;
        Ok(CharacterMove {
            translation: m.translation,
            grounded: m.grounded,
        })
    }
}
