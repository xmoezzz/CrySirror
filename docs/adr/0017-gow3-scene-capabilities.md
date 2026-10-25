# 0017: Native scene trials drive shared asset and rendering APIs

Date: 2026-10-25. Status: implemented first preview; remaining APIs proposed.

The owner requested rendering the local God of War III scene to identify missing
engine capabilities. The selected asset is **OLY100.WAD**, one resource in
OLY.PSARC. This acceptance scope is the selected resource, not the entire game,
every Olympus streaming cell, or execution of the PS3 executable.

## Implementation

Keep title-specific WAD/MG/MDL/entity/material layouts in the developer frontend
`tools/gow3_scene.py`. Reusable topology and image codecs belong to the safe,
device-free engine crates. The desktop and renderer receive ordinary glTF/PNG
and shared scene data. No GoW resource tags enter GPU code or either VN VM.

New shared APIs, actually used by the native conversion:

```rust
crysirror_asset::indices::decode(bytes, encoding, index_count, vertex_count)?;
// IndexEncoding::{U16Le, U16Be, Edge}; preserves encoded winding.
crysirror_image::decode_standard(dds_bytes)?; // validated RGBA8 surface
```

The independently implemented EDGE profile uses bounded MSB-first flags,
two-bit triangle controls, eight-lane delta prediction and exact vertex/reuse
totals. Counts, versions, indices, streams and padding ranges are checked.
Winding conversion belongs to the native frontend. DDS supports 2D BC1/BC2/BC3
through pinned image 0.24.9, plus explicitly masked RGB(A)32, R8 and A8. Images
have dimension and 64 MiB encoded/decoded limits; unsupported FourCC, cubemap,
volume and raw channel layouts fail. These are CPU capabilities, not GPU BCn
texture upload or high-mip streaming.

`crysirror-asset/examples/transcode.rs` is a developer batch adapter over those
APIs. Inputs and texture outputs resolve beneath the manifest directory, with
canonical-path and size checks. Its explicit result-file CLI argument is an
author-selected destination. This tool is not an unrestricted runtime/MCP file
service. The native frontend calls the Rust codecs for the actual conversion;
Python's independent topology decoder is retained as a cross-check.

`SystemsSpec.environment: Option<Environment>` initializes shared lighting
through the existing validated `SetEnvironment` command. Installation remains
transactional and the owning client scopes HDR paths and saved state. An absent
field leaves the preceding environment unchanged; the last explicit client in
project order owns the environment. Python schema validation and asset inventory
include HDR references and reject capability escapes. The camera already has a
data representation in `project.scene3d.camera`.

The native preview is **JSON + glTF + PNG**, with an initial scene entity and
environment; no JS/TS execution is required. The generic preview compiler keeps
its earlier UE authoring-script mode and adds explicit project identity,
optional tangent attributes and a data-only mode. This avoids a title-specific
engine host or new instruction aliases.

## Native layout and confidence

The WAD inspector treats `EntityCount` as quantity metadata with no payload.
Other unknown metadata still fails. Geometry follows declared relative offsets
and two verified attribute layouts, rather than signature scans. It decodes
float positions, signed packed 11/11/10 normals, optional signed-normalized
tangents and the observed float UV0 stream. Unknown vertex profiles fail.
Each surface's declared material slot is read from its group header, independent
of the surface ordinal. All 1,474 slots fit the referenced MDL material tables;
many fractured surfaces reuse one slot. Using ordinal assignment incorrectly
turns those surfaces into neutral fallbacks.

The selected resource contains 16,520 records, 247 MG groups, 1,474 surface
groups and 2,118 compressed topology streams: 495,471 vertices and 548,191
triangles. The entity/model graph names all 247 MG resources without missing
model references. This is resource association, not proof of native transform
or animation equivalence.

Some vertices already contain level coordinates: for example, tiny FX geometry
coincides with its entity translation. Applying that entity matrix directly
double-transforms it. The preview therefore **preserves encoded rest-space
positions**, records native entity matrices as diagnostic metadata, and makes no
claim that native entity/bind/animation composition is decoded. Doors, moving
platforms, overlapping props and helper geometry can remain in the wrong pose.

Texture previews select the validated inline low mips from WAD DDS wrappers,
normalize native header flags and decode them with the shared Rust image API.
The higher TS mips are not required or streamed. 97 texture previews decoded
(89 BC1, 7 BC3, 1 raw); 75 distinct textures are used by 393 explicit preview
material profiles. Source sampler parameter 0 is treated as base color with
opaque, two-sided, rough PBR **as an approximation**. Other native shader,
alpha, normal/specular, sampler and lighting semantics remain unverified.

1,380 surface groups have preview textures and 94 retain neutral materials.
83 groups have no verified UV stream, including seven with unresolved UV
cardinality/remapping. 53 source material records have no unique sampler 0;
those materials may use native shaders that require no base texture. The report
keeps these cases explicit rather than claiming source material equivalence.
1,006 non-unit source normals
are normalized (a zero vector uses an up-normal fallback). Neither is hidden
as original fidelity. Source coordinate units are preserved without physical
calibration. The report marks original materials, animation, lighting and
complete-game-scene coverage false. The first MG surface table is selected;
other native tables remain unclassified.

## Required engine APIs versus native importer work

The following are proposed next steps, not implemented capabilities:

| Priority | Need demonstrated by this trial | Shared engine direction | Native frontend responsibility |
|---|---|---|---|
| P0 | Correct placement and initial moving-part poses | Existing glTF nodes, skins and animation are already available; no new VM needed | Decode bind/entity/ANM composition and initial pose; 141 ANM records are present |
| P0 | Interior local lighting | `EntitySpec.light: Option<LightSpec>` and `Entity.setLight`; point/spot variants, bounded range/cones/intensity; renderer light culling and an explicit shadow budget | Decode the 207 light records (names include `PSpointLite*`); calibrate source units and behavior |
| P0 | Per-surface material authoring | Slot-addressed material overrides on model/scene entities; shared Rust/JSON/TS/save/MCP contract | Map MDL material slots, sampler roles, alpha and source parameters |
| P1 | Additional vertex attributes and baked appearance | PBR `COLOR_0`, UV1, texture-coordinate selection and lightmap/baked-lighting data | Identify remaining native bindings; do not guess that an unknown binding is a lightmap |
| P1 | Large resource sets and full mip chains | Client-scoped dependency resolver/import diagnostics, asynchronous source/transcode jobs, mip residency policy | Resolve PSARC/WAD/TS dependencies and source mip offsets; do not bake game paths into GPU code |
| P1 | Alternate geometry/LOD variants | Shared LOD descriptors, selection and diagnostics after source meaning is verified | Classify alternate MG tables; they are not yet proven to be LODs |

Current `PbrVertex` has position, normal, UV0 and tangent; current lighting has
sun + IBL; `Entity.setMaterial` replaces the whole entity material. Thus local
lights, additional attributes and slot overrides are concrete public-engine
gaps. Animation itself already exists: the missing GoW ANM interpretation is a
native frontend gap. Both distinctions matter when prioritizing the engine.

Do not add placeholder light/LOD APIs that silently discard data. Implement each
slice together with renderer behavior, save/validation contracts and a small
self-authored acceptance scene. A complete PS3 emulator, copied vendor SDK, or
one generic VM pretending to execute GoW would not satisfy this boundary.

## Provenance

The early model importer 0.65 beta by maver1k_XVII, posted in the
[primary format thread](https://www.cgig.ru/forum/viewtopic.php?t=2317), was
inspected in full as a layout lead. The retained implementation does not copy
its code or heuristic signature scans. The inspected primary code block was
8,235 bytes, SHA-256
`0e7b17aa198d87403fbdc0fd494c1fe5c64c8ab95e65e08e5e0eece0ce80cda6`.
The [original WAD discussion](https://zenhax.com/viewtopic.php@t=325.html)
also provided an EntityCount lead; the GoW III framing was checked against the
complete local resource. These are forum-source snapshots, not Git commits.
No fabricated revision or native SDK API is used. No external native decoder
was executed, and Rust/Python agreement is not a comparison against Sony's
implementation. Game bytes, external references and screenshots remain outside
the source export.

## Compilation and behavior

The final command results, test counts and capture metadata are recorded in
`../gow3-scene-results.json` and associated logs. Synthetic tests cover framing,
topology edge controls, negative eight-lane prediction, corruption/ranges,
declared geometry/normal layout, unknown attributes, DDS mip selection, image
channels/BC1, tangent export and data-driven environment initialization,
transactional rollback, ownership, persistence and HDR path isolation.

The actual native conversion compares all 2,118 Rust/Python topology results and
uses the Rust results as its output. Actual GPU rendering is recorded separately
from compilation and the default ignored GPU tests. No original-image equality,
complete scene/game fidelity, console execution, Windows/Linux execution,
hardware ray tracing or frame-time result is claimed.
