# Implementation and validation status

## 2026-10-31 — shared raster jobs, immutable Canvas images and installed NNN admission

Implementation:
- Added client/runtime-scoped persistent pattern resources and nonblocking worker receipts. Create/Advance/Restore preserve immutable generations and return portable RNG candidates; rendering uses the common image pipeline and memory reservations. Native policy commits its RNG only after its own receipt succeeds.
- Added HostServices.begin_canvas_image and explicit Runtime presenter admission. A request freezes engine Canvas nodes, published image leases and font membership. The existing wgpu compositor prepares a retained texture; readback waiting, depadding and immutable ImageResource construction execute on a worker. Desktop window, render and in-place offscreen paths install/poll the same service. Live Puppet and replaceable surface/history dependencies fail explicitly. Explicit readback snapshot cache generations are bounded; automatic PNG compaction remains a separate synchronous path.
- Image release now ends caller ownership while retained Canvas/Layer/filter/transition/mesh and World/VFX references keep the published path alive. Owner collection retires unreferenced paths without serializing mesh/particle payloads; open Layer transactions defer collection. Frozen frame/capture leases remain usable after live lookup ends.
- Added a dormant System-NNN installed factory and detection profile. Original MSI installed directories can be mounted at a selected shared windows/msi root alongside loose game resources, with original record IDs retained. Boot/config/script work uses scoped resource workers and the original reviewed callback module guard. Clock, image, text/UI and audio preparation use public engine services. The native controller remains the default entry; no data VM or original executable is substituted. This is partial integration; registration is not a playable-title or original-save claim.
- Offscreen native play/render now loads host font aliases before stepping a VM that can request metrics. Siglus's existing interval reveal explicitly selects no per-second override; its local workspace source hash was updated while original source provenance was preserved.

Compilation and independent behavior:
- Release Runtime/WGPU check passed (canvas-image-check.log); combined desktop check with rfvp,bgi,nscripter,krkr,arc-formats,circus,catsystem2,majiro,system-nnn passed (desktop-installed-check.log). Later NNN window-resource wiring is also being checked by the combined player build; no newer whole-workspace success is claimed.
- Shared resource release tests passed6/6: Canvas jobs3, pattern resources2, RGB lookup/publication1 (shared-resources-final.log). Coverage includes worker-only short reads, frozen Canvas mutation, retained terminal receipts, client isolation, publication release/retirement, hidden nested snapshots, World material references and frozen render leases.
- Explicit offscreen Metal Canvas capture test passed1/1 on Apple M5 (canvas-image-gpu-final.log). It uses engine-owned nested groups and retained copies, mutates/removes live input afterward, then verifies straight RGBA[255,0,0,128] from the immutable image job. This is an engine-only GPU test, not an original-game title result.
- Shared MSI tests passed3/3 with1 owner test ignored by default (msi-root-tests.log); independent dormant factory test passed1/1 (installed-tests-final.log). Original MUGEN2 detection tests passed2/2 including its explicitly invoked owner-media test: actual catalog/module selects system-nnn and a scoped installer root without extraction or executable execution (detect-installed-tests.log). Original NNN gameplay/title/save compatibility remains unverified by these tests.
- Python tooling passed201/201 after updating the explicit Siglus shared-API local source receipt (python-final2.log). Generated API docs passed8/8, fingerprint a7c6ab240046fa4c logged (api-shared-verified.log). Evidence is under ignored target/verification/systemnnn. No commit, push or CI change.

Follow-up startup validation:
- Actual player17 MUGEN2 controller invocation detected/mounted the installer, but20,000 fast offscreen frames ended with bootstrap_pending=true/phase=preparing. Its error=null and empty capture are **not** a successful native-game run: the simulated clock outran the cold worker before any original instruction started. Evidence: player17-controller-report.json/log.
- Added the opt-in VmAdapter.preparation_pending boundary and Runtime.poll_vm_preparations. Headless startup now polls only dormant preparing VMs before counting game frames, with no clock advance, peer replay or input consumption. Window hosts remain nonblocking. Independent release test passed1/1 and proves first normal input still reaches both peers exactly once (vm-preparation-tests.log). Combined player18 build and the next actual native run remain pending at this checkpoint.

Follow-up source precedence and device-free media preparation:
- Shared fallback mounts now keep existing overlays and actual loose files ahead of the mounted provider. Missing files alone fall back; decoder failure never selects a different source. Per-path origin queries report the bytes actually served. Resources.mountDecode adds an optional fallback flag while preserving the previous two-argument behavior. Independent release fallback test passed1/1 (fallback-mount-tests.log), and combined desktop release check passed (fallback-desktop-check2.log).
- Actual MUGEN2 media contains four-byte MSI GPK/GTB placeholders alongside full loose resources. Correcting source priority gets past those placeholders; weakening the shared BlackCyc parser was unnecessary. Player19 then exposed the original controller's repeated StoryName/SUB1/END sequence, confirming the missing native outer startup/title controller. This is not a successful title or gameplay result; the native adapter work continues separately.
- Added client-scoped HostServices.begin_media_probe plus Media.beginProbe/probeStatus/releaseProbe. Header/decoder preparation runs on bounded shared workers, retains completion and supports cancellation without creating a playback slot, picture, Layer, PCM or clock advance. Unknown audio identity/duration stays unknown; unsupported AlphaMovie header-only preparation fails explicitly. Public tickets become obsolete after Runtime restore.
- Release Engine check passed (probe-routing-engine-check.log). Independent Runtime MediaProbe tests passed2/2 (media-probe-tests3.log), including a sparse100MiB MPEG resource, retained malformed-header failure and subsequent admission. Public Media facade/restore-ticket test passed1/1 (media-probe-api-tests.log). These establish engine capability behavior, not original NNN movie or title compatibility.
- Added reflective InputRoutes methods/global over the canonical shared Scene router, with no private engine-API queue. Independent public receiver test passed1/1 and command API coverage passed1/1 (input-routes-api-tests.log): signed priority, retained leases, unhandled propagation, foreign/stale receipt rejection, registry retention and transient input retirement after restore. Native callback semantics remain the adapter's responsibility; this public API result is not a CatSystem2 gameplay result.
- Regenerated docs/api for MediaProbe and InputRoutes; public reference tests passed8/8 (api-probe-routes-verified.log), fingerprint1dbdf921cc54e571 logged in docs/api/CHANGES.md. New API source and documentation diff checks passed. The next combined player22 build and native-game behavior checks are separate pending validation.

Snapshot: 2026-10-25. Owner priority: the new AAA engine first;
compatibility layers are secondary (ADR 0010).

## KiriKiri CXDEC XP3 archives (ADR 0096, 2026-10-30)

- Implementation: `crysirror-formats-arc` XP3 gains a CXDEC content filter and
  the encrypted filename index (`yuz:`), ported from the owner's MIT xp3-brute
  @ c82cb14. Parameters are recovered at mount time from the user's own
  EXE/TPM and the archive's startup script; no per-game data is in the repo,
  no module code is executed. Emulator init, SteamStub and symbolic TJS
  execution are not ported (fail visibly).
- Tests: `crysirror-formats-arc` release 221 passed, 0 failed, 20 ignored
  (15 new synthetic CXDEC tests). Desktop `--features krkr,arc-formats` check
  passed. `tests.test_krkr_probe` 3/3.
- Behaviour (sampled reads, every 97th entry plus 128 at open, checksums
  verified): CLOVERDAYS (classic, tpm) 4/4 archives, 392 ok; Riddle Joker 8/8,
  476 ok; Cafe Stella 10/10, 490 ok, including prefix words read from the
  module (upstream xp3-brute fails on this title). Logs in
  `target/verification/krkr-cxdec/`.
- Boot: none of the three reaches a title. CLOVERDAYS stops at a TJS parse
  error on the `expr if cond` statement form (`MyYesNoDialog.tjs`); Riddle
  Joker and Cafe Stella stop because `Storages.setupArchiveData` is not
  provided by the adapter.

## In-place launch, native menu, window/fast-forward fixes (2026-10-30)

- `crysirror play [game-folder|file]` (default when no subcommand: the
  binary's folder) detects the engine and runs the game in place:
  KiriKiri (opens `data.xp3` with plugin-TPM CXDEC, HXV4, EXE CXDEC, then
  plain profiles; canvas = largest `scWidth/scHeight` in Config.tjs from any
  archive), AVG32, RealLive (`#SCREENSIZE_MOD`). Siglus/RFVP/NScripter are
  recognized and fail with a TODO. `--detect-only` prints the plan.
  `Runtime::with_asset_root` takes the chosen folder; saves go to the
  per-user state dir keyed by folder. Verified locally: detection matches the
  hand-made projects for game-kr, RiddleJoker, CafeStella (1920x1080),
  DC5t4LM, CLOVERDAYS (1280x720); `play` opened CLOVERDAYS, game-kr,
  DC5t4LM and Kanon windows without errors (25 s each, not played through).
- Desktop menus: OS menu bar by default (macOS via muda); other platforms show
  no bar yet (TODO) instead of drawing it over the game.
  `CRYSIRROR_ENGINE_MENU=1` restores the engine-drawn bar.
- Window: KAGEX forced fullscreen / borderless pseudo-fullscreen because
  `System.screenWidth/Height` reported the canvas. New
  `HostServices::display_info` (desktop: monitor in logical units; headless
  render: 1920x1080 virtual); krkr `System.screen*/desktop*` and windowEx
  `getMonitorInfo/getDisplayMonitors` use it; `Window.min/maxWidth/Height`,
  `setMinSize/setMaxSize` added. CLOVERDAYS and DC5t4LM now start windowed
  and decorated.
- Fast-forward: game-kr with Ctrl held for 6600 frames failed with "motion
  client texture budget exceeded" (budget only grew); motion textures are now
  leased per live model. The same run now skips into the story without error.
- ADR 0097 (proposed): interpreted no-JIT x86 filters, lifted IR, no brute
  force, no bulk extraction.

## KiriKiri: titles reached, cursors, AlphaMovie, media transport (2026-10-30)

- TJS2: a member both `extends` bases declare resolves to the last base
  (`TJS_DO_SUPERCLASS_PROXY` walks `rbegin()`); KAGEX's
  `AnimationLayerTransMove` now reaches `KAGTransMove.beginTransition`, so
  `children=true` transitions move the back page forward.
- Shared capabilities: `crysirror_image::cursor` (.cur/.ico/.ani) + UI
  `DefineCursor`/`SetCursor` mapped to winit custom/standard cursors;
  `ImageEffect::BoxBlur` (krkrz DoBoxBlur/ForAlpha arithmetic); bitmap math
  `MaskHoldAlphaScalar256` and `Ps{blend, hold_alpha}` (tvpps tables);
  `ClientStorage::serving_container`; media `Pause/Resume/Seek` with
  catch-up (no intermediate pictures on seek); AlphaMovie (AJPM) decoding:
  owner's amv_decoder 0.2.0 adopted as `crysirror-codec-amv` (SOURCE.json),
  streaming in `crysirror-video::amv` and the media workers.
- krkr adapter: `Layer.cursor = "file"` (krkrz TVPGetCursor ids from 2);
  `System.createUUID`; `getPlacedPath` in `archive>member` form; executable
  `XOPT_EMBED_AREA_` options seed `System.getArgument` (`-debugwin=no`);
  `operateAffine`, `doBoxBlur`, `setMainPixel`; copy/operate keep a CPU
  composite source (incl. holdAlpha normal, PS modes on CPU);
  VideoOverlay `pause`/`rewind`/`frame`; `AlphaMovie.dll` facade
  (open/play/stop/frame/showNextImage, members inferred from KAGEX
  GFX_AMovie.tjs; plugin source unavailable).
- Behaviour (local renders, owner-authorized games):
  - CLOVERDAYS: title screen renders (logo movie, background, six buttons);
    CONFIG opens; GAME START enters the scene player and draws a blurred
    character composite; stops later at `copyRect` with dfMain+holdAlpha
    (needs a canvas RGB-only copy).
  - DC5t4LM (HXV4): title with sakura particles and SCENARIO select render;
    entering a scenario hits the canvas presentation command limit (2187
    nodes) — canvas compaction is open.
  - RiddleJoker/CafeStella unchanged from the previous entry.
- Tests: new krkr cursor/alpha_movie/video_overlay/tjs semantics tests;
  image cursor/box blur/PS math; media pause/seek; AlphaMovie sample decode
  (149 frames, 30 fps, alpha) is opt-in (`CRYSIRROR_AMV_SAMPLE`).
- Known failing in the working tree: 42 runtime `siglus_audio`/`siglus_pcmch`
  tests fail at Siglus boot ("native entry scene not found": fixture
  Gameexe.ini has no START_SCENE, default `_start`) — from the earlier
  uncommitted Siglus installed-resource change, not this round.

## KiriKiri: CXDEC titles boot, VideoOverlay, TJS2 first-party (2026-10-30)

- TJS2/KAG: Kirakira-derived `krkr-tjs2`/`krkr-kag` are now first-party
  `crysirror-tjs2`/`crysirror-kag` (owner decision); the vendored-hash policy
  test is replaced by a licence check, AGPL boundary tests kept.
- Language/runtime fixes against krkrz fd5c4ba source: postfix `expr if
  cond` with `invalidate this`; `@set` preprocessor values persist per
  engine instance (`tTJS::PPValues`); expression mode is `return <text>;`
  (`tjsLex.cpp`), used by `Scripts.eval/evalStorage`, KAG `exp=` and
  textrender; host `Runtime::construct` (`new` from natives).
- Adapter fixes: `Layer.font` self-bound with native Font properties
  (Yuzusoft objectHookInjection), every Window gets its BasicDrawDevice
  (`recreate`), `System.versionInformation/drawThreadNum`, krkr2
  `Window.innerSunken/showScrollBars`, `beginTransition` `withchildren`
  defaults to true, many-continuation frames yield instead of failing.
- `VideoOverlay` facade over the shared media service: open/prepare/play/
  stop/close with krkrz immediate/async status semantics, layer and overlay
  modes; pause/rewind/seek/segment loops/other audio streams fail visibly.
- Probe tool: `--setup-profile auto|on|off` (classic `.tpm` CXDEC has no
  setupArchiveData profile); CXDEC diagnostic test can dump entries locally.
- Tests: crysirror-krkr/tjs2/kag/formats-arc release 740 passed, 0 failed,
  26 ignored (new: font hook, draw device, preprocessor persistence,
  expression mode); Python 200 passed; workspace all-targets check passed.
- Behaviour (600-frame CPU probes, final binary): RiddleJoker and
  CafeStella finish init (AfterInit, revision log) and wait at KAG
  `syshook`; renders at 1920x1080 show the ATTENTION screen with the
  first-run patch-check dialog. CLOVERDAYS (classic CXDEC) finishes init,
  plays the ALcot logo (MPEG-1) through shared media into a KAG layer and
  reaches the title script, but the title UI stays on the back page (black
  screen; KAGEnvironment transition path not yet traced). DC5t4LM (HXV4)
  boots into its scripts and stops at `Layer.cursor = <cursor file>`
  (needs a shared custom-cursor service). game-kr unchanged (KAG wait).
- Not done: AlphaMovie.dll (port from xp3-brute), custom cursors, CLOVERDAYS
  title transition, VideoOverlay transport.

## AVG32 installed games: Kanon through shared services (ADR 0095, 2026-10-30)

- Implementation: `crysirror-avg32` now vendors the original AVG32
  interpreter from siglus_rs 5f358ef (per-file hashes and seam list in
  `src/vm/SOURCE.json`) instead of the portable-vm Git build. The PDT bank,
  copy/mask/fade/stretch primitives and all transitions moved to
  `crysirror_image::avg32` (plus buffer-0 write damage and a present
  revision). The screen is one engine surface layer; buffer-0 text is a
  retained GlyphRun; menus/backlog/name entry are solid + GlyphRun layers;
  audio is engine channels; files and `SAVE.INI` use client storage and the
  save mount. Config is `{root, font, encoding?}` (breaking, API fingerprint
  `c6204fad405840ca`, logged).
- Install: owner-supplied Kanon CloneCD Disc 1 extracted per its SETUP.INI
  copy list to `~/Downloads/game_kanon`, CD-DA tracks 02-23 ripped to
  `BGM/TRACKnn.WAV`; Disc 2 (12 audio tracks, not referenced by
  GAMEEXE.INI) ripped to `DISC2/`. Unmodified upstream `avg32_probe` on the
  install reached SEEN050 prologue in 3000 frames.
- Compilation: `cargo check --release --workspace --all-targets` passed
  (after giving the vendored `reallive-dll` tests a test-only `game_fs`
  shim). Desktop release build with default features passed.
- Tests: avg32/image/reallive-dll/reallive/desktop/runtime release suites
  330 passed, 0 failed, 5 ignored; 4 new synthetic-install AVG32 tests
  (screen surface, GlyphRun text, `#CDTRACK` BGM, glyph removal on
  overwrite, movie warning, read-only game files/save mount), plus a fifth
  added afterwards for choice-row highlight (glyphs recoloured, not removed);
  `crysirror-avg32` then passed 25 (20 vendored unit + 5 integration). Python 3.11
  tooling 195 passed. API docs 7 passed.
- Behaviour (Kanon, release `render`/`trace`, Apple M5/Metal, scripted
  input): KEY logo and title, the graphical main menu, prologue text in the
  message frame, the right-click system menu, save to slot 5 and load from
  the title (prologue restored, 29 glyphs), and a Ctrl-skip trace into
  SEEN501 "１月８日 金曜日" after 12000 frames with 52 audio requests and
  no warnings. Images in `target/verification/avg32-kanon/`. A 900-frame
  windowed `run` exited cleanly; audible output was not verified. Local
  launcher: `~/Downloads/Run-kanon.command` (system Hiragino font, saves in
  `~/Downloads/CrySirror-kanon-state`).
- Limits: not played to completion; choices were only clicked blindly
  during skip. Text into PDT buffers other than 0, Cinepak movies,
  `FN.DAT`/`＊００` glyphs as engine glyphs, framework snapshots and script
  `Bitmap` exposure of the AVG32 raster primitives remain open. Glyph
  visibility follows buffer-0 writes, not native present timing.
- RealLive DLLs: the owner's EF00/PT00/DT00 emulation (`reallive-dll`) is
  wired to `LoadDLL`/`CallDLL`; its upstream tests now run (4 passed; the two
  game-DLL cases return early without a Little Busters!/Tomoyo After
  install). No DLL-using game has been run.

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

## Worker audio decoding and playback (ADR 0069, 2026-10-27)

Implementation:
- Added reusable TaskPermit admission so a caller reserves bounded queue
  capacity before moving a decoder/resource into a closure. Unused permits
  release their slots. TaskStats exposes reserved admission separately.
- Added device-free BufferedSource and explicit Frames/Pending/End, service,
  poll_seek, read_position and buffering status to shared AudioSource. One
  idle owner or one worker owns the decoder; immutable PCM chunks cross the
  boundary. Short positive reads are not mistaken for EOF. Regular read-ahead
  is bounded; initial/custom loop prefixes remain resident. Existing blocking
  decoder/seek contracts remain explicit and available to offline/native paths.
- Mixer interpolation consumes ready PCM, retains the previous chunk's edge
  sample and stalls only the starved voice's source/envelope/DSP clocks. Other
  voices continue. Stats report per-voice underrun frames. Exact seek sequences
  freeze until ready, discard stale generations and support cancellation back
  to the previous fractional clock. Synchronous seek rejects buffered sources.
- Added engine beginPreloadSfx/beginPlaySfx/beginPlayMusic/beginSeekVoice,
  playbackStatus/cancelPlaybackJob/sourceStatus/clearSfxCache and corresponding
  JS/TS await helpers. Capability reads, source probing, decode and priming run
  on bounded I/O workers. Owner publication revalidates spatial dependencies
  and uses the same mixer/acoustics/binaural attachment APIs. Named music only
  replaces/fades previous playback after successful preparation and admission.
- Retained jobs are limited to 16/client and 64/host. Shared residency receipts
  follow cached/live clips and worker decoders; the 512 MiB estimated audio
  budget survives reset while references remain. The cache has its own 128 MiB
  bound. These estimates do not comprehensively account provider/codec/transient
  allocations and are not a process-memory sandbox (see ADR 0069).
- Realtime cold synchronous SFX and music now reject with the async migration
  path; preloaded SFX/random variants and explicit offline calls remain usable.
  Cache clearing does not stop playing clips. Added generation checks at Audio
  API entry, fixing restore before the first frame leaving a prior voice alive.
- Regenerated docs/api and recorded fingerprint c619eaa9bacd0fc3 ->
  2d945b181a39e802 with realtime behavior migration instructions.

Compilation:
- Local release workspace/all-target check passed with desktop krkr,
  arc-formats and acoustics-obvhs features. Existing CBG dead-code and RealLive
  unused-variable warnings remain. No GitHub CI or remote build ran.

Behavior validation:
- Shared task/audio release validation passed 45 distinct tests (including
  seven buffered-source cases), with one owner-asset NWA test ignored. Engine/
  audio/acoustics/binaural/JS suites passed 31 tests, with one system-font demo
  ignored. API documentation/signature checks passed six tests. Total: 82
  distinct Rust tests passed; the two ignored cases are not passes.
- Analytic PCM and worker gates verify blocked I/O leaves other voices/frames
  progressing, pending PCM is not EOF, pool backpressure retains decoder
  ownership, fractional chunk interpolation matches resident PCM without
  backward decoder seeks, custom loops match, pinned heads obey queue limits,
  exact seek generations supersede stale work, and cancellation restores the
  prior fractional clock. Decoder read/seek probes execute off the owner.
  A worker packet failure retains queued PCM and then reports its error,
  rather than reporting a successful EOF or discarding the valid prefix.
- Engine tests verify cancellation before and after worker completion cannot
  start unpublished playback, per-client same-named assets/foreign tickets,
  bounded result admission, shared cache lifetime, failed music replacement,
  uncached seek readiness with different sample values at its destination,
  stop-during-seek ticket release and restore before the first audio frame.
- An authored NWA stream is opened through capability storage and decoded/
  looped by the shared Audio API without a native VM. JS/TS awaits preload,
  playback and seek, verifies helper ticket release and catches missing files.
  This does not establish a new native game boot or opcode/wait/save profile.
- Python 3.11 tooling passed 189 tests. Evidence is under target/verification:
  audio-worker-final-library-tests.log, audio-worker-final-engine-js-tests.log,
  audio-worker-workspace-check.log, audio-worker-python-tests.log,
  audio-buffered-failure-tests.log and audio-worker-api-tests.log.

Remaining boundaries:
- Native SceneState/channel source opening and AudioRequest decoding still
  retain synchronous boundaries; changing original native waits requires
  explicit pending events and continuations. Final mix/output, scratch/voice
  allocation/disposal and final task-pool joins remain synchronous/owner work.
  This stage is not a hard realtime mixer or all-engine async completion.
- Acoustic reflections, diffraction, near-field propagation and room reverb
  remain open. No GPU/device-output/Windows/Linux behavior or additional
  original VN compatibility is inferred from these offline results.

## Native channel audio readiness (ADR 0070, 2026-10-27)

Implementation:
- Core now declares deferred AudioAsset/PlayAsset and explicit preparation
  phases. HostServices begin_audio/audio_preparation/release_audio distinguish
  request admission, owner publication and actual voice completion. Core and
  compat retain their dependency-free/device-free boundaries.
- Shared ChannelAudio opens and probes capabilities, prepares source/intro,
  declared limits and exact startup/loop prefixes on bounded workers. It uses
  the existing standard/NWA decoder, source wrappers, BufferedSource and mixer.
  A native adapter owns no decoded PCM or private general audio algorithms.
- Mapped RealLive BGM/WAV commands retain the original pending VmEvent/PC until
  publication; AVG32 music/wave/SE/voice events retain the original portable
  event. Both install/continue native sound waits after readiness. Failure
  leaves later native instructions unacknowledged. VM parser/opcode source and
  previous unsupported original-save/profile behavior remain unchanged.
- Added OperationTable::insert_ready for explicit offline products and
  submit_deferred for bounded queued inputs. Full shared pools retain Pending
  requests and retry slot reservation on owner polling; cancellation before
  admission performs no I/O. Retained-result/memory limits still fail visibly.
- Channel preparation retains at most 16 receipts/client and 64/host. A separate
  512 MiB estimated channel budget follows worker/stream storage. Encoded inputs
  are bounded at 64 MiB per supplied memory segment and 512 MiB per streamed
  segment, subject to provider limits. Estimates do not account all provider,
  transient or codec memory and are not a process-memory sandbox.
- Stop/supersession prevent unpublished playback. Failed preparation preserves
  the old live voice. restart:false can reuse an active channel without asset
  I/O. Restore clears channel voices, queued PCM and receipts after successful
  state validation; late completions cannot revive prior runtime playback.
- Fixed stale stopped/ended channel handles blocking later owner publication.
  Added prepare_at_cancellable to prime exact nonzero startup positions, while
  retaining intro and loop caches. Source frame units remain explicit.
- Moved the existing common AssetIo cursor to crysirror-io (core + std, no
  codecs/devices). Resource retains its reexport; modern and native audio use
  decoder::open_asset over the same cursor, removing the engine-private wrapper.
- Regenerated docs/api and logged fingerprint 2d945b181a39e802 ->
  d36d62998490cf17, including the exhaustive AudioRequest enum migration.

Compilation:
- Local release workspace/all-target check passed with desktop krkr,
  arc-formats and acoustics-obvhs features. Existing CBG/RealLive warnings
  remain. Exact-file rustfmt and git diff checks passed. No CI ran.

Behavior validation:
- Shared I/O/task/audio/resource suites passed **79 tests**, with one owner NWA
  fixture ignored. Native/engine/JS bridge suites passed **38 tests**, with
  three font-dependent cases ignored. API documentation/signature checks
  passed **six tests**. Total: **123 distinct Rust tests passed**, four ignored
  cases excluded. The separately rerun six-case channel suite is not counted
  twice. Python 3.11 tooling passed **189 tests**.
- Synthetic capability readers record opening on workers. Gates verify pending
  preparation leaves owner frames free, including when every pool slot is
  reserved, without losing inputs. Tests verify cancellation before publication,
  foreign receipts, result bounds, no reopen with restart:false, newer request
  precedence, failed replacement, Stop invalidation and Ready receipts surviving
  actual voice completion. Runtime restore invalidates pending channel work.
- Authored PCM oracles verify intro/tail declared limits, decode/playback gain,
  nonzero exact startup and native loop offsets at sample points. The new
  cursor tests cover partial reads, end-relative seeks, underflow/overflow and
  invalid provider read sizes. Existing resource transform/reader tests pass
  through the preserved AssetIo reexport.
- Original RealLive bytecode and AVG32 TPC32 fixtures verify unchanged instruction
  counters while all I/O workers are blocked; following title/arithmetic state
  remains untouched. Once ready, playback and native sound waits complete.
  Invalid decoder input fails before following native instructions execute.
  These are original instruction/event paths, not VM aliases or hosted engines.
- Revalidated all 90 complete-reference RealLive Rust hashes at
  5f358ef1360b0439e92aed2b749e8f25d8721954 and inventoried 31 AVG32 Rust files at
  256c398e06fbcbae8e956eb046c831b3d6c8a593. Inspected native sound/event/wait
  branches at those pins. This source review is not blanket opcode coverage.
- Evidence in target/verification: channel-final-library-tests.log,
  channel-final-bridges-tests.log, channel-final-coordinate-tests.log,
  channel-api-tests.log, channel-final-workspace-check.log,
  channel-python-tests.log and channel-native-source-review.json.

Remaining boundaries:
- Native resource-name directory discovery, boot/scenario and other unmigrated
  image/video reads remain synchronous. SceneState/data-VM voice mirroring and
  encoded media AudioRequest submission retain synchronous opening. The legacy
  encoded submit API remains explicit; PlayAsset requires preparation.
- Final mix/output, allocation/disposal and final pool destruction still have
  owner/synchronous boundaries. No hard realtime, complete audio/engine async,
  additional native opcode/save/game profile or commercial game boot is claimed.
  No new device, GPU, hardware-RT or Windows/Linux behavior was measured.
- Near-field propagation, diffraction, acoustic reflections and room reverb
  remain open. These results advance the shared foundation without completing
  the broader modern 3D engine/VN integration objective.

## Canonical SceneState audio worker bridge (ADR 0071, 2026-10-27)

Implementation:
- Added device-free SceneAudio over a caller-owned Mixer. Engine App and plain
  desktop playback now use the same declaration/epoch reconciliation and worker
  preparation. Removed the engine-private canonical voice bridge and the plain
  desktop's live AudioSystem::sync/read_for path. The library's explicit
  synchronous compatibility API remains available and is documented as such.
- Reused native channel capability opening and source preparation, existing
  BufferedSource and deferred task admission. Canonical cold assets are opened,
  probed and primed off the realtime owner. No codec, mixer or general-purpose
  algorithm was added to a VM. Canonical and pooled App voices retain their
  existing shared Mixer, limits, buses and final PCM composition.
- Stops/replacements/restore reconcile before result publication. Published
  epochs remain distinct from desired epochs, so natural EOF/stolen handles do
  not automatically restart. Gain/routing preserve the timeline. Pause includes
  voices that finish preparation later. Failed preparation retains the prior
  voice, records the error and produces the existing visible frame failure.
- Added Audio.channelStatus, SceneVoicePhase and SceneVoiceStatus for scoped
  queued/loading/playing/paused/ended/failed state, position and failures.
  Streaming faults are visible while a valid predecoded prefix can still drain.
  Logical time waits remain separate from audio readiness; save/trace voice data
  contains no decoder, task or device handle.
- Preparation is limited to 16 jobs/client and 64/bridge. Additional declarations
  stay Queued and retry. A separate 512 MiB estimated bridge budget reserves
  4 MiB per realtime source and follows actual worker/source ownership through
  cancellation. These estimates are not comprehensive process-memory accounting.
  Offline playback retains same-frame preparation at the canonical 128-voice
  limit instead of inheriting realtime admission delays.
- Added Runtime audio execution override and effective mode inspection. Desktop
  Run enables audio workers before initialization, independently of native
  image/scenario Assets mode. Setter also configures direct channel calls before
  the first runtime tick. Explicit offline/test hosts inherit the Assets policy
  unless overridden. Cold modern SFX/music guards now use the audio policy.
- Regenerated docs/api and logged d36d62998490cf17 -> 102009774599dc55.
  ADR 0071 records initialization migration and remaining subsystem boundaries.

Compilation:
- Release workspace/all-target check passed with desktop krkr, arc-formats and
  acoustics-obvhs features. The desktop no-default-features reallive+avg32 check
  also passed, without scripting or audio-device output enabled. Existing CBG,
  RealLive and minimal-CLI warnings remain. No CI or remote build ran.

Behavior validation:
- Audio library suites passed **47 tests**, one owner-asset NWA case ignored.
  Engine/native/JS bridge suites passed **41 tests**, three font-dependent cases
  ignored. API documentation/signature checks passed **six tests**. Total:
  **94 distinct Rust tests passed**, four ignored cases excluded. Separate
  focused reruns are not counted again. Python 3.11 tooling passed **189 tests**.
- Six standalone SceneAudio tests verify full-pool pressure, cancellation before
  admission/publication, progress of unrelated voices, same-asset replay and
  superseded results, gain/bus changes without epoch reset, restore at frame
  zero, EOF without implicit replay, failure with old playback retained, pause
  and all 128 offline declarations in one sync. Excess realtime declarations
  eventually publish in bounded batches. Reader probes remain off the owner.
- Authored NWA data with an injected read fault proves streaming status records
  the error while already-valid PCM remains playable. No owner game/font asset
  or audio device was used for this case.
- Engine tests execute data VM Play/Wait/Stop while workers are blocked, check
  Stop-before-ready cannot leak playback, verify two client roots/readiness,
  inspect visible failed channel state, and reconstruct logical voices after
  restore without saved handles. Existing pooled/canonical output and replay
  regressions pass through the new bridge.
- Original RealLive/AVG32 bytecode fixtures now explicitly keep native Assets
  mode blocking while overriding audio async. Their original pending events,
  sound waits and failure checks pass with that independent mode. Direct native
  channel preparation before the first tick is also covered by restore tests.
- Corrected two test timing assumptions: canceled worker inputs retain storage
  until actually dropped, and an asynchronous onInit rejection may be observed
  at init, start or frame. The tests retain their reservation/fault assertions;
  production memory limits and error contracts were not weakened.
- Evidence in target/verification: scene-audio-final-library-tests.log,
  scene-audio-final-bridge-tests.log, scene-audio-final-api-tests.log,
  scene-audio-core-tests.log, scene-audio-workspace-check.log,
  scene-audio-minimal-check.log and scene-audio-python-tests.log.

Remaining boundaries:
- Final mixing, scratch/allocation/disposal, device output/queuing and final pool
  destruction remain owner/synchronous work. Media's encoded AudioRequest path
  still opens/decodes on the owner. No all-audio/all-engine async completion or
  hard realtime guarantee is claimed by this stage.
- Native directory discovery, boot/scenario and unmigrated image/video work
  retain their existing boundaries. No new original opcode/save/game profile,
  commercial-game run, device/GPU/hardware-RT or Windows/Linux result is claimed.
- Reflection/reverb, diffraction and near-field propagation remain acoustic
  work; the larger modern engine/VN capability objective remains active.

## Worker-prepared media PCM (ADR 0072, 2026-10-27)

Implementation:
- Added dependency-free immutable PcmAudio/PcmAudioInfo and AudioRequest::PlayPcm
  in core, plus shared audio PcmBuilder and resident-source playback. These work
  without any native VM, movie service or output device. Prepared publication
  performs no codec opening or sample validation scan. Core retains no device
  dependency; resources/receipts are derived and excluded from logical saves.
- Media workers now prepare PCM directly instead of encoding WAVE for the owner
  to reopen/decode. Conversion preserves i16 /32768, mono duplication and the
  existing first-two-channel projection. This is not surround downmix support.
  Start offsets, sample-rate conversion, loop points, gain, pan and client scope
  use the existing shared mixer. Movie PTS lead/delay and late visits retain
  their clock/skipping policy.
- Moved media capability opening and length queries off the owner. Audio/video
  share one lazy scoped capability with independent AssetIo cursors. Missing
  assets now produce asynchronous Loading -> Failed errors; invalid authority,
  options and worker admission still reject immediately.
- Added 16 live/starting decoder-worker reservations. Blocked canceled workers
  retain admission until they actually exit; replacement rejects before changing
  the current slot if no capacity remains. Loop-restart admission failures mark
  Failed and retain emitted Stop controls for the next owner audio drain.
- PCM sample-budget receipts follow actual queued/mixer references past resource
  Stop. Staging and final samples are reserved together during Arc conversion.
  The 128 MiB budget excludes allocator spare capacity, codec/provider buffers
  and other process overhead. Inspection adds decoder_worker_reservations.
- Generated API metadata now includes PcmAudioInfo; fingerprint changes from
  102009774599dc55 to f55ff4c1819aaa78. The change log records the Rust enum
  extension and asynchronous media opening errors. No JS sample-upload API was
  added and no native opcode or renderer implementation moved into an adapter.

Compilation:
- Release audio/runtime checks and the final workspace/all-target check passed
  with desktop krkr, arc-formats and acoustics-obvhs enabled. Existing CBG and
  RealLive warnings remain. No CI, remote build or new device/GPU validation ran.

Behavior validation:
- Audio suites passed **51 tests**, one unrelated owner-NWA test ignored. Media
  regression tests passed **11 tests**; its local-file case was explicitly run
  separately and passed. API signature/reference checks passed **six tests**.
  Total: **69 distinct Rust tests passed**. The explicitly executed movie is not
  counted again from the earlier full-media run. Python 3.11: **189 passed**.
- Four independent PCM tests verify invalid resources, signed extrema, mono and
  multichannel projection, partial/oversized append atomicity, shared storage
  lifetime, exact seek/EOF beyond the SFX Clip ceiling, sample-level start/rate/
  loop/gain/pan controls, foreign-client Stop isolation and final voice release.
- Media probes block opening while the owner continues ticking, assert open/len/
  read occur off-owner and both decoders share one capability, retain admission
  across canceled blocked providers, preserve the current slot on failed
  replacement and release leases after actual worker exit. Queued/live PCM keeps
  its sample reservation after resource Stop. Missing-open faults are visible;
  synthetic PTS cases verify early audio, delayed audio and late-visit sample
  skipping. Loop admission failure retains the completed voice's Stop.
- Explicit owner-supplied RUSUR.MPG run reached Ended without error: **2,995
  decoded frames / 2,987 published updates**, one PCM resource with **4,402,944
  stereo frames at 44,100 Hz**, **35,223,552 sample bytes**, nonzero shared mixer
  output (peak 0.99996948), and unchanged source size/mtime. This is CPU media/
  offline-mixer evidence without any native VM, GPU or audio output device;
  published updates are not a guarantee every source picture was presented.
- Evidence: target/verification/pcm-audio-tests.log, pcm-media-final-tests.log,
  pcm-media-regression-tests.log, pcm-api-final-tests.log,
  pcm-workspace-final-check.log and pcm-python-tests.log.

Remaining boundaries:
- Movie audio still decodes a bounded complete track. Long-track streaming,
  codec/provider/transient memory accounting and live device behavior remain
  open. Final mix/output, control/scratch allocation, last-reference disposal
  and final worker-pool destruction retain synchronous boundaries.
- Native boot/discovery/images, original save/opcode coverage, other-platform
  behavior, acoustic reflections/reverb and complete engine-wide async execution
  are not completed by this phase. No additional whole-game compatibility or
  hardware RT result is claimed; the broader engine objective remains active.

## Shared parametric late-reverb buses (ADR 0073, 2026-10-27)

Implementation:
- Integrated standalone FunDSP 0.23.0's 32-channel FDN behind engine-owned
  LateReverb/PreparedReverb. Original source revision is
  8479a70d813d539c67a908f88e65f6b0b344122e. Audited the complete reference checkout
  inventory (70 Rust files); all 52 published registry sources match after CRLF
  normalization, with four raw newline differences and both hash sets recorded.
  Reverb/feedback/delay/FIR/allocation hooks were inspected. Cargo pins the crate
  and checksum; upstream MIT/Apache terms remain intact. Optional file loading
  and FFT convolution are disabled. No other engine or mixer was imported.
- Shared ReverbSpec/ReverbSettings/ReverbSend provide authored room scale,
  approximate decay, damping, source-bus sends and wet-return routing. Sends are
  post source fader/duck/spatial/direct/HRTF and pre-Master. Wet uses its return
  bus and Master once, without feeding itself again. Dry output remains present.
  Tails survive source EOF/Stop/stealing; clear/replacement cuts old history.
- Added Audio beginReverb/reverbStatus/cancelReverbJob/clearReverb/reverbSettings
  and prepareReverbAsync. Workers construct/preallocate matched DSP objects for
  ordinary/canonical and native/media mixers; owner publication rolls back the
  first install if the second fails. Both call the same engine DSP/bus APIs.
  Native adapter implementations/opcodes are unchanged; only test dependencies
  and fixtures add shared effect configuration and output assertions.
- Deferred worker admission, cancellation/supersession, scoped tickets and
  restore invalidation reuse OperationTable. A 64 MiB estimated pair reservation
  follows actual worker/DSP owners against 512 MiB/host; canceled work does not
  release its receipt early. Tables retain at most 8 jobs/client and 32/host.
  Published receipt release keeps installed DSP. Effects/history are derived,
  never serialized; Mixer::clear removes them after restore/reset.
- Added source audit/attribution, ADR 0073, async/API usage notes and a standalone
  room_reverb example. Generated docs/api fingerprint changed from
  f55ff4c1819aaa78 to 98b40f5f4c7c26d0 and is logged in CHANGES.md.

Compilation:
- Final local release workspace/all-target check passed with desktop krkr,
  arc-formats and acoustics-obvhs. Desktop no-default-features reallive+avg32
  check also passed without scripting or device output. Existing CBG/RealLive
  and minimal-CLI warnings remain. No CI or remote build ran.

Behavior validation:
- Acoustics/audio suites with obvhs passed **68 tests**, two owner-resource cases
  ignored. Engine reverb passed **4**, JS async services **7** (one font case
  ignored), original RealLive/AVG32 bridge suites **12** (one font case ignored),
  and API reference/signature checks **6**. Total: **97 distinct Rust tests
  passed**, four ignored excluded. Focused reruns are not counted twice.
  Python 3.11 tooling: **189 passed**.
- Independent impulse tests verify source-EOF tails and decay, sample-identical
  arbitrary block boundaries, Master linear scaling without squaring, input
  mute, wet-return mute with dry preserved, client isolation, native PCM channel
  tails, invalid formats/routes and actual storage release after Mixer::clear.
- Engine tests verify both mixer publications, native PCM tail in App output,
  rollback preserving the old first-mixer room when the second cannot publish,
  foreign ticket access rejection, full-pool owner progress, cancel/supersede
  before publication, invalid replacement preserving old settings, ticket release
  preserving installed DSP and restore removing pending/installed histories.
  JS/TS executes prepareReverbAsync, checks publication and receipt release,
  catches invalid settings and clears the client room.
- Original RealLive/AVG32 sound fixtures retain their original source-completion
  waits and instruction/arithmetic boundaries with reverb enabled. Subsequent
  native instructions execute while a nonzero shared bus tail remains. Worker
  failure tests also pass. This is original-bytecode/service-boundary evidence,
  not additional opcode, native-save or commercial-game compatibility.
- Authored six-second dry/room/hall demo rendered stereo 48 kHz WAVs without
  assets/devices. Mix wall times: dry 0.006363 s, room 0.031424 s, hall 0.032521 s;
  post-source tail energy: 0, 0.01223493, 0.51438753; all peaks < 0.578. This is a
  single-effect offline CPU sample on this host, not full-engine performance or
  audio-device latency. Output stays ignored under target/verification/reverb-demo.
- Evidence: reverb-library-tests.log, reverb-engine-final-tests.log,
  reverb-js-tests.log, reverb-native-tests.log, reverb-api-final-tests.log,
  reverb-workspace-final-check.log, reverb-minimal-check.log,
  reverb-python-tests.log and reverb-demo.log in target/verification.

Remaining boundaries:
- Geometric early/specular/diffuse reflections, material-dependent IRs, physical
  room/RT60 inference, diffraction/pathing and Ambisonic return rendering remain
  open. The late FDN consumes authored parameters, not BVH materials. Its decay
  is approximate; sends follow existing direct processing rather than a separate
  physically simulated unoccluded indirect source path.
- Idle installed effects continue processing; replacement/clear does not crossfade
  or preserve old tails. Final DSP/mixing, control/scratch allocation, old-effect
  disposal, device queues and pool teardown remain synchronous boundaries.
  No complete engine/audio async migration, new GPU/hardware RT or Windows/Linux
  validation is claimed. The larger modern engine/VN objective remains active.

## Captured geometric early reflections (ADR 0074, 2026-10-27)

Implementation:
- Added engine-owned finite-triangle, first-order image-source queries over the
  existing pinned Parry/optional obvhs scene. Mirror candidates use Parry's
  primitive intersection; both path legs use the shared BVH and direct-path
  per-crossing amplitude transmission kernel. No legacy engine/renderer or new
  handwritten BVH is imported. Scientific/source boundaries are in ADR 0074;
  the pinned phonon_rs reverb parameter placeholder is not presented as a solver.
- Absorption, transmission and scattering now determine specular pressure via
  sqrt(max(0,1-absorption-transmission²)*(1-scattering)), with air/range/source
  directivity and both legs' transmission. Shared-edge duplicates are removed;
  finite surfaces, backfaces, units and ignored meshes retain explicit policies.
  Whole-scene triangle, audible-path and crossing overflow fail visibly.
- Added worker-prepared EarlyReflections with sparse linear fractional delays,
  equal-power stereo arrivals and shared complementary-band history. No geometry
  or ring allocation runs in sample processing. Seek uses a constant-time filled-
  history reset. Natural EOF drains echoes; explicit stop/steal/clear cuts history.
  Excess delay aligns to the existing dry clock and prepared direct HRTF bridge.
  Echoes receive raw pre-direct-filter input, so direct occlusion alone does not
  erase a separately visible indirect path.
- Added scoped engine reflection probe/voice/status/cancel/clear/inspection APIs,
  traceReflectionsAsync and prepareVoiceReflectionsAsync. Work uses immutable
  scene/pose snapshots and bounded deferred CPU jobs. Initial preparation holds
  only that source's clock. Obsolete pose/scene/latency work fails, releases the
  hold and cannot publish; failed replacement keeps old valid paths. These are
  captured-pose resources: automatic tracking is not implemented in this phase.
- Added OperationTable::fail for observable owner/generation-scoped invalidation
  without starting queued work. Worker/DSP leases retain actual ownership. Voice
  staging reserves 8 MiB, then shrinks to ring/tap/path storage plus 8 KiB against
  a 512 MiB estimated host budget. Jobs retain 16/client and 64/host. Restore
  invalidates derived effects/history/tickets without serializing pointers.
- Added Mixer::source_is_playing and Audio.voice.source_playing. Modern playing
  includes processing tails; native channel waits use source completion. Shared
  Rust channel/mixer effect APIs work without App/VM/device. Native adapter
  implementations/opcodes/save formats are unchanged; bridge tests configure the
  same engine resources rather than adding private native algorithms.
- Generated docs/api changed 98b40f5f4c7c26d0 -> 079531ae354ff224; CHANGES records
  APIs, snapshot semantics, source/tail distinction and initial paused-play recipe.

Compilation:
- Local release workspace/all-target check passed with desktop krkr, arc-formats
  and acoustics-obvhs; desktop no-default-features reallive+avg32 also passed
  without scripting or device output. Existing CBG/RealLive/minimal-CLI warnings
  remain. No CI or remote build ran.

Behavior validation:
- Acoustics/audio/tasks suites with obvhs passed **91 tests** (two owner-resource
  cases ignored); engine acoustics/HRTF/reverb/reflections suites **20**; JS async
  services **8** (one font case ignored); original RealLive/AVG32 bridges **12**
  (one font case ignored); API reference checks **6**. Total: **137 distinct Rust
  tests passed**, four ignored excluded. Focused reruns are not counted twice.
  Python 3.11 tooling: **189 passed**.
- Six geometry tests cover finite-wall analytic points, 1/r pressure, absolute/
  excess delays, shared-edge/backface identity on both BVHs, both-leg occlusion,
  air/units/dipole/ignore policies, per-band transmission and crossing limits,
  cancellation/overflow, and an opaque direct-path blocker with a visible echo.
- Five sample-level tests verify fractional-delay/pan oracles, reset without old
  history leakage, actual resource lifetime, pre-direct echo routing, source EOF
  with remaining echo, native source completion, initial hold/cancel, pose/seek
  invalidation and direct HRTF bridge alignment. Task tests prove invalidated
  queued work never starts and failed receipts preserve scopes/errors.
- Engine tests verify real isolated echo samples, shared scene queries, full-pool
  progress of other voices, scoped/superseded/canceled work, pose/scene failure,
  old-path preservation, restore invalidation and **64 installed voice effects
  plus successful replacement** after staging reservations shrink. JS uses both
  Promise helpers, checks actual receipt release, rejects incomplete queries,
  clears/reprepares a paused short source, then starts it.
- Original RealLive/AVG32 audio fixtures now install real geometric echo resources
  through shared channel APIs. At original source EOF their next instructions/
  arithmetic execute while the mixer still retains an active echo voice. Worker
  failure/PC-boundary tests also pass. This is bounded native bytecode/service
  evidence, not additional native opcode/save or commercial-game coverage.
- Apple M5 dense-grid oracle: **262,088 triangles**, ten checked source positions
  per backend. Parry build 0.027020 s / mean query 2.579733 ms; obvhs build
  0.026347 s / mean query 2.530567 ms. Candidate enumeration dominates this flat
  workload; these close timings do not establish a backend advantage or engine
  frame-rate ranking. All ten analytic path/pressure/delay checks passed each.
- Authored dry/hard-wall/absorber/blocked WAV demo: hard-wall excess delay
  **0.069970846 s**, post-source energy **2.638427589**; absorber energy
  **0.026384251** (about 1/100); dry/blocked post-source energy zero. All peaks
  0.528611. One-second mixing: 0.002065–0.002616 s on this host. No game data,
  measured HRIR, font, GPU or audio output device was used. WAVs remain ignored.
- Evidence in target/verification: reflection-library-final-tests.log,
  reflection-engine-final-tests.log, reflection-native-final-tests.log,
  reflection-js-final-tests.log, reflection-api-final-tests.log,
  reflection-workspace-check.log, reflection-minimal-check.log,
  reflection-python-tests.log, reflection-large-scene.log and
  reflection-demo-final.log.

Remaining boundaries:
- Automatic pose refresh, dynamic acoustic geometry/refit, higher-order/diffuse
  paths, diffraction, Doppler, reflection HRIR/Ambisonics, physical room/RT60
  inference and seamless tap transitions remain open. Echo DSP uses linear
  fractional delay/one-pole bands, not a bandlimited wave solver. Its 10 ms
  post-delay band-history horizon truncates tiny remaining IIR residue.
- Final DSP/mixing, control/scratch allocation, last-reference disposal and device/
  pool teardown retain owner/synchronous boundaries. No full engine-wide async
  completion, new GPU/hardware RT or Windows/Linux behavior is claimed. The
  broader modern engine/VN capability objective remains active.

## Automatic reflection tracking (ADR 0075, 2026-10-27)

Implementation:
- Added engine-owned ReflectionTrackingSpec and Audio.trackVoiceReflections,
  plus playback `reflections` registration before source consumption. Updates
  run after listener/camera and entity-follow poses. Initial short sounds wait
  for enabled HRTF then reflection preparation. Static captured APIs retain
  explicit invalidation/re-preparation semantics; mode/settings replacement
  clears history, while ordinary pose updates preserve it.
- Split immutable worker-prepared ReflectionPlan from mixer-owned EarlyReflections
  history. Tracked effects reserve configured maximum delay once and record even
  empty plans. At most two plans render with sample-bounded linear crossfades.
  Completed previous plans/leases are reclaimed by the control owner, not in the
  sample loop. Seek resets filled history without stale samples. Shared Rust
  Mixer/ChannelAudio publication works without App, VM, script or device.
- Coalesced immutable source/listener/revision/latency snapshots use round-robin
  admission: 64 configured voices, 32 tracked flights globally, eight/client,
  four launches/frame and one flight or ready result/voice. The cap is checked
  before advancing the queue, covering the five-voice starvation boundary.
  Full pools defer; residency exhaustion records waiting with interval retry.
  Initial effects reserve 8 MiB, updates 64 KiB, shrinking to prepared capacities
  plus 8 KiB against the existing 512 MiB estimated host reflection budget.
  Actual workers, results and installed/retired objects retain their leases.
- Publication accepts a completed captured pose behind continuous movement,
  then schedules the coalesced desired pose; scene revision and direct HRTF
  latency remain strict. Incompatible old taps mute immediately and compatible
  replacements fade from silence using retained input history. Worker failure
  releases initial holds and retains a still-valid old effect. Clear, stop and
  restore prevent stale publication. Source EOF stops new geometry jobs and
  drains the installed target; native waits still use source completion.
- reflectionsVoice exposes tracking settings, desired/submitted/published
  snapshots, capture time/published age, pending/waiting/error, muted/draining
  and successful update count. Targets may lag under contention; publication
  is separate from audible fade completion. API docs regenerated and logged
  079531ae354ff224 -> 1ff0b7ce813fb81e. No legacy engine or VM-private algorithm
  was imported; original native adapter implementations/opcodes are unchanged.

Compilation:
- Local release workspace/all-target check passed with desktop krkr, arc-formats
  and acoustics-obvhs. Minimal desktop no-default reallive+avg32 check passed.
  Existing CBG/RealLive/minimal-CLI warnings remain; no new warning was observed.
  No GitHub CI or remote build ran.

Behavior validation:
- Acoustics/audio/tasks with obvhs: **95 passed**, two ignored. Engine audio,
  acoustics/HRTF/reverb/static/tracked reflections: **41 passed**. JS async
  services: **8 passed**, one font case ignored. Original RealLive/AVG32 bridges:
  **12 passed**, one font case ignored. API reference: **6 passed**. Total:
  **162 distinct Rust tests passed**, four ignored excluded; the final five
  tracked-engine reruns are not counted twice. Python 3.11: **189 passed**.
- Four new device-free DSP/channel cases verify past samples survive changed
  delays, initially empty plans retain history, exact fade samples, busy-plan
  rejection, owner-side retirement/receipt lifetime, obsolete-geometry silence,
  seek isolation/capacity rejection, scoped channel publication and source EOF
  independent of retained echoes.
- Five engine cases verify continuous moving-source publication, listener
  orientation, blocked workers retaining audible old history, client scopes,
  clear/restore cancellation, scene replacement mute/recovery, both five-voice
  and twelve-voice fair admission, and a **one-frame sound** with HRTF-before-
  reflections dependency. Its real isolated echo reaches the output after
  source EOF. The same playback case verifies entity following, camera listener
  movement and entity destruction. Latest tracked-engine rerun passed after
  capture-age/draining-status additions.
- JS now exercises playSfxAsync with reflection settings, the explicit tracking
  method, listener movement, observed worker publication and resource clear.
  The initial JS failure was a fixture missing existing required listener
  forward/up fields; the corrected fixture passed. API blessing initially
  required logging the new fingerprint; the logged final reference test passed.
- Authored moving_reflections example exports 48 kHz stereo, three-second
  stationary/moving wet-only WAVs. Moving output uses **40 plan updates**, peak
  **0.121842**, versus stationary **0.127668**; squared PCM difference
  **71.441991051**. Worker wall time (including offline fences) 0.000328 s for
  moving plans; three-second mixing 0.005390 s on this Apple M5 run. These tiny
  authored-scene timings do not rank backends or establish live-scene throughput.
  An independent exported-PCM check verifies left-dominant early moving energy
  (4.388710 / 3.508156), right-dominant late energy (2.249148 / 2.863898), and
  a silent final half-second. Stationary output stays left-dominant. No game
  data, measured HRIR, font, GPU or audio output device was used.
- Evidence under target/verification: tracking-library-tests.log,
  tracking-engine-final-tests.log, tracking-native-tests.log,
  tracking-js-tests.log, tracking-api-final-tests.log,
  tracking-workspace-check.log, tracking-minimal-check.log,
  tracking-python-tests.log, tracking-demo.log, tracking-demo-oracle.log and
  moving-reflection-demo/*.wav. Generated WAVs/evidence remain ignored.

Remaining boundaries:
- Geometry remains baked/static; no mesh refit, higher/diffuse bounce,
  diffraction, reflection HRIR/Ambisonics, continuous Doppler or physical waves
  already in flight. Discrete captured delays crossfade; they are not continuous
  physical resampling. Source directivity stays authored in world space rather
  than implicitly following entity rotation. Explicit reconfiguration cuts tails.
- Final DSP/mixing, control/scratch allocation, last-reference disposal and device/
  pool teardown retain synchronous owner boundaries. No complete engine-wide
  async migration, new GPU/hardware RT, other-platform/audio-device behavior or
  additional commercial VN opcode/save compatibility is claimed. The broader
  modern engine/VN capability objective remains active.

## Shared transport and original Siglus BGM (ADR 0076, 2026-10-27)

Implementation:
- Added reusable core AudioPlaybackInfo/HostServices.audio_info and shared
  Mixer/ChannelAudio inspection. Decoded-source frame/rate/length, source
  completion, pause/loop mode and output-clock fade/delay counters are distinct.
  Audio.playbackInfo/nativeChannelInfo/scheduleVoiceResume expose the same
  engine resources without a native VM. Canonical channelStatus stays separate.
- AudioParameters now supports exact start_frame overriding start_ms, paused
  publication, optional clamped source/loop ranges and replacement crossfade.
  Workers seek/prewarm once; owner publication synchronizes both logical and
  resampling cursors so the first mix does not perform another seek. Immediate
  replacement uses the exact channel slot and restores old state on validation
  failure. Crossfade needs a spare voice; capacity failure preserves old audio
  without stealing another channel. Ended/stolen controls are idempotent.
- Added scheduled resume with a fresh fade-in and output-clock delay. Explicit
  stop/pause deadlines now progress through source starvation/preparation and
  silent paused input; source position and ordinary initial fade stay frozen
  without PCM. Pause/stop/resume supersedes a pending schedule. The mixer still
  executes DSP/output synchronously; no whole-engine async completion is claimed.
- Replaced Siglus BGM's unbound logical placeholder with semantic operations over
  shared channel services. Original SceneVm execution, positional/named argument
  decoding, native value/state codes and wait machinery remain. PLAY/one-shot/
  wait/READY, retained-source pause/resume/delay, stop/fade waits, WAIT_KEY,
  volume/name/position/check and BGMTABLE flags have mapped bounded profiles.
  No native BGM decoder/player/Kira hub/mixer/renderer was copied into the hook.
- Resource admission retains post-command VM state separately while the visible
  VM stays at the original instruction/arguments. Only Ready commits it; failure
  releases the receipt without running following instructions. Unexpected status
  errors keep the same live ticket. Native resource work remains cooperative,
  client-scoped and independent of other clients' playback.
- Native BGM files require explicit client resources aliases. Optional decoded
  UTF-8 Gameexe rows provide start/end/repeat frames; rows/declared count are
  bounded at 4096. Unknown opcodes/property fallbacks and malformed layouts fail
  visibly. Native gallery flags remain VM logical/save state, not engine DSP.
- Native Siglus runtime payload schema changed 1 -> 2; incompatible old payloads
  require migration. Pending snapshots serialize the uncommitted instruction,
  never tickets. Active snapshots reprepare declarations after candidate commit
  at authored start, without claiming device/sample-history checkpoints. New
  logical-state validation rejects malformed restored BGM before resource work.
  Original file codecs/header/layouts remain unchanged. Existing import rollback
  now advertises transactional import; direct original-file operations reject
  an in-flight resource command instead of mutating its separately retained VM.
- Verified the existing original Git pin 5f358ef1360b0439e92aed2b749e8f25d8721954
  and its 2519-file tree. Nine scoped source audit records cover full BGM/audio
  files and selected complete resource/wait/argument/save call paths, with exact
  immutable hashes in docs/siglus-bgm-source-audit.json. Existing reference
  opt-in-device differences are explicit; this is not whole-engine semantic
  coverage. Normal Siglus VM dependency inspection excludes kira, rodio, cpal,
  wgpu, winit and ash. API docs regenerated/logged
  1ff0b7ce813fb81e -> 4aee8cb171139420; Rust literal/enum and VM-payload breaking
  changes are documented separately from additive JS/TS methods.

Compilation:
- Local release workspace/all-target check passed with desktop krkr, arc-formats
  and acoustics-obvhs. Minimal no-default desktop reallive+avg32 check passed.
  Existing CBG/RealLive/minimal-CLI warnings remain. No GitHub CI or remote
  build ran; no output device/GPU or Windows/Linux behavior was exercised.

Behavior validation:
- Shared audio/core/compat/tasks suites: **104 passed**, one owner NWA-resource
  case ignored. Runtime/Siglus unit, native audio and media snapshot suites:
  **70 passed**, one font case ignored. Engine audio/acoustics/HRTF/reverb/
  reflections: **42 passed**. RealLive/AVG32 bridges: **12 passed**, one font
  case ignored. JS async services: **8 passed**, one font case ignored. API
  reference: **6 passed**. Total: **242 distinct Rust tests passed**, four
  ignored excluded. Focused reruns are not counted twice. Python 3.11 tooling:
  **189 passed**.
- Four transport sample cases verify exact frame-seven paused publication,
  source/output units, bounded range/repeat clamping, delayed resume/fade samples,
  cancellation, silent stop/pause deadlines on starvation/preparation, ordinary
  fade-in hold, source freeze, stale controls and atomic failed replacement.
  A full **256-voice** pool rejects crossfade without stealing/publishing, retains
  old audible samples, then replaces the exact channel immediately. A separate
  counted-source unit proves **one seek total** across worker preparation,
  publication and first mix.
- Seven authored native PCK/SCN cases run the original interpreter: worker-blocked
  playback retains PC/arguments while another client's PCM advances; Ready starts
  at source frame seven; source waits resume arithmetic while a real shared BVH
  echo remains/drains; pending checkpoints replay without tickets. READY, delayed
  resume, pause fade, WAIT_FADE and RESUME_WAIT observe shared state/positions.
  WAIT_KEY returns 1 only after DECIDE release, returns 0 on natural EOF, and
  leaves a looping sound running. Broken resources retain the original PC and
  retry after repair. Same-loop-name playback succeeds without reopening a
  deleted source; active snapshots rebind; gallery count/listened flags persist.
  Unknown operations, oversized tables and unsupported freed-source RESUME fail.
- Original native save-file export/import passes playing and READY-paused loop
  declarations with volume 128/255 and authored start/end/repeat frames. Invalid
  import rolls back native state; successful import reopens through shared
  preparation and resumes the original VM checkpoint. Actual PCM amplitude is
  checked before export/reload. Existing native save/text/DECIDE cases still pass.
  This bounded evidence does not establish every original save variant or title.
- Modern class tests inspect actual pooled/native channel state, exact output-
  clock delay and sample-level fade, and reject foreign handles/channels. JS
  inspects original 8 kHz source units versus the 48 kHz output delay counter
  through the new registry methods. Native generated music, scenes and Gameexe
  fixtures are self-authored; no game, measured HRIR, font, SDK or device data
  is redistributed. API blessing initially required logging its new fingerprint;
  the final logged reference check passed.
- Evidence in target/verification: transport-library-final-tests.log,
  transport-siglus-final-tests.log, transport-engine-final-tests.log,
  transport-other-native-tests.log, transport-js-tests.log,
  transport-api-final-tests.log, transport-workspace-check.log,
  transport-minimal-check.log, transport-python-tests.log,
  transport-final-focused.log, transport-native-validated-tests.log,
  transport-cursor-final-tests.log and siglus-bgm-dependencies.txt.

Remaining boundaries:
- RESUME after EOF/immediate stop needs an engine-owned retained prepared-source
  resource to match native data retention and replay. It explicitly fails;
  reopening a mutable asset path is not substituted as equivalent behavior.
  This shared resource primitive is the next concrete audio gap, also relevant
  to KiriKiri WaveSoundBuffer. Explicit PLAY remains new resource preparation.
- Other Siglus audio forms (PCM/PCMCH/SE/voice), resource/append discovery,
  encrypted Gameexe conversion, native archive-entry selection and broader native
  text/layout/save/game coverage remain open. Native boot/Gameexe reads, bounded
  metadata translation, original VM ordering, final DSP/output, owner allocation/
  disposal and teardown remain synchronous boundaries. Dynamic acoustic geometry,
  Doppler, higher/diffuse reflections and complete engine-wide async remain open.
  The larger modern engine/VN capability goal remains active.

## 2026-10-27 — immutable prepared audio resources and Siglus retained-slot replay (ADR 0077)

Implementation:
- Added engine-owned AudioResources/PreparedSound, independent of any VM/device.
  Core exposes plain scoped AudioResourceHandle/Info/Preparation and HostServices
  resource preparation/status/info/release; AudioRequest::PlayResource captures
  an immutable source lease before worker admission. Core gains no dependency.
- Workers snapshot client-capability encoded assets, probe shared decoders and
  optionally scan/discard bounded chunks when frame count is unknown. Each play
  creates its own decoder/cursor and uses existing buffered realtime transport;
  explicit offline native hosts retain immediate preparation receipts. A measured
  source boundary propagates into independent decoder instances. Unknown lengths
  are not presented as exact merely because a frame cap was requested.
- The bank bounds encoded assets at 128 MiB, handles at 32/client and 128/host,
  and retained preparation receipts at 16/client and 64/host. Atomic reservations
  cap the bank at 512 MiB, including estimated probe/metadata allowances; these
  are not whole-process RSS measurements. Stream/mixer budgets are separate.
  Snapshot Vec storage moves into an Arc without another full encoded copy.
- Only scoped preparation observation publishes resources. Cancellation cannot
  publish unobserved completed work. Receipt release keeps published resources;
  resource release prevents new lookup while admitted/live decoder leases retain
  bytes. Clear/restore invalidates capabilities without resetting charges still
  held by actual workers/instances. IDs use the existing monotonic host namespace.
- Modern Rust/JS/TS Audio adds beginPrepareSource/sourcePreparation/
  cancelSourcePreparation, sourceInfo/releaseSource and beginPlaySource, with
  prepareSourceAsync/playSourceAsync helpers. Voice spatial/follow/acoustic/HRTF/
  reflection options use the existing shared publication path. Asset in resource
  voice options is rejected; optional preparation options accept omission/null.
- Siglus PLAY/READY now has separate engine resource and voice completion stages.
  Original PC/stacks remain uncommitted until publication. Failed second-stage
  preparation releases its new resource and preserves the previous slot.
  RESUME/RESUME_WAIT after EOF/stop uses the retained engine resource without
  resolving/reopening its original file. Slot loop/range and current gain/fade/
  delay translate into shared parameters; STOP's cleared outer name/loop values
  remain distinct from the prepared source's loop region.
- Stopped/ended logical runtime declarations reconstruct only a resource, with
  no hidden voice or new gallery mark. Active declarations restart at authored
  start. Bindings/tickets are serde-skipped; Siglus runtime schema stays 2 and
  original file layouts stay unchanged. Snapshots still require asset aliases
  and files to rebuild; they do not embed encoded/decoder/device/DSP state.
- Existing pinned Siglus BGM/source audit remains applicable; reviewed retained
  slot prepare/start/stop/resume/retire behavior, without copying a native player,
  decoder, sample buffer, envelope, mixer or renderer into the adapter. API docs
  regenerated and logged: 4aee8cb171139420 -> 5b75e19d7feee9bf.

Compilation:
- Final local release workspace/all-target check passed with desktop krkr,
  arc-formats and acoustics-obvhs. Minimal no-default desktop reallive+avg32 check
  passed. Existing CBG/RealLive/minimal-CLI warnings remain. No CI, remote build,
  audio output device, GPU, Windows/Linux or console behavior was exercised.

Behavior validation:
- Final selected Rust suites: shared audio/core/compat/tasks **109 passed**;
  Runtime/Siglus **105 passed** (full run plus final 11-case native audio rerun
  replacing its earlier 10-case subset); engine audio/acoustics/HRTF/reverb/
  reflections and API reference **52 passed**; RealLive/AVG32 bridges **12 passed**;
  JS async services **9 passed**. **287 distinct passed**, four ignored excluded.
  Focused reruns/earlier extra smoke cases are not added again. Python 3.11
  tooling: **189 passed**.
- Five independent resource tests verify immutable bytes after replacement,
  independent seek/read cursors, WAV/Ogg/MP3/FLAC decoder instances, propagated
  frame metadata, worker-only reads, foreign/stale handles, ready-unobserved and
  queued cancellation, handle bounds, failed probe cleanup, admitted playback
  after release, and byte charges until the actual last lease drops after clear.
- Modern API tests share one resource between pooled and native channels, remove
  the original file, release the handle while admitted playback is worker-gated,
  then assert original 0.5-amplitude PCM. Other-client lookup/release, restore,
  optional options and canceled preparation are checked. JS promises publish
  resources/voices and preserve their separate receipt/handle/instance lifetimes.
- Eleven authored original SceneVm audio cases include deletion of the source
  file before stopped/EOF RESUME_WAIT, frame-seven PCM replay, inactive named and
  stopped snapshot reconstruction, preserved gallery/outer-loop semantics and
  failed voice-stage cleanup preserving the previous audible looping resource.
  Existing original-save, waits/keys/delay/fade/volume, failure/retry, worker gate,
  same-name idempotence, source-versus-effect-tail and unknown-op cases pass.
- Evidence: target/verification/resources-{shared-tests,runtime-tests,engine-tests,
  vn-bridge-tests,js-tests,siglus-tests,bank-final,python,workspace-final,
  minimal-final}.log. API generation/reference is included in engine-tests.

Remaining boundaries:
- This closes ADR 0076's retained-source replay gap. Native PCM/PCMCH/SE/voice,
  full-game/native resource discovery/encrypted Gameexe, full async final DSP/
  mixing/output, dynamic acoustic geometry, Doppler and higher/diffuse reflection
  remain open. Inactive snapshot reconstruction follows committed VM state;
  halted VMs are not stepped solely to observe subsequent audio EOF, so their
  snapshots can retain the last active declaration. Same-size writes during
  snapshot reads can race; probing known-length sources is not full-file packet
  validation. Broader engine/VN and ADR 0050 goals remain active.

## 2026-10-27 — shared mix groups, playback age and original PCM/global SE (ADR 0078)

Implementation:
- Added engine-owned client-scoped named gain groups, independent of any VM.
  Core supplies plain AudioMixGroupSettings/Info and SetMixGroup/SetMixGroupGain
  requests, without dependencies. VoiceSpec/AudioParameters gains optional
  mix_group; publication resolves a dense index in the owning mixer's namespace.
- Candidate graph validation checks parent authority, cycles, descendant depth,
  gains/fades/names and limits (64 groups/client, 256/mixer, depth eight) before
  mutation. Parent-before-child evaluation performs no name search/allocation
  per output frame. Removal rejects children and live source/effect references;
  pending playback revalidates its logical name at publication. Leaf slots reuse
  safely without moving indices referenced by another client/voice.
- Extracted the existing shared output-frame Ramp into one internal module.
  Group fades progress during silence, pauses, dependency holds and starvation;
  independent source positions/envelopes remain separate. Group/ancestor gains
  apply once before existing reverb sends/Master. Previously committed late wet
  history stays on the return bus. Dialogue duck admission evaluates group level
  per sample and remains invariant across block partitions.
- Public Audio setMixGroup/mixGroup/nativeMixGroup/removeMixGroup/
  setVoiceMixGroup reaches pooled and native/media control graphs. Both validate
  before either changes. Partial fields retain values; parent:null detaches,
  while null/nonfinite gain, null mute and unknown fields fail. Gain-only native
  updates preserve host parent/mute. Host group controls persist through playback
  clear/restore; they are not automatically serialized VM/project DSP graphs.
- AudioPlaybackInfo.age_frames is a shared output-frame publication age, distinct
  from decoded position/read-ahead. Pause, holds, starvation and effects count;
  seek does not reset age. Native PCM/SE resource preparation measures unknown
  lengths, then independent voice preparation captures a lease. Temporary lookup
  handles release at admission; workers/live instances retain actual encoded bytes.
- Original global PCM PLAY/STOP and global SE filename/number/explicit voice-alias
  play, volume/extremes/getter, stop/fade, CHECK and WAIT/WAIT_KEY use CrySirror
  resources, channels and siglus.pcm/siglus.se gain groups. The adapter retains
  native identity/volume/observed playing state and ordered semantic requests;
  no native decoder, sample cache, timer, envelope, Kira hub/mixer or renderer is
  copied/linked. Integer-ms SFX end predicates compare shared publication age;
  they can end before the final fractional-ms PCM samples. SE key wait keeps its
  native zero return, with playback continuing after skip.
- Canonical resources aliases are mandatory. Decoded Gameexe numbered SE uses
  native count default16/min8/max256; SE voice numbers use explicit CrySirror
  aliases koe/{number}, without guessing native packs/filenames. Unknown forms,
  missing entries/aliases and property fallbacks fail visibly. BGM/PCM/SE pending
  operations carry ephemeral ordering stamps so original import stops PCM/SE
  before a later BGM preparation can block; no stamp/ticket enters saved data.
- Siglus runtime payload schema is **3** for new PCM/SE logical declarations;
  schema-2 snapshots require migration and are rejected by the version gate.
  Active declarations rebuild only after candidate commit, with no exact PCM/
  decoder/age/effect checkpoint. Original file codec/layouts are unchanged:
  nonpersistent global PCM/SE stops on import and category volumes restore via
  shared controls. The isolated codec test host uses real ChannelAudio through
  a dev dependency; normal VM dependency inspection excludes crysirror-audio,
  Kira, rodio, cpal, wgpu, winit and ash.
- Verified existing immutable siglus_rs pin 5f358ef1360b0439e92aed2b749e8f25d8721954
  and six scoped immutable source/hash audit records, including full SFX/forms/
  hub and selected SE-table/save functions. Reference hub opt-in-device changes
  remain explicit. API reference regenerated/logged:
  5b75e19d7feee9bf -> f2af02453b2f74f8.

Compilation:
- Final release workspace/all-target check passed with desktop krkr, arc-formats
  and acoustics-obvhs; minimal no-default desktop reallive+avg32 check passed.
  Existing CBG/RealLive/minimal-CLI warnings remain; new fixture warning was fixed.
  No GitHub CI/workflow, remote build, output device, GPU, Windows/Linux or
  console validation ran. Normal VM dependency audit is in the local evidence.

Behavior validation:
- Final local release scopes: shared audio/core/compat/tasks **115 passed**;
  Runtime/Siglus **114 passed**; engine audio/acoustics/HRTF/reflection/reverb and
  API reference **54 passed**; RealLive/AVG32 bridges **12 passed**; JS async
  services **10 passed**. **305 distinct Rust passed**, four ignored excluded.
  Focused/earlier runs are not counted twice. Python 3.11 tooling: **189 passed**.
- Six independent group tests assert hierarchy/client routing, live re-routing
  without seek, bus/Master scaling, atomic graph/cycle/depth/capacity failure,
  silence/pause fade clocks, sample-boundary duck release and block partition
  invariance. Native gain requests preserve host parent/mute. A grouped impulse
  matches independent voice-gain dry/late-reverb PCM; committed tail survives
  group mute/removal. Transport assertions separate starved/paused source position
  from output publication age. Public API tests cover both mixers, invalid/null
  changes, parent detach/partial retention and second-mixer admission rollback.
- **20 authored original SceneVm audio cases** pass. New cases exercise native
  PCM/SE independent fade samples, file/SE-number/explicit voice-number playback,
  true worker gates and pending snapshots, failure preserving the old source and
  original PC followed by repair/retry, active rebind, native zero-return key wait,
  original file volume/stop behavior and cross-category stop ordering while BGM
  workers are blocked. A whole-ms wait releases while physical PCM is still live.
  Existing BGM/source/effect/save/unknown-op coverage remains passing.
- Ran standalone mix_groups example in release, producing a three-second stereo
  48 kHz WAV from authored tones. Parent music fades leave the UI stem unchanged;
  Rust asserts its samples and restored level. Clear/muted/restored RMS:
  **0.158114 / 0.070711 / 0.158114**, peak **0.286419**. No game data/fonts/device.
- Evidence: target/verification/groups-{shared-tests,runtime-final,engine-final,
  js-final,vn-tests,transport-final,python,workspace-final,minimal-final,demo,
  native-normal-dependencies}.log and mix-groups.wav. Source review:
  docs/siglus-sfx-source-audit.json; contracts/limits: ADR 0078.

Remaining boundaries:
- These are gain/control hierarchies, not full Sound Class/submix effect graphs,
  group concurrency or virtualization. PCMCH/PCM events, general KOE speech/
  JITAN/mouth tables, native UI sound slots, automatic resource/append discovery,
  encrypted Gameexe, all save variants and whole-game compatibility remain open.
  Native polling follows the engine's current VM-before-mix frame phase and is
  not an exact wall-clock/device timing match under starvation. Halted snapshots
  can retain the last observed native declaration. Final async DSP/mixing/output,
  dynamic geometry/Doppler and higher/diffuse acoustics remain active goals.

## 2026-10-27 — worker final DSP frames and desktop dependencies (ADR 0079)

Implementation:
- Added shared CPU MixJob/MixFrame over the bounded TaskPool. A worker owns the
  actual mixer/decoder/effect histories for a frame; owner slots retain scoped
  readonly observations. Completion validates all leases, restores DSP state and
  publishes combined unclipped PCM once. This works without a VM/device/window.
- Jobs admit 1–8 same-rate mixers and at most two seconds per frame, with per-mixer
  counts and 4096-frame cancellation blocks. Prepared frames retain PCM/counts
  while capacity is full. Ordinary controls fail during a lease; infallible
  client pause/stop applies at publication. Reset/restore invalidates old tokens;
  failures/panics release playback slots and publish no failed PCM.
- Runtime.begin_tick/poll_audio_frame and App.begin_frame/poll_frame expose the
  dependency. Callbacks, VM/world time and accepted input execute once. App joins
  pooled/canonical and native/media audio in one worker product; engine frame
  completion/input commit follows publication. Pause defers, shutdown cancels,
  and validated restore discards obsolete PCM. Synchronous frame/tick remain
  explicit offline/reference paths; tick_control requires an external coordinator.
- Plain and scripted desktop now use worker final DSP. The pending-frame gate
  precedes elapsed-time/gamepad/input-edge acquisition. CPU mixing remains
  available with the optional output device disabled; muted checks open no device.
- Audio.rendering includes prepared capacity waits and running work. Playback
  and group observations remain at the last publication. Prepared playback
  receipts cannot report ready before owner publication during a DSP lease.
- Extended the authored gain-group demo with --worker for sample and exported WAV
  comparison. Added independent worker/sample/transport/failure/reverb-history
  oracles, App lifecycle/backpressure/restore tests, desktop driver tests and an
  original SceneVm WAIT/restore test. No new VM hooks or legacy DSP were imported.

Compilation:
- Release workspace/all-target check passed with desktop krkr, arc-formats and
  acoustics-obvhs. Minimal no-default desktop reallive+avg32 check also passed.
  Existing CBG/RealLive/minimal-CLI warnings remain. API reference regenerated;
  signature fingerprint f2af02453b2f74f8 -> bec05c019d833cb0 is logged in CHANGES.
- No GitHub CI/workflow or remote build was added/run. No live window, GPU,
  output device, Windows/Linux or console behavior was tested by this phase.

Behavior validation:
- Final local release scopes: shared audio + full Runtime **192 passed**;
  engine App/audio/frame/API reference **42 passed**; JS async services **10
  passed**; desktop CPU driver/input/mix tests **6 passed**. **250 distinct Rust
  passed**, three ignored excluded. Earlier/focused reruns are not counted again.
  Python 3.11 tooling: **189 passed**.
- Eight shared worker cases assert actual source reads on a worker thread,
  synchronous-reference PCM, frozen queries/transport, rejected concurrent DSP
  mutations, capacity retries retaining PCM, independent producer frame counts,
  deferred client controls, stale generation/reset cancellation, source failures,
  task panic, queued cancellation and bounded validation. Multi-frame reverb
  history produces identical samples to a continuous synchronous render.
- Five App/frame cases cover callbacks/time/input once under backpressure,
  readonly Audio.rendering and native observations, paired PCM equivalence,
  deferred pause, restore/shutdown discarding old output, standalone Runtime
  completion and completion errors faulting the App. Both default desktop core
  variants enforce the gate before another tick and explicitly cancel at exit.
- All **21 authored original SceneVm audio cases** pass. The new case blocks CPU
  workers after native BGM preparation, verifies unchanged original VM diagnostic
  state and published age while DSP is pending, then observes publication and
  WAIT completion. Restore cancels the obsolete audio and rebinds resources via
  the same engine API; no native audio runtime or renderer is hosted.
- Full regression caught the new two-second admission check incorrectly rejecting
  a long idle synchronous tick. Fixed the no-audio path to retain its original
  contract. Runtime save rewind and the four-second VFX history test now pass;
  active audio/worker admission keeps its bounded profile.
- Ran the standalone mix_groups demo in synchronous and --worker release modes.
  Both authored assertions pass and exported WAV files are byte-identical:
  stereo 48 kHz, 16-bit, 144000 frames/three seconds, 576044 bytes; SHA-256
  a6cb3e72588260026d4b6a5789f47dcda27969d0f1694d01a236aa551849e0eb.
  Clear/muted/restored RMS 0.158114 / 0.070711 / 0.158114; peak 0.286419.
- Evidence: target/verification/mix-worker-{shared-runtime-final,engine-final,
  js-final,desktop-tests,workspace-check,minimal-check,api-bless,demo-reference,
  demo,python-final,diff-check}.log; mix-worker-validation.json,
  mix-worker-demo.json and mix-worker{,-reference}.wav. The earlier broad
  regression logs include the idle-tick failure; final scopes above pass.

Remaining boundaries:
- Final DSP is worker-owned but frame-dependent. Shared-pool contention can
  postpone simulation/render publication and starve device output. There is no
  dedicated device-clock renderer, sample-timestamped future command queue,
  allocation-free owner handoff or realtime throughput/latency guarantee.
- The existing two mixer instances retain independent duck/effect histories;
  final PCM addition does not create cross-mixer sidechains or a submix graph.
  PcmOutput still opens devices on submission and uses a mutex for its bounded
  queue/refill callback. Device lifecycle/loss, lock-free transfer and pool
  teardown/join remain work. No hardware performance claim follows from CPU tests.
- General native startup/I/O and VM scheduling, PCMCH/KOE, exact legacy timing,
  dynamic acoustic geometry, Doppler and diffuse/higher-order propagation remain
  open. This phase adds reusable scheduling and completion capabilities, not
  whole-game compatibility or completion of the engine-wide async requirement.

## 2026-10-27 — device worker, bounded PCM transfer and startup dependency (ADR 0080)

Implementation:
- Modern PcmOutput now owns a shared fixed-capacity stereo packet queue and an
  OutputBackend factory. A dedicated worker opens/builds/starts/destroys the
  platform stream on that same thread; it never borrows a VM/window/EngineContext.
  Streams need not be Send. submit admits work without device I/O or thread join
  on the caller. Opening/readiness, retained failure and actual worker-resource
  close completion are separate OutputWatch/OutputStatus observations.
- Crossbeam ArrayQueue 0.3.14 replaces the CrySirror Mutex<VecDeque>/512-sample
  refill. It stores at most 12000 complete stereo 48 kHz frames, keeps newest
  frames on overflow and records drops. Final clamp/nonfinite sanitization stays
  after engine mixing. Pause preserves source state; epochs/reset invalidate old
  queue/interpolation data. Callback transfer and its fixed UTF-8 error lane use
  no control mutex or growable buffer. This is lock-free, not wait-free or a
  claim about CPAL/OS internal locks, allocations or realtime deadlines.
- Native stereo 48 kHz bypasses rate conversion. Other device rates use fixed
  linear interpolation; mono averages L/R, channels 0/1 carry stereo and extra
  channels are silent. Misaligned callback buffers fail visibly without consuming
  input. CPAL sample conversion covers its integer/float formats. There is no
  bandlimited resampling or surround speaker-layout implementation in this bridge.
- begin_close and Drop request cancellation without joining the owner. Completion
  follows stream destructor/unwind return; cleanup panic remains a failure.
  Opening calls can only observe cancellation after returning. Explicit retry
  requires failed/closed completion and isolates a fresh session/queue/epoch.
  A callback failure retains its diagnostic even when close starts before cleanup.
- Desktop now enables cpal-output, directly creating a CPAL stream rather than a
  Rodio Sink. It polls failures without subsequent PCM, gates startup before time/
  gamepad/input acquisition while the device opens, excludes opening wall time
  from catch-up audio and requests close even if lifecycle shutdown fails. The
  explicit rodio-output compatibility feature still compiles its older synchronous
  AudioDevice API; default desktop does not use that endpoint.
- Independently checked current CPAL 0.18.2, but Cargo resolution rejected it:
  existing WMV CPAL 0.15/alsa-sys 0.3 conflicts with 0.18/alsa-sys 0.4 native links.
  The implemented backend pins existing CPAL 0.15.3. Source revisions/checksums
  are in docs/audio-output-source-audit.json. No upstream implementation, device
  code or private legacy mixer was copied into a native adapter.

Compilation:
- Release workspace/all-target check passed with desktop krkr, arc-formats and
  acoustics-obvhs plus the optional rodio-output compatibility feature. Minimal
  no-default desktop reallive+avg32 check also passed. API docs regenerated;
  reflective fingerprint remains bec05c019d833cb0, with Rust host migration logged.
- Fixed a new unused test import and a probe example's missing local binding;
  final validation passes. Existing CBG/RealLive/minimal-CLI warnings remain.
  No GitHub CI/workflow or remote build was added/run. Windows/Linux/console/GPU
  behavior and a live desktop window were not tested in this phase.

Behavior validation:
- Final release scopes: shared audio including optional device/compatibility
  code **102 passed**; desktop CPU driver/input/output **8 passed**; original
  Siglus audio **21 passed**; original RealLive/AVG32 bridges **12 passed**;
  engine frame/API reference **11 passed**. **154 distinct Rust passed**, two
  ignored excluded. Earlier/focused reruns are not counted again. Python 3.11
  tooling: **189 passed**.
- Eleven independent output cases exercise lazy silence, truly blocked worker
  open/Drop and gated destruction receipts, stereo overflow/order/clamping,
  pause/underrun, reset of cached interpolation, partial-frame rejection,
  continuous-signal 24/44.1/96 kHz oracles across callback partitions, mono/extra
  channels, failed-session retry, callback failure/open panic, concurrent reset/
  overflow and failure retention through close. Callback/error transfer also
  completes while both control mutexes are deliberately held. CPAL integer/full-
  scale/unsigned-silence conversion has a separate device-free test.
- A rate oracle initially omitted the lookahead input frame at its authored
  signal endpoint; supplying that input makes the continuous linear oracle
  valid. Its corrected sample and partition assertions pass. Desktop tests
  verify no second simulation tick during gated opening, failure polling without
  PCM, plain/scripted worker completion and shutdown cancellation.
- Ran the release output_probe against the local macOS CoreAudio device:
  stereo **48 kHz f32** opened on the worker and produced callbacks. Paused
  state rendered **558 device frames**, retained all **128 queued frames** and
  reported zero underruns. After reset/resume, another **128 MIN_POSITIVE-level
  stereo frames** were dequeued by the real callback, reaching **1115 rendered
  device frames**, then close completed with no error and an empty queue. The
  rendered counter remained unchanged for the following **20 ms** after close.
- This short burst intentionally exhausts its input and pads silence: its **429
  active underrun frames** are recorded, not reported as a streaming benchmark or
  performance pass. No listening-quality, capture-loopback, latency or sustained
  throughput assertion was made; test values were below audibility.
- Evidence: target/verification/output-{audio-final,desktop-final,siglus-final,
  vn-final,engine-final,workspace-check,minimal-check,device-probe-final,python-final,
  diff-check}.log; output-validation.json, output-summary.json and
  output-device-probe-final.json. output-fetch.log records the rejected latest
  CPAL attempt. Earlier logs retain the corrected oracle/example failures.

Remaining boundaries:
- Engine DSP still follows simulation frames/shared CPU admission; device callback
  execution is not an independent engine audio render clock. Timestamped future
  commands, clock drift correction, realtime scheduling/latency tuning,
  bandlimited output resampling, surround/ambisonics, cross-mixer sidechains and
  default-device monitoring remain open. Stream failure has explicit retry;
  automatic recovery is not implemented. OS open/close cancellation is cooperative.
- CPAL 0.18 migration requires resolving the existing video/compatibility native
  dependency graph first. The local CoreAudio result establishes neither other
  hardware/platform behavior nor a complete implementation of ADR 0050. Native
  opcode/save/timing/resource discovery gaps and broader acoustics remain active.


## 2026-10-27 — prepared voice compression and original global KOE (ADR 0081)

Implementation:
- Added engine-owned, safe PCM16 Siglus JITAN numeric processing. It preserves
  rate, performs native stereo sum/clamp and splice/search/smoothing, handles
  sub-100-ms cues as same-length silence and retains the native final fade. This
  is separate from playback rate/pitch. No original converter/player/mixer code
  is linked into the native adapter. Reference pin and exact reviewed blobs are
  recorded in docs/siglus-koe-source-audit.json.
- Added plain core AudioResourceTransform and transformed-resource preparation,
  plus public AudioSourcePrepareSpec/AudioTransformSpec options for the existing
  beginPrepareSource / JS prepareSourceAsync API. Shared audio owns capability
  reads, explicit decoder layout, bounded finite PCM conversion, processing,
  WAVE serialization, memory charges, worker products and owner publication.
  Voices retain independent shared decoder leases. Percent 100 preserves the
  encoded source; frame_limit applies to decoded input before processing.
- Nonidentity processing admits at most 4194304 input frames, mono/stereo and
  8–192 kHz, reserving 128 MiB scratch inside the shared 512 MiB resource budget.
  Temporary output overlap is charged; final storage shrinks to actual WAVE
  bytes plus 8 KiB. Existing receipt/handle scopes, cancellation and restore
  invalidation remain. Processing runs in the bounded asset/preparation pool,
  not a separate CPU DAG stage or a realtime streaming time-stretch effect.
- Replaced the volume-only global KOE stub with native identity/config/wait/save
  translation onto shared Voice bus, siglus.koe mix group and channel 3. KOE and
  EXKOE retain distinct message/playback identity; named wait/key/JITAN flags and
  selected JITAN config SET/DEFAULT/GET operations are mapped. Explicit canonical
  koe/{number} aliases supply resources. The 24-key number/rate cache contains
  engine handles; it owns no private generic audio buffers or decoding engine.
- Original replacement order stops the old voice before resource preparation,
  including failure/negative numbers. Readiness gates the original instruction
  commit. Failure is visible and repaired resources can retry. Native integer-ms
  completion uses shared output publication age; DECIDE releases a key wait
  without stopping the voice. Source completion excludes derived effect tails.
- Runtime adapter snapshots advance from schema 3 to 4 without old-schema
  migration. Active logical cues reconstruct at cue start, without preserving
  decoder/DSP history. Original Siglus file layouts/codecs are unchanged: local
  load stops speech and restores its volume.
- Review found that newer trailing KOE read-flag operands need unmapped MWND/
  read-skip bookkeeping. They now fail before audio admission and retain the
  unsupported diagnostic across subsequent steps; no fallback marks text read.
  The accepted global KOE profile uses the historical no-trailing-operand ABI.

Compilation:
- Final release workspace/all-target check passed with desktop krkr, arc-formats,
  acoustics-obvhs and optional rodio-output compatibility; minimal no-default
  desktop reallive+avg32 check also passed after the native read-flag guard.
- Generated API reference/JSON/TypeScript and logged fingerprint change from
  bec05c019d833cb0 to **24e44c371ca52eb7**. Final API reference tests pass.
  Existing CBG/RealLive/minimal CLI warnings remain. No CI/workflow, remote
  build, font/game data or proprietary SDK was added.

Behavior validation:
- Final release scopes: shared audio **106 passed**; engine audio/frame/API
  **28 passed**; JS async services **11 passed**; runtime library **50 passed**,
  authored original Siglus audio **28 passed** and original VM **12 passed**.
  **235 distinct Rust passed**, three ignored excluded. Earlier/focused reruns
  are not counted again. Python 3.11 tooling: **189 passed**.
- A temporary standalone harness executed the complete unmodified pinned JITAN
  converter and compared full PCM samples for **784 authored vectors**: four
  rates, two durations, mono/stereo, seven patterns and seven percentages. All
  matched after fixing an exclusive search-window boundary in the new profile.
  Permanent length/FNV-1a regression fixtures retain the independent results;
  the reference implementation/harness is not shipped in the engine.
- Worker-gated tests verify no early resource/playback publication, canceled work,
  strict transform options, client ownership, lease lifetime/file removal, exact
  transformed PCM hash, identity samples/encoded length, input-range semantics and measured output
  frames. Public Audio and real JS Promise calls exercise the same capability
  without a native VM or output device.
- Native Scene.pck tests additionally cover Voice bus PCM mute/unmute, JITAN
  config/cache replay after file deletion, shortened-source integer-ms wait,
  EXKOE named flags and preserved message identity, completed DECIDE versus
  unrelated direction input, replacement failure/repair, both save paths and
  repeated refusal of the newer unmapped read-flag ABI before any sound/resource.
  Early test assumptions about Space and anyhow's outer error text were fixed.
- The independent release voice_compression example exported authored 16-bit
  mono 48 kHz WAVE: **48000 input frames -> 24839 output frames** at 200 percent.
  Python's WAVE reader confirmed both headers/counts. No listening-quality,
  device, GPU, cross-platform or whole-game result is implied.
- Evidence: target/verification/koe-{audio-final,engine-final,js-final,native-final,
  workspace-check,minimal-check,api-final,demo,python-final}.log, koe-summary.json,
  koe-final-run.json and koe-reference-comparison-final.log. The initial API
  generation log retains the expected missing-fingerprint entry failure.

Remaining boundaries:
- Native speech pack/path/append-directory discovery, full MWND/KOE_ST and newer
  KOE read flags, backlog/menu replay, mouth-volume tables/emotes and complete
  character/system/BGM duck routing remain open. Storing a replay switch does
  not implement its consumer. At 100 percent the shared cache retains encoded
  bytes and redecodes per play; original decoded-cache performance is not claimed.
- Codec parity beyond the tested PCM profile, general high-quality streaming
  time stretching, an independent device render clock and the broader async/
  acoustics/3D backlog remain open. This is reusable engine processing plus a
  bounded native speech profile, not full Siglus/game or engine completion.


## 2026-10-27 — moving acoustic instances and World bindings (ADR 0082)

Implementation:
- Replaced the whole-scene flattened index with immutable per-mesh local BVHs
  and a Parry Qbvh world instance index. Local Parry/optional obvhs BVHs share
  across pose snapshots; unchanged meshes also share world-surface pages. Only
  changed surface pages and the top-level index rebuild on a pose commit. No
  handwritten BVH builder/refitter or Steam Audio SDK is added. Exact registry
  archive checksums, matching source-file hashes and inspected source scope are
  in docs/acoustic-instances-source-audit.json.
- Added AcousticMeshUpdate and immutable/cancellable instance commits for affine
  transforms and enabled flags. Queries retain normalized world-distance units
  through inverse transforms and nonuniform scale; winding normals include
  negative scale. Instance bounds use coordinate-scaled padding. Disabled meshes
  keep memory/stable triangle offsets and remain in complete-scene budgets.
- Public Audio.beginUpdateAcousticScene / updateAcousticsAsync gates explicit
  batches with scoped task receipts and owner publication. Unknown/duplicate IDs,
  empty/invalid batches, pending conflicting commits and manual edits to bound
  meshes fail visibly. Old probes keep immutable scenes; failed/canceled updates
  retain prior geometry. Local/world coordinates are now both bounded at +/-1e6.
- Audio.bindAcousticMesh / unbindAcousticMesh connects owned World entities via
  global matrix times explicit local offset, with optional inherited visibility.
  One automatic job/client coalesces latest poses at 50 ms minimum submissions;
  limits are 256 bindings/client, 1024/host. Missing entities disable geometry
  with diagnostics; invalid poses retain prior geometry. Unchanged failed poses
  do not spin/rebuild until changed or rebound. Unbind, successful full scene
  replacement, clear and restore cancel/remove derived binding work.
- Scene generation identifies full replacements; revision changes on every
  instance commit. Automatic direct and tracked reflection queries may publish
  completed older instance snapshots, expose geometry_stale/current_revision and
  then refresh. Full scene generations and HRTF latency retain strict invalidation.
  This avoids cancellation starvation during continued movement and preserves
  sample history/tap crossfades. Explicit static reflections remain revision exact.
- New geometry and automatic-binding stats expose actual reuse/work counts,
  desired/submitted/publication state, ages, pending/waiting/errors. Staging
  remains bounded by 128 MiB reservations / 512 MiB host budget; snapshot estimates
  include per-mesh and duplicated local/world storage. Shared allocations can be
  conservatively charged more than once, not reported as allocator measurements.
- Audio follow-Entity now uses the existing validated object resolver, preserving
  the object's actual client instead of reinterpreting a foreign ID locally.
  Native adapter opcodes/layouts and save schemas are unchanged; no new native
  spatial opcode or HostServices facade is claimed.

Compilation:
- Final release workspace/all-target check passed with desktop krkr, arc-formats,
  acoustics-obvhs and optional rodio-output; minimal no-default desktop
  reallive+avg32 check passed. Generated API reference/JSON/TypeScript and logged
  fingerprint change **24e44c371ca52eb7 -> e73bda1334c12a7f**. Final API and command
  coverage checks pass. Existing CBG/RealLive/minimal CLI warnings remain.
- Initial compile corrected the BoundingVolume trait import and the registry's
  List type hint. Release-only validation was used; no CI/workflow/remote build
  was added. No game, font, SOFA dataset or proprietary SDK bytes were added.

Behavior validation:
- Final release scopes: acoustic library/backend tests **23 passed** (including
  one distinct default-feature refusal case); engine acoustic/frame/async/API/
  reflection tests **43 passed**; JS async services **12 passed**. **78 distinct
  Rust passed**, three ignored excluded. Earlier/focused reruns are not counted
  again. Python 3.11 tooling: **189 passed**.
- Both local BVH backends pass analytic affine/nonuniform/negative-scale ray
  distance/normal cases, ignore/bounds/material behavior, immutable update errors,
  disabled reflection identity/budgets and existing direct/reflection tests.
  Unit tests assert shared Arc allocation identity for local BVHs and unchanged
  world pages; counters alone are not the evidence for reuse.
- Engine tests gate workers to verify no early commit, old captured probes,
  coalescing without resubmission starvation, parent transforms/visibility,
  stale-entity disable, unbind/cancel/restore and scope isolation. Invalid worker
  geometry retains the old scene/reservation, avoids identical retry spin and
  recovers after a valid pose change. A public obvhs scene retains that backend
  through an instance commit. Foreign Entity objects are rejected even when
  an entity with the same ID exists in the current client.
- Actual PCM goes from blocked silence to audible after geometry moves while
  the source age continues. Reflection tests remove a finite reflector, publish
  new paths and verify the output settles to silence after the tap transition.
  A first assertion incorrectly expected the crossfade prefix to be silent;
  the corrected oracle checks its settled suffix. Worker-gated direct/reflection
  tests require older instance snapshots to publish with geometry_stale=true,
  then converge to the current revision instead of discarding every completion.
- Real JS Promise/Entity facades prepare, update, probe and automatically follow
  World geometry. The authored moving_acoustic_door release demo exports stereo
  PCM16 **48 kHz / 72000 frames**, with closed/open/closed phase RMS approximately
  **0 / 0.1000 / 0** and independent reflected-path counts **0 / 1 / 0**. The WAV
  contains direct occlusion audio; the path counts separately verify echo
  visibility. Python WAVE decoding confirms PCM RMS **0 / 0.09998 / 0**. Geometry
  work runs on workers; no native VM or audio device is used.
- Evidence: target/verification/instances-{acoustics-final,engine-final,
  default-backend,workspace-check,minimal-check,api-final,door-demo,python-final,
  diff-check}.log, instances-final-run.json, instances-summary.json and
  moving-acoustic-door.wav. Initial API generation retains the expected
  missing-fingerprint failure.

Remaining boundaries:
- This is pose/enabled updates of fixed topology. It does not implement vertex
  deformation/skinning, cross-ID prototype sharing, TLAS refit/quality maintenance,
  automatic model/collider acoustic extraction or material mutation. Automatic
  geometry/result age has no deadline or extrapolation guarantee; invalid poses
  and capacity pressure remain observable.
- Doppler, diffraction/diffuse/higher-order sound, independent device clock,
  native spatial mappings and wider async/3D/VN capabilities remain open. No
  listening-quality, performance ranking, GPU/hardware-RT, cross-platform or
  whole-game result is implied by this CPU release validation.

## 2026-10-27 — Shared transport and original PCMCH (ADR 0083)

Implementation:
- Added shared AudioPlaybackInfo transport_frames, stopping and pausing. Running
  input intent excludes fully paused/freezing-delay time and effect tails while
  counting decoder/seek/effect dependency holds. Playback position and publication
  age remain separate. Unknown EOF can count its discovery frame.
- Shared AudioResumeSettings / ResumeWithOptions preserve an ongoing pause fade
  until an output-clock deadline, or retain the existing freezing policy.
  reset_gain restarts a gain fade toward the latest requested target at that
  deadline instead of multiplying an unfinished ramp. Stop deadlines are now
  independent of later pause envelopes and remain observable through short input
  EOF. Input EOF cancels pending resume even when HRTF/echo state is retained;
  a new resume on already-ended input leaves its tail alone. All processing and
  observations use the existing worker-owned DSP/publication boundary.
- Public Audio.scheduleVoiceResume retains its three-argument signature/freeze
  policy, including generated TypeScript. scheduleVoiceResumeWithOptions adds
  {policy:"freeze"|"preserve",reset_gain?:boolean}; unknown policies/options fail.
  The initial nullable fourth-argument design was replaced after checking that
  the generated TypeScript would require it and break old three-argument callers.
- Original PCMCH chains now translate PLAY/READY variants, pause/resume/delay,
  independent slot volume, source/fade/DECIDE waits and checks onto engine-owned
  prepared sources and shared channels 4+index. Global PCM remains channel 1.
  PLAY initial fades use shared gain so later volume commands replace them;
  native resume selects a fresh gain fade. There is no private native decoder,
  source buffer, envelope, sample timer or output device.
- Explicit resource aliases preserve PCM/BGM/KOE/SE selection. BGM registration
  supplies its filename only; KOE uses koe/{number} without JITAN; SE forces an
  actual one-shot/no-fade while preserving requested native persistent flags.
  Declared list count defaults to 16 and is bounded at 256, subject to shared
  voice/memory/receipt capacity. The pinned upstream Rust player allocates only
  16 PCMCH slots; honoring a larger declared list is an explicit extension.
  Native queues are bounded per slot and globally serial-sorted with other audio.
  Sparse BGM registration indices are bounded at 4096 before lookup.
- Original save codecs/layouts are unchanged. Corrected false empty-slot volume
  1000 to native 255, retained original loop/READY/delay restart predicates and
  PCM-first then independent BGM/KOE/SE restore order. Delay is only an original
  file restart predicate. Runtime payload schema advances 4 -> 5 without old
  payload migration; logical cues rebuild at source start, with READY/paused and
  remaining integer-ms delay preserved. Pause fades rebuild fully paused and
  pending stops rebuild stopped; exact sample/envelope/DSP history is not saved.
- Pinned reference blobs and inspected scope are recorded in
  docs/siglus-pcmch-source-audit.json at siglus_rs
  5f358ef1360b0439e92aed2b749e8f25d8721954. No original SFX/player/path/decoder
  implementation was copied into this bridge. Default native PCM routing is
  mapped; nondefault system/category/character/dual-BGM duck flags fail visibly.

Compilation:
- Final release workspace/all-target check passed with desktop krkr, arc-formats,
  acoustics-obvhs and optional rodio-output. Minimal no-default desktop
  reallive+avg32 check passed. No CI/workflow/remote build was added.
- Regenerated reference/JSON/TypeScript and logged fingerprint
  **e73bda1334c12a7f -> 0d9cec36638123d1**. Final API reference/documentation and
  typed three-argument compatibility checks passed. New Rust request/inspection
  members require exhaustive matches/struct literals to be updated.
- Initial compilation corrected a missing async-control match arm and a type-hint
  name; initial API generation retained its expected missing-fingerprint guard
  failure before the real fingerprint was logged. Existing CBG/RealLive/minimal
  CLI warnings remain. All cargo validation used --release.

Behavior validation:
- Shared audio library/integration scopes: **111 passed**; Runtime/Siglus scopes:
  **110 passed**; engine/public audio/frame/async/API and JS scopes: **53 passed**.
  **274 distinct Rust tests passed**, four ignored excluded. The later bounded
  BGM lookup scope supersedes the same earlier VM library cases; earlier/focused
  reruns and API generation guard failures are not counted again. Python 3.11
  tooling: **189 passed**.
- Sample oracles cover preserved versus frozen delay, pause fades before/after
  resume deadlines, running clocks through starvation/holds, latest gain targets,
  short-source EOF with retained stop completion, pause-versus-stop deadlines,
  scope and worker publication. An actual delayed HRTF tail keeps its full PCM
  after input EOF cancels resume; a new source resume does not freeze/restart it.
- Authored original Scene.pck cases verify independent global/PCMCH mixing,
  per-slot gain, declared channel 255, integer-ms source completion, named source
  priority, BGM filename-only mapping, KOE/SE policy, READY and delayed resume,
  source/fade/key return values, STOP/EOF source release and failed replacement
  retaining the previous voice/original PC before successful resource repair.
  Resource worker gates leave original commands uncommitted until publication.
- Both save contracts are exercised: READY/paused/looped framework cues rebuild,
  remaining 80 ms resume delay reconstructs; original load does not reschedule
  saved delay or restart ordinary one-shots. A stopped READY flag can restart a
  paused cue on original load while a stopped framework snapshot stays stopped.
  Raw original PCMCH records assert actual 255 and unchanged 37-byte field order.
  Unknown routing/opcodes, missing aliases and sparse BGM overflow fail visibly.
- The independent transport_timeline release example exports authored stereo
  PCM16 **8 kHz / 2400 frames** and checks every float sample without a VM/device.
  Python WAVE inspection confirms initial RMS 0.25, paused-delay silence and
  post-EOF silence. Final sample/DSP checks include the input-EOF/HRTF and sparse
  lookup follow-ups, with final workspace/minimal compilation afterward.
- Evidence: target/verification/pcmch-final-run.json, pcmch-summary.json,
  pcmch-{audio-eof-final,bounds-final,completion-check,completion-minimal}.log,
  pcmch-demo-inspection.json and transport-timeline.wav. No game, font, measured
  HRIR/SOFA data, credential or proprietary SDK was added.

Remaining boundaries:
- Full native system/category/character/dual-BGM duck routing, PCMEVENT and native
  pack/path/append-directory discovery remain open. Frame-phase clocks are not
  exact original wall-clock/device timing, and framework cues do not serialize
  exact decoder/DSP history. This does not establish a whole Siglus game or all
  original save variants.
- Doppler/diffraction/higher-order sound, independent device clock, native spatial
  mappings and broader engine-wide async/3D/VN requirements remain active work.
  No listening-quality, performance ranking, GPU/hardware-RT, other-platform or
  complete-engine result is claimed by this local CPU validation.

## 2026-10-27 — Direct Doppler and motion capture (ADR 0084)

Implementation:
- Added engine-owned radial Doppler calculation, opt-in Spatial.doppler and
  optional source/listener world velocity. The bounded target composes with
  existing authored rate/semitone pitch through an independent output-frame ramp;
  it does not overwrite user controls, restart input or reset downstream DSP.
  Paused/starved/held input does not consume PCM while interpolation can settle.
- New DopplerSettings controls factor, medium speed, symmetric target ratio and
  transition time. Finite vector/range validation, bilateral 99%-of-sound-speed
  radial bounds and target-ratio bounds have separate diagnostic flags. Existing
  non-opted-in playback retains unity; disabled observations skip the calculation.
- Shared MotionTracker infers finite differences from caller dt, not wall time.
  Explicit velocities override it. First observations, gaps, teleports and changed
  zero-time poses reset inference; unchanged zero-time poses retain velocity.
  App captures Entity global parent motion and camera/manual listener motion once
  after world/late updates. Full client/slot/generation keys prevent stale reuse;
  ended/disabled histories are pruned. Restore clears derived history while
  retaining client motion settings as host policy.
- Added Audio.dopplerVoice, motionSettings, setMotionSettings and resetMotion;
  setListener accepts optional velocity. Desired target, installed ratio/remaining
  ramp, effective rate and capture/reset reasons are inspectable. target_pending
  distinguishes a desired change awaiting render-block setup. Mixer ownership
  and read observations retain the existing final DSP worker publication gate;
  motion controls cannot mutate during a pending frame. No new task pool or
  callback lock is added; small bounded pose capture remains owner control work.
- Reviewed oddio's complete pinned spatial/speed/public modules at registry
  version 0.7.4 / Git 85ad0723b460bc8c641198ef02bf114ef04f04f9, including its
  propagation/scene ownership. The standard radial formula and velocity/unit
  conventions were checked against OpenAL 1.1 section 3.5.2 and X3DAudio docs.
  Exact archive/source/PDF hashes and the selection rationale are recorded in
  docs/doppler-source-audit.json. No oddio implementation, new dependency, SDK,
  native opcode or legacy audio host is added. Native two-dimensional sound
  does not opt into Doppler automatically; save schemas/layouts are unchanged.

Compilation:
- Release workspace/all-target check passed with desktop krkr, arc-formats,
  acoustics-obvhs and optional rodio-output. Minimal no-default desktop
  reallive+avg32 check passed. All cargo validation used --release; no workflow,
  CI or remote build was added. Existing CBG/RealLive/minimal CLI warnings remain.
- Regenerated reference/JSON/TypeScript and logged API fingerprint
  **0d9cec36638123d1 -> 768ebdf0e5892411**. API reference/command coverage checks
  passed. Added explicit public data descriptions/defaults for Spatial,
  VoiceSpec, Doppler and capture state. Old JSON remains default-disabled;
  fully specified Rust Listener/Spatial literals need their new fields/defaults.
- Initial compilation corrected the context-client accessor. Initial API
  generation retained its expected missing-fingerprint guard failure before the
  actual fingerprint was logged. These attempts are not behavior passes.

Behavior validation:
- Shared audio library/integration scopes: **117 passed**; engine API/motion and
  real JS/async service scopes: **29 passed**; existing mixer/frame/command and
  native Siglus audio/PCMCH regression scopes: **56 passed**. **202 distinct Rust
  tests passed**, three ignored excluded. Earlier/focused reruns are not counted
  again. Python 3.11 tooling: **189 passed**.
- Independent analytic cases cover approaching/receding source and listener,
  equal/transverse motion, coincidence, factor zero, extreme/clamped velocities
  and invalid input. Actual PCM and decoded-source positions verify resampling,
  composition with authored pitch/rate and chunk-invariant interpolation. Paused
  input keeps its position while Doppler settles. Listener scope and invalid
  update atomicity are checked.
- An actual HRIR case verifies Doppler resamples input before HRTF: input ends
  earlier while ear delays and filter tails keep their output-frame positions.
  Existing acoustic/reflection/reverb/resource/decoder/output tests remain green.
- Public engine cases follow a parent's Entity transform, camera motion, shared
  velocity, explicit overrides, teleport/zero-time/gap resets and host policy
  across restore. Worker-capacity gates keep ratio/observations frozen and prove
  polling does not capture movement or advance clocks twice. Unknown settings
  and foreign/stale handles fail. Real JS/TS Promise playback observes actual
  engine Doppler and capture reasons through the class registry.
- The initial parent-motion oracle confused 48 kHz output frames with the
  fixture's 8 kHz decoded-source units; it was corrected to 384 source frames
  after 16 ms at unity plus 16 ms at 2x. The production behavior was unchanged.
- The independent doppler_passby release demo exports stereo PCM16 **48 kHz /
  96000 frames** from an authored 440 Hz source moving past a listener. Windowed
  positive-crossing estimates are **500 Hz approaching / 395 Hz receding**;
  these are coarse 0.2-second-window estimates, not instantaneous-frequency
  claims. Worker and synchronous exports are byte-identical. Python independently
  decodes the WAVE, checks metadata/frequencies and records its SHA-256. No VM,
  output device, game, font or measured HRIR/SOFA dataset is used by this demo.
- Evidence: target/verification/doppler-{audio-completion,api-engine-final,
  regression-final,workspace-check,minimal-check,python-final,demo,demo-sync}.log,
  doppler-summary.json, doppler-demo-inspection.json and doppler-passby*.wav.

Remaining boundaries:
- This is a direct instantaneous radial model feeding existing linear source
  resampling. It adds no retarded-time/direct propagation delay, independent
  reflected-path Doppler, wind/moving medium, diffraction, shock-wave simulation,
  predictive motion, band-limited resampling or independent device clock.
  Automatic frame-difference velocity is not an exact physics-body velocity;
  explicit input is available. Existing interpolation may settle after reset.
- Native spatial opcodes/HostServices facade and broader async/3D/VN capability
  requirements remain open. No full native game, cross-platform/hardware result,
  performance ranking, listening-quality comparison or complete-engine claim is
  established by this CPU release validation.

## 2026-10-27: Shared cue programs, PCMEVENT prerequisite (ADR 0085)

Implementation:
- Reviewed complete pinned Siglus PCMEVENT source (472 lines), plus selected
  complete state/wait/source/save functions, at commit
  5f358ef1360b0439e92aed2b749e8f25d8721954. All audited local reference blobs
  match the pin; hashes/scope are in siglus-pcmevent-source-audit.json. No original
  PCMEVENT scheduler module, player, decoder or renderer is copied/imported.
- Added engine-owned once/loop/weighted cue programs to crysirror-audio, with
  device-free Core DTOs and HostServices methods. SharedHost and the public Audio
  class use the same scoped ChannelAudio bank. Six Audio operations create,
  advance, poll, acknowledge, inspect and release scheduling state.
- Programs select game/real owner clocks, signed/exclusive interval ranges,
  optional acknowledged source duration, reset/carry timing and softened repeats.
  Default uniform rejection sampling is separate from explicit original modulo32
  RNG semantics. Native shared-RNG injection is possible at a new due selection;
  interval RNG runs only once at acknowledgement. Keys convey no file authority.
- Pending emission serials freeze scheduling time across worker resource/voice
  dependencies; repeated polling is idempotent. Stale/foreign handles/receipts
  fail. Zero/negative intervals have a visible 1024-emission budget per explicit
  owner advance. Bank/program/key/weight/time bounds are documented in ADR 0085.
- Restore/clear invalidates scheduling state while retaining generation counters.
  Final DSP controls are fenced; immutable info stays readable. Scheduling release
  leaves independently published voices alone. App does not implicitly advance
  caller-owned sequence clocks, and working cursors are not implicitly saved.

Compilation and reference validation:
- Release workspace/all-targets check passed with desktop krkr, arc-formats,
  acoustics-obvhs and audio rodio-output enabled. Minimal desktop no-default
  reallive+avg32 release check also passed. Existing image/RealLive/CLI warnings
  remain. No GitHub workflow or remote build automation was added or enabled.
- Generated reference/JSON/TypeScript and logged actual API fingerprint
  **768ebdf0e5892411 -> 2077f8fd45fb4b36**. Reference and command coverage checks
  passed. Initial generation attempts hit the expected changed-reference and
  missing-fingerprint guards; those attempts are not counted as passes.

Behavior validation:
- Shared audio library/integration scopes: **124 passed**, one owner-NWA-data
  test ignored and excluded. Engine cue/API/reference/frame scopes: **16 passed**.
  Real JS/TS cue Promise scope: **1 passed**. **141 distinct Rust tests passed**;
  focused/initial reruns are not counted again. Python 3.11 tooling: **189 passed**.
- Independent sequence cases verify real/game clocks, pending time freeze, exact
  original repeat-weight/RNG draw order, deterministic uniform distributions,
  reset versus overdue carry, last-interval completion, malformed-loop bounds,
  zero-weight fallback, capacity, invalid update atomicity and generation scope.
- Public engine tests prove native HostServices and scripts inspect the same bank,
  restore invalidates identities, and real DSP capacity backpressure admits only
  frozen observation without advancing sequence clocks. Real JS resolves an
  emitted key through actual shared resource and voice Promises, acknowledges
  decoded duration, and verifies releasing scheduling does not stop that voice.
- Independent authored cue_sequence demo renders actual stereo PCM at **8000 Hz /
  160 frames**: positive cue starts frame 0, negative cue starts frame 56 (7 ms),
  inputs end at frames 24/72, and scheduling finishes at 13 ms. Every PCM sample
  is checked. Sync and final-worker exports are byte-identical; Python independently
  decodes the WAVE and checks amplitudes, timing and SHA-256. No VM, game, font,
  hardware output device or proprietary data is used by this demo.
- Evidence: target/verification/cue-sequence-{audio-regression,engine-final,js,
  workspace-check,minimal-check,api-final,python,demo-worker,demo-sync}.log,
  cue-sequence-{summary,demo-inspection}.json and cue-sequence-{worker,sync}.wav.

Remaining boundaries:
- **Native Siglus PCMEVENT is not yet mapped.** Dispatch, script-global RNG
  reconciliation, native channel/source/routing policy, waits/key behavior and
  original/runtime save reconstruction must translate onto this shared bank.
  Existing unsupported dispatch still fails visibly. This phase establishes an
  independently usable engine capability, not a running native game result.
- Program clocks/acknowledged durations are caller-supplied, not automatic sample
  timestamps. Scheduling is owner-frame granular; voice DSP remains sample-based.
  No hardware, cross-platform playback, performance ranking, propagation-delay or
  complete-engine claim is established. Broader 3D/VN/async work remains active.

## 2026-10-27: Original PCMEVENT over shared cue services (ADR 0086)

Implementation:
- Original Siglus START_ONESHOT/START_LOOP/START_RANDOM, STOP, CHECK, WAIT and
  WAIT_KEY now use engine-owned cue programs and existing global PCM/PCMCH
  resource/voice APIs. Native code translates arguments, channel/wait/save policy,
  derived observations and continuation intent; it has no new private scheduler,
  weighted selection, interval RNG, decoder, mixer or renderer implementation.
- Extended the pinned source audit with native clock feed and complete selected
  MATH.RAND/xorshift branches at commit 5f358ef1360b0439e92aed2b749e8f25d8721954.
  Six reference blobs match exactly; runtime/mod.rs has a pre-existing wipe_mask
  visibility export patch. Its reviewed clock feed matches the actual pinned blob.
  The final audit assertion caught this difference; the manifest records it, without
  claiming whole-file equality. No original PCMEVENT/player module is copied/imported.
  Shared Core admission validates event declarations and cursors.
- Added public Audio.captureCueSequence/restoreCueSequence and typed HostServices
  equivalents. Portable cursors preserve clock/line/RNG/emission/frame-budget data,
  reject outstanding dependencies and invalid input, and restore to a fresh scoped
  identity. These operations work independently of any VM and reconstruct no voice.
- Native script-global RNG is injected before selection and published after exact
  resource/voice acknowledgement, including an immediate final interval. Native
  math and event choice share the same stream. START/STOP release the old engine
  program in the command's commit batch, instead of leaving it alive another frame.
- Runtime payload schema **6** stores pre-selection cursors and native frame pass
  continuation, never worker/engine handles. Async resume does not tick input/VM
  time or draw twice. A clock stamp avoids charging dependency time on the next
  frame. Active events keep their owner service ticking after script EOF, without
  replaying the script. Original file codecs remain unchanged: LOOP/RANDOM restart;
  ONESHOT is discarded. Voices retain the existing reprepare/restart save policy.

Compilation and reference validation:
- Release workspace/all-targets check passed with desktop krkr, arc-formats,
  acoustics-obvhs and audio rodio-output. Minimal no-default desktop reallive+avg32
  check also passed. Existing image/RealLive/CLI warnings remain. No CI/workflow
  or remote build automation was added or enabled.
- Regenerated API reference/JSON/TypeScript and logged actual fingerprint
  **2077f8fd45fb4b36 -> 1c682b7ad1fb38cd**. API/reference coverage passed.
  Initial generation retained the expected drift/missing-fingerprint guard failures;
  those attempts are not counted as passes.

Behavior validation:
- Shared audio scopes: **126 passed**; original VM unit/save scopes: **16 passed**;
  native audio/PCMCH/PCMEVENT scopes: **55 passed** (including 11 PCMEVENT cases);
  engine cursor/frame/reference scopes: **17 passed**; actual JS/TS Promise scopes:
  **14 passed**. **228 distinct Rust tests passed**. Two ignored tests (owner NWA
  data and an explicitly supplied system font) are excluded; earlier/focused
  reruns are not counted again. Python 3.11 tooling: **189 passed**.
- Shared cases test portable cursor roundtrips, fresh identity, malformed staging,
  deterministic RNG continuation and preservation of a spent frame budget. Public
  engine/JS cases use actual source/voice Promises and keep voice lifetime distinct
  from scheduler restore/release. Read-only observations remain frozen under DSP
  capacity backpressure.
- Authored original Scene.pck cases inspect shared sequence identities, actual
  decoded PCM, global/slot routing, interval-vs-source completion, weighted line
  order, MATH.RAND interleaving, STOP with/without stopping PCM, and natural/key
  waits. WAIT_KEY leaves the event running, including a later cue after script
  EOF; loop events continue while the halted script's stack remains unchanged.
- A real I/O-capacity gate lets host time advance while preparation is pending.
  Snapshot/restore reproduces one selection/acknowledgement, the same RNG and
  interval, without duplicate playback or retroactive dependency-time advancement.
  Runtime cursor restoration and original file restart/discard policies are checked
  separately. Unknown operations/parameters/layouts/routes and missing aliases fail.
- The initial native run exposed a second cleanup from an original-load marker;
  explicit import now consumes that marker once. Code review also corrected deferred
  interval catch-up and immediate-final-interval RNG publication; stronger boundary
  cases verify the resulting behavior. The failed initial run is not a pass.
- Evidence: target/verification/pcmevent-{shared-regression,vm-published,
  native-published,engine-final,js,workspace-published,minimal-published,
  python-final}.log and pcmevent-summary.json. No game, font or proprietary data is
  committed; no audio device or hardware result is claimed.

Remaining boundaries:
- This is the explicit resource/default PCMCH routing profile, bounded to 128
  native event declarations and existing shared limits. Native START uses real
  time; saved game-clock events use the current unscaled native game clock.
  Skip acceleration, broader SCRIPT/SYSCOM routing and resource discovery remain.
- New input edges while native audio admission is pending are not yet buffered.
  Exact ordering of native frame-action callbacks versus this event pass also needs
  further work; these straight-line fixtures do not establish callback compatibility.
- Original MWND/layout/save coverage, sample-exact voice/DSP reconstruction,
  whole-game execution, cross-platform/hardware playback and broader 3D/async
  engine requirements remain open. The ongoing engine goal is not complete.


## 2026-10-27: Shared logical input journals and native continuations (ADR 0087)

Implementation:
- Core owns bounded input normalization, immutable published prefixes, scoped
  receipts, atomic admission and portable validation. Scene supplies shared
  serde transport. Optional public Input journals and Siglus use the same kernel.
  JSON u64 masks/clocks/receipts use exact decimal strings; binary saves retain u64.
- Siglus captures input before audio admission and resumes an interrupted script
  before consuming the next queued intent. Runtime payload 7 preserves backlog
  and phase; original files keep their codecs and discard pre-load clicks.
  The adapter only maps native VK/DECIDE/wait semantics, with no private queue.
- Source audit rechecked three original blobs against siglus_rs commit
  5f358ef1360b0439e92aed2b749e8f25d8721954; hashes and reference bytes match.

Compilation and reference validation:
- Release workspace/all-target check passed with krkr, arc-formats,
  acoustics-obvhs and rodio-output; minimal desktop reallive+avg32 also passed.
  Existing warnings remain. No GitHub CI/workflow was added or run.
- Regenerated API reference/JSON/TypeScript; actual fingerprint changed
  1c682b7ad1fb38cd -> 4b42007e15453574 and is logged. Final coverage/docs passed.

Behavior validation:
- Release scopes: Core input 5, Scene 22, original VM 16, native audio/PCMCH/
  PCMEVENT 58, engine input/frame/reference 23, actual JS async services 15:
  **139 distinct Rust tests passed**; one external-font test ignored, excluded.
  Python 3.11 tooling: **189 passed**. Repeated focused runs are not added again.
- Cases cover capacity/owner/epoch errors, immutable motion leases, exact high
  mask bits in actual JS, multi-client atomic ingress and restore invalidation.
  Native worker gates retain two clicks across preparation and snapshots, route
  down/up after event readiness, and discard pre-load clicks on original import.
- An initial command was rejected for duplicate --lib flags; corrected scopes
  ran successfully. Expected generation guards caught drift/missing fingerprint.
  The full JS run exposed an old zero-time assumption at Promise completion;
  single/full diagnostics passed but did not erase that failure. The test now
  checks stable source/pause/stop state and transport <= age, allowing a mixer
  publication before Promise polling. The corrected final suite passed.
- Evidence: target/verification/input-inbox-{core-tests,vm-scene,native-final,
  engine-final,js-published,workspace,minimal,python}.log and input-inbox-summary.json.

Remaining boundaries:
- Framed digital/pointer data only; no raw OS/IME/analog/sub-frame capture or
  hardware restoration. Public journals are opt-in; native default capacity256
  fails visibly on overflow. Other native adapters have not been migrated.
- Original frame-action versus audio-event ordering remains open. No complete
  game, hardware or whole-engine result is claimed. The broader goal remains.

## 2026-10-27 — KAG3 semantic adapter and shared prerequisites (ADR 0088)

Implementation:
- Original pinned TJS2/KAG parser crates remain byte-identical. New native
  KAGParser, AsyncTrigger, sound and saveStruct facades call shared CrySirror
  services; no upstream general engine/renderer/mixer/font/media implementation
  was copied or hosted. Stock KAG3 scripts are external runtime inputs only.
- Added reusable Core DeferredDispatch/RegionMap, shared simple scalar glyph
  placement (public Text.positionSimple), client-owned window policy/fullscreen,
  decoder opacity metadata and bounded worker-backed atomic capability writes.
  Native bold/unblurred shadow text uses retained GlyphRun. Whole-destination
  atlas cells use existing Canvas snapshots. Native region hits use Core's map.
- Native owner phases retain a fence/cursor across shared audio/write readiness;
  input ingress uses the Core journal before dependency checks. Native positive
  onHitTest callbacks preserve the super-call work flag, and disabled hits block
  fallthrough. Native reparenting rejects cycles/foreign windows. Audio source/
  voice ownership stays in HostServices, including cleanup after custom finalize.
- Source review pins, actual blob hashes and review scopes are recorded in
  docs/kag3-source-audit.json. Newly authored native code remains in the optional
  AGPL boundary; no game data, stock scripts, fonts or screenshots are committed.

Compilation and reference validation:
- Release workspace/all-target check passed with krkr, arc-formats,
  acoustics-obvhs and rodio-output. Minimal desktop reallive+avg32 check passed.
  Release desktop build with krkr, arc-formats and acoustics-obvhs passed.
  Existing warnings remain; no GitHub CI/workflow was added or run.
- Regenerated API reference/JSON/TypeScript and logged actual fingerprint
  4b42007e15453574 -> be619476a3bcddb4. Final docs and command coverage passed.

Behavior validation:
- **34 distinct release Rust tests passed**: Core dispatch/regions 4, VFS writes
  2, engine prerequisites/docs/coverage 10, native unit 1, native bridge 16 and
  pinned stock KAG3 scenario 1. External KAG3/font tests were explicitly executed, not counted
  as passes merely because they were ignored. No ignored cases in these runs.
  Python 3.11 tooling: **189 passed**. Repeated probes are excluded from totals.
- Unmodified KAG3 system scripts rendered authored text, held [l] without input,
  accepted one Enter, held [p], accepted one mouse pulse and reached [s]. Both
  scenario continuation markers were checked. Generated wait-glyph sheets match
  the original 384x24 atlas layout. Self-authored tests additionally cover native
  macros/conditions/script/call-return, live macro tables, label save dictionaries,
  deferred cancellation/fences, hit filters, atomic UTF-16 saveStruct, queued input
  during audio admission, nonzero offline mixed PCM and custom-finalizer cleanup.
- Actual offscreen wgpu raster capture on **Apple M5 / Metal**, 640x480, after
  300 owner ticks displayed stock menus, Latin/Japanese text and an animated wait
  glyph through CrySirror's renderer. Diagnostics: 151 engine API calls,
  owns_framebuffer=false, no native error, waiting at [l]. PNG inspected visually.
  This is one local raster frame, not a frame-rate, hardware RT or other-platform
  result. No live native audio-device or OS window-policy behavior is claimed.
- Earlier stock probes exposed missing gamma identity, ancestor visibility,
  atlas-copy admission, temporary cursor hiding and onHitTest routing. Those
  failures led to the mapped behavior above; they were not reported as boot
  success. Initial Rust borrow/Result issues and expected API generation guards
  were corrected before final checks. Default Python lacked tomllib and failed;
  the explicit Python 3.11 full run passed.
- Evidence (ignored local files): target/verification/kag3-{core-final,vfs-final,
  engine-final,native-regression,workspace-final,minimal-final,desktop-build,
  python-311-final,gpu-render}.log, kag3-summary.json, kag3-gpu-summary.json and
  kag3-demo/kag3.png. The external-source captures and system font stay local.

Remaining boundaries:
- Bounded KAG3 integration, not arbitrary commercial-game compatibility. Full
  native save/call/macro stacks, TJS runtime snapshots, XP3 auto-path mounting,
  mutable macro-parameter identity, native registered-object window shutdown,
  font-face selection, nonuniform pixel masks, focus/modal input, video,
  transitions and nonidentity gamma/gray/blur/
  flip or unsupported blend/partial-replace operations remain open and fail
  visibly. VideoOverlay currently admits only unloaded class construction.
- Native audio currently uses the shared Sfx bus; music/voice bus inference,
  live seek and delayed fades remain open. Nested Rust parser/input callbacks
  cannot suspend; async admission there fails. Source/metadata reads still have
  synchronous boundaries. ADR 0050 and the wider engine work remain unfinished.

## 2026-10-27 — shared Yamaha sources and worker image capabilities (ADR 0089–0090)

Implementation:
- Extracted the pinned UK2 pure YM2608 DSP and PMD score driver into shared
  crysirror-audio, retaining MPL provenance; no UK2 VM, Kira output or second
  mixer/renderer was imported. Existing Audio resources, decoder workers, buses
  and worker final mixing consume .opna register scores and bounded PMD-M
  .pmd/.mmm streams independently of any VM. Unsupported chip/driver modes,
  malformed directory/voice reads and zero-time loops fail visibly. Exact rational
  chip clocks, partition-invariant reads and replay seek bounds are explicit.
- Shared crysirror-image owns gamma/gray/invert/flips/noise, premultiplied Gaussian
  blur, tiny-skia vector paths/strokes, inverse projective sampling, mask/blur
  transitions and preserved-edge sine row copying. Core immutable image leases
  and owner-polled completion contain no device handles. Runtime reserves the
  existing CPU pool and bounded processing memory before accepting work.
- Public Bitmap begin/status/cancel and JS awaitable effect/transition/row-copy
  APIs use immutable source snapshots and client/epoch/target-revision checks.
  Native KiriKiri gamma/gray/flips and a real layerExRaster.dll copyRaster facade
  call that same shared service and resume the original TJS continuation only
  after publication. LR/UD flips also reflect shared Core province regions.
  Loaded/processed/uniform full-clip images are admitted; arbitrary composed
  Canvas/text readback, partial clip and native overlapping self-copy fail.
- Added self-authored examples/shared-effects and audio chiptune examples.
  Sources/pins/hashes and exclusions are in docs/synth-image-source-audit.json.
  No fonts, ROMs, sample banks, game files, captures or compiled outputs added.

Compilation:
- Release desktop build with krkr,arc-formats,acoustics-obvhs passed.
- Release workspace --all-targets check with desktop krkr/arc-formats/
  acoustics-obvhs and audio rodio-output passed. Existing CBG/RealLive warnings
  remain. An initial command incorrectly named the acoustics feature on audio;
  feature resolution failed before compilation, then the corrected command passed.

Behavior validation completed:
- Release synth integration: 4 passed; shared image-effect pixels: 7 passed;
  Core regions: 3 passed; public shared-effect/audio ownership tests: 3 passed.
  Earlier library regressions: audio 15 passed, 1 ignored; image 63 passed.
  The ignored environment-dependent audio test is not counted as a pass.
- Release native bridge with an explicitly supplied system font: 18 passed,
  including shared worker/TJS dependency gating, image pixels, province reflection
  and Plugins.link/copyRaster behavior. No native engine framebuffer is owned.
- Release JS/TypeScript Bitmap regressions: 2 passed, including awaited shared
  effects and transitions followed by ordinary retained-layer presentation.
- Release generated API docs: 7 passed; command coverage: 1 passed. Fingerprint
  be619476a3bcddb4 -> 4fa7e1d7f355d61a regenerated and logged.
- Explicit external stock KAG3 regression: 1 passed with the pinned checkout and
  supplied system font; the originally ignored test was actually executed.
- Python 3.11 tooling suite: 189 passed. Default Python lacked tomllib during
  source-audit generation; that failed invocation was rerun with Python 3.11.
- Actual public-API showcase captured at 780x520 on Apple M5 / Metal, raster
  renderer. All eight panels and labels visually inspected: original, gamma,
  Gaussian, vector, perspective, row distortion, mask and blur transition. This
  is one local raster capture, not GPU effect timing, hardware RT or cross-platform
  evidence. Self-authored WAV contains 96000 stereo PCM16 frames at 48000 Hz;
  worker mixing asserted finite/nonzero output with peak 0.3520. No live device
  or physical chip parity is claimed.
- Initial DSP edition/borrow issues and the expected generated-API fingerprint
  guard were corrected before final validation. Evidence stays ignored under
  target/verification/synth-effects-*.log, shared-effects.png and synth-demo/.

Remaining boundaries:
- This adds reusable capabilities, not a complete UK2 VM or all PMD/chip dialects.
  ADPCM/ROM banks, FM3 special mode and unsupported driver modes fail explicitly.
  This is a sound model with procedural rhythm, not silicon-accurate emulation.
- ADR 0088's former nonidentity gamma/gray/flip gaps are superseded only for the
  image-resource profile above. Full native save/media/blend/clip/mask semantics,
  general Canvas readback, GDI/metafiles and arbitrary plugin DLL compatibility
  remain open. Engine mask/blur transitions do not claim KaichoTrans equivalence.
- Processing is CPU-worker based. GPU shader variants, metadata/discovery async,
  every other ADR 0050 boundary and broader 3D engine completion remain open.

## 2026-10-28 — local game-kr probe and shared HXV4/document/vector capabilities

Implementation:
- Inspected the owner's local game-kr package: 39 DLL files and one 1.6 GB XP3.
  The inventory is not an executed dependency list. docs/krkr-plugin-audit.md
  distinguishes available reference modules from actually mapped capabilities.
- Extracted only the pinned xp3-brute pure HXV4 Special/authentication/object/hash,
  static PE bootstrap/KDF and reconstructed content-filter algorithms into the
  existing shared XP3 resource plugin. MIT notices and complete source hashes are
  retained. No CLI, native Win32 host, Unicorn, GPU solver or game runtime is
  imported. Companion bytes come through the caller's existing VFS authority.
- Authenticated requested-name lookup survives both ordinary and worker archive
  mounts. Protected streams are incrementally verified with a 64 KiB buffer;
  random-access reads apply the same native filter state. Plaintext bootstrap
  entries with a protected flag retain their checksum-verified bytes. The
  protected-warning pseudo entry is recognized by its actual prefix, not merely
  by a fake ID. Optional PE RCDATA startup bytes are a separately labelled derived
  resource, never a replacement for an archive's own startup.
- Core adds optional read streams, container-origin metadata queries and direct
  provider directory listings. Runtime supplies scoped asset/save/cache streams;
  unsupported stream providers reject before reading. Shared archive directory
  indices have a 16 MiB admission bound, avoiding repeated full-pack scans.
  Native current-directory/auto-path rules preserve native archive URIs and only
  refer to already installed client containers. Unknown media, unmounted archives,
  traversal and foreign native roots fail. No OS working directory is changed.
- A parallel agent implemented shared bounded document values/parser/writer and
  a real json.dll Scripts facade; saves retain original TJS until atomic worker
  completion. It also added shared vector geometry/Append matrices/bounds and a
  solid-linear LayerExDraw facade over existing shared image workers. Original
  superclass/function identity and RectF ObjThis behavior were preserved after
  tests exposed incorrect initial mappings. Pen/gradient/font/metafile and full
  native GDI edge equivalence remain unsupported.
- Added the self-authored examples/krkr-vector scene and tools/probe_krkr.py.
  The latter preserves project containment and removes its temporary project on
  native failure; a process exit or blank frame is never classified as a running
  game. Original user game files, fonts, keys, executables and DLLs are not shipped.
  ADR 0091–0093 and their source audits record the exact profiles.

Compilation and tooling:
- Release workspace --all-targets check with desktop krkr/arc-formats/
  acoustics-obvhs and audio rodio-output passed; existing CBG/RealLive warnings
  remain. Release desktop builds passed for the actual probes below.
- Generated API docs/command coverage: 7 + 1 passed. Explicit regeneration left
  the reflected registry fingerprint unchanged at 4fa7e1d7f355d61a; Rust-only
  contracts/native extensions are recorded under that fingerprint in CHANGES.md.
- Final Python 3.11 suite: 191 passed, including probe failure/cleanup and companion
  scope tests. No GitHub CI, workflows or remote automation added.

Behavior validation:
- Shared Core library: 20 passed; resource library: 3 passed; resource contracts:
  10 passed; formats library: 14 passed; ordinary archive tests: 7 passed.
  HXV4's 5 dedicated primitive tests cover the synthetic KDF vector, malformed
  objects, AEAD tampering, bounded inflation and partition-invariant filtering.
- Image library: 66 passed; image effects: 7 passed. Runtime library: 55 passed,
  1 environment-dependent test ignored (not counted as a pass). Independent
  vector geometry/pixels and document tests do not load any native VM.
- Final native fixtures with an explicitly supplied system font and pinned KAG3:
  LayerExDraw 3 passed; engine bridge 20 passed; JSON 2 passed; stock KAG3 1 passed.
  Includes ordered brushes/transforms/bounds, real override rejection, original
  caller suspension, atomic JSON save/load and scoped file/archive directory rules.
- Public asynchronous archive -> mounted hash alias -> shared PNG/image product:
  1 passed, exercising the SharedArchive wrapper, container origin and worker
  resource opening without a native VM.
- Explicit external HXV4 test was actually invoked: 1 passed. The local archive
  yielded 10616 actual resources (warning pseudo entry removed), plus the optional
  derived PE startup resource. Original startup is 316 bytes. Thirty-two protected
  sample entries passed checksums; the largest 218196292-byte entry also verified
  and supported nonzero reads. At that observation the tracked decoded-resource
  budget was 316 bytes; this is not a measurement of total process peak memory.
- Actual self-authored native-vector capture: Apple M5 / Metal, 640x420 raster.
  Brush offsets/order, polygon, even-odd hole and affine view were visually
  inspected. Native diagnostics: 9 engine API calls, 3 layers, no error,
  plugin_shells=[], owns_framebuffer=false, native-vector-demo-ready. This is a
  plugin/shared-API fixture, not a game-kr frame, Windows GDI pixel oracle,
  hardware RT, cross-platform or live audio-device result.

Actual game startup and remaining blocker:
- Initial actual resource open rejected unknown Hxv4; this was fixed in the shared
  parser. Opening the packed directory directly initially lacked startup because
  no archive was mounted. An early external symlink project was correctly rejected
  by containment; the reproducible probe keeps that check enabled instead.
- Executing the archive's ordinary startup produced its own corruption-sentinel
  message and zero game layers. The executable is not thereby declared corrupt,
  and that blank renderer completion is not a successful game boot.
- The correct, unchanged PE RCDATA startup now executes from its explicit derived
  resource, passes the real native archive-directory request, then fails at
  Plugins.linkZ (bytecode 43). No game scene/title/gameplay frame is established.
- linkZ is not an alias for ordinary link. Its bootstrap registers signature and
  CompoundStorage/media state beyond the already validated HXV4 reader. The
  separate pinned research record docs/krkr-bootstrap-gap.md identifies PSS-SHA256,
  bootstrap return-octet and CompoundStorage requirements, and explicit ABI/oracle
  gaps. No signature success, returned key/octet, media/domain state or native DLL
  execution is fabricated. Original executable identity/bres routing also remains
  part of that future native bootstrap profile. The 39 DLLs are not all implemented.

Corrections and evidence:
- Initial integration tests exposed advisory protected flags/ordinary startup,
  incorrect Special startup-by-basename binding, a local helper-name shadow,
  ambiguous map typing and test-fixture setup/borrowing issues; they were corrected
  before the final passing scopes. Early LayerExDraw checks also exposed inherited
  update lookup and RectF ObjThis errors; both were fixed and revalidated. Final
  review also moved built-in update identity capture before all scripts, with
  a passing native-method override-before-link regression. An
  initial repeated --lib CLI argument and Python macOS path-alias assertion failed;
  corrected invocations passed. Failures are not counted as successful validation.
- Local evidence is ignored under target/verification/game-kr/: hxv4-*-final.log,
  hxv4-stream-final.log, shared-resource-tests.log, image-runtime-tests.log,
  native-last-tests.log, async-hashed-test.log, final-{workspace-check,
  python-tests,api-tests}.log, desktop-last-build.log, embedded-container/
  summary.json and vector-demo/{final.png,final-render.log}. Source snapshots remain
  ignored under target/research/. The broader engine and ADR 0050 work remain open.

## 2026-10-28 — game-kr protected bootstrap runs through shared services

Implementation:
- `Plugins.linkZ` now installs the protected-bootstrap facade only when the
  storage equals the bootstrap module named by the engine-published profile
  (`hxv4_bootstrap_profile`, derived by the HXV4 resource plugin: prefix, unique
  key, media name, PUBKEY PEM, context token — never the archive key). Member
  surface taken from the bootstrap module's own string table: System.bootStrap,
  System.checkSignature, Storages.CompoundStorageMedia, parseArchiveIndex,
  archiveUniqueKey, isExistentStorageNoSearchNoNormalize, and the archive
  addAutoPath/removeAutoPath hook. Unmapped media members fail visibly.
- New shared `crysirror_resource::compound` (mount/domain/route state, pathHash/
  fileHash) and `crysirror_resource::text_stream` (KiriKiri FE FE mode 0/1/2,
  krkrz TextStream.cpp fd5c4ba). Signature checks use the existing shared
  RSA-PSS verifier. `krmovie.dll` is an engine facade (no TJS exports).
- Fixed shared pathHash: little-endian bytes over the exact spelling, directories
  with trailing `/`, root `""` (root = 94d4a97c61498621 as documented). The
  pinned xp3-brute spelling (big-endian, slashless) matched no directory record,
  so every hashed lookup previously missed. Hashed names now resolve only under
  an explicit `hxv4_hashed_root`, so the plain sentinel startup cannot shadow them.

Compilation/tests: `cargo test --release -p crysirror-resource -p
crysirror-formats-arc -p crysirror-krkr` all pass; the two ignored real-game
HXV4 tests pass with CRYSIRROR_HXV4_GAME. Desktop built with `krkr,arc-formats`.

Behavior (CPU probe, tools/probe_krkr.py, log under
target/verification/game-kr/linkz/): bootStrap → CompoundStorage mount of
data.xp3 → checkSignature(exe) true → krmovie link → real arc://./startup.tjs →
Storages.tjs (ciphered text) → KAG system scripts. Now stops at
`Plugins.link("layerExImage.dll")`. No rendered frame; whole game not established.
Not mapped: the bootstrap's Plugins.link signature hook, setFile/getFile/getHashes,
custom archive seeds; checkSignature reads synchronously (ADR 0050 split open).

### 2026-10-28 (later) — next plugins on game-kr's real call chain

- `layerExImage.dll` (wamsoft 3aff625): light/colorize/modulate/noise/
  generateWhiteNoise/gaussianBlur onto shared `ImageEffect`s. New shared
  `crysirror_image::cximage` ports the CxImage brightness/contrast LUT, HSL
  colorize and HSL modulate exactly (notice: docs/third-party/cximage.txt).
  gaussianBlur uses the native std-dev derivation with the shared sampled
  kernel (CxImage's 50-sample integration is not reproduced).
- `KAGParserEx.dll` (wamsoft 61d5f1e): extended-format parsers with `taglist`;
  the shared parser already keeps attributes before `*` (Ex rule).
  multiLineTagEnabled=true and paramMacros fail visibly (unused by game-kr).
- Tests: crysirror-image/resource/krkr pass. Probe now stops at
  `Plugins.link("textrender.dll")` (TextRenderBase message layout).
- `textrender.dll` TextRenderBase: new shared `crysirror_text::markup`
  (VM-free typed port of krkrz_rs b4a295f text_render.rs, MIT notice in
  docs/third-party/krkrz_rs.txt) with width/font/eval/label host callbacks;
  the adapter maps TJS values and script handlers only. Vertical ruby,
  graphical characters, word_break and width_time_scale fail visibly.
- System.confirm fails visibly with its prompt (no modal dialog service).
- Probe now stops at `Plugins.link("motionplayer_nod3d.dll")` (E-mote Motion).

### 2026-10-28 (session end) — game-kr boots into KAGWindow/UI construction

Implemented this round (all tests in krkr/scene/runtime/text/image/resource pass):
- Plugins: motionplayer(_nod3d) Motion namespace (real PSB ResourceManager via
  eluna_rs; playback members fail visibly), csvParser (shared
  `crysirror_runtime::csv`, TextStream BOM/FE FE decoding), scriptsEx, windowEx and
  win32dialog vendored from Kirakira c451918 (win32dialog open/messageBox fail
  visibly), addFont, known-surface table (msdfrender, k2compat, layerStwCopy:
  members installed, each fails visibly when used).
- Core: Scripts.setCallMissing/getClassNames/loadDataPack/getTraceString(empty),
  Window.BasicDrawDevice/drawDevice/left/top/zoom/registerHotKey + empty base
  event handlers, System.desktopLeft/Top, readRegValue (empty registry),
  WaveSoundBuffer.PhaseVocoder (non-identity filters fail at play), VideoOverlay
  geometry state, Layer.children/numChildren/order setter/callOnPaint+onPaint
  dispatch, plain-state Layer properties as real native properties, pre-construct
  left/top, standard cursor constants (shape not applied to host cursor).
- Shared canvas: partial region replace, TransformCopy (rotate/scale, no skew),
  Resize; krkr copyRect/affineCopy/stretchCopy/setImageSize use them.
- Window inner size may be any uniform zoom of the canvas (probe uses 1920x1080).
- Vendored lexer patch (regexp scan stops at line breaks in skipped @if blocks),
  recorded in krkr-upstream SOURCE.json `patched` / UPSTREAM.md.
- Diagnostics: CRYSIRROR_KRKR_TRACE_NATIVES, CRYSIRROR_KRKR_EXCEPTION_{EVAL,MATCH,ALL,REGS},
  CRYSIRROR_KRKR_TRACE_STORAGE; tests/decompile_tool.rs; hxv4 dump_named test.

Current stop: `Layer.convertType` missing during UI construction. Known open:
per-channel draw faces (face=1/holdAlpha), fstat.dll, System.versionInformation,
Debug.addLoggingHandler, Motion playback, psbfile/.scn scenarios, video/AMV,
audio decoders. No rendered game frame yet; whole game not established.

### 2026-10-29 — game-kr renders its first real screen

Behavior: `crysirror render` (Metal, 1920x1080, 900 frames of 16 ms) shows the
game's ATTENTION screen (attention.png drawn through the game's own AffineLayer
onPaint path onto the shared canvas) plus the game's menu bar via UI.MenuSpec;
Vorbis audio decodes. KAG is click-waiting at `sysvoice` in custom.ks *caution.
Capture: target/verification/game-kr/render/attention-900.png (local, ignored).

Fixes this round:
- Vendored krkr-tjs2 patches (recorded in SOURCE.json `patched` + UPSTREAM.md):
  `&a.p incontextof t` keeps the ignore-property context (parser rewrite matching
  tjsInterCodeGen.cpp); call opcodes record the caller's `this`
  (`Runtime::native_caller_this`), used by scriptsEx foreach `functhis`.
- Shared canvas (crysirror-scene): band pruning keeps partial replaces linear
  (400-cell test), copies keep only source nodes inside the copied rect,
  destination draw clips on Copy/TransformCopy, new `Fill` with replace.
- Adapter: Layer.update raises callOnPaint (krkrz UpdateByScript), layer growth
  enlarges the image (krkrz ImageLayerSizeChanged), setClip/clip* properties,
  partial translucent fillRect, convertType (alpha/add-alpha identity in the
  adapter's straight-alpha representation; to-opaque fails), Layer.focused (false,
  consistent with focusedLayer null), extrans/extNagano provider links,
  KAGParser.store with native callStack/macroArgs layout (restoring active call
  frames still fails visibly).
- Tests: krkr/scene/runtime/text/image/resource all pass; tests/tjs_semantics.rs.

Open: input/click to advance past the caution screen is unverified, title screen
not yet reached; Motion playback, psbfile/.scn, video, per-channel faces, fstat,
save-path cfu write, hotkey dispatch, cursor shapes, focus.

## 2026-10-28: game-kr continuation and shared codec work (Codex)

### Implementation

- Explicit `void` native storage strings convert to the empty string, distinct
  from a missing argument. `windowEx.getCursorPos` reads the client engine input
  snapshot in virtual window coordinates (not the OS desktop cursor).
- Native relative/absolute sibling ordering now maps to engine retained order.
  ADR 0094 adds engine-owned isolated subtree crossfades, public Layers commands
  and progress, with KiriKiri exchange/completion semantics in a thin adapter.
- Shared text-file encoding and scoped VFS metadata back `Array.save2` and the
  fstat read/query profile. Saves await actual atomic-write completion; failure
  does not resume the script as success. Unsupported mutation/saveStruct2 modes
  remain explicit errors.
- Shared M2 motion resources, worker samples, validated dynamic PuppetFrames and
  immutable textures back the Motion native profile. This does not imply all
  Motion timeline, physics, masking or feedback modes are implemented.
- MP4 AVC/HEVC/AV1 software decoding is in `crysirror-video`, with pinned Rust
  OxideAV decoders and provenance in its SOURCE.json. The unused `crysirror-mp4`
  prototype was moved to ignored local research storage, removed from the
  workspace. The dependency baseline is now Rust 1.95 (tested host: 1.98.1).

### Compilation and behavior evidence recorded so far

- Release storage-plugin tests: 3 passed. Independent runtime text-encoding and
  scoped metadata tests: 1 passed each. Exact CP932/UTF-8 bytes, newline modes,
  worker failure, client isolation and absent archive timestamps are covered.
- Release video tests: 16 passed, 1 existing local-file test ignored. Six
  generated MP4 samples cover AVC Baseline/Main with B frames, HEVC Main/Main10,
  AV1 8/10 bit; decoded pixels, ordered PTS, seek, rewind, cancel and EOF are
  checked. Old MPEG/WMV/AC3 tests also ran. No FFmpeg subprocess is used for
  runtime decode. These fixtures are synthetic, not user game data.
- Earlier in this phase, the native bridge suite passed 22 tests with 1 ignored,
  and Python tooling passed 191 tests. Those results precede the new crossfade
  and Motion changes and are not reported as verification of the final state.
- Actual game-kr clicks progressed beyond ATTENTION and exposed native storage
  string, cursor, ordering and transition gaps. At this checkpoint no title
  screen has been verified. Later release/GPU/game results are appended below;
  implementation or an authored test alone is not counted as a pass.

- Subsequent native bridge release run: **23 passed, 1 ignored**, including
  actual engine-clock crossfade completion, self-bound native object identity,
  order/visibility exchange and exactly one callback.
- Runtime Media release behavior run: **13 passed, 1 ignored** (47 unrelated
  tests filtered). AVC/HEVC/AV1 frames publish through the existing bounded media
  workers; an MP4 audio track with audio enabled fails explicitly. MP4 audio
  decode, HDR tone mapping and hardware video decode are not implemented.
- Desktop release build with `krkr,arc-formats` completed. The next real game
  replay failed in a retained Canvas batch with `layer missing`; this is not a
  successful title-screen result and is being investigated independently.

- Shared subtree state tests: **3 passed**. Explicit release GPU test on
  **Apple M5 / Metal: 1 passed, 0 ignored**. Pixels verify overlapping descendants,
  gamma blend, transparent alpha, clipping, endpoints, unchanged external layers
  and stop restoration; invalid progress and oversized worksets are rejected.
  An initial cache-key review concern was withdrawn after tracing the actual
  ordinary-image path; the original namespace is correct and is GPU-tested.

- Motion release native profile test: **1 passed**; public frame-validation and
  rectangle-clipping/UV-color interpolation tests: **2 passed**. Explicit local
  logo native tests both passed (yuzulogo: 5 parts/14 immutable textures at
  500 ms; m2logo: 13 parts/38 textures). They verify engine-owned 1920x1080
  presentation, real decoded texture resources, retained resource lifetime and
  one onSync event, with `owns_framebuffer=false`. These are headless native-to-
  engine checks, not whole-game or GPU visual validation.

- Canvas lifecycle release tests: **14 passed, 0 ignored** (3 new regressions
  plus 11 existing Canvas tests). Cleanup of missing derived nodes is scoped to
  Canvas ownership; ordinary Layer Remove still rejects an absent ID.
- API reference regenerated, command coverage passed (1 test), and the release
  API documentation binary passed all 7 tests after logging fingerprint
  `cca12b45f54104e4`. Public command coverage also now records the existing
  Canvas resize/fill/transform_copy commands.

- The next actual game replay passed the earlier Canvas failure and executed to
  `custom.ks:110` (`loadfadein prepare`, after the logo sequence). It then failed
  explicitly at `Layer.piledCopy`. No title image was captured. The run exited
  itself before a proposed termination signal was sent; it was not a timeout or
  a successful render. Evidence: local `title-bringup/render6.log`.
- Sampling this replay exposed repeated rebuilding of every Canvas presentation.
  Dirty-source/root filtering now updates only affected bindings. Its release
  tests plus lifecycle/existing Canvas coverage: **17 passed, 0 ignored**.
  This is behavior verification; actual post-change game timing is pending.

- CopySubtree now freezes one `Paint::Snapshot { size, layers }` rather than
  expanding its draw tree into the destination. Scene release coverage is
  **25 passed, 0 ignored**. Explicit Snapshot GPU tests on Apple M5/Metal:
  **2 passed, 0 ignored**; overall/ancestor opacity, scaling and source mutation
  preserve the frozen product with exactly one rasterization. Immutable-image
  validation and asynchronous readiness are tested; ordinary paths, dynamic
  surfaces and uncaptured font aliases are rejected in this first profile.
- Replay 7 reached the same prepare point but rejected a type-1 source. Upstream
  inspection identified that raw-image Complete is a direct RGBA copy independent
  of type/opacity; the native fast path is being corrected. Composite opaque
  alpha is not universally 255 in upstream and is not silently normalized here.

### Final verified checkpoint for this batch

- MP4: pure Rust AVC/HEVC/AV1 video is wired through the shared decoder and Media
  workers. The old prototype is outside the source tree. MP4 audio, hardware
  decode, HDR tone mapping and exhaustive codec conformance are not claimed.
- Native piledCopy release tests: **3 passed, 0 ignored**. The raw-image branch
  preserves RGBA (including hidden RGB in the synthetic PNG) independently of
  opaque type/opacity; the composed branch uses the shared Snapshot profile.
- Explicit GPU regression across Snapshot and subtree crossfade: **3 passed,
  0 ignored, Apple M5/Metal**. Image readiness is gated deterministically; an
  unfinished dependency never becomes a cached blank product. Crossfade's
  alternating-input snapshot cache retention remains a performance follow-up.
- Final API fingerprint: `08381e9cfceae9bf`. Reference regeneration and command
  coverage ran; after the fingerprint log update, all **7 API documentation
  checks passed** from the newly built release test executable.
- Desktop build8 with `krkr,arc-formats` passed. Actual replay8 still stops at
  `custom.ks:110`: source slot3 is type1, uniform alpha0, 1920x1440, with two
  visible type2 children; destination slot214 is type1 with no children. This is
  an opaque composed-subtree path, not the raw-image fast path. Its upstream
  cache-alpha behavior is distinct from ordinary alpha composition. It remains
  an explicit error rather than being forced to A255 or silently approximated.
  **The title screen has not been reached or captured.** The next engine gap is
  a defined opaque bitmap/subtree composition and presentation policy.
- No commits or pushes were made. No CI was added. Verification used local
  release builds, synthetic redistributable fixtures, existing local game data
  and runtime system fonts.
- Final native release regression (`engine_bridge`, `motion_plugin`,
  `piled_copy`, `storage_plugins`): **30 passed, 2 ignored, 0 failed**.
  Ignored tests are not included in the pass count. Evidence:
  `target/verification/game-kr/title-bringup/native-final.log`.

### 2026-10-28 — game-kr reaches its title screen

Behavior: `crysirror render` (Metal, 1920x1080, input track clicking the caution
screen at frames 900/1200) runs 1800 and 2600 frames with exit 0 and shows the
title screen: the seven menu buttons, the 体験版 badge (PSB pimg images), title
BGM (Ogg Opus with .sli loop). Captures: target/verification/game-kr/title-bringup/
title-1800.png, title-2600.png (local, ignored). Clicking はじめから reaches the
next UI, which stops on unmapped toml.dll (uitexts.toml) and an audio open
inside a non-resumable native callback.

Fixes this round (on top of Codex's uncommitted work):
- Repaired Codex's non-compiling image_processing edit and its `image_alpha`
  present field the canvas never accepted.
- Layer.piledCopy of a composed ltOpaque source goes through the shared
  copy_subtree with straight alpha and logs the deviation (krkrz displays stored
  RGB; where the source alpha is below 255 ours shows what lies beneath).
- Window.focusedLayer writable, Layer.focus/focused, onBlur/onFocus delivery.
- Shared canvas `fill keep_alpha` (holdAlpha); fillRect dfOpaque/holdAlpha.
- VideoOverlay.stop is a no-op (open always fails visibly, nothing can play).
- crysirror-audio: Ogg Opus decoder (opus-decoder 0.1.1, MIT/Apache, no unsafe),
  `.sli` parser; AudioParameters.loop_end; WaveSoundBuffer labels/flags.
- psbfile.dll facade: PSBFile.load/root via eluna_rs; `psb://` images decoded
  by the engine registry through new HostServices::upload_encoded_image.
- CSVParser accepts a null target. API docs regenerated (224cb89811441323).
- Tests: krkr (all binaries), scene, audio, core, runtime pass; api_docs pass.
  Opus decode/seek verified on the game's BGM via an ignored local-file test.

Open: the title background motion (title_bg.mtn) publishes PuppetFrames but
only its dark base parts render (character/logo art missing: motion renderer
gaps); operateRect/colorRect with face=1 holdAlpha; toml.dll; WIN32Dialog;
opaque-layer display semantics; fstat deleteFile.

### 2026-10-28 — game-kr plays its opening scenario

Behavior: `crysirror render` with an input track (caution clicks, はじめから at
frame 2010, then a click every 150 frames) runs 3000 and 4900 frames with exit
0: start.ks loads the PSB scenario (`.scn` via PSBFile) and the opening NVL
monologue advances line by line with the click-wait icon and quick menu.
Captures: target/verification/game-kr/title-bringup/start-3000.png,
story-4900.png (local, ignored). Backgrounds/characters of these opening lines
are dark/absent in the capture; whether that is the scene or a gap is not yet
established.

Fixes this round:
- toml.dll: Scripts.tomlDecodeFromStorage over shared structured_text TOML
  (toml_edit 0.25.17).
- WaveSoundBuffer: open/play queued from input callbacks (no suspend there);
  metadata reads while such an open prepares fail visibly; `frequency` writable
  (rate applied at play; live change fails visibly).
- Vendored krkr-kag: `%arg|default` resolves inside argument-less macros (KAG3
  argument-stack depth); recorded in SOURCE.json/UPSTREAM.md.
- Canvas: `fade` (krkr negative-opacity colorRect: alpha scaled, isolated
  snapshot); copy provenance turns a krkr save/restore of a cell around each
  redraw into (region over base) instead of nesting clones (was O(frames^2),
  hit the 4096-layer limit within ~100 frames).
- Layer: affineCopy clear (neutral-colour fill of the clip area, then copy);
  drawText void colour → 0 and font members read through PropGet (script font
  properties); blurred shadows via a new shared glyph blur ported from krkrz
  fd5c4baa CharacterData::Blur / TVPChBlurCopy (crysirror_text::glyph_blur,
  Paint::GlyphRun.blur; licence docs/third-party/krkrz.txt).
- PSBFile(storage) constructor; msdfrender.dll links without its SDF members
  (scripts feature-detect them and use the atlas stretchCopy path).
- TextRender: read-only state visible to callbacks; `%n[N];` line breaks per the
  game's TagTextConverter ([xr num]) and krkrz drop-in krkrsdl3 semantics (the
  wamsoft source was not available).
- Window.focusedLayer writable earlier this round; see previous entry.

Open: title_bg motion art; operateRect/colorRect face=1 holdAlpha; WIN32Dialog;
opaque-layer display; save/load not exercised.

### 2026-10-29 — game-kr plays into the prologue with backgrounds and characters

Behavior: `crysirror render` with an input track (New Game, then a click every
90 frames) runs 12000 frames with exit 0 through scene changes: background art,
event CG with name plate and dialogue (captures title-bringup/scene-8000.png,
long-12000.png; local, ignored). Remaining caught script errors in that run:
ctxfontprefs.tjs encoding, onResize `recreate` on void. System variables save
(datasu/datasc.ksd with the game's backup rotation) in a 3000-frame run.
A windowed `run` (Metal, Retina scale 2, surface 3024x1666 BGRA sRGB, opaque
alpha) presents a full, correctly letterboxed caution screen: verified by
reading back the presented swapchain image (CRYSIRROR_SURFACE_CAPTURE).

Fixes this round:
- Transitions: `universal` rule wipes (krkrz TVPInitUnivTransBlendTable) and
  transitions without children (temporary sibling presentation + krkrz Swap);
  strictly nested crossfades composed inside-out (scene + renderer); piledCopy
  of a tree with an active transition captures its nearest endpoint (logged).
- Keep-alpha drawing: Blend atop / atop_multiply / atop_screen; canvas atop
  copies/draws frozen into region snapshots (inlined, never nested); krkr
  operateRect/colorRect holdAlpha incl. omPsMultiplicative/omPsScreen (_HDA);
  single-draw copy opacity.
- Data packs: encrypted TJS/4s0 `.pbd` (ChaCha, LZ4 with 64 KiB dictionary)
  via crysirror_resource::krkr_pbd, ported from xmoezzz/xp3-brute (MIT).
- fstat deleteFile/moveFile/copyFile/createDirectory on the save mount.
- Performance: menu specs rebuilt only on change; layer validation skips frozen
  snapshot payloads (checked on entry); snapshot payloads shared via Arc.
- Snapshots admit images served by mounted containers; plain files refused.
- Tests: scene/krkr/resource/text/audio/runtime 458 passed 0 failed; GPU tests
  (Metal, single-threaded, CRYSIRROR_TEST_FONT set) all pass; api_docs pass.

Memory (same machine, `crysirror render`, input track long.json):
- Before: peak RSS 2.27 GB and rising. Cause: every Array/Dictionary carried
  its own native method objects, every call allocated a new this-proxy, and
  the TJS heap never freed anything.
- Now: shared built-in natives, a cached this-proxy, and a mark-and-sweep GC
  for the TJS heap (krkr-upstream UPSTREAM.md item 5). The GC runs between
  adapter steps. Swept objects invalidate their native state. PSBFile,
  CSVParser and TextRender instances are no longer GC roots.
- 9000 frames: peak RSS 821 MB, exit 0.
- 30000 frames (about 410 s, around 73 frames/s headless): RSS stayed between
  443 and 620 MB and ended at 455 MB, with no steady growth.
- GC: two collections, freeing 19982 and 43262 objects; the larger took
  90 ms (a visible hitch).
- `TJS_GC_NO_REUSE` (freed slots never reused) 9000-frame run: no
  invalid-object errors.
- Profile: about half of the live heap is TJS member tables (about 2 KB per
  Array, because methods are copied per instance as in TJS2). Most come from
  PSB trees that motion/psbfile convert eagerly.
- Tests: krkr-tjs2 lib 420 passed; crysirror-krkr all suites pass;
  scene/core/runtime 246 passed, 0 failed.

Follow-up (same day), measured on the same machine:
- Throughput: EngineSystems::submit cloned and fully revalidated all engine
  state per batch (about 87% of main-thread samples). Batches now stage only
  touched systems: one LayerSystem transaction (journaled, no tree clone),
  copy-on-write canvas surfaces, validation of touched surfaces only.
  9000 frames: 150 s -> 33.6 s (exit 0, frame 9000 pixel-identical);
  12000 frames ~50 s; 30000 frames 58 s (was 410 s).
- GC pause: dead objects are dropped on a background thread and collected
  objects only release weakly held native state (`object_collected`):
  ~27 ms -> ~11 ms (mark phase).
- Script errors fixed: completion callbacks (`_stop` missing) -- the port
  bound context-free closures when stored into a member, unlike krkrz
  PropSet; `Scripts.evalStorage`/`execStorage` read `KBAD100` binary struct
  packs (ctxfontprefs.tjs) as krkrz LoadByteCode does; that table then
  reached `Layer.operateStretch`, now an engine transformed blend with
  opacity (normal alpha modes; holdAlpha/other modes fail visibly). The
  12000-frame run logs no caught script exception.
- Native menu bar (macOS, muda 0.19.3): engine menu bars are mirrored into
  the OS menu bar; selections become the same engine menu events. Verified
  only by the install log listing the game's menus; clicking has not been
  exercised (screen locked, no accessibility/screen-recording permission).
  Windows/Linux keep the engine-drawn bar. `CRYSIRROR_NATIVE_MENU=0` opts out.
- A windowed run does not advance while the window is occluded or the
  screen is locked (macOS delivers no redraws; ticks run in redraw).
- Tests: krkr-tjs2/krkr/scene/core/runtime/desktop 738 passed 0 failed;
  wgpu incl. ignored GPU tests (Metal, single-threaded) 93 passed; api_docs
  pass (fingerprint 3a2df66087609087).

Title motion and close/confirm flow (same day), measured headless:
- Title (title_bg.mtn) now renders background, logo and character art. Fixes:
  motion samples continue from the player's previous native step (type-0
  HOLD keyframes keep state; previously everything vanished at the end
  hold); eluna port changes 8-10 (sprite centre = coord - origin + size/2;
  HOLD state keyed by structural path, labels repeat; a Null priority frame
  keeps the previous order); krkr/win 32-bit motion textures are BGRA;
  E-mote MODULATE2X (bm 0x10) vertex colour gain (0x80 neutral; the logo
  screens were grey).
- WIN32Dialog.messageBox is an engine-drawn modal dialog on every platform
  (UiSystem dialogs; Win32 button sets, MB_DEFBUTTONn, Escape rules,
  IDOK..IDCONTINUE returned on resume via a new VM resume-value hook).
  chooseColor still answers cancel (stub); template dialogs (open) fail
  visibly.
- Window.close runs onCloseQuery like krkrz (program close synchronous,
  user close posted); the desktop close button / native Quit go through the
  game and the window closes when the VM halts. The title's ゲームの終了 shows
  the game's own 「ゲームを終了しますか？」; はい saves system variables
  (datavc.ksd via safe-save rename) and halts.
- Added for that flow: Layer.setMode/removeMode (modal layers, hit/focus
  limits, onNodeEnabled/Disabled, BlurTree on disable/hide), transition
  `callback` tick sources (engine external crossfade clock), universal
  vague 0, bounded nested snapshots, Layer.setCursorPos (engine pointer
  warp, applied by the desktop), Scripts.saveDataPack (TJS/ns0; the
  cryptmode/compress/iv 4s0 options are not reproduced), Storages.saveOctet,
  registerHotKey(id) unregister, void integer arguments read as 0.
- Not verified: clicking the native menu bar; windowed play (screen locked
  during this session).
- Tests: krkr-tjs2 420; scene 84; krkr/core/runtime/resource/puppet/engine/
  desktop pass (api_coverage included); wgpu incl. ignored 93 (after the
  nested-snapshot change, Metal, single-threaded); api_docs pass (4936e7b23cc5a98a).
  12000-frame playthrough: exit 0, 45 s, no caught script exceptions.

Owner play report follow-up (same day):
- System Setting opens (headless capture): a script class with a later native
  base (`extends Script, Layer`) now gets that base's members copied at
  inclusion, as krkrz tTJSNativeClass::FuncCall does (VM change);
  System.personalPath/appDataPath map into the save root; transitions may
  join endpoints under different parents (KAG fore/back pages; krkrz
  Exchange on completion); engine layer budget 4096 -> 16384 (the screen
  needs 4793).
- PackinOne data packs are the Kirakira c451918 port (reference container:
  LZ4/ChaCha, IV, digest XXH32; thumbnails not captured), replacing the
  interim ns0 writer. Saving system variables and the voice clip index works.
- The engine-drawn in-window menu bar is the default again on every platform;
  the macOS system menu bar is opt-in (CRYSIRROR_NATIVE_MENU=1).
- Tests: krkr-tjs2/krkr/scene/engine 693 passed, 0 failed.

Open: real-time frame rate not yet measured in a window; save/load slots not
exercised; title_bg motion art; WIN32Dialog; owner-reported windowed "top-left
quarter / wrong colours" not reproduced with the current build.


## 2026-10-29 — RFVP through the vendored portable core

- Implementation: `crates/rfvp-upstream` vendors xmoezzz/rfvp 062fec8
  (`crates/rfvp`, MPL-2.0, byte-identical sources; the Microsoft fonts are
  excluded; new manifest building only the `no_std` profile). The old
  hand-written `crysirror-rfvp` interpreter (`script/`, `bridge.rs`) is
  removed. `crysirror-rfvp` is now the core's host: files come from
  ClientStorage, textures are engine surfaces, each frame's sprites and solids
  are one engine dynamic puppet mesh on one engine layer (scissor via
  clip_part_rect, blend modes mapped), sound slots are channel audio
  (EncodedAudio), time is the engine clock, saves go to the save mount
  (`rfvp/save/*`), and save dates use local wall time.
- Engine additions: HostServices::release_surface; per-client surface slots
  raised from 16 to 4096 (same 256 MiB budget). The `.sli` parser moved to
  crysirror-resource, so crysirror-krkr no longer depends on crysirror-audio.
  Also fixed stale krkr-upstream SOURCE.json hashes and recorded gc.rs as an
  addition.
- Behaviour (headless `render`, owner-authorized ~/Downloads/testcase, White
  Eternity, 1024x640): title with BGM; NewGame → prologue text → CG with the
  in-game toolbar; the toolbar menu (SAVE/LOAD/…); SAVE screen; saving slot 1
  writes `rfvp_s001.bin` and the slot shows text and date. Logged
  "Invalid duration" / "history_set … Nil" are the game script passing nil
  into upstream syscalls, not adapter errors.
- Not done / not verified: save thumbnails (capture_thumbnail; the engine has
  no synchronous frame readback, same as krkr); loading a slot; movies
  (meffect/*.wmv layer effects); windowed play; streaming audio
  (create_stream/submit_* return Unsupported). The core still decodes game
  images and rasterizes glyphs (ab_glyph) privately, so it hands over finished
  RGBA; moving these into engine image/text services is open extraction work.
- Tests: crysirror-rfvp 3 unit tests; runtime/krkr/scene/engine/resource/
  audio/rfvp/krkr-tjs2 598 passed, 0 failed; policy tests (krkr + new rfvp)
  8 OK; api_docs pass (4936e7b23cc5a98a, Rust-only changes logged).

## 2026-10-29 — RealLive: AIR SE and Game_RUSUR play through the story

Owner-authorized games: `~/Downloads/AIR Standard Edition` (a RealLive title,
not AVG32: Seen.txt scene index, g00 art, RealLive.exe) and
`~/Downloads/Game_RUSUR`. Scripts: `target/verification/reallive/scripts/`.

- Copied from siglus_rs 5f358ef into the adapter VM (recorded in
  `vm/SOURCE.json`): settings/syscom, frame counters, GAN, the object model
  (params, properties, mutators, pattern/GAN animation, promotion) and
  `modules/obj.rs`, and the VM-state 1:4 system functions (arithmetic, date,
  settings 22xx/23xx/26xx, syscom states, flags, savepoints, call stack,
  `end`). The extension DLLs (EF00, PT00 Little Busters, DT00 Tomoyo After)
  are vendored as `crates/reallive-upstream/reallive-dll` and wired to
  LoadDLL/CallDLL; no authorized game uses them, so they are compiled and
  wired but not game-tested. Unknown DLLs fail visibly.
- Engine additions: RealLive g00 atlas decoding with pattern regions
  (`crysirror_image::g00`; the shared g00 decoders now return multi-pattern
  files as that atlas), `#SEL` reveal rule images
  (`crysirror_image::reallive_wipe`, used with the engine mask transition),
  canvas `filter` (region mono/invert/light/colour), snapshots of
  self-contained plain project images (content-pinned), present_surface
  release, raw NWA with an unset packed size.
- Adapter mapping onto engine services: objects as retained layers (geometry
  from upstream blit_region: origin/rep origin, scale, rotation, clip,
  alpha, add/subtract, filters; digits; child objects), haikei backgrounds
  (1:40), grpMulti, grpZoom, grpStretchBlt, freeDC, grpFill/Outline,
  region colour effects, masked copies, manual draw mode (held batches until
  refresh), window frames (WAKU types other than nine-slice: BACK tinted by
  the window attribute, NAME on top), choices laid out in the text window,
  kinsoku and page-full continuation, koePlay (loose files, or voice
  archives mounted as engine containers), SE table, BGM replacement fades,
  volumes, screen shake (whole scene under one root layer), name entry OK.
- Behaviour (headless render, scripted clicks): AIR — title, chapter
  select, prologue, opening sequence, story into day 2 (July 19) with
  backgrounds, sprites, calendar object, choices, voices; 150,000 frames, no
  error, no approximated transition. RUSUR — title, name entry, story to day
  14; 300,000 frames, no error.
- Not done / known gaps: save/load and the system-command menu (right-click
  does nothing yet; RealLive saves need graphics-stack replay); save
  thumbnails; motion transition styles (scroll/squash/slide/zoom/ripple)
  are shown as crossfades and counted in diagnostics
  (`approximated_transitions`); nine-slice window frames, per-channel tint,
  drift (weather) objects, HIK animated backgrounds and filter-area objects
  fail visibly; the text-object control syntax (`#c` etc.) is not
  interpreted; window open/close animations, backlog and auto mode are not
  implemented. Windowed play not verified.
- Tests: reallive/reallive-dll/image/scene/runtime/engine/audio/siglus/
  formats-arc/rfvp/krkr 907 passed, 0 failed; policy tests OK; api_docs pass
  (fingerprint 0a86cec0fc22dca2, logged).

Follow-up (same day, owner report: kr crashes after a while, AIR `play` would not start):
- `run.py … play` passed a nonexistent `play` subcommand; it now runs `crysirror run`.
- kr exits reproduced and fixed at the engine level: snapshots may now contain
  text (fonts pinned by content digest in the snapshot key; previously
  "snapshot font requires an immutable captured font handle"); per-frame
  snapshots are de-duplicated and limited to 512 (the load screen had 198,
  extra mode 150); region freezes (fade/filter/opacity group/atop) are
  rasterized at the region's size instead of the whole canvas (the load
  screen exceeded the 256 MiB texture budget). All seven title buttons
  (new game, load, last save, flowchart, extra, settings, quit) render
  without error headless. An unresolved E-mote motion icon
  ("normal/mask", seen once in a window, not reproduced headless) now logs
  a warning and skips that frame instead of exiting.
- AIR window exit ("manual-mode draw backlog limit") fixed: manual draw mode
  now presents a frozen copy of DC 0 that `refresh` replaces, instead of
  holding batches.
- Canvas `restore` (replace a surface with a queried state) added for native
  saves. Tests: scene pass; wgpu with ignored GPU tests (Metal, font set)
  60 passed 0 failed (new: text inside a frozen canvas).
- Owner report "the text window stays up during AIR's OP": the adapter had
  msg 151/152/162 inverted. Now as upstream: 151 closes the active window
  (frame included), 152 clears it, 161 closes all, 162 clears all. AIR's
  opening shows no window; story text still shows it; RUSUR 300k frames pass.

## 2026-10-30 — RFVP/BGI integration boundary audit and owner requirement

- Read-only code audit: the current `crysirror-rfvp` depends on workspace
  `rfvp`, mapped to `crates/rfvp-upstream/rfvp` with its portable core. It
  translates sprite/solid commands into a shared PuppetFrame and uses shared
  channel audio/storage, but the original core still owns image decoding and
  glyph rasterization. Earlier game screenshots and save-write results do not
  establish completed capability extraction. RFVP does not yet meet the
  owner's thin-adapter / first-class-source acceptance requirement.
- BGI: no replacement native adapter is present; the desktop still rejects
  `bgi`. Shared CBG/resource decoders are not a completed BGI integration.
- Recorded the owner's first-class workspace-source rule in AGENTS.md.
  Merely renaming upstream packages is insufficient. Existing source notices
  and provenance must remain. No implementation migration, build, game replay,
  commit or push was performed during this audit.

## 2026-10-30 — First-class RFVP VM and shared native capabilities

- Implementation: RFVP bytecode/syscall/primitive/wait/save policy now lives in
  `crysirror-rfvp-vm`; the old `rfvp-upstream` package is removed. Font metrics,
  glyph/ruby/inline-token layout, outline/shadow, reveal surfaces and snapshots
  moved to `crysirror-text`. NVSG decode, channel conversion, tone, blits and
  coverage operations belong to `crysirror-image`; output-history LZSS and cyclic
  XOR belong to `crysirror-resource`. The native adapter translates handles and
  policy. Original software rasterizer/mixer and unsafe pixel storage are gone.
- Implementation: RFVP globals, legacy metadata and pending exit state are
  instance-owned. Engine audio uses scoped groups, buses and source EOF feedback.
  Shared immutable `puppet::cpu_snapshot::MeshSnapshot` provides bounded CPU mesh
  capture, interpolation and linear blending; RFVP previous-frame/mask capture
  calls it. Syscall errors now propagate instead of becoming `Nil`; unsupported
  operations retain their opcode/argument cursor for failure diagnosis.
- Implementation: the Siglus `SceneVm` remains first-class in
  `crysirror-siglus-vm`. Shared wipe masks, NWA, OMV plane conversion, MPEG probes,
  E-mote player helpers and the transition shader no longer pull the full old VM
  into engine services. Five pure codec crates are first-class; original players
  and their device dependencies are excluded. `siglus-upstream` is removed.
  Provenance/licences remain in SOURCE.json and the shared capability source map.
- Implementation: Siglus accepts capability-relative private decode material,
  encoded Gameexe.dat, START_SCENE/label and screen metadata. No user keys, fonts
  or game bytes are distributed. Scalar key-file parsing is corrected; native
  runtime snapshot schema is 8. API docs regenerated: `26edfb80137ff54a` (previous
  `c6204fad405840ca`); migration is recorded in docs/api/CHANGES.md.
- Compilation: desktop release build with `rfvp,krkr,arc-formats` passed; release
  `--all-targets` checks passed for the changed audio/image/puppet/puppet-eluna/
  RFVP/RFVP-VM/Siglus-VM/video crates and desktop. Existing warnings remain.
- Tests: release RFVP 4, RFVP-VM 31, Siglus-VM 19, Resource 14, Text 25 (3 ignored),
  Puppet 9 plus 2 public capture tests, Image 84 (1 ignored) plus 1 public wipe
  test, Audio 15 (1 ignored), Puppet-Eluna 9 (1 ignored), Video 18 (1 ignored),
  API docs 7 passed. The opt-in shared slot-font test passed with a runtime system
  font. Python tooling 199 passed. Ignored tests are not passes. Worker capture,
  instance isolation, cancellation/budgets, original slot serialization,
  plane/alpha pixels and shared source decoding have independent fixtures.
- Behaviour: `Downloads/testcase` renders its title through shared engine meshes,
  textures, text and CPU previous-frame capture on Apple M5/Metal. NewGame now
  exits with `Unsupported RFVP syscall Movie at 0x6206d` for `meffect/001.wmv`.
  Earlier CG renders that swallowed this Movie error are rejected evidence.
  Rewrite HF and Rewrite Plus decode installed resources and reach real boot
  scene `sys10_su00`, then explicitly fail at `system.check_dummy_file_once`.
  Neither has reached title/gameplay; the reference handler merely recording a
  request is not a verified disk-authentication implementation.
- Limits: native Movie/Media mapping, PCM streams, full font/MWND/UI coverage,
  whole-game/save coverage and BGI remain incomplete. Siglus still lacks form
  adapters for its shared movie/E-mote/wipe capabilities. Mesh capture is a CPU
  reference with a worker-safe job API; native RFVP currently calls its blocking
  convenience path. GPU parity and live async capture are unverified; native
  text/scenario/image/audio preparation still needs full ADR 0050 boundaries.
  No whole-engine async or complete compatibility claim is made. Logs and local
  screenshots: `target/verification/firstclass-native/`. No commit or push.

## 2026-10-30 — Parallel native API profiles and shared Media

- Implementation: shared Media workers now route capability readers through
  ASF/WMV/WMA and OMV/Ogg Theora/Vorbis. Float PCM appends preserve decoded
  samples without integer quantization. ASF metadata/packets/reassembly and
  Theora dimensions/Vorbis formats have explicit bounds. `media_frame` exposes
  immutable engine pictures whose clones retain memory leases; foreign clients
  cannot read another resource. Stop releases the manager's picture. No VM,
  decoder process, window or private native mixer owns this pipeline.
- Implementation: RFVP Movie submits shared Media requests and observes engine
  frames/EOF through native epoch/slot/layer policy. Native movie decoders are
  removed. Missing/header-only/bad load data, failed writes and unavailable
  thumbnails fail instead of logging-and-continuing. BGI is a new first-class
  bounded BP executor/semantic adapter; NScripter is a new first-class source
  interpreter. They call Canvas/Layer/GlyphRun/UI/resource/audio APIs, with
  worker loading and explicit unsupported operations. They do not host old
  App/renderers. Sources/pins/licences are recorded in their crates.
- Implementation: Siglus GLOBAL.MOV PLAY/WAIT/WAIT_KEY/STOP uses shared Media,
  ordinary engine Image layers and native EOF/DECIDE/CANCEL results 0/1/2.
  It owns no second decoder or movie clock. Live movie save reconstruction and
  OBJECT movie/seek/pause/loop remain unsupported; no authentication was bypassed.
- Compilation: final desktop release build with `rfvp,bgi,nscripter,krkr,arc-formats`
  passed (`parallel-final2-desktop-build.log`); release desktop checks passed.
  Source-generated adapter metadata and strict Python config/declared-inventory
  validation now include RFVP/BGI/NScripter. API fingerprint is
  `810d2690fcbf11b5` (formerly `26edfb80137ff54a`), logged in docs/api/CHANGES.md.
- Tests: RFVP-VM release 33 passed; RFVP facade 4 passed. BGI VM 9 and adapter 2
  passed, plus the runtime system-font test 1 passed (default font test remains
  ignored). NScripter syntax 2 and native integration 4 passed, including hidden
  choice ordinals and unsupported-save failure at the native PC. Siglus MOV
  release suite 23 passed/1 ignored; the fixture writer separately passed 1.
  ASF reader tests 2 passed/1 ignored, with owner WMV prefix separately passed 1;
  Theora/Vorbis reader test 1 passed/1 ignored, with an owner Rewrite OMV separately
  passed 1. Audio release 16 passed/1 ignored. API docs 7 and Python tooling 200
  passed. Ignored tests are not counted as passes. Logs distinguish earlier
  Canvas-ID/audio-admission/Focus failures from the final passing runs.
- Behaviour (final desktop, Apple M5/Metal): real RFVP testcase NewGame reaches
  its post-movie CG in 3500 ticks, decoding 979 and publishing 973 WMV frames.
  The game's save UI writes slot 1 with its thumbnail; a fresh process uses
  Continue/slot 1 to restore the saved scene. The final binary cold-load replay
  passed with strict load-error propagation enabled. Whole-game coverage,
  commercial original-save interchange, choices/endings, legacy shell/dialog
  APIs, full font/PCM modes and live-movie save reconstruction remain unproven
  or unsupported. Native CPU capture/admission still needs full async migration.
- Behaviour: the authored BGI BP scene renders shared image/text layers, mixes
  audio (peak 0.305), accepts Enter and halts. The authored NScripter source
  renders shared image/text layers and waits for input. These are native format
  fixtures, not real BGI/NScripter games. Siglus's authored SCN plays WMV/WMA,
  publishes four frames, mixes audio (peak 0.127), stops and halts with int_stack
  `[0,77]`. Rewrite HF/Plus still explicitly stop at disk authentication; neither
  has reached title/gameplay. No blank frame, rejected Movie error or factory
  registration is counted as game completion.
- Remaining scope: BGI native-bp-v1 is not the complete original VM: scenario/
  Buriko, archive search, complete heap/multi-module/scheduling, original saves
  and many native calls are missing. NScripter plain-nscript-v1 still lacks
  complex windows/markup/effects, archive mounting, numbered script concatenation,
  skippable delay, original saves and real-game validation. All four adapters
  therefore cannot be reported as fully completed. Native fixtures/screenshots
  and final logs are in `target/verification/firstclass-native/` (BGI project in
  `target/verification/bgi-native-profile/`). No commit, push or CI was added.


## 2026-10-30 — Desktop native menu bar and title-bar policy

- Implementation: desktop continues mirroring the engine-owned MenuSystem into
  AppKit's application menu bar on macOS and the HWND menu bar on Windows.
  Native selections and accelerators use the same scoped activation validation
  as shared UI, with diagnostic activation logs. Windows menu replacement now
  tracks attachment separately from its dirty cache, so a selection-triggered
  rebuild detaches the previous bar. macOS explicitly uses Regular application
  activation and disables winit's competing default menu. Linux keeps a visible
  engine-drawn fallback; this is explicitly not a native Linux menu bar.
- Implementation: ordinary desktop windows retain their OS title bar by default.
  `--allow-borderless` permits a game's undecorated-window request; fullscreen
  remains independent. The original native borderStyle and shared window-policy
  request are retained. Unchanged window styles are no longer reapplied every
  frame. No bundle was added.
- Compilation: full-feature desktop release check passed (rfvp,bgi,nscripter,
  krkr,arc-formats; native-launch-round/check-v2.log). An offline release
  cross-check for x86_64-pc-windows-gnu passed
  using the already installed target/toolchain. Its small ignored harness
  directly includes the production native_menu.rs with real Scene/winit/muda
  dependencies and deny(unsafe_code). This checks the Windows presenter source,
  not the full desktop application or Windows runtime behaviour.
- Tests: the final full-feature desktop release binary suite passed (17/17),
  including three native event-routing cases, title-bar/fullscreen/opt-out policy
  and content-based detection (native-launch-round/unified-tests.log). The final
  `native_menu_bridge` release integration suite passed (2/2), including refusal
  to acknowledge unsupported popupEx as user cancellation. Its host selection case
  enables native-bar presentation, activates a menu through the exact shared
  host entry used by native presenters and verifies the TJS onClick callback
  and authoritative radio state without canvas pointer input or an OS window.
- Behaviour: a windowed game-kr diagnostic before the owner's subsequent
  headless-only instruction installed the AppKit menu bar. Native logs identify
  the missing-title-bar cause: the game writes borderStyle=2, then borderStyle=0.
  That diagnostic process was stopped; all subsequent validation is device-free
  or offscreen. The final title-bar preference and physical OS menu clicking
  have not been window-tested. Non-bundled binaries cannot be selected by the
  available computer-use app resolver, and no bundle workaround was introduced.
  Native context popups/synchronous MenuItem.popup/popupEx remain unimplemented.
  The boundary audit changed popupEx's old log-and-void path into an explicit
  unsupported error: no host popup means no user cancellation may be reported.
  Logs: target/verification/native-menu-round/.

## 2026-10-30 — NScripter installed Tsukihime native bring-up

- Implementation: native named variable indices, backtick English strings,
  multiline selections, inline click/page/speed/wait controls, physical-line
  `skip`, date/timers/delay, square horizontal `setwindow`, staged sprite moves,
  character placement and native atlas buttons now call shared engine services.
  Image alpha/key/paired-mask extraction and stripe/stagger rule generation are
  shared `crysirror-image` worker effects. Sprite-sheet timing/UV selection is
  shared `crysirror-scene::sprite`; retained widget hover/focus/press inspection
  is a client-scoped engine query. No ONScripter runtime/renderer is linked.
- Implementation: original game archives stay packed. Native resource policy
  searches loose files then explicit engine archive prefixes, with scoped case
  matching; the desktop bootstrap owns shared mounts. Native global indices
  >=200 and accessed-resource metadata use worker-backed atomic Save writes to
  `nscripter-state.json`, a separate engine format, not an original game save.
  Framework snapshot schema is 2; pending work and prepared image reconstruction
  are explicitly rejected. Native savegame/loadgame remains unsupported.
- Compilation: the release owner_game target compiled and executed. Earlier
  failed iterations (private module access and native `trap` label tokenization)
  are retained as failures in the verification directory, not counted as passes.
  The final combined desktop/check/test run is recorded separately by the root
  task after merging all parallel changes.
- Behaviour (device-free installed game): owner-provided Downloads/Tsukihime
  nscript.dat, arc.sar, arc.nsa, arc1.nsa, default.ttf and CD Ogg tracks run in
  place. The ignored owner test reached the real title's btnwait at native line
  12792/PC10802 after 2616 ticks, asserting title1.png/titlebtn1.png resources.
  Reusing the same release test executable with CRYSIRROR_NSCRIPT_STORY=1 used
  the title and Start submenu buttons, passed the native Prologue chapter
  sequence and reached s20's hospital sentence at line14118/PC12003 after3152
  ticks; the background is image/bg/bg_51b.jpg. No script patch or PC jump was
  used. This proves native control/shared state, not a GPU screenshot or audible
  device output; the desktop render verification is separate.
- Validation: syntax tests 3 passed. A preceding native suite had 3 passes and
  one obsolete direct-file-asset assertion after worker surface extraction;
  that assertion now checks engine-owned RGBA and retained native source policy.
  Added a cold-instance shared metadata persistence test; its final execution,
  plus shared key/mask/sequence/interaction tests, belongs to the combined run.
- Limits: full original save/global-file interchange, system menus/backlog/skip,
  rich markup, general animated sprites, numbered source concatenation and full
  window enter/leave animation remain unfinished. Stored window/menu definitions
  do not establish those features. Rule transitions use 8-bit masks and have no
  original integer-pixel parity claim. Image metadata admission is synchronous;
  full ADR0050 staging remains open. No whole-game completion claim. Evidence:
  target/verification/nscripter-tsukihime-owner-third.log,
  nscripter-tsukihime-story-first.log and nscripter-tsukihime/*.json.


## 2026-10-30 — KiriKiri bitmap planes and Canvas capacity follow-up

- Implementation: completed the existing shared RGB-only Canvas copy and
  compaction path with zero-alpha RGB preservation and generation-safe
  materialization completion. An older completion cannot discard a newer
  pending request. Canvas shrink discards cropped pixels instead of exposing
  them again after growth. Node budgets remain bounded.
- Implementation: shared ImageCompositeLayer now carries destination clips;
  worker composition validates clips and preserves source sampling offsets.
  KiriKiri translates native clips, partial fills, holdAlpha fills and
  top-left image resizing into those immutable engine bitmap declarations.
  The adapter no longer loses or retains stale CPU image sources before
  subsequent shared SoftLight/effect jobs. No private pixel algorithm,
  renderer or composed legacy screen was introduced.
- Compilation: the desktop release build with rfvp,bgi,nscripter,krkr,arc-formats
  succeeded and its new binary ran both installed games below.
- Tests: shared runtime clipped bitmap pixels 1 passed; Canvas materialization
  ownership 2 passed. The final release KiriKiri engine_bridge harness passed
  24 tests, with 1 ignored, including clipped copy, same-size/grow/shrink resize,
  partial/holdAlpha fill and subsequent SoftLight publication. Two explicit
  GPU regressions passed on Apple M5/Metal: RGB-only copy/direct draw with
  transparent source, retained destination alpha and clip; compaction to PNG
  materialization preserves every channel within one quantization step.
  The additional shrink/regrowth GPU test is supplied but not counted in
  this local result; duplicate Cargo jobs were cancelled for unified validation.
- Behaviour: direct `crysirror play <installed-folder> --headless` required no
  hand-written project, mount script, font option or game extraction. CLOVERDAYS
  ran 6000 ticks through the old colour-only copyRect wall and the later
  SoftLight source-loss failure: background, two characters and dialogue
  rendered; 3130 engine API calls, native error=null. DC5t4LM ran 2700 ticks
  into its scenario CG/dialogue; 7199 API calls, 184 Canvas surfaces, a maximum
  of 203 logical nodes per surface and 13 materialized-image references, with
  no old presentation-node-limit error. Both exited successfully and explicitly
  reported window_created=false, os_menu_created=false and audio_emitted=false.
- Limits: these are real story-prefix results, not whole-game/save compatibility
  or audible-device/native-OS-menu validation. DC5t4LM still logs its existing
  opaque piledCopy/straight-alpha profile limitation. Screenshots, exact input
  tracks and logs are local under target/verification/krkr-walls-final/; final
  automatic-launch captures are clover-play-final.png and dc5-play-final.png.
  No game data/font was added to the repository; no commit, push or CI.

NScripter rendering follow-up (same round): the desktop hospital capture exposed
an adapter error that logical background checks alone missed. The original
bg_51b.jpg is a complete 640x480 image; the right half was covered by a stale
right-character layer from the chapter sequence. Pinned ONScripter bgCommand
removes all three character slots. The native adapter now clears their retained
and unpublished staged declarations on bg, while retaining ordinary lsp sprites.
An independent regression covers both published and staged character removal.
The corrected desktop image and final release test results are recorded by the
root task's final validation; the earlier half-black capture is rejected visual
acceptance evidence.


## 2026-10-30 — Shared pure-Rust content identification

- Implementation: `crysirror-resource::sniff` exposes FileDetector and owned
  FileIdentity evidence using pinned pure-magic 0.4.1 and magic-db 0.6.0. A
  OnceLock caches one verified immutable Arc-backed embedded database and its
  initialization failure, with a PNG rule/MIME sentinel. The public API uses
  caller slices or AssetReader capabilities; it has no libmagic FFI, file(1)
  subprocess, ambient file/mmap entry point or runtime rules installation.
  No filename extension is passed to the matcher. Source/version/archive hashes,
  licence provenance and the database's existing DER exclusion are recorded in
  crates/crysirror-resource/SNIFF_SOURCE.json.
- Implementation: identification requests at most a 64-KiB logical prefix.
  Partial reads are completed, while premature zero reads, invalid read counts
  and changed source lengths fail visibly. Generic empty/text/data fallback is
  None; positive MIME/description/rule/strength observations are explicitly
  evidence, not structural validation, authentication or decoder/VM support.
  The detector is Send+Sync; matching owns its result and retains no VM or asset.
  Desktop uses it from the named boot-detection worker before Runtime creation.
- Compilation: the unified full-feature desktop release build passed
  (native-launch-round/build-final.log). No libmagic library was required.
- Tests: six independent resource release tests passed, covering embedded PNG/
  WAVE rules, unknown/empty/short headers, partial and malformed capability
  reads, the fixed prefix budget, renamed extensions and deterministic shared
  concurrent queries. The same unified run passed the desktop's renamed-PNG
  native-script rejection test. Logs: native-launch-round/unified-tests.log.
- Limits: classification cannot inspect rules beyond the sampled prefix, verify
  complete files, authenticate bytes or detect arbitrary same-size mutation.
  Engine-specific signatures and parsers still establish native layouts;
  recognition does not add an unsupported format decoder or engine adapter.
  No CI, bundle, commit or push was added.

## 2026-10-30 — Installed-game bootstrap, content evidence and final offscreen verification

- Implementation: `play <folder-or-file>` now constructs typed native projects
  and archive mounts internally. NScripter, RFVP and Siglus metadata paths join
  the existing KiriKiri/RealLive/AVG32 detection. A named boot worker performs
  inspection; Resources `beginDecode`/completion/`mountDecode` indexes on engine
  workers and publishes scoped containers before VM execution. No generated
  `play-mount.ts` is needed. `--headless`, frame bounds, capture and input tracks
  exercise the same launch plan without a window/audio device. This boot fence
  still waits before the event loop; it is not a loading-screen/full-async claim.
- Implementation: shared `resource::sniff::FileDetector` uses pinned pure-magic
  0.4.1 and embedded magic-db 0.6.0, with immutable shared rules and at most 64 KiB
  per capability/slice. It has no C libmagic dependency or filename hint.
  Partial reads, premature EOF, invalid read counts and changing lengths fail;
  unknown fallback stays unknown. Detection records MIME/rule evidence while
  native parsers still validate signatureless/encrypted layouts. Renaming a PNG
  to 0.txt is rejected. This does not add format decoders by itself.
- Implementation: NS source/resource/UI/audio policy calls shared engine
  services. GPU inspection caught an additional real native-policy bug after
  successful control-flow tests: `bg` left a chapter's right-side TACHI in the
  scene, obscuring half the hospital background. The adapter now removes all
  three published/staged character slots together with background presentation,
  retaining ordinary sprites. A native regression covers both published and
  staged slots so a later print cannot resurrect them. The original half-black
  capture is marked `rejected-retained-character-story.*`, not accepted evidence.
- Compilation: the final full-feature desktop release build passed
  (`native-launch-round/build-final2.log`, features rfvp,bgi,nscripter,krkr,
  arc-formats). API docs regenerated and verified: fingerprint
  `8ff9a0e655c6babe`, previous `8e6792f059054768`, with migration notes.
- Tests: unified release validation passed 181 tests / 1 ignored across 12
  targets, including desktop 17, engine API docs 7, native menu bridge/popup
  failure 2, resource sniff 6, NS syntax 3/native integration 5, image 94
  (1 ignored), and shared UI/sprite state. After the final NS background fix,
  desktop 17, NS syntax 3 and native integration 6 passed again; these overlap
  the unified totals. Explicit Metal GPU canvas tests passed 3, including
  RGB-only copy, compaction/materialization parity and permanent resize crop.
  Python tooling passed 200. Ignored tests are not passes. No full-workspace
  or whole-game validation is claimed.
- Behaviour (final offscreen path, Apple M5/Metal): automatic original-directory
  launch reaches Tsukihime's real title (line 12792) and Start -> chapter curtain
  -> hospital dialogue (final PC 12004 / line 14119). The final picture shows the
  full hospital background and the English first sentence, with the stale TACHI
  absent. Engine audio is mixed (active 1, peak 1.0); device output is not tested.
  Title and final story captures are `native-launch-round/tsukihime-title.png`
  and `tsukihime-story-final.png`. SAR/NSA patches and local font are used in
  place; no user game data or font was added to the repository.
- Behaviour: CLOVERDAYS automatic `play --headless` runs 6000 ticks into
  background/characters/dialogue after the original colour-only copyRect and
  later SoftLight source paths. DC5t4LM runs 2700 ticks into CG/dialogue, with
  203 maximum retained canvas nodes and 13 materialized-image references. Logs
  and captures are in `krkr-walls-final/*-play-final.*`. Both report no window
  or OS-menu creation. game-kr's same typed bootstrap reaches ATTENTION at
  1200 ticks; its initial capture is not counted as a title/playthrough result.
- Launchers: the verified binary is refreshed at local/launchers/crysirror-dev;
  existing crysirror-krkr-dev shares that inode, so the existing game-kr
  CrySirror.command uses the new title-bar/menu policy. Tsukihime has a local
  CrySirror.command pointing at the same player. These were syntax-checked,
  not executed to create windows. No bundle, commit, push or CI was added.
- Limits: menu-bar event routing/title-bar policy are verified in tests and
  Windows presenter source cross-check; final OS clicking/window appearance
  remain untested under the requested headless workflow. Linux remains an
  explicit engine menu fallback; synchronous native popup/popupEx remain
  unsupported (popupEx no longer pretends cancellation). Original NS saves,
  system menus/backlog, full window animation, exact native integer transition
  parity and whole-game coverage remain open. DC5's existing opaque piledCopy
  limitation remains recorded. ADR 0097's generic x86 interpreter/lifted filter
  IR remains a design, while TJS executes verified portable bytecode today.

## 2026-10-30 — RiddleJoker native startup and copyRect semantics

- Implementation: KiriKiri Config.arguments supplies bounded native startup
  arguments before the first game script, overriding executable options.
  Automatic play supplies -debugwin=no, selecting the original game's
  production branch. The previous crash was an optional modeless developer
  LogPad WIN32Dialog.open request, not a gameplay message box. Unsupported
  modeless windows still fail explicitly; no fake dialog result or VM jump
  was introduced. Adding this startup policy preserves the automatic player's
  previous persistent-directory identity while snapshots retain full config.
- Implementation: native copyRect resolves dfAuto from the layer type and
  ignores HoldAlpha for alpha/add-alpha faces, matching pinned krkrz
  fd5c4baa6a2ef5978db1bd043634351f48667daf LayerIntf.cpp. Only the opaque
  face preserves destination alpha. Operations use existing shared Canvas
  Copy/CopyRgb and immutable image-composite workers; no private painter.
- Compilation: full-feature desktop release build passed (rfvp,bgi,nscripter,
  krkr,arc-formats), riddle-fix/build-final.log. Generated API docs verified
  fingerprint 2663a3650d0f8656, previous 8ff9a0e655c6babe.
- Tests: final release run passed desktop 18, engine_bridge 25 (1 ignored),
  startup_arguments 2 and win32_dialog 3. API docs 7 passed separately; Python
  tooling 200 passed. A first regression harness incorrectly used Gamma on
  native layer kind 13; the final harness selects straight-alpha inspection
  after copying and passes. No ignored test is counted as a pass.
- Behaviour: the shared confirmation was crossed with ordinary No-button input.
  The final automatic offscreen player reached the title at 6000 ticks, then
  NewGame/story at 9000 ticks with native error=null and last_tag=p. The final
  story capture contains starry background, dialogue frame and Japanese text;
  copyRect frame-load errors are absent. Engine audio mixed one active source;
  no window, OS menu or device audio was created. Evidence is local under
  target/verification/riddle-fix/{title-final,story-final}.{log,png} with exact
  input tracks. Original game resources and scripts were not changed.
- Launchers: refreshed local/launchers/crysirror-dev and the hardlinked
  crysirror-krkr-dev from this verified release binary; the installed game's
  CrySirror.command still points to it. Shell syntax checked, no window opened.
- Limits: the title menu is interactive but its dynamic background art remains
  incomplete; the title capture is not accepted visual-parity evidence.
  Missing native TFT fonts use fallback. Whole-game coverage, animated title
  rendering, smooth-loop sidecar parity, native window/menu interaction and
  audible-device output remain unproven. No commit, push, bundle or CI.


## 2026-10-30 — Siglus native MWND proc and shared motion follow-up

- Implementation: original SceneVm stage.MWND[0] commands now translate PRINT/
  NAMAE, OPEN/CLOSE (wait and nowait), CHECK_OPEN, deferred CLEAR, message-block
  boundaries, NL/NLI, WAIT_MSG/PP/R/PAGE and frame filename access onto retained
  CrySirror GlyphRun/Image/Solid/Filter layers and engine reveal state. Native
  read-flag stocks commit at clear/close boundaries. A DECIDE down-up that finishes
  reveal cannot also consume the following native message-key proc. R/PAGE clears
  the block after that key proc, retaining the old visible text until new text
  starts. CLOSE preserves text instead of pretending that it is a clear.
- Implementation: bounded horizontal Gameexe metadata provides actual window
  positions, message grid/count/size/spacing, palette colors and default font
  alias. Name spacing is independent; zero name-window dimensions derive from
  the native name grid. Supported WAKU textures use individual scoped G00 assets
  at intrinsic size and cropped origins, plus engine filter/opacity operations;
  filters support native margins, configured channel additions and transparency.
  Name message-position adjustments are retained. Native G00 names now search
  g00/ before the authored-fixture root fallback. No complete legacy screen,
  private glyph rasterizer or private image-composition algorithm was added.
- Implementation: reusable device-free scene::animation::ScalarTween exposes
  explicit monotonic-clock sampling, finite endpoints, continuous retargeting
  and serializable state, reusing shared layer::Easing. Siglus native window
  animation kinds 0..5 select directions/coordinates and map shared samples to
  Layer position/opacity. The native 500-ms type-3 slide has independent coverage.
  Runtime snapshots advance from schema 8 to 9 for native wait/window state;
  restoration validates derived message configuration and shared curve state.
  Original native MWND save-file reconstruction remains explicitly unsupported.
- Compilation: cargo test --release -p crysirror-siglus-vm -p crysirror-scene
  --lib completed successfully (native-tests-complete.log). Separate earlier
  compile/test failures are retained in the verification directory and are not
  counted as passes. The standalone self-authored mwnd_fixture generator also
  compiled with release dependency artifacts and produced its native PCK/
  Gameexe/project outside source export. Desktop build/render is a separate
  validation boundary and is recorded by the root task after unified build.
- Behaviour validation: shared scene tests passed 29; native Siglus tests passed
  29 with 1 ignored, including real native-ABI SCN execution, engine resource
  inspection, deferred clear/read flags, complete DECIDE release semantics and
  combined VM/engine snapshot replay. WAKU Image/Filter layers, native palette,
  grid, name spacing and slide position/opacity have direct shared-state
  assertions. These are device-free tests, not GPU/font-raster parity evidence.
- Installed-game evidence: both owner Rewrite HF and Rewrite Plus automatic
  offscreen launches reproduce system.check_dummy_file_once before title. The
  pinned siglus_rs reference only records that request; the official manual
  describes disk authentication using DummyMaker data plus env/DISKMARK. No
  verified authentication backend or supplied authentication medium was found.
  Recording/ignoring that request is not treated as success. No game script,
  PC, authentication result or user resource was altered. These baseline
  failures are not new-binary title/gameplay results.
- Limits: this remains a bounded Siglus integration. Gameexe vertical/novel
  profiles, rich markup/ruby, font decorations, WAKU buttons/callbacks and other
  unmapped native mechanisms still fail explicitly. The host default font is
  not a claim of native font/metric identity. Native image admission still has
  the prior synchronous capability/decode boundary; full ADR0050 resource-task
  staging is not claimed. Evidence is local under
  target/verification/siglus-completion/. No fonts/game data, CI, commit or push.

Siglus pointer follow-up in the same round:

- Implementation: native Gameexe MOUSE_CURSOR.DEFAULT is a table index; index
  zero selects the game's custom cursor rather than the OS default. Scoped
  G00 albums become shared UI CursorSpec definitions, preserving each 32x32
  cut, native center hotspot and SPEED step duration. SCRIPT.SET/GET_CURSOR_NO
  and SET_MOUSE_DISP_ON/OFF publish shared UI selection/visibility. Cursor
  definitions are re-established after framework restore; no OS pointer/device
  state enters the VM snapshot. Invalid sizes, out-of-image hotspots and
  unsupported cursor resources fail visibly. Automatic hide policies and
  exact original-versus-host animation phase parity are not established.
- Compilation/validation: the final release library run passed 29 scene tests
  and 31 Siglus tests, with 1 ignored (60 passes total). The cursor tests verify
  original-ABI default/custom index zero, SCRIPT selection/visibility changes,
  client ownership, RGBA/hotspot/schedule and resource redefinition on restore,
  and reject a one-pixel native cursor without fake fallback.
  Log: siglus-completion/cursor-tests-final.log. Python native-project tests
  passed 9 after aligning the tool validator with the supported explicit
  message-window override plus Gameexe boot metadata; no schema fields changed.
- Real resource validation: both Rewrite HF and Plus's original
  g00/mn_mw_mcur00.g00 decode through shared crysirror-siglus::g00 into 20 cuts,
  all 32x32 with supported hotspots; first hotspot is (0,0). Their Gameexe
  defaults select index 0 at 100 ms per cut. This is resource evidence, not
  title/gameplay success or physical OS-cursor validation. Diagnostic decoder
  output is siglus-completion/cursor-resources.log. Desktop behavior with the
  final unified binary is recorded separately. No game resource was committed.

Siglus final offscreen behavior (fixed desktop SHA-256
706f94778ceea33672577a4fc1384990bec3ebf6e429857543ef64a132094812):

- The self-authored native PCK/Gameexe fixture renders a visible engine-owned
  message filter, speaker and GlyphRun at 100 frames (native R input wait,
  PC153). A genuine mouse DECIDE down/up at frames150/152 advances to the
  second named message at 250 frames (native PP input wait, PC306); first-page
  text is cleared at the native boundary. Both screenshots were inspected.
  Native unknown-form/opcode maps are empty. Logs and captures are
  native-mwnd-first-final.* and native-mwnd-next-final.*. This validates shared
  rendering/input, not original font-metric/pixel parity or a real-game title.
- A separate authored SCN resource probe combines that fixture with the owner's
  unchanged Rewrite cursor G00 and its MOUSE_CURSOR metadata. Native VM startup
  publishes client1's engine custom cursor id0 with visible=true, verified in
  native-owner-cursor.log. The source resource has 20 validated 32x32 cuts; no
  original game script or authentication flow is executed by this resource
  probe. A headless image does not establish physical OS cursor appearance or
  exact host animation phase. User data stays only in ignored local target/.
- Automatic original-directory launch with the same final desktop binary was
  rerun on both Rewrite HF and Plus. Both still fail visibly at the real
  system.check_dummy_file_once before title; hf-final.log and plus-final.log
  record this. These are failed game starts, not successful game compatibility.
- The fixture's first, mixed command/read-flag ABI failed native validation;
  its generator was corrected to a consistent original CD_TEXT/-1 profile,
  without changing the production interpreter or bypassing validation. The
  rejected fixture run remains native-mwnd-first.log; accepted input is
  native-mwnd-fixture-v2/project.json and native-mwnd-input.json. No window,
  audio device, CI, commit or push was used for these checks.

## 2026-10-30 — Circus MES/CRXG/XPCM groundwork and explicit execution gap

- Implementation: new first-class crysirror-circus parses native offset2 MES
  versions 0xaab6/0xabb6. It preserves signed word/byte/string operands, original
  label metadata and instruction boundaries; native 0x77 continuation operands
  are consumed with their preceding command. Byte/string/instruction budgets,
  malformed headers, truncation and non-boundary PC lookups fail explicitly.
  Pinned MesTextTool format reference/provenance and GPL notice are preserved.
  This crate has no executable VM/factory. Circus instruction/system/variable,
  wait, UI event, scenario and save policy remain unimplemented; DC4PHDL cannot
  run. Automatic detection now reports the recognized unsupported profile,
  instead of a generic unrecognized-engine error or a pretend running client.
- Implementation: shared Image's content registry decodes CRXG compression
  1/2/3 with bounded LZSS/zlib, row predictors, palette/alpha conversion and
  origin metadata, independently of a VM. Shared AudioSource paths unwrap
  XPCM mode 0 PCM and mode 5 Ogg through a seekable bounded payload view and
  existing packet decoding; songs are not decoded/allocated in full at open.
  Spectral modes 1/3 remain explicit unsupported errors. GARbro source commit,
  hashes and MIT notices are retained; no original engine host was adopted.
- Compilation: final full-feature desktop release build passed with features
  rfvp,bgi,nscripter,krkr,arc-formats (dc4ph/desktop-final-build.log).
  Binary sha256 706f94778ceea33672577a4fc1384990bec3ebf6e429857543ef64a132094812.
  crysirror-circus and crysirror-audio release library builds/tests also passed.
- Tests: MES authored format/invalid-boundary tests 2 passed; XPCM sample/exact
  seek/EOF and malformed/unsupported mode tests 2 passed; CRX tests 3 passed,
  including row predictors, LZSS literals/block tables, palette/alpha/planar
  runs and malformed/bomb rejection. The final full Image release suite passed
  100 tests / 2 ignored, overlapping the CRX/color/cursor tests. Desktop release
  tests passed 19, including explicit recognized-unsupported Circus reporting.
  API docs regenerated/verified (7 passed), fingerprint remains 2663a3650d0f8656;
  host-only API additions are logged. Python tooling passed 200. Failed initial
  XPCM type inference was repaired before accepted builds; no failure is counted
  as a pass. Ignored tests are not counted unless explicitly run below.
- Behaviour/resources: explicitly invoked ignored local-corpus tests decoded
  all 1339 CRXG images, parsed all 536 MES scripts and opened/read/seeked 148 PCM
  and 52 Ogg XPCM resources from the owner's original DC4PHDL directory. Original
  assets were read in place. This is resource/format validation, not whole-file
  reference pixel/audio parity or native game execution. New binary --detect-only
  returns exit 1 with recognized Circus dc4ph / 0xabb6 and explicit missing
  execution support; no game title/frame is claimed. Logs and local executable
  analysis are ignored under target/verification/dc4ph/. No game data/font/key
  was added to the repository, no native executable was run, no window created,
  and no commit, push, bundle or CI was added.

## 2026-10-30 — Native pointer resources and measured RFVP capture cost

- Implementation: RFVP now discovers native numbered loose ANI resources
  through client-scoped storage, decodes them with crysirror-image::cursor,
  and defines/selects shared UI cursor resources. Pinned rfvp 062fec8203
  setup policy prefers available slot 1; native CursorChange(0) selects the
  OS default, and subsequent selected indices, CursorShow and CursorMove
  reach the same UI cursor/visibility/warp services. The VM supplies only
  native indices and one-shot window requests; it retains no OS cursor or
  private decoder. Asset reads occur at boot, not on every cursor frame.
- Implementation: AVG32 native CUR loading now calls the shared bounded
  CUR16M decoder. BGR conversion, AND-mask/red-key coverage, hotspot and the
  native 150-ms schedule live in crysirror-image::cursor. The old VM cursor
  painter is removed. Native area-load/clear and visibility select the shared
  UI custom cursor or OS default; unchanged immutable images are not copied
  or redefined each frame. Sources/licences and affected VM hashes are retained.
- Implementation: RFVP's actual title uses a visible full-screen Sprt(-1)
  feedback layer with alpha 200/255. Sampling identified shared CPU frame
  capture as approximately 96% of main-thread samples; this is real required
  previous-frame sampling, not a request that may be silently skipped. Shared
  MeshSnapshot capture now specializes strictly validated rectangular topology,
  affine rectangular UVs and constant vertex colours, retaining the triangle
  reference for other geometry. Bilinear sampling, linear-light blending,
  cancellation and work bounds remain in the engine. Shared image::color adds
  a process-shared sRGB8 transfer with batch access. Byte transition boundaries
  are derived from the prior f32 powf/round expression; bucket prediction is
  corrected against the original unquantized input, avoiding per-pixel powf
  without replacing the colour contract with an approximate LUT.
- Compilation: the final full-feature desktop release build passed, SHA-256
  706f94778ceea33672577a4fc1384990bec3ebf6e429857543ef64a132094812.
  It includes the shared capture/colour and native RFVP/AVG32 cursor paths.
- Tests: RFVP adapter release tests 6 passed, native RFVP VM 34 passed,
  shared Puppet library 12 passed, shared CUR16M decoder 1 passed and AVG32
  installed-game tests 6 passed. The latter executes native area-load/clear
  instructions and checks engine cursor pixels/hotspot/schedule/default return.
  Colour tests 2 passed: every byte round-trip, 65,537 linear samples and
  +/-8 adjacent f32 values around every byte transition match the old f32
  transfer exactly. Rectangle tests cover clipping, filtered UV edges, five
  blend modes, translucent overlap and nonrectangular/gradient fallback.
  Python RFVP provenance/policy tests 5 passed. No ignored test is a pass.
- Behaviour: quiet A/B uses identical automatic play of Downloads/testcase,
  600 native ticks, no input, separate empty state roots and final offscreen
  capture. The previous binary (5b703468...) took 14.704 s, versus 7.059 s for
  the final binary: approximately 2.08x faster. VM-step mean was 24.03 ->
  11.32 ms, median 36.91 -> 17.55 ms, p95 44.11 -> 21.16 ms. Both reached
  the real title successfully, and every final RGBA byte matches exactly.
  The final instance decoded ANI slots 1/2 and selected slot 0 because the
  game explicitly selected the OS default. No window/OS menu/audio device
  was created. An earlier 34.56-s run overlapped desktop test LTO and is
  rejected performance evidence. Full logs, commands, binary hashes, CPU
  samples, timings and images are local in target/verification/rfvp-perf-cursor/;
  accepted measurements are quiet-report.json and image-comparison.json.
- Limits: these are title-prefix CPU/offscreen measurements, not interactive
  display FPS or whole-game performance. p95 still exceeds a 16.7-ms frame
  budget; previous-frame capture remains synchronous CPU work. No stable-60-FPS
  or GPU capture pipeline claim is made. Physical OS cursor presentation was
  not tested in the requested headless workflow. KiriKiri already uses the
  shared cursor service; Siglus cursor work is recorded by its separate task.
  NScripter setcursor is its message-wait indicator, not an OS pointer override.
  RealLive/BGI complete custom-pointer coverage remains unverified. No commit,
  push, bundle, proprietary font or game-data distribution was added.

Final local launcher publication for this round: after both agents completed
release tests and offscreen behaviour checks, the verified 706f94778ceea3367...
full-feature player replaced local/launchers/crysirror-dev. krkr-dev, rfvp-dev
and siglus-dev share that inode; the preserved RFVP baseline-player remains a
separate old inode for reproducible comparison. A new local
Downloads/testcase/CrySirror.command calls the common automatic player; shell
syntax passed, it was not executed to open a window. Siglus launchers now carry
the fixes but Rewrite HF/Plus still fail their actual disk-authentication call;
no runnable Circus launcher was created. Generated API references were blessed
then verified again successfully (the same 7 tests, not 14 distinct tests),
with reflective fingerprint 2663a3650d0f8656 unchanged by Rust-only additions.
No commit/push/CI or bundle was produced.

### 2026-10-30 — OUSAMA shared XP3 resource-filter bring-up (Codex)

- Implementation: added the shared XP3 `adler_xor_modules` profile for the
  31-byte Adler-derived rotating XOR family. Companion PE32 callbacks are
  inspected as bounded data; no DLL, executable, emulator or JIT runs. Known
  instruction semantics identify this family, and protected resources verify
  their original XP3 Adler checksum before a random-access reader is published.
  Unsupported callback shapes fail explicitly. Automatic desktop detection
  tries this profile for supplied TPM companions and reports each rejected
  profile/canvas rather than stopping after an index-only open.
- Compilation/verification: release filter library tests passed 3/3 (authored
  key vector, large offsets/split reads, companion evidence, decoded checksums
  and corruption rejection). An explicitly invoked ignored local-corpus test
  opened all six Downloads/game_kr_ousama XP3 archives and verified 187 sampled
  entries with zero failures; native startup/configuration were separately
  decoded and checksum-verified. These are actual local resource checks, not
  a claim that an ignored test passed without invocation.
- Behaviour: the existing player initially stopped before VM execution with
  `Config.tjs with scWidth/scHeight not found`. Static analysis of the supplied
  TPM identified the missing resource filter; the decoded game's own config
  declares 1920x1080. Native startup/title behaviour still awaits the rebuilt
  player. This is not evidence of a completed OUSAMA adapter or whole game.
  No missing optical-disc resource has yet been established. Original media,
  scripts and plugins were not modified or executed; diagnostic copies/dumps
  stay under ignored target/verification/ousama. No game bytes, fonts or keys
  are committed; no commit, push or CI was performed.
- Follow-up implementation: `lineParser.dll` now maps its native constructor,
  init/initStorage/getNextLine/parse/parseStorage and read-only line-number
  semantics to the shared resource `LineCursor`, with adapter-owned callback
  targets and GC cleanup. The complete public source/interface was reviewed at
  wamsoft/lineParser d8b6ec56810f1332f3d5e322986203630559791d. The shared cursor
  release test passed (mixed CR/LF/CRLF, blank lines, Unicode, byte budgets,
  reset and EOF); the native integration test is still compiling at this point.
  This plugin addition does not imply all plugins in OUSAMA are implemented.
- Additional real native evidence (player c4ccc1c7bf9c0254a449b617acc69cda8...):
  a diagnostic typed project using the verified 1920x1080 configuration and
  original shared archive mounts executed startup.tjs, Initialize, Config,
  Utils, KAGLayer, HistoryLayer, BGM, SE, Movie, Conductor, AnimationLayer,
  GraphicLayer, MessageLayer, Menus, DefaultMover and MainWindow. Execution
  then failed explicitly at Override.tjs:5, `Plugins.link("varfile.dll")`.
  Automatic play additionally exposed a missing ordinary UTF-16LE BOM decode
  in launch detection. That codec policy was extracted from the adapter into
  shared resource `text_stream::decode_auto`; automatic play awaits rebuilding.
  The diagnostic did not change original scripts or skip the varfile error.
- LineParser's native release integration test subsequently passed, including
  scoped UTF-8 input, callback target binding, reset/EOF and read-only line
  count. Varfile now has an implementation pending compilation/behaviour
  validation: portable TJS storage-media dispatch; dynamic global octet reads,
  ordinary text/binary writes, namespace traversal and image requests through
  shared encoded-image upload. Unmapped offset/append/compressed stream modes
  and property-backed writes fail explicitly. No varfile success is claimed
  until its authored binary/text/image integration test runs.
- Portable TJS media dispatch's targeted release test passed: handlers read and
  write current VM values, unknown media fails, duplicate registration fails,
  recursive media requests stop at the budget and later requests still work.
  `cargo check --release -p crysirror-krkr` completed successfully before the
  final synchronous-memory saveStruct guard was added. That guard's first
  native-test compile required an explicit Vec<Variant> closure argument;
  this was corrected, and the native varfile test is being rebuilt. These are
  distinct compile/test results, not a game-start success claim.

## 2026-10-30 Codex: RFVP feedback moved to shared presenter history

Implementation: core `MeshFrameHistory`/provider capabilities reserve client-owned
logical mesh-layer resources. Runtime rejects unreserved references; wgpu owns
bounded history textures, queue copies and once-per-declaration rendering.
Ordinary asset revisions remain dependencies. Explicit native thumbnails use a
separate readback fence. RFVP maps `Sprt(-1)` to this shared capability; absent
presenters keep the shared CPU reference. Desktop headless ticks advance the
same mesh renderer and final capture retains it. ADR 0098 records ownership,
completion and remaining blocking-save/movie limitations. No original renderer
is hosted or imported; VM data contains logical IDs/paths only.

Compilation: `cargo check --release -p crysirror-wgpu -p crysirror-rfvp` passed.
The first union desktop release build passed with
`rfvp,bgi,nscripter,krkr,arc-formats`; its binary SHA-256 was
`c4ccc1c7bf9c0254a449b617acc69cda87eb94e03b325cb1a8b8aa79dc352f49`.
The subsequent resource-revision guard was compiled in the engine-only GPU test.
Later Siglus/Circus/plugin additions require their own final union rebuild;
this paragraph does not validate those changes.

Behavior: explicitly executed release GPU test
`mesh_history_is_gpu_resident_scoped_and_advances_once_per_declaration` passed on
Apple M5 / Metal (one passed, zero ignored), including retained-declaration
idempotence, cross-layer latest-picture reads, mutable surface replacement,
client isolation, resize/reset and last-lease cleanup. No game/VM/font is used.
The first attempted fixture used an invalid generated-resource namespace and
failed before rendering; it was corrected, then rerun successfully.
Log: `target/verification/rfvp-perf-cursor/gpu-history-test-final.log`.

Actual RFVP testcase: the above player completed 600 native ticks offscreen in
0.488 s while other compilation was active. It submitted 600 mesh renders,
337 feedback copies and one explicit native readback, with native automatic save
slot 901 written. Final 1024x640 title RGBA matched the prior shared CPU reference
byte for byte (all channel differences zero). This is a functional preflight,
not a quiet benchmark or a measured window presentation rate. Evidence:
`gpu-preflight.{json,stdout,stderr,png}` and
`gpu-preflight-image-difference.json` in the same verification directory.
Python 3.11 tooling tests: 200 passed after refreshing RFVP source-provenance
hashes. System Python 3.9 could not import tomllib and is not counted as passing.
API generation at this intermediate stage passed seven tests with fingerprint
`2663a3650d0f8656`; new glyph/sampling/Circus public data additions will be
regenerated and logged separately.


## 2026-10-30 — Siglus native-host policy clarification and continued bring-up

- Owner policy correction: CrySirror does not execute the original Windows
  SiglusEngine.exe. Requiring its DVD authentication as a condition for this
  native host was an incorrect compatibility policy. Form 92.2 now validates
  and consumes its original three arguments/void ABI, records native once
  identity and explicitly reports disabled-for-native-host / not-applicable /
  authentication_performed=false. It neither fabricates a true return value
  nor creates original authentication data. Snapshot schema 10 stores and
  validates the selected host policy. Previous failed starts remain historical
  observations of the superseded policy, not a current DVD requirement.
- Implementation: SYSTEM file presence queries use client-scoped shared storage
  metadata with instruction retry, rather than reading entire resources or
  committing a pending result as false. SCRIPT font overrides and native
  Gameexe bold/shadow/fuchi defaults map onto shared GlyphRun coverage layers.
  Shared Text adds bounded circular outline and square-face alpha union; the
  existing weighted blur remains default. Renderer glyph resources/reveal
  remain engine-owned, and a pure engine example requires no Siglus VM.
- Compilation/tests so far: the decoration-stage release suite passed Scene
  29, Siglus 36 and Text 26 tests (91 passes / 4 ignored). Python native tooling
  passed 9. These runs preceded the later optical policy/character-voice
  additions and are not asserted as final verification of those additions.
  Explicit GPU/system-font validation is being coordinated separately; an
  ignored GPU test is not counted as a pass.
- Actual native behavior: a release device-free probe uses the real Runtime,
  SharedHost and unmodified detected projects/game roots. With the new optical
  policy, Rewrite HF completed 208 ticks and Plus 66 before both hit native
  SYSCOM.SET_CHARAKOE_VOLUME [63,186] at sys10_ginit00 line111. After implementing
  CHRKOE defaults/membership and actual shared Audio gain/stop routing, HF
  completed 209 ticks and Plus 134 before both hit [63,14], the native global
  extra-switch setter (lines142/140). These are real subsequent failures, not
  titles, game screenshots or completed Siglus compatibility. Extra switch/mode
  semantics are now implemented and await the next probe.
- Performance investigation: a local one-second CPU sample found heavy
  per-instruction SceneVm checkpoint cloning, especially immutable Gameexe
  strings/indexes. The metadata is now shared through Arc while mutable VM
  state and rollback stay independent; no speedup/FPS claim is made before
  measurement. Evidence, reports and private diagnostic copies remain ignored
  under target/verification/siglus-full-bringup. No game data/font/key, original
  executable execution, CI, commit or push was added.
- KaichoTrans implementation now grows shared scene/GPU capabilities rather
  than loading its DLL: additive `BeginCrossfadeEffect` commands describe
  bounded encoded-byte box-blur input transitions and grayscale/blur/inverted
  rule wipes. Static inputs use generation-scoped GPU snapshots; dynamic inputs
  are rendered every frame. Separable workgroup tiles read each source texel
  once, use exact horizontal sums and u32 vertical normalization, and retain
  GPU-local textures without per-frame CPU screen readback/upload. Radius,
  workgroup/format support and memory budgets are explicit; plain crossfades
  do not create the new compute pipelines.
- Source/compilation: complete KaichoTrans 0.2.5.1 Main/blur/dim source and its
  license were reviewed at the author-release SHA-256 recorded in
  docs/third-party/kaicho-trans.txt. Shared scene/GPU release checks passed;
  both filter shader variants and the final composite shader passed Naga
  parsing/validation. An Apple M5/Metal headless GPU run passed byte-domain
  blur, edge replication, static/dynamic lifetime and alpha-coverage assertions
  before failing a hand-calculated dim test expectation. The expected value
  was corrected to an independent u32 scalar kernel (x=1 still truncates to
  127; x=2 becomes 128), with no shader change to accommodate that failure.
  The stricter full GPU rerun is pending. No whole-game success is claimed.

## 2026-10-31 — Siglus shared glyph/clock validation and native title callbacks

- Implementation: shared Text `glyph_coverage` owns bounded disk outline and
  square-face alpha union; GlyphRun retains independent per-glyph resources and
  synchronized reveal. Scene `timeline_counter::Counter` now owns the former
  generic native integer clock/range/loop mechanism; the VM keeps its original
  callback/opcode and original-save policy. Existing Scene IntEvent remains the
  implementation behind native property offsets/events. Shared Image/Solid/
  Layer parents represent bounded 2D native PCT/RECT children and preserve the
  distinction between a child-transform origin and a payload's image center.
  Unmapped inherited 3D/filter/blend or non-default subtree ordering fails.
- Implementation: native character-voice groups, initialization switches and
  preferences now translate to saved VM policy plus shared Audio gain/stop and
  retained UI fullscreen requests. SYSTEM path presence uses capability metadata
  (including directories), not payload reads. Host focus is observed through the
  shared optional focus API; offscreen activity is an explicit host policy.
  Native frame callbacks now use the normal capability I/O rollback/retry fence,
  preventing resource requests from escaping their instance or ticking input/
  time twice on a missing-resource retry. The optical-host policy remains
  `disabled-for-native-host`, with no forged authentication result or original
  executable invocation.
- Compilation/CPU checks: the later release Scene suite passed 31 tests including
  the independent Counter test. The initial expanded native suite passed 44,
  failed one new offset assertion and ignored one fixture writer. That assertion
  incorrectly expected IntEvent.set_value to change cur_value immediately; the
  pinned/shared implementation commits it at the next native frame. The test
  now asserts the original 17px same-frame position and 22px after TIMEWAIT;
  the shared timing implementation was not changed to satisfy the assertion.
  The corrected Siglus/Text release run is pending at this record. Python 3.11
  tooling passed all 200 tests after refreshing the current 90-file Siglus
  SOURCE.json hashes; original source/licence hashes remain unchanged.
- GPU behavior: root explicitly ran the shared face/outline GPU test on Apple
  M5/Metal: 1 passed, 0 ignored; its per-glyph/reveal/color assertions passed and
  `shared-face-gpu.png` was inspected. A prior relative capture-path write failed;
  rerunning the same test binary with an absolute capture path passed. Root's
  desktop binary SHA-256 `749eae9d97993db912a3c1667e33ef813c7c48218baba9461158688e1612d067`
  also rendered the independent data project (30 frames) and authored native SCN
  decoration fixture (100 frames); both PNGs were inspected. These are shared
  API/native-fixture evidence, not screenshots of a working Rewrite title.
- Actual game behavior: device-free release probes use the real Runtime,
  SharedHost, ordinary detected plans and unchanged game roots. Rewrite HF and
  Plus now pass sys10_ginit00 and enter sys10_tt01/$FaMain. Probe13 completed
  210/136 ticks respectively before GLOBAL.GET_MWND; probe14 passes that getter
  and both stop at native MWND.BUTTON[0].GET_BUTTON_HIT_STATE, which the old
  parser misleadingly reported as a window-number error. HF has also read the
  original mn_mw_hint00 G00 cut through the callback resource fence. No real
  title, interactive menu or complete Siglus compatibility is claimed. Logs,
  reports and original-format diagnostic extracts stay ignored under
  target/verification/siglus-full-bringup; no game/font/key data was committed.

### OUSAMA native bring-up continuation (2026-10-31, local verification)

- Implemented: first-class KAG VM ExtKAGParser dialect (official 0.1.4.3 archive SHA-256 `c379325129ebc8b70f063b7bb650d1c180e89a4b991f43fd66c86962fb9dbc46`): native while/init/each, continue/break and structural scope errors; local dictionaries, `$` arguments, call/return local ownership; physical multiline tags, parameter macros, numeric macro arguments, macro default eval tags and macro iscript recording. Internal extension snapshots have a separate v2 appendix while classic v1 encoding remains unchanged. Native original-format quiescent save fields preserve local structured values and IgnoreCR. Fuzzy return, enableNP=true, Ext assign and active original call/macro/loop save mapping still fail explicitly. Original notices/reference retained in docs/third-party/ext-kag-parser.txt.
- Compilation: `cargo check --release -p crysirror-kag` and `cargo check --release -p crysirror-krkr` passed (`ext-kag-check.log`, `ext-native-check2.log`, `ext-native-finalcheck.log`). The KAG release suite passed 50/50 twice, including authored nested-loop, macro, excluded-effect, snapshot and multiline cases (`ext-kag-tests.log`, `ext-kag-tests-final.log`).
- Implemented: kropus/wuvorbis bind to the client-validated shared decoder capability; no DLL or COM module is loaded and no dummy TJS codec class is manufactured. Shared AudioSource/AudioResourceInfo expose actual ReplayGain metadata without silently applying gain; KRKR refuses tagged assets where original wuvorbis would require it. Optional native codec gain/format settings remain explicit unsupported requests. Pinned original module references are recorded in docs/third-party/krkr-audio-codecs.txt.
- Compilation: `cargo check --release -p crysirror-krkr` passed with the shared codec/metadata changes (`codecs-native-check.log`).
- Behavior: shared decoder test was explicitly invoked against locally generated 2.5-second 440-Hz Opus/Vorbis and tagged Vorbis fixtures: 2/2 passed, including decoding, seeking and real metadata (`shared-codec-tests.log`). The native WaveSoundBuffer -> shared worker/resource/mixer path produced nonzero PCM for both codecs and rejected tagged Vorbis before playback: 1/1 explicitly invoked ignored test passed (`native-codec-test.log`). Authored native LineParser/varfile/dirlist/Kaicho/ExtKAG tests passed 5/5, with 2 separate corpus/codec tests ignored in that run (`native-api-final4.log`).
- Behavior: the read-only six-XP3 installed corpus probe now passes the entire Override.tjs plugin loading chain and reaches construction of the real KAGWindow/BaseLayer. `native-shared-host5.json` records the new first error `k2compat.tjs:267 MemberNotFound(font)` during a Layer constructor injection. This is a TJS class/constructor ordering defect under investigation; no game script, PC, exception or plugin is skipped. GPU/title/gameplay success is not claimed. No evidence yet establishes that the omitted optical-media files are required.
- Earlier shared graphics verification finalized independently of this VM: original subtree crossfade GPU regression 1/1 and new subtree effects 2/2 passed on Apple M5/Metal (`gpu-final-test.log`); extensionless/misnamed standard-image magic test 1/1 passed (`image-magic-test.log`). Parent's later headless-tick capture regression is separate and has not been counted here.

### OUSAMA window construction, audio budgets and original text saves (2026-10-31)

- Implemented: native class-body initialization copies primary native members after installing native state; a hooked named constructor is not mistaken for the extends operand. Native `super.Class(...)` and direct native construction honor the actual replaced constructor member and retain it after forwarding to the original. These are general TJS/native-class semantics pinned to krkrz `fd5c4baa6a2ef5978db1bd043634351f48667daf`, not OUSAMA class-name patches.
- Compilation/behavior: `cargo test --release -p crysirror-tjs2` passed 422/422 (`native-class-tjs-tests2.log`). The first test attempt had a missing test import, fixed before the successful run. Supplemental optimized direct-rustc testing independently passed the same 422 cases. KAG final release tests passed 51/51 (`ext-kag-tests-final2.log`), including missing Ext macro arguments remaining void. Native direct/derived Layer constructor injection and Font creation ordering passed (`native-api-final7.log`).
- Implemented: common audio group capacity is bounded at 128 per client and 256 globally; the adapter's logical sound-buffer pool is bounded at 127, reserving its root group. Original game configuration asks for 100 SE buffers. An engine-only test verifies both capacities, client isolation and released-slot reuse (`audio-group-capacity-test.log`, 1/1 passed). The native 100-idle-buffer path now passes without invoking playback.
- Implemented: public resource `text_stream::TextWriteMode` and bounded `encode` own UTF16 BOM, original mode-1 adjacent-bit cipher and mode-2 zlib/64-bit LE lengths. KRKR only maps mode strings and retains atomic Save capability/continuation policy; `var://` uses the same codec. Offset/update and unknown modes still fail explicitly.
- Behavior: shared codec tests 4 passed/1 ignored (`text-write-tests.log`); the ignored authored-fixture exporter was explicitly invoked and passed 1/1 (`text-writer-export.log`). Independent Python `zlib` verified z0/z6/z9 headers, compressed/raw lengths and exact UTF16 bytes, including emoji/CRLF; it also checked plain and mode-1 bytes (`text-streams/external-verification.json`). The initial external checker normalized CRLF through Python read_text; using read_bytes.decode corrected that reference-only mistake.
- Behavior: latest native authored suite passed 8/8 with 2 separately invoked corpus/codec tests ignored (`native-api-final7.log`). This includes c1/z/z0/z9 Dictionary.saveStruct original-format output, successful return only after shared atomic commit, readback, and the memory-media codec path.
- Behavior: `native-shared-host8.json` advances the real read-only OUSAMA game through completed KAGWindow construction, GeneralFunc/ConfigFunc/BackLogFunc/OriginalFunc/AfterInit, local font registration and into original scenario/plugin execution. It completes 71 logical frames, then honestly fails at CFPlugin_Base.ks/CF_DB.tjs because `Scripts.execStorage(name,mode,this)` had ignored the original third context argument. Native storage exec/eval context mapping is now implemented using the existing VM execution API and has a new authored regression queued; that later change has not yet been counted as a pass. GPU/title/gameplay acceptance remains outstanding.

### Native Circus execution and shared capabilities (2026-10-31)

- Implemented: first-class `crysirror-circus` native0xabb6 interpreter retains original operands, labels/metadata, variable banks, branches/calls, resource/wait policy and explicit unsupported errors. A worker-backed thin adapter publishes shared CRX/PCK/cursor/image/layer/UI/audio resources; no original executable, renderer, mixer or composed-screen engine is hosted.0xaab6 remains parser-only and rejects execution.
- Shared implementation: image `pad_to_extent`; explicit scene ImageSampling with clamp/repeat/mirror addressing and a bounded UV contract; wgpu per-sampling texture aliases over the same base texture; owner-scoped `Ui.snapshotLayers` / logical UI snapshots; configurable text color/inset and native press activation; resource `PeData` safe read-only PE32/64 views, bounded RVA/VA slices and immutable SHA256; VFX `screen::pack_screen_emitter` converts existing immutable CPU particle snapshots into ordinary PuppetFrame quads with explicit atlas UVs. UI/particle snapshots are logical engine resources, not a private VM rasterizer.
- Native implementation: bootstrap reaches real name/mail input using shared TextField/Button/Dialog state, normal pointer events and empty-input confirmation; local CLEARNAME/CLEARMAIL preferences commit on a worker. The original `system.dat` two-block21368-byte preference format is imported/written separately, preserving unknown bytes and native read-range layout; ordinary game-slot saves and VM snapshots remain explicit unsupported. Music loop points are read from the exact recognized original executable as data, not guessed or shipped; source43C390 points are decoded frames. Native centibel gain is mapped into common client audio groups. Scene fades now use public externally-clocked crossfades so resource waits do not manufacture native render frames.
- Compilation: release Circus/runtime examples check passed, including the external-clock changes (`check-music-clock.log`). Native release unit tests passed16, with the separate installed-corpus test ignored in that invocation (`tests-settings-music.log`). The installed MES corpus test was then explicitly invoked against the original536 MES files and passed1/1 (`corpus-final.log`).
- Shared behavior: UI snapshot/text-style/press tests3, shared TextField tests4 and interaction test1 passed (`ui-snapshot-final-tests.log`); PE32/64/bounds tests2 passed (`pe-tests.log`); VFX screen packing tests3 passed, including independent common mesh CPU capture without a VM (`vfx-screen-tests.log`). Parent separately ran real shared image-sampling/frame-history GPU tests2/2 and the original20396-entry PCK voice range test; ignored GPU tests are not counted as passes here.
- Installed behavior: a fresh private Save capability plus authored ordinary input (click submit, then confirm empty fields) executes original START/WIN_SIZE through the complete playerinput callback, writes native preferences and enters MAIN. `headless-music1.log` reaches MAIN pc0x1193/op28 after loading TITLEBACK02/TITLECHIP01/TITLECHIP02/EFFSAKURA01 and preparing original music with its42 native loop records. This is an honest first failure at the not-yet-mapped falling-atlas particle effect; a complete visible/interactable title or story is not claimed. The subsequently checked native external frame clock has not yet been behavior-revalidated by this log. No original game process/window was opened and no files were committed.


### Shared native-host validation and Siglus scheduling correction (2026-10-31)

- Implementation: a Siglus host instruction quota now retains the current SCRIPT
  pass. It does not rerun native frame callbacks or consume another logical
  input frame until the original proc/wait boundary. The portable optical-host
  check remains explicitly not applicable; no disc-authentication result or
  substitute dummy file is generated.
- Compilation/behavior: `cargo test --release -p crysirror-siglus-vm
  scheduling_quotas_resume_one_native_frame_and_preserve_input_edges` passed
  1/1. The authored original-format scene compares 401 budget-one pumps with
  one pass, asserts exactly one native render-frame tick, restores a mid-pass
  framework snapshot and retains the queued down/up edges. The compiled release
  optical-policy regression was explicitly invoked and passed 1/1. Logs are
  `siglus-full-bringup/budget-continuation-test.log` and
  `optical-policy-current-test.log` under target/verification. Actual Rewrite
  title behavior after this scheduling correction is still being re-probed.
- Shared behavior: release GPU frame-history, sampling and subtree-effect tests
  passed on Apple M5/Metal (`rfvp-perf-cursor/shared-gpu-current-tests.log`).
  Headless logical ticks freeze static transition inputs before any screenshot.
  RGBA snapshot readback uses the actual source texture format; the new BGRA
  presenter channel-order regression and six existing canvas snapshot cases
  passed serially (7/7). The earlier parallel invocation had three profiler
  compute-pipeline validation failures and is retained separately; its cause is
  not established, and it is not counted as a pass.
- Shared behavior: client-scoped UI snapshot query 1/1 and live host-focus
  restore/query regression 1/1 passed. Public API tests passed 8/8 against the
  generated fingerprint d7986e975e6c66a8; API changes and Rust-only service
  contracts are recorded in docs/api/CHANGES.md and README.md.
- Compilation/installed behavior: the combined release desktop built with
  rfvp,bgi,nscripter,krkr,arc-formats,circus (binary SHA256
  67b57c83a5824170df5fc53ed0600d064f8d7bc928c10369285264f784f79a9b).
  An actual offscreen OUSAMA launch with that binary stopped at the old
  CFPlugin_Base/TxHELP context error; the binary predates the subsequent native
  context/focus fixes. It was not published as a completed integration or used
  to claim title success. No original executable/window, CI, commit or push was
  added.


### OUSAMA script contexts, focus and conditional lvalues (2026-10-31)

- Implemented: native Scripts.execStorage/evalStorage preserve the original third object context for source and existing bytecode execution; the default bootstrap remains the ordinary context. System event slots start as original null values and actual host-focus edges call the stored closure with its original binding. Native windowEx monitor queries preserve the pinned original HRESULT truthiness behavior for dictionary/zero-HWND nearest-monitor calls, while foreign nonzero handles fail explicitly.
- Behavior: authored Runtime/SharedHost native tests passed 11/11, with the separately invoked installed-corpus/codec probes ignored in that run (`ousama/native-api-final10.log`). Source/context, default event values, focus edges and monitor fallback have explicit assertions. `native-shared-host11.json` advances the unchanged installed game to 77 logical frames and fails honestly in EX_Func.tjs at its valid conditional assignment target; this report has no GPU rendering or title-success claim.
- Implemented: first-class TJS compiler accepts conditional lvalues, lowers only the selected address after the single right-hand evaluation, and retains native compound/update/delete/swap behavior. Original T_TYPEOF examines only direct member syntax, so a conditional reads its selected value before typeof. The complete pinned tjsInterCodeGen.cpp reference remains `fd5c4baa6a2ef5978db1bd043634351f48667daf`; no script or instruction pointer is patched.
- Compilation/behavior: the release TJS suite passed 425/425 (`ousama/conditional-lvalue-tests3.log`) without increasing the fixed decompiler fuzz failure budget. The first draft exposed one extra deterministic fuzz case; matching original typeof lowering removed it. An initial new missing-member assertion incorrectly used a Dictionary (whose absent key reads void); the corrected explicit-class assertion also passed in the supplemental optimized isolated-source test, 3/3 (`ousama/conditional-verified/tests.log`). The next native integration build/corpus run is pending at this record.

### Siglus native callbacks, COW checkpoints and object Media admission (2026-10-31)

- Implemented: WAKU image buttons use ordinary engine Image layers and shared
  interaction-only Widgets. Native TYPE/CALL/ACTION/SE, stable backing-image hit
  rectangles, callback identity and selection policy remain in the original VM
  adapter. Shared activation serials prevent duplicate callbacks across repeated
  pumps; native EXCALL suspends/restores its caller's wait in schema11.
- Compilation/behavior before the object-Media extension: the release native
  suite passed50/50 with one separate fixture exporter ignored
  (`native-current-cow-proc-tests.log`). This includes a real original-format
  WAKU callback with its own wait and mid-callback framework restore; original
  save-byte compatibility, per-table COW flag clone isolation and budget-one
  continuation are separately asserted. This result does not validate later
  object-Media changes.
- Installed behavior: after the scheduling/COW corrections the unchanged HF
  data executes220 logical frames, and Plus145, before actual
  FRONT.OBJECT[0].CHILD[1].CREATE_MOVIE120 at the title logo. These replace the
  earlier transparent-root600-frame result as the meaningful next blocker.
  Neither log established a visible title or complete game compatibility
  (`native-hf-probe21.log`, `native-plus-probe21.log`).
- Implemented next: native OBJECT movie declarations, validated-header admission,
  metadata/position/EOF mirroring, pause/resume and auto-free/Remove now call the
  shared Media service. Retained Image layers consume only actual engine picture
  receipts. Native movie ticking no longer advances a second private clock.
  A client-local reusable0..1022 Media identity pool reserves1023 for GLOBAL.MOV.
  Active decoder reconstruction in original saves/framework snapshots remains
  explicitly unsupported; unknown duration remainsNone. EOS/explicit FREE reset
  only original type-owned state, preserving children and render parameters.
- Compilation: the first object-Media `cargo check --release` passed
  (`object-movie-check.log`). The first full behavioral run passed50 tests but
  failed the new movie test on duplicate layer-tree removal: deleting a parent
  recursively removed its child before a second removal command. The adapter now
  removes preorder-assigned tree IDs in reverse order; shared Layer validation
  was preserved. Subsequent compilation attempts hit concurrent VFX/Scene API
  write windows; those failures are retained and not counted as behavior passes.
- Installed behavior limitation: probe24 used a binary linked before the Media
  identity correction and honestly failed at220/600 with `media identity`.
  It is not evidence for the corrected implementation. Its221-sample CPU profile
  reports native-step p50=13.07ms/p95=17.72ms; this is initialization-only,
  device-free evidence, not a GPU/FPS claim. Corrected movie/title validation is
  still pending at this record. Private game/font/key/extract data remains under
  ignored local verification paths; no original executable was run.

- Corrected object-Media compilation/behavior: the final release native suite
  passed51/51 with the separate fixture exporter ignored
  (`native-object-movie-full-tests3.log`). Its authored original-format movie
  scene waits at CREATE until actual metadata admission, does not create another
  decoder while pending, keeps metadata separate from a decoded picture, reads
  both intrinsic dimensions, publishes the shared Media surface at the original
  object transform, mirrors the engine position without advancing a private
  clock, retains unknown duration, and releases the decoder/tree on auto-free
  before permitting a framework snapshot. This is a deterministic service
  contract test; it does not replace installed-game/GPU evidence. The latest
 95-file source hash manifest and dedicated Python native-boundary tests passed
  9/9 (`python-siglus-object-movie-final.log`).


### OUSAMA title script and shared frozen group opacity (2026-10-31)

- Implemented: ExtKAGParser.assign maps complete supported VM cursor/call/loop/macro state, retains destination owner switches, shallow-copies the original LocalVariables array/parameter-macro dictionaries, and allocates independent active macro-argument dictionaries. Ordinary publication retains unchanged shared parameter-macro identities. Native source operator= and TJSAry/TJSDic assignment were inspected in the pinned 0.1.4.3 archive. KAG release tests passed 52/52 (`ousama/ext-assign-kag-tests3.log`), including destination interruption/switch retention and source continuation after clearing the copy.
- Implemented: native ltAddAlpha resources already use the adapter's canonical straight-alpha representation, so piledCopy now permits that display type through the same public Canvas CopySubtree operation as ltAlpha. An authored half-transparent child exposed a real shared group-opacity gap: scene-owned subtree freezing now folds a clipped partial-opacity group into an immutable nested Paint::Snapshot and applies group alpha once, after child composition. Unbounded local groups and excessive snapshot nesting still fail explicitly. No composed-screen upload or VM compositor was added.
- Compilation/behavior: shared scene release check passed, and the release scene suite passed 33/33 including two new engine-only isolation/source-ownership/unbounded-group cases (`ousama/group-opacity-scene-check.log`, `group-opacity-scene-tests2.log`). The new ignored GPU regression `frozen_canvas_composes_overlapping_children_before_group_opacity` is pending explicit execution at this record; it is not counted as a pass.
- Implemented: native Font is a regular first-class facade using the existing shared text measurement/draw path and per-VM rasterizer preference. frGDI/frFreeType map explicitly to the portable outline compatibility backend and log that native GDI/FreeType raster pixels are not implemented. This is not a Windows GDI backend or a pixel-identity claim; unknown selectors and unsupported bound Font(layer) requests fail explicitly.
- Compilation: `cargo check --release -p crysirror-krkr` passed with the Font and shared group changes (`ousama/font-group-native-check.log`). Native run16 passed 13/14 authored cases; the remaining newly added piledCopy fixture had left its logical Window invisible, so no frame was extracted. The fixture's visibility is corrected and its rebuilt test run is pending. The earlier failed fixture had an omitted endwhile, corrected before its assign case passed; neither failure was bypassed in engine code.
- Actual installed behavior: `ousama/native-shared-host16.json` completed 1200/1200 real Runtime/SharedHost updates (1132 native frame passes), reached title.ks/*title_menu, and stopped on the original [s] command without an uncaught exception. It reports 236 retained layers, 1016 engine API calls, 18 real facade plugin registrations, no plugin shells, owns_framebuffer=false, no pending shared tasks and no active VM suspension. Original game resources/scripts remained unchanged. This is device-free execution evidence; visible GPU title and menu interaction remain separate acceptance work.
- Known behavior gap: original automatic-bookmark thumbnail saving reaches unsupported saveLayerImage; the game catches the error and calls System.inform, currently represented by a log. The probe therefore does not establish successful thumbnail/bookmark output. Compressed text offset-update writes and exact original active Ext parser saves also remain unsupported. No omitted optical-media file has yet been shown necessary for this startup path.

- Installed corrected-build behavior: HF probe25 reaches original
  sys10_tt01:795/pc94369,519 logical owner ticks; Plus reaches
  sys10_tt01:888/pc83689,434 ticks. The first movie declaration and PAUSE125
  now pass through shared Media; both fail honestly on the later logo's
  CREATE_MOVIE real_time=false requirement. The native original trace confirms
  named argument1=0 for mn_tt_rlogo00; it is not inferred from a filename.
  Real/trace runs can spend different logical tick counts awaiting asynchronous
  metadata, so these counts are not deterministic instruction-progress or FPS
  measurements. HF publishes78 retained layers and Plus80; evaluating visible
  and opacity through all ancestors yields only one transparent Solid and no
  visible Image in either capture. No GPU/title success is claimed. Shared
  decoder reservations/queued video and the current native PC are preserved in
  native-{hf,plus}-report25-boundary.json. The next common capability is an
  explicitly sourced media clock; the VM must not add another playback timer.


### OUSAMA final native regressions and offscreen title evidence (2026-10-31)

- Compilation/behavior: the final release native integration binary built (`ousama/native-api-build18.log`) and all 14 authored Runtime/SharedHost cases passed, with 2 separately invoked installed-corpus/codec cases ignored in that ordinary run (`native-api-final18.log`). The half-transparent ltAddAlpha child is retained as a nested shared snapshot: pixel alpha128 remains inside and native group opacity128 is applied outside, even after the live source changes. The drawing-only fixture explicitly selects province hit testing; the default alpha-mask query on a nonuniform canvas snapshot remains unsupported, and is not counted as an input-capability pass.
- Behavior: root explicitly ran the combined release desktop player SHA256 `2a6fb5607253667acef5708c76e44f14174ebda8c16d6f16fa5f635c33e56668` offscreen for 1200 frames against the unchanged installed game and inspected `ousama/title-player.png`: character/title artwork and CONTINUE/START/LOAD/EVENT/CONFIG/EXIT are visible. Reported native execution is1184 frame passes,1016 API calls and236 retained layers; Metal on Apple M5, window_created=false, owns_framebuffer=false, plugin_shells=[]. This establishes a rendered native title, not completed gameplay or bookmark saving. Normal pointer activation of START is separate verification still in progress at this record.
- Shared GPU behavior: root explicitly ran all 8 canvas snapshot tests serially on Apple M5/Metal, including `frozen_canvas_composes_overlapping_children_before_group_opacity`; 8/8 passed (`ousama/group-opacity-shared-gpu-test.log`). The new overlap test asserts group alpha128 across overlapping opaque children, the expected sRGB output, clip bounds and immutability after source mutation. The prior pending/failed authored-fixture records above are superseded by this successful final run; their logs remain available.

### Siglus source-clock selection through shared Media (2026-10-31)

- Implemented: ordinary FRONT object movies sample existing native
  local_game_time/local_real_time using shared Media.SetClock. The engine owns
  start/position/loop/seek/pause compensation; no playback timer was added to the
  VM. Stage time-stop and the object's native pause flag form an overlay without
  mutating that flag. Native Ctrl/read skip selects32x game delta and1x real delta
  with the original Ctrl-disable/Shift/focus precedence. Movie metadata admitted
  from a native frame callback retains the existing event/SCRIPT continuation
  ownership instead of ticking the callback again. EXCALL/MWND movie locations
  and negative source clocks fail explicitly.
- Behavior/compilation: the release native suite passed53/53, one separate
  exporter ignored (`native-media-clock-tests.log`). New assertions cover native
  game/real clock selection, skip precedence, stage pause ownership, clock samples
  while blocked in TIMEWAIT, and sample-before-pause/resume request ordering.
  All original save/EXCALL/WAKU/quota/COW regressions remain in the passing suite.
  Updated95-file current source hashes and dedicated Python native boundary
  tests passed9/9 (`python-siglus-media-clock.log`).
- Installed asset evidence: a bounded, read-only Ogg structural inspection of
  both game directories found the related5 logo OMV files contain one complete
  Theora stream each, with contiguous page sequence and BOS/EOS; no Vorbis,
  Opus or unknown stream was present (`logo-ogg-layout.json`). This is a
  container/audio-presence check, not image fidelity or game completion.
  External-clock sources with decoded audio must fail visibly until a shared
  audio-master mapping exists; disabling game audio to hide this requirement
  was not used. Real-game behavior after the new clock mapping is pending.


### Shared screen VFX and native Circus execution progress (2026-10-31)

- Implemented in shared engine crates: CPU screen-sprite declarations publish
  ordinary owner-scoped dynamic Puppet layers from the existing VFX worker.
  The reusable falling/atlas node uses explicit integration ticks and wall time,
  a serializable integer sequence, bounded atlas rows and bottom cropping.
  Completed worker receipts and fenced random-sequence exchanges keep the
  submitted declaration distinct from completed simulation state. Empty screen
  declarations avoid cloning the entire layer tree. Existing/foreign targets,
  unsupported GPU modes and active screen-producer snapshots fail explicitly.
- Implemented in the thin Circus adapter: native28 type1 density0..5 declares
  five ordered depth emitters in one shared system; native25 borrows the same
  process stream and waits for exchange acknowledgment before assigning99/2
  and recording original random history. The virtual millisecond seed is fixed
  at adapter startup, before I/O latency. Native1e/19 performs the original
 356324-byte save-prefix availability probe on a capability worker; it does not
  load original slot saves. Native78/32 preserve group movement/flag data;
  they do not pretend to activate or animate an unsupported group.
- Compilation: release Circus/example metadata check passed
  (`native-effects-check.log`). The latest release native library tests passed
 20 with one installed-corpus test ignored (`native-effects-final-tests.log`);
  that ignored item is not counted as a pass here. Both actual headless examples
  compiled and executed with the normal runtime, without window/audio devices.
- Shared behavior: final VFX tests passed8 library tests and4 CpuWorld tests,
  including one stream across emitters, independent external clocks, completed
  tick accounting, fenced exchanges and birth/reset atlas cells
  (`vfx-falling-final-tests.log`). Runtime screen publication/ownership/cleanup/
  snapshot gates passed5 tests (`vfx-publication-final2-tests.log`); the separate
  default-budget/client-pause receipt test passed1
  (`vfx-sequence-runtime-tests.log`). These are device-free behavior results.
- Installed behavior: fresh private save storage plus ordinary submit/empty-field
  confirmation input executes original START/WIN_SIZE into MAIN. The first probe
  passed native28 and stopped at the then-unmapped save-availability query.
  The corrected second probe reaches MAIN pc0x14f9/op0b at frame806, where title
  group activation still fails visibly (`headless-effects2.log`). The completed
  engine snapshot contains5 emitters/60 particles and five owner1 dynamic mesh
  layers, each with12 parts and one texture. Tick0/epoch0 random state3672816812
  is the completed shared stream after540 draws from virtual startup seed16.
  This establishes actual simulation/publication, not a visible/interactable
  title, story, GPU rendering or60-FPS result.
- Remaining boundaries: title group activation, its20-native-frame layer/UI
  slide and original button policy; immutable freezing of dynamic mesh producers;
  storm first-birth/reset differences, other/concurrent/replaced effect pools,
  nonempty native text/markup and original slot-save reconstruction. No native
  executable was run, no game/font data was committed, and no commits were made.


### 2026-10-31 — shared media clocks, VFX acknowledgements and verified desktop title

- Implementation: MediaInfo/MediaStatus now publish validated display size before
  the first picture and declared duration where available. OMV packet/page hints
  remain unknown duration. Incremental ASF/Theora header callbacks precede frame
  publication. External media clock samples retain engine-owned origin, pause,
  seek and loop offsets; backward samples fail. Decoded audio plus an external
  clock explicitly rejects until audio-master mapping, rather than desynchronizing.
  Remove releases the scoped resource/surface; Stop retains reconstruction debt.
  Retained resource bounds are now16/client32/Runtime, with unchanged128MiB video
  and128MiB audio byte budgets and bounded cancellation leases.
- Implementation: VFX completion queries validate entity lifetime, authored-source
  fingerprint, revision, completed tick and exchange epoch. Declared RNG seeds are
  never returned as ready work. Both screen producer and sequence/exchange
  snapshots explicitly reject missing worker/resource reconstruction. Python
  authoring validation now accepts the actual serde defaults and enforces the
  supported screen/clock/sequence/falling profiles without rewriting declarations.
- Compilation/behavior: shared Media release suite passed18, ignored1 separate
  owner-corpus case (`shared-media-external-clock-tests.log`); public Media
  resource/clock/remove object tests passed2/2 (`shared-media-clock-object-api-tests.log`).
  The earlier shared metadata suite passed16/1ignored; independent real HF logo
  OMV decoding with header-before-picture assertions was explicitly invoked and
  passed1/1,0ignored (`shared-real-logo-metadata-test.log`). No game media was
  copied into the repository. The newly increased capacity test is queued.
- Behavior: VFX object API really waits for CPU worker initialization and exchange
  acknowledgements, rejects same-revision stale declarations, and rejects both
  pending and completed sequence snapshots:1/1 passed
  (`vfx-sequence-snapshot-gate-test.log`). Public API generation/tests passed8/8
  at fingerprint4094eacf6dc444ee; earlier validated stages9e71ed5151241ed2 and
  497d019748fb64d4 remain separately logged in docs/api/CHANGES.md. Python3.11
  full tooling passed201/201 before the last native-only edits; default Python's
  missing tomllib was an environment error, not a passing run.
- GPU behavior: shared canvas snapshot suite was explicitly invoked serially on
  AppleM5/Metal and passed8/8,0ignored, including frozen group opacity over
  overlapping children (`ousama/group-opacity-shared-gpu-test.log`). The combined
  release player built successfully (`ousama/title-union-player-build.log`), SHA256
  2a6fb5607253667acef5708c76e44f14174ebda8c16d6f16fa5f635c33e56668.
  Its ordinary detected offscreen OUSAMA launch rendered the actual1920x1080
  title image; root inspected characters, logo and all six menu items. The report
  records window_created=false, owns_framebuffer=false, plugin_shells=[], retained
  engine Layer/Canvas rendering,236layers and1016engine calls. This is real GPU
  title evidence, not a hosted framebuffer or a VM-only probe. Four managed local
  launchers now point to that player; their previous749eae9d build is retained.
- Installed input behavior: the same player consumed ordinary pointer down/up
  on START and reached original0_1.ks after loading bg5_0 and the main-window
  frame. It stopped honestly at a typed MacroArgs value being overwritten as a
  String (`ousama/start-player.log`), under active KRKR semantic repair. Bookmark
  thumbnail save remains unsuccessful and is caught/informed by the game; no
  complete save, story or windowed-menu acceptance is claimed. All evidence is
  ignored under target/verification. No original executable/window, CI, commit
  or push was created.

### Siglus sound-category and shared capability name lookup (2026-10-31)

- Installed evidence: clock-enabled direct device-free probe27 reaches HF
  sys10_tt01:812/pc95478 and fails on PCMCH[0].READY("mn_ss_se31",volume_type3).
  Trace28 confirms this is the ordinary SE category with no character or duck
  request. Its real source is wav/MN_SS_SE31.nwa, while the detected plan has no
  hand-authored aliases. Plus reaches sys10_tt01:927/pc87405 and honestly rejects
  its fifth retained object movie at the old4/client cap. Both reports retain
  zero effectively visible Image layers; partial initialized media or zero
  unknown-opcode counts are not a completed title. Shared Media cap changes are
  separately owned/validated by the parent task.
- Implemented: reusable Resource capability_lookup resolves ASCII component
  names using only an already scoped ClientStorage directory capability. It
  reads no payload and constructs no ambient path; canonical validation,
  ambiguous-name errors,64-component depth and65536 aggregate-entry bounds are
  explicit. Siglus Movie and sound facades both use it while retaining their
  own native directory/extension/append policy. Ordinary native wav/bgm names
  no longer require manual aliases; numeric voice packs, nonempty append policy
  and implicit dual-source PCM/BGM reconstruction remain bounded limitations.
- Implemented: PCMCH default system-gain category3 joins category2 through shared
  channel audio. Per-channel script gain has no GLOBAL.PCM script-volume group,
  preserving the pinned source's independent terms. Unsupported character,
  system-gain and duck semantics continue to fail explicitly instead of dropping
  factors. No mixer, decoder, fader or playback clock was added to the VM.
- Compilation/behavior: release Siglus check passed
  (`audio-capability-check2.log`); the initial helper check required a mutable
  HostServices reference for capability acquisition and was corrected before
  the successful run. Resource lookup tests passed2/2 without opening files
  (`shared-capability-lookup-tests.log`). Native release suite passed54/54 with
  the separate exporter ignored (`native-audio-routing-tests.log`), including
  independent PCMCH gain and unmapped-gain rejection. Current96-file source
  hashes and native Python checks passed9/9
  (`python-siglus-audio-resource.log`). Installed audio behavior after this
  mapping is being checked with the ordinary Runtime/SharedHost probe29.


### CXDEC clear-bootstrap recovery and Natsuzora archives (2026-10-31)

- Diagnosis: raw TPM facts expose a control block but no complete generator.
  The previous bitperm route could expose the two Classic LCG words while still
  leaving invalid instruction bytes. Static IDA analysis of the clear link
  bootstrap established current-state byte selection, a different four-case
  permutation and arithmetic signed state shift; the older route uses an
  unsigned shift before selection. A separate Classic PRNG instruction window
  is needed when no callback descriptor/newer lane-manager anchor exists.
- Implemented in the shared XP3 recovery layer: bounded clear-loop/backedge,
  four-case table, recurrence and seed-instruction recognition; addresses and
  seeds come from the supplied module. Complete control/dispatch grammar now
  validates decoded candidates, rather than two constant words. Previous v1
  support remains. No title name/hash/key/table or fixed game RVA selects a
  profile; no original module code or Unicorn fallback executes.
- Compilation/behavior: the release CXDEC library suite passed19/19 with no
  ignored tests (`recovery-regressions.log`). The final seed-operand-boundary
  hardening was additionally verified against the same production modules by
  six release regressions, including varied seeds/case permutations, old v1,
  wrong shift, duplicate cases, missing descriptor and LCG-only rejection
  (`standalone-final-regressions.log`). Original TPM alone recovers one candidate
  that verifies all32 real protected samples; combined TPM/EXE facts yield eight
  candidates with exactly one32/32 match (`tpm-only-checksum.log`,
  `current-final-checksum.log`). No recovered parameter values are published.
- Installed resource behavior: explicitly invoked the existing ignored CXDEC
  integration test with the original directory/modules; it passed1/1 with zero
  ignored tests (`actual-archive-tests.log`). All10 XP3 archives opened through
  the normal ArchiveFormat/DecodeContext path:25,995 entries,25,985 protected.
  Stride97 reads verified273 entries with zero failures,13,466,275 bytes;
  startup.tjs is read from original data.xp3. This is archive/filter evidence,
  not a full game, title-screen, rendering or frame-rate claim. The complete
  player build/launch is coordinated separately by the root agent. Diagnostic
  binaries, source snapshots and private samples remain under ignored target/;
  no game executable was run and no commit was made.

### Siglus DISP native display boundary (2026-10-31)

- Installed evidence before this mapping: probe29 passes the previous audio and
  media-capacity walls. HF prepares two real NWA-backed READY channels and stops
  at sys10_tt01:823/pc106651 on GLOBAL.DISP6; Plus retains five actual decoded
  paused media pictures and stops at sys10_tt01:953/pc99393 on the same command.
  Channel audio peak remains0 because those sounds are READY; audible playback
  has not been claimed. Title containers are still hidden at this early boundary.
- Implemented: pinned native GLOBAL.DISP calls wait_next_frame(render_frame) and
  selects ProcKind::Disp. Existing engine-owned layer publication runs at the
  adapter's proc boundary; no legacy display/renderer is called. Native FRAME
  retains its separate kind, and host quota continuation remains distinct.
- Compilation/behavior: native release suite passed55/55, with one separate
  exporter ignored (`native-disp-tests.log`). A self-authored original-format
  scene publishes a red engine Rect at DISP/frame1, asserts pump_continuation
  isfalse and proc kind isDisp, then produces a blue Rect/EOF only on the next
  native frame2. Current96-file source hashes and native Python checks passed
  9/9 (`python-siglus-disp.log`). The next installed-game/GPU run uses the parent
  task's combined player; source is frozen for that build. Full title/menu/story
  compatibility remains unverified at this record.


### Circus original system preference reload26 (2026-10-31)

- Matched the reported original START failure exactly: native pc0x2b1d has
  bytes18/1a/00, opcode18 selector26. It is40EB40 system.dat reload, entered
  on a later boot when the original100/10 preference flag is set; earlier
  fresh-state probes skipped this branch. This was a real missing handler,
  independently reproduced with the prior binary and copied existing private
  preferences at frame41 (`reload26-before.log`).
- Implemented: the native instruction waits for a client-scoped Save worker,
  then replaces both validated original blocks and configured-name buffers.
  Runtime script variables/names/PC remain distinct. A missing file follows
 40E9C0 full first-block reset while preserving the separate2040-byte source
  block. The reset's retained-name music replay uses the existing shared Audio
  path; an empty name emits nothing. No instruction is skipped and no shared
  renderer or second engine is added.
- Compilation/behavior: release Circus library tests passed22 with the separate
  installed-corpus test ignored (`reload26-tests.log`). The new authored tests
  cover pending-PC stability, loaded unknown bytes/names, ordinary script state
  preservation and missing-file reset/source-tail behavior. Further inspection
  of43CE40 corrected its retained-name music replay; final validation of that
  additional branch is recorded separately below.
- Installed behavior: the rebuilt release headless example uses the same
  copied existing preferences, passes START18/26 and follows original boot
  control flow into MAIN; this saved-state run needs no new profile input.
  It next stops visibly at native title-group
  activation, pc0x14f9/op0b, frame786 (`reload26-after.log`). This fixes the
  reported reload wall; it does not establish a working title/menu/story.
  Original files were not changed, no native executable/window was opened,
  no Core/Runtime/public object API changed, and no commit was made.

### Siglus object events over shared Scene curves (2026-10-31)

- Installed evidence: the combined player's normal detected headless launch
  passes DISP for both installed Rewrite directories, then honestly rejects
  native object chain[72,0], OBJECT.TR_EVE.SET (`gpu-hf.log`, `gpu-plus.log`).
  That player wrote no PNG; this record does not claim a visible title.
- Implemented: ordinary object property event targets map SET/SET_REAL,
  LOOP/TURN and real variants, END and CHECK onto the existing public Scene
  IntEvent. Native field/subop/named-start policy remains in the adapter;
  interpolation, looping and time-domain mechanics remain engine code. Retained
  layer publication now samples existing event totals, including completed
  one-shots. PCT/RECT/movie creation synchronizes X/Y/PATNO through native
  setters, preserving static positions and event-backed getters. Event WAIT/KEY,
  parameter slots and offset-event-list suboperations still fail explicitly.
- Compilation/behavior: release native suite passed56/56, one separate exporter
  ignored (`native-object-event-tests2.log`). Its authored native Rect fade uses
  a shared curve, keeps position10,20, observes opacity0/127/255, and after a
  mid-fade snapshot returns opacity191 at75ms. The first suite run found the
  hand-made parent/child fixture had bypassed the native event backing by
  writing base fields directly; converting that fixture to the same native
  setters preserved its existing geometric assertions and all tests passed.
  Current97-file hashes and dedicated Python checks passed9/9
  (`python-siglus-object-event.log`). Next real-game render is delegated to the
  combined player owner; no additional renderer or runtime-example build was
  introduced for this mapping.


- Circus reload26 final follow-up: inspection of43CE40 corrected the reset's
  retained-name music action to looping replay, not an unconditional stop.
  Empty names emit no request. A previously nonlooping resource acquires its
  actual loop metadata before replay; the completed storage result is retained
  while that second worker is pending. Final release tests passed23 with the
  separate corpus ignored (`reload26-corrected-tests.log`), asserting one reload,
  stable PC/preferences while pending and the prepared100..200 authored loop.
  The latest rebuilt example again passes the user's existing-preferences18/26
  path and reaches MAIN pc0x14f9/op0b, at headless frame803
  (`reload26-fenced-boot.log`). No working title or original slot save is claimed.


### 2026-10-31 — published reload26 fix and current installed-game boundaries

- Compilation: combined desktop release build passed in
  `circus-execution/reload26-player-build.log`. All four managed local launchers
  now have SHA256
  `09c492c843372cec49f5ea5acd0344f433730e7bdb47cded2f2e7fb174df1f80`.
  The prior title player remains retained under ignored verification files.
- Behavior: the normal detected DC4PHDL desktop player, offscreen and with a copy
  of the same existing original system.dat, passes START pc0x2b1d/op18 and
  reaches MAIN pc0x14f9/op0b (`circus-execution/desktop-reload26.log`).
  The next original title-group/menu instruction remains under implementation;
  the reported reload instruction is fixed, not skipped. Original game data
  was not modified and no title/menu completion is claimed here.
- Implementation/behavior: KAGParser is registered as the original built-in
  class, also without a DLL link; the independent native parser test passes1/1
  (`natsuzora/builtin-kag-test.log`). The same published player opens Natsuzora
  archives and loads the original KAG system, then stops at custom.ks line2
  eval with an explicit range error (`natsuzora/builtin-player.log`). Archive
  decryption is no longer that launch's blocker; its title is not verified.
- Implementation: restore-resource presentation/cursor requests now mutate
  staged engine state only. Unknown clients and arbitrary scene mutations are
  rejected during reconstruction. Release Runtime suite passed70 with one
  separate corpus case ignored (`siglus-full-bringup/runtime-staged-restore-final-tests.log`),
  including all previously failing Siglus restore cases. Current Python3.11
  tooling passed201/201 (`natsuzora/python311-current.log`).
- Installed Siglus behavior: a real offscreen HF capture now displays its
  opening film through shared Media/Image/Layer; root inspected
  `siglus-full-bringup/trace-hf-movie.png`. Native PLAY_WAIT(op00) is unskippable
  and still waiting for actual decoder EOS; a fast logical frame budget is not
  wall time or finished media decoding. This is opening-video evidence, not a
  verified title/menu or full compatibility claim. New Plus indexed offset
  events have separate native tests and await the next combined player.
- No original executable, window, CI, commit or push was created.

### Siglus indexed events and actual HF opening movie (2026-10-31)

- Installed evidence: combined player09c492c843372cec49f5ea5acd0344f433730e7bdb47cded2f2e7fb174df1f80
  reaches Plus's native Y_REP_EVE[0].SET chain[113,-1,0,0]
  (`event-plus.log`). HF produces a black1800-tick capture, but its retained
  graph has one effective Image: the top-level GLOBAL.MOV surface1280x720.
  Title/MWND image children remain hidden by their native ancestors; this was
  not evidence that a visible native title was discarded by the renderer.
- Implemented: indexed X/Y/Z/TR event-list targets share the same native
  subop facade and existing engine IntEvent used by ordinary property events.
  New read-only wait/movie/proc/frame diagnostics expose scheduling intent
  without polling a wait, consuming input or advancing time. Event WAIT/KEY and
  parameter-slot semantics remain unsupported; no private tween was introduced.
- Compilation/behavior: the release native suite passed57/57 with the separate
  exporter ignored (`native-offset-events-tests.log`). An authored original
  scene animates a Y offset from20 to25 to30 and restores the50ms state to27
  at75ms; repeated diagnostic queries preserve its wait. Current97-file hashes
  and dedicated Python native tests passed9/9
  (`python-siglus-offset-event.log`). Sources are frozen for the next combined
  player; the indexed-event mapping has not yet received its real Plus run.
- Installed movie behavior: original command tracing identifies
  GLOBAL.MOV.PLAY_WAIT("op00"), opcode2, with no key skip. No DECIDE input,
  script/PC edit or synthetic EOF was used. The earlier1800-tick black image
  observes only1.440s of media clock/1.050s picture timestamp, not28s of movie
  playback. The same player at2400 ticks renders an actual nonblack1280x720
  opening frame: blue sky, flower field and two characters
  (`trace-hf-movie.png`, inspected locally); media position12.848s, picture
  timestamp13.229s,353 published pictures. It remains the opening film, not an
  interactive title. The only rendering path is shared Media -> engine Image.
- Independent read-only ffprobe metadata identifies mov/op00.mpg as MPEG1 video
  plus MP2 audio, container duration99.576s
  (`hf-op00-reference-metadata.json`). That external duration is verification
  data only and was never used to manufacture runtime completion. A later
 10000-tick observation still has true decoder progress at picture38.721s while
  the fast host clock is130.816s (`hf-opening-eos.png` / summary); logical tick
  counts therefore cannot establish that the decoder reached EOF.
- A single subsequently bounded natural-wait run ends on explicit native
  CGTABLE form78 chain[78,3] (`hf-opening-natural-eos.log`) before its90-second
  wall bound. It emits no PNG or final Runtime snapshot, so this error alone
  does not verify movie EOF or a completed title. CGTABLE was not altered or
  bypassed. No further run or build is active; root owns the next combined
  player check while these sources remain frozen. No commit was made.

### 2026-10-31 OUSAMA native macro values and original Ext saves (Codex)

- Implemented: ExtKAG macro argument publication preserves typed dictionaries,
  arrays and alias identity through nested macro calls/returns. A separately
  maintained native-save translator maps original active call-stack buffers,
  UTF-16 positions, argument pools and local-variable ownership into the
  first-class KAG VM; it does not add an opaque extension to game save data.
  Original label replay and onScenarioLoaded ownership are preserved.
- Compilation: release `crysirror-krkr` check passed
  (`target/verification/ousama/native-save-final-check.log`). The release KAG
  suite passed54/54 (`native-save-kag-tests2.log`). The current authored native
  SharedHost regression binary passed18/18, with three separate corpus/font
  cases ignored (`font-native-regressions.log`).
- Behavior: actual ordinary START input in the installed OUSAMA corpus reached
  `0_1.ks`, loaded its background/character/window resources and completed the
  native `saveAnywhere.ks` call/return path. The device-free native25 replay
  stopped at the first dialogue's missing shared default font after1566 ticks;
  it is evidence of that native control path, not a completed game/render run.
- Limits: original active while-loop saves and unsupported active macro states
  fail visibly. The game's caught automatic thumbnail-save error remains a
  limitation; no successful thumbnail/bookmark output is claimed. Dynamic font
  registration is being implemented separately; its final game replay is not
  part of the native25 result above. No game data/fonts, window, CI, commit or
  push was created.

### Circus process11 and reusable Layer/UI motion/coverage (2026-10-31)

- Implementation: owner-scoped retained Layer/UI motion tracks use shared
  `ScalarTween`, with explicit external caller units or automatic engine
  milliseconds, typed query/status, monotonic elapsed, transactional rollback,
  serialized logical state, Put/Remove cancellation and track quotas. Linear
  progress retains f64 precision until its final coordinate conversion. UI
  immutable `WidgetHitMask` samples engine-owned alpha coverage, supports
  explicit strict local edges and threshold, and retains normal keyboard
  behavior. `ImageResource::alpha_region` performs bounded checked extraction;
  UI masks have per-resource, owner and global byte limits. The native VM
  performs neither interpolation nor pixel hit sampling.
- Native implementation: DC4PH0x0b now finalizes its real group and waits in
  process11. Shared motion drives the original20-tick slide and15-tick hover
  scale. Native34[0] belongs to each button record; shared masks use the first
  normal-state image's source-cut origin and alpha>=1. Input helper442D60 /
  4428E0 confirms release activation; retained shared UI events preserve that
  action through asynchronous preparation. Only a real action resolves a MES
  label. Keyboard/controller/page navigation remains explicitly unsupported.
- Compilation/behavior: `cargo check --release -p crysirror-scene` and
  `cargo check --release -p crysirror-circus --features runtime` passed.
  `motion-alpha-final-tests.log` passed9/9, including20 exact coordinates,
  parent motion before hit layout, owner isolation, rollback/restore,
  transparency pass-through, mask validation and delayed event consumption.
  `menu-native-tests.log` passed24 with one separate installed corpus test
  ignored; the authored process11 test cannot fall through on a raw click and
  resolves its supplied real label only after engine activation.
- Installed behavior: using a private copy of the existing original preferences,
  the release headless example waits for completed shared UI motion and presses
  an actual covered pixel at1014,201 on frame830; release activates the native
  `prologue` action. Execution then fails explicitly at MAIN pc0x1512 opcode13
  on frame838 (`menu-title1-boot.log`). No PC, VM flag or native event was
  injected. Inspection also found SCENE_ROOT still hidden by the prior fade,
  so this interaction result is not a verified visible title screenshot.
- Follow-up implementation: original49→425C20→41F2B0/407BC0 explicitly clears
  current UI groups and the old transition draw state. A native ResetScene
  event now maps that cleanup to shared layers/canvas/menu state before new
  background publication, correcting the stale hidden scene/color overlay.
  Its new release fixture and actual offscreen verification are pending;
  later native13, complex animations and original slot restoration remain
  unsupported. No original exe was run and no commit/CI was made. Root owns
  public API generation and its integration validation.

### 2026-10-31 Shared dynamic fonts and OUSAMA dialogue (Codex)

- Implemented: device-free `FontLibrary`/font job and immutable resource handles;
  bounded client-scoped storage I/O, CPU outline/shaping preparation, and atomic
  publication into a shared TextSystem catalog. Existing measurement handles
  and renderer Assets share the same catalog. Canonical aliases pin content
  and fit the shared128-byte menu font boundary; duplicate alias/content jobs
  retain the original handle/revision. Old canonical faces remain stable when
  a logical alias is updated. Admission is bounded to16 jobs/64 faces per client,
 64 jobs/128 faces globally,32 MiB per source and256 MiB charged resident bytes.
- Implemented: public `Text.beginFont/fontStatus/resolveFont/releaseFontJob/
  releaseFont`; rich/simple/glyph rendering uses owner-scoped font views. Native
  `System.addFont` now submits shared work and resumes TJS only after publication;
  Font face-family selection, getTextWidth and drawText use these shared faces.
  It no longer validates and discards a temporary font. Complete wtnbgo/addFont
  source was inspected at86e6ecc27844a2b4aea8ca9d171061b70bd0072b; the new engine
  font resource manager is authored Rust, without a second native font engine.
- Compilation: release Runtime font tests compiled successfully
  (`font-shared-tests2.log`); the final native test target compiled successfully
  (`font-native-build4.log`). Root also compiled the engine API bundle. Earlier
  attempts interrupted by unfinished shared CatSystem2 source/manifests are
  retained as failed logs, not counted as compilation passes.
- Behavior: shared resource tests passed2/2; the separate ignored system-font
  case was explicitly run and passed1/1 (`font-shared-system-test.log`). It
  checks existing measurement handles, CPU raster/metrics consistency, client
  isolation, duplicate registration, old-face immutability and release. Public
  Text API testing passed1/1 with an explicitly supplied system font
  (`font-engine-system-test.log`). The native addFont suspend/resume/metrics/
  draw test passed1/1 (`font-native-system-test2.log`); the current authored
  native suite passed18/18 with three separate cases ignored
  (`font-native-regressions2.log`). Font files are read only at test runtime.
- Installed behavior: OUSAMA ordinary START replay now passed1800/1800 ticks
  with no error (`start-native26.json`, `start-native26-full.log`), native1622
  passes and1983 engine API calls. Its own Kosugi and Kosugi Maru fonts were
  registered with distinct verified digests; the first dialogue proceeded to
  native `[p]` click-wait, with no suspended VM, owned framebuffer or plugin
  shells. This replay is device-free; the new dialogue GPU image is awaiting
  the combined desktop player, separately from the earlier verified title.
- Limits: collection-wide native addFont registration and native `@` vertical
  font semantics fail explicitly; the shared API can select collection faces.
  Native rasterizer selectors use the existing portable outline profile and
  do not promise GDI/FreeType pixel identity. Runtime snapshots with dynamic
  font resources/jobs are explicitly gated until font reconstruction exists.
  The game's caught automatic-thumbnail error (opaque stored-RGB composed with
  straight-alpha child) remains; no successful thumbnail/bookmark output is
  claimed. No game/font payload, window, CI, commit or push was created.
- Follow-up compilation/installed behavior: the scene-reset fixture passed in
  the release native suite25 passed/1 separate corpus ignored
  (`menu-visibility-tests.log`); a fresh release example run against another
  private copy of those same preferences again activates the actual title
  button and stops at native13 (`menu-title2-boot.log`). Its retained scene now
  has SCENE_ROOT visible, COLOUR_ROOT hidden and the menu root visible
  (`menu-title2-summary.json`). An actual renderer PNG is still pending.
  Read-only analysis found normal native13 is a staged centibel fade through
 43CC90/43BFF0/43C0C0, distinct from forced422330 immediate stop; it remains
  explicit unsupported rather than being replaced with an inaccurate stop.
  Shared/public API docs were regenerated and verified by root at fingerprint
  `0f42d774636005f8` (its public motion/restore and API tests are separate from
  the native and installed results above).


### 2026-10-31 — desktop Circus title and KRKR dialogue rendered through shared APIs

- Implementation: client-scoped font resources now join SharedHost and Assets
  through one dynamic Text catalog. Runtime polls completion before VM work;
  retained measurement handles and client-scoped text/GlyphRun GPU cache lookups
  observe the same immutable canonical faces. Pending/retained dynamic fonts
  explicitly reject framework capture and prepared restore until reconstruction
  exists; staged restore cannot lose a face after its addFont instruction.
- Behavior: the release snapshot gate test passed1/1
  (`ousama/font-snapshot-gate-test2.log`). Public Layer/UI motion, mask routing
  and mid-track Runtime restore passed1/1 (`circus-execution/public-motion-mask-test.log`).
  Documentation was generated/verified8/8 at0f42d774636005f8 before the newer
  menu-enabled API addition, which has its own pending generation below.
- Compilation: combined desktop release features rfvp,bgi,nscripter,krkr,
  arc-formats,circus,catsystem2 passed (`circus-execution/title-font-cat-player-build2.log`).
  Frozen player SHA256 is
  e1a627a3e855128ffcc4749095c190e16d367050326f20d21685c60d65d1f7cb;
  all four existing managed local launchers now point to this player. The first
  combined attempt stopped on missing PE traits in the new INT helper; the
  helper was repaired before this successful build. No failed build is a pass.
- GPU behavior: normal detected DC4PHDL launch with copied existing system.dat
  rendered its actual1280x720 title on AppleM5/Metal, without a window or original
  executable. Root inspected the full background, character, logo and seven
  menu items in `circus-execution/desktop-title2.png`; report is desktop-title2.log,
 1400 host frames, native MAIN pc0x14fc waiting for menu input. Separate shared
  pointer-release input reaches original prologue and the explicit next0x13
  audio-control boundary. Title rendering does not imply completed story/audio.
- GPU behavior: the same player, original OUSAMA data, ordinary START input
  track and no font override rendered real bedroom/character/name/dialogue and
  quick menu at1920x1080 (`ousama/desktop-start26.png`, inspected by root).
  The1800-frame report has error=null, original KAG last_tag=p/click_waiting=true,
 1983engineAPIcalls, owns_framebuffer=false and plugin_shells=[]. Two original
  game fonts came through the shared registration path. The caught thumbnail
  piledCopy/opaque-alpha save failure remains; complete save compatibility is
  not claimed.
- CatSystem2 behavior: this combined player's detect-only launch recognizes
  the outer game-nekopara-vol3 directory, selects its data client capability,
  preserves startup kcs.int/main.kcs/1280x720/heap, and plans31 original INT
  mounts without extraction (`catsystem2/desktop-detected-plan2.json`). Its
  native syscall mapping is not yet in this frozen player. New shared menu
  enable state has independently passed the release owner/ancestor/activation/
  restore test1/1 (`catsystem2/shared-menu-enabled-tests.log`); native integration
  and the next player are separate unfinished work.
- Evidence remains ignored under target/verification. No proprietary game/font
  files, original executable process, native window, CI, commit or push was added.

### Circus audio-release contract draft, paused before admission (2026-10-31)

- Static analysis identifies native13's owner-update db100 release and retired
  PCM channel transfer; its200ms refill timer does not drive the fade. Default
 30-step f32 accumulation stops on update29 or30 depending on starting level.
  This is a single target selected by a threshold, not an automatic two-stage
  release. The ignored `audio-release-plan.json` and arithmetic oracle record
  sources and distinguish static simulation from native execution.
- Added device-free Core release spec/handle/actual-receipt contracts and an
  audio owner/controller draft. At the owner's request to free an agent slot,
  the public admission method explicitly rejects unverified release producers;
  native13 remains unsupported. No PCM behavior, native13 progression or
  script/API integration is claimed. `cargo check --release -p crysirror-audio`
  passed at this guarded boundary (`audio-release-paused-check.log`). Remaining
  gain capture, preparation/cancellation fences and required sample tests are
  recorded in ignored `AUDIO_RELEASE_HANDOFF.json`. No commit was made.

### KAG bare-label semantics and bound XP3 containers (2026-10-31)

- Installed evidence: the published offscreen player reaches Natsuzora's original
  custom.ks, then fails in MainWindow.tjs setRecordLabel3304. The debugger
  identifies the incorrectly emitted `*` alias: original script strips it and
  reads index1 of the empty result. This is a parser semantic wall, separate
  from the already validated CXDEC resources. ExE's actual case-sensitive path
  is game-kr-ExE; its six XP3 resources begin with MZ and the earlier player
  rejects all profiles at the offset-zero signature check.
- Implementation: native KAGParser's one-character `*` line has an empty callback
  alias and no jump target; `*|page` still requires a preceding named label.
  Shared XP3 now searches paragraph-aligned signatures in256KiB read blocks,
  applies native base-relative index/segment offsets (including unsigned
  negative-relative addresses), and admits bounded raw/zlib continuation chains
  with cycle and cumulative-byte limits. Bound HXV4/indirect descriptors remain
  explicit unsupported profiles. No executable code or native renderer runs.
  Real pinned source files/hashes/notices are recorded in
  docs/krkr-bound-xp3-label-sources.json.
- Compilation/tests: release KAG suite passed54/54; the initial shared archive
  release suite passed9/9, including offset-zero XP3 regression, bound v1/v2
  compressed/raw indices, negative-relative continuation, missing/unaligned
  signatures, cycles and invalid segment ranges. Additional chunk-boundary and
  protection-regression runs are recorded separately after completion.
- Behavior: these changes have not yet been verified with a rebuilt combined
  player. Neither game's title/menu/story is established by these parser/archive
  results. Local diagnostics remain in target/verification/krkr-natsuzora-exe/;
  original game resources were read only and no original EXE/DLL was executed.
- Further validation: final release archive suite passed10/10, including a
  chunk-boundary/bounded-read scanner fixture. The separately requested local
  CXDEC corpus test ran for both actual directories with zero ignored cases:
  ExE six XP3 archives/25,586 entries,267 stride97 reads with checksum0 failures;
  Natsuzora ten archives/273 stride97 reads with checksum0 failures. ExE uses
  its own plugin/cxdec.tpm via the existing static bootstrap recovery. Logs:
  krkr-natsuzora-exe/{archive-final-tests,exe-archive,natsu-archive-final}.log.
  These sampled reads establish neither complete-game nor title behavior.
- Combined player3 behavior (SHA256
  84af91007810fa3f8b8b2d592cae6832f3b7e46fe38b1a2f85d46d6487d7cd22):
  both previous walls are passed. Natsuzora now reaches custom.ks92 image,
  *title_white, then AffineLayer.tjs458 fails because Layer.loadImages returns
  void instead of native null metadata. The actual white.png has no text/tag
  chunks. ExE now loads its original KAG system and reaches AfterInit's
  showdate.tjs176 callback, which fails after with-dot calling incorrectly
  selects the callback dictionary as this. These are actual later VM walls;
  neither run produced a PNG or reached a title input wait, and short-Ctrl
  interaction has not yet occurred.
- Implemented follow-up: empty Layer image metadata returns native null Object;
  TJS WithMember calls follow original T_WITHDOT GPD+CALL rather than direct
  CALLD. The original compiler tests haspropnode only for T_DOT/T_LBRACKET;
  ordinary named dictionary member calls retain their distinct receiver policy.
  Targeted release regressions are queued; compilation/behavior outcomes remain
  pending at this entry. No game flags/PC or original scripts were changed.
- Follow-up validation: release TJS with-dot regression passed1/1; executing
  that same test binary's full suite passed426/426. The native empty-image
  metadata regression passed1/1, and its compiled engine_bridge binary's full
  suite passed26 with one separate installed case ignored. Logs are under krkr-natsuzora-exe/{with-call-tests,
  tjs-full-tests,image-null-test,engine-bridge-full-tests}.log. The optional
  installed Runtime test harness now explicitly selects CXDEC companions via
  CRYSIRROR_KRKR_CXDEC while retaining its existing Adler default. Its initial
  build was interrupted by a concurrent shared-resource Reservation initializer
  missing cancellation (E0063); this is a compilation failure, not game behavior.

## 2026-10-31 CatSystem2 native VM, shared MENU/XML and INT namespace handoff

- Implementation: first-class `crysirror-catsystem2` executes original KCS v2
  bytecode with numeric owned memory, native frames/typed values, transactional
  native outparams, pending calls, increment/compound operations and explicit
  unsupported PC/native errors. No hosted engine, original executable/DLL
  execution, private renderer/mixer or game/font distribution. Public source pins,
  licence scope and original-module IDA evidence are in its SOURCE.json.
- Shared extraction: `resource::pe_menu` parses bounded standard PE MENU v0;
  original production MENU104 is retained by shared UiSystem and native393 uses
  its enabled-state command. `resource::xml` owns bounded immutable UTF-8 XML,
  attributes/entities/text and slash-path/unique configuration lookup; startup
  and native XML590-593 reuse it. Resource workers reserve before scoped I/O and
  parsing, then owner publication precedes native handle returns. Native
  copy/format/task/config policies remain in the thin adapter.
- Compilation: `xml-adapter-check.log` passed release runtime-feature check;
  `int-alias-check2.log` passed release formats-arc check after the shared
  DecodeContext cancellation/INT namespace changes. An initial mechanical
  Reservation field edit caused E0063 and was corrected before the passing
  second check. The source is frozen for a parent-coordinated handoff.
- Independent validation: `native-values-tests2.log` passed6 unit and11 KCS
  tests, including atomic native writes, native narrow increment DWORD returns,
  unsigned compound-divide remainder policy, stack-value push and allocation
  budgets. `shared-resource-final-tests.log` passed MENU3/3 and XML4/4. Later
  XML name validation/cancellation/namespace changes have final tests queued;
  queued tests are not counted as passes.
- Actual behavior: original owner data indexed31 encrypted INT archives with
  11786 raw entries;14 KCS and66 CST records parsed. `owner-game-strings.log`
  ran actual configured `kcs.int/main.kcs`8328 instructions through shared
  MENU/worker/XML resources: native590 load2,592 integer-query626,593 string-query1,
 591 close2,14 formatted-copy27. XML handles/resident bytes ended at0. The
  unacknowledged next native is400 atPC0x4cd, args[19,222]. Unmapped production
  menu commands fail visibly. This does not validate a title/image/audio/gameplay
  or original saves, and is not completed game compatibility.
- Public-entry gap and fix: parent desktop player3 exposed `duplicate archive
  path or id` while mounting scene.int. Its26 same-name CSTL pairs have different
  offsets but fully identical stored payloads; native INT body encryption uses
  only the archive-wide file key, independently of entry ordinal/offset.
  Provider-only coalescing now admits proven equivalents, retains every original
  entry_id/native_index/path/offset/size row in metadata, and rejects conflicting
  duplicates/custom transform ambiguity. Comparisons use64KiB chunks, a64MiB
  cumulative read budget and cancellation checks. Common ArchiveMount duplicate
  rejection remains strict. Index tokens are stripped on publication/mount.
- Pending at handoff: release `int_aliases` tests, original all31 ArchiveMount
  regression, and resource XML/MENU/resource-job tests are running/queued in
  `int-alias-tests2.log`, `owner-game-mounted-final.log`, and
  `resource-final-validation.log` respectively; no pass is claimed yet. Root
  takes over observing these and public desktop bootstrap/API-doc regeneration.
  Exact source boundaries, native400 evidence and next requirements are in
  `crates/crysirror-catsystem2/HANDOFF.md`.

### Two installed KRKR games: original Start paths and passive UI tooltips (2026-10-31)

- Behavior (shared Runtime CPU probe, original archives/scripts, no GPU/window):
  Natsuzora passes the prior loadImages wall, enters the title script and starts
  original title_beach.mpg through shared Media. An ordinary pointer/down/up at
  the original title.csv Start rectangle is handled by native onMouseDown/Up/
  Click and enters start.ks then １日目.ks/*day_start. ExE passes the prior
  showdate callback wall, then exposed a genuine intermediate crossfade geometry
  validation failure. Grouping all dirty native containers into one engine Layer
  batch fixes the half-updated endpoint pair while preserving strict validation.
  The repeated original Start click then enters ExE 01.ks. Logs and reports are
  natsu-start-native4 and exe-start-native5 under krkr-natsuzora-exe/.
- Compilation/tests: the atomic live-crossfade resize regression passed1/1;
  the same compiled native probe's authored suite passed19 with3 separate cases
  ignored. Both real games next stop on nonempty Layer.hint while constructing
  original system buttons (Natsuzora meswinload; ExE sysuiload). Neither run
  reached the later scripted Ctrl-down event; no Ctrl delivery or GPU title/
  dialogue rendering is claimed by those CPU runs.
- Implemented: shared Scene UI TooltipSpec/TooltipSystem with scoped put/remove/
  show/hide commands, real TextMeasurement extents, delay/lifetime, canvas-clamped
  position, passive Solid/Text primitives and transient hover state. Root's
  Runtime preparation/public Ui API work is coordinated separately. The native
  adapter retains hint strings, parent-hint/ignore-sensing policy and existing
  native topmost-hit decisions; no private tooltip paint, layout or hit tester
  is added. Native hidden/retired/focus-lost targets cancel the shared intent.
  The bounded profile accepts single-line plain text and rejects control/rich/
  rotated declarations instead of fabricating support.
- Compilation/tests: shared tooltip release tests passed2/2 for measured bounds,
  timing/expiry, passive clicks, client isolation, batch rollback, removal and
  restoration. Initial scene compilation failed on an unavailable serde_json
  production dependency; replacing the derived preparation key with typed
  bounded Spec comparisons removed that dependency. Native final probe rebuild
  and actual-game continuation with tooltips remain pending at this entry.

## RealLive AIR clip, native cursors and installed after/LB regressions (2026-10-31)

- Implementation: native object screen-space clips now belong to transparent
  identity parent layers; the image keeps its original rect/UV/transform. Parent
  object scale and adjustment offsets also reach child sprites. Shared
  `crysirror-image::cursor` sprite/header kernels prepare Gameexe cursor sheets,
  and existing ImageJob workers own full resource reads/decoding; the native PC
  awaits completion before shared UI cursor publication. No private pointer
  renderer/clock is added. Header reads, source/output pixels and frame counts
  have explicit bounds; source provenance is in
  `docs/reallive-cursor-source-audit.json`.
- Shared Canvas `sample_pixel` provides a bounded opaque-solid profile with
  exact local clips and terminal opaque coverage. Unknown image/text/group,
  transformed, filtered, blended or translucent coverage fails explicitly.
  Native GetDCPixel reads that service and preflights all three RGB destinations.
  DSTRACK nonzero starts use exact decoded-source frame boundaries and existing
  audio seek/loop services. Native optical checks have explicit portable/strict
  policy; portable diagnostics report no authentication or verification result.
- Compilation/tests: initial RealLive release suite 45 unit + 9 bridge passed;
  later targeted release checks passed: shared cursor/header 3, shared Canvas
  sampling 3, native RGB atomic write 1, cursor/query bridges 3 and original
  Gameexe DLL-slot preload 1. The cursor fence test was corrected to gate the
  actual CPU worker pool rather than the unrelated I/O pool, then passed.
- Behaviour: immutable combined player4
  (`a8a1c108b7162dad0161c8a27c9e1f4fb187e2571546019452603a7a3a2ce83f`),
  offscreen Metal: AIR's 400-frame Key logo is complete and centered; logical
  custom cursor 0 is selected. Physical OS cursor presentation was not verified.
  Ordinary Ctrl down/up, Enter and mouse input reach Tomoyo After's complete
  title and Start enters scene628:28 with the June 28 calendar, room and Japanese
  dialogue. Further clicks stop at scene628:33 because `koe/z0629.nwk` is not
  mounted by installed-game launch. Little Busters! reaches its complete title
  (scene9012:541), with the optical step explicitly policy-disabled; NewGame
  stops at scene9012:1204 because configured EF00 slot 1 was not preloaded.
  The original pinned Machine::new DLL.nnn preload policy is now restored and
  independently tested; installed-game behaviour after that change awaits the
  next combined binary. No original executable/DLL or window was run; headless
  playback emitted no sound. Evidence: `target/verification/reallive-air-after-lb/`.

## Circus prologue frontier, shared audio release and mesh freeze (2026-10-31)

- Implementation: shared `AudioReleaseSpec/Handle/Info` transfers an actual live
  decoder/cursor to an independent nonlooping tail, freezes current native group
  attenuation and advances native f32 db100 release on explicit owner updates.
  Public Audio release methods and Runtime snapshot/restore gates use that actor.
  Shared `Paint::Mesh` freezes an owner-completed `PuppetFrame` and reuses the
  existing mesh renderer in Canvas snapshots; live surface/history dependencies
  fail explicitly. Circus maps native13, stage scale/pivot declarations and1c
  transient checkpoints to these engine-owned resources and retains native policy.
- Validation: audio release/all audio release suites **150 passed, 0 failed,
  4 ignored**; public Audio/Runtime PCM/worker/snapshot acceptance **2 passed**;
  shared frozen Canvas **9 passed**; explicitly executed Metal frozen Canvas GPU
  suite **9 passed, 0 failed, 0 ignored**. Native tests **29 passed, 0 failed,
  1 installed-corpus test ignored**. GPU assertions include source removal,
  changed mesh content under the same source ID, serialization, immutable resource
  rejection and later outer opacity. Audible device output was not verified.
- Actual installed behavior: normal title pointer press/release passes native13
  and fade capture, enters `DC4_SEN20181213B`, completes initial stage/placement
  declarations and1c's actual shared visual/resource checkpoint. It stops at
  **pc0x83e, opcode18 selector4 arg1=1**, the mandatory original autosave0.
  `native13-checkpoint-installed-boot.log` records this headless run. This is
  prologue initialization evidence, not rendered dialogue or complete playability.
- Remaining: native autosave requires the complete **356324-byte** original body
  plus its **128x72** thumbnail; no fake/JSON/zero-filled original save was written.
  Original1d/slot restoration, later dialogue, PCK sequence playback and numerous
  native profiles remain unsupported. Checkpoint UI paints are not reconstructed
  interactive UI. Exact layout and remaining fields are in the local ignored
  `target/verification/circus-execution/CIRCUS_AUTOSAVE_HANDOFF.json`.

### KRKR media facade and actual Ctrl continuation (2026-10-31)

- Shared Image preparation now accepts an empty effect chain for Asset/Resource/
  Solid inputs, keeping its existing worker reservations, memory lease,
  cancellation and owner publication. Raster/Transition still require their
  matching operation. The release image_composite_clip suite passed2/2,
  including exact authored PNG RGBA/coverage, a foreign client resource rejection
  and failed fingerprint admission. Native positive mask hits use a completed
  shared ImageResource.alpha_at only while provenance remains an equivalent
  immutable bitmap; unsupported composed canvases still fail explicitly.
- layerExMovie's complete original plugin/header/base/manual/licence was read
  at real krkrz49c4d53506edecb824cd7b2cff8d32959b1f1b70. Its four native methods
  and original callbacks map to shared Media metadata preparation, pictures,
  loop/stop and resource retirement; no DirectShow, pixel copier, native clock
  or decoder is adopted. Open waits for actual header dimensions before original
  TJS geometry reads. Audio is disabled as in the original primary-video-only
  plugin. Side-by-side alpha, reopening/restarting while playing and resized
  playback images fail explicitly. Source hashes/notices are recorded in
  krkr-bound-xp3-label-sources.json and third-party/layer-ex-movie.txt.
- Compilation/tests: final native probe release build passed; its authored
  suite passed22 with3 separate cases ignored. New cases assert transparent and
  opaque original input hits, real movie dimensions/frames/engine loop counts,
  stop/retirement, and exactly one EOF callback after the final picture.
- Installed CPU behavior: native7 observes Natsuzora's actual original title stop
  at host frame432 before starting an ordinary input track. Start enters
  １日目.ks; Ctrl down/up are each delivered once,49 actual onUpdateMovie events
  update the original loop background, then original script retires media640
  and enters op.wmv through shared Media512 without VM/codec errors. ExE's
  immediate click during its title animation did not enter the story; this is
  recorded rather than counted as success. Keeping the same original Start
  coordinate and clicking110 updates after the observed title stop enters its
  story, delivers Ctrl down/up once, and ends at an original page wait with
  162 retained GlyphRun nodes and two actual audio channels, without errors.
  Input update counts are simulated host updates, not claimed wall-clock time.
- Installed GPU behavior: immutable player4 (SHA256
  a8a1c108b7162dad0161c8a27c9e1f4fb187e2571546019452603a7a3a2ce83f)
  renders both original titles; both PNGs were visually inspected. ExE ordinary
  Start renders its original first Japanese dialogue and message frame; a
  separate short-Ctrl/release run reaches and renders the original OP movie.
  First-run opmovie is deliberately unskippable in original macro.ks until
  movieseen, so extra clicks correctly do not fake an EOF. Reports confirm no
  window/audio device and zero rejected audio channels; audible output is not
  verified. Native7 CPU behavior is later than player4 and Natsuzora's first
  dialogue still requires the coordinated rebuilt player GPU check.

## First-class native RealLive DLL semantics and shared spatial extraction (2026-10-31)

- Implementation: `crates/crysirror-reallive-dll` replaces the former nested
  helper; package, imports and workspace dependency now use that first-class
  name. Original MPL-2.0 licence, pinned hashes and the previous adoption
  manifest remain beside the maintained source. This is not only a move:
  EF00 cubic B-spline/perspective sampling and PT00 f64 geometry, matrix and
  parabolic kernels now live in public `crysirror-vfx::spatial`; native argument,
  bank, RNG, addressing, actor/dungeon and numeric-profile policy stay native.
  The private PE parser is removed in favour of shared Resource `PeData`.
  DT00's `image` means PE table/data memory, never a rendered screen. Production
  environment-selected trace/dump filesystem writes and the test-only fake
  game_fs package are removed. Source/extraction ledger:
  `crates/crysirror-reallive-dll/SOURCE.json` and `SOURCE_NOTES.md`.
- Compilation/tests: one release Cargo invocation compiled VFX, native DLL and
  RealLive crates. Shared spatial tests 3 passed; EF00 golden native-bank and
  unknown-DLL tests 2 passed. The release DLL test binary then explicitly ran
  both otherwise-ignored owner-file cases against original PT00.dll/dt00.dll
  static tables: 2 passed. Those calls execute Rust translations only, not PE
  code; they do not establish full minigame gameplay or hardware behaviour.
- Behaviour remains separate: player5 attempted installed after/LB voice mounts
  but both failed at the existing provider-mount quota before native boot.
  The new extraction and a revised shared mount budget require the next
  combined player for installed-game validation. Logs are under
  `target/verification/reallive-air-after-lb/`.

RealLive installed follow-up, immutable player6
(`00f1b6ef71ac4471cc45e0e37d16c6d3456e91f475bedea977eff3c8e3e37304`):
Little Busters! NewGame now executes 4,176 EF00 calls through the first-class
DLL/shared spatial path, advances through scene8516 and reaches scene513:51.
At 7,000 frames the actual GPU capture shows the first Japanese dialogue line
in its native message window. The black background is the script's explicit
KURO image, not a claim of a missing render. Further ordinary clicks stop at
scene513:58, voice51300042; after similarly stops at scene628:33, voice62900017.
Both now fail with `Not a directory` while listing a voice-container root
backed by an original archive file. The shared VFS fix needs the next binary;
these failed voice preparations are not successful voice-playback evidence.

### Codex 2026-10-31 — new native program foundations and shared installation media

Implementation:
- Added first-class `crysirror-system4` AIN v5 program reader. Legacy MT19937-1998 byte XOR is a reusable `crysirror-resource::legacy_stream` kernel; AI2 has bounded exact zlib framing. Original byte strings, functions/globals/structures/HLL declarations/switches and function-type size metadata are retained. The original reader does not use that size metadata as a serialized span; an initial invented equality check was removed after actual-game validation exposed it. Native operand-layout decoding is separate from instruction execution.
- Added first-class `crysirror-system-nnn` bounded original SPT data/instruction reader. The generic rolling 32-bit XOR/add transform lives in shared resources; the adapter data preserves word PCs and table/text addresses.
- Added shared read-only `windows/msi` resource archive and `InstallerCatalog`: MSI File/Component/Directory/Media metadata maps embedded CAB members to installed paths. MSI/CAB readers are local to worker invocations, decoded members use common memory leases and cancellation checks. External cabinets require an explicit future scoped dependency mapping; no installer action, COM/native EXE execution or filesystem extraction occurs.
- These two native crates do **not** yet provide executable VM factories or playable adapters. System 4 stack/heap/calls/HLL mappings and System-NNN expressions/control flow/native configuration/syscalls/save semantics remain unfinished. Recognized instruction layouts and successfully read game assets are not integration completion.

Compilation and local tests:
- `cargo check -p crysirror-system4 --release` passed. AIN authored tests: 2 passed, 1 owner-data test ignored by default; latest full instruction-layout validation is recorded separately when actually run.
- Shared MSI authored tests: 2 passed, 1 owner-data test ignored by default. Native System-NNN authored tests: 2 passed, 1 owner-data test ignored by default. No game fixtures, fonts, extracted files or original code are distributed.
- Python 3.11 tooling: 201 passed (`target/verification/majiro/python311-final-tests.log`).

Behavior:
- Explicitly executed the owner AIN test against `game-rance7/GAMEDATA`: parsed version5, 5,750,996 code bytes, 6,580 functions, 219 globals, 209 structures, 10,281 strings, 92,120 messages, 15 HLL libraries and native main function16. This establishes the data reader, not gameplay (`system4-rance7-program3.log`).
- Explicitly executed the owner MSI test: read 385 installed files and decoded original `mugen2/spt/preprologue.spt` directly from the embedded CAB. Explicit System-NNN owner test then validated 11,248 words, 78 messages, 176 strings and first original instruction PC0x20/identity0x66660007/code0x33330051 (`systemnnn-owner-program.log`). No command was acknowledged or executed by that data-only test.

## Majiro original MJO execution and shared Canvas frontier (2026-10-31)

- Implementation: first-class `crysirror-majiro` owns bounded X/V1.000 MJO
  instruction/function/jump validation, native stack/local/control semantics and
  pending syscall receipts. MJO's fixed CRC-table XOR is separate from RCT image
  passwords. Native resources are read on bounded capability workers; there is
  no original EXE execution, hosted engine, private renderer/text algorithm or
  mixer. The thin facade maps reviewed true-colour page creation into shared
  Canvas state; actual native allocation is opaque 24-bit BGR, not the public
  wiki's 32-bit description. Native auto-mode configuration and explicit release
  startup-mode policy stay in the adapter. Source pins/licences/module SHA and
  static inspection boundaries are in `crates/crysirror-majiro/SOURCE.json`.
- Compilation/independent validation: release native tests passed5/5; the
  authored original-MJO-to-shared-Runtime Canvas bridge passed1/1. It verifies
  real engine resource dimensions and preservation of an unknown following
  syscall at its native PC. Final tests strengthen its opaque-black assertion;
  that final assertion awaits the next compiled test binary. No visual/headless
  screenshot or physical device output is claimed by these CPU tests.
- Actual owner behavior: explicitly executed local owner tests passed2/2 in
  `target/verification/majiro/owner-runtime-final.log`. All181 original MJO modules
  parse, covering496549 instructions. The unchanged `start.mjo` entry0x203b runs
  through its first native policy calls under shared Runtime and a read-only
  SHA-guarded native module, then stops after15 instructions at **pc0x213f,
  line446, syscall0xc6282f19 `rand(100)`**. Its original pending operands/PC remain
  intact; engine_api_calls=0/pages=0 for that game run. This is native execution
  evidence, not a rendered title or completed Majiro integration.
- Installed-game boot: the optional desktop `majiro` feature has an original
  ARC/ENV/start helper, shared private archive mounts and root/bin executable
  metadata lookup, with bounded header probes and capability-root checks before
  reading. Conflicting patch providers and unlinked builds fail explicitly.
  Parent coordinates final desktop compilation/public API references; no pass is
  claimed for the combined player at this entry. Cross-module linking, arrays,
  full value/call profiles, text/control, image/audio/video and saves remain open.

RealLive tooling follow-up: the Python authoring validator now accepts exactly
`optical_disc_check = disabled_for_native_host | strict`, matching native Config;
unknown values and non-string substitutes fail. Native API schema tests 8 passed.

### KRKR installed GPU follow-up: original dialogue, movie loop and passive hint (2026-10-31)

- Actual behaviour with immutable combined player6
  (`00f1b6ef71ac4471cc45e0e37d16c6d3456e91f475bedea977eff3c8e3e37304`):
  Natsuzora normal bootstrap and original Start reach the complete original first
  sentence, 【壮太】「そろそろ、暖かくなってきたな」, over the actual decoded ferry/sea
  movie background. `natsu-first-line6.png` was inspected; its report has no
  native error, original `p`/click wait, one completed background loop and239
  original `onUpdateMovie` callbacks. Ordinary Ctrl down4300/up4420 then advances
  the original scenario into `op.wmv`; `natsu-ctrl6.png` was inspected and the
  report records one KeyDown/KeyUp and actual published video pictures. Host
  frame budgets are not wall-clock or completed-movie evidence.
- Actual passive tooltip: moving the original pointer to the original Hide
  button after Start presents 「ウィンドウ消去」 through shared Ui measured text,
  without advancing the original first-line click wait. `natsu-tooltip6.png`
  was inspected; its native error is null, `p` remains waiting, and the report
  retains the real engine tooltip specification/extent and248 movie callbacks.
  All runs are offscreen with no game window or original executable execution;
  mixed audio samples are reported, but audible device playback is unverified.
- Remaining visual defect is confirmed, not accepted as correct UI: six toolbar
  buttons all show the same pink X. Original CSV declares six separate14x14
  controls and their original normal/off PNGs have six different RGBA hashes;
  the source centre pixels are blue, while every rendered centre is exactly
  `[244,106,195,255]`, matching Hide's original hover image. Read-only graph
  inspection finds all normal/hover atlas copies retain the same mutable
  `__surface/1073741864` asset from temporary Layer40. `image_api::poll` publishes
  each prepared image into that reused surface slot, so later loads replace
  earlier copied images. Bare-image alpha preparation must retain its immutable
  original asset paint; effect-result publication also needs immutable generation
  semantics before general copy correctness can be claimed. No fix/rebuild is
  claimed here. Detailed source hashes, sample pixels and Canvas declarations:
  `target/verification/krkr-natsuzora-exe/natsu-toolbar-evidence.json`.

Codex follow-up validation (same work, actual later results):
- AIN v5 full code-layout/entry-boundary validation explicitly passed on Rance7: 1,062,488 original instructions, main PC0x73a. All 6,580 function entries lie on decoded boundaries; validation uses a bounded bitset rather than an unbounded tree (`system4-rance7-layout-final2.log`). The initially omitted renamed EOF/MSG metadata rows were fixed from their original declarations before this pass. Authored AIN tests remain 2 passed; the actual owner test was explicitly run, not counted as an ignored pass.
- Shared MT19937-1998 profile matched an independently compiled, unmodified reference C oracle: 1 passed (`legacy-stream-tests-final.log`). The C oracle is research tooling, not an engine dependency.
- Runtime VFS quota test passed at 256 mounts per client. Separate mounted-archive-file directory/read/client-isolation test passed. Late-installed static default fonts and shared dynamic font publication passed the explicit runtime-system-font regression; no font file was committed.
- API reference regenerated and all 8 checks passed at fingerprint `d1da391dfe839e4a`, including the partial Majiro config (`target/verification/majiro/api-final-verified.log`).
- Combined player7 compiled in release and was copied immutably: SHA256 `3af2da91bce72920c9aa54651f8667e13e1587b215d74c574f96ed6081dc51ea`. Normal installed-game detection selected Majiro, all six original ARC mounts, 800x600 ENV and `bin/暁の護衛.exe` as read-only ABI metadata. Normal headless boot then reached the real unsupported rand(100), start.mjo PC0x213f/line446/hash0xc6282f19; no page/title/engine API call had yet occurred. A short 100-frame run was still loading, and was not called successful integration. The longer actual run is `desktop7-native-1200.log`.
- Majiro follow-up: the thin native RNG policy now calls the existing shared
  `SequenceRng::msvc` kernel, with original `srand(time(0))` Unix-second seeding
  captured once by the startup I/O worker. Signed modulo and zero-bound error
  policy stay native; state is explicit seed/accumulator/draw-count data and can
  be captured/restored independently, with no global Random. Release tests now
  pass1 RNG/capture +1 shared Canvas (including exact opaque-black sampling)
  +5 native/ENV tests. The two explicit owner tests also pass again in
  `owner-runtime-rng.log`: actual startup advances33 instructions and2 CRT draws,
  then stops at **pc0x214, line58, syscall0x111bd910 `get_effectspeed`** inside an
  original script function. Native ENV/settings getter mapping remains missing;
  engine_api_calls is still0 for the real game and no title is claimed.

KRKR toolbar follow-up implementation and independent validation:
- `image_api` now keeps unchanged Asset preparation as an immutable resource for
  alpha queries while retaining its original Canvas asset paint. Only this bare
  preparation path avoids surface publication; pixel effects/composite output
  publication is unchanged. No shared worker/scene/public API was changed.
- Release targeted regression passed1/1. It loads authored A into a temporary
  Layer, copies it, reloads different B, then asserts destination paint is A,
  temporary paint is B, actual shared-worker A bytes/alpha are unchanged, and B's
  opposite native alpha coverage triggers precisely its opaque first pixel.
  The same immutable native test binary passed23/23 authored tests (3 externally
  provisioned tests remain ignored by default).
- Explicitly executed the installed Natsuzora test with that binary: normal boot
  observed original title wait at frame498, original Start reached original `p`
  click wait, no native error,198 original movie updates. The retained Canvas
  graph now has separate original normal/hover/toggle asset paths for each of
  the six buttons, without the reused temporary surface alias. Evidence:
  `natsu-immutable8-toolbar.json`, `immutable-alpha-regression.log` and
  `native-immutable8-suite.log` in the existing ignored verification directory.
  New combined-player GPU pixels are being checked separately.

RealLive final player7 behavior and read-only next-wall diagnosis:
- Immutable player7 SHA256 `3af2da91bce72920c9aa54651f8667e13e1587b215d74c574f96ed6081dc51ea` crossed actual after/LB voice through shared NWK decoding/mixing after the shared mounted-file directory fix. Headless GPU only; no original EXE/window or physical OS cursor claim. after at frame5000 reaches scene628:43 Pause17 without error,4 acknowledged KOE operations,3,120,384 mixed PCM frames,peak1.3376832,rejected0/underrun0 (active1 is looping BGM). LB at frame7800 reaches scene513:59 Pause17 without error,1 acknowledged KOE,5,226,240 PCM frames,peak0.9996352,rejected0/underrun0,active0 after voice completion; screenshot shows original May13 bedroom narration. EF00 original execution retains4,176 successful samples. Evidence/screenshots/logs stay in ignored `target/verification/reallive-air-after-lb`.
- Further normal LB clicks at frame10000 reach SEEN9030:82 element133 byte1425, native `1:72:1000` overload1 `objOfFile(110,strS1001,1)` and visibly fail loading `CGMS12?030`. Original preceding string branch appends that suffix. Complete pinned RLVM lookup strips `?` only for the resource filename, then SDL graphics applies tone curve `suffix/10-1`:030 selects table2. It loads a3×256 RGB lookup from configured TCC and preserves alpha. Existing shared G00 atlas/pattern expresses geometry, not this lookup; current ImageEffect has no arbitrary RGB LUT input. This is the explicit next shared capability/native policy gap; no guessed suffix stripping or production expansion was made. RealLive source is frozen.

KRKR combined player8 GPU verification
(`e86cbd7f0fb37b4a5913466a569392cf35bf1c316347bb9d6cbfd0c3376ad574`):
`natsu-first-line8.png` and `natsu-tooltip8.png` were both inspected after normal
installed-game bootstrap and ordinary original Start. The original complete first
line and decoded ferry/sea background remain correct. All six normal/off toolbar
icons now have distinct expected original artwork; each centre pixel matches its
own source PNG exactly, including Skip's different grey-blue pixel. Hovering the
original Hide location displays the shared measured 「ウィンドウ消去」 tooltip and
does not advance the original `p` click wait. Both reports have null native error
and250/251 original movie update callbacks respectively. This verifies the
bare-image alias correction in actual GPU pixels, beyond the CPU graph test.

A separate native input frontier remains visible: Hide's hover artwork stays
normal blue instead of the original pink hover image. The original ButtonLayer
sets `Butt_mouseOn` and redraws only in `onMouseEnter`/`onMouseLeave`, while the
current adapter dispatches `onMouseMove`/Down/Up without those enter/leave events.
The actual hover report likewise has no enter/leave dispatch. Shared tooltip
presentation does not establish correct native hover highlighting; no event fix
or newer binary is claimed in this entry.

Codex installed-player8 publication (local only):
- Release build with RFVP/BGI/NScripter/KRKR/arc formats/Circus/CatSystem2/Majiro features passed; immutable SHA256 `e86cbd7f0fb37b4a5913466a569392cf35bf1c316347bb9d6cbfd0c3376ad574`, `target/verification/majiro/player8`. Four existing managed `local/launchers/crysirror[-krkr/-rfvp/-siglus]-dev` launchers atomically repointed to this verified local binary. No commit/push/CI/deployment.
- Root viewed player8 Natsuzora first dialogue and toolbar evidence: normal button sprites now retain separate original immutable images instead of aliasing a reused temporary surface. Original opening movie/background and measured tooltips remain shared Media/UI. Native Enter/Leave hover highlighting is a separately identified remaining gap, not claimed fixed by this publication.
- Player7 already explicitly consumed after/LB first voice samples through shared NWK/OVK mounts and mixer. after reached scene628:43 with visible room/Tomoyo/dialogue. LB reached scene513:59 after the voice naturally ended. LB later stops at original `CGMS12?030`: the pinned native suffix selects tone-curve table2, not an atlas frame. Arbitrary RGB LUT support remains a shared-engine gap; the suffix was not stripped to fake success.
- Majiro ENV/settings follow-up: detection records the actual unique ENV resource
  path. The startup worker reads it and an explicitly labelled `portable_client`
  settings file from that client's Save capability. Native REG_SZ decimal text,
  ENV defaults and current speed globals remain distinct; generic reg_write does
  not overwrite the current speed global. Dedicated setters stage a candidate,
  call shared begin_write_atomic, retain their native PC while Pending, and only
  publish/ACK after Ready. Strict original registry import is unsupported and
  diagnostics report import_performed=false; this is not an original game save.
- Final release validation passed4 unit +1 shared Canvas +5 native tests.
  These include Pending/Failed write receipts, a stale receipt, one admission,
  unchanged PC/operands/current values on failure, portable override/restart,
  foreign scopes and duplicate/corrupt JSON. A first failure-injection assertion
  inspected only anyhow's outer message; the test now checks its complete cause
  chain and passes. Local owner tests additionally passed2/2 in
  `owner-runtime-settings.log`: the original script advances40 instructions,
  completes an actual shared storage commit, and stops at **pc0x23e, line60,
  syscall0x81ce0485 `fast_mode_disable`**. It still has no Canvas/title image.
  That native call also conditionally clears CapsLock through original keyboard
  state; no fake void ACK or private keyboard controller was substituted. Shared
  keyboard lock-state input/policy is the next capability requirement.

### KRKR original pointer event semantics follow-up (2026-10-31)

- Implementation: native LayerManager policy now dispatches Leave, re-hit, Enter
  and at most one corrective re-hit after an Enter handler changes the target;
  same-layer movement does not repeat Enter. Existing engine Layer/Canvas hit
  resources are reused. Automatic mouse capture receives drag Move/Up, native
  `releaseCapture` can cancel capture during Down, and all released mouse buttons
  trigger release/recheck. Hiding/disabling runs original BlurTree Leave without
  dropping capture; reparent Part releases capture; shutdown suppresses retired
  callbacks. Focus deactivation releases capture. A slow >1000ms recheck uses the
  existing shared host clock and does not synthesize Move or restart an unchanged
  tooltip. No shared input/scene/runtime API or graphics algorithm was added.
- Native click mapping follows the inspected Win32 caller chain: Window then
  Layer, Click before Up, and Click coordinates are the shared last mouse Down
  point (updated by either supported left/right button). Complete LayerManager
  source and the relevant Layer/Window/DrawDevice/Win32 caller routines were
  inspected at real krkrz commit `fd5c4baa6a2ef5978db1bd043634351f48667daf`;
  added file hashes/review scope are in `docs/krkr-bound-xp3-label-sources.json`.
- Compilation/tests: release authored suite passed27 tests,3 external-resource
  tests ignored by default. Four mouse test groups cover same-layer/cross-layer
  order, reentrant hit changes with the original one-recheck cap, captured drag,
  Click-before-Up/press coordinates including Window order, explicit cancellation,
  focus loss, hiding/Part/shutdown and idle recheck. Final left/right chord Down
  point refinement and its additional assertion were added afterward; that
  assertion has not yet been executed. The queued repeat test was cancelled
  before compilation to avoid blocking the coordinated player9 build. Player9's
  KRKR release library compiled after the final source write; production source
  hashes are retained in `native-mouse9-source.json` (set SHA256
  `919ae61fe0220d53bb34fa1b00a1af5b7eb612032474a2917459bbc71380f61d`).
- Behaviour remains distinct: player8 also rendered ExE's original complete
  cough dialogue and message UI (`exe-first-line8.png`, inspected), with null
  native error and original `p` click wait. Final pointer changes await player9
  original-script GPU hover verification; player8 cannot establish them.

KRKR final player9 behaviour
(`3af4e6303e6dd320afea76fd89009bd4d3143333dab83a1c3f3d8c804140973b`):
- `natsu-tooltip9.png` was actually inspected after normal original bootstrap,
  Start and ordinary pointer hover. Original first dialogue and decoded background
  remain visible; five normal/off icons match their original centre pixels, while
  Hide alone now matches the original hover pixel `[244,106,195,255]`. The measured
  shared 「ウィンドウ消去」 tooltip remains visible. Native error is null, original
  `p` is still click-waiting, with9 Enter,6 Leave and253 movie-update callbacks.
  This establishes actual original ButtonLayer hover execution with the new
  native event sequence; the player8 hover limitation above is historical.
- The final chord refinement was verified without another Cargo build using
  player9's real offscreen `render` command and the authored ignored-target
  `chord9-fixture` project. Its native TJS compares complete Window/Layer traces
  for left→right and right→left Down chords, including shared last-Down point15,
  Click before Up, captured local coordinates and capture surviving until both
  buttons release. Both assertions ran, then a Timer asserted stage2 completion
  and emitted `native-chord-source9-complete`. `chord9.log` was parsed to require
  null native error, both assertion-stage receipts and the final receipt.
  This is current-binary execution evidence; the newly added equivalent Rust
  chord assertion itself remains unexecuted. No extra production edits, Cargo
  builds, original executable execution or game windows followed source freeze.
- Earlier real Ctrl down/hold/up GPU runs for both games, first story waits and
  original OP media are recorded above. These bounded runs do not establish
  full-game completion, all plugins/saves or audible device output.

System4 native VM implementation (first-class crate, source frozen):
- `crysirror-system4` now contains a bounded original AIN v5 stack/heap/page executor, native i32/f32 bit values, slot identity/reference counts/sequences, two-word variable references, original argument/return convention, and resumable constructor/destructor/array continuations. Startup allocates all global storage, applies initializers/references, runs original function `0`, then global constructors and main. Lazy member reads and sequential struct-array construction preserve mutation and allocation order; local/copy identity is reserved before child allocation. No C engine, renderer, text layout, image/effect/audio mechanism or private logging system was adopted.
- The actual VmAdapter port owns capability reads, AIN decode/layout validation, heap allocation and native instruction batches on shared TaskPool workers. Owner completion uses existing HostServices clock for GetTime/Sleep. Unknown HLL/syscalls retain native PC/arguments; no factory is registered, no title/playable integration is claimed, and serialized/framework/original saves remain explicitly unsupported. SOURCE.json/README/NATIVE_SEMANTICS retain GPL2+ provenance, fixed commits and exact source hashes.

System4 release compilation and validation:
- `cargo check --release -p crysirror-system4` passed. Final `cargo test --release -p crysirror-system4` passed **2 AIN +13 native VM/bridge/worker tests**; two installed-game tests remain ignored by default. Tests cover native references, constructor/destructor order, array trees and sequential object identity, local/copy allocation order, HLL cleanup without destructors, same-PC pending capture, unknown-op failure, real shared-clock deadlines and worker/owner thread ownership. Final log: `target/verification/system4-vm-freeze-verified.log`.
- Explicit installed Rance7 bootstrap test separately passed1/1 (`system4-rance7-vm-final-verified.log`). Original OutputLog.dll was inspected statically only (SHA256 `8c023f38776994db5c7cc36a5ce409b433efc251a879fcf832b607b62fee04c8`): reviewed Create RVA0x1000 queries an already-loaded DPLogViewer.ade/GetInterface; absent interface returns -1. The native adapter takes only that verified disconnected branch, twice in the actual constructors. No original DLL/EXE was loaded or executed; no game bytes/fonts were added.

System4 actual behavior and next shared boundary:
- Actual original Rance7 initialization executes1,540 native instructions and stops at `CSactSystemLog@Output`, PC0xa4a, original CALLSYS6 (`system.Output`), with the original string still on stack,3 call frames,7,325 live heap slots and212,108 accounted values. Main has not yet been entered. Repeated pending polling/capture preserves the exact PC and stack; no GPU/audio result is claimed. Evidence: `target/verification/system4-vm-evidence.json`.
- Next requirement is a client-scoped HostServices logging operation backed by existing EngineContext.log state/limits, also usable without System4. CALLSYS6 must emit the original encoded text through that shared operation and leave the caller's following S_POP responsible for native string release. Connected named OutputLog buffers/persistence, if later supported, must likewise be shared engine resources. Shared source remained frozen; this missing API was not replaced with a second VM log or silent ACK.

### Codex 2026-10-31 — final local publication and truthful native boundaries

Implementation and compilation:
- The final combined player9 release build passed, immutable binary `target/verification/majiro/player9`, SHA256 `3af4e6303e6dd320afea76fd89009bd4d3143333dab83a1c3f3d8c804140973b`. The four existing managed local launchers now atomically point to player9. No CI, commit or push was added.
- Public API/config reference passed all 8 tests at `61f9f936ff0989a4`, including explicit Majiro ENV and portable-client settings policy. Final Python tooling run: 201 passed (`python311-final2-tests.log`). `git diff --check` was clean.
- System-NNN partial machine authored tests: 3 passed; native program authored tests: 2 passed, one owner test ignored by default. The owner test was explicitly executed afterward and passed its **partial VM boundary assertions**, not a gameplay acceptance test.

Actual behavior:
- Root independently viewed player9 Natsuzora's original first sentence over the looping sea video, six distinct original buttons, the pink Hide hover state and shared measured tooltip. Native Enter/Leave and source-accurate capture/click ordering are active. The same binary also executed strict authored TJS assertions for both left/right chord orders; this provides final-source verification separately from the earlier 27-pass/3-default-ignored Rust suite.
- Root independently ran/viewed ExE player9 via normal installed-game bootstrap and original Start input: original first dialogue, last_tag=p/click_waiting=true/error=null, actual shared PCM consumption, no window or audio output device. This does not establish full-game, save or audible playback acceptance.
- Root ran Majiro player9 normally from the unmodified installed directory. Shared atomic persistence actually wrote the portable named EffectSpeed value; the original script then failed visibly at start.mjo PC0x23e/line60/hash0x81ce0485, before any game page/title. No CapsLock operation was swallowed.
- System-NNN owner media probe executed eight original internal/metadata instructions and retained the original story name, then explicitly stopped at original IF PC0x41/code0x55560002. Expressions, native resource/graphics/audio mapping and a playable factory remain unfinished (`systemnnn-owner-final2.log`).
- Normal CatSystem2 bootstrap now mounts all 31 actual INT archives, including the verified equivalent scene aliases, and reaches native400 at PC0x4cd. Final owner mount/source tests 2 passed, independent alias tests 3 passed; this is still an unsupported native catalog operation, not a rendered title.

Shared-production scope is frozen according to the earlier Claude coordination. The next concrete missing capabilities are shared keyboard-lock policy/state (Majiro), a client-scoped admission/completion path into the existing EngineContext log sink (System4), arbitrary RGB tone-curve LUTs (RealLive LB), and complete original Circus save-body/thumbnail/restore mapping. No private VM renderer, image algorithm or second logging implementation was introduced to evade those boundaries. Original save support has not been claimed for the new partial profiles.

RealLive RGB tone curves: shared implementation after owner resumed shared work:
- Added reusable `crysirror_image::rgb_lut::{RgbLookup,RgbLookupProfile}` and `ImageEffect::RgbLookup`, automatically accepted by existing file/Asset and immutable-resource ImageJobs and public Bitmap.beginEffect. Three arbitrary256-byte RGB tables transform all straight RGB (including hidden RGB at alpha0), preserving alpha exactly. Premultiplied profiles explicitly fail: ImageSurface/resources have no association metadata. Processing is immutable, row-cancellable and capped at4,194,304 pixels/768 table bytes; shared worker admission/memory reservation remains in force. No native LUT/compositor implementation exists.
- The RealLive adapter retains native `?NNN -> NNN/10-1` selection, case-resolved configured dat/TCC source and a bounded immutable first-use tone-bank snapshot. TCC reading/metadata/source fingerprint run on a bounded worker; shared ImageJob owns decoding and RGB execution. Original object creation stays pending at its PC until Ready, then invokes original creation semantics with original atlas/pattern metadata. Each variant gets its own immutable shared publication, never a reused surface slot; cache limits are32 variants/128MiB plus<=1MiB TCC metadata. New publication uses HostServices.publish_image_resource, retaining RGBA Arc/resource lease without owner PNG/scan/hash/copy. Source hashes/provenance and limits: `docs/reallive-tone-source-audit.json`; generic API notes: `crates/crysirror-image/RGB_LOOKUP.md`.

RealLive RGB tone curves: actual release validation:
- Final targeted tests passed8/8: shared arbitrary-table/alpha/strict-profile/cancellation4; native suffix/TCC1; native pending/immutable variants/bad-TCC bridge2; independent runtime file worker/client isolation/publication/release1. Native test removes the TCC after first load and verifies a second variant uses the retained original bank, both pixel results remain distinct and the first stays unchanged. Invalid TCC does not install an object or acknowledge the original opcode. Runtime rejects foreign input/publish/release, rejects premultiplied admission, and verifies a frozen Surface retains pixels after valid release. Logs under ignored `target/verification/reallive-air-after-lb`: `tone-straight-unit-final.log`, `tone-native-published-final.log`, `tone-runtime-published-final.log`.
- New combined-player LB behavior remains to be tested separately; these independent passes do not claim that the installed game has crossed SEEN9030:82.


## 2026-10-31 — shared host capability extraction after owner lifted coordination freeze

Implementation:
- Owner cancelled the earlier05:50 Claude restart/freeze. Native Output and Engine.log now use one bounded client-scoped canonical LogBook; existing App deque is a derived view. Engine.logs exposes bounded records with exact decimal-string u64 metadata. Owner append returns an actual commit receipt; no native log file/device output is implied.
- Added shared logical Caps/Num/Scroll state/revisions. Physical import is unknown and control unsupported; patch validation is atomic, client scoped, and only a committed Runtime restore resets session locks. Majiro fast_mode_disable maps the reviewed depth/zero result and logical CapsLock clearing, retaining Num/Scroll. Explicit physical policy fails.
- Added an actual local civil-time dependency on cancellable host workers, distinct from virtual clock. Added immutable client-scoped ImageResource publication/release with unique generation paths and retained resource leases; no owner PNG encoding or pixel copy. Frozen frames retain released resources. Runtime capture/restore rejects live publications pending reconstruction support.
- System4 CALLSYS6 CP932 Output now commits to the common log before native acknowledgement, preserving the native string result/stack ownership. No System4 factory or native presentation is claimed. Shared image format contexts expose cancellation checks for decoders.

Compilation:
- Release check of crysirror-runtime/crysirror-engine/crysirror-wgpu passed (shared-capabilities-final-check.log); Majiro runtime release check passed (logical-lock-check.log).
- Combined release desktop build passed with rfvp,bgi,nscripter,krkr,arc-formats,circus,catsystem2,majiro features. Immutable player10 SHA256 is913f4b795b1b38f5cc9920ea5d9fc8cc84fb353cc290eabba90970f9b3a7d45c. Four managed local launchers were atomically refreshed only after confirming their previous player9 identity. This binary does not register unfinished System4/System-NNN factories.

Behavior validation:
- Independent host_services3/3 passed: canonical native/public log, scope/atomic logical-lock policy/restore boundaries, and actual civil worker completion. Core log ring test1/1 passed. These tests need no native game or hardware device.
- Actual Rance7 constructor with shared Runtime committed2 Output records and advanced past0xa4a to unsupported SACT2.Init at0x6f6 after1,586 original instructions. Original HLL call stays pending; main has not been entered. Explicit ignored owner test was run and passed1/1 (system4-shared-log-owner2.log).
- Explicit Majiro owner tests passed2/2:181 original modules/496,549 instructions parsed; Runtime advanced past fast_mode_disable0x23e to unsupported syscall0x0c070535 at0x24a/line62 after42 instructions. Depth1/shared API completion confirmed, original next syscall not acknowledged (majiro/logical-lock-owner.log). No title or full game claim.
- Python tooling tests201 passed (shared-capabilities-python.log). Shared log/input/civil Rust API tests and generated reference validation are distinct from game behavior. RealLive LUT tests and installed LB behavior are recorded separately below/by their owner.


### Circus original save-body state: audited partial live model (2026-10-31)

Implementation:
- Added lossless original356324-byte slot-body codec (`native_save`) and a separate `State.native_live` model. Initial body bytes follow40EC50; all150x960-byte live sprite records follow complete444BC0/444180 constructor defaults, including original nonzero scale/viewport/visibility and integer clipping fields. No generic renderer or image algorithm was added to the VM.
- Successful native operations now retain both1100-byte reset banks,40D240 live selectors233/241, original configured-name copy order, full14-byte read-history fields/counters/contiguous extension/dedup, bounded random history, completed shared texture backing/logical sizes, sprite transforms, placement rows, music filename/id and voice timing. Sprite texture ownership is semantic metadata; host/device identities are never serialized. Imported opaque slot bytes remain lossless.
- Transient1c capture now additionally constructs original native body/sprite snapshot bytes from that live model. Unaudited operations record explicit coverage gaps; `admitted_checkpoint` rejects any gap. This remains an internal partial native image, not a writable original save. The forced autosave operation has no success acknowledgement, no skipped PC and no fabricated body/PNG.

Compilation and behavior validation:
- `cargo check -p crysirror-circus --release --features runtime` passed. Native slot codec5/5; newly added live defaults/banks/names/read-history/transforms/random tests4/4; added music/placement/voice-memory and metadata-snapshot validation2/2 passed. The freshly rebuilt release unit-test binary then passed23/23 existing native-machine regression tests after the State metadata hooks. Logs: `target/verification/circus-execution/native-save-codec-tests.log`, `native-live-tests.log`, `native-live-new-tables-tests.log`, `native-live-machine-regression.log`. Tests use authored bytes; they do not establish original save reconstruction.
- A newly built actual headless engine accepted original title press/release and executed1,563 native instructions intoDC4_SEN20181213B. At frame876 it stopped at the unchanged forced autosave0, pc0x83e/opcode0x18/selector4. The native1c checkpoint retained1,082,140 bytes of logical/native metadata and172,372 bytes of completed shared resource declarations. Inspection listed43 conservative opcode coverage gaps. This binary precedes the last audit narrowing for13/47/70, native music-stop id255 correction and logical opcode11 random-history counter fix; no new game progress is inferred from those changes. Evidence: `native-live-installed-boot.log`; exact commands and remaining offsets: `CIRCUS_AUTOSAVE_HANDOFF.json`.

Explicit remaining boundaries:
- Native49/425C20 sprite/resource resets and animated final-state metadata;4f/425F50/425DC0/423070 scene activation, object740/908/904 and conditional string fields;3c background-root reset and resource alias/presentation lifecycle;28 falling-wrapper saved policy;4c/71 native voice/SE filenames/loops;ab backend1 saved movie tail. The live image cannot yet be admitted as a complete slot, and original1d/slot restoration remains unsupported.
- Original save thumbnail4250B0 enables6276/6277, calls4226B0(1), enables native layer1 and stops the render loop at native index80. Native UI at70 has additional suppression/selection rules, including5970/2599 under a2=1. Generic scene plus all UI is not equivalent. ImageInput::Raster consumes image resources; MeshFrameHistory captures a single mesh. A client-scoped asynchronous capture of the proven selected complete snapshot must reuse existing desktop materialize_snapshot/read_snapshot and shared resize/PNG facilities before128x72 thumbnail persistence can be accepted. No private screenshot renderer was added.


## Shared API reference final validation

- Final generated API reference validation passed8/8 after adding shared host data and opaque Circus NativeLiveState; current fingerprint f59d9a19867f34c9 is logged in docs/api/CHANGES.md (shared-capabilities-api-final2.log). Python tooling rerun after regeneration passed201/201; git diff --check passed. No commit, push or CI was performed.

- Final small native11 correction separately passed1/1 in release: resetting the original random-history count also clears its logical Vec, while original native raw history tail is retained. `native-live-random-reset-tests.log` records this current-source result; no further game behavior was claimed.

RealLive LB actual GPU validation of the shared RGB LUT fix:
- Immutable player10 SHA256 `913f4b795b1b38f5cc9920ea5d9fc8cc84fb353cc290eabba90970f9b3a7d45c` was verified locally and run through normal installed-game detection/bootstrap, original Seen, voice mounts and ordinary Ctrl/mouse down-up input. Apple M5/Metal offscreen capture created no window or audio device. At10,000 frames LB crosses the former SEEN9030:82 wall and reaches scene513:91, showing the May13 dormitory corridor and original Japanese narration. At16,000 frames it reaches scene513:203, showing the cafeteria and original 【理樹】「…恭介は！？」 dialogue. Both screenshots were viewed; both reports have error=null and normal Pause17 input wait. No new unsupported native wall was observed within this bounded run.
- Native tone-image cache is ready2/pendingfalse at both10k and16k; repeated original object-file calls rose7→21 without new variants. The16k report acknowledges4 KOE calls and actual shared PCM mixing11,523,840 frames (peak1.7876658,rejected0,underrun0; active1 at final capture). This is decoder/mixer consumption, not a hardware audio-output claim. Reports/screenshots/logs are ignored local evidence under `target/verification/reallive-air-after-lb/lb/player10-story-{10000,16000}*`. Actual shared publication byte/count statistics are not exposed in player10 inspection; ordinary asset-cache resident_bytes was not misreported as LUT-resource memory. The native count2 remained stable; source limits32 variants/128MiB RGBA/1MiB TCC still apply.
- Pixel execution, immutable RGBA, publication identity/lease and renderer access belong to shared engine services. The native adapter retains filename/table choice, original atlas/pattern metadata, pending opcode and bounded policy state. Runtime snapshot/restore rejects live image publications, and RealLive checkpoint/original-save export remains unsupported; ephemeral published paths/Arcs are not persistent save identities. Future save support must reconstruct these shared resources from original native resource/table policy. Production source is frozen after the verified tests; no additional game runs, commits or CI were made.

### System-NNN execution and shared Black Cyc formats (2026-10-31)

Implementation:
- First-class `crysirror-system-nnn` now independently expresses bounded postfix
  i32 expressions, reference-preserving assignment/compound/unary operations,
  native positive-only conditionals, IF/ELSIF/ELSE/WHILE/block control, exact
  original operand consumption and instruction-local variable commit. Direct
  and postfix constant profiles preserve their distinct original decoding.
  Native user calls/control boundaries retain pending receipts and original PCs.
- Native encoded-name tables retain empty comma slots and byte names. Explicit
  Legacy1000/Expanded2200 profiles preserve original persistent/nonpersistent
  ranges and actual configured NewGame initial data. Unresolved system/calculated
  variables and Layer-affecting writes fail visibly. No System-NNN runnable
  factory, selection UI, private image/text/renderer/audio engine or original
  save compatibility was introduced. Unlicensed SystemNNN source was used only
  for reviewed format/behaviour facts, independently implemented in Rust.
- New public MIT `crysirror-formats-blackcyc` implements Resource ArchiveFormat
  GPK/GTB and VPK/VTB, ImageFormat DWQ, and FileFormat VAW/WGQ→standard WAVE/Ogg.
  Root workspace changes are only its member/dependency rows. Companion reads
  use scoped resolvers; entry readers are checked subranges, not archive copies.
  Index/name expansion, placement, dimensions, mask palettes and row runs are
  bounded. DWQ preserves plain RGB versus packed BGR and top-down rows; loops
  use ImageDecodeContext cancellation. VAW delta output reserves a shared lease
  before allocation, rejects truncated bits and feeds existing shared Audio.
  Over-2GiB/64-bit-footer archive variants and unsupported image layouts fail.
  GARbro's four complete source files were inspected at real MIT commit
  `b09ee4570ccb1daf6ac56710ee8934dc0b8baeb0`; hashes/licence remain in the crate.
  Public registries are callable independently of a VM; Runtime auto-registration
  and desktop/System-NNN playable support are not claimed for player10.

Compilation and local validation:
- Release checks/tests compiled both crates. Final authored tests passed13:
  Black Cyc3, System-NNN expressions/config5, machine3 and program2. Two
  installed-data tests are ignored by default and were explicitly executed
  separately, each passed1/1. `target/verification/systemnnn/final-authored.log`.
- Black Cyc owner test opened all6 GPK and35 VPK companion tables. All1625 DWQ
  decoded:1122 PNG8A,464 JPEG5,35 plain BMP0,2 packed BMP+mask3A and2 BMP+mask2A.
  Five actual decoded profile images were inspected. The 2A source is the game's
  own `ta_error.dwq`/UNKNOWN DATA artwork, not a CrySirror fallback. Original
  entry names/encoded hashes/decoded RGBA hashes/alpha ranges are recorded in
  `owner-image-profile-evidence.json`; actual mask extrema are3A0..252,2A0..255.
-156 selected VPK entries actually decoded through shared Audio.open_asset,
  producing2,555,904 finite PCM frames, peak0.9739774. This exceeds header-only
  unwrap validation; no audible output device, playback/seek/loop acceptance or
  complete voice-corpus decoding is claimed. Evidence:
  `target/verification/systemnnn/owner-blackcyc-final.log` and `dwq-captures/`.

Actual original-script boundary:
- Shared InstallerCatalog reads the original MSI without installation/extraction.
  Its `init2/game.fxf` has varInitFlag=1; `var.fxf` has1000 slots, and original
  `varinitdata.xtx` supplies variable310=1. The probe applies that original
  NewGame policy/data; it does not inject a PC, branch flag or replacement script.
- Original `preprologue.spt` now executes16 instructions rather than stopping
  after8 at IF0x41. It retains story18bytes and film16bytes and yields the exact
  pending FilmTypeTime(0) receipt at PC0x173/code0x33330052, next0x178, serial4.
  Repeated polling preserves that receipt/PC/executed count. The reviewed modern
  source contains an empty FilmTypeTime handler, but equivalence to this legacy
  callback ABI remains unverified; no generic no-op acknowledgement was used.
  Next known script operation is ClearAllEffect at0x178, which still requires
  a real semantic/shared-engine mapping. Evidence: `owner-control-final.log`.
- These are executable native-control and reusable resource results, not a title
  screen, rendered System-NNN gameplay, completed integration or original saves.
  Production source is frozen for this bounded round; no full player build, CI,
  commit or game/font/key distribution was performed by this agent.


## 2026-10-31 — CatSystem2 continuation: shared loop catalog and original settings records

Implementation:
- Added reusable engine-owned AudioLoopCatalog with client-scoped atomic publish/lookup/clear receipts, revision/entry/path budgets, exact decoded-vs-reference-frame timebases, explicit Floor/Ceil endpoint conversion and no implicit PlayAsset change. Native400 retains only original comment/quote/integer/path/duplicate policy. Original44.1k reference positions and -1 open end remain explicit. Worker reads/validates the complete batch; the owner commits before native acknowledgement.
- Shared XmlDocument now exposes source-order child iteration with constant-time document ownership validation, rejecting foreign/cloned nodes without unsafe code. Confirmed mounted virtual-directory misses return NotFound consistently for streamed/read/metadata paths instead of leaking the underlying physical archive file's ENOTDIR. Shared VFS scope, provider order and size budgets remain intact.
- Native359/789 use reviewed actual constructor/default bytes and startup overlays, then gamedef and client Save/setup.xml in original WinMain order. Native mode3 resource policy prefers the configured loose INT directory before packed members: actual gamedef is loose config/gamedef.xml. Original four configuration spans are5628/72/32/12 bytes, with nonzero48-row defaults and effective-volume math; GAME/flag/max is retained. These are native VM value/layout policies, never a private renderer/mixer. Diagnostics explicitly say Windows configuration was not imported. Unknown/malformed shared XML fails visibly; permissive original XML parse-failure behavior is not claimed.
- Runtime snapshot/capture and restore reject live loop catalogs until resource reconstruction is implemented. Common publication remains usable without CatSystem2; original game saves and native audio playback remain unfinished.

Compilation:
- Native loop/runtime release check passed (native-loop-check3.log). Native settings plus Runtime release check passed (native-settings-check.log). No original EXE/DLL was executed. All commands/logs are under ignored target/verification/catsystem2.

Behavior validation:
- Shared audio catalog/explicit conversion3/3, native loop-text policy2/2 and XML5/5 passed. Runtime registered-client/atomic/restore coverage2/2 passed again after final shared VFS change (loop-runtime-final-tests.log). Mounted virtual missing-member open/read/metadata/client-isolation regression1/1 passed (vfs-virtual-missing-tests.log).
- Fresh CatSystem2 library tests11/11 passed including real nonzero constructor, startup overlays, loose-default priority, setup override order, original effective-volume math, unknown XML and cancellation (native-settings-final-tests.log).
- Explicit actual owner CPU test passed1/1: original main executed8,342 instructions, committed17 BGM loop metadata records/revision1, applied real loose gamedef with no setup file, copied original settings and returned original flag count2048. It advanced past native400/359/789 and stopped at native209 PC0x526,args[2048], retaining the original call. Log: owner-settings-native-actual2.log. The earlier strengthened owner test caught a missing gamedef overlay and failed; it was corrected through source-proven mode3 loose-file precedence, not by weakening the assertion.
- No CatSystem2 title, CST/FES execution, gameplay, physical audio or original-save pass is claimed. Native209 kcInterpriter constructor contains variable banks,22528-byte command state, a list and an underlying parser; it remains unsupported pending actual implementation. No commit/push/CI.

CatSystem2 continuation final local validation:
- Full default-feature release suite passed22 tests (library11 +native KCS integration11), with2 owner tests ignored by default; ignored tests were not treated as passes (native-catsystem-final-suite.log).
- The freshly compiled owner binary was then explicitly run with authorized Neko resources: both original31-archive/KCS/CST corpus and actual native adapter tests passed2/2 (owner-catsystem-final.log). The adapter still stops at209/PC0x526; this does not add a title/gameplay claim. All production source is frozen for the parent's combined player build; no validation process remains in this subtask.


### 2026-10-31 — System4 shared SACT initialization and text-format extraction

Implementation:
- The first-class native worker Config now carries optional logical canvas and
  explicit Gothic/Mincho font aliases. SACT2.Init requires published shared font
  faces/measurement and device-free channel audio, then commits the shared
  opaque-black hidden main Canvas1 and engine Layers through one atomic owner
  batch. Repeated same-client Init is idempotent; cross-client reuse fails.
  Missing font/canvas admission does not acknowledge the original HLL. Native
  screen/main-surface/message-count/clock queries preserve their reviewed ABI.
- Added public safe CPU crysirror_text::format::format_one_bytes, independent of
  VMs/fonts: first compatible `%d`/`%s`, bounded width/zero padding, encoded-byte
  preservation, original ignored integer/string precision and declared-size
  string copying (including value NUL). Template/output<=1MiB and fields<=510;
  unsupported conversions/template NUL fail visibly. No private VM formatter.
- S_MOD maps only native type/refcount/PC policy to that shared kernel. The format
  slot is released; a string argument is released only if a matching `%s` used
  it. Unsupported formatting leaves the original stack, heap and PC unchanged.
  Provenance and scope remain in crysirror-system4/SOURCE.json; no original
  executable, C engine, renderer, font engine, SDL mixer or hosted screen runs.

Compilation:
- Final `cargo check -p crysirror-system4 --release` passed
  (system4-final-check.log). Release VM/host and shared-format tests compiled.

Behavior validation:
- Shared byte formatter passed5 targeted tests (three unit tests plus two
  independent integration tests); literal `%` overlap, sign/zero/precision,
  encoded byte width, embedded value NUL, unused string and output limits are
  covered (system4-format-final-tests.log, system4-format-byte-final.log).
- Original VM release tests15 passed/1 default ignored; includes new native
  S_MOD reference-release and unsupported-operation atomicity tests. Shared-host
  missing-font/no-resource-commit test1 passed/2 default ignored. Both ignored
  runtime-input tests were then explicitly executed:2/2 passed using a local
  system font published through engine FontLoadJobs. Idempotent initialization,
  foreign-client rejection and actual engine Canvas pixel/hidden Layer state
  were verified (system4-format-vm-tests.log, system4-sact-format-owner-final.log).
- Rance7's real original constructors now execute11,053 instructions and commit
  five canonical Output records, advancing through SACT2.Init0x6f6 and
  S_MOD0x17e0e. Current explicit frontier is SR_ASSIGN opcode0x4e at0x384a in
  menu_metrics_t@0. It is original struct-page assignment, not string formatting;
  old-page deletion/deep copy/destructor/slot identity must be implemented.
  Main remains false. No factory, native title/gameplay, complete HLL, physical
  audio/window/GPU or original-save support is claimed. Root player13 does not
  link System4; this evidence is library/VM plus shared Runtime owner execution.
- Next-step source/commands are in ignored target/verification/system4/HANDOFF.json.
  Source is frozen after this bounded result; no commit, push or CI was added.


### System-NNN: real Core VmAdapter worker/owner seam (2026-10-31)

Implementation:
- Added first-class `adapter::NativeAdapter::from_prepared`. Original VM instruction batches own a bounded TaskPool worker; original pending receipts return to an owner-only `Bridge`. No HostServices/window/GPU/audio pointer reaches that worker. Bootstrap read/decrypt/config/NewGame data also runs on a cancellable scoped resource worker in the owner test. There is no factory/desktop automatic-play registration.
- Proved the legacy FilmTypeTime no-op by static original-module inspection: moduleSHA e85e92ec538d50eae08d27286177149972d443a96d1415277c1ae1f9ee108bdb, CGameCallBack RTTI/vtable0x564f04→complete ReceiveScriptCommand0x541af0 has no branch; original ExecScript0x4ea210 passes its scalar then advances. The original executable was read from MSI but never run/distributed. Exact legacy Fill defaults/RGB behavior are additionally proven by CEffectFill0x4aba20/0x4abb10. Public unlicensed source was used for facts only, not copied/linked.
- Native policy now maps original effect clear/fill into shared retained Layers, PrepareOverlap into shared Canvas copy_subtree, and the bounded initial absent-music stop into shared Audio. No VM pixel/composition/audio algorithm was added. Existing opaque caller backdrop is explicit; it is not a reconstructed native title. Unsupported frame/read/music-fade/message/effect/service/save semantics remain fenced at native locations.

Compilation and behavior validation:
- Release check passed and final `cargo test -p crysirror-system-nnn --release` passed13 authored tests (bridge/Core-port3, expression/config5, machine3, program2), with2 installed-data tests ignored by default. Independent host tests verify frozen snapshots survive live layer removal, stale receipts/owner errors do not acknowledge or mutate, and the actual Core worker port commits only through owner services. Log: `target/verification/systemnnn/core-port-final-tests.log`.
- Explicit original-data owner test1/1 passed on its fresh binary (`owner-core-port-actual.log`). Original NewGame config/Legacy1000 variables execute unchanged original preprologue through both direct bridge and real VmAdapter port:25 instructions,6 shared owner API calls,1 Canvas snapshot and actual opaque800x600 shared Fill Layer100. Original story18B/film16B remain. It now stops at originalPC0x1a1 / SetEffect29(SUNA), rather than16 instructions at0x173. Exact pending receipt persists; Core diagnostics worker_pending=false/owns_framebuffer=false/factory_registered=false; repeated terminal step makes no owner mutation. No original title, picture render, playable game, audio device, full compatibility or saves are claimed.
- Existing ImageEffect WhiteNoise/Noise and VFX CurlNoise/SequenceRng were checked. They do not express SUNA's persistent packed2x2 mono state, original CRT15-bit draw/update order, alternating half-bank XOR,8x2 region alignment and temporal separate-term/256 blending. No static-noise substitution/private pixel loop was used. Required reusable temporal-raster API and exact next operands are recorded in `crates/crysirror-system-nnn/HANDOFF.md`. Production is frozen; no shared runtime/desktop/root-Cargo edits, full player build, CI or commit were made by this agent.


## 2026-10-31 — reproduced KRKR Ctrl exits and engine-owned fixes

Implementation:
- Reproduced the owner's game_kr_ousama exit in immutable player10 after START: native layer2502/canvas8 required alpha for an immutable Composite source. Reproduced game-kr-natsuzora after OP/long Ctrl: same missing coverage at layer34825/canvas71. Native input now prepares these graphs through shared ImageJobs before consuming the input receipt; retained Canvas paint/copies are not retargeted by coverage preparation. No private alpha compositor or ignored error was added.
- The next actual Natsuzora blocker was original１日目.ks:275 copyRect dfProvince3. Added shared Core RegionMap clipped snapshot copy, zero-hole replacement, resize and atomic fill-budget validation. Native province allocation/deallocation, absent-source source-rectangle behavior, assignImages and overlap policy remain in the adapter. These operations do not modify visible RGBA channels.
- The next actual Ousama blocker was0_1.ks:image: bare resource decoding incorrectly used the4MP expensive-effect ceiling. Shared decode/composition now accepts16MP resources; actual effect outputs retain4MP. Decode-only jobs do not reserve nonexistent Gaussian planes. Graph128MiB/node bounds and common512MiB reservation remain enforced.
- A longer Natsuzora choice/normal-input track reached２日目.ks:316 and exposed missing Layer.onBlur. Added reviewed native default onBlur/onFocus event dictionaries to constructor Window.action, preserving subtype super calls and focus parameters. Missing action follows the original default, not a swallowed unknown operation. Public source pin/hashes and native policy: docs/krkr-region-copy-source-audit.json.

Compilation and independent behavior:
- Release KRKR check passed. Fresh native suite30passed/3defaultignored, including immutable composite alpha/mutation/input, province clipping/overlap/absent-plane/resize and focus action/super dispatch (native-full-focus-final.log). Core region copy tests2passed. Public shared image tests3passed, including a3072x2048 RGBA PNG and rejection of oversized resources/effect targets (large-image-tests2.log).
- Final combined desktop release build passed (player13-build.log) with rfvp,bgi,nscripter,krkr,arc-formats,circus,catsystem2,majiro. Immutable player13 and target/release/crysirror SHA256 da3646fc82fe2505d51ca9791e1949e3384b16dcbc8805a44c45eeb02659ea7e. Four managed launchers were atomically refreshed from verified player10 identities. The binary includes Cat400/359 changes; unfinished System4/System-NNN factories are not linked.

Actual installed-game behavior:
- Natsuzora player13 completed110,000 offscreen host frames with genuine Ctrl down/up, OP, choice selection and ordinary click. Native frame count97,099; original event counters include3 key-down/3 key-up and8 clicks. Error=null, current original tagselect/normal sleeping wait at a later choice; viewed screenshot shows the evening street/another character, beyond the earlier hospital choice and old２日目:316 error. This validates the bounded route, not all branches/saves. natsu-choice13-report.json and PNG contain compact evidence.
- Ousama player13 completed50,000 offscreen host frames with two held/released Ctrl phases and an ordinary click between them. Native frame count15,792,4 clicks,2 key-down/2 key-up,12,268 onFire. Error=null and normal paragraph click wait. Viewed screenshot shows two original characters/dialogue in the house after the earlier street scene. No window/audio device was opened; offscreen behavior does not validate physical OS menus or audible playback.
- All evidence lives under ignored target/verification/krkr-ctrl-exits. These fixes demonstrate API operations and bounded real-game progress, not completed compatibility of every referenced engine. No commit/push/CI was performed.

### 2026-10-31 — KRKR shared archive/font checkpoint (ExE and Sabbat)

Implementation: XP3 now reads bounded inline alternate-name records, validates
next-File hash/name-token association, and publishes preferred-name resources
with original info-token aliases and shadowed native record IDs retained. The
shared resource XOR/Adler-32 candidate solver supports a generic folded-hash
profile; bias and zero substitution are separately inferred and every protected
open verifies the decoded Adler-32. No native executable/DLL runs. Shared font
workers/catalog/TextSystem now accept fixed-size TFT v0/v1 coverage faces;
`Text.beginPrecomputedFont(alias, asset, pixelSize)` exposes the capability.
Native Font map/unmap only binds font signatures to these shared resources.
The historical Motion.Player.varibleKeys property has static DLL registration/
getter evidence and returns actual parameter records, not an empty shell.
Source audits: `docs/krkr-folded-xor-source-audit.json`,
`docs/krkr-precomputed-font-source-audit.json`, and
`docs/krkr-motion-legacy-keys-source-audit.json`.

Compilation/tests: release resource XOR tests 3 passed; XP3 tests 32 passed;
inline parser 2 plus explicitly invoked owner archive test 1 passed. Shared TFT
decoder 2, FontResources publication/isolation/frozen lifetime/fixed-size identity
2, native Font mapping 1, and legacy Motion keys 1 passed. Player16 release build
passed (SHA256 4fb701caaed516a3d52a23b7dff59507cec7f8108ad7e1d272299a3747cd4f99).
The two fixed pixel sizes participate independently in font digests/aliases.

Behavior: all 8 owner Sabbat XP3 archives were independently sampled, including
large video and zero-domain entries (262,107,773 bytes, about 250 MiB verified). Player16
actually initialized KAG and all font mappings and rendered ATTENTION with
error:null, 140 native layers, and a real click wait. An ordinary mouse down/up
advanced beyond it and exposed the legacy varibleKeys wall; that adapter fix is
unit-validated, with player17 actual title/gameplay validation still pending.
Current player13 ExE ran two real input tracks, including Ctrl down/up and a
selection click, up to a later night-time choice: 110,000 requested frames,
96,309 native ticks, error:null. The originally reported ExE exit was not
reproduced; its exact original cause is not established. These observations do
not establish complete game or engine compatibility. Logs/images remain under
ignored `target/verification/krkr-ctrl-exits`; managed launchers are untouched.

### System-NNN native frame policy and real MSI resource diagnosis (2026-10-31)

Implementation:
- Native DRAW now uses shared frozen Canvas images, Pattern jobs and image
  resources for every original print. Zero-frame DRAW prints once without
  aging effect counts; positive frame waits count after Print and retain the
  immutable base capture. FILL54's reserved first parameter does not replace
  its native9999 maximum; NOP0 retains native99999 lifetime. Loading an image
  preserves its current Fill effect, and replacing Fill restores the assigned
  image. Active-picture PRELOAD remains an explicit unsupported boundary.
- Native resource errors retain the actual GTB path and byte length. No parser
  bounds were relaxed, corrupt high-priority pack skipped or private renderer
  added. Full frame-debt scheduling, ongoing effects during message waits and
  original VM/resource save restoration remain incomplete.

Compilation:
- Release check passed (`native-draw-effect-policy-check.log`). The current
  release bridge test binary compiled successfully (`native-draw-tests.log`).

Behavior:
- All8 current authored bridge tests passed, including DRAW0/2-frame exact
  shared Pattern steps/global RNG consumption, effect counters, immutable image
  display and retention of the next unsupported native PC. These use an
  authored known-pixel capture provider, not an original-game GPU capture.
  Evidence: `target/verification/systemnnn/native-all-current-bridge-tests.log`.
- The explicit system-font native window50 fixture passed using shared image
  jobs plus separate GlyphRuns; source masks0/128/255 yielded exact50-percent
  RGB255/190/128 and the reveal/advance click contract was asserted
  (`native-window50-font-actual.log`). This is not original dialogue completion.
- Actual player18 now waits for startup work and reports a real GTB error before
  any original instruction. Reading the MSI through the shared InstallerCatalog
  proved all6 GPK and6 GTB records are four-byte placeholders with values0..11;
  the external distribution has full same-named packs. The MSI mount currently
  shadows them. This is a source-priority issue, not a new GTB profile. Parent
  is correcting shared mount precedence; original title/play is not claimed.
  Evidence: `target/verification/systemnnn/probe-gtb.log` and
  `player18-controller.log`.
- After extracting the shared detached GpkIndex parser, its explicit owner-pack
  release regression passed1/1 again: all1625 DWQ decoded,156 sampled audio
  wrappers decoded2,555,904 PCM frames with finite nonzero samples. Evidence:
  `target/verification/systemnnn/owner-blackcyc-index-regression.log`.

### KRKR player18 actual regression checkpoint (2026-10-31)

Compilation: parent-built player18 release artifact SHA256
`03fb691a8f7e5babfc641bc79177b58bc9c4294f59703e331154919192453719`
includes shared TFT fonts and the legacy Motion varibleKeys semantic mapping.

Behavior: ExE independently repeated the 110,000-frame input track on player18,
including two Ctrl down/up phases and ordinary selection clicks. It reached the
later night-time choice with error:null, 96,775 native ticks, onClick8 and
onKeyDown2/onKeyUp2; the captured scene was visually inspected. This is bounded
progression evidence, not a reproduction or diagnosis of the originally
reported exit. Evidence: `exe-regression18.log` and `exe-regression18.png` under
`target/verification/krkr-ctrl-exits`.

Sabbat advanced beyond ATTENTION and both logos into `title.ks` at `*wait`,
line36, with error:null on player18, but the capture is dark and title UI is
hidden. Additional ordinary clicks do not resolve it. This remains an actual
behavior blocker: title/gameplay are not validated. Read-only parser/object
state diagnostics are being expanded to identify its precise semantic cause;
no visibility forcing or game-script patch was added. Evidence:
`sabbat-state18.log`, `sabbat-state18.png` and `sabbat-title-advance17.log`.
- Native stopf5 now resolves through the original function-name table, resets
  the5 explicit native skip flags and acknowledges only that reviewed semantic.
  Static original0x5355c0 and constructor table binding were verified; actual
  owner's function2 is stopf5. Authored relocation to index0 passed1/1, while an
  unknown function kept its original PC/receipt and reported its table name.
  Release check passed (`native-stopf5-check.log`, `native-stopf5-tests.log`).

### KRKR player19 title command checkpoint (2026-10-31)

Compilation: the unified desktop release build passed in3m32s with defaults
plus rfvp,bgi,nscripter,krkr,arc-formats,circus,catsystem2,majiro,system-nnn.
Frozen player19 SHA256 is
`5f5ae9a1774ef98b9b7ba204499baa71c11fffae27db5d2e5e6b7197462e7ca2`.
The subsequently added opt-in raw TJS source-line diagnostic passed the KRKR
release check (`tjs-source-trace-check.log`); it is not in player19.

Behavior: actual Sabbat9000-frame first-click run still captures a dark screen,
error:null,8285 native ticks. Opt-in tag trace proves title.loop entered
custom.ks *title_start and executed title_bg show/waitmovie, UI load and msgon.
Raw fields show kag.msgState=1,_skipMode=0,_skipNoDisp=void, while
world.msgVisible=1,envTransMode=1,basePlane=1 and current._visible=0. This rules
out the earlier hook-jump/skip suppression hypotheses and narrows investigation
to environment command/transition publication. It is not a title acceptance
pass and no game visibility/state is forced. Evidence: `sabbat-title19.log` and
visually inspected `sabbat-title19.png` under the same ignored verification root.

## 2026-10-31 CatSystem2 native interpreter and shared depth continuation

Implementation: native209 now constructs real live/backup flag banks, parser and
256 native command slots, with executable bounded integer expression semantics
(native210–213). Backup allocation is explicitly uninitialised save state.
Native1 publishes original debug text through shared LogBook;19–22 preserve
raw/formatted argument and signed-byte/CRT-fold comparison differences;36 fills
owned VM memory. Native41 uses new independently usable
`crysirror-text::byte_split` with original SJIS boundary profile, cancellation,
preflight capacity checks, spans and NUL output;42 only maps native NUL-list ABI.

Shared Scene `SetDepth` owns finite float sorting and client canonical stable
history, explicit `commit_frame_order`, group scope inheritance, transactional
rollback/clone/restore validation and frozen snapshot order. Frame computation
is pure; intermediate updates do not change committed tie order. Ordinary
layers precede depth scopes in the same major bucket. Crossfade and WGPU merge
use shared comparison. Source renderer evidence shows native Cat Z is flat:
only Cat payloads receive depth, never Task parents or transparent groups.
Native88 preserves ordinary display range0<=Z<1,000,000; distinct native capture
range0<=Z<65,536 stays unsupported. Native66 type2 constructs a genuine shared
transparent Layer group;79 creates original ARGB Solid geometry initially
hidden;92 delivers original imperative visibility events through Task ancestry.
The native empty rectangle remains separate from the engine1x1 group extent.
Type6 source constructor is also implemented as actual unbound click-controller
state; no source input registration or geometry exists until bind message89.
Its validation is queued and is not counted as a behavior pass here.

Native938 reads the actual sound configuration into shared XmlDocument AST and
returns its genuine found predicate. This is declaration representation, with
`audio_playback_mapped=false`. Native247 reads client Save CSS containers through
scoped storage and existing shared Resource zlib; original header/tag/record
layout and logical handles belong to VM policy. Missing file returns0 after
actual read. Save writing/restoration remain unmapped. Native276/277 preserve
numeric raw-byte string variables and dirty state, with assigned-empty distinct
from absent. Subsequent unknown natives retain their original PC/arguments.

Compilation: release `cargo check -p crysirror-scene -p crysirror-wgpu` passed
(`shared-depth-check.log`). Release Cat lib tests passed19/19 after247
(`native247-tests.log`, repeated after27619/19). Shared byte_split3/3 passed;
shared depth history4/4 passed after None→Some intermediate-update correction
(`shared-byte-split-tests.log`, `shared-depth-history-tests.log`). New type6 and
additional string-variable tests are queued, not included in these totals.

Actual behavior: opt-in release owner adapter tests each passed1/1 using genuine
mounted archives and original `kcs.int/main.kcs`. Native209 advanced8342→16971
instructions;41 ran100 actual calls;938 retained real5groups/6parts and79216AST
bytes;247 genuinely found isolated client Save/savegen.dat absent. Latest
completed run `native276-owner.log` reaches20059 instructions then visibly stops
at native66 type6 PC0x868, args[6,3,268], name“bg button”. Existing MENU104 command119
remains disabled;XML handles/resident bytes end0. Native88/92 were each actually
called once on engine-owned solid geometry. CPU owner test derives/commits
shared logical frames, but no Cat GPU/title/text/image/audio/gameplay, clicks,
Ctrl press/release, original-save compatibility or whole-game completion has
been established. Original EXE/DLL was never executed; owner data is ignored
under target/verification/catsystem2. Source audits: interpreter, byte-split,
task-plane, shared-depth and state JSON documents under docs/. Root owns public
API doc regeneration/fingerprints and desktop Runtime frame-history hook.

Cat continuation validation update: type6 native66 now actually executes. Release
Cat lib21/21 passed (`native66-button-tests.log`), including unbound-controller
versus Plane separation and assigned-empty string variable semantics. Actual
owner1/1 passed (`native66-button-owner.log`) and advances20059→20071instructions,
then native104 PC0x8b5 args[5,0,0,1280,720,10000,5]. Binding104 remains unacknowledged
while reusable input routing is being designed from the complete registration,
priority, candidate, capture, hover and handled-receipt source. No click/Ctrl or
rendered-title result is implied. Shared Resource inflate now checks scoped
cancellation each16KiBblock and before output publication; release transform
unit tests2/2 passed, including cancel during first block/final EOF with no
partial result (`shared-inflate-cancel-tests.log`). No public API change in this
small Resource cancellation fix.

### KRKR actual title and TJS scope checkpoint (2026-10-31)

Implementation: corrected KAGParserEx ordered tag metadata to include tagname
first and preserve repeated written attribute names, matching pinned
ArgValue::add. The native game's single-flag object-creation guard previously
returned null. Separately, the TJS for statement now uses an existing HIR Block
scope over initializer/condition/step/body. The original metadata loader reused
var i in an inner loop; leaking its binding selected an absent outer record.
No game script, filename fallback or forced layer visibility was added.
Decompiled TopLevel negative-register locals now have one root declaration and
subsequent assignments, avoiding a shadowed counter during round-trip execution.
Sources: `docs/krkr-kag-ordered-tags-source-audit.json` and
`docs/krkr-tjs-for-scope-source-audit.json`; original notices are retained.

Compilation/tests: KAG native parser release regression2/2 passed; For binding
VM regression3/3 passed; final complete TJS2 library426/426 passed, including
control-flow round-trip and fuzz tests. The first broad run exposed the
old decompiler loop-declaration problem and was terminated at its stuck own
round-trip test; it is not counted as a pass. Final evidence is
`kag-ex-ordered-tests.log`, `tjs-for-scope-final.log` and
`tjs-for-scope-lib-final.log` in the KRKR verification root.
Player21 release build passed2m10s, frozen SHA256
`efd29b6bf935f429dfa7eac2d71a10deaa0e104eb8e606e4dd7881d64dcd45a1`.

Behavior: player21 renders the real Sabbat title with all five menu items,
character artwork and logo; capture visually inspected. After the ordinary
ATTENTION click it reports error:null/8307 native ticks,
envTransMode=0/basePlane=0/current._visible=1. Clicking START in a fresh profile
then encounters a real StandImage.updateImage filename access on void baseInfo.
Raw source breakpoints proved baseInfo was already absent when the native
metadata setter ran, before onPaint; this produced the For scope diagnosis.
The scope fix has not yet been behavior-validated in a new desktop artifact.
The attempted45000-frame input track stopped at that error and is not a
45000-frame success. Evidence: `sabbat-title21.log/png`, `sabbat-start21.log`,
and `sabbat-stand21.log`. Full game progression remains unverified.

Cat continuation input update: reusable Scene InputRouting now owns scoped
receiver registry, signed stable priority, half-open press/coverage, global key
receivers, captured release/repeat, distinct hover, Core input journal and one
leased delivery receipt. Epoch/event/delivery numbers use decimal-string JSON.
Registry/priority ordering persist; live device journal/hover/captures/leases
never replay on restore. Runtime query/submit/tick/restore wiring and Scene
ServiceBatch/EngineSystems fields are in place. Unsupported multi-owner or
interactive-widget/dialog mixing explicitly fails. Menu conflicts with actual
button activity roll back before journal/clock/UI publication. Root provides
reflected InputRoutes and API docs. Shared release unit tests5/5 passed
(`shared-input-routing-final-tests.log`); runtime+Cat release checks passed
(`shared-input-routing-check*.log`). Engine lifecycle input isolation has been
implemented with matching async-frame/end_frame input ownership; its independent
3-test release run is queued, not yet counted as a pass.

Real original native104 and105 each executed once after shared publication;
`native105-owner.log` owner test1/1 reaches20079instructions then visibly fails
at848 PC0x8e5,args[5,0]. The controller's actual full-canvas native RECT is source
constant[-100000,-100000,100000,100000], distinct from empty/global press regions.
Native71 uniquely arms the original event callback; it remains unimplemented.
Ungated hover has no VM callback effect per877310 and may complete its receipt;
press/release/keyslot callbacks remain explicit original-PC failures. Source
Cat/shared production is temporarily frozen for the coordinated unique player22
build; native848 is only statically reviewed so far. No Cat GPU/title/whole-game,
actual click/Ctrl callback or original-save completion has been established.

Input ownership validation completed: release Engine lifecycle isolation tests
3/3 passed (`shared-input-routing-lifecycle-tests.log`). Both synchronous and
worker-audio completion paths expose sanitized input to behaviours/actions/
optional Input journals while the shared router retains actual Ctrl down/up;
on_update receiver creation does not reoffer an already-observed press or held
state. Menu conflicts precede behaviour/journal/clock changes. Production source
remains frozen for player22. Static native848 review found original hover-enabled
suspension retains each receiver's hover marker; shared per-receiver cache
continuation is required before mapping this call, with no completion claim.

### KRKR player22 actual progress (2026-10-31)

Compilation: the single coordinated full-feature release player22 build passed,
including shared MediaProbe/InputRoutes and native lifecycle changes. Frozen
SHA256 `5d0a93d5800a806edd22f5ca080aa691f8d2ad0456a7e19904251068c519acd2`.
Final For binding regression now4/4 passed, adding TopLevel global-index
preservation; the previously recorded complete TJS library426/426 remains valid.

Behavior: ExE repeated the full110000-frame track on player22 with error:null,
96506 native ticks, onClick8 and Ctrl down/up2 each. The later night-time girl
and two-choice capture was visually inspected (`exe-regression22.log/png`).
This still does not reproduce or establish the cause of the reported old exit.

Sabbat22 START+Ctrl passed the prior baseInfo-on-void point, then stopped with
an explicit native layer quota error1000 in PSDInfoLayer/getDispLayer/updateDisp.
The requested45000 frames were not completed. Adapter quota1000, Scene Canvas
quota1024, Runtime surface quota4096 and Scene layer quota16384 are distinct;
the next step is to distinguish legitimate PSD cache capacity from repeated
creation/retirement faults before changing limits. No enormous private quota or
silent skip was added. Evidence: `sabbat-start22.log`.

After player22 SHA was fixed, shared hover suspension was completed without
changing DTO/command/query forms. Each receiver retains its marker while hover
is suspended; same-position property changes trigger recomputation. Re-enabling
an already marked receiver emits no duplicate enter, while leaving other active
receivers in source order. Input disable/cancel/remove/restore clear appropriate
live state. Release Scene routing tests6/6 passed
(`shared-input-routing-hover-tests.log`); release Cat runtime-feature check
passed (`native848-check.log`). Native848 now changes the real shared receiver's
hover_enabled property and actual original execution advances to20083instructions,
native849 PC0x8fd,args[5,0] (`native848-owner.log`,1/1).

That owner run also injects genuine shared-journal click-down/click-up and
Ctrl-down/Ctrl-up into the actual original-script session. All four reach the
explicit unsupported native callback boundary at the same PC; repeated polling
and VM step retain the same receipt, instructions and pending native call.
Cancellation between probes is explicit test-owner action. This negative-path
receipt/PC validation is not successful game input/gameplay. Native849 remains
unacknowledged pending shared drag-state routing review; source6489E0→Base99→
INPUT153 changes receiver drag_enabled, and5E7950 broadcasts matching origin-
rectangle pointer drag messages without pixel coverage or handled short-circuit.


### System-NNN native outer startup and picture transparency — 2026-10-31

Implementation: first-class Controller now follows reviewed Nothing -> real
shared MediaProbe -> Logo -> original configured Opening background SPT, with
native mode3 and original variables (no forced NewGame/VarInit). Shared image
jobs/Layers implement Logo; native click edges survive admitted source/image
work. Separate NativeAdapter/InstalledAdapter completion pumps drain dependencies
without advancing clock, sampling input or scheduling a new VM batch. Core-wide
maintenance routing is separate parent work. Native six-rectangle geometry maps
to Layer crop/UV, LayerGrey uses public grouped fixed-luma, and CTRANS9 maps its
actual legacy14 fields to public clipped masked/unkeyed Q8 image composition.
Raw/opaque/display images are separate engine-owned immutable leases; no VM
pixel loop, renderer, private decoder or original EXE is hosted. Movie/Title
continuation and original save restoration remain explicit incomplete boundaries.

Compilation: release checks passed in native-rectangle-check.log,
native-ctrans-check.log and installed-dependency-check.log under
`target/verification/systemnnn`. No desktop binary was independently rebuilt.

Behavior: fixed player22 SHA256
`5d0a93d5800a806edd22f5ca080aa691f8d2ad0456a7e19904251068c519acd2`
ran ordinary `play Downloads/game-MUGEN2 --headless --no-audio --frames 20000`.
It passed original Nothing/Logo and entered actual Opening background, then
stopped at original PC0x37d/instruction22/CTRANS9 with the pending receipt
retained. See player22-controller.log. CTRANS source landed after22 and is not
an actual-game pass in that binary. Independent native-ctrans-tests.log passed
lib5/5 + bridge12/12 (exact mask0/128/255 and unkeyed RGB, immutable source/base,
unchanged RNG, existing Draw/Overlap/image/audio regression). First new shell
click test exposed removal before first publication; fixed only-presented-layer
retirement and reran native-shell-completion-tests2.log: installed2/2 + shell4/4
passed, including no-clock completion/failure retention and pending click edge.
No title screenshot, Start/story, long input or complete original save is claimed.


### System-NNN Opening effect expansion and reusable sampling — 2026-10-31

Implementation: reviewed original-module OldLamp109 now keeps only native
Calcu/Print trigger/count/global-RNG policy; public image Composite owns drawing.
All lamp Calcu candidates precede layer Print and commit with shared-frame/RNG
receipts. Bura74 maps native quadrant parameters into the public immutable
PiecewiseResample image operation (signed16.16 nearest sampling, independent Q8
weights, checked endpoints/work budget and row cancellation). Public ProgressCurve
owns bounded f64 envelopes/right-continuous steps; native numeric curve mapping
covers original time maps0/4 and quadratic/sqrt/cosine shapes. Other native warp
and random families still explicitly fail. Explicit MAE1 preserves current state
as proven by actual SetEffectScript0x4c18e0; no unknown-operation fallback exists.

Behavior/compilation: release native-oldlamp-tests.log passed lib6/6+bridge13/13;
native-bura-tests.log passed lib8/8+bridge13/13; standalone piecewise sampler2/2
and progress envelope2/2 passed. Native Bura shared ImageJob/count fixture and
MAE state-preservation fixture each passed1/1. These are authored fixtures and
static original-module facts; actual game frontier remains player22 PC0x37d
until the single coordinated player23 is built/run. No title/play/save pass.

API reference: shared sampler/envelope DTOs were registered from actual source.
Merged docs also include KRKR's shared Canvases.getLimits and Cat's explicit
backend Drag DTO fields (no automatic hardware drag claim). Generated fingerprint
changed `1dbdf921cc54e571` -> `bd71afbaa37a19da`; CHANGES records it. Initial bless
7/8 requested this new fingerprint entry; final no-bless release API reference
verification passed8/8 in piecewise-api-docs-verify.log. No commits or CI added.

### Shared Canvas capacity and public query (2026-10-31)

Implementation: Core now owns MAX_CANVAS_SURFACES_PER_CLIENT=4096; Scene Canvas
and Runtime pixel-surface admission use that one count policy (Runtime already
had4096). Public Canvases.getLimits() returns surfacesPerClient without a native
VM. Native KRKR layers derive2048 slots from half the shared count, retaining
one temporary transition-canvas slot per native layer. Existing node, dimension
and256MiB pixel-byte constraints remain. This is a bounded engine capability,
not an assertion that every native retirement pattern is correct. Opt-in raw
source diagnostics now also report host time, total layers and attached layers.

Compilation/tests: independent release Scene capacity/atomic-overflow/client
isolation/slot-replacement/restore1/1 passed; public reflected getLimits1/1
passed; native2048-layer admission/explicit overflow/invalidated-slot reuse1/1
passed. No device, system font or game data was needed. API docs8/8 passed and
fingerprint bd71afbaa37a19da was recorded by the coordinated System-NNN worker.
Logs: shared-canvas-capacity-tests.log, public-canvas-capacity-test.log,
native-canvas-capacity-test.log. Unified full-feature player23 release build
passed3m09s, SHA256
`2c7a0ac497d068d207b846ddbaa63c0b4f173342a2d74a0077e28762c557caf8`.

Behavior: player22 PSD cache trace made262 constructor attempts with262 distinct
info objects;261 cache assignments completed before the final quota failure.
Seven PSD storages were involved; no same-info repeated construction was
observed. This does not by itself rule out retained-resource lifetime faults.
Actual player23 Sabbat progression with the new shared capacity is pending.
Evidence: sabbat-cache22.log. No managed launcher was replaced.


### Published worker images in Canvas snapshots — 2026-11-01

Actual game: coordinated player23 (SHA256
`2c7a0ac497d068d207b846ddbaa63c0b4f173342a2d74a0077e28762c557caf8`)
advanced MUGEN2 original Controller/Opening beyond CTRANS to instruction25 /
PC0x397 at the first OVERRAP. It then explicitly failed because WGPU snapshot
preflight attempted a filesystem read for `__image_resource/16-100`; source
image resources were still engine-owned. player23-controller.log records this,
not a completed title, final screenshot or playable game.

Implementation: common WGPU snapshot dependency preflight now resolves actual
client-scoped published Surface entries directly. It retains admission-time
catalog leases and does not schedule PNG encoding/decoding or a file AssetLease.
Fake published names, foreign owners and released live paths still fail. No
NNN private renderer/screenshot/provider or resource-format parser was added;
no Core/Runtime/public signatures changed, so API fingerprint stays unchanged.

Validation: release CPU preflight1/1passed verifies direct in-memory resolution,
cross-client rejection and frozen/live release behavior. Explicit real GPU
fixture1/1passed on Apple M5/Metal in published-snapshot-gpu-test3.log: shared
ImageJob -> publication -> capture admission -> clear/remove/release live path
-> existing engine compositor -> worker4x4 red readback with clock0. First GPU
fixture had incorrectly combined Layer creation and Canvas copy in one ordered
service batch; it was corrected to create the source first and rerun. This fix
is after player23 and still needs the next coordinated actual-game run.


## 2026-11-01 — CatSystem2 native event waits and shared image-list initialization

Implementation:
- Mapped native848/849 to canonical shared InputRouting hover/drag-enabled state. Shared per-receiver hover markers preserve source suspension/re-enable behavior; explicit Drag events retain origin-rectangle and broadcast semantics. Automatic hardware drag and controller observation remain unimplemented. Engine lifecycle isolates routed/raw input consistently through behavior registration and async frame completion.
- Mapped native71 original event wait: validate caller slot/result, yield, then actual nativeMain next owner tick resumes with false and leaves its16-byte event slot unchanged. KCS VM atomic message-copy/boolean resume is independently tested; original logical keyslots/controller callback/result projection remain unsupported.
- Added native type15 image-list constructor and256 initialization using reusable shared ImageJobs and actual engine ImageResource publication. The original empty bitmap wrapper is preserved; it is not a directory scanner. Page nodes are dirty rectangles, not a second packing/render algorithm. Native260 stores upload XY only,262 source publication flags only. Original image composition/entry admission/upload/capture, task34/FES and source resource replacement remain unsupported. Source audits record the full reviewed vtables/functions and shared/native boundary.

Compilation and independent behavior:
- Cat release library passed22/22 (native262-tests.log); KCS VM release passed12/12 (native71-vm-tests.log); runtime-feature releasecheck passed (native256-check.log). Shared routing release passed7/7 (shared-input-routing-drag-tests.log), Engine lifecycle isolation3/3 (shared-input-routing-lifecycle-tests.log), Resource inflate cancellation2/2 (shared-inflate-cancel-tests.log).
- Actual authorized original main adapter test passed1/1 (native262-owner.log), executes20,218 original instructions and retains native66 class34 atPC0x18eb,args[34,3,400]. Native71 executes twice,256/260/262 once each. Atlas3072x4096/50,331,648bytes/page256x256/count192 and hidden basePlane1280x720/Z1000 are actual client1 shared engine resources, not fake image handles. Upload XY is[0,0]; source policy fields120/124/140 are1/1/1.
- Four real Core input-journal edge probes(click down/up,Ctrl down/up) retain original failure PC/instruction count/pending call and same leased receipt on repeated attempts. They establish explicit callback failure ownership, not successful gameplay input. Owner tests use an actual shared Runtime for image preparation/publication, but do not establish Cat GPU/window/title/image-content/text/audio/FES/CST execution or completed integration.
- Evidence is under ignored target/verification/catsystem2. Root/NNN public API documentation validation separately passed8/8 with fingerprint bd71afbaa37a19da. No commit, push, CI change, original executable execution or owner data/font distribution.


### KRKR Layer lifetime and actual encoded-byte admission — 2026-11-01

Behavior before this fix: frozen player23 advanced Sabbat beyond the old1000
layer quota, reaching2022 attached native layers at engine time165600ms. The
original START+Ctrl route then failed explicitly with image-processing memory
budget while loading a1280x242 UI image; the requested45000-frame run did not
complete. Evidence: sabbat-start23.log. The shared image-processing budget is
512MiB; the separate dynamic/published image stores have256MiB budgets.

Implementation: source audit krkr-layer-lifetime-source-audit.json records pinned
krkrz native Children/Primary raw-pointer ownership. A native Layer registry
entry no longer permanently roots its TJS object. Actual script references,
suspended work and event roots remain traced; collection drops native image
leases and queues engine Canvas/Layer retirement before render/slot reuse.
This does not claim exact reference-count finalizer timing for every TJS object.
The image worker now reserves actual bounded-stream encoded bytes before input
allocation/read instead of a fixed128MiB penalty for every Asset;512MiB total,
64MiB encoded-file and existing pixel/graph/effect limits remain enforced.

Validation: release lifetime3/3 and intentionally retained2048-layer admission/
reuse1/1 passed (native-layer-lifetime-final3.log). Release image admission3/3
passed: small actual encoded input under occupied budget, encoded reservation
failure before read, and changed-fingerprint charge rollback
(image-encoded-budget-tests.log). No real512MiB allocation was needed to test
these counters. Actual game continuation after these changes remains pending.

Shared resource retirement: CanvasCommand::Destroy releases only an existing
unpresented canvas; hide its presentations first. It removes derived state and
retains independent frozen-copy nodes. Scene scoped/strict/atomic/frozen-copy
release1/1 plus capacity1/1 passed (canvas-destroy-tests.log). KRKR processed
images now use existing immutable publish_image_resource/release_image_resource
instead of a second pixel copy to mutable numeric surface slots. Caller
publication ownership ends after ordered Canvas edits commit; engine retained
paths and admitted snapshots retain their own leases. Public facade, native
publication, final compilation/docs and actual games are still being verified.


Canvas retirement follow-up validation: public data-VM Canvases.apply destroy,
active-presentation rejection, scoped same-id isolation1/1 passed
(public-canvas-destroy-test.log). Native gamma/flip/gray worker continuation,
actual processed pixels, immediate live publication reclamation after destroy
and frozen generation survival1/1 passed (native-publication-retirement-test.log).
Shared vector/path drawing plugin3/3 passed after removing numeric mutable-surface
assumptions (native-publication-drawing-test.log). Full native bridge/docs and
actual player24 behavior are still pending; these are independent CPU tests.


Final CPU validation for the retirement changes: full KRKR engine bridge27passed,
1ignored (licensed runtime-font fixture not executed); no ignored result is a
pass. Release check for KRKR/Engine succeeded. Public command coverage1/1 and
API docs8/8 passed; coordinated fingerprint d22690a23af1bf9f recorded in
CHANGES.md. The single full-feature player24 release build has begun;
actual Sabbat/ExE continuation with this source is not yet validated.

Cat follow-up source/tooling validation:
- Python tooling passed201/201 using /opt/homebrew/bin/python3.11 (python-tooling-311.log). The default system Python lacked tomllib and failed6imports; that environment failure is retained separately in python-tooling-latest.log and is not counted as a pass.
- Complete static class34 vtable/constructor/attach/tick/message review confirms a real FES task, with first-tick low-button registration and later331/332 player loading. Constructor ASM628072 reads an uninitialized native INPUT target; its125 query can have input-release side effects through7106F0, so the adapter does not invent INPUT1/zero cursor or acknowledge this task. Read-only bytecode inspection identifies later449/PC0x190e and447/PC0x1926; these are static next calls, not executed behavior. Current actual pending remains66/class34 at0x18eb.

Cat startup fence follow-up implementation and independent behavior:
- Cat now implements the existing VmAdapter.preparation_pending contract while its immutable source job is pending. Source publication yields without executing original KCS instructions, so shared Runtime startup polling does not move the native script into a game tick. No new shared API was introduced.
- Independent release preparation test passed1/1 (bootstrap-preparation-tests.log), using actual Runtime, an authored original-format KCS and a peer client. Preparation keeps native instructions/PC and engine frame/time at0; first ordinary tick executes2KCS instructions thenHalt while the peer observes that same Ctrl press. Real owner regression for this latest hook is pending separately (bootstrap-owner.log).

Cat latest startup-hook real behavior:
- Authorized original adapter release regression passed1/1 (bootstrap-owner.log), explicitly confirms program publication at0original instructions/PC before ordinary stepping. It then again executes20,218instructions and retains66/class34 at0x18eb, with the same actual engine atlas/base and four retained input callback failure probes. This supersedes the pending owner check immediately above.
- Immutable unified player24 SHA256 independently verified asf6cc7fc86de4ed35650c510ecdda07cc2007900f0876500727295da611ef8d98; detect-only selects CatSystem2/data prefix/1280x720 (player24-detect.json). This artifact predates the new startup-hook implementation; actual GPU/run validation is separate and remains pending at this checkpoint.


Actual unified player24 (SHA256
f6cc7fc86de4ed35650c510ecdda07cc2007900f0876500727295da611ef8d98,
release build3m08s): Sabbat START+Ctrl passed the earlier165600ms failure;
source counters show repeated Layer GC drops around900 instead of monotonically
accumulating2022 entries. It nevertheless failed again at176640ms with1015
native Layers on shared image-processing memory admission. The requested45000
frames did not complete (sabbat-start24.log); there is no final screenshot/pass.

Further inspection found native loadImages eagerly launched another complete
RGBA decode for every non-opaque Asset, even hidden PSD cache layers. The native
source now retains its immutable Asset description; existing shared effect jobs
decode when required, and existing before-input prepare_hit_images prepares
visible positive-alpha candidates before acknowledging input. This avoids
unconditional hidden-cache RGBA copies without weakening alpha hits or memory
limits. Authored hidden-cache/alpha-input and full native bridge regression are
in progress; actual game validation of this refinement is still pending.

Cat unified24 actual launch:
- Ran immutable player24 against the actual authorized game folder with --headless --frames20000. It exited1 after about0.32seconds with the precise native66/class34/PC0x18eb failure and explicit unacknowledged original call (player24-original.log). This traverses the actual installed desktop bootstrap and Runtime; it is not a source-read-only result. Failure preceded final capture and player24-original.png was not created. No successful Cat frame pixels/title/gameplay/input callback are claimed. GPU was released to KRKR for ExE.


Actual ExE regression with frozen player24 completed110000 requested/runtime
frames, time1759984ms, native95569 frames, error:null,8 ordinary clicks and two
Ctrl down/up phases. Native Layers101, compared with484 in player22's earlier
route, after non-owning native Layer retirement. This verifies the new lifetime
and immutable publication path on the actual game; the initially reported early
exit is still not reproduced and its exact original cause is not claimed.
Evidence: exe-regression24.log/png/report.json. Sabbat lazy-alpha refinement has
release bridge28passed/1ignored and is now in the one coordinated player25 build;
its actual-game result remains pending.

Cat actual canonical Runtime acceptance-boundary regression:
- New opt-in owner_actual_runtime test passed1/1 in release (canonical-runtime-owner.log,0.39seconds). It mounts31original INT namespaces into one real Runtime, independently of the focused MenuHost fixture. Bootstrap publication preserves0instructions/frame/time; normal stepping reaches20,218instructions and original66/class34/PC0x18eb pending.
- The true50,331,648byte atlas resolves through public Assets.surface and remains unavailable to a foreign client. The same Runtime owns the actual hidden1280x720/Z1000 base Layer and original task state. Four click/Ctrl down/up events pass through Runtime.tick and canonical InputRouting; unsupported native callbacks preserve original pending/instruction count and the exact leased receipt on repeated query. Explicit test-owner Cancel separates failure probes. No successful original callback, title, game frame pixels or completed integration is claimed.


## 2026-11-01 — System-NNN actual Controller crosses first overlap

Actual behavior: immutable unified player24 SHA256
`f6cc7fc86de4ed35650c510ecdda07cc2007900f0876500727295da611ef8d98`
ran ordinary `play ~/Downloads/game-MUGEN2 --headless --no-audio --frames 20000`.
The original Nothing/Logo/Opening chain passed player23's25instructions/PC0x397
capture failure and reached141original instructions/PC0x6c2. It then explicitly
failed on interleaved picture/effect bands. Log: ignored
`target/verification/systemnnn/player24-controller.log`. No final screenshot,
title, Start/story or complete integration result is claimed.

Implementation after that artifact: native ordered Paint DTOs place shared
ImageInput composition between original effect slots. Shared MaskSeparate256
models the reviewed x86 mask rule (each product floors separately, zero/full
endpoints, destination display alpha retained). Generic raster work stays in
crysirror-image/Runtime jobs, not the VM. Release compilation, independent
behavior tests and API regeneration are being validated separately.

System-NNN ordered painter validation follow-up: release lib8/8 plus bridge16/16
passed (`ordered-paint-native-tests.log`), including two consecutive native frames
with grey → upper red fill → grey yielding exact RGB75 and unchanged RNG.
Independent shared bitmap mask kernel1/1 and Runtime image graph1/1 passed
(`mask-separate-kernel-tests.log`, `mask-separate-service-tests.log`): mask0/1/128/254/255,
separate-product floors, clipping, retained receipt, client isolation and invalid
opacity rejection. Final releasecheck passed (`ordered-paint-check-final.log`);
first check had an Error-vs-anyhow type mismatch corrected before these results.
API generation recorded `d22690a23af1bf9f` → `6781d4170ef6e9f2`; initial bless7/8
required the CHANGES entry, then no-bless docs8/8 plus command coverage1/1 passed
(`mask-separate-api-docs-verify.log`). The actual game has not yet run this new
source; coordinated next desktop artifact is still required.


## 2026-11-01 — CatSystem2 real FES task construction and image-list binding

Implementation:
- Resolved the conservative class34 constructor boundary through complete native message routing.627E50 sends125 before attachment:5DC340 parent0/empty child map,5DD130/5DD210 and self/base125 handlers prove INPUT unreachable regardless of indeterminate target bits. The earlier concern about live INPUT125 side effects remains true for live calls but does not apply here. Cursor staysUnknown; no fabricated INPUT1/zero coordinate or host-policy substitution was used.
- Added first-class original FES task state, attach177 bank reference and26 parameter policy, with source-empty player/object lists and real original flags/rect/exit/selection state. Its first native child tick registers the true lowbutton through shared InputRouting; existing controller results reset via original5DE8A0. Native child ticks occur only when Main resumes from actual71, never from host quota exhaustion or asset polling. Other parent profiles remain unsupported.
- Native449 binds actual image-list manager bitmap dimensions from the engine ImageResource and source task uploadXY. It changes native references/rect, not drawing. Genuine447 creates a player/local bank and loads FES/object resources; it remains unsupported rather than acknowledging an empty player.

Compilation and actual behavior:
- Release Cat library passed23/23 (native66-fes-tests.log), including real shared lowerbutton registration, first/second native tick distinction and prior-controller result reset. Actual original adapter passed1/1 (native66-fes-owner.log), now20,227instructions, original447 atPC0x1926,args[8,406], textflow, pending retained.66 executes4times,449once,71twice.
- Real FesTask8 binds imageList6/rect[0,0,3072,4096]/upload[0,0], raw-string bankowner3, unknown cursor, unloaded player and first_tick=true/lowbutton absent. This correctly avoids running the new task during quota/worker retries before the original Main yields again. Existing controller5result is0. Four click/Ctrl down/up probes retain the same failure PC/pending/instruction count/lease.
- No actual FES commands, Cat title/gameplay/image-content/GPU pixels/audio, successful native input callback or original-save reconstruction are claimed. New canonical Runtime regression is pending separately; previous canonical31INT result and immutable player24/25 frontiers refer to their earlier fixed source.


### System-NNN native FadeOut11 — 2026-11-01

Implementation: complete pinned FadeOut source plus original static callbacks
0x4aab70/0x4aac80/0x4aab10 establish argument defaults, reverse Q15 weights and
CountIsMax -> Fill54. The adapter uses existing public RgbLookup jobs. Terminal
fill preserves native count/max, suppresses picture underlay, uses exact bytes
and remains present during DRAW0. Native g/b=-1 fade inheritance deliberately
changes to independent byte wrapping when command becomes Fill. No native pixel
loop, new public API or original-engine execution was introduced.

Independent release behavior: native-fadeout-final-tests.log passed9/9 library
and17/17 bridge tests. The new cases prove intermediate Q15 weights, negative
channel transition, exact255 terminal colour, frame/count/RNG policy, DRAW0 and
unknown-PC retention. The earlier native-fadeout-tests.log passed8+17 before the
negative-channel follow-up; the final run is authoritative. Dedicated release
check is separate. Actual MUGEN2 remains at the player24 observed141/0x6c2
frontier until the next unique desktop build; static0x86d is not execution proof.


### Sabbat actual clipping frontier and shared Canvas CPU cost — 2026-11-01

Actual player25 (SHA256
4241833a6b0bfdfd9185df9277a7450264588403f7f52e2291da47b07106cab5,
release build2m13s) passed both previous image-budget failure points. Source
traces reached225136ms/native1893 Layers before the diagnostic2048-hit cap;
subsequent original sceneplay stopped explicitly at missing clipAlphaRect,
AffineLayer.tjs:onPaint489. No45000-frame completion or final capture is claimed.
Evidence: sabbat-start25.log. The repeated hidden-cache RGBA preparation was
removed without disabling before-input alpha preparation. Independent complete
native bridge28passed/1ignored at that stage, including32 hidden non-opaque
Assets returning without parking and real positive/transparent input delivery.

Implementation after25: one Canvas per native Layer now shares all4096 logical
surfaces. Previous2048 adapter cap incorrectly reserved a second canvas for an
image-only transition, which actually adds another presentation of the existing
source. No engine count/byte/node limit was raised. Native4096 admission/reuse
1/1 and image-only transition/surface-count fixture passed within complete
bridge29passed/1ignored. Existing native lifetime3/3 also passed.

CPU evidence: actual macOS1s sample (sabbat-start25-sample.txt) placed600/608 main
thread samples under native drive_sound.flush -> shared Canvas rebuild; the hot
Layer apply path rebuilt the whole client parent/child index on each small
Remove batch. LayerSystem now retains a derived per-client adjacency index,
updates Put/Remove incrementally, and invalidates rollback/install/deserialize.
It is skipped in saves and public DTOs; clones remain independent. Releasecheck
passed; independent reparent/cross-client, failed batch/nested rollback and
clone/deserialization3/3 passed, plus Canvas capacity/destruction2/2. Actual
speed improvement still needs the next original-game run, not inferred solely
from the profiler or unit timings.

Missing clipAlphaRect: complete pinned wtnbgo/layerExBTOA Main.cpp/manual/readme
inspected at a57e7a25a9e079f95b93cf9768949e0f8789f685. Public engine AlphaClipRect
and ImageEffect pair job implement exact coverage multiplication, optional
exterior clearing, associated-channel profile and forward overlapping self
reads. Bitmap.beginAlphaClip exposes the same worker without a native VM.
Native mapping only coerces arguments, waits for engine Canvas image preparation
when needed, and publishes the shared result. Source/licence notices retained;
no original DLL execution or private pixel loop. Other BTOA exports remain
unsupported; this is not whole-plugin completion.

Validation: kernel3/3 passed (including all65536 alpha pairs, geometry, hidden
RGB, clear profiles, self feedback, cancellation and wide coordinates); public
Bitmap pair snapshot/client/revision fixture1/1 passed. Native complete bridge
30passed/1ignored, native4096 admission1/1, lifetime3/3 passed after math/cache
changes. The final partial plugin registration test, encoded-budget regression,
API docs verification and actual player26 are pending.

System-NNN FadeOut compilation follow-up: dedicated `cargo check --release -p
crysirror-system-nnn --features runtime` passed in9.79s
(`native-fadeout-final-check.log`). Actual player continuation remains pending.


Alpha-clip final CPU follow-up: partial layerExBTOA registration plus original
TJS call/continuation/pixels1/1 passed (alpha-clip-plugin-test.log); unchanged
actual-encoded image budget3/3 passed (alpha-clip-budget-regression.log). Public
Bitmap client/snapshot/revision fixture1/1 passed. New effect/source API signatures
are frozen; docs verification is coordinated separately and player26 is queued
behind it. No actual26 success is yet claimed and no launcher was copied.

Cat latest canonical447 regression passed1/1 in release (native449-canonical-runtime-owner.log,0.40seconds):all31INTs inoneRuntime,20,227instructions/447PC0x1926/flow;real Fes8 constructor/binding/unknowncursor/deferredfirsttick,sharedatlas isolation andbaseLayer verified. Four Runtime.tick click/Ctrl edge failures preserveoriginalcontinuation/exactlease. This supersedes the pending canonical check above.
