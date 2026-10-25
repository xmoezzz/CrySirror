# Desktop architecture and data ownership

Engine integrations exist to expand CrySirror capabilities. Native VMs must
translate their operations into engine APIs and engine-owned logical state.
Hosting a second engine and uploading its composed screen is prohibited.
The previous RFVP/BGI/RealLive hosting paths were removed on 2026-10-26.

## Modules

| Crate | Role |
|---|---|
| crysirror-core | no_std + alloc service contracts, logical state, save envelope, VM port |
| crysirror-compat | Legacy path/behavior profiles and command translation boundary |
| crysirror-host | Original in-memory reference host for contract tests |
| crysirror-cli | Original schema-1 replay CLI, binary crysirror-contract |
| crysirror-runtime | Schema-2 data, capability VFS/assets, factories, scheduler, snapshots |
| crysirror-image | Shared safe CPU surfaces, content/extension decoder registry, scoped dependency/key contexts and bitmap operations; explicit native pixel-math profiles |
| crysirror-resource | CPU archive registry, random-access streams, decode budgets, client-scoped transform/key contracts and VFS mounts |
| crysirror-formats-arc | Optional GPL Rust archive and 96 native image plugins from pinned arc_unpacker; explicit per-format coverage inventory (ADR 0044/0046) |
| crysirror-physics | Persistent Rapier 3D CPU backend; rigid bodies, CCD, joints, queries, event queues and scoped handles (ADR 0045) |
| crysirror-wgpu | GPU uploads, clipped/blended 2D/text, extracted dynamic 3D and readback |
| crysirror-desktop | winit lifecycle/input, audio sink, CLI and persistent state location |
| crysirror-author | Optional Oxc TS/JS compiler + Boa offline project builder; no player dependency |
| crysirror-siglus | Native G00 resource frontend and metadata policies; independent VM lives in crysirror-siglus-vm |
| crysirror-audio | Engine voice scheduling/buses and AudioDevice boundary; optional rodio output |
| crysirror-text | Engine CPU fonts, shaping/bidi, rich layout/ruby/vertical text, reveal and CPU SDF data |
| crysirror-scene | Safe CPU World/Layer/UI state, migrated input/animation/projection, transactional mutations and frame extraction |
| crysirror-siglus-vm | Independent subset port registered as `siglus`; full vendored SceneVm host/factory switch pending (ADR 0009) |
| crysirror-engine | Unity-style public API: reflective classes/objects, App lifecycle, behaviours, plugins, ScriptHost contract (ADR 0004/0005) |
| crysirror-script-js | Default JS/TS script host (Boa + Oxc) bound to the class registry; desktop feature `scripting` (ADR 0005) |
| crysirror-asset | Device-free importers; glTF 2.0/GLB (Blender exports) into world entities (ADR 0007) |

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
Hardware ray tracing is a separate unsupported capability. Selection is explicit
in CLI/host state; a VM/save never stores device-specific handles. See ADR 0003
for the bounded prototype's limits and actual Metal execution evidence.

The legacy service text contract accepts simple-horizontal-v1. The newer engine
Text API uses shared shaping/bidi, rich styles, ruby/vertical layout and reveal
to publish client-scoped RGBA surfaces (ADR 0031/0033). CPU SDF atlas data exists;
its GPU/3D sampling path remains pending. Neither API implicitly connects native
message windows. The registered Siglus subset rejects CD_TEXT/CD_NAME, and
native page/read/wait/save policies require a tested adapter bridge. See
NATIVE_ADAPTERS.md and ADR 0034–0036 for executable coverage.

Textures and assets are bounded. Unsupported dimensions/formats/layouts report
errors. This snapshot has no asynchronous asset streaming or device-loss rebuild
beyond surface reconfiguration. These are implementation limitations, not
performance claims.

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
