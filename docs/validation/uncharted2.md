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

Player state machine (2026-11-25): `examples/anim_states anim-player.bin idle run idle-run-f run-idle` lists 988 states with the native transitions (`run` → `run-idle` 0.2 s, `idle-run-f` → `run` on `auto` 0.2 s, `run-idle` → `idle` on `auto` 2 s). `FRAMES=1 examples/clip_pose proto.pak proto.pak walk-fwd` gives an alternating looping gait (ankles ±0.45 m, swing lift 0.26 m). In `train-wreck-climb` with W held the log shows `player animation idle-hurt` then `walk-hurt`, following the scripts' `walk-hurt` movement type; the capture shows the player inside the hanging passenger car among its seats and luggage instead of on the valley floor 80 m below.

Log-space rotations (2026-11-25): with the `CalcInfluences` formulas, `FRAMES=1 examples/clip_pose proto.pak proto.pak walk` reports no unsupported opcodes; the left wrist stays at 0.85–0.87 m height and 0.26–0.29 m from the body axis over the cycle (the constant `idle` pose has 0.91 m and 0.26 m), the `l_shoulder` rotation equals `idle`'s to within 0.04 per component up to sign, and the knee flexes about 23° at frame 0. Before the change the wrist stayed 0.56 m out at 0.97 m (arms raised). In `museum-steam-vault`, with W held from frame 1000 and Shift from frame 1100, the log shows `player animation walk` then `move`; the frame-1060 capture shows the walk with arms at the sides. The frame-1160 capture shows the follow camera inside the player model and the railing during the run.

Blend trees and controller (2026-11-25):

- **Offline evaluation:** `examples/anim_tree anim-player.bin` evaluates all 988 states without errors. `anim_tree anim-player.bin run no-weapon idle run run@28=0.5` gives `idle` plus `idle-base-big-add` (additive), `move`, and `move` 0.5 / `move-lean-r` 0.5. With `walk-hurt` the same states give `idle-hurt` and `walk-hurt-long`.
- **Museum run:** in `museum-steam-vault` with W, Shift, release, Space, Shift+W and Space, the log shows the states `ambient-casual`, `idle-run-f`, `run`, `run-idle`, `idle`, `jump-in-place`, `jump-in-place-land`, `idle-run-f`, `run-jump-long`, `jump-long` (0.8 m above the floor), `jump-long-land-run`, `run-idle` and `idle`. The rig loads 138 of the 206 blend-tree clips.
- **Captures:** the frame-1180 capture shows the run behind the railing and the frame-1460 capture the long-jump pose, with the camera behind the player.
- **Known defect:** stretched triangles at the shoulder and hip remain.

NPC animation (2026-11-25):

- **Offline evaluation:** `MODULE=anim-npc examples/anim_tree anim-npc.bin` evaluates all 264 states.
- **Museum binding:** in `museum-steam-vault` the log shows `ally continue-flynn-steam-vault-1`, and `flynn-heist` binds to `flynn-base` with the sets `*flynn-heist-anim-set*` and `*flynn-anim-set*`. Flynn's layers are `idle` plus the additive `look-add--left-right` and `look-add--up-down` at their centre phase.
- **Capture:** the frame-940 capture shows Flynn standing on the lower walkway with his arms at his sides, and the player in `ambient-casual` after ten seconds at rest, framed from the continue camera's direction.

Helper groups (2026-11-25):

- **Offline measure:** `examples/skin_stretch hero-heist.pak proto.pak proto.pak walk` counts the triangles stretched more than 4× in the frame-0 pose. Before the helper groups followed their limbs it reported 33 of 28,055, with the elbow, shoulder-twist and knee helpers at the top. Afterwards it reports 20, made up of crotch triangles between the thighs and small lip triangles.
- **Capture:** the frame-1180 museum capture shows no spikes at the arms or hips.

Script animation (2026-11-25):

- **Stub run:** `examples/script_run GAME_DIR --all 300` with the stub `World` reporting `animate` lists 7,776 calls. The first is in `train-wreck-weather`, which animates `train-wreck-car-a-1`, `train-wreck-car-b-1`, `train-wreck-car-b-undercarriage-1` and `shift-01-ladder-1`.
- **Prologue run:** booting through the intro into `train-wreck-climb` logs `animate` for `idle-00-car-a`/`-car-b`/`-undercarriage`/`-ladder` (10.07 s), `drake-platform-drop-door` (1.20 s) and `broken` (0 s). Before the data-base fix, three of these failed with "ND clip read" and two decoded to no channels.
- **Offline decode:** `clip_pose train-wreck-car-b.pak anim-train-wreck-climb.pak idle-00-car-b` decodes 151 frames on 3 joints, and the `pistol-base` `idle` decodes on 9 joints.
- **Capture:** the prologue capture shows the car, the undercarriage and the ladder posed on the cliff face.

Root motion and rig libraries (2026-11-25):

- **Offline decode:** `clip_pose` reports `walk` 1.90 m/s, `move` 4.00 m/s, `move^turn-180^move` ending at quaternion (0, 1, 0, 0) and `jump-long` ending at (−0.005, −3.25, 6.98).
- **Prologue capture:** after the change the hanging car stands over the cliff with the player inside it.
- **Bench-climb track:** boot, then two Page Down presses (`train-wreck-climb` → `load-tw-a-high` → `train-wreck-treadmill`). Before the shared libraries this aborted with "asset cache exceeds 256 MiB" (255 MiB generated). Afterwards the run completes and logs `rig library Player: 138 clips, 15631 KiB` once, plus the treadmill animations `idle-04-car-a`/`-car-b`, `shift-03-door`, `drake-tight-motor` and `drake-tight-beam`. The museum run still shows the player states and Flynn's idle.

Cameras and prop collision (2026-11-25):

- **Prologue run:** the log shows `camera ada068bf` (`cam-railing-center-intro-1`, whose spawner exists) followed by `camera release None`.
- **Prologue capture:** the final capture still shows the camera against the player inside the hanging car.
- **Museum and treadmill:** the museum run is unchanged, and the bench and treadmill runs complete with exit code 0.

Collision, edges and hanging (2026-11-25):

- **Offline collision:** `examples/collision_census train-wreck-car-b.pak wrk-passenger-car-02.pak train-wreck-b.pak train-wreck-b-ingame10.pak` reads car-b as 372 hull + 2,325 mesh triangles with 438 edges and 132 corners, the passenger car as 16 objects, and the `train-wreck-b` level as 1,236 triangles from (13.64, −64.09, −96.97), equal to its MOPP offset, with 35 edges in its `-ingame10` feature database.
- **Material programs:** `examples/material_samplers train-wreck-b.pak` reports slope layering for `snow-mod-cliff-cheap` (side 1, top sampler 2), `cliffwall-snow`, `cliffwall-snow-cheap` and `snow-boulder-med` (computed top) and `tree-bark-cheap` (side 1, top 2); `examples/normal_check` gives a mean cosine of 0.857 between decoded vertex normals and triangle normals. `cargo test -p crysirror-naughty-dog` covers the fragment-program decode and recognizer and the hull triangulation.
- **Prologue run:** the log shows `player hangs from edge 7 of Some(400)` (`trn-wrk-rail`). The follow camera stays 4 m out for the whole run, where every direction was blocked within 0.8 m before. The frame-3600 capture shows the player hanging from the car's end rail over the cliff.
- **Input tracks:** holding D logs `ledge-shimmy-right` and then `ledge-grab`, moving the root 0.3 m along the rail. Holding W logs `ledge-climb-up`, `ledge-climb-up-2`, `climb-onto-1-75m^idle` and `idle`, and the capture shows the player over the rail on the car's end platform.
- **Camera animation:** the log shows `camera animation 27f169e6` and `camera clip camera-train-wreck-exterior-small: 1201 frames`.
- **Snow capture:** a free-camera capture shows snow on cliff tops and rock on the walls, where before the rock was bare and the noblend snow ground was white. The light-blue flat region behind the cliffs (the wide backdrop) is unchanged.
- **Height-mask layers:** `examples/material_samplers` on `train-wreck-wide`, `ice-cave-temple` and `menu-temple` reports `Mask { sampler: 2, softness: 0.1 }` blends (rock → unit 2, mask → unit 0, snow → unit 3 in `rock-to-snow`). The free-camera backdrop capture shows snow on the mountain faces, where before the rock was bare.
- **Edge jumps:** from the prologue hang, D with Space logs `ledge-jump-begin`, `-mid`, `-end` and `ledge-grab` 1.1 m to the right along the rail; Space alone finds no edge above; C logs `fall` without an immediate re-grab.

Trigger regions (2026-11-25):

- **Offline:** `examples/trigger_regions train-wreck-climb-fg-ingame10.pak continue-player car-b-seat cam-car-b-interior` reads 69 regions; every seat and interior camera spawner lies in `climb-car-1` (parent `train-wreck-car-b-1`), `continue-player-climb-1` in `rs-stick-to-move-drake-1`, and `continue-player-treadmill-1` in `rs-task-bench-climb-complete-1`, as the static analysis of the EBOOT tests predicts.
- **Prologue run:** on the climb track the log shows `region enter` for `rs-stick-to-move-drake-1`, `rs-disable-circle-1`, `rs-camera-cut-compensate-1`, `rgn-fall-to-death-treadmill-1`, `rs-kill-journal-2` and `rs-cam-railing-center-1` at the start and `region exit rs-cam-railing-center-1` after climbing onto the platform, with no script errors.

Materials (2026-11-25):

- **Captures:** after the change the prologue capture shows the car metal matte with normal-map relief and the rock faces without the white grazing-angle sheen they had before. Painting the river material red in a test build left the pale flat region seen from 25–30 m above the cliffs unchanged: it is sky beyond the authored canyon geometry, not the river (`trnwrk-wide-river-water`, 509 × 1505 m at y −125), and does not appear from the player's position.

Render settings (2026-11-25):

- **Title screen:** the main menu capture against a capture of the original title screen gives mean RGB (68, 58, 33) against (57, 60, 32) and luminance p10/p50/p90 17/44/121 against 21/43/104; before the change the capture measured (141, 135, 90) and 74/125/203. The gold statue shows its gold `color-01` layer over `colordrk` by the mask, where before it rendered white.
- **Prologue:** the camera at the hanging car lies in `regiongroup-train-wreck-climb-states-1` (`camera` category, record `#f8cd5e96`); the capture shows the blue canyon fog (0.17, 0.31, 0.52) and the earlier exposure. With fog the menu measures 23/40/93 against the original's 21/43/104.

## 2026-11-26 — Script lights and shader instance state

- **Native evidence.** BCAS20097's `sub_7B998` converts the fourth argument
  of `add-dynamic-light` from degrees to radians. The fifth controls enablement.
  `sub_7AD88` writes the optional colour gain at +0xb8.
  `sub_7AE48` updates an existing named light, resets its colour to white
  and detaches its joint. See the [native ABI](../../crates/crysirror-naughty-dog/NATIVE_ABI.md).
- **Release tests.** `cargo test --offline --locked --release -p crysirror-naughty-dog --features runtime --lib`
  passed 26 tests. New fixtures run the script native dispatcher and apply the
  emitted commands to CrySirror's scene. They check a persistent scene handle
  across edits, light shape and gain, distinct names with equal low bits,
  joint movement and removal, 32-record pool reuse, and both shader parameter
  setters' layout in object state.
- **Desktop build.** `cargo build --offline --locked --release -p crysirror-desktop --bin crysirror`
  linked the arm64 executable at `target/release/crysirror`.
- **Coverage.** The installed game was examined statically for this change.
  No gameplay replay was run. Joint tests cover object transforms composed
  with bind joints. Shader tests cover state writes. Animated skeletal joints
  and shader parameter effects on the rendered materials have no coverage here.

## Installed-disc soundtrack

2026-11-26, BCAS20097 01.00, Apple M4 Max. The installed-game launcher
mounts the original music, streams and English speech archives. XVAG codec 6
uses the shared audio decoder on the mixer’s preparation and playback paths.
Each decoder holds its ADPCM predictor history, one 28-frame stereo row and
a 64 KiB encoded buffer. Logo and story mixes use different channel orders.

The release player took ordinary menu input from boot to New Game.
The offline mixer produced nonzero samples for `sce-ndi-logos.xvag`,
`81_UNCH2_NatesTheme.xvag` and `intro-cliffhanger.xvag`. Skipping the logo
cleared its voice before menu playback. Starting the story cleared the menu
voice, and skipping the story film cleared its voice at `train-wreck-climb`.
An 8,200-frame menu run finished Nate’s Theme and played `64_TrainRun.xvag`.
All five final headless runs reported zero mixer underrun frames. The adapter
library passed 51 tests, the XVAG group passed three, and the explicitly
invoked installed-disc regression passed.
A separate unmuted 180-frame window run exited successfully on Metal.
Physical speaker output and frame-exact synchronization against original
hardware were not compared. This pass does not verify the adaptive music
director or every Scream grain and mixer snapshot.

The installed-data regression opened all 47 matching film companions and all
446 music entries through their PSARC readers. Each produced nonzero stereo
PCM within its first 20 seconds. The probe stops at the first signal and does
not assert full-length playback. The music corpus includes 210 entries with
XVAG flags 0x03, seven stereo files with right before left, and a 96 MiB
entry. The launch profile allows 128 MiB logical entries with block reads.

To run the installed-data regression with the read-only disc root:

```sh
UNCHARTED2_ROOT="$HOME/Downloads/game-Uncharted2" cargo test --release --offline --locked \
    -p crysirror-naughty-dog --example level_resources installed_movie_and_music_audio -- --ignored
```


## 2026-11-27 — Opening train traversal and player action sounds

BCAS20097 01.00 on Apple M4 Max. The release player took ordinary menu and
game inputs from boot through the damaged train. The input track is
`crates/crysirror-naughty-dog/tests/data/train-wreck-climb.json`. Replay with
an empty scratch save directory:

```sh
./target/release/crysirror play "$HOME/Downloads/game-Uncharted2" \
  --headless --no-audio --frames 5530 --data-dir "$(mktemp -d)" \
  --input-track crates/crysirror-naughty-dog/tests/data/train-wreck-climb.json
```

The player took the left outside corner, entered `rs-shift-00` and rode the
scripted carriage movement. Alternating handholds and a lateral adjustment
reached `rs-hint-jump-1`. The script displayed `HINT_X_JUMP` at 84.6 seconds.
The jump left the region and the last grip was edge 95, with no script errors.
This route crosses the hint using controller input and the original script.

The local disc fixture `installed_wall_rope_controls_play_native_clips` drives
the production player controller with the original state table and decoded
actor clips. It checks both transfer directions on each side, continuous root
placement through the two clips, the apex turn and its auto return, a mirrored
release and catch, and the final hanging state. The geometry fixture covers
parallel and reversed edge normals and verifies the ballistic endpoint. These
fixtures start on a rope or an edge and do not traverse a retail level.

The separate `crates/crysirror-naughty-dog/tests/data/dig-wall-rope.json`
track starts at the retail task `dig-dropdown-post-pit-fight` with the
complete install mounted. The checkpoint places the player at
(−9.54, −0.53, −132.45). Walking and jumping reaches
`dig-pit-fight-rope-1` at (−10.58, 3.95, −134.22), whose authored
length is 4 m. A release trace with 470 frames at 16 ms recorded the
bottom grab, settle, swing, mirrored `wall-vine-run-r-turn-180`, return
to the other side, and `wall-vine-run-right^jump-right-miss` after
the jump input at frame 450. The player took off at (−10.48, 0.33,
−134.50). In a second trace, holding the climb input reached 1.75 m
root height. Four high releases in this approach missed. A targeted jump
to another ledge therefore has controller and geometry fixture coverage
only.

The train replay reported 129 player animation sound requests. Its last
request was 0.40 seconds into the ending clip of a long ledge jump. The
installed-data mixer test checked these original cues and measured peak
absolute stereo PCM output:

| Action clip | Original 30 Hz event frame | Sound | Peak |
| --- | ---: | --- | ---: |
| Hurt run | 1 | drake-heel-metal-sheet | 0.37735 |
| Hard landing to idle | 0.5 | drake-impact-foot | 0.17719 |
| Soft landing to idle | 1.5 | drake-heel-metal-sheet | 0.37735 |
| Two-hand ledge contact | 7 | drake-grab-metal-sheet | 0.46749 |
| One-hand ledge impact | 4 | imp-body-fall | 0.07943 |

Before the host cache fix, the full dig checkpoint failed with 255 MiB of
encoded generated resources and zero cached file-read bytes. After the fix
the same checkpoint loaded from the full install and reported 430,682,548
bytes of generated model backing. The model backing and prepared cache have
separate residency counters. These measurements refer to CPU bytes.

## 2026-11-27 — Retail title actors

BCAS20097 01.00 `menu-temple` has 45 spawners. Twenty-three are SimpleActors, consisting of 14 pigeons, eight cloth banners and one vine. The other 22 are the camera and 21 PartSpawners. The default task sequence and ordinary Enter input at frames 200, 500 and 800 reached the New Game menu. Apple M4 Max Metal rendered the release build headlessly at 1280 × 720 with audio muted.

| Frame | Rendered and runtime observations |
| --- | --- |
| 1150 | All 14 spawners played distinct clips on the 19-joint pigeon rig. Two pigeons crossed the statue at projected head positions around (347, 229) and (346, 491). All eight 117-joint banners and the 26-joint vine had their native `idle` layers. Before the fix all 23 entities had zero animation layers. |
| 1200 | A pigeon crossed the banner with its wings raised. The banner silhouette and vine pose advanced. |
| 2848 | The second flight pass reached 10.2733 s, within 0.002 s of the 10.2720 s sampled at frame 1150. Both captures retained all 14 scripted clips and nine prop idles. The intermediate one-shot implementation stalled at the end of the first pass. |

All three captures reported task `menu`, 14 state-script processes and zero script errors. The installed disc supplies the skeletons, geometry and clips. This run verifies CrySirror's rendering and timing on Metal. Direct pixel comparison against original PS3 video remains unverified. The captures, logs and input track are local under `target/uncharted2-validation/title-1127/actors/`.

### Temple lighting and title captures

On BCAS20097 01.00 all 37 placed temple fragments carry vertex lighting.
Their 12,383 colours reach the shared baked-irradiance material, including
the two fragments with kind 6 quantization. The level sets the live
baked-vertex multiplier to 6.99962. The Metal world snapshot identifies a
generated probe-radiance HDR and keeps sky display off.

On the M4 Max the ordinary launcher entered the title from an empty save
directory, with Enter at frames 200 and 500. Before and after images use the
same 1280 × 720 headless frame 2050. On the 640 × 360 previews the statue
cheek rectangle (30,162)–(139,261) changed from mean display RGB (51,46,43)
to (74,54,29). The raised hand rectangle (224,151)–(254,193) changed from
(68,61,53) to (137,122,98). These are measurements of the port's two
captures. The supplied 640 × 426 reference is cropped and has no menu UI or
frame number. The broad upper shafts remain brighter in the port.

The new build also replayed the full 5,530-frame train input track without
a script error or an audio underrun. Captures, input and world snapshots
are in the ignored local directory `target/uncharted2-validation/title-1127/`
(`before.png`, `after.png`, `actors/menu-1200.png`, `train.png`).

### Temple particle coverage

On the M4 Max, a controlled 1280 × 720 Metal capture used the disc's 24
temple shafts, their original placements, camera and textures, against a
shared procedural sky. Both renders used frame 30 and fixed exposure;
changing only the blend from direct additive colour to accumulation and
a single coverage composite changed mean display RGB from (108,123,149)
to (74,80,89). The luma difference reached 8/255 on 680,050 of 921,600
pixels. Both runs drew 24 of 24 particles with zero pending textures.
The 512 × 512 mask retains the green channel at its native dimensions.
The three installed-particle tests passed, including the descriptor
pass nibble, material blend factors and scroll parameters.
The five release VFX renderer tests passed on Metal. The overlap test
checks the pixel from two colours over a blue background after the combined
coverage reduces the blue channel to 127/255.
These measurements isolate the compositor. The original temple background and
the PS3 adaptive low-resolution buffers are outside that comparison.
In the normal title capture, enabling the experimental layer on all 24
shafts produced dark rectangular borders. The 1280 × 720 intermediate used
7,372,800 bytes; the real scene drew 35 batches, one composite, and 755
visible particles. The adapter keeps the independent additive colour path
until the source of the material's low-resolution alpha is verified.
After restoring that path the final release entered the menu normally at
frame 950, drawing 755 particles in 35 batches, with zero pending textures
and zero bytes allocated to the experimental target. The rectangular
borders were absent in `/private/tmp/u2-rope-fixture-1127/title-final-default.png`.
The locally generated A/B images and reports are in
`/private/tmp/u2-rope-fixture-1127/shaft-isolated/` as
`additive_color` and `additive_coverage_layer`.

### Root menu colour and metallic highlights

The owner's 2406 × 1348 YouTube capture `222.png` includes the menu UI.
The area record `*menu-temple-render-settings-01*` has a neutral global
tint (1, 1, 1), yellow-green world fog (2.6, 2.75, 1.5) and a warm-white
sun (1.24, 1.11, 0.92) with intensity 6.2083. Its shadow distance is 20 m.
The direction and colour now come from the selected area record. Previously
the player always used the orange light in the level package.
The menu overlays its own blue-green edge board below the lettering.
The installed-data regression checks both the real root-page flags and
the decoded edge pixels. Original textures and fonts stay on the disc.

During full-title validation the experimental RGBA particle layer introduced
black polygons behind the logo. At 640 × 360 the background patch
(335,212)–(355,234) fell to RGB (6,5,3), compared with (112,116,71) in
the video capture. The title uses its direct additive path pending evidence
for the original low-resolution target's colour and alpha writes. The
standalone compositor's pixel tests exercise its blend equation separately.

The final desktop release re-entered the regular title on M4 Max / Metal.
`present.png` is the 1280 × 720 output at frame 2050. At 640 × 360 the
chin patch (14,235)–(69,271) averages (29,31,23), against (26,31,20)
in `222.png` and (24,15,5) before the menu board. The forehead patch
(70,136)–(130,169) averages (116,103,79) against the reference's
(161,153,113). The central background patch recovered to (79,70,37).
The top shafts remain too bright, the background too brown and the
forehead too dark. These are comparisons with the supplied video frame.
The final build drew 35 particle batches, loaded 14 pigeons and finished
the 5,530-frame train regression with 129 motion sound requests and no
script errors. All 66 adapter library tests ran and passed with the disc
mounted, including the gold material, menu board and font regressions.



## 2026-11-27 — Title gold and layer masks

On Apple M4 Max / Metal, six separate 96 × 96 renders exercised the shared
`layered_blinn` material with a black cube, full red or full green mask
textures, a fixed front camera, and bloom and screen-space AO disabled.
The center red byte for disabled, half-weight base and full-weight upper
masks was 0 / 181 / 240 under the directional light alone, and
0 / 97 / 144 under baked light alone. Both channels of illumination use the
green specular masks and respond to the independent layer bindings.
The explicitly executed Metal test
`layered_blinn_lobes_and_green_specular_masks_reach_pixels` passed.
`installed_temple_uses_placed_lighting_in_render_primitives` opened the
local BCAS20097 installation and checked all 13 generated materials. Six
gold materials use scale 1.33, the statue's accessory uses 1.32 and all
seven use exponent 194.5. It also checked the gold mask/image bindings,
two-channel normal encoding and placed vertex-light stream.

The normal title input track on the desktop release reached frame 1150
from fresh save directories on Metal. In the 640 × 360 preview the left
face crop (7,125)–(134,234) moved from mean display luma 62.3 to 71.2
and P90 from 121 to 133. The supplied title reference gives 87.1 and 170
over the same rectangle, with a slightly different composition. The
forehead still lacks the broad highlight in the reference. Both port
captures include the other scene changes made between those releases.
Local evidence is in `target/uncharted2-validation/title-1127/gold/`
(`before.png`, `final.png`, `gold-detail.png`).
