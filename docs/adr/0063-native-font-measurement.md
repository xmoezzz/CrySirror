# 0063: Native font callbacks use engine text measurement

Date: 2026-10-26. Status: implemented; validation recorded separately in STATUS.md.

## Decision

`crysirror-text::TextSystem::measure_simple` measures one unwrapped line using
the same font, pair kerning, tab advances and missing-glyph checks as the
engine's `simple-horizontal-v1` drawing path. It returns advance, ascent,
descent and line pitch without rasterization or a surface allocation. Multiline
requests fail explicitly; complex shaping remains `layout_rich`'s contract.
`Text.measureSimple(font, size, text)` exposes this to ordinary JS/TS callers.

`HostServices::text_measurement(client)` checks client authority and returns
an immutable `Arc<dyn TextMeasurement + Send + Sync>` resource snapshot. Native
VM callbacks can call it without borrowing the mutable owner host. Font maps
clone resource handles; font bytes are shared. Handles never enter save data.
Replacing a runtime/font resource does not mutate an existing snapshot. Obtain
a new capability after replacement. Calls are bounded CPU work; the caller
owns completion, and no worker or GPU join occurs inside measurement.

KiriKiri `Layer.font.getTextWidth` translates to this API, with width rounded
up. `getTextHeight` preserves KiriKiri's absolute nominal `Font.height`, even
for an empty string, instead of substituting line pitch or ink height. The
adapter's configured font alias is also used by Canvas text drawing. Unmapped
bold, italic and rotated styles now fail in measurement and drawing, rather
than silently producing a different style. Font size outside magnitude 1..512
also fails rather than being clamped.

Canvas text also accepts `simple-horizontal-unwrapped-v1`: it uses the same
metrics, preserves glyph positions beyond the right bound and clips during
rasterization instead of wrapping. KiriKiri's single-line `drawText` requests
this shared profile, so long text is cropped rather than appearing on a second
line. Multiline native draw requests remain an explicit unsupported policy.

Desktop font overrides populate both the renderer's CPU font resources and
native host resources before the first capture VM tick. Loading only before
GPU capture was too late for a script measuring text during startup.
The interactive Run command also loads overrides before script init/start.

## Source and acceptance boundaries

Reviewed the complete local pinned Kirakira source checkout at
`c451918023477620ff9055a2eb8a6714b947d9c1`, including native Font methods,
font resource/measurement contracts and parser/VM dependency boundaries. No
Kirakira font, renderer, audio or other engine implementation is copied or
linked. The already-vendored original TJS2 VM stays unchanged.

Independent engine measurement tests and original TJS2 callback tests use a
caller-provided licensed system font at runtime and are marked ignored by
default. `examples/krkr-demo/metrics.json` is a self-authored script through
the actual VM. Neither the example nor these callbacks constitute complete
KAG3/game compatibility; XP3 boot, KAGParser natives, native save semantics
and unmapped drawing operations remain separate work.
