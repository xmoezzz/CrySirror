# ADR 0094: Isolated retained-layer subtree crossfades

## Decision

`LayerSystem` owns a client-scoped transition between two sibling subtrees.
`BeginCrossfade { id, source, duration_ms }` starts its engine clock;
`LayerCrossfade(id)` / `Layers.crossfadeStatus(id)` reads progress;
`EndCrossfade { id }` releases it without changing either original subtree.
The same commands are exposed as `Layers.beginCrossfade/endCrossfade` to ordinary
engine scripts. They do not depend on a native VM or on a previously captured
whole screen.

The renderer isolates each endpoint using its normal image, canvas, text and
puppet rendering path, then blends the two products. Fading every descendant's
opacity independently would change overlap results and is not equivalent.
Normal engine rendering composites the product at the destination's order.
Nested/overlapping transitions and unsupported ordering/geometry are rejected.
Validation bounds admission and GPU texture memory before allocating targets.

## Ownership and execution

The owner thread applies transactional layer commands and advances elapsed time.
A frame contains immutable endpoint primitives, with renderer-owned GPU targets;
no GPU handles or textures enter saved project/VM data. GPU commands run on the
existing renderer queue in dependency order: endpoint passes, blend, presentation.
There is no separate pixel compositor inside the native adapter. Snapshot state
contains only validated endpoint IDs and bounded time; rendering products are
reconstructed.

The KiriKiri adapter maps the `crossfade`/with-children profile to these APIs.
It tracks original object identities, exchanges sibling placement and visibility
on completion/explicit stop, and emits the synchronous native completion event
once. The engine decides progress; the adapter decides the VM's exchange and
callback semantics. Object invalidation cancels its native mapping and engine
transition. Unsupported handlers, external clocks and surface-only exchange
fail explicitly.

Native reference: krkrz commit
`fd5c4baa6a2ef5978db1bd043634351f48667daf`,
`visual/LayerIntf.cpp` (`StartTransition`, `InternalStopTransition`,
`StopTransition`, `onTransitionCompleted`). The implementation is a semantic
adapter, not a hosted copy of the upstream renderer.

## Verification

Independent scene-state, native completion and explicit GPU pixel tests accompany
this change. Execution results and the actual game-kr progression are recorded
separately in STATUS.md and the local verification handoff; authored tests and
successful compilation alone do not establish rendered or whole-game support.

## Frozen retained snapshots

`Canvases.apply` also exposes `copy_subtree` for copying an engine-owned layer
subtree into a canvas in source-local coordinates. It replaces the clipped
region including alpha. The owner captures logical draw state atomically; later
source tree edits/removal do not mutate the captured nodes. Existing immutable
image/font resources retain their normal engine resource lifetime. This is not
a GPU readback API or a claim that mutable external asset contents are frozen.

The first profile preserves static normal-blend drawing, leaf opacity,
transform/clip ancestry and explicit painter order. It ignores the source root's
placement, visibility, opacity and external ancestors. Hidden descendants are
excluded. Dynamic puppet evaluation, active composites/reveals and unsupported
isolated group blending are rejected instead of capturing stale or incomplete
frames. Snapshot expansion is bounded by Canvas admission limits. The native
`piledCopy` adapter translates coordinates, image/draw-clip semantics and original
handles; it does not implement a separate compositor.

The frozen representation is one `Paint::Snapshot { size, layers }` draw. The
renderer rasterizes it at `size` once and caches a straight-alpha texture; outer
opacity, scaling and filters then operate on the whole product. This avoids the
incorrect overlap behavior of expanding the copied tree into the destination.
The cache identity includes owner/store and the validated frozen definition,
not the outer draw's opacity or transform. Its dependencies stay pinned while
rendering. Pending image decode never publishes a successful empty texture.

The initial renderer admits solids and already-published immutable engine PNGs,
including immutable filter images. It verifies `Assets::generated_for` and the
self-contained encoding rather than trusting a path prefix. Disk/pack paths,
dynamic surfaces, uncaptured font aliases and nested snapshots are rejected.
A restored snapshot requires its referenced immutable resources to be restored
as well. It does not reconstruct them by guessing an external file path.
KiriKiri's direct-image `Complete` fast path remains an RGBA canvas copy; opaque
composite cache-alpha behavior is a separate, not yet general, native profile.
