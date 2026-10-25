# ADR 0002: a modern 3D engine with independent compatibility frontends

Status: direction accepted for this source iteration; only the offline TS/JS
authoring path below is implemented. The dynamic 3D systems remain planned.

## Product goal

CrySirror is a Rust 3D engine with structured authoring and inspection APIs suitable
for MCP clients. An action game with third-person traversal, animation and combat
is the long-term target. "Like God of War" describes the kind of experience to
support, not current rendering quality, content scale or a compatibility claim.

RFVP and Siglus support is a pair of compatibility frontends on shared engine
services. Their common service needs inform those services, but their VN command
sets must not define the limits of the modern 3D world model.

## Ownership and dependency direction

```text
Editor / MCP       JSON scene/component/behavior data       Optional TS/JS
      |                           |                             |
      +---------- validated project operations <---- authoring compiler
                                  |
                    authoritative logical world [planned]
                    compiled Rust simulation systems
                                  |
Original RFVP VM ---- adapter -----+----- adapter ---- Original Siglus VM
       private values/stacks/waits/saves remain in each original VM
                                  |
                  scoped assets / input / time / audio / saves
                                  |
                render extraction -> wgpu renderer -> desktop host
                                                        winit
```

The current implementation has retained 2D service state and a static `Scene3d`,
not the authoritative dynamic world in this diagram. Add the world as a CPU-only
module/crate when it becomes executable. The `no_std` core stays small and free of
window, GPU and audio SDK dependencies. The renderer consumes extracted frame data
and owns device resources; the desktop owns the window and event loop.

Do not place RFVP/Siglus instructions, TypeScript ASTs, GC objects or GPU pointers
in the shared world or save format. Logical resource handles must carry client
identity and a generation when reusable. Device handles are reconstructed from
logical resources. Original VM numeric handles need explicit per-client maps.

## Data-driven behavior

Scene entities, stable IDs, parent relationships, transforms, asset references,
materials, animation graphs, input actions, state machines, timelines, quests,
combat definitions and tuning belong in versioned project data. Built-in Rust
systems validate and execute that data. Data-driven does not mean executable
behavior disappears: it means the common behavior is implemented once in systems
and configured by authored data. The current `Action` transcript remains useful
for service tests; it is not the final gameplay/scene language.

Unity also uses scripts; CrySirror's preference for data is its own design choice.
Use fixed simulation steps, explicit input sampling and ordered command commits
for the future world. Rendering may interpolate extracted snapshots independently.
Each native VM retains its own tested clock, yield, redraw and blocking behavior;
the modern fixed step must not erase native wait semantics.

## Optional TypeScript and JavaScript

The implemented first profile is **offline authoring**:

```text
.ts / .js -> Oxc parse/semantic checks/TS transform -> JavaScript
          -> Boa execution of synchronous buildProject()
          -> JSON -> existing Project validation -> normal player or MCP tools
```

Both Oxc and Boa are Rust implementations. Oxc is the compiler/transpiler; Boa is
the JavaScript execution engine. TypeScript is lowered to JavaScript, not directly
executed as a distinct TS VM. Oxc's syntax/semantic checks are not TypeScript type
checking. No claim of a Rust `tsc` replacement is made. See the pinned dependencies
and lockfile for the actual versions, rather than assuming the latest releases.

The separate `crysirror-author` crate keeps compiler/GC dependencies out of the
default player and out of core/compat. JSON-only projects need neither the TS
compiler nor the JS engine. No native VM factory is replaced or registered by this
authoring tool. Generated native client specs still require real factories.

Optional **runtime extensions** are a later profile, not yet implemented. They
must emit validated, client-scoped commands at scheduler boundaries, consume
logical snapshots/events, and have explicit persistent JSON state and schema
migrations. Do not serialize the Boa heap or map native VM saves into JS objects.
Do not claim replay or deterministic execution merely because source is hashed:
time/randomness, module resolution, scheduling and external input need defined
policies first. A runtime script error needs an explicit reported failure policy.

## MCP as a first-class control surface

Keep schemas and checked operations shared between CLI/editor/MCP. A future live
session API should expose discoverable capabilities, entity/component inspection,
revision-guarded command batches, asset import jobs, timeline stepping, captures
and trace subscriptions. Mutations enter the scheduler at a defined boundary;
inspectors read a consistent published snapshot. Errors report unsupported
capabilities and validation failures with object/field identity.

MCP should author data and run coarse jobs. Simulation, animation and draw calls
remain local. The existing Python MCP server operates on project files and fresh
capture processes; live sessions, hot reload and runtime script execution over
MCP are not present. Source evaluation is not added as an unrestricted MCP tool.

## Compatibility extraction

Extract implementations only after auditing complete source at the commits in
`upstream-lock.json`, then identifying behavior proven common. Initial candidates
are capability VFS and random-access streams, decoded image/texture resources,
glyph positioning, audio voices/streams, input and time, and save envelopes.
RFVP/Siglus keep their own archive codecs, opcode/value/coroutine logic, text
layout policies, syscall dispatch, transitions and original save codecs. Common
codecs can become reusable libraries only with format/behavior fixture tests.

The current host API is smaller than the audited upstream surfaces: e.g. RFVP
has microsecond ticks, pan/fades/PCM streams and render targets, while Siglus
couples pumping to redraw waits. Do not truncate those contracts into the
existing millisecond clock or simple play/stop commands. Add the required service
capabilities or reject them visibly; no successful no-op adapters.

## Milestones with observable acceptance criteria

1. **Current foundation:** compile/test the existing player, lock complete upstream
   revisions, and execute TS/JS -> validated data -> runtime -> GPU capture.
2. **Dynamic 3D world:** stable entities/transforms and revisioned command batches,
   fixed-step simulation, scene persistence and renderer extraction. Demonstrate a
   moving object with save/restore and the same edits through CLI and MCP.
3. **Asset/render pipeline:** glTF import, asset IDs/revisions, asynchronous loading,
   PBR, lighting/shadows and skeletal animation. Validate CPU transforms plus
   images on documented devices; measure frame time and memory.
4. **Playable third-person slice:** character collision/controller, camera,
   locomotion/animation blending, one enemy, attack/hit/reaction data, spatial
   audio and checkpoint restore. Ship one small original test arena first.
5. **Compatibility slices, independently:** audit complete VM/service call paths,
   link RFVP and Siglus separately, execute legal fixtures and compare native
   waits/resources/text/audio/original saves against the reference engines.
6. **Optional live JS and production work:** versioned script command API,
   observable job budgets, reload/state migration, profiling, streaming/LOD and
   platform packaging. Switch and PS Vita remain deferred.

Native compatibility work can proceed alongside modern-world work. Each step
must record implementation, compilation and actual behavior evidence separately.
