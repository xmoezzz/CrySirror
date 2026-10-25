# Siglus transform bridge verification

Source: complete local `siglus_rs` checkout at pinned commit
`5f358ef1360b0439e92aed2b749e8f25d8721954`.

## Implementation

`crysirror-siglus-vm/src/export.rs` already sends native scale, rotation, pivot
and color effects through shared `SpriteTransform` / `LayerFilter`. The bridge
included `center_rep_x/y` in the pivot but omitted their corresponding anchor
translation. That changed an object's position even at unit scale.

The correction follows the pinned collector's sequence:

- `runtime/mod.rs:13923`, `apply_object_render_info_to_sprite`: XY pivot receives
  `center + center_rep`, and the native XY anchor is bounded to i32.
- `runtime/mod.rs:13021`, `configure_sprite_3d`: Z pivot is reset to `center_z`.
- `runtime/mod.rs:13952`, `finalize_object_center_rep_to_sprite`: XY anchor gains
  `center_rep_x/y`, bounded to i32; Z anchor gains `center_rep_z`.
- Both supported rectangle and image objects take this collector path
  (`runtime/mod.rs:13607–13636`).

Only the compatibility translation and a native-bytecode regression test were
changed. No renderer, original VM opcode, value, event or save implementation
was changed.

## Compilation

See `native-audit-siglus-transform-results.json` and the `check` log for the
actual targeted all-targets check. The runtime and Siglus test builds also
compiled the changed adapter.

## Behavior

The new `native_center_rep_moves_the_scale_origin_without_shifting_unit_scale_xy`
test runs self-authored native SCN/PCK bytes through the registered Siglus
adapter. Native setters configure a rectangle at (28,28), with center offsets
(20,-12,30). Its shared `EngineSystems` quad remains at (28,28) at unit scale;
after native X/Y scale setters change scale to (2,1.5), its top-left becomes
(8,34). The depth assertion checks the added Z anchor independently.

The targeted case passes. The Siglus/runtime suites report 43 passed, 0 failed,
0 ignored. This is CPU geometry validation; no GPU pixel comparison or complete
game run is claimed. Command times and exact results are recorded in the JSON
and adjacent logs.

## Two rejected speculative mappings

Native `color_add` is **not signed** in the pinned render pipeline.
`apply_object_render_info_to_sprite` clamps each native channel to 0..255 before
storing a `u8` sprite channel (`runtime/mod.rs:13942–13944`). The existing clamp
in the compatibility bridge is therefore correct, even though the shared engine
filter supports a wider signed addition range.

Native destination clipping is a logical-canvas scissor independent of the
object transform (`render_plan/mod.rs:2859`, `dst_scissor_rect_to_viewport`).
Shared `LayerSpec.clip` is instead a local rectangle projected through the
object transform (`crysirror-scene/src/layer.rs`, `LayerSystem::frame`). A direct
copy would silently move/rotate the clip with the object. Supporting native
destination clipping needs an explicit screen-space clip contract in the common
engine or an equivalent audited translation; the adapter continues to reject
this unsupported state visibly. Source clipping remains unsupported as well.
