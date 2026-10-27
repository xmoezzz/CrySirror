# Desktop architecture and data ownership

Engine integrations exist to expand CrySirror capabilities. Native VMs must
translate their operations into engine APIs and engine-owned logical state.
Hosting a second engine and uploading its composed screen is prohibited.
The previous RFVP/BGI/RealLive hosting paths were removed on 2026-10-26.

## Modules

| Crate | Role |
|---|---|
| crysirror-io | Shared capability read-at to standard I/O cursor; no codecs or device dependencies |
| crysirror-core | no_std + alloc service contracts, logical state, save envelope, VM port |
| crysirror-compat | Legacy path/behavior profiles and command translation boundary |
| crysirror-host | Original in-memory reference host for contract tests |
| crysirror-cli | Original schema-1 replay CLI, binary crysirror-contract |
| crysirror-runtime | Schema-2 data, capability VFS/assets, factories, scheduler, snapshots |
| crysirror-image | Shared safe CPU surfaces, content/extension decoder registry, scoped dependency/key contexts and bitmap operations; explicit native pixel-math profiles |
| crysirror-resource | CPU archive/file/audio contracts, bounded decode tasks and streams, transform/key/companion capabilities, WAVE containers and VFS mounts |
| crysirror-formats-arc | Optional GPL Rust plugins for 152 archives, 96 images, 16 audio and 25 files from pinned arc_unpacker; explicit partial-format ledger (ADR 0065) |
| crysirror-physics | Persistent Rapier 3D CPU backend; rigid bodies, CCD, joints, queries, event queues and scoped handles (ADR 0045) |
| crysirror-wgpu | GPU uploads, clipped/blended 2D/text, extracted dynamic 3D and readback |
| crysirror-desktop | winit lifecycle/input, audio sink, CLI and persistent state location |
| crysirror-author | Optional Oxc TS/JS compiler + Boa offline project builder; no player dependency |
| crysirror-siglus | Native G00 resource frontend and metadata policies; independent VM lives in crysirror-siglus-vm |
| crysirror-acoustics | Device-free acoustic meshes/materials, default Parry and optional obvhs BVH2, volume visibility/direct propagation and SOFA/HRIR resources with worker-prepared convolution, standalone FunDSP late-reverb DSP, and worker-prepared first-order image-source paths/sparse echoes (ADR 0067/0068/0073/0074/0075) |
| crysirror-audio | Engine voice scheduling/buses, shared canonical SceneState bridge, worker-owned buffered decoders, exact seek completion and client reverb bus sends/returns; worker-owned final DSP and PcmOutput lifecycle, optional CPAL and compatibility rodio output |
| crysirror-text | Engine CPU fonts, shaping/bidi, rich layout/ruby/vertical text, fixed-cell layout, synchronous font-resource measurement handles, reveal and CPU SDF data |
| crysirror-scene | Safe CPU World/Layer/UI state, migrated input/animation/projection, transactional mutations and frame extraction |
| crysirror-reallive | Original RealLive bytecode VM and thin semantic/syscall adapter; retained Canvas/Layer/GlyphRun/UI and engine audio/Media services, no hosted renderer |
| crysirror-krkr | Optional AGPL KiriKiri adapter: vendored Kirakira TJS2 VM (crates/krkr-upstream) with native classes translated onto engine Layer/Canvas, clock and storage (ADR 0053); not in default builds |
| crysirror-avg32 | Original AVG32 Scenario library at a pinned siglus_rs Git revision, portable-vm only; semantic events call engine Canvas/GlyphRun/UI/audio, without the old System/renderer (ADR 0062) |
| crysirror-siglus-vm | Independent original SceneVm subset registered as `siglus`; native operations call shared engine services, including the explicit plain TEXT/NAME profile (ADR 0061) |
| crysirror-engine | Unity-style public API: reflective classes/objects, App lifecycle, behaviours, plugins, ScriptHost contract (ADR 0004/0005) |
| crysirror-script-js | Default JS/TS script host (Boa + Oxc) bound to the class registry; desktop feature `scripting` (ADR 0005) |
| crysirror-asset | Device-free importers; glTF 2.0/GLB/VRM, binary FBX and PMX (converted to GLB in memory) into world entities and PBR models (ADR 0007, 0054) |

Layering for engine users (ADR 0004): `App` (crysirror-engine) owns a `Runtime`
and drives it from lifecycle hooks. Rust behaviours, the JS/TS host and future
tools reach every service through one class registry. Native VMs stay
`VmAdapter` peers inside the runtime and appear as `Client` objects. Filesystem
providers mount per client (ADR 0006). Decisions and the requirement backlog
are in [docs/adr](adr/README.md).

The core has no external crate dependencies. Runtime can execute traces without
creating a window/device. The renderer depends on the shared runtime's asset
store and scene description, not on original engine VM internals. The desktop
frontend alone creates windows and surfaces.

Offline native scene frontends translate title-specific formats into glTF/PNG
and shared project data (ADR 0017). Generic bounded topology decoding lives in
crysirror-asset::indices; DDS decoding lives in crysirror-image. Both are safe CPU
code. Client systems can initialize `environment` data through the existing
validated World command; project cameras and scene entities need no JS/TS boot
script. Native bind/animation semantics remain the importer's responsibility.

[ENGINE_SYSTEMS.md](ENGINE_SYSTEMS.md) is the engine-first ownership specification
for Audio/SFX, Text, UI, Layer and World. Audio/text implementations now live in
engine crates. CPU font resources belong to Runtime Assets; desktop and wgpu
only connect the services to lifecycle and output. World/Layer/UI now execute in
crysirror-scene and are installed, ticked, extracted and saved by Runtime.

The modern 3D product direction and data/VM/optional-script ownership decisions
are in [ADR 0002](adr/0002-modern-engine-and-optional-typescript.md). The executed
TS/JS authoring profile is documented in [AUTHORING.md](AUTHORING.md). Compiler
and JS execution dependencies are confined to the optional authoring crate;
JSON-only projects run without them. Dynamic 3D systems now use ClientSpec.systems;
per-frame JS remains pending. Existing original VM contracts retain their roles.

[ADR 0003](adr/0003-siglus-monorepo-and-rendering-capabilities.md) records the
monorepo extraction, executed Siglus resource comparison, desktop platform scope,
and compute/hardware ray tracing capability split. Shared runtime and renderer
code dispatches through image services; it does not own G00 layout or VM policy.

## Canonical state

[ADR 0066](adr/0066-awaitable-services-and-native-plugin-facades.md) adds bounded
Text/Resources/Save preparation and JS/TS Promise subscriptions. CPU/I/O workers
own immutable inputs/results; the engine owner publishes surfaces and atomically
commits prepared saves. Native VM checkpoints remain ordered. Shared UI menus
own their logical trees, input and paint; native MenuItem handles are adapters.
See [ASYNC_SERVICES](ASYNC_SERVICES.md) for ticket lifetime and migration rules.

Shared retained service state is logical: namespaced IDs, asset paths, text,
rectangles, tint and voice settings. One `(ClientId, local_id)` pair identifies
a visual or voice. Each client has a capability directory under the asset root;
project fonts use a separate namespace. A relative symlink cannot intentionally
route a client's asset read into another client's root through this API.

Per-client save/cache roots use adapter and game identity. Two instances of the
same adapter/game intentionally see that game's persistent storage. Object and
asset cache identities are instance-scoped. No arbitrary native Rust module is
sandboxed by these conventions: linked code remains trusted.

Project.scene3d provides a base camera/mesh scene. ClientSpec.systems adds a
dynamic typed World, retained Layer and UI. They live in EngineSystems, separate
from the old SceneState command enum. `Require3d` in that old contract still
rejects unsupported legacy service access. Rapier physics/character control, glTF skins/animation and gameplay systems are
covered by ADR 0012/0014. There is no general query ECS or implicit native-VM
translation.

## Scheduling and input

The desktop converts physical window input to logical canvas coordinates and
an InputFrame. Runtime routes retained UI first (consuming owned button input),
updates World on fixed 16 ms steps independently of VM halt, advances its
millisecond clock and calls each VM
with an instruction budget. The data VM batches service operations until a
wait, halt or budget boundary. Failed batch submission leaves its PC unchanged.
One input edge cannot pass two sequential waits in the same VM tick.

Service batches are atomic at the SceneState boundary. A complete frame across
multiple independently executing VMs is **not** a global transaction. A fatal
VM error terminates the current host run rather than claiming complete rollback
of earlier sessions or filesystem writes.

## Rendering

wgpu owns the GPU pipelines and caches; it is independent of winit. Desktop
owns the event loop/window. The PBR raster path imports glTF, shades in linear
HDR with IBL and cascaded sun shadows, and applies SSAO, TAA, bloom and tonemap.
Shared retained Layer/UI/Text overlays composite afterward. Readback produces
RGBA8 sRGB PNGs through the same renderer used for desktop frames.

Material data lives in safe CPU `crysirror-scene::material`, shared by projects,
Rust commands, `Entity.setMaterial`, snapshots and importers. String texture paths
are client-scoped; model materials use logical image indices. Opaque/mask and
transparent draws have separate depth/shadow policies. Sorted alpha/additive/
premultiplied draws composite against read-only opaque depth **after opaque TAA
and before bloom/tonemap**, without depth writes or shadow casting. Unlit,
independent opacity, shared UV0 transforms and soft depth intersections are
engine features. A versioned engine-owned glTF extras envelope carries those
fields from external importers. Source-specific material conversion stays in
tools/profiles. See ADR 0015 for the contract and measured behavior; center
sorting, single-sample transparency and source shader approximations remain limits.

Preset/custom shader effects extend that shared material data (ADR 0016).
WGSL source is a separate client code asset; projects/saves retain a path and
parameters. ABI v1 supplies surface inputs and accepts color/opacity, emission
and PBR factors. Color/depth/shadow variants run the same hook. Naga validates
all assembled stages on the CPU; renderer pipeline errors carry source context.
The shader-check CLI and MCP shader_validate do not create a GPU. Custom modules
cannot declare entry points or global resource bindings. Vertex/post/compute
replacement, hot reload and global/volumetric fog remain outside this interface.

The optional compute-raytrace path traces primary and directional shadow rays
against the extracted transformed triangles and composites the same overlays afterward.
Native hardware ray queries are a separate capability, negotiated through actual
Vulkan/DX12/Metal adapter features and limits (ADR 0051). The renderer builds
BLAS/TLAS for opaque triangles and offers simple primary/hard-shadow rays or
hybrid PBR sun visibility. PBR uses the same world transforms, current skin pose
and material ownership; immutable CPU snapshots are prepared on bounded workers.
The AO/visibility texture preserves the existing material binding budget.
Changed positions rebuild the AS, material metadata updates do not, and removed
geometry clears it. Only successful submission commits the cache build state.
Since ADR 0055 BLAS read the renderer's own GPU vertex/index streams, skinned/morphed instances own GPU-skinned dynamic BLAS, MASK casters are alpha-tested by re-tracing against a baked alpha atlas and blended surfaces do not cast; custom fragment-discard shaders fail visibly.
ADR 0060 adds GGX glossy reflections, per-pixel single-bounce diffuse GI and
local-light shading at reflection/GI hits. Reflection and pixel irradiance
share a two-layer array at the existing SSR sampled binding. Probe updates
read an immutable history copy to avoid cross-workgroup write/read races.
Hit lighting requires six storage-buffer bindings, negotiated against actual
device limits. This remains an opaque/alpha-tested approximation: hit shading
uses the material atlas and geometric normals; dedicated glossy/GI denoising,
transmission and full path tracing are not implemented.
Raster/compute remain independent selections and need no experimental RT opt-in.
Selection is explicit in CLI/host state; a VM/save never stores device-specific
handles. Native API eligibility, enabled device features, encoded commands and
physically verified pixels are separate evidence. See ADR 0003 for the bounded
compute prototype, ADR 0051 for native RT limits, and STATUS.md for actual GPUs.

The legacy service text contract accepts simple-horizontal-v1 and an explicit
unwrapped variant; synchronous immutable font measurement handles share those
metrics (ADR 0063). The newer engine
Text API uses shared shaping/bidi, rich styles, ruby/vertical layout and reveal
to publish client-scoped RGBA surfaces (ADR 0031/0033). CPU SDF atlas data exists;
its GPU/3D sampling path remains pending. Native windows require explicit
mapping: Siglus supports bounded plain TEXT/NAME through shared grid layout,
GlyphRun and reveal state, while the independent AVG32 portable VM maps its
documented plain ADV slice (ADR 0061–0062). Unmapped native page/read/wait/save
policies fail visibly. See NATIVE_ADAPTERS.md for executable coverage.

Textures and assets are bounded. Unsupported dimensions/formats/layouts report
errors. Scoped image/model reads and imports now run asynchronously on engine workers
(ADR 0049). Demand-paged streaming and device-loss rebuild beyond surface
reconfiguration remain unimplemented. These are implementation boundaries,
not whole-engine performance claims.

## Save and restore

Runtime snapshots serialize shared service state and each VM's private bytes.
Restore validates envelope/identity, EngineSystems and registered client scopes,
checks render extraction, reconstructs a new scene and every new VM,
and swaps live state only after all stages succeed. Factories must construct
dormant VMs without filesystem/audio/device side effects. Original native VM
implementations must honor that contract before registration.

The save identity hashes serialized project data, including content_revision.
It does not hash every external asset. Bump content_revision when asset semantics
change. GPU handles, font override contents, native device state and audio sample
positions are not serialized. Audio restarts at sample zero after restore.
CRC is corruption detection, not authentication. Native save formats are not
imported or claimed compatible.

## AI boundary

The Python stdio MCP server operates on one selected project. Read-only by
default, it optionally validates and atomically replaces project JSON with an
expected-hash guard, or invokes a configured executable for capture/packaging.
This is a single-writer tool, not cross-process compare-and-swap. Captures start
a new process; they do not inspect a live native VM. There is no shell tool.

An eventual live transport should use the same validated runtime operations,
not expose arbitrary memory writes or parallel bypasses around the scheduler.

## RealLive VM API bridge (2026-10-26)

The new `crysirror-reallive` dependency direction is VM/semantic adapter to
shared engine services. It does not restore the removed RealLive hosted engine.
The source pin and excluded engine modules are in
[`SOURCE.json`](../crates/crysirror-reallive/src/vm/SOURCE.json). Original
bytecode/value/call semantics stay in the VM; pending Command/Text events are
acknowledged only after the corresponding engine calls succeed.

Shared capabilities added for this bridge are usable without RealLive:

- `CanvasSystem` owns logical drawing surfaces and immutable copy-node
  snapshots. Presentation expands them into ordinary Layer nodes. This does
  not consume a scene composed by another engine's framebuffer renderer.
- `Paint::GlyphRun` stores explicit positions and styles; shared `GlyphReveal`
  and TextSystem own reveal state, glyph metrics and cached glyph rendering.
- Shared UI widgets/events own choices, editing, title and cursor requests.
- Shared native audio sources and channel/mixer services own decoding/output.
- `MediaManager` owns scoped incremental MPEG workers, bounded queues, media
  clocks and errors. Each decoded picture updates a distinct media texture;
  a video Layer is one engine scene object, not the entire native scene.
- Public Canvases/Media classes and existing Layer/UI/audio APIs reach the same
  services as native syscalls. Client-scoped queries expose the logical state.

The adapter retains native DC numbers, resource naming, grid/markup and
wait/selection policies, rather than a second general-purpose engine. Unsupported
operations fail instead of reviving a private engine implementation. Native
save files/snapshots, DLLs, complete-game behavior and other platform/device
results remain outside current claims. See [ADR 0048](adr/0048-reallive-vm-engine-apis.md)
and [REALLIVE.md](REALLIVE.md) for current limits and separated evidence.

## Multithreaded tasks and asynchronous loading (2026-10-26)

`crysirror-tasks` owns bounded worker pools and wakeable Rust task Futures.
Assets captures only scoped storage, immutable bytes and CPU decoders for its
background reads/imports; the owner thread publishes completed resources.
Desktop image/model rendering defers pending resources. Offline capture waits
explicitly. PBR character poses run in bounded parallel batches, with results
collected in stable instance order before owner-thread GPU writes. World, VM,
script and fixed-step physics state retain their ordered execution.

See [ADR 0049](adr/0049-multithreaded-tasks-and-async-assets.md) for cancellation,
backpressure, revision invalidation, API usage and remaining synchronous paths.

## Engine-wide asynchronous execution requirement (2026-10-26)

[ADR 0050](adr/0050-engine-wide-async-execution.md) is the accepted target for
all engine subsystems, including World, physics, rendering, text/UI, audio/video,
scripts/native VMs, save/restore and MCP. The current worker pools and asset/pose
paths are stage one. Present owner-thread limitations are migration work, not
permanent exemptions. Thread affinity and native ordering remain explicit graph
dependencies/commit boundaries. Validation is local; GitHub CI was removed at
the owner's request and must not be restored without explicit authorization.

## Frame pipeline, profiling and streamed cells (2026-10-26)

[ADR 0052](adr/0052-frame-pipeline-streaming-and-profiling.md) adds a bounded
dependency graph for immutable PBR/simple/layer extraction. Desktop polls one
pending snapshot and retains one ready snapshot while owner simulation advances.
Restore and resource-store identity invalidate stale snapshots. Simulation/VM
ordering remains on its owner; this is partial progress on ADR 0050.

Streaming cells prepare entity hierarchies and PBR resources on workers, publish
at most two ready cells per frame, and use resource leases for cancellation and
unload. Offline tools have an explicit readiness fence without another logical
tick. GPU uploads and collision cooking still need asynchronous budgets.

The shared Profiler exports bounded CPU traces, stage percentiles and optional
completed GPU durations with separate clocks. Native PBR RT now shares local
mesh BLAS across TLAS instances and retains unchanged CPU geometry. See
[FRAME_PIPELINE.md](FRAME_PIPELINE.md) for public showcase commands and limits.

## Modular visual effects (2026-10-26)

`crysirror-vfx` owns reusable effect definitions, compilation, deterministic CPU
simulation and GPU tick planning. `VfxRuntime` schedules bounded worker jobs and
publishes immutable products into the existing render-extraction DAG. The wgpu
backend owns resident GPU simulation, submission receipts, per-view culling,
sorting and Sprite/Mesh/Ribbon drawing; it no longer simulates legacy CPU
particles. Data projects, JS/TS `Vfx`/`Entity` APIs and future VM adapters use
these same resources and operations. Interactive updates poll completions;
offline capture has an explicit clock-preserving readiness fence.

[ADR 0064](adr/0064-modular-vfx-cpu-gpu.md) specifies typed modules, event routing,
budgets, timeline backpressure and remaining Niagara gaps. GPU capacity and
submission work are not reported as measured live counts. Save/restore retains
declarations and resets derived visuals rather than claiming exact particle
checkpoint persistence. No Unreal runtime or source was copied or hosted.

## Native resource conversion and scheduling (2026-10-26)

The optional arc resource plugin now supplies every named decoder in the pinned
inventory through shared archive/image/file/audio registries. It contains native
layouts and codecs, without hosting another engine. `crysirror-resource` owns
key aliases, transforms, bounds, companion capabilities, WAVE serialization and
`ResourceTasks`; `crysirror-image` owns masks/pixel operations and PNG output.
`Resources.beginDecode` submits immutable inputs to the bounded engine CPU pool;
owner polling and VFS publication retain client/profile/resident ownership.
Offline synchronous conversion is rejected in realtime hosts. Lazy mounted
entries still use normal async Assets loading. See [ADR 0065](adr/0065-native-resource-codecs-and-async-decode.md)
and [RESOURCES](RESOURCES.md) for boundaries and the reviewed partial-format ledger.


## Tracked acoustic reflection plans (2026-10-27)

[ADR 0075](adr/0075-tracked-reflection-plans.md) extends captured echoes with
bounded automatic source/listener tracking after entity/camera updates. Workers
prepare immutable geometry/tap snapshots; the mixer owns fixed-capacity sample
history and two-plan crossfades. Initial short sources wait for HRTF then echoes.
Scene/latency changes mute obsolete taps until a compatible replacement arrives.
Scopes, completion, fair admission, coalescing, residency leases and source-EOF
versus echo-drain boundaries are explicit; final mixing/output remains owner work.


## Shared source transport and Siglus BGM (2026-10-27)

[ADR 0076](adr/0076-shared-audio-transport-and-siglus-bgm.md) adds exact-frame,
paused and crossfaded channel publication plus source/output-clock inspection
and scheduled resume. Workers prepare sources; the owner publishes an already
prepared cursor; the mixer owns samples, delay counters and transport fades.
Native Siglus BGM translates those operations through HostServices while retaining
its original VM command until preparation commits. Gallery flags and native wait/
save/value policies remain VM state, without a second audio engine. Restore queues
derived declarations after candidate commit; source/device history is not saved.

## Prepared audio resources (ADR 0077)

Native/media ChannelAudio and the modern Audio API share AudioResources:
client-scoped immutable encoded snapshots, owner-published preparation receipts,
and independent worker-created decoder leases. Releasing a handle blocks future
lookup; admitted/live playback keeps encoded storage charged until its last lease
drops. Restore clears capabilities without resetting outstanding byte charges.
Core owns only plain contracts. No resource handles or decoder/device pointers
enter VM saves. Siglus stopped/EOF replay now submits the same PlayResource API;
stopped snapshot reconstruction prepares a resource without starting a voice.
See [ADR 0077](adr/0077-prepared-audio-resources-and-native-replay.md) for budgets,
read races, metadata limits and remaining native/async boundaries.

## Audio control hierarchy and native SFX (ADR 0078)

Mixer owns bounded client-scoped named gain groups, compiled parent ordering and
output-clock interpolation. VoiceSpec/AudioParameters only carry logical group
names; publication resolves dense indices. Voice, group, fixed bus and Master
remain separate gain stages. Shared late-reverb history belongs to its return
bus after source/group sends, not to a native player. Source positions and
publication age in output frames are distinct reusable observations.

Native PCM/global SE translates opcode/value/resource/wait/save policy onto the
same resource, channel and group APIs. Cross-category semantic queues retain
original call order; workers never own a borrowed VM/host. Siglus runtime payload
schema is now 4 (ADR 0081); original file layouts are unchanged. PCMCH, full
KOE/MWND, exact legacy timing
and an independent audio/device clock remain open. Final mix/output workers
are covered by ADR 0079/0080. Details: ADR 0078/0081.

## Worker final audio frames (ADR 0079)

Live App and plain/scripted desktop coordinate final pooled/canonical and
native/media DSP through shared bounded MixFrame jobs. Workers exclusively own
mixer histories; owner slots retain readonly published observations. Completion
restores matching leases and publishes combined PCM once. Admission backpressure
does not repeat simulation/input; restore/clear cancels obsolete audio. Public
Audio.rendering includes prepared frames awaiting capacity. Runtime.begin_tick
and App.begin_frame expose nonblocking completion; their synchronous counterparts
remain explicit offline paths. See [ADR 0079](adr/0079-worker-final-audio-frames.md)
for ownership, cancellation and the still-synchronous device queue/lifecycle.

## Device worker and PCM transfer (ADR 0080)

The modern desktop PCM endpoint opens/starts/destroys a CPAL stream on a dedicated
worker. Shared bounded stereo packets, epochs and callback signals replace the
CrySirror PCM mutex/refill buffer. OutputWatch reports readiness, retained failure
and actual worker close completion; platform resources never enter VM/project
state. Desktop gates startup time/input on device opening and polls failures even
without new PCM. Explicit retry isolates sessions. CPU fake backends and a real
silent-level device probe exercise the same endpoint. Final DSP remains frame-
dependent; CPAL internals, clock independence and portability limits are in
[ADR 0080](adr/0080-async-output-worker-and-bounded-pcm-transfer.md).


## Prepared voice processing (ADR 0081)

Shared audio resources can bake the bounded Siglus JITAN PCM16 profile on their
preparation worker through AudioSourcePrepareSpec.transform. Core declares the
transform; crysirror-audio owns capability reads, decoder layout, PCM conversion,
processing, WAVE bytes, memory charges and resource publication. Percent 100
preserves the original encoded snapshot. Modern scripts and original native
channels use independent decoder leases from the same bank.

The Siglus global KOE/EXKOE adapter retains only native identity/config/wait/save
policy and up to 24 engine resource handles. Native snapshot schema 4 rebuilds
these derived resources; original save-file layouts are unchanged. This profile
requires explicit koe/{number} aliases. MWND, mouth tables, full routing/ducking
and native archive discovery remain open; see ADR 0081 and NATIVE_ADAPTERS.md.
