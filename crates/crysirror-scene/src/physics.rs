// SPDX-License-Identifier: MPL-2.0
//! Saved, client-scoped physics declarations. Solver handles and caches live
//! in the runtime; snapshots retain transforms/velocities, forces and joints.
use crate::world::{EntityHandle, World};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn gravity() -> [f32; 3] {
    [0., -9.81, 0.]
}
fn axis() -> [f32; 3] {
    [1., 0., 0.]
}
fn max_force() -> f32 {
    1000.
}
fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= 1e9)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JointFrame {
    /// Position in body-local meters for a joint, or world meters for a target.
    pub translation: [f32; 3],
    /// XYZ Euler rotation in degrees, in the same frame as translation.
    pub rotation_degrees: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JointMotor {
    /// Radians for a revolute joint; meters for a prismatic joint.
    pub target_position: f32,
    /// Radians/s for a revolute joint; meters/s for a prismatic joint.
    pub target_velocity: f32,
    /// Nonnegative motor position stiffness.
    pub stiffness: f32,
    /// Nonnegative motor velocity damping.
    pub damping: f32,
    /// Maximum motor force (N), or torque (N·m) for a revolute joint.
    pub max_force: f32,
}
impl Default for JointMotor {
    fn default() -> Self {
        Self {
            target_position: 0.,
            target_velocity: 0.,
            stiffness: 0.,
            damping: 0.,
            max_force: max_force(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JointSpec {
    Fixed {
        #[serde(default)]
        local_frame1: JointFrame,
        #[serde(default)]
        local_frame2: JointFrame,
        #[serde(default)]
        contacts_enabled: bool,
    },
    Spherical {
        #[serde(default)]
        local_anchor1: [f32; 3],
        #[serde(default)]
        local_anchor2: [f32; 3],
        #[serde(default)]
        contacts_enabled: bool,
    },
    Revolute {
        #[serde(default)]
        local_anchor1: [f32; 3],
        #[serde(default)]
        local_anchor2: [f32; 3],
        #[serde(default = "axis")]
        axis1: [f32; 3],
        #[serde(default = "axis")]
        axis2: [f32; 3],
        #[serde(default)]
        limits: Option<[f32; 2]>,
        #[serde(default)]
        motor: Option<JointMotor>,
        #[serde(default)]
        contacts_enabled: bool,
    },
    Prismatic {
        #[serde(default)]
        local_anchor1: [f32; 3],
        #[serde(default)]
        local_anchor2: [f32; 3],
        #[serde(default = "axis")]
        axis1: [f32; 3],
        #[serde(default = "axis")]
        axis2: [f32; 3],
        #[serde(default)]
        limits: Option<[f32; 2]>,
        #[serde(default)]
        motor: Option<JointMotor>,
        #[serde(default)]
        contacts_enabled: bool,
    },
}
impl JointSpec {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Fixed {
                local_frame1: a,
                local_frame2: b,
                ..
            } => ensure!(
                finite(&a.translation)
                    && finite(&a.rotation_degrees)
                    && finite(&b.translation)
                    && finite(&b.rotation_degrees),
                "invalid joint frames"
            ),
            Self::Spherical {
                local_anchor1: a,
                local_anchor2: b,
                ..
            } => ensure!(finite(a) && finite(b), "invalid joint anchors"),
            Self::Revolute {
                local_anchor1: a,
                local_anchor2: b,
                axis1,
                axis2,
                limits,
                motor,
                ..
            }
            | Self::Prismatic {
                local_anchor1: a,
                local_anchor2: b,
                axis1,
                axis2,
                limits,
                motor,
                ..
            } => {
                ensure!(
                    finite(a)
                        && finite(b)
                        && finite(axis1)
                        && finite(axis2)
                        && axis1.iter().map(|v| v * v).sum::<f32>() > 1e-12
                        && axis2.iter().map(|v| v * v).sum::<f32>() > 1e-12,
                    "invalid joint anchors/axes"
                );
                ensure!(
                    limits.is_none_or(|v| finite(&v) && v[0] <= v[1]),
                    "invalid joint limits"
                );
                if let Some(m) = motor {
                    ensure!(
                        finite(&[
                            m.target_position,
                            m.target_velocity,
                            m.stiffness,
                            m.damping,
                            m.max_force
                        ]) && m.stiffness >= 0.
                            && m.damping >= 0.
                            && m.max_force >= 0.,
                        "invalid joint motor"
                    );
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointHandle {
    /// Owning registered client; joints never connect different clients.
    pub client: u32,
    /// Monotonic client-local joint identifier.
    pub id: u32,
    /// Logical generation; stale saved/script handles are rejected.
    pub generation: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JointState {
    /// Generation corresponding to the logical joint handle.
    pub generation: u32,
    /// First client-scoped entity/body and its generation.
    pub a: EntityHandle,
    /// Second distinct entity/body in the same client.
    pub b: EntityHandle,
    /// Saved constraint, anchors, limits and optional motor configuration.
    pub joint: JointSpec,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyForces {
    /// Generation of the dynamic entity receiving these forces.
    pub generation: u32,
    /// Persistent force in world-space newtons, until explicitly cleared.
    #[serde(default)]
    pub force: [f32; 3],
    /// Persistent torque in world-space N·m, until explicitly cleared.
    #[serde(default)]
    pub torque: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KinematicTarget {
    /// Generation of the position-driven kinematic entity.
    pub generation: u32,
    /// World-space pose to reach at the next fixed step; consumed afterward.
    pub pose: JointFrame,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientPhysicsScene {
    /// World-space gravity in meters/s². Default [0,-9.81,0].
    pub gravity: [f32; 3],
    /// Persistent forces keyed by this client's entity id.
    pub forces: BTreeMap<u32, BodyForces>,
    /// Saved, not-yet-consumed position-kinematic targets by entity id.
    pub kinematic_targets: BTreeMap<u32, KinematicTarget>,
    /// Saved joint declarations keyed by this client's joint id.
    pub joints: BTreeMap<u32, JointState>,
    /// Last minted joint id; ids are not reused within the saved scene.
    pub next_joint: u32,
}
impl Default for ClientPhysicsScene {
    fn default() -> Self {
        Self {
            gravity: gravity(),
            forces: BTreeMap::new(),
            kinematic_targets: BTreeMap::new(),
            joints: BTreeMap::new(),
            next_joint: 0,
        }
    }
}
impl ClientPhysicsScene {
    pub fn validate(&self, client: u32, world: &World) -> Result<()> {
        ensure!(
            finite(&self.gravity)
                && self.gravity.iter().all(|v| v.abs() <= 100_000.)
                && self.forces.len() <= 4096
                && self.joints.len() <= 4096,
            "physics scene limits"
        );
        for (&id, force) in &self.forces {
            let h = EntityHandle {
                client,
                id,
                generation: force.generation,
            };
            let entity = world.entity(h)?;
            ensure!(
                entity
                    .collider
                    .as_ref()
                    .is_some_and(|c| c.body == crate::components::BodyKind::Dynamic)
                    && finite(&force.force)
                    && finite(&force.torque),
                "forces require a live dynamic body"
            );
        }
        ensure!(
            self.kinematic_targets.len() <= 4096,
            "kinematic target limit"
        );
        for (&id, target) in &self.kinematic_targets {
            let e = world.entity(EntityHandle {
                client,
                id,
                generation: target.generation,
            })?;
            ensure!(
                e.collider
                    .as_ref()
                    .is_some_and(|c| c.body == crate::components::BodyKind::KinematicPosition)
                    && finite(&target.pose.translation)
                    && finite(&target.pose.rotation_degrees),
                "invalid position-kinematic target"
            );
        }
        for (&id, j) in &self.joints {
            ensure!(
                id > 0
                    && id <= self.next_joint
                    && j.generation > 0
                    && j.a.client == client
                    && j.b.client == client
                    && j.a != j.b,
                "invalid/scoped physics joint identity"
            );
            ensure!(
                world.entity(j.a)?.collider.is_some() && world.entity(j.b)?.collider.is_some(),
                "joint bodies need colliders"
            );
            j.joint.validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysicsScene {
    /// Per-client simulation declarations; each client has an isolated solver.
    pub clients: BTreeMap<u32, ClientPhysicsScene>,
}
impl PhysicsScene {
    pub fn validate(&self, world: &World) -> Result<()> {
        ensure!(self.clients.len() <= 64, "physics client limit");
        for (&client, spec) in &self.clients {
            spec.validate(client, world)?;
        }
        Ok(())
    }
    pub fn retain_live(&mut self, world: &World) {
        for (&client, spec) in &mut self.clients {
            spec.forces.retain(|&id, f| {
                world
                    .entity(EntityHandle {
                        client,
                        id,
                        generation: f.generation,
                    })
                    .is_ok_and(|e| {
                        e.collider
                            .as_ref()
                            .is_some_and(|c| c.body == crate::components::BodyKind::Dynamic)
                    })
            });
            spec.kinematic_targets.retain(|&id, t| {
                world
                    .entity(EntityHandle {
                        client,
                        id,
                        generation: t.generation,
                    })
                    .is_ok_and(|e| {
                        e.collider.as_ref().is_some_and(|c| {
                            c.body == crate::components::BodyKind::KinematicPosition
                        })
                    })
            });
            spec.joints.retain(|_, j| {
                world.entity(j.a).is_ok_and(|e| e.collider.is_some())
                    && world.entity(j.b).is_ok_and(|e| e.collider.is_some())
            });
        }
    }
}
