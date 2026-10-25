# 0033: Localization catalogs and shaped text surfaces

Date: 2026-10-25. Status: implemented first slice, CPU/engine validation below.

The owner authorized parallel audio/text/input work while another developer owns
the renderer, scene view/components, mesh clustering and GoW import tools.
This change uses the CPU text APIs from ADR 0031 and existing dynamic RGBA
surfaces, without changing those renderer/native-import paths.

## Implementation

`crysirror-engine::localization` provides client-owned catalogs and the
`Localization` reflective class, available to Rust and JS/TS as `localization`.
Catalogs contain strings or plural forms and an ordered font-alias list. Locale
identifiers are parsed/canonicalized by `unic-langid` 0.9; CLDR plural selection
uses `intl_pluralrules` 7.0.2 (the actual cached crate source was inspected).
This is the dependency's bundled CLDR generation, not a claim of current CLDR.

Lookup tries the requested locale, then drops variants, region and script, then
does the same for the configured fallback locale. Plural rules belong to the
locale of the message actually found, preserving the fallback language's
grammar. `count` may be a decimal string: English `"1"` and `"1.0"` therefore
retain distinct plural operands. Cardinal and ordinal rules are supported;
unsupported locales for plural rules fail visibly. Every plural message must
have `other`. Missing keys/parameters, malformed templates and unknown fields
are errors. `{name}` substitutes scalar values once; `{{`/`}}` escape braces.
Substituted values are never recursively expanded.

Per locale: at most 65,536 entries, 4 MiB of key/message bytes and 16 font aliases.
Per client: at most 64 catalogs. The resolved, deduplicated fallback chain is
also limited to the text service's 16 aliases. Definitions, locale changes and
profile application validate before commit. Font aliases reference explicitly
installed project fonts; localization never searches the OS or reads font files.

```js
localization.define("en", {
  strings: { coins: { forms: { one: "{count} coin", other: "{count} coins" } } },
  fonts: ["latin", "cjk"]
});
localization.setLocale("en-US");
const label = localization.translate("coins", {count: 2});
```

`crysirror-engine::text_api` exports `Text` / global `text`:

- `draw(name, markup, options) -> surfacePath`: strict rich markup plus
  `style`, `layout`, `raster` and optional `graphemes_per_second`.
  Missing style uses the current localization font chain, or explicit alias
  `default` when no locale fonts were configured. Redraw after changing locale
  to replace already-rendered content and its chosen fonts.
- `state` / `restore`: content/options plus the serializable grapheme-reveal
  clock. Restore rebuilds shaping/raster resources; it does not replay pending
  events. These are explicit application checkpoints, not an extension to the
  original VM save format or automatic `Runtime::snapshot` contents.
- `pause`, `finish`, `drainEvents`: grapheme, authored marker and finished
  events. Reveal advances from App delta time; pausing the App stops it.
  Individual world-client pause does not implicitly pause this UI clock.
- `remove`: releases the client-owned block and dynamic surface.

```js
const surface = text.draw("dialogue", "[ruby=にほんご]日本語[/ruby]", {
  layout: {canvas:[640,160]},
  graphemes_per_second: 30,
  raster: {outline:{width:1,color:[0,0,0,255]}}
});
// Use surface as the image asset in an existing Layers.put call.
// Poll text.drainEvents("dialogue") for grapheme/marker/finished events.
```

Eight blocks per client use dynamic slots 50000–50007. The existing total
surface-count/memory limits still apply across videos, transitions and text.
Inline icons resolve through the current client's capability asset root or
its own dynamic surfaces. Fonts remain in the explicit project font namespace.
No glyph outlines, device pointers or font bytes enter saved logical text data.
Event queues are bounded to 4,096 entries. Overflow or publication failure
restores the affected block's prior reveal clock and pending queue, instead
of losing reveal events. Surface memory/revision checks happen before mutation;
surface removal addresses `(client,slot)`.

## Boundaries and remaining work

The renderer consumes ordinary RGBA surfaces here. ADR 0031's SDF atlas is CPU
data with glyph placements/UVs; this change does not add an SDF GPU shader or
claim 3D SDF rendering. Locale-aware number/date formatting, ICU MessageFormat,
automatic UI retranslation, catalog-file watchers, and automatic engine snapshot
integration for localization/text-session configuration remain future work.
Explicit `profile/applyProfile` and `state/restore` allow an application to
persist these configurations without serializing derived fonts/surfaces.

## Compilation and behavior validation

See `../codex-services-results.json` for final aggregate results. Dedicated tests:

- `engine/tests/localization.rs`: English visible fraction digits, Arabic plural
  categories, Japanese `other`, locale fallback, font-alias deduplication,
  literal substitution, client isolation and invalid-profile rollback.
- `engine/tests/rich_text_api.rs`: ignored by default because it requires an
  explicitly supplied licensed system font. It reads that font at runtime,
  exercises real rich rasterization, dynamic surfaces, localization alias
  selection, reveal/pause/finish/events, state restore, icon path rejection,
  client isolation, surface release and event-overflow rollback. No font is
  copied into the source tree. Explicit execution is reported separately from
  default ignored-test counts.

No changes to `crysirror-wgpu`, scene `view.rs`/`components.rs`,
`crysirror-asset`, `tools/gow3_*`, or probe code are part of this parallel slice.
