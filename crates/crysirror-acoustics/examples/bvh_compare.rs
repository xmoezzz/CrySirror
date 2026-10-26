// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see ../LICENSE.
//! Synthetic downward-ray workload, not a general engine/backend ranking.
use crysirror_acoustics::*;
use glam::{Mat4, Vec3};
use std::time::Instant;
fn spec(backend: AcousticBackend) -> AcousticSceneSpec {
    let mut mesh = AcousticMesh {
        id: 1,
        vertices: vec![],
        triangles: vec![],
        materials: vec![],
        transform: Mat4::IDENTITY.to_cols_array(),
    };
    for y in 0..128 {
        for x in 0..128 {
            let z = height(x, y);
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([
                [x as f32, y as f32, z],
                [x as f32 + 1., y as f32, z],
                [x as f32 + 1., y as f32 + 1., z],
                [x as f32, y as f32 + 1., z],
            ]);
            mesh.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        }
    }
    AcousticSceneSpec {
        backend,
        materials: vec![Default::default()],
        meshes: vec![mesh],
    }
}
fn height(x: u32, y: u32) -> f32 {
    2. + (x as f32 * 0.13).sin() * (y as f32 * 0.17).cos()
}
fn next(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    (*seed >> 8) as f32 / 16777216.
}
fn main() -> anyhow::Result<()> {
    let mut seed = 0x43535953;
    let rays = (0..100000)
        .map(|_| {
            let x = next(&mut seed) * 160. - 16.;
            let y = next(&mut seed) * 160. - 16.;
            let min = next(&mut seed) * 12.;
            (Vec3::new(x, y, 10.), min)
        })
        .collect::<Vec<_>>();
    let mut reports = vec![];
    #[allow(unused_mut)]
    let mut backends = vec![AcousticBackend::Parry];
    #[cfg(feature = "obvhs")]
    backends.push(AcousticBackend::Obvhs);
    for backend in backends {
        let mut build = vec![];
        let mut retained = None;
        for _ in 0..3 {
            let spec = spec(backend);
            let start = Instant::now();
            let scene = AcousticScene::build(spec)?;
            build.push(start.elapsed().as_secs_f64() * 1000.);
            retained = Some(scene);
        }
        let scene = retained.unwrap();
        let mut times = vec![];
        let mut hits = 0u64;
        for iteration in 0..5 {
            let start = Instant::now();
            let mut count = 0;
            for (origin, min) in &rays {
                let hit = scene.closest_hit(*origin, -Vec3::Z, *min, 14., None)?;
                count += u64::from(hit.is_some());
                if iteration == 0 {
                    let expected =
                        if origin.x >= 0. && origin.x < 128. && origin.y >= 0. && origin.y < 128. {
                            let d = 10. - height(origin.x.floor() as u32, origin.y.floor() as u32);
                            (d >= *min).then_some(d)
                        } else {
                            None
                        };
                    anyhow::ensure!(
                        hit.is_some() == expected.is_some(),
                        "backend oracle mismatch {origin:?} min {min}"
                    );
                    if let (Some(hit), Some(d)) = (hit, expected) {
                        anyhow::ensure!(
                            (hit.distance - d).abs() < 0.0001 && hit.mesh == 1,
                            "distance/identity mismatch"
                        );
                    }
                }
            }
            if iteration > 0 {
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
            hits = count;
        }
        build.sort_by(f64::total_cmp);
        times.sort_by(f64::total_cmp);
        reports.push(serde_json::json!({"backend":backend,"triangles":scene.triangle_count(),"rays":rays.len(),"hits":hits,"oracle_pass":true,"build_median_ms":build[1],"query_median_ms":(times[1]+times[2])/2.,"query_iterations":4}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"workload":"seeded vertical rays against 128x128 terraced grid, misses and interval clipping; single-thread queries/build; BVH2 only","results":reports})
        )?
    );
    Ok(())
}
