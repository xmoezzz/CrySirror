# 0054: FBX, PMX and VRM model import through the glTF pipeline

Date: 2026-10-26. Status: implemented (first slice). Evidence in STATUS.md.

## Context

The engine imported glTF 2.0/GLB only (ADR 0007). The owner asked for FBX
(the DCC/marketplace interchange format) and VRM/PMX (anime characters).

## Decision

1. **One pipeline.** `crysirror-asset::interchange` recognises a model by its
   content (`Kaydara FBX Binary`, `PMX `), converts it into an in-memory GLB
   and hands that to the existing glTF code. `import_gltf`, `load_pbr*` and
   `load_geometry` call it, so async loading, PBR materials, skinning,
   animation, streaming, physics colliders and ray tracing need no
   per-format code. VRM is glTF and already takes this path.
2. **Own parsers, no new format dependency.** The binary FBX record parser
   (32/64-bit headers, zlib arrays via `flate2`) and the PMX 2.0/2.1 parser
   are written in safe Rust with explicit bounds (64 MiB arrays, 4 M
   elements, nesting depth 64) and reject truncated or out-of-range data.
3. **FBX semantics.** Full local transform
   `T·Roff·Rp·Rpre·R·Rpost⁻¹·Rp⁻¹·Soff·Sp·S·Sp⁻¹`, six Euler orders,
   geometric transforms baked into unskinned vertices; axis system and
   `UnitScaleFactor` as one root node; fan-triangulated polygons with
   normal/UV/material layers in every mapping/reference mode; Skin/Cluster
   deformers; embedded (`Video.Content`) or external textures.
   - Bind matrices: bones use `TransformLink`; the mesh uses the `BindPose`
     entry, else its rest transform. The cluster `Transform` field is not
     used because exporters disagree on it (Blender stores it relative to
     the bone).
   - Animation: the first layer of each stack, sampled at the union of key
     times with linear interpolation, decomposed per node. Stack names are
     kept as written (Blender writes `object|action`).
4. **PMX semantics.** MMD units (≈ 8 cm) → metres, left-handed → right-handed
   by negating Z and reversing winding; BDEF1/2/4 weights, SDEF/QDEF as
   linear blends; bones as joint nodes; the skin contains only weighted
   bones so typical rigs fit the 256-joint palette.
5. **Visible losses.** Converters record what they cannot represent in the
   GLB `extras` (`crysirror_import_dropped`); `ImportReport.dropped` shows
   them with glTF morph targets and unsupported glTF extensions. A PBR
   reference import no longer reports its skins/animations as dropped, and
   `triangles` is counted.

## Morph targets (second step)

- glTF/VRM morph targets load as sparse per-primitive deltas named by the
  `targetNames` extras convention; PMX vertex morphs and group morphs made
  of vertex morphs convert to glTF targets (one mesh per material, so a
  mesh carries only the targets that touch it).
- Weights are entity state (`AnimationState.morphs`, set by
  `Entity.setMorph/setMorphs`) and are extracted with whole-scene
  drawables. The renderer blends positions and normals on the CPU into a
  per-draw vertex buffer, recomputed only when the weights of that
  primitive change, and every raster pass (prepass, shadows, opaque,
  transparent, probes) draws from it.
- Limits: per-mesh (`pbr: true`) model references ignore weights and say
  so in the import report; glTF weight animation channels, FBX blend
  shapes and PMX bone/UV/material morphs are reported, not applied;
  hardware ray-traced shadows still use unmorphed positions.

## Not done yet

- MToon shading (VRM uses its `KHR_materials_unlit` fallback), VRM spring
  bones/constraints/humanoid mapping, PMX IK, toon/sphere maps, physics.
- VMD motions, FBX cubic tangents, extra animation layers, ASCII FBX,
  FBX normal/specular maps, cameras and lights from FBX.

## Evidence

- `crates/crysirror-asset/tests/interchange.rs`: a self-authored Blender
  FBX fixture (skin, 1 s bend animation, embedded texture), ASCII/truncated
  FBX rejection, a synthetic PMX (units, winding, BDEF2 skin, alpha cutout,
  reported morphs/toon) and PMX index validation.
- Local, not in the repository: Khronos Fox glTF → Blender 5.2.2 FBX →
  CrySirror matches the glTF in three clips (relative bounds error
  ≤ 2·10⁻⁴); rendered running in the engine; VRM 1.0 Seed-san renders.
