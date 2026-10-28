# Local game-kr plugin audit

Date: 2026-10-28. This is a disk inventory, **not an executed dependency graph**.
The main startup was blocked by HXV4 resource handling when this inventory was
taken; no claim is made that the game loads every DLL shipped beside it.

Current probe: shared HXV4 indexing, checksum-verified reads and explicit EXE
RCDATA startup loading now work for this local package. The unchanged native
startup passes its archive-directory request and stops at Plugins.linkZ; no game
scene is established. See [krkr-bootstrap-gap.md](krkr-bootstrap-gap.md). JSON and
solid-linear LayerExDraw fixtures are separate passing capability evidence.

Reference catalog: Kirakira `c451918023477620ff9055a2eb8a6714b947d9c1`.
Module location and an upstream status are reference leads, not CrySirror behavior
evidence. Only the explicitly implemented/tested CrySirror mappings count.

| Shipped file | Reference module located | CrySirror state |
|---|---|---|
| `AlphaMovie.dll` | `alpha_movie` (Movie) | AMV/alpha-video native lifetime/timing/blend mapping missing. |
| `DrawDeviceD2D.dll` | `—` (unknown) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `GlitchEffect.dll` | `glitch_effect` (Layer) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `KAGParserEx.dll` | `kag_parser_ex` (Script) | Original KAGParser facade exists; this extended parser is not an alias and is not yet mapped. |
| `PackinOne.dll` | `packinone` (Script) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `SteamDrawDevice.dll` | `steam_draw_device` (Render) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `dmmcloud.dll` | `dmmcloud` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `extNagano.dll` | `extnagano` (Transition) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `extrans.dll` | `extrans` (Transition) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `gamepad.dll` | `gamepad` (Input) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `getLangName.dll` | `get_lang_name` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `getSample.dll` | `get_sample` (Audio) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `httprequest.dll` | `http_request` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `json.dll` | `json` (Script) | Shared document parser/writer and four Scripts mappings implemented in this batch; explicit Japanese ACP/UTF-8 and bounded plain-data profile. |
| `k2compat.dll` | `k2compat` (Script) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `kagexopt.dll` | `kagexopt` (Script) | Reference identifies an option-descriptor-only DLL, not runtime drawing. Descriptor ingestion/options consumer missing; not marked supported. |
| `krkrsteam.dll` | `krkrsteam` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `krmovie.dll` | `krmovie` (Movie) | Shared video primitives exist; this native movie module is not mapped. |
| `layerExDraw.dll` | `layer_ex_draw` (Layer) | Bounded ordered solid-brush Appearance/line Path/RectF and Layer draw/transform facade implemented in second batch; shared VectorPath worker raster. Pen/curve/text/metafile profiles remain unsupported. |
| `layerStwCopy.dll` | `—` (unknown) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `lzfs.dll` | `lzfs` (Archive) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `menu.dll` | `menu` (System) | Existing engine facade: MenuSpec/UI state; native modal popup waits remain unsupported. |
| `motionplayer.dll` | `motion_player` (Movie) | Shared M2 model/clock/worker and dynamic PuppetFrame publication; bounded single-timeline native profile. Two real logo headless paths verified; full masks/actions/physics remain unsupported. |
| `motionplayer_nod3d.dll` | `motion_player` (Movie) | Same shared single-timeline profile as motionplayer; uses CrySirror PuppetRenderer, no native framebuffer. |
| `msdfrender.dll` | `—` (unknown) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `psbfile.dll` | `psb_file` (Script) | Shared resource/puppet PSB foundations exist; PSBFile/PSBValue native object mapping missing. |
| `psd.dll` | `psd` (Layer) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `textrender.dll` | `text_render` (Text) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `toml.dll` | `—` (unknown) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `wfBasicEffect.dll` | `wf_basic_effect` (Audio) | Shared audio DSP is not this native WaveFilter protocol; mapping missing. |
| `wfTypicalDSP.dll` | `wf_typical_dsp` (Audio) | Shared buses/DSP exist; native WaveFilter classes/options/state mapping missing. |
| `win32dialog.dll` | `win32_dialog` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `win32ole.dll` | `win32ole` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `windowEx.dll` | `window_ex` (System) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `wumultitrack.dll` | `—` (unknown) | Native protocol is not mapped; link/calls fail visibly. Shared engine primitives alone do not establish plugin compatibility. |
| `wuopus.dll` | `wuopus` (Audio) | Opus module and decode/options are not yet mapped. |
| `wuvorbis.dll` | `wuvorbis` (Audio) | Shared Vorbis decoding exists; this media module registration/options are not mapped. |
| `wvdecoder.dll` | `—` (unknown) | WavPack media module/decoder not yet mapped. |
| `yuzuex.dll` | `yuzuex` (Archive) | Vendor-specific protocol/recovered reference only; actual game callsites and semantics must be verified. |

## Implemented JSON document profile

Official semantic reference: [`wamsoft/json`](https://github.com/wamsoft/json),
`331577f704d1d64c648646f2a4fdfd4b816f9387`.
`Main.cpp` and `Writer.hpp` were read completely before mapping. No upstream
parser/writer code was copied: the new safe Rust parser/writer lives in
`crysirror-runtime::structured_text` and has tests without a VM. The thin AGPL
`crysirror-krkr::json_plugin` maps TJS data and capability storage only.

The grammar includes comments, single/double quotes, `:`/`=`/`=>`, comma/semicolon
separators, trailing separators, empty array slots, void, signed exact integers
and the reference integer exponent quirk. Quoted/scalar keys are accepted;
bare-word keys are rejected (unlike Kirakira's documented lenient extension).
Malformed Unicode, cycles, callables, nonfinite numbers and excessive inputs
fail. Input is capped at 1 MiB, 65536 nodes and depth 64; output is at most 4 MiB.

Native in-memory parsing/serialization and reads remain synchronous, with explicit
bounds. This is an unfinished ADR 0050 boundary, not full asynchronous JSON I/O.
saveJSON uses the existing shared atomic storage worker and suspends original TJS
until commit; nonresumable callbacks reject before admission. Storage encoding
admits explicit UTF-8 or Japanese CP932, not arbitrary Windows ACP/hidden-member
flag equivalence. Only plain arrays/dictionaries/values are admitted.

## Next extraction targets

1. LayerExDraw: extend ADR 0093's solid-linear profile with shared stroke/cap/join/
   curve/text/image semantics before mapping their native calls.
2. TextRender/MSDF: map native metrics/style/lifetime onto shared Text shaping,
   glyph resources and SDF atlases; no silent empty measurements.
3. Codec modules and WaveFilter DSP: shared decoder/DSP resources first, then
   decoder options, source waits and native class state.
4. Vendor-specific/OS/storefront modules need verified callsites and source or
   native semantics. Missing services must not become fake success shells.

No DLL, signature file, game script, font or game key is copied into this document.
No native binary is loaded or executed.

## Second batch: solid-brush LayerExDraw profile

Official reference: `wtnbgo/layerExDraw` at
`f665a153fa774e47069f53e303329be15975d1e0`, selected complete function scopes and
hashes in krkr-draw-source-audit.json. No C++ or Kirakira rasterizer is imported.

Native GdiPlus.Appearance retains an ordered list of solid ARGB brushes and their
per-brush offsets. GdiPlus.Path retains logical rectangle/polygon/connected-line
figures. GdiPlus.RectF provides the returned x/y/width/height and live left/top/
right/bottom data properties. Layer.drawPath/drawRectangle/drawPolygon/drawLine/
drawLines submit an ordered ImageEffect vector chain as one immutable worker
product, then resume original TJS. Layer.clear uses shared Canvas clear. Scalar
native geometry conversion stays in the adapter; authoring contours, matrix
composition, filled bounds and raster all live in shared image APIs, available
without a VM through runtime::vector and ordinary ImageEffect resources.

The two native transforms follow Append order: path -> brush offset -> transform
-> viewTransform. Returned bounds are the union of all transformed filled-path
bounds, independently of clipping/pixel coverage. Filled polygon/rectangle bounds
are verified as geometry; this does not claim GDI stroke/curve bounds equivalence.
Smoothing modes 2/4 share the existing AA raster policy and preserve requested
state. Disabling auto-update, non-AA modes, records, pens, gradients, fonts/text,
curves/ellipses, arbitrary GDI objects and overridden Layer.update callbacks fail
visibly. Arbitrary composed Canvas readback remains unsupported. These limits are
not silently downgraded to default brushes or empty measurements.

There are at most 256 native Appearance objects and 256 Path objects, 16 brushes
per Appearance and 8192 commands per path. Original native invalidation releases
state even when a script overrides finalize. No pixel buffer lives in this state.
Worker/memory/client/epoch/resource publication boundaries remain ADR 0090's.

See ADR 0093 for the admitted native and raster evidence boundaries.
The shared raster now preserves untouched straight RGBA pixels exactly, avoiding
unrelated premultiply/unpremultiply quantization of semitransparent background
pixels. Independent geometry/pixel regressions and native publication tests are
included. Their final release results are recorded by the integrating owner.
