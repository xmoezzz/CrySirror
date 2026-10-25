# 0025: Authored reflection probes and GoW III environment cubemaps

Date: 2026-10-25. Status: implemented and locally validated.

## Context

GoW III materials bind a texture at sampler slot 22 that ADR 0017 reported as
unmapped. Inspecting the OLY100 WAD (tool output, not guessed) shows:
- 4 distinct slot-22 textures across 384 material bindings. One texture
  serves 339 materials, one serves 36, and the last two serve 5 and 4.
- All four have DDS caps2 0xFE00 (cubemap, all six faces), 128², DXT5,
  8 mips inline. The payload equals six face-major mip chains
  (6 × 21872 + 204 header = 131436 bytes).
- The dominant cube shows the OLY100 hall itself: dome, statue, and the
  round floor with its blue centre.
- Decoded alpha is 0 everywhere and is not interpreted.

## Decision

- **Authored probes.** `ReflectionProbeSpec` gains `source` (a relative
  equirect Radiance `.hdr` asset) and `parallax` (default true; false gives
  infinitely distant reflections). A sourced probe skips the engine capture.
  Its image gets the same box-filtered mip chain and GGX prefilter as the
  environment (shared `equirect_levels`/`upload_levels`, refactored out of
  the environment loader). Slot bookkeeping keys sourced probes by
  `source:<client>:<asset>`.
- **`tools/gow3_cubemaps.py`.**
  - Splits each slot-22 cube into six DDS faces, normalizing the native
    header flags as `dds_preview` does, and decodes them with the engine
    codec (`transcode`).
  - Verifies the D3D face order and orientation from content: the mean
    colour difference across all 12 cube edges must be below 2/3 of the best
    of 20 permuted face orders. Otherwise it refuses.
  - Writes a 512×256 equirect `.hdr` with the engine prefilter's direction
    convention, treating colour as sRGB LDR.
  - A global mirror of the whole cube is not detectable by seams and is
    reported as unverified.
  - Output stays outside the repository.

## Results (OLY100)

| Texture | Materials | Seam error (D3D / best permutation) | Result |
|---|---|---|---|
| TX739B54CE | 339 | 19.5 / 38.4 | converted |
| TX7993D701 | 36 | 0.73 / 4.18 | converted |
| TX47C1E6DD | 4 | 1.30 / 11.6 | converted |
| TX93C2CE91 | 5 | 15.5 / 20.8 | refused (margin too small) |

The hall rendered with the dominant cube as a non-parallax probe changes 31%
of pixels by more than 8/255 relative to the sky-only render. The engine's
own capture (ADR 0024) of the same hall differs from the native cube render
in only 0.7% of pixels at the import's roughness of 0.6. These are
independent sources that agree. Per-material cube assignment is not used:
the dominant cube is applied globally and the three minor cubes (45 material
bindings) are reported.

## Validation (2026-10-25, Apple M5 / Metal)

- GPU `authored_equirect_probe_without_parallax_uses_the_prefilter_direction_convention`:
  a synthetic `.hdr` that is red only around −Z. A mirror floor viewed toward
  −Z reflects red; viewed toward +X it reflects blue.
- Python `tests/test_gow3_cubemaps.py` (synthetic only):
  - The seam check accepts a correct cube and rejects swapped faces.
  - The equirect conversion uses the engine's direction convention.
  - The face split checks size and caps and fails visibly on bad layouts.
- Schema mirror for `source` and `parallax`.

## Gaps

- Per-material cubemap selection (a material-level reflection override).
- The capture positions of the native cubes are unknown, so no box
  parallax.
- The alpha channel and possible HDR scale of native cubes.
- `TX93C2CE91` is not converted.
