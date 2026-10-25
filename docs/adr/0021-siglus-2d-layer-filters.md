# 0021: Siglus-style 2D layer filters

Date: 2026-10-25. Status: implemented and locally validated (sprite stages,
distortions, masks, tone curves). Screen-transition wipes are pending.

## Context

The owner asked for Siglus's 2D and shader capabilities in the engine. The
pinned siglus_rs renderer (`render/shaders/sprite.wgsl`, `render_plan`)
applies a fixed per-sprite stack: tr/alpha, tone-curve LUT, reverse, mono,
bright, dark, color rate toward a color, color add, mask texture, gray-as-alpha
mask mode, alpha test, the blend codes 0 to 5, and source distortions
(wipe modes 1 to 6: mosaic, horizontal/vertical raster, explosion blur,
shimi, shimi_inv). CrySirror layers had only opacity, clip, UV and five blends,
so `crysirror-siglus-vm` refused objects using any of these.

## Decision

`LayerSpec.filter: Option<LayerFilter>`, set with `Layers.put` or
`Layers.setFilter` and `LayerCommand::SetFilter`, carries those stages with
Siglus's order and units (rates and channels 0..1 = value/255):

- color: `mono`, `invert`, `bright`, `dark`, `color_rate` + `color`, `add`
- `tone_curve {asset, row, saturation}`: per-channel LUT lookup after mixing
  `saturation` toward mono
- `mask {asset, gray_alpha}`: multiplies color; `gray_alpha` replaces alpha
  with the final gray (mask mode 1)
- `alpha_test`
- `distortion`: `mosaic {cut, aspect}`,
  `raster {vertical, fractions, waves, power, progress}` (with Siglus's
  `raster_amp`), `radial_blur {center, power, coeff}` (the same 10 weights),
  `threshold_fade {fade, threshold, invert}` (luminance of the texture
  sample, as `sample_shimi`)

The renderer keeps the plain fast path. A filtered layer uses a second
pipeline per blend mode, a storage buffer of per-frame filter records indexed
by a flat vertex attribute (index 0 = identity), and a bind group with the
mask and tone-curve textures (white / identity ramp by default). Layer
textures are sRGB, so the stage math converts to gamma space and back to match
Siglus's 8-bit arithmetic. Out-of-texture taps are transparent, as in
`sample_tex0_safe`. CrySirror's multiply/screen equations are algebraically
Siglus's `mix(1,c,a)` / `mix(0,c,a)` forms, so blend codes 0 to 4 map directly.

`crysirror-siglus-vm` now exports mono/reverse/bright/dark/color rate/color
add/alpha test and blend codes 0 to 4 instead of refusing them. Alpha test alone
does not select the filtered path: without depth writes it is invisible for
every blend mode, and it is Siglus's object default. Masks, tone curves, lights,
fog and overlay (code 5) still fail visibly in that export.

## Validation (2026-10-25, Apple M5 / Metal)

GPU test `siglus_style_layer_filters_match_reference_arithmetic` checks the
reference formulas on (128,64,32) within ±3: mono luma weights
0.2989/0.5886/0.1145, invert, bright, dark, color rate, add, an inverting tone
curve (equals invert), gray-as-alpha coverage blended over the background,
threshold fade on texture luminance (red kept, blue faded) and a full-image
mosaic cell. Unit test `siglus_object_effects_map_to_layer_filter_units`
checks the /255 mapping, clamping and the default-object identity. The Python
schema mirrors validation.

## Gaps

Screen transitions (`wipe.wgsl`: mask/crossfade wipes over two images),
overlay blend (needs a destination copy), Siglus per-sprite lights/fog,
`src_clip`, and exporting mask/tone-curve images from the Siglus VM.
