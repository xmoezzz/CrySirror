# 0057: Ray-traced point/spot light shadows

Date: 2026-10-26. Status: implemented (Metal validated). Builds on ADR
0055/0056.

## Context

Local lights used only the raster shadow atlas (ADR 0018). The PBR fragment
stage already binds 16 sampled textures, the downlevel default, so a new
per-light visibility texture would break devices at that limit. The RT
visibility target (RGBA16F) used R for AO and G for the sun.

## Decision

- With hardware RT, the **nearest two** shadow-casting point/spot lights
  (same nearest-first order as atlas allocation) are traced into B and A of
  the existing visibility target. The PBR shader reads them when
  `frame.rt_shadow.y` is set and the light's `cone.w` names a channel;
  other shadowed lights keep atlas shadows. Atlas tiles stay allocated for
  the RT lights because probe captures and transparent surfaces do not
  sample the main view's visibility.
- Shadow rays go from the surface to the emitter and stop short of it by
  its radius (+1 cm), so emitter proxy meshes never shadow their own light;
  alpha-tested re-tracing (ADR 0055) applies.
- `LightSpec.source_radius` (default 0) makes the light an emitter disc
  facing the surface; rays sample it with the same per-pixel/frame PCG
  sequence and are denoised by the ADR 0056 denoiser, now three channels
  (sun, light 0, light 1) with view depth in its own R32F history and only
  stochastic channels filtered. A radius of 0 is one exact ray.
- Offline captures warm up 16 frames when any soft RT channel is active.

## Limits

Two traced local lights per frame (binding budget); rect lights keep their
unshadowed behaviour; transparent surfaces read the opaque pixel's
visibility, as for the sun.

## Evidence (Apple M5, Metal)

`pbr_hardware_point_light_shadows_hard_and_soft`: a traced point-light
shadow darkens 1389 floor pixels; a 0.35 m emitter reduces sharp floor
shadow edges 119 → 79. Whole `crysirror-wgpu` ignored GPU suite,
`--test-threads=1`: 41 passed.
