# 0028: Dynamic diffuse GI with rasterized irradiance probes (step 1)

Date: 2026-10-25. Status: implemented and locally validated, including
step 2 (visibility, probe classification and cheaper refreshes, below).
Lumen-style surface caches and SDF tracing are not implemented.

## Context

Indirect diffuse light came only from the global environment SH. That is
wrong in every interior: emissive surfaces and local lights do not light
their surroundings, and a room is lit by a sky it cannot see. UE5 solves this
with Lumen. Our first step is an independently implemented, dynamically
refreshed irradiance volume. It reuses the reflection-capture rendering
(ADR 0024), needs no ray tracing hardware, and adds no bind group slots.

## Decision

- **Component.** `EntitySpec.irradiance_volume = {extent, counts = [8,4,8],
  intensity = 1, probes_per_frame = 8, revision}`, at most 4096 probes.
  - Commands and script: `WorldCommand::SetIrradianceVolume`, spawn field
    `irradianceVolume`, `Entity.setIrradianceVolume`, property
    `irradianceVolume`.
  - Validated, and mirrored in `tools/engine_schema.py`.
  - The renderer uses the first volume by key.
- **Probes.**
  - A regular grid including the box corners.
  - Each frame `probes_per_frame` probes are refreshed round-robin. Each one
    renders the six 128² capture faces with the full forward shading: local
    lights, emissive, sky and the volume itself. Because each refresh reads
    the previous result, bounces accumulate.
  - `gi_project.wgsl` projects the faces (every 4th texel, weighted by solid
    angle) onto L1 SH and convolves them to irradiance (A0 = π, A1 = 2π/3).
- **Storage.** A third section of the binding-12 shadow-data buffer, after
  the tile matrices and the VSM page table: four vec4 per probe (r, g, b and
  a valid flag). It is cleared when the volume or its revision changes.
- **Shading.** `gi_irradiance` uses trilinear interpolation over valid
  probes with a backface weight `((dot(dir,n)+1)/2)² + 0.2`. Positions are
  clamped into the volume, so walls and floors on its boundary get GI. The
  result fades to the environment SH over one cell outside the volume. The
  volume holds absolute irradiance, so the environment intensity scales only
  the fallback and the specular IBL term.
- Captures render at least 3 refreshes per probe, capped at 64 frames.

## Validation (2026-10-25, Apple M5 / Metal)

- GPU `irradiance_volume_bounces_emissive_light_onto_the_room`: a closed
  white room with one red emissive wall, no sun and no sky.
  - Without the volume the floor is black (0, 0, 0).
  - With a 6×3×6 volume the floor is lit red, with a falloff away from the
    wall (`dist/gi-test.png`).
  - The first attempt left the floor black. The floor lies on the volume
    boundary, and the original inner-edge fade returned the fallback there.
    Boundary clamping and the outer fade fixed it.
- The engine API coverage test includes the new command. The full explicit
  GPU suite passes.
- Real content: the OLY100 hall with an 8×4×8 volume and 64 probes per frame
  takes 46 s for a 16-frame headless capture (1,024 probe refreshes of the
  ~1M-triangle scene), so the cost is dominated by the refreshes. With auto
  exposure on, 1.4% of pixels change by more than 8/255. This is a smoke run
  on real content, not a fidelity claim.

## Gaps after step 1

- Visibility (depth moments) against light leaking through thin walls, and
  probe relocation out of geometry.
- In captures the sun is shadowed only where the camera requested VSM pages
  or cascades cover the probe, so sun light can leak indoors.
- Cheaper refreshes: lower-resolution faces, coarser LOD cuts per probe,
  and prioritizing probes near the camera or near changes.
- Multiple and cascaded volumes; specular GI from the volume.
- A Lumen-style surface cache with SDF or hardware ray tracing.

## Step 2: visibility, classification, refresh cost (2026-10-25)

- **Capture output.** Irradiance captures set `misc.w`.
  - Opaque surfaces then write the hit distance to alpha, negative for back
    faces; back faces write no radiance.
  - The sky writes 0, which counts as a miss.
  - Captures draw every opaque primitive double-sided so that back faces
    become visible. A legitimately double-sided material seen from behind
    therefore also counts as a back face.
- **Visibility.** `gi_project.wgsl` stores 8×8 octahedral distance moments
  per probe: (mean, mean²) over 4×4 directions per texel. Misses and
  distances are clamped to twice the cell diagonal. The data is in a fourth
  binding-12 section after the probe states (`GI_VIS`, 512 B per probe).
- **Shading.** `gi_irradiance` biases the point by
  `(0.2·n + 0.8·v)·0.3·min cell` and samples the moments bilinearly, with
  octahedral border wrapping. It multiplies the probe weight by the cubed
  Chebyshev bound when the point is farther than the mean. It then crushes
  weights below 0.2, as in DDGI. `IrradianceVolumeSpec.visibility = false`
  restores step 1.
- **Classification.** Probes whose solid-angle-weighted back-face fraction
  exceeds 0.25 are marked unusable and skipped by the interpolation. Probes
  are not relocated.
- **Cost.**
  - Irradiance faces are 32² (`probes::GI_FACE`), down from 128².
  - Draws whose bounding sphere is outside a 90° face frustum are skipped.
  - Each sweep refreshes probes in order of distance to the camera.
  - After three complete sweeps without a change to lights, static object
    transforms, material overrides, the sun or the environment, the rate
    drops to `idle_probes_per_frame` (default 0). A sweep that was in
    progress during a change does not count. Skinned meshes are excluded
    from the change hash.
  - The capture MSAA targets now store instead of discarding. With
    Discard, wgpu 0.20's per-encoder discard tracking rescanned its growing
    list on every pass, so capture passes cost O(n²) per frame. Sampling
    the OLY100 run found this was the main CPU cost.
- `PbrRenderer::gi_stats()` (probes, refreshed, totals, sweeps, converged)
  is reported by the desktop `render` JSON. `debug_gi_probe_states()` reads
  the per-probe states back for tests.
- Capture warm-up is now the maximum of the TAA, SSR, GI and VSM needs. It
  was previously the first matching rule, which let TAA cap GI at 16
  frames.

### Validation (2026-10-25, Apple M5 / Metal)

- GPU `irradiance_visibility_stops_light_leaking_through_a_wall_and_disables_buried_probes`.
  The scene has two closed rooms split by a 0.2 m wall, with a red emissive
  wall in the left room only. Mean red of the dark-room view:

  | Setting | Mean red |
  |---|---|
  | No volume | 0.00 |
  | `visibility: false` | 78.2 |
  | Visibility on | 4.3 |

  - The lit room keeps its bounce light (mean red > 30).
  - Exactly the three probes buried in a pillar are disabled (back-face
    fractions 1.0, 1.0 and 0.975).
  - Captures: `dist/gi-leak-off-test.png`, `dist/gi-leak-on-test.png` and
    `dist/gi-leak-lit-test.png`.
- GPU `irradiance_volume_idles_when_converged_and_restarts_after_a_change`.
  - After convergence a second capture refreshes 0 probes and reproduces
    the image exactly.
  - Moving the emissive wall restarts at least three sweeps and changes the
    image.
- The step-1 room test still passes with visibility on.
- OLY100 hall, the same `hall-gi.ts` as step 1 (8×4×8 probes, 64 per frame,
  TAA):
  - The headless capture drops from 46 s to 8.8 s; without the volume it
    takes 2.6 s.
  - 768 refreshes (three sweeps), then the volume is converged and the last
    frames refresh none.
  - Before the Store change the same run took 13.7 s; the two images differ
    in 0.3% of pixels.
  - About 8 ms per refresh remains (six 32² faces of a ~1M-triangle scene
    with ~200 local lights). In the samples the CPU mostly waits on the GPU.
    Per-pass GPU timing was not measured.

### Remaining

- Probe relocation; back-face handling for double-sided materials.
- Sun shadows in captures (see above).
- Per-probe coarse LOD cuts. Captures still draw the camera's unculled
  cluster cut, which is the main remaining cost.
- Surface cache, SDF or hardware tracing; specular GI.
