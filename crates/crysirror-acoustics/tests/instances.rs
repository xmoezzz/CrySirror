// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
use crysirror_acoustics::*;
use glam::{Mat4, Quat, Vec3};
fn backends() -> Vec<AcousticBackend> {
    #[allow(unused_mut)]
    let mut b = vec![AcousticBackend::Parry];
    #[cfg(feature = "obvhs")]
    b.push(AcousticBackend::Obvhs);
    b
}
fn wall(id: u32, z: f32) -> AcousticMesh {
    AcousticMesh {
        id,
        vertices: vec![[-2., -2., z], [2., -2., z], [2., 2., z], [-2., 2., z]],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        materials: vec![],
        transform: Mat4::IDENTITY.to_cols_array(),
    }
}
fn spec(backend: AcousticBackend) -> AcousticSceneSpec {
    AcousticSceneSpec {
        backend,
        materials: vec![AcousticMaterial {
            transmission: [0.; 3],
            absorption: [0.; 3],
            scattering: 0.,
        }],
        meshes: vec![wall(7, 0.)],
    }
}
#[test]
fn affine_instance_queries_keep_world_distance_normals_and_original_snapshots() {
    for backend in backends() {
        let base = AcousticScene::build(spec(backend)).unwrap();
        for mirror in [1., -1.] {
            for i in 0..24 {
                let angle = i as f32 * 0.19;
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::new(mirror * (0.3 + angle), 1.2, 2.4),
                    Quat::from_rotation_y(angle) * Quat::from_rotation_x(0.3),
                    Vec3::new(0.7, -1.3, 4.),
                );
                let changed = base
                    .updated(&[AcousticMeshUpdate {
                        id: 7,
                        transform: Some(transform.to_cols_array()),
                        enabled: None,
                    }])
                    .unwrap();
                let points = [
                    Vec3::new(-2., -2., 0.),
                    Vec3::new(2., -2., 0.),
                    Vec3::new(2., 2., 0.),
                ]
                .map(|p| transform.transform_point3(p));
                let normal = (points[1] - points[0])
                    .cross(points[2] - points[0])
                    .normalize();
                for fraction in [0.2, 0.5, 0.8] {
                    let target = points[0] * fraction
                        + points[1] * (1. - fraction) * 0.3
                        + points[2] * (1. - fraction) * 0.7;
                    for side in [-1., 1.] {
                        let hit = changed
                            .closest_hit(
                                target + normal * side * 7.,
                                normal * (-side * 3.),
                                1.,
                                9.,
                                None,
                            )
                            .unwrap()
                            .unwrap();
                        assert!((hit.distance - 7.).abs() < 3e-5, "{backend:?} {i}: {hit:?}");
                        assert!(Vec3::from(hit.normal).dot(normal) > 0.9999);
                        assert_eq!((hit.mesh, hit.material), (7, 0));
                    }
                }
                assert_eq!(changed.geometry_stats().reused_mesh_bvhs, 1);
            }
        }
        let original = base
            .closest_hit(Vec3::Z * 3., -Vec3::Z, 0., 4., None)
            .unwrap()
            .unwrap();
        assert!((original.distance - 3.).abs() < 1e-6);
        assert_eq!(
            base.mesh_pose(7).unwrap().transform,
            Some(Mat4::IDENTITY.to_cols_array())
        );
    }
}
#[test]
fn disabling_geometry_keeps_triangle_ids_and_reflection_budget_stable() {
    use crysirror_acoustics::reflections::{ReflectionQuery, simulate};
    for backend in backends() {
        let mut s = spec(backend);
        s.meshes.insert(0, wall(8, -10.));
        let base = AcousticScene::build(s).unwrap();
        let q = ReflectionQuery {
            source: [-1., 0., 2.],
            listener: [1., 0., 2.],
            listener_forward: [0., 0., -1.],
            listener_up: [0., 1., 0.],
            options: Default::default(),
        };
        let before = simulate(&base, &q).unwrap();
        let stable = before.paths.iter().find(|p| p.mesh == 7).unwrap().triangle;
        let changed = base
            .updated(&[AcousticMeshUpdate {
                id: 8,
                transform: None,
                enabled: Some(false),
            }])
            .unwrap();
        let after = simulate(&changed, &q).unwrap();
        assert_eq!(
            after.paths.iter().find(|p| p.mesh == 7).unwrap().triangle,
            stable
        );
        assert_eq!(after.tested_triangles, 4);
        assert!(
            changed
                .closest_hit(Vec3::Z * -5., -Vec3::Z, 0., 6., None)
                .unwrap()
                .is_none()
        );
        assert!(
            base.closest_hit(Vec3::Z * -5., -Vec3::Z, 0., 6., None)
                .unwrap()
                .is_some()
        );
        let off = changed
            .updated(&[AcousticMeshUpdate {
                id: 7,
                transform: None,
                enabled: Some(false),
            }])
            .unwrap();
        assert!(simulate(&off, &q).unwrap().paths.is_empty());
        assert_eq!(off.geometry_stats().active_meshes, 0);
        assert_eq!(off.triangle_count(), 4);
    }
}
#[test]
fn invalid_atomic_batches_leave_the_original_geometry_unchanged() {
    let base = AcousticScene::build(spec(AcousticBackend::Parry)).unwrap();
    let valid = AcousticMeshUpdate {
        id: 7,
        transform: Some(Mat4::from_translation(Vec3::X * 5.).to_cols_array()),
        enabled: None,
    };
    let unknown = AcousticMeshUpdate {
        id: 99,
        ..valid.clone()
    };
    for batch in [
        vec![],
        vec![valid.clone(), valid.clone()],
        vec![valid.clone(), unknown],
        vec![AcousticMeshUpdate {
            id: 7,
            transform: Some([0.; 16]),
            enabled: None,
        }],
        vec![AcousticMeshUpdate {
            id: 7,
            transform: None,
            enabled: None,
        }],
    ] {
        assert!(base.updated(&batch).is_err());
    }
    assert!(
        base.closest_hit(Vec3::Z * 3., -Vec3::Z, 0., 4., None)
            .unwrap()
            .is_some()
    );
    let mut bad = spec(AcousticBackend::Parry);
    bad.meshes[0].transform[0] = f32::MAX;
    assert!(AcousticScene::build(bad).is_err());
}
