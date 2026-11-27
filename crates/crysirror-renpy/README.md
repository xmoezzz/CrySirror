# crysirror-renpy

Ren'Py support for CrySirror.

**Mode: `adopt`** (ADR 0102). The owner selected a complete Rust implementation
(2026-11-26): no CPython, no hosted `renpy` package, no pygame/SDL runtime.
The workspace's [Rust Python interpreter](../crysirror-python) runs embedded
Python and the script-facing Ren'Py modules maintained here. The adapter owns
script execution and native state. CrySirror owns asset mounts, retained
layers, text, audio, input, and the window.

The installed-game launcher recognizes **6.99.12.4**. Source fixtures exercise
dialogue, branching, ATL motion, screen callbacks and persistent data. The
[validation record](../../docs/validation/renpy.md) records the latest
installed DDLC result and the limits of the release profile.

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

| Area | Current behavior |
| --- | --- |
| Source and compiled scripts | Source lexer/parser, RPC2 slots, pickle shells and AST conversion. Original 6.99 DDLC archive scripts decode through shared Resources. |
| Bootstrap | Profile detection, loose and mounted archive data, startup `.rpym[c]` modules, early Python, scoped Python packages and priority-ordered init. |
| Script execution | Labels, parameter binding, dynamic scope restoration, conditional branches, `call`, `return`, `jump` and registered statement parse/execute callbacks. |
| Interaction | Dialogue, conditional choices, timed or input waits and nested label calls through suspendable Python tasks. |
| Images | Named image definitions, tag/layer order, image/solid/text displayables and shared cross-dissolve. |
| Audio | Registered channels and aliases, single-asset playback, looping, and fades through CrySirror's channel audio service. |
| ATL | Sequential placement, scaling, opacity and child steps use the host clock and shared layer presentation. Parallel blocks, event handlers, splines and custom displayables require additional execution paths. |
| Screens | SL2 text, buttons, containers, conditionals, iteration and screen inclusion use retained widgets. Activated widgets invoke script actions. Authored dialogue screens can replace the built-in dialogue panel. |
| State | Dialogue and menu rollback restore mutable objects and preserve the excluded persistent and `NoRollback` graphs. Persistent data and preferences use the 6.99 zlib-wrapped pickle format. Native game save slots and runtime snapshots remain unsupported. |
| Platforms | macOS release and offscreen validation. Windows, Linux, mobile, browser and console execution are unverified. |

The [native contract](../../docs/native-api/renpy.md) describes profile
selection, script continuations and supported service mappings.
