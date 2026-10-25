# 0031: Shared rich text shaping, layout, reveal and CPU SDF atlases

Date: 2026-10-25. Status: implemented and CPU behavior-validated; SDF GPU
rendering is not connected by this change.

## Requirement and ownership

The owner requested the text capabilities needed by visual novels and a modern
3D engine: complex shaping, bidirectional text, Unicode wrapping with Chinese /
Japanese prohibition tailoring, ruby, vertical writing, rich spans and icons,
outlines/shadows, typewriter events, fallback fonts and signed-distance glyphs.

These are shared safe CPU services in `crysirror-text`. The existing
`simple-horizontal-v1`, `TextService` and explicit positioned-glyph raster APIs
retain their behavior; a native VM is never treated as a new data VM and an
unknown native text profile still fails. This adds a separate rich API rather
than silently changing original layout/positioning semantics. No wgpu, scene,
GoW importer, probe or VM file is changed by this work.

## Implementation

Font resources loaded by `TextSystem::add_font` also retain their bytes in CPU
memory for the shaper. `add_font_face(alias, bytes, index)` supports TTC/OTC face
selection. Bytes and face/device objects do not enter project, save, layout or
SDF output data. The service limits faces to 64 aliases and each input to 32 MiB;
an alias replacement changes its digest, and stale shaped placements fail when
rasterized. Font fallback occurs at extended-grapheme boundaries so a base and
its combining marks select one face. An uncovered grapheme is an error.

The exact new dependencies were checked against their official documentation and
downloaded source before implementation:

- [rustybuzz 0.20.1](https://docs.rs/rustybuzz/0.20.1/rustybuzz/): OpenType
  shaping, glyph IDs, positions, language and monotone grapheme clusters;
- [unicode-bidi 0.3.18](https://docs.rs/unicode-bidi/0.3.18/unicode_bidi/): UAX #9
  paragraph levels and visual line runs;
- [unicode-linebreak 0.1.5](https://docs.rs/unicode-linebreak/0.1.5/unicode_linebreak/):
  UAX #14 opportunities;
- unicode-segmentation 1.13.3 and unicode-script 0.5.8: extended graphemes and
  shaping-script boundaries.

`RichText` consists of typed text spans, atomic ruby groups, icons and named
event markers. Each style carries a fallback alias chain, em size, RGBA color
and bold. The optional strict markup parser accepts `[b]`, `[color=#rrggbb[aa]]`,
`[size=24]`, `[font=alias]`, `[ruby=reading]base[/ruby]`,
`[icon=logical/path.png,width,height]` and `[event=name]`; `[[` quotes a bracket.
Unknown, mismatched, excessively nested and unclosed tags fail. There is no
HTML or script evaluation. Font aliases are resource names, icons remain logical
paths, and the caller resolves image bytes through its capability root.

`layout_rich(document, options)` returns font-qualified glyph placements, icon
rectangles, line metrics and UTF-8 / logical-grapheme ranges. Font digest is
included in each glyph identity. Layout validates a maximum 1 MiB of text,
65,536 graphemes and a four-million-pixel canvas. Options select language,
automatic/LTR/RTL base direction, start/center/end alignment, line spacing,
ruby sizing and horizontal or right-to-left-column vertical writing.

The pipeline measures shaped clusters, chooses line breaks, and shapes the final
visual runs with the correct directional/font/script context. Arabic joins and
ligatures are preserved, glyph order can differ from logical byte order, and
ligature clusters/ruby remain unbreakable. Start alignment follows the paragraph
direction. UAX #14 opportunities are supplemented with a documented strict
subset of Japanese prohibition classes: no opening brackets at a line end,
closing punctuation/small kana at a line start, or breaks around nonbreaking
spaces and word joiners. Emergency wrapping uses grapheme boundaries while
preserving these restrictions. Over-wide unbreakable content is reported via
`LayoutLine.overflow` / `Layout.clipped`, not silently dropped.

Ruby groups center the reading above the base or to its right in vertical
writing, reserve line/column space and reveal atomically with the base. Vertical
shaping uses OpenType top-to-bottom shaping and its vertical substitutions;
columns progress right to left. Complex scripts use real font features rather
than one glyph per Unicode code point.

`rasterize_rich(layout, options, icon_resolver)` produces straight-alpha sRGB
RGBA bytes. Color composition is evaluated in linear light. It supports a
round outline (disk dilation), a box-blurred offset shadow and an optional
logical grapheme reveal count. Synthetic bold emboldens the outline; an actual
bold face can instead be selected by alias. Glyph-mask memory is bounded to
64 MiB, individual icon inputs to 16 MiB and their combined cache to 64 MiB,
outline width to 16 px and shadow radius to 32 px.
Disk dilation uses monotonic row windows rather than a quadratic disk scan.
Missing CPU outlines in non-whitespace glyphs fail visibly; bitmap, SVG and
color-font rasterizers are not implemented here.

`Typewriter` uses extended graphemes, a bounded integer microsecond accumulator,
pause/resume and `finish`. It emits `Grapheme {index, bytes, text}`, named
`Marker` and a single `Finished` event. State stores logical counters and a
content/marker/rate fingerprint; restore onto another document/rate fails.
Combining sequences and ZWJ emoji do not split into separate reveal steps.
Logical ranges remain valid after bidi reordering. Font-dependent ligatures and
ruby appear once their complete required grapheme range has been revealed.

`build_sdf_atlas(layout, options)` rasterizes unique glyph keys and applies an
exact separable Euclidean distance transform. The output is shelf-packed R8
pages, UV rectangles, baseline-relative plane bounds and placements. The key
includes face digest, glyph ID, size and synthetic-bold state; identical glyphs
share an entry. R8 128 is the edge, values above it are inside; distance in
source pixels is `(r / 255 - 0.5) * 2 * spread`. Pages have independent transparent
borders, explicit spread and a combined 64 MiB budget. Icons are forwarded as
separate placements, not baked into glyph distance fields.

All SDF pages and placements are derived CPU products. They contain no font
file or GPU handle. A renderer may upload them and apply derivative-based
coverage for scaling/3D, but this work does not add that GPU path or claim a
world-space SDF draw result. Engine rich-text dynamic-surface binding is a
separate integration described in ADR 0033.

## Compilation

`cargo check -p crysirror-text --all-targets` and
`cargo test -p crysirror-text` passed on the local macOS arm64 host.
`cargo clippy -p crysirror-text --all-targets -- -D warnings` also passed. Full
workspace, script binding and desktop validation belong to the parent
integration run; this ADR does not infer their results from a crate build.

## Behavior validation

The ordinary crate suite contains no font fixture: **10 passed / 0 failed / 3
ignored**. Tests cover strict rich markup, Unicode bidi run order, UAX #14 and
CJK prohibited breaks, atomic wrapping, alpha composition, independent
brute-force checking of the optimized dilation, Euclidean distance values,
grapheme reveal, event order, pause, chunking, fingerprinted restore and the
unchanged legacy error/bounds paths.

All three font-dependent tests are explicitly `#[ignore]`; they are not counted
as ordinary passes. They were then executed explicitly with **3 passed / 0
failed** by reading caller-selected system files at runtime:

```sh
CRYSIRROR_TEST_FONT=/System/Library/Fonts/Supplemental/Arial.ttf \
CRYSIRROR_TEST_CJK_FONT='/System/Library/Fonts/Hiragino Sans GB.ttc' \
CRYSIRROR_TEST_ARABIC_FONT=/System/Library/Fonts/GeezaPro.ttc \
cargo test -p crysirror-text -- --ignored --nocapture
```

Those tests check Arabic lam-alef shaping, Hebrew visual order with logical
clusters, combining marks, CJK fallback, ruby, vertical flow, real raster colors,
synthetic bold, icons, outline/shadow coverage, zero-grapheme visibility, SDF
deduplication and atlas UVs. Font bytes are neither copied into tests nor checked
in. The tests require caller-licensed fonts and do not establish all-script or
all-font conformance.

The `rich_preview` example accepts three font paths and an absolute external
output directory. It refuses source-tree destinations. Actual CPU outputs were
generated and visually inspected at
`~/.cache/crysirror/codex-text/rich-text.png` (1080x480) and
`sdf-atlas-0.png` (512x512): 214 glyph placements / 84 unique glyph entries.
No font file or derived system-font capture is distributed with the source.

## Remaining limits

- The CJK tailoring is not a full JIS X 4051/JLREQ conformance implementation.
  Automatic hyphenation, justification, hanging punctuation, all ruby styles,
  tate-chu-yoko and full UAX #50 mixed-script vertical orientation remain work.
  Vertical text uses upright glyphs plus available OpenType substitutions;
  mixed-direction ruby spanning multiple bidi runs fails explicitly.
- Variable-font axis selection and bitmap/SVG/COLR color glyph drawing are not
  exposed. Fallback is by font coverage; no system-font discovery or licensing
  policy is inferred from an alias.
- Layout and rasterization are synchronous CPU calls. SDF pages are built for a
  layout, not an asynchronous globally evicting GPU glyph cache. No frame-time,
  GPU scalability, 3D text, Windows/Linux hardware or native Siglus typography
  equivalence result is claimed.
