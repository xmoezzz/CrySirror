# Architecture decision records

Every requirement that changes ownership, a public API, a dependency boundary or
a validation claim gets an ADR before (or together with) its code. An ADR states
the requirement, the decision, rejected alternatives, and acceptance evidence.
Status values: `proposed`, `accepted`, `implemented (partial)`, `implemented`,
`superseded by NNNN`. Implementation, compilation and behavior evidence are
recorded separately, as AGENTS.md requires; STATUS.md links the evidence files.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-service-boundary.md) | Data-driven core, engine-specific compatibility frontends | accepted |
| [0002](0002-modern-engine-and-optional-typescript.md) | Modern 3D engine with independent compatibility frontends | accepted |
| [0003](0003-siglus-monorepo-and-rendering-capabilities.md) | Siglus resources and rendering capabilities | implemented (partial) |
| [0004](0004-object-model-and-lifecycle.md) | Engine object model, lifecycle and plugins (OOP API) | implemented (partial) |
| [0005](0005-script-hosts.md) | Pluggable script hosts: JS/TS default, native VMs as peers | implemented (partial) |
| [0006](0006-virtual-filesystem.md) | Virtual filesystem providers and mount tables | implemented (partial) |
| [0007](0007-modern-rendering-and-assets.md) | Modern rendering features and Blender/glTF assets | implemented (partial) |
| [0008](0008-native-engine-integration.md) | Native engine integration: shared seams, original engines whole | implemented (RFVP) |
| [0009](0009-siglus-full-engine.md) | Siglus: full original SceneVm + native frame-plan render plugin | in progress (paused) |
| [0010](0010-aaa-renderer.md) | AAA rendering: physically based HDR pipeline and showcase ports | implemented (CSM/SSAO/TAA added) |
| [0011](0011-game-ports.md) | Porting real games (Tomb Raider via OpenLara) to drive engine requirements | accepted |
| [0014](0014-gameplay-systems.md) | Gameplay systems: GAS, anim graphs, BT AI/nav, particles, terrain, streaming, spatial audio | implemented (first slice) |
| [0013](0013-unreal-demo-port-assessment.md) | Assessing the Ashen Vow Unreal demo as a port target | proposed (owner decisions pending) |
| [0012](0012-animation-physics-input.md) | Skeletal animation, Rapier physics/character control, input actions | implemented (first slice) |
| [0015](0015-engine-materials-transparency.md) | Engine material data, transparency, unlit/opacity/soft intersections | implemented (locally validated first slice) |
| [0016](0016-shader-effects.md) | Preset effects and developer WGSL surface ABI; CPU/MCP shader checking | implemented (surface ABI v1) |
| [0017](0017-gow3-scene-capabilities.md) | GoW III scene trial, shared topology/DDS codecs and data-driven lighting; capability gaps | implemented first preview; further APIs proposed |
| [0018](0018-local-lights.md) | Clustered point/spot/rect/volume lights, budgeted local shadows, glTF/UE/GoW light import | implemented |
| [0019](0019-height-fog-auto-exposure.md) | Exponential height fog and auto exposure in the shared environment | implemented |
| [0020](0020-video-service.md) | Video service over the vendored Siglus decoders (OMV/Theora, MPEG-1/2, WMV) with world-timed playback | implemented; Theora compile-only |
| [0021](0021-siglus-2d-layer-filters.md) | Siglus-style 2D layer filter stack (color stages, tone curve, mask, distortions) | implemented; wipes in 0022 |
| [0022](0022-screen-transitions.md) | Screen transitions: retained frames + the vendored Siglus wipe shader and pattern masks | implemented; page curl/VM export pending |
| [0023](0023-screen-space-reflections.md) | Screen-space reflections from the previous HDR frame over the environment specular | implemented; glossy/Hi-Z/probes pending |
| [0024](0024-reflection-probes.md) | Box-projected reflection probes captured by the engine (6 faces → equirect → GGX prefilter) | implemented; sphere/realtime/GoW cubemaps pending |
| [0025](0025-authored-probes-gow3-cubemaps.md) | Authored (.hdr) reflection probes; GoW III slot-22 cubemaps verified by seams and converted | implemented; per-material cubes pending |
| [0026](0026-cluster-lod.md) | Cluster LOD DAG (own QEM simplifier, crack-free cuts) drawn through all PBR passes | GPU selection + frustum culling; HZB/streaming pending |
| [0027](0027-virtual-shadow-maps.md) | Sun virtual shadow maps: clipmap pages marked on GPU, LRU-cached in the shared atlas | static/dynamic split done; GPU allocation pending |
| [0028](0028-irradiance-volume-gi.md) | Dynamic diffuse GI: rasterized irradiance probes (SH L1) refreshed round-robin, multi-bounce | step 2: visibility, classification, idle refresh |
| [0029](0029-puppets.md) | 2D deformable puppets: engine rig format, PuppetSource interface, eluna E-mote bridge and bake | implemented; no real E-mote file validated |
| [0040](0040-level-sequences.md) | Level sequences: camera paths, transform/light tracks, flicker noise, events | implemented; GoW ANM not decoded |
| [0041](0041-engine-capability-roadmap.md) | Engine capability roadmap: done/partial/missing matrix and work order | living list |
| [0043](0043-arcade-vehicles.md) | Arcade vehicle controller: wheel-ray suspension, bicycle steering, grip/drift, swept collision; road centrelines | implemented; state not saved |
| [0042](0042-nfs-carbon-import.md) | NFS Carbon PC import (canyons): BUN chunks, sections, solids, TPK textures | canyon renders; instances/cars/vehicle pending |
| [0047](0047-engine-api-capability-extraction.md) | Expand engine APIs; prohibit and remove hosted legacy engines | accepted; hosting removed |

## Requirement backlog

Requirements raised by the project owner that are tracked by these ADRs. A row
moves to `done` only with recorded behavior evidence.

| ID | Requirement | ADR | State |
|---|---|---|---|
| R1 | General-purpose (Unity-like) engine; VN engines are clients, not the core | 0002, 0004 | in progress |
| R2 | Extract shared features (world, transforms, scaling, text, UI) into CrySirror | 0004 | world/layer/UI/text exist; object API wraps world |
| R3 | Everything reachable through an OOP API (classes, properties, methods) | 0004 | class registry + World/Entity classes |
| R4 | Engine lifecycle abstracted (init/start/update/fixed/late/pause/stop) | 0004 | implemented in crysirror-engine |
| R5 | Not script-centric, but can export OOP API to scripts | 0004, 0005 | implemented for JS |
| R6 | Default scripting: Rust-implemented TS/JS | 0005 | Boa runtime host + Oxc transpile |
| R7 | Other VMs (siglus_rs SceneVm, RFVP) plug in as script hosts | 0005, 0008 | Siglus subset + RFVP registered as VmAdapters |
| R8 | Abstract filesystem; native engines keep their own FS semantics | 0006 | mount table + providers |
| R9 | Load Blender models | 0007, 0010 | glTF 2.0 import incl. textured PBR; .blend via Blender CLI export |
| R10 | Ray tracing | 0003, 0007, 0051 | native opaque primary queries and hybrid PBR sun shadows; broader RT effects and multi-device validation remain |
| R11 | DLSS-class upscaling | 0007 | capability contract only; vendor SDKs not distributed |
| R12 | Asynchronous/multithreaded execution across every engine subsystem | 0049, 0050, 0052 | required; assets/poses and extraction DAG exist; remaining subsystem migration is partial |

## Parallel service work (Codex, 2026-10-25)

| ADR | Title | Status |
|---|---|---|
| [0030](0030-audio-mixer-and-music.md) | Scoped SFX pools, spatial PCM mixing, streaming music and Audio API | implemented first slice |
| [0031](0031-rich-text-shaping-and-sdf.md) | Shaping, bidi, rich layout, ruby/vertical text and CPU SDF atlas | implemented; SDF GPU integration pending |
| [0032](0032-input-rebinding-gamepads.md) | Runtime rebinding, optional gamepads and semantic input prompts | implemented; no hardware pad validation |
| [0033](0033-localization-rich-text-surfaces.md) | Locale catalogs, CLDR plurals and engine rich-text surfaces/reveal | implemented first slice |
| [0034](0034-siglus-runtime-verification.md) | Execute native Siglus fixtures and preserve input/save failure boundaries | verified subset; native message bridge missing |
| [0035](0035-native-family-coverage.md) | Audit the independent RealLive and AVG32 VMs in pinned siglus_rs | external source/tests verified; no CrySirror adapters |
| [0036](0036-siglus-service-boundary-audit.md) | Distinguish shared engine services from native Siglus message/effect coverage | audit complete; native bridges pending |
| [0037](0037-bgi-local-runtime-trial.md) | Local BGI game and device-free native CPU-frame embedding experiment | local behavior verified; formal adapter pending |
| [0038](0038-shared-bitmap-operations.md) | Shared bitmap primitives from RealLive/AVG32 and public Bitmap API; accurate native transform translation | implemented CPU capabilities; native VM ports remain separate |
| [0039](0039-bgi-shared-service-adapter.md) | Original BGI VM on shared image/glyph/audio/storage APIs and desktop registry | implemented first profile; bounded native game evidence |
| [0044](0044-archive-format-resource-plugins.md) | Shared resource contracts and optional Rust archive plugins from arc_unpacker | five decoder IDs, explicit partial/pending inventory |
| [0045](0045-rapier-shared-physics.md) | Persistent Rapier 3D physics shared by data projects, native clients and engine APIs | rigid-body simulation and integration validation |
| [0046](0046-native-image-resource-pipeline.md) | All pinned image decoder IDs through the shared runtime, object API and renderer | 96 image IDs, per-variant limits and actual Metal archive-render evidence |
| [0048](0048-reallive-vm-engine-apis.md) | Pure RealLive VM and semantic adapter over reusable retained Canvas/GlyphRun/UI/audio/Media APIs | API bridge implemented; opening-profile evidence separate |
| [0049](0049-multithreaded-tasks-and-async-assets.md) | Bounded CPU/asset workers, awaitable reads, asynchronous image/model imports and parallel poses | stage one implemented; engine-wide requirement in 0050 |
| [0050](0050-engine-wide-async-execution.md) | Engine-wide asynchronous/multithreaded execution and explicit subsystem migration; local validation, no unsolicited GitHub CI | accepted requirement; partially implemented |
| [0051](0051-hardware-ray-query.md) | Native BLAS/TLAS ray queries, capability negotiation across Vulkan/DX12/Metal, opaque primary rays and hybrid PBR sun visibility | initial implementation; validation recorded in STATUS.md |
| [0052](0052-frame-pipeline-streaming-and-profiling.md) | Immutable frame extraction DAG, CPU/GPU profiling, asynchronous leased streaming cells and mesh RT instancing | implemented slice; engine-wide async migration remains partial |
| [0053](0053-krkr-tjs2-adapter.md) | Optional AGPL KiriKiri adapter: vendored Kirakira TJS2 VM over engine Layer/Canvas, clock and storage | first slice implemented; KAG3 boot pending |
| [0054](0054-fbx-pmx-vrm-import.md) | FBX (binary) and PMX converted in memory to GLB; VRM through glTF; losses reported | first slice implemented; morph targets next |
| [0055](0055-rt-gpu-geometry-dynamic-blas-alpha.md) | RT BLAS from renderer GPU streams, GPU-skinned dynamic BLAS, alpha-tested casters by re-tracing with a baked alpha atlas | implemented; Metal validated |
| [0056](0056-rt-soft-shadows-denoising.md) | Soft ray-traced sun shadows sampling the sun disc, with temporal reprojection and edge-aware spatial denoising | implemented; Metal validated |
| [0057](0057-rt-local-light-shadows.md) | Ray-traced shadows for the nearest two point/spot lights in spare visibility channels, emitter-radius penumbrae, three-channel denoiser | implemented; Metal validated |
| [0058](0058-rt-reflections-material-atlas.md) | Ray-traced mirror reflections with previous-frame reuse and atlas hit shading; all-material atlas and corner attributes | implemented; Metal validated |
