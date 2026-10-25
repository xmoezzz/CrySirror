# 0018: Clustered local lights

Date: 2026-10-25. Status: implemented and locally validated (point, spot,
rect, directional/ambient volumes; budgeted point/spot shadows).

## Context

Until now the PBR path had one directional sun (cascaded shadows), image-based
lighting and SSAO. Imported content looked flat because interiors and night
scenes depend on many local lights: the Ashen Vow levels extracted by
`ue-import level` contain 201 (HauntedVillage) and 285 (FantasyCastle) light
components, and the ADR 0017 capability audit also listed local lights as a
gap. Lights must be ordinary engine data reachable from the OOP/script API,
not an importer-specific path.

## Decision

`LightSpec` is an entity component (`EntitySpec.light`, `WorldCommand::SetLight`).
Position and orientation come from the entity's world transform; spot and
rect lights emit along local -Z (the glTF convention).

| Field | Meaning |
|---|---|
| `shape` | `{kind:"point"}`, `{kind:"spot", inner_degrees, outer_degrees ≤ 89}`, `{kind:"rect", size:[w,h]}`, `{kind:"directional"}` (parallel light confined to the range sphere), `{kind:"ambient"}` (uniform diffuse irradiance inside the range sphere) |
| `color` | linear RGB |
| `intensity` + `units` | candela (native) or lumens: point/spot lm/4π (cone-independent, as in UE), rect lm/π |
| `range` | attenuation radius; influence ends smoothly at it |
| `falloff` | `{kind:"inverse_square"}` (default) or `{kind:"exponent", exponent}`: `(1 − d²/r²)^n` without 1/d², as UE's non-inverse-squared lights |
| `shadows`, `enabled` | point/spot shadows from the shared atlas (budgeted); other shapes ignore the flag |

Validation rejects non-finite values, inverted cones, unknown fields and
shapes; invalid lights fail visibly rather than being clamped.

Renderer: CPU-built clusters (16×9 screen tiles × 24 logarithmic depth slices).
Each light's range sphere is assigned conservatively (exact depth range,
NDC bounds from the sphere extremes). Up to 4096 lights are uploaded (nearest
first) and 256 per cluster; anything dropped is counted in
`PbrRenderer::light_stats()`. Three read-only fragment storage buffers fit the
`downlevel_defaults` limits already requested. The forward shader evaluates the
same GGX/Smith/Schlick BRDF as the sun with inverse-square falloff and the
`(1 − (d/r)⁴)²` window. Spots use a smooth squared cone, and rect lights use
the closest point on the rectangle with one-sided Lambertian emission.

glTF `KHR_lights_punctual` point/spot lights import as entity lights (candela
in both). Without a glTF range, the range ends at 0.01 lux. Directional glTF
lights are reported in `ImportReport.dropped` (use the environment sun).

UE levels: `ue-import level` exports light components and keeps absent
(class-default) and zero-masked fields distinct. `tools/ue_lights.py` converts
them: cm/Z-up/LH → m/Y-up/RH as (x, z, y)·0.01; local +X → -Z rotation; sRGB
`LightColor` → linear. A missing IntensityUnits is the class default
(Unitless); 103 castle lights serialize `Candelas` explicitly, so the default is
not Candelas. Unitless converts to candela by 1/625, and lumens as above.
Directional lights become the sun; sky lights are reported as skipped.

Directional and ambient volumes take illuminance (lux) and reject lumens.
They exist because older engines author localized light rigs and ambient
volumes. GoW III's OLY100 has 87 radius-limited directional and 17 ambient
records, and faking them in an importer would lose data.

GoW III: `tools/gow3_lights.py` decodes the 156-byte version-6 light records
(kind 0x00010017). The type is bits 0-1 of the flags (0 ambient, 1 point,
2 directional); bits 2-5 and the high flags are reported, not interpreted.
All **207** OLY100 records convert (103 point, 87 directional, 17 ambient).
The native falloff curve and photometric units are not decoded. The
conversion uses explicit approximations (exponent-2 falloff, color×intensity as
illuminance, ambient ×π) recorded in its report; `original_lighting` stays false.

## Validation (2026-10-25, Apple M5 / Metal)

- Unit: cluster assignment, culling, row orientation and per-cluster budget
  (`crysirror-wgpu` lib); glTF punctual import; Python schema mirror and UE
  conversion (units, defaults, explicit zeros, axis/rotation math).
- Engine: `tests/lights.rs`: spawn/setLight/null removal, lumens→candela,
  parented spot direction, visible rejection of invalid lights, generated `.d.ts`.
- GPU: `clustered_point_and_spot_lights_light_only_their_regions`: dark scene,
  red point and blue spot reach their pixels, cut off outside the cone and beyond
  range. The full explicit GPU suite passed after the frame-layout change.
- GPU: `directional_and_ambient_volumes_and_exponent_falloff_stay_inside_their_ranges`.
- Python: synthetic GoW light records (type bits, units, volume shapes,
  rotation, visible failure on unknown version/type/size/radius).
- Real content: OLY100 rendered with its 207 converted lights through the
  data-driven preview (`interior-native-lights.png`, external cache).
- Real content: L_FantasyCastle with its 284 converted lights rendered through
  the scripted API (captures stay in `~/.cache/crysirror/ashen/castle-lit`).
  This shows the lights are applied, not that the image matches the original.

## Local-light shadows

A 4096² depth atlas holds 64 tiles of 512². Each frame the nearest
shadow-casting lights get tiles first: a spot takes one (perspective with
fov 2·outer + 4°), and a point light takes six cube faces (+X, −X, +Y, −Y,
+Z, −Z). A light that does not fit is not shadowed and is counted in
`local_shadow_stats()` (requested/granted/tiles). Casters are culled per tile
against the light's range sphere and reuse the sun's shadow pipeline (masked
materials still clip). The shader selects the cube face by major axis, applies
a ~1.5-texel normal offset and 3×3 PCF clamped inside the tile. Tile matrices
are the fourth fragment storage buffer, within `downlevel_defaults`.
Validation: `allocate_shadows` budget/face-orientation unit test and GPU test
`local_point_and_spot_shadows_darken_only_occluded_floor` (umbra under a blocker,
unchanged open floor). The castle hall was also re-rendered with UE `CastShadows` lights.

## Gaps (not implemented)

Rect/volume-light shadows, shadow caching for static lights, rect-light LTC/area
specular, light functions/IES profiles, sky light capture, auto exposure,
volumetric/height fog, GPU (compute) clustering and frame-time measurements.
