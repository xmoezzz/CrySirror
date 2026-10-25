# 0015: Engine material data and transparent PBR rendering

Date: 2026-10-25. Status: implemented and locally validated (first slice).

The local castle trial exposed an engine gap: glTF BLEND surfaces were treated
as alpha masks, the PBR fragment output forced alpha to one, and transparent
surfaces took part in opaque depth/shadow passes. Removing the source fog planes
made geometry inspectable but did not provide a reusable engine feature.

## Implementation

`crysirror-scene::material::{Material, MaterialSpec, AlphaMode}` is the shared,
validated CPU description. It contains no device objects, shader programs or
native-game policy. `Material<String>` references canonical client asset paths;
`crysirror-asset::pbr::PbrMaterial` uses the same description with model-local
image indices. Core/compat remain safe and device-free; winit stays in desktop.

The contract includes:

- Linear base RGBA, metallic/roughness, normal scale, occlusion strength,
  emissive RGB/strength; base, metallic-roughness, normal, AO and emissive maps.
- Opaque, mask, blend, additive and premultiplied alpha modes; cutoff, two-sided
  and unlit flags. Opaque ignores alpha; mask discards before depth/shadow writes.
- An independent linear opacity map (red by default, optional alpha channel),
  multiplied by base alpha; shared UV0 scale/offset.
- Optional `depth_fade` in world units: transparency fades near opaque geometry
  using reconstructed view depth. This is a soft intersection, not volumetric fog.

Entity project data stores an optional `material`. Rust callers submit
`WorldCommand::SetMaterial { id, material }`; `Entity.setMaterial(value | null)`
exports that operation through the reflective Rust and JS/TS API. `null` restores
imported per-primitive materials; an override replaces all materials of the
entity. It is not a per-slot override registry. Invalid descriptors roll back
transactionally. Snapshots preserve the descriptor and paths; GPU resources
are reconstructed. No JS heap or device pointer enters saves.

The renderer resolves material textures under the owning client's capability
root, caches by client plus descriptor and prunes unused overrides. A frame
accepts at most 256 distinct entity overrides. Native model materials retain
their existing model cache. Base/emissive maps are sRGB; opacity and data maps
are linear. Raw legacy cube/triangle geometry still has zero UVs: use authored
model UVs for spatial texture detail.

Transparent draws use separate static/skinned and single/two-sided pipelines.
They skip opaque prepass, shadow casting and depth writes, sort far to near by
transformed primitive-bound center in view space, and depth-test against opaque
geometry. Blend equations operate in linear HDR. Premultiplied mode consumes
straight base/color inputs and premultiplies RGB in the fragment shader; additive
uses source-alpha plus destination RGB and preserves destination alpha.

The frame order is opaque/MSAA → opaque TAA → copy resolved HDR → transparent
composition → bloom → tonemap → shared overlays. Transparency samples read-only
opaque depth. The prepass uses a separate frame binding so it never samples its
writable attachment. The alpha-mask shadow fragment uses the visible material's
UV/alpha/opacity cutoff. Transparent surfaces do not cast shadows.

## Import boundary

Standard glTF BLEND/MASK/OPAQUE and `KHR_materials_unlit` retain their own
semantics. Importers may additionally put an engine description in material
extras:

```json
{"extras":{"crysirror_material":{"schema":1,"material":{
  "alpha_mode":"blend", "unlit":true, "depth_fade":4,
  "base_color_factor":[0.4,0.5,0.6,0.5], "opacity_texture":0
}}}}
```

This optional **CrySirror-owned data envelope** replaces the material descriptor;
texture numbers index glTF **images**, not textures or GPU handles. Unknown
schema/fields, invalid scalars and out-of-range image references fail before
texture decoding. It is not a Khronos extension or a UE shader interpreter.

The external UE bridge records serialized local scalar/vector overrides,
including dynamic material instances. `tools/ue_preview.py --material-profile`
accepts an explicit schema-1 map from exact source material paths to shared
material descriptions and required provenance/approximation notes. PNG paths
are relative to that profile's capability directory. Unmatched paths fail.
Game-specific parameter interpretation remains in external profiles, never in
engine crates. See [LOCAL_GAME_IMPORT.md](../LOCAL_GAME_IMPORT.md).

Python project validation, declared asset inventory (including widget material
mutations), MCP validation/replace/trace/capture and generated .d.ts use the same
entity data path. MCP still starts fresh processes, not an in-process live edit
transport. `examples/materials/project.json` demonstrates the feature entirely
as data, without a script or compatibility frontend.

## Compilation validation

On macOS arm64 / Apple M5 / Rust 1.98.1:

- Workspace/all-target check, desktop no-default check and release desktop build
  passed. The no-default configuration retains an existing `unused_mut` warning;
  Cargo reports the existing `block 0.1.6` future-incompatibility warning.
- The external CUE4Parse bridge build passed, with upstream compiler/NuGet warnings.
  It remains developer-only; no .NET dependency was added to the player.

## Behavior validation

- Workspace: **162 passed / 0 failed / 11 ignored**. Ignored = nine GPU tests,
  one caller-font test and one RFVP-game test; these are not passes here.
- Explicit GPU suite: **9 passed** on actual Metal. New pixel checks exercise
  blend ordering, premultiplication, additive, opacity channels, opaque occlusion,
  soft intersection, transparent removal without opaque TAA ghosts, mask shadow
  discard and two clients resolving the same opacity filename independently.
- CPU tests cover glTF alpha/unlit preservation, engine-envelope validation and
  linear opacity decoding, API rollback, snapshots and TS bindings/save/load.
- Python 3.11: **109 passed**, including material/path validation, capability-root
  inventory and explicit transparent import profiles. Fixtures are self-authored.
- Actual MCP stdio `project_validate`, `engine_trace` (66 scoped material entities,
  all five modes) and `scene_capture` succeeded; metadata is in
  `docs/engine-materials-mcp.json`. `dist/engine-materials.png` is the native demo.
- Native castle capture retained all 42 fog-sheet instances: **7,709 nodes /
  13,426,500 submitted triangles / 31 base maps + one self-authored opacity map**.
  Source color/max-opacity/depth-fade parameters feed an explicitly approximate
  unlit material. Only the original sky sphere is omitted. The original UE
  noise/projection/panning graph was not evaluated. All user content stays external.

Logs: `docs/engine-materials-{cargo-check,no-default,release,rust-tests,gpu-tests,python-tests}.txt`.
Metadata: `docs/engine-materials-results.json`.

## Limits

Center sorting does not solve intersecting/self-overlapping transparent triangles;
there is no OIT, refraction, transmission or volumetric lighting. Transparent
composition is single-sample and excluded from TAA history; particle draws keep
their previous pass and are not jointly sorted with transparent meshes. Soft
intersection uses opaque depth only. UV transforms affect shared UV0, not
independent texture coordinate sets. Source UE shaders remain approximations.
No hardware RT, upscaler, GoW mesh decoding, Windows/Linux execution, GPU frame
time or full-game/original-image equivalence is established by this work.


## Addendum (2026-10-25): two-channel normals and specular strength

`Material.normal_encoding` (`rgb` default, `rg` for BC5-style maps, `ag` for
DXT5nm with X in alpha) reconstructs Z in the shader. It applies only when a
normal texture is present, so the default flat-normal texture stays correct.
`specular_factor` and the sRGB `specular_texture` follow KHR_materials_specular:
the texture tints dielectric F0 and the factor weights the whole dielectric
Fresnel (direct BRDF, sun and split-sum IBL including the LUT bias term).
Metals are unaffected. The texture is material binding 8; glTF engine-material
extras, TS/JSON and the Python schema accept both fields. GPU test:
`two_channel_normal_maps_and_specular_strength_change_shading`.
