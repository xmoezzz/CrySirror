# 0060: Glossy reflections and per-pixel ray-traced diffuse GI

Date: 2026-10-26. Status: implemented; validation recorded below. Builds on
0055–0059 and uses the same GPU geometry, alpha coverage and material atlas.

## Decision

- `Environment.reflections.ray_traced` traces GGX-distributed reflection rays
  using material roughness from the existing prepass. `ray_samples` (default
  16, 1–64) bounds work; exact mirrors use one ray. The output is a normalized
  NdotL-weighted split-sum prefilter with hit coverage. The forward BRDF LUT
  applies Fresnel/Smith once, as for the prefiltered environment. This is a
  glossy radiance estimate, not a full microfacet path tracer. Native glossy
  reflections survive beyond the old screen-space roughness fade.
- `Environment.ray_traced_gi = { samples: 8, max_distance: 30, intensity: 1 }`
  enables diffuse GI at every opaque/alpha-tested visible pixel. `samples`
  is bounded to 1–64. Cosine-weighted hemisphere rays integrate hit radiance
  and the actual environment on misses. It requires the explicit hardware
  rendering path; it does not silently substitute raster or probe GI.
- **Binding budget:** the existing SSR sampled texture becomes a two-layer
  array: layer 0 is specular radiance/coverage, layer 1 diffuse irradiance.
  Both have separate 2D render/storage views. The PBR stage still uses the
  same 16 sampled-texture bindings. Captures disable both screen contributions
  so reflection and GI probe captures do not recursively sample the screen.
- The shared RT hit-lighting helper reads the existing uploaded `GpuLight`
  buffer, independent of camera cluster lists. It evaluates point/spot,
  rectangle and volume-light attenuation using the engine's light semantics;
  finite emitters use a visibility ray from the bounce point. Reflection,
  per-pixel diffuse and probe GI use this one helper. The existing 4096-light
  upload budget still applies; performance scales with the lights and rays.
- Probe bounce reads use an immutable GPU copy taken before the whole update
  (including all job chunks). Workgroups write the current volume but never
  read each other's in-flight writes. GI history is renderer-owned, transient
  and reused; no GPU handles enter project/save data.
- RT capability negotiation now requests **six** storage buffers per shader
  stage, only after checking the adapter's actual limit. This is reported with
  other RT limits. The raster path retains the downlevel baseline.
- Hit normals use the inverse-transpose transform for nonuniform scales.
  Environment intensity scales the sky radiance; it no longer extinguishes
  absolute local reflection-probe/RT radiance when set to zero.

## Ownership and scheduling

All resources and operations belong to CrySirror and work without a VM. The
frame owner encodes AS preparation, probe refresh, screen-ray computation and
forward sampling in explicit dependency order on the GPU queue. CPU workers
never own a device pointer. No CPU/GPU join was added to interactive rendering;
readbacks and timestamp completion fences are confined to explicit offline
validation/benchmark calls.

## Limits

Per-pixel GI currently estimates one diffuse bounce; the existing probe path
accumulates additional diffuse bounces over sweeps. Offscreen hit shading uses
baked base colour, constant emission, direct sun/local lights and SH ambient;
normal maps, emissive textures, hit specular BSDFs, transmission and blended
surfaces are still outside that profile. The screen rays use geometric normals.
The material atlas remains 128² per tile. There is no dedicated diffuse-GI or
glossy denoiser yet; default TAA may filter their final contribution. The fixed
stratified sequence is stable across captures, with Monte Carlo noise remaining
at low sample counts. Enabling per-pixel GI replaces the visible surface's probe
irradiance rather than adding both and double-counting diffuse energy.

## Validation

CPU/WGSL, native pixel tests and GPU timings are recorded separately below. `crysirror-wgpu/tests/rt_lighting.rs`
contains reproducible native tests and the controlled comparison. GPU tests
remain ignored by default; an ignored test is not a successful hardware result.

The benchmark uses the same 160×90 scene, camera, light and rendering path;
probe modes refresh 16 of 48 probes every measured frame. Raster probes use six
32² face captures per refreshed probe; RT probes use 256 rays per probe, while
per-pixel GI uses 64 rays per pixel and no volume. It measures complete GPU
frames (not isolated GI dispatches), after each mode's own convergence warmup.
Its different representations and sample densities do **not** establish equal
quality or a generally applicable speedup. Cold warmup wall time is reported
separately and is not interactive FPS. Real shader/device/sample diagnostics
and p50/p95 are retained in the ignored JSON artifact.

### Actual evidence (Apple M5 / Metal)

- Implementation compilation: `cargo check -p crysirror-wgpu --tests` passed.
  The WGSL suite passed; the regular library run passed 26 tests, with two
  hardware/font tests ignored at that CPU-only stage. Subsequent full GPU
  execution explicitly ran those tests rather than counting the ignores.
- Behaviour: all **46** ignored wgpu GPU tests passed in one serialized run
  with `CRYSIRROR_TEST_FONT=/System/Library/Fonts/Supplemental/Arial.ttf`.
  The earlier run without that environment variable failed the font-dependent
  test; that is a failed setup run, not a passing skip. New lighting tests
  passed 3/3. Sun, environment and emission were zero in the local-light
  bounce fixture: floor red-channel excess was 0 without GI, 29.67 with
  ray-traced probes, 25.11 with per-pixel GI, and 0 with the light disabled.
  Per-pixel GI was exercised with no reflection setting or probe volume;
  requesting it on the raster path failed visibly.
- Behaviour: a roughness-0.65 floor reflected the offscreen emissive object
  outside the sharp mirror footprint; the explicit GGX test measured 10,829
  newly coloured pixels with 64 reflection rays. Reflection radiance also
  remained visible with environment intensity zero. Both images were saved.
- Performance: final serialized-suite run, 24 valid completed GPU frame
  measurements per mode. Mean/p50/p95 in milliseconds were raster probes
  **7.578 / 7.712 / 10.512**, RT probes **0.810 / 0.776 / 0.974**, and
  per-pixel GI **0.644 / 0.625 / 0.893**. No failed or pending timing samples
  remained. RT-probe warmup overflowed the four-slot timing ring three times;
  those dropped warmup samples are reported, not fabricated as measurements.
  All 24 measured steady frames per mode yielded valid samples. These values
  are full-frame timings for this tiny controlled scene, not equal-quality
  comparisons, isolated GI timings, or results for other vendors/platforms.

Evidence stays ignored in `target/verification/rt0060-*.log` and
`target/verification/rt0060/{gi-benchmark.json,local-*.png,gloss-*.png}`. The
benchmark JSON includes the actual negotiated adapter/device limits and all
sample diagnostics. No proprietary assets or fonts were committed.
