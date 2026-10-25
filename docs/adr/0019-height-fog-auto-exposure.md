# 0019: Exponential height fog and auto exposure

Date: 2026-10-25. Status: implemented and locally validated.

## Context

Imported AAA scenes depend on atmosphere and eye adaptation. Night interiors
(Ashen Vow castle, UE auto exposure) and hazy halls (GoW III) looked wrong with
a fixed exposure and no aerial perspective. Both belong to the shared
`Environment`, not to an importer.

## Decision

`Environment.fog: Option<Fog>` follows the exponential-height-fog model: the
density at `base_height` decays as e^(−height_falloff·Δh), and the opacity is
1 − e^(−∫density) along the camera ray (closed-form line integral, with a
straight-line limit for tiny falloff). `start_distance` and `max_opacity` match
the usual artist controls. `sun_scattering`/`sun_exponent` add the sun color
toward the sun direction. The forward PBR shader fogs lit surfaces, and the sky
is fogged at the camera far plane, so distant sky obeys `max_opacity`. Unlit
materials and particles are not fogged yet.

`Environment.auto_exposure: Option<AutoExposure>` meters the resolved HDR frame
(32×32 grid, center-weighted mean of log2 luminance) in a fullscreen pass into
a ping-pong 1×1 R32Float state. It adapts toward the target clamped to
[`min_log2`, `max_log2`] at `speed_up`/`speed_down` per second, using world
time between frames. The first frame snaps, so offline captures are
deterministic. The tonemapper scales by 0.18·2^compensation / 2^adapted on top
of `exposure`.

Rust, TS/JS (`World.setEnvironment`) and the Python schema validate both;
unknown fields and out-of-range values fail.

## Validation (2026-10-25, Apple M5 / Metal)

GPU test `height_fog_and_auto_exposure_reach_pixels`: near ground keeps its
color while the horizon turns fully into the fog color; a scene lit at 1/64
brightness averages < 20 (green channel) with fixed exposure and > 60 with auto
exposure. Real-content captures (external cache) for the castle hall and the
OLY100 hall. The GoW fog there is **authored for the preview**; native GoW fog
records are not decoded.

## Gaps

Volumetric fog/froxels, light shafts, fog on particles/unlit, a histogram
meter with percentile clipping, and local exposure.
