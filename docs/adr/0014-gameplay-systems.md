# ADR 0014: gameplay systems surfaced by the Unreal port assessment

Status: implemented (first slice) on 2026-10-25. The system list comes from
the Ashen Vow inventory (ADR 0013): GAS, montages/blend spaces/anim
graphs, StateTree/BT AI with navigation, Niagara-class particles,
landscape, level streaming and spatial audio.

## Principles (shared by all systems)

- **Definitions are world data** (`World.gameplay`, `.animation`, `.ai`,
  `.streaming`) and **per-entity state is a component** (`gameplay`,
  `animation.machine/montage`, `ai`, `particles`, terrain geometry). All of
  it is saved and validated, changes only through transactional world
  commands, and advances on the 16 ms world step.
- **Derived data is never saved:** poses, navigation grids, path caches,
  particle simulations, terrain meshes, color maps, heightfield colliders and
  spatial-voice positions.
- **Every system is reachable through the OOP API, and therefore from
  scripts and typings.** `COMMAND_API` plus tests/api_coverage.rs enforce
  this for all 18 world command variants.

## Systems

| System | Model | API |
|---|---|---|
| Gameplay (GAS-like) | Attributes (base/min/max/max_attribute), effects (instant/duration/infinite, periodic, add/multiply/override, stacking, blocked-by tags, cues), hierarchical tags (loose + granted), abilities (required/blocked/active tags, cost, cooldown, cancel-by-tag, timed end), event queue | `world.defineGameplay`; `entity.initAttributes/attribute/setAttribute/applyEffect/removeEffect/hasTag/addTag/removeTag/grantAbility/activate/endAbility/drainEvents`, `tags` |
| Animation graphs | State machines (params, `*` transitions, conditions, min_time, fades), 1D/2D blend spaces with phase sync, montages (rate, sections, notifies, blend in/out) composed over the machine into skinned layers | `world.defineAnimation`; `entity.setStateMachine/setAnimParam/playMontage/stopMontage/drainAnimEvents` |
| AI | Reactive behaviour trees (selector, sequence, invert, cooldown, check, has_tag, near, wait, move_to, face, ability, set, perceive), blackboards; 2.5D nav grid baked from collision (slope, headroom, step, radius erosion), A* + string pulling | `world.defineBehaviorTree/buildNavigation/findPath`; `entity.runBehaviorTree/setBlackboard`, `ai` |
| Particles | Emitter spec (rate/burst, lifetime, cone, gravity, drag, size/color over life, HDR colors, additive/alpha, seed); CPU simulation with steady-state warm-up; instanced billboards in the MSAA HDR pass, alpha sorted back to front | spawn `particles`; `entity.setParticles` |
| Terrain | Procedural fBm or 8/16-bit heightmap; smooth-normal mesh; baked sand/grass/rock/snow color map from the actual height range and slope; Rapier heightfield collider | spawn `geometry: {kind: "terrain", ...}` with a mesh collider |
| Level streaming | Volumes with load/unload hysteresis around the camera; levels are imported under a root and can get mesh colliders; loaded state survives save/load | `world.streamLevel` |
| Spatial audio | Core `SetAudioGain` (changes gain without restart, keeps the epoch); positional or entity-following voices attenuate with camera distance | `audio.playAt` |

## Engine bugs found by building the showcase

- **Exactly vertical rays against Rapier/parry 0.17 heightfields miss along
  cell boundaries.** The first nav bake produced a striped grid
  (25,600 of 53,824 cells walkable). Fix: `physics::robust_dir` tilts
  axis-vertical rays by 1e-4. The regression test casts 1,600 vertical rays
  with 0 misses, and the nav grid is now fully walkable.
- AI movement was computed but not committed (translation delta lost):
  fixed and covered by the knight test.
- Particle captures were empty because a fresh renderer has no history.
  Fixed with UE-style warm-up to steady state.
- Physically implausible terrain albedos washed the image out; switched to
  linear albedos in measured ranges.

## Evidence

Unit and integration tests: gameplay (effects/stacking/periodic/clamp/
depletion; ability tags/cost/cooldown/cancel), animgraph (blend space,
transitions, montage notifies), terrain (deterministic noise, mesh
winding/normals), crysirror-engine tests/gameplay_ai.rs (a knight perceives,
paths around a wall, faces, and bites on cooldown with stamina accounting;
state machine plus montage through the API), tests/systems.rs (terrain
landing, streaming with hysteresis and save/load, spatial attenuation
without restart, vertical-ray regression), and an explicit GPU test
rendering terrain plus HDR particles.

examples/aaa-systems (Fox + HDR fetched; lantern copied locally): hunters
close from 58 m to 1.4 m over the nav grid, then bite on cooldown; player
health goes 100 → 82 → 52 → 10 while Warmth heals periodically at the
bonfire. dist/aaa-systems.png shows the fight, the bonfire flames, embers
and smoke, the HUD, the streamed shrine and the terrain. Release wall time
for 1,500 simulated frames plus capture is about 1 s; GPU frame time is
not measured yet.

## Not yet done

Motion matching (PoseSearch), IK and root motion; Recast-quality navmesh
with dynamic obstacles; GPU particles, collisions and ribbons; terrain
layers/splat textures and LOD streaming; asynchronous level loading;
HRTF/panning and reverb; a material graph; ability tasks/prediction for
networking.
