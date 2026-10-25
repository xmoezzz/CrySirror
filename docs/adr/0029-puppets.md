# 0029: 2D deformable puppets (engine rig format, PuppetSource, eluna bridge)

Date: 2026-10-25. Status: implemented and validated on synthetic rigs. The
eluna bridge has not been run on a real E-mote file, because none is
available locally.

## Context

The owner asked for their eluna E-mote runtime (the Live2D-class player
vendored with siglus_rs) to become an engine capability, with two
constraints:
- Packages must not use PSB, a third-party container format.
- The engine provides the interface; the underlying runtime provides the
  data.

Siglus keeps its own E-mote path (ADR 0009); this ADR is about the general
engine.

## Decision

- **`crysirror-puppet`** is pure Rust with no GPU, window or file system.
  - **Rig format** `crysirror-puppet` v1: JSON next to PNG textures. It
    contains:
    - parameters (min, default, max);
    - textured mesh parts with per-parameter keyforms (vertex offsets,
      opacity factor, order offset);
    - deformer chains (rotation, translation and scale about a pivot,
      keyed by parameters);
    - masks (a union of mask parts, optionally inverted);
    - five blends: normal, add, subtract, multiply and screen;
    - motions (parameter curves; linear, step or smooth; override or
      additive; fades; looping);
    - damped pendulums that drive parameters from weighted inputs.
  - Validation checks every reference, array length, key ordering and
    deformer cycle, and rejects `.psb` textures.
  - **Evaluation order.** Parameters start at their defaults. Motions are
    applied in start order. Scripted values (eased, latched until
    released) override motions. Physics runs at 120 Hz on the resulting
    values. Scripted values then override physics outputs again. Values
    are clamped. Parts get their keyform offsets, then the deformer chain,
    then opacity, and are sorted by order.
  - The state (time, values, overrides, motions, pendulums) is serde and is
    saved with the layers.
  - **`PuppetSource`** is the interface the engine drives: parameters,
    motions, parameter values, set/release/play/stop/advance, frames and
    textures. Rigs implement it natively.
  - **`bake`** samples any `PuppetSource` through that interface into a rig:
    - Each parameter is sampled at evenly spaced values plus the source's
      own keys, with all other parameters at their defaults. Vertex offsets
      and opacity ratios become keyforms.
    - Parts that appear only away from the defaults get transparent rest
      keys.
    - Motions are recorded as 30 Hz parameter curves and simplified.
    - The report lists topology changes, draw-order changes (not baked),
      parts hidden at rest, and the model's limitation: additive,
      per-parameter keyforms.
- **`crysirror-puppet-eluna`** implements `PuppetSource` on the vendored
  `SiglusEmoteRuntime`, through upstream change 7: wrappers only.
  - Sprite geometry follows siglus_scene_vm `render_plan/emote.rs` in model
    space: rotation/scale, the world affine, mesh patch sampling and corner
    colours.
  - Stencil chains map to masks (phase 1) or inverted masks (phase 2).
  - Native blend modes map to engine blends.
  - Variables become parameters; timelines become motions (60 ticks/s).
  - Feedback sprites, missing textures and mixed phase-1/phase-2 chains are
    counted as diagnostics.
- **`crysirror puppet-import <psb…> --out dir [--key k]`** (desktop feature
  `emote-import`, on by default) bakes local files into `puppet.json`,
  `texture_NNN.png` and `import-report.json`.
- **Engine.**
  - `Paint::Puppet {asset, tint}` on layers rejects PSB assets.
  - `LayerCommand::Puppet {id, command}` queues set/release/play/stop.
  - The runtime loads rigs from the client's assets, applies the queued
    commands, advances every puppet on each tick and publishes meshes. The
    layer system keeps one saved slot per puppet layer, and a different rig
    resets it.
  - The `Puppets` class checks names against the rig before submitting.
  - Python mirrors: `engine_schema.paint` accepts puppets and rejects PSB.
    The packager adds the rig and its textures and refuses PSB.
- **Renderer** (`puppet.rs`, `puppet.wgsl`).
  - Parts are drawn premultiplied into an Rgba16Float target with a
    Depth24PlusStencil8 attachment. The puppet is fitted into the layer
    rect with its aspect ratio preserved.
  - Masks write a per-part stencil reference where mask alpha ≥ 0.5; a new
    pass with a cleared stencil starts after 255 references.
  - A resolve pass un-premultiplies into the layer's sRGB texture, so the
    existing compositor, filters and transitions apply unchanged.

## Validation (2026-10-25, Apple M5 / Metal)

- `crysirror-puppet` unit tests (6):
  - JSON round trip and rejection of broken references, cycles, PSB
    textures and wrong offset counts.
  - Deformer rotation matches the analytic value, and mask indices and
    order are correct.
  - Motions loop, fade and end, and scripted overrides ease, hold and
    release.
  - The pendulum overshoots and then settles; the state round-trips
    exactly.
  - Baking the evaluator through `PuppetSource`: single-parameter poses
    agree within 0.5 px (nine samples over ±15° rotation), two combined
    parameters agree within 2 px, and the blink motion bakes to an
    `eye_open` curve.
- `crysirror-puppet-eluna` unit tests (2): the blend mapping matches
  `native_blend_state`, and invalid PSB input fails visibly.
- Engine `tests/puppets.rs`:
  - Unknown names and non-puppet layers fail at the call.
  - A motion plus a scripted head turn moves the extracted meshes, and the
    pendulum follows.
  - The state is saved in `layers.puppets` and validates after a round
    trip.
  - Release returns a parameter to its default; replacing the paint drops
    the state; PSB layers are rejected.
- GPU `puppet_layers_render_meshes_with_masks_blends_and_deformation`:
  - Exact pixels for the masked dot, the inverted-mask ring, the face and
    the transparent background.
  - A 90° deformer rotation moves the arm (`dist/puppet-test.png`,
    `dist/puppet-turned-test.png`).
- Python: packaging includes the rig and its textures and refuses PSB.

## Gaps

- **Real E-mote validation.** No PSB is available. Before any claim of
  equivalence, `puppet-import` must be run on an owner-supplied model, and
  its frames compared with the Siglus E-mote renderer.
- **Bake model.** Interactions between parameters, draw-order changes and
  native physics (only baked as motion curves) are not reproduced.
  Multi-dimensional keyform grids would be the next format step.
- No GPU skinning or caching of unchanged frames: vertices are rebuilt on
  the CPU each frame.
- Masks are binary at alpha 0.5, with no soft-mask blending.
