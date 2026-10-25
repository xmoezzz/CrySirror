// SPDX-License-Identifier: MPL-2.0
use crate::Shape;
use crate::{shapes::*, types::*};
use anyhow::{ensure, Result};
use glam::Vec3;
use rapier3d::prelude::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
static NEXT_WORLD: AtomicU64 = AtomicU64::new(1);
pub(crate) struct Entry {
    pub key: BodyKey,
    pub desc: BodyDesc,
    pub colliders: Vec<ColliderHandle>,
    pub collider_desc: Vec<ColliderDesc>,
    pub sleeping: bool,
}
#[derive(Clone, Copy)]
enum RawEvent {
    Collision(CollisionEvent),
    Force(ContactForceEvent),
}
struct Collector {
    raw: Mutex<Vec<RawEvent>>,
    limit: usize,
    dropped: Mutex<u64>,
}
impl EventHandler for Collector {
    fn handle_collision_event(
        &self,
        _: &RigidBodySet,
        _: &ColliderSet,
        event: CollisionEvent,
        _: Option<&ContactPair>,
    ) {
        self.push(RawEvent::Collision(event));
    }
    fn handle_contact_force_event(
        &self,
        dt: f32,
        _: &RigidBodySet,
        _: &ColliderSet,
        pair: &ContactPair,
        magnitude: f32,
    ) {
        self.push(RawEvent::Force(ContactForceEvent::from_contact_pair(
            dt, pair, magnitude,
        )));
    }
}
impl Collector {
    fn push(&self, event: RawEvent) {
        let mut raw = self.raw.lock().unwrap();
        if raw.len() < self.limit {
            raw.push(event);
        } else {
            *self.dropped.lock().unwrap() += 1;
        }
    }
}
pub struct PhysicsWorld {
    pub(crate) client: u32,
    pub(crate) nonce: u64,
    pub(crate) bodies: RigidBodySet,
    pub(crate) colliders: ColliderSet,
    pub(crate) queries: QueryPipeline,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: BroadPhaseMultiSap,
    narrow: NarrowPhase,
    ccd: CCDSolver,
    pub(crate) impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    pub(crate) entries: HashMap<RigidBodyHandle, Entry>,
    dynamic: HashSet<RigidBodyHandle>,
    kinematic: HashSet<RigidBodyHandle>,
    keys: HashMap<BodyKey, RigidBodyHandle>,
    ids: HashMap<u32, BodyKey>,
    pub(crate) collider_owner: HashMap<ColliderHandle, (RigidBodyHandle, u32)>,
    pub(crate) joint_bodies: HashMap<ImpulseJointHandle, (RigidBodyHandle, RigidBodyHandle)>,
    shapes: ShapeCache,
    config: WorldConfig,
    integration: IntegrationParameters,
    accumulator: f64,
    pub(crate) pending_changes: bool,
    dirty: HashSet<ColliderHandle>,
    removed: Vec<ColliderHandle>,
    collector: Collector,
    events: VecDeque<PhysicsEvent>,
    pairs: HashMap<(ColliderHandle, ColliderHandle), bool>,
    stats: PhysicsStats,
}
pub(crate) fn groups(g: CollisionGroups) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_retain(g.memberships),
        Group::from_bits_retain(g.filter),
    )
}
fn native_kind(kind: BodyKind) -> RigidBodyType {
    match kind {
        BodyKind::Fixed => RigidBodyType::Fixed,
        BodyKind::Dynamic => RigidBodyType::Dynamic,
        BodyKind::KinematicPosition => RigidBodyType::KinematicPositionBased,
        BodyKind::KinematicVelocity => RigidBodyType::KinematicVelocityBased,
    }
}
fn locked(desc: &BodyDesc) -> LockedAxes {
    let mut axes = LockedAxes::empty();
    for (on, flag) in desc.locked_translation.into_iter().zip([
        LockedAxes::TRANSLATION_LOCKED_X,
        LockedAxes::TRANSLATION_LOCKED_Y,
        LockedAxes::TRANSLATION_LOCKED_Z,
    ]) {
        if on {
            axes |= flag;
        }
    }
    for (on, flag) in desc.locked_rotation.into_iter().zip([
        LockedAxes::ROTATION_LOCKED_X,
        LockedAxes::ROTATION_LOCKED_Y,
        LockedAxes::ROTATION_LOCKED_Z,
    ]) {
        if on {
            axes |= flag;
        }
    }
    axes
}
fn validate_body(desc: &BodyDesc, colliders: &[ColliderDesc]) -> Result<()> {
    validate_pose(desc.pose)?;
    ensure!(
        desc.linear_velocity.is_finite()
            && desc.angular_velocity.is_finite()
            && desc.gravity_scale.is_finite()
            && desc.linear_damping.is_finite()
            && desc.linear_damping >= 0.
            && desc.angular_damping.is_finite()
            && desc.angular_damping >= 0.,
        "invalid rigid-body parameters"
    );
    if let Some(m) = desc.mass {
        ensure!(m.is_finite() && m > 0., "mass must be positive");
    }
    ensure!(
        !colliders.is_empty() && colliders.len() <= 256,
        "body collider count outside 1..256"
    );
    for c in colliders {
        validate_pose(c.local_pose)?;
        c.shape.validate()?;
        ensure!(
            c.friction.is_finite()
                && c.friction >= 0.
                && c.restitution.is_finite()
                && (0.0..=1.).contains(&c.restitution)
                && c.density.is_finite()
                && c.density >= 0.,
            "invalid collider material"
        );
        if let Some(threshold) = c.contact_force_threshold {
            ensure!(
                threshold.is_finite() && threshold >= 0.,
                "invalid force event threshold"
            );
        }
        if desc.kind == BodyKind::Dynamic {
            ensure!(
                !matches!(c.shape, Shape::TriMesh { .. } | Shape::HeightField { .. }),
                "dynamic concave mesh/heightfield requires convex decomposition"
            );
        }
    }
    Ok(())
}
impl PhysicsWorld {
    pub fn new(client: u32, config: WorldConfig) -> Result<Self> {
        ensure!(
            config.gravity.is_finite()
                && config.fixed_dt.is_finite()
                && (1e-5..=1.).contains(&config.fixed_dt)
                && (1..=1024).contains(&config.max_substeps)
                && config.max_frame_time.is_finite()
                && config.max_frame_time > 0.
                && (1..=128).contains(&config.solver_iterations)
                && (1..=32).contains(&config.ccd_substeps)
                && config.shape_cache_capacity > 0
                && config.max_bodies > 0
                && config.max_colliders > 0
                && config.max_events > 0,
            "invalid physics world config"
        );
        let integration = IntegrationParameters {
            dt: config.fixed_dt,
            num_solver_iterations: NonZeroUsize::new(config.solver_iterations as usize).unwrap(),
            max_ccd_substeps: config.ccd_substeps as usize,
            ..Default::default()
        };
        Ok(Self {
            client,
            nonce: NEXT_WORLD.fetch_add(1, Ordering::Relaxed),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            queries: QueryPipeline::new(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: BroadPhaseMultiSap::new(),
            narrow: NarrowPhase::new(),
            ccd: CCDSolver::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            entries: HashMap::new(),
            dynamic: HashSet::new(),
            kinematic: HashSet::new(),
            keys: HashMap::new(),
            ids: HashMap::new(),
            collider_owner: HashMap::new(),
            joint_bodies: HashMap::new(),
            shapes: ShapeCache::new(config.shape_cache_capacity),
            integration,
            accumulator: 0.,
            pending_changes: false,
            dirty: HashSet::new(),
            removed: Vec::new(),
            collector: Collector {
                raw: Mutex::new(Vec::new()),
                limit: config.max_events,
                dropped: Mutex::new(0),
            },
            events: VecDeque::new(),
            pairs: HashMap::new(),
            stats: PhysicsStats::default(),
            config,
        })
    }
    pub fn client(&self) -> u32 {
        self.client
    }
    pub fn key(&self, h: BodyHandle) -> Result<BodyKey> {
        Ok(self.entries[&self.native_handle(h)?].key)
    }
    pub fn fixed_dt(&self) -> f32 {
        self.integration.dt
    }
    pub(crate) fn query_shape(&mut self, shape: &Shape) -> Result<SharedShape> {
        self.shapes.get(shape)
    }
    pub(crate) fn public_handle(&self, native: RigidBodyHandle) -> BodyHandle {
        let (index, generation) = native.into_raw_parts();
        BodyHandle {
            client: self.client,
            world: self.nonce,
            index,
            generation,
        }
    }
    pub(crate) fn native_handle(&self, handle: BodyHandle) -> Result<RigidBodyHandle> {
        ensure!(
            handle.client == self.client && handle.world == self.nonce,
            "foreign physics handle"
        );
        let h = RigidBodyHandle::from_raw_parts(handle.index, handle.generation);
        ensure!(
            self.entries.contains_key(&h) && self.bodies.contains(h),
            "stale physics handle"
        );
        Ok(h)
    }
    pub fn handle(&self, key: BodyKey) -> Option<BodyHandle> {
        self.keys.get(&key).copied().map(|h| self.public_handle(h))
    }
    pub fn keys(&self) -> impl Iterator<Item = BodyKey> + '_ {
        self.keys.keys().copied()
    }
    pub fn configure(&mut self, gravity: Vec3, fixed_dt: f32) -> Result<()> {
        ensure!(
            gravity.is_finite() && fixed_dt.is_finite() && (1e-5..=1.).contains(&fixed_dt),
            "invalid physics gravity/timestep"
        );
        if gravity != self.config.gravity {
            self.pending_changes = true;
            for (_, body) in self.bodies.iter_mut() {
                if body.is_dynamic() {
                    body.wake_up(true);
                }
            }
        }
        self.config.gravity = gravity;
        self.config.fixed_dt = fixed_dt;
        self.integration.dt = fixed_dt;
        Ok(())
    }
    pub fn upsert(
        &mut self,
        key: BodyKey,
        desc: &BodyDesc,
        colliders: &[ColliderDesc],
    ) -> Result<BodyHandle> {
        validate_body(desc, colliders)?;
        if let Some(handle) = self.keys.get(&key).copied() {
            let old = &self.entries[&handle];
            if old.desc == *desc && old.collider_desc == colliders {
                return Ok(self.public_handle(handle));
            }
        }
        // All fallible shape work precedes body mutation.
        let prepared = colliders
            .iter()
            .map(|c| self.shapes.get(&c.shape))
            .collect::<Result<Vec<_>>>()?;
        let weights = prepared
            .iter()
            .zip(colliders)
            .map(|(shape, c)| shape.mass_properties(c.density).mass())
            .collect::<Vec<_>>();
        let sum: f32 = weights.iter().sum();
        if desc.kind == BodyKind::Dynamic {
            ensure!(
                sum.is_finite() && sum > 0.,
                "dynamic body needs nonzero finite collider mass"
            );
        }
        let masses = desc.mass.map(|mass| {
            if sum > 0. {
                weights.iter().map(|w| w / sum * mass).collect::<Vec<_>>()
            } else {
                vec![mass / colliders.len() as f32; colliders.len()]
            }
        });
        if let Some(previous) = self.ids.get(&key.id).copied() {
            if previous != key {
                self.remove(previous)?;
            }
        }
        let existing = self.keys.get(&key).copied();
        ensure!(
            existing.is_some() || self.bodies.len() < self.config.max_bodies,
            "physics body budget exceeded"
        );
        let old_count = existing.map_or(0, |h| self.entries[&h].colliders.len());
        ensure!(
            self.colliders.len() - old_count + colliders.len() <= self.config.max_colliders,
            "physics collider budget exceeded"
        );
        let handle = if let Some(h) = existing {
            h
        } else {
            let body = RigidBodyBuilder::new(native_kind(desc.kind))
                .position(iso(desc.pose))
                .linvel(vector(desc.linear_velocity))
                .angvel(vector(desc.angular_velocity))
                .gravity_scale(desc.gravity_scale)
                .linear_damping(desc.linear_damping)
                .angular_damping(desc.angular_damping)
                .ccd_enabled(desc.ccd)
                .can_sleep(desc.can_sleep)
                .enabled(desc.enabled)
                .locked_axes(locked(desc))
                .build();
            let h = self.bodies.insert(body);
            self.entries.insert(
                h,
                Entry {
                    key,
                    desc: desc.clone(),
                    colliders: Vec::new(),
                    collider_desc: Vec::new(),
                    sleeping: false,
                },
            );
            self.keys.insert(key, h);
            self.ids.insert(key.id, key);
            self.stats.bodies_created += 1;
            h
        };
        self.pending_changes = true;
        let old = self.entries[&handle].desc.clone();
        if existing.is_some() {
            let body = self.bodies.get_mut(handle).unwrap();
            let mut changed = false;
            if old.kind != desc.kind {
                body.set_body_type(native_kind(desc.kind), true);
                changed = true;
            }
            if old.pose != desc.pose {
                let target = iso(desc.pose);
                if (body.position().translation.vector - target.translation.vector).norm_squared()
                    > 1e-12
                    || body.rotation().angle_to(&target.rotation) > 1e-6
                {
                    body.set_position(target, true);
                    changed = true;
                }
            }
            if old.linear_velocity != desc.linear_velocity
                && vec3(*body.linvel()) != desc.linear_velocity
            {
                body.set_linvel(vector(desc.linear_velocity), true);
                changed = true;
            }
            if old.angular_velocity != desc.angular_velocity
                && vec3(*body.angvel()) != desc.angular_velocity
            {
                body.set_angvel(vector(desc.angular_velocity), true);
                changed = true;
            }
            if old.gravity_scale != desc.gravity_scale {
                body.set_gravity_scale(desc.gravity_scale, true);
                changed = true;
            }
            if old.linear_damping != desc.linear_damping {
                body.set_linear_damping(desc.linear_damping);
                changed = true;
            }
            if old.angular_damping != desc.angular_damping {
                body.set_angular_damping(desc.angular_damping);
                changed = true;
            }
            if old.ccd != desc.ccd {
                body.enable_ccd(desc.ccd);
                changed = true;
            }
            if old.enabled != desc.enabled {
                body.set_enabled(desc.enabled);
                changed = true;
            }
            if old.locked_translation != desc.locked_translation
                || old.locked_rotation != desc.locked_rotation
            {
                body.set_locked_axes(locked(desc), true);
                changed = true;
            }
            if old.can_sleep != desc.can_sleep {
                let a = body.activation_mut();
                a.normalized_linear_threshold = if desc.can_sleep {
                    RigidBodyActivation::default_normalized_linear_threshold()
                } else {
                    -1.
                };
                a.angular_threshold = if desc.can_sleep {
                    RigidBodyActivation::default_angular_threshold()
                } else {
                    -1.
                };
                body.wake_up(true);
                changed = true;
            }
            if changed {
                self.stats.body_updates += 1;
                self.mark_body_dirty(handle);
            }
        }
        let old_colliders = self.entries[&handle].colliders.clone();
        if old_colliders.len() != colliders.len() {
            for h in old_colliders {
                self.remove_collider(h);
            }
            let mut created = Vec::new();
            for (i, (c, shape)) in colliders.iter().zip(prepared).enumerate() {
                let mut builder = ColliderBuilder::new(shape)
                    .position(iso(c.local_pose))
                    .sensor(c.sensor)
                    .active_collision_types(if c.sensor {
                        ActiveCollisionTypes::all()
                    } else {
                        ActiveCollisionTypes::default()
                    })
                    .friction(c.friction)
                    .restitution(c.restitution)
                    .density(c.density)
                    .enabled(c.enabled)
                    .collision_groups(groups(c.groups))
                    .active_events(
                        ActiveEvents::COLLISION_EVENTS
                            | if c.contact_force_threshold.is_some() {
                                ActiveEvents::CONTACT_FORCE_EVENTS
                            } else {
                                ActiveEvents::empty()
                            },
                    );
                if let Some(masses) = &masses {
                    builder = builder.mass(masses[i]);
                }
                if let Some(threshold) = c.contact_force_threshold {
                    builder = builder.contact_force_event_threshold(threshold);
                }
                let h = self
                    .colliders
                    .insert_with_parent(builder, handle, &mut self.bodies);
                self.collider_owner.insert(h, (handle, i as u32));
                self.dirty.insert(h);
                created.push(h);
                self.stats.colliders_created += 1;
            }
            self.entries.get_mut(&handle).unwrap().colliders = created;
        } else {
            for (i, ((h, c), shape)) in old_colliders
                .iter()
                .zip(colliders)
                .zip(prepared)
                .enumerate()
            {
                let old = &self.entries[&handle].collider_desc[i];
                if old == c && self.entries[&handle].desc.mass == desc.mass && masses.is_none() {
                    continue;
                }
                let collider = self.colliders.get_mut(*h).unwrap();
                if old.shape != c.shape {
                    collider.set_shape(shape);
                }
                collider.set_position_wrt_parent(iso(c.local_pose));
                collider.set_sensor(c.sensor);
                collider.set_active_collision_types(if c.sensor {
                    ActiveCollisionTypes::all()
                } else {
                    ActiveCollisionTypes::default()
                });
                collider.set_friction(c.friction);
                collider.set_restitution(c.restitution);
                collider.set_density(c.density);
                if let Some(masses) = &masses {
                    collider.set_mass(masses[i]);
                }
                collider.set_enabled(c.enabled);
                collider.set_collision_groups(groups(c.groups));
                collider.set_active_events(
                    ActiveEvents::COLLISION_EVENTS
                        | if c.contact_force_threshold.is_some() {
                            ActiveEvents::CONTACT_FORCE_EVENTS
                        } else {
                            ActiveEvents::empty()
                        },
                );
                collider.set_contact_force_event_threshold(
                    c.contact_force_threshold.unwrap_or(f32::MAX),
                );
                self.dirty.insert(*h);
                self.stats.collider_updates += 1;
            }
        }
        self.bodies
            .get_mut(handle)
            .unwrap()
            .recompute_mass_properties_from_colliders(&self.colliders);
        if desc.kind == BodyKind::Dynamic {
            self.dynamic.insert(handle);
        } else {
            self.dynamic.remove(&handle);
        }
        if matches!(
            desc.kind,
            BodyKind::KinematicPosition | BodyKind::KinematicVelocity
        ) {
            self.kinematic.insert(handle);
        } else {
            self.kinematic.remove(&handle);
        }
        let entry = self.entries.get_mut(&handle).unwrap();
        entry.desc = desc.clone();
        entry.collider_desc = colliders.to_vec();
        Ok(self.public_handle(handle))
    }
    pub(crate) fn mark_body_dirty(&mut self, h: RigidBodyHandle) {
        self.pending_changes = true;
        if let Some(entry) = self.entries.get(&h) {
            self.dirty.extend(entry.colliders.iter().copied());
        }
    }
    fn remove_collider(&mut self, h: ColliderHandle) {
        self.pending_changes = true;
        let affected = self
            .pairs
            .keys()
            .filter(|(a, b)| *a == h || *b == h)
            .copied()
            .collect::<Vec<_>>();
        for pair in affected {
            let sensor = self.pairs.remove(&pair).unwrap();
            self.push_pair_event(
                pair.0,
                pair.1,
                if sensor {
                    EventKind::SensorExited
                } else {
                    EventKind::ContactStopped
                },
                Vec3::ZERO,
                0.,
            );
        }
        self.colliders
            .remove(h, &mut self.islands, &mut self.bodies, true);
        self.collider_owner.remove(&h);
        self.dirty.remove(&h);
        self.removed.push(h);
    }
    pub fn remove(&mut self, key: BodyKey) -> Result<bool> {
        let Some(handle) = self.keys.remove(&key) else {
            return Ok(false);
        };
        let colliders = self.entries[&handle].colliders.clone();
        for h in colliders {
            self.remove_collider(h);
        }
        self.bodies.remove(
            handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
        self.entries.remove(&handle);
        self.dynamic.remove(&handle);
        self.kinematic.remove(&handle);
        self.ids.remove(&key.id);
        self.joint_bodies
            .retain(|_, (a, b)| *a != handle && *b != handle);
        self.stats.bodies_removed += 1;
        Ok(true)
    }
    pub fn state(&self, h: BodyHandle) -> Result<BodyState> {
        let h = self.native_handle(h)?;
        let body = &self.bodies[h];
        Ok(BodyState {
            pose: pose(body.position()),
            linear_velocity: vec3(*body.linvel()),
            angular_velocity: vec3(*body.angvel()),
            sleeping: body.is_sleeping(),
        })
    }
    pub fn set_pose(&mut self, h: BodyHandle, value: Pose, wake: bool) -> Result<()> {
        validate_pose(value)?;
        let h = self.native_handle(h)?;
        self.bodies[h].set_position(iso(value), wake);
        self.mark_body_dirty(h);
        Ok(())
    }
    pub fn set_kinematic_target(&mut self, h: BodyHandle, value: Pose) -> Result<()> {
        validate_pose(value)?;
        let h = self.native_handle(h)?;
        ensure!(
            self.bodies[h].body_type() == RigidBodyType::KinematicPositionBased,
            "kinematic target needs position-based body"
        );
        self.pending_changes = true;
        self.bodies[h].set_next_kinematic_position(iso(value));
        Ok(())
    }
    pub fn set_velocity(
        &mut self,
        h: BodyHandle,
        linear: Vec3,
        angular: Vec3,
        wake: bool,
    ) -> Result<()> {
        ensure!(
            linear.is_finite() && angular.is_finite(),
            "nonfinite physics velocity"
        );
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        self.bodies[h].set_linvel(vector(linear), wake);
        self.bodies[h].set_angvel(vector(angular), wake);
        Ok(())
    }
    pub fn impulse(&mut self, h: BodyHandle, value: Vec3, point_at: Option<Vec3>) -> Result<()> {
        ensure!(
            value.is_finite() && point_at.is_none_or(Vec3::is_finite),
            "invalid impulse"
        );
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        if let Some(p) = point_at {
            self.bodies[h].apply_impulse_at_point(vector(value), point(p), true);
        } else {
            self.bodies[h].apply_impulse(vector(value), true);
        }
        Ok(())
    }
    pub fn force(&mut self, h: BodyHandle, value: Vec3, point_at: Option<Vec3>) -> Result<()> {
        ensure!(
            value.is_finite() && point_at.is_none_or(Vec3::is_finite),
            "invalid force"
        );
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        if let Some(p) = point_at {
            self.bodies[h].add_force_at_point(vector(value), point(p), true);
        } else {
            self.bodies[h].add_force(vector(value), true);
        }
        Ok(())
    }
    pub fn torque(&mut self, h: BodyHandle, value: Vec3, impulse: bool) -> Result<()> {
        ensure!(value.is_finite(), "invalid torque");
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        if impulse {
            self.bodies[h].apply_torque_impulse(vector(value), true);
        } else {
            self.bodies[h].add_torque(vector(value), true);
        }
        Ok(())
    }
    pub fn clear_forces(&mut self, h: BodyHandle) -> Result<()> {
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        self.bodies[h].reset_forces(true);
        self.bodies[h].reset_torques(true);
        Ok(())
    }
    pub fn wake(&mut self, h: BodyHandle) -> Result<()> {
        let h = self.native_handle(h)?;
        self.pending_changes = true;
        self.bodies[h].wake_up(true);
        Ok(())
    }
    pub fn sleep(&mut self, h: BodyHandle) -> Result<()> {
        let h = self.native_handle(h)?;
        ensure!(self.entries[&h].desc.can_sleep, "body disables sleeping");
        self.pending_changes = true;
        self.bodies[h].sleep();
        Ok(())
    }
    pub fn update_queries(&mut self) {
        if self.dirty.is_empty() && self.removed.is_empty() {
            return;
        }
        self.bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
        let modified = self.dirty.drain().collect::<Vec<_>>();
        self.queries
            .update_incremental(&self.colliders, &modified, &self.removed, true);
        self.removed.clear();
        self.stats.query_updates += 1;
    }
    pub fn step_fixed(&mut self, dt: f32) -> Result<StepReport> {
        ensure!(
            dt.is_finite() && (1e-5..=1.).contains(&dt),
            "invalid fixed physics step"
        );
        self.integration.dt = dt;
        self.run_step();
        Ok(StepReport {
            steps: 1,
            simulated_seconds: dt as f64,
            dropped_seconds: 0.,
            interpolation_alpha: 0.,
        })
    }
    pub fn advance(&mut self, seconds: f64) -> Result<StepReport> {
        ensure!(
            seconds.is_finite() && seconds >= 0.,
            "invalid elapsed physics time"
        );
        let accepted = seconds.min(self.config.max_frame_time);
        let mut report = StepReport {
            dropped_seconds: seconds - accepted,
            ..Default::default()
        };
        self.accumulator += accepted;
        let dt = self.config.fixed_dt as f64;
        self.integration.dt = self.config.fixed_dt;
        while self.accumulator + 1e-12 >= dt && report.steps < self.config.max_substeps {
            self.run_step();
            self.accumulator -= dt;
            report.steps += 1;
            report.simulated_seconds += dt;
        }
        if self.accumulator >= dt {
            let whole = (self.accumulator / dt).floor() * dt;
            self.accumulator -= whole;
            report.dropped_seconds += whole;
        }
        report.interpolation_alpha = (self.accumulator / dt) as f32;
        Ok(report)
    }
    fn run_step(&mut self) {
        // Dormant worlds need neither broadphase refits nor constraint solves.
        // A newly woken body, changed kinematic target, edited/removed collider
        // or unreported activation transition always goes through Rapier.
        let dormant = self.kinematic.is_empty()
            && self.impulse_joints.is_empty()
            && self.multibody_joints.iter().next().is_none()
            && !self.pending_changes
            && self.dirty.is_empty()
            && self.removed.is_empty()
            && self.dynamic.iter().all(|h| {
                let b = &self.bodies[*h];
                (!b.is_enabled() || b.is_sleeping()) && self.entries[h].sleeping == b.is_sleeping()
            });
        if dormant {
            self.stats.simulation_steps += 1;
            self.stats.idle_steps += 1;
            return;
        }
        self.stats.pipeline_steps += 1;
        self.pipeline.step(
            &vector(self.config.gravity),
            &self.integration,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            Some(&mut self.queries),
            &(),
            &self.collector,
        );
        self.pending_changes = false;
        self.dirty.clear();
        self.removed.clear();
        self.stats.simulation_steps += 1;
        self.stats.query_updates += 1;
        let mut raw = std::mem::take(&mut *self.collector.raw.lock().unwrap());
        self.stats.events_dropped += std::mem::take(&mut *self.collector.dropped.lock().unwrap());
        for event in raw.drain(..) {
            match event {
                RawEvent::Collision(e) => {
                    let a = e.collider1();
                    let b = e.collider2();
                    let pair = if a.into_raw_parts() < b.into_raw_parts() {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    if e.started() {
                        self.pairs.insert(pair, e.sensor());
                    } else {
                        self.pairs.remove(&pair);
                    }
                    let kind = match (e.started(), e.sensor()) {
                        (true, true) => EventKind::SensorEntered,
                        (false, true) => EventKind::SensorExited,
                        (true, false) => EventKind::ContactStarted,
                        (false, false) => EventKind::ContactStopped,
                    };
                    self.push_pair_event(a, b, kind, Vec3::ZERO, 0.);
                }
                RawEvent::Force(e) => self.push_pair_event(
                    e.collider1,
                    e.collider2,
                    EventKind::ContactForce,
                    vec3(e.total_force),
                    e.total_force_magnitude,
                ),
            }
        }
        *self.collector.raw.lock().unwrap() = raw;
        let transitions = self
            .dynamic
            .iter()
            .filter_map(|h| {
                let e = self.entries.get_mut(h).unwrap();
                let sleeping = self.bodies[*h].is_sleeping();
                if sleeping != e.sleeping {
                    e.sleeping = sleeping;
                    Some((*h, sleeping))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (h, sleeping) in transitions {
            self.push_event(PhysicsEvent {
                kind: if sleeping {
                    EventKind::Sleeping
                } else {
                    EventKind::Awakened
                },
                body: self.public_handle(h),
                key: self.entries[&h].key,
                other: None,
                other_key: None,
                collider: 0,
                other_collider: 0,
                force: Vec3::ZERO,
                force_magnitude: 0.,
            });
        }
    }
    fn push_pair_event(
        &mut self,
        a: ColliderHandle,
        b: ColliderHandle,
        kind: EventKind,
        force: Vec3,
        magnitude: f32,
    ) {
        let (Some(&(a, ai)), Some(&(b, bi))) =
            (self.collider_owner.get(&a), self.collider_owner.get(&b))
        else {
            return;
        };
        self.push_event(PhysicsEvent {
            kind,
            body: self.public_handle(a),
            key: self.entries[&a].key,
            other: Some(self.public_handle(b)),
            other_key: Some(self.entries[&b].key),
            collider: ai,
            other_collider: bi,
            force,
            force_magnitude: magnitude,
        });
    }
    fn push_event(&mut self, event: PhysicsEvent) {
        if self.events.len() == self.config.max_events {
            self.events.pop_front();
            self.stats.events_dropped += 1;
        }
        self.events.push_back(event);
    }
    pub fn drain_events(&mut self) -> Vec<PhysicsEvent> {
        self.events.drain(..).collect()
    }
    pub fn stats(&self) -> PhysicsStats {
        let mut out = self.stats;
        out.bodies = self.bodies.len();
        out.colliders = self.colliders.len();
        out.joints = self.impulse_joints.len();
        out.sleeping_bodies = self
            .dynamic
            .iter()
            .filter(|h| self.bodies[**h].is_sleeping())
            .count();
        out.active_dynamic_bodies = self
            .dynamic
            .iter()
            .filter(|h| {
                let b = &self.bodies[**h];
                !b.is_sleeping() && b.is_enabled()
            })
            .count();
        out.shape_cache_entries = self.shapes.len();
        out.shape_builds = self.shapes.builds;
        out.shape_cache_hits = self.shapes.hits;
        out
    }
}
