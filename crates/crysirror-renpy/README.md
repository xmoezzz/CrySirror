# crysirror-renpy

Ren'Py support for CrySirror.

**Mode: `adopt`** (ADR 0102). The owner selected a complete Rust implementation
(2026-11-26): no CPython, no hosted `renpy` package, no pygame/SDL runtime.
The workspace's [Rust Python interpreter](../crysirror-python) runs embedded
Python and the script-facing Ren'Py modules maintained here. The adapter owns
script execution and native state. CrySirror owns asset mounts, retained
layers, text, audio, input, and the window.

The installed-game launcher detects Ren'Py 6.7 through 8.6 installations
(`renpy/__init__.py`, `vc_version.py`; through 6.11 the version string
`"Ren'Py 6.10.2d"`, whose letter orders the builds) and selects the matching
release profile. The common directory is `renpy/common` from 6.15 and
`common/` beside `renpy/` before. The [validation record](../../docs/validation/renpy.md) records
behavior evidence; this README describes the implementation.

## Source identity

Semantics follow the MIT-licensed Ren'Py source
(`https://github.com/renpy/renpy`). It is inspected locally at
`local/reference/renpy`, HEAD `1feb0365bf65c68cdf0739459a882d7d2315ce4a`, with tags
6.7 through 8.6.1. Version differences are taken from `git show <tag>:renpy/...`.
The 6.99 configuration constants are generated from the MIT-licensed
6.99.12.4.2187 tag. Boot modules and common scripts are read from the original
installation through the game's read-only asset capability.

## Script decoder profiles

| Profile | Releases | Script container | Embedded Python | Selection evidence |
| --- | --- | --- | --- | --- |
| `rpyc-v1` | ≤ 6.17 | whole-file zlib pickle | Python 2 | no `RENPY RPC2` header |
| `rpyc2-py2` | 6.18 – 7.x | `RENPY RPC2` slot table (slot 1, plus slot 2 when present) + md5 trailer | Python 2 (`print_function`/`unicode_literals` first, classic grammar fallback, classic division) | header + `renpy/__init__` version tuple, or py2 pickle opcodes |
| `rpyc2-py3` | 8.x | same container | Python 3 | Ren'Py ≥ 8 version tuple |
| `rpy-source` | version-dependent statement grammar | `.rpy` text | per `rpy python` directives and the game version | file extension |

Container decoding and source parsing are broader than the installed-game launch profile.

### AST node classes per release (pickled `renpy.ast.*`)

| Release | Node classes |
| --- | --- |
| 6.18 | Say Init Label Python EarlyPython Image Transform Show ShowLayer Scene Hide With Call Return Menu Jump Pass While If UserStatement Define Screen Translate EndTranslate TranslateString TranslatePython TranslateBlock Style |
| 6.99 | + StoreNamespace Default TranslateEarlyBlock Testcase |
| 7.3 | + PostUserStatement |
| 7.4 | + Camera RPY |
| 7.7 / 8.3+ | + TranslateSay |
| 8.5 | ParameterInfo/ArgumentInfo moved to `renpy.parameter` |

## Implementation status

### Runtime architecture

The script thread (`src/rt/boot.rs`, `src/adapter.rs`) owns one Python VM per
session and runs upstream's control flow literally: `renpy.main.main`, the
init phase, `Interface.interact` and nested interactions block on the host
link (`rt/link.rs`) instead of returning to the host. `Adapter::step` sends
input and time ticks and applies the returned layer, audio, title and speech
commands to the host services.

Every upstream engine module is a native module registered under its upstream
name in `src/rt/` (owner decision 2026-11-27: reimplement, never host the
upstream Python). Minimal Python shells live in `py/rt/renpy`. Release
differences are gated on the running profile (`rt::state`), and names follow
the release that defines them:

| Layer | Modules |
| --- | --- |
| Script | `renpy.ast` (arena node handles), `parser`, `lexer`, `lexersupport`, `statements`, `script`, `loader`, `python`, `execution`, `rollback`, `revertable`, `translation` (+ `generation`, `scanstrings`, `dialogue`, `extract`, `merge`), `substitutions`, `character`, `exports` (+ the 7.8 / 8.3+ `*exports` submodules) |
| Display | `render`, `core`, `layout`, `transform`, `atl`, `motion`, `transition`, `movetransition`, `behavior`, `viewport`, `dragdrop`, `particle`, `imagemap`, `im`, `image`, `screen`, `scenelists`, `predict`, `focus`, `emulator`/`controller`/`gesture`/`joystick`, `quaternion`, `error` |
| Screens | SL2 parser, executor and displayables (`renpy.sl2.*`), `renpy.ui` |
| Text | `renpy.text.{text,font,textsupport,texwrap,shader,extras,emoji_trie,bidi}`, FreeType/HarfBuzz font objects over the shared font service |
| Audio/media | `renpy.audio.{audio,music,sound,filter,renpysound}`, `display.video`, `display.tts` |
| State | `persistent`, `preferences`, `savelocation`, `loadsave`, `savetoken`, `encryption` |
| pygame | `pygame_sdl2` (and the vendored `renpy.pygame` of 8.5+, SDL3 names in 8.6): event, key, mouse, time, scrap, display, joystick, controller, surface, image, transform, draw, gfxdraw, rwobject/iostream, locals |
| Software surfaces | `_renpy`, `renpy.display.{module,pgrender,scale,accelerator}` over `crysirror_image::fixed_raster` (core.c's fixed-point arithmetic) |
| GL drivers | `renpy.gl*`, `renpy.gl2.*`, `renpy.uguu*` with each release's names and literals (`rt/gl_api.json`); GL, Live2D Cubism and assimp operations raise naming the missing host |
| Tooling | `performance`, `debug`, `lint`, `memory`, `editor`, `scriptedit`, `test.*`, `arguments`, `dump`, `warp` |

Names a module only imports from elsewhere resolve lazily through a module
`__getattr__` from the generated `rt/import_aliases.rs`
(`tools/import_aliases.py`). `tools/api_inventory.py` and the ignored
`api_surface_matches_reference_releases` test compare every module's names
with the reference releases.

### Releases before 6.18

Each release line from 6.7 has a generated configuration profile
(`py/renpy/_profiles/config_6_7.py` ... `config_6_17.py`). Release gates
follow upstream's changes: the 6.7 – 6.11 `main()` (script before
persistent data and styles, `renpy.game.init_phase`, no `_errorhandling`),
the 6.12 – 6.18 post-init steps, `renpy.game.options` and
`RestartException` through 6.13, the pre-6.17 `sound` style property,
numbered audio channels 0 – 7 through 6.8, `ATLTransform.show()` through
6.10, `renpy.display.prefer_renderers` in 6.11 – 6.12. Python 2 `print`
statements use the softspace rules, and a repeated parameter name is
accepted as the `compiler` package of Ren'Py through 6.10 did.

Screens of 6.11 – 6.16 (screen language 1) are parsed by the SL2 parser and
run by the SL2 executor. `renpy.screenlang` itself, which the pickled
screens of compiled 6.11 – 6.16 scripts reference, is not implemented yet.

### Shared capabilities added for Ren'Py

`crysirror-text`: WOFF/WOFF2 font sources (`webfont`), pair line breaking,
emoji segmentation, bidi reordering, Arabic presentation forms, Thai C90.
`crysirror-core` `FontLibrary`: kerning, OpenType shaping and variation
metadata. `crysirror-image`: `fixed_raster` (Ren'Py's software raster
operations).

### Unavailable hosts

CrySirror owns the window, presentation and devices. Operations that need a
host this runtime does not have raise an exception that names it: OpenGL
contexts and programs, Live2D Cubism, assimp model loading, presenting a
pygame window surface, SDL joysticks/gamepads as devices (the host maps
gamepads to input itself), non-text clipboard formats, Steam.

### Remaining work

`renpy.screenlang` (SL1 screens of pre-6.18 games), `renpy.display.swdraw`,
`renpy.display.model` and the mesh/shader pipeline (`mesh`, `shader`,
`blur` transform properties), `renpy.compat.fixes`, 8.6 `renpy.util` helpers,
`renpy.versions`, `renpy.add_from`, web/mobile audio and loader backends,
`renpy.update`.

| Area | Validation |
| --- | --- |
| Fixtures | `the_question` (6.7.1b, 6.9.3c) and both games for 6.10.2d, 6.11.2b, 6.12.2, 6.13.12, 6.14.1, 6.15.7, 6.16.5 and 6.17.7 boot to the first interaction headless. `the_question` and `tutorial` (MIT, from the reference tags) for 6.18.3, 6.99.12.4, 7.0, 7.3.5, 7.4.11, 7.5.3, 7.6.3, 7.8.7, 8.0.3, 8.1.3, 8.2.3, 8.3.7, 8.4.1, 8.5.3 and 8.6.1 boot through init to the first interaction headless; see the validation record. |
| Platforms | macOS release builds. Other hosts are unverified. |

The [native contract](../../docs/native-api/renpy.md) describes profile
selection, script continuations and supported service mappings.
