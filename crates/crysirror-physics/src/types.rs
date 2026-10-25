// SPDX-License-Identifier: MPL-2.0
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BodyKey {
    pub id: u32,
    pub generation: u32,
}
/// Not a Rapier pointer; valid only in the world/client which minted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyHandle {
    pub(crate) client: u32,
    pub(crate) world: u64,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JointHandle {
    pub(crate) client: u32,
    pub(crate) world: u64,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub translation: Vec3,
    pub rotation: Quat,
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    }
}
impl Pose {
    pub fn at(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::default()
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyKind {
    #[default]
    Fixed,
    Dynamic,
    KinematicPosition,
    KinematicVelocity,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub pose: Pose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    /// Total body mass, distributed across the colliders proportionally to
    /// their configured density/volume. None uses those densities directly.
    pub mass: Option<f32>,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub ccd: bool,
    pub can_sleep: bool,
    pub enabled: bool,
    pub character: bool,
    pub locked_translation: [bool; 3],
    pub locked_rotation: [bool; 3],
}
impl Default for BodyDesc {
    fn default() -> Self {
        Self {
            kind: BodyKind::Fixed,
            pose: Pose::default(),
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            mass: None,
            gravity_scale: 1.,
            linear_damping: 0.,
            angular_damping: 0.,
            ccd: false,
            can_sleep: true,
            enabled: true,
            character: false,
            locked_translation: [false; 3],
            locked_rotation: [false; 3],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Cuboid {
        half_extents: Vec3,
    },
    Ball {
        radius: f32,
    },
    Capsule {
        radius: f32,
        half_height: f32,
    },
    TriMesh {
        vertices: Arc<Vec<Vec3>>,
        indices: Arc<Vec<[u32; 3]>>,
    },
    ConvexHull {
        points: Arc<Vec<Vec3>>,
    },
    /// Row-major heights. Rapier's matrix indexing is supplied explicitly by
    /// the backend; the grid spans x/z and height values are y coordinates.
    HeightField {
        heights: Arc<Vec<f32>>,
        rows: usize,
        cols: usize,
        scale: Vec3,
    },
}
impl Default for Shape {
    fn default() -> Self {
        Self::Cuboid {
            half_extents: Vec3::splat(0.5),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollisionGroups {
    pub memberships: u32,
    pub filter: u32,
}
impl Default for CollisionGroups {
    fn default() -> Self {
        Self {
            memberships: u32::MAX,
            filter: u32::MAX,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColliderDesc {
    pub shape: Shape,
    pub local_pose: Pose,
    pub sensor: bool,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub groups: CollisionGroups,
    pub enabled: bool,
    pub contact_force_threshold: Option<f32>,
}
impl Default for ColliderDesc {
    fn default() -> Self {
        Self {
            shape: Shape::default(),
            local_pose: Pose::default(),
            sensor: false,
            friction: 0.5,
            restitution: 0.,
            density: 1.,
            groups: CollisionGroups::default(),
            enabled: true,
            contact_force_threshold: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyState {
    pub pose: Pose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub sleeping: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldConfig {
    pub gravity: Vec3,
    pub fixed_dt: f32,
    pub max_substeps: u32,
    pub max_frame_time: f64,
    pub solver_iterations: u32,
    pub ccd_substeps: u32,
    pub shape_cache_capacity: usize,
    pub max_bodies: usize,
    pub max_colliders: usize,
    pub max_events: usize,
}
impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0., -9.81, 0.),
            fixed_dt: 1. / 60.,
            max_substeps: 8,
            max_frame_time: 0.25,
            solver_iterations: 8,
            ccd_substeps: 4,
            shape_cache_capacity: 256,
            max_bodies: 65_536,
            max_colliders: 262_144,
            max_events: 16_384,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct StepReport {
    pub steps: u32,
    pub simulated_seconds: f64,
    pub dropped_seconds: f64,
    pub interpolation_alpha: f32,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct PhysicsStats {
    pub bodies: usize,
    pub colliders: usize,
    pub joints: usize,
    pub sleeping_bodies: usize,
    pub active_dynamic_bodies: usize,
    pub shape_cache_entries: usize,
    pub shape_builds: u64,
    pub shape_cache_hits: u64,
    pub bodies_created: u64,
    pub bodies_removed: u64,
    pub body_updates: u64,
    pub colliders_created: u64,
    pub collider_updates: u64,
    pub query_updates: u64,
    pub simulation_steps: u64,
    pub pipeline_steps: u64,
    pub idle_steps: u64,
    pub events_dropped: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct QueryOptions {
    pub ignore: Option<BodyHandle>,
    pub static_only: bool,
    pub exclude_characters: bool,
    pub include_sensors: bool,
    pub groups: Option<CollisionGroups>,
}
impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            ignore: None,
            static_only: false,
            exclude_characters: false,
            include_sensors: false,
            groups: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct QueryHit {
    pub body: BodyHandle,
    pub key: BodyKey,
    pub collider: u32,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    ContactStarted,
    ContactStopped,
    SensorEntered,
    SensorExited,
    ContactForce,
    Sleeping,
    Awakened,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PhysicsEvent {
    pub kind: EventKind,
    pub body: BodyHandle,
    pub key: BodyKey,
    pub other: Option<BodyHandle>,
    pub other_key: Option<BodyKey>,
    pub collider: u32,
    pub other_collider: u32,
    pub force: Vec3,
    pub force_magnitude: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct CharacterConfig {
    pub offset: f32,
    pub step_height: f32,
    pub min_step_width: f32,
    pub snap_to_ground: f32,
    pub max_slope_radians: f32,
    pub slide: bool,
    pub push_dynamic: bool,
    pub mass: f32,
}
impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            offset: 0.02,
            step_height: 0.3,
            min_step_width: 0.1,
            snap_to_ground: 0.3,
            max_slope_radians: 45f32.to_radians(),
            slide: true,
            push_dynamic: false,
            mass: 80.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct CharacterMove {
    pub translation: Vec3,
    pub grounded: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motor {
    pub target_position: f32,
    pub target_velocity: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub max_force: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum JointDesc {
    Fixed {
        local_frame1: Pose,
        local_frame2: Pose,
        contacts_enabled: bool,
    },
    Revolute {
        local_anchor1: Vec3,
        local_anchor2: Vec3,
        axis1: Vec3,
        axis2: Vec3,
        limits: Option<[f32; 2]>,
        motor: Option<Motor>,
        contacts_enabled: bool,
    },
    Prismatic {
        local_anchor1: Vec3,
        local_anchor2: Vec3,
        axis1: Vec3,
        axis2: Vec3,
        limits: Option<[f32; 2]>,
        motor: Option<Motor>,
        contacts_enabled: bool,
    },
    Spherical {
        local_anchor1: Vec3,
        local_anchor2: Vec3,
        contacts_enabled: bool,
    },
}
