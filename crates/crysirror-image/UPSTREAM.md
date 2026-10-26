# CPU bitmap source and semantic attribution

`bitmap.rs` supplies shared CPU image operations; no VM, presentation, numbered
buffer, filesystem, font, window or audio policy is introduced here.

Native pixel formulas and clipping behavior were adapted from the complete
`xmoezzz/siglus_rs` source at pinned commit
`5f358ef1360b0439e92aed2b749e8f25d8721954` (MPL-2.0):

- `crates/reallive/src/surface.rs`: `Rect` and `Surface::blit` (lines 10–54,
  250–303), `lerp` (129–130), `combine` (470–508). `Copy` corresponds to
  native `Blend::Copy` without an external mask. `Mask255` corresponds to
  native `Blend::Mask` without an external mask. The source uses straight
  RGB and alpha/mask bytes, opacity in 0..255, integer floor when combining
  source alpha and opacity, then rounded RGB interpolation using +127 /255.
- `crates/avg32/src/buffer.rs`: `blend_pixel` (110–120). Its RGB is already
  premultiplied by a separate mask. `PremultipliedMask256` preserves the
  destination alpha/mask and computes `min(255, src + ((256-mask)*dst >> 8))`.
  Mask zero leaves the entire destination unchanged. The byte value 255
  intentionally leaves a 1/256 destination contribution before saturation.
- `crates/avg32/src/pdtmgr.rs`: `color_key_copy` (355–374), `stretch_from`
  (615–650). `CopyRgb` with an exact RGB key preserves destination alpha;
  nearest sampling computes `source_origin + floor(offset*source_size /
  destination_size)` before clipping, preserving the original scale phase.

All shared rectangles are half-open `(x,y,width,height)`, with pixel units.
Native inclusive corners, ordering reversed corners, fixed 640x480 restrictions,
negative buffer-number presentation behavior, mask-plane replacement, native
fade parameter remapping, and self-copy snapshots remain adapter duties.
Coordinates are widened before arithmetic; malformed RGBA storage fails before
mutation and surfaces retain existing dimension/allocation limits.

`SourceOver` is conventional straight RGBA source-over, accounting for
source/destination alpha. It is not substituted for either native profile.
`fill_rect` directly replaces RGBA. Native partial-opacity fills (which can retain
alpha) must use the appropriate copy profile and a solid source surface.
`color_key` compares exact source RGB before blending and skips all writes.
For same-surface overlapping blits the caller passes a snapshot.

For `bitmap.rs`, only nearest sampling is implemented. RealLive's pixel-center bilinear sampling,
external masks, inverted/quantized/tiled masks, saturating add/sub, bitwise
blends, arbitrary transforms and AVG32 fade operations are not implemented by
this API. Unsupported sampling enum values fail deserialization; the
premultiplied /256 profile explicitly rejects opacity values other than 255.

Validation is recorded separately from source behavior. The crate's unit tests
check clipping, rounding/alpha distinctions, invalid input, nearest phase and
color-key preservation. External differential validation compares these shared
operations directly to the unchanged pinned upstream implementations; it uses
synthetic byte arrays, no game data or fonts, and no device creation.

Actual local validation (2026-10-25):

- Implementation: public checked bitmap API and explicit math profiles above.
- Compilation: `cargo check -p crysirror-image` exited 0.
- Unit behavior: `cargo test -p crysirror-image --lib` passed 7 tests, including
  4 new bitmap tests. No ignored test is counted as a pass.
- Upstream differential behavior: the external Rust driver at
  `/Users/xmoe/.cache/crysirror/bitmap-differential` used path dependencies to
  the unchanged pinned `reallive`/`avg32` crates and this shared image crate.
  `CARGO_TARGET_DIR=/Users/xmoe/.cache/crysirror/native-family-target
  AVG32_NO_AUDIO=1 REALLIVE_NO_AUDIO=1 cargo run --manifest-path
  /Users/xmoe/.cache/crysirror/bitmap-differential/Cargo.toml` exited 0.
  The exact pixel arrays matched for 19,200 RealLive Copy/Mask255 combinations
  (offscreen source/destination, zero extents, six opacity boundaries), 9,216
  AVG32 premultiplied mask cases (all 256 mask values, channel extrema), and
  12 AVG32 nearest stretch cases (clipped destinations, out-of-source sampling,
  destination RGB and mask retention). The driver log is
  `/tmp/codex-bitmap-differential.log`.

These results establish byte-level CPU primitive parity for the stated corpus;
they do not establish complete VM/game compatibility or GPU rendering results.
All arithmetic works on stored channel bytes without color-space conversion.

Reproduce the same differential corpus using the reviewable repository driver:

```sh
python3 tools/compare_bitmap_upstream.py \
  --source-root /absolute/path/to/complete/siglus_rs \
  --work-dir /absolute/path/to/external/cache \
  --target-dir /absolute/path/to/external/cargo-target
```

The tool checks the actual HEAD against `upstream-lock.json` and rejects index
or working-tree modifications to tracked upstream workspace/dependency sources.
Its generated Cargo workspace and target directory stay outside both source
trees, including when output directories use symbolic links. The Rust corpus
is `tools/bitmap_reference.rs`; the Python guard tests do not invoke Cargo.


## BGI frame display and packed CPU primitives

`composition.rs` adapts float-coordinate CPU drawing from `ethornell-rs`
`ethornell-core::composite_native_rgba` and `ethornell-app/src/snapshot.rs`.
`packed.rs` preserves the recovered integer bitmap kernels from
`ethornell-app/src/bitmap_blend.rs`, with checked public wrappers. The audited
source HEAD is `4c41dc3a566338d2ea8400635e23b062613e3782`; the local source
has other working changes, while these three referenced files match that HEAD.
The upstream workspace declares MPL-2.0. No original tree was modified.

Float-coordinate profiles retain explicit source-mask opacity, per-channel
add/subtract and signed /256 constant interpolation. Rectangular sampling uses
pixel centers and clamped source coordinates. Affine quads preserve TL, BL, BR,
TR vertices, clip bounds, half-texel source offsets, and either transparent
nearest-floor or transparent bilinear sampling quantized to 1/16 with floor.
These are distinct from standard RGBA source-over and from RealLive bilinear.
Per-glyph coverage remains floating point until color opacity is applied; bold
samples are duplicated in native order and italic rows use the recovered 20%
offset. Layout, ruby/reveal, native buffers and format selection stay adapters.

BGI display selectors 1 and its native alias 0x20 are explicitly source-alpha coverage in the pinned Rust
snapshot/GPU display path. Its origin is the BackF constructor's explicit
`sub_41B600(this, 1)`, dispatched by native `sub_40A9E0` cases 1 and
0x20 together to `sub_40B320`.
The original Rust display implementation uses floating-point coverage for that
selector family, whereas native packed format-pair kernels have distinct 7-bit/SSE
rounding. The shared BGI frame bridge preserves the Rust display path, retains
raw selectors, and does **not** claim commercial C-kernel bit equivalence for
that display operation. Other unsupported display selectors still fail; the
full native legal-mode table is not silently aliased to ordinary coverage.

The packed API separately provides those explicit format 1/2 native kernels,
including paired-pixel/tail differences, mask operations, fades, additive,
subtractive, multiply, screen, overlay/hard-light and channel extraction.
Unknown packed formats/modes fail before mutation; a recovered unsupported
format pair retains its native no-op result. These kernels are also used for
native glyph glow records. XRGB byte 3 is payload and must be normalized for
presentation rather than treated as display transparency.

Actual verification, 2026-10-25:

- `cargo test -p crysirror-image -p crysirror-text --lib`: shared image 32 pass;
  shared text 11 pass, 1 licensed-font test ignored (not counted as pass).
- `cargo test -p crysirror-bgi --lib`: 5 pass, including decoded native draw
  composition, strict unsupported-selector failure and client-scoped publication.
- `tools/compare_bgi_cpu.py` compiled actual unmodified original reference
  functions and matched 276,480 float pixel cases (including display modes 1 and 0x20),
  4,320 rect/quad/clip/sampling cases, 2,400 packed-kernel cases and 1,080
  positioned-glyph cases, byte for byte. Glyph cases use an explicitly supplied
  system font, fractional sizes/positions, bold, italic, clipping and opacity.
  No font was copied into the repository.

```sh
python3 tools/compare_bgi_cpu.py \
  --source-root /absolute/path/to/ethornell-rs \
  --work-dir /absolute/path/to/external/cache \
  --target-dir /absolute/path/to/external/cargo-target \
  --font /absolute/path/to/licensed-font.ttc
```

The comparison tool checks the actual reference HEAD and tracked-file changes.
It reads synthetic pixels and the explicit font only; no games/devices are used.
Log from the local run: `/tmp/codex-bgi-cpu-differential.log`. These results
establish parity with the stated original Rust reference paths and corpus,
not every native syscall, commercial engine build, font or hardware platform.

## Associated RGB, independent coverage and integer filters (2026-10-26)

`associated.rs` adds engine-owned checked operations without copying an AVG32
System, PDT manager or renderer. `AssociatedBlitOptions` selects copy, source-over
or draw-under with explicit 0..256 weights and independent source coverage;
coverage writes are explicit. `RgbFilter` implements signed-floor RGB fades and
integer multiplication while retaining alpha. `Bitmap.composite/filterRgb` expose
these same operations to scripts with client-scoped surface and asset paths.

The stored RGB remains associated intermediate data. This API does not normalize
it for the renderer's straight-alpha sampling or establish a complete native PDT
buffer/display policy. Native fade parameter remapping stays in adapters. The
existing `bitmap.rs` mode restrictions continue to apply to that older interface.

Actual differential execution against the unchanged pinned `avg32::buffer`
`fade_pixel/fade_under` and `avg32::pdtmgr::fade_color` passed 1,776,384 associated
copy/over/under cases (all 256 masks and 257 weights) and 2,304 signed/clipped fade
cases. Source RGB, destination RGB and coverage include byte extrema and
intermediate values. The existing 19,200 RealLive, 9,216 AVG32 mask and 12 nearest
cases were rerun and passed. The reviewed corpus is in `tools/bitmap_reference.rs`;
the guard and command are the same `tools/compare_bitmap_upstream.py` above.
The actual log is `target/associated-upstream.log`; no game, font or device was
used. These results cover those byte operations, not full AVG32 VM compatibility.
