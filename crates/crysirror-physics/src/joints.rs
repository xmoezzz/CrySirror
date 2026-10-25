// SPDX-License-Identifier: MPL-2.0
use crate::{shapes::*, BodyHandle, JointDesc, JointHandle, Motor, PhysicsWorld};
use anyhow::{ensure, Result};
use rapier3d::{na::Unit, prelude::*};
fn motor(data: &mut GenericJoint, axis: JointAxis, m: Option<Motor>) -> Result<()> {
    if let Some(m) = m {
        ensure!(
            [
                m.target_position,
                m.target_velocity,
                m.stiffness,
                m.damping,
                m.max_force
            ]
            .iter()
            .all(|v| v.is_finite())
                && m.stiffness >= 0.
                && m.damping >= 0.
                && m.max_force >= 0.,
            "invalid joint motor"
        );
        data.set_motor(
            axis,
            m.target_position,
            m.target_velocity,
            m.stiffness,
            m.damping,
        );
        data.set_motor_max_force(axis, m.max_force);
    }
    Ok(())
}
fn joint(desc: &JointDesc) -> Result<GenericJoint> {
    let data = match desc {
        JointDesc::Fixed {
            local_frame1,
            local_frame2,
            contacts_enabled,
        } => {
            validate_pose(*local_frame1)?;
            validate_pose(*local_frame2)?;
            FixedJointBuilder::new()
                .local_frame1(iso(*local_frame1))
                .local_frame2(iso(*local_frame2))
                .contacts_enabled(*contacts_enabled)
                .build()
                .data
        }
        JointDesc::Spherical {
            local_anchor1,
            local_anchor2,
            contacts_enabled,
        } => {
            ensure!(
                local_anchor1.is_finite() && local_anchor2.is_finite(),
                "invalid joint anchor"
            );
            SphericalJointBuilder::new()
                .local_anchor1(point(*local_anchor1))
                .local_anchor2(point(*local_anchor2))
                .contacts_enabled(*contacts_enabled)
                .build()
                .data
        }
        JointDesc::Revolute {
            local_anchor1,
            local_anchor2,
            axis1,
            axis2,
            limits,
            motor: m,
            contacts_enabled,
        }
        | JointDesc::Prismatic {
            local_anchor1,
            local_anchor2,
            axis1,
            axis2,
            limits,
            motor: m,
            contacts_enabled,
        } => {
            ensure!(
                local_anchor1.is_finite()
                    && local_anchor2.is_finite()
                    && axis1.is_finite()
                    && axis2.is_finite()
                    && axis1.length_squared() > 1e-12
                    && axis2.length_squared() > 1e-12,
                "invalid joint anchors/axes"
            );
            if let Some([lo, hi]) = limits {
                ensure!(
                    lo.is_finite() && hi.is_finite() && lo <= hi,
                    "invalid joint limits"
                );
            }
            let mut data = if matches!(desc, JointDesc::Revolute { .. }) {
                RevoluteJointBuilder::new(Unit::new_normalize(vector(*axis1)))
                    .local_anchor1(point(*local_anchor1))
                    .local_anchor2(point(*local_anchor2))
                    .contacts_enabled(*contacts_enabled)
                    .build()
                    .data
            } else {
                PrismaticJointBuilder::new(Unit::new_normalize(vector(*axis1)))
                    .local_anchor1(point(*local_anchor1))
                    .local_anchor2(point(*local_anchor2))
                    .contacts_enabled(*contacts_enabled)
                    .build()
                    .data
            };
            data.set_local_axis2(Unit::new_normalize(vector(*axis2)));
            let axis = if matches!(desc, JointDesc::Revolute { .. }) {
                JointAxis::AngX
            } else {
                JointAxis::LinX
            };
            if let Some(limits) = limits {
                data.set_limits(axis, *limits);
            }
            motor(&mut data, axis, *m)?;
            data
        }
    };
    Ok(data)
}
impl PhysicsWorld {
    fn native_joint(&self, h: JointHandle) -> Result<ImpulseJointHandle> {
        ensure!(
            h.client == self.client && h.world == self.nonce,
            "foreign physics joint"
        );
        let handle = ImpulseJointHandle::from_raw_parts(h.index, h.generation);
        ensure!(
            self.joint_bodies.contains_key(&handle) && self.impulse_joints.contains(handle),
            "stale physics joint"
        );
        Ok(handle)
    }
    pub fn create_joint(
        &mut self,
        a: BodyHandle,
        b: BodyHandle,
        desc: &JointDesc,
    ) -> Result<JointHandle> {
        let a = self.native_handle(a)?;
        let b = self.native_handle(b)?;
        ensure!(a != b, "self joint is invalid");
        let data = joint(desc)?;
        self.pending_changes = true;
        let h = self.impulse_joints.insert(a, b, data, true);
        self.joint_bodies.insert(h, (a, b));
        let (index, generation) = h.into_raw_parts();
        Ok(JointHandle {
            client: self.client,
            world: self.nonce,
            index,
            generation,
        })
    }
    pub fn update_joint(&mut self, h: JointHandle, desc: &JointDesc) -> Result<()> {
        let h = self.native_joint(h)?;
        let data = joint(desc)?;
        self.pending_changes = true;
        self.impulse_joints.get_mut(h).unwrap().data = data;
        let (a, b) = self.joint_bodies[&h];
        self.bodies[a].wake_up(true);
        self.bodies[b].wake_up(true);
        Ok(())
    }
    pub fn remove_joint(&mut self, h: JointHandle) -> Result<()> {
        let h = self.native_joint(h)?;
        self.pending_changes = true;
        self.impulse_joints.remove(h, true);
        self.joint_bodies.remove(&h);
        Ok(())
    }
}
