# 0023: Screen-space reflections

Date: 2026-10-25. Status: implemented and locally validated.

## Context

The PBR path's specular ambient term came only from the prefiltered
environment map (ADR 0010). Interiors such as the OLY100 hall or the Ashen Vow
castle reflect the sky instead of their own walls, which is the most visible
remaining difference from UE-class renderers on glossy floors, metal and
water. Reflection captures and Lumen-style GI are larger efforts. SSR is the
standard first layer and needs only buffers the frame already has.

## Decision

`Environment.reflections: {intensity=1, max_distance=30 m, thickness=0.3 m,
max_roughness=0.6, steps=48}` (absent = off; validated, mirrored in
`tools/engine_schema.py`).

- **Pass.** Runs after the depth/normal prepass and SSAO, before the forward
  pass (`shaders/ssr.wgsl`).
  - A mirror ray from the prepass view position and normal is clipped to the
    near plane and marched in screen space, interpolating 1/w for
    perspective-correct depth.
  - A crossing behind the depth buffer is refined by bisection, and only then
    tested against `thickness`.
  - Crossings within 1.5 px of the origin, or on surfaces whose normal faces
    along the ray, are skipped and marching continues. This removes
    self-intersection of the start surface at grazing angles, which was
    observed as hatching before the fix.
- **Radiance.** Taken from the *previous* frame's final HDR color (after
  transparency, before bloom/tonemap), reprojected with
  `prev_view_proj · view⁻¹`. Confidence fades at screen edges, with world
  distance, and for camera-facing rays.
- **Shading.** The forward shader replaces the prefiltered environment
  radiance by the SSR radiance, weighted by confidence × intensity ×
  `1 − smoothstep(max_roughness/2, max_roughness, roughness)`. The split-sum
  Fresnel/BRDF weighting still applies, so energy matches the IBL term it
  replaces. Opaque materials only.
- A capture without TAA renders two frames when SSR is enabled, because the
  first frame has no previous color. The frame bind group gains binding 14,
  the eighth sampled texture of the frame group.

## Validation (2026-10-25, Apple M5 / Metal)

- GPU `screen_space_reflections_show_the_object_in_a_mirror_floor`: a
  metallic roughness-0.02 floor and an unlit red box.
  - With SSR, the floor point mirroring the box is red; without SSR it shows
    the sky.
  - Floor beside the box is unchanged within ±12, so there are no false hits.
  - Captures: `dist/ssr-test.png`, `dist/ssr-off-test.png`.
- The full explicit GPU suite passes after the frame layout change, including
  developer surface shaders, which share the frame group.
- Python schema mirror cases (defaults, bounds, unknown field).
- Real content: OLY100 hall with SSR at the import's roughness of 0.6 changes
  no pixel by more than 8/255. That is correct behaviour: the GoW material
  profile sets roughness 0.6 for all materials (an approximation, see
  ADR 0017), which equals the default fade-out. With `max_roughness: 1`, 33%
  of pixels change by more than 8/255 (external cache `hall-ssr-r1.png`).
  This shows the pass runs on real geometry, not that the result matches the
  original game.

## Gaps

- Glossy SSR (cone tracing or a blurred color pyramid). Rough surfaces fade
  to the environment instead.
- Hierarchical-Z tracing; no measured frame cost yet.
- Reflections of transparent surfaces.
- Local reflection captures: done in ADR 0024.
- Native GoW gloss values, which are not decoded.
