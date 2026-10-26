# 0064: Engine-owned modular VFX on asynchronous CPU and resident GPU paths

Date: 2026-10-26. Status: implemented profile; validation recorded separately.
Replaces the renderer-private simulation of ADR 0014. Extends ADR 0050/0052.

## Decision

CrySirror owns a modular effect system rather than importing or hosting another
engine. Niagara is a capability/authoring reference; no UE source, runtime,
bytecode, proprietary SDK or cooked effect is included. Effect definitions are
serde data. JS/TS and native semantic adapters call the same engine APIs.

`crysirror-vfx` is a safe, device-independent crate. `SystemSpec` holds multiple
named emitters, typed float/vector/color/bool parameters, fixed-step timing,
spawn schedules, initialization, ordered update modules, render declarations
and CPU event routing. Compilation checks names, types, curves, bounds,
resources and explicit backend support before opening a device. Structural
changes reset affected emitters; parameter/control/transform edits retain live
particles. A revision change explicitly restarts the whole instance.

The public API is `Vfx.compile/play/setParameter/pause/stop/restart/stats/
drainEvents`, plus `Entity.vfx/setVfx/setVfxParameter/pauseVfx/stopVfx/restartVfx`
and `WorldCommand::SetVfx`. Ordinary scoped, generational entities own instances.
The same `EntitySpec.vfx` works in data projects without scripting or a VM.
Legacy `EntitySpec.particles` is lowered to this system. Its one-shot burst,
radius-to-full-size conversion, continuous emission and explicit prewarm stay
in a small semantic conversion; the old renderer CPU simulator is deleted.
An entity cannot configure both components simultaneously.

## Ownership, tasks and completion

The runtime creates lifetime identities from client/entity/generation. It moves
one `CpuWorld` into the shared bounded CPU worker pool, captures immutable input
specifications/transforms and publishes an immutable `Arc<FrameSnapshot>` only
after completion. Live frames call nonblocking `try_take`; admission failure
returns ownership and retains elapsed time for retry. The renderer never ages
CPU particles. Render extraction captures the published product as a dependency
alongside the frozen World snapshot. Hidden effects continue simulation, while
view visibility controls draw submission. Client pause freezes simulation.

The owner journals changed parameter/control/transform samples while a worker
is busy. Adjacent identical intervals can coalesce; distinct temporal boundaries
are processed in order after the preceding fixed-step debt. The input journal
is bounded to 2,048 samples and an estimated 16 MiB. Exhaustion is an explicit
producer-throttling error, never silently replacing intermediate script frames
with the newest transform. This also preserves moving trails during fast
offline ticking, independently of worker scheduling.

Each system accumulates fixed-step debt under a bounded step budget, independent
of rendering cadence. Zero-time bursts initialize particles without aging them.
Warmup is authored work under the same budget. Stop suppresses new automatic and
child births while live particles finish; pause freezes their clock. Logical
seeds and monotonic IDs drive a shared integer random oracle, independent of
free-slot allocation order. CPU workers can move between threads.

GPU-target systems only plan ticks on CPU; there is no CPU particle simulation
or live GPU readback. Historical tick packets carry transforms, parameters,
spawn IDs/counts and simulation time. The owner retains unsubmitted packets
under a 65,536-packet / conservatively estimated 32 MiB history budget. It
reduces planning work or applies visible backpressure when the backlog fills;
it never drops a simulation step to keep rendering. The core's small rolling
window is merged into this owner history before the next job.

The wgpu backend owns resident 96-byte particle records, active indices, event
records and indirect arguments. Update/death, free-slot scan, block prefix,
parallel birth initialization and compaction execute in explicit queue order.
The free-slot prefix allocator chooses lowest free slots deterministically.
Each encoded batch is limited to 32 ticks and 200,000 birth requests. Pending
tokens can be cancelled before submit; submitted tick receipts advance only
after `queue.submit`. Cross-simulation tokens and time rewinds fail visibly.
Direct PBR encoding rejects a second encode while its VFX batch is pending:
submit and acknowledge the first encoder, or abandon it and explicitly cancel.
The ordinary Renderer owns this protocol and cancels only an encoder left by
its own preceding failed frame.
Each view separately culls and sorts resident particles, even without a new
simulation tick. Sort/cull and draw remain on the GPU.

`Runtime.finish_vfx_offline` is the explicit CPU completion fence used by tests,
still capture and movies. It advances no World/VM/input clock. Still capture
submits enough bounded batches to catch the GPU timeline up to the frozen
snapshot, then performs ordinary temporal warmup. GPU readback and timestamp
completion waits occur only in explicit offline diagnostics and tests.
Desktop F12/end-of-run capture reuses the live renderer, including its resident
GPU state. Both RGBA and BGRA sRGB targets produce RGBA readback images.

## Implemented capability profile

| Area | Implemented |
|---|---|
| Scheduling | Fixed ticks, fractional rate, timed bursts, separate burst looping, duration/loop, explicit warmup, pause, stop, restart, time scale |
| Birth | Point, sphere surface/volume, box, cone, line; randomized lifetime/speed/cone/rotation; typed color and size bindings |
| Update | Acceleration, exponential drag, analytic curl noise, attraction, orbit, rotation, lifetime color/size curves with constant/linear/smooth interpolation |
| Space | World/local, current transform, nonuniform scale, correct world-space CPU event coordinates |
| Collision | Analytic plane and sphere, bounce/friction/kill and bounded event records |
| Events | CPU birth/death/collision child emitters, inherited velocity, probability, next-tick deferred chains and bounded queues |
| Sprite | Client-relative texture, flipbook, camera/velocity/axis facing, depth-based soft intersections, fog |
| Mesh | Instanced built-in Cube/Quad or client-relative static glTF geometry, explicit texture/color |
| Ribbon | CPU particle-ID/ribbon-ID history into real triangle bands, UV scale, width, segment limits |
| Rendering | Alpha/additive/premultiplied/multiply, per-emitter transparent depth sort, view culling, GPU indirect drawing |
| Budgets | Shared admission: 200k reserved particles, 256 emitters, 128 systems; CPU drop-new/kill-oldest; bounded event/command history |
| Inspection | Actual CPU counts, pending worker/timeline pressure, dropped events, draw/cull/resource counters and GPU submission counts; no fabricated live GPU count |

All texture/model access uses the existing client-scoped Assets service and its
asynchronous readiness behavior. GPU objects never enter project or save data.
The VFX draw pipeline has independent resource layouts, so it does not consume
additional bindings in the existing 16-texture PBR material stage.

Python/MCP project validation mirrors this authored schema, including typed
parameter bindings, curve/module constraints and rejected GPU profiles. Asset
inventory includes VFX textures/static mesh paths from initial entities and
widget spawn/set-VFX actions, checked against each client's capability root.
These tools validate declarations; they do not simulate particles or replace
the Rust compiler/device capability checks.

## Explicit remaining work toward Niagara parity

This profile is not complete Niagara equivalence or `.uasset` compatibility.
There is no visual node editor, arbitrary custom HLSL/WGSL module graph,
skeletal/texture/grid/physics-world data interface, distance-field collision,
fluid solver, effect-level LOD/significance scheduler, or cross-emitter GPU
event spawning yet. GPU ribbon history and replace-oldest allocation fail at
compilation; choose the supported CPU targets for those effects. GPU event
records are available to offline diagnostics, not live host callbacks.

Mesh particles currently render static unlit geometry with texture/color, not
full per-particle PBR/skinning/morphs. Transparent effects are sorted within and
between emitters, not jointly with all ordinary scene transparency. Ribbons
use explicit band width rather than sprite size-over-life. Particles are not
yet captured by reflection/GI probes or ray-tracing acceleration structures.

Canonical saves retain effect declarations and parameters. Simulation buffers
are derived state: restore resets/re-prewarms CPU/GPU visuals. Exact particle
checkpoint persistence and cross-device restore are not implemented. GPU state
is currently renderer-owned rather than shared by a device-level multi-view
simulation broker; a replacement renderer can only explicitly diagnose/restart
from retained history, and cannot reconstruct a paused, fully retired history.
This limitation must remain visible rather than claiming an exact checkpoint.

## Validation

Implementation, compilation, CPU/API tests, actual GPU tests, integrated demo
captures and performance measurements are recorded separately in STATUS.md.
Ignored device tests do not count as passing hardware validation. The public
DamagedHelmet/HDR showcase and our procedural texture are reproducible through
`examples/aaa-vfx/README.md`; downloaded/copied assets remain ignored.
