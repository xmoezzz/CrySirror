// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
use crysirror_acoustics::{
    AcousticBackend, AcousticMaterial, AcousticMesh, AcousticScene, AcousticSceneSpec,
    AcousticSource, reflections::*,
};
use glam::{Mat4, Vec3};
fn wall(id: u32, z: f32) -> AcousticMesh {
    AcousticMesh {
        id,
        vertices: vec![
            [-10., -10., z],
            [10., -10., z],
            [10., 10., z],
            [-10., 10., z],
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        materials: vec![],
        transform: Mat4::IDENTITY.to_cols_array(),
    }
}
fn q() -> ReflectionQuery {
    ReflectionQuery {
        source: [-2., 0., 2.],
        listener: [2., 0., 2.],
        listener_forward: [0., 0., -1.],
        listener_up: [0., 1., 0.],
        options: ReflectionOptions {
            propagation: AcousticSource {
                air_absorption: [0.; 3],
                ..Default::default()
            },
            ..Default::default()
        },
    }
}
fn spec(backend: AcousticBackend) -> AcousticSceneSpec {
    AcousticSceneSpec {
        backend,
        materials: vec![AcousticMaterial {
            absorption: [0., 0.75, 1.],
            transmission: [0.; 3],
            scattering: 0.,
        }],
        meshes: vec![wall(7, 0.)],
    }
}
fn backends() -> Vec<AcousticBackend> {
    #[allow(unused_mut)]
    let mut b = vec![AcousticBackend::Parry];
    #[cfg(feature = "obvhs")]
    b.push(AcousticBackend::Obvhs);
    b
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}
#[test]
fn analytic_single_wall_delay_pressure_bands_and_shared_edge_dedup_match_both_backends() {
    for backend in backends() {
        let scene = AcousticScene::build(spec(backend)).unwrap();
        let result = simulate(&scene, &q()).unwrap();
        assert_eq!(result.tested_triangles, 2);
        assert_eq!(result.paths.len(), 1);
        let p = &result.paths[0];
        assert_eq!(p.mesh, 7);
        assert!(Vec3::from(p.bounce).length() < 1e-5);
        let length = 32f32.sqrt();
        near(p.length_metres, length);
        near(p.delay_seconds, length / 343.);
        near(p.excess_delay_seconds, (length - 4.) / 343.);
        for (actual, want) in p.gains.into_iter().zip([1. / length, 0.5 / length, 0.]) {
            near(actual, want);
        }
        let mut other = q();
        other.source[2] = -2.;
        other.listener[2] = -2.;
        let back = simulate(&scene, &other).unwrap();
        assert_eq!(back.paths.len(), 1);
        near(back.paths[0].length_metres, length);
    }
}
#[test]
fn both_legs_need_visibility_and_finite_triangles_do_not_become_infinite_planes() {
    for backend in backends() {
        for x in [-1., 1.] {
            let mut s = spec(backend);
            s.meshes.push(AcousticMesh {
                id: 9,
                vertices: vec![[x, -0.5, 0.6], [x, 0.5, 0.6], [x, 0.5, 1.4], [x, -0.5, 1.4]],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
                materials: vec![],
                transform: Mat4::IDENTITY.to_cols_array(),
            });
            let scene = AcousticScene::build(s).unwrap();
            assert!(simulate(&scene, &q()).unwrap().paths.is_empty());
        }
        let scene = AcousticScene::build(spec(backend)).unwrap();
        let mut outside = q();
        outside.source[0] = -40.;
        outside.listener[0] = -20.;
        assert!(simulate(&scene, &outside).unwrap().paths.is_empty());
        let mut opposite = q();
        opposite.source[2] = -2.;
        assert!(simulate(&scene, &opposite).unwrap().paths.is_empty());
    }
}
#[test]
fn energy_accounting_air_units_directionality_and_explicit_ignores_are_independent() {
    let mut s = spec(AcousticBackend::Parry);
    s.materials[0].absorption = [0.; 3];
    s.materials[0].transmission = [0.6; 3];
    s.materials[0].scattering = 0.75;
    let scene = AcousticScene::build(s).unwrap();
    let result = simulate(&scene, &q()).unwrap();
    let length = 32f32.sqrt();
    near(result.paths[0].gains[0], 0.4 / length);
    let mut query = q();
    query.options.propagation.air_absorption = [0.1; 3];
    near(
        simulate(&scene, &query).unwrap().paths[0].gains[0],
        0.4 / length * (-0.1 * length).exp(),
    );
    query = q();
    query.options.propagation.forward = [-1., 0., 1.];
    query.options.propagation.dipole_weight = 0.5;
    assert!(
        simulate(&scene, &query)
            .unwrap()
            .paths
            .iter()
            .all(|p| p.gains[0] < 1e-6)
    );
    query = q();
    query.options.propagation.ignore_mesh = Some(7);
    assert!(simulate(&scene, &query).unwrap().paths.is_empty());
    let mut s = spec(AcousticBackend::Parry);
    s.meshes[0].transform = Mat4::from_scale(Vec3::splat(100.)).to_cols_array();
    let scene = AcousticScene::build(s).unwrap();
    let mut query = q();
    query.source = (Vec3::from(query.source) * 100.).to_array();
    query.listener = (Vec3::from(query.listener) * 100.).to_array();
    query.options.propagation.metres_per_unit = 0.01;
    near(
        simulate(&scene, &query).unwrap().paths[0].length_metres,
        length,
    );
}
#[test]
fn budgets_cancellation_and_invalid_queries_fail_instead_of_reporting_incomplete_success() {
    let mut s = spec(AcousticBackend::Parry);
    s.meshes.push(wall(8, 4.));
    let scene = AcousticScene::build(s).unwrap();
    let mut query = q();
    query.options.max_paths = 1;
    assert!(
        simulate(&scene, &query)
            .unwrap_err()
            .to_string()
            .contains("path budget")
    );
    query = q();
    query.options.triangle_budget = 1;
    assert!(
        simulate(&scene, &query)
            .unwrap_err()
            .to_string()
            .contains("triangle budget")
    );
    query = q();
    query.options.max_delay_seconds = 0.;
    assert!(simulate(&scene, &query).unwrap().paths.is_empty());
    query = q();
    query.source[0] = f32::NAN;
    assert!(simulate(&scene, &query).is_err());
    query = q();
    query.listener_up = query.listener_forward;
    assert!(simulate(&scene, &query).is_err());
    query = q();
    query.options.propagation.radius = 1.;
    assert!(simulate(&scene, &query).is_err());
    assert!(
        simulate_cancellable(&scene, &q(), || anyhow::bail!("test canceled"))
            .unwrap_err()
            .to_string()
            .contains("test canceled")
    );
}

#[test]
fn reflection_legs_share_per_crossing_transmission_and_enforce_their_own_budget() {
    for backend in backends() {
        let mut s = spec(backend);
        s.materials[0].absorption = [0.; 3];
        s.materials.push(AcousticMaterial {
            absorption: [0.; 3],
            transmission: [0.5, 0.25, 0.125],
            scattering: 0.,
        });
        for (i, x) in [-1., 1.].into_iter().enumerate() {
            s.meshes.push(AcousticMesh {
                id: 9 + i as u32,
                vertices: vec![[x, -0.5, 0.6], [x, 0.5, 0.6], [x, 0.5, 1.4], [x, -0.5, 1.4]],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
                materials: vec![1, 1],
                transform: Mat4::IDENTITY.to_cols_array(),
            });
        }
        let scene = AcousticScene::build(s.clone()).unwrap();
        let result = simulate(&scene, &q()).unwrap();
        assert_eq!(result.paths.len(), 1);
        let path = &result.paths[0];
        assert_eq!(path.crossings, 2);
        for (i, t) in [0.25, 0.0625, 0.015625].into_iter().enumerate() {
            near(path.leg_transmission[i], t);
            near(path.gains[i], t / 32f32.sqrt());
        }
        s.meshes.push(AcousticMesh {
            id: 20,
            vertices: vec![
                [-1.5, -0.5, 1.2],
                [-1.5, 0.5, 1.2],
                [-1.5, 0.5, 1.8],
                [-1.5, -0.5, 1.8],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            materials: vec![1, 1],
            transform: Mat4::IDENTITY.to_cols_array(),
        });
        let scene = AcousticScene::build(s).unwrap();
        let mut query = q();
        query.options.propagation.transmission_hits = 1;
        let error = simulate(&scene, &query).unwrap_err();
        assert!(format!("{error:#}").contains("crossing budget"));
    }
}

#[test]
fn a_visible_wall_echo_can_survive_a_fully_occluded_direct_path() {
    for backend in backends() {
        let mut s = spec(backend);
        s.materials[0].absorption = [0.; 3];
        s.meshes.push(AcousticMesh {
            id: 9,
            vertices: vec![[0., -1., 1.6], [0., 1., 1.6], [0., 1., 2.4], [0., -1., 2.4]],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
            materials: vec![],
            transform: Mat4::IDENTITY.to_cols_array(),
        });
        let scene = AcousticScene::build(s).unwrap();
        let query = q();
        let direct = crysirror_acoustics::simulate(
            &scene,
            &crysirror_acoustics::AcousticQuery {
                source: query.source,
                listener: query.listener,
                settings: query.options.propagation.clone(),
            },
        )
        .unwrap();
        assert_eq!(direct.gains, [0.; 3]);
        let reflections = simulate(&scene, &query).unwrap();
        assert_eq!(reflections.paths.len(), 1);
        assert!(reflections.paths[0].gains.iter().all(|g| *g > 0.1));
        assert_eq!(reflections.paths[0].crossings, 0);
    }
}
