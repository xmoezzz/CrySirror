# ADR 0045: Persistent shared Rapier physics

Date: 2026-10-25

Status: implemented CPU backend; engine integration validation is recorded separately.

## Decision

`crysirror-physics` wraps Rapier3D **0.22.0** directly. Rapier performs rigid-body integration, broadphase/narrowphase collision detection, constraint solving, continuous collision detection, sleeping, joint constraints and character sweeps. CrySirror does not introduce a replacement solver. This crate has `#![forbid(unsafe_code)]` and no window, GPU or audio-device dependency. That restriction concerns CrySirror's adapter code, not a claim that every transitive numerical dependency contains no unsafe code.

One client owns one persistent `PhysicsWorld`. Each world retains its `PhysicsPipeline`, islands, `BroadPhaseMultiSap`, narrowphase, CCD solver, body/collider sets, impulse/multibody joint sets and query pipeline. Public handles include client, world nonce and native slot generation; stale and foreign-world handles fail. Entity keys carry the engine entity generation. Native pointers never become project data.

Unchanged authored descriptions retain body and collider handles and do not teleport moving bodies. In-place material, shape and body-property changes retain handles when collider count is unchanged. Changing collider count replaces that body's colliders; removing a body also removes its attached joints. Repeated shapes share cached Rapier shapes in a bounded cache, with full equality checks after hash matches. Queries incrementally apply modified/removed collider handles and avoid rebuilding the whole tree on every query.

Runtime calls `step_fixed(dt)` inside each 16 ms World fixed step, including pure data projects. Physics-owned entities skip World's ordinary velocity integration. App behavior fixed hooks do not add another physics step. The separate backend `advance(seconds)` API supports a bounded accumulator, caps catch-up work and reports dropped time; Runtime does not run that second accumulator alongside World.

## Implemented service surface

- Fixed, dynamic, position-based kinematic and velocity-based kinematic bodies; gravity scaling, damping, CCD, enabled state, sleep/wake, translation/rotation locks, force/torque/impulse and optional world application point.
- Density-driven collider mass or an explicit **total body mass**, distributed by each collider's density and volume. Changing one collider's shape recomputes all shares of an explicit total mass. Forces persist until cleared, following Rapier semantics.
- Cuboids, balls, capsules, triangle meshes, convex hulls and row-major heightfields; friction, restitution, sensor state and collision groups. Dynamic concave meshes/heightfields reject explicitly rather than substituting another volume.
- Native fixed, spherical, revolute and prismatic impulse joints; revolute/prismatic limits and motors. Joint update retains its handle. A spherical joint currently exposes anchors and contact behavior, without a cone/twist authoring layer.
- Closest ray and convex-shape/sphere sweeps; overlap queries; group, sensor, ignored-body, static-only and character filters. Disabled bodies/colliders disappear from queries immediately after authoring edits.
- Rapier character controller with explicit skin offset, step height/width, snap-to-ground, slope/slide and optional dynamic-body impulses. It returns an adjusted displacement; the engine applies that displacement exactly once.
- Bounded contact start/stop, sensor enter/exit, force and sleep/wake event queues. Removed colliders emit logical stop/exit events before owner metadata is discarded. Entity keys remain in events after handle invalidation. Queue overflow is reported in statistics.

Positive nonuniform scale is baked into boxes and explicit geometry. Nonuniform balls/capsules, reflection, perspective and sheared transforms fail visibly. A rotated parent/child combination that introduces shear must be baked into explicit geometry by an importer or authoring tool. Translation magnitude cannot relax the shear tolerance.

## Dormant-world optimization

Rapier's sleeping remains the normal active-world mechanism. An adapter-level tick skips Rapier only when there are no kinematic bodies, no joints of either kind, no pending mutations, no dirty/removed query colliders and every dynamic body is disabled or sleeping with an already reported activation state. The optimization never integrates or moves a body.

Kinematic targets, velocities, forces, impulses, wake/sleep, gravity changes, authored body/collider changes, removals and joint operations mark pending work. A query flush does not erase that work: a newly overlapping static sensor still goes through the next native pipeline tick. Joint-bearing and kinematic worlds always execute Rapier even when their current velocity is zero. `simulation_steps`, `pipeline_steps` and `idle_steps` distinguish logical ticks from actual native pipeline work.

## Source and compilation evidence

The dependency is pinned to crates.io `rapier3d = "=0.22.0"`; `Cargo.lock` records package checksum `87360935d1a54802efe0ddf908489436bb49c84865c36c930131843c7b1dc97d`. The locally installed complete source package was used to check pipeline, sets/removal, incremental query filtering, handle generations, material/mass setters, activation, character-controller and joint APIs. The full four joint implementations and integration-parameter source were read. This is not a claim that every transitive Rapier/Parry file received a full semantic audit.

`cargo check -p crysirror-physics` passed. `cargo run --release -p crysirror-physics --example benchmark -- 512` compiled and exited successfully. Raw local logs are in `target/verification/physics-check.log` and `target/verification/physics-benchmark-build.log`. The backend does not need a renderer, commercial game, font or hardware ray-tracing device.

## Behavior validation

`cargo test -p crysirror-physics` passed **25 tests**: 21 simulation tests and 4 joint tests; zero failures and zero ignored tests. The log is `target/verification/physics-tests.log`.

The tests cover falling/stacking and natural sleep, total mass and persistent forces/damping, collision/sensor/force events and removal exits, kinematic sensor detection, CCD versus an intentionally tunneling discrete control, wake/despawn/stale generations, foreign-client/world handles, unchanged dynamic sync without teleport or double gravity, stable handle/cache/query behavior, group filtering with actual collision response, ray/sweep/overlap queries, exact scale and shear/perspective rejection, terrain/trimesh geometry, convex hull collision, failed edits preserving a body, character sweeps without double motion, fixed-step chunking and reported time drops. Regression cases include static sensor changes after a query flush, disabled-object queries, multi-collider total-mass redistribution and large translations hiding shear. Real joint tests constrain fixed relative pose, spherical pivot/swing, revolute limits and a prismatic motor.

## Measured release microbenchmark

The reproducible primitive workload is `examples/benchmark.rs`; complete numeric results and commands are in [physics-benchmark-results.json](../physics-benchmark-results.json). These are single runs on an **Apple M5, macOS aarch64**, rustc 1.98.1, release profile, single-threaded Rapier, eight solver iterations and a 1/60-second fixed tick.

| Measurement | 512 dynamic cubes + floor | 2048 dynamic cubes + floor |
| --- | ---: | ---: |
| Mean first 120 falling ticks | 0.682 ms | 2.380 ms |
| Mean 600 settled ticks, including idle skip | 0.00419 ms | 0.01650 ms |
| Unchanged upsert per body, 50 rounds | 49.98 ns | 47.27 ns |
| Mean raycast, 10,000 queries | 0.236 µs | 0.300 µs |
| Mean 120 ticks after waking every eighth cube | 0.137 ms | 0.537 ms |
| Body/collider recreation during unchanged sync | 0 / 0 | 0 / 0 |

Each run completed 1,140 logical ticks: 300 native pipeline ticks and 840 idle ticks. All cubes were sleeping before the settled measurement. Each workload built two cached shapes, and every one of its 10,000 downward rays hit geometry. The settled values measure the dormant path, **not active solver throughput**. The floor and separated cubes do not measure a crowded level, complex meshes, large joint networks, whole-engine frame time, another operating system or another CPU.

## Remaining boundaries

This is a shared rigid-body foundation, not evidence of a complete production physics toolchain. It does not yet provide soft bodies, cloth, fluids, deformation, a ragdoll editor, fracture authoring, navigation, network rollback, cross-platform determinism or bit-identical serialization of Rapier's contact/solver caches. Scene snapshots may reconstruct logical bodies, poses, velocities and joints; native warm-start/island state is not an exact saved continuation. Engine/runtime integration and scene save behavior require their own tests and are not inferred from backend unit tests.

## Runtime and public API validation

Runtime now owns the persistent physics bridge. `World.tick_with` invokes it
inside each 16 ms substep; 32 ms in one host tick agrees with two 16 ms ticks,
including contact with a moving kinematic platform. Physics-owned entities skip
ordinary World velocity integration. A failed staged step invalidates derived
Rapier worlds before returning an error, so a retry cannot retain partial solver
progress from a discarded World. Unchanged authored scales avoid collider churn
from the floating-point noise of repeated rotation/decomposition.

`SystemsSpec.physics` provides data-only configuration. The reflective Physics,
PhysicsJoint and Entity APIs reach the same service. Entity object payloads
include client/id/generation, preventing same-id objects from aliasing another
client after a tool switches its active client. Snapshots save logical velocity,
forces, joint declarations and unconsumed kinematic targets, then rebuild the
derived solver. Collider declarations remain backwards-readable through defaults.
Explicit linear and angular impulses update saved velocities even while a
client is paused; paused fixed ticks still leave its pose unchanged.

The separately executed integration command passes **13 tests**: six Runtime
JSON simulations, two object-API tests and five existing character/camera/vehicle
regressions. Exact command, timestamps, source hashes, outcome and raw output
are in [physics-integration-validation.json](../physics-integration-validation.json)
and [physics-integration-tests.txt](../physics-integration-tests.txt).
A further real TypeScript test passes through Oxc, Boa and the public API:
a 2 kg body receives a 2 N·s impulse, reaches 1 m/s and moves 0.96 m in 60
16 ms Runtime steps, with one body/collider creation. These results are separate
from the backend's 25 tests and from its release benchmark; no device or game
level was involved. Authoring and API usage are documented in
[PHYSICS.md](../PHYSICS.md).
