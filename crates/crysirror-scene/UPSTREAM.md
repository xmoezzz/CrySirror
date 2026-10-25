# Shared CPU source migration

License: MPL-2.0; full text is the monorepo root LICENSE. This crate forbids
unsafe code and has no window, GPU or audio SDK dependency.

| Origin | Exact commit | Migrated implementation |
|---|---|---|
| https://github.com/xmoezzz/siglus_rs | `5f358ef1360b0439e92aed2b749e8f25d8721954` | Complete runtime/input.rs -> input.rs, runtime/int_event.rs -> animation.rs, render_math.rs -> projection.rs |
| https://github.com/xmoezzz/rfvp | `062fec82033b554b9eb3b5da069518c0c96a354e` | UiSurfaceTransform, inside_rect, map_window_to_virtual from exit_confirm_ui.rs -> ui.rs |

The complete listed source files were read at the pinned commits before this
migration. Siglus layer.rs was also read as the reference for retained layer
storage, ordering and sprite fields. It is NOT presented as a verbatim port:
our safe scoped LayerSystem and frame extraction are new integration code.
RFVP's capture-on-down/activate-on-matching-release behavior was generalized to
retained widgets; its native GPU/window confirmation dialog was not copied.
Source SHA256 values, including reviewed reference files, are in upstream-files.json.

Changes to directly migrated code:

- input.rs exposes vmkey_to_vk_code publicly; algorithm and original tests stay.
- animation.rs adds serde snapshot derives with strict fields. The original
  distinction between stored value and interpolated value is preserved.
- projection.rs binds the original math to the engine's SpriteTransform type,
  including original radian rotation, object anchors, billboard/camera, culling
  and homogeneous W. Formatting changed; original tests remain.
- RFVP coordinate mapping exposes methods and content_rect for engine callers.
  It keeps half-open hit regions and aspect-fit letterbox behavior.

World/typed entity lifecycle, fixed-step scheduling, hierarchy, render extraction,
layer clips/mutations, widget tree/layout/focus/modal routing and persistence are
new engine integration. World tweens execute the migrated IntEvent. UI executes
the migrated InputState. Both raster and compute paths consume layer primitives.
No original VM, opcode/value decoder, native save codec or game UI is registered
by this migration. Those policies remain separate compatibility work.

Reproduce the actual original-vs-migrated algorithm comparison:

```sh
python3 tools/compare_scene_upstream.py --source-root /absolute/external/references
```

It verifies actual Git pins and source SHA256, compiles the original input,
IntEvent, projection and RFVP helper code in a temporary driver, then compares
175,700 observations. No game data, SDK, credentials or fonts are imported.
