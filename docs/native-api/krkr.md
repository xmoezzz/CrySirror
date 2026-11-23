# KiriKiri / TJS2 / KAG developer reference

[All engines](README.md)

Mode: `adopt`.

## ABI and host contract

TJS2 bytecode, registered native APIs, KAG parser control tags and KAG/native
class methods have separate namespaces. TJS2 uses register operands and typed
values; property instruction families preserve direct/indexed/property forms.
Native API identity includes the registration receiver. Game-defined TJS
functions, KAG macros and computed property names are open namespaces rather
than a finite builtin syscall catalogue.

| Native area | CrySirror service / implementation | Return / completion |
| --- | --- | --- |
| TJS2 opcodes, values and property calls | `crysirror-tjs2` | Native registers, calls, exceptions and execution budgets. |
| KAG tags, macros, branches and extension loops | `crysirror-kag` / native KAG classes | Script continuation, parameters and current labels remain native. |
| Layers, images, transitions and draw plug-ins | Shared Image/Canvas/Layer/Vector services | Resource/capture completion before native acknowledgement. |
| Font, text, ruby and message UI | Shared Text and retained UI | KAG/native class layout policy stays separate from generic layout. |
| `System.confirm(message, caption?)` | Shared modal message dialog | Suspends the calling TJS frame. Yes returns `1`. No or Escape returns `0`. The initial focus is No. |
| `Rect()` / `Rect(existing)` / `Rect(left, top, right, bottom)` | TJS-owned signed edge value | `width` and `height` derive from edges. The right and bottom edges are exclusive for point tests. `clip` and `union` change the receiver only when the resulting area is positive. `nativeArray` requires native pointer interop. |
| Sound buffers / movie layers | Shared Audio/Media | Native buffer/layer state, callbacks and waits. |
| XP3, plug-in facades and structured saves | Scoped resources, Storage and Documents | Declared native facades; original DLL code is not executed. |

`{name}` in a registration is a parameterized name expression; its loop or
caller supplies the name. Selector rules show its routing body and guards.
The 128-entry TJS2 catalogue is a bytecode namespace, not a total for KAG tags or
native class APIs. Registered names do not imply every argument/plugin variant.
Unsupported plugin, protection and native ABI profiles remain explicit.

Source: [TJS2 VM](../../crates/crysirror-tjs2/src/vm/),
[KAG parser](../../crates/crysirror-kag/src/parser.rs),
[native adapter](../../crates/crysirror-krkr/src/).
Compatibility and supported plug-ins: [KiriKiri guide](../KIRIKIRI.md).
Behavior: [bitmap batch validation](../validation/krkr-bitmap-batches.md).
Native rectangle reference: [KiriKiri Z RectItf.cpp](https://github.com/krkrz/krkrz/blob/fd5c4baa6a2ef5978db1bd043634351f48667daf/visual/RectItf.cpp) and [ComplexRect.cpp](https://github.com/krkrz/krkrz/blob/fd5c4baa6a2ef5978db1bd043634351f48667daf/visual/ComplexRect.cpp).

<!-- BEGIN GENERATED DISPATCH TABLES -->

## Opcode and API implementation tables

`Handled` means an executable source branch exists. It is not a claim that every argument/subselector or a complete game has been audited. Explicit gaps are shown per row.

| Status | Entries |
| --- | ---: |
| Empty handler | 2 |
| Handled | 909 |
| Parameterized | 1 |
| Partial | 2 |
| Unsupported | 111 |

These tables cover the checked-in catalogues and explicit dispatch branches. An engine-wide upstream denominator is not inferred from these rows.

Native API names use the Rust registration receiver when the native class is selected dynamically. The section names identify the registration module and function.

## KAG parser control tags

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `call` | Call a function | Handled | — | [parser.rs:1900](../../crates/crysirror-kag/src/parser.rs#L1900) |
| `else` | else | Handled | — | [parser.rs:1847](../../crates/crysirror-kag/src/parser.rs#L1847) |
| `elsif` | elsif | Handled | — | [parser.rs:1828](../../crates/crysirror-kag/src/parser.rs#L1828) |
| `emb` | emb | Handled | — | [parser.rs:1885](../../crates/crysirror-kag/src/parser.rs#L1885) |
| `endif` | endif | Handled | — | [parser.rs:1854](../../crates/crysirror-kag/src/parser.rs#L1854) |
| `endignore` | endignore | Handled | — | [parser.rs:1858](../../crates/crysirror-kag/src/parser.rs#L1858) |
| `endmacro` | endmacro | Unsupported | endmacro tag without a matching macro tag | [parser.rs:1875](../../crates/crysirror-kag/src/parser.rs#L1875) |
| `erasemacro` | erasemacro | Handled | — | [parser.rs:1878](../../crates/crysirror-kag/src/parser.rs#L1878) |
| `if` | if | Handled | — | [parser.rs:1800](../../crates/crysirror-kag/src/parser.rs#L1800) |
| `ignore` | ignore | Handled | — | [parser.rs:1814](../../crates/crysirror-kag/src/parser.rs#L1814) |
| `jump` | Transfer to another scenario | Handled | — | [parser.rs:1896](../../crates/crysirror-kag/src/parser.rs#L1896) |
| `macro` | macro | Handled | — | [parser.rs:1871](../../crates/crysirror-kag/src/parser.rs#L1871) |
| `macropop` | macropop | Handled | — | [parser.rs:1908](../../crates/crysirror-kag/src/parser.rs#L1908) |
| `return` | return | Handled | — | [parser.rs:1904](../../crates/crysirror-kag/src/parser.rs#L1904) |

## Registered TJS native APIs / alpha_movie / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.FPSRate` | FPSRate | Handled | — | [alpha_movie.rs:224](../../crates/crysirror-krkr/src/alpha_movie.rs#L224) |
| `c.FPSScale` | FPSScale | Handled | — | [alpha_movie.rs:224](../../crates/crysirror-krkr/src/alpha_movie.rs#L224) |
| `c.frame` | frame | Handled | — | [alpha_movie.rs:196](../../crates/crysirror-krkr/src/alpha_movie.rs#L196) |
| `c.numOfFrame` | numOfFrame | Handled | — | [alpha_movie.rs:224](../../crates/crysirror-krkr/src/alpha_movie.rs#L224) |
| `c.screenHeight` | screenHeight | Handled | — | [alpha_movie.rs:224](../../crates/crysirror-krkr/src/alpha_movie.rs#L224) |
| `c.screenWidth` | screenWidth | Handled | — | [alpha_movie.rs:224](../../crates/crysirror-krkr/src/alpha_movie.rs#L224) |

## Registered TJS native APIs / builtins / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `global.Array` | Array | Handled | — | [builtins.rs:17](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L17) |
| `global.Date` | Date | Handled | — | [builtins.rs:23](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L23) |
| `global.Dictionary` | Dictionary | Handled | — | [builtins.rs:19](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L19) |
| `global.Exception` | Exception | Handled | — | [builtins.rs:26](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L26) |
| `global.RegExp` | RegExp | Handled | — | [builtins.rs:21](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L21) |

## Registered TJS native APIs / builtins / install_empty_finalize

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.finalize` | finalize | Handled | — | [builtins.rs:58](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L58) |

## Registered TJS native APIs / builtins / install_math

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `math.RandomGenerator` | RandomGenerator | Handled | — | [builtins.rs:228](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L228) |
| `math.abs` | abs | Handled | — | [builtins.rs:197](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L197) |
| `math.acos` | acos | Handled | — | [builtins.rs:198](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L198) |
| `math.asin` | asin | Handled | — | [builtins.rs:199](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L199) |
| `math.atan` | atan | Handled | — | [builtins.rs:200](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L200) |
| `math.atan2` | atan2 | Handled | — | [builtins.rs:210](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L210) |
| `math.ceil` | ceil | Handled | — | [builtins.rs:201](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L201) |
| `math.cos` | cos | Handled | — | [builtins.rs:207](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L207) |
| `math.exp` | exp | Handled | — | [builtins.rs:202](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L202) |
| `math.floor` | floor | Handled | — | [builtins.rs:203](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L203) |
| `math.log` | log | Handled | — | [builtins.rs:204](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L204) |
| `math.max` | max | Handled | — | [builtins.rs:212](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L212) |
| `math.min` | min | Handled | — | [builtins.rs:213](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L213) |
| `math.pow` | pow | Handled | — | [builtins.rs:211](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L211) |
| `math.random` | random | Handled | — | [builtins.rs:216](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L216) |
| `math.round` | round | Handled | — | [builtins.rs:205](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L205) |
| `math.sin` | sin | Handled | — | [builtins.rs:206](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L206) |
| `math.sqrt` | sqrt | Handled | — | [builtins.rs:208](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L208) |
| `math.tan` | tan | Handled | — | [builtins.rs:209](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L209) |

## Registered TJS native APIs / builtins / native_regexp

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle._compile` | _compile | Handled | — | [builtins.rs:126](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L126) |
| `handle.compile` | compile | Handled | — | [builtins.rs:125](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L125) |
| `handle.exec` | exec | Handled | — | [builtins.rs:140](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L140) |
| `handle.match` | match | Handled | — | [builtins.rs:139](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L139) |
| `handle.test` | test | Handled | — | [builtins.rs:138](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L138) |

## Registered TJS native APIs / builtins / random_generator

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.random` | random | Handled | — | [builtins.rs:352](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L352) |
| `handle.random32` | random32 | Handled | — | [builtins.rs:354](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L354) |
| `handle.random63` | random63 | Handled | — | [builtins.rs:355](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L355) |
| `handle.random64` | random64 | Handled | — | [builtins.rs:356](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L356) |
| `handle.randomize` | randomize | Handled | — | [builtins.rs:353](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L353) |
| `handle.serialize` | serialize | Handled | — | [builtins.rs:357](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L357) |

## Registered TJS native APIs / csv_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.currentLineNumber` | currentLineNumber | Handled | — | [csv_plugin.rs:144](../../crates/crysirror-krkr/src/csv_plugin.rs#L144) |
| `c.parse` | parse | Handled | — | [csv_plugin.rs:140](../../crates/crysirror-krkr/src/csv_plugin.rs#L140) |
| `c.parseStorage` | parseStorage | Handled | — | [csv_plugin.rs:140](../../crates/crysirror-krkr/src/csv_plugin.rs#L140) |

## Registered TJS native APIs / d2d_view / attach

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `layer.viewState` | viewState | Handled | — | [d2d_view.rs:97](../../crates/crysirror-krkr/src/d2d_view.rs#L97) |
| `state.opacity` | opacity | Handled | — | [d2d_view.rs:104](../../crates/crysirror-krkr/src/d2d_view.rs#L104) |
| `state.order` | order | Handled | — | [d2d_view.rs:104](../../crates/crysirror-krkr/src/d2d_view.rs#L104) |
| `state.setRect` | setRect | Handled | — | [d2d_view.rs:134](../../crates/crysirror-krkr/src/d2d_view.rs#L134) |
| `state.visible` | visible | Handled | — | [d2d_view.rs:104](../../crates/crysirror-krkr/src/d2d_view.rs#L104) |
| `state.zinterp` | zinterp | Handled | — | [d2d_view.rs:104](../../crates/crysirror-krkr/src/d2d_view.rs#L104) |

## Registered TJS native APIs / date_api / construct

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.getDate` | getDate | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getDay` | getDay | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getHours` | getHours | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getMinutes` | getMinutes | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getMonth` | getMonth | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getSeconds` | getSeconds | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getTime` | getTime | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getTimezoneOffset` | getTimezoneOffset | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.getYear` | getYear | Handled | — | [date_api.rs:146](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L146) |
| `h.parse` | parse | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setDate` | setDate | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setHours` | setHours | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setMinutes` | setMinutes | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setMonth` | setMonth | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setSeconds` | setSeconds | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setTime` | setTime | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |
| `h.setYear` | setYear | Handled | — | [date_api.rs:176](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L176) |

## Registered TJS native APIs / date_api / install_class

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.getTimezoneOffset` | getTimezoneOffset | Handled | — | [date_api.rs:106](../../crates/crysirror-tjs2/src/runtime/date_api.rs#L106) |

## Registered TJS native APIs / draw_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `layer.clear` | clear | Handled | — | [draw_plugin.rs:496](../../crates/crysirror-krkr/src/draw_plugin.rs#L496) |
| `layer.drawLine` | drawLine | Handled | — | [draw_plugin.rs:492](../../crates/crysirror-krkr/src/draw_plugin.rs#L492) |
| `layer.drawLines` | drawLines | Handled | — | [draw_plugin.rs:492](../../crates/crysirror-krkr/src/draw_plugin.rs#L492) |
| `layer.drawPath` | drawPath | Handled | — | [draw_plugin.rs:492](../../crates/crysirror-krkr/src/draw_plugin.rs#L492) |
| `layer.drawPolygon` | drawPolygon | Handled | — | [draw_plugin.rs:492](../../crates/crysirror-krkr/src/draw_plugin.rs#L492) |
| `layer.drawRectangle` | drawRectangle | Handled | — | [draw_plugin.rs:492](../../crates/crysirror-krkr/src/draw_plugin.rs#L492) |
| `layer.resetTransform` | resetTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.resetViewTransform` | resetViewTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.rotateTransform` | rotateTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.rotateViewTransform` | rotateViewTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.scaleTransform` | scaleTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.scaleViewTransform` | scaleViewTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.setTransform` | setTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.setViewTransform` | setViewTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.translateTransform` | translateTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |
| `layer.translateViewTransform` | translateViewTransform | Handled | — | [draw_plugin.rs:528](../../crates/crysirror-krkr/src/draw_plugin.rs#L528) |

## Registered TJS native APIs / draw_plugin / install_path

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.drawLine` | drawLine | Handled | — | [draw_plugin.rs:380](../../crates/crysirror-krkr/src/draw_plugin.rs#L380) |
| `h.drawLines` | drawLines | Handled | — | [draw_plugin.rs:380](../../crates/crysirror-krkr/src/draw_plugin.rs#L380) |
| `h.drawPolygon` | drawPolygon | Handled | — | [draw_plugin.rs:380](../../crates/crysirror-krkr/src/draw_plugin.rs#L380) |
| `h.drawRectangle` | drawRectangle | Handled | — | [draw_plugin.rs:380](../../crates/crysirror-krkr/src/draw_plugin.rs#L380) |

## Registered TJS native APIs / fftgraph / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `global.drawFFTGraph` | drawFFTGraph | Handled | — | [fftgraph.rs:41](../../crates/crysirror-krkr/src/fftgraph.rs#L41) |

## Registered TJS native APIs / font_class / initialize

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `font.finalize` | finalize | Handled | — | [font_class.rs:21](../../crates/crysirror-krkr/src/font_class.rs#L21) |
| `font.getEscHeightX` | getEscHeightX | Handled | — | [font_class.rs:142](../../crates/crysirror-krkr/src/font_class.rs#L142) |
| `font.getEscHeightY` | getEscHeightY | Handled | — | [font_class.rs:142](../../crates/crysirror-krkr/src/font_class.rs#L142) |
| `font.getEscWidthX` | getEscWidthX | Handled | — | [font_class.rs:142](../../crates/crysirror-krkr/src/font_class.rs#L142) |
| `font.getEscWidthY` | getEscWidthY | Handled | — | [font_class.rs:142](../../crates/crysirror-krkr/src/font_class.rs#L142) |
| `font.getGlyphDrawRect` | getGlyphDrawRect | Handled | — | [font_class.rs:22](../../crates/crysirror-krkr/src/font_class.rs#L22) |
| `font.getList` | getList | Handled | — | [font_class.rs:55](../../crates/crysirror-krkr/src/font_class.rs#L55) |
| `font.getTextHeight` | getTextHeight | Handled | — | [font_class.rs:127](../../crates/crysirror-krkr/src/font_class.rs#L127) |
| `font.getTextWidth` | getTextWidth | Handled | — | [font_class.rs:121](../../crates/crysirror-krkr/src/font_class.rs#L121) |
| `font.mapPrerenderedFont` | mapPrerenderedFont | Handled | — | [font_class.rs:102](../../crates/crysirror-krkr/src/font_class.rs#L102) |
| `font.rasterizer` | rasterizer | Partial | Font.rasterizer={preference}: CrySirror portable outline compatibility profile; native GDI/FreeType raster pixels are not implemented | [font_class.rs:164](../../crates/crysirror-krkr/src/font_class.rs#L164) |
| `font.unmapPrerenderedFont` | unmapPrerenderedFont | Handled | — | [font_class.rs:112](../../crates/crysirror-krkr/src/font_class.rs#L112) |

## Registered TJS native APIs / get_sample / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.getSample` | getSample | Handled | — | [get_sample.rs:96](../../crates/crysirror-krkr/src/get_sample.rs#L96) |
| `class.sampleAhead` | sampleAhead | Handled | — | [get_sample.rs:119](../../crates/crysirror-krkr/src/get_sample.rs#L119) |
| `class.sampleCount` | sampleCount | Handled | — | [get_sample.rs:119](../../crates/crysirror-krkr/src/get_sample.rs#L119) |
| `class.sampleValue` | sampleValue | Unsupported | WaveSoundBuffer.sampleValue is read-only | [get_sample.rs:106](../../crates/crysirror-krkr/src/get_sample.rs#L106) |
| `class.setDefaultAheads` | setDefaultAheads | Handled | — | [get_sample.rs:141](../../crates/crysirror-krkr/src/get_sample.rs#L141) |
| `class.setDefaultCounts` | setDefaultCounts | Handled | — | [get_sample.rs:141](../../crates/crysirror-krkr/src/get_sample.rs#L141) |

## Registered TJS native APIs / http_plugin / members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.abort` | abort | Handled | — | [http_plugin.rs:176](../../crates/crysirror-krkr/src/http_plugin.rs#L176) |
| `h.contentLength` | contentLength | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.contentType` | contentType | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.contentTypeEncoding` | contentTypeEncoding | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.finalize` | finalize | Handled | — | [http_plugin.rs:209](../../crates/crysirror-krkr/src/http_plugin.rs#L209) |
| `h.getAllResponseHeaders` | getAllResponseHeaders | Handled | — | [http_plugin.rs:187](../../crates/crysirror-krkr/src/http_plugin.rs#L187) |
| `h.getResponseHeader` | getResponseHeader | Handled | — | [http_plugin.rs:196](../../crates/crysirror-krkr/src/http_plugin.rs#L196) |
| `h.getResponseText` | getResponseText | Handled | — | [http_plugin.rs:201](../../crates/crysirror-krkr/src/http_plugin.rs#L201) |
| `h.open` | open | Partial | unsupported HTTP method {other} | [http_plugin.rs:131](../../crates/crysirror-krkr/src/http_plugin.rs#L131) |
| `h.readyState` | readyState | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.response` | response | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.responseData` | responseData | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.send` | send | Handled | — | [http_plugin.rs:174](../../crates/crysirror-krkr/src/http_plugin.rs#L174) |
| `h.sendStorage` | sendStorage | Handled | — | [http_plugin.rs:174](../../crates/crysirror-krkr/src/http_plugin.rs#L174) |
| `h.sendStorageSync` | sendStorageSync | Handled | — | [http_plugin.rs:174](../../crates/crysirror-krkr/src/http_plugin.rs#L174) |
| `h.sendSync` | sendSync | Handled | — | [http_plugin.rs:174](../../crates/crysirror-krkr/src/http_plugin.rs#L174) |
| `h.setRequestHeader` | setRequestHeader | Handled | — | [http_plugin.rs:160](../../crates/crysirror-krkr/src/http_plugin.rs#L160) |
| `h.status` | status | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |
| `h.statusText` | statusText | Unsupported | HttpRequest.{name} is read-only | [http_plugin.rs:230](../../crates/crysirror-krkr/src/http_plugin.rs#L230) |

## Registered TJS native APIs / image_api / install_raster

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.copyRaster` | copyRaster | Handled | — | [image_api.rs:456](../../crates/crysirror-krkr/src/image_api.rs#L456) |

## Registered TJS native APIs / json_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `scripts.evalJSON` | evalJSON | Handled | — | [json_plugin.rs:110](../../crates/crysirror-krkr/src/json_plugin.rs#L110) |
| `scripts.evalJSONStorage` | evalJSONStorage | Handled | — | [json_plugin.rs:126](../../crates/crysirror-krkr/src/json_plugin.rs#L126) |
| `scripts.saveJSON` | saveJSON | Handled | — | [json_plugin.rs:157](../../crates/crysirror-krkr/src/json_plugin.rs#L157) |
| `scripts.toJSONString` | toJSONString | Handled | — | [json_plugin.rs:116](../../crates/crysirror-krkr/src/json_plugin.rs#L116) |

## Registered TJS native APIs / json_plugin / install_toml

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `scripts.tomlDecodeFromStorage` | tomlDecodeFromStorage | Handled | — | [json_plugin.rs:198](../../crates/crysirror-krkr/src/json_plugin.rs#L198) |

## Registered TJS native APIs / k2compat / install_touch_mouse_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.enabled` | enabled | Handled | — | [k2compat.rs:59](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L59) |
| `handle.onTouchMouseClick` | onTouchMouseClick | Handled | — | [k2compat.rs:81](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L81) |
| `handle.onTouchMouseDblClick` | onTouchMouseDblClick | Handled | — | [k2compat.rs:81](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L81) |
| `handle.onTouchMouseDown` | onTouchMouseDown | Handled | — | [k2compat.rs:81](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L81) |
| `handle.onTouchMouseMove` | onTouchMouseMove | Handled | — | [k2compat.rs:81](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L81) |
| `handle.onTouchMouseUp` | onTouchMouseUp | Handled | — | [k2compat.rs:81](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L81) |

## Registered TJS native APIs / k2compat / owner_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.registerMessageReceiver` | registerMessageReceiver | Handled | — | [k2compat.rs:136](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L136) |
| `class.show` | show | Handled | — | [k2compat.rs:130](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L130) |
| `class.stayOnTop` | stayOnTop | Handled | — | [k2compat.rs:149](../../crates/crysirror-krkr/src/plugin_ports/k2compat.rs#L149) |

## Registered TJS native APIs / kag / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.callStackDepth` | callStackDepth | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.curLabel` | curLabel | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.curLine` | curLine | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.curLineStr` | curLineStr | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.curPos` | curPos | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.curStorage` | curStorage | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.debugLevel` | debugLevel | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.ignoreCR` | ignoreCR | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.macroParams` | macroParams | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.macros` | macros | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.mp` | mp | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |
| `c.processSpecialTags` | processSpecialTags | Handled | — | [kag.rs:475](../../crates/crysirror-krkr/src/kag.rs#L475) |

## Registered TJS native APIs / kag / install_ex

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.multiLineTagEnabled` | multiLineTagEnabled | Handled | — | [kag.rs:205](../../crates/crysirror-krkr/src/kag.rs#L205) |
| `c.paramMacros` | paramMacros | Unsupported | KAGParser.paramMacros / KAGParserEx parameter macros | [kag.rs:219](../../crates/crysirror-krkr/src/kag.rs#L219) |

## Registered TJS native APIs / kag_ext / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.currentLocalVariables` | currentLocalVariables | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.enableNP` | enableNP | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.fuzzyReturn` | fuzzyReturn | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.ifLevel` | ifLevel | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.lf` | lf | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.localVariables` | localVariables | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.localVariablesDepth` | localVariablesDepth | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.multiLineTagEnabled` | multiLineTagEnabled | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.numericMacroArgumentsEnabled` | numericMacroArgumentsEnabled | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.paramMacros` | paramMacros | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.pmacros` | pmacros | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.prelf` | prelf | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.returnErrorStorage` | returnErrorStorage | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |
| `c.whileStackDepth` | whileStackDepth | Handled | — | [kag_ext.rs:385](../../crates/crysirror-krkr/src/kag_ext.rs#L385) |

## Registered TJS native APIs / line_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.currentLineNumber` | currentLineNumber | Handled | — | [line_plugin.rs:126](../../crates/crysirror-krkr/src/line_plugin.rs#L126) |

## Registered TJS native APIs / motion_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.captureCanvas` | captureCanvas | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.clear` | clear | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.draw` | draw | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.drawAffine` | drawAffine | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.fadeOutTimeline` | fadeOutTimeline | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.findMotion` | findMotion | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.findSource` | findSource | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getCommandList` | getCommandList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getDiffTimelineLabelList` | getDiffTimelineLabelList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLayerGetter` | getLayerGetter | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLayerGetterList` | getLayerGetterList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLayerMotion` | getLayerMotion | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLayerNames` | getLayerNames | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLayerSetter` | getLayerSetter | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getLoopTimeline` | getLoopTimeline | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getMainTimelineLabelList` | getMainTimelineLabelList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getPlayingTimelineInfoList` | getPlayingTimelineInfoList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getSubImageLayers` | getSubImageLayers | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getTimelineBlendRatio` | getTimelineBlendRatio | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getTimelinePlaying` | getTimelinePlaying | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getVariable` | getVariable | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getVariableFrameList` | getVariableFrameList | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.getVariableRange` | getVariableRange | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.initPhysics` | initPhysics | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.isExistMotion` | isExistMotion | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.modifyRoot` | modifyRoot | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.play` | play | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.playTimeline` | playTimeline | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.progress` | progress | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.releaseSyncWait` | releaseSyncWait | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.serialize` | serialize | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setCameraOffset` | setCameraOffset | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setColor` | setColor | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setCoord` | setCoord | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setDrawAffineTranslateMatrix` | setDrawAffineTranslateMatrix | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setRotate` | setRotate | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setScale` | setScale | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setSlant` | setSlant | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setTimelineBlendRatio` | setTimelineBlendRatio | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setVariable` | setVariable | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.setZoom` | setZoom | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.skip` | skip | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.skipToSync` | skipToSync | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.startWind` | startWind | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.stop` | stop | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.stopTimeline` | stopTimeline | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.stopWind` | stopWind | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.unloadUnusedTextures` | unloadUnusedTextures | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `c.unserialize` | unserialize | Unsupported | Motion.Player.{m} / Motion playback mapped onto the shared puppet service | [motion_plugin.rs:381](../../crates/crysirror-krkr/src/motion_plugin.rs#L381) |
| `h.getSubImageLayers` | getSubImageLayers | Handled | — | [motion_plugin.rs:408](../../crates/crysirror-krkr/src/motion_plugin.rs#L408) |

## Registered TJS native APIs / motion_plugin / install_player

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.allplaying` | allplaying | Handled | — | [motion_plugin.rs:524](../../crates/crysirror-krkr/src/motion_plugin.rs#L524) |
| `c.animating` | animating | Handled | — | [motion_plugin.rs:524](../../crates/crysirror-krkr/src/motion_plugin.rs#L524) |
| `c.chara` | chara | Handled | — | [motion_plugin.rs:497](../../crates/crysirror-krkr/src/motion_plugin.rs#L497) |
| `c.colorWeight` | colorWeight | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.completionType` | completionType | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.frameLastTime` | frameLastTime | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.frameLoopTime` | frameLoopTime | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.frameTickCount` | frameTickCount | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.lastTime` | lastTime | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.loopTime` | loopTime | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.maskMode` | maskMode | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.motion` | motion | Handled | — | [motion_plugin.rs:497](../../crates/crysirror-krkr/src/motion_plugin.rs#L497) |
| `c.opacity` | opacity | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.outline` | outline | Handled | — | [motion_plugin.rs:618](../../crates/crysirror-krkr/src/motion_plugin.rs#L618) |
| `c.playing` | playing | Handled | — | [motion_plugin.rs:524](../../crates/crysirror-krkr/src/motion_plugin.rs#L524) |
| `c.resourceManager` | resourceManager | Handled | — | [motion_plugin.rs:598](../../crates/crysirror-krkr/src/motion_plugin.rs#L598) |
| `c.speed` | speed | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.tickCount` | tickCount | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |
| `c.variableKeys` | variableKeys | Handled | — | [motion_plugin.rs:611](../../crates/crysirror-krkr/src/motion_plugin.rs#L611) |
| `c.varibleKeys` | varibleKeys | Handled | — | [motion_plugin.rs:611](../../crates/crysirror-krkr/src/motion_plugin.rs#L611) |
| `c.visible` | visible | Handled | — | [motion_plugin.rs:548](../../crates/crysirror-krkr/src/motion_plugin.rs#L548) |

## Registered TJS native APIs / msdf_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `scripts.makeColorTable` | makeColorTable | Handled | — | [msdf_plugin.rs:166](../../crates/crysirror-krkr/src/msdf_plugin.rs#L166) |

## Registered TJS native APIs / multitrack / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `wave.rebindMultiTrackID` | rebindMultiTrackID | Handled | — | [multitrack.rs:453](../../crates/crysirror-krkr/src/multitrack.rs#L453) |
| `wave.setDelayMultiTrackMask` | setDelayMultiTrackMask | Handled | — | [multitrack.rs:415](../../crates/crysirror-krkr/src/multitrack.rs#L415) |
| `wave.setGainQueryCallback_mtbiOPUS` | setGainQueryCallback_mtbiOPUS | Handled | — | [multitrack.rs:474](../../crates/crysirror-krkr/src/multitrack.rs#L474) |
| `wave.setGainQueryCallback_mtbiVORBIS` | setGainQueryCallback_mtbiVORBIS | Handled | — | [multitrack.rs:474](../../crates/crysirror-krkr/src/multitrack.rs#L474) |
| `wave.setMultiTrackMask` | setMultiTrackMask | Handled | — | [multitrack.rs:403](../../crates/crysirror-krkr/src/multitrack.rs#L403) |

## Registered TJS native APIs / native_bitmap / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.height` | height | Handled | — | [native_bitmap.rs:149](../../crates/crysirror-krkr/src/native_bitmap.rs#L149) |
| `c.width` | width | Handled | — | [native_bitmap.rs:149](../../crates/crysirror-krkr/src/native_bitmap.rs#L149) |

## Registered TJS native APIs / native_pad / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.borderStyle` | borderStyle | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.color` | color | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fileName` | fileName | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontBold` | fontBold | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontColor` | fontColor | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontFace` | fontFace | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontHeight` | fontHeight | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontItalic` | fontItalic | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontSize` | fontSize | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontStrikeOut` | fontStrikeOut | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.fontUnderline` | fontUnderline | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.height` | height | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.left` | left | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.opacity` | opacity | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.readOnly` | readOnly | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.showScrollBars` | showScrollBars | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.showStatusBar` | showStatusBar | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.statusText` | statusText | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.text` | text | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.title` | title | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.top` | top | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.visible` | visible | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.width` | width | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |
| `c.wordWrap` | wordWrap | Handled | — | [native_pad.rs:176](../../crates/crysirror-krkr/src/native_pad.rs#L176) |

## Registered TJS native APIs / native_rect / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.addOffset` | addOffset | Handled | — | [native_rect.rs:167](../../crates/crysirror-krkr/src/native_rect.rs#L167) |
| `class.clear` | clear | Handled | — | [native_rect.rs:167](../../crates/crysirror-krkr/src/native_rect.rs#L167) |
| `class.clip` | clip | Handled | — | [native_rect.rs:199](../../crates/crysirror-krkr/src/native_rect.rs#L199) |
| `class.equal` | equal | Handled | — | [native_rect.rs:199](../../crates/crysirror-krkr/src/native_rect.rs#L199) |
| `class.included` | included | Handled | — | [native_rect.rs:199](../../crates/crysirror-krkr/src/native_rect.rs#L199) |
| `class.includedPos` | includedPos | Handled | — | [native_rect.rs:148](../../crates/crysirror-krkr/src/native_rect.rs#L148) |
| `class.intersects` | intersects | Handled | — | [native_rect.rs:199](../../crates/crysirror-krkr/src/native_rect.rs#L199) |
| `class.isEmpty` | isEmpty | Handled | — | [native_rect.rs:137](../../crates/crysirror-krkr/src/native_rect.rs#L137) |
| `class.nativeArray` | nativeArray | Handled | — | [native_rect.rs:131](../../crates/crysirror-krkr/src/native_rect.rs#L131) |
| `class.set` | set | Handled | — | [native_rect.rs:167](../../crates/crysirror-krkr/src/native_rect.rs#L167) |
| `class.setOffset` | setOffset | Handled | — | [native_rect.rs:167](../../crates/crysirror-krkr/src/native_rect.rs#L167) |
| `class.setSize` | setSize | Handled | — | [native_rect.rs:167](../../crates/crysirror-krkr/src/native_rect.rs#L167) |
| `class.union` | union | Handled | — | [native_rect.rs:199](../../crates/crysirror-krkr/src/native_rect.rs#L199) |
| `class.{name}` | {name} | Parameterized | Registration name supplied by the caller; inspect the linked registration function | [native_rect.rs:101](../../crates/crysirror-krkr/src/native_rect.rs#L101) |

## Registered TJS native APIs / natives / draw_device_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.interface` | interface | Unsupported | property is read-only | [natives.rs:1123](../../crates/crysirror-krkr/src/natives.rs#L1123) |

## Registered TJS native APIs / natives / font_properties

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `font.angle` | angle | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.bold` | bold | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.face` | face | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.faceIsFileName` | faceIsFileName | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.height` | height | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.italic` | italic | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.strikeout` | strikeout | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |
| `font.underline` | underline | Handled | — | [natives.rs:1708](../../crates/crysirror-krkr/src/natives.rs#L1708) |

## Registered TJS native APIs / natives / install_layer

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.attentionLeft` | attentionLeft | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.attentionTop` | attentionTop | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.callOnPaint` | callOnPaint | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.enabled` | enabled | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.face` | face | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.focusable` | focusable | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.font` | font | Unsupported | property is read-only | [natives.rs:1898](../../crates/crysirror-krkr/src/natives.rs#L1898) |
| `class.hitThreshold` | hitThreshold | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.hitType` | hitType | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.holdAlpha` | holdAlpha | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.ignoreHintSensing` | ignoreHintSensing | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.imeMode` | imeMode | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.joinFocusChain` | joinFocusChain | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.neutralColor` | neutralColor | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.showParentHint` | showParentHint | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |
| `class.type` | type | Handled | — | [natives.rs:1901](../../crates/crysirror-krkr/src/natives.rs#L1901) |
| `class.useAttention` | useAttention | Handled | — | [natives.rs:1932](../../crates/crysirror-krkr/src/natives.rs#L1932) |

## Registered TJS native APIs / natives / install_system

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.drawThreadNum` | drawThreadNum | Handled | — | [natives.rs:611](../../crates/crysirror-krkr/src/natives.rs#L611) |
| `h.title` | title | Handled | — | [natives.rs:629](../../crates/crysirror-krkr/src/natives.rs#L629) |

## Registered TJS native APIs / natives / install_window

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.HWND` | HWND | Unsupported | property is read-only | [natives.rs:1195](../../crates/crysirror-krkr/src/natives.rs#L1195) |
| `class.borderStyle` | borderStyle | Handled | — | [natives.rs:1404](../../crates/crysirror-krkr/src/natives.rs#L1404) |
| `class.caption` | caption | Handled | — | [natives.rs:1369](../../crates/crysirror-krkr/src/natives.rs#L1369) |
| `class.drawDevice` | drawDevice | Handled | — | [natives.rs:1203](../../crates/crysirror-krkr/src/natives.rs#L1203) |
| `class.focusedLayer` | focusedLayer | Handled | — | [natives.rs:1285](../../crates/crysirror-krkr/src/natives.rs#L1285) |
| `class.fullScreen` | fullScreen | Handled | — | [natives.rs:1384](../../crates/crysirror-krkr/src/natives.rs#L1384) |
| `class.innerSunken` | innerSunken | Handled | — | [natives.rs:1235](../../crates/crysirror-krkr/src/natives.rs#L1235) |
| `class.layerLeft` | layerLeft | Handled | — | [natives.rs:1309](../../crates/crysirror-krkr/src/natives.rs#L1309) |
| `class.layerTop` | layerTop | Handled | — | [natives.rs:1309](../../crates/crysirror-krkr/src/natives.rs#L1309) |
| `class.left` | left | Handled | — | [natives.rs:1359](../../crates/crysirror-krkr/src/natives.rs#L1359) |
| `class.maxHeight` | maxHeight | Handled | — | [natives.rs:1262](../../crates/crysirror-krkr/src/natives.rs#L1262) |
| `class.maxWidth` | maxWidth | Handled | — | [natives.rs:1262](../../crates/crysirror-krkr/src/natives.rs#L1262) |
| `class.minHeight` | minHeight | Handled | — | [natives.rs:1262](../../crates/crysirror-krkr/src/natives.rs#L1262) |
| `class.minWidth` | minWidth | Handled | — | [natives.rs:1262](../../crates/crysirror-krkr/src/natives.rs#L1262) |
| `class.setMaxSize` | setMaxSize | Handled | — | [natives.rs:1277](../../crates/crysirror-krkr/src/natives.rs#L1277) |
| `class.setMinSize` | setMinSize | Handled | — | [natives.rs:1277](../../crates/crysirror-krkr/src/natives.rs#L1277) |
| `class.showScrollBars` | showScrollBars | Handled | — | [natives.rs:1235](../../crates/crysirror-krkr/src/natives.rs#L1235) |
| `class.top` | top | Handled | — | [natives.rs:1359](../../crates/crysirror-krkr/src/natives.rs#L1359) |
| `class.zoomDenom` | zoomDenom | Handled | — | [natives.rs:1334](../../crates/crysirror-krkr/src/natives.rs#L1334) |
| `class.zoomNumer` | zoomNumer | Handled | — | [natives.rs:1334](../../crates/crysirror-krkr/src/natives.rs#L1334) |

## Registered TJS native APIs / natives / methods

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.add` | add | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.addAutoPath` | addAutoPath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.addContinuousHandler` | addContinuousHandler | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.addFont` | addFont | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.adjustGamma` | adjustGamma | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.affineCopy` | affineCopy | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.assignImages` | assignImages | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.bringToFront` | bringToFront | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.chopStorageExt` | chopStorageExt | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.clipAlphaRect` | clipAlphaRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.close` | close | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.colorRect` | colorRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.confirm` | confirm | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.convertType` | convertType | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.copyAlphaToProvince` | copyAlphaToProvince | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.copyBottomBlueToTopAlpha` | copyBottomBlueToTopAlpha | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.copyRect` | copyRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.copyRightBlueToLeftAlpha` | copyRightBlueToLeftAlpha | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.createAppLock` | createAppLock | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.createUUID` | createUUID | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.doBoxBlur` | doBoxBlur | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.doCompact` | doCompact | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.doGrayScale` | doGrayScale | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.drawText` | drawText | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.eval` | eval | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.evalStorage` | evalStorage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.exec` | exec | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.execStorage` | execStorage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.exit` | exit | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.extractStorageExt` | extractStorageExt | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.extractStorageName` | extractStorageName | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.extractStoragePath` | extractStoragePath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.fillAlpha` | fillAlpha | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.fillByProvince` | fillByProvince | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.fillRect` | fillRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.fillToProvince` | fillToProvince | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.finalize` | finalize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.flipLR` | flipLR | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.flipUD` | flipUD | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.focus` | focus | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.freeImage` | freeImage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getArgument` | getArgument | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getClassNames` | getClassNames | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getFullPath` | getFullPath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getLayerAt` | getLayerAt | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getLocalName` | getLocalName | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getMainPixel` | getMainPixel | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getPlacedPath` | getPlacedPath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getProvincePixel` | getProvincePixel | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getTickCount` | getTickCount | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.getTraceString` | getTraceString | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.hideMouseCursor` | hideMouseCursor | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.inform` | inform | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.isExistentStorage` | isExistentStorage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.loadDataPack` | loadDataPack | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.loadImages` | loadImages | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.loadProvinceImage` | loadProvinceImage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.logAsError` | logAsError | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.message` | message | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.notice` | notice | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onActivate` | onActivate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onBlur` | onBlur | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onClick` | onClick | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onCloseQuery` | onCloseQuery | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onDeactivate` | onDeactivate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onDisplayRotate` | onDisplayRotate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onDoubleClick` | onDoubleClick | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onFileDrop` | onFileDrop | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onFocus` | onFocus | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onHintChanged` | onHintChanged | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onHitTest` | onHitTest | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onKeyDown` | onKeyDown | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onKeyPress` | onKeyPress | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onKeyUp` | onKeyUp | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseDown` | onMouseDown | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseEnter` | onMouseEnter | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseLeave` | onMouseLeave | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseMove` | onMouseMove | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseUp` | onMouseUp | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMouseWheel` | onMouseWheel | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onMultiTouch` | onMultiTouch | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onNodeDisabled` | onNodeDisabled | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onNodeEnabled` | onNodeEnabled | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onPaint` | onPaint | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onPopupHide` | onPopupHide | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onResize` | onResize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTimer` | onTimer | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTouchDown` | onTouchDown | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTouchMove` | onTouchMove | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTouchRotate` | onTouchRotate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTouchScaling` | onTouchScaling | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.onTouchUp` | onTouchUp | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.operateAffine` | operateAffine | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.operateRect` | operateRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.operateStretch` | operateStretch | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.overwrapRect` | overwrapRect | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.piledCopy` | piledCopy | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.readRegValue` | readRegValue | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.recreate` | recreate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.registerHotKey` | registerHotKey | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.releaseCapture` | releaseCapture | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.remove` | remove | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.removeAutoPath` | removeAutoPath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.removeContinuousHandler` | removeContinuousHandler | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.removeMode` | removeMode | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.saveLayerImage` | saveLayerImage | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.searchCD` | searchCD | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.sendToBack` | sendToBack | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setArgument` | setArgument | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setCallMissing` | setCallMissing | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setClip` | setClip | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setDefaultPath` | setDefaultPath | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setImagePos` | setImagePos | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setImageSize` | setImageSize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setInnerSize` | setInnerSize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setLayerPos` | setLayerPos | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setMainPixel` | setMainPixel | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setMaskRegion` | setMaskRegion | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setMode` | setMode | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setPos` | setPos | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setSize` | setSize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setSizeToImageSize` | setSizeToImageSize | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.setZoom` | setZoom | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.shellExecute` | shellExecute | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.showModal` | showModal | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.shrinkCopy` | shrinkCopy | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.shrinkCopyFast` | shrinkCopyFast | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.startLogToFile` | startLogToFile | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.stretchCopy` | stretchCopy | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.terminate` | terminate | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.toActualColor` | toActualColor | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.touchImages` | touchImages | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.unregisterHotKey` | unregisterHotKey | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |
| `h.update` | update | Handled | — | [natives.rs:125](../../crates/crysirror-krkr/src/natives.rs#L125) |

## Registered TJS native APIs / natives / properties

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.absolute` | absolute | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.absoluteOrderMode` | absoluteOrderMode | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.appDataPath` | appDataPath | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.capacity` | capacity | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.children` | children | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.clipHeight` | clipHeight | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.clipLeft` | clipLeft | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.clipTop` | clipTop | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.clipWidth` | clipWidth | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.dataPath` | dataPath | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.desktopHeight` | desktopHeight | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.desktopLeft` | desktopLeft | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.desktopTop` | desktopTop | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.desktopWidth` | desktopWidth | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.enabled` | enabled | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.exeName` | exeName | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.exePath` | exePath | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.focused` | focused | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.hasImage` | hasImage | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.height` | height | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.hint` | hint | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.ignoreHintSensing` | ignoreHintSensing | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.imageHeight` | imageHeight | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.imageLeft` | imageLeft | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.imageModified` | imageModified | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.imageTop` | imageTop | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.imageWidth` | imageWidth | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.innerHeight` | innerHeight | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.innerWidth` | innerWidth | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.interval` | interval | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.isPrimary` | isPrimary | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.left` | left | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.menu` | menu | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.mode` | mode | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.nodeEnabled` | nodeEnabled | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.nodeVisible` | nodeVisible | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.numChildren` | numChildren | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.opacity` | opacity | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.order` | order | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.osName` | osName | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.parent` | parent | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.personalPath` | personalPath | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.platformName` | platformName | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.primaryLayer` | primaryLayer | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.screenHeight` | screenHeight | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.screenWidth` | screenWidth | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.showParentHint` | showParentHint | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.top` | top | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.versionInformation` | versionInformation | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.versionString` | versionString | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.visible` | visible | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.width` | width | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |
| `h.window` | window | Handled | — | [natives.rs:136](../../crates/crysirror-krkr/src/natives.rs#L136) |

## Registered TJS native APIs / offline_plugins / install_draw_device

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.finalize` | finalize | Handled | — | [offline_plugins.rs:162](../../crates/crysirror-krkr/src/offline_plugins.rs#L162) |

## Registered TJS native APIs / offline_plugins / install_steam

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.achievementsCount` | achievementsCount | Unsupported | Steam.{name} is read-only | [offline_plugins.rs:101](../../crates/crysirror-krkr/src/offline_plugins.rs#L101) |
| `class.clearAchievement` | clearAchievement | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.cloudEnabled` | cloudEnabled | Handled | — | [offline_plugins.rs:106](../../crates/crysirror-krkr/src/offline_plugins.rs#L106) |
| `class.cloudFileCount` | cloudFileCount | Unsupported | Steam.{name} is read-only | [offline_plugins.rs:101](../../crates/crysirror-krkr/src/offline_plugins.rs#L101) |
| `class.copyCloudFile` | copyCloudFile | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.deleteCloudFile` | deleteCloudFile | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.getAchievement` | getAchievement | Handled | — | [offline_plugins.rs:91](../../crates/crysirror-krkr/src/offline_plugins.rs#L91) |
| `class.getCloudFileInfo` | getCloudFileInfo | Handled | — | [offline_plugins.rs:95](../../crates/crysirror-krkr/src/offline_plugins.rs#L95) |
| `class.getCloudQuota` | getCloudQuota | Handled | — | [offline_plugins.rs:93](../../crates/crysirror-krkr/src/offline_plugins.rs#L93) |
| `class.getLanguage` | getLanguage | Handled | — | [offline_plugins.rs:97](../../crates/crysirror-krkr/src/offline_plugins.rs#L97) |
| `class.hookBroadcasting` | hookBroadcasting | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.hookScreenshots` | hookScreenshots | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.initialized` | initialized | Unsupported | Steam.{name} is read-only | [offline_plugins.rs:101](../../crates/crysirror-krkr/src/offline_plugins.rs#L101) |
| `class.isBroadcasting` | isBroadcasting | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.requestInitialize` | requestInitialize | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.setAchievement` | setAchievement | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.triggerScreenshot` | triggerScreenshot | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |
| `class.writeScreenshot` | writeScreenshot | Handled | — | [offline_plugins.rs:89](../../crates/crysirror-krkr/src/offline_plugins.rs#L89) |

## Registered TJS native APIs / offline_plugins / members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.close_ime` | close_ime | Handled | — | [offline_plugins.rs:43](../../crates/crysirror-krkr/src/offline_plugins.rs#L43) |
| `h.finalize` | finalize | Handled | — | [offline_plugins.rs:45](../../crates/crysirror-krkr/src/offline_plugins.rs#L45) |
| `h.getController` | getController | Handled | — | [offline_plugins.rs:319](../../crates/crysirror-krkr/src/offline_plugins.rs#L319) |
| `h.initialize` | initialize | Handled | — | [offline_plugins.rs:317](../../crates/crysirror-krkr/src/offline_plugins.rs#L317) |
| `h.launch_ime` | launch_ime | Handled | — | [offline_plugins.rs:43](../../crates/crysirror-krkr/src/offline_plugins.rs#L43) |
| `h.new_tab` | new_tab | Handled | — | [offline_plugins.rs:43](../../crates/crysirror-krkr/src/offline_plugins.rs#L43) |
| `h.new_window` | new_window | Handled | — | [offline_plugins.rs:43](../../crates/crysirror-krkr/src/offline_plugins.rs#L43) |
| `h.open_store` | open_store | Handled | — | [offline_plugins.rs:43](../../crates/crysirror-krkr/src/offline_plugins.rs#L43) |
| `h.update` | update | Handled | — | [offline_plugins.rs:317](../../crates/crysirror-krkr/src/offline_plugins.rs#L317) |

## Registered TJS native APIs / offline_plugins / ole_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.addEvent` | addEvent | Handled | — | [offline_plugins.rs:241](../../crates/crysirror-krkr/src/offline_plugins.rs#L241) |
| `h.finalize` | finalize | Handled | — | [offline_plugins.rs:263](../../crates/crysirror-krkr/src/offline_plugins.rs#L263) |
| `h.get` | get | Unsupported | Unknown failure : FFFFFFFF | [offline_plugins.rs:232](../../crates/crysirror-krkr/src/offline_plugins.rs#L232) |
| `h.getConstant` | getConstant | Handled | — | [offline_plugins.rs:253](../../crates/crysirror-krkr/src/offline_plugins.rs#L253) |
| `h.height` | height | Handled | — | [offline_plugins.rs:282](../../crates/crysirror-krkr/src/offline_plugins.rs#L282) |
| `h.invoke` | invoke | Unsupported | Unknown failure : FFFFFFFF | [offline_plugins.rs:232](../../crates/crysirror-krkr/src/offline_plugins.rs#L232) |
| `h.isValidWindow` | isValidWindow | Unsupported | ActiveX.isValidWindow is read-only | [offline_plugins.rs:298](../../crates/crysirror-krkr/src/offline_plugins.rs#L298) |
| `h.left` | left | Handled | — | [offline_plugins.rs:282](../../crates/crysirror-krkr/src/offline_plugins.rs#L282) |
| `h.missing` | missing | Handled | — | [offline_plugins.rs:237](../../crates/crysirror-krkr/src/offline_plugins.rs#L237) |
| `h.set` | set | Unsupported | Unknown failure : FFFFFFFF | [offline_plugins.rs:232](../../crates/crysirror-krkr/src/offline_plugins.rs#L232) |
| `h.setExternalUI` | setExternalUI | Handled | — | [offline_plugins.rs:267](../../crates/crysirror-krkr/src/offline_plugins.rs#L267) |
| `h.setPos` | setPos | Handled | — | [offline_plugins.rs:272](../../crates/crysirror-krkr/src/offline_plugins.rs#L272) |
| `h.setSize` | setSize | Handled | — | [offline_plugins.rs:272](../../crates/crysirror-krkr/src/offline_plugins.rs#L272) |
| `h.top` | top | Handled | — | [offline_plugins.rs:282](../../crates/crysirror-krkr/src/offline_plugins.rs#L282) |
| `h.visible` | visible | Handled | — | [offline_plugins.rs:297](../../crates/crysirror-krkr/src/offline_plugins.rs#L297) |
| `h.width` | width | Handled | — | [offline_plugins.rs:282](../../crates/crysirror-krkr/src/offline_plugins.rs#L282) |

## Registered TJS native APIs / packinone / install_data_pack

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `target.loadDataPack` | loadDataPack | Handled | — | [packinone.rs:126](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L126) |
| `target.makeDataPackDigest` | makeDataPackDigest | Handled | — | [packinone.rs:124](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L124) |
| `target.makeDataPackThumb` | makeDataPackThumb | Handled | — | [packinone.rs:123](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L123) |
| `target.saveDataPack` | saveDataPack | Handled | — | [packinone.rs:122](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L122) |

## Registered TJS native APIs / packinone / install_storages_octet

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `storages.loadOctet` | loadOctet | Handled | — | [packinone.rs:56](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L56) |
| `storages.saveOctet` | saveOctet | Handled | — | [packinone.rs:55](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L55) |

## Registered TJS native APIs / plugins / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `global.CanLoadPlugin` | CanLoadPlugin | Handled | — | [plugins.rs:214](../../crates/crysirror-krkr/src/plugins.rs#L214) |

## Registered TJS native APIs / pointer / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.cursor` | cursor | Handled | — | [pointer.rs:66](../../crates/crysirror-krkr/src/pointer.rs#L66) |
| `c.cursorX` | cursorX | Handled | — | [pointer.rs:116](../../crates/crysirror-krkr/src/pointer.rs#L116) |
| `c.cursorY` | cursorY | Handled | — | [pointer.rs:116](../../crates/crysirror-krkr/src/pointer.rs#L116) |

## Registered TJS native APIs / protect_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `storages.archiveUniqueKey` | archiveUniqueKey | Handled | — | [protect_plugin.rs:196](../../crates/crysirror-krkr/src/protect_plugin.rs#L196) |

## Registered TJS native APIs / psbfile_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.root` | root | Handled | — | [psbfile_plugin.rs:117](../../crates/crysirror-krkr/src/psbfile_plugin.rs#L117) |

## Registered TJS native APIs / psd_plugin / members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `h.channels` | channels | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |
| `h.clearStorageCache` | clearStorageCache | Handled | — | [psd_plugin.rs:450](../../crates/crysirror-krkr/src/psd_plugin.rs#L450) |
| `h.color_mode` | color_mode | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |
| `h.depth` | depth | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |
| `h.getBlend` | getBlend | Handled | — | [psd_plugin.rs:500](../../crates/crysirror-krkr/src/psd_plugin.rs#L500) |
| `h.getGuides` | getGuides | Handled | — | [psd_plugin.rs:518](../../crates/crysirror-krkr/src/psd_plugin.rs#L518) |
| `h.getLayerComp` | getLayerComp | Handled | — | [psd_plugin.rs:568](../../crates/crysirror-krkr/src/psd_plugin.rs#L568) |
| `h.getLayerData` | getLayerData | Handled | — | [psd_plugin.rs:491](../../crates/crysirror-krkr/src/psd_plugin.rs#L491) |
| `h.getLayerDataMask` | getLayerDataMask | Handled | — | [psd_plugin.rs:497](../../crates/crysirror-krkr/src/psd_plugin.rs#L497) |
| `h.getLayerDataRaw` | getLayerDataRaw | Handled | — | [psd_plugin.rs:494](../../crates/crysirror-krkr/src/psd_plugin.rs#L494) |
| `h.getLayerInfo` | getLayerInfo | Handled | — | [psd_plugin.rs:486](../../crates/crysirror-krkr/src/psd_plugin.rs#L486) |
| `h.getLayerName` | getLayerName | Handled | — | [psd_plugin.rs:481](../../crates/crysirror-krkr/src/psd_plugin.rs#L481) |
| `h.getLayerType` | getLayerType | Handled | — | [psd_plugin.rs:476](../../crates/crysirror-krkr/src/psd_plugin.rs#L476) |
| `h.getSlices` | getSlices | Handled | — | [psd_plugin.rs:531](../../crates/crysirror-krkr/src/psd_plugin.rs#L531) |
| `h.height` | height | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |
| `h.layer_count` | layer_count | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |
| `h.load` | load | Handled | — | [psd_plugin.rs:431](../../crates/crysirror-krkr/src/psd_plugin.rs#L431) |
| `h.width` | width | Unsupported | PSD.{name} is read-only | [psd_plugin.rs:466](../../crates/crysirror-krkr/src/psd_plugin.rs#L466) |

## Registered TJS native APIs / save_struct / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.saveStruct` | saveStruct | Handled | — | [save_struct.rs:18](../../crates/crysirror-krkr/src/save_struct.rs#L18) |

## Registered TJS native APIs / scripts_ex / install_scripts_ex

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `scripts.clone` | clone | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.equalStruct` | equalStruct | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.equalStructNumericLoose` | equalStructNumericLoose | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.foreach` | foreach | Handled | — | [scripts_ex.rs:135](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L135) |
| `scripts.getMD5HashString` | getMD5HashString | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.getObjectContext` | getObjectContext | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.getObjectCount` | getObjectCount | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.getObjectKeys` | getObjectKeys | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.isNullContext` | isNullContext | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |
| `scripts.rehash` | rehash | Handled | — | [scripts_ex.rs:121](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L121) |

## Registered TJS native APIs / sound / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.frequency` | frequency | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.globalVolume` | globalVolume | Handled | — | [sound.rs:586](../../crates/crysirror-krkr/src/sound.rs#L586) |
| `c.labels` | labels | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.looping` | looping | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.pan` | pan | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.paused` | paused | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.position` | position | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.samplePosition` | samplePosition | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.status` | status | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.totalSamples` | totalSamples | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.totalTime` | totalTime | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.useVisBuffer` | useVisBuffer | Handled | — | [sound.rs:274](../../crates/crysirror-krkr/src/sound.rs#L274) |
| `c.volume` | volume | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |
| `c.volume2` | volume2 | Handled | — | [sound.rs:446](../../crates/crysirror-krkr/src/sound.rs#L446) |

## Registered TJS native APIs / sound / install_phase_vocoder

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.finalize` | finalize | Handled | — | [sound.rs:208](../../crates/crysirror-krkr/src/sound.rs#L208) |
| `c.overlap` | overlap | Handled | — | [sound.rs:194](../../crates/crysirror-krkr/src/sound.rs#L194) |
| `c.window` | window | Handled | — | [sound.rs:180](../../crates/crysirror-krkr/src/sound.rs#L180) |

## Registered TJS native APIs / storage_plugins / install_dirlist

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `global.getDirList` | getDirList | Handled | — | [storage_plugins.rs:221](../../crates/crysirror-krkr/src/storage_plugins.rs#L221) |

## Registered TJS native APIs / storage_plugins / install_fstat

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `storages.changeDirectory` | changeDirectory | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.clearStorageCaches` | clearStorageCaches | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.copyFile` | copyFile | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.copyFileNoNormalize` | copyFileNoNormalize | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.createDirectoryNoNormalize` | createDirectoryNoNormalize | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.dirlist` | dirlist | Handled | — | [storage_plugins.rs:87](../../crates/crysirror-krkr/src/storage_plugins.rs#L87) |
| `storages.dirlistEx` | dirlistEx | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.exportFile` | exportFile | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.fstat` | fstat | Handled | — | [storage_plugins.rs:116](../../crates/crysirror-krkr/src/storage_plugins.rs#L116) |
| `storages.getDisplayName` | getDisplayName | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.getFileAttributes` | getFileAttributes | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.getLastModifiedFileTime` | getLastModifiedFileTime | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.getMD5HashString` | getMD5HashString | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.getTemporaryName` | getTemporaryName | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.getTime` | getTime | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.isExistentDirectory` | isExistentDirectory | Handled | — | [storage_plugins.rs:104](../../crates/crysirror-krkr/src/storage_plugins.rs#L104) |
| `storages.removeDirectory` | removeDirectory | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.resetFileAttributes` | resetFileAttributes | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.searchPath` | searchPath | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.selectDirectory` | selectDirectory | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.setFileAttributes` | setFileAttributes | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.setLastModifiedFileTime` | setLastModifiedFileTime | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.setTime` | setTime | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |
| `storages.truncateFile` | truncateFile | Unsupported | scoped fstat operation with native completion semantics | [storage_plugins.rs:207](../../crates/crysirror-krkr/src/storage_plugins.rs#L207) |

## Registered TJS native APIs / storage_plugins / install_save_struct

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `array.save2` | save2 | Handled | — | [storage_plugins.rs:18](../../crates/crysirror-krkr/src/storage_plugins.rs#L18) |
| `class.saveStruct2` | saveStruct2 | Unsupported | saveStruct2 typed expression writer | [storage_plugins.rs:73](../../crates/crysirror-krkr/src/storage_plugins.rs#L73) |
| `class.toStructString` | toStructString | Unsupported | saveStruct2 typed expression writer | [storage_plugins.rs:73](../../crates/crysirror-krkr/src/storage_plugins.rs#L73) |

## Registered TJS native APIs / surfaces / define_class

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.finalize` | finalize | Handled | — | [surfaces.rs:59](../../crates/crysirror-krkr/src/surfaces.rs#L59) |

## Registered TJS native APIs / system_ex / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `layer.fetchImageSize` | fetchImageSize | Handled | — | [system_ex.rs:49](../../crates/crysirror-krkr/src/system_ex.rs#L49) |
| `system.commandExecute` | commandExecute | Handled | — | [system_ex.rs:36](../../crates/crysirror-krkr/src/system_ex.rs#L36) |
| `system.setDpiAwareness` | setDpiAwareness | Handled | — | [system_ex.rs:26](../../crates/crysirror-krkr/src/system_ex.rs#L26) |
| `system.waitForAppLock` | waitForAppLock | Handled | — | [system_ex.rs:23](../../crates/crysirror-krkr/src/system_ex.rs#L23) |

## Registered TJS native APIs / system_plugins / install_lang_name

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `system.getCurrentLocaleName` | getCurrentLocaleName | Handled | — | [system_plugins.rs:25](../../crates/crysirror-krkr/src/system_plugins.rs#L25) |
| `system.getCurrentUILangName` | getCurrentUILangName | Handled | — | [system_plugins.rs:25](../../crates/crysirror-krkr/src/system_plugins.rs#L25) |

## Registered TJS native APIs / text_render_plugin / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.fontScale` | fontScale | Handled | — | [text_render_plugin.rs:353](../../crates/crysirror-krkr/src/text_render_plugin.rs#L353) |
| `c.maxScrollLine` | maxScrollLine | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.maxScrollOffset` | maxScrollOffset | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderBottom` | renderBottom | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderCount` | renderCount | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderDelay` | renderDelay | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderLeft` | renderLeft | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderLines` | renderLines | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderOver` | renderOver | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderRight` | renderRight | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderText` | renderText | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.renderTop` | renderTop | Handled | — | [text_render_plugin.rs:379](../../crates/crysirror-krkr/src/text_render_plugin.rs#L379) |
| `c.timeScale` | timeScale | Handled | — | [text_render_plugin.rs:353](../../crates/crysirror-krkr/src/text_render_plugin.rs#L353) |
| `c.vertical` | vertical | Handled | — | [text_render_plugin.rs:353](../../crates/crysirror-krkr/src/text_render_plugin.rs#L353) |

## Registered TJS native APIs / transitions / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.inTransition` | inTransition | Handled | — | [transitions.rs:51](../../crates/crysirror-krkr/src/transitions.rs#L51) |

## Registered TJS native APIs / video / install

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.enabledAudioStream` | enabledAudioStream | Handled | — | [video.rs:428](../../crates/crysirror-krkr/src/video.rs#L428) |
| `c.frame` | frame | Handled | — | [video.rs:441](../../crates/crysirror-krkr/src/video.rs#L441) |
| `c.numberOfAudioStream` | numberOfAudioStream | Handled | — | [video.rs:428](../../crates/crysirror-krkr/src/video.rs#L428) |
| `c.status` | status | Handled | — | [video.rs:453](../../crates/crysirror-krkr/src/video.rs#L453) |

## Registered TJS native APIs / wf_plugins / class

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.finalize` | finalize | Handled | — | [wf_plugins.rs:143](../../crates/crysirror-krkr/src/wf_plugins.rs#L143) |
| `c.interface` | interface | Handled | — | [wf_plugins.rs:137](../../crates/crysirror-krkr/src/wf_plugins.rs#L137) |

## Registered TJS native APIs / wf_plugins / osc_class

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.oscillator` | oscillator | Handled | — | [wf_plugins.rs:268](../../crates/crysirror-krkr/src/wf_plugins.rs#L268) |

## Registered TJS native APIs / wf_plugins / real_property

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.bands` | bands | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.damping` | damping | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.effectMix` | effectMix | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.extend` | extend | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.filtersPerBand` | filtersPerBand | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.formantShift` | formantShift | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.mode` | mode | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.reactionTime` | reactionTime | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.roomSize` | roomSize | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |
| `c.width` | width | Handled | — | [wf_plugins.rs:149](../../crates/crysirror-krkr/src/wf_plugins.rs#L149) |

## Registered TJS native APIs / win32_dialog / blob_constructor

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.ReferPointer` | ReferPointer | Handled | — | [win32_dialog.rs:608](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L608) |

## Registered TJS native APIs / win32_dialog / install_blob_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.getByte` | getByte | Handled | — | [win32_dialog.rs:645](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L645) |
| `handle.getDWord` | getDWord | Handled | — | [win32_dialog.rs:645](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L645) |
| `handle.getDWordLong` | getDWordLong | Handled | — | [win32_dialog.rs:658](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L658) |
| `handle.getText` | getText | Handled | — | [win32_dialog.rs:652](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L652) |
| `handle.getWord` | getWord | Handled | — | [win32_dialog.rs:645](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L645) |
| `handle.pointer` | pointer | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:640](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L640) |
| `handle.setByte` | setByte | Handled | — | [win32_dialog.rs:662](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L662) |
| `handle.setDWord` | setDWord | Handled | — | [win32_dialog.rs:662](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L662) |
| `handle.setDWordLong` | setDWordLong | Handled | — | [win32_dialog.rs:669](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L669) |
| `handle.setText` | setText | Handled | — | [win32_dialog.rs:662](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L662) |
| `handle.setWord` | setWord | Handled | — | [win32_dialog.rs:662](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L662) |

## Registered TJS native APIs / win32_dialog / install_dialog_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.bringToFront` | bringToFront | Handled | — | [win32_dialog.rs:273](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L273) |
| `handle.close` | close | Handled | — | [win32_dialog.rs:152](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L152) |
| `handle.closeProgress` | closeProgress | Handled | — | [win32_dialog.rs:273](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L273) |
| `handle.deleteAllTab` | deleteAllTab | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.deleteTab` | deleteTab | Handled | — | [win32_dialog.rs:229](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L229) |
| `handle.finalize` | finalize | Handled | — | [win32_dialog.rs:105](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L105) |
| `handle.getBaseUnits` | getBaseUnits | Handled | — | [win32_dialog.rs:276](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L276) |
| `handle.getClientRect` | getClientRect | Handled | — | [win32_dialog.rs:278](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L278) |
| `handle.getCurSel` | getCurSel | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItem` | getItem | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemClassName` | getItemClassName | Handled | — | [win32_dialog.rs:195](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L195) |
| `handle.getItemEnabled` | getItemEnabled | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemHeight` | getItemHeight | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemID` | getItemID | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemInt` | getItemInt | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemLeft` | getItemLeft | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemLong` | getItemLong | Handled | — | [win32_dialog.rs:229](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L229) |
| `handle.getItemText` | getItemText | Handled | — | [win32_dialog.rs:196](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L196) |
| `handle.getItemTop` | getItemTop | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getItemWidth` | getItemWidth | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.getPlacement` | getPlacement | Handled | — | [win32_dialog.rs:297](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L297) |
| `handle.getScrollInfo` | getScrollInfo | Handled | — | [win32_dialog.rs:187](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L187) |
| `handle.getWindowRect` | getWindowRect | Handled | — | [win32_dialog.rs:278](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L278) |
| `handle.insertTab` | insertTab | Handled | — | [win32_dialog.rs:259](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L259) |
| `handle.invalidateAll` | invalidateAll | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.invalidateRect` | invalidateRect | Handled | — | [win32_dialog.rs:229](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L229) |
| `handle.isExistentItem` | isExistentItem | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.loadResource` | loadResource | Handled | — | [win32_dialog.rs:113](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L113) |
| `handle.lockItemUpdate` | lockItemUpdate | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.makeTemplate` | makeTemplate | Handled | — | [win32_dialog.rs:114](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L114) |
| `handle.mapRect` | mapRect | Handled | — | [win32_dialog.rs:281](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L281) |
| `handle.maximize` | maximize | Handled | — | [win32_dialog.rs:305](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L305) |
| `handle.onCommand` | onCommand | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.onHScroll` | onHScroll | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.onInit` | onInit | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.onNotify` | onNotify | Handled | — | [win32_dialog.rs:209](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L209) |
| `handle.onSize` | onSize | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.onVScroll` | onVScroll | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.open` | open | Handled | — | [win32_dialog.rs:121](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L121) |
| `handle.openProgress` | openProgress | Handled | — | [win32_dialog.rs:178](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L178) |
| `handle.propSheetMessage` | propSheetMessage | Handled | — | [win32_dialog.rs:259](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L259) |
| `handle.restoreMaximize` | restoreMaximize | Handled | — | [win32_dialog.rs:305](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L305) |
| `handle.selectTab` | selectTab | Handled | — | [win32_dialog.rs:229](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L229) |
| `handle.sendItemMessage` | sendItemMessage | Handled | — | [win32_dialog.rs:117](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L117) |
| `handle.setActive` | setActive | Handled | — | [win32_dialog.rs:273](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L273) |
| `handle.setCurSel` | setCurSel | Handled | — | [win32_dialog.rs:229](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L229) |
| `handle.setItemBitmap` | setItemBitmap | Handled | — | [win32_dialog.rs:116](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L116) |
| `handle.setItemEnabled` | setItemEnabled | Handled | — | [win32_dialog.rs:209](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L209) |
| `handle.setItemFocus` | setItemFocus | Handled | — | [win32_dialog.rs:145](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L145) |
| `handle.setItemInt` | setItemInt | Handled | — | [win32_dialog.rs:209](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L209) |
| `handle.setItemLong` | setItemLong | Handled | — | [win32_dialog.rs:259](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L259) |
| `handle.setItemPos` | setItemPos | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.setItemSize` | setItemSize | Handled | — | [win32_dialog.rs:251](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L251) |
| `handle.setItemText` | setItemText | Handled | — | [win32_dialog.rs:216](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L216) |
| `handle.setMessageResult` | setMessageResult | Handled | — | [win32_dialog.rs:145](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L145) |
| `handle.setPlacement` | setPlacement | Handled | — | [win32_dialog.rs:298](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L298) |
| `handle.setPos` | setPos | Handled | — | [win32_dialog.rs:209](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L209) |
| `handle.setScrollInfo` | setScrollInfo | Handled | — | [win32_dialog.rs:145](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L145) |
| `handle.setSize` | setSize | Handled | — | [win32_dialog.rs:209](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L209) |
| `handle.show` | show | Handled | — | [win32_dialog.rs:145](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L145) |
| `handle.unlockItemUpdate` | unlockItemUpdate | Handled | — | [win32_dialog.rs:275](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L275) |

## Registered TJS native APIs / win32_dialog / install_dialog_statics

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `class.chooseColor` | chooseColor | Handled | — | [win32_dialog.rs:404](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L404) |
| `class.getOctetAddress` | getOctetAddress | Handled | — | [win32_dialog.rs:396](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L396) |
| `class.getStringAddress` | getStringAddress | Handled | — | [win32_dialog.rs:396](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L396) |
| `class.initCommonControls` | initCommonControls | Handled | — | [win32_dialog.rs:403](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L403) |
| `class.initCommonControlsEx` | initCommonControlsEx | Handled | — | [win32_dialog.rs:385](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L385) |
| `class.messageBox` | messageBox | Handled | — | [win32_dialog.rs:372](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L372) |
| `class.openPropertySheet` | openPropertySheet | Handled | — | [win32_dialog.rs:396](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L396) |

## Registered TJS native APIs / win32_dialog / install_draw_item_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.ctrlID` | ctrlID | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.ctrlType` | ctrlType | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.draw` | draw | Handled | — | [win32_dialog.rs:559](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L559) |
| `handle.hwndItem` | hwndItem | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.itemAction` | itemAction | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.itemData` | itemData | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.itemID` | itemID | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |
| `handle.itemRect` | itemRect | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:576](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L576) |
| `handle.itemState` | itemState | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:574](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L574) |

## Registered TJS native APIs / win32_dialog / install_header_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.store` | store | Handled | — | [win32_dialog.rs:514](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L514) |

## Registered TJS native APIs / win32_dialog / install_items_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.store` | store | Handled | — | [win32_dialog.rs:525](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L525) |

## Registered TJS native APIs / win32_dialog / install_notify_members

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.code` | code | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:581](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L581) |
| `handle.getByte` | getByte | Handled | — | [win32_dialog.rs:586](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L586) |
| `handle.getDWord` | getDWord | Handled | — | [win32_dialog.rs:586](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L586) |
| `handle.getWord` | getWord | Handled | — | [win32_dialog.rs:586](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L586) |
| `handle.hwndFrom` | hwndFrom | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:581](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L581) |
| `handle.idFrom` | idFrom | Unsupported | WIN32Dialog: property is read-only | [win32_dialog.rs:581](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L581) |

## Registered TJS native APIs / win32_dialog / subclass_constructor

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `handle.finalize` | finalize | Handled | — | [win32_dialog.rs:508](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L508) |
| `instance.finalize` | finalize | Handled | — | [win32_dialog.rs:502](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L502) |

## Registered TJS native APIs / window_ex / install_native_unless_present

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `object.breathe` | breathe | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.bringAfter` | bringAfter | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.bringTo` | bringTo | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.clearGraphicCache` | clearGraphicCache | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.expandEnvString` | expandEnvString | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.focusMenuByKey` | focusMenuByKey | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getAboutString` | getAboutString | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getCPUType` | getCPUType | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getClientRect` | getClientRect | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getCursorPos` | getCursorPos | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getDisplayMonitors` | getDisplayMonitors | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getDoubleClickTime` | getDoubleClickTime | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getMonitorInfo` | getMonitorInfo | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getNormalRect` | getNormalRect | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getNotificationName` | getNotificationName | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getNotificationNum` | getNotificationNum | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getPlacement` | getPlacement | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getRect` | getRect | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getSystemMetrics` | getSystemMetrics | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.getWindowRect` | getWindowRect | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.isBreathing` | isBreathing | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.maximize` | maximize | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.minimize` | minimize | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.ncHitTest` | ncHitTest | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.popupEx` | popupEx | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.readEnvValue` | readEnvValue | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.registerExEvent` | registerExEvent | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.resetExSystemMenu` | resetExSystemMenu | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.resetWindowIcon` | resetWindowIcon | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.restoreMaximize` | restoreMaximize | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.sendToBack` | sendToBack | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setApplicationIcon` | setApplicationIcon | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setCursorPos` | setCursorPos | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setEvalErrorLog` | setEvalErrorLog | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setIconicPreview` | setIconicPreview | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setMessageHook` | setMessageHook | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setOverlayBitmap` | setOverlayBitmap | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setPlacement` | setPlacement | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setPos` | setPos | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.setWindowIcon` | setWindowIcon | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |
| `object.showRestore` | showRestore | Handled | — | [window_ex.rs:770](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L770) |

## Registered TJS native APIs / window_ex / install_property_unless_present

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `object.bmpChecked` | bmpChecked | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.bmpItem` | bmpItem | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.bmpUnchecked` | bmpUnchecked | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.disableMove` | disableMove | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.disableResize` | disableResize | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.enableNCMouseEvent` | enableNCMouseEvent | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.exSystemMenu` | exSystemMenu | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.maximizeBox` | maximizeBox | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.maximized` | maximized | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.minimizeBox` | minimizeBox | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.minimized` | minimized | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |
| `object.rightJustify` | rightJustify | Handled | — | [window_ex.rs:783](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L783) |

## Registered TJS native APIs / wv_decoder / class

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `c.channels` | channels | Handled | — | [wv_decoder.rs:377](../../crates/crysirror-krkr/src/wv_decoder.rs#L377) |
| `c.close` | close | Handled | — | [wv_decoder.rs:209](../../crates/crysirror-krkr/src/wv_decoder.rs#L209) |
| `c.finalize` | finalize | Handled | — | [wv_decoder.rs:209](../../crates/crysirror-krkr/src/wv_decoder.rs#L209) |
| `c.foreach` | foreach | Handled | — | [wv_decoder.rs:238](../../crates/crysirror-krkr/src/wv_decoder.rs#L238) |
| `c.format` | format | Handled | — | [wv_decoder.rs:377](../../crates/crysirror-krkr/src/wv_decoder.rs#L377) |
| `c.frequency` | frequency | Handled | — | [wv_decoder.rs:377](../../crates/crysirror-krkr/src/wv_decoder.rs#L377) |
| `c.getLipSync` | getLipSync | Handled | — | [wv_decoder.rs:262](../../crates/crysirror-krkr/src/wv_decoder.rs#L262) |
| `c.getSample` | getSample | Handled | — | [wv_decoder.rs:215](../../crates/crysirror-krkr/src/wv_decoder.rs#L215) |
| `c.makeThumbnail` | makeThumbnail | Handled | — | [wv_decoder.rs:282](../../crates/crysirror-krkr/src/wv_decoder.rs#L282) |
| `c.open` | open | Handled | — | [wv_decoder.rs:202](../../crates/crysirror-krkr/src/wv_decoder.rs#L202) |
| `c.rawdata` | rawdata | Handled | — | [wv_decoder.rs:389](../../crates/crysirror-krkr/src/wv_decoder.rs#L389) |
| `c.samples` | samples | Handled | — | [wv_decoder.rs:377](../../crates/crysirror-krkr/src/wv_decoder.rs#L377) |
| `c.save` | save | Handled | — | [wv_decoder.rs:329](../../crates/crysirror-krkr/src/wv_decoder.rs#L329) |
| `c.setSample` | setSample | Handled | — | [wv_decoder.rs:222](../../crates/crysirror-krkr/src/wv_decoder.rs#L222) |
| `c.totalTime` | totalTime | Handled | — | [wv_decoder.rs:377](../../crates/crysirror-krkr/src/wv_decoder.rs#L377) |

## TJS2 VM opcodes

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `0x00 nop` | No-operation instruction | Empty handler | Original no-op status not established | [mod.rs:826](../../crates/crysirror-tjs2/src/vm/mod.rs#L826) |
| `0x01 const` | Load a constant | Handled | — | [mod.rs:827](../../crates/crysirror-tjs2/src/vm/mod.rs#L827) |
| `0x02 cp` | Copy a value | Handled | — | [mod.rs:851](../../crates/crysirror-tjs2/src/vm/mod.rs#L851) |
| `0x03 cl` | cl | Handled | — | [mod.rs:855](../../crates/crysirror-tjs2/src/vm/mod.rs#L855) |
| `0x04 ccl` | ccl | Handled | — | [mod.rs:856](../../crates/crysirror-tjs2/src/vm/mod.rs#L856) |
| `0x05 tt` | tt | Handled | — | [mod.rs:861](../../crates/crysirror-tjs2/src/vm/mod.rs#L861) |
| `0x06 tf` | tf | Handled | — | [mod.rs:862](../../crates/crysirror-tjs2/src/vm/mod.rs#L862) |
| `0x07 ceq` | ceq | Handled | — | [mod.rs:863](../../crates/crysirror-tjs2/src/vm/mod.rs#L863) |
| `0x08 cdeq` | cdeq | Handled | — | [mod.rs:868](../../crates/crysirror-tjs2/src/vm/mod.rs#L868) |
| `0x09 clt` | clt | Handled | — | [mod.rs:873](../../crates/crysirror-tjs2/src/vm/mod.rs#L873) |
| `0x0A cgt` | cgt | Handled | — | [mod.rs:878](../../crates/crysirror-tjs2/src/vm/mod.rs#L878) |
| `0x0B setf` | setf | Handled | — | [mod.rs:883](../../crates/crysirror-tjs2/src/vm/mod.rs#L883) |
| `0x0C setnf` | setnf | Handled | — | [mod.rs:884](../../crates/crysirror-tjs2/src/vm/mod.rs#L884) |
| `0x0D lnot` | lnot | Handled | — | [mod.rs:885](../../crates/crysirror-tjs2/src/vm/mod.rs#L885) |
| `0x0E nf` | nf | Handled | — | [mod.rs:889](../../crates/crysirror-tjs2/src/vm/mod.rs#L889) |
| `0x0F jf` | jf | Handled | — | [mod.rs:890](../../crates/crysirror-tjs2/src/vm/mod.rs#L890) |
| `0x10 jnf` | jnf | Handled | — | [mod.rs:895](../../crates/crysirror-tjs2/src/vm/mod.rs#L895) |
| `0x11 jmp` | jmp | Handled | — | [mod.rs:900](../../crates/crysirror-tjs2/src/vm/mod.rs#L900) |
| `0x12 inc` | inc | Handled | — | [mod.rs:901](../../crates/crysirror-tjs2/src/vm/mod.rs#L901) |
| `0x13 incpd` | incpd | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x14 incpi` | incpi | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x15 incp` | incp | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x16 dec` | dec | Handled | — | [mod.rs:901](../../crates/crysirror-tjs2/src/vm/mod.rs#L901) |
| `0x17 decpd` | decpd | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x18 decpi` | decpi | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x19 decp` | decp | Handled | — | [mod.rs:909](../../crates/crysirror-tjs2/src/vm/mod.rs#L909) |
| `0x1A lor` | lor | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x1B lorpd` | lorpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x1C lorpi` | lorpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x1D lorp` | lorp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x1E land` | land | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x1F landpd` | landpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x20 landpi` | landpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x21 landp` | landp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x22 bor` | bor | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x23 borpd` | borpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x24 borpi` | borpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x25 borp` | borp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x26 bxor` | bxor | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x27 bxorpd` | bxorpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x28 bxorpi` | bxorpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x29 bxorp` | bxorp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2A band` | band | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2B bandpd` | bandpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2C bandpi` | bandpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2D bandp` | bandp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2E sar` | sar | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x2F sarpd` | sarpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x30 sarpi` | sarpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x31 sarp` | sarp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x32 sal` | sal | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x33 salpd` | salpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x34 salpi` | salpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x35 salp` | salp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x36 sr` | sr | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x37 srpd` | srpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x38 srpi` | srpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x39 srp` | srp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3A add` | add | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3B addpd` | addpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3C addpi` | addpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3D addp` | addp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3E sub` | sub | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x3F subpd` | subpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x40 subpi` | subpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x41 subp` | subp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x42 mod` | mod | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x43 modpd` | modpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x44 modpi` | modpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x45 modp` | modp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x46 div` | div | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x47 divpd` | divpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x48 divpi` | divpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x49 divp` | divp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4A idiv` | idiv | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4B idivpd` | idivpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4C idivpi` | idivpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4D idivp` | idivp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4E mul` | mul | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x4F mulpd` | mulpd | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x50 mulpi` | mulpi | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x51 mulp` | mulp | Handled | — | [mod.rs:910](../../crates/crysirror-tjs2/src/vm/mod.rs#L910) |
| `0x52 bnot` | bnot | Handled | — | [mod.rs:911](../../crates/crysirror-tjs2/src/vm/mod.rs#L911) |
| `0x53 typeof` | typeof | Handled | — | [mod.rs:915](../../crates/crysirror-tjs2/src/vm/mod.rs#L915) |
| `0x54 typeofd` | typeofd | Handled | — | [mod.rs:919](../../crates/crysirror-tjs2/src/vm/mod.rs#L919) |
| `0x55 typeofi` | typeofi | Handled | — | [mod.rs:923](../../crates/crysirror-tjs2/src/vm/mod.rs#L923) |
| `0x56 eval` | eval | Handled | — | [mod.rs:927](../../crates/crysirror-tjs2/src/vm/mod.rs#L927) |
| `0x57 eexp` | eexp | Handled | — | [mod.rs:927](../../crates/crysirror-tjs2/src/vm/mod.rs#L927) |
| `0x58 chkins` | chkins | Handled | — | [mod.rs:928](../../crates/crysirror-tjs2/src/vm/mod.rs#L928) |
| `0x59 asc` | asc | Handled | — | [mod.rs:933](../../crates/crysirror-tjs2/src/vm/mod.rs#L933) |
| `0x5A chr` | chr | Handled | — | [mod.rs:937](../../crates/crysirror-tjs2/src/vm/mod.rs#L937) |
| `0x5B num` | num | Handled | — | [mod.rs:941](../../crates/crysirror-tjs2/src/vm/mod.rs#L941) |
| `0x5C chs` | chs | Handled | — | [mod.rs:945](../../crates/crysirror-tjs2/src/vm/mod.rs#L945) |
| `0x5D inv` | inv | Handled | — | [mod.rs:949](../../crates/crysirror-tjs2/src/vm/mod.rs#L949) |
| `0x5E chkinv` | chkinv | Handled | — | [mod.rs:952](../../crates/crysirror-tjs2/src/vm/mod.rs#L952) |
| `0x5F int` | int | Handled | — | [mod.rs:959](../../crates/crysirror-tjs2/src/vm/mod.rs#L959) |
| `0x60 real` | real | Handled | — | [mod.rs:963](../../crates/crysirror-tjs2/src/vm/mod.rs#L963) |
| `0x61 str` | str | Handled | — | [mod.rs:967](../../crates/crysirror-tjs2/src/vm/mod.rs#L967) |
| `0x62 octet` | octet | Handled | — | [mod.rs:971](../../crates/crysirror-tjs2/src/vm/mod.rs#L971) |
| `0x63 call` | Call a function | Handled | — | [mod.rs:975](../../crates/crysirror-tjs2/src/vm/mod.rs#L975) |
| `0x64 calld` | Call a named property | Handled | — | [mod.rs:1011](../../crates/crysirror-tjs2/src/vm/mod.rs#L1011) |
| `0x65 calli` | Call an indexed property | Handled | — | [mod.rs:1048](../../crates/crysirror-tjs2/src/vm/mod.rs#L1048) |
| `0x66 new` | Construct an object | Handled | — | [mod.rs:975](../../crates/crysirror-tjs2/src/vm/mod.rs#L975) |
| `0x67 gpd` | gpd | Handled | — | [mod.rs:1085](../../crates/crysirror-tjs2/src/vm/mod.rs#L1085) |
| `0x68 spd` | spd | Handled | — | [mod.rs:1096](../../crates/crysirror-tjs2/src/vm/mod.rs#L1096) |
| `0x69 spde` | spde | Handled | — | [mod.rs:1096](../../crates/crysirror-tjs2/src/vm/mod.rs#L1096) |
| `0x6A spdeh` | spdeh | Handled | — | [mod.rs:1096](../../crates/crysirror-tjs2/src/vm/mod.rs#L1096) |
| `0x6B gpi` | gpi | Handled | — | [mod.rs:1108](../../crates/crysirror-tjs2/src/vm/mod.rs#L1108) |
| `0x6C spi` | spi | Handled | — | [mod.rs:1119](../../crates/crysirror-tjs2/src/vm/mod.rs#L1119) |
| `0x6D spie` | spie | Handled | — | [mod.rs:1119](../../crates/crysirror-tjs2/src/vm/mod.rs#L1119) |
| `0x6E gpds` | gpds | Handled | — | [mod.rs:1085](../../crates/crysirror-tjs2/src/vm/mod.rs#L1085) |
| `0x6F spds` | spds | Handled | — | [mod.rs:1096](../../crates/crysirror-tjs2/src/vm/mod.rs#L1096) |
| `0x70 gpis` | gpis | Handled | — | [mod.rs:1108](../../crates/crysirror-tjs2/src/vm/mod.rs#L1108) |
| `0x71 spis` | spis | Handled | — | [mod.rs:1119](../../crates/crysirror-tjs2/src/vm/mod.rs#L1119) |
| `0x72 setp` | setp | Handled | — | [mod.rs:1130](../../crates/crysirror-tjs2/src/vm/mod.rs#L1130) |
| `0x73 getp` | getp | Handled | — | [mod.rs:1135](../../crates/crysirror-tjs2/src/vm/mod.rs#L1135) |
| `0x74 deld` | deld | Handled | — | [mod.rs:1140](../../crates/crysirror-tjs2/src/vm/mod.rs#L1140) |
| `0x75 deli` | deli | Handled | — | [mod.rs:1148](../../crates/crysirror-tjs2/src/vm/mod.rs#L1148) |
| `0x76 srv` | srv | Handled | — | [mod.rs:1156](../../crates/crysirror-tjs2/src/vm/mod.rs#L1156) |
| `0x77 ret` | Return from a call | Handled | — | [mod.rs:1157](../../crates/crysirror-tjs2/src/vm/mod.rs#L1157) |
| `0x78 entry` | entry | Handled | — | [mod.rs:1158](../../crates/crysirror-tjs2/src/vm/mod.rs#L1158) |
| `0x79 extry` | extry | Handled | — | [mod.rs:1165](../../crates/crysirror-tjs2/src/vm/mod.rs#L1165) |
| `0x7A throw` | Throw an exception | Handled | — | [mod.rs:1168](../../crates/crysirror-tjs2/src/vm/mod.rs#L1168) |
| `0x7B chgthis` | chgthis | Handled | — | [mod.rs:1176](../../crates/crysirror-tjs2/src/vm/mod.rs#L1176) |
| `0x7C global` | global | Handled | — | [mod.rs:1182](../../crates/crysirror-tjs2/src/vm/mod.rs#L1182) |
| `0x7D addci` | addci | Handled | — | [mod.rs:1183](../../crates/crysirror-tjs2/src/vm/mod.rs#L1183) |
| `0x7E regmember` | regmember | Handled | — | [mod.rs:1230](../../crates/crysirror-tjs2/src/vm/mod.rs#L1230) |
| `0x7F debugger` | Debugger instruction | Empty handler | Original no-op status not established | [mod.rs:826](../../crates/crysirror-tjs2/src/vm/mod.rs#L826) |

## TJS2 VM opcodes / inst.opcode=104..=106 \| 111

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `0x69` | Execute execute instruction branch | Handled | — | [mod.rs:1098](../../crates/crysirror-tjs2/src/vm/mod.rs#L1098) |
| `0x6A` | Execute execute instruction branch | Handled | — | [mod.rs:1099](../../crates/crysirror-tjs2/src/vm/mod.rs#L1099) |
| `0x6F` | Execute execute instruction branch | Handled | — | [mod.rs:1100](../../crates/crysirror-tjs2/src/vm/mod.rs#L1100) |

## TJS2 VM opcodes / inst.opcode=108 \| 109 \| 113

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `0x6D` | Execute execute instruction branch | Handled | — | [mod.rs:1121](../../crates/crysirror-tjs2/src/vm/mod.rs#L1121) |
| `0x71` | Execute execute instruction branch | Handled | — | [mod.rs:1122](../../crates/crysirror-tjs2/src/vm/mod.rs#L1122) |

## TextRenderBase default properties

| ID / API | Purpose | Status | Gap / condition | Source |
| --- | --- | --- | --- | --- |
| `TextRenderBase.defaultAlign` | Default Align markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultBigFontSize` | Default BigFontSize markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultBold` | Default Bold markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultChColor` | Default ChColor markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultEdge` | Default Edge markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultEdgeColor` | Default EdgeColor markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultFace` | Default Face markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultFontSize` | Default FontSize markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultItalic` | Default Italic markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultLineSize` | Default LineSize markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultLineSpacing` | Default LineSpacing markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultPitch` | Default Pitch markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultRubyOffset` | Default RubyOffset markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultRubySize` | Default RubySize markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultShadow` | Default Shadow markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultShadowColor` | Default ShadowColor markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultShadowDiff` | Default ShadowDiff markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultSmallFontSize` | Default SmallFontSize markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |
| `TextRenderBase.defaultValign` | Default Valign markup property | Handled | Field types/defaults are defined in crysirror_text::markup::FIELDS | [text_render_plugin.rs:334](../../crates/crysirror-krkr/src/text_render_plugin.rs#L334) |

## Subselectors and conditional dispatch

Patterns retain their source namespace and guards. `_` is the fallback of that table. Boolean/optional results follow the function's return contract. Call-site names identify the implementation to inspect, including indirect service calls.

### parser.rs / KagParser::capture_macro_body

```rust
fn capture_macro_body(&mut self, opening_tag: &Tag) -> Result<String>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `tag.tagname.as_str()` | `"endmacro"` | Local state/value operations | Handled | [1976](../../crates/crysirror-kag/src/parser.rs#L1976) |
| `tag.tagname.as_str()` | `_` | self.recorded_macro_fragment | Handled | [1980](../../crates/crysirror-kag/src/parser.rs#L1980) |

### parser.rs / KagParser::define_macro

```rust
fn define_macro(&mut self, tag: Tag) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name == "name" \|\| name == "tagname"` | Local state/value operations | Handled | [1935](../../crates/crysirror-kag/src/parser.rs#L1935) |

### parser.rs / KagParser::handle_condition_tag

```rust
fn handle_condition_tag(&mut self, tag: &Tag, host: &mut H) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `tag.tagname.as_str()` | `_` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [1862](../../crates/crysirror-kag/src/parser.rs#L1862) |

### parser.rs / KagParser::handle_special_tag

```rust
fn handle_special_tag(&mut self, tag: Tag, host: &mut H) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `tag.tagname.as_str()` | `_` | self.macros.contains_key, self.expand_macro | Handled | [1912](../../crates/crysirror-kag/src/parser.rs#L1912) |

### parser/ext.rs / KagParser::handle_ext_loop

```rust
fn handle_ext_loop(&mut self, tag: &Tag, host: &mut H) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under tag.tagname.as_str()="endwhile" \| "continue" \| "break"` | `tag.tagname != "endwhile" && !self.is_executing()` | Local state/value operations | Handled | [369](../../crates/crysirror-kag/src/parser/ext.rs#L369) |
| `condition; under tag.tagname.as_str()="endwhile" \| "continue" \| "break"` | `tag.tagname != "endwhile"` | self.expand_parameter_macros, self.apply_macro_arguments, self.resolve_local_arguments, self.process_cond_attr | Handled | [390](../../crates/crysirror-kag/src/parser/ext.rs#L390) |
| `condition; under tag.tagname.as_str()="endwhile" \| "continue" \| "break"` | `active && tag.tagname != "break"` | host.eval_void, host.eval_bool | Handled | [401](../../crates/crysirror-kag/src/parser/ext.rs#L401) |
| `condition; under tag.tagname.as_str()="endwhile" \| "continue" \| "break"` | `tag.tagname == "endwhile" && !cond` | self.ext.as_mut | Handled | [409](../../crates/crysirror-kag/src/parser/ext.rs#L409) |
| `tag.tagname.as_str()` | `"while"` | self.ext.as_ref, KagError::host, self.is_executing, self.validate_ext_macro_arguments, self.expand_parameter_macros | Handled | [322](../../crates/crysirror-kag/src/parser/ext.rs#L322) |
| `tag.tagname.as_str()` | `"endwhile" \| "continue" \| "break"` | self.is_executing, self.parse_error, self.condition_stack.len, self.expand_parameter_macros, self.apply_macro_arguments | Handled | [368](../../crates/crysirror-kag/src/parser/ext.rs#L368) |
| `tag.tagname.as_str()` | `_` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [428](../../crates/crysirror-kag/src/parser/ext.rs#L428) |

### parser/ext.rs / KagParser::handle_ext_tag

```rust
fn handle_ext_tag(&mut self, tag: &Tag, host: &mut H) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under tag.tagname.as_str()="pmacro"` | `ext.parameter_macros.len() >= 4096 && !ext.parameter_macros.contains_key(name)` | KagError::host | Unsupported: ExtKAGParser parameter macro limit | [466](../../crates/crysirror-kag/src/parser/ext.rs#L466) |
| `tag.tagname.as_str()` | `"localvar"` | host.sync_parser_state, host.local_variables | Handled | [436](../../crates/crysirror-kag/src/parser/ext.rs#L436) |
| `tag.tagname.as_str()` | `"pushlocalvar"` | self.push_ext_locals, self.ext.as_mut | Handled | [441](../../crates/crysirror-kag/src/parser/ext.rs#L441) |
| `tag.tagname.as_str()` | `"poplocalvar"` | self.local_variables_depth, self.local_scope_base, self.parse_error, self.ext.as_mut, host.sync_parser_state | Handled | [446](../../crates/crysirror-kag/src/parser/ext.rs#L446) |
| `tag.tagname.as_str()` | `"pmacro"` | self.ext.as_mut, KagError::host | Handled | [456](../../crates/crysirror-kag/src/parser/ext.rs#L456) |
| `tag.tagname.as_str()` | `_` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [472](../../crates/crysirror-kag/src/parser/ext.rs#L472) |

### parser/ext.rs / KagParser::validate_ext_condition

```rust
fn validate_ext_condition(&self, tag: &Tag) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.ext.is_some() && matches!( tag.tagname.as_str(), "elsif" \| "else" \| "endif" \| "endignore" )` | self.condition_stack.len, self.parse_error | Handled | [300](../../crates/crysirror-kag/src/parser/ext.rs#L300) |

### adapter.rs / KrkrVm::enqueue_timers

```rust
fn enqueue_timers(&mut self, now: u64) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `mode` | `1` | Local state/value operations | Handled | [650](../../crates/crysirror-krkr/src/adapter.rs#L650) |
| `mode` | `2` | Local state/value operations | Handled | [651](../../crates/crysirror-krkr/src/adapter.rs#L651) |
| `mode` | `_` | Local state/value operations | Handled | [652](../../crates/crysirror-krkr/src/adapter.rs#L652) |

### adapter.rs / KrkrVm::execute

```rust
fn execute(&mut self, h: &mut dyn HostServices, input: InputFrame) -> Result<VmYield>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `kind == 1 && !self .rt .as_ref() .unwrap() .host() .timers .get(&id) .is_some_and(\|t\| t.enabled && t.interval != 0)` | Local state/value operations | Handled | [872](../../crates/crysirror-krkr/src/adapter.rs#L872) |
| `condition` | `kind==3` | crate::layer_movie::take_event, self.rt.as_mut, self.events.entry | Handled | [884](../../crates/crysirror-krkr/src/adapter.rs#L884) |
| `kind` | `0` | Local state/value operations | Handled | [892](../../crates/crysirror-krkr/src/adapter.rs#L892) |
| `kind` | `1` | Local state/value operations | Handled | [893](../../crates/crysirror-krkr/src/adapter.rs#L893) |
| `kind` | `2` | Local state/value operations | Handled | [894](../../crates/crysirror-krkr/src/adapter.rs#L894) |
| `kind` | `_` | Local state/value operations | Handled | [895](../../crates/crysirror-krkr/src/adapter.rs#L895) |
| `condition` | `has_member(rt, ObjectHandle(id), "onPaint")` | self.call | Handled | [967](../../crates/crysirror-krkr/src/adapter.rs#L967) |

### adapter.rs / KrkrVm::hit_candidate

```rust
fn hit_candidate(&mut self, id: usize, lx: i64, ly: i64) -> Result<Option<bool>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!rt.object_member(ObjectHandle(id), "__lp_enabled").is_truthy()` | Local state/value operations | Handled | [511](../../crates/crysirror-krkr/src/adapter.rs#L511) |
| `condition` | `kind == 1` | Local state/value operations | Handled | [528](../../crates/crysirror-krkr/src/adapter.rs#L528) |
| `condition; under receiver=else: kind == 1` | `kind == 0` | i64::from | Handled | [533](../../crates/crysirror-krkr/src/adapter.rs#L533) |
| `condition; under receiver=else: kind == 1 / receiver=kind == 0` | `layer.kind == 1` | Local state/value operations | Handled | [544](../../crates/crysirror-krkr/src/adapter.rs#L544) |

### alpha_movie.rs / drive

```rust
fn drive(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Ok(_) \| Err(crysirror_core::Error::NotFound)` | Local state/value operations | Empty handler: Original no-op status not established | [262](../../crates/crysirror-krkr/src/alpha_movie.rs#L262) |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Err(e) if e.to_string().contains("media handle not found")` | Local state/value operations | Empty handler: Original no-op status not established | [263](../../crates/crysirror-krkr/src/alpha_movie.rs#L263) |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Err(e)` | Local state/value operations | Unsupported: Unconditional rejection | [264](../../crates/crysirror-krkr/src/alpha_movie.rs#L264) |

### async_trigger.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `rt.object_member(h, "mode").to_integer()?` | `0` | Local state/value operations | Handled | [36](../../crates/crysirror-krkr/src/async_trigger.rs#L36) |
| `rt.object_member(h, "mode").to_integer()?` | `1` | Local state/value operations | Handled | [37](../../crates/crysirror-krkr/src/async_trigger.rs#L37) |
| `rt.object_member(h, "mode").to_integer()?` | `2` | Local state/value operations | Handled | [38](../../crates/crysirror-krkr/src/async_trigger.rs#L38) |
| `rt.object_member(h, "mode").to_integer()?` | `_` | TjsError::runtime | Unsupported: invalid AsyncTrigger.mode | [39](../../crates/crysirror-krkr/src/async_trigger.rs#L39) |

### audio_plugin.rs / link

```rust
fn link(rt: &mut Rt, name: &str) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"kropus.dll" \| "wuopus.dll"` | Local state/value operations | Handled | [9](../../crates/crysirror-krkr/src/audio_plugin.rs#L9) |
| `name` | `"wuvorbis.dll"` | Local state/value operations | Handled | [10](../../crates/crysirror-krkr/src/audio_plugin.rs#L10) |
| `name` | `"wumsadp.dll"` | Local state/value operations | Handled | [11](../../crates/crysirror-krkr/src/audio_plugin.rs#L11) |
| `name` | `_` | TjsError::runtime | Unsupported: unknown native audio codec module | [12](../../crates/crysirror-krkr/src/audio_plugin.rs#L12) |

### d2d_view.rs / attach

```rust
fn attach(rt: &mut Rt, window: ObjectHandle, layer: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `key` | `"visible"` | i64::from | Handled | [115](../../crates/crysirror-krkr/src/d2d_view.rs#L115) |
| `key` | `"opacity"` | Local state/value operations | Handled | [116](../../crates/crysirror-krkr/src/d2d_view.rs#L116) |
| `key` | `"order"` | Local state/value operations | Handled | [117](../../crates/crysirror-krkr/src/d2d_view.rs#L117) |
| `key` | `_` | Local state/value operations | Handled | [118](../../crates/crysirror-krkr/src/d2d_view.rs#L118) |
| `key` | `"visible"` | Local state/value operations | Handled | [125](../../crates/crysirror-krkr/src/d2d_view.rs#L125) |
| `key` | `"opacity"` | Local state/value operations | Handled | [126](../../crates/crysirror-krkr/src/d2d_view.rs#L126) |
| `key` | `"order"` | Local state/value operations | Handled | [127](../../crates/crysirror-krkr/src/d2d_view.rs#L127) |
| `key` | `_` | Local state/value operations | Handled | [128](../../crates/crysirror-krkr/src/d2d_view.rs#L128) |

### d2d_view.rs / sync_window_zoom

```rust
fn sync_window_zoom(rt: &mut Rt, window: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `rt.object_member(window, key)` | `Variant::Integer(n) if n > 0` | Local state/value operations | Handled | [52](../../crates/crysirror-krkr/src/d2d_view.rs#L52) |
| `rt.object_member(window, key)` | `_` | Local state/value operations | Handled | [53](../../crates/crysirror-krkr/src/d2d_view.rs#L53) |

### draw_plugin.rs / draw

```rust
fn draw(rt: &mut Rt, t: Option<ObjectHandle>, name: &str, a: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name == "drawPath"` | Local state/value operations | Handled | [221](../../crates/crysirror-krkr/src/draw_plugin.rs#L221) |
| `condition` | `name == "drawRectangle" \|\| name == "drawLine"` | Local state/value operations | Handled | [224](../../crates/crysirror-krkr/src/draw_plugin.rs#L224) |
| `condition` | `name == "drawPath"` | Local state/value operations | Handled | [247](../../crates/crysirror-krkr/src/draw_plugin.rs#L247) |

### draw_plugin.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name.starts_with("reset")` | Local state/value operations | Handled | [530](../../crates/crysirror-krkr/src/draw_plugin.rs#L530) |
| `condition` | `name.starts_with("set")` | Local state/value operations | Handled | [532](../../crates/crysirror-krkr/src/draw_plugin.rs#L532) |
| `condition` | `name.starts_with("translate")` | vector::translation | Handled | [535](../../crates/crysirror-krkr/src/draw_plugin.rs#L535) |
| `condition` | `name.starts_with("scale")` | vector::scaling | Handled | [538](../../crates/crysirror-krkr/src/draw_plugin.rs#L538) |
| `condition` | `name.contains("View")` | Local state/value operations | Handled | [546](../../crates/crysirror-krkr/src/draw_plugin.rs#L546) |
| `condition` | `name.starts_with("set") \|\| name.starts_with("reset")` | Local state/value operations | Handled | [551](../../crates/crysirror-krkr/src/draw_plugin.rs#L551) |
| `condition` | `!matches!(mode, 2 \| 4)` | Local state/value operations | Unsupported: non-AA GDI mode / shared vector antialias policy | [585](../../crates/crysirror-krkr/src/draw_plugin.rs#L585) |

### draw_plugin.rs / install_appearance

```rust
fn install_appearance(rt: &mut Rt, h: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `object_arg(&a[0]).is_some()` | Local state/value operations | Unsupported: GDI brush object / solid ARGB brush | [309](../../crates/crysirror-krkr/src/draw_plugin.rs#L309) |

### draw_plugin.rs / install_path

```rust
fn install_path(rt: &mut Rt, h: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `p.commands.len() >= 8192` | Local state/value operations | Unsupported: Path command budget | [369](../../crates/crysirror-krkr/src/draw_plugin.rs#L369) |
| `condition` | `name == "drawLine" \|\| name == "drawLines"` | vector::append_polyline | Handled | [389](../../crates/crysirror-krkr/src/draw_plugin.rs#L389) |
| `condition` | `p.commands.len() + commands.len() > 8192` | Local state/value operations | Unsupported: Path command budget | [400](../../crates/crysirror-krkr/src/draw_plugin.rs#L400) |

### draw_plugin.rs / shape

```rust
fn shape(rt: &Rt, name: &str, a: &[Variant], skip: usize) -> Result<Vec<C>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"drawRectangle"` | vector::rectangle | Handled | [195](../../crates/crysirror-krkr/src/draw_plugin.rs#L195) |
| `name` | `"drawPolygon"` | vector::polygon | Handled | [199](../../crates/crysirror-krkr/src/draw_plugin.rs#L199) |
| `name` | `"drawLine"` | vector::polygon | Handled | [203](../../crates/crysirror-krkr/src/draw_plugin.rs#L203) |
| `name` | `"drawLines"` | vector::polygon | Handled | [208](../../crates/crysirror-krkr/src/draw_plugin.rs#L208) |
| `name` | `_` | Local state/value operations | Unsupported: solid rectangle/polygon/line path profile | [212](../../crates/crysirror-krkr/src/draw_plugin.rs#L212) |

### edit_dialog.rs / set_item_text

```rust
fn set_item_text(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `rt.object_member(owner, "__editControl") == Variant::Integer(id)` | Local state/value operations | Handled | [84](../../crates/crysirror-krkr/src/edit_dialog.rs#L84) |

### exe_options.rs / parse

```rust
fn parse(exe: &[u8]) -> Result<Vec<(String, String)>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let Some(options) = PeData::parse(exe).ok() .and_then(\|pe\| pe.raw_resources(&PeResourceName::Name("TEXT".into()), &\|\| Ok(())).ok()) .and_then(\|resources\| resources.into_iter().find(\|r\| r.name == PeResourceName::Id(139)))` | Local state/value operations | Handled | [24](../../crates/crysirror-krkr/src/exe_options.rs#L24) |

### font_resources.rs / signature

```rust
fn signature(rt: &mut Rt, font: ObjectHandle) -> Result<String>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `key.len() > 4096` | TjsError::runtime | Unsupported: native font binding signature length | [32](../../crates/crysirror-krkr/src/font_resources.rs#L32) |

### host.rs / encode_native_text

```rust
fn encode_native_text(text:&str,mode:&str) -> TjsResult<Vec<u8>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `mode` | `""` | Local state/value operations | Handled | [1230](../../crates/crysirror-krkr/src/host.rs#L1230) |
| `mode` | `"c"\|"c1"` | Local state/value operations | Handled | [1231](../../crates/crysirror-krkr/src/host.rs#L1231) |
| `mode` | `"c2"\|"z"` | Local state/value operations | Handled | [1232](../../crates/crysirror-krkr/src/host.rs#L1232) |
| `mode` | `mode if mode.len()==2&&mode.starts_with('z')&&mode.as_bytes()[1].is_ascii_digit()` | Local state/value operations | Handled | [1233](../../crates/crysirror-krkr/src/host.rs#L1233) |
| `mode` | `_` | TjsError::runtime | Unsupported: native text offset/unknown compression mode is unsupported | [1234](../../crates/crysirror-krkr/src/host.rs#L1234) |

### host.rs / KrkrHost::bind_reserved_layer

```rust
fn bind_reserved_layer(&mut self, handle: ObjectHandle, window: usize, parent: Option<usize>) -> TjsResult<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `id==handle.0` | TjsError::runtime | Unsupported: native parent cycle | [582](../../crates/crysirror-krkr/src/host.rs#L582) |

### host.rs / KrkrHost::canvas_op

```rust
fn canvas_op(&mut self, mut op: Value) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `op["op"].as_str() == Some("copy")` | usize::try_from, self.slot_owners.get, self.motion.has_frame, self.layers.get | Handled | [434](../../crates/crysirror-krkr/src/host.rs#L434) |
| `condition` | `matches!( op["op"].as_str(), Some("clear" \| "set" \| "draw" \| "copy" \| "create") )` | usize::try_from, self.slot_owners.get, self.layers.get_mut, u8::try_from | Handled | [444](../../crates/crysirror-krkr/src/host.rs#L444) |
| `condition` | `let Some(id) = op .get("destination") .or_else(\|\| op.get("id")) .and_then(Value::as_u64)` | usize::try_from, self.slot_owners.get, self.layers.get_mut, u8::try_from | Handled | [448](../../crates/crysirror-krkr/src/host.rs#L448) |
| `condition; under op["op"].as_str()=Some("clear")` | `let Ok(color)=serde_json::from_value::<[u8;4]>(op["color"].clone())` | Local state/value operations | Handled | [463](../../crates/crysirror-krkr/src/host.rs#L463) |
| `op["op"].as_str()` | `Some("clear")` | u8::try_from | Handled | [462](../../crates/crysirror-krkr/src/host.rs#L462) |
| `op["op"].as_str()` | `Some("draw") if l.full_alpha == Some(255)` | Local state/value operations | Handled | [469](../../crates/crysirror-krkr/src/host.rs#L469) |
| `op["op"].as_str()` | `_` | Local state/value operations | Handled | [470](../../crates/crysirror-krkr/src/host.rs#L470) |

### host.rs / KrkrHost::sync

```rust
fn sync(&mut self) -> TjsResult<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `l.kind == 0` | Local state/value operations | Handled | [944](../../crates/crysirror-krkr/src/host.rs#L944) |

### host.rs / KrkrHost::write_binary

```rust
fn write_binary(&mut self, name: &str, mode: &str, bytes: &[u8]) -> TjsResult<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `mode != "b"` | TjsError::runtime | Unsupported: native binary stream mode is unsupported | [1175](../../crates/crysirror-krkr/src/host.rs#L1175) |

### http_plugin.rs / members

```rust
fn members(rt: &mut Rt, h: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `"GET"` | Local state/value operations | Handled | [137](../../crates/crysirror-krkr/src/http_plugin.rs#L137) |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `"POST"` | Local state/value operations | Handled | [138](../../crates/crysirror-krkr/src/http_plugin.rs#L138) |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `"PUT"` | Local state/value operations | Handled | [139](../../crates/crysirror-krkr/src/http_plugin.rs#L139) |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `"DELETE"` | Local state/value operations | Handled | [140](../../crates/crysirror-krkr/src/http_plugin.rs#L140) |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `"HEAD"` | Local state/value operations | Handled | [141](../../crates/crysirror-krkr/src/http_plugin.rs#L141) |
| `a[0].to_tjs_string()?.to_ascii_uppercase().as_str()` | `other` | TjsError::runtime | Unsupported: unsupported HTTP method {other} | [142](../../crates/crysirror-krkr/src/http_plugin.rs#L142) |

### image_api.rs / begin_chain

```rust
fn begin_chain(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    effects: Vec<Value>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!(kind, 1 \| 2)` | Local state/value operations | Unsupported: native image processing blend type / straight-alpha/opaque image profile | [107](../../crates/crysirror-krkr/src/image_api.rs#L107) |
| `condition` | `matches!(kind, 1 \| 2)` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [107](../../crates/crysirror-krkr/src/image_api.rs#L107) |

### image_api.rs / prepare_hit_image

```rust
fn prepare_hit_image(
    rt: &mut Rt, host: &mut dyn HostServices, client: ClientId, id: usize, x: i64, y: i64,
) -> anyhow::Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `kind!=0 \|\| !(1..256).contains(&threshold) \|\| !layer.has_image \|\| layer.kind==1 \|\| layer.full_alpha.is_some() \|\| px<0 \|\| py<0 \|\| px>=layer.image_width \|\| py>=layer.image_height` | Local state/value operations | Handled | [427](../../crates/crysirror-krkr/src/image_api.rs#L427) |

### image_api.rs / queue_chain

```rust
fn queue_chain(
    rt: &mut Rt,
    h: ObjectHandle,
    input: ImageInput,
    effects: Vec<Value>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `e["kind"] == "flip"` | Local state/value operations | Handled | [512](../../crates/crysirror-krkr/src/image_api.rs#L512) |

### image_api.rs / save_format

```rust
fn save_format(mode:&str) -> Option<SaveFormat>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `mode.to_ascii_lowercase().trim_start_matches('.')` | `"bmp"\|"bmp32"\|"dib"` | SaveFormat::Bmp | Handled | [255](../../crates/crysirror-krkr/src/image_api.rs#L255) |
| `mode.to_ascii_lowercase().trim_start_matches('.')` | `"bmp24"` | SaveFormat::Bmp | Handled | [256](../../crates/crysirror-krkr/src/image_api.rs#L256) |
| `mode.to_ascii_lowercase().trim_start_matches('.')` | `"bmp8"` | SaveFormat::Bmp | Handled | [257](../../crates/crysirror-krkr/src/image_api.rs#L257) |
| `mode.to_ascii_lowercase().trim_start_matches('.')` | `"png"` | Local state/value operations | Handled | [258](../../crates/crysirror-krkr/src/image_api.rs#L258) |
| `mode.to_ascii_lowercase().trim_start_matches('.')` | `_` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [259](../../crates/crysirror-krkr/src/image_api.rs#L259) |

### image_api.rs / save_layer_image

```rust
fn save_layer_image(rt:&mut Rt,t:Option<ObjectHandle>,args:Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(mode.to_ascii_lowercase().trim_start_matches('.'),"jpg"\|"jpeg"\|"tlg"\|"tlg5"\|"tlg6")` | Local state/value operations | Handled | [271](../../crates/crysirror-krkr/src/image_api.rs#L271) |

### input_delivery.rs / Delivery::posted

```rust
fn posted(
        frame: InputFrame,
        window: ObjectHandle,
        method: &'static str,
        key: i64,
        flags: i64,
    ) -> Self
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `method == "onKeyPress"` | Local state/value operations | Handled | [68](../../crates/crysirror-krkr/src/input_delivery.rs#L68) |

### kag.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name; under name="ignoreCR" \| "processSpecialTags" \| "debugLevel"` | `"ignoreCR"` | Local state/value operations | Handled | [482](../../crates/crysirror-krkr/src/kag.rs#L482) |
| `name; under name="ignoreCR" \| "processSpecialTags" \| "debugLevel"` | `"processSpecialTags"` | Local state/value operations | Handled | [483](../../crates/crysirror-krkr/src/kag.rs#L483) |
| `name; under name="ignoreCR" \| "processSpecialTags" \| "debugLevel"` | `_` | h.wrap, KagError::host | Handled | [484](../../crates/crysirror-krkr/src/kag.rs#L484) |
| `name` | `"ignoreCR" \| "processSpecialTags" \| "debugLevel"` | h.wrap, KagError::host | Handled | [480](../../crates/crysirror-krkr/src/kag.rs#L480) |
| `name` | `"curStorage"` | Local state/value operations | Handled | [494](../../crates/crysirror-krkr/src/kag.rs#L494) |
| `name` | `_` | Local state/value operations | Unsupported: read-only KAG parser property | [502](../../crates/crysirror-krkr/src/kag.rs#L502) |

### kag_ext.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"multiLineTagEnabled"` | Variant::Integer | Handled | [395](../../crates/crysirror-krkr/src/kag_ext.rs#L395) |
| `name` | `"numericMacroArgumentsEnabled"` | Variant::Integer | Handled | [396](../../crates/crysirror-krkr/src/kag_ext.rs#L396) |
| `name` | `"localVariablesDepth"` | Variant::Integer | Handled | [399](../../crates/crysirror-krkr/src/kag_ext.rs#L399) |
| `name` | `"whileStackDepth"` | Variant::Integer | Handled | [400](../../crates/crysirror-krkr/src/kag_ext.rs#L400) |
| `name` | `"ifLevel"` | Variant::Integer | Handled | [401](../../crates/crysirror-krkr/src/kag_ext.rs#L401) |
| `name` | `"currentLocalVariables" \| "lf"` | Variant::self_bound | Handled | [402](../../crates/crysirror-krkr/src/kag_ext.rs#L402) |
| `name` | `"prelf"` | Variant::self_bound | Handled | [406](../../crates/crysirror-krkr/src/kag_ext.rs#L406) |
| `name` | `"localVariables"` | Variant::Object | Handled | [409](../../crates/crysirror-krkr/src/kag_ext.rs#L409) |
| `name` | `"pmacros" \| "paramMacros"` | Variant::Object | Handled | [410](../../crates/crysirror-krkr/src/kag_ext.rs#L410) |
| `name` | `"enableNP" \| "fuzzyReturn"` | Variant::Integer | Handled | [411](../../crates/crysirror-krkr/src/kag_ext.rs#L411) |
| `name` | `"returnErrorStorage"` | Variant::String, String::new | Handled | [412](../../crates/crysirror-krkr/src/kag_ext.rs#L412) |
| `name` | `_` | Local state/value operations | Unsupported: unknown ExtKAGParser property | [413](../../crates/crysirror-krkr/src/kag_ext.rs#L413) |
| `condition; under name="multiLineTagEnabled" \| "numericMacroArgumentsEnabled"` | `name == "multiLineTagEnabled"` | Local state/value operations | Handled | [418](../../crates/crysirror-krkr/src/kag_ext.rs#L418) |
| `name` | `"multiLineTagEnabled" \| "numericMacroArgumentsEnabled"` | Local state/value operations | Handled | [417](../../crates/crysirror-krkr/src/kag_ext.rs#L417) |
| `name` | `"enableNP" \| "fuzzyReturn"` | Local state/value operations | Handled | [426](../../crates/crysirror-krkr/src/kag_ext.rs#L426) |
| `name` | `"returnErrorStorage"` | Local state/value operations | Handled | [433](../../crates/crysirror-krkr/src/kag_ext.rs#L433) |
| `name` | `_` | Local state/value operations | Unsupported: ExtKAGParser {name} is read-only | [443](../../crates/crysirror-krkr/src/kag_ext.rs#L443) |

### kag_ext.rs / prepare_tag_values

```rust
fn prepare_tag_values(
    rt: &mut Rt,
    h: ObjectHandle,
    original: &Tag,
    resolved: &Tag,
) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name != "tagname" && name != "cond" && resolved .attributes .iter() .any(\|a\| a.name() == Some(name.as_str()))` | Local state/value operations | Handled | [192](../../crates/crysirror-krkr/src/kag_ext.rs#L192) |

### kag_ext_save.rs / decode

```rust
fn decode(rt: &Rt, h: ObjectHandle) -> Result<(NativeExtControl, Vec<Vec<Attribute>>)>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `number(rt, sentinel, key, 4 * 1024 * 1024)? != value` | Local state/value operations | Unsupported: native Ext call/while sentinel mismatch | [200](../../crates/crysirror-krkr/src/kag_ext_save.rs#L200) |

### kaicho_plugin.rs / effect

```rust
fn effect(rt: &mut Rt, name: &str, options: ObjectHandle) -> Result<SubtreeTransitionEffect>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name == "blur"` | Local state/value operations | Handled | [57](../../crates/crysirror-krkr/src/kaicho_plugin.rs#L57) |
| `condition` | `name == "blur"` | Local state/value operations | Handled | [75](../../crates/crysirror-krkr/src/kaicho_plugin.rs#L75) |

### layer_ex.rs / glitch_transition

```rust
fn glitch_transition(rt: &mut Rt, name: &str, options: ObjectHandle, has_source: bool) -> Result<GlitchStart>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name == "loopglitch"` | Local state/value operations | Handled | [273](../../crates/crysirror-krkr/src/layer_ex.rs#L273) |
| `condition` | `name == "glitch"` | Local state/value operations | Handled | [289](../../crates/crysirror-krkr/src/layer_ex.rs#L289) |

### menus.rs / sync

```rust
fn sync(host: &mut KrkrHost) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!specs.iter().any(\|s\| old["id"] == s.id)` | host.ui_op | Handled | [264](../../crates/crysirror-krkr/src/menus.rs#L264) |

### modal_window.rs / KrkrVm::modal_input

```rust
fn modal_input(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
    ) -> Result<InputFrame>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `event.widget.id == modal.widget + 3 && event.kind == "activate"` | Local state/value operations | Handled | [264](../../crates/crysirror-krkr/src/modal_window.rs#L264) |
| `condition` | `let Some(bit) = event .kind .strip_prefix("modal_press_") .and_then(\|s\| s.parse::<u64>().ok())` | Local state/value operations | Handled | [267](../../crates/crysirror-krkr/src/modal_window.rs#L267) |
| `condition` | `let Some(bit) = event .kind .strip_prefix("modal_release_") .and_then(\|s\| s.parse::<u64>().ok())` | Local state/value operations | Handled | [277](../../crates/crysirror-krkr/src/modal_window.rs#L277) |

### motion_plugin.rs / install_player

```rust
fn install_player(rt: &mut Rt, c: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `key == "chara"` | Local state/value operations | Handled | [503](../../crates/crysirror-krkr/src/motion_plugin.rs#L503) |
| `condition` | `key == "motion"` | Local state/value operations | Unsupported: Motion.Player.motion assignment / play(motion,flags) | [512](../../crates/crysirror-krkr/src/motion_plugin.rs#L512) |
| `key` | `"speed"` | Local state/value operations | Handled | [555](../../crates/crysirror-krkr/src/motion_plugin.rs#L555) |
| `key` | `"tickCount"` | Local state/value operations | Handled | [556](../../crates/crysirror-krkr/src/motion_plugin.rs#L556) |
| `key` | `"frameTickCount"` | Local state/value operations | Handled | [557](../../crates/crysirror-krkr/src/motion_plugin.rs#L557) |
| `key` | `"lastTime"` | Local state/value operations | Handled | [558](../../crates/crysirror-krkr/src/motion_plugin.rs#L558) |
| `key` | `"frameLastTime"` | Local state/value operations | Handled | [559](../../crates/crysirror-krkr/src/motion_plugin.rs#L559) |
| `key` | `"loopTime"` | Local state/value operations | Handled | [560](../../crates/crysirror-krkr/src/motion_plugin.rs#L560) |
| `key` | `"frameLoopTime"` | Local state/value operations | Handled | [561](../../crates/crysirror-krkr/src/motion_plugin.rs#L561) |
| `key` | `"opacity"` | f64::from | Handled | [562](../../crates/crysirror-krkr/src/motion_plugin.rs#L562) |
| `key` | `"visible"` | Local state/value operations | Handled | [563](../../crates/crysirror-krkr/src/motion_plugin.rs#L563) |
| `key` | `"completionType"` | Local state/value operations | Handled | [564](../../crates/crysirror-krkr/src/motion_plugin.rs#L564) |
| `key` | `"colorWeight"` | Local state/value operations | Handled | [565](../../crates/crysirror-krkr/src/motion_plugin.rs#L565) |
| `key` | `"maskMode"` | Local state/value operations | Handled | [566](../../crates/crysirror-krkr/src/motion_plugin.rs#L566) |
| `key` | `_` | Local state/value operations | Handled | [567](../../crates/crysirror-krkr/src/motion_plugin.rs#L567) |
| `key` | `"speed" if n >= 0.` | Local state/value operations | Handled | [579](../../crates/crysirror-krkr/src/motion_plugin.rs#L579) |
| `key` | `"tickCount" if n >= 0.` | Local state/value operations | Handled | [580](../../crates/crysirror-krkr/src/motion_plugin.rs#L580) |
| `key` | `"frameTickCount" if n >= 0.` | Local state/value operations | Handled | [581](../../crates/crysirror-krkr/src/motion_plugin.rs#L581) |
| `key` | `"opacity" if (0. ..=255.).contains(&n)` | Local state/value operations | Handled | [582](../../crates/crysirror-krkr/src/motion_plugin.rs#L582) |
| `key` | `"visible"` | Local state/value operations | Handled | [583](../../crates/crysirror-krkr/src/motion_plugin.rs#L583) |
| `key` | `"completionType" if n == 0. \|\| n == 1. \|\| n == 2.` | Local state/value operations | Handled | [584](../../crates/crysirror-krkr/src/motion_plugin.rs#L584) |
| `key` | `"colorWeight" if n == 4286611584.` | Local state/value operations | Empty handler: Original no-op status not established | [585](../../crates/crysirror-krkr/src/motion_plugin.rs#L585) |
| `key` | `"maskMode" if n == 1. \|\| n == 2.` | Local state/value operations | Empty handler: Original no-op status not established | [586](../../crates/crysirror-krkr/src/motion_plugin.rs#L586) |
| `key` | `_` | Local state/value operations | Unsupported: Motion.Player.{key}={n} / supported motion property profile | [587](../../crates/crysirror-krkr/src/motion_plugin.rs#L587) |

### motion_plugin.rs / State::invalidate

```rust
fn invalidate(&mut self, id: usize) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self .loading .as_ref() .is_some_and(\|p\| p.manager == id \|\| p.result.0 == id)` | self.loading.take | Handled | [214](../../crates/crysirror-krkr/src/motion_plugin.rs#L214) |

### native_extra.rs / post_input

```rust
fn post_input(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name.as_str()` | `"onKeyDown"` | Local state/value operations | Handled | [270](../../crates/crysirror-krkr/src/native_extra.rs#L270) |
| `name.as_str()` | `"onKeyUp"` | Local state/value operations | Handled | [271](../../crates/crysirror-krkr/src/native_extra.rs#L271) |
| `name.as_str()` | `"onKeyPress"` | Local state/value operations | Handled | [272](../../crates/crysirror-krkr/src/native_extra.rs#L272) |
| `name.as_str()` | `_` | TjsError::runtime | Unsupported: Specified event name is unknown: {name} | [273](../../crates/crysirror-krkr/src/native_extra.rs#L273) |
| `condition` | `method == "onKeyPress"` | Local state/value operations | Handled | [284](../../crates/crysirror-krkr/src/native_extra.rs#L284) |

### native_extra.rs / release_touch

```rust
fn release_touch(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    a: Vec<Variant>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `rt.host().extra.touches.get(&id) == Some(&h.0)` | Local state/value operations | Handled | [233](../../crates/crysirror-krkr/src/native_extra.rs#L233) |

### native_extra.rs / State::invalidate

```rust
fn invalidate(&mut self, h: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.attention.is_some_and(\|(id, _)\| id == h.0)` | Local state/value operations | Handled | [22](../../crates/crysirror-krkr/src/native_extra.rs#L22) |

### native_members.rs / default_key_down

```rust
fn default_key_down(rt: &mut Rt, h: ObjectHandle, key: i64, shift: i64) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `(key == VK_TAB && shift & TVP_SS_SHIFT != 0 && shift & (TVP_SS_ALT \| TVP_SS_CTRL) == 0) \|\| matches!(key, VK_LEFT \| VK_UP)` | Local state/value operations | Handled | [299](../../crates/crysirror-krkr/src/native_members.rs#L299) |

### native_pad.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `*key == "title"` | Local state/value operations | Handled | [116](../../crates/crysirror-krkr/src/native_pad.rs#L116) |
| `condition` | `key.starts_with("font") && v.is_truthy()` | Local state/value operations | Unsupported: Pad.{key} / styled editor text presentation | [192](../../crates/crysirror-krkr/src/native_pad.rs#L192) |
| `condition` | `matches!(key, "width" \| "height") && !(70..=8192).contains(&n) \|\| key == "opacity" && !(0..=255).contains(&n) \|\| key == "showScrollBars" && !(0..=3).contains(&n) \|\| matches!(key, "fontHeight" \| "fontSize") && (!(1..=512).contains(&n.unsigned_abs()))` | TjsError::runtime | Unsupported: Pad property out of range | [201](../../crates/crysirror-krkr/src/native_pad.rs#L201) |
| `condition` | `key == "fontSize"` | Variant::Integer | Handled | [217](../../crates/crysirror-krkr/src/native_pad.rs#L217) |
| `condition` | `key == "fontHeight"` | Variant::Integer | Handled | [223](../../crates/crysirror-krkr/src/native_pad.rs#L223) |

### native_pad.rs / poll

```rust
fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `number(&rt.host().pads.objects[&id], "showScrollBars") & 2 != 0` | serde_json::from_slice, host.query_engine, EngineQuery::UiInteraction, EngineQuery::UiWidget | Handled | [347](../../crates/crysirror-krkr/src/native_pad.rs#L347) |

### native_rect.rs / edges

```rust
fn edges(rt: &Rt, object: ObjectHandle) -> Result<Edges>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!rt.object_valid(object) \|\| !rt.has_object_member(object, FIELDS[0]) \|\| !rt .object_class_infos(object) .iter() .any(\|name\| name == "Rect")` | TjsError::runtime | Unsupported: Rect instance required | [20](../../crates/crysirror-krkr/src/native_rect.rs#L20) |

### native_rect.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"setSize"` | Local state/value operations | Handled | [177](../../crates/crysirror-krkr/src/native_rect.rs#L177) |
| `name` | `"setOffset"` | Local state/value operations | Handled | [178](../../crates/crysirror-krkr/src/native_rect.rs#L178) |
| `name` | `"addOffset"` | Local state/value operations | Handled | [184](../../crates/crysirror-krkr/src/native_rect.rs#L184) |
| `name` | `"clear" \| "set"` | Local state/value operations | Handled | [190](../../crates/crysirror-krkr/src/native_rect.rs#L190) |
| `name` | `_` | Local state/value operations | Handled | [191](../../crates/crysirror-krkr/src/native_rect.rs#L191) |
| `condition; under name="clip" \| "union"` | `name == "clip"` | Local state/value operations | Handled | [216](../../crates/crysirror-krkr/src/native_rect.rs#L216) |
| `name` | `"clip" \| "union"` | Local state/value operations | Handled | [215](../../crates/crysirror-krkr/src/native_rect.rs#L215) |
| `name` | `"intersects"` | Local state/value operations | Handled | [228](../../crates/crysirror-krkr/src/native_rect.rs#L228) |
| `name` | `"included"` | Local state/value operations | Handled | [229](../../crates/crysirror-krkr/src/native_rect.rs#L229) |
| `name` | `"equal"` | Local state/value operations | Handled | [232](../../crates/crysirror-krkr/src/native_rect.rs#L232) |
| `name` | `_` | Local state/value operations | Handled | [233](../../crates/crysirror-krkr/src/native_rect.rs#L233) |

### natives.rs / composite_source

```rust
fn composite_source(
    rt: &Rt,
    h: ObjectHandle,
    src: ObjectHandle,
    dest: &LayerState,
    source: &LayerState,
    rect: [i64; 4],
    at: [i64; 2],
    opacity: u8,
    replace: bool,
    mode: i64,
) -> Option<crysirror_core::image_processing::ImageInput>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `(replace, dest.kind == 1, alpha_source)` | `_ if mode == -1` | Local state/value operations | Handled | [3525](../../crates/crysirror-krkr/src/natives.rs#L3525) |
| `(replace, dest.kind == 1, alpha_source)` | `(true, _, _)` | Local state/value operations | Handled | [3526](../../crates/crysirror-krkr/src/natives.rs#L3526) |
| `(replace, dest.kind == 1, alpha_source)` | `(false, false, true)` | Local state/value operations | Handled | [3527](../../crates/crysirror-krkr/src/natives.rs#L3527) |
| `(replace, dest.kind == 1, alpha_source)` | `(false, false, false)` | Local state/value operations | Handled | [3528](../../crates/crysirror-krkr/src/natives.rs#L3528) |
| `(replace, dest.kind == 1, alpha_source)` | `(false, true, true)` | Local state/value operations | Handled | [3529](../../crates/crysirror-krkr/src/natives.rs#L3529) |
| `(replace, dest.kind == 1, alpha_source)` | `(false, true, false)` | Local state/value operations | Handled | [3530](../../crates/crysirror-krkr/src/natives.rs#L3530) |

### natives.rs / draw_face

```rust
fn draw_face(rt: &Rt, h: ObjectHandle) -> i64
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `rt.host().layers.get(&h.0).map_or(2, \|l\| l.kind)` | `1 \| 3 \| 4 \| 5 \| 8 \| 9 \| 10 \| 11` | Local state/value operations | Handled | [2895](../../crates/crysirror-krkr/src/natives.rs#L2895) |
| `rt.host().layers.get(&h.0).map_or(2, \|l\| l.kind)` | `12` | Local state/value operations | Handled | [2896](../../crates/crysirror-krkr/src/natives.rs#L2896) |
| `rt.host().layers.get(&h.0).map_or(2, \|l\| l.kind)` | `_` | Local state/value operations | Handled | [2897](../../crates/crysirror-krkr/src/natives.rs#L2897) |

### natives.rs / font_properties

```rust
fn font_properties(rt: &mut Rt, font: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `*name` | `"face"` | Variant::String, String::new | Handled | [1702](../../crates/crysirror-krkr/src/natives.rs#L1702) |
| `*name` | `"height"` | Variant::Integer | Handled | [1703](../../crates/crysirror-krkr/src/natives.rs#L1703) |
| `*name` | `_` | Variant::Integer | Handled | [1704](../../crates/crysirror-krkr/src/natives.rs#L1704) |

### natives.rs / install_layer

```rust
fn install_layer(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!(0..=31).contains(&kind)` | TjsError::runtime | Unsupported: invalid Layer.type | [1906](../../crates/crysirror-krkr/src/natives.rs#L1906) |
| `condition` | `id == h.0` | TjsError::runtime | Unsupported: native parent cycle | [2637](../../crates/crysirror-krkr/src/natives.rs#L2637) |

### natives.rs / install_scripts

```rust
fn install_scripts(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name.to_ascii_lowercase().ends_with(".pbd") \|\| rt.host().storage()?.resolve(&name).is_some()` | Local state/value operations | Handled | [900](../../crates/crysirror-krkr/src/natives.rs#L900) |

### natives.rs / install_window

```rust
fn install_window(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"minWidth"` | Local state/value operations | Handled | [1257](../../crates/crysirror-krkr/src/natives.rs#L1257) |
| `name` | `"minHeight"` | Local state/value operations | Handled | [1258](../../crates/crysirror-krkr/src/natives.rs#L1258) |
| `name` | `"maxWidth"` | Local state/value operations | Handled | [1259](../../crates/crysirror-krkr/src/natives.rs#L1259) |
| `name` | `_` | Local state/value operations | Handled | [1260](../../crates/crysirror-krkr/src/natives.rs#L1260) |

### natives.rs / layer_copy

```rust
fn layer_copy(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    a: Vec<Variant>,
    replace: bool,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `rt.host().layer(h)?.kind` | `2 \| 13..=28` | Local state/value operations | Handled | [3246](../../crates/crysirror-krkr/src/natives.rs#L3246) |
| `rt.host().layer(h)?.kind` | `12` | Local state/value operations | Handled | [3247](../../crates/crysirror-krkr/src/natives.rs#L3247) |
| `rt.host().layer(h)?.kind` | `_` | Local state/value operations | Handled | [3248](../../crates/crysirror-krkr/src/natives.rs#L3248) |
| `condition` | `atop && !matches!(mode, 2 \| 13 \| 128 \| 16 \| 17)` | Local state/value operations | Handled | [3277](../../crates/crysirror-krkr/src/natives.rs#L3277) |
| `(atop, mode)` | `(false, _)` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [3292](../../crates/crysirror-krkr/src/natives.rs#L3292) |
| `(atop, mode)` | `_ if copy_main` | Local state/value operations | Handled | [3293](../../crates/crysirror-krkr/src/natives.rs#L3293) |
| `(atop, mode)` | `(true, 2 \| 13 \| 128)` | Local state/value operations | Handled | [3294](../../crates/crysirror-krkr/src/natives.rs#L3294) |
| `(atop, mode)` | `(true, 16)` | Local state/value operations | Handled | [3295](../../crates/crysirror-krkr/src/natives.rs#L3295) |
| `(atop, mode)` | `(true, 17)` | Local state/value operations | Handled | [3296](../../crates/crysirror-krkr/src/natives.rs#L3296) |
| `(atop, mode)` | `(true, m)` | Local state/value operations | Unsupported: {member} holdAlpha mode {m} / keep-alpha canvas blend for this operation mode | [3297](../../crates/crysirror-krkr/src/natives.rs#L3297) |
| `condition` | `!matches!(mode, 2 \| 128) && atop_mode.is_none()` | Local state/value operations | Unsupported: {member} mode {mode} / canvas blend operation modes | [3314](../../crates/crysirror-krkr/src/natives.rs#L3314) |

### natives.rs / layer_get_layer_at

```rust
fn layer_get_layer_at(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `exclude_self && id == h.0` | Local state/value operations | Handled | [2809](../../crates/crysirror-krkr/src/natives.rs#L2809) |
| `condition` | `kind == 1` | TjsError::runtime | Handled | [2817](../../crates/crysirror-krkr/src/natives.rs#L2817) |
| `condition; under receiver=else: kind == 1` | `layer.kind == 1` | Local state/value operations | Handled | [2825](../../crates/crysirror-krkr/src/natives.rs#L2825) |

### natives.rs / layer_instance_defaults

```rust
fn layer_instance_defaults(rt: &mut Rt, h: ObjectHandle, window: ObjectHandle) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `key == "__preLeft"` | Local state/value operations | Handled | [1800](../../crates/crysirror-krkr/src/natives.rs#L1800) |

### natives.rs / layer_node_enabled_plain

```rust
fn layer_node_enabled_plain(rt: &Rt, h: ObjectHandle) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!rt.object_member(ObjectHandle(id), "__lp_enabled").is_truthy()` | Local state/value operations | Handled | [148](../../crates/crysirror-krkr/src/natives.rs#L148) |

### natives.rs / layer_operate_affine

```rust
fn layer_operate_affine(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!(mode, 2 \| 128)` | Local state/value operations | Unsupported: {M} mode {mode} / canvas blend operation modes | [3882](../../crates/crysirror-krkr/src/natives.rs#L3882) |

### natives.rs / layer_operate_stretch

```rust
fn layer_operate_stretch(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!(mode, 2 \| 128)` | Local state/value operations | Unsupported: {M} mode {mode} / canvas blend operation modes | [3776](../../crates/crysirror-krkr/src/natives.rs#L3776) |

### natives.rs / ps_blend

```rust
fn ps_blend(mode: i64) -> Option<crysirror_core::image_processing::PsBlend>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `mode` | `13` | Local state/value operations | Handled | [3411](../../crates/crysirror-krkr/src/natives.rs#L3411) |
| `mode` | `14` | Local state/value operations | Handled | [3412](../../crates/crysirror-krkr/src/natives.rs#L3412) |
| `mode` | `15` | Local state/value operations | Handled | [3413](../../crates/crysirror-krkr/src/natives.rs#L3413) |
| `mode` | `16` | Local state/value operations | Handled | [3414](../../crates/crysirror-krkr/src/natives.rs#L3414) |
| `mode` | `17` | Local state/value operations | Handled | [3415](../../crates/crysirror-krkr/src/natives.rs#L3415) |
| `mode` | `18` | Local state/value operations | Handled | [3416](../../crates/crysirror-krkr/src/natives.rs#L3416) |
| `mode` | `19` | Local state/value operations | Handled | [3417](../../crates/crysirror-krkr/src/natives.rs#L3417) |
| `mode` | `20` | Local state/value operations | Handled | [3418](../../crates/crysirror-krkr/src/natives.rs#L3418) |
| `mode` | `21` | Local state/value operations | Handled | [3419](../../crates/crysirror-krkr/src/natives.rs#L3419) |
| `mode` | `23` | Local state/value operations | Handled | [3420](../../crates/crysirror-krkr/src/natives.rs#L3420) |
| `mode` | `24` | Local state/value operations | Handled | [3421](../../crates/crysirror-krkr/src/natives.rs#L3421) |
| `mode` | `25` | Local state/value operations | Handled | [3422](../../crates/crysirror-krkr/src/natives.rs#L3422) |
| `mode` | `22` | Local state/value operations | Handled | [3423](../../crates/crysirror-krkr/src/natives.rs#L3423) |
| `mode` | `26` | Local state/value operations | Handled | [3424](../../crates/crysirror-krkr/src/natives.rs#L3424) |
| `mode` | `27` | Local state/value operations | Handled | [3425](../../crates/crysirror-krkr/src/natives.rs#L3425) |
| `mode` | `28` | Local state/value operations | Handled | [3426](../../crates/crysirror-krkr/src/natives.rs#L3426) |
| `mode` | `_` | Local state/value operations | Return only: Boolean/optional result interpreted by the caller | [3427](../../crates/crysirror-krkr/src/natives.rs#L3427) |

### piled_copy.rs / copy

```rust
fn copy(rt: &mut Rt, this: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!raw_image && rt.host().layer(source)?.kind == 1` | Local state/value operations | Handled | [82](../../crates/crysirror-krkr/src/piled_copy.rs#L82) |

### piled_copy.rs / validate_visible_profile

```rust
fn validate_visible_profile(
    rt: &Rt,
    h: ObjectHandle,
    root: bool,
    visited: &mut BTreeSet<usize>,
) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `kind` | `2 \| 12` | Local state/value operations | Empty handler: Original no-op status not established | [261](../../crates/crysirror-krkr/src/piled_copy.rs#L261) |
| `kind` | `1 if root` | Local state/value operations | Empty handler: Original no-op status not established | [262](../../crates/crysirror-krkr/src/piled_copy.rs#L262) |
| `kind` | `1 if layer.full_alpha == Some(255) && layer.image_left <= 0 && layer.image_top <= 0 && layer .image_left .checked_add(layer.image_width) .is_some_and(\|r\| r >= layer.width) && layer .image_top .checked_add(layer.image_height) .is_some_and(\|b\| b >= layer.height)` | Local state/value operations | Empty handler: Original no-op status not established | [263](../../crates/crysirror-krkr/src/piled_copy.rs#L263) |
| `kind` | `0 if !root && (layer.blank \|\| !layer.has_image)` | Local state/value operations | Empty handler: Original no-op status not established | [274](../../crates/crysirror-krkr/src/piled_copy.rs#L274) |
| `kind` | `_` | Local state/value operations | Unsupported: Layer.piledCopy source type={kind} / normal-alpha or proven opaque source content | [275](../../crates/crysirror-krkr/src/piled_copy.rs#L275) |

### plugin_ports/packinone.rs / data_pack_storage_name

```rust
fn data_pack_storage_name(name: &str) -> String
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name .rsplit_once('.') .is_some_and(\|(_, extension)\| extension.eq_ignore_ascii_case("pbd"))` | Local state/value operations | Handled | [1259](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L1259) |

### plugin_ports/packinone.rs / walk_png

```rust
fn walk_png(bytes: &[u8], mut segment: impl FnMut(usize, &[u8], &[u8]) -> Option<()>) -> Option<usize>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `kind == b"IEND"` | Local state/value operations | Handled | [1251](../../crates/crysirror-krkr/src/plugin_ports/packinone.rs#L1251) |

### plugin_ports/scripts_ex.rs / clone_scripts_value

```rust
fn clone_scripts_value(
    runtime: &mut Runtime<KrkrHost>,
    value: &Variant,
    cloned: &mut BTreeMap<ObjectHandle, ObjectHandle>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!(runtime.object_member(source, "clone"), Variant::Void) && let Ok(result) = runtime.call_object_method(source, "clone", Vec::new())` | Local state/value operations | Handled | [662](../../crates/crysirror-krkr/src/plugin_ports/scripts_ex.rs#L662) |

### plugin_ports/win32_dialog.rs / message_box

```rust
fn message_box(
    rt: &mut Runtime<KrkrHost>,
    _this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `kind & 0xF` | `0` | Local state/value operations | Handled | [434](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L434) |
| `kind & 0xF` | `1` | Local state/value operations | Handled | [435](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L435) |
| `kind & 0xF` | `2` | Local state/value operations | Handled | [436](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L436) |
| `kind & 0xF` | `3` | Local state/value operations | Handled | [437](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L437) |
| `kind & 0xF` | `4` | Local state/value operations | Handled | [438](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L438) |
| `kind & 0xF` | `5` | Local state/value operations | Handled | [439](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L439) |
| `kind & 0xF` | `6` | Local state/value operations | Handled | [440](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L440) |
| `kind & 0xF` | `other` | crate::natives::unsupported | Unsupported: {M} button set {other} / Win32 message box button sets 0..=6 | [441](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L441) |
| `condition` | `kind & 0x4000 != 0` | crate::natives::unsupported | Unsupported: {M} MB_HELP / a dialog help callback | [448](../../crates/crysirror-krkr/src/plugin_ports/win32_dialog.rs#L448) |

### plugin_ports/window_ex.rs / menu_item_property_get

```rust
fn menu_item_property_get(
    runtime: &mut Runtime<KrkrHost>,
    this_obj: Option<ObjectHandle>,
    name: &'static str,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"rightJustify"` | Variant::Integer, i64::from | Handled | [1464](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1464) |
| `name` | `_` | Variant::Integer | Handled | [1468](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1468) |

### plugin_ports/window_ex.rs / menu_item_property_set

```rust
fn menu_item_property_set(
    runtime: &mut Runtime<KrkrHost>,
    this_obj: Option<ObjectHandle>,
    name: &'static str,
    value: Variant,
) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"rightJustify"` | Local state/value operations | Handled | [1490](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1490) |
| `name` | `_` | Variant::Integer, TjsError::runtime, Variant::Object | Handled | [1499](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1499) |
| `name` | `"rightJustify"` | Local state/value operations | Handled | [1528](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1528) |
| `name` | `"bmpChecked"` | Local state/value operations | Handled | [1529](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1529) |
| `name` | `"bmpUnchecked"` | Local state/value operations | Handled | [1530](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1530) |
| `name` | `_` | Local state/value operations | Handled | [1531](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1531) |

### plugin_ports/window_ex.rs / window_property_get

```rust
fn window_property_get(
    runtime: &mut Runtime<KrkrHost>,
    this_obj: Option<ObjectHandle>,
    name: &'static str,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | Variant::Integer | Handled | [1389](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1389) |
| `name` | `"disableResize" \| "disableMove" \| "enableNCMouseEvent"` | Variant::Integer, i64::from | Handled | [1392](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1392) |
| `name` | `"exSystemMenu"` | Local state/value operations | Handled | [1398](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1398) |
| `name` | `_` | Local state/value operations | Handled | [1402](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1402) |

### plugin_ports/window_ex.rs / window_property_set

```rust
fn window_property_set(
    runtime: &mut Runtime<KrkrHost>,
    this_obj: Option<ObjectHandle>,
    name: &'static str,
    value: Variant,
) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name; under name="maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | `"maximizeBox"` | Local state/value operations | Handled | [1427](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1427) |
| `name; under name="maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | `"minimizeBox"` | Local state/value operations | Handled | [1428](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1428) |
| `name; under name="maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | `"maximized"` | Local state/value operations | Handled | [1429](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1429) |
| `name; under name="maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | `_` | Local state/value operations | Handled | [1430](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1430) |
| `name` | `"disableResize" \| "disableMove" \| "enableNCMouseEvent"` | Local state/value operations | Handled | [1417](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1417) |
| `name` | `"maximizeBox" \| "minimizeBox" \| "maximized" \| "minimized"` | Local state/value operations | Handled | [1424](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1424) |
| `name` | `"exSystemMenu"` | Local state/value operations | Handled | [1437](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1437) |
| `name` | `_` | Local state/value operations | Empty handler: Original no-op status not established | [1445](../../crates/crysirror-krkr/src/plugin_ports/window_ex.rs#L1445) |

### plugins.rs / canonical

```rust
fn canonical(name: &str) -> Result<String>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name.is_empty() \|\| name.len() > 128 \|\| name.contains('\0')` | TjsError::runtime | Unsupported: invalid native plugin name | [16](../../crates/crysirror-krkr/src/plugins.rs#L16) |

### plugins.rs / install

```rust
fn install(rt: &mut Rt, plugins: crysirror_tjs2::runtime::ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(name.as_str(), "motionplayer.dll" \| "motionplayer_nod3d.dll" \| "emoteplayer.dll")` | crate::motion_plugin::install | Handled | [159](../../crates/crysirror-krkr/src/plugins.rs#L159) |

### pointer.rs / apply

```rust
fn apply(rt: &mut Rt, target: ObjectHandle) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let Variant::Integer(v) = rt.object_member(ObjectHandle(id), "__cursor") && v != 0` | Local state/value operations | Handled | [292](../../crates/crysirror-krkr/src/pointer.rs#L292) |

### pointer.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `key` | `1` | Local state/value operations | Handled | [179](../../crates/crysirror-krkr/src/pointer.rs#L179) |
| `key` | `2` | Local state/value operations | Handled | [180](../../crates/crysirror-krkr/src/pointer.rs#L180) |
| `key` | `437..=479` | Local state/value operations | Handled | [181](../../crates/crysirror-krkr/src/pointer.rs#L181) |
| `key` | `_` | Local state/value operations | Handled | [182](../../crates/crysirror-krkr/src/pointer.rs#L182) |

### protect_plugin.rs / construct

```rust
fn construct(rt: &mut Rt, h: ObjectHandle, a: Vec<Variant>) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `kind.is_empty() \|\| !kind.chars().all(\|c\| c.is_ascii_lowercase()) \|\| kind == "file"` | Local state/value operations | Unsupported: CompoundStorageMedia: invalid media type | [479](../../crates/crysirror-krkr/src/protect_plugin.rs#L479) |

### psd_plugin.rs / PsdIndex::lookup

```rust
fn lookup(&self, name: &str) -> Option<usize>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let Some(rest) = name.strip_prefix("id/")` | self.ids.get | Handled | [79](../../crates/crysirror-krkr/src/psd_plugin.rs#L79) |

### sound.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `b.opening && !matches!(name, "status" \| "volume" \| "volume2" \| "pan" \| "paused" \| "looping" \| "labels")` | Local state/value operations | Unsupported: WaveSoundBuffer.{name} while an open from a native callback is preparing / synchronous audio metadata inside non-resumable callbacks | [451](../../crates/crysirror-krkr/src/sound.rs#L451) |
| `condition` | `matches!(name, "status" \| "volume" \| "volume2" \| "pan" \| "paused" \| "looping" \| "labels")` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [451](../../crates/crysirror-krkr/src/sound.rs#L451) |
| `name` | `"status"` | Variant::String | Handled | [458](../../crates/crysirror-krkr/src/sound.rs#L458) |
| `name` | `"volume"` | Variant::Integer | Handled | [459](../../crates/crysirror-krkr/src/sound.rs#L459) |
| `name` | `"volume2"` | Variant::Integer | Handled | [460](../../crates/crysirror-krkr/src/sound.rs#L460) |
| `name` | `"pan"` | Variant::Integer | Handled | [461](../../crates/crysirror-krkr/src/sound.rs#L461) |
| `name` | `"paused"` | Variant::Integer | Handled | [462](../../crates/crysirror-krkr/src/sound.rs#L462) |
| `name` | `"looping"` | Variant::Integer | Handled | [463](../../crates/crysirror-krkr/src/sound.rs#L463) |
| `name` | `"samplePosition"` | Variant::Integer | Handled | [464](../../crates/crysirror-krkr/src/sound.rs#L464) |
| `name` | `"position"` | Variant::Integer | Handled | [465](../../crates/crysirror-krkr/src/sound.rs#L465) |
| `name` | `"totalSamples"` | Variant::Integer | Handled | [470](../../crates/crysirror-krkr/src/sound.rs#L470) |
| `name` | `"totalTime"` | Variant::Integer | Handled | [473](../../crates/crysirror-krkr/src/sound.rs#L473) |
| `name` | `"frequency"` | Variant::Integer | Handled | [476](../../crates/crysirror-krkr/src/sound.rs#L476) |
| `name` | `"labels"` | Variant::Integer, Variant::Object | Handled | [480](../../crates/crysirror-krkr/src/sound.rs#L480) |
| `name` | `_` | Local state/value operations | Handled | [489](../../crates/crysirror-krkr/src/sound.rs#L489) |
| `condition; under name="volume" \| "volume2"` | `name == "volume"` | Local state/value operations | Handled | [497](../../crates/crysirror-krkr/src/sound.rs#L497) |
| `condition; under name="position" \| "samplePosition"` | `name == "position"` | Local state/value operations | Handled | [559](../../crates/crysirror-krkr/src/sound.rs#L559) |
| `name` | `"volume" \| "volume2"` | Local state/value operations | Handled | [495](../../crates/crysirror-krkr/src/sound.rs#L495) |
| `name` | `"pan"` | Local state/value operations | Handled | [512](../../crates/crysirror-krkr/src/sound.rs#L512) |
| `name` | `"paused"` | Local state/value operations | Handled | [525](../../crates/crysirror-krkr/src/sound.rs#L525) |
| `name` | `"looping"` | Local state/value operations | Handled | [538](../../crates/crysirror-krkr/src/sound.rs#L538) |
| `name` | `"position" \| "samplePosition"` | u64::try_from | Handled | [551](../../crates/crysirror-krkr/src/sound.rs#L551) |
| `name` | `"frequency"` | Local state/value operations | Handled | [569](../../crates/crysirror-krkr/src/sound.rs#L569) |
| `name` | `_` | Local state/value operations | Unsupported: read-only sound property | [579](../../crates/crysirror-krkr/src/sound.rs#L579) |

### sound.rs / Sounds::invalidate

```rust
fn invalidate(&mut self, object: usize) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(op, Op::Release(id) if *id == object)` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [83](../../crates/crysirror-krkr/src/sound.rs#L83) |

### storage.rs / Storage::full_uri

```rust
fn full_uri(&self, name: &str) -> Result<String>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name.as_bytes().get(1) == Some(&b':')` | self.media.as_ref, self.root.is_empty | Partial: drive-relative native paths unsupported | [244](../../crates/crysirror-krkr/src/storage.rs#L244) |
| `condition` | `name.starts_with("file://./")` | Local state/value operations | Handled | [272](../../crates/crysirror-krkr/src/storage.rs#L272) |

### storage_plugins.rs / install_save_struct

```rust
fn install_save_struct(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `class_name == "Scripts" && method == "saveStruct2"` | Local state/value operations | Handled | [69](../../crates/crysirror-krkr/src/storage_plugins.rs#L69) |

### text_render_plugin.rs / install

```rust
fn install(rt: &mut Rt) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"fontScale"` | Local state/value operations | Handled | [360](../../crates/crysirror-krkr/src/text_render_plugin.rs#L360) |
| `name` | `"timeScale"` | Local state/value operations | Handled | [361](../../crates/crysirror-krkr/src/text_render_plugin.rs#L361) |
| `name` | `_` | Variant::Integer | Handled | [362](../../crates/crysirror-krkr/src/text_render_plugin.rs#L362) |
| `name` | `"fontScale"` | markup::finite | Handled | [370](../../crates/crysirror-krkr/src/text_render_plugin.rs#L370) |
| `name` | `"timeScale"` | markup::finite | Handled | [371](../../crates/crysirror-krkr/src/text_render_plugin.rs#L371) |
| `name` | `_` | Local state/value operations | Handled | [372](../../crates/crysirror-krkr/src/text_render_plugin.rs#L372) |

### text_render_plugin.rs / output

```rust
fn output(rt: &mut Rt, t: Option<ObjectHandle>, name: &str) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"renderText"` | Local state/value operations | Handled | [190](../../crates/crysirror-krkr/src/text_render_plugin.rs#L190) |
| `name` | `"renderCount"` | Variant::Integer | Handled | [191](../../crates/crysirror-krkr/src/text_render_plugin.rs#L191) |
| `name` | `"renderLines"` | Variant::Integer | Handled | [192](../../crates/crysirror-krkr/src/text_render_plugin.rs#L192) |
| `name` | `"renderDelay"` | Local state/value operations | Handled | [193](../../crates/crysirror-krkr/src/text_render_plugin.rs#L193) |
| `name` | `"renderOver"` | Variant::Integer | Handled | [194](../../crates/crysirror-krkr/src/text_render_plugin.rs#L194) |
| `name` | `"renderLeft"` | Local state/value operations | Handled | [195](../../crates/crysirror-krkr/src/text_render_plugin.rs#L195) |
| `name` | `"renderRight"` | Local state/value operations | Handled | [196](../../crates/crysirror-krkr/src/text_render_plugin.rs#L196) |
| `name` | `"renderTop"` | Local state/value operations | Handled | [197](../../crates/crysirror-krkr/src/text_render_plugin.rs#L197) |
| `name` | `"renderBottom"` | Local state/value operations | Handled | [198](../../crates/crysirror-krkr/src/text_render_plugin.rs#L198) |
| `name` | `"maxScrollOffset"` | Local state/value operations | Handled | [199](../../crates/crysirror-krkr/src/text_render_plugin.rs#L199) |
| `name` | `_` | Variant::Integer | Handled | [200](../../crates/crysirror-krkr/src/text_render_plugin.rs#L200) |

### transitions.rs / begin

```rust
fn begin(rt: &mut Rt, t: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name != "crossfade" && name != "universal" && !kaicho && !glitch` | Local state/value operations | Unsupported: transition {name} / an engine transition implementation | [68](../../crates/crysirror-krkr/src/transitions.rs#L68) |
| `condition` | `name == "universal"` | TjsError::runtime | Handled | [125](../../crates/crysirror-krkr/src/transitions.rs#L125) |

### transitions.rs / exchange_tree

```rust
fn exchange_tree(rt:&mut Rt,a:ObjectHandle,b:ObjectHandle,keep:bool) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `id==a.0` | Local state/value operations | Handled | [335](../../crates/crysirror-krkr/src/transitions.rs#L335) |
| `condition` | `id==b.0` | Local state/value operations | Handled | [335](../../crates/crysirror-krkr/src/transitions.rs#L335) |
| `condition` | `next[&replaced].0==Some(id)` | Local state/value operations | Handled | [373](../../crates/crysirror-krkr/src/transitions.rs#L373) |
| `condition` | `next[&child].0==Some(id)` | Local state/value operations | Handled | [373](../../crates/crysirror-krkr/src/transitions.rs#L373) |
| `condition` | `next[&child].0==Some(id)&&!children.contains(&child)` | Local state/value operations | Handled | [376](../../crates/crysirror-krkr/src/transitions.rs#L376) |

### varfile_plugin.rs / parts

```rust
fn parts(name: &str) -> Result<Vec<&str>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `name.len() > 4096 \|\| name.contains('\0') \|\| !is_uri(name)` | Local state/value operations | Unsupported: invalid URI | [21](../../crates/crysirror-krkr/src/varfile_plugin.rs#L21) |

### varfile_plugin.rs / request

```rust
fn request(rt: &mut Rt, request: StorageRequest) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under request.action=StorageAction::ReadBinary` | `!matches!(request.mode.as_str(), "" \| "b")` | Local state/value operations | Unsupported: offset/read stream modes are not mapped | [121](../../crates/crysirror-krkr/src/varfile_plugin.rs#L121) |
| `condition; under request.action=StorageAction::WriteBinary(bytes)` | `!matches!(request.mode.as_str(), "" \| "b")` | Local state/value operations | Unsupported: offset/append stream modes are not mapped | [136](../../crates/crysirror-krkr/src/varfile_plugin.rs#L136) |

### video.rs / drive

```rust
fn drive(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Ok(_) \| Err(crysirror_core::Error::NotFound)` | Local state/value operations | Empty handler: Original no-op status not established | [595](../../crates/crysirror-krkr/src/video.rs#L595) |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Err(e) if e.to_string().contains("media handle not found")` | Local state/value operations | Empty handler: Original no-op status not established | [596](../../crates/crysirror-krkr/src/video.rs#L596) |
| `host.media_request(client, MediaRequest::Stop { id: media }); under request=Request::Stop(id)` | `Err(e)` | Local state/value operations | Unsupported: Unconditional rejection | [597](../../crates/crysirror-krkr/src/video.rs#L597) |

### video.rs / present

```rust
fn present(rt: &mut Rt, h: ObjectHandle) -> anyhow::Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `mode == 1` | Local state/value operations | Handled | [482](../../crates/crysirror-krkr/src/video.rs#L482) |

### wf_plugins.rs / oscillator

```rust
fn oscillator(rt: &Rt, h: ObjectHandle, depth: u32) -> Result<Oscillator>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `kind.as_str()` | `"const"` | Local state/value operations | Handled | [443](../../crates/crysirror-krkr/src/wf_plugins.rs#L443) |
| `kind.as_str()` | `"sine"` | Local state/value operations | Handled | [444](../../crates/crysirror-krkr/src/wf_plugins.rs#L444) |
| `kind.as_str()` | `"noise"` | Local state/value operations | Handled | [445](../../crates/crysirror-krkr/src/wf_plugins.rs#L445) |
| `kind.as_str()` | `"blit"` | Local state/value operations | Handled | [449](../../crates/crysirror-krkr/src/wf_plugins.rs#L449) |
| `kind.as_str()` | `"saw"` | Local state/value operations | Handled | [450](../../crates/crysirror-krkr/src/wf_plugins.rs#L450) |
| `kind.as_str()` | `"square"` | Local state/value operations | Handled | [451](../../crates/crysirror-krkr/src/wf_plugins.rs#L451) |
| `kind.as_str()` | `"modulate"` | Local state/value operations | Handled | [452](../../crates/crysirror-krkr/src/wf_plugins.rs#L452) |
| `kind.as_str()` | `"mix"` | Local state/value operations | Handled | [458](../../crates/crysirror-krkr/src/wf_plugins.rs#L458) |
| `kind.as_str()` | `"filter"` | Box::new | Handled | [461](../../crates/crysirror-krkr/src/wf_plugins.rs#L461) |
| `kind.as_str()` | `"loop"` | crate::wv_decoder::mono, f64::from | Handled | [471](../../crates/crysirror-krkr/src/wf_plugins.rs#L471) |
| `kind.as_str()` | `other` | Local state/value operations | Unsupported: unknown oscillator kind {other} | [475](../../crates/crysirror-krkr/src/wf_plugins.rs#L475) |

### wf_plugins.rs / stage

```rust
fn stage(rt: &Rt, f: ObjectHandle, depth: u32) -> Result<Option<WaveFilter>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `kind.as_str()` | `"delay"` | Local state/value operations | Handled | [604](../../crates/crysirror-krkr/src/wf_plugins.rs#L604) |
| `kind.as_str()` | `"verb"` | u32::try_from | Handled | [610](../../crates/crysirror-krkr/src/wf_plugins.rs#L610) |
| `kind.as_str()` | `"eq"` | Local state/value operations | Handled | [618](../../crates/crysirror-krkr/src/wf_plugins.rs#L618) |
| `kind.as_str()` | `"dsp"` | Local state/value operations | Handled | [625](../../crates/crysirror-krkr/src/wf_plugins.rs#L625) |
| `kind.as_str()` | `"limit"` | Local state/value operations | Handled | [629](../../crates/crysirror-krkr/src/wf_plugins.rs#L629) |
| `kind.as_str()` | `"abs"` | Local state/value operations | Handled | [634](../../crates/crysirror-krkr/src/wf_plugins.rs#L634) |
| `kind.as_str()` | `"add"` | Local state/value operations | Handled | [635](../../crates/crysirror-krkr/src/wf_plugins.rs#L635) |
| `kind.as_str()` | `"mul"` | Local state/value operations | Handled | [636](../../crates/crysirror-krkr/src/wf_plugins.rs#L636) |
| `kind.as_str()` | `"ring"` | Local state/value operations | Handled | [637](../../crates/crysirror-krkr/src/wf_plugins.rs#L637) |
| `kind.as_str()` | `"vocoder"` | Local state/value operations | Handled | [638](../../crates/crysirror-krkr/src/wf_plugins.rs#L638) |
| `kind.as_str()` | `other` | Local state/value operations | Unsupported: unknown wave filter kind {other} | [645](../../crates/crysirror-krkr/src/wf_plugins.rs#L645) |

### compiler/mir.rs / MirModule::validate

```rust
fn validate(&self) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self .sources .get(span.file.0 as usize) .is_none_or(\|source\| source.id != span.file)` | TjsError::mir | Unsupported: source span references an invalid file | [60](../../crates/crysirror-tjs2/src/compiler/mir.rs#L60) |

### compiler/mir.rs / MirObject::validate_slot

```rust
fn validate_slot(&self, slot: SlotId) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under slot=SlotId::Temp(id)` | `self .frame .temps .get(id.0 as usize) .is_some_and(\|temp\| temp.id == id)` | Local state/value operations | Handled | [604](../../crates/crysirror-tjs2/src/compiler/mir.rs#L604) |
| `condition; under slot=SlotId::Local(id)` | `self .frame .locals .get(id.0 as usize) .is_some_and(\|local\| local.id == id)` | Local state/value operations | Handled | [616](../../crates/crysirror-tjs2/src/compiler/mir.rs#L616) |

### frontend/hir.rs / Analyzer::find_binding

```rust
fn find_binding(&self, name: &str) -> Option<syntax::BindingId>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.bindings[binding.0].name == name` | Local state/value operations | Handled | [639](../../crates/crysirror-tjs2/src/frontend/hir.rs#L639) |

### frontend/hir.rs / Analyzer::push_expr_tasks

```rust
fn push_expr_tasks(
        &mut self,
        expr: &'a mut syntax::Expr,
        tasks: &mut Vec<AnalyzeTask<'a>>,
    ) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under &mut expr.kind=syntax::ExprKind::Unary { op, expr: inner }` | `matches!( op, syntax::UnaryOp::Delete \| syntax::UnaryOp::Increment \| syntax::UnaryOp::Decrement )` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [536](../../crates/crysirror-tjs2/src/frontend/hir.rs#L536) |
| `condition; under &mut expr.kind=syntax::ExprKind::Postfix { op, expr: inner }` | `matches!(op, syntax::UnaryOp::Increment \| syntax::UnaryOp::Decrement)` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [584](../../crates/crysirror-tjs2/src/frontend/hir.rs#L584) |

### frontend/lexer.rs / token_can_be_member_name

```rust
fn token_can_be_member_name(kind: &TokenKind) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!( kind, TokenKind::Ident(_) \| TokenKind::KwBreak \| TokenKind::KwCase \| TokenKind::KwCatch \| TokenKind::KwClass \| TokenKind::KwConst \| TokenKind::KwContinue \| TokenKind::KwDebugger \| TokenKind::KwDefault \| TokenKind::KwDelete \| TokenKind::KwDo \| TokenKind::KwElse \| TokenKind::KwEnum \| TokenKind::KwExport \| TokenKind::KwExtends \| TokenKind::KwFalse \| TokenKind::KwFinally \| TokenKind::KwFor \| TokenKind::KwFunction \| TokenKind::KwGlobal \| TokenKind::KwGoto \| TokenKind::KwGetter \| TokenKind::KwIf \| TokenKind::KwImport \| TokenKind::KwIn \| TokenKind::KwInContextOf \| TokenKind::KwInfinity \| TokenKind::KwInstanceOf \| TokenKind::KwInt \| TokenKind::KwInvalidate \| TokenKind::KwIsValid \| TokenKind::KwNan \| TokenKind::KwNew \| TokenKind::KwNull \| TokenKind::KwOctet \| TokenKind::KwPrivate \| TokenKind::KwProperty \| TokenKind::KwProtected \| TokenKind::KwPublic \| TokenKind::KwReal \| TokenKind::KwReturn \| TokenKind::KwSetter \| TokenKind::KwStatic \| TokenKind::KwString \| TokenKind::KwSuper \| TokenKind::KwSwitch \| TokenKind::KwSynchronized \| TokenKind::KwThis \| TokenKind::KwThrow \| TokenKind::KwTrue \| TokenKind::KwTry \| TokenKind::KwTypeOf \| TokenKind::KwVar \| TokenKind::KwVoid \| TokenKind::KwWhile \| TokenKind::KwWith )` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [1699](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L1699) |

### frontend/lexer.rs / token_starts_case_header

```rust
fn token_starts_case_header(kind: &TokenKind, was_statement_start: bool) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(kind, TokenKind::KwCase \| TokenKind::KwDefault)` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [1884](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L1884) |

### frontend/lexer.rs / token_starts_control_header

```rust
fn token_starts_control_header(kind: &TokenKind, was_statement_start: bool) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!( kind, TokenKind::KwIf \| TokenKind::KwWhile \| TokenKind::KwFor \| TokenKind::KwWith \| TokenKind::KwSwitch \| TokenKind::KwCatch \| TokenKind::KwSynchronized )` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [1867](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L1867) |

### frontend/lexer.rs / token_starts_for_header

```rust
fn token_starts_for_header(kind: &TokenKind, was_statement_start: bool) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(kind, TokenKind::KwFor)` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [1880](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L1880) |

### frontend/lexer.rs / token_starts_statement_after

```rust
fn token_starts_statement_after(
    kind: &TokenKind,
    closes_control_header: bool,
    closes_function_expression_body: bool,
    closes_case_header: bool,
) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!( kind, TokenKind::Semicolon \| TokenKind::LeftBrace \| TokenKind::RightBrace \| TokenKind::KwDo \| TokenKind::KwElse \| TokenKind::KwFinally \| TokenKind::KwPrivate \| TokenKind::KwProtected \| TokenKind::KwPublic \| TokenKind::KwStatic )` | Local state/value operations | Guard / allowlist: Predicate only; execution is defined by its caller | [1899](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L1899) |

### frontend/lexer.rs / CaseHeaderContext::observe

```rust
fn observe(&mut self, kind: &TokenKind) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `kind` | `TokenKind::Colon if self.paren_depth == 0 && self.bracket_depth == 0` | Local state/value operations | Handled | [239](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L239) |
| `kind` | `TokenKind::Question if self.paren_depth == 0 && self.bracket_depth == 0` | Local state/value operations | Handled | [245](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L245) |
| `kind` | `TokenKind::LeftParen` | Local state/value operations | Handled | [248](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L248) |
| `kind` | `TokenKind::RightParen` | self.paren_depth.saturating_sub | Handled | [251](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L251) |
| `kind` | `TokenKind::LeftBracket` | Local state/value operations | Handled | [254](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L254) |
| `kind` | `TokenKind::RightBracket` | self.bracket_depth.saturating_sub | Handled | [257](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L257) |
| `kind` | `_` | Local state/value operations | Empty handler: Original no-op status not established | [260](../../crates/crysirror-tjs2/src/frontend/lexer.rs#L260) |

### frontend/parser.rs / Parser::call_arg_trailing_star_pos

```rust
fn call_arg_trailing_star_pos(&self) -> Option<usize>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 && matches!( &token.kind, TokenKind::Comma \| TokenKind::RightParen \| TokenKind::Eof )` | Local state/value operations | Handled | [1459](../../crates/crysirror-tjs2/src/frontend/parser.rs#L1459) |

### frontend/parser.rs / Parser::parse_function_expression

```rust
fn parse_function_expression(&mut self) -> Result<Expr>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!(self.nth(1).kind, TokenKind::Ident(_))` | TjsError::parse, self.nth | Unsupported: function expressions are anonymous in TJS2 | [1105](../../crates/crysirror-tjs2/src/frontend/parser.rs#L1105) |

### runtime/builtins.rs / array_sort

```rust
fn array_sort(
    runtime: &mut Runtime<H>,
    this_obj: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!runtime.heap[handle.0].array_sort_by(\|lhs, rhs\| array_sort_less(lhs, rhs, mode))?` | TjsError::runtime | Unsupported: Array.sort called on a non-array object | [886](../../crates/crysirror-tjs2/src/runtime/builtins.rs#L886) |

### runtime/mod.rs / Runtime<H>::is_dictionary_instance

```rust
fn is_dictionary_instance(&self, object: ObjectHandle) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!( self.heap[object.0].kind, ObjectKind::NativeFunction { .. } \| ObjectKind::VmNativeFunction { .. } \| ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [625](../../crates/crysirror-tjs2/src/runtime/mod.rs#L625) |

### runtime/mod.rs / Runtime<H>::set_shared_native

```rust
fn set_shared_native(
        &mut self,
        object: ObjectHandle,
        key: &'static str,
        name: &str,
        function: F,
    ) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under self.shared_natives.get(key)=None` | `let ObjectKind::NativeFunction { id, .. } = self.heap[handle.0].kind` | self.native_call_trace.native_names.get_mut | Handled | [1791](../../crates/crysirror-tjs2/src/runtime/mod.rs#L1791) |

### runtime/mod.rs / Runtime<H>::write_storage_binary

```rust
fn write_storage_binary(&mut self, name: &str, mode: &str, bytes: &[u8]) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!name.split_once("://").is_some_and(\|(scheme, _)\| { self.storage_media .contains_key(&scheme.to_ascii_lowercase()) })` | self.host.write_binary | Handled | [515](../../crates/crysirror-tjs2/src/runtime/mod.rs#L515) |

### runtime/mod.rs / Runtime<H>::write_storage_text

```rust
fn write_storage_text(&mut self, name: &str, mode: &str, text: &str) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!name.split_once("://").is_some_and(\|(scheme, _)\| { self.storage_media .contains_key(&scheme.to_ascii_lowercase()) })` | self.host.write_text | Handled | [501](../../crates/crysirror-tjs2/src/runtime/mod.rs#L501) |

### runtime/object.rs / Object::get_raw

```rust
fn get_raw(&self, name: &str) -> Option<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under &self.kind=ObjectKind::Array { elements }` | `name == "count" \|\| name == "length"` | Variant::Integer | Handled | [70](../../crates/crysirror-tjs2/src/runtime/object.rs#L70) |

### runtime/object.rs / Object::set

```rust
fn set(&mut self, name: impl Into<String>, value: Variant) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let ObjectKind::Array { elements } = &mut self.kind && (name == "count" \|\| name == "length")` | self.sync_array_members | Handled | [151](../../crates/crysirror-tjs2/src/runtime/object.rs#L151) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::call_primary_class_method

```rust
fn call_primary_class_method(
        &mut self,
        object: ObjectHandle,
        name: &str,
        args: Vec<Variant>,
    ) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!( self.runtime.heap[class_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [2143](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2143) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::call_secondary_class_method

```rust
fn call_secondary_class_method(
        &mut self,
        object: ObjectHandle,
        name: &str,
        args: Vec<Variant>,
    ) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!( self.runtime.heap[class_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [2094](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2094) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::call_string_method

```rust
fn call_string_method(
        &mut self,
        value: String,
        name: &str,
        args: Vec<Variant>,
    ) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `name` | `"toString"` | Variant::String | Handled | [2815](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2815) |
| `name` | `"escape"` | Variant::String | Handled | [2816](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2816) |
| `name` | `"sprintf"` | Variant::String | Handled | [2817](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2817) |
| `name` | `"toLowerCase"` | TjsError::bad_param_count, Variant::String | Handled | [2818](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2818) |
| `name` | `"toUpperCase"` | TjsError::bad_param_count, Variant::String | Handled | [2826](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2826) |
| `name` | `"charAt"` | TjsError::bad_param_count, usize::try_from, Variant::String, String::new | Handled | [2832](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2832) |
| `name` | `"substr" \| "substring"` | TjsError::bad_param_count, Variant::String | Handled | [2845](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2845) |
| `name` | `"replace"` | TjsError::bad_param_count, Variant::String, self.string_replace | Handled | [2867](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2867) |
| `name` | `"indexOf"` | TjsError::bad_param_count, Variant::Integer | Handled | [2883](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2883) |
| `name` | `"lastIndexOf"` | Variant::Integer | Handled | [2910](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2910) |
| `name` | `"split"` | TjsError::bad_param_count, self.runtime.alloc_array_object, Variant::Object | Handled | [2923](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2923) |
| `name` | `"trim"` | TjsError::bad_param_count, Variant::String | Handled | [2950](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2950) |
| `name` | `"reverse"` | TjsError::bad_param_count, Variant::String | Handled | [2957](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2957) |
| `name` | `"repeat"` | TjsError::bad_param_count, Variant::String, String::new | Handled | [2964](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2964) |
| `name` | `_` | TjsError::member_not_found | Unsupported: Unconditional rejection | [2978](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2978) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::class_chain_owner

```rust
fn class_chain_owner(
        &mut self,
        handle: ObjectHandle,
        name: &str,
    ) -> Result<Option<ObjectHandle>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.heap[class_handle.0].get_raw(name).is_some()` | Local state/value operations | Handled | [1987](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1987) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::class_chain_provides_member

```rust
fn class_chain_provides_member(&mut self, handle: ObjectHandle, name: &str) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.heap[class_handle.0].get_raw(name).is_some()` | Local state/value operations | Handled | [2702](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2702) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::class_member_for_call

```rust
fn class_member_for_call(
        &mut self,
        class_handle: ObjectHandle,
        instance: ObjectHandle,
        name: &str,
    ) -> Result<Option<Variant>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let Some(member) = self.runtime.heap[class_handle.0].get_raw(name)` | self.bind_proxy_value | Handled | [2006](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2006) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::class_member_owner

```rust
fn class_member_owner(
        &mut self,
        class_handle: ObjectHandle,
        name: &str,
        skip_primary: bool,
    ) -> Result<Option<ObjectHandle>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.heap[class_handle.0].get_raw(name).is_some()` | Local state/value operations | Handled | [1881](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1881) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::default_prop_get

```rust
fn default_prop_get(
        &mut self,
        target: Variant,
        caller_this: Option<ObjectHandle>,
    ) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let ObjectKind::InterCode { file_id, object_index, context: BytecodeContextType::Property, } = self.runtime.heap[handle.0].kind` | self.runtime.script_file, self.effective_member_this, self.execute_file_object_with_this, Vec::new, TjsError::access_denied | Unsupported: Unconditional rejection | [786](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L786) |
| `condition` | `let ObjectKind::NativeProperty { id, access } = self.runtime.heap[handle.0].kind` | TjsError::access_denied, TjsError::runtime, self.effective_member_this | Handled | [808](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L808) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::default_prop_set

```rust
fn default_prop_set(
        &mut self,
        target: Variant,
        value: Variant,
        caller_this: Option<ObjectHandle>,
    ) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let ObjectKind::InterCode { file_id, object_index, context: BytecodeContextType::Property, } = self.runtime.heap[handle.0].kind` | self.runtime.script_file, self.effective_member_this, self.execute_file_object_with_this, TjsError::access_denied | Unsupported: Unconditional rejection | [835](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L835) |
| `condition` | `let ObjectKind::NativeProperty { id, access } = self.runtime.heap[handle.0].kind` | TjsError::access_denied, TjsError::runtime, self.effective_member_this | Handled | [852](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L852) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::execute_update_property

```rust
fn execute_update_property(
        &mut self,
        frame: &mut Frame,
        object: &CodeObject,
        inst: &Instruction,
    ) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `inst.opcode` | `19 \| 23` | self.data_slot_string, self.operate_property, OpMember::Name | Handled | [1164](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1164) |
| `inst.opcode` | `20 \| 24` | self.operate_property, OpMember::Value | Handled | [1181](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1181) |
| `inst.opcode` | `21 \| 25` | self.default_prop_get, self.default_prop_set | Handled | [1198](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1198) |
| `inst.opcode` | `_` | Local state/value operations | Handled | [1209](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1209) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::handle_class_name_matches

```rust
fn handle_class_name_matches(&self, handle: ObjectHandle, name: &str) -> bool
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.heap[handle.0] .class_infos .iter() .any(\|info\| info == name)` | Local state/value operations | Handled | [3182](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L3182) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::instance_of

```rust
fn instance_of(&mut self, value: &Variant, class_name: &str) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `class_name == "Class" && matches!( self.runtime.heap[handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [3288](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L3288) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::is_class_in_instance_chain

```rust
fn is_class_in_instance_chain(
        &mut self,
        instance: ObjectHandle,
        class_handle: ObjectHandle,
    ) -> Result<bool>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `matches!( self.runtime.heap[class_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [2046](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L2046) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::member_in_super_chain

```rust
fn member_in_super_chain(
        &mut self,
        handle: ObjectHandle,
        name: &str,
    ) -> Result<Option<Variant>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let Some(value) = self.runtime.heap[class_handle.0].get_raw(name)` | Local state/value operations | Handled | [768](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L768) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::octet_property

```rust
fn octet_property(&self, value: &[u8], member: MemberKind<'_>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under member=MemberKind::Name(name)` | `name == "length"` | Variant::Integer | Handled | [1098](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1098) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::operate_property

```rust
fn operate_property(
        &mut self,
        object_value: Variant,
        member: OpMember<'_>,
        rhs: Option<Variant>,
        caller_this: Option<ObjectHandle>,
        op: impl FnOnce(Variant, Option<Variant>) -> Result<Variant>,
    ) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.is_dictionary_instance(handle) && !self.runtime.heap[handle.0].call_missing && self.runtime.heap[handle.0].get_raw(&name).is_none()` | self.prop_set, DispatchFlags::ensure | Handled | [1322](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1322) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::prop_get_handle

```rust
fn prop_get_handle(
        &mut self,
        handle: ObjectHandle,
        name: &str,
        flags: DispatchFlags,
        caller_this: Option<ObjectHandle>,
    ) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let ObjectKind::Proxy { primary, fallback, bind_this, } = self.runtime.heap[handle.0].kind` | self.prop_get_handle, self.bind_proxy_value, self.handle_class_name_matches, Variant::Closure, Closure::new | Handled | [143](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L143) |
| `condition` | `let Some(this_obj) = bind_this && !flags.no_bound_instance_fallback && let Some(value) = self.runtime.heap[this_obj.0].get_raw(name) && !self.runtime.variant_is_property(&value)` | Local state/value operations | Handled | [170](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L170) |
| `condition` | `flags.must_exist && self.runtime.heap[handle.0].array_index_missing(name)` | Local state/value operations | Handled | [192](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L192) |
| `condition` | `let Some(this_obj) = bind_this && !flags.must_exist && !flags.no_bound_instance_fallback && let Some(value) = self.runtime.heap[this_obj.0].get_raw(name) && !self.runtime.variant_is_property(&value)` | Local state/value operations | Handled | [272](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L272) |
| `condition` | `bind_this.is_none() && let Variant::Object(native) = &value && matches!( self.runtime.heap[native.0].kind, ObjectKind::NativeFunction { constructable: false, .. } \| ObjectKind::VmNativeFunction { .. } )` | self.is_class_object, self.bind_proxy_value | Handled | [319](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L319) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::prop_set_handle

```rust
fn prop_set_handle(
        &mut self,
        handle: ObjectHandle,
        name: &str,
        value: Variant,
        flags: DispatchFlags,
        caller_this: Option<ObjectHandle>,
    ) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `let ObjectKind::Proxy { primary, fallback, bind_this, } = self.runtime.heap[handle.0].kind` | self.member_in_super_chain, self.set_bound_member, self.prop_set_handle | Handled | [496](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L496) |
| `condition` | `!flags.ignore_prop && let Some(existing) = self.runtime.heap[handle.0].get_raw(name) && self .property_setter(existing, value.clone(), caller_this)? .is_some()` | Local state/value operations | Handled | [578](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L578) |
| `condition` | `let Some(existing) = self.runtime.heap[class_handle.0].get_raw(name) && self .property_setter(existing, value.clone(), receiver)? .is_some()` | Local state/value operations | Handled | [590](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L590) |
| `condition` | `self.runtime.heap[handle.0].array_negative_index_after_wrap(name)` | TjsError::member_not_found | Unsupported: Unconditional rejection | [629](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L629) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::register_object_members

```rust
fn register_object_members(
        &mut self,
        source: ObjectHandle,
        dest: ObjectHandle,
    ) -> Result<()>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `self.runtime.heap[dest.0].call_missing && self.runtime.heap[dest.0].get_raw(&name).is_none() && self.call_set_missing(dest, &name, value.clone())?` | Local state/value operations | Handled | [3379](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L3379) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::secondary_class_member_for_call

```rust
fn secondary_class_member_for_call(
        &mut self,
        qualifier: ObjectHandle,
        instance: ObjectHandle,
        name: &str,
    ) -> Result<Option<Variant>>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `!matches!( self.runtime.heap[qualifier.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } ) \|\| !self.is_class_in_instance_chain(instance, qualifier)?` | Local state/value operations | Handled | [1774](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1774) |
| `condition` | `!matches!( self.runtime.heap[class_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } )` | Local state/value operations | Handled | [1811](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1811) |
| `condition` | `let Some(member) = self.runtime.heap[class_handle.0].get_raw(name)` | self.bind_proxy_value | Handled | [1820](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1820) |

### vm/dispatch.rs / Vm<'bc, 'rt, H>::string_property

```rust
fn string_property(&self, value: &str, member: MemberKind<'_>) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under member=MemberKind::Name(name)` | `name == "length"` | Variant::Integer | Handled | [1055](../../crates/crysirror-tjs2/src/vm/dispatch.rs#L1055) |

### vm/mod.rs / Vm<'bc, 'rt, H>::execute_instruction

```rust
fn execute_instruction(&mut self, ctx: InstructionContext<'_>) -> Result<Step>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under inst.opcode=18 \| 22` | `inst.opcode == 18` | Local state/value operations | Handled | [902](../../crates/crysirror-tjs2/src/vm/mod.rs#L902) |
| `condition; under inst.opcode=103 \| 110` | `inst.opcode == 110` | DispatchFlags::ignore_prop | Handled | [1086](../../crates/crysirror-tjs2/src/vm/mod.rs#L1086) |
| `inst.opcode; under inst.opcode=104..=106 \| 111` | `_` | DispatchFlags::default | Handled | [1101](../../crates/crysirror-tjs2/src/vm/mod.rs#L1101) |
| `condition; under inst.opcode=107 \| 112` | `inst.opcode == 112` | DispatchFlags::ignore_prop | Handled | [1109](../../crates/crysirror-tjs2/src/vm/mod.rs#L1109) |
| `inst.opcode; under inst.opcode=108 \| 109 \| 113` | `_` | DispatchFlags::default | Handled | [1123](../../crates/crysirror-tjs2/src/vm/mod.rs#L1123) |
| `condition; under inst.opcode=125` | `let Ok(getter_handle) = self.resolve_object(info.clone()) && matches!( self.runtime.heap[object_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::Class, .. } ) && matches!( self.runtime.heap[getter_handle.0].kind, ObjectKind::InterCode { context: BytecodeContextType::SuperClassGetter, .. } )` | TjsError::runtime, self.apply_class_extender, Step::Next | Handled | [1186](../../crates/crysirror-tjs2/src/vm/mod.rs#L1186) |
| `inst.opcode` | `_` | TjsError::runtime | Unsupported: opcode {} ({}) is not implemented in the VM | [1238](../../crates/crysirror-tjs2/src/vm/mod.rs#L1238) |

### vm/mod.rs / Vm<'bc, 'rt, H>::merge_nested_suspend

```rust
fn merge_nested_suspend(&mut self, stack: &mut Vec<CallFrame>, outer: CallFrame, pc: usize) -> ()
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition` | `forward && matches!(inst.opcode, 99..=101)` | Local state/value operations | Handled | [800](../../crates/crysirror-tjs2/src/vm/mod.rs#L800) |

### vm/mod.rs / Vm<'bc, 'rt, H>::run_call_stack

```rust
fn run_call_stack(&mut self, mut stack: Vec<CallFrame>, base_depth: usize) -> Result<Variant>
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `condition; under self.execute_instruction(InstructionContext { object: &call_frame.object, object_handle: call_frame.object_handle, frame: &mut call_frame.frame, inst: &call_frame.instructions[pc], index: pc, next_pc, jump: &call_frame.jump, })=Ok(Step::Next(next))` | `matches!(inst_opcode, 103 \| 107 \| 115)` | Local state/value operations | Handled | [374](../../crates/crysirror-tjs2/src/vm/mod.rs#L374) |
| `condition; under self.execute_instruction(InstructionContext { object: &call_frame.object, object_handle: call_frame.object_handle, frame: &mut call_frame.frame, inst: &call_frame.instructions[pc], index: pc, next_pc, jump: &call_frame.jump, })=Err(error)` | `self.runtime.suspended_call.is_some() && matches!(inst_opcode, 99..=102)` | self.merge_nested_suspend | Handled | [422](../../crates/crysirror-tjs2/src/vm/mod.rs#L422) |

### vm/opcode.rs / binary_family

```rust
fn binary_family(opcode: u8) -> BinaryFamily
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `(opcode - 26) / 4` | `0` | Local state/value operations | Handled | [64](../../crates/crysirror-tjs2/src/vm/opcode.rs#L64) |
| `(opcode - 26) / 4` | `1` | Local state/value operations | Handled | [65](../../crates/crysirror-tjs2/src/vm/opcode.rs#L65) |
| `(opcode - 26) / 4` | `2` | Local state/value operations | Handled | [66](../../crates/crysirror-tjs2/src/vm/opcode.rs#L66) |
| `(opcode - 26) / 4` | `3` | Local state/value operations | Handled | [67](../../crates/crysirror-tjs2/src/vm/opcode.rs#L67) |
| `(opcode - 26) / 4` | `4` | Local state/value operations | Handled | [68](../../crates/crysirror-tjs2/src/vm/opcode.rs#L68) |
| `(opcode - 26) / 4` | `5` | Local state/value operations | Handled | [69](../../crates/crysirror-tjs2/src/vm/opcode.rs#L69) |
| `(opcode - 26) / 4` | `6` | Local state/value operations | Handled | [70](../../crates/crysirror-tjs2/src/vm/opcode.rs#L70) |
| `(opcode - 26) / 4` | `7` | Local state/value operations | Handled | [71](../../crates/crysirror-tjs2/src/vm/opcode.rs#L71) |
| `(opcode - 26) / 4` | `8` | Local state/value operations | Handled | [72](../../crates/crysirror-tjs2/src/vm/opcode.rs#L72) |
| `(opcode - 26) / 4` | `9` | Local state/value operations | Handled | [73](../../crates/crysirror-tjs2/src/vm/opcode.rs#L73) |
| `(opcode - 26) / 4` | `10` | Local state/value operations | Handled | [74](../../crates/crysirror-tjs2/src/vm/opcode.rs#L74) |
| `(opcode - 26) / 4` | `11` | Local state/value operations | Handled | [75](../../crates/crysirror-tjs2/src/vm/opcode.rs#L75) |
| `(opcode - 26) / 4` | `12` | Local state/value operations | Handled | [76](../../crates/crysirror-tjs2/src/vm/opcode.rs#L76) |
| `(opcode - 26) / 4` | `13` | Local state/value operations | Handled | [77](../../crates/crysirror-tjs2/src/vm/opcode.rs#L77) |
| `(opcode - 26) / 4` | `_` | Local state/value operations | Handled | [78](../../crates/crysirror-tjs2/src/vm/opcode.rs#L78) |

### vm/opcode.rs / opcode_form

```rust
fn opcode_form(opcode: u8) -> OpcodeForm
```

| Selector / caller branch | Pattern / condition | Implementation calls | State / restriction | Source |
| --- | --- | --- | --- | --- |
| `(opcode - 26) % 4` | `0` | Local state/value operations | Handled | [84](../../crates/crysirror-tjs2/src/vm/opcode.rs#L84) |
| `(opcode - 26) % 4` | `1` | Local state/value operations | Handled | [85](../../crates/crysirror-tjs2/src/vm/opcode.rs#L85) |
| `(opcode - 26) % 4` | `2` | Local state/value operations | Handled | [86](../../crates/crysirror-tjs2/src/vm/opcode.rs#L86) |
| `(opcode - 26) % 4` | `_` | Local state/value operations | Handled | [87](../../crates/crysirror-tjs2/src/vm/opcode.rs#L87) |
