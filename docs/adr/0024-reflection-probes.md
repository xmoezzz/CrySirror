# 0024: Box-projected reflection probes

Date: 2026-10-25. Status: implemented and locally validated.

## Context

After ADR 0023, specular reflections that SSR cannot resolve (off-screen,
occluded, or rough) still fell back to the global sky. Indoors that is wrong
everywhere: the OLY100 hall and the Ashen Vow castle reflected a blue sky on
stone and metal. UE handles this with box/sphere reflection captures, and
GoW III ships per-area environment cubemaps (sampler slot 22, not decoded
yet). Either way, the engine needs local, parallax-corrected radiance maps.

## Decision

- **Component.** `EntitySpec.reflection_probe = {extent:[hx,hy,hz],
  intensity=1, blend_distance=0.5, revision=0}`. Command
  `WorldCommand::SetReflectionProbe`. Script: spawn field `reflectionProbe`,
  `Entity.setReflectionProbe`, property `reflectionProbe`. The capture origin
  is the entity's world position, and the box is world-axis aligned. Bumping
  `revision` requests a recapture. Validated, and mirrored in
  `tools/engine_schema.py`.
- **Slots.** Up to 8 probes (nearest to the camera) occupy slots of a
  256×128 equirect `D2Array` with the environment's 7 prefilter mips. A probe
  keeps its slot across frames. A slot is (re)captured when key, position,
  revision or environment change, so static probes are captured once.
  Counts are in `PbrRenderer::probe_stats()`.
- **Capture.** In the frame's encoder, after the shadow passes:
  - Six 128² faces of the opaque scene plus sky, with the existing surface
    pipelines at 4× MSAA.
  - Each face gets its own frame uniform and its own light clusters, so local
    lights and local shadows apply. AO, SSR and probes are off inside
    captures, and sun cascades are the main view's.
  - Faces use explicit forward/up bases (`probes::FACES`). `probe.wgsl` maps
    them to equirect with the same bases. This avoids cube-map handedness
    conventions, which would flip triangle winding.
  - The equirect then gets a box-filtered mip chain and the same GGX
    prefilter as the environment.
- **Shading.** For a point inside a box, the reflection ray is intersected
  with the box. The direction from the capture origin to that hit samples the
  probe at the roughness mip. Weights fade in over `blend_distance` from the
  box faces. Overlapping probes are weight-normalized, and the uncovered
  remainder keeps the environment. SSR (ADR 0023) is applied on top. The
  frame group gains binding 15, its ninth sampled texture (9 + 7 material =
  16, the downlevel limit).

## Validation (2026-10-25, Apple M5 / Metal)

- GPU `box_reflection_probes_capture_offscreen_walls_in_the_right_directions`:
  red/green/blue unlit walls on the front/left/right box faces, all
  off-screen, and a metallic roughness-0.05 floor.
  - Looking down toward each wall, the floor centre shows that wall's colour
    only with the probe; without it, it shows sky.
  - The side views show the front wall on the correct side, so face bases are
    not mirrored or swapped (`dist/probe-test-*.png`).
- The full explicit GPU suite passes (20) after the frame layout change.
- Engine `script_reflection_probes_extract_world_positions_and_validate`:
  parented world position, invalid extent and unknown field rejected,
  revision bump, null removal. API coverage includes the new command.
- Python mirror cases in `tests/test_lights.py`.
- Real content: OLY100 hall with one 140×90×140 m probe changes 40% of pixels
  by more than 8/255 against the sky-only render. The sky tint on stone is
  replaced by captured hall radiance (external cache `hall-probe.png`). There
  is no reference image of the original game for comparison.

## Gaps

- Sphere-shaped influence and rotated boxes.
- Diffuse irradiance from probes; the SH comes from the global environment.
- Time-sliced or realtime recapture (moving lights need a revision bump).
- Sun cascades fitted per capture.
- Decoding GoW III slot-22 cubemaps as authored probe sources.
- Frame-time measurements.
