# ADR 0011: porting real games to CrySirror to drive engine requirements

Status: accepted on 2026-10-24. Research done; the first port target is
selected. Implementation has not started.

## Requirement

The owner asked to take open-source implementations of real games (Zelda,
Tomb Raider, ...) and implement them completely on CrySirror. Doing the
work exposes what the engine must implement and optimize.

## Findings (checked 2026-10-24)

| Project | What it is | Code license (GitHub API / LICENSE) | Game data |
|---|---|---|---|
| [OpenLara](https://github.com/XProger/OpenLara) | Tomb Raider 1-3 engine reimplementation (C++) | **BSD-2-Clause** | Original TR level files, user-supplied |
| [snesrev/zelda3](https://github.com/snesrev/zelda3) | A Link to the Past C reimplementation | NOASSERTION (custom; must be read before any use) | Extracted from the user's US ROM |
| [zeldaret/oot](https://github.com/zeldaret/oot) | Ocarina of Time decompilation | none declared (all rights reserved by default) | Extracted from the user's ROM |
| [Ship of Harkinian](https://github.com/HarbourMasters/Shipwright) | OoT PC port on the decomp | none declared | ROM → .o2r archive |

The reference checkout of OpenLara is at
`~/.cache/crysirror/reference/OpenLara` (commit 3b268ca4bf98, outside the
deliverable). Core files: lara.h (3891 lines, character state machine),
camera.h, collision.h, animation.h, controller.h, level.h, format.h (6985
lines, TR level formats) and gltf.h (glTF export).

## Decision

1. **Primary port: Tomb Raider via OpenLara (BSD-2).** Port the gameplay
   semantics into CrySirror engine modules and keep OpenLara's copyright
   notice. The modules are the Lara state machine (states, transitions, input
   handling, physics constants), the third-person camera, floor-data
   collision, and animation state/commands. They run as engine systems with
   OOP/script APIs, not as a foreign VM.
2. **Data paths:** (a) a TR1 level importer (rooms, meshes, textures, sound,
   animations) for files the **user** owns, loaded through `ClientStorage`
   and never distributed. (b) An open-asset slice using CC0 characters and
   levels (Kenney / KayKit / Quaternius, licenses checked per pack), so the
   repository has a playable, distributable demo.
3. **Zelda:** no code is copied. Ocarina of Time / ALttP serve only as
   design references for systems (lock-on camera, item state machines,
   dungeon logic). We reconsider if a project with a clear permissive license
   is found.

## Engine requirements this exposes (to implement in order)

1. Skeletal animation from glTF (skins, keyframe sampling, blending) and
   TR-style animation commands and state changes.
2. Character controller and collision: capsule vs mesh/heightfield, step,
   ledge detection and grabbing, plus spatial queries. Physics needs its own
   ADR (in-house solver or a Rust crate such as Rapier, decided against
   determinism and save requirements).
3. Input action maps (gamepad and keyboard), exposed through the OOP API.
4. Third-person camera rig with collision; cutscene and camera tracks.
5. Positional 3D audio; sound events from animation.
6. Per-entity behaviours/components (ADR 0004 deferred this) and a save
   format for gameplay state.
7. Rendering for these scenes: many lights (clustered forward), water,
   particles, transparent sorting. Performance work: GPU culling, instancing
   and frame-time budgets measured with GPU timestamps.

## Acceptance

A port counts as done for a level only when: it plays from start to the
level exit with the original controls; Lara's movement and collision match
OpenLara with the same inputs (trace comparison); and save/load works at
defined points. Any unsupported original feature is reported in the import
report, never silently dropped.
