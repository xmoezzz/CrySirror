# 0058: Ray-traced reflections; material atlas and corner attributes

Date: 2026-10-26. Status: implemented (Metal validated). Builds on ADR
0055-0057.

## Context

Hit shading needs the hit's surface colour and normal, but wgpu offers no
bindless texture/buffer access here and the PBR fragment stage is at the
16 sampled-texture budget.

## Decision

1. **Material atlas.** Every opaque or alpha-tested material that reaches
   the RT scene is baked once into a 128² tile of a 2048² RGBA8 sRGB atlas
   by the material's own bind group and the raster surface function: linear
   base colour in RGB, coverage in A. Slots carry the alpha cutoff, a mask
   flag and the material's constant emission (emissive textures are not
   baked). Effect presets bake at the origin at time 0 (approximate hit
   colour; alpha-tested effects still fail visibly).
2. **Corner attributes.** For all RT geometry a GPU pass gathers, per
   triangle corner, the UV (2×f16) and the octahedral normal (2×unorm16):
   8 bytes per corner, bounded by the RT primitive budget.
3. **Shared prelude** `rt_common.wgsl`: instance table, attribute decoding,
   atlas lookup, `visible()` and `closest()` with alpha-tested re-tracing.
   Every query uses the closest-hit flag; the earlier shadow query used
   terminate-on-first-hit, which could skip a nearer opaque occluder behind
   a transparent texel, and is corrected here.
4. **Reflections** (`ScreenSpaceReflections.ray_traced`). On the hardware
   path a compute pass traces the mirror ray from the prepass up to
   `max_distance` and writes the SSR target (rgb, a = hit), so the forward
   pass and its roughness fade are unchanged:
   - a hit visible in this frame's depth (2% distance tolerance) reuses the
     previous frame's HDR colour at the hit's reprojected position;
   - otherwise the hit is shaded from the atlas colour, the sun with a
     ray-traced shadow, SH environment irradiance and constant emission;
   - a miss leaves alpha 0 (environment and probes).
   Other paths, or a scene without RT geometry, use screen-space marching.

## Limits

Mirror reflections only (glossy lobes are faded to the environment as for
SSR); off-screen hits omit local lights, GI, normal maps, specular and
emissive textures; transparent surfaces are not traced.

## Evidence (Apple M5, Metal)

`pbr_hardware_reflections_show_offscreen_objects`: a box above the frame
appears in a glossy floor with ray-traced reflections (2348 red pixels)
and not with SSR or without reflections (0). Whole `crysirror-wgpu`
ignored GPU suite, `--test-threads=1`: 42 passed.
