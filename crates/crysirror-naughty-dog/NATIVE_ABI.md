# Naughty Dog script ABI (BCAS20097 01.00)

Findings from the owner-supplied decrypted `EBOOT.elf` (not distributed) and the installed `dc1.psarc`. Addresses are PPU virtual addresses of that ELF. String references go through per-module constant pools that a TOC slot points to (for example the slot at 0xe0ff78 for the script VM module).

## DC modules (`dc1.psarc/*.bin`)

| Offset | Meaning |
| --- | --- |
| 0x00 | `"DC00"` |
| 0x04 | version 1 |
| 0x08 | payload size; the fix-up table follows it |
| 0x10 | pointer to the root value |
| size | fix-up count, then payload offsets of pointer words |

Every value is preceded by its type `StringId` (CRC-32, polynomial 0x04C11DB7, MSB first, initial 0). The root is a `script-file` `{1, count, [(name, value)]}`. A pointer may equal the payload size (an empty trailing array). `startup.bin`'s `top-level` lambda calls `add-import-module` (0xc66a1ce6) and `add-load-module` with `'script`; 86 modules load in that order. The reference SID database at icemesh `3e3025c` maps 0xc66a1ce6 to an unrelated multiplayer name.

## Script lambdas

`script-lambda` is `{code, statics, 'function}`. The interpreter is `sub_1594E0` ("Eval: Unknown opcode"); `sub_158288` ("RunScript") is a second entry and `sub_154F00` the tracing disassembler whose format strings name the opcodes. An instruction is a big-endian word `[op][a][b][c]`; registers are 32-bit.

| Op | Name | Semantics (`r` = registers, `s` = statics) |
| --- | --- | --- |
| 0x00 | Return | return `r[b]` to the caller's destination register |
| 0x01–0x04 | IAdd ISub IMul IDiv | `r[a] = r[b] op r[c]` (signed) |
| 0x05–0x08 | FAdd FSub FMul FDiv | float |
| 0x09–0x0b | LoadStaticInt/Float/Pointer | `r[a] = s[r[b]]` |
| 0x0c | LoadU16Imm | `r[a] = c << 8 \| b` |
| 0x0d–0x0f | LoadU32/Float/Pointer | `r[a] = *r[b]` |
| 0x10–0x12 | Store… | `*r[b] = r[c]; r[a] = r[c]` |
| 0x13–0x15 | Lookup… | `r[a]` = value of symbol `s[b]`; unknown: error and a null-symbol value |
| 0x16–0x18 | Move… | `r[a] = r[b]` |
| 0x19 / 0x1a | CastInteger / CastFloat | convert `r[a]` in place |
| 0x1b | CallFf | DC lambda `r[b]`, `c` arguments from r24…, result to `r[a]` |
| 0x1c | Call | native `r[b]` as `fn(result*, c, args[16] = r24…r39)` |
| 0x1d | IEqual | `r[a] = 1` or 0 |
| 0x1e–0x21 | IGT IGTE ILT ILTE | −1 or 0 |
| 0x22 | FEqual | 1.0f or 0.0f (`fsel` on −\|b−c\|) |
| 0x23–0x26 | FGT FGTE FLT FLTE | −1 or 0 |
| 0x27–0x2a | IMod FMod IAbs FAbs | |
| 0x2d / 0x2e / 0x2f | Branch / BranchIf / BranchIfNot | target `c << 8 \| a`, condition `r[b]` |
| 0x30–0x37 | logic/bitwise NOT AND NOT OR XOR NOR, logic AND OR | logic results −1 or 0 |
| 0x38 / 0x39 | INeg / FNeg | |
| 0x3a | LoadParamCnt | caller argument count |
| 0x3b–0x3e | IAddImm ISubImm IMulImm IDivImm | immediate `c`; IDivImm divides the sign-extended value as unsigned 64-bit |
| 0x3f–0x41 | LoadStatic…Imm | `r[a] = s[b]` |

Other opcodes report "Unknown opcode". After a native call the interpreter reads the status word before the argument block: 1 suspends the lambda, keeping the frame chain, and resumes at the next instruction; 2 abandons it. `sub_153618` sets 1 (waits) and `sub_15362C` sets 2 (`go`). A census of the 177 modules finds 52,401 lambdas and 582 distinct natives called through `Call`.

## Natives

`src/native_names.rs` lists 833 natives: the 782 records `{opd, object, "DcName", "script-name"}` of the ELF with their entry points, plus names that only appear as `Call` targets. Implemented semantics in `src/ss.rs` follow these entry points:

| Native | PPU | Behaviour |
| --- | --- | --- |
| `go` | 0x7d4884 | posts event `go` (0xc2c05f3a) with the state to the caller's process; status 2 |
| `wait` / `wait-frames` | 0x7d6b34 / 0x7d6914 | suspend for seconds / frames × 1/30 s when positive |
| `wait-one-frame` / `wait-for` / `signal` | 0x7d5f90 / 0x7d617c / 0x7d6d38 | |
| `set-/get-int32` etc. | 0x7d7b5c / 0x7d83a0 | `(var, value, [object])` on the process variables |
| `tag-symbol` etc. | 0x7d5664 | the caller's spawner property |
| `send-event` | 0x83da8 | `(event, target, args…)` |
| `spawn-script` | 0x7d4260 | start a process |
| `spawn-entity` / `kill-entity` / `object-spawned?` | 0x150674 / 0x140484 / 0x14032c | |
| `get-spawner-locator` | 0x14bf58 | heap locator |
| `task-find`, `task-subnode-active?`, `task-subnode-complete`, `task-subnode-play` | 0x143358, 0x140efc, 0x13f5e4, 0x141e50 | task state below |
| `fade-to-black` / `fade-to-white` | 0x13d3f4 / 0x13d3ac | `(alpha, seconds)` screen fade |
| `set-black-fade` / `set-white-fade` | 0x13d344 / 0x14aee4 | `(seconds, on)` |
| `player-disable-controls` | — | a control mask; `player-controls-all`/`-none` (`script-funcs.bin`) return 0xffff / 0 |
| `symbol-eq?` / `lerp-scale` / `random-float` | 0x2dbf60 / 0x1416cc / 0x141654 | equality; `clamp((x − a)/(b − a))` mapped onto `c…d`; uniform [0, 1) |
| `get-locator-distance` | 0x14f7b0 | distance between two locator records |
| `set-visible` | 0x13ae98 | `(object, on)`, any non-zero shows |
| `object-on-object?` | 0x143c54 | `(object, support)`: the object stands on the support's rigid body (here the player's platform or grip body) |
| `get-object-id` / `get-object-state` | 0x144838 / 0x1429e8 | the process id word (+76; here the name) / its state |
| `npc-alive?` / `npc-killed?` / `npc-die` | 0x22e1b4 / 0x22a900 / 0x22b8b8 | the NPC manager's process for the name (or the caller): not dead / dead, else the spawner's killed flag (flags 0x2000_0000) / kill. NPC spawners are `Soldier` processes and characters the NPC state machine drives |
| `npc-alive-count` / `npc-max-count` | 0x221708 / 0x221070 | the manager's live count / the constant 15 |
| `npc-group-alive-count` / `-dead-count` / `-find-first-alive` | — | the same tests over an entity group's members (inferred from the names) |
| `part-module-loaded?` | 0x1405a0 | the particle module is in the loaded-module table (`sub_765C50`) |
| `add-dynamic-light` | 0x14389c | `(name, locator, radius, [cone-degrees], [enabled])`. Zero selects a point light. The nonzero angle selects a spotlight. By default the light is enabled. |
| `add-dynamic-light-joint` | 0x14a778 | `(name, object, joint, radius, [cone-degrees], [enabled])`. The native record carries a generational object handle and joint index. |
| `update-dynamic-light` | 0x143778 | `(name, locator, radius, [cone-degrees])`. Edits an existing record, detaches its joint and restores white, preserving the light type and gain. |
| `set-dynamic-light-color` / `remove-dynamic-light` | 0x143678 / 0x13ceec | `(name, r, g, b, [gain])` with default gain 1. The colour call returns existence. Removal returns zero. |
| `set-shader-instance-param` / `set-single-shader-instance-param` | 0x145bb8 / 0x13dbd8 | Write a vector or one component in the drawable's four vectors of `g_optionalInstanceParameters`. CrySirror retains script writes in object state. The translated materials do not consume the block yet. |

`camera-fade-out` (0x13b210) fades a named camera object, not the screen.

The script light pool contains 32 records (`sub_7A88C`). Under pressure,
`sub_7AFB8` searches for a light behind the camera before comparing distance.
`sub_7B998` converts the optional cone from degrees to radians, and
`sub_133568` uses the half-angle for the cone. CrySirror maps the record to
shared point and spot lights, with an inner cone of 90% of the outer and
exponent falloff. Joint lights currently follow the object's frame and
its bind joint. They need the evaluated joint pose for bone animation.

## State scripts

`state-script {name, ?, initial state, ?, options, state count, states}`; state `{name, block count, blocks}`; block `{kind, event, 0, track group}` with kinds 0 begin, 1 end, 2 event, 3 update (strings of `sub_7DEB70`); track group `{count, tracks}`; track `{name, lambda count, [(label, lambda)]}`. Spawners name their script with the string property `state-script` (0x45c51f9f).

## Tasks

`tasks.bin` `*task-graph*` is `{node map, node edges}` with `%start`/`%end` edges. A node is `{name, world, ss-module, subnode map, subnode edges}`; a subnode holds name, title, level, a combined name, a record at +0x1c whose word 0 is 3 when complete (`task-subnode-complete?` in `task-defines.bin`), and a `continue` record naming the player and companion continue spawners. A subnode's +0x38 is a start lambda and +0x40 a task-manager record `{?, type, flag, params, name string, name}` with types `TaskManagerPlayMovie` (movie name), `TaskManagerMessageDialog` (text key), `TaskManagerMainMenu` and `none`. Text catalog IDs in `text1.psarc` are the `StringId` of the key.

## Main menu

`TaskManagerMainMenu`'s update (`sub_98478`) loads level `menu-temple` once through the level manager (`sub_BA9D0`) and `sub_98634` sets a `CameraControlFixed` camera; the subnode's own level `menu` is a flat board at the origin with the localized logos (`general/menu/logo2[-ita|-spa|-jpn|-kor].tga` with `-alpha` masks, stored bottom row first) and chapter/movie shots, drawn as 2D art. `menu-temple`'s `CAMERA_TABLE_1` start camera `|_cam_start_1` sits at the `menu-camera-1` spawner, whose `fov` property is 75. `dc1.psarc/menu.bin` `*menu-pages*` is `{count, pages}`: a page (0x44 bytes) has its name, title text key at +0x10, flags at +0x1c and buttons `{count, array}` at +0x20; a button (0x48 bytes) has its name, type +8, action +0xc, text key +0x14 and a condition +0x1c. `PAGE_ROOT_MAIN` (title `MENU_LEGAL_COPYRIGHT`, button `MENU_PRESS_START`) leads, without a save, to `PAGE_NEWPROFILE` (New Game, Load Game [0x13], Multiplayer, Options, Medals [0x13]) and New Game to `PAGE_NEWGAME` (Ultra-Easy [0x1d], Easy, Normal, Hard, Ultra-Hard [0x13]). Catalog text uses `\` for line breaks. The condition codes and the button-to-page dispatch live in the EBOOT and are not decoded; the player hides conditional buttons and maps page buttons by name.

## Load regions

`sub_BF890` (`game/level-mgr.cpp`, "[Load Regions] region '%s', task '%s'") reads `REGION_TREE` regions: name at +0, entry count (`i16`) at +0x1c, tag count at +0x1e, tags at +0x20, entries `{key, type, value}` at +0x24. Tag 0xf131b5e1 holds a script lambda stored in the package; its result selects scheme N (default 1). Keys `level`/`level-N` load and display, `level-no-display[-N]` fill the second list, 0x4f807b48 applies to every scheme. Each name was confirmed by its `StringId`.

## Sky

`sky-…` level packages hold one mesh (a closed dome about 120 m across on `sky-train-wreck`) whose first material binds the sky image to sampler 0 (`sky-train-wreck-color`, 4096 × 2048 DXT1) and two cloud layers with masks to samplers 1–4. The material table's shader array (+0x1c, 16-byte `{0, 0, vertex program, fragment program}`) gives shader 0 for the main pass. Its fragment program normalises the view vector (interpolant TEX2) and computes `u = atan2(z, x) / 2π` (an atan polynomial −0.0134805…0.999996 with the |z| > |x|, x < 0 and z < 0 fix-ups) and `v = acos(y) · 2/π` (the −0.0187293, 0.074261, −0.212114, 1.5707288 polynomial): a hemispherical latitude–longitude image with the zenith at the top and the horizon on the bottom row. The cloud layers use scrolled coordinates from interpolant TEX1 and mask textures; runtime constants (time, layer scales) are patched in and not read here. The player renders the sky image as the environment (`crysirror_asset::sky::ImageSky`) instead of drawing the dome, which otherwise blocks the sun's shadow map and hides geometry beyond 50 m.

## Baked lighting

`LIGHT_TABLE_1` holds a light record at +0x28, the probe count at +0x18 and the probe array at +0x30. A probe is 240 bytes: a position in metres and two sets of nine RGB spherical-harmonic coefficients (coefficient-major, RGB interleaved), then padding. The `ice-cave-temple` probes (635) lie on a 2 m grid. The player sums both sets, evaluates them as radiance with cosine convolution per vertex and draws the result through `baked_vertex_irradiance`. Which lighting each set holds and the brightness scale are not established. The `default-lightmap0/1` textures in level packages are 32 × 32 defaults, not baked maps.

Texture descriptors carry the source file name (`…/name-color.tga/….ndb`); materials are matched to their colour map by that name.

## Trigger regions

`src/trigger.rs` documents the layout. A region is tested in its parent spawner's live frame, and holds nothing while that parent is not spawned; a region without a parent is tested in level space. It is the union of its shape nodes: six-face plane sets (`max(n·p + d) < 0`), spheres and cylinders, while quads never hold a point. Flag bit 0 disables a region. The player is tested only against regions of the `player` category (the grid bucket, equal to the `apply-to` entry). `player-in-region?` and `object-in-region?` (`(object, region)`) use this test with threshold 0. Each frame the player's region set is compared with the last: an entered region runs its tag-0x1157c38c lambda and a left region its tag-0x5b4bfb24 lambda (`sub_115828`), with the package mapped into the script VM. 48 of the 69 `train-wreck-climb-fg` regions have enter scripts, including the `rs-task-…-complete` regions. The level origin is taken as zero, and region events to a region's own process (+0xc) and the object enter/exit cache (+0x12c) are not modelled.

## Vertex dequantization and skinning

Each descriptor attribute (`id`, kind << 4 | components, parameter offset, bit depths) is its own MSB-first bit stream. Kind 4: N floats `a`, N floats `b`, `value = a · (1 + v · 2⁻²¹) + b`; kind 6 adds N `u32` offsets `I`, `value = a · (1 + (4v + I) · 2⁻²³) + b`; kind 0 is signed normalised `v · 2 / (2ᵈ − 1) − 1` (normals, tangents, attribute 0x15). Positions are metres in the prototype's frame, texture coordinates are in textures. Source: the public Uncharted 3 PS3 actor decoder `bc160201983/Uncharted-The-Nathan-Drake-Collection-PC-port` (`u3mp/paktools/ps3geom.py`, commit `1294635`), checked there against the PS4 assets. On Uncharted 2 it puts the 1 m crate on the floor (y 0.004…0.999), the hero's feet at y = 0 (1.90 m tall, fingertips beside the `npc-normal-base` finger joints), the `train-wreck-b` box car roof on its Havok collision, and the `train-wreck-wide` valley floor at the river (−125.4 against −124.6 m). The earlier data fits (a 2^(N − 22) grid unit, origins fitted to the per-fragment world spheres at instance +0x14, UVs centred by +0.5, art groups moved to their `vis_sphere`) approximated these formulas and are replaced.

Fragment byte +0x20 counts descriptor rows and +0x2f all streams. Fragment +0x0c points to skin information on skinned fragments: `u16` sizes of streams s1–s4, the palette range count, s1–s4 pointers at +0xc and palette ranges `{u16 first joint, u16 count}` at +0x20. Groups of four vertices take one s1 code: 4/6 a joint list shared by the lanes from s2 (`u16` palette offsets × 48, +4 while more follow), 2/10 per-lane rows of four `u16` from s3 (lane order 1, 0, 3, 2), 0 repeats the previous group, 8 keeps its joints; 6/8/10 read four `u8` weights per joint row from s4, 2/4 weight the first joint fully. Every installed skinned fragment consumes its streams exactly.

Skeletons: `JOINT_HIERARCHY` `{8, joint count, JH block, flip data, names, 0, …, skeleton id}`; the JH block (magic `0x4a48010a`) holds the joint count at +0xc, offsets at +0x28 (per-joint `{flags, parent, first child, next sibling}`), +0x2c (default local poses, 48 bytes `scale | quat | translation`) and +0x44 (3 × 4 inverse bind matrices, 48 bytes each); names are `{StringId, string}` pairs. `npc-normal-base` `skel` has 124 joints rooted at the feet (ankles 0.12 m, pelvis 1.06 m, A-pose).

## Animation clips

`ANIM` items hold the clip at +8 (ICE "CD" clip data, magic `0x43440107`; `ANIM_RAWCHANNELS` carry only the `align` and `apReference` channels). Clip header: hierarchy hash +4 (the skeleton's JH hash: the player clips name `0xef94465d`, the 230-joint rig in `actor27.psarc/proto.pak`, 204 animated joints, with 1,548 clips), size +8, frame count `u16` +0xe, frames per second +0x1c, program block +0x30, first frame record +0x38. The program after the block's 16-byte mask is a list of `{u8 argument count, u8 opcode, u16 arguments}` with the EBOOT's animation command numbering (`sub_2D7058` names: 11 `CopyUniformDataToKeyCache`, 15/16 `KeyCacheUniformLerp/Slerp`, 21–24 constant vector/quaternion decompression, 26 `Dec3VecFloat16`, 27 `Dec3VecRange`, 28 `DecompressQuatSmallestThree`, 29/30 `PostDecompressQuatLog[Pca]`); data offsets are relative to the program (block + 0x10: 0x60 in the player clips, 0x50 in the `anim-train-wreck-climb` prop clips), lists are padded to multiples of 8 with a repeated output, and outputs are offsets into a 48-byte-per-joint buffer (scale, quaternion, translation). Decoding rules and their evidence are in `src/anim.rs`. `Dec3VecRange` records are 16 bytes (a 21-bit unsigned step, two signed steps, a three-float minimum, field starts at bits 21, 42, 64, 86 and 107) followed by a 4-byte `[a, b, a + b, c]` width table per item; `DecompressQuatSmallestThree` descriptors use the same width form.

The SPU handlers are in the `CalcInfluences` job (loaded at local store 0x4000; dispatcher at listing 0x946C, 62-entry handler table at 0xA850 indexed by the opcode above; `Dec3VecRange` 0x30C0, `PostDecompressQuatLog` 0x6C80, `PostDecompressQuatLogPca` 0x6E90), and their formulas were checked by executing those handlers on controlled inputs:

- `Dec3VecRange` widths use the low six bits of bytes 0, 1 and 3. The top two bits of each are a predictor `f`: when non-zero, component `f − 1` of the previous item's decoded vector of the same frame is added to that component.
- `PostDecompressQuatLog(count, keys, parameters, key slot)` turns each key vector `v` into `e = (s·v, c)` with `t = v·v`, `c = 1 − t/2 + 0.04152372 t² − 0.0012714077 t³` and `s = 1 − t/6 + 0.0083172908 t² − 0.00018526176 t³` (constants at listing 0xA7F0/0xA800), then `q = q0 ⊗ e`. The parameters are 16 bytes per pair of items with interleaved `s16 / 32768` halfwords `[x0 x1 y0 y1 z0 z1 w0 w1]`. Nothing is relative to the rest pose and the result is not renormalised.
- `PostDecompressQuatLogPca` gives `q = P ⊗ e ⊗ Q`, with 16 bytes per item `[Px Qx Py Qy Pz Qz Pw Qw]` (`s16 / 32768`).

Every opcode of `idle`, `walk`, `move` and `walk-hurt` decodes.

## Player animation state machine

State records are 0x48 bytes: +0 name, +4 id, +0x10 tree, +0x14 logical animation, +0x30 transitions, and +0x38/+0x3c script lambdas. The +0x3c lambda returns non-zero for mirrored states (`ledge-shimmy-left` 1, `-right` 0; both trees name `ledge-grab-shuffle-r-hurt`). Mirrored clips are built from the bind pose: each joint takes its `l_`/`r_` partner's pose reflected across the YZ plane, `W′ⱼ = R·W_p·R·(R·B_p·R)⁻¹·Bⱼ`. `SKELETON_FLIPDATA` holds 39 joint pairs, each with a 32-bit component code (`0x0ed8bed8`, `0x0ed8fa9c`, `0x0a9889ef`), and is not decoded.

### States

`dc1.psarc/anim-player.bin` `anim-actor` `anim-player`: state count +0x30, inline 0x48-byte states +0x34, initial state +0x38 (`idle`); layout in `src/anim_state.rs`. 988 states; transitions `{request, flags, blend record, target, 0, 0, fade, -1, 0}` with `auto` taken at the end of the state's animation. Blend trees are nested blend nodes (type `0xb249ac72`), layer/select nodes (`0x1c3f471c`) and `anim-node-animation` leaves; the `run` tree's select node lists clips by movement type (`run-tense`, `walk-balance`, `walk-hurt`, `run-hurt-long`, …), the options of the `move` `player-overlay-info` that `set-walk-type`/`set-run-type` choose. Hurt clips live in `actor27.psarc/anim-proto-hurt.pak`. The `align` raw channel gives root motion (`move` 2.8 m/s, `walk` 1.5, `walk-hurt` 0.8, `run-hurt-long` 3.9).

Blend-tree nodes (`src/anim_tree.rs`):

- **Blend** `{left, right, flags, lambda, 1.0}`: the lambda `(node, info, phase, instance)` returns the right child's weight.
- **Select** `{default, alternatives, flags, lambda, 0}`: the lambda `(node, info, instance)` returns the child to evaluate. DC arrays carry no length.

The 6,131 tree lambdas only read two per-actor blocks and call `lerp-scale` (PPU 0x1416cc: `t = clamp((x − a)/(b − a), 0, 1)`, `c(1 − t) + d t`) and `minmax` (0x14ab64: `min(max(x, lo), hi)`).

- **Instance block:** +48 is the movement-type index and +52 the weapon index.
  - Movement types: the 39 records of `#b35a5e92`, 0x6c bytes each, with the `StringId` at +0x40, the name at +0x48 and the index at +0x4c. `run` 0, `walk` 2, `run-tense` 3, `run-stealth` 7, `run-mo` 9, `run-hurt` 11, `walk-hurt` 14, `relaxed-run` 38.
  - Weapons: the `weapon-base` `player-overlay-info`: binoculars, no-weapon, pistol, long-gun, grenade, crossbow, rpg.
- **Info block (locomotion states):** +28/+32 lean right/left, +40 uphill (`run-incline`), +44 downhill spine, +48/+52 stairs up/down. +0x228 is a cycle phase. These meanings are inferred from the clips the blends lead to.

Every state evaluates. With zero inputs: `idle` is `idle` plus the additive `idle-base-big-add`, `walk-hurt` turns it into `idle-hurt` and `run` into `walk-hurt-long`, and a lean of 0.5 blends `move-lean-r`. Clips named `…-add` hold local deltas (near-identity rotations, untouched joints unanimated) and play as additive layers.

The PPU locomotion controller is reproduced from the transition table in `src/player.rs`. Jumps, falls and landings have no incoming transitions and are entered directly.

Animation leaves are `{clip, ?, ?, phase lambda, ?, clip lambda}`. The phase lambda `(node, info, instance, state phase, 0)` returns the sample phase; look sweeps read it from the info block (NPC +140 left–right, +144 up–down; aim +92/+96). The clip lambda `(node, info, instance)` replaces the clip (`face-reaction`).

`anim-npc.bin` has the same `anim-actor` layout: 264 states, initial `s_ambient-gunout-idle`. Its lambdas also call `angle-mod` (PPU 0x2db59c: `fmod(x, 360)` brought into [−180, 180] with the TOC constants 360, 180 and −180).

NPC leaves are logical animations that `anim-sets.bin` `anim-set`s (`{array, count}` of `{logical, clip}`) map to the character's clips. Example: `*flynn-anim-set*` maps 274 of its 276 leaves into `anim-flynn.pak`, and `*flynn-heist-anim-set*` overrides 4 of them. No module or EBOOT string links characters to sets. The `*NAME-anim-set*` names match 11 art-group and character names, which is the rule used here.

Task continuation records (subnode +0x28) are `{string, StringId}` slots: the player's spawner at +0, allies' at +0x08 to +0x48 (`continue-flynn-…` in 24 records, `continue-rescuer-…`, `continue-ally-…`) and the camera's at +0x50.

## Script animation

- **`animate` / `wait-animate`:** `(animate object clip key value …)` plays a clip once on an object: a process (it plays on the process's spawner entities), a spawner name, or `player`. The level scripts use three keys: 2 (blend-in seconds), 4 (blend-out seconds) and 7 (a locator record from `get-ap-ref-locator`/`get-spawner-locator`). Other keys are ignored. `wait-animate` suspends the track until the clip ends.
- **`get-animation-phase`:** `(get-animation-phase object)` returns the phase (0–1).
- **`get-object-locator` / `get-ap-ref-locator`:** `(get-object-locator object)` and `(get-ap-ref-locator ? object)` return that object's locator record `{x, y, z, 1, qx, qy, qz, qw}`.
- **Where clips come from:** the entity rig's packages, then the `pak22.txt` `actor` packages of the task's levels (`anim-train-wreck-climb` for the prologue cars). The clip is decoded for that rig's skeleton and played from a single-clip library.
- **Root motion:** `ANIM_RAWCHANNELS` channels hold the first and last transforms (two 11-float `{quaternion, scale, translation, 0}` records) and at +88 `{samples, u16 record bytes, u16 components, u16 count, u16 flags}`. Records are 12 bytes (translation, components 4) or 28 bytes (quaternion then translation, components 6), one per frame. `walk` moves 1.9 m/s, `move` 4.0 m/s, `move^turn-180^move` turns 180° and `jump-long` drops 3.25 m. A script animation places clip space at `locator · apReference(0)⁻¹`, or at the entity's start times `align(0)⁻¹`, and moves the entity along `align`.
- **Clips from other rigs:** story clips for the player (`anim-train-wreck`) name the `npc-normal-base` hierarchy (`6077142a`, 124 joints), while the player models skin to the `proto.pak` rig (230 joints, 106 joint names shared). Such clips decode with the rig whose hash they name and bind to the entity by joint name.
- **Prologue:** `train-wreck-climb` plays `idle-00-car-a`, `idle-00-car-b`, `idle-00-undercarriage` and `idle-00-ladder` (10.07 s), the 1.2 s `drake-platform-drop-door` and the single-frame `broken` pose. Props hold the last frame of a finished animation.

## Scripted cameras

- **`camera-enable`:** `(camera-enable 'cam-name …)` selects a camera spawner (`cam-railing-center-intro-1`). The camera looks along the spawner's local +z: from the climb start camera, +z points at the player's spawn with a dot product of 0.996. Up is local +y, and the spawner's `fov` property (75–110 in `train-wreck-climb-fg`) is horizontal.
- **Release:** `camera-disable` / `camera-abandon` / `camera-disable-all` return to the follow camera. In `train-wreck-climb` the intro camera is released right after it is enabled.
- **Camera animations:** `(camera-animation-enable name clip blend)` (`DcCameraAnimationEnable` → `sub_366730`) keeps up to 10 named camera animations (64-byte records); a running one keeps its phase, and the scripts call it every frame while it should run. `camera-animation-disable` (`sub_364550`) ends one. The clips are in the level's `camera-…` actor package (`camera-train-wreck.pak`: `camera-train-wreck-treadmill`, `-exterior-small`, …); their camera channel is raw channel `0x306c3455` (no string with that id is stored): per frame `[fov°, ?, ?, quaternion, translation]` (components 7) or `[quaternion, translation]` (components 6). They loop and offset the active camera in camera space, relative to their first frame; that application is inferred. `sub_36624C` beside them is a separate procedural shake.
- **Object-relative cameras:** camera spawners parented to a moved object (the `cam-car-b-…` cameras of `train-wreck-car-b-1`) follow its pose like other child spawners (below). `camera-local-space` (in `ss-sham`, `ss-train`, `ss-warzone` only) is not implemented.

## Foreground (art-group) meshes

Art-group meshes use the same descriptors as level geometry (above); their `vis_sphere` tags (`TAG_VEC4`) bound the model, and spheres of animated objects cover their motion. Skinned player models carry no `JOINT_HIERARCHY`; they bind to the `proto.pak` rig (230 joints), whose `idle` clip the player renders.

## Spawners

Spawner (+0x40) points to a parent spawner; locators of children are in the parent's frame. Children follow their parent object when it moves: script root motion places `train-wreck-car-b-1` about 1.1 m from its spawner (`idle-00-car-b`: `apReference` holds the inverse of the spawner rotation exactly, `align(0)` a further 0.67 m), and its rail, seats, shimmy rails and child cameras keep their spawner offsets from it (`child = parent now · parent spawner⁻¹ · child spawner`). The player stands and hangs on such bodies and rides their motion. Flag 0x40000000 is set on continue points, companion placements and cameras and is treated as no-auto-birth; the flag test in the ELF has not been located.

## Collision and climb edges

`src/collision.rs` documents the layouts. Art groups carry `COLLISION_DATA_FOREGROUND` (Havok 6.1 packfiles bound to joints, so the bind pose places them in the art-group frame); levels carry `COLLISION_DATA_HAVOK_BG` triangle subparts in the render meshes' metre space. PAT bits 0–5 are the surface type; the flag bits come from the name table at EBOOT 0xd6c9e0. Climb edges are built offline: levels store them in `FEATURE_DB_2`, props behind their collision wrapper (+0x10). `train-wreck-car-b` reads 2,697 collision triangles (extents −2.50…2.52 × −0.17…5.76 × −12.44…13.58 m), 438 edges and 132 corners; the `train-wreck-climb-fg` level holds no edges, its prologue edges belong to the car's props. Packages without native collision fall back to their render triangles for the viewer's ground.

## Material layering

`src/rsx.rs` decodes RSX fragment programs. The shader table is `MATERIAL_TABLE_1` +0x1c; each program stores its size at +0 and its microcode at +0x10, with instruction words half-swapped. Passes bind texture units through their parameter table at pass data +0x14: 16-byte records `{u16 register, u16, u32 class, u32 value, u32 name}`, with the count in the low half of word 2 and names in a package string pool. A record of class 0x0202 binds the k-th material sampler (sampler k + 1) to the unit in its high half. Class 0x0201 binds global buffers: the light pre-pass on 12/13 and the screen shadow. In `rock-to-snow`, rock is unit 2, mask unit 0 and snow unit 3. The recogniser follows weights as affine sums of the world normal's y (`TEX2`, from `v2 · c[8…10]`), the vertex mask (`COL0.x`, attribute 0x15) and texture channels, and finds two-layer blends:

- **Slope:** `mix(top, side, saturate(1 − n.y))`. `trnwrk-snow-mod-cliff-cheap` puts rock under snow on the same UV (`TEX0 = v1.xyxy`). The `cliffwall-snow` and `snow-boulder` programs use a computed light grey on top (0.5 plus three saturated terms in gamma space). `trnwrk-tree-bark-cheap` carries snow on its bark.
- **Height mask:** the side layer is weighted `saturate((mask.g − (1 + s)·m + 2s) / 2s)`, with `m` the vertex mask and `s` = `g_fBlendSoftness` (0.1). It appears in `trn-wide-mountains-snow/rock-to-snow`, 13 `ice-cave-temple` materials and the `menu-temple` gold and plaster blends.

Shading follows the native passes: diffuse light from the pre-pass and probes, with specular only through a `…-spec-mask` sampler, so materials without one reflect nothing (the shared PBR environment reflection is off for them). The first non-placeholder `…-normal` sampler perturbs the normal; DXT5 normal maps hold X in alpha and Y in green (mean channels `(0, 136, 0, 139)`), DXT1 ones XYZ in RGB. Water binds an environment reflection map (`…-env-reflection`, 128² lat-long) that its program adds at 0.6 along the reflection vector over the scrolling colour; here a smooth specular reflects the environment instead.

Pass 5 holds the albedo alone. Materials whose pass 0 binds none (`snow-ground-main-noblend`) take it from there.

## Hanging and climbing

The hang and climb clips locate the edge by `apReference` relative to `align`: with `AP = align(0) · apReference(0)`, the root stands at `edge · AP⁻¹ · align(t)`, where the edge frame has +z into the wall (against the edge's wall normal) and +y up. `ledge-grab-idle-hurt` holds the edge 1.53 m above and 0.5 m ahead of the root. The chest step (`ledge-grab^ledge-grab-chest-hurt`) ends, and the knee step (`ledge-grab-chest^ledge-grab-knee-hurt`) begins, with the root 0.6 m below and 0.34–0.37 m behind the edge. State flow from `anim-player.bin`: requests `ledge-shimmy-left`/`-right`/`ledge-grab` from the hang; `ledge-climb-up` → `ledge-climb-up-2` (`auto`), whose `idle`/`run` requests lead to `climb-onto-1-75m^idle`/`^run` and then `idle`/`run` (`auto`). The PPU controller that issues these requests and its edge queries (`sub_4FC568`) are not decompiled. Here a continue spawner within 0.3 m of an edge starts hanging (the spawner marks the grab point; `continue-player-climb-1` lies 0.11 m from `trn-wrk-rail` edge 7). Shimmy root motion slides the grip along the edge and across its `prev`/`next` links. The climb is offered when a standable surface lies within 0.35 m of the edge height 0.45 m beyond it. An airborne player grabs an edge within 0.45 m of the hang reach that faces it. The reach, offer and grab distances are not native. Jumps between edges (`ledge-jump-begin` → `-mid` → `-end`, `auto`) use clips the EBOOT names directly (`ledge-jump-up-short`/`-long`, `-right-short`/`-long`, …). Their `apReference` locates the target edge, 2.43 m (short) or 3.90 m (long) above the root at the start, against 1.53 m in the hang. Here the tree's default clip plays, and the root eases from the hang to the target edge's hang over 0.9 s.

## Render settings

`src/render_settings.rs` documents the layout. `render-settings.bin` holds 238 `render-settings-2` records named `*AREA-render-settings-NN*`. Its `render-settings-menu-entries` lists every field as `{":name", offset, type, step, step, min, max}`, in groups for lighting, shadow, fog, wind, sky, post and tonemap. The EBOOT's writer (`sub_123C1C`) places lighting at 0, post at 0x440 and tonemap at 0x4b0. Trigger regions select a record with a `render-settings` entry, and cutscenes use `set-render-settings` (`scenes.bin`). The tonemap group holds Hable's constants, a light range of ±4 stops, an adaption time and a key, `tonemap-manual-exposure-value`. Two records differ sharply:
- `*menu-temple-render-settings-01*`: key 0.052, shoulder 2, toe 0.75, bias −2.6;
- the train-wreck records: key 0.4–0.7, shoulder 0.42–0.6, bias 0.5.

Render-settings regions belong to the `camera` category (0x89335b6e) and are tested at the camera's eye. The prologue's `regiongroup-train-wreck-climb-states-1` selects `#f8cd5e96`. Here the camera adapts with a constant compensation of 0.5 stops under the record's curve and bias: the menu matches the original at 0.46, while the record's key would blow out the prologue (`log2(0.7 / 0.18)` + 2.25 ≈ 4.2 stops). The native exposure modes (the caller of `sub_ABB33C`, mode 3 reading +0x638…+0x644) are not decoded, and the probe scale (2) was fitted on the menu. Regions that pick their record by script lambda (`train-wreck`) are not evaluated. The writer prints `:fog` from 0x1b0 and `:sky-fog` from 0x3d0 (an earlier reading of 0x3d0 as the fog took the sky fog). The fog writer `sub_7587F8` turns the group into a Gaussian fog: opacity `density·(1 − exp(−(max((w − start)/range, 0)·max(1 − (y − height-start)/height-range, 0))²))` at view depth `w` and height `y`, with `density` applied only when the multiplier is at least 1; colour is linear (the menu temple's (2.6, 2.75, 1.5), start 10 m, range 25.7 m, density 0.2, a negative height range that thickens the fog upward). It maps to the shared exponential fog with the same cap and the same opacity one range past the start; upward thickening stays uniform. Particles drawn additively are attenuated by the fog instead of taking its colour, as the godray program does (`colour × (F·ρ + 1 − ρ)`).

## Particle effects

`src/particle.rs` documents the descriptor layout. Trigger regions name the area's particle DC module with a `part-module` entry (`menu-temple-ingame10` → `part-menu`, `train-wreck-ingame10` → `part-trainwreck`, `part-trainwreck-climb`), and `PartSpawner` spawners place an `effect-group-desc` by its `particle-group` string, with `color` and `effect-scale` (vec4, property type 8), `alpha` and `sprite-scale` tags (`sub_7616E8`). Each `particle-effect-desc` (version 8) holds looping cubic curves, ramps (step, linear, ease-in, ease-out, cosine), an attribute table, a spawn and an update program for the 23-operation interpreter of the SPU particle job (`sub_12E0`, the RNG `rng·0x6255 + 0x3619` restarted from each particle's seed), the emission (+0xc8, a burst count with flag 0x4, otherwise particles/s), the render orientation (+0x4c: 0 camera-facing, 1 about spawner Y, 2–4 velocity, 5/6 the spawner's YZ/XZ plane, 7 its frame) and the emitter shape (+0xec: point with a lifespan mode, or a volume). Materials are `particle.pak`'s `MATERIAL_TABLE_1` (+0xc count, +0x20 records, record +0x18 `StringId`; pass data +0x20 the colour blend factors). Float parameters at or above 2⁶³ reference ramps and above 2⁶⁴ curves. Every descriptor of the 61 installed particle modules translates (`examples/particle_effects --check`). The menu's light shafts are 1-particle infinite planes (`menu-godray-barndoor` 8.5 × 16 m about ±1.2 m offsets; `-skinny-hres` 0.63 × 10.5 m), the dust 14 particles/s for 3–10 s, 0.03 m, from a 2.0 × 0.4 × 2.3 m box.

## Navigation, splines and cover

`src/nav.rs` documents the layouts. `NAV_MESH_1` meshes hold up to 255 clockwise (x, z) polygons with per-edge neighbours, a lookup grid and links that pair meshes by `StringId` (a link polygon copies the partner's pre-link polygon). The path search follows `NavControl::PathFindPs` (`sub_2AEF88`): `FindPoly` with a 2 m vertical tolerance, A* with successor nodes at the closest point of the shared edge, `g + |step|`, `h = |goal − p|`, 300 open nodes. `ice-cave-temple-ingame10`'s rescuer route `ict-rescuer-5a` → `continue-rescuer-…-cp-2` is 15.13 m. `SPLINE_TABLE` records are uniform Catmull-Rom (linear with flag 2 or two points, closed with flag 1), lengths by Gauss–Legendre quadrature and 31-entry arc tables (`sub_36480`); `train-wreck-climb-fg-ingame10`'s curved splines measure 5.748, 2.954 and 13.795 m as stored. Their points are in the level frame and follow a moving parent spawner. Cover points come from `FEATURE_DB_2` corners at login (`sub_197F10`). `LEVEL_OFFSET_1` (level +0x20) offsets nav meshes, splines, features and collision; the installed train-wreck and menu levels store zero, `ice-cave-temple` (0, 0, −300).

| Native | PPU | Semantics |
|---|---|---|
| `spline-locator-at-distance` | 0x1666f8 | `(spline, distance)`: a locator at the point facing along the tangent |
| `spline-distance-locator` / `spline-distance-spawner` | 0x165be0 | `(spline, locator/spawner)`: arc distance of the closest point (sampled here) |

## NPC agents

The NPC natives (PPU 0x221000–0x22E200, `src/ss_npc.rs`) look the NPC up in
the NPC manager (`sub_1E2968`, live NPCs only); without a name they use the
calling process. Failures print the original message and return 0. State
lives in `src/npc.rs`:

| Native | Native field / rule |
| --- | --- |
| `npc-set-accuracy-multiplier` | Npc +0x26E4 (default 1.0); returns 1 when found |
| `npc-enable-shooting` / `npc-set-shot-damage-multiplier` / `npc-set-zone-update-method` / `npc-disable-flinch` | `m_pConfiguration` (+0x2660): +0x30 bit 29 = shooting disabled, +0x38 float, +0xCC word, word 0 bit 58 cleared |
| `npc-add-skill` / `npc-remove-skill` | configuration +0x20 bit mask |
| `npc-set-in-combat` / `npc-set-preoccupied` / `npc-in-combat?` | `m_pKnowledge` (+0x2664) state +0x1450: 4 combat, 1 preoccupied (`sub_1D6580`, `sub_1BB62C`) |
| `npc-override-contact-status` | argument 0/1/2 → contact status 1/2/3 (`sub_1D6748`) |
| `npc-prefer-target` | knowledge +0x1190 |
| `npc-set-demeanor` | 1 and 2 select those demeanours, others 0 (`sub_1A3EE4`) |
| `npc-set-zone-*` | two 32-byte zones at Npc +0x2680: radius +0, y-threshold +4 (absolute values); a marker spawner's tags 0xA28A21EC/0xAC43B6DD apply when ≥ 0 and set update method 3 |
| `npc-set-can-die` / `npc-set-can-be-hit` | health component (+0x15C) vtable +0x4C; can-be-hit also sends `ignore-all-fire-no-blood` with the inverted flag |
| `npc-set-ignore` / `npc-mark-as-defunct` | targeting bit 14 / Npc +0x1858 bit 42 |
| `set-object-faction` | factions 0–11; others are invalid (`sub_85A40`) |
| `npc-is-player-in-front?` | player within 2 m and unit offset · forward ≥ 0.5 |
| `broadcast-event-to-npcs` | event and arguments to every manager slot's process |
| `npc-move-to-spawner` (npc spawner [motion] [goal]) | motion 0 walk, 2 sprint, else run (codes 1/5/3); goal 1 stop-and-face, 2 continue, else stop; the spawner's 0xA28A21EC tag is the arrival radius |
| `npc-move-to` (npc point [motion]) | run by default; both fail "off the nav mesh" without a nav path |
| `npc-turn-to`, `wait-npc-bind`, `npc-unbind`, `npc-use-scripted-ap`, `wait-npc-leave-scripted-ap`, `npc-use-closest-cover` | orders of the scripted skill (skill 2, `sub_2A10B0`); the AP spawner must be of type `action-pack` |

The `wait-` forms suspend the track until the order completes. Speeds,
turn rate and the immediate bind are not native (the skill moves by clip
root motion).
