# Engine-owned systems

CrySirror owns World, Layer, UI, Text, Audio/SFX, input, assets, scheduling and
persistence. Data projects and compatibility frontends use these services.
Siglus/RFVP keep their own interpreters, values/opcodes, syscall dispatch,
resource policy, exact waits, native text rules and original save formats.

## Executed implementations

| System | Engine implementation | Current limits |
|---|---|---|
| World | crysirror-scene: scoped generational entity handles, transactional spawn/despawn/mutations, parent transforms, visibility, velocity/angular velocity, fixed 16 ms updates, tween tracks and render extraction | Typed entity records rather than a general query ECS; model/terrain/animation/physics/gameplay described in ADR 0012/0014 |
| Animation/input | Actual complete Siglus IntEvent and InputState ports; delayed one-shot/loop/ping-pong easing, game/real time, button/key stock lifecycle | Native behavioral algorithms are preserved; adapter clock/unit translation still needs native validation |
| Layer | Retained hierarchy, scoped mutation batches, transform/pivot/anchor, camera/billboard/culling projection, inherited visibility/opacity/clips, explicit order pair, UV ranges and five blend pipelines | Rotations in radians; clips are axis-aligned scissors (rotated clip uses four-corner AABB); 3D parents rejected visibly; no native effect/shader emulation |
| UI | Panel/Label/Button/Toggle/Slider, parent tree, absolute/row/column layout, padding/gap, clipping, hit testing, focus, Tab/Down navigation, pointer capture/release, modal routing, value/events and World actions | No theme editor/accessibility/complex layout; callbacks are checked typed world data, not JS; native message/menu/backlog behavior remains in adapters |
| Text | crysirror-text owns fonts, simple-horizontal-v1 and explicit positioned glyph rasterization; renderer caches GPU surfaces | No complex shaping or native ruby/reveal/layout compatibility |
| Audio/SFX | crysirror-audio owns voice pools/priorities, random containers, scoped buses/ducking, sample envelopes, streaming music, stereo spatial mixing and AudioDevice; crysirror-acoustics owns acoustic meshes/BVHs and worker direct-path propagation (ADR 0067) | No HRTF, reflections, reverb or applied travel delay yet; native audio wait/save fidelity needs per-adapter validation; decode/mix/output are not yet wholly asynchronous |
| Assets | Scoped capability directories, cache keys and decoder registry, crysirror-image RGBA/standard codecs | No asynchronous streaming; native G00 metadata/codec stays in crysirror-siglus |
| Material | Safe CPU PBR description in crysirror-scene, shared by importers, project entities, Rust/JS/TS mutations and snapshots; scoped textures, unlit, opacity, UV0 transforms and soft depth intersections | Fog/Fresnel/Dissolve and custom WGSL surface hooks exist (ADR 0016); no material graph or per-slot override registry; raw cube/triangle UVs are zero |
| Renderer | Independent wgpu PBR/HDR/IBL/CSM/SSAO/TAA with transparent composition before bloom/tonemap; shared retained overlays and bounded compute primary/shadow rays | Sorted single-sample transparency; no OIT/refraction, full UE shaders or hardware RT; PBR is raster only (ADR 0010/0015) |

The shared CPU package contains real migrated source, not an alias for a native
VM. Exact commits, source SHA256 and changes are recorded in
[crysirror-scene/UPSTREAM.md](../crates/crysirror-scene/UPSTREAM.md). Original
algorithms are compiled in a temporary differential driver; the port is compared
against them rather than only against newly authored expected values.

Current material behavior: `examples/materials` is a script-free native project;
all five alpha modes reach actual pixels. Texture paths and GPU caches use the
owning client. `Entity.setMaterial(null)` restores original geometry materials.
Invalid changes roll back; save/restore preserves logical data. Python/MCP
validation and declared asset inventory include material textures. Actual
compilation, CPU/GPU results and remaining limits: [ADR 0015](adr/0015-engine-materials-transparency.md).

## Data and execution

`ClientSpec.systems` holds `entities`, `layers` and `widgets`. Each object's ID
is local to that client. Parents and UI action targets cannot access another
client's objects. All installs/mutation batches stage and validate before commit.
The public WorldCommand, LayerCommand and UiCommand APIs also support dynamic
engine callers without depending on a compatibility VM.

The runtime installs these systems, routes UI before VMs, then advances the world
on a fixed step even after every data VM halts. UI consumes the button frame when
it owns interaction. World tweens explicitly choose game time or real time; client
pause stops motion/game-time tweens, while real-time tweens continue. Ticks are
bounded to 60 seconds when a dynamic world exists. Accumulated fractional time is
saved so chunking and restore preserve fixed-step behavior.

Every render extracts transformed triangles from the world plus immutable layer
primitives. Raster and compute consume the same resulting scene. GPU/device
objects never enter these data records. The renderer alone owns vertex buffers,
textures, pipelines and scissors; winit remains in the desktop frontend.

Snapshots include World hierarchy, entity generations, animation clocks/tracks,
layer records, UI values/focus/events and fixed-step remainder. Projects using
systems use services schema 2; legacy flat projects retain schema 1. Restore
checks client registration, state invariants and render extraction before any
live swap. Transient UI pointer/key capture is reset and held input must first
release; device state and audio sample cursors are not restored.

## Run the implementation

```sh
cargo run -p crysirror-desktop -- run examples/desktop/systems.json --no-audio
cargo run -p crysirror-desktop -- render examples/desktop/systems.json \
  --frames 64 --output dist/engine-systems.png --renderer compute-raytrace
cargo run -p crysirror-desktop -- trace examples/desktop/systems.json \
  --frames 64 --press-at 0
```

The two top buttons pause/resume the world. Tab selects the next control; Enter
activates it. Mouse activates on matching release, toggle stores state, slider
responds to dragging or Left/Right. The demo contains only our test images.
`systems-text.json` adds text; supply a font with `--font /absolute/licensed.ttf`
or configure an explicitly licensed project font. No local font is distributed.

The MCP server accepts the same systems schema for validated project replacement
and asset inventory. With a configured executable, `engine_trace` exposes actual
fresh-session world/UI state; `scene_capture` selects raster/compute rendering.
Both accept bounded frames, step_ms and press_at (Enter pulses). These are
repeatable new sessions, not an in-process live game control API.

## Validation and scope

Implementation, compilation and behavior evidence are separated in STATUS.md.
GPU tests explicitly exercise dynamic extraction on both paths, actual blend
pixels and clipping. CPU tests cover scoped/stale handles, hierarchy rollback,
fixed steps, pause clocks, UI release/focus/modal/slider, input consumption,
mutation rollback and saved state. These establish this engine profile; they do
not establish original Siglus/RFVP execution or a complete AAA engine.

## Modular VFX

`crysirror-vfx` owns authored systems, typed parameters, CPU simulation and GPU
tick planning. Runtime workers publish immutable products; wgpu owns resident
GPU execution and Sprite/Mesh/Ribbon drawing. `Vfx.play` and `Entity.setVfx`
store the same `EntitySpec.vfx` data as native `WorldCommand::SetVfx` calls.
Legacy `particles` lowers into this service rather than a renderer-owned CPU
simulator. Interactive updates poll tasks; offline capture uses an explicit
completion fence. Stop preserves living particles, pause freezes simulation
and revision changes restart it. MCP validation and scoped resource inventory
also understand the VFX declarations.

The [VFX laboratory](../examples/aaa-vfx/README.md) supplies runnable CPU/GPU
effects and a 100k resident stress example. [ADR 0064](adr/0064-modular-vfx-cpu-gpu.md)
documents supported modules, bounded histories/events and remaining Niagara
gaps, including exact checkpoints and GPU child/ribbon support.
