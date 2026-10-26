# 0056: Soft ray-traced sun shadows with temporal/spatial denoising

Date: 2026-10-26. Status: implemented (Metal validated). Builds on ADR 0055.

## Decision

- `Sun.angular_diameter_degrees` (default 0) describes the sun disc. When it
  is positive the hardware visibility pass traces `SOFT_SAMPLES` (2) rays
  per pixel and frame, uniformly inside the disc's cone, decorrelated by a
  PCG hash of pixel, frame and sample. Every ray keeps the alpha-tested
  re-trace of ADR 0055. A diameter of 0 keeps the exact single hard ray and
  bypasses denoising, so earlier output is unchanged.
- **Temporal accumulation.** The current world position is reprojected with
  the previous frame's view-projection; the history (visibility, view depth,
  sample count) is accepted when the stored view depth matches the expected
  previous view depth within 3%, and blended with weight 1/n up to
  `MAX_HISTORY` (32) samples. Disocclusions restart at one sample.
- **Spatial filter.** A 5×5 edge-aware filter weighted by distance, view
  depth (2% relative) and view-space normal (cos¹⁶); its step doubles while
  the history holds fewer than 4 samples.
- History targets are owned by the RT service, recreated on resize, and
  never saved. Offline captures render at least 16 warm-up frames when soft
  RT shadows are active.

## Limits

Directional sun only. Reprojection uses the jittered prepass and nearest
history texels; fast-moving casters lag by the history length (rejected
only by the depth test). No variance guidance, no recurrent blur yet; local
lights are the next step.

## Evidence (Apple M5, Metal)

- `pbr_hardware_soft_sun_shadows_have_penumbrae_and_converge`: sharp floor
  shadow edges 211 → 146 pixels with a 4° disc (contact edges stay sharp),
  frame-to-frame mean |ΔG| 0.008 after convergence, 32 denoised frames.
- Sponza (local) with a 2° sun: soft floor shadow boundaries, no visible
  noise after 20 frames (`target/verification/sponza-soft-compare.png`).
