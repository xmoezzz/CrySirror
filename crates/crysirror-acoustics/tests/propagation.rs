// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
use crysirror_acoustics::*;
use glam::{Mat4, Vec3};
fn plane(id: u32, z: f32, material: u32) -> AcousticMesh {
    AcousticMesh {
        id,
        vertices: vec![
            [-10., -10., z],
            [10., -10., z],
            [10., 10., z],
            [-10., 10., z],
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        materials: vec![material; 2],
        transform: Mat4::IDENTITY.to_cols_array(),
    }
}
fn backends() -> Vec<AcousticBackend> {
    #[allow(unused_mut)]
    let mut result = vec![AcousticBackend::Parry];
    #[cfg(feature = "obvhs")]
    result.push(AcousticBackend::Obvhs);
    result
}
fn spec(backend: AcousticBackend) -> AcousticSceneSpec {
    AcousticSceneSpec {
        backend,
        materials: vec![AcousticMaterial {
            transmission: [0.5, 0.25, 0.125],
            ..Default::default()
        }],
        meshes: vec![plane(7, 2., 0), plane(8, 4., 0)],
    }
}
fn query() -> AcousticQuery {
    AcousticQuery {
        listener: [0., 0., 0.],
        source: [0., 0., 6.],
        settings: AcousticSource {
            air_absorption: [0.; 3],
            ..Default::default()
        },
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
#[test]
fn segment_bounds_world_distance_backfaces_and_ignored_meshes() {
    for backend in backends() {
        let scene = AcousticScene::build(spec(backend)).unwrap();
        let ray = |o, d, min, max, ignore| scene.closest_hit(o, d, min, max, ignore).unwrap();
        assert!(ray(Vec3::ZERO, Vec3::Z, 1., 1.9, None).is_none());
        assert!(ray(Vec3::ZERO, Vec3::Z, 1., 2., None).is_none());
        let hit = ray(Vec3::ZERO, Vec3::Z * 2., 1., 3., None).unwrap();
        near(hit.distance, 2.);
        assert_eq!(hit.mesh, 7);
        let hit = ray(Vec3::ZERO, Vec3::Z, 2.01, 5., None).unwrap();
        near(hit.distance, 4.);
        assert_eq!(hit.mesh, 8);
        let hit = ray(Vec3::ZERO, Vec3::Z, 0., 5., Some(7)).unwrap();
        near(hit.distance, 4.);
        assert_eq!(hit.mesh, 8);
        near(
            ray(Vec3::Z * 6., -Vec3::Z, 0., 5., None).unwrap().distance,
            2.,
        );
        assert!(!scene.blocked(Vec3::ZERO, Vec3::Z * 1.9, None).unwrap());
        assert!(
            scene
                .closest_hit(Vec3::ZERO, Vec3::ZERO, 0., 1., None)
                .is_err()
        );
    }
}
#[test]
fn two_material_crossings_air_units_directivity_and_budget() {
    for backend in backends() {
        let scene = AcousticScene::build(spec(backend)).unwrap();
        let mut q = query();
        let result = simulate(&scene, &q).unwrap();
        assert_eq!(result.crossings, 2);
        near(result.visibility, 0.);
        for (a, b) in result.gains.into_iter().zip([0.25, 0.0625, 0.015625]) {
            near(a, b);
        }
        near(result.delay_seconds, 6. / 343.);
        q.settings.transmission_hits = 1;
        assert!(
            simulate(&scene, &q)
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
        q.settings.transmission_hits = 2;
        q.settings.ignore_mesh = Some(7);
        assert_eq!(simulate(&scene, &q).unwrap().crossings, 1);
        q.settings.air_absorption = [0.1; 3];
        q.settings.metres_per_unit = 2.;
        let path = simulate(&scene, &q).unwrap();
        near(path.air_absorption[0], (-1.2f32).exp());
        near(path.delay_seconds, 12. / 343.);
        q.settings.dipole_weight = 0.5;
        q.settings.forward = [0., 0., 1.];
        near(simulate(&scene, &q).unwrap().directivity, 0.);
        q.settings.forward = [0., 0., -1.];
        near(simulate(&scene, &q).unwrap().directivity, 1.);
        q.source = q.listener;
        near(simulate(&scene, &q).unwrap().gains[0], 1.);
    }
}
#[test]
fn volume_visibility_is_fractional_and_repeatable() {
    for backend in backends() {
        let mut s = spec(backend);
        s.meshes.truncate(1);
        for p in &mut s.meshes[0].vertices {
            p[0] = (p[0] + 10.) * 0.5;
        }
        let scene = AcousticScene::build(s).unwrap();
        let mut q = query();
        q.source[0] = 0.03;
        q.settings.radius = 1.;
        q.settings.samples = 64;
        let a = simulate(&scene, &q).unwrap();
        let b = simulate(&scene, &q).unwrap();
        assert!(a.visibility > 0.2 && a.visibility < 0.8, "{a:?}");
        assert_eq!(a.gains, b.gains);
    }
}
#[test]
fn transformed_geometry_and_invalid_inputs() {
    let mut s = spec(AcousticBackend::Parry);
    s.meshes.truncate(1);
    s.meshes[0].transform =
        Mat4::from_scale_rotation_translation(Vec3::new(2., 1., 3.), glam::Quat::IDENTITY, Vec3::Z)
            .to_cols_array();
    let scene = AcousticScene::build(s.clone()).unwrap();
    near(
        scene
            .closest_hit(Vec3::ZERO, Vec3::Z, 0., 10., None)
            .unwrap()
            .unwrap()
            .distance,
        7.,
    );
    s.meshes[0].triangles[0][0] = 999;
    assert!(AcousticScene::build(s).is_err());
    let mut s = spec(AcousticBackend::Parry);
    s.meshes[1].id = 7;
    assert!(AcousticScene::build(s).is_err());
    let mut s = spec(AcousticBackend::Parry);
    s.materials[0].transmission[0] = f32::NAN;
    assert!(AcousticScene::build(s).is_err());
    let scene = AcousticScene::build(Default::default()).unwrap();
    let mut q = query();
    q.source[0] = f32::NAN;
    assert!(simulate(&scene, &q).is_err());
    q = query();
    q.settings.samples = 65;
    assert!(simulate(&scene, &q).is_err());
    q = query();
    q.settings.forward = [f32::MAX; 3];
    assert!(simulate(&scene, &q).is_err());
}
#[test]
fn band_filter_preserves_unity_and_frequency_selectivity_without_startup_burst() {
    use crysirror_acoustics::spectrum::BandFilter;
    let mut unity = BandFilter::new(48000).unwrap();
    for i in 0..1000 {
        let x = (i as f32 * 0.17).sin();
        near(unity.process(x), x);
    }
    let mut muted = BandFilter::with_gains(48000, [0.; 3]).unwrap();
    assert_eq!(muted.process(1.), 0.);
    fn rms(freq: f32, gains: [f32; 3]) -> f32 {
        let mut f = BandFilter::with_gains(48000, gains).unwrap();
        let mut power = 0.;
        for i in 0..48000 {
            let x = (std::f32::consts::TAU * freq * i as f32 / 48000.).sin();
            let y = f.process(x);
            if i >= 24000 {
                power += y * y;
            }
        }
        (power / 24000.).sqrt()
    }
    assert!(rms(100., [1., 0., 0.]) > 0.65);
    assert!(rms(12000., [1., 0., 0.]) < 0.08);
    assert!(rms(12000., [0., 0., 1.]) > rms(100., [0., 0., 1.]) * 30.);
    assert!(muted.set_gains([f32::NAN; 3]).is_err());
    assert!(BandFilter::new(0).is_err());
}
#[cfg(not(feature = "obvhs"))]
#[test]
fn unavailable_backend_never_silently_falls_back() {
    assert!(
        AcousticScene::build(AcousticSceneSpec {
            backend: AcousticBackend::Obvhs,
            ..Default::default()
        })
        .is_err()
    );
}
