# 0022: Screen transitions (Siglus wipe compositor)

Date: 2026-10-25. Status: implemented and locally validated (crossfade,
image mask, generated Siglus pattern masks, slide/scale/raster/mosaic/
explosion-blur families via the original shader). Page curls (300/301) and
the Siglus VM bridge export of `WIPE` are not implemented.

## Context

ADR 0021 added the Siglus per-sprite effect stack; the other half of the
Siglus 2D look is the stage `WIPE`. The original composes the saved old
scene and the prepared new scene in one full-screen pass. siglus_rs carries
that pass as `render/shaders/wipe.wgsl` and the CPU mask generators
(`runtime/wipe_mask.rs`, following the original `eng_mask_*.cpp`). Scene
transitions are a general engine feature, not a VN-only one.

## Decision

- **Engine data.** `LayerCommand::Transition { transition }` and
  `LayerCommand::EndTransition` (script: `Layers.transition`,
  `Layers.endTransition`, property `Layers.activeTransition`). A transition is
  owned by the starting client. Another client cannot replace or end it.
  It advances with the engine tick, is part of the saved `LayerSystem`, and
  is removed at its duration. Each start gets a new serial.
- **Effects.** `crossfade` (Siglus type 0), `mask {asset, fade 0..=8}`
  (type 900: bright mask pixels reveal first), `slide {direction}` (type 200,
  both scenes move), `mosaic {size}` (type 230), and `siglus {code, options,
  mask?}` for the shader's other types: 5–199, 210–215, 220/221, 230/231,
  240–243 and 901. Any other code is rejected visibly. Easing is the Siglus
  speed mode (linear, ease in/out, smooth). Option slot 15 carries the
  per-transition random seed, as in siglus_rs `wipe_uniform`.
- **Generated masks.** For pattern types without a mask, the runtime calls the
  unchanged siglus_rs `wipe_mask::generate` once per transition at the canvas
  size. The result goes into the owner's surface slot 999. A type that has no
  generator fails visibly instead of drawing nothing.
- **Renderer.** Every frame is composed into one of two retained frame
  textures and then presented (texel-exact `textureLoad` copy). While a
  transition is active, the last frame presented before it stays frozen as
  the old scene (Siglus NEXT) and the current frame is the new scene (FRONT).
  The vendored `wipe.wgsl` is included unmodified from the vendored tree. As
  in siglus_rs, where images are `Rgba8Unorm`, wipe inputs are sampled
  through non-sRGB views. Blends therefore operate on gamma-encoded bytes,
  and the present pass decodes the wipe output exactly for the sRGB target.
  Masks are uploaded as `Rgba8Unorm`.
- **Vendored change 6.** `runtime::wipe_mask` visibility only (see
  `crates/siglus-upstream/UPSTREAM.md`).

## Validation (2026-10-25, Apple M5 / Metal)

- Engine: `tests/transition.rs`.
  - Invalid specs rejected (zero duration, page curl 300, fade 9, unknown kind).
  - A Siglus type-5 transition with smooth easing reaches elapsed 160 ms and
    progress 0.5 after ten 16 ms frames.
  - The generated mask lands in `__surface/999` at the canvas size.
  - The transition is serialized with the layer state, ends at its duration,
    and `endTransition` ends it immediately.
- API coverage: both commands have script methods
  (`every_command_variant_has_an_api_method_that_exists`).
- GPU: `screen_transitions_compose_old_and_new_frames_with_siglus_arithmetic`
  runs the full runtime → renderer path.
  - The crossfade midpoint of red→blue is (128, 0, 128) ±2, which is
    gamma-space and not linear-light (≈188).
  - At 50%, a hard image mask shows exactly old on the black half and new on
    the white half.
  - The generated type-5 pattern shows both scenes mid-wipe.
  - A finished transition presents the new frame exactly.
- The full explicit GPU suite still passes with the new present pass.

## Gaps

- Page-curl wipes 300/301 (siglus_rs `page_wipe.wgsl` plus its geometry).
- Partial-range wipes (begin/end order and layer) and the `crysirror-siglus-vm`
  bridge export of an active `WIPE`. The bridge still refuses visibly.
- Headless `crysirror render` captures a single frame, so it has no previous
  frame and mid-transition captures show black as the old scene.
- Starting a new transition while one is active keeps the original frozen
  frame rather than the currently displayed composite.
