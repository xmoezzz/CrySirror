// SPDX-License-Identifier: MPL-2.0
//! Synthetic CPU microbenchmark, not a game/frame-time claim.
use anyhow::{ensure, Result};
use crysirror_physics::*;
use glam::Vec3;
use std::time::Instant;
fn key(id: u32) -> BodyKey {
    BodyKey { id, generation: 0 }
}
fn main() -> Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "512".into())
        .parse()?;
    ensure!((1..=16_384).contains(&count), "body count outside 1..16384");
    let mut world = PhysicsWorld::new(
        1,
        WorldConfig {
            max_events: 4096,
            ..Default::default()
        },
    )?;
    let start = Instant::now();
    world.upsert(
        key(0),
        &BodyDesc {
            pose: Pose::at(-Vec3::Y * 0.5),
            ..Default::default()
        },
        &[ColliderDesc {
            shape: Shape::Cuboid {
                half_extents: Vec3::new(200., 0.5, 200.),
            },
            ..Default::default()
        }],
    )?;
    let side = (count as f64).sqrt().ceil() as usize;
    let mut descriptions = Vec::with_capacity(count);
    for i in 0..count {
        let desc = BodyDesc {
            kind: BodyKind::Dynamic,
            pose: Pose::at(Vec3::new(
                (i % side) as f32 * 1.2 - side as f32 * 0.6,
                5.,
                (i / side) as f32 * 1.2 - side as f32 * 0.6,
            )),
            ..Default::default()
        };
        let h = world.upsert(key(i as u32 + 1), &desc, &[ColliderDesc::default()])?;
        descriptions.push((h, desc));
    }
    let creation_ms = start.elapsed().as_secs_f64() * 1000.;
    let created = world.stats();
    let time_steps = |world: &mut PhysicsWorld, steps: usize| -> Result<f64> {
        let start = Instant::now();
        for _ in 0..steps {
            world.step_fixed(1. / 60.)?;
            world.drain_events();
        }
        Ok(start.elapsed().as_secs_f64() * 1000. / steps as f64)
    };
    let falling_ms = time_steps(&mut world, 120)?;
    time_steps(&mut world, 300)?;
    let sleeping_before = world.stats();
    let sleeping_ms = time_steps(&mut world, 600)?;
    let before_update = world.stats();
    let start = Instant::now();
    let update_rounds = 50;
    for _ in 0..update_rounds {
        for (i, (h, desc)) in descriptions.iter().enumerate() {
            let same = world.upsert(key(i as u32 + 1), desc, &[ColliderDesc::default()])?;
            ensure!(*h == same, "stable body handle changed");
        }
    }
    let unchanged_upsert_ns = start.elapsed().as_secs_f64() * 1e9 / (count * update_rounds) as f64;
    let after_update = world.stats();
    ensure!(
        before_update.bodies_created == after_update.bodies_created
            && before_update.colliders_created == after_update.colliders_created,
        "unchanged sync rebuilt physics objects"
    );
    let query_count = 10_000;
    let start = Instant::now();
    let mut query_hits = 0;
    let mut distance_sum = 0f64;
    for i in 0..query_count {
        let (_, desc) = &descriptions[i % count];
        if let Some(hit) = world.raycast(
            desc.pose.translation + Vec3::Y * 10.,
            -Vec3::Y,
            30.,
            QueryOptions::default(),
        )? {
            query_hits += 1;
            distance_sum += std::hint::black_box(hit.distance) as f64;
        }
    }
    let raycast_us = start.elapsed().as_secs_f64() * 1e6 / query_count as f64;
    for (h, _) in descriptions.iter().step_by(8) {
        world.impulse(*h, Vec3::Y * 2., None)?;
    }
    let waking_ms = time_steps(&mut world, 120)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"kind":"synthetic persistent CPU physics microbenchmark","arch":std::env::consts::ARCH,"os":std::env::consts::OS,"rapier":"0.22.0","dynamic_cubes":count,"fixed_colliders":1,"dt_seconds":1./60.,"solver_iterations":8,"creation_ms":creation_ms,"falling_120_steps_mean_ms":falling_ms,"settled_600_steps_mean_ms":sleeping_ms,"sleeping_before_settled":sleeping_before.sleeping_bodies,"unchanged_upsert_ns_per_body":unchanged_upsert_ns,"sync_rounds":update_rounds,"stable_body_handles":true,"body_recreates_during_sync":after_update.bodies_created-before_update.bodies_created,"collider_recreates_during_sync":after_update.colliders_created-before_update.colliders_created,"raycast_count":query_count,"raycast_hits":query_hits,"raycast_mean_us":raycast_us,"raycast_distance_sum":distance_sum,"wake_every_eighth_120_steps_mean_ms":waking_ms,"initial_stats":created,"final_stats":world.stats(),"limits":["one CPU host run, no renderer/audio devices","settled timing includes conservative idle skip, not an active solver frame time","primitive boxes and a flat floor, no commercial assets or complex-level performance claim"]})
        )?
    );
    Ok(())
}
