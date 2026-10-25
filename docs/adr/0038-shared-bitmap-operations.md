# ADR 0038: native image operations become shared Bitmap capabilities

Status: implementation and validation recorded with this change.

## Requirement

The owner's priority is to use existing engine implementations to expand
CrySirror's capabilities. Successfully launching an external engine is not the
deliverable. Shared primitives execute in CrySirror; adapters retain bytecode,
native handles, units, resource policies, waits and original saves.

## Decision

Add safe CPU bitmap operations to crysirror-image, which previously only exposed
checked surfaces, decoders and encoding. Add an engine Bitmap class using that
same implementation, automatically reachable by Rust and the JS/TS binding.
Do not introduce a second renderer. Publish mutable results through existing
client-scoped Assets surfaces; Layers already provides transform/rotation,
filtering, clipping, composition and presentation.

The implemented shared operations are uniform creation, clipped rectangle fill,
copy, opacity/mask blending, RGB color-keying and nearest-neighbor stretch.
Adapters can call ImageSurface directly rather than rendering a complete native
frame. Scripts use create/load/fill/blit/stretch/info/remove on the Bitmap class.
Loading goes through client FileSystem capability/provider mounts.

The source audit at pinned siglus_rs
5f358ef1360b0439e92aed2b749e8f25d8721954 identified different pixel math:
RealLive uses straight RGBA with rounded division by 255; AVG32 uses separate
mask/premultiplied RGB with truncating division by 256 and alpha-preserving
copies. The API exposes explicit PixelMath choices instead of silently equating
these behaviors. Native numbered buffers, inclusive endpoints, fade arguments
and presentation rules remain adapter policies. See crysirror-image/UPSTREAM.md
and docs/BITMAPS.md for the precise implemented boundary.

Operations stage their result before updating a retained surface. Parse,
unsupported mode or publication-budget failures leave its revision/pixels
unchanged. Overlapping self-blits read the old source image. Handles are scoped
by client and slot; paths never grant access to another client's surface.

## Rejected alternatives

- Treating native whole-frame output as shared-capability extraction.
- Inventing one generic alpha blend and changing native rounding or masks.
- Moving native VM dispatch, device dependencies or unsafe code into core/compat.
- Adding a renderer/filter implementation when Layers already supplies it.
- Pretending nearest stretch implements RealLive's bilinear scaling.

## Compilation and behavior acceptance

Record actual results in STATUS.md and bitmap-capabilities-results.json.
Acceptance includes comparison with the pinned original implementations,
pixel assertions for clipping/masks/rounding/scaling, client isolation,
provider-aware loading, overlap handling, failure rollback, actual JS/TS calls,
and composing the bitmap with existing shared Layer transform/filter data.
No device or game-data fixture is needed for these CPU checks. This iteration
does not claim that the RealLive/AVG32/BGI VM adapters have been ported.

## Limits

The existing Siglus adapter's shared-transform call was also corrected: native
CENTER_REP shifts the XY anchor as well as pivot, and native 3D configuration
resets pivot Z before the final anchor adjustment. An actual native setter
fixture now verifies unchanged unit-scale XY and correctly shifted scaled
geometry/depth through EngineSystems. Evidence and source references are in
native-audit-siglus-transform.md. Native screen-space destination clipping still
needs a separate engine contract; it must not be treated as local object clip.

Bitmaps are transient render products, subject to existing surface budgets and
a 64 MiB per-image processing limit. Runtime snapshots do not serialize them;
native adapters must preserve/reconstruct bitmap state from validated native
save semantics before declaring save compatibility. The explicit AVG32-style
premultiplied mode interprets source RGB as already premultiplied and alpha as
its mask; it preserves destination alpha. This is not automatic conversion to
straight-alpha display storage. Bilinear, external mask planes, native AVG32
fade, and complete native effect stacks remain separate capabilities to add.
