# Shared CPU bitmaps

`crysirror_image::ImageSurface` owns the implementation. Native Rust adapters
can call `filled`, `fill_rect`, `blit` and `stretch_blit` directly. The engine's
`Bitmap` class calls those same operations and publishes their results through
client-scoped dynamic surfaces. No original interpreter or device is needed.

The script global is `bitmap`. Slots share the client's existing surface store
(16 slots, shared 256 MiB budget); callers must coordinate slots with text and
native-frame producers. Bitmap operations limit each image to 64 MiB and each
dimension to 8192. A slot number/path only refers to the current client.

```typescript
const source = bitmap.load(10, "portraits/hero.png");
const result = bitmap.create(11, 640, 360, [0, 0, 0, 0]);
bitmap.stretch(11, source, {
  source_rect: {x: 0, y: 0, width: 320, height: 180},
  destination: {x: 0, y: 0, width: 640, height: 360},
  sampling: "nearest",
  mode: "copy"
});
layers.put({id: 1, rect: [0, 0, 640, 360],
  paint: {kind: "image", asset: result}});
layers.setTransform(1, {rotate: Math.PI / 12, pivot_x: 320, pivot_y: 180});
layers.setFilter(1, {mono: 0.5, bright: 0.1});
```

These calls combine image processing, rotation and filtering in shared engine
services. The asset in this example is supplied by the caller, not distributed.
Layer rotation is in radians. Image processing mutates pixels; Layer filters and
transforms are retained render parameters and leave source pixels unchanged.

| Method | Result |
|---|---|
| `create(slot, width, height, rgba?)` | Create/replace a uniformly filled image; default transparent |
| `load(slot, source)` | Decode a client asset (including provider mounts), or copy a current-client surface |
| `fill(slot, rect, rgba)` | Replace all four channels in a clipped rectangle |
| `blit(slot, source, options)` | Copy/blend a source rectangle at `[x,y]` |
| `stretch(slot, source, options)` | Nearest-neighbor scaling into a destination rectangle |
| `info(slot)` | Dimensions, byte size, revision and asset path |
| `remove(slot)` | Release this client's slot |

Mutation methods return the surface asset path `__surface/<slot>`. A source can
be such a path or an encoded image asset relative to the client's root. Unknown
slots, unsafe paths, unsupported modes and over-budget images fail. A failed
operation retains destination pixels and revision. Self-copy reads a snapshot
of the source; it cannot smear sequentially overwritten pixels.

Rectangles are half-open `{x,y,width,height}` in pixels, with signed origins.
Blit options require `source_rect` and `destination:[x,y]`; stretch requires two
rectangles. Both accept `opacity` (0–255, default 255), `mode` and optional exact
RGB `color_key`. Keyed pixels write no channels. Clipping preserves the original
source-to-destination correspondence and the original nearest-sampling phase.

| PixelMath / JSON mode | Color and alpha policy |
|---|---|
| `Copy` / `copy` (default) | Lerp all four channels using opacity, rounded division by 255 |
| `CopyRgb` / `copy_rgb` | Same RGB math, retain destination alpha |
| `SourceOver` / `source_over` | Straight RGBA source-over including destination alpha |
| `Mask255` / `mask255` | RealLive-style source-alpha × opacity truncation, rounded RGB lerp, native alpha accumulation |
| `PremultipliedMask256` / `premultiplied_mask256` | AVG32-style already-premultiplied RGB and byte mask; truncating division by 256, saturation, retain destination alpha; requires opacity 255 |

For the last mode source alpha carries the native mask. An adapter must supply
the appropriate representation and resolve mask/display-alpha policy. Choosing
it does not automatically premultiply an ordinary PNG. Standard `source_over`
is the normal choice for compositing straight-alpha image assets.

These services do not interpret native opcodes, buffer numbers, fade timing or
inclusive rectangle endpoints. RealLive bilinear sampling, external masks and
AVG32 native fade are not implemented by nearest stretch. Mutable surfaces are
transient and not automatically serialized by runtime snapshots; adapters must
reconstruct them under their original save rules. See ADR 0038 and the source
attribution for verified native coverage.
