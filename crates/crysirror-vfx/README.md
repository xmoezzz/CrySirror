# CrySirror VFX core

This MPL-2.0 crate is an independently implemented, safe CPU VFX service. It
contains no Unreal code, window, GPU, image painter, audio output or filesystem.
Authored data is a `SystemSpec`: multiple named emitters, typed user parameters,
spawn and update operations, renderer descriptions and event handlers.

`SystemSpec::compile` validates types, references, resource paths, numerical
ranges, limits and backend support. `CompiledEmitter::specialize` folds typed
bindings once per tick while keeping the authored plan available for GPU replay.
Unknown serialized operations and mismatched parameter updates fail visibly.

## Simulation and ownership

The owner supplies `SystemInstance { key, client, transform, spec }`. A key is a
logical lifetime identity, never a device pointer. Client authority is explicit
and cannot change on a retained identity. `CpuWorld` owns state and is movable
to an engine worker. A job advances the world and returns a `FrameSnapshot`;
published particle arrays are immutable `Arc<[Particle]>` resources.

`advance` accumulates supplied milliseconds and executes fixed ticks. Excess
time remains pending; `advance(inputs, 0)` drains it under the same bounded work
budget. No renderer cadence or implicit steady-state warmup changes the result.
Explicit warmup consumes the same tick budget. Initial time-zero bursts run an
initialization phase without aging/integration; GPU initialization uses `dt=0`.
Burst looping is separate from continuous-rate looping, so a legacy one-shot
burst can coexist with indefinitely running emission.
Disabling stops automatic and child emission while existing particles finish.
Pausing freezes the simulation clock. Parameter, control and transform edits
retain particles; structural template edits and `SystemSpec.revision` restart
the affected state. Local particles are transformed during rendering; world
particles retain their world-space trajectories when an emitter moves.

All CPU and GPU capacities count towards one reservation budget before any
particle pool is allocated. CPU event/deferred-event queues are bounded, with
drop counters. Overflow policy is explicit: drop new births or replace oldest
on CPU. Particle IDs are monotonic; the pure integer random oracle is keyed by
seed, ID and dimension, independent of allocation order.

## Module execution

Birth uses point, sphere surface/volume, box, cone or line sampling, randomized
lifetime, velocity cone, rotation, color and size. Force operations execute in
authored order, followed by integration, collision responses and appearance
curves. Forces after collision responses are rejected at compilation. Forces
include acceleration, exponential drag, a seeded analytic curl field, radial
attraction and orbiting. Curves support constant, linear and smoothstep
interpolation with checked normalized keys. Analytic plane/sphere collisions
can reflect velocity or kill particles and publish collision/death events.
These are point-particle analytic collisions, not a complete scene-geometry or
physics data interface.

CPU birth/death/collision handlers can spawn another emitter at the parent
event's position, converting local/world coordinates and inheriting velocity.
Child birth chains are deferred to the next tick, so cycles cannot recursively
consume an unbounded frame. Stopped targets receive no new children. Events
carry both system and emitter identities.

`Particle.size` is full width/height in world units. The renderer owns Sprite,
static Mesh and Ribbon interpretation. Ribbon width is an explicit full width,
independent of size-over-life. Asset paths are logical client-relative names;
the core never opens them or assumes that a named resource exists.

## GPU boundary

GPU emitters do not run CPU particle loops. Their owner produces immutable
fixed-tick birth commands including historical parameters and transforms.
The renderer owns disposable device state. A `GpuProgress` receipt changes
only after actual queue submission; it is neither authored data nor save data.
The runtime retains unsubmitted commands under a bounded backlog policy. The
core emits at most 32 newly planned GPU commands per emitter per advance,
including a zero-time initialization command. A missing device history must
cause an explicit reset/diagnostic, never a claim of continuous simulation.

`validate_gpu_profile` is shared by the core compiler and backend. GPU child
emitters, GPU ribbon history and GPU replace-oldest allocation are explicitly
unsupported in this slice. Choosing CPU provides the supported child/ribbon
path. GPU event-buffer readback is an offline/debug operation; it is not
silently presented as a complete asynchronous CPU event interface.

This is a modular VFX foundation, not complete Niagara compatibility. A visual
graph editor, UE cooked-program execution/import, custom HLSL, arbitrary data
interfaces, skeletal-mesh sampling, scene distance fields and fluid simulation
are not implemented by this crate. GPU state is not an exact cross-device save
format. Implementation and device evidence are recorded separately in STATUS.
