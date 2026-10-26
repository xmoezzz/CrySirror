# Native adapters: engine capability extraction

Owner requirement, 2026-10-26: reference engines are capability requirements
and test inputs. The goal is a reusable CrySirror engine on which different
VMs can run through thin semantic/syscall adapters.

## Removed hosting paths

RFVP, BGI and RealLive hosted adapters and their dedicated engine packages
have been deleted, including desktop features, factories and native project
configuration schemas. Whole-frame hosting is prohibited. Earlier successful
screenshots/builds document the rejected approach, not accepted integrations.
No fallback aliases to the data VM are provided.

## Current registry

The desktop default registers `crysirror-script-v1`, `siglus`, `reallive` and
`avg32`.
The non-default desktop feature `krkr` also registers the AGPL KiriKiri
adapter (ADR 0053).
The bare Runtime registers the first two; hosts explicitly register other VMs.
`siglus` is the independent subset in crysirror-siglus-vm; it is not a full
Siglus integration. Explicit `message_window` configuration enables the plain
TEXT/NAME profile described below. Unconfigured or unmapped text fails visibly.
`avg32` executes the original upstream Scenario library's portable profile;
it does not link the original System or renderer.

## Acceptance

1. Add missing generic image/text/UI/layer/effect/audio/video/resource abilities
   to CrySirror APIs and engine-owned state, usable without any native VM.
2. Keep only opcode/value/resource-policy/layout/wait/save semantics and handle
   translation in the compatibility layer, preserving original VM behavior.
3. Native operations must call these shared APIs; do not maintain a duplicate
   native renderer or generic engine implementation inside an adapter.
4. Verify the shared capability independently, then verify real native calls,
   including inputs, state inspection, unsupported operations and saves.
5. Matching pixels or starting a native game alone is not acceptance.

Shared CPU image/composition/glyph, PCM mixing, incremental MPEG, scoped
resource decoding and native format plugins remain engine capabilities.
CBG resource decoding lives in crysirror-image::cbg without any BGI runtime.
Pinned upstream source is reference material; it is not a product host.

## RealLive replacement API bridge (2026-10-26)

`crysirror-reallive` is a new pure-VM/semantic adapter after removal of the old
hosted implementation. The desktop includes its `reallive` feature by default;
minimal builds can enable that feature alone or
register its factory explicitly; the default Runtime registry remains data VM
plus the Siglus subset. This does not reinstate RFVP/BGI hosting.

Original parser/bank/control flow remains in the VM, whose external Command/Text
events stay pending until successful CrySirror calls. Native operations target
engine Canvas/Layer/GlyphRun/UI, shared channel audio, resource capabilities and
Media services. The adapter owns no screen pixels or private generic painter.
Canvas presentations expand logical draws into shared retained Layer nodes;
media textures represent only the relevant movie resource. Shared services are
also directly available to Rust/JS/TS clients without this VM.

The source/routing audit verifies the selected upstream hashes and 90-file
reference inventory, not blanket mapping of those source files. Native opening
profile evidence is recorded separately by the integration run; the independent
full RUSUR media-service test does not establish native script/game completion.
Native save-file import/export, whole-adapter snapshots, DLL emulation and all
native layout/effect/resource variants remain unsupported or unproven.

See [REALLIVE.md](REALLIVE.md),
[ADR 0048](adr/0048-reallive-vm-engine-apis.md) and
[reallive-source-audit.json](reallive-source-audit.json).

## KiriKiri TJS2 adapter (2026-10-26)

`crysirror-krkr` runs original TJS2 scripts on the vendored Kirakira VM and
translates `Layer`, `Window`, `Timer`, `System`, `Storages` and `Scripts` onto
engine Layer/Canvas services, the engine clock and capability storage. It
added two shared capabilities first: canvas presentations attached inside a
layer tree, and a thread-safe image metadata handle. It owns no pixels,
renderer or mixer. Members without an equivalent engine operation throw a
TJS exception naming the missing capability. Font width queries use a
client-authorized engine text measurement handle, sharing the configured font
alias and simple-layout metrics with Canvas drawing. Nominal height preserves
the native absolute-height rule; unsupported font styles fail visibly
([ADR 0063](adr/0063-native-font-measurement.md)). KAG3 boot, XP3 mounts,
native font-face selection, audio, video, transitions and saves are pending;
see [ADR 0053](adr/0053-krkr-tjs2-adapter.md).

## Siglus plain message profile (2026-10-26)

The original SceneVm's native PCK/SCN TEXT/NAME instructions can issue engine
GlyphRun resources using shared fixed-cell layout and engine-owned reveal
state. The adapter queries reveal completion through client-scoped engine
state; it does not rasterize glyphs or maintain a second typewriter clock.
The profile requires explicit font/window mapping for `FRONT.MWND[0]`, plain
text and read-flag operand `-1`. Chunk append, first-name policy, DECIDE edge
consumption and runtime snapshots have dedicated behavior tests.

Automatic Gameexe MWND conversion, other targets, markup, read-flag block
commits, pages, backlog and original MWND save files remain unsupported.
Supplying a different font does not establish matching native glyph metrics.
See [ADR 0061](adr/0061-siglus-plain-message-engine-apis.md).

## AVG32 original VM library (2026-10-26)

`crysirror-avg32` depends on `siglus_rs` commit
`256c398e06fbcbae8e956eb046c831b3d6c8a593`, with `portable-vm` enabled and
default features disabled. The original Scenario/parser/control-flow source
is compiled there; no interpreter source was copied into CrySirror. The
portable boundary was added in the original repository on a separate branch.
Its compiled dependency tree excludes the old renderer, painter and mixer.

The bounded `plain-adv-v1` adapter translates image loads into engine Canvas,
text into shared grid/GlyphRun state, choices into engine UI, and BGM/WAV/voice
into shared channel audio. Original waits, pending native events and hidden
choice ordinals are retained. Resource aliases stay inside client roots.
Only opaque full-size image loads and the documented plain text profile are
mapped initially. Other drawing/effect/layout/movie operations and both native
saves and framework snapshots fail visibly. No commercial AVG32 game is
claimed complete. See [ADR 0062](adr/0062-avg32-portable-vm-engine-apis.md).

## Shared modular effects (2026-10-26)

`EntitySpec.vfx` / `WorldCommand::SetVfx` now declare the same reusable effect
systems as the JS/TS `Vfx` and `Entity` APIs. The safe core owns compilation,
CPU simulation/events and GPU tick plans; runtime workers and renderer-owned
GPU resources implement execution. A native particle/effect instruction should
translate its scheduling/resource/coordinate semantics into these declarations
or add a missing shared module first. It must not copy a private VM particle
simulator or render a legacy engine framebuffer. This capability addition does
not itself establish new native Siglus/AVG32/RealLive opcode coverage. See
[ADR 0064](adr/0064-modular-vfx-cpu-gpu.md) for supported modules and explicit
backend gaps.
