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

KiriKiri's menu.dll now names a facade over CrySirror MenuSpec/Ui operations.
Explicit registration-only plugin shells are allowed through Config.plugin_shells;
they export no fake functionality and report their status. Shell registration
is not capability extraction or game compatibility. Native synchronous popup
waits and unmapped plugin methods continue to fail visibly. See
[ADR 0066](adr/0066-awaitable-services-and-native-plugin-facades.md).

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

## Native resource codecs (2026-10-26)

The complete named arc_unpacker inventory is now exposed by CrySirror resource
registries: archives become client VFS providers, pictures use the shared image
registry, and encoded files/audio become engine-owned readers. A VM adapter
requests these APIs and translates native names/parameters; it must not run an
unpacker or retain a private media engine. `Resources.beginDecode` and ordinary
async Assets loading provide the task/completion boundary. Keys are host aliases
and companion paths stay under the same client capability. Codec availability
alone does not establish VM/opcode, game, save or renderer compatibility. See
[ADR 0065](adr/0065-native-resource-codecs-and-async-decode.md) and the explicit
[resource](arc-resource-support.json)/[image](arc-image-support.json) ledgers.


## Native audio readiness (2026-10-27)

Mapped RealLive BGM/WAV calls and AVG32 music/wave/SE/voice events now submit
AudioAsset paths through HostServices::begin_audio and retain their original
pending events until audio_preparation reports owner publication. Opening,
probing, intro/loop/startup preparation and streaming use shared ChannelAudio
and BufferedSource workers. Admission is explicitly distinct from readiness.
Failures do not acknowledge following native instructions. Offline fixtures
retain immediate preparation behavior; original save/layout/opcode gaps remain.
Resource discovery and boot reads are still synchronous. See [ADR 0070](adr/0070-native-channel-audio-readiness.md).

## Shared late-reverb buses (ADR 0073)

Authored late reverberation is an engine resource over the standalone FunDSP
FDN, not an algorithm in a native adapter. Audio.prepareReverbAsync publishes
matched resources in ordinary and native/media channel mixers. Original native
play/wait handlers still target shared channel APIs and observe source completion;
the bus owns remaining tails independently. Adapter source/opcodes/save semantics
are unchanged. Original RealLive/AVG32 sound fixtures exercise that boundary.
This parametric capability does not establish geometry reflections, a physical
room model or additional whole-game compatibility.

## Captured early reflections and native sound completion (ADR 0074)

Finite one-bounce image-source paths, band transport, sparse delay histories and
reflection preparation/publication are shared engine capabilities. Rust channel
services can install those resources without adapter algorithms. Modern App
clients also use reflection probes and prepareVoiceReflectionsAsync. Source
completion is distinct from retained echo/HRTF tails: original RealLive/AVG32
sound waits still advance at source EOF while engine echo state drains. Native
adapter implementations/opcodes/save layouts are unchanged. This is a static
pose profile; dynamic tracking and higher-order/diffuse propagation remain open.


## Shared moving reflection plans (2026-10-27)

ADR 0075 adds reusable ReflectionPlan and persistent EarlyReflections history,
with Mixer/ChannelAudio publication and source-completion semantics independent
of App/VM/device. Modern engine Audio can automatically coalesce entity/listener
updates; native adapters can translate a spatial operation onto those shared
resources without private echo algorithms. This phase adds no native opcode or
save mapping. Original RealLive/AVG32 bridge fixtures still verify source waits
while engine-owned effects drain; that bounded evidence does not establish full
game support or native automatic spatialization.


## Original Siglus BGM through shared transport (ADR 0076)

BGM forms now translate native arguments, state codes and wait predicates onto
HostServices begin_audio/audio_preparation/submit_audio/audio_info. The adapter
owns no decoder/mixer/fader. Resource admission keeps the VM at its original
command until owner publication; failures do not acknowledge following arithmetic.
READY/pause/delayed resume, source/fade waits, WAIT_KEY return values, registered
name/position/volume and same-loop-name semantics have authored native fixtures.
BGMTABLE flags remain private native logical/save state.

An explicit resources alias maps each native music file to a client engine asset.
Optional decoded UTF-8 Gameexe BGM rows supply start/end/repeat frames, bounded
at 4096 entries. Discovery/append dirs, encrypted Gameexe and native pack-entry
selection are not inferred. Unsupported opcodes/property fallbacks fail visibly.
ADR 0078 adds global PCM and SE below; PCMCH and KOE speech remain unbound.
ADR 0077 adds immutable
CrySirror resource handles: native PLAY/READY commits after resource and voice
preparation; RESUME after EOF/stop replays the retained resource without reopening
its original path. Stopped runtime snapshots reconstruct resources without hidden
playback. Handles are ephemeral; rebuilding a snapshot still needs the assets.

Runtime Siglus payload schema is 3 (ADR 0078); schema-2 snapshots require migration.
Pending checkpoints replay uncommitted native
commands; active restores reprepare declarations at authored start. Original file
codecs remain intact, with BGM persistence translated through shared preparation.
No whole-game, every-save-variant, device or other-platform claim follows. See
[siglus-bgm-source-audit.json](siglus-bgm-source-audit.json) and ADR 0076.

## Original PCM/global SE and shared gain groups (ADR 0078)

Global PCM PLAY/STOP and SE filename/number/explicit voice-alias play, volume,
stop/fade, checks and waits now use shared audio resource/channel/group APIs.
`siglus.pcm` and `siglus.se` are client-owned gain groups; the adapter holds no
mixer, source buffer, decoder cache or private timer. Resource/voice completion
keeps original instructions pending; one-shot handles release at admission while
actual decoder leases retain bytes. Ordered semantic actions preserve cross-
category stop-before-BGM-prepare behavior on original file import.

Original integer-ms SFX end predicates use shared publication age, distinct from
physical source/effect EOF. Polling follows the current engine frame phase;
exact timer/device timing under starvation is not established. Decoded Gameexe
SE table count follows native 16/default, 8/min and 256/max policy. Every native
source needs a canonical resources alias; `koe/{number}` is an explicit CrySirror
mapping for SE voice-number calls, without automatic pack/filename discovery.
Missing/unknown operations fail visibly.

Runtime payload schema is 3 for PCM/SE logical declarations. Active snapshots
reconstruct after commit, without exact sample/age/DSP history. Original file
layouts remain unchanged; import stops nonpersistent PCM/SE and restores their
category volume while preserving host group parents/mute. PCMCH, PCM events,
general KOE/JITAN/mouth tables, native UI sound slots and full game/save coverage
remain open. Source audit: [siglus-sfx-source-audit.json](siglus-sfx-source-audit.json).

## Final audio worker dependency (ADR 0079)

Native semantic adapters continue using the same shared resources, transport and
channel completion contracts. Live App/desktop final DSP now uses shared CPU
workers; observations remain frozen at the previous owner publication while that
frame is pending. The host collects it before another VM/world tick. Backpressure
must not rerun the native instruction or consume another input edge. Restore
invalidates the obsolete frame and drops its PCM. This adds no legacy mixer or
new VM hooks, and does not establish exact native wall-clock timing or an
independent device clock. See [ADR 0079](adr/0079-worker-final-audio-frames.md).

## Host audio output ownership (ADR 0080)

Native channel resources, opcode waits and original save declarations continue
using CrySirror APIs. The output device is now owned by an independent host
worker, with separate readiness/failure/close observations and bounded stereo
PCM transfer. Native waits still observe logical source/DSP publication, not
hardware playback counters. No device handle is given to a native VM, and no
legacy engine audio runtime is introduced. See [ADR 0080](adr/0080-async-output-worker-and-bounded-pcm-transfer.md).
