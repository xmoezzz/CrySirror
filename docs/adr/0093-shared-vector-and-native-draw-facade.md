# ADR 0093: Shared vector geometry and native LayerExDraw facade

Date: 2026-10-28
Status: Accepted for the bounded solid-linear profile below

## Decision and ownership

The native adapter holds only Appearance/Path/transform semantic state. Shared
crysirror-image::vector_geometry owns rectangle/polygon/connected-line authoring,
Append matrix composition and transformed filled bounds; runtime::vector exposes
that same safe API without a VM. Existing VectorPath/tiny-skia raster executes on
the shared image worker. An Appearance becomes at most 16 ordered vector effects
in one worker product, preserving brush offsets/order and atomic publication.
No private renderer/painter or composed legacy screen is imported.

The profile maps GdiPlus.Appearance solid brushes, GdiPlus.Path line/rectangle/
polygon figures, returned RectF fields/edge getters and Layer drawPath/Rectangle/
Polygon/Line(s), clear, transform/viewTransform set/reset/translate/scale/rotate.
Bounds follow offset -> transform -> viewTransform Append semantics and union all
painted figures, independent of visible clipping. Only filled linear geometry is
claimed; GDI stroke widths/caps/joins, curve/ellipse bounds and text are not aliased
to superficially similar operations. Non-AA modes, disabled auto-update, metafile
recording, brush objects/pens and overridden Layer.update callbacks fail visibly.
256 Appearance objects and 256 Path objects, 8192 path commands and 16 brushes are explicit
admission bounds; the original built-in update identity is captured before any script executes;
original object invalidation releases semantic state even if
script finalizers omit their native base. Native shapes contain no pixels.

The shared vector kernel also preserves untouched straight RGBA pixels exactly
when round-tripping through tiny-skia's premultiplied representation; vector draws
must not quantize unrelated semitransparent background pixels. The test suite adds
independent geometry/pixel checks plus native transform/bounds/worker publication
and finalizer tests. These second-batch tests were provided to the integrating
owner before a unified release run; no unexecuted test is reported as a pass.
Source pin and review scopes are in docs/krkr-draw-source-audit.json.


## Raster evidence limits

The backend remains the existing tiny-skia antialias policy, not a declaration
of every GDI+ edge-coverage/rounding rule. Native tests cover opaque interiors,
brush order, holes and geometry/return bounds; no native Windows GDI+ pixel oracle
or whole-game compatibility is claimed. The scalar rectangle/polygon/connected
line profile has no stroke/curve-bound approximations hidden behind it.

The straight RGBA preservation rule compares output premultiplied bytes against
those derived from the original source pixel. If those bytes and alpha did not
change, it retains the exact original straight pixel. Changed pixels follow the
existing unpremultiply path. This avoids unrelated rounding (for example RGB 101
at alpha 128 becoming 102 or 100) without changing tiny-skia's blend calculation.
No second pixel layer or private native image buffer is introduced.

## Validation boundary

Implementation is recorded above. Compilation and the three shared geometry/
pixel tests plus three native bridge tests are reported separately by the owner
following the unified release run. Tests that have merely been authored are not
counted as passing. No GPU, fonts, native DLL or game data is needed for these
fixtures. This phase leaves the shared image-worker API and ADR 0090 publication
bounds intact; metadata geometry computation is bounded owner-side CPU work.
