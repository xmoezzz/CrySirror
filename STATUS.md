# Implementation and validation status

Snapshot: 2026-10-25. Owner priority: the new AAA engine first;
compatibility layers are secondary (ADR 0010).

## RT step 6: ray-traced irradiance probes (ADR 0059)

- `irradiance_volume.ray_traced`: probes refreshed by 256 rays each against
  the RT scene (sun + RT shadow, emission, volume multi-bounce, environment
  on misses) into the same SH/visibility data as raster captures.
- Behaviour (Apple M5, Metal): closed red-emissive room floor black without
  GI, (245, 60, 60) raster probes, (211, 50, 50) ray-traced probes. Hardware
  RT GPU tests **8 passed**; whole `crysirror-wgpu` ignored GPU suite
  `--test-threads=1` **43 passed**. Cost relative to raster captures not
  measured.

## RT step 5: ray-traced reflections (ADR 0058)

- `reflections.ray_traced`: mirror rays traced on the hardware path; hits
  visible this frame reuse the previous frame's colour, others are shaded
  from a baked all-material atlas, GPU-gathered UV/normal attributes, the
  sun with an RT shadow and SH irradiance. Shadow queries now use the
  closest-hit flag (fixes a possible missed occluder behind alpha holes).
- Behaviour (Apple M5, Metal): hardware RT GPU tests **7 passed**; whole
  `crysirror-wgpu` ignored GPU suite `--test-threads=1` **42 passed**.
  Off-screen box reflected: RT 2348 red pixels, SSR 0, none 0.

## RT step 4: ray-traced point/spot light shadows (ADR 0057)

- Nearest two shadowed point/spot lights traced into the spare B/A
  channels of the RT visibility target (16-texture binding budget kept);
  `LightSpec.source_radius` gives denoised penumbrae; others keep atlas
  shadows.
- Behaviour (Apple M5, Metal): hardware RT GPU tests **6 passed**; whole
  `crysirror-wgpu` ignored GPU suite `--test-threads=1` **41 passed**.
  Point light: 1389 shadowed floor pixels; 0.35 m emitter: sharp floor
  shadow edges 119 → 79.

## RT step 3: soft sun shadows and denoising (ADR 0056)

- `Sun.angular_diameter_degrees` (default 0): 2 cone-sampled shadow rays per
  pixel and frame, temporal accumulation with depth-validated reprojection
  (≤ 32 samples) and a 5×5 edge-aware spatial filter. 0 keeps the exact
  hard ray.
- Behaviour (Apple M5, Metal): hardware RT GPU tests **5 passed**
  (release, `--test-threads=1`); soft test: sharp floor edges 211 → 146 px,
  converged frame-to-frame mean |ΔG| 0.008. Sponza (local) with a 2° sun
  renders clean penumbrae.

## RT step 2: GPU-stream BLAS, dynamic BLAS, alpha-tested shadows (ADR 0055)

- PBR hardware RT builds BLAS from the renderer's GPU vertex/index streams
  (no CPU triangle copies); skinned/morphed instances own resident BLAS fed
  by a GPU skinning pass and rebuilt only when pose/morphs change. wgpu
  29.0.x performs `PreferUpdate` as a full rebuild: no refit yet.
- MASK casters: GPU-gathered UVs, a 2048² alpha atlas baked with the raster
  coverage function, shadow rays re-traced past transparent hits (≤ 8
  layers). Blended surfaces cast nothing. The old CPU triangle instance
  path was removed.
- Behaviour (Apple M5, Metal):
  - Hardware RT GPU tests, release, `--test-threads=1`: **4 passed**
    (instancing/rigid motion, uniform-alpha mask, checker-alpha texture
    shadow 410 → 205 pixels, skinned dynamic BLAS reuse/rebuild).
  - Whole `crysirror-wgpu` ignored GPU suite, `--test-threads=1`: **39
    passed**.
  - Khronos Sponza (local), `--renderer hardware-raytrace`: renders; 103
    instances, 14 alpha-tested, 3 atlas tiles. Before this step hardware RT
    rejected the scene's masked materials.

## Morph targets (ADR 0054, second step)

- glTF/VRM morph targets load (sparse, `targetNames`); PMX vertex and
  group morphs convert. `Entity.setMorph/setMorphs` store weights in
  `AnimationState.morphs`; whole-scene drawables carry them; the renderer
  blends positions/normals on the CPU into a per-draw vertex buffer used by
  every raster pass, re-uploaded only when that primitive's weights change.
- Not applied: per-mesh `pbr: true` references (reported), glTF weight
  animation channels, FBX blend shapes, PMX non-vertex morphs, hardware RT
  shadow geometry (unmorphed).
- Behaviour:
  - VRM 1.0 Seed-san, release raster on Apple M5/Metal: `eye_smile`,
    `lip_a`, `cheek` at 1.0 close the eyes, open the mouth and add blush
    (local images `target/verification/vrm-face0.png`/`vrm-face1.png`).
  - Rust workspace `--no-fail-fast`: **601 passed / 0 failed / 56 ignored**.
  - Explicit Metal GPU smoke suite, `--test-threads=1`: **29 passed**. Run
    in parallel, 21 of them panic with a wgpu validation error; the
    unmodified previous commit fails identically (pre-existing, not
    investigated).
  - Python 3.11: **169 passed**.

## FBX, PMX and VRM import (ADR 0054)

- glTF/GLB was already loading (DamagedHelmet, Sponza, Fox); a render of
  `examples/aaa-helmet/project.json` without `--script game.ts` is empty
  because the scene is created by that script.
- New `crysirror-asset::interchange`: own binary FBX 7.x and PMX 2.0/2.1
  parsers, converted in memory to GLB and consumed by every glTF path.
- Import reports now list morph targets and unsupported glTF extensions;
  PBR references no longer report skins/animations as dropped; triangle
  counts are filled in.
- Behaviour:
  - `crysirror-asset` tests: **23 passed** (5 new interchange tests, one of
    them comparing local Fox glTF/FBX copies when present).
  - Fox glTF → Blender 5.2.2 FBX → CrySirror: relative bounds error
    2·10⁻⁵ (Survey), 1.9·10⁻⁴ (Walk), < 10⁻⁵ (Run). Release raster render
    of `root|Run` vs the glTF `Run` at the same frame: 1.05% of pixels
    differ by more than 8/255.
  - VRM 1.0 Seed-san (VRM Public License 1.0, local only) renders with
    the unlit fallback; reported: 129 morph targets, VRMC_vrm,
    VRMC_springBone, VRMC_materials_mtoon, VRMC_node_constraint,
    KHR_texture_transform.
  - No real PMX model was rendered (none with a redistributable licence was
    fetched); PMX is validated by a synthetic file only.

## KiriKiri TJS2 adapter, first slice (ADR 0053)

- **Source.** Kirakira `krkr-tjs2` and `krkr-kag` vendored unmodified from
  `c451918023477620ff9055a2eb8a6714b947d9c1` (SHA-256 per file). AGPL is
  confined to them and `crysirror-krkr`; the desktop `krkr` feature is not
  in `default`. `tests/test_krkr_policy.py` checks both.
- **Engine capabilities added first:** canvas presentations inside layer
  trees (`present.parent/position`, 1024 surfaces) and a thread-safe
  `image_metadata` handle.
- **Translated natives:** Layer (tree, geometry, visibility, opacity,
  `absolute`, loadImages, fillRect, colorRect, copyRect, operateRect,
  assignImages, drawText), Window, Timer, System, Debug, Storages, Scripts.
  Unsupported members throw TJS exceptions naming the missing capability.
- **Behaviour.**
  - `crysirror-krkr` tests: **3 passed** (layers/timer/click to engine
    state; visible failures; Shift_JIS and case-insensitive storage), plus
    1 unit test. `crysirror-scene` canvas tests: **7 passed**.
  - `examples/krkr-demo` (self-authored) rendered through the release
    desktop renderer (`--features krkr`, raster, Metal): 8 KiriKiri layers
    as engine nodes, script subclass `Orbit extends Layer` driven by a Timer.
  - Python 3.11: **169 passed**.
  - No commercial title was run; KAG3 boot, XP3 mounts, font metrics,
    audio, video, transitions and saves are not implemented.

## Latest: NFS Carbon canyon decoded and driven; arcade vehicles (ADR 0042, 0043)

- **NFS Carbon geometry and textures** (`tools/nfsc_geometry.py`,
  `tools/nfsc_textures.py`, our own decoders).
  - All 962 Palmont stream sections decode: 40,677 solids and 4.62 million
    triangles, with every vertex normal unit length.
  - 3,496 textures (DXT1/3/5, RGBA8) are indexed and go through the
    engine's DDS codec.
  - The TNDRIFTB canyon was identified from evidence (solid names match the
    disc's canyon movies). Its 17 sections, with 98,431 triangles and 55
    textured materials, render in the engine.
- **Arcade vehicles (ADR 0043).** `Entity.setVehicle` and `Entity.drive`
  provide wheel-ray suspension, bicycle steering, grip and handbrake drift,
  and swept wall collision. They are tested for acceleration, wall stops,
  steering, drift and a 10° ramp.
- **`tools/road_centerline.py`** derives a centred road centreline (tested
  on a synthetic road).
- **Canyon drive.** An autopilot drove about 3 km down the decoded canyon in
  142 s, off the road in 0 of 8,880 frames (`canyon-drive.mp4`).
- Behavior after this step:
  - Rust workspace `--no-fail-fast`: **312 passed / 0 failed / 34 ignored**.
  - Python 3.11: **152 passed**.
  - GPU suite not re-run; no renderer code changed.

## clipping fixes, local data layout, NFS Carbon start (ADR 0041, 0042)

- **Clipping.** New engine APIs, each with tests:
  - `World.sphereCast`;
  - `World.groundPose`;
  - `World.modelInfo(..).bounds` (rest pose, skinned vertices posed
    through their joints).

  The walking slice now sizes its collider from the model, conforms the
  body to the ground and sweeps the camera. A per-frame body-axis sweep
  over 1,500 frames counted 216 clipping frames (14%) with the old capsule
  and 0 with the fitted one.
- **Local data.**
  - Game sources (GoW III PS3, Ashen Vow, the NFS Carbon ISO), converted
    work projects, references and the private .NET SDK are in the
    gitignored `local/` (see local/README.md).
  - Representative renders are in `images/`; `images/third-party/` is
    gitignored.
  - The cleanup of `~/.cache/crysirror` kept only Codex's directories.
- **NFS Carbon (ADR 0042).** `tools/nfsc_bun.py` reads the disc zips
  directly. It walks BUN chunks and parses the 962 streaming sections of
  Palmont (`L5RA`). Geometry and texture decoding are next.
- Behavior after this step:
  - Rust workspace `--no-fail-fast`: **311 passed / 0 failed / 34 ignored**.
  - Python 3.11: **148 passed**. The policy test now skips the gitignored
    `local/` and `images/third-party/` and also forbids `.psb`.
  - GPU suite not re-run; no renderer code changed since the last run of
    29 passed.

## walking character in the GoW hall, puppets, sequences, capability roadmap (ADR 0029, 0040, 0041)

- **One scene a character walks in.** The imported OLY100 hall
  (`hall-walk.ts`, user data, outside the repository) has:
  - level mesh collision;
  - a capsule character controller;
  - the skinned Fox stand-in with walk/run/idle clips and cross-fades;
  - navigation and A* to random goals;
  - an orbit camera;
  - WASD control in `crysirror run`.

  A 320-frame autopilot movie (`hall-walk.mp4`) rendered in 69 s.
  Kratos' own skeleton, skin weights and `ANM_hero` (10.5 MB) are not
  decoded yet.
- **Navigation fix.** Small steep bumps no longer erode the floor. The
  OLY100 hall went from 1,015 to 4,153 walkable cells, with a regression
  test.
- **Puppets (ADR 0029).**
  - The engine owns the rig format, evaluator and `PuppetSource`
    interface, plus bake.
  - The eluna E-mote bridge and `crysirror puppet-import` exist; PSB is
    rejected for layers and packages.
  - Puppet layers render with stencil masks, and the `Puppets` API drives
    them.
  - Not validated on a real E-mote file (none available locally).
- **Level sequences (ADR 0040).** Camera, transform and light tracks with
  spline interpolation, flicker noise and events. In the OLY100 hall: a
  dolly shot and 102 flickering warm lights with embers (`hall-sequence.mp4`).
  - `render --movie-frames/--movie-dir` writes consecutive frames; scripts
    keep running while it captures.
- **Capability roadmap (ADR 0041)** lists done/partial/missing
  capabilities, with priorities and an order of work.
- Behavior after this step:
  - Rust workspace `--no-fail-fast`: **309 passed / 0 failed / 34 ignored**.
  - Explicit Metal GPU (release): **29 passed** (run before the navigation
    change; no GPU code changed since).
  - Python 3.11: **145 passed**.

## generated API reference; GI visibility and refresh cost (ADR 0028 step 2)

- **API documentation** (`docs/api/`).
  - `REFERENCE.md`, `api.json` and `crysirror.d.ts` are generated from the
    class registry, the command table, serde defaults and the `///`
    comments of the scene specs and adapter configs.
  - `crysirror-engine/tests/api_docs.rs` fails in three cases: a generated
    file is stale, the signature fingerprint is missing from
    `docs/api/CHANGES.md`, or a class member has no doc.
  - The reference covers 13 classes, 11 globals, 44 commands and 37 data
    types, plus the Siglus, RFVP and BGI configs. Two BGI config fields
    (`font_face_index`, `gbk`) have no description yet.
  - AGENTS.md now requires regenerating the reference and logging
    fingerprint changes.
- **GI step 2.**
  - Octahedral distance moments per probe with Chebyshev visibility.
  - Probes inside geometry are disabled, detected by their back-face
    fraction.
  - Captures use 32² faces with per-face draw culling and a nearest-first
    sweep order.
  - Converged volumes idle until something changes.
  - Measured:
    - Dark-room leak (mean red): 78.2 without visibility, 4.3 with it.
    - The three buried probes are exactly the disabled ones.
    - An idle capture refreshes 0 probes and reproduces the image.
    - OLY100 GI capture: 46 s → 8.8 s; 2.6 s without GI.
  - A wgpu discard-tracking cost that was quadratic in the number of
    capture passes was found by sampling and avoided by storing instead of
    discarding.
  - New optional fields: `visibility` and `idle_probes_per_frame`.
- Behavior after this step:
  - Rust workspace `--no-fail-fast`: **295 passed / 0 failed / 33 ignored**
    (now including Codex's BGI work).
  - Explicit Metal GPU (release): **28 passed**.
  - Desktop release build passed.
  - Python 3.11: **143 passed**.

## GPU cluster selection, VSM static/dynamic split, irradiance GI (ADR 0026–0028)

- **Cluster LOD on the GPU.**
  - A compute pass per draw selects clusters, frustum-culls the main view,
    and writes compacted indices plus indirect args. Every pass draws
    indirectly; shadows use an unculled cut.
  - The GPU cut equals the CPU cut exactly.
  - OLY100: 0.5% of pixels differ from the CPU path; the headless render
    drops from 3.57 s to 2.71 s.
- **VSM static/dynamic split.** Moved, added, removed and skinned casters
  dirty only the pages under their old and new footprints. The depth range
  only grows. A moving box re-rendered 4 of 8 pages, and the static
  pillar's shadow stayed identical.
- **Irradiance-volume GI** (`irradiance_volume` component).
  - Probes are refreshed round-robin by 6-face captures with full shading,
    projected to SH L1 on the GPU, and stored in the shadow-data buffer.
    Bounces accumulate.
  - A red emissive wall lights a closed room that is black without GI.
  - OLY100 smoke run: 46 s for a 16-frame capture, dominated by probe
    refreshes.
  - Not done: light-leak visibility and sun shadows in captures.
- Behavior after this step:
  - Rust `--no-fail-fast`: **226 passed / 0 failed / 31 ignored**.
  - Explicit Metal GPU: **26 passed**.
  - Python 3.11: **126 passed**.
  - Desktop release build passed.

## Virtual shadow maps for the sun (ADR 0027)

- `Environment.virtual_shadows`: a camera-centred clipmap of 8192² virtual
  levels. Pages are marked by a compute pass over the prepass, read back,
  allocated and LRU-cached in the right half of the 8192×4096 shadow atlas,
  and sampled with a coarser-level fallback. Only new pages are rendered.
- The GPU test shows a shadow beyond the 10 m cascade range, 0 pages
  re-rendered on the second capture, and a byte-identical image.
- Ashen castle: 55 pages requested, 0 dropped; 0.84% of pixels differ from
  the cascade render. A first-frame auto-exposure seeding bug (99.6%
  difference) was found and fixed.
- Behavior after this step:
  - Rust `--no-fail-fast`: **225 passed / 0 failed / 28 ignored**.
  - Explicit Metal GPU: **23 passed**.
  - Python 3.11: **126 passed**.
  - Desktop release build passed.

## GoW III cubemap probes and cluster LOD (ADR 0025, ADR 0026)

- Authored reflection probes (`source` `.hdr`, `parallax`).
  `tools/gow3_cubemaps.py` decodes the GoW III slot-22 cubemaps; their face
  convention is verified by seam continuity. 3 of 4 are converted and 1 is
  refused.
  - OLY100 with the native hall cube and with the engine's own capture
    differ in 0.7% of pixels.
- Cluster LOD DAG (own QEM simplifier, crack-free cuts, seam welding),
  drawn through every PBR pass with `Environment.geometry_lod`.
  - OLY100 at 1 px: 574,234 of 999,296 triangles drawn.
  - GPU-driven culling and virtual shadow maps are not implemented yet.
- Behavior after this step (shared tree; Codex was editing audio, text and
  input concurrently):
  - Rust `--no-fail-fast`: **217 passed / 1 failed / 27 ignored**. The
    failure is `crysirror-audio`
    `compressed_music_streams_decode_and_seek_without_whole_song_pcm`, which
    is in-progress Codex work.
  - Explicit Metal GPU: **22 passed**.
  - Python 3.11: **125 passed / 1 failed**. The failure is
    `test_engine_audio_and_text_are_independent_subsystems`: the desktop
    crate's new `rodio` dependency conflicts with the subsystem-boundary
    rule, also in-progress Codex work.

## Box reflection probes (ADR 0024)

- `reflection_probe` entity component / `Entity.setReflectionProbe`.
  - The engine captures six faces with the real surface pipelines, including
    local lights and shadows, then converts them to an equirect map and runs
    the environment GGX prefilter.
  - Up to 8 slots. Static probes are captured once; bumping `revision`
    recaptures.
  - Shading uses box projection, blend fade and environment fallback, with
    SSR applied on top.
- The GPU test puts coloured off-screen walls on the probe box faces and
  checks that each appears in the mirror floor on the correct side.
- OLY100 hall with one probe: 40% of pixels change by more than 8/255
  against the sky-only reflection. No original reference image exists to
  compare against.
- Behavior after this step:
  - Rust: **182 passed / 0 failed / 22 ignored**.
  - Explicit Metal GPU: **20 passed**.
  - Python 3.11: **123 passed**.
  - Desktop release build passed.

## Screen transitions and screen-space reflections (ADR 0022, ADR 0023)

- Screen transitions (`Layers.transition` / `endTransition` /
  `activeTransition`). The renderer retains the last presented frame and
  composes old → new with the vendored siglus_rs `wipe.wgsl`, sampling gamma
  bytes as Siglus does. Pattern wipes use the original generated masks
  (vendored change 6: `wipe_mask` visibility only). The transition is
  world-timed and saved.
  - Not done: page curls 300/301, partial-range wipes, and the Siglus VM
    bridge export of `WIPE` (it still refuses visibly).
- Screen-space reflections (`Environment.reflections`): prepass ray march
  with bisection and self-intersection rejection, previous-frame HDR
  radiance, roughness fade.
  - On OLY100 the import's flat roughness of 0.6 equals the default fade-out,
    so the default settings change nothing there. That is reported, not
    tuned away.
- Regression found and fixed in this step: a class member that was both a
  property and a method broke every JS binding. The binding generator now
  rejects such collisions visibly.
- Behavior after this step:
  - Rust: **181 passed / 0 failed / 21 ignored**.
  - Explicit Metal GPU: **19 passed**, including the new transition and SSR tests.
  - Python 3.11: **123 passed**.
  - Desktop release build passed.

## Siglus 2D filters, video, atmosphere (ADR 0019–0021)

- Siglus-style layer filters (`LayerSpec.filter`, `Layers.setFilter`): color
  stages, tone curve, mask/gray-alpha, alpha test, mosaic/raster/radial
  blur/threshold fade, matching the reference arithmetic within ±3 on Metal.
  `crysirror-siglus-vm` exports Siglus mono/reverse/bright/dark/color
  rate/add and blend codes 0–4 instead of refusing them. Overlay blend is
  still pending; wipes are in ADR 0022.
- Behavior after this step: Rust **180 passed / 0 failed / 19 ignored**;
  explicit Metal GPU **17 passed**; Python 3.11 **123 passed**; desktop release
  build passed.

### Atmosphere, exposure and video (ADR 0019, ADR 0020)

- Exponential height fog and auto exposure in `Environment` (GPU test). Video
  service `crysirror-video` over the vendored Siglus decoders with world-timed,
  saved playback into dynamic surfaces and `World.playVideo` etc. MPEG-2 and
  WMV2 are behavior-validated with synthetic fixtures; OMV/Theora is
  compile-only (no fixture). Upstream finding: `na_mpeg2_decoder` partially
  decodes the final MPEG-2 picture at flush.
- Behavior: Rust **179 passed / 0 failed / 18 ignored**; explicit Metal GPU
  **16 passed**; Python 3.11 **123 passed**. Desktop release build passed.

## Clustered local lights (ADR 0018)

- Update 2: budgeted point/spot shadows from a 64-tile atlas; material
  `normal_encoding` (rgb/rg/ag) and `specular_factor`/`specular_texture`
  (ADR 0015 addendum). GoW III OLY100: complete mip chains from `OLYMPUS.TS`
  (byte offset at wrapper +60; the streamed sizes of all textures sum exactly to the
  TS length), **241** textures decode with 0 errors, and sampler slots 0/5/9 are
  mapped as base/DXT5nm normal/specular from texture content (other slots
  reported). Actual Metal renders: `~/.cache/crysirror/gow3/olympus-hd/preview`
  and the castle with shadows. Behavior: Rust **175 passed / 0 failed / 17
  ignored**; explicit Metal GPU **15 passed**; Python 3.11 **123 passed**.

- Update: range-limited `directional` and `ambient` volume lights plus
  `falloff: {kind:"exponent"}`. GoW III OLY100: `tools/gow3_lights.py` converts
  all **207** native light records (103 point / 87 directional / 17 ambient)
  with documented approximations; actual Metal render of the interior with them.
  Behavior after this update: Rust **174 passed / 0 failed / 15 ignored**;
  explicit Metal GPU **13 passed**; Python 3.11 **122 passed**.

- Implementation: `LightSpec` entity component (point/spot/rect, candela or
  lumens, range, enabled, stored shadow flag), `WorldCommand::SetLight`,
  `World.spawn({light})`, `Entity.light`/`Entity.setLight`, Python schema
  mirror. The renderer uses CPU clusters (16×9×24) with storage buffers, and
  the PBR shader shades lights with the sun's BRDF. glTF `KHR_lights_punctual`
  point/spot lights import; directional lights are reported as dropped.
- UE path: pure-Rust `ue-import level` (recovered schemas + container
  header + archetype merge) extracts placements and lights without CUE4Parse.
  Its FantasyCastle mesh positions agree with the CUE4Parse preview glTF for
  **4,683 / 4,815** instances (the rest include omitted sky/fog meshes).
  `tools/ue_lights.py` converts **284** castle and **200** village lights;
  sky lights are skipped and reported.
- Compilation: `cargo check --workspace --tests` and desktop release build passed.
  Behavior: Rust **174 passed / 0 failed / 14 ignored**; explicit Metal GPU
  **12 passed** (new: clustered point/spot test); Python 3.11 **120 passed**.
  Ignored tests are not counted as passes.
- Actual Metal captures of the castle hall with and without the converted
  lights are in `~/.cache/crysirror/ashen/castle-lit` (external, not
  distributed). No image-equivalence or frame-time claim.
- Not implemented: local-light shadows (light leaks through walls), rect-light
  area specular, sky-light capture, auto exposure, height/volumetric fog.

## God of War III OLY100 rest-space scene trial (ADR 0017)

- Actual Apple M5 / Metal capture from the local OLY100.WAD: **247 resource
  nodes / 1,474 surface groups / 548,191 triangles**, with 75 used texture
  previews and no selected MG geometry omitted. This is the selected WAD's first
  surface table in encoded rest space, **not the complete original scene/game**.
  Entity/bind/animation composition remains unresolved; applying entity matrices
  directly double-transforms some geometry. Native matrices remain diagnostic
  metadata. 1,380 surfaces use preview textures; 94 remain neutral, including
  seven unresolved UV-remap groups. Original shaders,
  lighting and animation poses are not reproduced.
- Reusable safe CPU APIs: `crysirror-asset::indices` (raw U16 and bounded EDGE
  profile) and DDS BC1/2/3 + explicit raw formats in `crysirror-image`.
  Actual conversion uses Rust codecs: all **2,118** topology streams agree with
  the independent Python implementation; **97** low-mip DDS previews decode.
  This is not a comparison against an original/proprietary native decoder.
- `SystemsSpec.environment` initializes validated, client-owned lighting from
  project data. The native preview renders from JSON/glTF/PNG without TS/JS.
  Python validation/inventory includes scoped HDR assets. Existing shared glTF
  PBR and Metal rendering are reused; no title-specific GPU path or console host.
- Compilation: workspace/all-targets, desktop no-default, desktop release and
  CPU transcode example passed. Behavior: Rust **170 passed / 0 failed / 13
  ignored**; explicit Metal GPU **11 passed**; Python 3.11 **117 passed**.
  Ignored GPU/font/game cases remain excluded from the workspace pass count.
  A separate CPU transcode capability-path test also passed. Actual CLI
  validation/render and MCP stdio validate/trace/capture succeeded.
- Capability audit proposes local point/spot lights, per-slot material overrides,
  extra vertex attributes/UV sets/baked lighting and dependency/mip streaming.
  Native ANM/transform and UV decoding are frontend work; engine animation
  already exists. These proposed APIs are not implemented by this trial.
- Evidence and exact limits: [ADR 0017](docs/adr/0017-gow3-scene-capabilities.md),
  docs/gow3-scene-results.json and docs/gow3-scene-mcp.json. Game assets/captures
  stay external. No full-scene fidelity, hardware RT, Windows/Linux or frame-time
  result is claimed.

## Earlier: shader presets and developer WGSL (ADR 0016)

- `Material.effect` uses safe shared CPU `ShaderEffect` data: Fog (moving UV
  noise/density/edges), Fresnel emission, Dissolve coverage/glowing edges, or a
  client-scoped custom WGSL path plus four vec4 parameters. Code stays in separate
  assets; snapshots contain only paths/parameters. No code/device pointers in data.
- Public surface ABI v1 changes linear color/opacity, emission and PBR factors.
  One implementation runs in color, prepass and shadow variants, including
  skinned meshes. The engine retains depth/blend/lighting ownership. Diagnostics
  cover bad WGSL/ABI/resources and driver pipeline validation; no silent fallback.
- CPU `shader-check` CLI and MCP `shader_validate` check all assembled stages
  without a GPU. Schema, inventory, Rust/TS APIs, saves and external import
  profiles support effect data/code assets. `examples/shader-effects` runs as
  a script-free project; dist/shader-effects.png is its actual Metal capture.
- The native castle's **42 fog sheets / 7,709 nodes / 13,426,500 scene triangles**
  render using the Fog preset and source color/opacity/depth parameters, replacing
  the prior self-authored opacity mask. Noise/flow/edges are engine approximations;
  full UE material/lighting/gameplay fidelity is not claimed. User assets stay
  external. The source sky sphere remains the only omitted geometry instance.
- Compilation: workspace/all-targets, desktop release and no-default checks passed.
  Behavior: Rust **166 passed / 0 failed / 13 ignored**; explicit Metal GPU
  **11 passed**; Python 3.11 **110 passed**. Ignored = eleven GPU, one font and
  one RFVP-game test, excluded from workspace passes. Actual MCP validation,
  scoped shader compilation, path rejection, effect trace and capture succeeded.
- Limits: surface hooks only, no custom bindings/vertex/post/compute replacement,
  hot reload or shader graph. Fog is a sheet effect, not global/volumetric fog.
  No new hardware RT, GoW geometry, Windows/Linux or GPU frame-time result.
- Contract and evidence: [ADR 0016](docs/adr/0016-shader-effects.md),
  docs/shader-effects-results.json and docs/shader-effects-mcp.json.

## Earlier: engine material system and transparent PBR (ADR 0015)

- Shared, device-free `MaterialSpec` in crysirror-scene: PBR factors/maps,
  opaque/mask/blend/additive/premultiplied, unlit, independent opacity,
  shared UV0 transform and soft depth intersection. Project entities, Rust
  commands, `Entity.setMaterial` in JS/TS, snapshots and MCP use this data.
  Canonical texture paths and GPU override caches remain client-scoped.
- Renderer: distinct transparent static/skinned pipelines, back-to-front sorting,
  opaque depth tests without writes, transparent HDR composition after opaque TAA
  and before bloom/tonemap. Mask cutoff now also applies in shadow rendering.
  Standard glTF BLEND and unlit import correctly; a versioned engine-owned extras
  envelope carries additional shared material data. No title-specific renderer code.
- Behavior: script-free `examples/materials` rendered all five alpha modes;
  actual MCP stdio validated/traced/captured it. Native castle capture retained
  **all 42 fog sheets: 7,709 nodes / 13,426,500 submitted triangles**. Only the
  original sky sphere is omitted. Fog uses recorded source color/opacity/depth
  parameters and a self-authored mask; UE noise/projection/panning remains an
  explicit approximation. User content/profile/capture stays outside the repo.
- Compilation: workspace/all-targets, desktop no-default and release builds passed.
  Behavior: Rust **162 passed / 0 failed / 11 ignored**; explicit Metal GPU suite
  **9 passed**; Python 3.11 **109 passed**. Ignored = nine GPU, one font and one
  RFVP game test, excluded from the workspace pass count. No-default retains an
  existing unused-mut warning; the external developer bridge has upstream warnings.
- Limits: center sorting, single-sample transparency outside TAA history; no OIT,
  refraction, complete UE shader graphs or jointly sorted particles. No new GoW
  mesh decoder, hardware RT, Windows/Linux or frame-time result.
- Contract/limits: [ADR 0015](docs/adr/0015-engine-materials-transparency.md).
  Evidence: docs/engine-materials-results.json and docs/engine-materials-mcp.json;
  native demo: dist/engine-materials.png.

The sections below retain earlier checkpoints; their missing-feature statements
and test counts describe those checkpoints, not the latest implementation.

## Earlier local game asset trial (2026-10-25, before transparent materials)

- Ashen Vow's actual FantasyCastle static geometry was exported from the local
  UE 5.7.4 shipping build and rendered by CrySirror on Apple M5/Metal. Native
  extraction: 132 distinct meshes / 7,710 instances; preview: **7,667 nodes,
  13,426,416 submitted conventional-LOD triangles, 31 base-color textures**.
  One sky sphere and 42 unsupported fog-sheet instances are explicitly omitted.
  Materials are approximate, original lights/gameplay/landscape/characters are
  absent; this is partial scene rendering, not full-game compatibility.
- Existing static reflection recovery now exports interoperable `.usmap` data
  (7,312 schemas / 1,276 enums); 100 sampled native mesh components parsed with
  expected tails. Developer-only CUE4Parse bridge, preview converter, bounded
  PSARC reader and partial WAD inspector are retained as tools.
- God of War III (Japan): 16 PSARC directories / 6,041 names read; hero WAD
  decompressed and all 885 records inspected. The selected Olympus level fails
  explicitly on unframed `EntityCount` at `0xd60`; no mesh decoder or scene
  rendering is implemented. No console backend was added.
- Compilation: workspace/all-targets check and external bridge build passed.
  Behavior: workspace **158 passed / 0 failed / 8 ignored**, Python **106 passed**,
  actual Metal screenshot produced. GPU smoke suite was not rerun; ignored tests
  remain excluded. No GPU frame-time or original-image-equivalence result.
- Extracted content and screenshot remain outside the repository. Commands,
  limitations and metadata evidence: [LOCAL_GAME_IMPORT.md](docs/LOCAL_GAME_IMPORT.md)
  and docs/local-game-import-results.json.

## Earlier: gameplay systems (ADR 0014)

- GAS-style gameplay (attributes, effects, tags, abilities, cues), animation
  state machines/blend spaces/montages with notifies, reactive behaviour-tree
  AI with a nav grid baked from collision (A* plus string pulling), CPU
  particles in the HDR pass with warm-up, procedural/heightmap terrain with
  heightfield collision, level streaming with hysteresis, spatial audio
  (core `SetAudioGain`), and the IoStore/Zen reader for the Ashen Vow
  assessment (ADR 0013).
- examples/aaa-systems (valley hunt): AI hunters path 58 m → 1.4 m and
  bite on cooldown; player HP 100→10 with bonfire healing;
  dist/aaa-systems.png. Found and fixed a parry heightfield vertical-ray
  miss (regression-tested).
- Evidence: `cargo test --workspace` **156 passed / 0 failed / 8 ignored**
  (6 GPU, 1 font, 1 RFVP game; not counted as passes); explicit GPU run
  **6 passed** (incl. terrain + HDR particles); Python 97 passed;
  workspace and no-default-features checks pass. The API coverage test
  maps all 18 world command variants. GPU frame time is not measured yet.

## Earlier: animation, physics, input, playable slice (ADR 0012)

- glTF skeletal animation (skins, T/R/S channels incl. cubic spline,
  4-layer blending, cross-fades as saved world state) with GPU skinning in
  the opaque, prepass and shadow passes, plus previous-frame palettes for TAA.
- Rapier 0.22 physics derived from World colliders: trimesh/box/sphere/
  capsule, a kinematic character controller (slide, autostep, slopes, ground
  snap) and raycasts. Input action maps over 19 named keys. `run --script`
  makes projects playable in a window.
- examples/aaa-fox-sponza: third-person Fox in Sponza (WASD/Shift/Space,
  orbit camera with raycast collision, Survey/Walk/Run blends) →
  dist/fox-sponza.png and dist/fox-sponza-gaits.png. Release CPU simulation
  about 0.3 ms/frame; GPU frame time not measured yet.
- Evidence: `cargo test --workspace` **144 passed / 0 failed / 7 ignored**;
  explicit GPU 5 passed; Python 97 passed; workspace and no-default checks
  pass.

## Earlier additions (2026-10-24, late night)

- Renderer: 4 stable cascaded sun shadows, a depth/normal/motion prepass,
  SSAO with a depth-aware blur, and TAA (Halton jitter, reprojection,
  YCoCg clamp). The DamagedHelmet and Sponza demos were re-rendered on
  Apple M5/Metal (dist/aaa-*.png; release wall time about 1 s each, which
  includes asset loading and 16 TAA accumulation frames, not frame time).
- API coverage: Layers/Ui/Audio/Save classes; World.camera/environment and
  Entity.angularVelocity/color/geometry. `COMMAND_API` and
  tests/api_coverage.rs require an API method for every World/Layer/UI
  command variant, and that each method appears in the generated .d.ts. A
  JS test drives layers, UI, camera, environment and save/load rollback.
- ADR 0011: game-port research. OpenLara (BSD-2) is the Tomb Raider target;
  Zelda projects lack permissive licenses and are reference-only.
- Evidence: `cargo test --workspace` 138 passed / 0 failed / 7 ignored;
  explicit GPU tests 5 passed; Python 97 passed; workspace and
  no-default-features checks pass.

## Previous: AAA PBR renderer, RFVP running, Siglus vendored

- **AAA rendering (ADR 0010), implemented:** glTF metallic-roughness
  materials with textures, a 4×MSAA HDR pipeline, GGX/Smith/Schlick shading,
  normal/AO/emissive maps, alpha mask, double-sided materials, IBL
  (GGX-prefiltered HDR, SH9 irradiance, split-sum LUT), HDR sky, a 4096²
  PCF sun shadow map, dual-filter bloom and PBR Neutral/ACES tonemapping.
  Script API: `world.setEnvironment`, `world.setCamera`,
  `importModel(..., {pbr:true})`.
- **Showcase ports executed on Apple M5/Metal:** examples/aaa-helmet (port of
  three.js webgl_loader_gltf: DamagedHelmet + Royal Esplanade HDR) →
  dist/aaa-helmet.png; examples/aaa-sponza (Sponza, 103 primitives, about
  70 textures, sun shadows) → dist/aaa-sponza.png. Assets are fetched with
  tools/fetch_demo_assets.py (pinned SHA-256, licenses recorded) and not
  committed.
- **RFVP (ADR 0008):** the original engine is vendored (crates/rfvp; fonts
  removed and host-supplied) and runs as the `rfvp` VmAdapter through
  ClientStorage/present_surface. rfvp's win95_painter_demo produces
  framebuffers bit-identical to the unmodified upstream headless host
  (docs/rfvp-differential.json). The demo's click-to-paint does not work in
  either host; this is an upstream finding.
- **Siglus (ADR 0009, paused):** the full upstream SceneVm and its 8
  dependency crates are vendored. They build headless and with an offscreen
  original renderer. The embedding host and fixture game are not done.
- Core gained `ClientStorage`, `present_surface`, `service::buttons`
  and the `VfsProvider` mounts.

Evidence: `cargo test --workspace` **136 passed, 0 failed, 7 ignored**
(docs/aaa-rust-tests.txt; ignored = 5 GPU, 1 font, 1 RFVP game). Explicit
GPU run **5 passed**, including the PBR test (docs/aaa-gpu-tests.txt).
Explicit RFVP game test **1 passed** (system Arial supplied at test time).
Python **97 passed** (docs/aaa-python-tests.txt). `cargo check
--workspace --all-targets` and `--no-default-features` desktop: passed.
Vendored upstream unit tests are excluded and documented (UPSTREAM.md files;
eluna has 2 failures that also occur upstream). No frame-time, Windows/Linux,
hardware-RT or upscaler results are claimed.

## Earlier iteration: engine object model, scripting, VFS, Blender assets

Decisions and backlog: docs/adr/README.md (ADR 0004–0007 new).

- Implemented: crysirror-engine (reflective Class/Object API, App lifecycle
  Created/Initialized/Running/Paused/Stopped/Faulted, fixed/update/late hooks,
  plugins, ScriptHost contract; built-in Engine/World/Entity/Fs/Client
  classes over shared runtime services); crysirror-script-js (Boa runtime +
  Oxc TS lowering, generated JS bindings, catchable engine errors, loop/
  recursion limits); `crysirror script-types` emits registry-generated .d.ts;
  core `VfsProvider` + per-client provider mounts in DiskVfs/Runtime;
  crysirror-asset glTF/GLB importer and `world.importModel`;
  tools/blender_export_gltf.py; core `Upscaler`/`UpscalerSupport` contract.
- Compilation: `cargo check --workspace --all-targets` passed
  (docs/engine-api-cargo-check.txt); `cargo check -p crysirror-desktop
  --no-default-features` passed (docs/engine-api-no-default-check.txt);
  clippy reports no warnings in the three new crates.
- Behavior: `cargo test --workspace` **131 passed, 0 failed, 5 ignored**
  (docs/engine-api-rust-tests.txt; ignored = 4 GPU + 1 caller-font test,
  not counted as passes). Explicit GPU run `cargo test -p crysirror-wgpu --
  --ignored`: **4 passed** (docs/engine-api-gpu-tests.txt). Python tools:
  **97 passed** (docs/engine-api-python-tests.txt).
- Executed end to end on Apple M5/Metal: Blender 5.2.2 headless exported
  examples/engine/assets/lantern.glb; `crysirror render
  examples/engine/project.json --script examples/engine/game.ts` ran a TS
  behaviour for 30 frames, imported the model three times (13 entities,
  nothing dropped), and produced dist/engine-api-raster.png and
  dist/engine-api-compute-raytrace.png (docs/engine-api-render.json).
  hardware_raytracing=false.
- A Siglus SceneVm loaded its scene from a memory provider mount and was
  driven to Halted by App frames, visible as a `Client` object (tests).
- Not done: DLSS/FSR/XeSS/MetalFX (contract only; no motion vectors/frame
  graph; vendor SDKs not distributed), hardware RT (wgpu 0.20), PBR/textures/
  skinning from glTF, component ECS, JS modules/determinism/heap saves,
  archive providers, interactive `run --script` window path, RFVP adapter.

Note: the earlier text below says only crysirror-script-v1 is registered.
That is out of date. The runtime registry now also registers `siglus`
(crysirror-siglus-vm). Its native-fidelity gates in NATIVE_ADAPTERS.md are
still not claimed as met.

## Previous snapshot (shared CPU source migration)

CrySirror now runs engine-owned World, Layer, UI, input/animation/projection,
Text and Audio/SFX services. This iteration directly migrated upstream source
and connected real state, rendering, input and snapshots. It is still not a
complete AAA engine or a linked original Siglus/RFVP interpreter.

## Implemented source

- Thirteen workspace crates; crysirror-desktop remains the default executable.
  Core remains no_std/dependency-free; core/compat stay safe and device-free.
- New crysirror-scene contains the complete pinned Siglus input.rs,
  int_event.rs and render_math.rs ports plus RFVP letterbox/hit helpers and
  generalized capture/release behavior. Per-file provenance, exact source
  SHA256, license and changes are in its UPSTREAM.md/upstream-files.json.
- World owns scoped generational entity handles, validated transactional
  mutations, cascading removal, parent transforms/visibility, velocity/angular
  velocity, fixed 16 ms steps, fractional remainder and game/real-time tweens.
  World extraction bakes dynamic hierarchy transforms into renderable triangles.
- Layer owns retained hierarchy, mutation batches, pivot/anchor/rotation/scale,
  inherited opacity/visibility, clips, explicit order pairs, UVs and original
  camera/billboard/culling projection. CPU data includes homogeneous W; GPU
  interpolation is perspective-correct. Rotated clips use four-corner AABBs.
- UI owns Panel/Label/Button/Toggle/Slider trees, layout/padding/gap, clipping,
  focus/navigation, pointer capture/release, modal input routing, value/event
  state, typed World callbacks and transactional mutations. UI consumes owned
  input before VMs, preventing Enter from also resuming a background wait.
- ClientSpec.systems installs those systems independently of VM progression;
  World continues updating after data VMs halt. Desktop/capture extract the
  same scene and overlays for raster and compute rendering.
- Renderer has executed normal/add/subtract/multiply/screen pipelines and
  scissors; it remains independent of winit. Desktop owns windows/event loop.
- Runtime snapshots include engine state and fixed-step/animation clocks.
  Systems projects use services schema 2; legacy flat projects retain schema 1.
  Restore validates scopes/hierarchies/render extraction before live swaps;
  transient UI capture is cleared and held input must first release.
- MCP project validation/replacement and asset inventory include systems.
  With a configured host, engine_trace returns actual fresh-session world/UI
  state. scene_capture accepts raster/compute plus bounded frames/step_ms and
  Enter pulses. There is no in-process live game control API.
- Existing crysirror-text owns fonts/simple layout/positioned glyph rasterization;
  crysirror-audio owns voice lifecycle/buses/AudioDevice with optional rodio.
- Existing crysirror-siglus preserves native G00 cut/frame/pivot rules and its
  actual pinned runtime decoder, using common image/endian/JPEG primitives.
  Generic sprite usage rejects unrepresentable native metadata/multiple frames.
- Optional crysirror-author retains Rust Oxc TS/JS compilation plus Boa offline
  buildProject execution. Runtime JSON needs no JS engine. No per-frame JS was
  added; original native VMs remain separate from the built-in data VM.

## Compilation evidence

Local host: macOS arm64, Apple M5 Metal; Rust/cargo 1.98.1 and Python 3.11.
Desktop targets remain Windows/Linux/macOS. A minimum Rust 1.88 is declared from
resolved requirements; it was not independently tested here.

- `cargo check --workspace --all-targets`: passed;
  docs/engine-systems-cargo-check.txt.
- `cargo check -p crysirror-desktop --no-default-features`: passed;
  docs/engine-systems-no-default-check.txt.
- `cargo test --workspace`: **103 passed, 4 ignored**, no failures;
  docs/engine-systems-rust-tests.txt. Ignored tests are three GPU tests and one
  caller-supplied font test; those are not counted as passes in this run.
- Python tools: **91 passed**;
  docs/engine-systems-python-tests.txt.
- Cargo reports the existing block 0.1.6 future-incompatibility warning.
  No successful Windows/Linux build or hardware-RT execution is claimed.

## Behavior evidence

- tools/compare_scene_upstream.py verified pinned revisions and SHA256, compiled
  the actual original CPU algorithms and compared **175,700 observations**:
  input edge state, integer events across clocks/delays/easing/modes, projection
  and RFVP letterbox mapping all agreed. docs/engine-systems-differential.json.
- CPU tests exercise stale/scoped handles, hierarchy/cycle rollback, negative
  scale winding, fixed-step chunking and save continuity, pause versus real
  time, UI capture/release/focus/modal/slider, engine mutation rollback, input
  consumption, post-halt simulation and transactional engine restore.
- Explicit GPU run: **3 passed, 0 ignored**;
  docs/engine-systems-gpu-tests.txt. Tests check actual legacy/native-image
  pixels, dynamic World extraction on raster AND compute paths, UI preservation,
  scissor exclusion, and all five blend equations in linear color.
- Explicit local-font CPU run: **1 passed, 0 ignored**;
  docs/engine-systems-font-test.txt. Arial was supplied from the local OS for
  this run only. No font binary was copied into the repository or packages.
- Real GPU captures: dist/engine-systems-raster.png and
  dist/engine-systems-raytrace.png. The latter includes actual Text/UI services
  and dynamic World at frame 64; docs/engine-systems-render.json records the
  real adapter/runtime. Compute traces primary/shadow rays; hardware RT is false.
- Actual MCP stdio engine_trace executed 64 frames and its Enter pulse activated
  the pause-world UI button. MCP scene_capture returned real compute PNG bytes;
  asset inventory included the layer's client-scoped PNG. Evidence:
  docs/engine-systems-mcp.json.
- Previous executed G00 original-decoder differential and hardware-RT rejection
  evidence remain in the resource/renderer records referenced by ADR 0003.
  This iteration adds no original VM hooks, game runs or original-save claims.

## Remaining work and limits

| Item | Actual status |
|---|---|
| Original Siglus SceneVm/syscalls/forms/waits/native saves | Not linked; retain independent compatibility implementation |
| Original RFVP VM and native acceptance | Not linked; independent adapter still required |
| Complete upstream semantic audit | Selected migrated modules reviewed; full native VM/text/audio/save audit pending |
| Full modern 3D feature set | No general query ECS, physics/character controller, skeletal animation, combat, terrain or asset streaming |
| Layer/native effects | Scissor AABB clips and standard blends implemented; no native shader/effect equivalence; 3D layer parents fail visibly |
| Text fidelity | Basic horizontal and explicit positioned glyphs; no full shaping/ruby/reveal/native profiles |
| Audio fidelity | Voice lifecycle/bus execution; native seek/fade/wait policies and project/save bus schema remain absent |
| Hardware RT/PBR/BVH/reflections/GI | Not implemented; hardware requests fail visibly |
| Compute scale | Opaque triangles, linear traversal, <=4096 triangles, <=1,048,576 pixels, <=256 million pixel/triangle pairs; buffers rebuilt per frame |
| Runtime performance | Correctness profile with state staging/validation and per-frame mesh uploads; no AAA throughput claim |
| Desktop interactive input/audible output | CPU/GPU behavior exercised; human window interaction and audible output not validated here |
| Windows/Linux behavior | CI targets them; not executed locally in this iteration |
| Mobile/Web | Later, per user direction |
| Switch/PS Vita | Deferred; neither built/tested; no other console scope |
| Live JS execution/type checking | Absent; current TS/JS profile is synchronous offline authoring |
| CI/releases | CPU reference comparison job extended; workflow not triggered; release bundles not regenerated |

Only crysirror-script-v1 is registered as a VM. Unknown native adapters and text
profiles fail visibly. A successful common subsystem test is not evidence of
native game compatibility. Snapshot identity hashes project data; bump
content_revision for changed external assets. Device pointers/code do not enter
project/VM data, GPU state is rebuilt and audio restores at sample zero.

Complete reference checkouts remain outside the deliverable at
/Users/xmoe/.cache/crysirror/upstream-20261024, with actual pins in upstream-lock.json.
Only attributed selected source was migrated; no game data, font, credential,
SDK or proprietary binary was imported. Workspace has no .git; no remote or PR
was created. Older bundle/hash/missing-toolchain records are historical and do
not validate this source. tools/verify.py/release recovery were not rerun.


## Codex parallel service work: audio, text, input and localization (2026-10-25)

This appended checkpoint owns ADR 0030–0033. Renderer, scene view/components,
asset mesh clustering, GoW tools and probe work remain with the parallel author.
Earlier historical descriptions above, including the source-export Git state,
retain their original checkpoint meaning.

- Implementation: shared CPU SFX pools with scoped generation handles and
  priority stealing; seeded weighted variants, stereo spatial attenuation/pan,
  occlusion lowpass, client buses/ducking, sample-level fades/crossfades/loops,
  WAV/MP3/Vorbis/FLAC streaming and rate/pitch. The Audio class and App feed one
  mixer; optional device output stays inside crysirror-audio, not desktop rodio.
- Text: rustybuzz shaping, bidi, UAX #14 + documented CJK tailoring, fallback,
  ruby/vertical layout, rich styles/icons/outline/shadow, grapheme reveal/events
  and CPU SDF atlas data. The engine Text class publishes scoped RGBA surfaces;
  localization catalogs choose CLDR plurals and font aliases. Input adds runtime
  rebinding/profiles, analog/dead zones, optional GilRs and prompt tokens.
- Compilation: workspace/all-targets, desktop release and no-default checks
  passed; audio/text strict clippy passed. Existing no-default unused_mut and
  dependency future-incompatibility warnings remain.
- Behavior: workspace **219 passed / 0 failed / 27 ignored**, plus a new actual
  Oxc/Boa service integration test **1 passed** after that workspace run:
  **220 unique normal CPU tests verified**. Python 3.11 **126 passed**.
  Explicit licensed-system-font tests **4 passed** separately. The 27 default
  ignored cases (22 GPU, 4 text-font, 1 RFVP-font fixture) are not workspace
  passes. Audio's 15 tests include real sample assertions and compressed seek;
  device queues and input use offline/injected tests, without device opening.
- Limits: CPU SDF data only, no SDF GPU/3D pipeline. Advanced audio/session
  configuration is not automatically in Runtime snapshots; explicit text,
  localization and binding checkpoints are documented. Audio decode/mixing
  remains synchronous on App, with resampling pitch affecting duration; no
  HRTF, hard-real-time, physical soundcard/gamepad or new GPU validation claim.
- At owner request Git was initialized and origin set to
  `git@github.com:xmoezzz/CrySirror.git`; SSH read succeeded, with no remote
  branches at configuration. No commit or push was performed on parallel work.
  Fonts are excluded by Git patterns; no font files were added.
- Evidence: docs/codex-services-results.json, codex-services-*.txt, and
  ADR 0030–0033. Actual CPU typography/SDF previews remain under
  ~/.cache/crysirror/codex-text, outside the source export.

## Codex native coverage audit and local BGI trial (2026-10-25)

This appended checkpoint owns ADR 0034–0037. Earlier native adapter descriptions
above are historical; docs/NATIVE_ADAPTERS.md now describes the current factory
and service boundaries. No protected renderer/scene/asset/GoW code was changed.

- Implementation: added native Siglus fixture generation and three adapter tests
  covering DECIDE release edges, checkpoint/original-save round trips and
  transactional rejection, plus CD_TEXT/CD_NAME rejection. No VM opcode or
  syscall semantics were changed. Current `siglus` still constructs the subset,
  not the complete vendored SceneVm. Existing native message/audio/movie/effect
  bridges are incomplete despite the shared engine APIs from ADR 0030–0033.
- Compilation: `cargo check --locked --workspace --all-targets` passed. The
  separate local ethornell CLI and cached CPU-host example both built in release;
  upstream warnings remain. The original ethornell dirty-source patch hash was
  unchanged after compiling. Cached experimental source is not a shipped crate.
- Siglus behavior: 42 focused VM/runtime tests passed, 0 failed/ignored;
  Python 3.11 tooling 126 passed. Actual native PCK fixtures executed through
  release CLI, including raster G00 and compute-with-3D overlays. Native TEXT
  and NAME fixtures exit 1 with the explicit missing-message-layout error.
  Original save tests cover supported synthetic checkpoints, not a full game.
- RealLive/AVG32: both independent VMs already exist in pinned siglus_rs at
  5f358ef1360b0439e92aed2b749e8f25d8721954. External AVG32 tests passed
  33 library + 1 NLS + 4 native synthetic Engine cases; RealLive passed
  58 library tests. Missing-installed-data early returns were excluded. Neither
  family is vendored/registered in CrySirror; no installed game was run for them.
- BGI behavior: actual local senmomo trial title, Japanese story and click-driven
  next sentence captured at 1280×720. Rebuilt upstream CLI strict run completed
  3600 frames. A separate cached HostRuntime CPU experiment also completed
  3600 strict native ticks with renderer=false/audio=false, injected time/input,
  read-only resources and a separate user root. Original game/source were not
  edited; startup marker was moved only in the independent APFS execution copy.
- Limits: BGI is not a CrySirror factory; these captures still use its native
  CPU text/compositor. No native/shared text/audio extraction, full-game finish,
  audio correctness, movie, original save/load or native GPU effect-equivalence
  claim follows. Game images/fonts/source experiments remain outside the repo.
- Evidence: docs/native-audit-results.json, siglus-runtime-verification-results.json,
  siglus-service-coverage.json, native-family-audit.json and
  bgi-local-trial-results.json. Tests from different scopes are reported
  separately rather than added together as workspace coverage.

## Codex shared capability extraction: Bitmap API (2026-10-25)

The owner's correction changes the acceptance target: use existing native
implementations to enrich CrySirror, rather than demonstrate standalone games.
The earlier BGI experiment remains historical evidence, not shared-service
extraction. ADR 0038 records this delivered capability increment.

- Implementation: crysirror-image now owns checked CPU bitmap fill, clipped
  copy/blit, opacity/mask math, RGB color-key and nearest stretch. RealLive
  /255 and AVG32 premultiplied /256 arithmetic are explicit profiles alongside
  standard source-over. Native VM/presentation/buffer policies stay separate.
- API: new Bitmap class/global supplies create/load/fill/blit/stretch/info/remove
  to Rust and JS/TS through the existing registry. Results use the existing
  client-scoped surface store, Layer transform/filter/composition and budgets.
  Asset/provider reads and generated native images are supported. Operations
  stage pixels before publication, including source snapshots for self-blits.
- Native caller: corrected Siglus CENTER_REP translation to shared transform
  anchor/pivot/depth. Actual native setters execute through the registered VM
  and EngineSystems regression, without changing opcodes or original saves.
- Compilation: workspace all-targets cargo check passed. Focused image/engine/
  JS/Siglus/runtime cargo suites passed **94 tests / 0 failed / 1 ignored**;
  the ignored font test was not counted. Final Python tooling passed
  **132 tests**, including six new source/pin/path guard tests for the
  reproducible upstream bitmap comparison tool.
- Behavior: comparison against unchanged pinned upstream matched all
  **19,200** RealLive Copy/Mask cases, **9,216** AVG32 premultiplied cases and
  **12** nearest stretch cases. Rust and actual Oxc/Boa tests verify provider
  loading, generated images, alpha policies, self-copy, clipping, failed-write
  rollback, client isolation and existing shared Layer composition parameters.
  tools/compare_bitmap_upstream.py and tools/bitmap_reference.rs reproduce the
  original-source comparison, with pin/dirty-source/output-path validation.
- Limits: no new GPU execution, complete native VM port, bilinear/external mask,
  AVG32 fade or automatic bitmap snapshot/save compatibility claim. No original
  game assets/fonts or protected renderer/scene/asset/GoW code were changed.
- Evidence: docs/bitmap-capabilities-results.json, related logs, BITMAPS.md,
  crysirror-image/UPSTREAM.md and native-audit-siglus-transform.md. Shared Rust
  primitive APIs and script APIs execute the same implementation.


## 2026-10-25 Codex: shared resource formats and Rapier physics

- Implementation — resources: added the safe CPU `crysirror-resource` contracts,
  registry, random-access archive streams, bounded decoding and scoped key/transform
  pipelines. The optional GPL `crysirror-formats-arc` crate registers BGI ARC,
  Majiro ARC, SAR/NSA and XP3. Its complete pinned inventory records 289 decoder
  IDs: 3 ported, 2 partial and 284 pending. Existing BGI VM integration remains
  the ADR 0039 adapter; a resource plugin does not register another VM.
- Implementation — engine integration: `Resources` exposes metadata, profiles
  and archive mounts through the shared object API. Bitmap/Fs/Assets consume
  those mounts. Client storage handles observe later mounts; callbacks release
  the mount lock first. Mounts invalidate client asset/physics caches and retain
  generated resource identities. Generic encryption interfaces do not imply
  universal game-key or crypt-filter support; keys are not saved in project JSON.
- Implementation — physics: `crysirror-physics` directly adapts Rapier 3D 0.22.0;
  Rapier performs integration, collisions, CCD, joints and sleeping. Runtime owns
  the persistent service, called once inside each 16 ms World step, including
  pure JSON projects. Scene declarations and Physics/PhysicsJoint/Entity APIs
  cover rigid bodies, materials/groups/sensors, forces/impulses, queries/events,
  joints and kinematic targets. Client/generation checks and incremental caches
  retain ownership and stable handles. Logical snapshots preserve velocities,
  forces, targets and joints, including explicit impulses while paused.
- Compilation: `cargo check --workspace --all-targets --features
  crysirror-desktop/arc-formats` passed on local macOS aarch64. API docs were
  regenerated and the signature fingerprint was logged in docs/api/CHANGES.md.
- Behavior: final workspace tests with desktop/arc-formats passed 368 tests,
  0 failed and 35 ignored. The ignored GPU/font/external-corpus cases were not
  counted as passes. Python tooling passed 157 tests. Focused physics evidence
  separately records 25 backend tests, 13 Runtime/object/legacy regressions and
  one real Oxc/Boa TypeScript test; these are subsets of the workspace result.
  Resource tests cover limits/truncation/key isolation, cross-block AES seeks,
  synthetic large streams, actual archive-to-Bitmap pixels, provider visibility
  and client cache invalidation. A separately invoked external-source test passed
  against 14 upstream synthetic fixtures and original declared expected bytes;
  the C++ executable was not built or run and no fixture binaries were copied.
- Measured performance: release Rapier-only primitive workloads on this Apple
  M5 host averaged 0.682 ms per active falling step for 512 cubes and 2.380 ms
  for 2048; unchanged sync recreated zero bodies/colliders. This does not measure
  whole-engine frames or a commercial level. Raw and summarized evidence is in
  docs/physics-benchmark-results.json and target/verification/.
- Limits: 284 arc_unpacker decoders remain pending, with variant boundaries
  recorded for the five registered IDs. No new commercial-game, GPU or other-OS
  behavior is claimed. Compressed entries are currently materialized under
  decode limits; retained-byte accounting is not a bound on all process memory.
  Physics tooling, cloth/soft bodies/fluids and exact solver-cache snapshots
  remain future work. Original native opcode/wait/save policies remain adapters.
- Evidence: docs/RESOURCES.md, docs/PHYSICS.md, ADR 0044/0045,
  docs/physics-integration-validation.json and docs/resource-physics-validation.json.


## 2026-10-25 Codex: all pinned image decoder IDs on the shared resource path

- Implementation: all 96 image decoder IDs in the complete pinned arc_unpacker
  inventory now have concrete registered Rust implementations/adapters, with
  80 ported and 16 partial entries. These include TLG5/6, QNT/PMS, G00/PDT, PB3,
  ERI, CBG, RCT/RC8, PGD, NVSG, GIM/GTF/XTX, wrappers and palette/mask codecs.
  Standard entropy codecs reuse shared libraries; no original VM, renderer or
  unpacker executable is embedded. The five archive IDs remain implemented;
  188 other archive/audio/file decoders remain pending in the complete ledger.
- Implementation: the safe CPU ImageFormat/ImageDecodeContext registry handles
  specificity/path hints, explicit ambiguity, scoped bases/palettes/masks,
  bounded recursive decoding and caller key/transform services. Bitmap, text
  icons, retained layers and renderer uploads share Runtime Assets decoding.
  glTF/PBR accepts that decoder for native textures and engine mip generation.
  Resources adds imageFormats/probeImage/decodeImage; API docs were regenerated
  and the fingerprint logged. Native VM cuts/pivots/messages/saves stay adapters.
- Compilation: final cargo check --workspace --all-targets with
  crysirror-desktop/arc-formats passed on macOS aarch64. Optional format plugins
  remain explicitly enabled by arc-formats.
- Behavior: final workspace tests passed 464 tests, 0 failed, 42 ignored.
  Python 3.11 tooling passed 162 tests. Ignored cases are not passes. The image
  groups separately executed seven external tests with 177 output comparisons:
  174 exact dimensions/RGBA, three documented AJP/JPEG/WebP lossy rounding
  differences. Eighty-eight ordinary image tests use generated resources;
  original external fixture binaries are not distributed.
- Behavior: three end-to-end engine tests verify SAR→QNT/TEYL→Bitmap/Resources,
  native glTF textures and exact sRGB/linear mips, caller key/client isolation,
  same-client reference/mask reads, strict denied/corrupt errors and failed
  decode preserving the prior surface revision. Common registry tests cover
  priority/ambiguity, recursion/path escapes and invalid outputs. Dispatch
  tests cover shared raw-mask layouts and strong PNG signatures over weak hints.
- Actual GPU behavior: an Apple M5 Metal raster capture loads self-authored
  TEYL, PGX and NVSG data from SAR through Resources.mount and ordinary retained
  layers. All 49,152 native pixels match the PNG reference region, and 16
  interior samples equal their source RGBA. Reproduce using
  tools/make_native_image_smoke.py and the command in docs/RESOURCES.md.
  images/native-image-registry.png contains only these generated test colors.
- Limits: named variants, key/filename/dependency constraints and fidelity
  boundaries remain in docs/arc-image-support.json; 96 registered decoders do
  not imply every game/format variant works. This does not register RealLive or
  AVG32 VMs or claim a commercial level, other GPU/OS or hardware-RT result.
- Evidence: ADR 0046, docs/arc-image-validation.json, per-group source/hash and
  behavior records, docs/RESOURCES.md and target/verification/arc-image-*.log.

## 2026-10-26 — Original RealLive through shared CPU services

- Implementation: added dormant `crysirror-reallive` and the attributed
  original `reallive-original` VM from actual siglus_rs commit
  `5f358ef1360b0439e92aed2b749e8f25d8721954`. Desktop registers `reallive`
  by default. Original Seen instructions, values, waits, resource metadata,
  text layout and savepoints remain independent of the data VM.
- Shared implementation: CPU Surface kernels, PDT10/11, glyph coverage and
  painting, channel audio, incremental MPEG video/audio, logical input and
  scoped resource/save services. No native window/GPU/audio device is created.
  Native loops can update the shared mixer without replaying the sound.
  Unknown operations, unsupported host requests and worker faults fail visibly.
- Native savepoints: Runtime now supports explicitly transactional native
  imports without requiring a whole-VM snapshot. RealLive stages all fallible
  work once, commits original restore fields, and preserves non-slot global,
  settings, clock/timer and sound state. Invalid late sections leave the live
  VM and queued audio unchanged. RLRSSAV1 is the upstream Rust savepoint format;
  it does not establish Windows binary-save or exact-frame snapshot support.
- Compilation: workspace check, minimal desktop check, and optimized desktop
  build completed successfully. The generated API reference and change log
  agree on fingerprint `22687066e1f5f9c5`.
- Automated behavior: `cargo test --workspace` exited 0, **551 passed / 0 failed /
  50 ignored** across 112 reported suites. Python tooling: **162 passed**.
  Separately executed adapter tests passed all 8 cases, including four normally
  ignored system-font cases; the explicit glyph differential matched 36 cells.
  Shared Surface differential covers 1,920 cases. A read-only local MPEG prefix
  matched 12 video RGBA/PTS frames and two PCM/PTS chunks. Ignored tests remain
  distinct from these explicit executions; original DLL tests were not run.
- Actual game/GPU behavior: the owner-supplied Game_RUSUR ran from disclaimer
  through title/name settings, the opening movie, and the first story pause at
  scene 6030 line 65. Final optimized run: 10,000 ticks, zero unknown operations
  and errors, 51 shared glyph rasters, and 7,679,232 offline stereo audio frames
  with no rejected voices. Both 800×600 title and first-story captures match the
  pinned original VM's reference pixels exactly. A real winit/Metal window ran
  180 ticks with a diagnostic scene-10 start override and muted audio; its title
  capture also matches exactly. The original game's 975 files were rehashed
  after the final runs: no content/size/mtime changes or added/missing files.
- Limits: bounded opening profile only; no complete-game, Windows/Linux,
  hardware-RT or device-audio claim. Committed text/IME and whole-runtime
  snapshots remain unsupported. Native movie scheduling and some effect
  policies remain in compatibility code; movie audio still accumulates bounded
  PCM rather than streaming a long track. Private game/font/captures stay ignored.
- Evidence: ADR 0047, docs/REALLIVE.md, docs/reallive-validation.json,
  docs/reallive-source-audit.json and the vendored UPSTREAM/source manifest.

### BGI extraction boundary audit

- Source inspection confirms real shared image/glyph/channel-audio calls;
  `cpu-embedded` selects embedding.rs rather than the original desktop player.
- Extraction is still partial: rotation/scaling/tone/concentration/process
  kernels, perspective rasterization, some effects and codecs remain active
  inside BGI packages. The earlier claim must not be read as all reusable
  BGI functionality having moved into CrySirror. These kernels should move
  behind shared APIs while native opcode/layout/wait/save policies stay native.
- BGI publishes the composed CPU surface as layer 0; its individual native
  images/text/effects are not yet separate engine Layer/UI nodes. RealLive's
  current surface publication has the same retained-scene mapping boundary.
- This is a call-chain audit, not a new local BGI game run. Details were added
  to docs/BGI.md and docs/NATIVE_ADAPTERS.md without changing BGI production code.


## 2026-10-26 — Hosted engines removed at owner request

- The owner explicitly prohibits engine hosting: integration work exists to
  expand CrySirror APIs and engine-owned state so different original VMs can
  use those capabilities through thin semantic/syscall adapters. A running
  original engine and matching composed screen do not satisfy that goal.
- Deleted the RFVP/BGI/RealLive hosted adapter packages, dedicated embedded
  engine source trees, Cargo members/dependencies/features, desktop factories,
  configuration schemas and hosted integration tests. Old project names fail
  visibly. Previous hosted-integration completion claims are withdrawn.
- Kept reusable engine capabilities: shared image/composition/PDT, glyphs,
  audio/mixer looping, incremental MPEG, input tracks and capability services.
  Existing standalone CBG resource decoding was moved into crysirror-image::cbg;
  its optional format plugin no longer depends on any embedded BGI package.
- Recorded the requirement in AGENTS.md, ARCHITECTURE.md, NATIVE_ADAPTERS.md and
  ADR 0047. Removed hosted setup instructions and regenerated API documentation:
  current fingerprint d233217062201fa1.
- Compilation: cargo check --workspace passed. Behavior: cargo test --workspace
  passed 474 tests, zero failures, 42 ignored; Python tooling passed 157 tests.
  Ignored tests were not counted as passes. These results validate the remaining
  engine and removal, not native game compatibility.

## 2026-10-26 — RealLive VM calls engine-owned APIs

- Implementation: added `crysirror-reallive`, retaining the original Seen
  parser/interpreter, values/banks, call stack and pure builtins from the audited
  siglus_rs pin `5f358ef1360b0439e92aed2b749e8f25d8721954`. Engine operations
  remain pending until CrySirror service calls succeed. No original Engine,
  System, Graphics, Sound, renderer or framebuffer is linked. The desktop
  default enables the new factory; the bare Runtime remains host controlled.
- Shared capabilities: retained Canvas drawing/copy/presentation, positioned
  GlyphRun and engine-clock reveal, ordered-dither GPU transitions, Unicode UI
  TextField editing/query, common PDT/NWA/KOEPAC decoding and engine Media
  resources. Ordinary engine/data/JS clients use the same APIs. Unsupported
  copying/layout/audio/native variants fail visibly. API documentation was
  regenerated; fingerprint `8d83111697a3c377` is logged.
- Compilation: workspace check and minimal desktop check with only the
  `reallive` feature passed; optimized desktop/native GPU test compiled.
- Automated behavior: the clean workspace test run exited 0: **561 passed,
  zero failed, 47 ignored**. Python tooling: **165 passed**. Ignored cases are
  not counted as successful executions. A first workspace attempt encountered
  a rustdoc StableCrateId artifact collision; refreshing affected dev artifacts
  and rerunning the workspace completed successfully.
- Explicit behavior: shared media tests, including the complete local movie,
  passed **6/6** without a VM/GPU/device. The separate opt-in native GPU test
  passed on Apple M5/Metal: title, name input, choice, complete opening movie
  (2,995 decoded frames), then scene 6030 line 65, with 22 retained glyphs and
  five engine layers. Removing foreground/shadow text with the VM PC and clock
  unchanged changed GPU pixels. EOF retired the media overlay. This establishes
  engine-owned state and rendering, beyond merely starting an original VM.
- Source integrity: all 975 original game files were rehashed; no content,
  size, mtime, added-file or missing-file changes. Game assets/fonts/captures
  remain ignored. Only a self-authored synthetic media fixture is committed.
- Limits: bounded opening profile, not a full playthrough or pixel-perfect
  native layout claim. Original save files, VM snapshots, DLLs, unsupported
  window skins/advanced layout, preserving DC growth, transparent non-mask
  copies and queued outgoing music replacement remain unsupported. Media audio
  uses bounded full-track PCM; media resources reject snapshots until a real
  reconstruction contract exists. No device-audio, Windows/Linux or hardware
  ray-tracing result is claimed.
- Evidence: ADR 0048, docs/REALLIVE.md and docs/reallive-source-audit.json;
  ignored logs in target/verification/reallive-api-{acceptance.log,
  workspace-tests-final.log,python-final.log}. Private captures and before/after
  engine inspection are in images/third-party/reallive-api/.

## 2026-10-26 — Engine CPU workers and asynchronous assets

- Implementation: added device-free `crysirror-tasks` with bounded worker
  queues, wakeable Rust Futures, cooperative cancellation, panic reporting,
  worker counters and same-pool blocking-wait rejection. Runtime Assets shares
  an asset lane and a CPU lane across clients; no thread is spawned per asset.
- Actual integration: scoped image decoding and glTF dependency/model imports
  execute on asset workers and publish on the owner thread. Desktop data-VM
  projects defer pending draws while continuing frames. Offline capture and
  native compatibility projects retain explicit first-use waits. Independent
  PBR scene poses execute in bounded CPU batches and are collected in instance
  order before GPU writes. Loaded-resource revisions invalidate stale tickets
  and GPU keys; late cancelled results cannot overwrite refreshed resources.
- API: Rust awaitable reads and `Assets.load/status/cancel/release/stats` with
  the `assets` global. Requests are client/store scoped, deduplicated and
  bounded; renderer overload defers work rather than growing unbounded queues.
  API docs regenerated, fingerprint `df17e49431e56860` logged. ADR 0049 records
  thread ownership, limits and remaining synchronous work.
- Compilation: workspace check and the final desktop check passed.
- Automated behavior: `cargo test --workspace` exited 0, **571 passed / zero
  failed / 49 ignored**. Python tooling passed **165 tests**. Its source-data
  boundary scan now prunes private/generated directories before walking them,
  preventing races with rustc temporary directories during parallel validation.
- Explicit GPU behavior: **2/2** async tests passed on Apple M5/Metal. A blocked
  image provider did not block frame capture or logical ticks; completed pixels
  matched offline resolution. A glTF scene used CPU pose workers and matched
  blocking-mode GPU pixels. A separately executed reflection-probe regression
  passed **1/1**. Pending model imports do not bake empty reflection probes;
  resource-generation changes invalidate capture/GI identities. These explicit
  GPU executions are separate from ignored workspace tests.
- Other behavior: two barrier-synchronized jobs ran on distinct workers;
  Future wakeup, bounded admission, cancellation, failure/panic recovery and
  nested-wait rejection passed. Tests cover slow I/O with advancing Runtime,
  stale-generation rejection, client isolation, shared decode/import caches,
  overload deferral and 37 animated pose results matching serial evaluation.
- Limits: World/VM/script/fixed-step physics remain ordered on their owner
  thread. This is not a general frame task graph or complete simulation/render
  overlap. GPU uploads/compilation, cluster-LOD construction, collision cooking,
  HDR/probe loading and some shader/material/puppet/native paths retain
  synchronous work. Cancellation cannot forcibly interrupt a provider syscall;
  prepared-cache accounting does not bound all transient or GPU memory. No
  whole-engine speedup, other-OS hardware result or hardware-RT result is claimed.
- Evidence: target/verification/async-{workspace-check.log,workspace-tests.log,
  final-desktop-check.log,python-tests.log,gpu-tests.log,probe-regression.log};
  the new focused tests and ADR 0049 are committed without external game data.

## 2026-10-26 — Owner scope: no GitHub CI, engine-wide async requirement

- Removed `.github/workflows/ci.yml`. GitHub workflow `desktop-and-contracts`
  (396111179) was disabled; run 40221613250 was cancelled and subsequently
  verified as completed with conclusion `cancelled`. No other active workflow
  was returned by the repository workflow listing. Existing historical run
  records were not deleted.
- Added accepted ADR 0050: asynchronous/multithreaded execution is required
  across every engine subsystem. It defines task/dependency, completion,
  ownership, backpressure and cancellation boundaries plus subsystem migration
  and acceptance criteria. ADR 0049 is explicitly stage one; the present
  asset/pose work does not complete the engine-wide requirement.
- Recorded the owner's no-unsolicited-CI policy in AGENTS.md and ADR 0050;
  validation stays local unless the owner explicitly requests remote automation.
  Updated architecture, roadmap and README references to match.
- Validation: local Python tooling passed 165 tests; Markdown links and absence
  of workflow files were checked. No Rust/API implementation changed, so no new
  compilation or engine/hardware behavior is claimed for this documentation
  and workflow-removal change.

## 2026-10-26 — Native hardware ray queries and shared PBR sun visibility

- Implementation: migrated the default host to pinned wgpu 29.0.3 (resolved
  core/HAL/types 29.0.4), retaining renderer/event-loop separation. Native
  Vulkan/DX12/Metal adapters are selected by actual ray-query feature bits,
  surface compatibility and AS limits, without vendor-name allowlists. Raster
  devices do not opt into experimental features. Unsupported explicit hardware
  requests fail with adapter/backend reasons. ADR 0051 records this contract.
- Implementation: renderer-owned BLAS/TLAS, opaque primary rays and hard sun
  queries; position-based AS cache, metadata-only uploads, move/removal handling
  and recovery from abandoned encoders. PBR uses shared world/model/terrain/skin
  geometry prepared on bounded workers, its existing depth/normal prepass and a
  combined AO/visibility target. Other PBR shading remains in CrySirror's existing
  pipeline. Engine overlays still compose after the primary-ray output.
- Implementation: CLI reports adapter availability separately from device
  enablement and exposes encoded build/query counters. Counters are not GPU
  completion or performance evidence. Portable API support does not certify
  dedicated RT silicon; that report field remains unverified/null.
- Compilation/CPU validation: renderer unit suite passed 26 tests, with three
  GPU tests ignored in that invocation. The synthetic capability matrix covers
  native backends and vendor identifiers, missing features/limits, software
  rejection and device opt-in. Geometry tests verify shared transformed/skinned
  vertices. Shader validation uses Naga. API documentation regeneration/check
  passed five tests without a signature/fingerprint change. Python tooling
  passed 165 tests. `cargo check --locked --workspace --all-targets` passed.
  The generated API fingerprint remains `df17e49431e56860`.
- GPU backend behavior: explicit unit invocation on Apple M5 / Metal passed
  all six tests (including two actual GPU pixel tests): primary hits/occlusion,
  metadata recoloring without rebuild, moved/removed geometry, abandoned-build
  recovery, and AO-preserving directional shadow/empty-scene output. Evidence:
  `target/verification/hardware-rt-backend-gpu-tests.log`. The pinned Naga Metal
  ready-flag loop issue was reproduced and avoided with single-proceed queries
  for force-opaque triangles; this is not a candidate-processing implementation.
- GPU integration behavior: on Apple M5 / Metal, the 120-triangle primary scene
  matched the existing compute reference within the asserted pixel tolerance,
  composited the engine's green overlay and reused its AS on a repeated frame.
  The 24-triangle PBR floor/cube scene produced a measured shadow-on/off pixel
  difference through native sun queries and rejected a masked material.
  Evidence: `hardware-rt-{primary,pbr}-after-reset.log` and the rendered images
  in `target/verification/hardware-rt/`. Initial renderer tests timed out while
  even an empty GPU submission stalled; those failed runs are not passes.
  Following device/session recovery, the above explicit tests passed. Capture
  now tracks its submitted readback and returns a finite timeout with context.
  The final-source integration invocation passed both tests together in 7.23s:
  `target/verification/hardware-rt-renderer-gpu-final.log`.
- GPU CLI/regression behavior: the real CLI raster/hardware selection and
  encoded-work-report test passed on the recovered Metal device. The existing
  raster PBR sky/lighting/sun-shadow pixel regression also passed. Evidence:
  `hardware-rt-cli-render-final.log` and `hardware-rt-raster-pbr-final.log`.
- Broad validation: `cargo test --locked --workspace --all-targets` ran 678
  passing tests, 55 ignored and two failing tests before stopping in vendored
  `eluna_rs`. Failures were
  `runtime::reverse_parity_tests::timeline_hold_markers_do_not_pollute_authored_ranges`
  (0 versus 3) and `vertex::tests::builds_single_cell_strip` (float UV equality).
  `crates/siglus-upstream/UPSTREAM.md` already records these exact two failures
  on the unmodified pinned upstream (96 passed / 2 failed). Its source and
  dependency set were unchanged by this RT commit; its manifest sets
  `lib.test=false`, which explicit `--all-targets` overrides. This broad command
  is therefore a failed validation, not a green workspace claim. Evidence:
  `target/verification/hardware-rt-workspace-tests.log`.
- Default workspace validation: `cargo test --locked --workspace` passed,
  with 581 passed, zero failed and 55 ignored, respecting the documented
  upstream `lib.test=false` settings. Ignored tests are not counted as passes;
  the explicit Metal runs above provide separate GPU evidence. Evidence:
  `target/verification/hardware-rt-engine-workspace-tests.log`.
- Limits: single world-space BLAS geometry/identity TLAS instance, at most
  1,000,000 primitives; no refit/compaction/per-mesh instancing, RT reflections,
  RT GI or denoiser. Masked/transparent casters and custom fragment-discard
  materials fail visibly. Fixed world-space ray bias is an initial profile.
  Windows/Vulkan/DX12 and NVIDIA/AMD/Intel physical execution remain unverified.
  Core/compat remain device-free and forbid unsafe code; the renderer isolates
  wgpu's required unsafe experimental-consent token at explicit device opt-in.
  No game data, fonts, proprietary SDKs or GitHub workflows were added.

## 2026-10-26 — Frame DAG, profiler, leased streaming and instanced RT

- Implementation: ADR 0052 adds a bounded dependency executor over immutable
  results. Independent PBR/simple/layer extraction runs on workers; desktop
  polls one pending snapshot while retaining one ready snapshot. World/VM/input
  ordering stays on its owner. Dynamic surface generations and transition state
  freeze with snapshots; scoped resource lookup restores live access on error
  or unwind. Restore/store identity rejects stale results.
- Implementation: shared Profiler class/global exports bounded CPU traces and
  stage percentiles. GPU timing uses four asynchronous slots, pass-boundary
  markers, frame-completion callbacks, a later resolve/map submission and
  validated nonzero samples. Capture/shutdown have explicit bounded completion
  fences. CPU/GPU clocks remain uncalibrated; capture wall time is not FPS.
- Implementation: streaming volumes prepare hierarchy/PBR data on workers,
  atomically publish ready cells, reject obsolete work and hold scoped resource
  leases. The renderer shares decoded models and releases unused model/RT
  caches. Explicit caller-owned asset requests retain their lifetime. Host CPU
  prepared-cache budgets default to 256 MiB and are explicitly configurable
  from 16 to 4096 MiB; this is not a total RAM/VRAM limit. Offline readiness
  fences commit resources without another script/physics/time step. Ticket
  validation also rejects forged client fields on another client's request ID.
- Implementation: PBR shadows share local-space BLAS and use affine TLAS
  instances. Cached source/pose data avoids repeated rigid-mesh preparation;
  source Arcs prevent pointer-ID reuse. Static frames reuse TLAS; rigid moves
  rebuild only TLAS. Budgets are 4096 instances/1024 unique meshes/1M unique
  triangles, bounded by device limits. The simple primary path stays separate.
- Compilation/CPU validation: workspace all-target check passed. Default
  workspace tests passed 590, failed zero, ignored 56. Focused tests cover real
  concurrent graph execution/dependencies, pressure retry/failure cancellation,
  delayed workers with advancing simulation, immutable geometry/surface data,
  restore rejection, leased/shared cache reclamation, budget policy, forged
  tickets, streaming hierarchy/collision/save behavior and offline clock fences.
  Python tooling passed 165 tests. Generated API checks passed five tests;
  fingerprint changed `df17e49431e56860` → `e3a0831acf720cb7`, logged with the
  streaming readiness migration in docs/api/CHANGES.md.
- GPU validation: explicit Apple M5 / Metal primary/PBR integration and GPU
  profiler tests passed (2 + 1). Assertions cover matching primary pixels and
  overlays, shared BLAS, static TLAS reuse, moved-instance pixels without BLAS
  rebuild, opaque-profile rejection, explicit raster recovery and actual
  nonzero timestamps. The final profiler run recorded eight valid samples,
  zero failed/dropped/pending. Earlier immediate timestamp resolution produced
  invalid/zero samples; those were rejected and are not successful measurements.
- Public showcase behavior: existing Khronos resource files matched all local
  manifest hashes (2 helmet/HDR files; 73 Fox/Sponza/HDR files). Nine textured
  helmets plus floor rendered as 10 native RT instances/2 BLAS, 139,080
  instance-expanded triangles; eight rotating captures built BLAS only twice
  and TLAS eight times. All eight final GPU timing samples were valid. The
  streaming movie produced entity counts 1 → 3 → 1 → 3 at the expected logical
  radii/time, reclaimed prepared data to zero after unload and reloaded it.
  The prepared cell was 112,737,441 bytes; no game VM/renderer was hosted.
- Public larger-scene behavior: Sponza failed visibly under the default
  256 MiB prepared budget. With an explicit 1024 MiB host budget it rendered
  through CrySirror's raster material path, retaining 392,730,673 prepared bytes.
  Four final GPU samples were valid, with no failed/dropped/pending samples.
  Source alpha-tested materials were not forced into the opaque RT profile.
- Window behavior: an earlier 1800-presented-frame run completed with actual
  10-instance/2-BLAS window rendering and a trace spanning owner/worker threads.
  Its profiler trace predates the final completion-order fix and includes
  rejected samples; use the final GPU tests/offscreen reports for timing
  evidence. A later repeat did not reach its presentation count and was stopped;
  sampling showed continuing owner/surface-acquisition activity. That repeat
  is not a pass. Window visibility/surface timing is not an offscreen result.
- Evidence: target/verification/frame-pipeline-{workspace-check-final.log,
  workspace-tests-final.log,api-docs-final.log,python-tests.log,
  lifecycle-final.log,gpu-final.log}; frame-pipeline/{helmet-report.json,
  streaming-report.json,sponza-report.json,*.png,
  interactive-pre-completion-fix-trace.json,interactive-hang.sample.txt}.
  Recipes/attribution links are in docs/FRAME_PIPELINE.md; downloaded assets,
  captures and traces stay ignored.
- Limits: ADR 0050 remains partial. General simulation parallelism, budgeted
  GPU uploads/pipeline compilation, collision cooking, full resource GUID/cook
  databases and chunk/mip/world-partition streaming remain. World freezing still
  clones logical state; skinned geometry changes rebuild BLAS. RT alpha
  candidates/refit/reflections/GI/denoising, an editor/profiler overlay, complete
  GPU memory accounting and other desktop hardware/platform validation remain.
  No new GitHub CI, proprietary data, fonts or SDKs were added.

## Native VM API expansion and ray lighting (2026-10-26, ADR 0060–0063)

- Implementation: native GGX glossy reflections now sample the actual material
  roughness. Optional per-pixel diffuse GI shares a two-layer array with the
  reflection target, retaining the forward pass's 16 sampled-texture bindings.
  Reflection, probe-GI and pixel-GI hits use the shared local-light buffer.
  Probe dispatches read an immutable history copy, removing inter-workgroup
  read/write races. Native device negotiation checks six storage-buffer
  bindings. Nonuniform-scale hit normals and environment-zero reflection
  attenuation were corrected. These extend the earlier mirror/probe-only limits.
- Implementation: reusable CPU text now provides fixed-cell layout, immutable
  font measurement handles and an unwrapped Canvas layout. JS/TS callers can
  use Text.measureSimple without a VM. KiriKiri Font width/nominal height and
  single-line clipping call engine services. Font overrides are available before
  both native ticks and script init/start; no private font system was added.
- Implementation: Siglus's existing original SceneVm supports explicit plain
  FRONT.MWND[0] TEXT/NAME mappings through shared grid/GlyphRun/reveal/query
  operations. Chunk append, first-name policy, DECIDE consumption and runtime
  snapshots are covered. Other native MWND/read-flag/page/original-save policies
  remain explicit failures; this is not automatic Gameexe window conversion.
- Implementation: AVG32 is a pinned Git library dependency on original
  siglus_rs commit 256c398e06fbcbae8e956eb046c831b3d6c8a593, published on
  codex/avg32-portable-vm without merging upstream main. Its portable build
  compiles the same Scenario/parser/control-flow source; no VM source was copied
  into CrySirror. The dependency tree contains no original System, renderer,
  font/image painter or audio device. The thin adapter issues scoped engine
  Canvas/GlyphRun/UI/audio operations, preserving original pending events,
  waits, hidden choice ordinals, initial RNG state and VM-relative clock.
  Both native saves and framework snapshots remain visibly unsupported here.
- Implementation: Bitmap.composite/filterRgb add engine-owned associated-byte
  composition, separate coverage and integer fade/multiply. Associated output
  is intermediate data, not an implicit conversion to straight-alpha display.
  The new AVG32 profile currently accepts only opaque full-size image loads;
  these primitives do not establish complete PDT effect/mask compatibility.
- Compilation: locked workspace all-target check and the desktop build with
  optional krkr passed. Default workspace tests completed: 623 passed, zero
  failed, 67 ignored. Python tooling passed 170 tests. Generated API checks
  passed 5/5; final fingerprint changed 3ff1c210d5f93836 → 1050976234c41b17
  and is logged in docs/api/CHANGES.md. The CLI registry assertion was updated
  to verify the actual feature set, including AVG32 and optional KiriKiri.
- CPU behavior: AVG32's native bridge passed five tests; its licensed-font test
  was then explicitly executed and passed. The original upstream native engine
  passed 33 library, one NLS and four synthetic tests (one installed-game audio
  case filtered); portable-vm passed 11 tests. Shared bitmap differential tests
  matched the unchanged original pin for 1,776,384 associated cases and 2,304
  signed/clipped fades, plus existing 19,200 / 9,216 / 12 cases. Explicit system
  font tests also passed for engine measurement, script rich text and KiriKiri.
  Ignored font tests in the default suite are not counted as default passes.
- GPU behavior: Apple M5 / Metal explicitly passed all 46 wgpu GPU tests in
  one serialized run. New tests distinguish a rough reflection from the mirror
  footprint and isolate local-light GI with sun/environment/emission disabled.
  Floor red-channel excess was 0 without GI, 29.67 with RT probes, 25.11 with
  per-pixel GI and 0 with the local light disabled. Raster requests for pixel
  RT GI fail visibly. A first GPU-suite setup lacked the requested system font
  and failed that test; the later complete font-supplied run passed.
- GPU performance: same 160×90 room, 24 valid steady GPU frame samples per
  mode, mean 7.578 ms raster probes / 0.810 ms RT probes / 0.644 ms pixel GI.
  Probe modes refresh 16 probes/frame; pixel GI uses 64 rays/pixel. These are
  different representations/sample densities, not equal-quality speedup claims
  or isolated GI timings. RT-probe warmup dropped three timing-ring samples;
  no failed/pending measurements remained. ADR 0060 records p50/p95 and limits.
- Render behavior: the public Khronos DamagedHelmet/HDR files matched both
  recorded hashes and rendered with three roughness panels, a local light,
  native reflections and pixel GI: four instances, two BLAS, eight reflection
  and eight pixel-GI dispatches. Original self-authored Siglus PCK and KiriKiri
  TJS2 examples rendered through engine glyph/Canvas state. The AVG32 TPC32
  example rendered background/text/wait icon and a subsequent native input
  trace reached Halted with 81 engine API calls and no private framebuffer.
  Configured font/cell metrics are explicit; these captures are not commercial
  game completion or proof of matching a title's original fonts/layout.
- Remaining limits: pixel GI is one diffuse bounce, without dedicated GI or
  glossy denoising. Hit shading retains atlas/geometric-normal/constant-emission
  approximations; transparent/transmission and full path tracing remain out.
  Other GPU vendors/platforms were not physically tested. Siglus/AVG32 complete
  native drawing/layout/save support and KAG3 boot remain unfinished. New native
  VM steps publish on the owner; bounded first-use resource reads still need the
  remaining ADR 0050 async migration. No old engine host, new GitHub CI, game
  data, font file, credential or proprietary SDK was added.
- Evidence: target/verification/vn-rt-{workspace-tests-complete.log,
  all-targets-verified.log,python-final.log,api-verified.log,
  avg32-dependencies.log,*-report.json,avg32-trace.json};
  target/verification/avg32/{cpu-tests-final.log,font-test-final.log};
  target/associated-upstream.log; rt0060-all-gpu-font.log and
  rt0060/gi-benchmark.json; rendered PNGs in target/verification/vn-rt/.

## Modular CPU/GPU VFX (2026-10-26, ADR 0064)

- Implementation: added safe, device-independent `crysirror-vfx` definitions,
  compilation, deterministic CPU simulation, typed parameters, multi-emitter
  systems, five spawn shapes, ten update modules, checked lifetime curves,
  world/local space, analytic collisions and bounded deferred CPU child events.
  Engine `Vfx`/`Entity` APIs and data `EntitySpec.vfx` use the same state.
  Legacy `particles` lowers into this core; its renderer-private CPU simulator
  was deleted. No Unreal source, VM host, game data or fonts were added.
- Async implementation: runtime moves simulation ownership into the bounded
  worker pool and publishes immutable frame products. Live frames poll without
  waiting. A bounded input journal retains intermediate parameters/transforms
  while workers are busy; fixed-step debt is drained before the next sample.
  Full journals fail visibly and an explicit offline fence can drain/recover
  them without advancing World/VM clocks. Epochs reject stale restore products.
- GPU implementation: resident 96-byte particle state, parallel update/death,
  deterministic free-slot prefix allocation and parallel birth initialization,
  active compaction, bounded device event records and indirect drawing. GPU
  targets do not run CPU particle loops. Historical tick packets include the
  parameters/transforms at that tick. The owner retains unsubmitted commands
  under packet/byte budgets; receipts advance after queue submission. A second
  direct PBR encode with pending work fails instead of resimulating a tick.
- Render implementation: texture/flipbook sprites, camera/velocity/axis facing,
  soft depth intersections, four blend modes, static mesh instances, CPU ribbon
  bands, per-view culling and GPU alpha sorting. Resource access uses ordinary
  scoped Assets; no extra texture bindings enter the PBR material pipeline.
  Inspection separates CPU counts, GPU capacity/submissions, resident buffer
  bytes and pending commands. Live GPU counts stay null without readback.
- Compilation: local `cargo check --locked --workspace --all-targets` and the
  default desktop build succeeded. Existing image/RealLive warnings remain.
  API reference/types regenerated with 32 distinct VFX data aliases; signature
  fingerprint changed from `1050976234c41b17` to `ff5f3d2050247846` and is logged.
- Workspace behavior: the normal locked workspace suite completed with 659
  passed, zero failed and 76 explicitly ignored tests. The subsequently added
  end-to-end GPU integration test was run explicitly, including its BGRA
  desktop-capture path. Ignored tests are not counted as hardware passes.
- CPU behavior: core 18 tests, runtime 10 tests, engine VFX API 2 tests and API
  documentation 6 tests passed. Tests include paused/stopped/restarted state,
  scoped events, time-zero initialization, transactional type errors, retained
  GPU histories beyond 32 ticks, hidden GPU simulation and submission receipts.
  A deliberately blocked worker produced exactly the same particle arrays as
  a per-frame offline baseline across changing parameters/transforms. Queue
  overflow and subsequent offline recovery also passed. Python 3.11 tooling
  passed 181 tests, including 11 new VFX schema/resource/MCP tests. Strict
  tooling rejects unknown modules, invalid bindings/curves, unsupported GPU
  profiles and competing components. Asset inventory covers initial entities
  and widget spawn/set-VFX textures/static meshes; client/symlink isolation and
  rejected MCP writes were exercised without a GPU or VM.
- Actual GPU behavior: Apple M5 / Metal explicitly passed all five GPU compute
  tests, three render pixel tests, the private cull/indirect test, the engine
  timeline/PBR/submission integration test and the existing terrain/HDR legacy
  particle test (11 device tests total). The integration test submitted every
  tick of a 61-tick history, compared paused pixels, rejected duplicate pending
  encoding, then abandoned/retried encoding without advancing a receipt early.
  Device tests remain ignored in the default suite; their explicit runs are
  separate evidence. Other desktop GPUs/platforms were not physically tested.
- GPU performance: 100,000 resident particles, 24 timestamp samples per scope.
  Update/compact with acceleration, drag and curl: mean 66.448 us, p50 71.250 us,
  p95 95.000 us. Independent parallel 100k birth/allocation/compaction at dt=0:
  mean 358.286 us, p50 115.875 us, p95 761.083 us, maximum 1.991 ms. These
  GPU-only intervals exclude draw, CPU encoding, allocation and image/readback
  cost; the wide burst distribution is retained in the raw JSON. No whole-frame
  or equal-quality Niagara performance equivalence is claimed.
- Integrated behavior: the public DamagedHelmet/HDR showcase rendered through
  the actual desktop JS/TS API with GPU fire, textured flipbooks, CPU soft smoke,
  instanced shards, collision child sparks and a moving world-space ribbon. Four
  consecutive captured frames kept one renderer. The 100k stress script also
  rendered with zero CPU particle arrays, 116 submitted ticks, all 100,000 birth
  requests, zero unreplayed ticks and zero pending commands after capture.
  The same PBR/effects showcase also rendered on the native hardware RT path
  with three RT scene instances, two static BLAS and one TLAS build. Particles
  coexist with that pipeline but do not themselves enter the RT structures.
  Desktop captures now reuse the live renderer, retain resident particle state
  and convert BGRA surfaces into RGBA PNGs; the BGRA pixel test passed explicitly.
  A real winit window also ran the 100k script for 60 presented frames, captured
  its retained GPU particles on exit and exported a trace. Its 60 completed
  whole-frame GPU samples (including startup; simple additive-particle scene)
  measured mean 1.347 ms, p50 0.609 ms, p95 2.378 ms. One timing-ring sample was
  dropped, none failed or remained pending. The explicit offline VFX fence
  occurred only for the final capture, not once per interactive tick.
- Limits: this is a substantial modular VFX profile, not complete Niagara
  parity. Visual graphs, arbitrary custom compute, skeletal/scene/physics data
  interfaces, fluid simulation and effect-level significance/LOD scheduling
  remain unfinished. GPU ribbon/child emitters/replace-oldest fail explicitly;
  CPU supports them. Mesh particles are static unlit geometry with texture/color.
  Transparency is not sorted jointly with ordinary scene meshes. Particle
  probe/RT visibility and exact save/device particle checkpoints remain out.
  Replacing a renderer after its history is retired cannot reconstruct a paused
  GPU simulation; this fails visibly. Canonical restore resets derived visuals.
- Evidence: target/verification/vfx-{core-tests-final,runtime-tests,
  api-runtime-final,integration-gpu,legacy-particles-gpu,all-targets,
  desktop-final-build,desktop-capture-build,integration-bgra-gpu,
  changed-crates-final,all-targets-final,python-complete}.log;
  vfx-gpu-parallel-final.log, vfx-gpu/benchmark.json,
  vfx-render-{gpu,cull-gpu,geometry,cpu}.log, vfx-demo-final.json,
  vfx-stress-final.json, vfx-demo-rt.json, vfx-live.log and PNGs/movies plus
  live-profile.json in target/verification/vfx-demo/.

## 2026-10-26 — Native resource migration and asynchronous conversion (ADR 0065)

Implementation:
- Continued the unfinished arc_unpacker batches and wired every concrete named
  decoder from clean pin `456834ecf2e5686813802c37efd829310485c57d`: 152 archives,
  96 images, 16 audio and 25 file converters. The reviewed ledger records
  **258 ported / 31 partial**, including original-source unsupported branches,
  explicit encrypted-title profiles and bounded Unity/PSB/pickle/XP3 variants.
  There is no pending ID registered as a success stub. This is resource-codec
  migration, not a new native VM or full-game compatibility claim.
- Shared engine contracts now include FileRegistry/DecodedFile, capability-only
  companions, cycle/depth checks, LZSS, generic crypto primitives, WAVE/loop
  serialization, PNG output, channel masks and nontransparent pixel copying.
  Format-specific layouts/codecs stay in the optional GPL plugin. HCA uses
  safe Rust cridecoder; no original-engine process or C/C++ codec is hosted.
- Resources exposes file/audio discovery, paging, converted-file mounts and
  bounded client-scoped decode jobs. Provider-backed source opening and codec
  work both run on CPU workers; owner polling/publication never joins them.
  Cancellation discards late results; mounted readers retain their original
  profile/budget. Synchronous file probe/conversion APIs are offline-only.
- Fixed companion authority lifetime without a VFS/provider reference cycle;
  checked WARC short-tail/index/region-image limits and S25 aggregate row and
  predictor-work limits. Native keys remain host aliases, not bundled tables.
- Regenerated docs/api with fingerprint `d4f4c6edc250ee72`; CHANGES records the
  new APIs, data specs and Rust PixelMath exhaustive-match compatibility change.

Compilation:
- Local `cargo check --workspace --all-targets`: passed.
- Local engine + desktop `arc-formats` all-target check: passed.
- Latest arc-enabled API/file-resource tests compiled the realtime probe gate
  and worker-side mounted-source opening. Existing unrelated warnings remain.

Behavior validation:
- Full local `cargo test --workspace`, including documentation tests: **780
  passed, 0 failed, 85 ignored**. Ignored GPU/font/external tests are not passes.
- Arc-enabled engine resource/image/file suites plus API docs: **19 passed**;
  final realtime-source/gate/API rerun: **13 passed**. These overlapping runs
  are not added together as distinct tests. No VM, GPU or audio device required.
- Final i-family review: **8 native unit tests passed** and **4 archive tests
  passed with the external S25 corpus explicitly enabled**.
- Final external pinned corpus: **15 test groups passed**. The separate FC01
  external-key test was excluded and is not claimed as a pass. Fixture media
  remains outside Git. Existing lossy-image tolerances and alpha0 hidden-RGB
  comparison rules remain explicit in the ledgers.
- Original pinned WARC C++ table-cipher functions compiled and reproduced
  **3 portable vectors**; Rust matches them. This excludes title-key branches
  and upstream's undefined negative floating-to-unsigned conversion behavior.
- Python 3.11 tooling: **187 tests passed**, including inventory/source-linkage
  checks. No GitHub CI/workflow or remote build automation was added.

No commercial-game run, console validation or hardware-render result is claimed
from these CPU codec/API checks. Per-format limits remain in RESOURCES.md,
arc-resource-support.json and arc-image-support.json.


## 2026-10-27 — Awaitable services, shared menus and native plugin facades (ADR 0066)

Implementation:
- Added engine-owned bounded OperationTable tickets with client/restore-generation
  checks, retained completion/failure, cooperative cancellation, dependent-phase
  backpressure and owner publication. Native resource jobs share its ID namespace.
- Resources.beginProbe/beginImage open scoped inputs and prepare results on
  I/O/CPU workers. Ready image pixels publish by shared immutable Arc ownership;
  both client-profile and host-wide 256 MiB staging reservations retain their
  lifetime. Existing dynamic surfaces retain their independent 256 MiB budget.
- Text parsing, shaping, icon loading and rasterization now have worker APIs.
  Reveal events publish with their corresponding pixels, and failed replacement
  keeps the previous block. Restored generations discard stale work.
- Save captures native checkpoints/logical state at an ordered safepoint, then
  workers encode, perform I/O and validate. Ready loads remain staged until
  atomic commitLoad. Native capture/reconstruction and unsupported-save behavior
  remain explicit; the existing snapshot wire format is preserved.
- JS/TS has generated CancellablePromise helpers for Assets/Resources/Text/Save,
  client-scoped continuations, bounded Boa microtask/future turns, async init/start
  fences, pending-hook reentry prevention and bounded shutdown draining.
  Blocking legacy resource/text/save operations reject live-host use.
- Engine MenuSpec/MenuItemSpec owns trees, radio groups, popup/bar layout,
  mouse/keyboard input, paint and saved logical state. Ui.drainEvents consumes
  client-scoped events once; ordinary reads retain them. Menus share the existing
  last-256 UI event policy. Snapshot validation includes menu/focus/event owners.
- Native MenuItem translates handles/properties/callbacks onto these APIs.
  Plugins distinguishes engine_facade, explicit registration_only shells and
  unsupported modules. menu.dll is a real facade; shells export no fake APIs.
  No native DLL, extra engine implementation, compositor or device is hosted.
- Added a standalone TS menu/async-text/save example, ADR 0066 and migration
  documentation. Regenerated docs/api: d4f4c6edc250ee72 -> 5394949c5999cde6.
  Edited sources use full license notices while retaining third-party licenses
  and authors; pinned upstream files/hash manifests remain unchanged.
- Local validation now defaults to release. Dev/test disable incremental output
  and debug information. tools/verify.py no longer rewrites the lockfile or
  formats pinned sources; --fmt opts into a readonly format check. No CI added.
- Cleaned rebuildable target/debug after it reached about 313 GB (165 GB
  incremental and 145 GB dependencies). Available disk increased from about
  24 GiB to 336 GiB immediately after cleanup; after release verification about
  329 GiB remains. Release output (~11 GB), local data (~44 GB) and verification
  evidence were preserved; debug output is now about 621 MB.

Compilation:
- cargo check --release --locked --workspace --all-targets: passed on macOS.
- Desktop arc-formats + krkr all-target release check: passed.
- Formatting of the 38 edited Rust files and git diff whitespace checks: passed.
  No claim that unrelated existing whole-workspace formatting is clean.

Behavior validation:
- Targeted release suites: **100 passed, 0 failed, 1 ignored**: operation table 3,
  resource contracts 9, resource jobs 7, arc-enabled resource APIs 10, shared menus
  4, runtime state 44, engine async services 6, JS Promises/example 4, original
  TJS native bridge 7 and API docs 6. Repeated gate/font reruns are not added.
- Explicitly executed all three system-font tests using a runtime system font;
  these cover offline/worker pixel equality, reveal publication, native font
  callbacks and the standalone TS example. No font is committed. The example
  verifies a real menu click saves once and stays single after 30 more frames.
- Gates verify live frames progress while CPU/I/O workers are blocked, failed or
  cancelled work does not mutate live state, prepared loads require commit,
  mounted sources open on workers and foreign/stale tickets cannot publish.
  Tests also cover foreign menu/event owners, event consumption, held-button
  dismissal and malformed saved menu paths.
- Python 3.11 tooling: **189 passed**, including pinned-source policy and menu
  authoring validation. The external complete-movie test remains ignored; an
  accidental include-ignored attempt lacked its explicit MPEG path and is not
  counted as a pass.
- Original pinned KAG3 release diagnostic **failed**, visibly, at startup.tjs:11:
  menu.dll linked; KAGParser.dll has no implemented facade. This is recorded as
  an integration gap, outside the successful targeted suites, never as a pass.
- Evidence: target/verification/async-release-results.json, *-release-final.log
  for the named suites; async-api-release-check.log, async-workspace-release-check.log,
  async-desktop-features-release-check.log, async-python-final.log and
  kag3-release-probe.log.

Limits:
- Full KAG3/game boot, KAGParser natives, native synchronous popup continuations
  and menu keyboard accelerators remain unsupported. Engine menus are logical
  canvas overlays. Original TJS libraries are unchanged, not replaced by data VM.
- ADR 0050 remains partial: native interpretation/storage callbacks and startup
  compilation still have synchronous boundaries; native checkpoint capture and
  reconstruction remain owner operations. Remaining renderer/audio/media/editor
  scheduling gaps keep their prior status. This is not all-engine async completion.
- Text blocks require explicit Text.state/restoreAsync reconstruction; media
  snapshots and unsupported original-VM checkpoints still fail visibly. No new
  commercial-game, GPU, hardware-RT or cross-platform result is claimed here.

## Shared acoustic scenes and worker direct sound (ADR 0067, 2026-10-27)

Implementation:
- Added device-free `crysirror-acoustics`: engine-owned acoustic mesh/material
  resources, per-triangle materials, affine transform baking, default Parry
  TriMesh/QBVH and optional obvhs 0.4.0 BVH2. The old physics occlusion path
  already used Rapier/Parry QBVH; this adds distinct acoustic geometry and
  propagation, not the engine's first BVH.
- Reviewed pinned phonon_rs direct/scene/model/DSP sources; adapted volumetric
  visibility, exponential air absorption and weighted-dipole directivity with
  original Apache/Valve/phonon attribution. Source inventory and semantic
  differences are in crysirror-acoustics/UPSTREAM.md. No native SDK or HRTF data
  is bundled. BVH build/traversal use existing libraries.
- Added multi-ray source-volume visibility, three-band per-surface amplitude
  transmission, air loss, directivity and reported travel time. Existing mixer
  consumes smoothed three-band coefficients without querying geometry, retaining
  its voice pool, distance, pan, buses, ducking and envelopes. New acoustic
  voices are muted until their first result rather than briefly leaking sound.
- Audio API prepares scene assets and probes on bounded workers, publishes
  immutable client/revision resources on the owner, and exposes status/cancel/
  clear/stats/per-voice results plus JS/TS cancellable async helpers. Automatic
  jobs coalesce poses and use bounded round-robin dispatch. Reservations follow
  pending/captured scenes through cancellation/clear; restore invalidates them.
- Corrected original-origin ray intervals and an obvhs shared-edge signed-zero
  miss (use Parry triangle intersection inside obvhs traversal). Legacy physics
  occlusion now explicitly queries the voice's client. One existing archive
  test buffer needed an explicit Vec<u8> annotation with the additional glam
  dependency; no archive behavior was changed.
- Regenerated docs/api and logged fingerprint `5394949c5999cde6` ->
  `0d7b882595278ffb`. Added ADR 0067 and a synthesized offline acoustic-room demo.

Compilation:
- Local `cargo check --release --workspace --all-targets --features
  crysirror-desktop/krkr,crysirror-desktop/arc-formats,
  crysirror-desktop/acoustics-obvhs` passed on macOS aarch64. Default engine/JS
  all-target checks and the optional acoustic backend check also passed.
  Existing unused-code/variable warnings remain. No CI was added or run.

Behavior validation:
- Release targeted suites: **56 passed, zero failed, three ignored** across
  default acoustic propagation (6), audio library/channels/offline PCM (25),
  engine acoustic lifecycle (6), engine async services (5), engine mixer (4),
  JS async services (4) and generated API checks (6). The ignored cases need
  explicit external NWA/system-font input and are not counted as passes.
  The obvhs-enabled propagation suite separately passed its five tests.
- Worker gates prove frame progress while builds/probes are pending; tests
  cover foreign owners, cancellation, failed replacement, retained-result
  capacity, restore invalidation and immutable probes surviving scene clear.
  Native physics occlusion for client 2 uses its own wall. A blocked acoustic
  voice produces zero actual PCM, including before initial query completion;
  clearing the scene restores audible direct samples through the same mixer.
- JS awaits prepare/probe, checks scene revision and attenuation, and catches
  missing-asset rejection. Successful helper completion releases tickets while
  leaving scene resources installed. No native VM is involved in these tests.
- Reproducible BVH2 microbenchmark: 32768 terraced triangles, 100000 seeded
  downward rays with misses and interval clipping; all hits checked against an
  analytic oracle. This host measured Parry build/query medians 4.891/13.605 ms
  and obvhs 3.885/21.748 ms. Single-thread workload only; no general speed or
  CWBVH/GPU claim. Parry remains default.
- The synthesized nine-second 48 kHz stereo WAV has clear/wall/clear sections.
  Measured wall/clear amplitudes at 200/3000/10000 Hz were 0.683/0.319/0.212;
  the final clear section matches the initial section. Crossover responses
  overlap; these are sample measurements, not exact material-band gains.
- Python 3.11 tooling: **189 passed**. Evidence: target/verification/
  acoustics-{library-tests,regression-tests,api-verify,core-tests,python-tests}.log,
  acoustics-workspace-check.log, acoustics-bvh-compare.json,
  acoustics-audio-demo-analysis.json and acoustics-room.wav. Output/media stays
  ignored under target; all demo audio is synthesized from authored tones.

Remaining boundaries:
- HRTF, diffraction, early reflections, late reverb and dynamic mesh refit are
  not implemented by this profile. Absorption/scattering remain validated
  metadata reserved for reflections; travel time is not applied to playback.
  Source forward is a world direction, without automatic entity rotation.
- Audio source opening/decoding, streaming decoder reads and mix/output still
  have synchronous boundaries; ADR 0050 is not complete. BVH library builds
  are bounded non-preemptible phases with cancellation between phases. Ready
  coefficients can be stale while workers are busy; there is no latency SLA.
- These are independent engine capabilities, not a new completed VN adapter,
  game boot, GPU/hardware-RT result or Windows/Linux behavior validation. KAG3's
  previously recorded missing KAGParser facade remains an integration gap.

## HRTF resources and engine binaural voices (ADR 0068, 2026-10-27)

Implementation:
- Added immutable engine HrtfDataset/HrtfOptions/HrtfEarOrder resources in
  crysirror-acoustics. Inputs are SOFA SimpleFreeFieldHRIR/FIR or prepared
  HrirAsset JSON. SOFA resamples through sofar; JSON requires the mixer rate.
  Native SOFA uses library neighborhood interpolation; prepared JSON weights
  the nearest three directions. No measured human HRIR is bundled.
- Integrated the standalone sofar 0.4.0 DSP library, pinned at
  303532bd609db2d74c1b9e7f7483666dad1210fa. Its SOFA reader, FFT plans,
  partitioned convolution, per-ear delays, crossfades and filter mailbox are
  library components, not another game engine or mixer. vendor/sofar retains
  original licenses/source hashes and one audited HDF5 v3 range correction.
  No native libmysofa SDK, audio-device demo or workflow is imported.
- Added preflight checks for original/resampled dimensions, sample rates,
  required coordinate Type/Units, little-endian numeric encodings, finite FIRs,
  delay axes and supported listener frames. Fixed the M=R=2 global-ear-delay
  ambiguity using Data.Delay dimensions. Ear order is explicit: authored
  left_right by default, right_left or receiver_positions when requested.
  The local CIPIC profile demonstrates why metadata must not silently flip
  conventional response-channel order.
- Audio beginHrtf/status/cancel/clear/stats/voice controls and prepareHrtfAsync
  prepare datasets, voice buffers and filters on bounded workers. Ready
  resources publish on the owner; failed replacements preserve the prior
  profile. Latest direction updates coalesce. Limits are 32 voices/client,
  64/host and 16 in-flight voice/filter jobs. Epoch checks and storage roots
  remain client-scoped. Reservations follow actual retained stream buffers.
- playSfx accepts binaural:true with spatial/follow. The existing mixer replaces
  spatial panning with convolution while retaining acoustic bands, distance,
  gain/envelopes, buses and ducking. Pending initial preparation freezes short
  sources; arbitrary block sizes retain history; natural EOF drains FIR/ear
  delay tails. Pause/seek and disabling have explicit behavior. The default
  bridge adds 64 frames (1.33 ms at 48 kHz), separate from device/queue latency.
- hrtfVoice reports published_direction and pending preparation; it does not
  pretend the audible crossfade has already completed. Resource replacement
  re-prepares enabled voices; seamless profile replacement is not claimed.
- Added authored netCDF4 SOFA impulses, including HDF5 v3, reversed receiver
  records, bad units and a missing coordinate type. Generator dependencies
  are development-only. Added an offline synthesized-source orbit example.
  Regenerated docs/api; fingerprint 0d7b882595278ffb -> c619eaa9bacd0fc3.

Compilation:
- Local release workspace/all-target check passed with desktop krkr,
  arc-formats and acoustics-obvhs features. Engine/JS and library release
  builds passed. Original unused-code/variable warnings remain. No CI ran.
- The vendored source audit verified all 33 original Rust-source hashes after
  reversing the single declared parser fix; zero unexpected source changes.
  Manifest changes exclude upstream native/device/data test targets. This
  does not count the dependency's omitted external-data suites as passes.

Behavior validation:
- Release library suites: **41 passed, zero failed, two ignored** (external
  SOFA and owner NWA fixtures). Engine/JS/API suites: **31 passed, zero failed,
  two system-font tests ignored**. Thus default targeted suites passed 72;
  ignored tests are not counted as default passes.
- The external SOFA case was separately executed successfully with the explicit
  local CIPIC path. The complete HRTF kernel suite, including that normally
  ignored case, passed **8 tests**: 1250 measurements resampled to 48 kHz,
  218 taps, finite nonzero stereo output. This input remains local/ignored.
- Analytic FIR/sample-point oracles verify partition latency, tail preservation,
  ear delays, crossfades, reset and three ear-order policies. The M=R=2 fixtures
  verify constant ear delays independently of measurement index; the HDF5 v3
  case verifies the dependency fix. Bad/missing coordinate metadata fails.
- Engine worker gates prove dataset and initial voice work leave frames free
  to progress. Tests verify cancellation, failed replacement, bounded results/
  voice admission, idempotent enable, listener rotation and real PCM changes,
  restore invalidation, distinct same-named profiles under two client roots,
  foreign handles and clearing one client without changing the other's PCM.
- JS awaits resource preparation, checks metadata, verifies automatic ticket
  release with the installed resource retained, and catches missing assets.
  Native audio predictor/channel/mixer regressions remain in the library suite;
  no new VN opcode or complete game boot is claimed by HRTF.
- Python 3.11 tooling: **189 passed**. Evidence in target/verification:
  hrtf-{library-release-tests,engine-js-release-tests,core-and-sofa-tests}.log,
  hrtf-workspace-release-check.log, hrtf-python-tests.log and
  hrtf-source-audit.json. Offline orbit output and timings are in
  hrtf-orbit.wav / hrtf-orbit.log; all source audio is synthesized. The Apple M5
  single-voice offline run measured dataset read/plan 62.244 ms, eight seconds
  of mixer rendering 16.414 ms, and 80 filter updates 0.151 ms. Source allocation
  and final tail drain are outside the render timing; this is no multi-voice
  deadline claim. hrtf-orbit-analysis.json verifies stereo energy and final zeros.

Remaining boundaries:
- Fixed-reference HRTF is not dynamic near-field propagation, diffraction,
  early reflections or late reverb. Direct travel time remains metadata only.
  Unsupported SOFA conventions, legacy object-header v1 and some generic HDF5
  group layouts still fail visibly. No all-format or subjective localization
  quality claim is made from the impulse tests or generated WAV.
- Full audio async remains open: source opening, decoder reads, mixing/output
  and teardown retain their prior synchronous boundaries. DSP processing
  preallocates its own buffers, but the existing mixer still allocates scratch
  and drops ended voices during render. This is not a hard real-time or
  all-engine async completion claim. Host reservations are estimates, not a
  process-memory sandbox; library phases are not preemptible mid-call.
- No new GPU, device-output or Windows/Linux behavior result is claimed. Native
  VM syscall/wait/save mappings retain their previous integration status.
