use crysirror_physics::*;
use glam::{Quat, Vec3};
fn key(id: u32) -> BodyKey {
    BodyKey { id, generation: 0 }
}
fn world() -> PhysicsWorld {
    PhysicsWorld::new(1, WorldConfig::default()).unwrap()
}
fn body(w: &mut PhysicsWorld, id: u32, kind: BodyKind, p: Vec3) -> BodyHandle {
    w.upsert(
        key(id),
        &BodyDesc {
            kind,
            pose: Pose::at(p),
            ..Default::default()
        },
        &[ColliderDesc::default()],
    )
    .unwrap()
}
fn steps(w: &mut PhysicsWorld, n: usize) {
    for _ in 0..n {
        w.step_fixed(1. / 60.).unwrap();
    }
}
#[test]
fn fixed_joint_holds_relative_pose_and_is_removed_with_body() {
    let mut w = world();
    let a = body(&mut w, 0, BodyKind::Fixed, Vec3::Y * 2.);
    let b = body(&mut w, 1, BodyKind::Dynamic, Vec3::Y);
    let j = w
        .create_joint(
            a,
            b,
            &JointDesc::Fixed {
                local_frame1: Pose::at(-Vec3::Y),
                local_frame2: Pose::default(),
                contacts_enabled: false,
            },
        )
        .unwrap();
    w.impulse(b, Vec3::X * 10., None).unwrap();
    steps(&mut w, 180);
    assert!((w.state(b).unwrap().pose.translation - Vec3::Y).length() < 0.03);
    w.remove(key(1)).unwrap();
    assert_eq!(w.stats().joints, 0);
    assert!(w.remove_joint(j).is_err());
}
#[test]
fn spherical_joint_keeps_pivot_but_allows_a_real_swing() {
    let mut w = world();
    let a = body(&mut w, 0, BodyKind::Fixed, Vec3::Y * 3.);
    let b = body(&mut w, 1, BodyKind::Dynamic, Vec3::Y);
    w.create_joint(
        a,
        b,
        &JointDesc::Spherical {
            local_anchor1: Vec3::ZERO,
            local_anchor2: Vec3::Y * 2.,
            contacts_enabled: false,
        },
    )
    .unwrap();
    w.impulse(b, Vec3::X * 3., None).unwrap();
    let mut max_angle = 0f32;
    for _ in 0..360 {
        steps(&mut w, 1);
        let s = w.state(b).unwrap();
        let pivot = s.pose.translation + s.pose.rotation * (Vec3::Y * 2.);
        assert!((pivot - Vec3::Y * 3.).length() < 0.05);
        max_angle = max_angle.max(s.pose.rotation.angle_between(Quat::IDENTITY));
    }
    assert!(max_angle > 0.2);
}
#[test]
fn revolute_limits_restrain_rotation_and_update_keeps_joint_alive() {
    let mut w = world();
    let a = body(&mut w, 0, BodyKind::Fixed, Vec3::ZERO);
    let b = body(&mut w, 1, BodyKind::Dynamic, -Vec3::Y);
    let desc = JointDesc::Revolute {
        local_anchor1: Vec3::ZERO,
        local_anchor2: Vec3::Y,
        axis1: Vec3::Z,
        axis2: Vec3::Z,
        limits: Some([-0.3, 0.3]),
        motor: None,
        contacts_enabled: false,
    };
    let joint = w.create_joint(a, b, &desc).unwrap();
    w.torque(b, Vec3::Z * 10., true).unwrap();
    steps(&mut w, 240);
    let s = w.state(b).unwrap();
    assert!(s.pose.rotation.to_euler(glam::EulerRot::XYZ).2.abs() < 0.4);
    assert!((s.pose.translation + s.pose.rotation * Vec3::Y).length() < 0.05);
    w.update_joint(joint, &desc).unwrap();
    assert_eq!(w.stats().joints, 1);
    w.remove_joint(joint).unwrap();
    assert!(w.update_joint(joint, &desc).is_err());
}
#[test]
fn prismatic_motor_moves_only_along_its_axis() {
    let mut w = world();
    w.configure(Vec3::ZERO, 1. / 60.).unwrap();
    let a = body(&mut w, 0, BodyKind::Fixed, Vec3::ZERO);
    let b = body(&mut w, 1, BodyKind::Dynamic, Vec3::ZERO);
    w.create_joint(
        a,
        b,
        &JointDesc::Prismatic {
            local_anchor1: Vec3::ZERO,
            local_anchor2: Vec3::ZERO,
            axis1: Vec3::X,
            axis2: Vec3::X,
            limits: Some([-1., 1.]),
            motor: Some(Motor {
                target_position: 0.6,
                target_velocity: 0.,
                stiffness: 100.,
                damping: 20.,
                max_force: 100.,
            }),
            contacts_enabled: false,
        },
    )
    .unwrap();
    steps(&mut w, 240);
    let s = w.state(b).unwrap();
    assert!((s.pose.translation.x - 0.6).abs() < 0.05, "{:?}", s);
    assert!(s.pose.translation.y.abs() < 0.01);
    assert!(s.pose.rotation.angle_between(Quat::IDENTITY) < 0.01);
}
