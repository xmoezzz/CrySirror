# NScripter native adapter

`crysirror-nscripter` is a first-class Rust native text-script interpreter. It
never invokes or embeds ONScripter, SDL, an original renderer or a screen host.
The pinned reference, notices and boundaries are in SOURCE.json and LICENSE.
No game data or fonts are included.

Configuration requires capability-relative `script` (`0.txt` or XOR-encoded
`nscript.dat`) and a licensed project `font` alias. `encoding` is `sjis`
(default) or `utf8`; `canvas` defaults to 640×480. `text` supplies initial plain
message bounds/size/color; native `setwindow` can replace those declarations.
Optional `archives` contains engine mount prefixes in priority order. Loose
files are searched first, then those prefixes, with scoped case-insensitive
native filename resolution. The desktop `play` bootstrap mounts original
archives in memory through the shared resource API; no extraction is required.
Numbered 00..99.txt concatenation and other script protection remain unmapped.

Native labels, numeric/string variables and aliases, expressions, conditionals,
loops, subroutines, physical-line `skip`, multiline selections and backtick
English strings execute directly. Inline click/page/speed/wait controls are
native policy; shared glyph layout/reveal, Layer/UI, Image and Audio services
own their implementations. Source read/decode/parse and resource processing have
bounded worker/completion/cancellation boundaries. Metadata lookup is currently
synchronous and full engine-wide async resource staging is still open.

Native backgrounds, sprites and left/center/right characters use ordinary engine
image layers. Resource transparency calls shared `ImageEffect::SpriteAlpha`
(opaque, exact color key, decoded PNG alpha, paired color/mask sheets). `effect`
definitions prepare reusable stripe/stagger mask images on shared workers;
crossfade, directional slide and rule transitions call engine services. The
8-bit rule-map path has not been proven pixel-identical to ONS integer stripe
edges. Complex animated sprites and arbitrary transparency tags remain explicit
failures. Native animated text-wait icons use shared `SheetSequence` clock/UV
selection; these are separate from the OS pointer cursor.

`setwindow` supports square horizontal fixed-cell layout, spacing, bold, shadow
and a color multiplication backing. Text color is retained per glyph. Native
selection and atlas buttons use scoped retained UI hit/focus/activation state.
`play "*N"` resolves local `CD/trackNN.ogg`; wave/BGM operations use shared
worker-backed channel audio admission. Date/timers and skippable delay preserve
native logical control. Window-effect declarations and menu/save metadata are
retained, but full window enter/leave animation, right-click system menus,
backlog/skip modes, rich markup, plugins, original save/load and original global
file interchange remain unfinished. They are not compatibility claims.

`globalon` and `filelog` use engine Save capabilities for bounded, worker-written
`nscripter-state.json` metadata (global indices ≥200 and accessed resource names).
This private engine format does not overwrite or claim to import original game
saves. Framework snapshots have schema 2 and reject pending jobs and prepared
image reconstruction, which is not implemented. Original `savegame`, `loadgame`
and unsupported native commands fail at their native PC.

Validation separates syntax/semantic tests, shared engine capability tests and
the ignored `owner_game` test using `CRYSIRROR_NSCRIPT_GAME`. The latter reads the
owner's installed Tsukihime and its font at runtime; it does not ship their
bytes. Actual compilation and game milestones are recorded in STATUS.md. A
registered factory or a self-authored screenshot is not a completed game port.
