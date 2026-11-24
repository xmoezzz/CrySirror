# Uncharted 2 PS3 installed-resource verification

## 2026-11-23 — BCAS20097 01.00

The local `PS3_GAME/PARAM.SFO` identifies *Uncharted 2: Among Thieves*, title ID BCAS20097, APP_VER and VERSION 01.00. The following measurements concern clear on-disc resources and do not require execution of the PlayStation binary. The input directory was opened read-only. No assets were added to the repository.

| Mounted source under `PS3_GAME/USRDIR/build/main` | Named files |
| --- | ---: |
| `pak22.psarc` | 781 |
| `actor27.psarc` | 1,311 |
| `dc1.psarc` | 177 |
| `text1.psarc` | 51 |
| `sound1/banks.psarc` | 182 |
| `sound1/music.psarc` | 446 |
| `sound1/streams.psarc` | 357 |
| `sound1/speech/english.psarc` | 17,127 |
| **Total** | **20,432** |

The 5,596,444,529-byte level archive uses 40-bit big-endian entry addresses. An independent bounded-block reader provided the comparison digests. The checked first directory record in `pak22.txt` is `con-1-start`. Its exact package match is 9,350,592 bytes; SHA-256 is `27351f592bf987f6978b740c224a0c1ecbddeb2f56f7641e33936ca9009ef253`. Its sound-bank reference maps to `convoy-local.bnk`, 3,998,544 bytes; SHA-256 is `df4158e4c8645b0f7934c633a9dee8ef7064a8591fe83c61ec2959f9820c268a`. The catalog also declares `con-1-start-ingame`. The archive has a numbered `con-1-start-ingame10.pak`, whose native selection rule remains open.

The 50,442,560-byte `train-cliff.pak` yields SHA-256 `3423bea4eabc7699e5adee0169f9807e24cf666210ce8933cb28d795aae2b945`. Native item types include the background section, material and light tables and collision data. Reading their headers establishes the resource directory; traversing them into renderable level geometry is outstanding.

The 51 native localization tables cover 17 numeric language slots and three categories. The pool holds UTF-8 strings; records contain sorted big-endian IDs and offsets relative to the pool. Independent inspection counted 120,564 string records; the same ID `0x00049e15` resolves to the English “Slow Motion” and Japanese “スローモーション”.

The release `level_resources` example ran to completion on macOS for `con-1-start`, `train-cliff` and `war-1-market-coop`. Every run mounted all eight archives and parsed all localization tables. The controller DDS decoded through the shared image pipeline to 1280 × 720 with 9,076 distinct RGBA values. The full convoy and train package digests matched the independent reads above.

| Selected level | Verified inner PAK directory |
| --- | --- |
| `con-1-start` | 10 pages and 48 items, including 35 VRAM descriptors and the native camera table. The declared sound bank resolved in its separate mount. |
| `train-cliff` | 38 pages and 107 items, including 95 VRAM descriptors and the background, lights and material table. |
| `war-1-market-coop` | Physical archive offset 4,312,806,630, 21,901,568 logical bytes, six pages and 173 items, including water and two collision records. Both referenced actor packages resolved through the actor mount with 32 and 38 items. |

Six release regression checks passed. The native checks exercise duplicate/malformed level IDs, repeated cache references, localized key lookups and invalid string boundaries, and page-relative item names and broken links. Shared format checks exercise mounted seeks across three block forms, partial final reads, EOF, corrupted zlib, invalid flags, invalid hashes, invalid offsets and directory traversal. The format registry check confirms the new ID is present once.

Run from the repository root:

```sh
cargo test --release --offline --locked -p crysirror-naughty-dog --lib
cargo test --release --offline --locked -p crysirror-formats-arc --test psarc
cargo test --release --offline --locked -p crysirror-formats-arc --test archives explicit_registration_lists_only_implemented_decoders
cargo run --release --offline --locked -p crysirror-naughty-dog --example level_resources -- "$HOME/Downloads/game-Uncharted2" train-cliff
```

Runtime entry, game-script execution, geometry, texture bindings, real-time audio, playable controls and native saves have no gameplay verification. Package selection for numbered names is unresolved. [Current implementation and scoped gaps](../../crates/crysirror-naughty-dog/README.md)

The verified local release executable is available at `target/uncharted2-validation/level_resources`. The same directory holds `con-1-start.log`, `train-cliff.log` and `war-1-market-coop.log` from the three installed-data runs.

## 2026-11-24 — textures and XVAG

The release `level_resources` example decoded every `VRAM_DESC` level 0 in `menu-temple.pak` (48), `train-cliff.pak` (95, including DXT3), `con-1-start.pak` (35) and `war-1-market-coop.pak` (159), plus the menu level's actor packages. A separate PS-ADPCM reference decode of `146_JungleExplore_LOOP1.xvag` (2 channels, 48 kHz) produced no invalid filters, no clipping and an inter-channel correlation of 0.64. Census of 200 headers per archive: music and streams are 48 kHz with 1–6 channels; speech is 32 kHz mono or stereo, all codec 6. Unit tests cover descriptor bounds, Morton order, DXT3 alpha and XVAG interleave/history.

## 2026-11-24 — fragment indices

All 37 fragments of the `menu-temple` prototype mesh decoded with the header counts exactly consumed: 41,040 half-edges, 31,830 with an opposite twin, no duplicate directed edges and no degenerate triangles. The two shared-edge code meanings were selected by maximizing opposite-twin edges over all 36 candidates; the chosen pair scored 31,830 against 11,086 for the next. Positions are not yet validated.

Positions: `menu-temple` (1 mesh, 37 fragments, 12,383 vertices, 13,680 triangles) and `train-cliff` decode through `level_resources … LEVEL OUT.glb`, which writes an inspection GLB outside the repository. Kind-4 fragments form coherent shapes in wireframe inspection. `con-1-start` and `war-1-market-coop` fail the kind-6 grid bound, and actor packages fail on a different index variant; both are open. No level has been rendered in the player.

Kind-6 positions are `(q << 2) + offset`: on `con-1-start` mesh 4 and `train-cliff` mesh 3, neighbouring fragments share 147 and 1,438 seam vertices exactly with a two-bit shift and none with shifts 0, 1 or 3. With the shift, the needed grid equals the mesh's largest position depth in 96 of 97 meshes over six levels; the remaining mesh reaches `4 · (2²⁰ − 1)` exactly. The release `background_census` example decoded every one of the 334 `pak22.psarc` packages that contain prototype meshes: 19,864 meshes, 80,154 fragments and 21,612,412 triangles, with two-triangle position-less quads in `post-56` skipped.

Texture coordinates use 1/256 of the texture: uniquely mapped statue fragments of `menu-temple` span at most 256 units, and with that unit the release `texel_density` example measures 0.37–1.51 texels per native unit across the 13 `menu-temple` material groups (123 `train-cliff` groups: median 0.76). Sampler texture IDs resolve to the package's `VRAM_DESC` word 3 for every installed material visited by `background_census` (334 packages, 80,160 fragments). Images were not rendered for this check.

`background_census` also covers `actor27.psarc`: all 1,164 actor packages with prototype meshes decode (1,683 meshes, 19,582 fragments, 9,208,671 triangles), skipping collision-mesh collections tagged `0x1001`. Skinned character fragments carry up to 17 attributes. One prop, `ice-t-prayer-wheel-gears`, stores texture coordinates in format 0xb8 and is exported without UVs.

## 2026-11-24 — player level viewer

`crysirror play ~/Downloads/game-Uncharted2 --headless --no-audio --frames 120 --capture …` detected “Naughty Dog (Uncharted 2)” and presented `menu-temple`. The 1280 × 720 capture contained 11,902 distinct colours, with no colour above 1% of the sampled pixels. A second run used an input track of 74 Page Down presses to step from `menu-temple` through the intermediate levels to `train-cliff`. It finished in 12.3 s with exit code 0 and no task panics. Its capture contained 6,668 distinct colours. Captures stayed outside the repository. Visual fidelity to the original renderer has not been compared, and window/interactive use was not tested.

## 2026-11-24 — placement and start camera

Prototype records list instance pointers. Each instance holds a culling sphere at +0x30 and a row-major world matrix in metres at +0x40. Over the installed `con-1-start` and `train-cliff` packages, 526 and 2,571 instances respectively decoded with finite matrices. Large background chunks carry identity matrices with origin-centred mesh bounds. The release `camera_rays` example cast 576 rays from the `menu-temple` start camera through `menu-temple` and its cache levels. With identity placement no ray hit. With chunks moved to their sphere centres all 576 hit, at a median of 15.2 m. On `con-1-start` (with `con-2-mid` and the numbered `-ingame10` packages) 23 and 31 rays hit respectively, at 256–304 m: inconclusive for that outdoor view.

The viewer now loads a level's packages plus those of its `cache` levels, one package per tick. The host limits generated model bytes per tick, and loading `museum-belltower` in one tick had exceeded that limit. A headless run with 74 Page Down presses ended on `train-cliff` with exit code 0. The `menu-temple` capture starts at the native camera with 30,773 distinct colours, against 1,731 before sphere placement.

Normals: in `menu-temple` 35 of 37 fragments store kind-0 byte normals. Decoded as `q / 127.5 − 1`, they have unit length (10th–90th percentile 1.00), and on fragment 0 all 224 triangles agree with the vertex-normal orientation. `background_census` still decodes every package. With the normals exported, the headless `menu-temple` frame changed in every pixel; mean luma went from 69.3 to 69.2.

## 2026-11-24 — level lights

`LIGHT_TABLE_1` light records hold colour, position, direction, position, zero, range and intensity. On `menu-temple`, `con-1-start`, `train-cliff` and `war-1-market-coop` the direction is unit length and points downward, and the range is `f32::MAX`. Intensities are 3.99, 1.25, 0.90 and 19.05. The probe record starts with a position and RGB spherical-harmonic terms: the first term is zero on `menu-temple` and (0.245, 0.312, 0.421) on `con-1-start`. With these applied, the headless `menu-temple` frame's mean RGB moved from (73.2, 68.3, 64.5) to (52.6, 43.6, 49.6). The 74-switch run finished on `train-cliff` with exit code 0, after a table without a light record (`museum-sewer-end`) was handled.

## 2026-11-24 — spawned actors

The numbered `-ingame10` packages hold `SPAWNER_GROUP` tables: 45 spawners in `menu-temple`, 87 in `con-1-start`, 132 in `con-2-mid`. Spawners with an art group are placed from `actor27.psarc`. The headless `menu-temple` run presented 41 art groups and 814,892 triangles, and the 74-switch run ended on `train-cliff` with exit code 0. Spawner positions are used as world positions in metres; parent spawners and level offsets are not applied.

## 2026-11-24: script runtime and task start

`examples/script_run GAME_DIR ice-cave-temple 300` booted 86 DC modules from `dc1.psarc` through `startup.bin`'s `top-level`, selected task `ice-cave-temple` (node `ice-cave`, module `ss-ice-cave`) and started 188 state-script processes from 1,125 spawners of 7 packages. After 300 frames, processes had moved to other states through `go`. A few spawners named scripts that are not loaded, and some tags were missing. These were logged as errors and the processes continued. 19 natives without an implementation were called 5,459 times.

`crysirror play ~/Downloads/game-Uncharted2 --headless --no-audio --frames 2300` with an input track (Enter at frame 400, W held for 120 frames from frame 2000, Up held for 40 frames) exited with code 0 in 3.7 s. The first story task `movie-intro-cliffhanger` skipped its 84 MB `intro-cliffhanger.m2v` (over the 64 MiB world-video read limit) and advanced to `train-wreck-climb`. The world package's load region `level-load-train-wreck-1` ran its scheme lambda in the interpreter (scheme 1) and added `train-wreck-b`, `train-wreck`, `train-wreck-wide`, `sky-train-wreck` and `train-wreck-sound-const`. 39 packages loaded (670,628 triangles, 144,511 walkable). 28 processes ran. The level script printed "PLAYER DISABLE CONTROLS" and "movement type is walk-hurt+walk-hurt". The player started at `continue-player-climb-1` (parent-frame position (12.91, −19.64, −1.59)) inside the vertically hanging car `train-wreck-car-b` and moved under input. The capture showed the car interior around the player model, with 10,946 distinct colours. The materials had a blue cast, so actor base colours are not correct yet. 32 script errors came from process 0x4cb7839b dereferencing the 0 returned by the unimplemented `pad-get-info`. The most frequent unimplemented natives were `set-walk-type`, `set-run-type` and `player-force-hard-land` (2,205 calls each), and `camera-animation-enable` and `allow-camera-cut-compensate` (1,892 calls each). Window, audio, controller and visual comparison with the original were not tested.

`examples/script_run GAME_DIR --all 90` boots the modules for each of the 494 task subnodes in task order. It starts the state-script processes of each subnode's `-ingame` spawner packages and runs 90 frames. Two runtime faults appeared: track entries without a lambda, and processes whose states change on every `go`. After processing these per track and deferring repeated state changes to the next frame, all 494 subnodes run with 26,005 processes, no interpreter faults and no unknown opcodes. 184 distinct unimplemented natives were called. The most frequent were `object-in-region?` (260,179 calls), `stat-value?`, `player-in-region?`, `object-on-object?` and `get-dda`.

`examples/probe_stats` on the installed packages: `ice-cave-temple` 635 probes over 127,338 vertices of single-placed meshes, irradiance luminance p5/p50/p95 0.030/0.126/0.257; `train-wreck-b` 320 probes, 0.018/0.099/0.112; `ice-cave-chamber` 0.066/0.170/0.557; at most 0.1% of vertices received zero. With texture roles from the `VRAM_DESC` names, `examples/base_colors` reports no material whose base colour is the blue placeholder; the remaining material without a colour map in `ice-cave-chamber` is a water normal-map material. The ice-cave task run (`go-ice` input track, 2,700 frames) exited with code 0 in 1 min 51 s.

Boot: `crysirror play … --headless --frames 3200` with Enter every 300 frames from frame 200 exited with code 0 in 6.3 s. The task log ran `introduction`, `ndi` (`sce-ndi-logos.m2v`, 12.2 s), `kiosk-trailer` (left by its start lambda), `autosave-warning` (dialog dismissed), `menu` (level `menu`), New Game, `movie-intro-cliffhanger` and `train-wreck-climb` with 28 processes. Font rendering of the dialog and menu text uses the host default font; the native `.fnt` fonts are not decoded.

## 2026-11-24 — mesh unit, sky environment

`examples/instance_spheres` on `train-wreck-wide`: with millimetres × 2^(N − 12) the bridge (15-bit grid) has a half-length of 61.1 m against a 62.5 m culling sphere, the river 772 against 770, the treeline 43 against 47 and the mountain backdrop 7,686 against 7,734 m. `examples/collision_fit` on `train-wreck-b` (1,224 Havok vertices around the box car): median distance from placed render vertices to the nearest collision vertex 0.86 m at 6.4 cm per unit and 1.40 m at 1 cm. The `wrk-box-car-02` art group and the background box car roof have half-diagonals of 190 and 199 raw units. Headless free-camera captures of `train-wreck-climb` (F1 then held movement keys; captures outside the repository) showed the `train-wreck-b` chunks forming one cliff with the train hanging from its top, where centimetres had left separate pillars. The mountain backdrop is still misplaced (see SATD).

The `sky-train-wreck` dome was drawn as a 120 m mesh around the origin, which shadowed the sun and hid all geometry past it. Its fragment program (shader 0, disassembled from the material table) maps the view direction to `u = atan2(z, x) / 2π`, `v = acos(y) · 2/π` on the 4096 × 2048 sky image. The player now renders that image as a 2048 × 1024 environment through `upload_hdr`. `cargo test --release -p crysirror-asset --lib sky` covers the mapping. The boot run (Enter every 300 frames, 3,400 frames) exited with code 0 into `train-wreck-climb`. The scene remains darker than the original, and the player model's base colour is wrong. Window, audio and controller use were not tested.

Texture coordinates: `examples/uv_stats` showed 14-bit UV grids (levels, crate, oil drum) spanning about ±1 texture at 1/256, while the 10-bit grids of `hero-dirty-beatup` and `train-wreck-car-b` spanned ±8 and ±480. The unit is now 2^(M − 22) (1/256 at M = 14), with the per-material symmetric ranges centred on the texture by +0.5. A follow-camera capture of `train-wreck-climb` showed the player's shirt, holster and arms with their colour atlases, where the model had shown repeated stripes. `background_census` still decodes all 334 level and 1,164 actor packages.

Lighting model: the `trnwrk-cliffwall-snow` main pass (shader 57) is a light pre-pass material. Its fragment program adds the shadowed sun (`saturate(N·L)` × sun colour) to an ambient term from interpolant TEX5, and its vertex program (scratch NV40 vertex disassembler) writes TEX5 = attribute `0x4c` × constant `c[467]`. Attribute `0x4c` has an all-zero stream in the package and is filled at runtime. The player's per-vertex probe irradiance takes its place, with an uncalibrated scale. The environment now uses `Tonemap::Filmic` (Hable's published Uncharted 2 curve) with auto exposure at +0.5 EV, where the scene had used −1.5 EV with PBR Neutral. Captures show the cliff rock grey and the snow ledges white under the overcast sky, where both had been near black. Below the horizon the sky environment is the bottom row's mean colour.

Placement from fragment spheres: `examples/fragment_spheres` reads instance +0x14 (one world sphere per fragment). On `train-wreck-b`, a tree's spheres sit 4 m above its matrix translation and the box car roof's 4 m off, so decoded positions lack the pivot. Scaling the unit until the vertices of the tight props fit their spheres gave a maximum of 0.977–0.995 × millimetres·2^(N − 12). The unit is now 2^(N − 22), the UV rule. With the fitted origins `examples/column` finds the `train-wreck-wide` valley floor at −127 m beside the river at −128 m, where sphere-centre placement had put it at +403 m. All 576 rays of the `menu-temple` start camera still hit geometry (median 12.2 m), and `background_census` decodes all packages. Free-camera captures of `train-wreck-climb` with a 20 km far plane showed the passenger car hanging on the cliff face below the bridge and the train, with the river valley and mountain ranges around.

Main menu: with Enter at frames 200, 500, 1000 and 1040 the headless run showed `menu-temple` through the menu camera with the title logo and "Press the START button", then the New Game / Multiplayer / Options page and the Easy / Normal / Hard page with Normal selected. The boot track (Enter every 300 frames) ran `introduction`, `menu` (level `menu-temple`, menu camera), `movie-intro-cliffhanger` and `train-wreck-climb` and exited with code 0. Fonts are the host default, not the native `.fnt`.

Exact dequantization: with `a · (1 + v · 2⁻²¹) + b` (kind 4) and `a · (1 + (4v + I) · 2⁻²³) + b` (kind 6), `examples/extremes` gives the crate y 0.004…0.999 m and x −0.502…0.500 m, and the hero y −0.005…1.904 m with fingertips at (±0.684, 0.947). `examples/column` finds the `train-wreck-wide` valley floor at −125.4 m below the river's −124.6 m. `background_census` decodes all 334 level and 1,164 actor packages, and the skin decoder consumes every skinned fragment's streams exactly. Headless captures of the menu and the prologue matched the earlier fitted results.

Skeleton and clips: `examples/clip_pose proto.pak proto.pak idle` reads the 230-joint player rig (hash `0xef94465d`) and its one-frame `idle` clip (51 joints with channels, no unsupported commands); the posed ankles sit at 0.12 m and the hands at hip height (0.91 m). The 48-bit constant quaternions swap the reconstructed component into the index slot (1,719 of 2,072 non-w-largest samples match the default pose, 193 when inserted). A headless follow-camera capture of `train-wreck-climb` shows the player standing in the idle pose with the frosted clothes skinned to the rig, where it had stood in its bind pose. `background_census` still decodes all packages.
