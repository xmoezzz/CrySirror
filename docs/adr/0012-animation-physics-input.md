# ADR 0012: skeletal animation, physics/character control and input actions

Status: implemented (first slice) on 2026-10-24. These are the first engine
requirements surfaced by the Tomb Raider port plan (ADR 0011).

## Decisions

1. **Skeletal animation (glTF 2.0).** `crysirror-asset` parses nodes, skins
   (inverse bind matrices, at most 256 joints), animation channels
   (linear/step/cubic spline; T/R/S) and JOINTS_0/WEIGHTS_0. When normals are
   missing, smooth normals are computed. `anim::pose` samples and blends up
   to 4 layers (hemisphere-aligned quaternion averaging) and evaluates the
   hierarchy. `anim::palette` builds joint matrices.
   Saved state: `EntitySpec.animation` holds layers {clip, time, speed,
   weight, target_weight, fade, looping}. The world fixed step advances time
   and cross-fades; poses are derived.
   GPU: a palette storage buffer, skinned variants of the opaque, prepass and
   shadow pipelines, and **previous-frame palettes in the prepass**, so TAA
   motion vectors follow limbs.
   `Geometry::Scene { asset }` draws a whole glTF scene as one entity.
2. **Physics on Rapier 3D 0.22 (Apache-2.0, pinned).** Chosen over an
   in-house solver: it has a mature kinematic character controller
   (slide, autostep, slope limits, ground snap), BVH queries and trimeshes.
   Saved state stays in the World (`EntitySpec.collider` plus transforms).
   Rapier colliders are derived incrementally by world revision, so
   snapshot/restore needs nothing extra. Mesh colliders bake the entity's own
   geometry, including glTF models and scenes (geometry-only load, no texture
   decoding). Character bodies are excluded from velocity integration.
3. **Input actions.** Core `service::buttons` now names 19 inputs
   (WASD, Shift, Ctrl, E/Q/F/R, arrows, mouse buttons). The `Input` class
   provides bind/isDown/pressed/axis/pointerDelta. The desktop maps the keys.
4. **Playable window runs.** `crysirror run --script` drives the engine App
   inside the winit loop (`Core::Scripted`). `engine.quit()` closes the
   window.

## API (all exported to scripts and typings)

`Entity.play/stopAnimation/setCollider/moveCharacter` and the properties
`animation`/`collider`; `World.raycast/modelInfo`;
`importModel(..., {asScene: true})`; spawn accepts `collider`; `Input.*`.
`COMMAND_API` covers `set_animation` and `set_collider`.

## Evidence (Apple M5, Metal)

- `crysirror-asset` anim test: hierarchy, slerp sampling, blending, palette.
- `crysirror-scene` tests: cross-fade weights/time and collider validation.
- `crysirror-engine/tests/physics.rs`: capsule falls and lands (center
  0.8 ± 0.05), is blocked by a wall, autosteps a 0.2 m ledge, a raycast hits
  the floor entity, and the animation cross-fade advances with world time.
- `crysirror-script-js`: a TS script uses input actions to walk 2.4 m,
  lands, jumps, and raycasts.
- examples/aaa-fox-sponza (Fox CC0/CC-BY 4.0 + Sponza, fetched): a
  third-person slice with WASD/Shift/Space, an orbit camera with raycast
  collision, and Survey/Walk/Run cross-fades. dist/fox-sponza.png and
  dist/fox-sponza-gaits.png show idle and run poses (side views from a
  test-only camera script). Release CPU simulation is about 0.3 ms/frame
  (602 vs 2 frames: +0.17 s wall). GPU frame time has not been measured yet.

## Known gaps / next optimizations

Gamepad input (gilrs), root motion, animation events, IK, dynamic rigid
bodies, ragdolls, triggers/contact events, swept collision for fast
movers, GPU timestamp profiling. `World::tick` and `apply_batch` still clone
the whole world for transactional safety, which will need copy-on-write
before large levels.
