// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
//! Dense finite reflector with an analytic image-source oracle, not a hardware ranking.
use anyhow::{Result, ensure};
use crysirror_acoustics::{
    reflections::{ReflectionOptions, ReflectionQuery},
    *,
};
use glam::Mat4;
fn main() -> Result<()> {
    let side: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "128".into())
        .parse()?;
    ensure!((1..=362).contains(&side), "grid side must be 1..362");
    let mut vertices = Vec::new();
    for y in 0..=side {
        for x in 0..=side {
            vertices.push([
                x as f32 - side as f32 * 0.5,
                y as f32 - side as f32 * 0.5,
                -12.,
            ]);
        }
    }
    let mut triangles = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let i = (y * (side + 1) + x) as u32;
            let stride = side as u32 + 1;
            triangles.extend([[i, i + 1, i + stride + 1], [i, i + stride + 1, i + stride]]);
        }
    }
    let mesh = AcousticMesh {
        id: 1,
        vertices,
        triangles,
        materials: vec![],
        transform: Mat4::IDENTITY.to_cols_array(),
    };
    #[allow(unused_mut)]
    let mut backends = vec![AcousticBackend::Parry];
    #[cfg(feature = "obvhs")]
    backends.push(AcousticBackend::Obvhs);
    for backend in backends {
        let start = std::time::Instant::now();
        let scene = AcousticScene::build(AcousticSceneSpec {
            backend,
            materials: vec![AcousticMaterial {
                absorption: [0.; 3],
                transmission: [0.; 3],
                scattering: 0.,
            }],
            meshes: vec![mesh.clone()],
        })?;
        let build = start.elapsed().as_secs_f64();
        let start = std::time::Instant::now();
        for i in 0..10 {
            let x = i as f32 * 0.01;
            let q = ReflectionQuery {
                source: [x, 0., 2.],
                listener: [0.; 3],
                listener_forward: [0., 0., -1.],
                listener_up: [0., 1., 0.],
                options: ReflectionOptions {
                    propagation: AcousticSource {
                        air_absorption: [0.; 3],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            };
            let p = reflections::simulate(&scene, &q)?;
            ensure!(
                p.paths.len() == 1,
                "shared-edge reflector duplicated/missed: {}",
                p.paths.len()
            );
            let length = (x * x + 26. * 26.).sqrt();
            let direct = (x * x + 4.).sqrt();
            ensure!(
                (p.paths[0].length_metres - length).abs() < 1e-4
                    && (p.paths[0].excess_delay_seconds - (length - direct) / 343.).abs() < 1e-5
                    && (p.paths[0].gains[0] - 1. / length).abs() < 1e-5,
                "analytic mirror oracle mismatch"
            );
        }
        println!(
            "backend={backend:?}; triangles={}; build_seconds={build:.6}; mean_query_ms={:.6}; oracle=10/10",
            scene.triangle_count(),
            start.elapsed().as_secs_f64() * 100.
        );
    }
    Ok(())
}
