# 0059: Ray-traced irradiance probe updates

Date: 2026-10-26. Status: implemented (Metal validated). Builds on ADR 0028
(irradiance volume) and ADR 0058 (material atlas, corner attributes).

## Context

ADR 0028 refreshes each probe with six raster face captures and a GPU SH
projection. A GI texture of its own for per-pixel ray-traced GI would
exceed the PBR stage's 16 sampled-texture budget; the irradiance volume's
storage section is already read by the forward pass.

## Decision

`IrradianceVolumeSpec.ray_traced` (default false) switches probe refreshes on
the hardware RT path to a compute pass after this frame's RT scene is built:

- one workgroup per probe, one invocation per 8×8 octahedral texel, four
  jittered rays per texel (256 rays per probe), solid-angle weighted;
- hits are lit by the sun with a ray-traced shadow, the material's constant
  emission and the volume's current irradiance at the hit (trilinear SH,
  multi-bounce over successive updates); back faces, judged by the shading
  normal so no backend winding convention matters, count towards disabling
  probes inside geometry exactly as in ADR 0028; misses sample the
  environment map;
- results are written into the same SH L1 and distance-moment sections the
  raster path writes, blended with hysteresis 0.85 after a probe's first
  update, so the forward pass, Chebyshev visibility and scheduling are
  unchanged. Ray-traced volumes count as converged after 12 sweeps.
- Opt-in because hit shading omits local lights, which raster captures
  include.

## Limits

Local lights, specular and emissive textures are not part of the bounce;
updates of probes in one dispatch read each other's previous values without
ping-pong; no per-pixel RT GI (binding budget).

## Evidence (Apple M5, Metal)

`pbr_hardware_ray_traced_irradiance_probes_bounce_emissive_light`: in a
closed room lit only by a red emissive wall the floor stays black without
GI, is lit to (245, 60, 60) by raster probe captures and to (211, 50, 50)
by ray-traced updates (1296 probe updates). Whole `crysirror-wgpu` ignored
GPU suite, `--test-threads=1`: 43 passed. Relative cost was not measured.
