# ADR 0013: assessing the "Ashen Vow" Unreal demo as a port target

Status: in progress. The owner confirmed authorization for the packaged
build (no project source) and chose not to run the original on this Mac
yet. Extracted data stays in ~/.cache/crysirror/ashen (outside the repo);
nothing extracted is committed.

## Facts gathered (2026-10-24, from the local build in ~/Downloads only)

- UE5 Windows shipping build, project name `Portfolio`, packaged with IoStore
  (`Portfolio-Windows.utoc/.ucas`, 2.9 GB). TOC version 8, 6702 chunks.
  Container flags: compressed + indexed; **not encrypted**; compression
  method **Oodle**. Shaders are D3D SM5/SM6.
- Only the uncompressed TOC directory index was read, with a 60-line
  metadata parser (kept in the scratchpad, not in the repo). It lists 4116
  game paths. Only aggregate counts are recorded here:
  - Maps: 3 (a world root, a fantasy castle, a haunted village).
  - Gameplay Ability System: 61 abilities, 36 effects, 21 cues, plus an
    AbilitySystem folder.
  - Animation: 150 sequences, 96 montages, 10 blend spaces, 7 anim
    blueprints, 6 skeletal meshes.
  - AI: 8 StateTrees, EQS. VFX: 26 Niagara systems. UI: 26 UMG widgets.
    Input: 24 Enhanced Input actions. Audio: 167 waves, 75 cues.
    Materials: 444 / 236 instances / 57 functions. PCG: 10.
  - Gameplay keywords seen in names: parry, groggy/stagger, oil flask,
    health flask, bonfire, boss room, portal, sealed gate.
- Running it here: the Homebrew Wine is x86_64 and Rosetta is absent
  (`bad CPU type`). Even with Rosetta, D3D12 needs a D3D12→Metal layer
  (CrossOver or Apple Game Porting Toolkit), which is not installed.

## Options

1. **Owner has the UE project source:** export meshes/animations with the
   UE editor (FBX/glTF), read Blueprint/C++ logic as source, and port it
   system by system. This is the cleanest and fastest route, with no
   reverse engineering.
2. **Build only:** extracting content requires Oodle decompression (Epic's
   oo2core under the UE EULA, or a reverse-engineered decoder of unclear
   legal status), plus UE5 Zen package, mesh, animation and material
   deserialization. C++ gameplay exists only as compiled machine code, and
   porting it would mean binary reverse engineering. Not recommended
   without explicit rights to the content.
3. **Run the original** for reference captures: install Rosetta plus
   CrossOver/GPTK. These are system changes for the owner to make.

Whatever is chosen, the inventory above becomes CrySirror's next system
backlog: a GAS-like ability/effect/attribute system, animation montages
and blend spaces with an animation state machine, behaviour-tree/StateTree
AI with navigation, a particle system, a material graph or layered
materials, level streaming, and audio cues.

## Progress (2026-10-24)

`crates/crysirror-ue-import` (a developer tool, not in the player) adds:

- An IoStore TOC v3–8 reader: chunk ids, BE40 offsets/lengths,
  compression blocks and the directory index.
- Chunk extraction with Oodle decompression via `oozextract` 0.5.6 (MIT,
  open-source decoder).
- A Zen package header parser (UE 5.5+ 60-byte summary with cell maps,
  name batches, 72-byte export entries) and the global ScriptObjects table.

Result on the build: **3476/3476 packages parsed, 0 failures**; 45,321
script objects resolved. Export classes include 10,068
StaticMeshComponent, 4,490 StaticMeshActor, 1,141 ISM components,
256 Landscape components, 1,003 Texture2D, 408 StaticMesh, 506
AnimSequence, 94 AnimMontage, PoseSearch (motion matching), Optimus
deformers, PCG, Niagara and Cascade, 411 point lights, 213 Blueprint
classes, and a game C++ module `/Script/Portfolio`.

## Historical blocker: class schemas

Cooked UE5 packages use **unversioned property serialization**. Decoding
UObject properties (component transforms, mesh references, material
parameters, Blueprint defaults) requires each class's property schema (a
`.usmap` mapping). The schema exists only in the running game's
reflection data. The standard way to get it is to run the game once on
Windows with a mapping dumper (e.g. Dumper-7) and save `Mappings.usmap`.
With it, the next steps are: StaticMesh render data → glTF, Texture2D (BC
formats) → PNG, level actors → a CrySirror scene, then skeletal meshes and
animations. Game C++ logic exists only as machine code; it is
re-implemented from observed behaviour, while Blueprint bytecode can be
decompiled once schemas are available.


## 2026-10-25: actual static scene import and Metal capture

The schema blocker was overcome without running the original executable:
the existing `reflect` module recovered UHT registrations from the shipping PE,
and `ue-import schemas` now writes a standard uncompressed `.usmap`. CUE4Parse
at an actual pinned revision consumed it and loaded the conventional static LODs,
component hierarchy and instance transforms. CrySirror rendered the castle map:
7,667 preview nodes, 13,426,416 submitted triangles, 31 original base-color textures.
42 unsupported fog sheets and the sky sphere are explicitly excluded.

This is a partial visual asset import. UE shader graphs, original lights,
landscape/characters/particles and gameplay remain unsupported; texture-name
ambiguity is reported, and no original-image comparison was made. Details,
commands, source revisions, tests and the separate GoW III failure are in
[LOCAL_GAME_IMPORT.md](../LOCAL_GAME_IMPORT.md).
